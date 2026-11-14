//! Isolated HTTP/PostgreSQL coverage for delayed sticky request locations.
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedRoomId, UnixMillis, UserId};
use crate::{config, data};

async fn send(service: &Service, request: RequestBuilder) -> (StatusCode, Value) {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = request
            .add_header("Authorization", "Bearer delayed-body", true)
            .send(router)
            .await;
        let status = response.status_code.unwrap();
        let body = response.take_json().await.unwrap_or(Value::Null);
        (status, body)
    })
    .await
    .unwrap()
}
async fn ok(service: &Service, request: RequestBuilder) -> Value {
    let (status, body) = send(service, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_delayed_sticky_body_reaches_storage_and_delivery() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(delayed_sticky_body());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn delayed_sticky_body() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"delayed-body.example", "db":{"url":"unused"},
                "auto_join_rooms":[], "delayed_events":{"enable":true},
                "rc_message":{"per_second":0.0,"burst":0}
            }))
            .unwrap(),
        )
        .unwrap();
    let user = UserId::parse("@owner:delayed-body.example").unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "owner".into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&user).await.unwrap();
    data::user::device::create_device(&user, &OwnedDeviceId::from("D"), "delayed-body", None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&user).await.unwrap();
    let service = Service::new(crate::routing::root());
    let created = ok(
        &service,
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .json(&json!({"room_version":"11", "preset":"public_chat"})),
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    let base = "http://localhost/_matrix/client/unstable/org.matrix.msc4140";
    let mut delivered = Vec::new();
    for (txn, state, body_sticky, query_sticky) in [
        ("body-message", false, true, false),
        ("body-state", true, true, false),
        ("legacy-query", false, false, true),
        ("matching-both", false, true, true),
        ("ordinary", false, false, false),
    ] {
        let ty = if state {
            "m.room.topic"
        } else {
            "m.room.message"
        };
        let mut body = json!({"delay_ms":60000, "content":if state {
            json!({"topic":"scheduled"})
        } else { json!({"msgtype":"m.text","body":txn}) }});
        if state {
            body["state_key"] = json!("");
        }
        if body_sticky {
            body["org.matrix.msc4354.sticky_duration_ms"] = json!(60000);
        }
        let query = if query_sticky {
            "?org.matrix.msc4354.sticky_duration_ms=60000"
        } else {
            ""
        };
        let url = format!("{base}/rooms/{room}/delayed_event/{ty}/{txn}{query}");
        let scheduled = ok(&service, TestClient::put(&url).json(&body)).await;
        let delay_id = scheduled["delay_id"].as_str().unwrap();
        assert_eq!(
            ok(&service, TestClient::put(&url).json(&body)).await,
            scheduled
        );
        let row = data::room::delayed_event::get_by_delay_id(&user, delay_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.sticky_duration_ms,
            (body_sticky || query_sticky).then_some(60000)
        );
        if txn.starts_with("body-") {
            ok(
                &service,
                TestClient::post(format!("{base}/delayed_events/{delay_id}/restart")),
            )
            .await;
            let restored = data::room::delayed_event::get_by_delay_id(&user, delay_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(restored.sticky_duration_ms, Some(60000));
            ok(
                &service,
                TestClient::post(format!("{base}/delayed_events/{delay_id}/send")),
            )
            .await;
            let row = data::room::delayed_event::get_by_delay_id(&user, delay_id)
                .await
                .unwrap()
                .unwrap();
            let event_id = row.event_id.unwrap();
            let pdu = crate::room::timeline::get_pdu(&event_id).await.unwrap();
            assert_eq!(pdu.sticky_object().unwrap()["duration_ms"], 60000);
            delivered.push(event_id);
        } else {
            ok(
                &service,
                TestClient::post(format!("{base}/delayed_events/{delay_id}/cancel")),
            )
            .await;
        }
    }
    let sync_url = "http://localhost/_matrix/client/v3/sync?timeout=0";
    let first = ok(&service, TestClient::get(sync_url)).await;
    let ttl = |sync: &Value, id: &str| {
        sync["rooms"]["join"][room.as_str()]["timeline"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["event_id"] == id)
            .unwrap()["unsigned"]["msc4354_sticky_duration_ttl_ms"]
            .as_u64()
            .unwrap()
    };
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let second = ok(&service, TestClient::get(sync_url)).await;
    for id in delivered {
        let initial = ttl(&first, id.as_str());
        assert!(initial > 0 && initial <= 60000);
        assert!(ttl(&second, id.as_str()) < initial);
    }
    for (index, invalid) in [json!(-1), json!(3600001), json!(1.5), json!("60000")]
        .into_iter()
        .enumerate()
    {
        let (status, _) = send(
            &service,
            TestClient::put(format!(
                "{base}/rooms/{room}/delayed_event/m.room.message/invalid-{index}"
            ))
            .json(&json!({"delay_ms":60000, "content":{},
            "org.matrix.msc4354.sticky_duration_ms":invalid})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, body) = send(&service, TestClient::put(format!(
        "{base}/rooms/{room}/delayed_event/m.room.message/conflict?org.matrix.msc4354.sticky_duration_ms=30000"
    )).json(&json!({"delay_ms":60000,"content":{},
        "org.matrix.msc4354.sticky_duration_ms":60000}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["errcode"], "M_INVALID_PARAM");
}
