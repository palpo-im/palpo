use std::time::Duration;

use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

pub(super) async fn request(builder: RequestBuilder, service: &Service) -> Value {
    let router = service.router();
    let mut response = tokio::spawn(async move { builder.send(router).await })
        .await
        .unwrap();
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_filtered_room_skips_payloads_and_keeps_independent_membership() {
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
pub(super) async fn setup() -> (Service, OwnedRoomId, OwnedUserId, OwnedUserId) {
    crate::test_database::init();
    config::CONFIG.set(serde_json::from_value(json!({"server_name":"filtered.example", "db":{"url":"unused"},"auto_join_rooms":[]})).unwrap()).unwrap();
    for name in ["alice", "bob"] {
        let user = UserId::parse(format!("@{name}:filtered.example")).unwrap();
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
        data::user::device::create_device(&user, &OwnedDeviceId::from("D"), name, None, None)
            .await
            .unwrap();
    }
    let service = Service::new(crate::routing::root());
    let created = request(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", "Bearer alice", true)
            .json(&json!({"room_version":"11","preset":"public_chat"})),
        &service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    request(
        TestClient::post(format!("http://localhost/_matrix/client/v3/join/{room}"))
            .add_header("Authorization", "Bearer bob", true)
            .json(&json!({})),
        &service,
    )
    .await;
    let sender = UserId::parse("@alice:filtered.example").unwrap();
    let peer = UserId::parse("@bob:filtered.example").unwrap();
    (service, room, sender, peer)
}

async fn run() {
    let (_service, room, sender, peer) = setup().await;
    let next = BatchToken::new_publication(data::curr_sn().await.unwrap() + 1);
    let mut filter = FilterDefinition::default();
    filter.room.not_rooms.push(room.clone());
    let mut joined = HashSet::new();
    let mut left = HashSet::new();
    let spy = crate::test_database::sql_spy::SqlSpy::install().await;
    let (body, _) = load_joined_room(
        &sender,
        "D".into(),
        &room,
        false,
        None,
        None,
        next,
        false,
        &filter,
        false,
        &mut joined,
        &mut left,
    )
    .await
    .unwrap();
    assert!(body.is_empty());
    assert!(
        joined.contains(&peer),
        "excluded room must retain independent membership side effects"
    );
    assert!(
        !spy.snapshot()
            .iter()
            .any(|query| query.contains("event_receipts")),
        "filtered room fetched receipt payload"
    );
    spy.reset();
    filter.room.not_rooms.clear();
    load_joined_room(
        &sender,
        "D".into(),
        &room,
        false,
        None,
        None,
        next,
        false,
        &filter,
        false,
        &mut joined,
        &mut left,
    )
    .await
    .unwrap();
    assert!(
        spy.snapshot()
            .iter()
            .any(|query| query.contains("event_receipts")),
        "positive control did not load selected room payload"
    );
    spy.assert_coverage().await;
    spy.uninstall().await;
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_sync_batches_frames_and_skips_empty_event_sections() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(filtered_batch());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn filtered_batch() {
    let (service, first_room, sender, peer) = setup().await;
    for _ in 0..11 {
        request(
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .add_header("Authorization", "Bearer alice", true)
                .json(&json!({"room_version":"11","preset":"public_chat"})),
            &service,
        )
        .await;
    }
    let rooms = data::user::joined_rooms(&sender).await.unwrap();
    assert_eq!(rooms.len(), 12);
    let initial_args = serde_json::from_value(json!({})).unwrap();
    let initial = sync_events(&sender, "D".into(), &initial_args)
        .await
        .unwrap();
    let since: BatchToken = initial.next_batch.parse().unwrap();
    // Advance the global stream without changing any joined room.
    data::next_sn().await.unwrap();
    let until = data::curr_sn().await.unwrap();
    let spy = crate::test_database::sql_spy::SqlSpy::install().await;
    let mut scalar = HashMap::new();
    for room in &rooms {
        scalar.insert(
            room.clone(),
            captured_sync_frame(room, until).await.unwrap(),
        );
    }
    let scalar_queries = spy.snapshot().len();
    spy.reset();
    let batched = captured_sync_frames(&rooms, until).await.unwrap();
    let batch_queries = spy.snapshot().len();
    assert_eq!(
        batched
            .iter()
            .map(|(room, p)| (room.clone(), p.frame_id))
            .collect::<HashMap<_, _>>(),
        scalar
    );
    assert!(
        batch_queries < scalar_queries,
        "scalar={scalar_queries} batch={batch_queries}"
    );
    spy.reset();
    let across_chunks: Vec<_> = rooms.iter().cycle().take(257).cloned().collect();
    assert_eq!(
        captured_sync_frames(&across_chunks, until).await.unwrap(),
        batched
    );
    assert_eq!(
        spy.snapshot()
            .iter()
            .filter(|q| q.contains("DISTINCT ON"))
            .count(),
        2
    );
    let filters = json!({"room":{
        "timeline":{"not_types":["*"]},"state":{"not_types":["*"]},
        "ephemeral":{"not_types":["*"]},"account_data":{"not_types":["*"]}
    }});
    let args =
        serde_json::from_value(json!({"since":initial.next_batch,"filter":filters.to_string()}))
            .unwrap();
    spy.reset();
    let response = sync_events(&sender, "D".into(), &args).await.unwrap();
    assert!(response.rooms.join.is_empty());
    let queries = spy.snapshot();
    assert!(
        !queries.iter().any(|q| q.contains("event_push_summaries")),
        "filtered unchanged rooms loaded unused notification summaries"
    );
    assert!(
        !queries
            .iter()
            .any(|q| q.contains("event_receipts") && q.contains("json_data")),
        "empty ephemeral section fetched receipt payloads"
    );
    assert!(
        queries
            .iter()
            .any(|q| q.contains("event_receipts") && !q.contains("json_data")),
        "empty ephemeral section suppressed independent notification read progress"
    );
    assert!(
        !queries.iter().any(|q| q.contains("room_typings")),
        "empty ephemeral section fetched typing"
    );
    assert!(
        !queries
            .iter()
            .any(|q| q.contains("user_datas") && q.contains(first_room.as_str())),
        "empty room account-data section loaded payload"
    );
    let frame_batches = queries
        .iter()
        .filter(|q| q.contains("DISTINCT ON") && q.contains("live_published_frame_id"))
        .count();
    assert_eq!(
        frame_batches, 2,
        "current/previous frames were not batched: {queries:?}"
    );
    spy.assert_coverage().await;
    spy.uninstall().await;

    // Equal captured bounds stay stable if later room events are published.
    let published = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{first_room}/state/m.room.topic"
        ))
        .add_header("Authorization", "Bearer alice", true)
        .json(&json!({"topic":"after snapshot"})),
        &service,
    )
    .await;
    assert_eq!(captured_sync_frames(&rooms, until).await.unwrap(), batched);
    // Partial type filters still fetch matching payloads, including independent typing.
    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{first_room}/typing/{peer}"
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({"typing":true,"timeout":30000})),
        &service,
    )
    .await;
    let filter = json!({"room":{"ephemeral":{"types":["m.typing"]}}});
    let args =
        serde_json::from_value(json!({"since":since.to_string(),"filter":filter.to_string()}))
            .unwrap();
    let response = sync_events(&sender, "D".into(), &args).await.unwrap();
    let body = serde_json::to_value(response).unwrap();
    assert!(
        body["rooms"]["join"][first_room.as_str()]["ephemeral"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "m.typing")
    );
    // Empty payloads must retain truncation and independent publication-gap metadata.
    let bound = BatchToken::new_live(data::curr_sn().await.unwrap() + 1);
    let zero: RoomEventFilter =
        serde_json::from_value(json!({"limit":0,"types":["m.room.topic"]})).unwrap();
    let truncated = load_timeline(&sender, &first_room, Some(since), Some(bound), Some(&zero))
        .await
        .unwrap();
    assert!(truncated.events.is_empty() && truncated.limited);
    let empty: RoomEventFilter = serde_json::from_value(json!({"types":[]})).unwrap();
    assert!(
        !load_timeline(&sender, &first_room, Some(since), Some(bound), Some(&empty))
            .await
            .unwrap()
            .limited
    );
    use horme::RunQueryDsl;
    diesel::sql_query("INSERT INTO timeline_gaps(room_id,event_sn,event_id) SELECT room_id,sn,id FROM events WHERE id=$1")
        .bind::<diesel::sql_types::Text,_>(published["event_id"].as_str().unwrap())
        .execute(&mut data::db()).await.unwrap();
    for filter in [
        empty.clone(),
        serde_json::from_value(json!({"not_types":["*"]})).unwrap(),
    ] {
        let gap = load_timeline(
            &sender,
            &first_room,
            Some(since),
            Some(bound),
            Some(&filter),
        )
        .await
        .unwrap();
        assert!(
            gap.events.is_empty() && gap.limited,
            "filtered gap lost discontinuity"
        );
    }
    assert!(
        !load_timeline(&sender, &first_room, Some(bound), Some(bound), Some(&empty))
            .await
            .unwrap()
            .limited,
        "gap escaped the captured window"
    );
    // Failed storage must not turn an incomplete frame set into a successful sync.
    diesel::sql_query("ALTER TABLE event_points RENAME TO review_missing_frame_points")
        .execute(&mut data::db())
        .await
        .unwrap();
    let failed = captured_sync_frames(&rooms, until).await;
    diesel::sql_query("ALTER TABLE review_missing_frame_points RENAME TO event_points")
        .execute(&mut data::db())
        .await
        .unwrap();
    assert!(failed.is_err());
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_live_promotion_and_purge_share_fence_and_atomic_tips() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(promotion_purge());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn promotion_purge() {
    use horme::RunQueryDsl;
    let (service, room, sender, _peer) = setup().await;
    let (pdu, json) = crate::event::PduBuilder {
        event_type: "m.room.message".into(),
        timestamp: Some(UnixMillis(1)),
        content: serde_json::value::to_raw_value(
            &json!({"body":"cached remote promotion","msgtype":"m.text"}),
        )
        .unwrap(),
        ..Default::default()
    }
    .hash_sign(&sender, &room, &crate::core::RoomVersionId::V11)
    .await
    .unwrap();
    let (pdu, json, guard) = crate::event::PduBuilder::save_as_outlier(pdu, json, &sender)
        .await
        .unwrap();
    drop(guard);
    // A pending outlier is not history and must survive purge before promotion.
    data::room::timeline::purge_room_history(&room, i64::MAX)
        .await
        .unwrap();
    assert!(timeline::get_pdu(&pdu.event_id).await.is_ok());
    let event_id = pdu.event_id.clone();
    let old_tips = state::get_forward_extremities(&room).await.unwrap();
    let mut control = data::checkout().await.unwrap();
    diesel::sql_query("BEGIN")
        .execute(&mut control)
        .await
        .unwrap();
    diesel::sql_query("SELECT pg_advisory_xact_lock(1346456656,2121)")
        .execute(&mut control)
        .await
        .unwrap();
    diesel::sql_query("CREATE FUNCTION pause_forward_tip() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(1346456656,2121); RETURN NEW; END $$").execute(&mut data::db()).await.unwrap();
    diesel::sql_query("CREATE TRIGGER pause_forward_tip BEFORE INSERT ON event_forward_extremities FOR EACH ROW EXECUTE FUNCTION pause_forward_tip()").execute(&mut data::db()).await.unwrap();
    let publish_room = room.clone();
    let publish = tokio::spawn(async move {
        timeline::append_pdu(&pdu, json, &room::lock_state(&publish_room).await).await
    });
    wait_advisory_waiters(1).await;
    // DELETE and INSERT of tips are one SQL transaction; the old set is visible
    // until the new set commits, even with insertion deliberately paused.
    assert_eq!(
        state::get_forward_extremities(&room).await.unwrap(),
        old_tips
    );
    let purge_room = room.clone();
    let purge = tokio::spawn(async move {
        data::room::timeline::purge_room_history(&purge_room, i64::MAX).await
    });
    wait_advisory_waiters(2).await;
    diesel::sql_query("COMMIT")
        .execute(&mut control)
        .await
        .unwrap();
    publish.await.unwrap().unwrap();
    purge.await.unwrap().unwrap();
    assert!(timeline::get_pdu(&event_id).await.is_ok());
    assert!(
        state::get_forward_extremities(&room)
            .await
            .unwrap()
            .contains(&event_id)
    );
    // The local builder already owns the fence; its final append must not try
    // to reacquire it through another connection after the purge completes.
    let sent = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/after-purge"
        ))
        .add_header("Authorization", "Bearer alice", true)
        .json(&json!({"body":"after purge","msgtype":"m.text"})),
        &service,
    )
    .await;
    let after = timeline::get_pdu(sent["event_id"].as_str().unwrap().try_into().unwrap())
        .await
        .unwrap();
    assert!(after.prev_events.contains(&event_id));
}

async fn wait_advisory_waiters(expected: i64) {
    use horme::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Waiting {
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        count: i64,
    }
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let waiting=diesel::sql_query("SELECT COUNT(*) AS count FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())").get_result::<Waiting>(&mut data::db()).await.unwrap();
            if waiting.count>=expected { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("expected publication/purge advisory waiters");
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_sync_notification_queries_follow_emitted_fields() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(notification_queries());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn notification_queries() {
    use horme::RunQueryDsl;
    let (service, room, sender, _) = setup().await;
    let root = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/summary-root"
        ))
        .add_header("Authorization", "Bearer alice", true)
        .json(&json!({"msgtype":"m.text","body":"root"})),
        &service,
    )
    .await;
    let root: OwnedEventId = root["event_id"].as_str().unwrap().try_into().unwrap();
    let initial = sync_events(
        &sender,
        "D".into(),
        &serde_json::from_value(json!({})).unwrap(),
    )
    .await
    .unwrap();
    assert!(
        initial.rooms.join[&room]
            .unread_notifications
            .notification_count
            .is_some()
    );
    let since = initial.next_batch;
    seed_notification_summaries(&sender, &room, root.as_str()).await;
    // Force a non-idle server window without activity in the joined room.
    data::next_sn().await.unwrap();
    let filter = json!({"room":{
        "timeline":{"not_types":["*"]},"state":{"not_types":["*"]},
        "ephemeral":{"not_types":["*"]},"account_data":{"not_types":["*"]}
    }});
    let args: SyncEventsReqArgs =
        serde_json::from_value(json!({"since":since,"filter":filter.to_string()})).unwrap();
    let spy = crate::test_database::sql_spy::SqlSpy::install().await;
    let omitted = sync_events(&sender, "D".into(), &args).await.unwrap();
    assert!(omitted.rooms.join.is_empty());
    let queries = spy.snapshot();
    assert!(!queries.iter().any(|q| q.contains("event_push_summaries")));
    assert!(
        queries.iter().any(|q| q.contains("event_receipts")),
        "read progress was not independently checked"
    );
    let mut thread_filter = filter.clone();
    thread_filter["room"]["timeline"]["unread_thread_notifications"] = json!(true);
    let thread_args: SyncEventsReqArgs =
        serde_json::from_value(json!({"since":since,"filter":thread_filter.to_string()})).unwrap();
    spy.reset();
    let threads = sync_events(&sender, "D".into(), &thread_args)
        .await
        .unwrap();
    let body = &threads.rooms.join[&room];
    assert_eq!(
        body.unread_thread_notifications[&root].notification_count,
        Some(2)
    );
    assert_eq!(
        body.unread_thread_notifications[&root].highlight_count,
        Some(1)
    );
    assert!(body.unread_notifications.notification_count.is_none());
    assert!(body.unread_count.is_none());
    assert_eq!(
        spy.snapshot()
            .iter()
            .filter(|q| q.contains("event_push_summaries"))
            .count(),
        1
    );
    spy.assert_coverage().await;
    spy.uninstall().await;

    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/summary-new"
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({"msgtype":"m.text","body":"new"})),
        &service,
    )
    .await;
    seed_notification_summaries(&sender, &room, root.as_str()).await;
    let visible_args: SyncEventsReqArgs = serde_json::from_value(json!({"since":since})).unwrap();
    let visible = sync_events(&sender, "D".into(), &visible_args)
        .await
        .unwrap();
    let body = &visible.rooms.join[&room];
    assert!(!body.timeline.events.is_empty());
    assert_eq!(body.unread_notifications.notification_count, Some(5));
    assert_eq!(body.unread_notifications.highlight_count, Some(2));
    assert_eq!(body.unread_count, Some(12));
    assert!(body.unread_thread_notifications.is_empty());
    // Required summary failures propagate; omitted summaries need no table access.
    diesel::sql_query("ALTER TABLE event_push_summaries RENAME TO review_missing_push_summaries")
        .execute(&mut data::db())
        .await
        .unwrap();
    let failed = sync_events(&sender, "D".into(), &visible_args).await;
    let still_omitted = sync_events(&sender, "D".into(), &args).await;
    diesel::sql_query("ALTER TABLE review_missing_push_summaries RENAME TO event_push_summaries")
        .execute(&mut data::db())
        .await
        .unwrap();
    assert!(failed.is_err());
    assert!(still_omitted.unwrap().rooms.join.is_empty());
    let recovered = sync_events(&sender, "D".into(), &visible_args)
        .await
        .unwrap();
    assert_eq!(
        recovered.rooms.join[&room]
            .unread_notifications
            .notification_count,
        Some(5)
    );
}

async fn seed_notification_summaries(user: &UserId, room: &RoomId, root: &str) {
    use diesel::sql_types::Text;
    use horme::RunQueryDsl;
    diesel::sql_query("DELETE FROM event_push_summaries WHERE user_id=$1 AND room_id=$2")
        .bind::<Text, _>(user.as_str())
        .bind::<Text, _>(room.as_str())
        .execute(&mut data::db())
        .await
        .unwrap();
    diesel::sql_query("INSERT INTO event_push_summaries(user_id,room_id,notification_count,highlight_count,unread_count,stream_ordering,thread_id) VALUES($1,$2,3,1,5,0,NULL),($1,$2,2,1,7,0,$3)")
        .bind::<Text,_>(user.as_str()).bind::<Text,_>(room.as_str()).bind::<Text,_>(root).execute(&mut data::db()).await.unwrap();
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_filtered_sync_skips_inert_rooms_without_losing_sticky_or_read_updates() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(filtered_inert_rooms());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn filtered_inert_rooms() {
    use horme::RunQueryDsl;
    let (service, room, sender, peer) = setup().await;
    let mut rooms = vec![room.clone()];
    for _ in 0..2 {
        let created = request(
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .add_header("Authorization", "Bearer alice", true)
                .json(&json!({"room_version":"11","preset":"public_chat"})),
            &service,
        )
        .await;
        rooms.push(created["room_id"].as_str().unwrap().try_into().unwrap());
    }
    let initial = sync_events(
        &sender,
        "D".into(),
        &serde_json::from_value(json!({})).unwrap(),
    )
    .await
    .unwrap();
    let filter = json!({"room":{
        "timeline":{"not_types":["*"]},"state":{"not_types":["*"]},
        "ephemeral":{"not_types":["*"]},"account_data":{"not_types":["*"]}
    }});
    let mut args: SyncEventsReqArgs =
        serde_json::from_value(json!({"since":initial.next_batch,"filter":filter.to_string()}))
            .unwrap();
    data::next_sn().await.unwrap();
    let spy = crate::test_database::sql_spy::SqlSpy::install().await;
    // A failed per-room loader must be irrelevant when no room payload or
    // independent membership/notification/sticky stream needs it.
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = Some(room.clone());
    let inert = sync_events(&sender, "D".into(), &args).await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = None;
    assert!(inert.unwrap().rooms.join.is_empty());
    let queries = spy.snapshot();
    assert_eq!(
        queries
            .iter()
            .filter(|q| q.contains("event_receipts"))
            .count(),
        1,
        "{queries:#?}"
    );
    assert_eq!(
        queries
            .iter()
            .filter(|q| q.contains("event_stickies"))
            .count(),
        1,
        "{queries:#?}"
    );
    assert!(!queries.iter().any(|q| q.contains("event_push_summaries")));
    spy.assert_coverage().await;
    spy.uninstall().await;

    let full_state: SyncEventsReqArgs = serde_json::from_value(json!({
        "since":args.since,"filter":filter.to_string(),"full_state":true
    }))
    .unwrap();
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = Some(room.clone());
    let needed = sync_events(&sender, "D".into(), &full_state).await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = None;
    assert!(
        needed.is_err(),
        "full-state loads must remain independent of the optimization"
    );

    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/m.room.member/{peer}"
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({"membership":"join","displayname":"updated"})),
        &service,
    )
    .await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = Some(room.clone());
    let membership = sync_events(&sender, "D".into(), &args).await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = None;
    assert!(
        membership.is_err(),
        "changed membership frames require their independent streams"
    );
    let recovered = sync_events(&sender, "D".into(), &args).await.unwrap();
    args.since = Some(recovered.next_batch);

    let sticky = request(TestClient::put(format!("http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/inert-sticky?org.matrix.msc4354.sticky_duration_ms=60000"))
        .add_header("Authorization","Bearer alice",true)
        .json(&json!({"msgtype":"m.text","body":"sticky"})), &service).await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = Some(room.clone());
    let needed = sync_events(&sender, "D".into(), &args).await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = None;
    assert!(
        needed.is_err(),
        "sticky delivery must survive all four event filters"
    );
    let delivered = sync_events(&sender, "D".into(), &args).await.unwrap();
    let events = &delivered.rooms.join[&room].sticky.events;
    assert_eq!(events.len(), 1);
    let event: Value = serde_json::from_str(events[0].as_str()).unwrap();
    assert_eq!(event["event_id"], sticky["event_id"]);
    assert!(delivered.rooms.join[&room].timeline.events.is_empty());
    args.since = Some(delivered.next_batch);

    let message = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/inert-read"
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({"msgtype":"m.text","body":"read me"})),
        &service,
    )
    .await;
    request(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/receipt/m.read/{}",
            message["event_id"].as_str().unwrap()
        ))
        .add_header("Authorization", "Bearer alice", true)
        .json(&json!({})),
        &service,
    )
    .await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = Some(room.clone());
    let needed = sync_events(&sender, "D".into(), &args).await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = None;
    assert!(
        needed.is_err(),
        "read progress still emits notification counters"
    );
    let delivered = sync_events(&sender, "D".into(), &args).await.unwrap();
    assert_eq!(
        delivered.rooms.join[&room]
            .unread_notifications
            .notification_count,
        Some(0)
    );

    seed_receipt_history(&sender, &room, message["event_id"].as_str().unwrap()).await;
    let expected = room::user::last_read_notification(&sender, &room)
        .await
        .unwrap();
    let other = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/inert-other-user"
        ))
        .add_header("Authorization", "Bearer alice", true)
        .json(&json!({"msgtype":"m.text","body":"other"})),
        &service,
    )
    .await;
    request(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/receipt/m.read/{}",
            other["event_id"].as_str().unwrap()
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({})),
        &service,
    )
    .await;
    let mut many = rooms.clone();
    many.extend((0..1000).map(|i| RoomId::parse(format!("!empty{i}:filtered.example")).unwrap()));
    let spy = crate::test_database::sql_spy::SqlSpy::install().await;
    let cursors = room::user::last_read_notifications(&sender, &many)
        .await
        .unwrap();
    assert_eq!(
        cursors[&room], expected,
        "another user's later receipt must not change this cursor"
    );
    assert!(!cursors.contains_key(&rooms[1]));
    assert!(
        spy.snapshot()
            .iter()
            .filter(|q| q.contains("event_receipts"))
            .count()
            <= 5,
        "receipt reads grew with room count"
    );
    spy.assert_coverage().await;
    spy.uninstall().await;
    assert_receipt_probe_plan(&sender, &rooms).await;
    diesel::sql_query(
        "ALTER TABLE event_receipts_notification_cursors RENAME TO review_receipts_unavailable",
    )
    .execute(&mut data::db())
    .await
    .unwrap();
    let failed = room::user::last_read_notifications(&sender, &rooms).await;
    diesel::sql_query(
        "ALTER TABLE review_receipts_unavailable RENAME TO event_receipts_notification_cursors",
    )
    .execute(&mut data::db())
    .await
    .unwrap();
    assert!(
        failed.is_err(),
        "failed cursor batches must not acknowledge unread work"
    );
    assert_eq!(
        room::user::last_read_notifications(&sender, &rooms)
            .await
            .unwrap()[&room],
        expected
    );

    // Reading an old event must still publish fresh notification counts. Its
    // event position predates `since`; only the receipt's publication is new.
    args.since = Some(delivered.next_batch);
    request(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/receipt/m.read/{}",
            message["event_id"].as_str().unwrap()
        ))
        .add_header("Authorization", "Bearer alice", true)
        .json(&json!({})),
        &service,
    )
    .await;
    let old_read = sync_events(&sender, "D".into(), &args).await.unwrap();
    assert_eq!(
        old_read.rooms.join[&room]
            .unread_notifications
            .notification_count,
        Some(0)
    );

    receipt_publication_fence(&sender, &room, message["event_id"].as_str().unwrap()).await;
    receipt_fence_keeps_unrelated_inboxes_independent(&sender).await;
}

async fn receipt_fence_keeps_unrelated_inboxes_independent(user: &UserId) {
    use diesel::sql_types::BigInt;
    use diesel_async::SimpleAsyncConnection as _;
    use horme::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Waiting {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    data::user::device::create_device(
        user,
        &OwnedDeviceId::from("UNRELATED"),
        "receipt-unrelated-device",
        None,
        None,
    )
    .await
    .unwrap();
    let mut gate = data::checkout().await.unwrap();
    gate.batch_execute("BEGIN").await.unwrap();
    data::user::device::lock_inbox_stream(&mut gate, user, "D".into())
        .await
        .unwrap();
    let blocked_user = user.to_owned();
    let blocked = tokio::spawn(async move {
        crate::event::sticky::curr_sn_after_sync_writes(&blocked_user, "D".into()).await
    });
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let waiting = diesel::sql_query("SELECT count(*) AS count FROM pg_locks WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND locktype='advisory' AND NOT granted")
                .get_result::<Waiting>(&mut data::db()).await.unwrap();
            if waiting.count > 0 { break; }
            assert!(!blocked.is_finished(), "the first snapshot must wait for its inbox");
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let independent = tokio::time::timeout(
        Duration::from_secs(2),
        crate::event::sticky::curr_sn_after_sync_writes(user, "UNRELATED".into()),
    )
    .await;
    gate.batch_execute("COMMIT").await.unwrap();
    drop(gate);
    tokio::time::timeout(Duration::from_secs(4), blocked)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    independent
        .expect("a slow inbox must not hold the global receipt snapshot fence")
        .unwrap();
}

async fn seed_receipt_history(user: &UserId, room: &RoomId, event: &str) {
    use diesel::sql_types::{BigInt, Text};
    use horme::RunQueryDsl;
    let event = EventId::parse(event).unwrap();
    let event_sn = crate::event::get_event_sn(&event).await.unwrap();
    diesel::sql_query("INSERT INTO event_receipts (sn, ty, room_id, user_id, event_id, event_sn, thread_id, json_data, receipt_at) SELECT nextval('occur_sn_seq'), 'm.read.private', $2, $1, $3, $4, NULL, '{}'::jsonb, 0 FROM generate_series(1, 30000)")
        .bind::<Text,_>(user.as_str()).bind::<Text,_>(room.as_str())
        .bind::<Text,_>(event.as_str()).bind::<BigInt,_>(event_sn)
        .execute(&mut data::db()).await.unwrap();
    diesel::sql_query("ANALYZE event_receipts")
        .execute(&mut data::db())
        .await
        .unwrap();
    let mut row = data::room::receipt::last_private_read_receipt(user, room)
        .await
        .unwrap()
        .unwrap();
    let older = data::next_sn().await.unwrap();
    let newer = data::next_sn().await.unwrap();
    for sn in [newer, older] {
        row.sn = sn;
        diesel::insert_into(data::schema::event_receipts::table)
            .values(&row)
            .execute(&mut data::db())
            .await
            .unwrap();
    }
    assert_eq!(
        room::user::last_read_notification(user, room)
            .await
            .unwrap(),
        newer
    );
}

async fn assert_receipt_probe_plan(user: &UserId, rooms: &[OwnedRoomId]) {
    use diesel::sql_types::{Array, Json, Text};
    use horme::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Plan {
        #[diesel(sql_type = Json, column_name = "QUERY PLAN")]
        value: Value,
    }
    let explain = diesel::sql_query(format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {}",
        room::user::NOTIFICATION_CURSOR_QUERY
    ))
    .bind::<Text, _>(user.as_str())
    .bind::<Array<Text>, _>(rooms.iter().map(|room| room.as_str()).collect::<Vec<_>>())
    .get_result::<Plan>(&mut data::db())
    .await
    .unwrap();
    fn probe(node: &Value) -> Option<&Value> {
        assert_ne!(
            node["Relation Name"], "event_receipts",
            "sync scanned the receipt history"
        );
        if node["Relation Name"] == "event_receipts_notification_cursors" {
            return Some(node);
        }
        node.get("Plans")?.as_array()?.iter().find_map(probe)
    }
    let index = probe(&explain.value[0]["Plan"]).expect("sync must read the watermark projection");
    assert!(
        index["Actual Rows"].as_f64().unwrap() <= rooms.len() as f64,
        "{index}"
    );
}

async fn receipt_publication_fence(user: &UserId, room: &RoomId, event: &str) {
    use diesel::sql_types::BigInt;
    use horme::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Waiting {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    const LOCK: i64 = 0x50414c52435054;
    let event = EventId::parse(event).unwrap();
    let event_sn = crate::event::get_event_sn(&event).await.unwrap();
    for rollback in [false, true] {
        let before = room::user::last_read_notification(user, room)
            .await
            .unwrap();
        let receipt = data::room::DbReceipt {
            sn: 0,
            ty: "m.read.private".into(),
            room_id: room.to_owned(),
            user_id: user.to_owned(),
            event_id: event.clone(),
            event_sn,
            thread_id: None,
            json_data: json!({}),
            receipt_at: UnixMillis::now(),
        };
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let writer = tokio::spawn(async move {
            data::db()
                .transaction::<_, data::DataError, _>(async |conn| {
                    data::room::receipt::publish_receipt_with_conn(conn, &receipt).await?;
                    let sn = diesel::select(diesel::dsl::sql::<BigInt>("currval('occur_sn_seq')"))
                        .get_result::<Seqnum>(conn)
                        .await?;
                    ready_tx.send(sn).unwrap();
                    release_rx.await.unwrap();
                    if rollback {
                        Err(data::DataError::internal("intentional receipt rollback"))
                    } else {
                        Ok(())
                    }
                })
                .await
        });
        let published = tokio::time::timeout(Duration::from_secs(4), ready_rx)
            .await
            .unwrap()
            .unwrap();
        let snapshot_user = user.to_owned();
        let snapshot = tokio::spawn(async move {
            crate::event::sticky::curr_sn_after_sync_writes(&snapshot_user, "D".into())
                .await
                .unwrap()
        });
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let waiting = diesel::sql_query("SELECT count(*) AS count FROM pg_locks WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND locktype='advisory' AND NOT granted AND mode='ExclusiveLock' AND objsubid=1 AND classid::bigint=$1 AND objid::bigint=$2")
                    .bind::<BigInt,_>(LOCK >> 32).bind::<BigInt,_>(LOCK & 0xffff_ffff)
                    .get_result::<Waiting>(&mut data::db()).await.unwrap();
                if waiting.count > 0 { break; }
                assert!(!snapshot.is_finished(), "snapshot crossed an uncommitted receipt");
                tokio::task::yield_now().await;
            }
        }).await.expect("sync cursor must wait for receipt publication");
        assert!(!snapshot.is_finished());
        release_tx.send(()).unwrap();
        assert_eq!(writer.await.unwrap().is_err(), rollback);
        let cursor = snapshot.await.unwrap();
        assert!(cursor >= published);
        let committed = room::user::last_read_notifications(user, &[room.to_owned()])
            .await
            .unwrap()[room];
        assert_eq!(committed, if rollback { before } else { published });
    }
}
