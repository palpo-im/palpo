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
    let before_bob_join_frame = room::get_frame_id(&room, None).await.unwrap();
    let alice_member = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        alice.as_str(),
        None,
    )
    .await
    .unwrap();
    let alice_before_frame = room::state::get_pdu_before_frame_id(&alice_member.event_id)
        .await
        .unwrap();
    let alice_after_frame = room::state::get_pdu_frame_id(&alice_member.event_id)
        .await
        .unwrap();
    assert!(!presence_users(&previous).contains(&bob.as_str()));
    // Public-room federation joins resolve the full state before promotion.
    // Use the actual send_join pipeline, including signed PDU validation, so
    // it cannot silently move an old event's historical frame past this token.
    let (join_pdu, mut join) = crate::event::PduBuilder {
        event_type: "m.room.member".into(),
        state_key: Some(bob.to_string()),
        content: serde_json::value::to_raw_value(&json!({"membership":"join"})).unwrap(),
        ..Default::default()
    }
    .hash_sign(&bob, &room, &crate::core::RoomVersionId::V11)
    .await
    .unwrap();
    // An already cached outlier may have a historical frame. Only promotion
    // of that event may replace it; retaining other events must never do so.
    let (_, _, guard) =
        crate::event::PduBuilder::save_as_outlier(join_pdu.clone(), join.clone(), &bob)
            .await
            .unwrap();
    crate::event::update_frame_id(&join_pdu.event_id, before_bob_join_frame)
        .await
        .unwrap();
    drop(guard);
    join.remove("event_id");
    let join = serde_json::value::to_raw_value(&join).unwrap();
    crate::federation::membership::send_join_v2(SERVER.try_into().unwrap(), &room, &join)
        .await
        .unwrap();
    assert_eq!(
        super::captured_sync_frame(&room, token.event_sn() - 1)
            .await
            .unwrap(),
        before_bob_join_frame,
        "federated state resolution must preserve the pre-join historical frame"
    );
    assert_eq!(
        room::state::get_pdu_before_frame_id(&alice_member.event_id)
            .await
            .unwrap(),
        alice_before_frame
    );
    assert_eq!(
        room::state::get_pdu_frame_id(&alice_member.event_id)
            .await
            .unwrap(),
        alice_after_frame
    );
    assert_eq!(
        room::state::get_pdu_frame_id(&join_pdu.event_id)
            .await
            .unwrap(),
        room::get_frame_id(&room, None).await.unwrap()
    );
    assert_eq!(
        room::user::join_sn(&bob, &room).await.unwrap(),
        token.event_sn(),
        "fixture must put the membership exactly at the inclusive timeline cursor"
    );
    // send_join imports can allocate newer local positions for older cached
    // auth/state. Model that metadata without publishing it on the timeline.
    // The unqualified historical lookup must demonstrably select the stale
    // frame, while the actual sync must retain the published Bob membership.
    {
        use diesel::sql_types::{BigInt, Text};
        use diesel_async::RunQueryDsl;
        let imported_sn = data::next_sn().await.unwrap();
        let imported_id = "$cached-import-after-live-join";
        let mut connection = data::connect().await.unwrap();
        diesel::sql_query("INSERT INTO events
            (id,sn,ty,room_id,depth,topological_ordering,stream_ordering,origin_server_ts,contains_url,is_outlier)
            VALUES ($1,$2,'m.room.message',$3,1,1,$2,0,false,true)")
            .bind::<Text,_>(imported_id).bind::<BigInt,_>(imported_sn)
            .bind::<Text,_>(room.as_str()).execute(&mut connection).await.unwrap();
        diesel::sql_query(
            "INSERT INTO event_points (event_id,event_sn,room_id,frame_id)
            VALUES ($1,$2,$3,$4)",
        )
        .bind::<Text, _>(imported_id)
        .bind::<BigInt, _>(imported_sn)
        .bind::<Text, _>(room.as_str())
        .bind::<BigInt, _>(before_bob_join_frame)
        .execute(&mut connection)
        .await
        .unwrap();
        drop(connection);
        assert_eq!(
            room::get_frame_id(&room, Some(imported_sn)).await.unwrap(),
            before_bob_join_frame
        );
    }
    // Reproduce another initial sync winning the upstream parallel test race.
    let initial = sync(&service, None, None).await;
    assert!(presence_users(&initial).contains(&bob.as_str()));
    // The upstream parallel join-events checker can win the race and populate
    // this device's delivery cache before its presence checker uses the same
    // old cursor. Initial sync can put Bob only in timeline, so run that real
    // incremental sibling rather than assuming initial state marked the cache.
    let sibling = sync(&service, Some(since), None).await;
    assert!(
        sibling["device_lists"]["changed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|user| user.as_str() == Some(bob.as_str())),
        "the real federated join must report Bob's device list: {sibling}"
    );
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
                    event["content"]["displayname"], "future Alice beyond captured window",
                    "since={since:?}, section={section}, event={event}"
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

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn http_delayed_membership_promotion_uses_publication_order() {
    std::thread::Builder::new()
        .name("delayed-member-publication-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(delayed_member_publication());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn delayed_member_publication() {
    use diesel::prelude::*;
    use diesel_async::{AsyncConnection, RunQueryDsl, SimpleAsyncConnection};
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name": SERVER, "db": {"url":"unused-test-config"},
                "ip_range_denylist":[], "auto_join_rooms":[]
            }))
            .unwrap(),
        )
        .unwrap();
    for name in ["alice", "bob", "eve"] {
        provision(name).await;
    }
    let service = Service::new(crate::routing::root());
    let bob = UserId::parse(format!("@bob:{SERVER}")).unwrap();
    let alice = UserId::parse(format!("@alice:{SERVER}")).unwrap();
    let room = request(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", "Bearer alice-presence", true)
            .json(&json!({"room_version":"11","preset":"public_chat"})),
        &service,
    )
    .await;
    let room: OwnedRoomId = room["room_id"].as_str().unwrap().try_into().unwrap();
    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/presence/{bob}/status"
        ))
        .add_header("Authorization", "Bearer bob-presence", true)
        .json(&json!({"presence":"online"})),
        &service,
    )
    .await;
    let before_frame = room::get_frame_id(&room, None).await.unwrap();

    // Cache the real signed join at N, without publishing it or retaining its
    // original sequence guard. A different live event then overtakes it.
    let (join_pdu, mut join) = crate::event::PduBuilder {
        event_type: "m.room.member".into(),
        state_key: Some(bob.to_string()),
        content: serde_json::value::to_raw_value(&json!({"membership":"join"})).unwrap(),
        ..Default::default()
    }
    .hash_sign(&bob, &room, &crate::core::RoomVersionId::V11)
    .await
    .unwrap();
    let (cached, _, guard) =
        crate::event::PduBuilder::save_as_outlier(join_pdu.clone(), join.clone(), &bob)
            .await
            .unwrap();
    crate::event::update_frame_id(&cached.event_id, before_frame)
        .await
        .unwrap();
    drop(guard);
    let before_invalid = data::curr_sn().await.unwrap();
    let mut soft_failed = cached.clone();
    soft_failed.soft_failed = true;
    let mut rejected = cached.clone();
    rejected.pdu.rejection_reason = Some("fixture rejected verdict".into());
    let mut backfilled = cached.clone();
    backfilled.is_backfill = true;
    for invalid in [&soft_failed, &rejected, &backfilled] {
        let mut conn = data::connect().await.unwrap();
        let result = conn
            .transaction::<_, crate::AppError, _>(async |conn| {
                crate::event::sticky::publish_live_frame_with_conn(conn, invalid, before_frame)
                    .await
            })
            .await;
        assert!(
            result.is_err(),
            "non-live or unauthenticated PDU gained a publication"
        );
    }
    for invalid in [&soft_failed, &rejected] {
        assert!(
            room::timeline::append_pdu(invalid, join.clone(), &room::lock_state(&room).await)
                .await
                .is_err(),
            "invalid PDU reached derived membership publication"
        );
    }
    assert_eq!(data::curr_sn().await.unwrap(), before_invalid);
    assert!(!room::user::is_joined(&bob, &room).await.unwrap());
    let unpublished = data::schema::event_points::table
        .find(&*cached.event_id)
        .select((
            data::schema::event_points::live_published_sn,
            data::schema::event_points::live_published_frame_id,
        ))
        .first::<(Option<i64>, Option<i64>)>(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    assert_eq!(unpublished, (None, None));
    let message = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/overtakes-outlier"
        ))
        .add_header("Authorization", "Bearer alice-presence", true)
        .json(&json!({"msgtype":"m.text","body":"newer live event"})),
        &service,
    )
    .await;
    let message_id = crate::core::EventId::parse(message["event_id"].as_str().unwrap()).unwrap();
    let message_sn = room::timeline::get_pdu(&message_id).await.unwrap().event_sn;
    assert!(message_sn > cached.event_sn);
    // Sign a valid historical message against this pre-promotion state, but
    // defer its receipt until the join-history recovery check below.
    let (history_pdu, history_json) = crate::event::PduBuilder {
        event_type: "m.room.message".into(),
        timestamp: Some(UnixMillis(1)),
        content: serde_json::value::to_raw_value(
            &json!({"body":"recovered history","msgtype":"m.text"}),
        )
        .unwrap(),
        ..Default::default()
    }
    .hash_sign(&alice, &room, &crate::core::RoomVersionId::V11)
    .await
    .unwrap();
    let previous = sync(&service, None, None).await;
    let since = previous["next_batch"].as_str().unwrap();
    let old_until = since.parse::<BatchToken>().unwrap().event_sn() - 1;
    assert!(old_until >= message_sn);
    assert!(!presence_users(&previous).contains(&bob.as_str()));
    let before_promotion = super::captured_sync_frame(&room, old_until).await.unwrap();
    let watch_before = crate::watcher::live_position_for_test(&alice, "D".into())
        .await
        .unwrap();

    // Pause after allocation of the publication cursor but before commit.
    // With the original cached identity guard gone, the sync watermark lock
    // itself must prevent a reader from advancing past this uncommitted write.
    let mut blocker = data::connect().await.unwrap();
    blocker
        .batch_execute(
            "CREATE TABLE publication_test_target(event_id TEXT PRIMARY KEY);
        CREATE FUNCTION block_member_publication() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN IF EXISTS(SELECT 1 FROM publication_test_target WHERE event_id=NEW.event_id) THEN
            PERFORM pg_advisory_xact_lock(616,404);
        END IF; RETURN NEW; END $$;
        CREATE TRIGGER block_member_publication BEFORE UPDATE OF live_published_sn ON event_points
        FOR EACH ROW WHEN (OLD.live_published_sn IS NULL AND NEW.live_published_sn IS NOT NULL)
        EXECUTE FUNCTION block_member_publication();",
        )
        .await
        .unwrap();
    diesel::sql_query("INSERT INTO publication_test_target(event_id) VALUES ($1)")
        .bind::<diesel::sql_types::Text, _>(cached.event_id.as_str())
        .execute(&mut blocker)
        .await
        .unwrap();
    blocker
        .batch_execute("BEGIN; SELECT pg_advisory_xact_lock(616,404)")
        .await
        .unwrap();
    join.remove("event_id");
    let joining_room = room.clone();
    let promotion = tokio::spawn(async move {
        crate::federation::membership::send_join_v2(
            SERVER.try_into().unwrap(),
            &joining_room,
            &serde_json::value::to_raw_value(&join).unwrap(),
        )
        .await
    });
    let (start_sync, started) = tokio::sync::oneshot::channel();
    let waiting_sync = tokio::spawn(async {
        started.await.unwrap();
        let service = Service::new(crate::routing::root());
        sync(&service, None, None).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        wait_for_publication_lock(616, 404, 2, "ExclusiveLock").await;
        start_sync.send(()).unwrap();
        wait_for_publication_lock(0, 1_346_456_654, 1, "ShareLock").await;
    })
    .await
    .expect("writer and sync must meet the deterministic publication barriers");
    assert_eq!(
        crate::watcher::live_position_for_test(&alice, "D".into())
            .await
            .unwrap(),
        watch_before,
        "an uncommitted publication must not become a watcher baseline"
    );
    assert_eq!(
        super::captured_sync_frame(&room, old_until).await.unwrap(),
        before_promotion
    );
    assert!(
        data::schema::events::table
            .find(&*cached.event_id)
            .select(data::schema::events::is_outlier)
            .first::<bool>(&mut data::connect().await.unwrap())
            .await
            .unwrap()
    );
    blocker
        .batch_execute(
            "COMMIT; DROP TRIGGER block_member_publication ON event_points;
        DROP FUNCTION block_member_publication(); DROP TABLE publication_test_target;",
        )
        .await
        .unwrap();
    drop(blocker);
    promotion.await.unwrap().unwrap();
    assert!(
        crate::watcher::live_position_for_test(&alice, "D".into())
            .await
            .unwrap()
            > watch_before,
        "promotion of an older cached identity must wake the batched watcher"
    );
    let committed_reader = waiting_sync.await.unwrap();
    assert!(
        presence_users(&committed_reader).contains(&bob.as_str()),
        "{committed_reader}"
    );
    assert!(room::user::is_joined(&bob, &room).await.unwrap());
    assert_eq!(
        room::user::join_sn(&bob, &room).await.unwrap(),
        cached.event_sn,
        "the cached event identity must not be restamped"
    );
    let now = data::curr_sn().await.unwrap();
    assert_eq!(
        super::captured_sync_frame(&room, now).await.unwrap(),
        room::get_frame_id(&room, None).await.unwrap(),
        "maximum original stream position selected the pre-promotion frame"
    );
    assert_eq!(
        super::captured_sync_frame(&room, old_until).await.unwrap(),
        before_promotion,
        "late publication must not enter an already captured response window"
    );

    // Both an initial sync and the old cursor must see publication without
    // needing another room event, an incidental presence write, or a retry.
    for response in [
        sync(&service, None, None).await,
        sync(&service, Some(since), None).await,
    ] {
        assert!(
            presence_users(&response).contains(&bob.as_str()),
            "{response}"
        );
        assert!(
            response["device_lists"]["changed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|user| user.as_str() == Some(bob.as_str())),
            "{response}"
        );
        assert!(
            ["state", "timeline"]
                .iter()
                .any(
                    |section| response["rooms"]["join"][room.as_str()][section]["events"]
                        .as_array()
                        .is_some_and(|events| events
                            .iter()
                            .any(|event| event["type"] == "m.room.member"
                                && event["state_key"] == bob.as_str()))
                ),
            "the late member must be in initial timeline/state or incremental state: {response}"
        );
    }
    // Old captured state remains free of the newly promoted member, even
    // though their original identity N falls below this old upper bound.
    let (mut changed, mut joined, mut left) = Default::default();
    let (bounded, _) = super::load_joined_room(
        &alice,
        "D".into(),
        &room,
        false,
        None,
        Some(BatchToken::new_live(old_until)),
        BatchToken::new_live(old_until + 1),
        false,
        &Default::default(),
        false,
        &mut changed,
        &mut joined,
        &mut left,
    )
    .await
    .unwrap();
    assert!(!changed.contains(&bob) && !joined.contains(&bob));
    let state = serde_json::to_value(bounded).unwrap();
    for section in ["state", "timeline"] {
        assert!(
            !state[section]["events"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|event| event["type"] == "m.room.member" && event["state_key"] == bob.as_str())
        );
    }

    // Materialize a genuinely negative-history event through the normal
    // backfill append path. Only the remote fetch result is substituted below;
    // missing-frontier detection and the real joined-room reload SQL execute.
    let (mut history, history_json, history_guard) =
        crate::event::PduBuilder::save_as_outlier(history_pdu, history_json, &alice)
            .await
            .unwrap();
    history.is_backfill = true;
    crate::event::update_before_frame_id(&history.event_id, before_promotion)
        .await
        .unwrap();
    room::timeline::append_pdu(&history, history_json, &room::lock_state(&room).await)
        .await
        .unwrap();
    drop(history_guard);
    let history = room::timeline::get_pdu(&history.event_id).await.unwrap();
    assert!(history.is_backfill && !history.is_outlier);
    let history_publication = data::schema::event_points::table
        .find(&*history.event_id)
        .select((
            data::schema::event_points::live_published_sn,
            data::schema::event_points::live_published_frame_id,
        ))
        .first::<(Option<i64>, Option<i64>)>(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    assert_eq!(history_publication, (None, None));
    let old_next = BatchToken::new_live(old_until + 1);
    // Positive control: ordinary historical pagination still follows original
    // topology and identity; only a captured sync adds the publication bound.
    let generic = room::timeline::topolo::load_pdus_backward(
        Some(&alice),
        &room,
        Some(old_next),
        None,
        None,
        100,
    )
    .await
    .unwrap();
    assert!(generic.contains_key(&cached.event_sn));
    assert!(generic.contains_key(&history.event_sn));
    diesel::sql_query("INSERT INTO event_backward_extremities(room_id,event_id) VALUES ($1,'$missing-history-frontier')")
        .bind::<diesel::sql_types::Text, _>(room.as_str())
        .execute(&mut data::connect().await.unwrap()).await.unwrap();
    *room::timeline::BACKFILL_RESULT_FOR_TEST.lock().unwrap() =
        Some(room::timeline::BackfillResultForTest {
            room_id: room.clone(),
            event_id: history.event_id.clone(),
            calls: 0,
        });
    let mut filter = crate::core::client::filter::FilterDefinition::default();
    filter.room.timeline.limit = Some(100);
    let new_next = BatchToken::new_live(data::curr_sn().await.unwrap() + 1);
    for (upper, expect_join) in [(old_next, false), (new_next, true)] {
        let normal = super::load_timeline(
            &alice,
            &room,
            None,
            Some(upper),
            Some(&filter.room.timeline),
        )
        .await
        .unwrap();
        assert!(
            normal.events.contains_key(&history.event_sn),
            "normal captured timeline must retain already fetched negative history"
        );
        assert_eq!(
            normal.events.contains_key(&cached.event_sn),
            expect_join,
            "normal captured timeline crossed its publication window"
        );
        let (mut changed, mut joined, mut left) = Default::default();
        let (recovered, _) = super::load_joined_room(
            &alice,
            "D".into(),
            &room,
            false,
            None,
            None,
            upper,
            false,
            &filter,
            false,
            &mut changed,
            &mut joined,
            &mut left,
        )
        .await
        .unwrap();
        let value = serde_json::to_value(recovered).unwrap();
        let events = value["timeline"]["events"].as_array().unwrap();
        assert!(
            events
                .iter()
                .any(|event| event["event_id"] == history.event_id.as_str()),
            "true negative history was dropped: {value}"
        );
        assert_eq!(
            events
                .iter()
                .any(|event| event["event_id"] == cached.event_id.as_str()),
            expect_join,
            "join-history recovery crossed its publication window: {value}"
        );
    }
    let injected = room::timeline::BACKFILL_RESULT_FOR_TEST
        .lock()
        .unwrap()
        .take()
        .unwrap();
    assert_eq!(
        injected.calls, 2,
        "both sync windows must take the nonempty backfill branch"
    );
    diesel::sql_query("DELETE FROM event_backward_extremities WHERE room_id=$1 AND event_id='$missing-history-frontier'")
        .bind::<diesel::sql_types::Text,_>(room.as_str())
        .execute(&mut data::connect().await.unwrap()).await.unwrap();
    late_published_timeline(&service, &alice, &history.event_id).await;
}

async fn late_published_timeline(
    service: &Service,
    alice: &UserId,
    history_id: &crate::core::EventId,
) {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    let created = request(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", "Bearer alice-presence", true)
            .json(&json!({"room_version":"11","preset":"public_chat"})),
        service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    let mut pending = Vec::new();
    for (event_type, state_key, content) in [
        (
            "m.room.message",
            None,
            json!({"msgtype":"m.text","body":"cached before sync, accepted later"}),
        ),
        (
            "m.room.server_acl",
            Some("".to_owned()),
            json!({"allow":["*"],"deny":[],"allow_ip_literals":true}),
        ),
    ] {
        let (pdu, json) = crate::event::PduBuilder {
            event_type: event_type.into(),
            state_key,
            content: serde_json::value::to_raw_value(&content).unwrap(),
            ..Default::default()
        }
        .hash_sign(alice, &room, &crate::core::RoomVersionId::V11)
        .await
        .unwrap();
        let (cached, json, guard) = crate::event::PduBuilder::save_as_outlier(pdu, json, alice)
            .await
            .unwrap();
        drop(guard);
        pending.push((cached, json));
    }
    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/earlier-anchor"
        ))
        .add_header("Authorization", "Bearer alice-presence", true)
        .json(&json!({"msgtype":"m.text","body":"visible before publication"})),
        service,
    )
    .await;
    let old = sync(service, None, None).await;
    let lower = old["next_batch"].as_str().unwrap();
    let lower_sn = lower.parse::<BatchToken>().unwrap().event_sn();
    for (cached, json) in &pending {
        assert!(cached.event_sn < lower_sn);
        assert!(!timeline_ids(&old, &room).contains(&cached.event_id.as_str()));
        // V11 federation PDUs omit the local storage-only event_id, which was
        // added after signing and must not enter signature verification.
        let mut wire = json.clone();
        wire.remove("event_id");
        crate::event::handler::process_incoming_pdu(
            SERVER.try_into().unwrap(),
            &cached.event_id,
            &room,
            &crate::core::RoomVersionId::V11,
            wire,
            true,
            false,
        )
        .await
        .unwrap();
        let accepted = room::timeline::get_pdu(&cached.event_id).await.unwrap();
        assert!(!accepted.is_outlier && !accepted.soft_failed);
        assert_eq!(
            accepted.event_sn, cached.event_sn,
            "event identity was rewritten"
        );
        let publication = data::schema::event_points::table
            .find(&*cached.event_id)
            .select(data::schema::event_points::live_published_sn)
            .first::<Option<i64>>(&mut data::connect().await.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(lower_sn <= publication);
        eprintln!(
            "late timeline witness identity={} http_since={} publication={}",
            cached.event_sn, lower_sn, publication
        );
    }
    let newer = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/newer-live"
        ))
        .add_header("Authorization", "Bearer alice-presence", true)
        .json(&json!({"msgtype":"m.text","body":"newer live identity"})),
        service,
    )
    .await;
    let newer_id = newer["event_id"].as_str().unwrap();
    let full = sync(service, Some(lower), None).await;
    let ids = timeline_ids(&full, &room);
    for (cached, _) in &pending {
        assert_eq!(
            ids.iter()
                .filter(|id| **id == cached.event_id.as_str())
                .count(),
            1,
            "published old identity was lost from actual incremental HTTP timeline: {full}"
        );
    }
    assert_eq!(
        ids,
        vec![
            pending[0].0.event_id.as_str(),
            pending[1].0.event_id.as_str(),
            newer_id
        ],
        "publication selection must preserve original timeline chronology"
    );
    let next = full["next_batch"].as_str().unwrap();
    let repeated = sync(service, Some(next), None).await;
    assert!(
        timeline_ids(&repeated, &room).is_empty(),
        "late events were repeated: {repeated}"
    );
    assert!(
        !repeated.to_string().contains(history_id.as_str()),
        "incremental sync replayed negative history"
    );

    let limited = sync(
        service,
        Some(lower),
        Some(json!({"room":{"timeline":{"limit":1}}})),
    )
    .await;
    let room_data = &limited["rooms"]["join"][room.as_str()];
    assert_eq!(timeline_ids(&limited, &room), vec![newer_id]);
    assert_eq!(room_data["timeline"]["limited"], true);
    let mut from = room_data["timeline"]["prev_batch"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..20 {
        let mut url = url::Url::parse(&format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/messages"
        ))
        .unwrap();
        url.query_pairs_mut()
            .append_pair("dir", "b")
            .append_pair("limit", "1")
            .append_pair("from", &from);
        let page = request(
            TestClient::get(url.as_str()).add_header(
                "Authorization",
                "Bearer alice-presence",
                true,
            ),
            service,
        )
        .await;
        let events = page["chunk"].as_array().unwrap();
        if events.is_empty() {
            break;
        }
        for event in events {
            let id = event["event_id"].as_str().unwrap().to_owned();
            assert_ne!(id, newer_id, "history repeated the limited sync event");
            assert!(seen.insert(id), "history continuation repeated an event");
        }
        if pending
            .iter()
            .all(|(pdu, _)| seen.contains(pdu.event_id.as_str()))
        {
            break;
        }
        let end = page["end"].as_str().unwrap().to_owned();
        assert_ne!(from, end, "history cursor did not advance");
        from = end;
    }
    assert!(
        pending
            .iter()
            .all(|(pdu, _)| seen.contains(pdu.event_id.as_str())),
        "limited history continuation lost a late publication"
    );
    let unbounded = room::timeline::stream::load_pdus_backward_captured(
        Some(alice),
        &room,
        None,
        Some(lower.parse().unwrap()),
        None,
        100,
    )
    .await
    .unwrap();
    for (cached, _) in &pending {
        assert!(
            unbounded.contains_key(&cached.event_sn),
            "lower-only capture fell back to original identity bounds"
        );
    }
    let future = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/future-live"
        ))
        .add_header("Authorization", "Bearer alice-presence", true)
        .json(&json!({"msgtype":"m.text","body":"outside captured upper bound"})),
        service,
    )
    .await;
    let future_id = crate::core::EventId::parse(future["event_id"].as_str().unwrap()).unwrap();
    let captured = super::load_timeline(
        alice,
        &room,
        Some(lower.parse().unwrap()),
        Some(next.parse().unwrap()),
        None,
    )
    .await
    .unwrap();
    assert!(
        !captured
            .events
            .values()
            .any(|pdu| pdu.event_id == future_id)
    );
    for (cached, _) in &pending {
        assert!(captured.events.contains_key(&cached.event_sn));
    }
}

fn timeline_ids<'a>(sync: &'a Value, room: &crate::core::RoomId) -> Vec<&'a str> {
    sync["rooms"]["join"][room.as_str()]["timeline"]["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|event| event["event_id"].as_str())
        .collect()
}

async fn wait_for_publication_lock(class_id: i64, key: i64, sub_id: i16, mode: &str) {
    use diesel_async::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Waiting {
        #[diesel(sql_type=diesel::sql_types::Bool)]
        found: bool,
    }
    loop {
        let waiting = diesel::sql_query(
            "SELECT EXISTS(SELECT 1 FROM pg_locks
            WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database())
            AND locktype='advisory' AND NOT granted AND classid::bigint=$1 AND objid::bigint=$2
            AND objsubid=$3 AND mode=$4) AS found",
        )
        .bind::<diesel::sql_types::BigInt, _>(class_id)
        .bind::<diesel::sql_types::BigInt, _>(key)
        .bind::<diesel::sql_types::SmallInt, _>(sub_id)
        .bind::<diesel::sql_types::Text, _>(mode)
        .get_result::<Waiting>(&mut data::connect().await.unwrap())
        .await
        .unwrap();
        if waiting.found {
            return;
        }
        tokio::task::yield_now().await;
    }
}
