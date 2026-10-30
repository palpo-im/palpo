use std::collections::{BTreeMap, HashSet};

use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;

use crate::core::Seqnum;
use crate::core::events::TimelineEventType;
use crate::core::federation::backfill::{BackfillReqArgs, BackfillResBody, backfill_request};
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonObject, JsonValue, RawJsonValue};
use crate::core::state::Event;
use crate::data::connect;
use crate::data::schema::*;
use crate::event::handler::process_to_timeline_pdu;
use crate::event::{BatchToken, handler, parse_fetched_pdu};
use crate::{AppError, AppResult, GetUrlOrigin, SnPduEvent, room};

/// Rejected events and validated imported state stay outside the timeline, but
/// are still part of the known DAG. Walk their predecessors to find actual gaps
/// instead of fetching events whose data is already available locally.
pub(crate) async fn history_has_missing_predecessors(
    room_id: &RoomId,
    prev_events: &[OwnedEventId],
) -> AppResult<bool> {
    const MAX_PREDECESSORS: usize = 256;
    if prev_events.len() > MAX_PREDECESSORS {
        return Ok(true);
    }
    // The common case stays one batched metadata query. Only outlier ancestry
    // needs JSON or a graph walk.
    let mut complete = events::table
        .filter(events::room_id.eq(room_id))
        .filter(events::id.eq_any(prev_events))
        .filter(events::is_outlier.eq(false))
        .select(events::id)
        .load::<OwnedEventId>(&mut connect().await?)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    let mut pending: Vec<_> = prev_events.iter().cloned().map(|id| (id, false)).collect();
    let mut active = HashSet::new();
    let mut remaining = MAX_PREDECESSORS;
    while let Some((event_id, leaving)) = pending.pop() {
        if leaving {
            active.remove(&event_id);
            complete.insert(event_id);
            continue;
        }
        if complete.contains(&event_id) {
            continue;
        }
        if remaining == 0 || active.contains(&event_id) {
            return Ok(true);
        }
        remaining -= 1;
        let pdu = match super::get_pdu(&event_id).await {
            Ok(pdu) => pdu,
            Err(error) if error.is_not_found() => return Ok(true),
            Err(error) => return Err(error),
        };
        if pdu.room_id != room_id {
            return Ok(true);
        }
        if !pdu.is_outlier {
            complete.insert(event_id);
            continue;
        }
        if !pdu.rejected() {
            // A cached JSON row alone proves nothing about authorization. The
            // join importer records state-before only after signature/auth checks;
            // require that record and the persisted successful verdict before
            // following a state outlier. Never publish it as a timeline event.
            if pdu.soft_failed || pdu.state_key.is_none() {
                return Ok(true);
            }
            match room::state::get_pdu_before_frame_id(&event_id).await {
                Ok(_) => {}
                Err(error) if error.is_not_found() => return Ok(true),
                Err(error) => return Err(error),
            }
        }
        if pdu.prev_events.len() > remaining {
            return Ok(true);
        }
        active.insert(event_id.clone());
        pending.push((event_id, true));
        pending.extend(pdu.prev_events.iter().cloned().map(|id| (id, false)));
    }
    Ok(false)
}

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
        if history_has_missing_predecessors(room_id, &pdu.prev_events).await? {
            return backfill_from_extremities(room_id, std::slice::from_ref(&pdu.event_id), limit)
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
            .load(&mut connect().await?)
            .await?;

        let mut fill_from: Vec<OwnedEventId> = Vec::new();
        for ext_id in extremity_ids {
            let Ok(pdu) = super::get_pdu(&ext_id).await else {
                // Event isn't locally known — it's a synthetic frontier marker
                // for a parent we couldn't fetch yet. Use it as-is.
                fill_from.push(ext_id);
                continue;
            };
            if pdu.prev_events.is_empty() {
                continue;
            }
            if history_has_missing_predecessors(room_id, &pdu.prev_events).await? {
                fill_from.push(ext_id);
            }
        }

        if !fill_from.is_empty() {
            return backfill_from_extremities(room_id, &fill_from, limit).await;
        }
    }

    Ok(vec![])
}

/// Keep a backward `/messages` page contiguous after a federation backfill.
///
/// Backfilled events can sit above older local state events. If we return both
/// sides of that gap in one page, the `end` token points below the gap and the
/// next pagination misses the newly backfillable events.
pub async fn trim_pdus_until_backfill_gap(
    pdus: IndexMap<Seqnum, SnPduEvent>,
) -> AppResult<IndexMap<Seqnum, SnPduEvent>> {
    let mut trimmed = IndexMap::with_capacity(pdus.len());
    for (event_sn, pdu) in pdus {
        let has_missing_prev =
            history_has_missing_predecessors(&pdu.room_id, &pdu.prev_events).await?;

        trimmed.insert(event_sn, pdu);
        if has_missing_prev {
            break;
        }
    }
    Ok(trimmed)
}

/// Backfill events from the given backward extremities. Used when the messages
/// endpoint returns fewer events than the limit and we need to fetch history
/// from federation (e.g., after re-joining a room).
#[tracing::instrument(skip_all)]
pub async fn backfill_from_extremities(
    room_id: &RoomId,
    extremities: &[OwnedEventId],
    _limit: usize,
) -> AppResult<Vec<SnPduEvent>> {
    let mut backfill_servers = match room::admin_servers(room_id, false).await {
        Ok(servers) => servers,
        Err(error) if error.is_not_found() => Vec::new(),
        Err(error) => return Err(error),
    };
    // History can come from any participating server. In room v12 the
    // creator's power is implicit, so the explicit power-level user map may
    // contain no administrator candidates at all.
    for server in room::participating_servers(room_id, false).await? {
        if !backfill_servers.contains(&server) {
            backfill_servers.push(server);
        }
    }
    let room_version = room::get_version(room_id).await?;

    for backfill_server in &backfill_servers {
        info!("asking {backfill_server} for backfill from extremities");
        let request = backfill_request(
            &backfill_server.origin().await,
            BackfillReqArgs {
                room_id: room_id.to_owned(),
                v: extremities.to_vec(),
                // Synapse caps `/backfill` at 100 events per call. Use that
                // bounded batch size regardless of the client page size: small
                // `/messages` pages should still fetch enough history to
                // continue locally, while large pages should not block on more
                // than one federation batch.
                limit: 100,
            },
        )?
        .into_inner();
        let response =
            match crate::sending::send_federation_request(backfill_server, request, None).await {
                Ok(response) => response.json::<BackfillResBody>().await.map_err(Into::into),
                Err(error) => Err(error),
            };
        match response {
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
                    } else if let Ok(pdu) = super::get_pdu(&event_id).await {
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
    if let Some(pdu) = super::get_non_outlier_pdu(&event_id).await? {
        info!("we already know {event_id}, skipping backfill");
        let value = super::get_pdu_json(&event_id)
            .await?
            .ok_or_else(|| AppError::public("event json not found"))?;
        return Ok((pdu, value));
    }
    let Some(outlier_pdu) =
        handler::process_to_outlier_pdu(remote_server, &event_id, room_id, room_version, value)
            .await?
    else {
        return Err(AppError::internal("failed to process backfilled pdu"));
    };
    let (pdu, value, _) = outlier_pdu.save_to_database(true).await?;

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

/// History joins the timeline without publishing its old memberships or room
/// state, generating notifications, executing commands, or extending stickiness.
pub(crate) async fn append_backfilled_pdu(
    pdu: &SnPduEvent,
    json: CanonicalJsonObject,
) -> AppResult<()> {
    let before = room::state::get_pdu_before_frame_id(&pdu.event_id).await?;
    let after = if let Some(key) = &pdu.state_key {
        let field = room::state::ensure_field_id(&pdu.event_ty.to_string().into(), key).await?;
        let mut state = room::state::get_full_state_ids(before).await?;
        state.insert(field, pdu.event_id.clone());
        let mut compressed = room::state::CompressedState::new();
        for (field, id) in state {
            let event = super::get_pdu(&id).await?;
            compressed.insert(room::state::CompressedEvent::new(field, event.event_sn));
        }
        let hash = crate::utils::hash_keys(compressed.iter().map(|event| &event[..]));
        match room::state::get_frame_id(&pdu.room_id, &hash).await {
            Ok(frame) => frame,
            Err(error) if error.is_not_found() => {
                let frame = room::state::ensure_frame(&pdu.room_id, hash).await?;
                room::state::save_state_delta(
                    &pdu.room_id,
                    frame,
                    room::state::StateDiff {
                        parent_id: None,
                        appended: std::sync::Arc::new(compressed),
                        disposed: std::sync::Arc::new(room::state::CompressedState::new()),
                    },
                )
                .await?;
                frame
            }
            Err(error) => return Err(error),
        }
    } else {
        before
    };
    let data = crate::data::room::DbEventData {
        event_id: pdu.event_id.clone(),
        event_sn: pdu.event_sn,
        room_id: pdu.room_id.clone(),
        internal_metadata: None,
        json_data: serde_json::to_value(&json)?,
        format_version: None,
    };
    let promoted = connect()
        .await?
        .transaction::<_, AppError, _>(async |conn| {
            let (outlier, rejected) = events::table
                .find(&pdu.event_id)
                .select((events::is_outlier, events::is_rejected))
                .for_update()
                .first::<(bool, bool)>(conn)
                .await?;
            if rejected {
                return Err(AppError::internal(
                    "cannot promote a rejected backfill event",
                ));
            }
            if !outlier {
                return Ok(false);
            }
            diesel::update(event_points::table.find(&pdu.event_id))
                .set(event_points::frame_id.eq(after))
                .execute(conn)
                .await?;
            data.save_with_conn(conn).await?;
            diesel::update(events::table.find(&pdu.event_id))
                .set((
                    events::is_outlier.eq(false),
                    events::soft_failed.eq(false),
                    events::stream_ordering.eq(-pdu.event_sn.abs()),
                ))
                .execute(conn)
                .await?;
            diesel::delete(
                event_stickies::table.filter(event_stickies::event_id.eq(&pdu.event_id)),
            )
            .execute(conn)
            .await?;
            Ok(true)
        })
        .await?;
    if promoted {
        for prev in &pdu.prev_events {
            crate::data::room::NewDbEventEdge {
                room_id: pdu.room_id.clone(),
                event_depth: pdu.depth as i64,
                event_id: pdu.event_id.clone(),
                event_sn: pdu.event_sn,
                prev_id: prev.clone(),
            }
            .save()
            .await?;
        }
        crate::event::search::save_pdu(pdu, &json).await?;
        let mut promoted = pdu.clone();
        promoted.is_outlier = false;
        room::state::update_backward_extremities(&promoted).await?;
    }
    Ok(())
}

#[cfg(test)]
mod history_database_tests {
    use super::*;
    use crate::data::room::{DbEventData, NewDbEvent};
    use crate::event::PduEvent;

    async fn save_event(
        room_id: &RoomId,
        event_id: &str,
        prev_events: &[&str],
        outlier: bool,
        rejected: bool,
    ) -> SnPduEvent {
        let mut pdu: PduEvent = serde_json::from_value(serde_json::json!({
            "event_id": event_id, "room_id": room_id, "sender": "@alice:example.org",
            "type": "m.room.message", "content": {"body": "history", "msgtype": "m.text"},
            "origin_server_ts": 1, "depth": 3, "hashes": {"sha256": ""},
            "prev_events": prev_events
        }))
        .unwrap();
        if rejected {
            pdu.rejection_reason = Some("rejected auth event".to_owned());
        }
        let json = crate::core::serde::to_canonical_object(&pdu).unwrap();
        let (sn, _guard) = crate::event::ensure_event_sn(room_id, &pdu.event_id)
            .await
            .unwrap();
        let mut event =
            NewDbEvent::from_canonical_json_with_room_id(&pdu.event_id, sn, &json, false, room_id)
                .unwrap();
        event.is_outlier = outlier;
        event.is_rejected = rejected;
        event.soft_failed = rejected;
        event.rejection_reason = pdu.rejection_reason.clone();
        let mut conn = connect().await.unwrap();
        event.save_with_conn(&mut conn).await.unwrap();
        DbEventData {
            event_id: pdu.event_id.clone(),
            event_sn: sn,
            room_id: room_id.to_owned(),
            internal_metadata: None,
            json_data: serde_json::to_value(json).unwrap(),
            format_version: None,
        }
        .save_with_conn(&mut conn)
        .await
        .unwrap();
        SnPduEvent::new(pdu, sn, outlier, rejected, false)
    }

    async fn save_imported_state(
        room_id: &RoomId,
        event_id: &str,
        prev_events: &[&str],
        state_key: &str,
        validated: bool,
    ) -> SnPduEvent {
        let mut pdu = save_event(room_id, event_id, prev_events, true, false).await;
        pdu.pdu.event_ty = "my.room.breaker".into();
        pdu.pdu.state_key = Some(state_key.to_owned());
        let json = crate::core::serde::to_canonical_object(&pdu.pdu).unwrap();
        let mut conn = connect().await.unwrap();
        diesel::update(events::table.find(&pdu.event_id))
            .set((
                events::ty.eq(pdu.event_ty.to_string()),
                events::state_key.eq(&pdu.state_key),
            ))
            .execute(&mut conn)
            .await
            .unwrap();
        DbEventData {
            event_id: pdu.event_id.clone(),
            event_sn: pdu.event_sn,
            room_id: room_id.to_owned(),
            internal_metadata: None,
            json_data: serde_json::to_value(json).unwrap(),
            format_version: None,
        }
        .save_with_conn(&mut conn)
        .await
        .unwrap();
        drop(conn);
        if validated {
            // Reproduce the historical validation marker written by the join
            // importer; no current room state or membership is published.
            room::state::set_event_state_before(
                &pdu.event_id,
                room_id,
                std::sync::Arc::new(room::state::CompressedState::new()),
            )
            .await
            .unwrap();
        }
        pdu
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_rejected_history_is_known_but_unknown_ancestors_remain_gaps() {
        crate::test_database::init();
        let room: OwnedRoomId = "!rejected-history:example.org".try_into().unwrap();
        let root = save_event(&room, "$history-root", &[], false, false).await;
        let rejected = save_event(&room, "$history-rejected", &["$history-root"], true, true).await;
        let sentinel = save_event(
            &room,
            "$history-sentinel",
            &["$history-rejected"],
            false,
            false,
        )
        .await;
        room::state::update_backward_extremities(&rejected)
            .await
            .unwrap();
        // The old query reports no known predecessor despite its stored verdict and JSON.
        let old_known_count = events::table
            .filter(events::id.eq_any(&sentinel.prev_events))
            .filter(events::is_outlier.eq(false))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(old_known_count, 0);
        assert!(
            !history_has_missing_predecessors(&room, &sentinel.prev_events)
                .await
                .unwrap()
        );
        room::state::update_backward_extremities(&sentinel)
            .await
            .unwrap();
        let gaps = timeline_gaps::table
            .filter(timeline_gaps::event_id.eq(&sentinel.event_id))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            gaps, 0,
            "a known rejected predecessor is not missing history"
        );
        let mut page = IndexMap::new();
        page.insert(sentinel.event_sn, sentinel.clone());
        assert!(
            backfill_if_required(&room, &sentinel.historic_token(), &page, 10)
                .await
                .unwrap()
                .is_empty()
        );
        page.insert(root.event_sn, root);
        assert_eq!(trim_pdus_until_backfill_gap(page).await.unwrap().len(), 2);
        assert!(
            super::super::get_pdu(&rejected.event_id)
                .await
                .unwrap()
                .rejected()
        );
        assert!(
            super::super::get_non_outlier_pdu(&rejected.event_id)
                .await
                .unwrap()
                .is_none()
        );

        let incomplete = save_event(
            &room,
            "$history-incomplete",
            &["$history-unknown"],
            true,
            true,
        )
        .await;
        let later = save_event(
            &room,
            "$history-later",
            &["$history-incomplete"],
            false,
            false,
        )
        .await;
        assert!(
            history_has_missing_predecessors(&room, &later.prev_events)
                .await
                .unwrap()
        );
        room::state::update_backward_extremities(&later)
            .await
            .unwrap();
        let gaps = timeline_gaps::table
            .filter(timeline_gaps::event_id.eq(&later.event_id))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            gaps, 1,
            "unknown history behind a rejected event remains fetchable"
        );
        assert!(
            history_has_missing_predecessors(&room, std::slice::from_ref(&incomplete.event_id))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_validated_state_history_avoids_backfill_without_hiding_real_gaps() {
        crate::test_database::init();
        let room: OwnedRoomId = "!known-state-history:example.org".try_into().unwrap();
        save_event(&room, "$known-state-root", &[], false, false).await;
        let long_key = "💥".repeat(70);
        assert_eq!(long_key.len(), 280);
        assert_eq!(long_key.chars().count(), 70);
        let state = save_imported_state(
            &room,
            "$known-state-import",
            &["$known-state-root"],
            &long_key,
            true,
        )
        .await;
        let join = save_event(
            &room,
            "$known-state-join",
            &["$known-state-import"],
            false,
            false,
        )
        .await;
        let old_known = events::table
            .filter(events::id.eq_any(&join.prev_events))
            .filter(events::is_outlier.eq(false))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            old_known, 0,
            "the old timeline-only query invents a gap at the join"
        );
        assert!(
            !history_has_missing_predecessors(&room, &join.prev_events)
                .await
                .unwrap()
        );
        room::state::update_backward_extremities(&state)
            .await
            .unwrap();
        room::state::update_backward_extremities(&join)
            .await
            .unwrap();
        let mut page = IndexMap::new();
        page.insert(join.event_sn, join.clone());
        assert!(
            backfill_if_required(&room, &join.historic_token(), &page, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            super::super::get_pdu(&state.event_id)
                .await
                .unwrap()
                .is_outlier
        );
        assert!(room::state::get_room_frame_id(&room, None).await.is_err());
        assert_eq!(
            timeline_gaps::table
                .filter(timeline_gaps::event_id.eq(&join.event_id))
                .count()
                .get_result::<i64>(&mut connect().await.unwrap())
                .await
                .unwrap(),
            0
        );

        let unvalidated =
            save_imported_state(&room, "$known-state-raw", &["$known-state-root"], "", false).await;
        assert!(
            history_has_missing_predecessors(&room, std::slice::from_ref(&unvalidated.event_id))
                .await
                .unwrap()
        );
        diesel::update(events::table.find(&state.event_id))
            .set(events::soft_failed.eq(true))
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert!(
            history_has_missing_predecessors(&room, &join.prev_events)
                .await
                .unwrap()
        );
        diesel::update(events::table.find(&state.event_id))
            .set(events::soft_failed.eq(false))
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();

        let missing = save_imported_state(
            &room,
            "$known-state-incomplete",
            &["$known-state-unknown"],
            "",
            true,
        )
        .await;
        let later = save_event(
            &room,
            "$known-state-later",
            &["$known-state-incomplete"],
            false,
            false,
        )
        .await;
        assert!(
            history_has_missing_predecessors(&room, &later.prev_events)
                .await
                .unwrap()
        );
        room::state::update_backward_extremities(&later)
            .await
            .unwrap();
        assert_eq!(
            timeline_gaps::table
                .filter(timeline_gaps::event_id.eq(&later.event_id))
                .count()
                .get_result::<i64>(&mut connect().await.unwrap())
                .await
                .unwrap(),
            1
        );
        assert!(
            history_has_missing_predecessors(&room, std::slice::from_ref(&missing.event_id))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_known_state_history_is_cycle_safe_room_scoped_and_bounded() {
        crate::test_database::init();
        let room: OwnedRoomId = "!known-state-boundaries:example.org".try_into().unwrap();
        let foreign_room: OwnedRoomId = "!known-state-foreign:example.org".try_into().unwrap();
        let root = save_event(&room, "$history-bound-root", &[], false, false).await;
        let foreign = save_event(&foreign_room, "$history-bound-foreign", &[], false, false).await;
        assert!(
            history_has_missing_predecessors(&room, std::slice::from_ref(&foreign.event_id))
                .await
                .unwrap()
        );
        let a =
            save_imported_state(&room, "$history-cycle-a", &["$history-cycle-b"], "", true).await;
        save_imported_state(&room, "$history-cycle-b", &["$history-cycle-a"], "", true).await;
        assert!(
            history_has_missing_predecessors(&room, std::slice::from_ref(&a.event_id))
                .await
                .unwrap()
        );
        let left = save_imported_state(
            &room,
            "$history-diamond-left",
            &["$history-bound-root"],
            "",
            true,
        )
        .await;
        let right = save_imported_state(
            &room,
            "$history-diamond-right",
            &["$history-bound-root"],
            "",
            true,
        )
        .await;
        assert!(
            !history_has_missing_predecessors(
                &room,
                &[left.event_id.clone(), right.event_id.clone()]
            )
            .await
            .unwrap()
        );
        assert!(
            !history_has_missing_predecessors(&room, &vec![root.event_id.clone(); 256])
                .await
                .unwrap()
        );
        assert!(
            history_has_missing_predecessors(&room, &vec![root.event_id.clone(); 257])
                .await
                .unwrap()
        );
        let mut previous = root.event_id.clone();
        let mut heads = Vec::new();
        for index in 0..256 {
            let event = save_imported_state(
                &room,
                &format!("$history-budget-{index}"),
                &[previous.as_str()],
                "",
                true,
            )
            .await;
            previous = event.event_id.clone();
            heads.push(previous.clone());
        }
        assert!(
            !history_has_missing_predecessors(&room, std::slice::from_ref(&heads[254]))
                .await
                .unwrap()
        );
        assert!(
            history_has_missing_predecessors(&room, std::slice::from_ref(&heads[255]))
                .await
                .unwrap()
        );
    }
}
