use salvo::prelude::*;

use crate::core::client::room::{ThreadsReqArgs, ThreadsResBody};
use crate::room::state;
use crate::{AuthArgs, DepotExt, JsonResult, MatrixError, json_ok};

/// #GET /_matrix/client/r0/rooms/{room_id}/threads
#[endpoint]
pub(super) async fn list_threads(
    _aa: AuthArgs,
    args: ThreadsReqArgs,
    depot: &mut Depot,
) -> JsonResult<ThreadsResBody> {
    let authed = depot.authed_info()?;

    if args.limit == Some(0) {
        return Err(MatrixError::invalid_param("Thread limit must be greater than zero.").into());
    }

    // Use limit or else 10, with maximum 100
    let limit = args
        .limit
        .and_then(|l| l.try_into().ok())
        .unwrap_or(10)
        .min(100);

    let (events, next_batch) =
        crate::room::thread::get_threads(&args.room_id, &args.include, limit, args.from.as_deref())
            .await?;

    let mut threads = Vec::new();
    for (sn, pdu) in events.into_iter() {
        if state::user_can_see_event(authed.user_id(), &pdu.event_id)
            .await
            .unwrap_or(false)
        {
            threads.push((sn, pdu));
        }
    }

    json_ok(ThreadsResBody {
        chunk: threads
            .into_iter()
            .map(|(_, pdu)| pdu.to_room_event_for(authed.user_id(), authed.device_id()))
            .collect(),
        next_batch,
    })
}
