//! MSC2444 ongoing federated peeking.
//!
//! Resident side (this file, Phase C): track which remote servers hold a live
//! peek on our rooms and expose them as extra event-distribution destinations.
//! Peeking side (added below): establish/renew/cancel an outbound peek on a
//! remote room and ingest its state + ongoing events into the local store.

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;

use crate::core::UnixMillis;
use crate::core::federation::peek::{PeekStartResBody, peek_cancel_request, peek_start_request};
use crate::core::identifiers::*;
use crate::core::serde::CanonicalJsonObject;
use crate::data::room::{DbEventData, NewDbEvent};
use crate::event::handler::process_incoming_pdu;
use crate::event::{ensure_event_sn, parse_fetched_pdu};
use crate::room::state::{CompressedEvent, DeltaInfo};
use crate::room::{state, timeline};
use crate::{
    AppError, AppResult, GetUrlOrigin, OptionalExtension, SnPduEvent, config, data, room, sending,
};

/// How long a peek subscription is valid before the peeking server must renew.
/// Returned to peers as `renewal_interval`.
pub const PEEK_RENEWAL_INTERVAL_MS: u64 = 3_600_000; // 1 hour

/// Current time in unix milliseconds as an `i64` (the column type).
fn now_ms() -> i64 {
    UnixMillis::now().get() as i64
}

// ---------------------------------------------------------------------------
// Resident side: remote servers peeking our rooms.
// ---------------------------------------------------------------------------

/// Register or renew a remote server's peek on one of our rooms, valid for
/// `PEEK_RENEWAL_INTERVAL_MS` from now.
pub async fn register_peeking_server(
    room_id: &RoomId,
    server: &ServerName,
    peek_id: &str,
) -> crate::AppResult<()> {
    let renew_at = now_ms() + PEEK_RENEWAL_INTERVAL_MS as i64;
    data::room::peek::upsert_peeking_server(room_id, server, peek_id, renew_at).await?;
    Ok(())
}

/// Cancel a remote server's peek on one of our rooms.
pub async fn unregister_peeking_server(
    room_id: &RoomId,
    server: &ServerName,
    peek_id: &str,
) -> crate::AppResult<()> {
    data::room::peek::remove_peeking_server(room_id, server, peek_id).await?;
    Ok(())
}

/// Remote servers with a live (non-expired) peek on this room. These receive new
/// room events in addition to the room's participating servers. Errors are
/// swallowed to a empty list so a peek-tracking hiccup never blocks delivery to
/// real members.
pub async fn active_peeking_servers(room_id: &RoomId) -> Vec<OwnedServerName> {
    data::room::peek::active_peeking_servers(room_id, now_ms())
        .await
        .unwrap_or_default()
}

/// Drop every peek subscription (held by remote servers on our rooms) that has
/// lapsed. Safe to call periodically; returns rows removed.
pub async fn purge_expired_peeking_servers() -> crate::AppResult<usize> {
    Ok(data::room::peek::purge_expired_peeking_servers(now_ms()).await?)
}

// ---------------------------------------------------------------------------
// Peeking side: outbound peeks we hold on remote rooms.
// ---------------------------------------------------------------------------

/// Start (or renew, when `peek_id` is reused) an outbound peek on a remote room.
///
/// PUTs `/_matrix/federation/v1/peek/{room_id}/{peek_id}` to `target_server`,
/// ingests the returned current state + auth chain + recent timeline into the
/// local store (mirroring the send_join ingestion path), and records the peek so
/// the background task renews it. After this returns, ongoing room events arrive
/// via the normal `/send` transaction path and are appended locally.
///
/// NOTE: This path needs validation against a live two-server federation setup;
/// it compiles and mirrors the join ingestion but has not been exercised
/// end-to-end here.
pub async fn start_peek(
    room_id: &RoomId,
    target_server: &ServerName,
    peek_id: &str,
) -> AppResult<()> {
    let request = peek_start_request(&target_server.origin().await, room_id, peek_id, &[])
        .map_err(|e| AppError::public(format!("failed to build peek request: {e}")))?
        .into_inner();

    let body = sending::send_federation_request(target_server, request, None)
        .await?
        .json::<PeekStartResBody>()
        .await?;

    let room_version = body.room_version.clone();
    room::ensure_room(room_id, &room_version).await?;

    // Record the peek BEFORE ingesting anything. The incoming-PDU gate accepts
    // events for a non-joined room only when `is_peeked` is true, so the seed
    // messages below (and any /send events racing in during ingestion) would be
    // dropped if the row didn't exist yet. Renew at half the resident's interval.
    let renew_at = now_ms() + (body.renewal_interval as i64 / 2).max(1);
    data::room::peek::upsert_peek(room_id, peek_id, target_server, renew_at).await?;

    // Ingest the snapshot. On failure, roll back the registration we just wrote
    // so we don't leave an ownerless federation subscription that the
    // maintenance task renews forever (and that the incoming-PDU gate would keep
    // accepting events for).
    if let Err(e) = ingest_peek_snapshot(room_id, target_server, &room_version, &body).await {
        let _ = data::room::peek::remove_peek(room_id).await;
        return Err(e);
    }
    Ok(())
}

/// Ingest a peek snapshot (auth chain + current state + recent messages) into
/// the local store, mirroring the send_join ingestion path. Called by
/// `start_peek` only after the `room_peeks` row exists, so the incoming-PDU gate
/// accepts the seed events.
async fn ingest_peek_snapshot(
    room_id: &RoomId,
    target_server: &ServerName,
    room_version: &RoomVersionId,
    body: &PeekStartResBody,
) -> AppResult<()> {
    // Make sure we hold the signing keys needed to verify the returned events.
    crate::server_key::acquire_events_pubkeys(body.auth_chain.iter().chain(body.state.iter()))
        .await;

    // Store auth chain + state events (as the join path does), in topological
    // (depth) order so each event's ancestors are present first.
    let mut parsed: IndexMap<OwnedEventId, CanonicalJsonObject> = IndexMap::new();
    for raw in body.auth_chain.iter().chain(body.state.iter()) {
        if let Ok((event_id, value)) = parse_fetched_pdu(room_id, room_version, raw) {
            parsed.insert(event_id, value);
        }
    }
    let mut ordered: Vec<_> = parsed.into_iter().collect();
    ordered.sort_by_key(|(_, v)| v.get("depth").and_then(|d| d.as_integer()).unwrap_or(0));
    for (event_id, value) in &ordered {
        if let Err(e) = process_incoming_pdu(
            target_server,
            event_id,
            room_id,
            room_version,
            value.clone(),
            true,
            false,
        )
        .await
        {
            warn!("peek: failed to process state/auth event {event_id}: {e}");
        }
    }

    // Build the resolved current state map and persist it as the room's state.
    let mut state_map = HashMap::new();
    for raw in &body.state {
        let (event_id, value) = match parse_fetched_pdu(room_id, room_version, raw) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let pdu = if let Some(pdu) = timeline::get_pdu(&event_id).await.optional()? {
            pdu
        } else {
            let (event_sn, event_guard) = ensure_event_sn(room_id, &event_id).await?;
            let pdu = SnPduEvent::from_canonical_object(
                room_id, &event_id, event_sn, value.clone(), false, false, false,
            )
            .map_err(|e| {
                warn!("peek: invalid state pdu {event_id}: {e}");
                AppError::public("invalid state pdu in peek response")
            })?;
            NewDbEvent::from_canonical_json_with_room_id(&event_id, event_sn, &value, false, room_id)?
                .save()
                .await?;
            DbEventData {
                event_id: event_id.clone(),
                event_sn,
                room_id: room_id.to_owned(),
                internal_metadata: None,
                json_data: serde_json::to_value(&value)?,
                format_version: None,
            }
            .save()
            .await?;
            drop(event_guard);
            pdu
        };

        if let Some(state_key) = &pdu.state_key {
            let state_key_id =
                state::ensure_field_id(&pdu.event_ty.to_string().into(), state_key).await?;
            state_map.insert(state_key_id, (pdu.event_id.clone(), pdu.event_sn));
        }
    }

    let state_lock = room::lock_state(room_id).await;
    let DeltaInfo {
        frame_id,
        appended,
        disposed,
    } = state::save_state(
        room_id,
        Arc::new(
            state_map
                .into_iter()
                .map(|(k, (_event_id, event_sn))| Ok(CompressedEvent::new(k, event_sn)))
                .collect::<AppResult<_>>()?,
        ),
    )
    .await?;
    state::force_state(room_id, frame_id, appended, disposed).await?;
    state::set_room_state(room_id, frame_id).await?;
    drop(state_lock);

    // Seed recent timeline (best effort); ongoing events arrive via /send.
    let mut msgs: Vec<_> = body
        .messages
        .iter()
        .filter_map(|raw| parse_fetched_pdu(room_id, room_version, raw).ok())
        .collect();
    msgs.sort_by_key(|(_, v)| v.get("depth").and_then(|d| d.as_integer()).unwrap_or(0));
    for (event_id, value) in &msgs {
        let _ = process_incoming_pdu(
            target_server,
            event_id,
            room_id,
            room_version,
            value.clone(),
            true,
            false,
        )
        .await;
    }

    Ok(())
}

/// Renew an existing outbound peek without re-ingesting room state. The
/// subscription is already live and events flow via `/send`; this is just a
/// heartbeat that re-registers us with the resident and pushes the renewal
/// deadline back. If the resident refuses (e.g. the room is no longer
/// world-readable) this errors and the caller drops the peek.
async fn renew_peek(room_id: &RoomId, target_server: &ServerName, peek_id: &str) -> AppResult<()> {
    let request = peek_start_request(&target_server.origin().await, room_id, peek_id, &[])
        .map_err(|e| AppError::public(format!("failed to build peek renewal: {e}")))?
        .into_inner();
    let body = sending::send_federation_request(target_server, request, None)
        .await?
        .json::<PeekStartResBody>()
        .await?;
    let renew_at = now_ms() + (body.renewal_interval as i64 / 2).max(1);
    data::room::peek::upsert_peek(room_id, peek_id, target_server, renew_at).await?;
    Ok(())
}

/// Ensure we hold a live peek on a remote room: no-op if one is already active
/// or if our server already participates (a local user is joined, so we receive
/// events as a member); otherwise establishes a new peek. The room's home server
/// (from the room id) is the peek target.
pub async fn ensure_peek(room_id: &RoomId) -> AppResult<()> {
    if data::room::peek::is_peeked(room_id).await? {
        // Already peeking; renewal is handled by the background task.
        return Ok(());
    }
    if room::is_server_joined(&config::get().server_name, room_id)
        .await
        .unwrap_or(false)
    {
        // We already receive this room's events as a participating server.
        return Ok(());
    }
    let target = room_id
        .server_name()
        .map_err(|_| AppError::public("room id has no server name to peek via"))?;
    let peek_id = new_peek_id();
    start_peek(room_id, target, &peek_id).await
}

/// Cancel an outbound peek and stop receiving the room's events.
pub async fn stop_peek(room_id: &RoomId) -> AppResult<()> {
    if let Some(peek) = data::room::peek::get_peek(room_id).await? {
        if let Ok(request) =
            peek_cancel_request(&peek.target_server.origin().await, room_id, &peek.peek_id)
        {
            // Best effort: tell the resident server to drop us.
            let _ =
                sending::send_federation_request(&peek.target_server, request.into_inner(), None)
                    .await;
        }
        data::room::peek::remove_peek(room_id).await?;
    }
    Ok(())
}

/// Renew all outbound peeks whose renewal deadline has passed. On repeated
/// failure the peek is dropped so we stop trying. Also purges lapsed inbound
/// peekers. Intended to be called periodically by a background task.
pub async fn run_maintenance() {
    if let Err(e) = purge_expired_peeking_servers().await {
        warn!("peek: failed to purge expired peeking servers: {e}");
    }

    // Tear down federation peeks no local user device wants anymore (e.g. every
    // peeker was dropped during sync because the room stopped being
    // world-readable, or all users unpeeked). Prevents ownerless subscriptions
    // from being renewed forever.
    for room_id in data::room::peek::peeked_room_ids().await.unwrap_or_default() {
        if data::room::peek::room_peeker_count(&room_id).await.unwrap_or(1) == 0 {
            let _ = stop_peek(&room_id).await;
        }
    }

    let due = match data::room::peek::peeks_due_for_renewal(now_ms()).await {
        Ok(due) => due,
        Err(e) => {
            warn!("peek: failed to load peeks due for renewal: {e}");
            return;
        }
    };
    for peek in due {
        if renew_peek(&peek.room_id, &peek.target_server, &peek.peek_id)
            .await
            .is_ok()
        {
            continue;
        }
        // Lightweight renewal failed. It may be transient (a network blip) or
        // the resident may have forgotten us, so attempt a full re-establish,
        // which re-registers and re-ingests current state.
        if let Err(e) = start_peek(&peek.room_id, &peek.target_server, &peek.peek_id).await {
            // Still failing — give up. Drop the federation subscription AND the
            // per-device user peeks for this room, so a dead peek stops being
            // surfaced in sync (rather than freezing on stale state forever).
            warn!(
                "peek: renewal and re-establish of {} via {} both failed, dropping: {e}",
                peek.room_id, peek.target_server
            );
            let _ = data::room::peek::remove_peek(&peek.room_id).await;
            let _ = data::room::peek::remove_room_user_peeks(&peek.room_id).await;
        }
    }
}

/// Generate an opaque peek id.
fn new_peek_id() -> String {
    format!("peek_{}", ulid::Ulid::new())
}
