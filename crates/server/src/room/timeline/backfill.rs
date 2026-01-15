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

#[tracing::instrument(skip_all)]
pub async fn backfill_if_required(
    room_id: &RoomId,
    from_tk: &BatchToken,
    pdus: &IndexMap<Seqnum, SnPduEvent>,
    limit: usize,
) -> AppResult<Vec<SnPduEvent>> {
    let mut depths = pdus
        .values()
        .map(|p| (p.event_id.clone(), p.depth as i64))
        .collect::<Vec<_>>();
    if let Some(topological_ordering) = from_tk.topological_ordering() {
        if let Ok(event_id) = events::table
            .filter(events::room_id.eq(room_id))
            .filter(events::topological_ordering.eq(topological_ordering))
            .select(events::id)
            .first::<OwnedEventId>(&mut connect()?)
        {
            depths.push((event_id, topological_ordering.abs()));
        }
    }
    depths.sort_unstable_by(|a, b| b.1.cmp(&a.1));

    let (prev_event, prev_depth) = if let Some(depth) = depths.first() {
        depth
    } else {
        return Ok(vec![]);
    };

    let mut prev_depth = *prev_depth;
    let mut prev_event = prev_event;
    let last_depth = depths.last().map(|&(_, d)| d).unwrap_or_default() as i64;
    if prev_depth == last_depth {
        return Ok(vec![]);
    }

    let depths = events::table
        .filter(events::depth.lt(prev_depth))
        .filter(events::depth.ge(last_depth))
        .order(events::depth.desc())
        .select((events::id, events::depth))
        .load::<(OwnedEventId, i64)>(&mut connect()?)?;

    let mut found_big_gap = false;
    let mut number_of_gaps = 0;
    let mut fill_from = None;
    for &(ref event_id, depth) in depths.iter() {
        let delta = prev_depth - depth;
        if delta > 1 {
            number_of_gaps += 1;
            if fill_from.is_none() {
                fill_from = Some(prev_event);
            }
        }
        if delta >= 2 {
            found_big_gap = true;
            if fill_from.is_none() {
                fill_from = Some(prev_event);
            }
            break;
        }
        prev_depth = depth;
        prev_event = event_id;
    }

    if number_of_gaps < 3 && !found_big_gap {
        return Ok(vec![]);
    };
    let Some(fill_from) = fill_from else {
        return Ok(vec![]);
    };

    let admin_servers = room::admin_servers(room_id, false)?;

    let room_version = room::get_version(room_id)?;
    for backfill_server in &admin_servers {
        info!("asking {backfill_server} for backfill");
        let request = backfill_request(
            &backfill_server.origin().await,
            BackfillReqArgs {
                room_id: room_id.to_owned(),
                v: vec![fill_from.to_owned()],
                limit: (limit * 2).max(50), /* Avoid not enough filled and will get messages with
                                             * gap. */
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
                        Ok(p) => {
                            saved_pdu_contents.push(p);
                        }
                        Err(e) => {
                            warn!("failed to add backfilled pdu: {e}");
                        }
                    }
                }
                for (pdu, content) in saved_pdu_contents {
                    let event_id = pdu.event_id.clone();
                    if let Err(e) =
                        process_to_timeline_pdu(pdu, content, Some(backfill_server)).await
                    {
                        error!("failed to process backfill pdu to timeline {}", e);
                    } else {
                        let pdu = super::get_pdu(&event_id)?;
                        events.push(pdu);
                        debug!("succeed to process backfill pdu to timeline {}", event_id);
                    }
                }
                return Ok(events);
            }
            Err(e) => {
                warn!("{backfill_server} could not provide backfill: {e}");
            }
        }
    }

    info!("no servers could backfill");
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
