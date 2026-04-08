use std::collections::BTreeMap;

use diesel::prelude::*;
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;

use crate::core::Seqnum;
use crate::core::events::TimelineEventType;
use crate::core::federation::backfill::{BackfillReqArgs, BackfillResBody, backfill_request};
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonObject, JsonValue, RawJsonValue};
use crate::data::connect;
use crate::data::schema::*;
use crate::event::handler::process_to_timeline_pdu;
use crate::event::{BatchToken, handler, parse_fetched_pdu};
use crate::{AppError, AppResult, GetUrlOrigin, SnPduEvent, room};

/// Decide whether `/messages` needs to backfill more history from federation
/// before responding, and if so, do it.
///
/// Strategy: figure out the "frontier" — the point we need to fetch history older
/// than. Prefer the oldest event in the current page (so we walk strictly older
/// than what we already have). If the page is empty, fall back to the room's
/// recorded backward extremities. We only backfill when there is actual evidence
/// of missing history (a known prev_event that isn't in the local DB), so rooms
/// that have been fully fetched don't ping federation on every paginate.
#[tracing::instrument(skip_all)]
pub async fn backfill_if_required(
    room_id: &RoomId,
    _from_tk: &BatchToken,
    pdus: &IndexMap<Seqnum, SnPduEvent>,
    limit: usize,
) -> AppResult<Vec<SnPduEvent>> {
    // Walk the current page from lowest depth upward and pick the first event
    // that still has prev_events missing locally. This is a concrete
    // missing-history signal — back-filling from that event reaches into the
    // gap immediately rather than re-asking from a stale extremity that has
    // already been satisfied. We can't just take `min_by_key(depth)` because
    // the page often contains state events at depth 1-6 whose prev_events are
    // either empty (room create) or already present, while the actual gap
    // sits above them at the timeline messages we just paginated past.
    let mut sorted: Vec<&SnPduEvent> = pdus.values().collect();
    sorted.sort_by_key(|p| p.depth);
    for pdu in &sorted {
        if pdu.prev_events.is_empty() {
            continue;
        }
        let existing: Vec<OwnedEventId> = events::table
            .filter(events::id.eq_any(&pdu.prev_events))
            .select(events::id)
            .load(&mut connect()?)?;
        if pdu.prev_events.iter().any(|id| !existing.contains(id)) {
            return backfill_from_extremities(
                room_id,
                std::slice::from_ref(&pdu.event_id),
                limit,
            )
            .await;
        }
    }

    // Otherwise, if we returned fewer events than the limit and there are
    // recorded backward extremities, fall back to backfilling from those.
    // This handles the "user just joined, only state events present" case.
    //
    // We must filter the extremities first: `update_backward_extremities` only
    // ever removes the inserted event itself, so siblings that are already
    // fully satisfied (all prev_events present) can linger in the table and
    // mislead the remote's depth-first walk. If we hand those stale entries
    // to `/backfill`, the remote pops the highest-depth seed first and walks
    // back from there, easily exhausting its `limit` on events we already
    // have — never reaching the real frontier. So we only keep entries whose
    // event either isn't in our DB at all (a synthetic marker) or still has
    // missing prev_events.
    if pdus.len() < limit {
        let extremity_ids: Vec<OwnedEventId> = event_backward_extremities::table
            .filter(event_backward_extremities::room_id.eq(room_id))
            .select(event_backward_extremities::event_id)
            .distinct()
            .load(&mut connect()?)?;

        let mut fill_from: Vec<OwnedEventId> = Vec::new();
        for ext_id in extremity_ids {
            let Ok(pdu) = super::get_pdu(&ext_id) else {
                // Event isn't locally known — it's a synthetic frontier marker
                // for a parent we couldn't fetch yet. Use it as-is.
                fill_from.push(ext_id);
                continue;
            };
            if pdu.prev_events.is_empty() {
                continue;
            }
            let existing: Vec<OwnedEventId> = events::table
                .filter(events::id.eq_any(&pdu.prev_events))
                .select(events::id)
                .load(&mut connect()?)?;
            if pdu.prev_events.iter().any(|id| !existing.contains(id)) {
                fill_from.push(ext_id);
            }
        }

        if !fill_from.is_empty() {
            return backfill_from_extremities(room_id, &fill_from, limit).await;
        }
    }

    Ok(vec![])
}

/// Backfill events from the given backward extremities. Used when the messages
/// endpoint returns fewer events than the limit and we need to fetch history
/// from federation (e.g., after re-joining a room).
#[tracing::instrument(skip_all)]
pub async fn backfill_from_extremities(
    room_id: &RoomId,
    extremities: &[OwnedEventId],
    limit: usize,
) -> AppResult<Vec<SnPduEvent>> {
    let admin_servers = room::admin_servers(room_id, false)?;
    let room_version = room::get_version(room_id)?;

    for backfill_server in &admin_servers {
        info!("asking {backfill_server} for backfill from extremities");
        let request = backfill_request(
            &backfill_server.origin().await,
            BackfillReqArgs {
                room_id: room_id.to_owned(),
                v: extremities.to_vec(),
                // Synapse caps `/backfill` at 100 events per call, so asking
                // for more is wasted; ask for the max so we make as much
                // progress per round-trip as possible.
                limit: limit.max(100),
            },
        )?
        .into_inner();
        match crate::sending::send_federation_request(backfill_server, request, None)
            .await?
            .json::<BackfillResBody>()
            .await
        {
            Ok(response) => {
                let mut events = Vec::new();
                let pdus = response
                    .pdus
                    .into_iter()
                    .filter_map(|pdu| {
                        let val =
                            serde_json::from_str::<BTreeMap<String, JsonValue>>(pdu.get()).ok()?;
                        let depth = val.get("depth")?.as_i64()?;
                        Some((pdu, depth))
                    })
                    .sorted_by(|a, b| a.1.cmp(&b.1))
                    .map(|(pdu, _)| pdu)
                    .collect::<Vec<_>>();
                let mut saved_pdu_contents = Vec::new();
                for pdu in pdus {
                    match backfill_pdu(backfill_server, room_id, &room_version, pdu).await {
                        Ok(p) => saved_pdu_contents.push(p),
                        Err(e) => warn!("failed to add backfilled pdu: {e}"),
                    }
                }
                for (pdu, content) in saved_pdu_contents {
                    let event_id = pdu.event_id.clone();
                    if let Err(e) =
                        process_to_timeline_pdu(pdu, content, Some(backfill_server)).await
                    {
                        error!("failed to process backfill pdu to timeline {}", e);
                    } else if let Ok(pdu) = super::get_pdu(&event_id) {
                        events.push(pdu);
                    }
                }
                return Ok(events);
            }
            Err(e) => {
                warn!("{backfill_server} could not provide backfill from extremities: {e}");
            }
        }
    }
    Ok(vec![])
}

#[tracing::instrument(skip(pdu))]
pub async fn backfill_pdu(
    remote_server: &ServerName,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    pdu: Box<RawJsonValue>,
) -> AppResult<(SnPduEvent, CanonicalJsonObject)> {
    let (event_id, value) = parse_fetched_pdu(room_id, room_version, &pdu)?;
    // Skip the PDU if we already have it as a timeline event
    if let Ok(pdu) = super::get_pdu(&event_id) {
        info!("we already know {event_id}, skipping backfill");
        let value = super::get_pdu_json(&event_id)?
            .ok_or_else(|| AppError::public("event json not found"))?;
        return Ok((pdu, value));
    }
    let Some(outlier_pdu) =
        handler::process_to_outlier_pdu(remote_server, &event_id, room_id, room_version, value)
            .await?
    else {
        return Err(AppError::internal("failed to process backfilled pdu"));
    };
    let (pdu, value, _) = outlier_pdu.save_to_database(true)?;

    if pdu.event_ty == TimelineEventType::RoomMessage {
        #[derive(Deserialize)]
        struct ExtractBody {
            body: Option<String>,
        }

        let _content = pdu
            .get_content::<ExtractBody>()
            .map_err(|_| AppError::internal("invalid content in pdu."))?;
    }

    Ok((pdu, value))
}
