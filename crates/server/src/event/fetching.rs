use std::collections::HashSet;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use indexmap::IndexMap;
use salvo::http::StatusError;

use crate::core::federation::authorization::{EventAuthResBody, event_auth_request};
use crate::core::federation::event::{
    EventReqArgs, EventResBody, MissingEventsReqBody, MissingEventsResBody,
    RoomStateAtEventReqArgs, RoomStateIdsResBody, RoomStateReqArgs, RoomStateResBody,
    event_request, missing_events_request, room_state_ids_request, room_state_request,
};
use crate::core::identifiers::*;
use crate::core::state::Event;
use crate::data::diesel_exists;
use crate::data::schema::*;
use crate::event::handler::{process_pulled_pdu_with_pending_state, process_to_outlier_pdu};
use crate::event::{connect, parse_fetched_pdu, seen_event_ids};
use crate::exts::*;
use crate::room::state::ensure_field_id;
use crate::room::timeline;
use crate::sending::send_federation_request;
use crate::{AppError, AppResult, OptionalExtension as _, PduEvent, SnPduEvent, room};

pub struct FetchedState {
    pub state_events: IndexMap<i64, OwnedEventId>,
    pub auth_events: IndexMap<i64, OwnedEventId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingEventsRecovery {
    Complete,
    /// Every missing immediate predecessor was supplied and signature-checked
    /// in this response, but its earlier state/auth still needs recovery. This
    /// permits the existing state recovery path, never timeline acceptance.
    PendingState,
    Incomplete,
}

async fn recovered_predecessors(
    room_id: &RoomId,
    required: &[OwnedEventId],
) -> AppResult<HashSet<OwnedEventId>> {
    Ok(events::table
        .inner_join(
            event_datas::table.on(event_datas::event_id
                .eq(events::id)
                .and(event_datas::room_id.eq(events::room_id))),
        )
        .filter(events::room_id.eq(room_id))
        .filter(events::id.eq_any(required))
        .filter(events::is_outlier.eq(false))
        .filter(events::is_rejected.eq(false))
        .filter(events::soft_failed.eq(false))
        .select(events::id)
        .load::<OwnedEventId>(&mut connect().await?)
        .await?
        .into_iter()
        .collect())
}

pub async fn fetch_and_process_missing_events(
    remote_server: &ServerName,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    incoming_pdu: &PduEvent,
    is_backfill: bool,
) -> AppResult<MissingEventsRecovery> {
    let required = &incoming_pdu.prev_events;
    let recovered = recovered_predecessors(room_id, required).await?;
    if required.iter().all(|id| recovered.contains(id)) {
        return Ok(MissingEventsRecovery::Complete);
    }
    // Imported state and rejected predecessors can have a usable historical
    // state without being timeline anchors. Let the live-event guard authorize
    // that reconnect locally, instead of fetching the already-known history.
    let cached = events::table
        .filter(events::room_id.eq(room_id))
        .filter(events::id.eq_any(required))
        .select(events::id)
        .load::<OwnedEventId>(&mut connect().await?)
        .await?;
    if required.iter().all(|id| cached.contains(id))
        && crate::event::resolver::resolve_state_at_incoming(
            incoming_pdu,
            &crate::room::get_version_rules(room_version)?,
        )
        .await?
        .is_some()
    {
        return Ok(MissingEventsRecovery::Incomplete);
    }
    let min_depth = timeline::first_pdu_in_room(room_id)
        .await?
        .map(|p| p.depth)
        .unwrap_or(0);

    let mut earliest_events = Vec::new();
    for event_id in room::state::get_forward_extremities(room_id).await? {
        let Some(pdu) = timeline::get_pdu(&event_id).await.optional()? else {
            continue;
        };
        if !pdu.is_outlier && !pdu.soft_failed && !pdu.rejected() {
            earliest_events.push(event_id);
        }
    }
    let request = missing_events_request(
        &remote_server.origin().await,
        room_id,
        MissingEventsReqBody {
            limit: 10,
            min_depth,
            earliest_events,
            latest_events: vec![incoming_pdu.event_id.clone()],
        },
    )?
    .into_inner();

    let response = send_federation_request(remote_server, request, None).await?;
    let res_body = response.json::<MissingEventsResBody>().await?;

    process_missing_events_response(
        remote_server,
        room_id,
        room_version,
        is_backfill,
        res_body,
        required,
    )
    .await
}

pub(super) async fn process_missing_events_response(
    remote_server: &ServerName,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    is_backfill: bool,
    res_body: MissingEventsResBody,
    required: &[OwnedEventId],
) -> AppResult<MissingEventsRecovery> {
    let mut fetched_events = IndexMap::with_capacity(res_body.events.len());

    for event in res_body.events {
        let (event_id, event_val) = parse_fetched_pdu(room_id, room_version, &event)?;
        if fetched_events.contains_key(&event_id) {
            continue;
        }
        let cached = events::table
            .filter(events::room_id.eq(room_id))
            .filter(events::id.eq(&event_id))
            .select((events::is_outlier, events::is_rejected, events::soft_failed))
            .first::<(bool, bool, bool)>(&mut connect().await?)
            .await
            .optional()?;
        // Rejected events are immutable. A cached, unvalidated outlier must
        // still go through pulled-event processing before it can be an anchor.
        if cached
            .is_some_and(|(outlier, rejected, soft_failed)| rejected || (!outlier && !soft_failed))
        {
            continue;
        }
        fetched_events.insert(event_id.clone(), event_val);
    }

    fetched_events.sort_by(|_x1, v1, _k2, v2| {
        let depth1 = v1.get("depth").and_then(|v| v.as_integer()).unwrap_or(0);
        let depth2 = v2.get("depth").and_then(|v| v.as_integer()).unwrap_or(0);
        depth1.cmp(&depth2)
    });
    let mut pending_state = HashSet::new();
    for (event_id, event_val) in fetched_events {
        match process_pulled_pdu_with_pending_state(
            remote_server,
            &event_id,
            room_id,
            room_version,
            event_val.clone(),
            is_backfill,
        )
        .await
        {
            Ok(true) => {
                pending_state.insert(event_id);
            }
            Ok(false) => {}
            Err(e) => {
                if matches!(
                    e,
                    AppError::Data(_) | AppError::Diesel(_) | AppError::Pool(_)
                ) {
                    return Err(e);
                }
                error!(
                    "failed to process fetched missing prev event {}: {}",
                    event_id, e
                );
            }
        }
    }
    // HTTP success (including an empty or partial body) is not DAG recovery.
    // Recheck every required predecessor, also observing concurrent recovery.
    let recovered = recovered_predecessors(room_id, required).await?;
    if required.iter().all(|id| recovered.contains(id)) {
        return Ok(MissingEventsRecovery::Complete);
    }
    // A cached outlier alone proves nothing. Only this response's successfully
    // processed pending events can permit snapshot recovery, and their payload
    // and non-rejected pending metadata must still exist in this room.
    let pending_state: HashSet<OwnedEventId> = events::table
        .inner_join(
            event_datas::table.on(event_datas::event_id
                .eq(events::id)
                .and(event_datas::room_id.eq(events::room_id))),
        )
        .filter(events::room_id.eq(room_id))
        .filter(events::id.eq_any(pending_state))
        .filter(events::is_outlier.eq(true))
        .filter(events::is_rejected.eq(false))
        .filter(events::soft_failed.eq(true))
        .select(events::id)
        .load::<OwnedEventId>(&mut connect().await?)
        .await?
        .into_iter()
        .collect();
    Ok(
        if required
            .iter()
            .all(|id| recovered.contains(id) || pending_state.contains(id))
        {
            MissingEventsRecovery::PendingState
        } else {
            MissingEventsRecovery::Incomplete
        },
    )
}

pub async fn fetch_and_process_auth_chain(
    remote_server: &ServerName,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    event_id: &EventId,
) -> AppResult<Vec<SnPduEvent>> {
    let request =
        event_auth_request(&remote_server.origin().await, room_id, event_id)?.into_inner();
    let response = send_federation_request(remote_server, request, None).await?;
    if !response.status().is_success()
        && let Some(status) = StatusError::from_code(response.status())
    {
        return Err(status.into());
    }
    let res_body = response.json::<EventAuthResBody>().await?;
    let mut auth_events = Vec::new();
    for event in res_body.auth_chain {
        let (event_id, event_value) =
            crate::event::parse_fetched_pdu(room_id, room_version, &event)?;
        if let Ok(pdu) = timeline::get_pdu(&event_id).await {
            auth_events.push(pdu);
            continue;
        }
        if !diesel_exists!(
            events::table
                .filter(events::id.eq(&event_id))
                .filter(events::room_id.eq(&room_id)),
            &mut connect().await?
        )? {
            let Some(outlier_pdu) = process_to_outlier_pdu(
                remote_server,
                &event_id,
                room_id,
                room_version,
                event_value,
            )
            .await?
            else {
                continue;
            };
            let pdu = outlier_pdu.save_to_database(true).await?.0;
            auth_events.push(pdu);
        }
    }
    Ok(auth_events)
}

/// Call /state_ids to find out what the state at this pdu is. We trust the
/// server's response to some extend (sic), but we still do a lot of checks
/// on the events
pub(super) async fn fetch_and_process_missing_state_by_ids(
    remote_server: &ServerName,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    event_id: &EventId,
) -> AppResult<Vec<OwnedEventId>> {
    debug!("calling /state_ids");
    // Call /state_ids to find out what the state at this pdu is. We trust the server's
    // response to some extend, but we still do a lot of checks on the events
    let RoomStateIdsResBody {
        pdu_ids,
        auth_chain_ids,
    } = fetch_state_ids(remote_server, room_id, event_id).await?;
    debug!("fetching state events at event");

    let mut desired_events = pdu_ids;
    desired_events.push(event_id.to_owned());
    desired_events.extend(auth_chain_ids);

    let desired_count = desired_events.len();
    let mut failed_missing_events = Vec::new();
    let seen_events = seen_event_ids(room_id, &desired_events).await?;
    let missing_events: Vec<_> = desired_events
        .into_iter()
        .filter(|e| !seen_events.contains(e))
        .collect();
    // Same as synapse
    // Making an individual request for each of 1000s of events has a lot of
    // overhead. On the other hand, we don't really want to fetch all of the events
    // if we already have most of them.
    //
    // As an arbitrary heuristic, if we are missing more than 10% of the events, then
    // we fetch the whole state.
    if missing_events.len() * 10 >= desired_count {
        debug!("requesting complete state from remote");
        fetch_and_process_missing_state(remote_server, room_id, room_version, event_id).await?;
    } else {
        debug!("fetching {} events from remote", missing_events.len());
        let failed_events =
            fetch_and_process_events(remote_server, room_id, room_version, &missing_events).await?;
        if !failed_events.is_empty() {
            failed_missing_events.extend(failed_events);
        }
    }

    Ok(failed_missing_events)
}

pub async fn fetch_and_process_missing_state(
    origin: &ServerName,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    event_id: &EventId,
) -> AppResult<FetchedState> {
    debug!("fetching state events at event: {event_id}");
    let request = room_state_request(
        &origin.origin().await,
        RoomStateReqArgs {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
        },
    )?
    .into_inner();
    let res_body = send_federation_request(origin, request, None)
        .await?
        .json::<RoomStateResBody>()
        .await?;

    let mut state_events: IndexMap<_, OwnedEventId> = IndexMap::new();
    let mut auth_events: IndexMap<_, OwnedEventId> = IndexMap::new();
    for pdu in &res_body.pdus {
        let (event_id, event_val) = parse_fetched_pdu(room_id, room_version, pdu)?;
        let event_type = match event_val.get("type") {
            Some(v) => v.as_str().unwrap_or(""),
            None => continue,
        };
        let state_key = match event_val.get("state_key") {
            Some(v) => v.as_str().unwrap_or(""),
            None => continue,
        };
        let field_id = ensure_field_id(&event_type.into(), state_key).await?;
        state_events.insert(field_id, event_id);
    }

    for event in &res_body.auth_chain {
        let (event_id, event_val) = parse_fetched_pdu(room_id, room_version, event)?;
        let event_type = match event_val.get("type") {
            Some(v) => v.as_str().unwrap_or(""),
            None => continue,
        };
        let state_key = match event_val.get("state_key") {
            Some(v) => v.as_str().unwrap_or(""),
            None => continue,
        };
        let field_id = ensure_field_id(&event_type.into(), state_key).await?;
        auth_events.insert(field_id, event_id);
    }

    Ok(FetchedState {
        state_events,
        auth_events,
    })
}

pub async fn fetch_state_ids(
    remote_server: &ServerName,
    room_id: &RoomId,
    event_id: &EventId,
) -> AppResult<RoomStateIdsResBody> {
    debug!("calling /state_ids");
    // Call /state_ids to find out what the state at this pdu is. We trust the server's
    // response to some extend, but we still do a lot of checks on the events
    let request = room_state_ids_request(
        &remote_server.origin().await,
        RoomStateAtEventReqArgs {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
        },
    )?
    .into_inner();
    let res_body = send_federation_request(remote_server, request, None)
        .await?
        .json::<RoomStateIdsResBody>()
        .await?;
    debug!("fetching state events at event: {event_id}");

    Ok(res_body)
}

pub async fn fetch_event(
    remote_server: &ServerName,
    event_id: &EventId,
) -> AppResult<EventResBody> {
    let request =
        event_request(&remote_server.origin().await, EventReqArgs::new(event_id))?.into_inner();

    let body = crate::sending::send_federation_request(remote_server, request, None)
        .await?
        .json::<EventResBody>()
        .await?;
    Ok(body)
}

pub async fn fetch_and_process_events(
    remote_server: &ServerName,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    event_ids: &[OwnedEventId],
) -> AppResult<HashSet<OwnedEventId>> {
    let mut done_ids = Vec::new();
    for event_id in event_ids {
        match fetch_and_process_event(remote_server, room_id, room_version_id, event_id).await {
            Ok(_) => done_ids.push(event_id.clone()),
            Err(e) => {
                error!("failed to fetch/process event {event_id} : {e}");
            }
        }
    }

    Ok(event_ids
        .iter()
        .filter(|e| !done_ids.contains(e))
        .cloned()
        .collect())
}

pub async fn fetch_and_process_event(
    remote_server: &ServerName,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    event_id: &EventId,
) -> AppResult<()> {
    let res_body = fetch_event(remote_server, event_id).await?;
    let Some(outlier_pdu) = process_to_outlier_pdu(
        remote_server,
        event_id,
        room_id,
        room_version_id,
        serde_json::from_str(res_body.pdu.get())?,
    )
    .await?
    else {
        return Ok(());
    };
    outlier_pdu.save_to_database(true).await?;
    Ok(())
}
