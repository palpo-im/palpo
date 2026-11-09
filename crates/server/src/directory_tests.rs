//! End-to-end directory contracts, including the signed federation boundary.
use std::sync::{Arc, Mutex};

use salvo::conn::Acceptor;
use salvo::conn::rustls::{Keycert, RustlsConfig};
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
use crate::core::serde::CanonicalJsonObject;
use crate::core::signatures::{Ed25519KeyPair, sign_json};
use crate::core::{OwnedDeviceId, UnixMillis, UserId};
use crate::{config, data};

#[path = "server_key/validity/notary_fixture.rs"]
mod tls_fixture;

const LOCAL: &str = "directory-review.example";
const REMOTE: &str = "directory-origin.example";
const TOKEN: &str = "directory-review-token";

async fn send(request: RequestBuilder, service: &Service) -> (StatusCode, Value) {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        let status = response.status_code.unwrap();
        (status, response.take_json().await.unwrap())
    })
    .await
    .unwrap()
}

async fn ok(request: RequestBuilder, service: &Service) -> Value {
    let (status, body) = send(request, service).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn client_post(body: &Value) -> RequestBuilder {
    TestClient::post("http://localhost/_matrix/client/v3/publicRooms")
        .add_header("authorization", format!("Bearer {TOKEN}"), true)
        .json(body)
}

fn federation(key: &Ed25519KeyPair, path: &str, body: Option<&Value>) -> RequestBuilder {
    let method = if body.is_some() { "POST" } else { "GET" };
    let mut signed: CanonicalJsonObject = serde_json::from_value(json!({
        "method":method,"uri":path,"origin":REMOTE,"destination":LOCAL
    }))
    .unwrap();
    if let Some(body) = body {
        signed.insert(
            "content".into(),
            serde_json::from_value(body.clone()).unwrap(),
        );
    }
    sign_json(REMOTE, key, &mut signed).unwrap();
    let signed = serde_json::to_value(signed).unwrap();
    let signature = signed["signatures"][REMOTE]["ed25519:directory"]
        .as_str()
        .unwrap();
    let request = if let Some(body) = body {
        TestClient::post(format!("http://localhost{path}")).json(body)
    } else {
        TestClient::get(format!("http://localhost{path}"))
    };
    request.add_header("authorization", format!(
        "X-Matrix origin=\"{REMOTE}\",destination=\"{LOCAL}\",key=\"ed25519:directory\",sig=\"{signature}\""
    ), true)
}

fn ids(body: &Value) -> Vec<String> {
    body["chunk"]
        .as_array()
        .unwrap()
        .iter()
        .map(|room| room["room_id"].as_str().unwrap().to_owned())
        .collect()
}

fn assert_rooms(body: &Value, expected: &[String]) {
    let mut actual = ids(body);
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(actual, expected, "{body}");
    assert_eq!(body["total_room_count_estimate"], expected.len(), "{body}");
}

#[derive(Clone)]
struct Peer {
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

type CapturedRequest = (String, String, Value);

#[async_trait::async_trait]
impl Handler for Peer {
    async fn handle(
        &self,
        req: &mut Request,
        _depot: &mut Depot,
        res: &mut Response,
        _ctrl: &mut FlowCtrl,
    ) {
        let body: Value = req.parse_json().await.unwrap();
        let authorization = req.headers()["authorization"].to_str().unwrap();
        assert!(authorization.starts_with("X-Matrix "));
        self.requests.lock().unwrap().push((
            req.method().to_string(),
            req.uri().path().into(),
            body,
        ));
        res.render(Json(json!({
            "chunk":[{"room_id":"!remote:directory-peer.example","name":"remote sentinel",
                "num_joined_members":3,"world_readable":false,"guest_can_join":false,"room_type":"m.space"}],
            "next_batch":"remote-next","prev_batch":"remote-prev","total_room_count_estimate":7
        })));
    }
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_public_rooms_filters_networks_and_bidirectional_pages() {
    std::thread::Builder::new()
        .name("directory-review-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn run() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":LOCAL,"db":{"url":"unused-test-config"},"auto_join_rooms":[],
                "ip_range_denylist":[],"allow_invalid_tls_certificates":true,
                "http_client":{"federation_retries":0}
            }))
            .unwrap(),
        )
        .unwrap();
    let owner = UserId::parse(format!("@owner:{LOCAL}")).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: owner.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "owner".into(),
        server_name: owner.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&owner).await.unwrap();
    data::user::device::create_device(&owner, &OwnedDeviceId::from("D"), TOKEN, None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&owner).await.unwrap();
    let key =
        Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), "directory".into()).unwrap();
    let mut keys = ServerSigningKeys::new(
        REMOTE.try_into().unwrap(),
        UnixMillis(UnixMillis::now().0 + 86_400_000),
    );
    keys.verify_keys.insert(
        "ed25519:directory".try_into().unwrap(),
        VerifyKey::from_bytes(key.public_key().to_vec()),
    );
    crate::server_key::add_signing_keys(keys).await.unwrap();
    let service = Service::new(crate::routing::root());
    let mut ordinary = Vec::new();
    let mut spaces = Vec::new();
    let mut custom = Vec::new();
    for (index, ty) in [
        None,
        None,
        Some("m.space"),
        Some("m.space"),
        Some("org.example.custom"),
        None,
    ]
    .into_iter()
    .enumerate()
    {
        let mut body = json!({"room_version":"11","preset":"public_chat","visibility":"public","name":format!("directory-{index}")});
        if let Some(ty) = ty {
            body["creation_content"] = json!({"type":ty});
        }
        if index == 5 {
            body["visibility"] = json!("private");
        }
        let created = ok(
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .add_header("authorization", format!("Bearer {TOKEN}"), true)
                .json(&body),
            &service,
        )
        .await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        if index == 5 {
            continue;
        }
        match ty {
            None => ordinary.push(room),
            Some("m.space") => spaces.push(room),
            _ => custom.push(room),
        }
    }
    let mut all = ordinary
        .iter()
        .chain(&spaces)
        .chain(&custom)
        .cloned()
        .collect::<Vec<_>>();
    all.sort();
    let unfiltered = ok(
        TestClient::get("http://localhost/_matrix/client/v3/publicRooms"),
        &service,
    )
    .await;
    assert_rooms(&unfiltered, &all);
    assert!(unfiltered.get("next_batch").is_none());
    assert!(unfiltered.get("prev_batch").is_none());
    let first = ok(
        federation(&key, "/_matrix/federation/v1/publicRooms?limit=2", None),
        &service,
    )
    .await;
    assert_eq!(ids(&first), all[..2]);
    assert_eq!(first["total_room_count_estimate"], all.len());
    let second_path = format!(
        "/_matrix/federation/v1/publicRooms?limit=2&since={}",
        first["next_batch"].as_str().unwrap()
    );
    let second = ok(federation(&key, &second_path, None), &service).await;
    assert_eq!(ids(&second), all[2..4]);
    // Empty and omitted filters retain the existing all-types contract.
    for (filter, expected) in [
        (json!({}), all.clone()),
        (json!({"room_types":[]}), all.clone()),
        (json!({"room_types":[null]}), ordinary.clone()),
        (json!({"room_types":["m.space"]}), spaces.clone()),
        (json!({"room_types":["org.example.custom"]}), custom.clone()),
        (json!({"room_types":["org.example.absent"]}), Vec::new()),
        (
            json!({"room_types":[null,"m.space"]}),
            ordinary.iter().chain(&spaces).cloned().collect(),
        ),
    ] {
        let body = json!({"filter":filter});
        let client = ok(client_post(&body), &service).await;
        assert_rooms(&client, &expected);
        let federated = ok(
            federation(&key, "/_matrix/federation/v1/publicRooms", Some(&body)),
            &service,
        )
        .await;
        assert_rooms(&federated, &expected);
    }
    let search = ok(
        client_post(&json!({"filter":{"room_types":[null],"generic_search_term":"DIRECTORY-1"}})),
        &service,
    )
    .await;
    assert_rooms(&search, &ordinary[1..2]);
    let search = ok(
        client_post(
            &json!({"filter":{"room_types":["m.space"],"generic_search_term":"DIRECTORY-1"}}),
        ),
        &service,
    )
    .await;
    assert_rooms(&search, &[]);

    // The same fixed, filtered directory can be traversed in both directions.
    let filter = json!({"room_types":[null,"m.space"]});
    let mut expected = ordinary.iter().chain(&spaces).cloned().collect::<Vec<_>>();
    expected.sort();
    let mut pages = Vec::new();
    let mut since = None;
    for room in &expected {
        let mut body = json!({"limit":1,"filter":filter});
        if let Some(token) = &since {
            body["since"] = json!(token);
        }
        let page = ok(client_post(&body), &service).await;
        assert_eq!(ids(&page).as_slice(), std::slice::from_ref(room));
        assert_eq!(page["total_room_count_estimate"], expected.len());
        let federated = ok(
            federation(&key, "/_matrix/federation/v1/publicRooms", Some(&body)),
            &service,
        )
        .await;
        assert_eq!(ids(&federated), ids(&page));
        assert_eq!(
            federated["total_room_count_estimate"],
            page["total_room_count_estimate"]
        );
        since = page["next_batch"].as_str().map(ToOwned::to_owned);
        pages.push(page);
    }
    assert!(since.is_none(), "last filtered page must terminate");
    for index in (1..pages.len()).rev() {
        let body = json!({"limit":1,"filter":filter,"since":pages[index]["prev_batch"]});
        let previous = ok(client_post(&body), &service).await;
        assert_eq!(ids(&previous), expected[index - 1..index]);
        let previous = ok(
            federation(&key, "/_matrix/federation/v1/publicRooms", Some(&body)),
            &service,
        )
        .await;
        assert_eq!(ids(&previous), expected[index - 1..index]);
    }
    let previous = ok(
        client_post(&json!({"limit":2,"filter":filter,"since":pages[1]["prev_batch"]})),
        &service,
    )
    .await;
    assert_eq!(
        ids(&previous),
        expected[..1],
        "a larger backward limit must not cross its end boundary"
    );
    let unlimited = ok(client_post(&json!({"limit":0})), &service).await;
    assert_rooms(&unlimited, &all);
    assert!(unlimited.get("next_batch").is_none());

    // There are no appservice network publications: that write API rejects
    // them explicitly. All includes Matrix; a named network returns no entries.
    for (network, expected) in [
        (json!({"include_all_networks":true}), all.clone()),
        (
            json!({"third_party_instance_id":"not-published"}),
            Vec::new(),
        ),
    ] {
        assert_rooms(&ok(client_post(&network), &service).await, &expected);
        assert_rooms(
            &ok(
                federation(&key, "/_matrix/federation/v1/publicRooms", Some(&network)),
                &service,
            )
            .await,
            &expected,
        );
    }
    assert_rooms(
        &ok(
            federation(
                &key,
                "/_matrix/federation/v1/publicRooms?include_all_networks=true",
                None,
            ),
            &service,
        )
        .await,
        &all,
    );
    assert_rooms(
        &ok(
            federation(
                &key,
                "/_matrix/federation/v1/publicRooms?third_party_instance_id=not-published",
                None,
            ),
            &service,
        )
        .await,
        &[],
    );
    assert_eq!(send(federation(&key,"/_matrix/federation/v1/publicRooms?include_all_networks=true&third_party_instance_id=both",None),&service).await.0,StatusCode::BAD_REQUEST);
    let conflict = json!({"include_all_networks":true,"third_party_instance_id":"both"});
    assert_eq!(
        send(client_post(&conflict), &service).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(
            federation(&key, "/_matrix/federation/v1/publicRooms", Some(&conflict)),
            &service
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, unsigned) = send(
        TestClient::post("http://localhost/_matrix/federation/v1/publicRooms").json(&json!({})),
        &service,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{unsigned}");
    assert_eq!(unsigned["errcode"], "M_MISSING_TOKEN");
    assert!(unsigned.get("chunk").is_none());
    let body_server =
        json!({"server":"must-not-forward.example","filter":{"room_types":["m.space"]}});
    assert_rooms(
        &ok(
            federation(
                &key,
                "/_matrix/federation/v1/publicRooms",
                Some(&body_server),
            ),
            &service,
        )
        .await,
        &spaces,
    );

    // A real loopback HTTPS peer observes the exact signed outbound POST.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let acceptor = TcpListener::new("127.0.0.1:0")
        .rustls(RustlsConfig::new(
            Keycert::new().cert(tls_fixture::CERT).key(tls_fixture::KEY),
        ))
        .bind()
        .await;
    let address = acceptor.holdings()[0]
        .local_addr
        .clone()
        .into_std()
        .unwrap();
    let peer = Peer {
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let server = tokio::spawn(
        Server::new(acceptor)
            .serve(Router::with_path("_matrix/federation/v1/publicRooms").post(peer.clone())),
    );
    let remote = format!("127.0.0.1:{}", address.port());
    for network in [
        json!({"include_all_networks":true}),
        json!({"third_party_instance_id":"bridge-network"}),
    ] {
        let mut body = network.clone();
        body["filter"] = json!({"room_types":[null,"m.space"]});
        body["limit"] = json!(2);
        body["since"] = json!("remote-before");
        // The standard query parameter takes precedence over the legacy body.
        body["server"] = json!("must-not-forward.example");
        let response = ok(
            TestClient::post(format!(
                "http://localhost/_matrix/client/v3/publicRooms?server={remote}"
            ))
            .add_header("authorization", format!("Bearer {TOKEN}"), true)
            .json(&body),
            &service,
        )
        .await;
        assert_eq!(
            response["chunk"][0]["room_id"],
            "!remote:directory-peer.example"
        );
        assert_eq!(response["total_room_count_estimate"], 7);
        assert_eq!(response["next_batch"], "remote-next");
        assert_eq!(response["prev_batch"], "remote-prev");
        let request = peer.requests.lock().unwrap().last().unwrap().clone();
        assert_eq!(request.0, "POST");
        assert_eq!(request.1, "/_matrix/federation/v1/publicRooms");
        body.as_object_mut().unwrap().remove("server");
        assert_eq!(request.2, body);
    }
    let response = ok(TestClient::get(format!("http://localhost/_matrix/client/v3/publicRooms?server={remote}&limit=1&since=remote-page")),&service).await;
    assert_eq!(response["next_batch"], "remote-next");
    let request = peer.requests.lock().unwrap().last().unwrap().clone();
    assert_eq!(request.2, json!({"limit":1,"since":"remote-page"}));
    assert_eq!(peer.requests.lock().unwrap().len(), 3);
    server.abort();
}
