use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

const EXT: &str = "org.matrix.msc4354.sticky_events";
async fn send(service: &Service, request: RequestBuilder, token: &str) -> (StatusCode, Value) {
    let router = service.router();
    let token = token.to_owned();
    tokio::spawn(async move {
        let mut response = request
            .add_header("Authorization", format!("Bearer {token}"), true)
            .send(router)
            .await;
        (
            response.status_code.unwrap(),
            response.take_json().await.unwrap_or(Value::Null),
        )
    })
    .await
    .unwrap()
}
async fn ok(service: &Service, request: RequestBuilder, token: &str) -> Value {
    let (status, body) = send(service, request, token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn sync(service: &Service, body: Value, pos: Option<&str>) -> Value {
    let url = format!(
        "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync?timeout=0{}",
        pos.map(|p| format!("&pos={p}")).unwrap_or_default()
    );
    ok(service, TestClient::post(url).json(&body), "alice").await
}
fn ids(value: &Value, room: &RoomId) -> Vec<String> {
    value["extensions"][EXT]["rooms"][room.as_str()]["events"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .map(|e| e["event_id"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}
fn config(room: &RoomId, conn: &str, limit: u64) -> Value {
    json!({"conn_id":conn,"room_subscriptions":{room.as_str():{"timeline_limit":0,"required_state":[["m.room.member","$LAZY"]]}},
        "extensions":{EXT:{"enabled":true,"limit":limit,"lists":[],"rooms":[room]}}})
}
async fn append(service: &Service, room: &RoomId, txn: &str) -> String {
    ok(service,TestClient::put(format!("http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/{txn}?org.matrix.msc4354.sticky_duration_ms=60000"))
        .json(&json!({"msgtype":"m.text","body":txn})),"bob").await["event_id"].as_str().unwrap().to_owned()
}
#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_sliding_sticky_delivery_ack_scope_and_restart() {
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
    crate::config::CONFIG
        .set(
            serde_json::from_value(json!({"server_name":"sticky.example","db":{"url":"unused"},
                "auto_join_rooms":[],"rc_message":{"per_second":0.0,"burst":0},
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
            }))
            .unwrap(),
        )
        .unwrap();
    for name in ["alice", "bob"] {
        let user = UserId::parse(format!("@{name}:sticky.example")).unwrap();
        data::user::create_user(&data::user::NewDbUser {
            id: user.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: name.into(),
            server_name: user.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        data::user::ensure_profile_exists(&user).await.unwrap();
        data::user::device::create_device(&user, &OwnedDeviceId::from("D"), name, None, None)
            .await
            .unwrap();
    }
    let service = Service::new(crate::routing::root());
    let created = ok(
        &service,
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .json(&json!({"room_version":"11","preset":"public_chat"})),
        "alice",
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    ok(
        &service,
        TestClient::post(format!("http://localhost/_matrix/client/v3/join/{room}"))
            .json(&json!({})),
        "bob",
    )
    .await;
    let secret = ok(
        &service,
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .json(&json!({"room_version":"11","preset":"private_chat"})),
        "bob",
    )
    .await;
    let secret: OwnedRoomId = secret["room_id"].as_str().unwrap().try_into().unwrap();
    let forbidden = append(&service, &secret, "private").await;
    let first = append(&service, &room, "first").await;
    let second = append(&service, &room, "second").await;
    let third = append(&service, &room, "third").await;
    let mut empty = config(&room, "paging", 1);
    empty["extensions"][EXT]["rooms"] = json!([]);
    let base = sync(&service, empty, None).await;
    assert!(ids(&base, &room).is_empty());
    let input = base["pos"].as_str().unwrap();
    let page = sync(&service, config(&room, "paging", 1), Some(input)).await;
    assert_eq!(ids(&page, &room), vec![first.clone()]);
    let members = page["rooms"][room.as_str()]["required_state"]
        .as_array()
        .unwrap();
    assert!(
        members
            .iter()
            .any(|e| e["type"] == "m.room.member" && e["state_key"] == "@bob:sticky.example")
    );
    let ttl = page["extensions"][EXT]["rooms"][room.as_str()]["events"][0]["unsigned"]
        [STICKY_TTL_KEY]
        .as_u64()
        .unwrap();
    assert!(ttl > 0 && ttl <= 60000);
    let fourth = append(&service, &room, "fourth").await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    // Drop memory only: the durable connection row must reproduce the offered page.
    let key = connection_key(
        UserId::parse("@alice:sticky.example").unwrap().as_ref(),
        "D".into(),
        &Some("paging".into()),
    );
    CONNECTIONS.lock().unwrap().remove(&key);
    let replay = sync(&service, config(&room, "paging", 1), Some(input)).await;
    assert_eq!(ids(&replay, &room), vec![first.clone()]);
    assert!(
        replay["extensions"][EXT]["rooms"][room.as_str()]["events"][0]["unsigned"][STICKY_TTL_KEY]
            .as_u64()
            .unwrap()
            < ttl
    );
    let page2 = sync(&service, config(&room, "paging", 1), page["pos"].as_str()).await;
    assert_eq!(ids(&page2, &room), vec![second.clone()]);

    // Replay and queued data are rechecked for redaction and expiry.
    ok(
        &service,
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/redact/{second}/redact"
        ))
        .json(&json!({})),
        "alice",
    )
    .await;
    use diesel::sql_types::{BigInt, Text};
    diesel::sql_query("UPDATE event_stickies SET expires_at=$2 WHERE event_id=$1")
        .bind::<Text, _>(&third)
        .bind::<BigInt, _>(UnixMillis::now().0 as i64 - 1)
        .execute(&mut data::db())
        .await
        .unwrap();
    let replay2 = sync(&service, config(&room, "paging", 1), page["pos"].as_str()).await;
    assert!(ids(&replay2, &room).is_empty());
    let page3 = sync(&service, config(&room, "paging", 1), page2["pos"].as_str()).await;
    assert_eq!(ids(&page3, &room), vec![fourth.clone()]);
    let drained = sync(&service, config(&room, "paging", 1), page3["pos"].as_str()).await;
    assert!(ids(&drained, &room).is_empty());

    // Disabling (including omission) and re-enabling creates a new sticky snapshot.
    let disabled = sync(
        &service,
        json!({"conn_id":"paging"}),
        drained["pos"].as_str(),
    )
    .await;
    assert!(ids(&disabled, &room).is_empty());
    let again = sync(
        &service,
        config(&room, "paging", 1),
        disabled["pos"].as_str(),
    )
    .await;
    assert_eq!(ids(&again, &room), vec![first.clone()]);

    // Timeline duplicates have TTLs and are not repeated in the extension.
    let mut timeline = config(&room, "timeline", 100);
    timeline["room_subscriptions"][room.as_str()]["timeline_limit"] = json!(100);
    let timeline = sync(&service, timeline, None).await;
    let events = timeline["rooms"][room.as_str()]["timeline"]
        .as_array()
        .unwrap();
    for id in [&first, &fourth] {
        let event = events
            .iter()
            .find(|e| e["event_id"] == id.as_str())
            .unwrap();
        assert!(event["unsigned"][STICKY_TTL_KEY].as_u64().unwrap() > 0);
    }
    assert!(ids(&timeline, &room).is_empty());

    // Explicit selectors can target rooms outside the top-level room subset,
    // but cannot expose a non-joined private room.
    let selected=sync(&service,json!({"conn_id":"explicit","extensions":{EXT:{"enabled":true,"rooms":[room,secret],"lists":[]}}}),None).await;
    assert_eq!(ids(&selected, &room), vec![first.clone(), fourth.clone()]);
    assert!(ids(&selected, &secret).is_empty());
    assert!(!selected.to_string().contains(&forbidden));
    assert!(selected["extensions"][EXT].get("next_batch").is_none());
    let wildcard=sync(&service,json!({"conn_id":"wildcard","room_subscriptions":{room.as_str():{"timeline_limit":0}},"extensions":{EXT:{"enabled":true,"rooms":["*"],"lists":[]}}}),None).await;
    assert_eq!(ids(&wildcard, &room), vec![first.clone(), fourth.clone()]);
    // List selectors resolve against this response's actual window.
    let listed=sync(&service,json!({"conn_id":"lists","lists":{"all":{"ranges":[[0,0]],"timeline_limit":0}},"extensions":{EXT:{"enabled":true,"lists":["all"],"rooms":[]}}}),None).await;
    assert_eq!(ids(&listed, &room), vec![first.clone(), fourth.clone()]);
    let excluded=sync(&service,json!({"conn_id":"no-list","lists":{"all":{"ranges":[[0,0]],"timeline_limit":0}},"extensions":{EXT:{"enabled":true,"lists":[],"rooms":[]}}}),None).await;
    assert!(ids(&excluded, &room).is_empty());

    // Leaving revokes queued delivery; joining again resets even an acknowledged queue.
    ok(
        &service,
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/m.room.history_visibility"
        ))
        .json(&json!({"history_visibility":"joined"})),
        "alice",
    )
    .await;
    ok(
        &service,
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/leave"
        ))
        .json(&json!({})),
        "alice",
    )
    .await;
    let left = sync(&service, config(&room, "paging", 1), again["pos"].as_str()).await;
    assert!(ids(&left, &room).is_empty());
    ok(
        &service,
        TestClient::post(format!("http://localhost/_matrix/client/v3/join/{room}"))
            .json(&json!({})),
        "alice",
    )
    .await;
    let rejoined = sync(&service, config(&room, "paging", 1), left["pos"].as_str()).await;
    assert_eq!(
        ids(&rejoined, &room),
        vec![first.clone()],
        "joined users must receive still-sticky pre-join history"
    );

    // Ignore rules apply independently of membership and sticky privileges.
    ok(&service,TestClient::put("http://localhost/_matrix/client/v3/user/@alice:sticky.example/account_data/m.ignored_user_list")
        .json(&json!({"ignored_users":{"@bob:sticky.example":{}}})),"alice").await;
    let ignored = sync(&service, config(&room, "ignored", 100), None).await;
    assert!(ids(&ignored, &room).is_empty());
    let (status, _) = send(
        &service,
        TestClient::post(
            "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync?timeout=0",
        )
        .json(&config(&room, "bad-limit", 0)),
        "alice",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    ok(&service,TestClient::put("http://localhost/_matrix/client/v3/user/@alice:sticky.example/account_data/m.ignored_user_list")
        .json(&json!({"ignored_users":{}})),"alice").await;
    let start = sync(&service, config(&room, "poll", 100), None).await;
    let poll_body = config(&room, "poll", 100);
    let poll_url = format!(
        "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync?timeout=1500&pos={}",
        start["pos"].as_str().unwrap()
    );
    let router = service.router();
    let began = std::time::Instant::now();
    let poll = tokio::spawn(async move {
        let mut response = TestClient::post(poll_url)
            .add_header("Authorization", "Bearer alice", true)
            .json(&poll_body)
            .send(router)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        response.take_json::<Value>().await.unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let wake = append(&service, &room, "wake-poll").await;
    let woken = poll.await.unwrap();
    assert!(
        began.elapsed() < std::time::Duration::from_millis(1400),
        "sticky-only update did not wake long poll"
    );
    assert!(ids(&woken, &room).contains(&wake));

    // Delivery state is part of response correctness: no successful cursor on failure.
    diesel::sql_query(
        "ALTER TABLE sliding_sync_connections RENAME TO review_unavailable_sticky_connections",
    )
    .execute(&mut data::db())
    .await
    .unwrap();
    let (failed, _) = send(
        &service,
        TestClient::post(
            "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync?timeout=0",
        )
        .json(&config(&room, "storage-failure", 100)),
        "alice",
    )
    .await;
    diesel::sql_query(
        "ALTER TABLE review_unavailable_sticky_connections RENAME TO sliding_sync_connections",
    )
    .execute(&mut data::db())
    .await
    .unwrap();
    assert_eq!(failed, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        !ids(
            &sync(&service, config(&room, "storage-failure", 100), None).await,
            &room
        )
        .is_empty()
    );
}
