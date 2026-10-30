use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedRoomId, RoomId, RoomVersionId, UnixMillis, UserId};
use crate::data::schema::{event_backward_extremities, events, timeline_gaps};
use crate::data::{self, connect};
use crate::event::SnPduEvent;
use crate::room::{self, timeline};
use crate::{AppError, PduBuilder, config};

async fn send(request: RequestBuilder, service: &Service) -> Response {
    let router = service.router();
    tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap()
}

async fn ok(mut response: Response) -> Value {
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{value}");
    value
}

async fn create_room(service: &Service) -> OwnedRoomId {
    let body = ok(send(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", "Bearer timestamp-gap-token", true)
            .json(&json!({"room_version":"11", "preset":"public_chat"})),
        service,
    )
    .await)
    .await;
    RoomId::parse(body["room_id"].as_str().unwrap()).unwrap()
}

async fn append(room_id: &RoomId, timestamp: UnixMillis, body: &str) -> SnPduEvent {
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: "m.room.message".into(),
            content: serde_json::value::to_raw_value(&json!({"msgtype":"m.text", "body":body}))
                .unwrap(),
            timestamp: Some(timestamp),
            ..Default::default()
        },
        &UserId::parse("@alice:gap.example").unwrap(),
        room_id,
        &RoomVersionId::V11,
        &room::lock_state(room_id).await,
    )
    .await
    .unwrap()
}

async fn gap_count(event: &SnPduEvent) -> i64 {
    timeline_gaps::table
        .filter(timeline_gaps::room_id.eq(&event.room_id))
        .filter(timeline_gaps::event_sn.eq(event.event_sn))
        .count()
        .get_result::<i64>(&mut connect().await.unwrap())
        .await
        .unwrap()
}

async fn timestamp(service: &Service, room_id: &RoomId, ts: UnixMillis, dir: &str) -> Value {
    ok(send(
        TestClient::get(format!(
            "http://localhost/_matrix/client/v1/rooms/{room_id}/timestamp_to_event?ts={}&dir={dir}",
            ts.0,
        ))
        .add_header("Authorization", "Bearer timestamp-gap-token", true),
        service,
    )
    .await)
    .await
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn timestamp_gap_http_recovery_is_idempotent() {
    std::thread::Builder::new()
        .name("timestamp-gap-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(recovery())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn recovery() {
    crate::test_database::init();
    config::CONFIG.set(serde_json::from_value(json!({
        "server_name":"gap.example", "db":{"url":"unused-test-config"}, "ip_range_denylist":[]
    })).unwrap()).unwrap();
    let user_id = UserId::parse("@alice:gap.example").unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user_id.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "alice".into(),
        server_name: user_id.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::device::create_device(
        &user_id,
        &OwnedDeviceId::from("D"),
        "timestamp-gap-token",
        None,
        None,
    )
    .await
    .unwrap();
    let service = Service::new(crate::routing::root());
    let before_creation = UnixMillis(UnixMillis::now().0 - 1_000);
    let room_id = create_room(&service).await;
    let old_ts = UnixMillis(1_641_168_000_000);
    let recent_ts = UnixMillis(UnixMillis::now().0 + 10_000);
    // The production builder hashes/signs the explicit old timestamp just like
    // an application-service import, preserving a genuine event for backfill.
    let old = append(&room_id, old_ts, "imported before room creation").await;
    let latest = append(&room_id, recent_ts, "after the imported event").await;
    let other_room = create_room(&service).await;
    let other_old = append(&other_room, recent_ts, "other missing history").await;
    let other_latest = append(
        &other_room,
        UnixMillis(recent_ts.0 + 1),
        "other gap remains real",
    )
    .await;
    for predecessor in [&old, &other_old] {
        // A joining server can have the signed predecessor only as an outlier.
        diesel::update(events::table.find(&predecessor.event_id))
            .set(events::is_outlier.eq(true))
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();
    }
    super::update_backward_extremities(&latest).await.unwrap();
    super::update_backward_extremities(&other_latest)
        .await
        .unwrap();
    assert_eq!(gap_count(&latest).await, 1);
    // This exact second call failed with timeline_gaps_room_id_event_sn before
    // the fix: no concurrent scheduling is needed to reproduce the retry bug.
    super::update_backward_extremities(&latest).await.unwrap();
    let (one, two, three, four) = tokio::join!(
        super::update_backward_extremities(&latest),
        super::update_backward_extremities(&latest),
        super::update_backward_extremities(&latest),
        super::update_backward_extremities(&latest),
    );
    for result in [one, two, three, four] {
        result.unwrap();
    }
    assert_eq!(gap_count(&latest).await, 1);
    assert_eq!(gap_count(&other_latest).await, 1);
    assert_eq!(
        timeline_gaps::table
            .filter(timeline_gaps::room_id.eq(&latest.room_id))
            .filter(timeline_gaps::event_sn.eq(latest.event_sn))
            .select(timeline_gaps::event_id)
            .first::<crate::core::OwnedEventId>(&mut connect().await.unwrap())
            .await
            .unwrap(),
        latest.event_id
    );

    // Exercise real signed backfill loading and concurrent promotion rather
    // than just clearing the outlier flag by hand. The row lock permits one
    // promotion; repeated backfill and gap maintenance must remain idempotent.
    let mut wire = timeline::get_pdu_json(&old.event_id)
        .await
        .unwrap()
        .unwrap();
    wire.remove("event_id");
    let raw = serde_json::value::to_raw_value(&wire).unwrap();
    let (backfill, json) = timeline::backfill_pdu(
        user_id.server_name(),
        &room_id,
        &RoomVersionId::V11,
        raw.clone(),
    )
    .await
    .unwrap();
    let (one, two, three) = tokio::join!(
        timeline::append_backfilled_pdu(&backfill, json.clone()),
        timeline::append_backfilled_pdu(&backfill, json.clone()),
        timeline::append_backfilled_pdu(&backfill, json),
    );
    for result in [one, two, three] {
        result.unwrap();
    }
    assert!(!timeline::get_pdu(&old.event_id).await.unwrap().is_outlier);
    super::update_backward_extremities(&latest).await.unwrap();
    assert_eq!(gap_count(&latest).await, 0);
    assert_eq!(
        gap_count(&other_latest).await,
        1,
        "unrelated real gaps must survive recovery"
    );
    assert!(
        timeline::is_event_next_to_backward_gap(&other_latest)
            .await
            .unwrap()
    );
    assert_eq!(
        event_backward_extremities::table
            .filter(event_backward_extremities::room_id.eq(&latest.room_id))
            .filter(event_backward_extremities::event_id.eq(&latest.event_id))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap(),
        0
    );
    let (again, json) =
        timeline::backfill_pdu(user_id.server_name(), &room_id, &RoomVersionId::V11, raw)
            .await
            .unwrap();
    timeline::append_backfilled_pdu(&again, json).await.unwrap();
    super::update_backward_extremities(&latest).await.unwrap();
    assert_eq!(gap_count(&latest).await, 0);
    assert_eq!(gap_count(&other_latest).await, 1);

    // Preserve both originally failing timestamp directions and the imported
    // event's actual timestamp, including queries before room creation.
    for (query, dir) in [(UnixMillis(old_ts.0 - 1), "f"), (before_creation, "b")] {
        let found = timestamp(&service, &room_id, query, dir).await;
        assert_eq!(found["event_id"], old.event_id.as_str());
        assert_eq!(found["origin_server_ts"], old_ts.0);
    }
    assert_eq!(
        timestamp(&service, &room_id, recent_ts, "b").await["event_id"],
        latest.event_id.as_str()
    );
    let missing = send(
        TestClient::get(format!(
            "http://localhost/_matrix/client/v1/rooms/{room_id}/timestamp_to_event?ts={}&dir=f",
            recent_ts.0 + 1,
        ))
        .add_header("Authorization", "Bearer timestamp-gap-token", true),
        &service,
    )
    .await;
    assert_eq!(missing.status_code, Some(StatusCode::NOT_FOUND));

    // Only the documented idempotency key is ignored. Other PostgreSQL errors
    // must still reach the caller instead of being swallowed as a retry.
    diesel::sql_query("ALTER TABLE timeline_gaps ADD CONSTRAINT gap_regression_error CHECK (event_id <> '$gap-propagated-db-error')")
        .execute(&mut connect().await.unwrap()).await.unwrap();
    let mut invalid = latest.clone();
    invalid.pdu.event_id = "$gap-propagated-db-error".try_into().unwrap();
    invalid.event_sn = 9_000_000_000;
    invalid.pdu.prev_events = vec!["$genuinely-unknown-parent".try_into().unwrap()];
    let result = super::update_backward_extremities(&invalid).await;
    assert!(matches!(
        result,
        Err(AppError::Diesel(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::CheckViolation,
            _
        )))
    ));
    diesel::sql_query("ALTER TABLE timeline_gaps DROP CONSTRAINT gap_regression_error")
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();
}
