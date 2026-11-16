use salvo::test::TestClient;
use serde_json::{Value, json};

use super::resource_review_tests::{request, setup};
use super::*;

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_incremental_sync_skips_quiet_rooms_and_delivers_independent_updates() {
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

async fn sync_needed(sender: &UserId, room: &RoomId, args: &mut SyncEventsReqArgs) -> Value {
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = Some(room.to_owned());
    let failed = sync_events(sender, "D".into(), args).await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = None;
    assert!(failed.is_err(), "changed room was incorrectly skipped");
    let response = sync_events(sender, "D".into(), args).await.unwrap();
    args.since = Some(response.next_batch.clone());
    serde_json::to_value(response).unwrap()["rooms"]["join"][room.as_str()].clone()
}

async fn sync_quiet(sender: &UserId, room: &RoomId, args: &mut SyncEventsReqArgs) {
    data::next_sn().await.unwrap();
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = Some(room.to_owned());
    let response = sync_events(sender, "D".into(), args).await;
    *joined_room_failure_tests::FAIL_ROOM.lock().unwrap() = None;
    let response = response.expect("quiet room entered its payload loader");
    assert!(response.rooms.join.is_empty());
    args.since = Some(response.next_batch);
}

async fn run() {
    use horme::RunQueryDsl;
    let (service, room, sender, peer) = setup().await;
    for _ in 0..2 {
        request(
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .add_header("Authorization", "Bearer alice", true)
                .json(&json!({"room_version":"11","preset":"public_chat"})),
            &service,
        )
        .await;
    }
    let initial = sync_events(
        &sender,
        "D".into(),
        &serde_json::from_value(json!({})).unwrap(),
    )
    .await
    .unwrap();
    let mut args: SyncEventsReqArgs =
        serde_json::from_value(json!({"since":initial.next_batch})).unwrap();
    let spy = crate::test_database::sql_spy::SqlSpy::install().await;
    sync_quiet(&sender, &room, &mut args).await;
    assert!(
        !spy.snapshot()
            .iter()
            .any(|q| q.contains("event_push_summaries")),
        "ordinary quiet sync loaded per-room notification payloads"
    );
    spy.assert_coverage().await;
    spy.uninstall().await;

    let message = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/activity"
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({"msgtype":"m.text","body":"new live publication, unchanged state frame"})),
        &service,
    )
    .await;
    let event_id = message["event_id"].as_str().unwrap();
    let delivered = sync_needed(&sender, &room, &mut args).await;
    assert!(
        delivered["timeline"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["event_id"] == event_id)
    );
    sync_quiet(&sender, &room, &mut args).await;

    // A read of an old message is new publication activity for its owner.
    request(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/receipt/m.read.private/{event_id}"
        ))
        .add_header("Authorization", "Bearer alice", true)
        .json(&json!({})),
        &service,
    )
    .await;
    let delivered = sync_needed(&sender, &room, &mut args).await;
    assert_eq!(delivered["unread_notifications"]["notification_count"], 0);
    assert!(
        delivered["ephemeral"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["content"][event_id]["m.read.private"][sender.as_str()].is_object())
    );

    // Someone else's private receipt cannot trigger a room payload or leak.
    request(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/receipt/m.read.private/{event_id}"
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({})),
        &service,
    )
    .await;
    sync_quiet(&sender, &room, &mut args).await;
    request(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/receipt/m.read/{event_id}"
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({})),
        &service,
    )
    .await;
    let delivered = sync_needed(&sender, &room, &mut args).await;
    assert!(
        delivered["ephemeral"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["content"][event_id]["m.read"][peer.as_str()].is_object())
    );

    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/typing/{peer}"
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({"typing":true,"timeout":60000})),
        &service,
    )
    .await;
    let delivered = sync_needed(&sender, &room, &mut args).await;
    assert!(
        delivered["ephemeral"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "m.typing" && e["content"]["user_ids"] == json!([peer]))
    );
    diesel::sql_query("UPDATE room_typings SET timeout_at=1 WHERE room_id=$1")
        .bind::<diesel::sql_types::Text, _>(room.as_str())
        .execute(&mut data::db())
        .await
        .unwrap();
    room::typing::publish_expired_typings().await.unwrap();
    let delivered = sync_needed(&sender, &room, &mut args).await;
    assert!(
        delivered["ephemeral"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "m.typing" && e["content"]["user_ids"] == json!([]))
    );

    request(TestClient::put(format!("http://localhost/_matrix/client/v3/user/{sender}/rooms/{room}/account_data/com.example.activity"))
        .add_header("Authorization", "Bearer alice", true).json(&json!({"value":1})), &service).await;
    let delivered = sync_needed(&sender, &room, &mut args).await;
    assert!(
        delivered["account_data"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "com.example.activity" && e["content"]["value"] == 1)
    );
    data::user::delete_room_data(&sender, &room, "com.example.activity")
        .await
        .unwrap();
    let delivered = sync_needed(&sender, &room, &mut args).await;
    assert!(
        delivered["account_data"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "com.example.activity" && e["content"] == json!({}))
    );

    // Default readers are unbounded for auxiliary streams, including an update
    // beyond the captured timeline bound. Batch across more than 1,000 rooms.
    let before = data::curr_sn().await.unwrap() + 1;
    request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/typing/{peer}"
        ))
        .add_header("Authorization", "Bearer bob", true)
        .json(&json!({"typing":true,"timeout":60000})),
        &service,
    )
    .await;
    let mut rooms = vec![room.clone()];
    rooms.extend(
        (0..1000).map(|i| RoomId::parse(format!("!activity{i}:filtered.example")).unwrap()),
    );
    let spy = crate::test_database::sql_spy::SqlSpy::install().await;
    assert_eq!(
        changed_auxiliary_rooms(&sender, &rooms, &RoomDataFilter::default(), before)
            .await
            .unwrap(),
        HashSet::from([room.clone()])
    );
    let queries = spy.snapshot();
    let activity_queries = queries
        .iter()
        .filter(|q| {
            q.contains("event_receipts") || q.contains("room_typings") || q.contains("user_datas")
        })
        .count();
    // Pool checkout health probes are independent of the activity batches.
    assert!(
        activity_queries > 0 && activity_queries <= 3 * rooms.len().div_ceil(256),
        "{queries:#?}"
    );
    spy.assert_coverage().await;
    spy.uninstall().await;
    diesel::sql_query("ALTER TABLE room_typings RENAME TO review_missing_activity_typings")
        .execute(&mut data::db())
        .await
        .unwrap();
    let failed = sync_events(&sender, "D".into(), &args).await;
    diesel::sql_query("ALTER TABLE review_missing_activity_typings RENAME TO room_typings")
        .execute(&mut data::db())
        .await
        .unwrap();
    assert!(
        failed.is_err(),
        "activity lookup failure acknowledged an incomplete sync"
    );
    sync_needed(&sender, &room, &mut args).await;

    // Zero limits and empty type filters can still deliver discontinuities.
    for timeline_filter in [json!({"limit":0}), json!({"types":[]})] {
        let message = request(
            TestClient::put(format!(
                "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/gap-{}",
                data::curr_sn().await.unwrap()
            ))
            .add_header("Authorization", "Bearer bob", true)
            .json(&json!({"msgtype":"m.text","body":"gap"})),
            &service,
        )
        .await;
        diesel::sql_query("INSERT INTO timeline_gaps(room_id,event_sn,event_id) SELECT room_id,sn,id FROM events WHERE id=$1")
            .bind::<diesel::sql_types::Text,_>(message["event_id"].as_str().unwrap()).execute(&mut data::db()).await.unwrap();
        args.filter = Some(Filter::FilterDefinition(
            serde_json::from_value(json!({"room":{"timeline":timeline_filter}})).unwrap(),
        ));
        let delivered = sync_needed(&sender, &room, &mut args).await;
        assert_eq!(delivered["timeline"]["limited"], true);
        assert!(
            delivered["timeline"]["events"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    args.filter = Some(Filter::FilterDefinition(
        serde_json::from_value(json!({"room":{"timeline":{"unread_thread_notifications":true}}}))
            .unwrap(),
    ));
    sync_needed(&sender, &room, &mut args).await;
}
