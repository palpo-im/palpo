use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use super::OutlierPdu;
use crate::core::federation::event::MissingEventsResBody;
use crate::core::serde::to_canonical_object;
use crate::core::state::Event;
use crate::core::{
    OwnedDeviceId, OwnedEventId, OwnedRoomId, RoomId, RoomVersionId, UnixMillis, UserId,
};
use crate::data::schema::{event_datas, events};
use crate::data::{self, connect};
use crate::event::PduEvent;
use crate::event::fetching::{
    MissingEventsRecovery, fetch_and_process_missing_events, process_missing_events_response,
};
use crate::{AppError, PduBuilder, config};

const REMOTE: &str = "127.0.0.1:9";

fn outlier(pdu: PduEvent) -> OutlierPdu {
    OutlierPdu {
        json_data: to_canonical_object(&pdu).unwrap(),
        room_id: pdu.room_id.clone(),
        pdu,
        soft_failed: true,
        policy_refused: false,
        remote_server: REMOTE.try_into().unwrap(),
        room_version: RoomVersionId::V11,
        event_sn: None,
    }
}

fn child(template: &PduEvent, id: &str, prev: &[OwnedEventId]) -> OutlierPdu {
    let mut pdu = template.clone();
    pdu.event_id = id.try_into().unwrap();
    pdu.prev_events = prev.to_vec();
    outlier(pdu)
}

async fn pending(room_id: &RoomId, body: &str) -> (PduEvent, Box<serde_json::value::RawValue>) {
    let (pdu, mut value) = PduBuilder {
        event_type: "m.room.message".into(),
        content: serde_json::value::to_raw_value(&json!({"msgtype":"m.text","body":body})).unwrap(),
        ..Default::default()
    }
    .hash_sign(
        &UserId::parse("@alice:missing.example").unwrap(),
        room_id,
        &RoomVersionId::V11,
    )
    .await
    .unwrap();
    value.remove("event_id");
    (pdu, serde_json::value::to_raw_value(&value).unwrap())
}

async fn response(
    room_id: &RoomId,
    required: &[OwnedEventId],
    events: Vec<Box<serde_json::value::RawValue>>,
) -> crate::AppResult<MissingEventsRecovery> {
    process_missing_events_response(
        REMOTE.try_into().unwrap(),
        room_id,
        &RoomVersionId::V11,
        false,
        MissingEventsResBody::new(events),
        required,
    )
    .await
}

async fn assert_retry_without_storage(
    candidate: OutlierPdu,
    recovery: crate::AppResult<MissingEventsRecovery>,
) {
    let id = candidate.event_id.clone();
    let result = candidate
        .finish_missing_events_recovery(REMOTE.try_into().unwrap(), true, false, recovery)
        .await;
    assert!(
        matches!(result, Err(AppError::HttpStatus(ref e)) if e.code == StatusCode::SERVICE_UNAVAILABLE)
    );
    assert_eq!(
        events::table
            .filter(events::id.eq(&id))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        event_datas::table
            .filter(event_datas::event_id.eq(&id))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap(),
        0
    );
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn missing_events_http_recovery_requires_usable_predecessors() {
    std::thread::Builder::new()
        .name("missing-events-recovery".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(recovery_cases())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn setup_room() -> OwnedRoomId {
    crate::test_database::init();
    // Disallow the sentinel destination. If an incomplete live recovery reaches
    // the state-fetch path, it cannot contact any external server and the test
    // detects the incorrect persistence/return instead of relying on a timeout.
    config::CONFIG.set(serde_json::from_value(json!({
        "server_name":"missing.example", "db":{"url":"unused-test-config"}, "ip_range_denylist":[],
        "federation":{"allowed_servers":["missing.example"]}
    })).unwrap()).unwrap();
    let user_id = UserId::parse("@alice:missing.example").unwrap();
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
        "missing-events-token",
        None,
        None,
    )
    .await
    .unwrap();
    let router = crate::routing::root();
    let mut created = tokio::spawn(async move {
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", "Bearer missing-events-token", true)
            .json(&json!({"room_version":"11", "preset":"public_chat"}))
            .send(router)
            .await
    })
    .await
    .unwrap();
    let body = created.take_json::<Value>().await.unwrap();
    assert_eq!(created.status_code, Some(StatusCode::OK), "{body}");
    RoomId::parse(body["room_id"].as_str().unwrap()).unwrap()
}

async fn recovery_cases() {
    let room_id = setup_room().await;
    let unknown: OwnedEventId = "$unknown-predecessor".try_into().unwrap();
    let (template, _) = pending(&room_id, "template").await;

    // Empty successful responses and the exact {} body from Complement cannot
    // authorize a live child or enter the later /state fallback.
    let empty = response(&room_id, std::slice::from_ref(&unknown), vec![]).await;
    assert_eq!(empty.as_ref().unwrap(), &MissingEventsRecovery::Incomplete);
    assert_retry_without_storage(
        child(&template, "$empty-child", std::slice::from_ref(&unknown)),
        empty,
    )
    .await;
    let empty_object = serde_json::from_str::<MissingEventsResBody>("{}").unwrap_err();
    assert_retry_without_storage(
        child(
            &template,
            "$empty-object-child",
            std::slice::from_ref(&unknown),
        ),
        Err(empty_object.into()),
    )
    .await;

    // Process an actual signed, authorized missing event through the production
    // pulled-PDU pipeline. A partial response still cannot complete two anchors.
    let (first, first_raw) = pending(&room_id, "first recovered").await;
    let (second, second_raw) = pending(&room_id, "second recovered").await;
    let required = [first.event_id.clone(), second.event_id.clone()];
    assert_eq!(
        response(&room_id, &required, vec![first_raw.clone()])
            .await
            .unwrap(),
        MissingEventsRecovery::Incomplete
    );
    let first_stored = crate::room::timeline::get_pdu(&first.event_id)
        .await
        .unwrap();
    assert!(!first_stored.is_outlier && !first_stored.soft_failed && !first_stored.rejected());
    assert_retry_without_storage(
        child(&template, "$partial-child", &required),
        Ok(MissingEventsRecovery::Incomplete),
    )
    .await;
    assert_eq!(
        response(&room_id, &required, vec![first_raw.clone(), second_raw])
            .await
            .unwrap(),
        MissingEventsRecovery::Complete
    );
    let sequence = first_stored.event_sn;
    assert_eq!(
        response(&room_id, &required, vec![first_raw.clone(), first_raw])
            .await
            .unwrap(),
        MissingEventsRecovery::Complete
    );
    assert_eq!(
        crate::room::timeline::get_pdu(&first.event_id)
            .await
            .unwrap()
            .event_sn,
        sequence
    );
    // An empty response can observe recovery completed concurrently by another
    // receiver; the result is based on authoritative rows, not response length.
    assert_eq!(
        response(&room_id, &required, vec![]).await.unwrap(),
        MissingEventsRecovery::Complete
    );
    let (concurrent, concurrent_raw) = pending(&room_id, "concurrently recovered").await;
    let barrier = tokio::sync::Barrier::new(2);
    let (writer, late_empty) = tokio::join!(
        async {
            let result = response(
                &room_id,
                std::slice::from_ref(&concurrent.event_id),
                vec![concurrent_raw],
            )
            .await;
            barrier.wait().await;
            result
        },
        async {
            // This receiver's empty HTTP response arrives after a concurrent
            // receiver has completed processing the same missing predecessor.
            barrier.wait().await;
            response(&room_id, std::slice::from_ref(&concurrent.event_id), vec![]).await
        }
    );
    assert_eq!(writer.unwrap(), MissingEventsRecovery::Complete);
    assert_eq!(late_empty.unwrap(), MissingEventsRecovery::Complete);
    assert_eq!(
        response(
            &RoomId::parse("!other:missing.example").unwrap(),
            &required,
            vec![]
        )
        .await
        .unwrap(),
        MissingEventsRecovery::Incomplete
    );

    // A legacy/partial metadata row without its room-scoped JSON payload is
    // not a usable predecessor, even when its flags claim timeline acceptance.
    let orphan = child(&template, "$orphan-metadata", &[]);
    let (orphan, ..) = orphan.save_to_database(false).await.unwrap();
    diesel::update(events::table.find(&orphan.event_id))
        .set((events::is_outlier.eq(false), events::soft_failed.eq(false)))
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();
    diesel::delete(event_datas::table.filter(event_datas::event_id.eq(&orphan.event_id)))
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();
    let recovery = response(&room_id, std::slice::from_ref(&orphan.event_id), vec![]).await;
    assert_eq!(
        recovery.as_ref().unwrap(),
        &MissingEventsRecovery::Incomplete
    );
    assert_retry_without_storage(
        child(
            &template,
            "$orphan-metadata-child",
            std::slice::from_ref(&orphan.event_id),
        ),
        recovery,
    )
    .await;

    // A cached raw outlier is not a usable anchor, even if it has parsed JSON.
    let raw = child(
        &template,
        "$cached-unvalidated",
        std::slice::from_ref(&unknown),
    );
    let (raw, ..) = raw.save_to_database(false).await.unwrap();
    let raw_child = child(
        &template,
        "$cached-unvalidated-child",
        std::slice::from_ref(&raw.event_id),
    );
    let recovery = fetch_and_process_missing_events(
        REMOTE.try_into().unwrap(),
        &room_id,
        &RoomVersionId::V11,
        &raw_child,
        false,
    )
    .await;
    assert!(
        recovery.is_err(),
        "cached raw data must not return Complete"
    );
    assert_retry_without_storage(raw_child, recovery).await;

    // Existing signed outliers must be processed, rather than skipped merely
    // because their event ID already has a database row.
    let (cached, cached_raw) = pending(&room_id, "cached signed event").await;
    outlier(cached.clone())
        .save_to_database(false)
        .await
        .unwrap();
    assert_eq!(
        response(&room_id, std::slice::from_ref(&cached.event_id), vec![])
            .await
            .unwrap(),
        MissingEventsRecovery::Incomplete
    );
    assert_eq!(
        response(
            &room_id,
            std::slice::from_ref(&cached.event_id),
            vec![cached_raw]
        )
        .await
        .unwrap(),
        MissingEventsRecovery::Complete
    );

    // Soft-failed and rejected rows never count as completed recovery.
    for (rejected, soft_failed) in [(false, true), (true, false)] {
        diesel::update(events::table.find(&first.event_id))
            .set((
                events::is_rejected.eq(rejected),
                events::soft_failed.eq(soft_failed),
            ))
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            response(&room_id, std::slice::from_ref(&first.event_id), vec![])
                .await
                .unwrap(),
            MissingEventsRecovery::Incomplete
        );
    }
    diesel::update(events::table.find(&first.event_id))
        .set((events::is_rejected.eq(false), events::soft_failed.eq(false)))
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();

    // A malformed canonical-JSON predecessor preserves the existing durable
    // rejection. Its next child gets a retryable result, not /state fallback.
    let malformed =
        serde_json::value::RawValue::from_string(r#"{"content":{"bad_val":1.1}}"#.into()).unwrap();
    let bad_response = response(&room_id, std::slice::from_ref(&unknown), vec![malformed]).await;
    assert!(
        matches!(&bad_response, Err(AppError::Matrix(e)) if e.kind == crate::core::error::ErrorKind::BadJson)
    );
    let bad_child = child(&template, "$bad-json-child", std::slice::from_ref(&unknown));
    let (rejected, ..) = bad_child
        .finish_missing_events_recovery(REMOTE.try_into().unwrap(), true, false, bad_response)
        .await
        .unwrap();
    assert!(rejected.rejected() && rejected.soft_failed);
    let reason = rejected.rejection_reason.clone();
    let next = child(
        &template,
        "$child-after-bad-json",
        std::slice::from_ref(&rejected.event_id),
    );
    assert_retry_without_storage(
        next,
        response(&room_id, std::slice::from_ref(&rejected.event_id), vec![]).await,
    )
    .await;
    assert_eq!(
        crate::room::timeline::get_pdu(&rejected.event_id)
            .await
            .unwrap()
            .rejection_reason,
        reason
    );

    // A rejected predecessor with legitimate historical state still allows a
    // normal descendant to reconnect, without any missing-events HTTP request.
    let mut historical = child(
        &template,
        "$historical-rejected",
        std::slice::from_ref(&cached.event_id),
    );
    historical.rejection_reason = Some("rejected historical update".into());
    let (historical, ..) = historical.save_to_database(false).await.unwrap();
    let reconnect = child(
        &template,
        "$reconnected-child",
        std::slice::from_ref(&historical.event_id),
    );
    let recovery = fetch_and_process_missing_events(
        REMOTE.try_into().unwrap(),
        &room_id,
        &RoomVersionId::V11,
        &reconnect,
        false,
    )
    .await
    .unwrap();
    assert_eq!(recovery, MissingEventsRecovery::Incomplete);
    let (reconnected, ..) = reconnect
        .finish_missing_events_recovery(REMOTE.try_into().unwrap(), true, false, Ok(recovery))
        .await
        .unwrap();
    assert!(!reconnected.rejected() && !reconnected.soft_failed);

    // Use a genuinely signed and authorized state PDU, not a message whose ID
    // merely suggests that it is imported state.
    let (state_pdu, mut state_json) = PduBuilder {
        event_type: "m.room.topic".into(),
        state_key: Some("".into()),
        content: serde_json::value::to_raw_value(&json!({"topic":"imported historical topic"}))
            .unwrap(),
        ..Default::default()
    }
    .hash_sign(
        &UserId::parse("@alice:missing.example").unwrap(),
        &room_id,
        &RoomVersionId::V11,
    )
    .await
    .unwrap();
    state_json.remove("event_id");
    let mut imported = crate::event::handler::process_to_outlier_pdu(
        REMOTE.try_into().unwrap(),
        &state_pdu.event_id,
        &room_id,
        &RoomVersionId::V11,
        state_json,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!imported.soft_failed && !imported.rejected());
    imported.soft_failed = true;
    let (imported, ..) = imported.save_to_database(false).await.unwrap();
    assert!(imported.is_outlier);
    assert_eq!(imported.event_ty.to_string(), "m.room.topic");
    assert_eq!(imported.state_key.as_deref(), Some(""));
    let reconnect = child(
        &template,
        "$state-import-child",
        std::slice::from_ref(&imported.event_id),
    );
    let resolved = crate::event::resolver::resolve_state_at_incoming(
        &reconnect,
        &crate::room::get_version_rules(&RoomVersionId::V11).unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    let topic_field =
        crate::room::state::ensure_field_id(&crate::core::events::StateEventType::RoomTopic, "")
            .await
            .unwrap();
    assert_eq!(resolved.get(&topic_field), Some(&imported.event_id));
    let recovery = fetch_and_process_missing_events(
        REMOTE.try_into().unwrap(),
        &room_id,
        &RoomVersionId::V11,
        &reconnect,
        false,
    )
    .await
    .unwrap();
    assert_eq!(recovery, MissingEventsRecovery::Incomplete);
    let (reconnected, ..) = reconnect
        .finish_missing_events_recovery(REMOTE.try_into().unwrap(), true, false, Ok(recovery))
        .await
        .unwrap();
    assert!(!reconnected.rejected() && !reconnected.soft_failed);

    // The live-only guard does not disable pulled/backfill state recovery.
    let imported = child(
        &template,
        "$incomplete-non-timeline",
        std::slice::from_ref(&unknown),
    );
    let (imported, ..) = imported
        .finish_missing_events_recovery(
            REMOTE.try_into().unwrap(),
            false,
            false,
            Ok(MissingEventsRecovery::Incomplete),
        )
        .await
        .unwrap();
    assert!(
        imported.is_outlier
            && imported.soft_failed
            && !imported.rejected()
            && !imported.is_backfill
    );

    // Run identical signed state through the actual live and send_join entry
    // points. Only the live timeline push should get the new retryable error;
    // historical imports retain their recoverable outlier for later state fetch.
    let (_, mut imported_state) = PduBuilder {
        event_type: "m.room.topic".into(),
        state_key: Some("".into()),
        content: serde_json::value::to_raw_value(
            &json!({"topic":"imported with a missing predecessor"}),
        )
        .unwrap(),
        ..Default::default()
    }
    .hash_sign(
        &UserId::parse("@alice:missing.example").unwrap(),
        &room_id,
        &RoomVersionId::V11,
    )
    .await
    .unwrap();
    imported_state.remove("event_id");
    imported_state.insert(
        "prev_events".into(),
        crate::core::serde::CanonicalJsonValue::Array(vec![
            crate::core::serde::CanonicalJsonValue::String(unknown.to_string()),
        ]),
    );
    crate::server_key::hash_and_sign_event(&mut imported_state, &RoomVersionId::V11).unwrap();
    let imported_id = crate::event::gen_event_id(&imported_state, &RoomVersionId::V11).unwrap();
    let live = crate::event::handler::process_incoming_pdu(
        REMOTE.try_into().unwrap(),
        &imported_id,
        &room_id,
        &RoomVersionId::V11,
        imported_state.clone(),
        true,
        false,
    )
    .await;
    assert!(
        matches!(live, Err(AppError::HttpStatus(ref error)) if error.code == StatusCode::SERVICE_UNAVAILABLE)
    );
    assert_eq!(
        events::table
            .filter(events::id.eq(&imported_id))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        event_datas::table
            .filter(event_datas::event_id.eq(&imported_id))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap(),
        0
    );
    crate::event::handler::process_join_response_pdu(
        REMOTE.try_into().unwrap(),
        &imported_id,
        &room_id,
        &RoomVersionId::V11,
        imported_state,
    )
    .await
    .unwrap();
    let imported = crate::room::timeline::get_pdu(&imported_id).await.unwrap();
    assert!(
        imported.is_outlier
            && imported.soft_failed
            && !imported.rejected()
            && !imported.is_backfill
    );
    assert_eq!(imported.event_ty.to_string(), "m.room.topic");
    assert_eq!(imported.state_key.as_deref(), Some(""));

    // With the sentinel transport denied, those paths retain a recoverable
    // outlier instead of returning the new live 503 or permanently rejecting it.
    let backfill = child(
        &template,
        "$incomplete-backfill",
        std::slice::from_ref(&unknown),
    );
    let (backfill, ..) = backfill
        .finish_missing_events_recovery(
            REMOTE.try_into().unwrap(),
            true,
            true,
            Ok(MissingEventsRecovery::Incomplete),
        )
        .await
        .unwrap();
    assert!(
        backfill.is_backfill && backfill.is_outlier && backfill.soft_failed && !backfill.rejected()
    );
    let pulled = child(
        &template,
        "$incomplete-pulled",
        std::slice::from_ref(&unknown),
    );
    let (pulled, ..) = pulled
        .process_pulled(REMOTE.try_into().unwrap(), false)
        .await
        .unwrap();
    assert!(pulled.is_outlier && pulled.soft_failed && !pulled.rejected());

    let database_error = child(
        &template,
        "$database-error-child",
        std::slice::from_ref(&unknown),
    )
    .finish_missing_events_recovery(
        REMOTE.try_into().unwrap(),
        true,
        false,
        Err(AppError::Diesel(diesel::result::Error::RollbackTransaction)),
    )
    .await;
    assert!(matches!(
        database_error,
        Err(AppError::Diesel(diesel::result::Error::RollbackTransaction))
    ));
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn missing_events_signed_predecessors_preserve_pending_state() {
    std::thread::Builder::new()
        .name("missing-events-pending-state".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(pending_state_cases())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn signed_pending(
    room_id: &RoomId,
    profile: bool,
    body: &str,
    prev: &[OwnedEventId],
    missing_auth: &OwnedEventId,
) -> (PduEvent, crate::core::serde::CanonicalJsonObject) {
    let user = UserId::parse("@alice:missing.example").unwrap();
    let content = if profile {
        json!({"membership":"join", "displayname":body})
    } else {
        json!({"msgtype":"m.text", "body":body})
    };
    let (_, mut value) = PduBuilder {
        event_type: if profile {
            "m.room.member"
        } else {
            "m.room.message"
        }
        .into(),
        state_key: profile.then(|| user.to_string()),
        content: serde_json::value::to_raw_value(&content).unwrap(),
        ..Default::default()
    }
    .hash_sign(&user, room_id, &RoomVersionId::V11)
    .await
    .unwrap();
    value.remove("event_id");
    value.insert(
        "prev_events".into(),
        crate::core::serde::to_canonical_value(prev).unwrap(),
    );
    let mut auth: Vec<OwnedEventId> =
        serde_json::from_value(value["auth_events"].clone().into()).unwrap();
    auth.push(missing_auth.clone());
    value.insert(
        "auth_events".into(),
        crate::core::serde::to_canonical_value(auth).unwrap(),
    );
    crate::server_key::hash_and_sign_event(&mut value, &RoomVersionId::V11).unwrap();
    let id = crate::event::gen_event_id(&value, &RoomVersionId::V11).unwrap();
    let mut pdu_json = value.clone();
    pdu_json.insert("event_id".into(), id.to_string().into());
    let pdu =
        PduEvent::from_json_value(room_id, &id, serde_json::to_value(pdu_json).unwrap()).unwrap();
    (pdu, value)
}

async fn pending_state_cases() {
    let room_id = setup_room().await;
    let remote = REMOTE.try_into().unwrap();
    let missing_auth: OwnedEventId = "$unavailable-auth-B".try_into().unwrap();
    let state_ids_event: OwnedEventId = "$snapshot-predecessor".try_into().unwrap();

    // E is signed but its auth ancestry cannot be completed. Passing it through
    // the production pulled pipeline must not publish its profile change.
    let (profile, profile_json) = signed_pending(
        &room_id,
        true,
        "poisoned profile E",
        std::slice::from_ref(&missing_auth),
        &missing_auth,
    )
    .await;
    crate::event::handler::process_pulled_pdu(
        remote,
        &profile.event_id,
        &room_id,
        &RoomVersionId::V11,
        profile_json,
        false,
    )
    .await
    .unwrap();
    let stored = crate::room::timeline::get_pdu(&profile.event_id)
        .await
        .unwrap();
    assert!(stored.is_outlier && stored.soft_failed && !stored.rejected());

    // Like TestCorruptedAuthChain, GME really supplies the immediate predecessor,
    // whose earlier snapshot/auth cannot be fetched. The closed-world transport
    // denies the snapshot request; it never contacts an external server.
    let (gme, gme_json) = signed_pending(
        &room_id,
        false,
        "actual signed GME predecessor",
        std::slice::from_ref(&state_ids_event),
        &profile.event_id,
    )
    .await;
    let gme_raw = serde_json::value::to_raw_value(&gme_json).unwrap();
    let recovery = response(
        &room_id,
        std::slice::from_ref(&gme.event_id),
        vec![gme_raw.clone()],
    )
    .await
    .unwrap();
    assert_ne!(
        recovery,
        MissingEventsRecovery::Incomplete,
        "a supplied signature-checked predecessor may retain pending state recovery"
    );
    assert_ne!(
        recovery,
        MissingEventsRecovery::Complete,
        "pending auth is never an accepted timeline anchor"
    );
    let stored = crate::room::timeline::get_pdu(&gme.event_id).await.unwrap();
    assert!(stored.is_outlier && stored.soft_failed && !stored.rejected());

    let (second, second_json) = signed_pending(
        &room_id,
        false,
        "second actual signed GME predecessor",
        std::slice::from_ref(&state_ids_event),
        &profile.event_id,
    )
    .await;
    assert_eq!(
        response(
            &room_id,
            &[gme.event_id.clone(), second.event_id.clone()],
            vec![
                gme_raw.clone(),
                serde_json::value::to_raw_value(&second_json).unwrap()
            ],
        )
        .await
        .unwrap(),
        MissingEventsRecovery::PendingState
    );
    let (accepted, accepted_raw) = pending(&room_id, "accepted complete anchor").await;
    assert_eq!(
        response(
            &room_id,
            &[gme.event_id.clone(), accepted.event_id.clone()],
            vec![gme_raw.clone(), accepted_raw],
        )
        .await
        .unwrap(),
        MissingEventsRecovery::PendingState
    );

    let (send, send_json) = signed_pending(
        &room_id,
        false,
        "live child with unavailable auth",
        std::slice::from_ref(&gme.event_id),
        &profile.event_id,
    )
    .await;
    let candidate = crate::event::handler::process_to_outlier_pdu(
        remote,
        &send.event_id,
        &room_id,
        &RoomVersionId::V11,
        send_json,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(candidate.soft_failed && !candidate.rejected());
    let (retained, value, _) = candidate
        .finish_missing_events_recovery(remote, true, false, Ok(recovery))
        .await
        .unwrap();
    assert!(retained.is_outlier && retained.soft_failed && !retained.rejected());
    assert!(
        crate::event::handler::process_to_timeline_pdu(retained, value, Some(remote))
            .await
            .is_err()
    );
    for id in [&profile.event_id, &gme.event_id, &send.event_id] {
        assert!(
            crate::room::timeline::get_non_outlier_pdu(id)
                .await
                .unwrap()
                .is_none()
        );
    }
    let current = crate::room::get_state(
        &room_id,
        &crate::core::events::StateEventType::RoomMember,
        "@alice:missing.example",
        None,
    )
    .await
    .unwrap();
    assert_ne!(current.event_id, profile.event_id);

    // An earlier successful pull is not provenance for a later empty response.
    assert_eq!(
        response(&room_id, std::slice::from_ref(&gme.event_id), vec![])
            .await
            .unwrap(),
        MissingEventsRecovery::Incomplete
    );
    let unknown: OwnedEventId = "$omitted-second-anchor".try_into().unwrap();
    assert_eq!(
        response(
            &room_id,
            &[gme.event_id.clone(), unknown],
            vec![gme_raw.clone()]
        )
        .await
        .unwrap(),
        MissingEventsRecovery::Incomplete
    );

    // Cached soft-failed JSON cannot stand in for signature verification of the
    // current response. Rejected cached IDs remain immutable as well.
    let mut forged: Value = serde_json::from_str(gme_raw.get()).unwrap();
    forged["signatures"] = json!({});
    assert_eq!(
        response(
            &room_id,
            std::slice::from_ref(&gme.event_id),
            vec![serde_json::value::to_raw_value(&forged).unwrap()]
        )
        .await
        .unwrap(),
        MissingEventsRecovery::Incomplete
    );
    diesel::update(events::table.find(&gme.event_id))
        .set(events::is_rejected.eq(true))
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        response(&room_id, std::slice::from_ref(&gme.event_id), vec![gme_raw])
            .await
            .unwrap(),
        MissingEventsRecovery::Incomplete
    );
}
