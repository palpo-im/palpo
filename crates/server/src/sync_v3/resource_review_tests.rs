use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

async fn request(builder: RequestBuilder, service: &Service) -> Value {
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
async fn setup() -> (Service, OwnedRoomId, OwnedUserId, OwnedUserId) {
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
    assert_eq!(batched, scalar);
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
