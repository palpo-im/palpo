use std::collections::BTreeMap;

use salvo::oapi::extract::*;
use salvo::prelude::*;
use ulid::Ulid;

use crate::core::Seqnum;
use crate::core::client::dehydrated_device::{
    DehydratedDeviceEventsReqArgs, DehydratedDeviceEventsReqBody, DehydratedDeviceEventsResBody,
};
use crate::core::device::DirectDeviceContent;
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::core::to_device::{
    DeviceIdOrAllDevices, SendEventToDeviceReqArgs, SendEventToDeviceReqBody,
};
use crate::{
    AppResult, AuthArgs, DepotExt, EmptyResult, IsRemoteOrLocal, JsonResult, MatrixError, data,
    empty_ok, json_ok,
};

pub fn authed_router() -> Router {
    Router::with_path("sendToDevice/{event_type}/{txn_id}").put(send_to_device)
}

/// #PUT /_matrix/client/r0/sendToDevice/{event_type}/{txn_id}
/// Send a to-device event to a set of client devices.
#[endpoint]
async fn send_to_device(
    _aa: AuthArgs,
    args: SendEventToDeviceReqArgs,
    body: JsonBody<SendEventToDeviceReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    // Check if this is a new transaction id
    if crate::transaction_id::txn_id_exists(
        &args.txn_id,
        authed.user_id(),
        Some(authed.device_id()),
    )
    .await?
    {
        return empty_ok();
    }

    for (target_user_id, map) in &body.messages {
        for (target_device_id_maybe, event) in map {
            if target_user_id.server_name().is_remote() {
                let mut map = BTreeMap::new();
                map.insert(target_device_id_maybe.clone(), event.clone());
                let mut messages = BTreeMap::new();
                messages.insert(target_user_id.clone(), map);

                let message_id = Ulid::generate();
                crate::sending::send_reliable_edu(
                    target_user_id.server_name(),
                    &Edu::DirectToDevice(DirectDeviceContent {
                        sender: authed.user_id().to_owned(),
                        ev_type: args.event_type.clone(),
                        message_id: message_id.to_string().into(),
                        messages,
                    }),
                    &message_id.to_string(),
                )
                .await?;

                continue;
            }

            match target_device_id_maybe {
                DeviceIdOrAllDevices::DeviceId(target_device_id) => {
                    data::user::device::add_to_device_event(
                        authed.user_id(),
                        target_user_id,
                        target_device_id,
                        &args.event_type.to_string(),
                        event
                            .deserialize_as()
                            .map_err(|_| MatrixError::invalid_param("Event is invalid"))?,
                    )
                    .await?
                }

                DeviceIdOrAllDevices::AllDevices => {
                    for target_device_id in
                        data::user::all_to_device_target_ids(target_user_id).await?
                    {
                        data::user::device::add_to_device_event(
                            authed.user_id(),
                            target_user_id,
                            &target_device_id,
                            &args.event_type.to_string(),
                            event
                                .deserialize_as()
                                .map_err(|_| MatrixError::invalid_param("Event is invalid"))?,
                        )
                        .await?;
                    }
                }
            }
        }
    }

    // Save transaction id with empty data
    crate::transaction_id::add_txn_id(
        &args.txn_id,
        authed.user_id(),
        Some(authed.device_id()),
        None,
        None,
    )
    .await?;

    empty_ok()
}

/// How many to-device messages one batch carries when the client does not say.
const DEFAULT_EVENT_LIMIT: usize = 100;

/// An upper bound on `limit`, so one request cannot be made to load an unbounded inbox.
const MAX_EVENT_LIMIT: usize = 1000;

/// #GET /_matrix/client/unstable/org.matrix.msc3814.v1/dehydrated_device/{device_id}/events
/// Retrieves the to-device messages sent to a dehydrated device ([MSC3814]).
///
/// Reading does not consume: an interrupted rehydration must be restartable, possibly by a
/// different device, so messages already returned stay available. `next_batch` is present
/// only while more messages remain, which is how the client knows it has everything and
/// can replace the dehydrated device.
///
/// [MSC3814]: https://github.com/matrix-org/matrix-spec-proposals/pull/3814
#[endpoint]
pub(super) async fn for_dehydrated(
    _aa: AuthArgs,
    args: DehydratedDeviceEventsReqArgs,
    depot: &mut Depot,
) -> JsonResult<DehydratedDeviceEventsResBody> {
    let authed = depot.authed_info()?;
    let body = collect_dehydrated_events(
        authed.user_id(),
        &args.device_id,
        args.from.as_deref(),
        args.limit,
    )
    .await?;
    json_ok(body)
}

/// #POST /_matrix/client/unstable/org.matrix.msc3814.v1/dehydrated_device/{device_id}/events
/// Retrieves the to-device messages sent to a dehydrated device ([MSC3814]).
///
/// The `POST` form comes from an earlier draft of the MSC and is kept for clients that
/// still use it. It differs from `GET` in taking the resume token in the body and in
/// always returning `next_batch`, so a client cannot use its absence to detect the last
/// batch; those clients stop when a response comes back empty.
///
/// [MSC3814]: https://github.com/matrix-org/matrix-spec-proposals/pull/3814
#[endpoint]
pub(super) async fn for_dehydrated_legacy(
    _aa: AuthArgs,
    args: DehydratedDeviceEventsReqArgs,
    body: JsonBody<DehydratedDeviceEventsReqBody>,
    depot: &mut Depot,
) -> JsonResult<DehydratedDeviceEventsResBody> {
    let authed = depot.authed_info()?;
    let next_batch = body.into_inner().next_batch;

    let mut body = collect_dehydrated_events(
        authed.user_id(),
        &args.device_id,
        next_batch.as_deref(),
        args.limit,
    )
    .await?;

    // Always present in this form, even for the final batch.
    if body.next_batch.is_none() {
        body.next_batch = Some(next_batch.unwrap_or_else(|| "0".to_owned()));
    }

    json_ok(body)
}

async fn collect_dehydrated_events(
    user_id: &UserId,
    device_id: &DeviceId,
    from: Option<&str>,
    limit: Option<usize>,
) -> AppResult<DehydratedDeviceEventsResBody> {
    // Only the user's current dehydrated device may be read this way; any other device ID
    // -- including one of the user's live devices -- is forbidden, per MSC3814.
    let dehydrated_device_id = data::user::get_dehydrated_device(user_id)
        .await?
        .map(|(device_id, _)| device_id);
    if dehydrated_device_id.as_deref() != Some(device_id) {
        return Err(MatrixError::forbidden(
            "The given device ID is not the user's dehydrated device.",
            None,
        )
        .into());
    }

    let since_sn = parse_batch_token(from)?;
    let events = data::user::device::to_device_events_from(
        user_id,
        device_id,
        since_sn,
        resolve_limit(limit),
    )
    .await?;

    // The batch is the last one when nothing follows its final message. Deciding this from
    // the number of events returned would be wrong when the inbox happens to hold exactly
    // `limit` more.
    let next_batch = match events.last().map(|(occur_sn, _)| *occur_sn) {
        Some(last_sn)
            if data::user::device::has_to_device_events_after(user_id, device_id, last_sn)
                .await? =>
        {
            Some(last_sn.to_string())
        }
        _ => None,
    };

    Ok(DehydratedDeviceEventsResBody {
        events: events.into_iter().map(|(_, event)| event).collect(),
        next_batch,
    })
}

/// Parses a `next_batch` / `from` token into the stream position it names.
fn parse_batch_token(from: Option<&str>) -> Result<Option<Seqnum>, MatrixError> {
    from.map(|from| {
        from.parse::<Seqnum>()
            .map_err(|_| MatrixError::invalid_param("Invalid next_batch token."))
    })
    .transpose()
}

/// The batch size to use, bounded so one request cannot pull an entire inbox into memory.
fn resolve_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(DEFAULT_EVENT_LIMIT)
        .clamp(1, MAX_EVENT_LIMIT)
}

#[cfg(test)]
mod dehydrated_tests {
    use super::{DEFAULT_EVENT_LIMIT, MAX_EVENT_LIMIT, parse_batch_token, resolve_limit};

    #[test]
    fn batch_tokens_are_stream_positions() {
        assert_eq!(parse_batch_token(None).unwrap(), None);
        assert_eq!(parse_batch_token(Some("42")).unwrap(), Some(42));

        // A token the server never issued must be rejected rather than silently restarting
        // the client from the beginning of the inbox.
        assert!(parse_batch_token(Some("")).is_err());
        assert!(parse_batch_token(Some("abc")).is_err());
        assert!(parse_batch_token(Some("1.5")).is_err());
    }

    #[test]
    fn the_batch_size_is_bounded() {
        assert_eq!(resolve_limit(None), DEFAULT_EVENT_LIMIT);
        assert_eq!(resolve_limit(Some(10)), 10);
        assert_eq!(resolve_limit(Some(usize::MAX)), MAX_EVENT_LIMIT);
        // A zero limit would return an empty batch forever, so it never makes progress.
        assert_eq!(resolve_limit(Some(0)), 1);
    }
}
