//! Signed federation HTTP requests over a real PostgreSQL graph fixture.
use diesel::prelude::*;
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
use crate::core::serde::{CanonicalJsonObject, to_canonical_object};
use crate::core::signatures::{Ed25519KeyPair, sign_json};
use crate::core::{OwnedRoomId, UnixMillis, UserId};
use crate::data::room::{DbEventData, NewDbEvent};
use crate::data::schema::*;
use crate::{config, data};

const LOCAL: &str = "missing-review.example";
const REMOTE: &str = "peer.missing.example";

async fn signed(
    service: &Service,
    key: &Ed25519KeyPair,
    room: &OwnedRoomId,
    body: Value,
) -> (StatusCode, Value) {
    let path = format!("/_matrix/federation/v1/get_missing_events/{room}");
    let mut authorization: CanonicalJsonObject = serde_json::from_value(json!({
        "method":"POST","uri":path,"origin":REMOTE,"destination":LOCAL,"content":body
    }))
    .unwrap();
    sign_json(REMOTE, key, &mut authorization).unwrap();
    let authorization = serde_json::to_value(authorization).unwrap();
    let signature = authorization["signatures"][REMOTE]["ed25519:review"]
        .as_str()
        .unwrap();
    let router = service.router();
    let request = TestClient::post(format!("http://localhost{path}")).json(&body)
        .add_header("authorization",format!("X-Matrix origin=\"{REMOTE}\",destination=\"{LOCAL}\",key=\"ed25519:review\",sig=\"{signature}\""),true);
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        (
            response.status_code.unwrap(),
            response.take_json().await.unwrap(),
        )
    })
    .await
    .unwrap()
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_missing_events_deduplicates_dag_and_honors_depth() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run());
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
                "server_name":LOCAL,"db":{"url":"unused"},"auto_join_rooms":[],
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
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
    data::user::device::create_device(&owner, "D".into(), "owner", None, None)
        .await
        .unwrap();
    let service = Service::new(crate::routing::matrix());
    let router = service.router();
    let created = tokio::spawn(async move {
        let mut response = TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("authorization","Bearer owner",true).json(&json!({
                "room_version":"11","preset":"public_chat","initial_state":[{
                    "type":"m.room.history_visibility","state_key":"","content":{"history_visibility":"world_readable"}
                }]
            })).send(router).await;
        let body: Value = response.take_json().await.unwrap();
        assert_eq!(response.status_code,Some(StatusCode::OK),"{body}"); body
    }).await.unwrap();
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    let frame = crate::room::get_frame_id(&room, None).await.unwrap();
    let peer = crate::core::ServerName::parse(REMOTE).unwrap();
    assert!(data::room::add_joined_server(&room, &peer).await.unwrap());
    // Accepted storage graph with a shared ancestor; the endpoint's traversal,
    // access checks and serialization are real. Event admission is tested elsewhere.
    for (id, depth, parents) in [
        ("$missing-D", 100, vec![]),
        ("$missing-C", 101, vec!["$missing-D"]),
        ("$missing-P", 102, vec!["$missing-C"]),
        ("$missing-Q", 102, vec!["$missing-C"]),
        ("$missing-L", 103, vec!["$missing-P", "$missing-Q"]),
    ] {
        let sn = data::next_sn().await.unwrap();
        let pdu: crate::event::PduEvent = serde_json::from_value(json!({
            "event_id":id,"room_id":room,"sender":owner,"type":"m.room.message",
            "content":{"msgtype":"m.text","body":id},"origin_server_ts":1,
            "depth":depth,"prev_events":parents,"hashes":{"sha256":""}
        }))
        .unwrap();
        let json = to_canonical_object(&pdu).unwrap();
        let event =
            NewDbEvent::from_canonical_json_with_room_id(&pdu.event_id, sn, &json, false, &room)
                .unwrap();
        let mut conn = data::connect();
        event.save_with_conn(&mut conn).await.unwrap();
        DbEventData {
            event_id: pdu.event_id.clone(),
            event_sn: sn,
            room_id: room.clone(),
            json_data: serde_json::to_value(json).unwrap(),
            internal_metadata: None,
            format_version: None,
        }
        .save_with_conn(&mut conn)
        .await
        .unwrap();
        diesel::insert_into(event_points::table)
            .values((
                event_points::event_id.eq(&pdu.event_id),
                event_points::event_sn.eq(sn),
                event_points::room_id.eq(&room),
                event_points::frame_id.eq(Some(frame)),
                event_points::before_frame_id.eq(Some(frame)),
            ))
            .execute(&mut conn)
            .await
            .unwrap();
    }
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
    for (min_depth, earliest, expected) in [
        (
            0,
            vec![],
            vec!["$missing-C", "$missing-D", "$missing-P", "$missing-Q"],
        ),
        (102, vec![], vec!["$missing-P", "$missing-Q"]),
        (103, vec![], vec![]),
        (0, vec!["$missing-C"], vec!["$missing-P", "$missing-Q"]),
    ] {
        let (status,body) = signed(&service,&key,&room,json!({
            "latest_events":["$missing-L","$missing-L"],"earliest_events":earliest,"limit":4,"min_depth":min_depth
        })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let mut ids: Vec<_> = body["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                assert!(event["depth"].as_u64().unwrap() >= min_depth);
                // Room v11 federation PDUs derive their event ID and omit it on wire.
                event["content"]["body"].as_str().unwrap().to_owned()
            })
            .collect();
        ids.sort();
        assert_eq!(ids, expected, "{body}");
    }
    let (status, body) = signed(
        &service,
        &key,
        &room,
        json!({
            "latest_events":["$missing-L"],"earliest_events":[],"limit":2
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["events"].as_array().unwrap().len(), 2);
}
