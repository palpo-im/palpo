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
    // Give an unrelated user a fresh presence position in this response window,
    // so privacy is tested against a real candidate rather than an old row.
    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/presence/{eve}/status"
        ))
        .add_header("Authorization", "Bearer eve-presence", true)
        .json(&json!({"presence":"unavailable"})),
        &service,
    )
    .await;

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
    let member_filter = json!({"room": {
        "state": {"not_types": ["m.room.member"], "limit": 0},
        "timeline": {"not_types": ["m.room.member"], "limit": 0}
    }});
    let filtered = sync(&service, Some(since), Some(member_filter.clone())).await;
    assert!(
        presence_users(&filtered).contains(&bob.as_str()),
        "{filtered}"
    );
    assert!(!presence_users(&filtered).contains(&eve.as_str()));
    for section in ["state", "timeline"] {
        assert!(
            filtered["rooms"]["join"][room.as_str()][section]["events"]
                .as_array()
                .is_none_or(|events| events.is_empty()),
            "{filtered}"
        );
    }
    // MSC4495 can independently restamp presence after a join. Verify the real
    // unfiltered membership accumulator as well, so that stream refresh cannot
    // hide a missing semantic delta in the all-features test.
    let filter: crate::core::client::filter::FilterDefinition =
        serde_json::from_value(member_filter).unwrap();
    let now = data::curr_sn().await.unwrap();
    let (mut keys, mut joined, mut left) = (
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    );
    super::load_joined_room(
        &alice,
        "D".into(),
        &room,
        false,
        Some(token),
        Some(BatchToken::new_live(now)),
        BatchToken::new_live(now + 1),
        false,
        &filter,
        false,
        &mut keys,
        &mut joined,
        &mut left,
    )
    .await
    .unwrap();
    assert!(joined.contains(&bob));
    assert!(!joined.contains(&eve));

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

    #[cfg(feature = "unstable-msc4495")]
    {
        use crate::core::events::presence::sharing::PresenceSharingEventContent;
        use crate::core::events::{GlobalAccountDataEventType, StaticEventContent};
        data::user::set_data(
            &bob,
            None,
            &GlobalAccountDataEventType::from(PresenceSharingEventContent::TYPE).to_string(),
            json!({"share_locally":true, "users":{(alice.as_str()):"deny"}}),
        )
        .await
        .unwrap();
        let denied = sync(&service, Some(since), None).await;
        assert!(!presence_users(&denied).contains(&bob.as_str()), "{denied}");
        assert!(!presence_users(&denied).contains(&eve.as_str()));
    }

    // The room's current state may move beyond a sync worker's captured end.
    // Future membership and state must not enter that earlier response window.
    data::user::device::create_device(&alice, "WINDOW".into(), "alice-window", None, None)
        .await
        .unwrap();
    let previous_alice_member = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        alice.as_str(),
        None,
    )
    .await
    .unwrap();
    let message = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/window-message"
        ))
        .add_header("Authorization", "Bearer alice-presence", true)
        .json(&json!({"msgtype":"m.text", "body":"within captured window"})),
        &service,
    )
    .await;
    let message_id = crate::core::EventId::parse(message["event_id"].as_str().unwrap()).unwrap();
    let message_sn = room::timeline::get_pdu(&message_id).await.unwrap().event_sn;
    let captured_until = data::curr_sn().await.unwrap();
    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/profile/{alice}/displayname"
        ))
        .add_header("Authorization", "Bearer alice-presence", true)
        .json(&json!({"displayname":"future Alice beyond captured window"})),
        &service,
    )
    .await;
    let future_alice_member = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        alice.as_str(),
        None,
    )
    .await
    .unwrap();
    assert!(future_alice_member.event_sn > captured_until);
    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/m.room.name"
        ))
        .add_header("Authorization", "Bearer alice-presence", true)
        .json(&json!({"name":"future name beyond captured window"})),
        &service,
    )
    .await;
    request(
        TestClient::post(format!("http://localhost/_matrix/client/v3/join/{room}"))
            .add_header("Authorization", "Bearer eve-presence", true)
            .json(&json!({})),
        &service,
    )
    .await;
    request(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/leave"
        ))
        .add_header("Authorization", "Bearer bob-presence", true)
        .json(&json!({})),
        &service,
    )
    .await;
    assert!(room::user::join_sn(&eve, &room).await.unwrap() > captured_until);
    assert!(room::user::left_sn(&room, &bob).await.unwrap() > captured_until);
    for since in [Some(token), None] {
        let (mut keys, mut joined, mut left) = (
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        );
        let (bounded_room, _) = super::load_joined_room(
            &alice,
            "D".into(),
            &room,
            false,
            since,
            Some(BatchToken::new_live(captured_until)),
            BatchToken::new_live(captured_until + 1),
            false,
            &crate::core::client::filter::FilterDefinition::default(),
            false,
            &mut keys,
            &mut joined,
            &mut left,
        )
        .await
        .unwrap();
        assert!(!joined.contains(&eve));
        assert!(!keys.contains(&eve));
        assert!(!left.contains(&bob));
        if since.is_none() {
            assert!(
                joined.contains(&bob),
                "historical join was overwritten by a later leave"
            );
            assert!(keys.contains(&bob));
        }
        let bounded = serde_json::to_value(bounded_room).unwrap();
        for section in ["state", "timeline"] {
            for event in bounded[section]["events"].as_array().into_iter().flatten() {
                assert_ne!(
                    event["content"]["name"],
                    "future name beyond captured window"
                );
                assert_ne!(
                    event["content"]["displayname"],
                    "future Alice beyond captured window"
                );
                assert!(!(event["type"] == "m.room.member" && event["state_key"] == eve.as_str()));
                assert!(
                    !(event["type"] == "m.room.member"
                        && event["state_key"] == bob.as_str()
                        && event["content"]["membership"] == "leave")
                );
            }
        }
    }

    // An unchanged in-window membership must also be used when a cold device
    // needs the timeline sender's member event as lazy-loading supplementation.
    assert!(
        !room::lazy_loading::lazy_load_was_sent_before(&alice, "WINDOW".into(), &room, &alice)
            .await
            .unwrap()
    );
    let lazy_filter = serde_json::from_value(json!({"room": {
        "state": {"lazy_load_members":true}, "timeline":{"limit":1}
    }}))
    .unwrap();
    let (mut keys, mut joined, mut left) = (
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    );
    let (bounded_room, _) = super::load_joined_room(
        &alice,
        "WINDOW".into(),
        &room,
        false,
        Some(BatchToken::new_live(message_sn)),
        Some(BatchToken::new_live(captured_until)),
        BatchToken::new_live(captured_until + 1),
        false,
        &lazy_filter,
        false,
        &mut keys,
        &mut joined,
        &mut left,
    )
    .await
    .unwrap();
    let bounded = serde_json::to_value(bounded_room).unwrap();
    assert!(
        bounded["timeline"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event_id"] == message_id.as_str())
    );
    let sender_member = bounded["state"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["type"] == "m.room.member" && event["state_key"] == alice.as_str())
        .expect("cold lazy-load cache must receive the message sender's membership");
    assert_eq!(
        sender_member["event_id"],
        previous_alice_member.event_id.as_str()
    );
    assert_ne!(
        sender_member["event_id"],
        future_alice_member.event_id.as_str()
    );
}
