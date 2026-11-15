use std::time::Duration;

use diesel::sql_types::{BigInt, Text};
use horme::RunQueryDsl;
use serde_json::json;

use super::*;
use crate::core::UnixMillis;

const PAUSE_LOCK: i64 = 7_151_324;

async fn event(room: &RoomId, user: &UserId, label: &str) -> (OwnedEventId, i64) {
    let sn = crate::data::next_sn().await.unwrap();
    let id = EventId::parse(format!("$push-count-{label}-{sn}:counter.example")).unwrap();
    diesel::sql_query(
        "WITH stored AS (INSERT INTO events \
         (id, sn, ty, room_id, sender_id, depth, topological_ordering, stream_ordering, origin_server_ts, contains_url, is_outlier) \
         VALUES ($1, $2, 'm.room.message', $3, $4, $2, $2, $2, 1700000000000, false, false)) \
         INSERT INTO event_points (event_id, event_sn, room_id) VALUES ($1, $2, $3)",
    )
    .bind::<Text, _>(id.as_str())
    .bind::<BigInt, _>(sn)
    .bind::<Text, _>(room.as_str())
    .bind::<Text, _>(user.as_str())
    .execute(&mut crate::data::db()).await.unwrap();
    (id, sn)
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

async fn wait_for_waiters(expected: i64) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count = diesel::sql_query(
                "SELECT COUNT(*) AS count FROM pg_locks WHERE locktype = 'advisory' \
                 AND NOT granted AND database = (SELECT oid FROM pg_database WHERE datname = current_database())",
            ).get_result::<Count>(&mut crate::data::db()).await.unwrap().count;
            if count >= expected { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("both action and receipt must reach the controlled lock boundary");
}

async fn review_counts() {
    crate::test_database::init();
    crate::config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"counter.example", "db":{"url":"unused"},
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
            }))
            .unwrap(),
        )
        .unwrap();
    let user = UserId::parse("@reader:counter.example").unwrap();
    crate::data::user::create_user(&crate::data::user::NewDbUser {
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
    let room = RoomId::parse("!push-count:counter.example").unwrap();
    diesel::insert_into(rooms::table)
        .values(&crate::data::room::NewDbRoom {
            id: room.clone(),
            version: "11".into(),
            is_public: false,
            min_depth: 0,
            has_auth_chain_index: false,
            created_at: UnixMillis::now(),
        })
        .execute(&mut crate::data::db())
        .await
        .unwrap();
    let (old, old_sn) = event(&room, &user, "old").await;
    let (new, new_sn) = event(&room, &user, "new").await;
    upsert_push_action(&room, &old, &user, true, true)
        .await
        .unwrap();
    upsert_push_action(&room, &old, &user, true, true)
        .await
        .unwrap();
    let summary = crate::room::user::notify_summary(&user, &room)
        .await
        .unwrap();
    assert_eq!(
        (summary.notification_count, summary.highlight_count),
        (1, 1),
        "retry must count an action once"
    );

    // Pause the real summary INSERT while the action writer owns its summary
    // transaction lock. A receipt must wait rather than observe its half-write.
    let mut pause = <diesel::PgConnection as diesel::Connection>::establish(
        &std::env::var("PALPO_TEST_DATABASE_URL").unwrap(),
    )
    .unwrap();
    diesel::RunQueryDsl::execute(
        diesel::sql_query(format!("SELECT pg_advisory_lock({PAUSE_LOCK})")),
        &mut pause,
    )
    .unwrap();
    diesel::sql_query(format!(
        "CREATE FUNCTION review_pause_push_count() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.stream_ordering = {new_sn} THEN PERFORM pg_advisory_xact_lock({PAUSE_LOCK}); END IF; \
         RETURN NEW; END $$"
    )).execute(&mut crate::data::db()).await.unwrap();
    diesel::sql_query(
        "CREATE TRIGGER review_pause_push_count BEFORE INSERT OR UPDATE ON event_push_summaries \
         FOR EACH ROW EXECUTE FUNCTION review_pause_push_count()",
    )
    .execute(&mut crate::data::db())
    .await
    .unwrap();
    let (writer_room, writer_user, writer_event) = (room.clone(), user.clone(), new.clone());
    let writer = tokio::spawn(async move {
        upsert_push_action(&writer_room, &writer_event, &writer_user, true, true).await
    });
    wait_for_waiters(1).await;
    let (reader_room, reader_user, reader_event) = (room.clone(), user.clone(), old.clone());
    let receipt = tokio::spawn(async move {
        let receipt = DbReceipt {
            sn: crate::data::next_sn().await?,
            ty: "m.read.private".into(),
            room_id: reader_room,
            user_id: reader_user,
            event_id: reader_event,
            event_sn: old_sn,
            thread_id: None,
            json_data: serde_json::to_value(crate::core::events::receipt::Receipt::new(
                UnixMillis::now(),
            ))?,
            receipt_at: UnixMillis::now(),
        };
        record_read_and_clear(&receipt, &ReceiptThread::Unthreaded).await
    });
    wait_for_waiters(2).await;
    assert!(!writer.is_finished() && !receipt.is_finished());
    diesel::RunQueryDsl::execute(
        diesel::sql_query(format!("SELECT pg_advisory_unlock({PAUSE_LOCK})")),
        &mut pause,
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), writer)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), receipt)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    for sql in [
        "DROP TRIGGER review_pause_push_count ON event_push_summaries",
        "DROP FUNCTION review_pause_push_count()",
    ] {
        diesel::sql_query(sql)
            .execute(&mut crate::data::db())
            .await
            .unwrap();
    }
    let summary = crate::room::user::notify_summary(&user, &room)
        .await
        .unwrap();
    assert_eq!(
        (summary.notification_count, summary.highlight_count),
        (1, 1),
        "one actual unread action must have one notification"
    );
    let ids = event_push_actions::table
        .filter(event_push_actions::user_id.eq(&user))
        .select(event_push_actions::event_id)
        .load::<OwnedEventId>(&mut crate::data::db())
        .await
        .unwrap();
    assert_eq!(ids, vec![new.clone()]);
    upsert_push_action(&room, &old, &user, true, true)
        .await
        .unwrap();
    upsert_push_action(&room, &new, &user, true, true)
        .await
        .unwrap();
    refresh_notify_summary(&user, &room).await.unwrap();
    assert_eq!(
        crate::room::user::notify_summary(&user, &room)
            .await
            .unwrap()
            .notification_count,
        1,
        "retries may not resurrect read events"
    );

    // A failed counter write must roll back the action too, so retrying can
    // neither silently lose a counter nor double-count an existing action.
    let (failed, failed_sn) = event(&room, &user, "rollback").await;
    diesel::sql_query(format!(
        "CREATE FUNCTION review_fail_push_count() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.stream_ordering = {failed_sn} THEN RAISE EXCEPTION 'review counter fault'; END IF; RETURN NEW; END $$"
    )).execute(&mut crate::data::db()).await.unwrap();
    diesel::sql_query("CREATE TRIGGER review_fail_push_count BEFORE INSERT OR UPDATE ON event_push_summaries FOR EACH ROW EXECUTE FUNCTION review_fail_push_count()")
        .execute(&mut crate::data::db()).await.unwrap();
    assert!(
        upsert_push_action(&room, &failed, &user, true, true)
            .await
            .is_err()
    );
    assert_eq!(
        event_push_actions::table
            .filter(event_push_actions::event_id.eq(&failed))
            .count()
            .get_result::<i64>(&mut crate::data::db())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        crate::room::user::notify_summary(&user, &room)
            .await
            .unwrap()
            .notification_count,
        1
    );
    for sql in [
        "DROP TRIGGER review_fail_push_count ON event_push_summaries",
        "DROP FUNCTION review_fail_push_count()",
    ] {
        diesel::sql_query(sql)
            .execute(&mut crate::data::db())
            .await
            .unwrap();
    }
    upsert_push_action(&room, &failed, &user, true, true)
        .await
        .unwrap();
    assert_eq!(
        crate::room::user::notify_summary(&user, &room)
            .await
            .unwrap()
            .notification_count,
        2
    );
}

#[test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
fn review_push_actions_are_atomic_idempotent_and_receipt_serialized() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(review_counts());
        })
        .unwrap()
        .join()
        .unwrap();
}
