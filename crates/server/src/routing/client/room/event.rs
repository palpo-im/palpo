use std::collections::HashSet;

use palpo_core::Direction;
use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde_json::json;
use serde_json::value::to_raw_value;
use state::DbRoomStateField;

use crate::core::client::filter::LazyLoadOptions;
use crate::core::client::redact::{RedactEventReqArgs, RedactEventReqBody, RedactEventResBody};
use crate::core::client::room::{
    ContextReqArgs, ContextResBody, ReportContentReqBody, RoomEventResBody,
};
use crate::core::events::room::message::RoomMessageEventContent;
use crate::core::events::room::redaction::RoomRedactionEventContent;
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::room::{RoomEventReqArgs, TimestampToEventReqArgs, TimestampToEventResBody};
use crate::data::room::DbEvent;
use crate::event::fetching::fetch_event;
use crate::event::handler::{process_pulled_pdu, remote_timestamp_to_event};
use crate::event::parse_fetched_pdu;
use crate::room::{state, timeline};
use crate::utils::HtmlEscape;
use crate::{
    AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, OptionalExtension, PduBuilder,
    empty_ok, json_ok, room,
};

/// #GET /_matrix/client/r0/rooms/{room_id}/event/{event_id}
/// Gets a single event.
///
/// - The event must be visible to the user under the room's history visibility
#[endpoint]
pub(super) async fn get_room_event(
    _aa: AuthArgs,
    args: RoomEventReqArgs,
    depot: &mut Depot,
) -> JsonResult<RoomEventResBody> {
    let authed = depot.authed_info()?;

    let event = DbEvent::get_by_id(&args.event_id).await?;
    if event.rejection_reason.is_some() {
        warn!("event {} is rejected", &args.event_id);
        return Err(MatrixError::not_found("event not found").into());
    }
    if event.is_outlier {
        warn!("event {} is outlier", &args.event_id);
        return Err(MatrixError::not_found("event not found").into());
    }

    let event = timeline::get_pdu(&args.event_id).await?;

    if !state::user_can_see_event(authed.user_id(), &args.event_id).await? {
        return Err(MatrixError::not_found("event not found").into());
    }
    if crate::event::is_ignored_sender_pdu(&event, authed.user_id()).await {
        return Err(MatrixError::sender_ignored(
            "the sender of the requested event is ignored",
            Some(event.sender.clone()),
        )
        .into());
    }
    if crate::event::is_ignored_pdu(&event, authed.user_id()).await {
        return Err(MatrixError::not_found("event not found").into());
    }

    json_ok(RoomEventResBody::new(
        event.to_room_event_for(authed.user_id(), authed.device_id()),
    ))
}

/// #POST /_matrix/client/r0/rooms/{room_id}/report/{event_id}
/// Reports an inappropriate event to homeserver admins
#[endpoint]
pub(super) async fn report(
    _aa: AuthArgs,
    args: RoomEventReqArgs,
    body: JsonBody<ReportContentReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let pdu = timeline::get_pdu(&args.event_id).await?;
    let body = body.into_inner();

    if let Some(true) = body.score.map(|s| !(-100..=0).contains(&s)) {
        return Err(MatrixError::invalid_param("invalid score, must be within 0 to -100").into());
    };

    if let Some(true) = body.reason.clone().map(|s| s.chars().count() > 250) {
        return Err(MatrixError::invalid_param(
            "reason too long, should be 250 characters or fewer",
        )
        .into());
    };

    let report_reason = body.reason.clone();
    let report_score = body.score;
    crate::data::room::create_event_report(crate::data::room::NewDbEventReport::new(
        pdu.room_id.clone(),
        pdu.event_id.clone(),
        authed.user_id().to_owned(),
        report_reason.clone(),
        Some(json!({
            "reason": report_reason,
            "score": report_score,
        })),
        report_score,
    ))
    .await?;

    let _ = crate::admin::send_message(RoomMessageEventContent::text_html(
        format!(
            "Report received from: {}\n\n\
                Event ID: {:?}\n\
                Room ID: {:?}\n\
                Sent By: {:?}\n\n\
                Report Score: {:?}\n\
                Report Reason: {:?}",
            authed.user_id(),
            pdu.event_id,
            pdu.room_id,
            pdu.sender,
            body.score,
            body.reason
        ),
        format!(
            "<details><summary>Report received from: <a href=\"https://matrix.to/#/{0:?}\">{0:?}\
                </a></summary><ul><li>Event Info<ul><li>Event ID: <code>{1:?}</code>\
                <a href=\"https://matrix.to/#/{2:?}/{1:?}\">🔗</a></li><li>Room ID: <code>{2:?}</code>\
                </li><li>Sent By: <a href=\"https://matrix.to/#/{3:?}\">{3:?}</a></li></ul></li><li>\
                Report Info<ul><li>Report Score: {4:?}</li><li>Report Reason: {5}</li></ul></li>\
                </ul></details>",
            authed.user_id(),
            pdu.event_id,
            pdu.room_id,
            pdu.sender,
            body.score,
            HtmlEscape(body.reason.as_deref().unwrap_or(""))
        ),
    )).await;
    empty_ok()
}

/// #GET /_matrix/client/r0/rooms/{room_id}/context/{event_id}
/// Allows loading room history around an event.
///
/// - Only works if the user is joined (TODO: always allow, but only show events if the user was
/// joined, depending on history_visibility)
#[endpoint]
pub(super) async fn get_context(
    _aa: AuthArgs,
    args: ContextReqArgs,
    depot: &mut Depot,
) -> JsonResult<ContextResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();

    let (lazy_load_enabled, lazy_load_send_redundant) = match &args.filter.lazy_load_options {
        LazyLoadOptions::Enabled {
            include_redundant_members,
        } => (true, *include_redundant_members),
        _ => (false, false),
    };

    let mut lazy_loaded = HashSet::new();
    let base_token = crate::event::get_live_token(&args.event_id)
        .await
        .map_err(|_| MatrixError::not_found("base event id not found"))?;
    let base_event = timeline::get_pdu(&args.event_id).await?;
    let room_id = base_event.room_id.clone();

    if !state::user_can_see_event(sender_id, &args.event_id).await? {
        return Err(
            MatrixError::forbidden("you don't have permission to view this event", None).into(),
        );
    }
    if crate::event::is_ignored_sender_pdu(&base_event, sender_id).await {
        return Err(MatrixError::sender_ignored(
            "the sender of the requested event is ignored",
            Some(base_event.sender.clone()),
        )
        .into());
    }

    if !crate::room::lazy_loading::lazy_load_was_sent_before_for(
        sender_id,
        authed.device_id(),
        &room_id,
        &base_event.sender,
    )
    .await?
        || lazy_load_send_redundant
    {
        lazy_loaded.insert(base_event.sender.as_str().to_owned());
    }

    // Use limit with maximum 100
    let limit = args.limit.min(100);
    let base_event = base_event.to_room_event_for(sender_id, authed.device_id());
    let events_before_loaded = timeline::stream::load_pdus_backward(
        Some(sender_id),
        &room_id,
        Some(base_token),
        None,
        None,
        limit / 2,
    )
    .await?;
    let mut events_before = Vec::new();
    for (sn, pdu) in events_before_loaded {
        if state::user_can_see_event(sender_id, &pdu.event_id)
            .await
            .unwrap_or(false)
        {
            events_before.push((sn, pdu));
        }
    }

    for (_, event) in &events_before {
        if !crate::room::lazy_loading::lazy_load_was_sent_before_for(
            sender_id,
            authed.device_id(),
            &room_id,
            &event.sender,
        )
        .await?
            || lazy_load_send_redundant
        {
            lazy_loaded.insert(event.sender.as_str().to_owned());
        }
    }

    let start_token = events_before
        .last()
        .map(|(_, pdu)| pdu.historic_token())
        .unwrap_or_else(|| base_token);
    let events_before = events_before
        .into_iter()
        .map(|(_, pdu)| pdu.to_room_event_for(sender_id, authed.device_id()))
        .collect::<Vec<_>>();
    let events_after = timeline::stream::load_pdus_forward(
        Some(sender_id),
        &room_id,
        Some(base_token),
        None,
        None,
        limit / 2,
    )
    .await?;

    for (_, event) in &events_after {
        if !crate::room::lazy_loading::lazy_load_was_sent_before_for(
            sender_id,
            authed.device_id(),
            &room_id,
            &event.sender,
        )
        .await?
            || lazy_load_send_redundant
        {
            lazy_loaded.insert(event.sender.as_str().to_owned());
        }
    }

    let frame_id = match state::get_pdu_frame_id(
        events_after
            .last()
            .map_or(&*args.event_id, |(_, e)| &*e.event_id),
    )
    .await
    {
        Ok(s) => s,
        Err(_) => crate::room::get_frame_id(&room_id, None)
            .await
            .unwrap_or_default(),
    };
    let state_ids = state::get_full_state_ids(frame_id)
        .await
        .unwrap_or_default();
    let end_token = events_after
        .last()
        .map(|(_, e)| e.live_token())
        .unwrap_or_else(|| base_token);
    let events_after: Vec<_> = events_after
        .into_iter()
        .map(|(_, pdu)| pdu.to_room_event_for(sender_id, authed.device_id()))
        .collect();
    let mut state = Vec::new();

    for (field_id, event_id) in state_ids {
        let DbRoomStateField {
            event_ty,
            state_key,
            ..
        } = state::get_field(field_id).await?;

        if event_ty != StateEventType::RoomMember {
            let pdu = match timeline::get_pdu(&event_id).await {
                Ok(pdu) => pdu,
                Err(_) => {
                    error!("pdu in state not found: {}", event_id);
                    continue;
                }
            };
            state.push(pdu.to_state_event_for(sender_id, authed.device_id()));
        } else if !lazy_load_enabled || lazy_loaded.contains(&state_key) {
            let pdu = match timeline::get_pdu(&event_id).await {
                Ok(pdu) => pdu,
                Err(_) => {
                    error!("pdu in state not found: {}", event_id);
                    continue;
                }
            };
            state.push(pdu.to_state_event_for(sender_id, authed.device_id()));
        }
    }

    json_ok(ContextResBody {
        start: Some(start_token.to_string()),
        end: Some(end_token.to_string()),
        events_before,
        event: Some(base_event),
        events_after,
        state,
    })
}

/// #PUT /_matrix/client/r0/rooms/{room_id}/redact/{event_id}/{txn_id}
/// Tries to send a redaction event into the room.
#[endpoint]
pub(super) async fn send_redact(
    _aa: AuthArgs,
    args: RedactEventReqArgs,
    body: JsonBody<RedactEventReqBody>,
    depot: &mut Depot,
) -> JsonResult<RedactEventResBody> {
    let authed = depot.authed_info()?;

    let state_lock = crate::room::lock_state(&args.room_id).await;
    let endpoint = format!("room/redact/{}", args.event_id);
    let event_id = timeline::build_and_append_pdu_with_transaction(
        PduBuilder {
            event_type: TimelineEventType::RoomRedaction,
            content: to_raw_value(&RoomRedactionEventContent {
                redacts: Some(args.event_id.clone()),
                reason: body.reason.clone(),
            })
            .expect("event is valid, we just created it"),
            redacts: Some(args.event_id),
            unsigned: [("transaction_id".to_owned(), to_raw_value(&args.txn_id)?)].into(),
            transaction_device: authed.device_id().map(ToOwned::to_owned),
            ..Default::default()
        },
        authed.user_id(),
        &args.room_id,
        &crate::room::get_version(&args.room_id).await?,
        &state_lock,
        Some((&endpoint, &args.txn_id, authed.device_id())),
    )
    .await?
    .pdu
    .event_id;

    let event_id = (*event_id).to_owned();
    json_ok(RedactEventResBody { event_id })
}

/// #GET /_matrix/client/r0/rooms/{room_id}/timestamp_to_event
#[endpoint]
pub(super) async fn timestamp_to_event(
    _aa: AuthArgs,
    args: TimestampToEventReqArgs,
    depot: &mut Depot,
) -> JsonResult<TimestampToEventResBody> {
    let authed = depot.authed_info()?;
    if !room::user::is_joined(authed.user_id(), &args.room_id).await? {
        return Err(MatrixError::forbidden("You are not joined to this room.", None).into());
    }
    let local_event = crate::event::get_event_for_timestamp(&args.room_id, args.ts, args.dir)
        .await
        .optional()?;

    let mut is_event_next_to_backward_gap = false;
    let mut is_event_next_to_forward_gap = false;
    if let Some(local_event) = &local_event {
        let local_event = timeline::get_pdu(&local_event.0).await?;
        match args.dir {
            Direction::Backward => {
                is_event_next_to_forward_gap =
                    timeline::is_event_next_to_forward_gap(&local_event).await?
            }
            Direction::Forward => {
                is_event_next_to_backward_gap =
                    timeline::is_event_next_to_backward_gap(&local_event).await?
            }
        }
    }
    if local_event.is_none() || is_event_next_to_backward_gap || is_event_next_to_forward_gap {
        let remote_servers = room::admin_servers(&args.room_id, false).await?;
        let Ok((
            remote_server,
            TimestampToEventResBody {
                event_id,
                origin_server_ts,
            },
        )) = remote_timestamp_to_event(
            &remote_servers,
            &args.room_id,
            args.dir,
            args.ts,
            local_event.as_ref(),
        )
        .await
        else {
            return if let Some((event_id, origin_server_ts)) = local_event {
                json_ok(TimestampToEventResBody {
                    event_id,
                    origin_server_ts,
                })
            } else {
                Err(StatusError::not_found().brief("no event found").into())
            };
        };
        let room_version = crate::room::get_version(&args.room_id).await?;
        let Ok((event_id, event_value)) = parse_fetched_pdu(
            &args.room_id,
            &room_version,
            &fetch_event(&remote_server, &event_id).await?.pdu,
        ) else {
            error!("failed parse featch pdu for timestamp to event");
            return json_ok(TimestampToEventResBody {
                event_id,
                origin_server_ts,
            });
        };
        let event_value_for_promotion = event_value.clone();
        process_pulled_pdu(
            &remote_server,
            &event_id,
            &args.room_id,
            &room_version,
            event_value,
            true,
        )
        .await?;

        // Per the spec, after finding the event the server "should try to
        // backfill this event". Fetching the single event above leaves it as an
        // outlier whenever its prev_events aren't local yet, which keeps it out
        // of `/messages` pagination (the timeline page stops at the gap). Pull
        // the surrounding history so the event becomes a connected, non-outlier
        // timeline event that subsequent backward pagination can return.
        if let Err(e) =
            timeline::backfill_from_extremities(&args.room_id, std::slice::from_ref(&event_id), 100)
                .await
        {
            warn!("failed to backfill history around timestamp_to_event result: {e}");
        }

        // `/backfill` returns the predecessors of its seed, not necessarily the
        // seed itself. The first processing attempt above can therefore leave
        // the timestamp result as an outlier while its predecessor is missing.
        // Retry after backfill so the now-connected result gets an event-time
        // state frame and can be authorized by `/context` and `/messages`.
        if timeline::get_non_outlier_pdu(&event_id).await?.is_none() {
            process_pulled_pdu(
                &remote_server,
                &event_id,
                &args.room_id,
                &room_version,
                event_value_for_promotion,
                true,
            )
            .await?;
        }

        return json_ok(TimestampToEventResBody {
            event_id,
            origin_server_ts,
        });
    }
    if let Some((event_id, origin_server_ts)) = local_event {
        json_ok(TimestampToEventResBody {
            event_id,
            origin_server_ts,
        })
    } else {
        Err(StatusError::not_found().brief("no event found").into())
    }
}

#[cfg(test)]
mod transaction_tests {
    use diesel::prelude::*;
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
    use salvo::test::{RequestBuilder, ResponseExt, TestClient};
    use serde_json::Value;

    use super::*;
    use crate::data::connect;
    use crate::data::schema::*;

    async fn request(service: &Service, request: RequestBuilder) -> (Option<StatusCode>, Value) {
        request_as(service, "redact-transaction", request).await
    }

    async fn request_as(
        service: &Service,
        token: &str,
        request: RequestBuilder,
    ) -> (Option<StatusCode>, Value) {
        let router = service.router();
        let request = request.add_header("authorization", format!("Bearer {token}"), true);
        let mut response = tokio::spawn(async move { request.send(router).await })
            .await
            .unwrap();
        (response.status_code, response.take_json().await.unwrap())
    }

    #[test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
    fn database_redaction_retries_recover_publication_and_preserve_endpoint_scope() {
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

    async fn run() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name":"redact.example", "db":{"url":"unused-test-config"}
            }))
            .unwrap()
        });
        let user = UserId::parse(format!(
            "@redacttransaction:{}",
            crate::config::server_name()
        ))
        .unwrap();
        crate::data::user::create_user(&crate::data::user::NewDbUser {
            id: user.clone(),
            ty: None,
            is_admin: true,
            is_guest: false,
            is_local: true,
            localpart: user.localpart().into(),
            server_name: user.server_name().to_owned(),
            appservice_id: None,
            created_at: crate::core::UnixMillis::now(),
        })
        .await
        .unwrap();
        crate::data::user::device::create_device(
            &user,
            "REDACT_TRANSACTION".into(),
            "redact-transaction",
            None,
            None,
        )
        .await
        .unwrap();
        let service = Service::new(
            Router::new()
                .hoop(crate::hoops::auth_by_access_token)
                .push(
                    Router::with_path("createRoom").post(crate::routing::client::room::create_room),
                )
                .push(crate::routing::client::room::authed_router()),
        );
        let (status, created) = request(
            &service,
            TestClient::post("http://localhost/createRoom").json(&json!({"room_version":"11"})),
        )
        .await;
        assert_eq!(status, Some(StatusCode::OK), "{created}");
        let room = created["room_id"].as_str().unwrap();
        let (status, message) = request(
            &service,
            TestClient::put(format!(
                "http://localhost/rooms/{room}/send/m.room.message/shared"
            ))
            .json(&json!({"msgtype":"m.text","body":"target"})),
        )
        .await;
        assert_eq!(status, Some(StatusCode::OK), "{message}");
        let target = message["event_id"].as_str().unwrap();
        let url = format!("http://localhost/rooms/{room}/redact/{target}/shared");
        let (first, second) = tokio::join!(
            request(
                &service,
                TestClient::put(&url).json(&json!({"reason":"same"}))
            ),
            request(
                &service,
                TestClient::put(&url).json(&json!({"reason":"same"}))
            )
        );
        assert_eq!(first.0, Some(StatusCode::OK), "{:?}", first.1);
        assert_eq!(second.0, Some(StatusCode::OK), "{:?}", second.1);
        assert_eq!(first.1["event_id"], second.1["event_id"]);
        let (_, retry) = request(
            &service,
            TestClient::put(&url).json(&json!({"reason":"changed retry body"})),
        )
        .await;
        assert_eq!(retry["event_id"], first.1["event_id"]);

        // Simulate a failed receipt write after the event was published. Recovery
        // must use the persisted event's trusted metadata rather than append again.
        connect().await.unwrap().batch_execute("CREATE FUNCTION q15_fail_receipt() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.endpoint LIKE 'room/redact/%' AND NEW.txn_id = 'recover' THEN RAISE EXCEPTION 'injected receipt failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER q15_fail_receipt BEFORE INSERT ON event_idempotents FOR EACH ROW EXECUTE FUNCTION q15_fail_receipt();").await.unwrap();
        let recovery_url = format!("http://localhost/rooms/{room}/redact/{target}/recover");
        let failed = request(&service, TestClient::put(&recovery_url).json(&json!({}))).await;
        assert_eq!(
            failed.0,
            Some(StatusCode::INTERNAL_SERVER_ERROR),
            "{:?}",
            failed.1
        );
        let before: i64 = events::table
            .filter(events::room_id.eq(room))
            .filter(events::ty.eq("m.room.redaction"))
            .count()
            .get_result(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(before, 2);
        connect().await.unwrap().batch_execute("DROP TRIGGER q15_fail_receipt ON event_idempotents; DROP FUNCTION q15_fail_receipt();").await.unwrap();
        let (status, recovered) =
            request(&service, TestClient::put(&recovery_url).json(&json!({}))).await;
        assert_eq!(status, Some(StatusCode::OK), "{recovered}");
        let after: i64 = events::table
            .filter(events::room_id.eq(room))
            .filter(events::ty.eq("m.room.redaction"))
            .count()
            .get_result(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            before, after,
            "retry appended a second event after failed receipt write"
        );
        let (_, replayed) =
            request(&service, TestClient::put(&recovery_url).json(&json!({}))).await;
        assert_eq!(replayed["event_id"], recovered["event_id"]);

        // A normal /send of the same event type, target and transaction string
        // belongs to a different endpoint and must never satisfy /redact recovery.
        let (status, normal_redaction) = request(
            &service,
            TestClient::put(format!(
                "http://localhost/rooms/{room}/send/m.room.redaction/cross-endpoint"
            ))
            .json(&json!({"redacts":target})),
        )
        .await;
        assert_eq!(status, Some(StatusCode::OK), "{normal_redaction}");
        let (status, dedicated_redaction) = request(
            &service,
            TestClient::put(format!(
                "http://localhost/rooms/{room}/redact/{target}/cross-endpoint"
            ))
            .json(&json!({})),
        )
        .await;
        assert_eq!(status, Some(StatusCode::OK), "{dedicated_redaction}");
        assert_ne!(
            normal_redaction["event_id"],
            dedicated_redaction["event_id"]
        );

        // Appservice sender authentication has no device assertion. Trusted
        // provenance must still be stored with an explicit NULL device.
        let (sender, _receiver) = tokio::sync::mpsc::channel(64);
        let _ = crate::sending::MPSC_SENDER.set(sender);
        crate::appservice::register_appservice(
            serde_json::from_value(json!({
                "id":"redact-device-less", "url":null, "as_token":"redact-device-less",
                "hs_token":"fixture-hs", "sender_localpart":"redacttransaction",
                "namespaces":{"users":[],"aliases":[],"rooms":[]}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        connect().await.unwrap().batch_execute("CREATE FUNCTION q15_fail_as_receipt() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.endpoint LIKE 'room/redact/%' AND NEW.txn_id = 'as-recover' THEN RAISE EXCEPTION 'injected AS receipt failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER q15_fail_as_receipt BEFORE INSERT ON event_idempotents FOR EACH ROW EXECUTE FUNCTION q15_fail_as_receipt();").await.unwrap();
        let as_url = format!("http://localhost/rooms/{room}/redact/{target}/as-recover");
        let (status, failed_as) = request_as(
            &service,
            "redact-device-less",
            TestClient::put(&as_url).json(&json!({})),
        )
        .await;
        assert_eq!(
            status,
            Some(StatusCode::INTERNAL_SERVER_ERROR),
            "{failed_as}"
        );
        let before: i64 = events::table
            .filter(events::room_id.eq(room))
            .filter(events::ty.eq("m.room.redaction"))
            .count()
            .get_result(&mut connect().await.unwrap())
            .await
            .unwrap();
        connect().await.unwrap().batch_execute("DROP TRIGGER q15_fail_as_receipt ON event_idempotents; DROP FUNCTION q15_fail_as_receipt();").await.unwrap();
        let (status, recovered_as) = request_as(
            &service,
            "redact-device-less",
            TestClient::put(&as_url).json(&json!({})),
        )
        .await;
        assert_eq!(status, Some(StatusCode::OK), "{recovered_as}");
        let after: i64 = events::table
            .filter(events::room_id.eq(room))
            .filter(events::ty.eq("m.room.redaction"))
            .count()
            .get_result(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            before, after,
            "device-less recovery appended another redaction"
        );
        let metadata: Option<Value> = event_datas::table
            .filter(event_datas::event_id.eq(recovered_as["event_id"].as_str().unwrap()))
            .select(event_datas::internal_metadata)
            .first(&mut connect().await.unwrap())
            .await
            .unwrap();
        let metadata = metadata.unwrap();
        assert!(metadata["transaction_device"].is_null());
        assert_eq!(
            metadata["transaction_endpoint"],
            format!("room/redact/{target}")
        );
    }
}
