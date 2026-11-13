//! Real HTTP/PostgreSQL contracts for the protocol review fixes.
use std::time::Duration;

use diesel::prelude::*;
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedRoomId, UnixMillis, UserId};
use crate::{config, data};

const SERVER: &str = "protocol-review.example";
const TOKEN: &str = "protocol-review-token";

async fn send(service: &Service, request: RequestBuilder) -> (StatusCode, Value) {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        let status = response.status_code.unwrap();
        (status, response.take_json().await.unwrap())
    })
    .await
    .unwrap()
}

fn authed(request: RequestBuilder) -> RequestBuilder {
    request.add_header("authorization", format!("Bearer {TOKEN}"), true)
}

async fn ok(service: &Service, request: RequestBuilder) -> Value {
    let (status, body) = send(service, authed(request)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn sync(service: &Service, since: Option<&str>, filter: Value, full: bool) -> Value {
    let mut url = url::Url::parse("http://localhost/_matrix/client/v3/sync").unwrap();
    url.query_pairs_mut()
        .append_pair("filter", &filter.to_string())
        .append_pair("full_state", if full { "true" } else { "false" });
    if let Some(since) = since {
        url.query_pairs_mut().append_pair("since", since);
    }
    // Deliberately omit timeout: the contract requires an immediate response.
    tokio::time::timeout(
        Duration::from_secs(3),
        ok(service, TestClient::get(url.as_str())),
    )
    .await
    .expect("sync without timeout must not long poll")
}

fn events<'a>(body: &'a Value, room: &str, section: &str) -> Vec<&'a Value> {
    body["rooms"]["join"][room][section]["events"]
        .as_array()
        .map(|events| events.iter().collect())
        .unwrap_or_default()
}

#[test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
fn review_protocol_openid_full_state_filters_and_leave_retry() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(contract());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn contract() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name": SERVER, "db": {"url": "unused"}, "auto_join_rooms": [],
                "federation": {"enable": true}, "ip_range_denylist": [],
                "presence": {"allow_local": false, "allow_incoming": false, "allow_outgoing": false}
            }))
            .unwrap(),
        )
        .unwrap();
    let user = UserId::parse(format!("@reader:{SERVER}")).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "reader".into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&user).await.unwrap();
    data::user::device::create_device(&user, &OwnedDeviceId::from("D"), TOKEN, None, None)
        .await
        .unwrap();
    let service = Service::new(crate::routing::root());

    // An OpenID token is a distinct credential, never a Matrix session token.
    let token = ok(
        &service,
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/user/{user}/openid/request_token"
        ))
        .json(&json!({})),
    )
    .await;
    let openid = token["access_token"].as_str().unwrap();
    for version in ["v1", "v2"] {
        let (status, info) = send(&service, TestClient::get(format!(
            "http://localhost/_matrix/federation/{version}/openid/userinfo?access_token={openid}"
        ))).await;
        assert_eq!(status, StatusCode::OK, "{info}");
        assert_eq!(info["sub"], user.as_str());
    }
    for invalid in ["invalid", TOKEN] {
        let (status, body) = send(
            &service,
            TestClient::get(format!(
                "http://localhost/_matrix/federation/v1/openid/userinfo?access_token={invalid}"
            )),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["errcode"], "M_UNKNOWN_TOKEN");
    }
    diesel::update(data::schema::user_openid_tokens::table)
        .set(data::schema::user_openid_tokens::expires_at.eq(0_i64))
        .execute(&mut data::db())
        .await
        .unwrap();
    let (status, body) = send(
        &service,
        TestClient::get(format!(
            "http://localhost/_matrix/federation/v1/openid/userinfo?access_token={openid}"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["errcode"], "M_UNKNOWN_TOKEN");

    let created = ok(
        &service,
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .json(&json!({"room_version":"11", "preset":"private_chat", "name":"Review"})),
    )
    .await;
    let room = created["room_id"].as_str().unwrap();
    let room_id: OwnedRoomId = room.try_into().unwrap();
    let initial = sync(&service, None, json!({}), false).await;
    let initial_state = events(&initial, room, "state");
    assert!(!initial_state.is_empty());
    let since = initial["next_batch"].as_str().unwrap();
    // The idle-token early return and unchanged-frame paths must both respect full_state.
    let full = sync(&service, Some(since), json!({}), true).await;
    assert!(
        events(&full, room, "state")
            .iter()
            .any(|event| event["type"] == "m.room.name")
    );
    let _idle = sync(&service, Some(since), json!({}), false).await;
    let message = ok(
        &service,
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/review-message"
        ))
        .json(&json!({"msgtype":"m.text", "body":"hello"})),
    )
    .await;
    let message_id = message["event_id"].as_str().unwrap();
    let full = sync(&service, Some(since), json!({}), true).await;
    assert!(
        events(&full, room, "state")
            .iter()
            .any(|event| event["type"] == "m.room.name")
    );
    assert!(
        events(&full, room, "timeline")
            .iter()
            .any(|event| event["event_id"] == message_id)
    );

    ok(&service, TestClient::put(format!(
        "http://localhost/_matrix/client/v3/user/{user}/rooms/{room}/account_data/org.review_tag"
    )).json(&json!({"label":"secret"}))).await;
    crate::room::typing::add_typing(&user, &room_id, UnixMillis::now().get() + 60_000, false)
        .await
        .unwrap();
    // Positive controls prove the fixture generated data before exclusion checks.
    let positive = sync(&service, Some(since), json!({}), false).await;
    assert!(
        events(&positive, room, "account_data")
            .iter()
            .any(|event| event["type"] == "org.review_tag")
    );
    assert!(
        events(&positive, room, "ephemeral")
            .iter()
            .any(|event| event["type"] == "m.typing")
    );
    let filtered = sync(
        &service,
        Some(since),
        json!({"room": {
            "timeline":{"types":["m.room.*"]}, "state":{"types":["m.room.*"]},
            "account_data":{"not_types":["org.*"]}, "ephemeral":{"types":[]}
        }}),
        true,
    )
    .await;
    assert!(
        events(&filtered, room, "timeline")
            .iter()
            .any(|event| event["event_id"] == message_id)
    );
    assert!(!events(&filtered, room, "state").is_empty());
    assert!(events(&filtered, room, "account_data").is_empty());
    assert!(events(&filtered, room, "ephemeral").is_empty());
    let empty = sync(
        &service,
        Some(since),
        json!({"room": {
            "timeline":{"types":[]}, "state":{"types":[]}, "ephemeral":{"not_types":["m.*"]}
        }}),
        true,
    )
    .await;
    assert!(events(&empty, room, "timeline").is_empty());
    assert!(events(&empty, room, "state").is_empty());
    assert!(events(&empty, room, "ephemeral").is_empty());
    // SQL filtering must happen before pagination and agree with in-memory state filtering.
    for (filter, expected) in [
        (json!({"types":["m.room.*"]}), true),
        (json!({"types":["m.room.mess*age"]}), true),
        (json!({"types":["*"]}), true),
        (json!({"types":["m.room.%"]}), false),
        (json!({"types":[r"m.room.mess\age"]}), false),
        (json!({"types":[]}), false),
        (json!({"not_types":["m.room.*"]}), false),
        (json!({"types":["m.room.mess_ge"]}), false),
        (json!({"senders":[]}), false),
        (json!({"rooms":[]}), false),
    ] {
        let mut url = url::Url::parse(&format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/messages"
        ))
        .unwrap();
        url.query_pairs_mut()
            .append_pair("dir", "b")
            .append_pair("filter", &filter.to_string());
        let history = ok(&service, TestClient::get(url.as_str())).await;
        let found = history["chunk"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event_id"] == message_id);
        assert_eq!(found, expected, "filter={filter}, response={history}");
    }

    let last = sync(&service, None, json!({}), false).await;
    let leave_since = last["next_batch"].as_str().unwrap();
    ok(
        &service,
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/leave"
        ))
        .json(&json!({})),
    )
    .await;
    // A real SQL failure must return an error instead of a new acknowledgement token.
    diesel::sql_query("ALTER TABLE rooms RENAME COLUMN state_frame_id TO review_unavailable_frame")
        .execute(&mut data::db())
        .await
        .unwrap();
    let url = format!("http://localhost/_matrix/client/v3/sync?since={leave_since}&timeout=0");
    let failure = send(&service, authed(TestClient::get(&url))).await;
    diesel::sql_query("ALTER TABLE rooms RENAME COLUMN review_unavailable_frame TO state_frame_id")
        .execute(&mut data::db())
        .await
        .unwrap();
    assert_eq!(
        failure.0,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        failure.1
    );
    assert!(failure.1.get("next_batch").is_none());
    let retry = sync(&service, Some(leave_since), json!({}), false).await;
    assert!(retry["rooms"]["leave"].get(room).is_some(), "{retry}");
}
