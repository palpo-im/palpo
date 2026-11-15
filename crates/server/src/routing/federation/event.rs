use std::collections::{HashSet, VecDeque};

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::UnixMillis;
use crate::core::federation::authorization::{EventAuthReqArgs, EventAuthResBody};
use crate::core::federation::event::{
    EventReqArgs, EventResBody, MissingEventsReqBody, MissingEventsResBody,
};
use crate::core::identifiers::*;
use crate::core::room::{TimestampToEventReqArgs, TimestampToEventResBody};
use crate::data::room::DbEvent;
use crate::room::{state, timeline};
use crate::{
    AppError, AppResult, AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, config, json_ok,
};

pub fn router() -> Router {
    Router::new()
        .push(Router::with_path("event/{event_id}").get(get_event))
        .push(Router::with_path("event_auth/{room_id}/{event_id}").get(auth_chain))
        .push(Router::with_path("timestamp_to_event/{room_id}").get(timestamp_to_event))
        .push(Router::with_path("get_missing_events/{room_id}").post(missing_events))
        .push(
            Router::with_path("exchange_third_party_invite/{room_id}")
                .put(exchange_third_party_invite),
        )
}

/// State/auth recovery uses current server membership, rather than message
/// history visibility. Bind the requested event to that same authorized room
/// using its database identity, including v12's room-ID-less create events.
pub(super) async fn check_room_event_access(
    origin: &ServerName,
    room_id: &RoomId,
    event_id: &EventId,
) -> AppResult<()> {
    crate::federation::access_check(origin, room_id, None).await?;
    if DbEvent::get_by_id(event_id).await?.room_id != room_id {
        return Err(MatrixError::forbidden("event belongs to a different room", None).into());
    }
    Ok(())
}

/// #GET /_matrix/federation/v1/event/{event_id}
/// Retrieves a single event from the server.
///
/// - Only works if a user of this server is currently invited or joined the room
#[endpoint]
async fn get_event(
    _aa: AuthArgs,
    args: EventReqArgs,
    depot: &mut Depot,
) -> JsonResult<EventResBody> {
    let origin = depot.origin()?;
    let event = DbEvent::get_by_id(&args.event_id).await?;
    if event.rejection_reason.is_some() {
        warn!("event {} is rejected, returning 404", &args.event_id);
        return Err(MatrixError::not_found("event not found").into());
    }

    let event_json = timeline::get_pdu_json(&args.event_id)
        .await?
        .ok_or_else(|| {
            warn!("event not found, event id: {:?}", &args.event_id);
            MatrixError::not_found("event not found")
        })?;

    // Look up the room_id from the events table column rather than from the
    // event JSON. v12 events do not include `room_id` in their JSON content
    // (it's derived from the create event id), so reading it from JSON would
    // fail for those rooms.
    crate::federation::access_check(origin, &event.room_id, Some(&args.event_id)).await?;
    json_ok(EventResBody {
        origin: config::get().server_name.to_owned(),
        origin_server_ts: UnixMillis::now(),
        pdu: crate::sending::convert_to_outgoing_federation_event(event_json).await,
    })
}

/// #GET /_matrix/federation/v1/event_auth/{room_id}/{event_id}
/// Retrieves the auth chain for a given event.
///
/// - This does not include the event itself
#[endpoint]
async fn auth_chain(
    _aa: AuthArgs,
    args: EventAuthReqArgs,
    depot: &mut Depot,
) -> JsonResult<EventAuthResBody> {
    let origin = depot.origin()?;
    check_room_event_access(origin, &args.room_id, &args.event_id).await?;

    let auth_chain_ids =
        crate::room::auth_chain::get_auth_chain_ids(&args.room_id, [&*args.event_id].into_iter())
            .await?;

    let mut auth_chain_events = Vec::new();
    for id in auth_chain_ids.into_iter() {
        if let Some(json) = timeline::get_pdu_json(&id).await.ok().flatten() {
            auth_chain_events
                .push(crate::sending::convert_to_outgoing_federation_event(json).await);
        }
    }

    json_ok(EventAuthResBody {
        auth_chain: auth_chain_events,
    })
}

#[endpoint]
async fn timestamp_to_event(
    _aa: AuthArgs,
    args: TimestampToEventReqArgs,
    depot: &mut Depot,
) -> JsonResult<TimestampToEventResBody> {
    let origin = depot.origin()?;
    crate::federation::access_check(origin, &args.room_id, None).await?;

    let (event_id, origin_server_ts) =
        crate::event::get_event_for_timestamp(&args.room_id, args.ts, args.dir).await?;
    json_ok(TimestampToEventResBody {
        event_id,
        origin_server_ts,
    })
}

/// #POST /_matrix/federation/v1/get_missing_events/{room_id}
/// Retrieves events that the sender is missing.
#[endpoint]
async fn missing_events(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<MissingEventsReqBody>,
    depot: &mut Depot,
) -> JsonResult<MissingEventsResBody> {
    let origin = depot.origin()?;

    let room_id = room_id.into_inner();
    crate::federation::access_check(origin, &room_id, None).await?;

    let latest: HashSet<_> = body.latest_events.iter().cloned().collect();
    let earliest: HashSet<_> = body.earliest_events.iter().cloned().collect();
    let mut seen = HashSet::new();
    let mut queued_events = VecDeque::new();
    for event_id in &body.latest_events {
        if seen.insert(event_id.clone()) {
            queued_events.push_back(event_id.clone());
        }
    }
    let mut events = Vec::new();

    while events.len() < body.limit {
        let Some(event_id) = queued_events.pop_front() else {
            break;
        };
        if earliest.contains(&event_id) {
            continue;
        }
        if let Some(pdu) = timeline::get_pdu_json(&event_id).await? {
            // Bind identity and depth to trusted storage, including v12 events
            // whose federation JSON omits room_id.
            let db_event = DbEvent::get_by_id(&event_id).await?;
            let event_room_id_owned = db_event.room_id;

            if *event_room_id_owned != *room_id {
                warn!(
                    "evil event detected: Event {} found while searching in room {}",
                    event_id, &room_id
                );
                return Err(MatrixError::invalid_param("evil event detected").into());
            }

            if u64::try_from(db_event.depth).unwrap_or_default() < body.min_depth {
                continue;
            }

            let parents =
                serde_json::from_value::<Vec<OwnedEventId>>(
                    serde_json::to_value(pdu.get("prev_events").cloned().ok_or_else(|| {
                        AppError::internal("Event in db has no prev_events field.")
                    })?)
                    .expect("canonical json is valid json value"),
                )
                .map_err(|_| AppError::internal("invalid prev_events content in pdu in db::"))?;
            for parent in parents {
                if seen.insert(parent.clone()) {
                    queued_events.push_back(parent);
                }
            }
            if !latest.contains(&event_id)
                && state::server_can_see_event(origin, &room_id, &event_id).await?
            {
                events.push((
                    event_id,
                    crate::sending::convert_to_outgoing_federation_event(pdu).await,
                ));
            }
        } else {
            warn!("event not found, event id: {:?}", event_id);
        }
    }
    let events = events.into_iter().rev().map(|(_, event)| event).collect();
    json_ok(MissingEventsResBody { events })
}

#[endpoint]
async fn exchange_third_party_invite(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::unrecognized("Third-party invite exchange is not implemented.").into())
}

#[cfg(test)]
#[path = "event_review_tests.rs"]
mod review_tests;
