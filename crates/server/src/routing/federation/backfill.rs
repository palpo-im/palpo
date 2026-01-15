use std::collections::BTreeMap;

use diesel::prelude::*;
use indexmap::IndexMap;
use salvo::prelude::*;

use crate::core::UnixMillis;
use crate::core::federation::backfill::{BackfillReqArgs, BackfillResBody};
use crate::data::connect;
use crate::data::schema::*;
use crate::room::{state, timeline};
use crate::routing::prelude::*;

pub fn router() -> Router {
    Router::with_path("backfill/{room_id}").get(get_backfill)
}

/// #GET /_matrix/federation/v1/backfill/{room_id}
/// Retrieves events from before the sender joined the room, if the room's
/// history visibility allows.
#[endpoint]
async fn get_backfill(
    _aa: AuthArgs,
    args: BackfillReqArgs,
    depot: &mut Depot,
) -> JsonResult<BackfillResBody> {
    let origin = depot.origin()?;
    debug!("got backfill request from: {}", origin);

    let seeds = events::table
        .filter(events::id.eq_any(&args.v))
        .select((events::id, events::depth))
        .load::<(OwnedEventId, i64)>(&mut connect()?)?;

    let mut queue = BTreeMap::new();
    for (seed_id, seed_depth) in seeds {
        queue.insert(seed_depth, seed_id);
    }

    let limit = args.limit;

    let mut events = IndexMap::with_capacity(limit);
    while !queue.is_empty() && events.len() < limit {
        let Some((_depth, event_id)) = queue.pop_last() else {
            break;
        };
        let mut prev_ids = event_edges::table
            .filter(event_edges::event_id.eq(&event_id))
            .select(event_edges::prev_id)
            .load::<OwnedEventId>(&mut connect()?)?;
        prev_ids.retain(|p| !events.contains_key(p));
        if !events.contains_key(&event_id) {
            if let Ok((pdu, data)) = timeline::get_pdu_and_data(&event_id)
                && state::server_can_see_event(origin, &args.room_id, &pdu.event_id)?
            {
                events.insert(
                    event_id.clone(),
                    crate::sending::convert_to_outgoing_federation_event(data),
                );
            }
        }
        let prevs = events::table
            .filter(events::id.eq_any(&prev_ids))
            .select((events::id, events::depth))
            .load::<(OwnedEventId, i64)>(&mut connect()?)?;
        for (prev_id, prev_depth) in prevs {
            queue.insert(prev_depth, prev_id);
        }
    }
    json_ok(BackfillResBody {
        origin: config::get().server_name.to_owned(),
        origin_server_ts: UnixMillis::now(),
        pdus: events.into_values().collect(),
    })
}
