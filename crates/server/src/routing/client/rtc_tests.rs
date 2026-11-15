use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::client::rtc::transports::{RtcTransportsResBody, StableRtcTransportsResBody};
use crate::core::{OwnedDeviceId, UnixMillis, UserId};
use crate::{config, data};

fn run(enabled: bool, transports: Value) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(discovery(enabled, transports));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_rtc_discovery_current_legacy_and_custom() {
    run(
        true,
        json!([
            {"type":"m.livekit","url":"wss://sfu.example"},
            {"type":"livekit","livekit_service_url":"https://jwt.example"},
            {"type":"org.example.rtc","settings":{"region":"eu"}}
        ]),
    );
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_rtc_discovery_disabled() {
    run(
        false,
        json!([{"type":"m.livekit","url":"wss://sfu.example"}]),
    );
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_rtc_discovery_empty() {
    run(true, json!([]));
}

async fn discovery(enabled: bool, transports: Value) {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"rtc.example", "db":{"url":"unused"},
                "rtc":{"enabled":enabled,"transports":transports}
            }))
            .unwrap(),
        )
        .unwrap();
    let user = UserId::parse("@alice:rtc.example").unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "alice".into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::device::create_device(&user, &OwnedDeviceId::from("D"), "rtc-token", None, None)
        .await
        .unwrap();
    let service = Service::new(crate::routing::root());
    let mut responses = Vec::new();
    for path in [
        "v1/rtc/transports",
        "unstable/org.matrix.msc4143/rtc/transports",
    ] {
        let url = format!("http://localhost/_matrix/client/{path}");
        for token in [None, Some("invalid")] {
            let request = TestClient::get(&url);
            let request = if let Some(token) = token {
                request.add_header("Authorization", format!("Bearer {token}"), true)
            } else {
                request
            };
            let response = request.send(&service).await;
            assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
        }
        let mut response = TestClient::get(&url)
            .add_header("Authorization", "Bearer rtc-token", true)
            .send(&service)
            .await;
        assert_eq!(
            response.status_code,
            Some(if enabled {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            })
        );
        responses.push(response.take_json::<Value>().await.unwrap());
    }
    if enabled {
        assert_eq!(responses[0], json!({"transports":transports}));
        assert_eq!(responses[1], json!({"rtc_transports":transports}));
        let stable: StableRtcTransportsResBody =
            serde_json::from_value(responses[0].clone()).unwrap();
        let legacy: RtcTransportsResBody = serde_json::from_value(responses[1].clone()).unwrap();
        assert_eq!(stable.transports, legacy.rtc_transports);
    }
    let mut response = TestClient::get("http://localhost/_matrix/client/versions")
        .send(&service)
        .await;
    let versions: Value = response.take_json().await.unwrap();
    for flag in ["org.matrix.msc4143", "org.matrix.msc4143.stable"] {
        assert_eq!(
            versions["unstable_features"][flag]
                .as_bool()
                .unwrap_or(false),
            enabled
        );
    }
    assert_ne!(versions["unstable_features"]["org.matrix.msc4195"], true);
}
