use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedRoomId, OwnedUserId, UnixMillis, UserId};
use crate::event::BatchToken;
use crate::{config, data, room};

async fn send(request: RequestBuilder, service: &Service) -> Response {
    let router = service.router();
    tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap()
}

async fn ok(request: RequestBuilder, service: &Service) -> Value {
    let mut response = send(request, service).await;
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

async fn provision(localpart: &str) -> (OwnedUserId, String) {
    let user = UserId::parse(format!("@{localpart}:{}", config::server_name())).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: localpart.into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    let token = format!("{localpart}-token");
    data::user::device::create_device(&user, &OwnedDeviceId::from("D"), &token, None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&user).await.unwrap();
    (user, token)
}

async fn sync(service: &Service, token: &str, since: Option<&str>) -> Value {
    let mut url = url::Url::parse("http://localhost/_matrix/client/v3/sync").unwrap();
    url.query_pairs_mut()
        .append_pair("timeout", "0")
        .append_pair(
            "filter",
            &json!({"room":{"timeline":{"limit":1}}}).to_string(),
        );
    if let Some(since) = since {
        url.query_pairs_mut().append_pair("since", since);
    }
    ok(
        TestClient::get(url.as_str()).add_header("Authorization", format!("Bearer {token}"), true),
        service,
    )
    .await
}

fn members_request(room: &OwnedRoomId, token: &str, at: &str) -> RequestBuilder {
    let mut url = url::Url::parse(&format!(
        "http://localhost/_matrix/client/v3/rooms/{room}/members"
    ))
    .unwrap();
    url.query_pairs_mut().append_pair("at", at);
    TestClient::get(url.as_str()).add_header("Authorization", format!("Bearer {token}"), true)
}

fn member<'a>(body: &'a Value, user: &OwnedUserId) -> Option<&'a Value> {
    body["chunk"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["state_key"] == user.as_str())
}

fn snapshot(body: &Value) -> std::collections::BTreeMap<String, (Value, Value)> {
    body["chunk"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            (
                event["state_key"].as_str().unwrap().to_owned(),
                (event["event_id"].clone(), event["content"].clone()),
            )
        })
        .collect()
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_members_publication_points_preserve_snapshot_and_leave_cap() {
    std::thread::Builder::new()
        .name("members-publication-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(members_publication());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn members_publication() {
    crate::test_database::init();
    // Other database_ tests may already have initialized this immutable config.
    // All fixture identities use its actual server and a unique localpart.
    let _ = config::CONFIG.set(
        serde_json::from_value(json!({
            "server_name":"members-publication.example", "db":{"url":"unused-test-config"},
            "ip_range_denylist":[], "auto_join_rooms":[]
        }))
        .unwrap(),
    );
    let prefix = format!("members-p-{}", uuid::Uuid::new_v4().simple());
    let (alice, alice_token) = provision(&format!("{prefix}-alice")).await;
    let (bob, bob_token) = provision(&format!("{prefix}-bob")).await;
    let (charlie, charlie_token) = provision(&format!("{prefix}-charlie")).await;
    let (_, keeper_token) = provision(&format!("{prefix}-keeper")).await;
    let service = Service::new(crate::routing::root());
    let created = ok(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", format!("Bearer {alice_token}"), true)
            .json(&json!({"room_version":"11", "preset":"public_chat"})),
        &service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    // Keep local participation while Alice and Bob both leave below.
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/join"
        ))
        .add_header("Authorization", format!("Bearer {keeper_token}"), true)
        .json(&json!({})),
        &service,
    )
    .await;
    let before = sync(&service, &alice_token, None).await;
    let previous = before["next_batch"].as_str().unwrap();
    assert!(matches!(
        previous.parse().unwrap(),
        BatchToken::Publication { .. }
    ));
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/join"
        ))
        .add_header("Authorization", format!("Bearer {bob_token}"), true)
        .json(&json!({})),
        &service,
    )
    .await;
    let bob_join = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        bob.as_str(),
        None,
    )
    .await
    .unwrap();
    let joined = sync(&service, &alice_token, Some(previous)).await;
    let timeline = &joined["rooms"]["join"][room.as_str()]["timeline"];
    assert_eq!(timeline["events"].as_array().unwrap().len(), 1, "{joined}");
    assert_eq!(timeline["events"][0]["state_key"], bob.as_str(), "{joined}");
    let prev = timeline["prev_batch"].as_str().unwrap();
    let next = joined["next_batch"].as_str().unwrap();
    assert!(matches!(
        prev.parse().unwrap(),
        BatchToken::Publication { .. }
    ));
    assert!(matches!(
        next.parse().unwrap(),
        BatchToken::Publication { .. }
    ));
    let before_join = ok(members_request(&room, &alice_token, prev), &service).await;
    assert!(member(&before_join, &bob).is_none(), "{before_join}");
    assert_eq!(
        member(&before_join, &alice).unwrap()["content"]["membership"],
        "join"
    );
    let after_join = ok(members_request(&room, &alice_token, next), &service).await;
    assert_eq!(
        member(&after_join, &bob).unwrap()["content"]["membership"],
        "join"
    );

    // Both Alice and Charlie join after Bob's departure. Bob's future token
    // must still resolve Alice's old leave and must not reveal Charlie at all.
    for token in [&alice_token, &bob_token] {
        ok(
            TestClient::post(format!(
                "http://localhost/_matrix/client/v3/rooms/{room}/leave"
            ))
            .add_header("Authorization", format!("Bearer {token}"), true)
            .json(&json!({})),
            &service,
        )
        .await;
    }
    assert!(!room::state::user_can_see_events(&bob, &room).await.unwrap());
    for token in [&alice_token, &charlie_token] {
        ok(
            TestClient::post(format!(
                "http://localhost/_matrix/client/v3/rooms/{room}/join"
            ))
            .add_header("Authorization", format!("Bearer {token}"), true)
            .json(&json!({})),
            &service,
        )
        .await;
    }
    let future = sync(&service, &alice_token, None).await;
    let future = future["next_batch"].as_str().unwrap();
    let current = ok(members_request(&room, &alice_token, future), &service).await;
    assert_eq!(
        member(&current, &charlie).unwrap()["content"]["membership"],
        "join"
    );
    let capped = ok(members_request(&room, &bob_token, future), &service).await;
    assert!(member(&capped, &charlie).is_none(), "{capped}");
    assert_eq!(
        member(&capped, &alice).unwrap()["content"]["membership"],
        "leave"
    );
    assert_eq!(
        member(&capped, &bob).unwrap()["content"]["membership"],
        "leave"
    );
    let still_before = ok(members_request(&room, &alice_token, prev), &service).await;
    assert_eq!(
        snapshot(&still_before),
        snapshot(&before_join),
        "an issued p boundary moved with current state"
    );

    // The existing identity and topology token branches keep their semantics.
    let legacy = ok(
        members_request(&room, &alice_token, &bob_join.live_token().to_string()),
        &service,
    )
    .await;
    let historical = ok(
        members_request(&room, &alice_token, &bob_join.historic_token().to_string()),
        &service,
    )
    .await;
    assert_eq!(snapshot(&legacy), snapshot(&historical));
    assert_eq!(
        member(&legacy, &bob).unwrap()["content"]["membership"],
        "join"
    );
    assert!(member(&legacy, &charlie).is_none());
    let q = BatchToken::new_publication_page(
        None,
        Some(next.parse::<BatchToken>().unwrap().stream_ordering()),
        bob_join.depth as i64,
        bob_join.event_sn,
    )
    .to_string();
    assert!(matches!(
        q.parse().unwrap(),
        BatchToken::PublicationPage { .. }
    ));
    let mut rejected = send(members_request(&room, &alice_token, &q), &service).await;
    let body: Value = rejected.take_json().await.unwrap();
    assert_eq!(
        rejected.status_code,
        Some(StatusCode::BAD_REQUEST),
        "{body}"
    );
}
