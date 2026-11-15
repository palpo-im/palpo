//! Signed HTTP regressions for room-bound federation reads and auth-chain caches.
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
use crate::core::serde::CanonicalJsonObject;
use crate::core::signatures::{Ed25519KeyPair, sign_json};
use crate::core::{EventId, OwnedEventId, OwnedRoomId, ServerName, UnixMillis, UserId};
use crate::{config, data};

const LOCAL: &str = "federation-room-access.example";
const REMOTE: &str = "federation-room-peer.example";
const TOKEN: &str = "federation-room-owner-token";
const SECRET_TYPE: &str = "org.palpo.private-state";

async fn send(request: RequestBuilder, service: &Service) -> (StatusCode, Value) {
    let router = service.router();
    let mut response = tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap();
    let status = response.status_code.unwrap();
    let body = response.take_json::<Value>().await.unwrap();
    (status, body)
}

async fn ok(request: RequestBuilder, service: &Service) -> Value {
    let (status, body) = send(request, service).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn signed_get(
    service: &Service,
    key: &Ed25519KeyPair,
    origin: &str,
    path: &str,
) -> (StatusCode, Value) {
    let mut signed: CanonicalJsonObject = serde_json::from_value(json!({
        "method":"GET", "uri":path, "origin":origin, "destination":LOCAL
    }))
    .unwrap();
    sign_json(origin, key, &mut signed).unwrap();
    let signed = serde_json::to_value(signed).unwrap();
    let key_id = format!("ed25519:{}", key.version());
    let signature = signed["signatures"][origin][&key_id].as_str().unwrap();
    send(
        TestClient::get(format!("http://localhost{path}")).add_header(
            "Authorization",
            format!(
                "X-Matrix origin=\"{origin}\",destination=\"{LOCAL}\",key=\"{key_id}\",sig=\"{signature}\""
            ),
            true,
        ),
        service,
    )
    .await
}

async fn create_room(
    service: &Service,
    version: &str,
    history_visibility: &str,
) -> (OwnedRoomId, OwnedEventId) {
    let created = ok(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", format!("Bearer {TOKEN}"), true)
            .json(&json!({
                "room_version":version, "preset":"private_chat",
                "initial_state":[
                    {"type":"m.room.history_visibility", "state_key":"",
                     "content":{"history_visibility":history_visibility}},
                    {"type":SECRET_TYPE, "state_key":"",
                     "content":{"secret":format!("private-v{version}")}}
                ]
            })),
        service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    let sent = ok(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/probe-{version}-{history_visibility}"
        ))
        .add_header("Authorization", format!("Bearer {TOKEN}"), true)
        .json(&json!({"msgtype":"m.text", "body":"visibility probe"})),
        service,
    )
    .await;
    let event = sent["event_id"].as_str().unwrap().try_into().unwrap();
    (room, event)
}

fn read_paths(version: &str, room: &OwnedRoomId, event: &EventId) -> [String; 2] {
    [
        format!("/_matrix/federation/{version}/event_auth/{room}/{event}"),
        format!("/_matrix/federation/{version}/state/{room}?event_id={event}"),
    ]
}

async fn deny_reads(service: &Service, key: &Ed25519KeyPair, room: &OwnedRoomId, event: &EventId) {
    for version in ["v1", "v2"] {
        for path in read_paths(version, room, event) {
            let (status, body) = signed_get(service, key, REMOTE, &path).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
            assert_eq!(body["errcode"], "M_FORBIDDEN", "{path}: {body}");
            assert!(body.get("auth_chain").is_none(), "{path}: {body}");
            assert!(body.get("pdus").is_none(), "{path}: {body}");
        }
    }
}

async fn allow_reads(
    service: &Service,
    key: &Ed25519KeyPair,
    origin: &str,
    room: &OwnedRoomId,
    event: &EventId,
) {
    for version in ["v1", "v2"] {
        let [auth_path, state_path] = read_paths(version, room, event);
        let (status, body) = signed_get(service, key, origin, &auth_path).await;
        assert_eq!(status, StatusCode::OK, "{auth_path}: {body}");
        assert!(!body["auth_chain"].as_array().unwrap().is_empty());
        let (status, body) = signed_get(service, key, origin, &state_path).await;
        assert_eq!(status, StatusCode::OK, "{state_path}: {body}");
        assert!(
            body["pdus"]
                .as_array()
                .unwrap()
                .iter()
                .any(|pdu| pdu["type"] == SECRET_TYPE && pdu["content"]["secret"].is_string()),
            "{state_path}: {body}"
        );
    }
}

async fn clear_auth_caches() {
    super::AUTH_CHAIN_CACHE.lock().unwrap().clear();
    diesel::delete(data::schema::event_auth_chains::table)
        .execute(&mut data::db())
        .await
        .unwrap();
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn review_federation_event_reads_bind_rooms_before_auth_cache() {
    std::thread::Builder::new()
        .name("federation-room-access-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(federation_event_reads())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn federation_event_reads() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":LOCAL, "db":{"url":"unused-test-config"},
                "auto_join_rooms":[], "rc_message":{"per_second":10000.0,"burst":10000}
            }))
            .unwrap(),
        )
        .expect("run this HTTP regression alone");
    let owner = UserId::parse(format!("@owner:{LOCAL}")).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: owner.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: owner.localpart().into(),
        server_name: owner.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&owner).await.unwrap();
    data::user::device::create_device(&owner, "OWNER".into(), TOKEN, None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&owner).await.unwrap();
    let remote = ServerName::parse(REMOTE).unwrap();
    let remote_key =
        Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), "access".into()).unwrap();
    let mut keys =
        ServerSigningKeys::new(remote.clone(), UnixMillis(UnixMillis::now().0 + 86_400_000));
    keys.verify_keys.insert(
        "ed25519:access".try_into().unwrap(),
        VerifyKey::from_bytes(remote_key.public_key().to_vec()),
    );
    crate::server_key::seed_verified_keys(keys, &remote_key).await;
    let service = Service::new(crate::routing::root());

    // Exercise both the older wire format and v12's room-ID-less create event.
    for version in ["11", "12"] {
        let (allowed_room, allowed_event) = create_room(&service, version, "world_readable").await;
        let (private_room, private_event) = create_room(&service, version, "joined").await;
        // The real local creator is in both rooms. The remote peer is represented
        // only in A; world-readable event history makes A a valid positive control.
        data::room::add_joined_server(&allowed_room, &remote)
            .await
            .unwrap();
        allow_reads(&service, &remote_key, REMOTE, &allowed_room, &allowed_event).await;
        if version == "12" {
            let create = crate::room::get_create(&private_room).await.unwrap();
            assert!(
                crate::room::timeline::get_pdu_json(&create.event_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .get("room_id")
                    .is_none()
            );
        }

        clear_auth_caches().await;
        // Cold cache: neither event_auth nor state may use an accessible path
        // room to authorize an event or snapshot from a different room.
        deny_reads(&service, &remote_key, &allowed_room, &private_event).await;
        assert!(
            super::get_auth_chain_ids(&allowed_room, [&*private_event].into_iter())
                .await
                .is_err()
        );
        deny_reads(&service, &remote_key, &private_room, &private_event).await;

        // A legitimate same-room read warms both the RAM and persistent caches.
        allow_reads(
            &service,
            config::keypair(),
            LOCAL,
            &private_room,
            &private_event,
        )
        .await;
        for persistent_only in [false, true] {
            if persistent_only {
                super::AUTH_CHAIN_CACHE.lock().unwrap().clear();
            }
            assert!(
                super::get_auth_chain_ids(&allowed_room, [&*private_event].into_iter())
                    .await
                    .is_err(),
                "wrong-room chain was accepted with persistent_only={persistent_only}"
            );
            deny_reads(&service, &remote_key, &allowed_room, &private_event).await;
            deny_reads(&service, &remote_key, &private_room, &private_event).await;
        }
        allow_reads(
            &service,
            config::keypair(),
            LOCAL,
            &private_room,
            &private_event,
        )
        .await;

        // A current member must be able to recover the room's state and auth
        // chain even for joined-only history predating its membership. This
        // recovery metadata must not unlock the historical message itself.
        data::room::add_joined_server(&private_room, &remote)
            .await
            .unwrap();
        allow_reads(&service, &remote_key, REMOTE, &private_room, &private_event).await;
        for api_version in ["v1", "v2"] {
            let path = format!("/_matrix/federation/{api_version}/event/{private_event}");
            let (status, body) = signed_get(&service, &remote_key, REMOTE, &path).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
            assert_eq!(body["errcode"], "M_FORBIDDEN", "{path}: {body}");
        }
    }
}
