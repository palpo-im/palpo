use std::time::Duration;

use salvo::prelude::*;

use crate::core::client::sync_events;
use crate::{AuthArgs, DepotExt, JsonResult, json_ok};

/// #GET /_matrix/client/r0/sync
/// Synchronize the client's state with the latest state on the server.
///
/// - This endpoint takes a `since` parameter which should be the `next_batch` value from a
/// previous request for incremental syncs.
///
/// Calling this endpoint without a `since` parameter returns:
/// - Some of the most recent events of each timeline
/// - Notification counts for each room
/// - Joined and invited member counts, heroes
/// - All state events
///
/// Calling this endpoint with a `since` parameter from a previous `next_batch` returns:
/// For joined rooms:
/// - Some of the most recent events of each timeline that happened after since
/// - If user joined the room after since: All state events (unless lazy loading is activated) and
/// all device list updates in that room
/// - If the user was already in the room: A list of all events that are in the state now, but were
/// not in the state at `since`
/// - If the state we send contains a member event: Joined and invited member counts, heroes
/// - Device list updates that happened after `since`
/// - If there are events in the timeline we send or the user send updated his read mark:
///   Notification counts
/// - EDUs that are active now (read receipts, typing updates, presence)
/// - TODO: Allow multiple sync streams to support Pantalaimon
///
/// For invited rooms:
/// - If the user was invited after `since`: A subset of the state of the room at the point of the
///   invite
///
/// For left rooms:
/// - If the user left after `since`: prev_batch token, empty state (TODO: subset of the state at
///   the point of the leave)
///
/// - Sync is handled in an async task, multiple requests from the same device with the same
/// `since` will be cached
#[endpoint]
pub(super) async fn sync_events_v3(
    _aa: AuthArgs,
    args: sync_events::v3::SyncEventsReqArgs,
    depot: &mut Depot,
    req: &mut Request,
) -> JsonResult<sync_events::v3::SyncEventsResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let device_id = authed.require_device_id()?;

    #[cfg(not(feature = "unstable-msc4532"))]
    crate::user::ping_presence(sender_id, &args.set_presence).await?;
    #[cfg(feature = "unstable-msc4532")]
    crate::user::presence::revised::touch(
        sender_id,
        device_id.as_str(),
        args.set_presence.as_str(),
        false,
    )
    .await?;
    let mut body = crate::sync_v3::sync_events(sender_id, device_id, &args).await?;

    if !args.full_state
        && body.rooms.is_empty()
        && body.presence.is_empty()
        && body.account_data.is_empty()
        && body.device_lists.is_empty()
        && body.to_device.is_empty()
    {
        // Hang a few seconds so requests are not spammed
        // Stop hanging if new info arrives
        let default = Duration::from_secs(30);
        let duration = std::cmp::min(args.timeout.unwrap_or(default), default);
        // Setup watchers, so if there's no response, we can wait for them
        let watcher = crate::watcher::watch(sender_id, device_id, false, None);
        _ = tokio::time::timeout(duration, watcher).await;

        // Retry returning data
        body = crate::sync_v3::sync_events(sender_id, device_id, &args).await?;
    }
    #[cfg(feature = "unstable-msc4532")]
    {
        crate::user::presence::revised::touch(
            sender_id,
            device_id.as_str(),
            args.set_presence.as_str(),
            false,
        )
        .await?;
        let revised = req
            .query::<bool>(crate::core::presence::revised::REVISED)
            .unwrap_or(false);
        for raw in &mut body.presence.events {
            let mut event = serde_json::to_value(&*raw)?;
            let user = crate::core::UserId::parse(event["sender"].as_str().unwrap_or_default())?;
            crate::user::presence::revised::project(&user, &mut event["content"], revised).await?;
            *raw = crate::core::serde::RawJson::from_value(&event)?;
        }
    }
    #[cfg(not(feature = "unstable-msc4532"))]
    let _ = req;
    json_ok(body)
}
