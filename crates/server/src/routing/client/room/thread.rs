use salvo::prelude::*;

use crate::core::client::room::{ThreadsReqArgs, ThreadsResBody};
use crate::room::state;
use crate::{AuthArgs, DepotExt, JsonResult, json_ok};

/// #GET /_matrix/client/r0/rooms/{room_id}/threads
#[endpoint]
pub(super) async fn list_threads(
    _aa: AuthArgs,
    args: ThreadsReqArgs,
    depot: &mut Depot,
) -> JsonResult<ThreadsResBody> {
    let authed = depot.authed_info()?;

    // Use limit or else 10, with maximum 100
    let limit = args
        .limit
        .and_then(|l| l.try_into().ok())
        .unwrap_or(10)
        .min(100);

    let from: Option<i64> = if let Some(from) = &args.from {
        Some(from.parse()?)
    } else {
        None
    };

    let (events, next_batch) =
        crate::room::thread::get_threads(&args.room_id, &args.include, limit, from)?;

    let threads = events
        .into_iter()
        .filter(|(_, pdu)| {
            state::user_can_see_event(authed.user_id(), &pdu.event_id).unwrap_or(false)
        })
        .collect::<Vec<_>>();

    json_ok(ThreadsResBody {
        chunk: threads
            .into_iter()
            .map(|(_, pdu)| pdu.to_room_event())
            .collect(),
        next_batch: next_batch.map(|b| b.to_string()),
    })
}
