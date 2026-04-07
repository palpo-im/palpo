use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use diesel::prelude::*;
use indexmap::IndexMap;

use crate::core::identifiers::*;
use crate::core::room_version_rules::{RoomVersionRules, StateResolutionV2Rules};
use crate::core::state::{Event, StateError, StateMap, resolve};
use crate::data::connect;
use crate::data::schema::*;
use crate::event::PduEvent;
use crate::room::state::{CompressedState, DbRoomStateField};
use crate::room::{state, timeline};
use crate::utils::SeqnumQueueGuard;
use crate::{AppError, AppResult, room};

pub async fn resolve_state(
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    incoming_state: IndexMap<i64, OwnedEventId>,
) -> AppResult<(Arc<CompressedState>, Vec<SeqnumQueueGuard>)> {
    debug!("loading current room state ids");
    let current_state_ids = if let Ok(current_frame_id) = crate::room::get_frame_id(room_id, None) {
        state::get_full_state_ids(current_frame_id)?
    } else {
        IndexMap::new()
    };

    debug!("loading fork states");
    let fork_states = [current_state_ids, incoming_state];

    let mut auth_chain_sets = Vec::new();
    for state in &fork_states {
        auth_chain_sets.push(crate::room::auth_chain::get_auth_chain_ids(
            room_id,
            state.values().map(|e| &**e),
        )?);
    }

    let fork_states: Vec<_> = fork_states
        .into_iter()
        .map(|map| {
            map.into_iter()
                .filter_map(|(k, event_id)| {
                    state::get_field(k)
                        .map(
                            |DbRoomStateField {
                                 event_ty,
                                 state_key,
                                 ..
                             }| {
                                ((event_ty.to_string().into(), state_key), event_id)
                            },
                        )
                        .ok()
                })
                .collect::<StateMap<_>>()
        })
        .collect();
    debug!("resolving state");

    let version_rules = crate::room::get_version_rules(room_version_id)?;
    let state = match crate::core::state::resolve(
        &version_rules.authorization,
        version_rules
            .state_resolution
            .v2_rules()
            .unwrap_or(StateResolutionV2Rules::V2_0),
        &fork_states,
        auth_chain_sets
            .iter()
            .map(|set| set.iter().map(|id| id.to_owned()).collect::<HashSet<_>>())
            .collect::<Vec<_>>(),
        &async |id| timeline::get_pdu(&id).map_err(|_| StateError::other("missing pdu 4")),
        |map| {
            let mut subgraph = HashSet::new();
            for event_ids in map.values() {
                for event_id in event_ids {
                    if let Ok(pdu) = timeline::get_pdu(event_id) {
                        subgraph.extend(pdu.auth_events.iter().cloned());
                        subgraph.extend(pdu.prev_events.iter().cloned());
                    }
                }
            }
            let subgraph = events::table
                .filter(events::id.eq_any(subgraph))
                .filter(events::state_key.is_not_null())
                .select(events::id)
                .load::<OwnedEventId>(&mut connect().unwrap())
                .unwrap()
                .into_iter()
                .collect::<HashSet<_>>();
            Some(subgraph)
        },
    )
    .await
    {
        Ok(new_state) => new_state,
        Err(e) => {
            error!("state resolution failed: {}", e);
            return Err(AppError::internal(
                "state resolution failed, either an event could not be found or deserialization",
            ));
        }
    };

    debug!("state resolution done, compressing state");
    let mut new_room_state = BTreeSet::new();
    let mut guards = Vec::new();
    for ((event_type, state_key), event_id) in state {
        let state_key_id = state::ensure_field_id(&event_type.to_string().into(), &state_key)?;
        let (event_sn, guard) = crate::event::ensure_event_sn(room_id, &event_id)?;
        if let Some(guard) = guard {
            guards.push(guard);
        }
        new_room_state.insert(state::compress_event(room_id, state_key_id, event_sn)?);
    }

    Ok((Arc::new(new_room_state), guards))
}

// pub(super) async fn state_at_incoming_degree_one(
//     incoming_pdu: &PduEvent,
// ) -> AppResult<IndexMap<i64, OwnedEventId>> {
//     let room_id = &incoming_pdu.room_id;
//     let prev_event = &*incoming_pdu.prev_events[0];
//     let Ok(prev_frame_id) =
//         state::get_pdu_frame_id(prev_event).or_else(|_| room::get_frame_id(room_id, None))
//     else {
//         return Ok(IndexMap::new());
//     };

//     let Ok(mut state) = state::get_full_state_ids(prev_frame_id) else {
//         return Ok(IndexMap::new());
//     };

//     debug!("using cached state");
//     let prev_pdu = timeline::get_pdu(prev_event)?;

//     if let Some(state_key) = &prev_pdu.state_key {
//         let state_key_id =
//             state::ensure_field_id(&prev_pdu.event_ty.to_string().into(), state_key)?;

//         state.insert(state_key_id, prev_event.to_owned());
//         // Now it's the state after the pdu
//     }

//     Ok(state)
// }

pub(super) async fn resolve_state_at_incoming(
    incoming_pdu: &PduEvent,
    version_rules: &RoomVersionRules,
) -> AppResult<Option<IndexMap<i64, OwnedEventId>>> {
    debug!("calculating state at event using state resolve");
    let mut extremity_state_hashes = HashMap::new();
    let mut had_prev_events = false;
    let mut all_prev_events_rejected = true;

    for prev_event_id in &incoming_pdu.prev_events {
        had_prev_events = true;
        let Ok(prev_event) = timeline::get_pdu(prev_event_id) else {
            return Ok(None);
        };

        if prev_event.rejected() {
            continue;
        }
        all_prev_events_rejected = false;

        if let Ok(frame_id) = state::get_pdu_frame_id(prev_event_id) {
            extremity_state_hashes.insert(frame_id, prev_event);
        } else {
            return Ok(None);
        }
    }

    // If we had prev_events but every single one was rejected, the new event
    // logically follows the room state from BEFORE the rejected events were
    // attempted. Use the room's current frame as a best-effort fallback so
    // that an otherwise-valid event (e.g., a sentinel after rejected events)
    // can still be added to the timeline. The proper Matrix-spec answer is
    // to walk back through the rejected events' prev_events, but in practice
    // current room state is a sensible approximation when those rejections
    // didn't actually contribute to room state.
    if had_prev_events && all_prev_events_rejected && extremity_state_hashes.is_empty() {
        if let Ok(frame_id) = state::get_room_frame_id(&incoming_pdu.room_id, None) {
            let state = state::get_full_state_ids(frame_id)?;
            return Ok(Some(state));
        }
    }

    let mut fork_states = Vec::with_capacity(extremity_state_hashes.len());
    let mut auth_chain_sets = Vec::with_capacity(extremity_state_hashes.len());

    for (frame_id, prev_event) in extremity_state_hashes {
        let mut leaf_state = state::get_full_state_ids(frame_id)?;

        if let Some(state_key) = &prev_event.state_key {
            let state_key_id =
                state::ensure_field_id(&prev_event.event_ty.to_string().into(), state_key)?;
            leaf_state.insert(state_key_id, prev_event.event_id.clone());
            // Now it's the state after the pdu
        }

        let mut state = StateMap::with_capacity(leaf_state.len());
        let mut starting_events = Vec::with_capacity(leaf_state.len());

        for (k, id) in leaf_state {
            if let Ok(DbRoomStateField {
                event_ty,
                state_key,
                ..
            }) = state::get_field(k)
            {
                // FIXME: Undo .to_string().into() when StateMap is updated to use StateEventType
                state.insert((event_ty.to_string().into(), state_key), id.clone());
            } else {
                warn!("failed to get_state_key_id");
            }
            starting_events.push(id);
        }

        for starting_event in starting_events {
            auth_chain_sets.push(crate::room::auth_chain::get_auth_chain_ids(
                &incoming_pdu.room_id,
                [&*starting_event].into_iter(),
            )?);
        }

        fork_states.push(state);
    }

    let state_lock = room::lock_state(&incoming_pdu.room_id).await;
    let result = resolve(
        &version_rules.authorization,
        version_rules
            .state_resolution
            .v2_rules()
            .unwrap_or(StateResolutionV2Rules::V2_0),
        &fork_states,
        auth_chain_sets
            .iter()
            .map(|set| set.iter().map(|id| id.to_owned()).collect::<HashSet<_>>())
            .collect::<Vec<_>>(),
        &async |event_id| {
            timeline::get_pdu(&event_id)
                .map(|s| s.pdu)
                .map_err(|_| StateError::other("missing pdu 5"))
        },
        |map| {
            let mut subgraph = HashSet::new();
            for event_ids in map.values() {
                for event_id in event_ids {
                    if let Ok(pdu) = timeline::get_pdu(event_id) {
                        subgraph.extend(pdu.auth_events.iter().cloned());
                        subgraph.extend(pdu.prev_events.iter().cloned());
                    }
                }
            }
            Some(subgraph)
        },
    )
    .await;
    drop(state_lock);

    match result {
        Ok(new_state) => Ok(Some(
            new_state
                .into_iter()
                .map(|((event_type, state_key), event_id)| {
                    let state_key_id =
                        state::ensure_field_id(&event_type.to_string().into(), &state_key)?;
                    Ok((state_key_id, event_id))
                })
                //  .chain(outlier_state.into_iter().map(|(k, v)| Ok((k, v))))
                .collect::<AppResult<_>>()?,
        )),
        Err(e) => {
            warn!("state resolution on prev events failed: {}", e);
            Ok(None)
        }
    }
}
