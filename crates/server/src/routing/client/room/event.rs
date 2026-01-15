use std::collections::HashSet;

use palpo_core::Direction;
use salvo::oapi::extract::*;
use salvo::prelude::*;
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
/// - You have to currently be joined to the room (TODO: Respect history visibility)
#[endpoint]
pub(super) fn get_room_event(
    _aa: AuthArgs,
    args: RoomEventReqArgs,
    depot: &mut Depot,
) -> JsonResult<RoomEventResBody> {
    let authed = depot.authed_info()?;

    let event = DbEvent::get_by_id(&args.event_id)?;
    if event.rejection_reason.is_some() {
        warn!("event {} is rejected", &args.event_id);
        return Err(MatrixError::not_found("event not found").into());
    }
    if event.is_outlier {
        warn!("event {} is outlier", &args.event_id);
        return Err(MatrixError::not_found("event not found").into());
    }

    let event = timeline::get_pdu(&args.event_id)?;

    if !state::user_can_see_event(authed.user_id(), &args.event_id)? {
        return Err(MatrixError::not_found("event not found").into());
    }

    let mut event = event.clone();
    event.add_age()?;

    json_ok(RoomEventResBody::new(event.to_room_event()))
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
    let pdu = timeline::get_pdu(&args.event_id)?;

    if let Some(true) = body.score.map(|s| !(-100..=0).contains(&s)) {
        return Err(MatrixError::invalid_param("invalid score, must be within 0 to -100").into());
    };

    if let Some(true) = body.reason.clone().map(|s| s.chars().count() > 250) {
        return Err(MatrixError::invalid_param(
            "reason too long, should be 250 characters or fewer",
        )
        .into());
    };

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
pub(super) fn get_context(
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
        .map_err(|_| MatrixError::not_found("base event id not found"))?;
    let base_event = timeline::get_pdu(&args.event_id)?;
    let room_id = base_event.room_id.clone();

    if !state::user_can_see_event(sender_id, &args.event_id)? {
        return Err(
            MatrixError::forbidden("you don't have permission to view this event", None).into(),
        );
    }

    if !crate::room::lazy_loading::lazy_load_was_sent_before(
        sender_id,
        authed.device_id(),
        &room_id,
        &base_event.sender,
    )? || lazy_load_send_redundant
    {
        lazy_loaded.insert(base_event.sender.as_str().to_owned());
    }

    // Use limit with maximum 100
    let limit = args.limit.min(100);
    let base_event = base_event.to_room_event();
    let events_before = timeline::stream::load_pdus_backward(
        Some(sender_id),
        &room_id,
        Some(base_token),
        None,
        None,
        limit / 2,
    )?
    .into_iter()
    .filter(|(_, pdu)| state::user_can_see_event(sender_id, &pdu.event_id).unwrap_or(false))
    .collect::<Vec<_>>();

    for (_, event) in &events_before {
        if !crate::room::lazy_loading::lazy_load_was_sent_before(
            sender_id,
            authed.device_id(),
            &room_id,
            &event.sender,
        )? || lazy_load_send_redundant
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
        .map(|(_, pdu)| pdu.to_room_event())
        .collect::<Vec<_>>();
    let events_after = timeline::stream::load_pdus_forward(
        Some(sender_id),
        &room_id,
        Some(base_token),
        None,
        None,
        limit / 2,
    )?;

    for (_, event) in &events_after {
        if !crate::room::lazy_loading::lazy_load_was_sent_before(
            sender_id,
            authed.device_id(),
            &room_id,
            &event.sender,
        )? || lazy_load_send_redundant
        {
            lazy_loaded.insert(event.sender.as_str().to_owned());
        }
    }

    let frame_id = match state::get_pdu_frame_id(
        events_after
            .last()
            .map_or(&*args.event_id, |(_, e)| &*e.event_id),
    ) {
        Ok(s) => s,
        Err(_) => crate::room::get_frame_id(&room_id, None).unwrap_or_default(),
    };
    let state_ids = state::get_full_state_ids(frame_id).unwrap_or_default();
    let end_token = events_after
        .last()
        .map(|(_, e)| e.live_token())
        .unwrap_or_else(|| base_token);
    let events_after: Vec<_> = events_after
        .into_iter()
        .map(|(_, pdu)| pdu.to_room_event())
        .collect();
    let mut state = Vec::new();

    for (field_id, event_id) in state_ids {
        let DbRoomStateField {
            event_ty,
            state_key,
            ..
        } = state::get_field(field_id)?;

        if event_ty != StateEventType::RoomMember {
            let pdu = match timeline::get_pdu(&event_id) {
                Ok(pdu) => pdu,
                Err(_) => {
                    error!("pdu in state not found: {}", event_id);
                    continue;
                }
            };
            state.push(pdu.to_state_event());
        } else if !lazy_load_enabled || lazy_loaded.contains(&state_key) {
            let pdu = match timeline::get_pdu(&event_id) {
                Ok(pdu) => pdu,
                Err(_) => {
                    error!("pdu in state not found: {}", event_id);
                    continue;
                }
            };
            state.push(pdu.to_state_event());
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
///
/// - TODO: Handle txn id
#[endpoint]
pub(super) async fn send_redact(
    _aa: AuthArgs,
    args: RedactEventReqArgs,
    body: JsonBody<RedactEventReqBody>,
    depot: &mut Depot,
) -> JsonResult<RedactEventResBody> {
    let authed = depot.authed_info()?;

    let state_lock = crate::room::lock_state(&args.room_id).await;
    let event_id = timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomRedaction,
            content: to_raw_value(&RoomRedactionEventContent {
                redacts: Some(args.event_id.clone()),
                reason: body.reason.clone(),
            })
            .expect("event is valid, we just created it"),
            redacts: Some(args.event_id),
            ..Default::default()
        },
        authed.user_id(),
        &args.room_id,
        &crate::room::get_version(&args.room_id)?,
        &state_lock,
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
    if !room::user::is_joined(authed.user_id(), &args.room_id)? {
        return Err(MatrixError::forbidden("You are not joined to this room.", None).into());
    }
    let local_event =
        crate::event::get_event_for_timestamp(&args.room_id, args.ts, args.dir).optional()?;

    let mut is_event_next_to_backward_gap = false;
    let mut is_event_next_to_forward_gap = false;
    if let Some(local_event) = &local_event {
        let local_event = timeline::get_pdu(&local_event.0)?;
        match args.dir {
            Direction::Backward => {
                is_event_next_to_forward_gap = timeline::is_event_next_to_forward_gap(&local_event)?
            }
            Direction::Forward => {
                is_event_next_to_backward_gap =
                    timeline::is_event_next_to_backward_gap(&local_event)?
            }
        }
    }
    if local_event.is_none() || is_event_next_to_backward_gap || is_event_next_to_forward_gap {
        let remote_servers = room::admin_servers(&args.room_id, false)?;
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
        let room_version = crate::room::get_version(&args.room_id)?;
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
        process_pulled_pdu(
            &remote_server,
            &event_id,
            &args.room_id,
            &room_version,
            event_value,
            true,
        )
        .await?;

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
