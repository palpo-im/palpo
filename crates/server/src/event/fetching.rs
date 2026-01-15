use std::collections::HashSet;

use diesel::prelude::*;
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
use crate::event::handler::{process_pulled_pdu, process_to_outlier_pdu};
use crate::event::{connect, parse_fetched_pdu, seen_event_ids};
use crate::exts::*;
use crate::room::state::ensure_field_id;
use crate::room::timeline;
use crate::sending::send_federation_request;
use crate::{AppResult, PduEvent, SnPduEvent, room};

pub struct FetchedState {
    pub state_events: IndexMap<i64, OwnedEventId>,
    pub auth_events: IndexMap<i64, OwnedEventId>,
}

pub async fn fetch_and_process_missing_events(
    remote_server: &ServerName,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    incoming_pdu: &PduEvent,
    is_backfill: bool,
) -> AppResult<()> {
    let min_depth = timeline::first_pdu_in_room(room_id)
        .ok()
        .and_then(|pdu| pdu.map(|p| p.depth))
        .unwrap_or(0);
    let mut fetched_events = IndexMap::with_capacity(10);

    let earliest_events = room::state::get_forward_extremities(room_id)?;
    let mut known_events = HashSet::new();
    let mut missing_events = Vec::with_capacity(incoming_pdu.prev_events.len());
    for prev_id in &incoming_pdu.prev_events {
        let pdu = timeline::get_pdu(prev_id);
        if let Ok(pdu) = &pdu {
            if pdu.rejected() {
                missing_events.push(prev_id.to_owned());
            } else {
                known_events.insert(prev_id.to_owned());
            }
        } else if !earliest_events.contains(prev_id) {
            missing_events.push(prev_id.to_owned());
        }
    }
    if missing_events.is_empty() {
        return Ok(());
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

    for event in res_body.events {
        let (event_id, event_val) = parse_fetched_pdu(room_id, room_version, &event)?;

        if known_events.contains(&event_id) {
            continue;
        }

        if fetched_events.contains_key(&event_id) || timeline::get_pdu(&event_id).is_ok() {
            known_events.insert(event_id.clone());
            continue;
        }

        fetched_events.insert(event_id.clone(), event_val);
        known_events.insert(event_id.clone());
    }

    fetched_events.sort_by(|_x1, v1, _k2, v2| {
        let depth1 = v1.get("depth").and_then(|v| v.as_integer()).unwrap_or(0);
        let depth2 = v2.get("depth").and_then(|v| v.as_integer()).unwrap_or(0);
        depth1.cmp(&depth2)
    });
    for (event_id, event_val) in fetched_events {
        let is_exists = diesel_exists!(
            events::table
                .filter(events::id.eq(&event_id))
                .filter(events::room_id.eq(&room_id)),
            &mut connect()?
        )?;
        if is_exists {
            continue;
        }

        if let Err(e) = process_pulled_pdu(
            remote_server,
            &event_id,
            room_id,
            room_version,
            event_val.clone(),
            is_backfill,
        )
        .await
        {
            error!(
                "failed to process fetched missing prev event {}: {}",
                event_id, e
            );
        }
    }
    Ok(())
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
        if let Ok(pdu) = timeline::get_pdu(&event_id) {
            auth_events.push(pdu);
            continue;
        }
        if !diesel_exists!(
            events::table
                .filter(events::id.eq(&event_id))
                .filter(events::room_id.eq(&room_id)),
            &mut connect()?
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
            let pdu = outlier_pdu.save_to_database(true)?.0;
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
    desired_events.extend(auth_chain_ids.into_iter());

    let desired_count = desired_events.len();
    let mut failed_missing_events = Vec::new();
    let seen_events = seen_event_ids(room_id, &desired_events)?;
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
        let field_id = ensure_field_id(&event_type.into(), state_key)?;
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
        let field_id = ensure_field_id(&event_type.into(), state_key)?;
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
    outlier_pdu.save_to_database(true)?;
    Ok(())
}
