//! Client endpoints for MSC4140 delayed events.

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::delayed_events::{
    DelayedEventData, DelayedEventsResBody, SendDelayedEventReqArgs, SendDelayedEventReqBody,
    SendDelayedEventResBody, UpdateDelayedEventReqArgs, UpdateDelayedEventReqBody,
};
use crate::routing::prelude::*;

pub(super) fn authed_router() -> Router {
    Router::with_path("org.matrix.msc4140")
        .push(
            Router::with_path("rooms/{room_id}/delayed_event/{event_type}/{txn_id}")
                .put(send_delayed_event),
        )
        .push(
            Router::with_path("delayed_events")
                .get(list_delayed_events)
                .push(
                    Router::with_path("{delay_id}")
                        .get(get_delayed_event)
                        .post(update_delayed_event_v1)
                        .push(Router::with_path("{action}").post(update_delayed_event)),
                ),
        )
}

/// `PUT /_matrix/client/unstable/org.matrix.msc4140/rooms/{room_id}/delayed_event/{event_type}/
/// {txn_id}`
///
/// Schedule a message or state event to be sent into the room after a delay.
#[endpoint]
async fn send_delayed_event(
    _aa: AuthArgs,
    args: SendDelayedEventReqArgs,
    body: JsonBody<SendDelayedEventReqBody>,
    depot: &mut Depot,
) -> JsonResult<SendDelayedEventResBody> {
    let authed = depot.authed_info()?;
    let delay_id = crate::delayed_event::schedule(
        authed.user_id(),
        Some(authed.device_id()),
        authed.appservice().is_some(),
        &args,
        &body,
    )
    .await?;
    json_ok(SendDelayedEventResBody::new(delay_id))
}

/// `GET /_matrix/client/unstable/org.matrix.msc4140/delayed_events`
///
/// List the requesting user's scheduled delayed events in chronological order
/// of their intended send time.
#[endpoint]
async fn list_delayed_events(_aa: AuthArgs, depot: &mut Depot) -> JsonResult<DelayedEventsResBody> {
    let authed = depot.authed_info()?;
    let delayed_events = crate::delayed_event::list(authed.user_id()).await?;
    json_ok(DelayedEventsResBody::new(delayed_events))
}

/// `GET /_matrix/client/unstable/org.matrix.msc4140/delayed_events/{delay_id}`
///
/// Get the details of one of the requesting user's delayed events, whether
/// still scheduled or already finalized.
#[endpoint]
async fn get_delayed_event(
    _aa: AuthArgs,
    delay_id: PathParam<String>,
    depot: &mut Depot,
) -> JsonResult<DelayedEventData> {
    let authed = depot.authed_info()?;
    let delayed_event = crate::delayed_event::get(authed.user_id(), &delay_id.into_inner()).await?;
    json_ok(delayed_event)
}

/// `POST /_matrix/client/unstable/org.matrix.msc4140/delayed_events/{delay_id}/{action}`
///
/// Restart, send, or cancel a scheduled delayed event.
#[endpoint]
async fn update_delayed_event(
    _aa: AuthArgs,
    args: UpdateDelayedEventReqArgs,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    crate::delayed_event::update(authed.user_id(), &args.delay_id, &args.action).await?;
    empty_ok()
}

/// `POST /_matrix/client/unstable/org.matrix.msc4140/delayed_events/{delay_id}`
///
/// Deprecated body-action variant of the update endpoint, kept for clients
/// that implement the earlier iteration of MSC4140.
#[endpoint]
async fn update_delayed_event_v1(
    _aa: AuthArgs,
    delay_id: PathParam<String>,
    body: JsonBody<UpdateDelayedEventReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    crate::delayed_event::update(authed.user_id(), &delay_id.into_inner(), &body.action).await?;
    empty_ok()
}
