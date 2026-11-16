use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::json;

use super::*;
use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
use crate::core::serde::CanonicalJsonObject;
use crate::core::signatures::{Ed25519KeyPair, sign_json};
use crate::core::{OwnedDeviceId, UnixMillis};

const LOCAL: &str = "stripped.example";
const REMOTE: &str = "peer.stripped.example";
const PREFIX: &str =
    "http://localhost/_matrix/client/unstable/org.continuwuity.msc4560.stripped_state/";

fn selected() -> Selection {
    serde_json::from_value(json!({"state":["m.space.child"],"num_joined_members":true})).unwrap()
}

#[test]
fn sanitizes_peer_pdus_and_preserves_all_space_keys_and_zero_count() {
    let events = ["!a:example", "!b:example"].map(|key| json!({
        "type":"m.space.child", "state_key":key, "sender":"@alice:example", "content":{"via":["example"]},
        "event_id":"$secret", "room_id":"!private:example", "auth_events":["$auth"],
        "signatures":{"private":"secret"}, "origin_server_ts":123, "unsigned":{"private":"secret"}
    }));
    let response = serde_json::from_value(
        json!({"state":events,"num_joined_members":0,"via":"forged.example"}),
    )
    .unwrap();
    let value = serde_json::to_value(
        sanitize(response, &selected(), "actual.example".try_into().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(value["via"], "actual.example");
    assert_eq!(value["num_joined_members"], 0);
    assert_eq!(value["state"].as_array().unwrap().len(), 2);
    for event in value["state"].as_array().unwrap() {
        assert_eq!(event.as_object().unwrap().len(), 4);
        assert!(event.get("event_id").is_none());
    }
}

#[test]
fn rejects_bad_peer_types_duplicates_senders_counts_and_oversized_content() {
    let event =
        json!({"type":"m.space.child","state_key":"!a:example","sender":"@a:example","content":{}});
    let invalid = [
        json!({"state":[{"type":"m.room.member","state_key":"@a:example","sender":"@a:example","content":{}}],"num_joined_members":0}),
        json!({"state":[event.clone(),event],"num_joined_members":0}),
        json!({"state":[{"type":"m.space.child","state_key":"!a:example","sender":"bad","content":{}}],"num_joined_members":0}),
        json!({"state":[]}),
        json!({"state":[{"type":"m.space.child","state_key":"!a:example","sender":"@a:example","content":{"large":"x".repeat(MAX_RESPONSE)}}],"num_joined_members":0}),
    ];
    for body in invalid {
        let response = serde_json::from_value(body).unwrap();
        assert!(sanitize(response, &selected(), "actual.example".try_into().unwrap()).is_err());
    }
    let response = serde_json::from_value(json!({"state":[],"num_joined_members":42})).unwrap();
    let body = sanitize(
        response,
        &Selection::default(),
        "actual.example".try_into().unwrap(),
    )
    .unwrap();
    assert!(body.num_joined_members.is_none());
}

#[tokio::test]
async fn oversized_peer_bodies_are_rejected_before_eof() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for chunked in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).await.unwrap();
                header.push(byte[0]);
            }
            let wire = if chunked {
                format!(
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n41\r\n{}\r\n",
                    "x".repeat(65)
                )
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 65\r\n\r\n".to_owned()
            };
            socket.write_all(wire.as_bytes()).await.unwrap();
            std::future::pending::<()>().await;
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/state"))
            .send()
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), read_limited(response, 64))
            .await
            .expect("oversized responses must not wait for EOF");
        assert!(result.is_err());
        task.abort();
    }
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_stripped_state_visibility_validation_and_signed_federation() {
    run(true);
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_stripped_state_disabled() {
    run(false);
}

fn run(enabled: bool) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(http_contract(enabled));
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn request(service: &Service, request: RequestBuilder) -> (StatusCode, Value) {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        (
            response.status_code.unwrap(),
            response.take_json().await.unwrap_or(Value::Null),
        )
    })
    .await
    .unwrap()
}

fn client(room: impl std::fmt::Display, body: Value, token: Option<&str>) -> RequestBuilder {
    let mut url = reqwest::Url::parse(PREFIX).unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(&room.to_string());
    let request = TestClient::post(url.as_str()).json(&body);
    match token {
        Some(token) => request.add_header("Authorization", format!("Bearer {token}"), true),
        None => request,
    }
}

async fn create(service: &Service, body: Value) -> OwnedRoomId {
    let (status, body) = request(
        service,
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", "Bearer alice", true)
            .json(&body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["room_id"].as_str().unwrap().try_into().unwrap()
}

async fn put(service: &Service, room: &RoomId, kind: &str, key: &str, body: Value) {
    let mut url = reqwest::Url::parse("http://localhost/_matrix/client/v3/rooms/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .extend([room.as_str(), "state", kind, key]);
    let (status, body) = request(
        service,
        TestClient::put(url.as_str())
            .add_header("Authorization", "Bearer alice", true)
            .json(&body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn signed(
    service: &Service,
    key: &Ed25519KeyPair,
    room: &RoomId,
    body: Value,
) -> (StatusCode, Value) {
    let path =
        format!("/_matrix/federation/unstable/org.continuwuity.msc4560.stripped_state/{room}");
    let mut signed: CanonicalJsonObject = serde_json::from_value(
        json!({"method":"POST","uri":path,"origin":REMOTE,"destination":LOCAL,"content":body}),
    )
    .unwrap();
    sign_json(REMOTE, key, &mut signed).unwrap();
    let signed = serde_json::to_value(signed).unwrap();
    let signature = signed["signatures"][REMOTE]["ed25519:review"]
        .as_str()
        .unwrap();
    request(service, TestClient::post(format!("http://localhost{path}")).json(&body)
        .add_header("Authorization", format!("X-Matrix origin=\"{REMOTE}\",destination=\"{LOCAL}\",key=\"ed25519:review\",sig=\"{signature}\""), true)).await
}

async fn http_contract(enabled: bool) {
    crate::test_database::init();
    config::CONFIG.set(serde_json::from_value(json!({
        "server_name":LOCAL,"db":{"url":"unused"},"auto_join_rooms":[],"enable_stripped_state":enabled,
        "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false},
        "rc_message":{"per_second":1000.0,"burst":1000}
    })).unwrap()).unwrap();
    for name in ["alice", "bob"] {
        let id = UserId::parse(format!("@{name}:{LOCAL}")).unwrap();
        crate::data::user::create_user(&crate::data::user::NewDbUser {
            id: id.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: name.into(),
            server_name: id.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        crate::data::user::device::create_device(&id, &OwnedDeviceId::from("D"), name, None, None)
            .await
            .unwrap();
    }
    let service = Service::new(crate::routing::matrix());
    let (_, versions) = request(
        &service,
        TestClient::get("http://localhost/_matrix/client/versions"),
    )
    .await;
    assert_eq!(
        versions["unstable_features"]["org.continuwuity.msc4560"]
            .as_bool()
            .unwrap_or(false),
        enabled
    );
    if !enabled {
        assert_eq!(
            request(&service, client("!unknown:example", json!({}), None))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        return;
    }
    let public = create(&service, json!({"room_version":"11","preset":"public_chat","room_alias_name":"preview","name":"Public"})).await;
    for child in ["!one:example", "!two:example"] {
        put(
            &service,
            &public,
            "m.space.child",
            child,
            json!({"via":["example"]}),
        )
        .await;
    }
    let body = json!({"state":["m.room.name","m.space.child"],"num_joined_members":true});
    let (status, public_state) = request(&service, client(&public, body.clone(), None)).await;
    assert_eq!(status, StatusCode::OK, "{public_state}");
    assert_eq!(public_state["via"], LOCAL);
    assert_eq!(public_state["num_joined_members"], 1);
    assert_eq!(public_state["state"].as_array().unwrap().len(), 3);
    for event in public_state["state"].as_array().unwrap() {
        assert_eq!(event.as_object().unwrap().len(), 4);
    }
    assert_eq!(
        request(
            &service,
            client(format!("#preview:{LOCAL}"), body.clone(), None)
        )
        .await
        .1,
        public_state
    );
    assert_eq!(
        request(&service, client(&public, json!({}), None)).await.1,
        json!({"state":[],"via":LOCAL})
    );
    let private = create(
        &service,
        json!({"room_version":"11","preset":"private_chat","name":"Secret"}),
    )
    .await;
    for selection in [
        json!({}),
        json!({"num_joined_members":true}),
        json!({"state":["m.room.name"]}),
    ] {
        assert_eq!(
            request(&service, client(&private, selection, None)).await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        request(&service, client(&private, json!({}), Some("alice")))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&service, client(&private, json!({}), Some("bob")))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&service, client(&public, json!({}), Some("invalid")))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // Invalid selection wins over alias lookup and over local visibility denial.
    let (status, error) = request(
        &service,
        client(
            "#unknown:unreachable.invalid",
            json!({"state":["m.room.member"]}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["errcode"], "M_INVALID_PARAM");
    let mut url = reqwest::Url::parse(PREFIX).unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(private.as_str());
    for _ in 0..33 {
        url.query_pairs_mut().append_pair("via", REMOTE);
    }
    let (status, error) = request(&service, TestClient::post(url.as_str()).json(&json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["errcode"], "M_TOO_LARGE");
    url.set_query(None);
    url.query_pairs_mut().append_pair("via", REMOTE);
    assert_eq!(
        request(&service, TestClient::post(url.as_str()).json(&json!({})))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let huge = json!({"padding":"x".repeat(MAX_REQUEST),"state":[]});
    assert_eq!(
        request(&service, client(&public, huge, None)).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let fed_url = format!(
        "http://localhost/_matrix/federation/unstable/org.continuwuity.msc4560.stripped_state/{public}"
    );
    let (status, fed) = request(&service, TestClient::post(&fed_url).json(&body)).await;
    assert_eq!(status, StatusCode::OK, "{fed}");
    assert_eq!(fed["state"], public_state["state"]);
    assert!(fed.get("via").is_none());
    assert_eq!(
        request(
            &service,
            TestClient::post(&fed_url).json(&body).add_header(
                "Authorization",
                "X-Matrix invalid",
                true
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let key =
        Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), "review".into()).unwrap();
    let mut keys = ServerSigningKeys::new(
        REMOTE.try_into().unwrap(),
        UnixMillis(UnixMillis::now().0 + 86_400_000),
    );
    keys.verify_keys.insert(
        "ed25519:review".try_into().unwrap(),
        VerifyKey::from_bytes(key.public_key().to_vec()),
    );
    crate::server_key::seed_verified_keys(keys, &key).await;
    assert_eq!(
        signed(&service, &key, &private, json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    let peer_server = ServerName::parse(REMOTE).unwrap();
    crate::data::room::add_joined_server(&private, &peer_server)
        .await
        .unwrap();
    assert_eq!(
        signed(&service, &key, &private, json!({})).await.0,
        StatusCode::OK
    );
    put(
        &service,
        &public,
        "m.room.server_acl",
        "",
        json!({"allow":["*"],"deny":[REMOTE]}),
    )
    .await;
    assert_eq!(
        signed(&service, &key, &public, json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    // Anonymous calls have no claimed server origin; ACL rules still apply to
    // signed calls rather than inventing an origin for anonymous clients.
    assert_eq!(
        request(&service, TestClient::post(&fed_url).json(&body))
            .await
            .0,
        StatusCode::OK
    );
    let bob = format!("@bob:{LOCAL}");
    let (status, result) = request(
        &service,
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{private}/invite"
        ))
        .add_header("Authorization", "Bearer alice", true)
        .json(&json!({"user_id":bob})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        request(&service, client(&private, json!({}), Some("bob")))
            .await
            .0,
        StatusCode::OK
    );
    // Restricted joins use any matching m.room_membership allow rule.
    let (status, result) = request(
        &service,
        TestClient::post(format!("http://localhost/_matrix/client/v3/join/{public}"))
            .add_header("Authorization", "Bearer bob", true)
            .json(&json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let restricted = create(&service, json!({"room_version":"11","preset":"private_chat","initial_state":[{
        "type":"m.room.join_rules","state_key":"","content":{"join_rule":"restricted","allow":[
            {"type":"m.room_membership","room_id":"!unrelated:example"}, {"type":"m.room_membership","room_id":public}
        ]}
    }]})).await;
    assert_eq!(
        request(&service, client(&restricted, json!({}), Some("bob")))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&service, client(&restricted, json!({}), None))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    crate::data::room::add_joined_server(&public, &peer_server)
        .await
        .unwrap();
    assert_eq!(
        signed(&service, &key, &restricted, json!({})).await.0,
        StatusCode::OK
    );
    put(
        &service,
        &private,
        "m.room.history_visibility",
        "",
        json!({"history_visibility":"world_readable"}),
    )
    .await;
    assert_eq!(
        request(&service, client(&private, json!({}), None)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(&service, client("!unknown:example", json!({}), None))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}
