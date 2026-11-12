//! Run each case alone with an empty PALPO_TEST_DATABASE_URL (CI exact-name list).
use crate::{
    core::{RoomId, UserId},
    data,
};
use horme::RunQueryDsl;

#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}
async fn count(table: &str) -> i64 {
    diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
        .get_result::<Count>(&mut data::db())
        .await
        .unwrap()
        .count
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_purge_preserves_tips_and_resumes_committed_batches() {
    crate::test_database::init();
    let room: &RoomId = "!purge:example.org".try_into().unwrap();
    diesel::sql_query("INSERT INTO events (id,sn,ty,room_id,topological_ordering,stream_ordering,origin_server_ts,contains_url,is_outlier) SELECT '$purge-'||n,n,'m.room.message','!purge:example.org',n,n,1,false,false FROM generate_series(1,1003) n")
        .execute(&mut data::db()).await.unwrap();
    diesel::sql_query("UPDATE events SET state_key='' WHERE sn=1003")
        .execute(&mut data::db())
        .await
        .unwrap();
    diesel::sql_query("INSERT INTO event_forward_extremities(room_id,event_id) VALUES ('!purge:example.org','$purge-1001'),('!purge:example.org','$purge-1002')").execute(&mut data::db()).await.unwrap();
    // Fail the second batch: the first is durable and the second rolls back.
    diesel::sql_query("CREATE FUNCTION fail_purge_batch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.sn=600 THEN RAISE EXCEPTION 'injected purge failure'; END IF; RETURN OLD; END $$")
        .execute(&mut data::db()).await.unwrap();
    diesel::sql_query("CREATE TRIGGER fail_purge_batch BEFORE DELETE ON events FOR EACH ROW EXECUTE FUNCTION fail_purge_batch()")
        .execute(&mut data::db()).await.unwrap();
    assert!(
        data::room::timeline::purge_room_history(room, i64::MAX)
            .await
            .is_err()
    );
    assert_eq!(count("events").await, 503);
    assert_eq!(count("event_forward_extremities").await, 2);
    diesel::sql_query("DROP TRIGGER fail_purge_batch ON events")
        .execute(&mut data::db())
        .await
        .unwrap();
    assert_eq!(
        data::room::timeline::purge_room_history(room, i64::MAX)
            .await
            .unwrap(),
        500
    );
    assert_eq!(count("events").await, 3);
    assert_eq!(count("event_forward_extremities").await, 2);
    assert_eq!(
        data::room::timeline::purge_room_history(room, i64::MAX)
            .await
            .unwrap(),
        0
    );
    assert!(
        data::room::timeline::purge_room_history(room, -1)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_receipt_latest_projection_retains_log_and_threads() {
    crate::test_database::init();
    let room: &RoomId = "!receipt:example.org".try_into().unwrap();
    let alice: &UserId = "@alice:example.org".try_into().unwrap();
    let bob: &UserId = "@bob:example.org".try_into().unwrap();
    diesel::sql_query("INSERT INTO event_receipts(sn,ty,room_id,user_id,event_id,event_sn,json_data,receipt_at) SELECT n,'m.read','!receipt:example.org','@alice:example.org','$event-'||n,n,json_build_object('ts',n),n FROM generate_series(1,1000) n")
        .execute(&mut data::db()).await.unwrap();
    diesel::sql_query(r#"INSERT INTO event_receipts(sn,ty,room_id,user_id,event_id,event_sn,json_data,receipt_at) VALUES
        (1001,'m.read','!receipt:example.org','@alice:example.org','$event-1000',1000,'{"ts":1001}',1001),
        (1002,'m.read','!receipt:example.org','@alice:example.org','$main',1000,'{"ts":1002,"thread_id":"main"}',1002),
        (1003,'m.read','!receipt:example.org','@alice:example.org','$thread',1000,'{"ts":1003,"thread_id":"$root"}',1003),
        (1004,'m.read.private','!receipt:example.org','@alice:example.org','$private',1000,'{"ts":1004}',1004)"#)
        .execute(&mut data::db()).await.unwrap();
    let public = data::room::receipt::read_receipts(room, 0, Some(bob))
        .await
        .unwrap();
    let json = serde_json::to_value(&public[alice]).unwrap();
    assert_eq!(
        json.as_object().unwrap().len(),
        3,
        "one latest value per public thread key"
    );
    assert_eq!(json["$event-1000"]["m.read"][alice.as_str()]["ts"], 1001);
    assert_eq!(json["$main"]["m.read"][alice.as_str()]["thread_id"], "main");
    assert_eq!(
        json["$thread"]["m.read"][alice.as_str()]["thread_id"],
        "$root"
    );
    assert!(json.get("$private").is_none());
    let own = data::room::receipt::read_receipts(room, 0, Some(alice))
        .await
        .unwrap();
    assert!(
        own[alice]
            .0
            .keys()
            .any(|event| event.as_str() == "$private")
    );
    assert!(
        data::room::receipt::read_receipts(room, 1005, Some(alice))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        count("event_receipts").await,
        1004,
        "projection must not consume federation history"
    );
}
