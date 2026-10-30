use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedRoomId, UnixMillis, UserId};
use crate::event::BatchToken;
use crate::{config, data, room};

const SERVER: &str = "member-presence.example";

async fn request(request: RequestBuilder, service: &Service) -> Value {
    let router = service.router();
    let mut response = tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap();
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

async fn sync(service: &Service, since: Option<&str>, filter: Option<Value>) -> Value {
    let mut url = url::Url::parse("http://localhost/_matrix/client/v3/sync").unwrap();
    url.query_pairs_mut().append_pair("timeout", "0");
    if let Some(since) = since {
        url.query_pairs_mut().append_pair("since", since);
    }
    if let Some(filter) = filter {
        url.query_pairs_mut()
            .append_pair("filter", &filter.to_string());
    }
    request(
        TestClient::get(url.as_str()).add_header("Authorization", "Bearer alice-presence", true),
        service,
    )
    .await
}

fn presence_users(sync: &Value) -> Vec<&str> {
    sync["presence"]["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|event| event["sender"].as_str())
        .collect()
}

async fn provision(name: &str) {
    let user = UserId::parse(format!("@{name}:{SERVER}")).unwrap();
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
    data::user::device::create_device(
        &user,
        &OwnedDeviceId::from("D"),
        &format!("{name}-presence"),
        None,
        None,
    )
    .await
    .unwrap();
    // Keep later sync requests from allocating a push-rule account-data event.
    crate::user::get_push_rules(&user).await.unwrap();
    #[cfg(feature = "unstable-msc4495")]
    {
        use crate::core::events::presence::sharing::PresenceSharingEventContent;
        use crate::core::events::{GlobalAccountDataEventType, StaticEventContent};
        data::user::set_data(
            &user,
            None,
            &GlobalAccountDataEventType::from(PresenceSharingEventContent::TYPE).to_string(),
            json!({"share_locally": true}),
        )
        .await
        .unwrap();
    }
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn http_incremental_member_presence_survives_other_sync_and_member_filters() {
    std::thread::Builder::new()
        .name("member-presence-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(member_presence());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn member_presence() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name": SERVER, "db": {"url": "unused-test-config"},
                "ip_range_denylist": [], "auto_join_rooms": []
            }))
            .unwrap(),
        )
        .unwrap();
    for name in ["alice", "bob", "eve"] {
        provision(name).await;
    }
    let service = Service::new(crate::routing::root());
    let alice = UserId::parse(format!("@alice:{SERVER}")).unwrap();
    let bob = UserId::parse(format!("@bob:{SERVER}")).unwrap();
    let eve = UserId::parse(format!("@eve:{SERVER}")).unwrap();
    let created = request(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", "Bearer alice-presence", true)
            .json(&json!({"room_version": "11", "preset": "public_chat"})),
        &service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    for name in ["alice", "bob", "eve"] {
        request(
            TestClient::put(format!(
                "http://localhost/_matrix/client/v3/presence/@{name}:{SERVER}/status"
            ))
            .add_header("Authorization", format!("Bearer {name}-presence"), true)
            .json(&json!({"presence": "online"})),
            &service,
        )
        .await;
    }
    let previous = sync(&service, None, None).await;
    let since = previous["next_batch"].as_str().unwrap();
    let token: BatchToken = since.parse().unwrap();
    assert!(!presence_users(&previous).contains(&bob.as_str()));
    request(
        TestClient::post(format!("http://localhost/_matrix/client/v3/join/{room}"))
            .add_header("Authorization", "Bearer bob-presence", true)
            .json(&json!({})),
        &service,
    )
    .await;
    assert_eq!(
        room::user::join_sn(&bob, &room).await.unwrap(),
        token.event_sn(),
        "fixture must put the membership exactly at the inclusive timeline cursor"
    );
    // Reproduce another initial sync winning the upstream parallel test race.
    let initial = sync(&service, None, None).await;
    assert!(presence_users(&initial).contains(&bob.as_str()));
    // The upstream parallel join-events checker can win the race and populate
    // this device's delivery cache before its presence checker uses the same
    // old cursor. Initial sync can put Bob only in timeline, so run that real
    // incremental sibling rather than assuming initial state marked the cache.
    let sibling = sync(&service, Some(since), None).await;
    assert!(
        presence_users(&sibling).contains(&bob.as_str()),
        "{sibling}"
    );
    assert!(
        sibling["rooms"]["join"][room.as_str()]["timeline"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["type"] == "m.room.member" && event["state_key"] == bob.as_str())
    );
    assert!(
        room::lazy_loading::lazy_load_was_sent_before(&alice, "D".into(), &room, &bob)
            .await
            .unwrap()
    );

    let incremental = sync(&service, Some(since), None).await;
    assert!(
        presence_users(&incremental).contains(&bob.as_str()),
        "new member presence must survive a different sync priming its lazy cache: {incremental}"
    );
    assert!(
        incremental["rooms"]["join"][room.as_str()]["timeline"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["type"] == "m.room.member" && event["state_key"] == bob.as_str())
    );
    // Presence is a separate stream from response state/timeline filtering.
    let filtered = sync(
        &service,
        Some(since),
        Some(json!({"room": {
            "state": {"not_types": ["m.room.member"], "limit": 0},
            "timeline": {"not_types": ["m.room.member"], "limit": 0}
        }})),
    )
    .await;
    assert!(
        presence_users(&filtered).contains(&bob.as_str()),
        "{filtered}"
    );
    assert!(!presence_users(&filtered).contains(&eve.as_str()));

    let latest = filtered["next_batch"].as_str().unwrap();
    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/m.room.name"
        ))
        .add_header("Authorization", "Bearer alice-presence", true)
        .json(&json!({"name": "unrelated state change"})),
        &service,
    )
    .await;
    let unrelated = sync(&service, Some(latest), None).await;
    assert!(
        !presence_users(&unrelated).contains(&bob.as_str()),
        "{unrelated}"
    );
    assert!(!presence_users(&unrelated).contains(&eve.as_str()));
}
