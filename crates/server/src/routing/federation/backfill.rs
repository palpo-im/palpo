use salvo::prelude::*;

use crate::core::federation::backfill::{BackfillReqArgs, BackfillResBody};
use crate::core::{UnixMillis, user_id};
use crate::room::{state, timeline};
use crate::{AuthArgs, DepotExt, JsonResult, MatrixError, config, json_ok};

pub fn router() -> Router {
    Router::with_path("backfill/{room_id}").get(history)
}

/// #GET /_matrix/federation/v1/backfill/{room_id}
/// Retrieves events from before the sender joined the room, if the room's
/// history visibility allows.
#[endpoint]
async fn history(
    _aa: AuthArgs,
    args: BackfillReqArgs,
    depot: &mut Depot,
) -> JsonResult<BackfillResBody> {
    let origin = depot.origin()?;
    debug!("Got backfill request from: {}", origin);

    let until = args
        .v
        .iter()
        .filter_map(|event_id| crate::event::get_event_sn(event_id).ok())
        .max()
        .ok_or(MatrixError::invalid_param("No known eventid in v"))?;

    let limit = args.limit.min(100);

    let all_events = timeline::get_pdus_backward(
        user_id!("@doesntmatter:palpo.im"),
        &args.room_id,
        until,
        None,
        None,
        limit,
    )?;

    let mut events = Vec::with_capacity(all_events.len());
    for (_, pdu) in all_events {
        if state::server_can_see_event(origin, &args.room_id, &pdu.event_id)?
            && let Some(pdu_json) = timeline::get_pdu_json(&pdu.event_id)?
        {
            events.push(crate::sending::convert_to_outgoing_federation_event(
                pdu_json,
            ));
        }
    }

    json_ok(BackfillResBody {
        origin: config::get().server_name.to_owned(),
        origin_server_ts: UnixMillis::now(),
        pdus: events,
    })
}
