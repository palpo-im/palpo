use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::iter::once;
use std::sync::Arc;
use std::time::Instant;

use diesel::prelude::*;
use indexmap::IndexMap;
use palpo_core::Direction;

use super::fetching::fetch_and_process_missing_state;
use super::resolver::{resolve_state, resolve_state_at_incoming};
use crate::core::events::room::server_acl::RoomServerAclEventContent;
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::federation::event::timestamp_to_event_request;
use crate::core::identifiers::*;
use crate::core::room::{TimestampToEventReqArgs, TimestampToEventResBody};
use crate::core::room_version_rules::RoomVersionRules;
use crate::core::serde::{CanonicalJsonObject, CanonicalJsonValue, JsonValue, canonical_json};
use crate::core::signatures::Verified;
use crate::core::state::{Event, StateError, event_auth};
use crate::core::{Seqnum, UnixMillis};
use crate::data::room::DbEvent;
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::event::{OutlierPdu, PduEvent, SnPduEvent, handler};
use crate::exts::*;
use crate::room::state::{CompressedState, DeltaInfo, update_backward_extremities};
use crate::room::{state, timeline};
use crate::sending::send_federation_request;
use crate::{AppError, AppResult, MatrixError, room};

#[tracing::instrument(skip_all)]
pub(crate) async fn process_incoming_pdu(
    remote_server: &ServerName,
    event_id: &EventId,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    value: BTreeMap<String, CanonicalJsonValue>,
    is_timeline_event: bool,
    is_backfill: bool,
) -> AppResult<()> {
    if !crate::room::room_exists(room_id)? {
        return Err(MatrixError::not_found("room is unknown to this server").into());
    }

    let event = events::table
        .filter(events::id.eq(event_id))
        .first::<DbEvent>(&mut connect()?);
    if let Ok(event) = event {
        if !event.is_outlier {
            return Ok(());
        }
        if event.is_rejected || event.soft_failed {
            diesel::delete(&event).execute(&mut connect()?).ok();
            diesel::delete(event_points::table.filter(event_points::event_id.eq(event_id)))
                .execute(&mut connect()?)
                .ok();
            diesel::delete(event_datas::table.filter(event_datas::event_id.eq(event_id)))
                .execute(&mut connect()?)
                .ok();
        }
    }

    // 1.2 Check if the room is disabled
    if crate::room::is_disabled(room_id)? {
        return Err(MatrixError::forbidden(
            "federation of this room is currently disabled on this server",
            None,
        )
        .into());
    }

    // 1.3.1 Check room ACL on origin field/server
    handler::acl_check(remote_server, room_id)?;

    // 1.3.2 Check room ACL on sender's server name
    let sender: OwnedUserId = serde_json::from_value(
        value
            .get("sender")
            .ok_or_else(|| MatrixError::invalid_param("pdu does not have a valid sender key: {e}"))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("user id in sender is invalid."))?;

    if sender.server_name().ne(remote_server) {
        handler::acl_check(sender.server_name(), room_id)?;
    }
    // 1. Skip the PDU if we already have it as a timeline event
    if state::get_pdu_frame_id(event_id).is_ok() {
        return Ok(());
    }

    let Some(outlier_pdu) =
        process_to_outlier_pdu(remote_server, event_id, room_id, room_version_id, value).await?
    else {
        return Ok(());
    };

    let (incoming_pdu, val, event_guard) = outlier_pdu
        .process_incoming(remote_server, is_backfill)
        .await?;

    if incoming_pdu.rejected() {
        return Ok(());
    }

    check_room_id(room_id, &incoming_pdu)?;
    // 8. if not timeline event: stop
    if !is_timeline_event {
        return Ok(());
    }
    // Skip old events
    // let first_pdu_in_room = timeline::first_pdu_in_room(room_id)?
    //     .ok_or_else(|| AppError::internal("failed to find first pdu in database"))?;
    // if incoming_pdu.origin_server_ts < first_pdu_in_room.origin_server_ts {
    //     return Ok(());
    // }

    // Done with prev events, now handling the incoming event
    let start_time = Instant::now();
    crate::ROOM_ID_FEDERATION_HANDLE_TIME
        .write()
        .unwrap()
        .insert(room_id.to_owned(), (event_id.to_owned(), start_time));
    if let Err(e) = process_to_timeline_pdu(incoming_pdu, val, Some(remote_server)).await {
        error!("failed to process incoming pdu to timeline {}", e);
    } else {
        debug!("succeed to process incoming pdu to timeline {}", event_id);
        let pdu = timeline::get_pdu(event_id)?;
        update_backward_extremities(&pdu)?;
    }
    drop(event_guard);
    crate::ROOM_ID_FEDERATION_HANDLE_TIME
        .write()
        .unwrap()
        .remove(&room_id.to_owned());
    Ok(())
}

#[tracing::instrument(skip_all)]
pub(crate) async fn process_pulled_pdu(
    remote_server: &ServerName,
    event_id: &EventId,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    value: BTreeMap<String, CanonicalJsonValue>,
    is_backfill: bool,
) -> AppResult<()> {
    // 1.3.1 Check room ACL on origin field/server
    handler::acl_check(remote_server, room_id)?;

    // 1.3.2 Check room ACL on sender's server name
    let sender: OwnedUserId = serde_json::from_value(
        value
            .get("sender")
            .ok_or_else(|| MatrixError::invalid_param("pdu does not have a valid sender key: {e}"))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("user id in sender is invalid"))?;

    if sender.server_name().ne(remote_server) {
        handler::acl_check(sender.server_name(), room_id)?;
    }

    // 1. Skip the PDU if we already have it as a timeline event
    if state::get_pdu_frame_id(event_id).is_ok() {
        return Ok(());
    }

    let Some(outlier_pdu) =
        process_to_outlier_pdu(remote_server, event_id, room_id, room_version_id, value).await?
    else {
        return Ok(());
    };
    let (pdu, json_data, _) = outlier_pdu
        .process_pulled(remote_server, is_backfill)
        .await?;

    if pdu.soft_failed || pdu.rejected() {
        return Ok(());
    }

    if let Err(e) = process_to_timeline_pdu(pdu, json_data, Some(remote_server)).await {
        error!("failed to process pulled pdu to timeline: {}", e);
    } else {
        debug!("succeed to process incoming pdu to timeline {}", event_id);
        let pdu = timeline::get_pdu(event_id)?;
        update_backward_extremities(&pdu)?;

        let mut next_ids = event_missings::table
            .filter(event_missings::room_id.eq(&pdu.room_id))
            .filter(event_missings::missing_id.eq(&pdu.event_id))
            .select(event_missings::event_id)
            .load::<OwnedEventId>(&mut connect()?)?;
        while !next_ids.is_empty() {
            let mut new_timlined_event_ids = Vec::new();
            for next_id in next_ids {
                diesel::delete(
                    event_missings::table
                        .filter(event_missings::room_id.eq(&pdu.room_id))
                        .filter(event_missings::event_id.eq(&next_id))
                        .filter(event_missings::missing_id.eq(&pdu.event_id)),
                )
                .execute(&mut connect()?)?;
                let query = event_missings::table.filter(event_missings::event_id.eq(&next_id));
                if !diesel_exists!(query, &mut connect()?)? {
                    diesel::delete(
                        timeline_gaps::table
                            .filter(timeline_gaps::room_id.eq(&pdu.room_id))
                            .filter(timeline_gaps::event_id.eq(&next_id)),
                    )
                    .execute(&mut connect()?)?;

                    // let query = event_phases::table
                    //     .filter(event_phases::event_id.eq(&event_id))
                    //     .filter(event_phases::goal.eq("timeline"));
                    // if diesel_exists!(query, &mut connect()?)? {
                    if let Ok(pdu) = timeline::get_pdu(&next_id)
                        && pdu.is_outlier
                        && !pdu.rejected()
                    {
                        let content = pdu.get_content()?;
                        if let Err(e) =
                            process_to_timeline_pdu(pdu, content, Some(remote_server)).await
                        {
                            error!("failed to process incoming pdu to timeline {}", e);
                        } else {
                            debug!("succeed to process incoming pdu to timeline {}", next_id);
                            new_timlined_event_ids.push(next_id);
                        }
                    } else {
                        warn!("cannot find outlier pdu: {}", next_id);
                    }
                    // }
                }
            }
            next_ids = new_timlined_event_ids;
        }
    }
    Ok(())
}

#[tracing::instrument(skip_all)]
pub async fn process_to_outlier_pdu(
    remote_server: &ServerName,
    event_id: &EventId,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    mut value: CanonicalJsonObject,
) -> AppResult<Option<OutlierPdu>> {
    if let Some((room_id, event_sn, event_data)) = event_datas::table
        .filter(event_datas::event_id.eq(event_id))
        .select((
            event_datas::room_id,
            event_datas::event_sn,
            event_datas::json_data,
        ))
        .first::<(OwnedRoomId, Seqnum, JsonValue)>(&mut connect()?)
        .optional()?
        && let Ok(val) = serde_json::from_value::<CanonicalJsonObject>(event_data.clone())
        && let Ok(pdu) = timeline::get_pdu(event_id)
        && !pdu.soft_failed
        && !pdu.is_outlier
        && !pdu.rejected()
    {
        return Ok(Some(OutlierPdu {
            pdu: pdu.into_inner(),
            json_data: val,
            soft_failed: false,
            remote_server: remote_server.to_owned(),
            room_id: room_id.to_owned(),
            room_version: room_version.to_owned(),
            event_sn: Some(event_sn),
            rejected_auth_events: vec![],
            rejected_prev_events: vec![],
        }));
    }

    // 1.1. Remove unsigned field
    value.remove("unsigned");

    let version_rules = crate::room::get_version_rules(room_version)?;
    let origin_server_ts = value.get("origin_server_ts").ok_or_else(|| {
        error!("invalid pdu, no origin_server_ts field");
        MatrixError::missing_param("invalid pdu, no origin_server_ts field")
    })?;

    let _origin_server_ts = {
        let ts = origin_server_ts
            .as_integer()
            .ok_or_else(|| MatrixError::invalid_param("origin_server_ts must be an integer"))?;

        UnixMillis(
            ts.try_into()
                .map_err(|_| MatrixError::invalid_param("time must be after the unix epoch"))?,
        )
    };
    let mut val = match crate::server_key::verify_event(&value, room_version).await {
        Ok(Verified::Signatures) => {
            // Redact
            warn!("calculated hash does not match: {}", event_id);
            let obj = match canonical_json::redact(value, &version_rules.redaction, None) {
                Ok(obj) => obj,
                Err(_) => return Err(MatrixError::invalid_param("redaction failed").into()),
            };

            // Skip the PDU if it is redacted and we already have it as an outlier event
            if timeline::get_pdu_json(event_id)?.is_some() {
                return Err(MatrixError::invalid_param(
                    "event was redacted and we already knew about it",
                )
                .into());
            }

            obj
        }
        Ok(Verified::All) => value,
        Err(e) => {
            warn!("dropping bad event {}: {}  {value:#?}", event_id, e,);
            return Err(MatrixError::invalid_param("signature verification failed").into());
        }
    };

    // Now that we have checked the signature and hashes we can add the eventID and convert
    // to our PduEvent type
    val.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.as_str().to_owned()),
    );
    let mut incoming_pdu = PduEvent::from_json_value(
        room_id,
        event_id,
        serde_json::to_value(&val).expect("`CanonicalJson` is a valid `JsonValue`"),
    )
    .map_err(|_| AppError::internal("event is not a valid PDU."))?;

    check_room_id(room_id, &incoming_pdu)?;

    let server_joined = crate::room::is_server_joined(crate::config::server_name(), room_id)?;
    if !server_joined {
        if let Some(_state_key) = incoming_pdu.state_key.as_deref()
            && incoming_pdu.event_ty == TimelineEventType::RoomMember
        // && state_key.ends_with(&*format!(":{}", crate::config::server_name()))
        {
            debug!("added pdu as outlier");
            return Ok(Some(OutlierPdu {
                pdu: incoming_pdu,
                json_data: val,
                soft_failed: false,
                remote_server: remote_server.to_owned(),
                room_id: room_id.to_owned(),
                room_version: room_version.to_owned(),
                event_sn: None,
                rejected_auth_events: vec![],
                rejected_prev_events: vec![],
            }));
        }
        return Ok(None);
    }

    let mut soft_failed = false;
    let (prev_events, missing_prev_event_ids) =
        timeline::get_may_missing_pdus(room_id, &incoming_pdu.prev_events)?;
    if !missing_prev_event_ids.is_empty() {
        warn!(
            "process event to outlier missing prev events {}: {:?}",
            incoming_pdu.event_id, missing_prev_event_ids
        );
        soft_failed = true;
    }
    let rejected_prev_events = prev_events
        .iter()
        .filter_map(|pdu| {
            if pdu.rejected() {
                Some(pdu.event_id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if !rejected_prev_events.is_empty() {
        incoming_pdu.rejection_reason = Some(format!(
            "event's prev events rejected: {rejected_prev_events:?}"
        ));
        // soft_failed = true; // Will try to fetch rejected prev events again later
    }

    let (auth_events, missing_auth_event_ids) =
        timeline::get_may_missing_pdus(room_id, &incoming_pdu.auth_events)?;
    if !missing_auth_event_ids.is_empty() {
        warn!(
            "process event to outlier missing auth events {}: {:?}",
            incoming_pdu.event_id, missing_auth_event_ids
        );
        soft_failed = true;
    }
    let rejected_auth_events = auth_events
        .iter()
        .filter_map(|pdu| {
            if pdu.rejected() {
                Some(pdu.event_id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if !rejected_auth_events.is_empty() {
        incoming_pdu.rejection_reason = Some(format!(
            "event's auth events rejected: {rejected_auth_events:?}"
        ))
    }

    let auth_events = auth_events
        .into_iter()
        .map(|auth_event| {
            (
                (
                    auth_event.event_ty.to_string().into(),
                    auth_event.state_key.clone().unwrap_or_default(),
                ),
                auth_event,
            )
        })
        .collect::<HashMap<(StateEventType, _), _>>();

    // The original create event must be in the auth events
    if !matches!(
        auth_events.get(&(StateEventType::RoomCreate, "".to_owned())),
        Some(_) | None
    ) {
        incoming_pdu.rejection_reason =
            Some("incoming event refers to wrong create event".to_owned());
    }

    if incoming_pdu.rejection_reason.is_none() {
        if let Err(e) = auth_check(&incoming_pdu, &version_rules, None).await {
            match e {
                AppError::State(StateError::Forbidden(brief)) => {
                    incoming_pdu.rejection_reason = Some(brief);
                }
                _ => {
                    soft_failed = true;
                }
            }
        } else {
            soft_failed = false;
        }
    }

    Ok(Some(OutlierPdu {
        pdu: incoming_pdu,
        soft_failed,
        json_data: val,
        remote_server: remote_server.to_owned(),
        room_id: room_id.to_owned(),
        room_version: room_version.to_owned(),
        event_sn: None,
        rejected_auth_events,
        rejected_prev_events,
    }))
}

#[tracing::instrument(skip(incoming_pdu, json_data))]
pub async fn process_to_timeline_pdu(
    incoming_pdu: SnPduEvent,
    json_data: CanonicalJsonObject,
    remote_server: Option<&ServerName>,
) -> AppResult<()> {
    // Skip the PDU if we already have it as a timeline event
    if !incoming_pdu.is_outlier {
        return Ok(());
    }
    if incoming_pdu.rejected() {
        return Err(AppError::internal(
            "cannot process rejected event to timeline",
        ));
    }
    debug!("process to timeline event {}", incoming_pdu.event_id);
    let room_version_id = &room::get_version(&incoming_pdu.room_id)?;
    let version_rules = crate::room::get_version_rules(room_version_id)?;

    // 10. Fetch missing state and auth chain events by calling /state_ids at backwards extremities
    //     doing all the checks in this list starting at 1. These are not timeline events.
    debug!("resolving state at event");
    let server_joined =
        crate::room::is_server_joined(crate::config::server_name(), &incoming_pdu.room_id)?;

    if !server_joined {
        if let Some(state_key) = incoming_pdu.state_key.as_deref()
            && incoming_pdu.event_ty == TimelineEventType::RoomMember
            && state_key != incoming_pdu.sender().as_str() //????
            && state_key.ends_with(&*format!(":{}", crate::config::server_name()))
        {
            // let state_at_incoming_event = state_at_incoming_degree_one(&incoming_pdu).await?;
            let state_at_incoming_event = resolve_state_at_incoming(&incoming_pdu, &version_rules)
                .await
                .ok()
                .flatten()
                .unwrap_or_default();

            // 13. Use state resolution to find new room state
            let state_lock = crate::room::lock_state(&incoming_pdu.room_id).await;
            // Now that the event has passed all auth it is added into the timeline.
            // We use the `state_at_event` instead of `state_after` so we accurately
            // represent the state for this event.
            debug!("compressing state at event");
            let compressed_state_ids = Arc::new(
                state_at_incoming_event
                    .iter()
                    .map(|(field_id, event_id)| {
                        state::compress_event(
                            &incoming_pdu.room_id,
                            *field_id,
                            crate::event::ensure_event_sn(&incoming_pdu.room_id, event_id)?.0,
                        )
                    })
                    .collect::<AppResult<_>>()?,
            );
            debug!("preparing for stateres to derive new room state");

            // We also add state after incoming event to the fork states
            // let mut state_after = state_at_incoming_event.clone();

            let state_key_id =
                state::ensure_field_id(&incoming_pdu.event_ty.to_string().into(), state_key)?;

            let compressed_event =
                state::compress_event(&incoming_pdu.room_id, state_key_id, incoming_pdu.event_sn)?;
            let mut new_room_state = CompressedState::new();
            new_room_state.insert(compressed_event);

            // Set the new room state to the resolved state
            debug!("forcing new room state");
            let DeltaInfo {
                frame_id,
                appended,
                disposed,
            } = state::save_state(&incoming_pdu.room_id, Arc::new(new_room_state))?;

            state::force_state(&incoming_pdu.room_id, frame_id, appended, disposed)?;

            debug!("appended incoming pdu");
            timeline::append_pdu(&incoming_pdu, json_data, &state_lock).await?;
            state::set_event_state(
                &incoming_pdu.event_id,
                incoming_pdu.event_sn,
                &incoming_pdu.room_id,
                compressed_state_ids,
            )?;
            drop(state_lock);
        }
        return Ok(());
    }

    let state_at_incoming_event = resolve_state_at_incoming(&incoming_pdu, &version_rules).await?;
    let state_at_incoming_event = if let Some(state_at_incoming_event) = state_at_incoming_event {
        state_at_incoming_event
    } else if let Some(remote_server) = remote_server {
        fetch_and_process_missing_state(
            remote_server,
            &incoming_pdu.room_id,
            room_version_id,
            &incoming_pdu.event_id,
        )
        .await?
        .state_events
    } else {
        return Err(AppError::internal(
            "cannot process to timeline without state at event",
        ));
    };

    auth_check(
        &incoming_pdu,
        &version_rules,
        Some(&state_at_incoming_event),
    )
    .await?;

    // Soft fail check before doing state res
    debug!("performing soft-fail check");
    let soft_fail = match incoming_pdu.redacts_id(room_version_id) {
        None => false,
        Some(redact_id) => {
            !state::user_can_redact(
                &redact_id,
                &incoming_pdu.sender,
                &incoming_pdu.room_id,
                true,
            )
            .await?
        }
    };

    // 13. Use state resolution to find new room state
    let state_lock = crate::room::lock_state(&incoming_pdu.room_id).await;

    // Only keep those extremities were not referenced yet
    // extremities.retain(|id| !matches!(crate::room::pdu_metadata::is_event_referenced(room_id,
    // id), Ok(true)));

    debug!("compressing state at event");
    let compressed_state_ids = Arc::new(
        state_at_incoming_event
            .iter()
            .map(|(field_id, event_id)| {
                state::compress_event(
                    &incoming_pdu.room_id,
                    *field_id,
                    crate::event::ensure_event_sn(&incoming_pdu.room_id, event_id)?.0,
                )
            })
            .collect::<AppResult<_>>()?,
    );

    let guards = if let Some(state_key) = &incoming_pdu.state_key {
        debug!("preparing for stateres to derive new room state");

        // We also add state after incoming event to the fork states
        let mut state_after = state_at_incoming_event.clone();
        let state_key_id =
            state::ensure_field_id(&incoming_pdu.event_ty.to_string().into(), state_key)?;
        state_after.insert(state_key_id, incoming_pdu.event_id.clone());
        let (new_room_state, guards) =
            resolve_state(&incoming_pdu.room_id, room_version_id, state_after).await?;

        // Set the new room state to the resolved state
        debug!("forcing new room state");

        let DeltaInfo {
            frame_id,
            appended,
            disposed,
        } = state::save_state(&incoming_pdu.room_id, new_room_state)?;

        state::force_state(&incoming_pdu.room_id, frame_id, appended, disposed)?;
        guards
    } else {
        vec![]
    };

    // Now that the event has passed all auth it is added into the timeline.
    // We use the `state_at_event` instead of `state_after` so we accurately
    // represent the state for this event.
    let event_id = incoming_pdu.event_id.clone();
    // 14. Check if the event passes auth based on the "current state" of the room, if not soft fail
    //     it
    if soft_fail {
        debug!("starting soft fail auth check");
        // We start looking at current room state now, so lets lock the room
        // Now we calculate the set of extremities this room has after the incoming event has been
        // applied. We start with the previous extremities (aka leaves)
        debug!("calculating extremities");
        let mut extremities: BTreeSet<_> = state::get_forward_extremities(&incoming_pdu.room_id)?
            .into_iter()
            .collect();

        // Remove any forward extremities that are referenced by this incoming event's prev_events
        extremities.retain(|event_id| !incoming_pdu.prev_events.contains(event_id));

        let extremities = extremities
            .iter()
            .map(Borrow::borrow)
            .chain(once(event_id.borrow()));
        state::set_forward_extremities(&incoming_pdu.room_id, extremities, &state_lock)?;
        state::update_backward_extremities(&incoming_pdu)?;
        // Soft fail, we keep the event as an outlier but don't add it to the timeline
        warn!("event was soft failed: {:?}", incoming_pdu);
        crate::room::pdu_metadata::mark_event_soft_failed(&incoming_pdu.event_id)?;
        return Err(MatrixError::invalid_param("event has been soft failed").into());
    } else {
        debug!("appended incoming pdu");
        timeline::append_pdu(&incoming_pdu, json_data, &state_lock).await?;
        state::set_event_state(
            &incoming_pdu.event_id,
            incoming_pdu.event_sn,
            &incoming_pdu.room_id,
            compressed_state_ids,
        )?;
    }
    drop(guards);

    // Event has passed all auth/stateres checks
    drop(state_lock);
    Ok(())
}

pub async fn remote_timestamp_to_event(
    remote_servers: &[OwnedServerName],
    room_id: &RoomId,
    dir: Direction,
    ts: UnixMillis,
    exist: Option<&(OwnedEventId, UnixMillis)>,
) -> AppResult<(OwnedServerName, TimestampToEventResBody)> {
    async fn remote_event(
        remote_server: &ServerName,
        room_id: &RoomId,
        dir: Direction,
        ts: UnixMillis,
    ) -> AppResult<TimestampToEventResBody> {
        let request = timestamp_to_event_request(
            &remote_server.origin().await,
            TimestampToEventReqArgs {
                room_id: room_id.to_owned(),
                dir,
                ts,
            },
        )?
        .into_inner();
        let res_body = send_federation_request(remote_server, request, None)
            .await?
            .json::<TimestampToEventResBody>()
            .await?;
        Ok(res_body)
    }
    for remote_server in remote_servers {
        if let Ok(res_body) = remote_event(remote_server, room_id, dir, ts).await {
            if let Some((_exist_id, exist_ts)) = exist {
                match dir {
                    Direction::Forward => {
                        if res_body.origin_server_ts < *exist_ts {
                            return Ok((remote_server.to_owned(), res_body));
                        }
                    }
                    Direction::Backward => {
                        if res_body.origin_server_ts > *exist_ts {
                            return Ok((remote_server.to_owned(), res_body));
                        }
                    }
                }
            } else {
                return Ok((remote_server.to_owned(), res_body));
            };
        }
    }
    Err(AppError::internal(
        "failed to get timestamp to event from remote servers",
    ))
}

pub async fn auth_check(
    incoming_pdu: &PduEvent,
    version_rules: &RoomVersionRules,
    state_at_incoming_event: Option<&IndexMap<i64, OwnedEventId>>,
) -> AppResult<()> {
    let auth_rules = &version_rules.authorization;
    let state_at_incoming_event = if let Some(state_at_incoming_event) = state_at_incoming_event {
        state_at_incoming_event.to_owned()
    } else if let Some(state_at_incoming_event) =
        resolve_state_at_incoming(incoming_pdu, version_rules).await?
    {
        state_at_incoming_event
    } else {
        return Err(AppError::internal(
            "cannot auth check event without state at event",
        ));
    };

    if !state_at_incoming_event.is_empty() {
        debug!("performing auth check");
        // 11. Check the auth of the event passes based on the state of the event
        event_auth::auth_check(
            auth_rules,
            incoming_pdu,
            &async |event_id| {
                timeline::get_pdu( &event_id).map(|e|e.into_inner())
                    .map_err(|_| StateError::other("missing pdu in auth check event fetch"))
            },
            &async |k, s| {
                let Ok(state_key_id) = state::get_field_id(&k.to_string().into(), &s) else {
                    warn!("missing field id for state type: {k}, state_key: {s}");
                    return Err(StateError::other(format!(
                        "missing field id for state type: {k}, state_key: {s}"
                    )));
                };

                match state_at_incoming_event.get(&state_key_id) {
                    Some(event_id) => match timeline::get_pdu(event_id) {
                        Ok(pdu) => Ok(pdu.into_inner()),
                        Err(e) => {
                            warn!("failed to get pdu for state resolution: {}", e);
                            Err(StateError::other(format!(
                                "failed to get pdu for state resolution: {}",
                                e
                            )))
                        }
                    },
                    None => {
                        warn!(
                            "missing state key id {state_key_id} for state type: {k}, state_key: {s}, room: {}", incoming_pdu.room_id
                        );
                        Err(StateError::other(format!(
                            "missing state key id {state_key_id} for state type: {k}, state_key: {s}, room: {}", incoming_pdu.room_id
                        )))
                    }
                }
            },
        )
        .await?;
        debug!("auth check succeeded");
    }

    debug!("gathering auth events");
    let auth_events = state::get_auth_events(
        &incoming_pdu.room_id,
        &incoming_pdu.event_ty,
        &incoming_pdu.sender,
        incoming_pdu.state_key.as_deref(),
        &incoming_pdu.content,
        auth_rules,
    )?;
    event_auth::auth_check(
        auth_rules,
        incoming_pdu,
        &async |event_id| {
            timeline::get_pdu(&event_id).map(|e|e.into_inner()).map_err(|_| StateError::other("missing pdu 3"))
        },
        &async |k, s| {
            if let Some(pdu) = auth_events.get(&(k.clone(), s.to_string())).cloned() {
                return Ok(pdu.into_inner());
            }
            if auth_rules.room_create_event_id_as_room_id && k == StateEventType::RoomCreate {
                let pdu = crate::room::get_create(&incoming_pdu.room_id)
                    .map_err(|_| StateError::other("missing create event"))?;
                if pdu.room_id != incoming_pdu.room_id {
                    Err(StateError::other("mismatched room id in create event"))
                } else {
                    Ok(pdu.into_inner().into_inner())
                }
            } else {
                Err(StateError::other(format!(
                    "failed auth check when process to timeline, missing state event, event_type: {k}, state_key:{s}"
                )))
            }
        },
    )
    .await?;
    Ok(())
}
/// Returns Ok if the acl allows the server
pub fn acl_check(server_name: &ServerName, room_id: &RoomId) -> AppResult<()> {
    let acl_event = match room::get_state(room_id, &StateEventType::RoomServerAcl, "", None) {
        Ok(acl) => acl,
        Err(_) => return Ok(()),
    };

    let acl_event_content: RoomServerAclEventContent =
        match acl_event.get_content::<RoomServerAclEventContent>() {
            Ok(content) => content,
            Err(_) => {
                warn!("invalid ACL event");
                return Ok(());
            }
        };

    if acl_event_content.allow.is_empty() {
        // Ignore broken acl events
        return Ok(());
    }

    if acl_event_content.is_allowed(server_name) {
        Ok(())
    } else {
        info!(
            "server {} was denied by room ACL in {}",
            server_name, room_id
        );
        Err(MatrixError::forbidden("server was denied by room ACL", None).into())
    }
}

fn check_room_id(room_id: &RoomId, pdu: &PduEvent) -> AppResult<()> {
    if pdu.room_id != room_id {
        warn!("found event from room {} in room {}", pdu.room_id, room_id);
        return Err(MatrixError::invalid_param("Event has wrong room id").into());
    }
    Ok(())
}
