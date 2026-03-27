use std::cmp;
use std::time::Duration;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::sync_events::v5::*;
use crate::data;
use crate::routing::prelude::*;

/// `POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync`
/// ([MSC4186])
///
/// A simplified version of sliding sync ([MSC3575]).
///
/// Get all new events in a sliding window of rooms since the last sync or a
/// given point in time.
///
/// [MSC3575]: https://github.com/matrix-org/matrix-spec-proposals/pull/3575
/// [MSC4186]: https://github.com/matrix-org/matrix-spec-proposals/pull/4186
#[handler]
pub(super) async fn sync_events_v5(
    _aa: AuthArgs,
    args: SyncEventsReqArgs,
    req_body: JsonBody<SyncEventsReqBody>,
    depot: &mut Depot,
) -> JsonResult<SyncEventsResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let device_id = authed.device_id();

    let since_sn: i64 = args
        .pos
        .as_ref()
        .and_then(|string| string.parse().ok())
        .unwrap_or_default();

    let mut req_body = req_body.into_inner();

    let _conn_id = req_body.conn_id.clone();

    if since_sn == 0 {
        crate::sync_v5::forget_sync_request_connection(
            sender_id.to_owned(),
            device_id.to_owned(),
            req_body.conn_id.to_owned(),
        )
    }

    // Get sticky parameters from cache
    let known_rooms = crate::sync_v5::update_sync_request_with_cache(
        sender_id.to_owned(),
        device_id.to_owned(),
        &mut req_body,
    );

    let mut res_body =
        crate::sync_v5::sync_events(sender_id, device_id, since_sn, &req_body, &known_rooms)
            .await?;

    if since_sn > data::curr_sn()? || (args.pos.is_some() && res_body.is_empty()) {
        // No new events for this client — wait for changes instead of returning
        // immediately, which would cause the client to spin in a tight polling loop.
        let default = Duration::from_secs(30);
        let duration = cmp::min(args.timeout.unwrap_or(default), default);
        // Enforce a minimum wait on subsequent syncs. Without this, clients
        // sending timeout=0 would create a tight loop (~3ms/request) when idle.
        // The watcher still wakes us immediately when real events arrive, so
        // responsiveness is not affected.
        if args.pos.is_some() {
            let duration = cmp::max(duration, Duration::from_secs(3));
            let watcher = crate::watcher::watch(sender_id, device_id);
            _ = tokio::time::timeout(duration, watcher).await;
        }
        res_body =
            crate::sync_v5::sync_events(sender_id, device_id, since_sn, &req_body, &known_rooms)
                .await?;
    }

    trace!(
        rooms=?res_body.rooms.len(),
        account_data=?res_body.extensions.account_data.rooms.len(),
        receipts=?res_body.extensions.receipts.rooms.len(),
        "responding to request with"
    );
    json_ok(res_body)
}
