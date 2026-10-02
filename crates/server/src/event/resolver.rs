use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
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
    let current_state_ids =
        if let Ok(current_frame_id) = crate::room::get_frame_id(room_id, None).await {
            state::get_full_state_ids(current_frame_id).await?
        } else {
            IndexMap::new()
        };

    debug!("loading fork states");
    let fork_states = [current_state_ids, incoming_state];

    let mut auth_chain_sets = Vec::new();
    for state in &fork_states {
        auth_chain_sets.push(
            crate::room::auth_chain::get_auth_chain_ids(room_id, state.values().map(|e| &**e))
                .await?,
        );
    }

    let mut resolved_fork_states: Vec<StateMap<_>> = Vec::with_capacity(fork_states.len());
    for map in fork_states {
        let mut state_map = StateMap::new();
        for (k, event_id) in map {
            if let Ok(DbRoomStateField {
                event_ty,
                state_key,
                ..
            }) = state::get_field(k).await
            {
                state_map.insert((event_ty.to_string().into(), state_key), event_id);
            }
        }
        resolved_fork_states.push(state_map);
    }
    let fork_states = resolved_fork_states;
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
        &async |id| {
            timeline::get_pdu(&id)
                .await
                .map_err(|_| StateError::other("missing pdu 4"))
        },
        |map| {
            // Snapshot the event ids synchronously so the returned future owns its
            // data (no borrow of `map` across the await, no blocking the runtime).
            let event_ids: Vec<OwnedEventId> = map.values().flatten().cloned().collect();
            async move {
                let mut subgraph = HashSet::new();
                for event_id in &event_ids {
                    if let Ok(pdu) = timeline::get_pdu(event_id).await {
                        subgraph.extend(pdu.auth_events.iter().cloned());
                        subgraph.extend(pdu.prev_events.iter().cloned());
                    }
                }
                let subgraph = events::table
                    .filter(events::id.eq_any(subgraph))
                    .filter(events::state_key.is_not_null())
                    .select(events::id)
                    .load::<OwnedEventId>(&mut connect().await.ok()?)
                    .await
                    .ok()?
                    .into_iter()
                    .collect::<HashSet<_>>();
                Some(subgraph)
            }
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
        let state_key_id =
            state::ensure_field_id(&event_type.to_string().into(), &state_key).await?;
        let (event_sn, guard) = crate::event::ensure_event_sn(room_id, &event_id).await?;
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

pub(crate) async fn resolve_state_at_incoming(
    incoming_pdu: &PduEvent,
    version_rules: &RoomVersionRules,
) -> AppResult<Option<IndexMap<i64, OwnedEventId>>> {
    let mut resolving = HashSet::new();
    // Legacy events can require walking their predecessor DAG. Bound total work,
    // not only recursion depth, so a wide malicious DAG cannot expand unchecked.
    let mut remaining_events = 256;
    resolve_state_at_incoming_inner(
        incoming_pdu,
        version_rules,
        &mut resolving,
        &mut remaining_events,
    )
    .await
}

fn resolve_state_at_incoming_inner<'a>(
    incoming_pdu: &'a PduEvent,
    version_rules: &'a RoomVersionRules,
    resolving: &'a mut HashSet<OwnedEventId>,
    remaining_events: &'a mut usize,
) -> BoxFuture<'a, AppResult<Option<IndexMap<i64, OwnedEventId>>>> {
    async move {
        if *remaining_events == 0 || !resolving.insert(incoming_pdu.event_id.clone()) {
            return Ok(None);
        }
        *remaining_events -= 1;

        let result = resolve_state_at_incoming_impl(
            incoming_pdu,
            version_rules,
            resolving,
            remaining_events,
        )
        .await;
        resolving.remove(&incoming_pdu.event_id);
        result
    }
    .boxed()
}

fn resolve_state_at_incoming_impl<'a>(
    incoming_pdu: &'a PduEvent,
    version_rules: &'a RoomVersionRules,
    resolving: &'a mut HashSet<OwnedEventId>,
    remaining_events: &'a mut usize,
) -> BoxFuture<'a, AppResult<Option<IndexMap<i64, OwnedEventId>>>> {
    async move {
        debug!("calculating state at event using state resolve");
        let mut extremity_states = Vec::with_capacity(incoming_pdu.prev_events.len());

        for prev_event_id in &incoming_pdu.prev_events {
            let Ok(prev_event) = timeline::get_pdu(prev_event_id).await else {
                // Truly unknown prev event — don't fall back to current state. The
                // caller (e.g. process_incoming) needs to keep the event soft-failed
                // so the missing-events fetch path runs. Returning None here
                // signals "can't resolve locally".
                return Ok(None);
            };

            let mut leaf_state = match state::get_pdu_before_frame_id(prev_event_id).await {
                Ok(frame_id) => state::get_full_state_ids(frame_id).await?,
                Err(e)
                    if e.is_not_found()
                        && prev_event.state_key.is_none()
                        && !prev_event.rejected() =>
                {
                    let frame_id = match state::get_pdu_frame_id(prev_event_id).await {
                        Ok(frame_id) => frame_id,
                        Err(e) if e.is_not_found() => return Ok(None),
                        Err(e) => return Err(e),
                    };
                    state::get_full_state_ids(frame_id).await?
                }
                Err(e) if e.is_not_found() => {
                    let Some(state_before_prev) = resolve_state_at_incoming_inner(
                        &prev_event,
                        version_rules,
                        resolving,
                        remaining_events,
                    )
                    .await?
                    else {
                        return Ok(None);
                    };
                    state_before_prev
                }
                Err(e) => return Err(e),
            };

            // A rejected event leaves its predecessor state unchanged. Retain that
            // state as a fork input, but never apply the rejected state update.
            if !prev_event.rejected()
                && let Some(state_key) = &prev_event.state_key
            {
                let state_key_id =
                    state::ensure_field_id(&prev_event.event_ty.to_string().into(), state_key)
                        .await?;
                leaf_state.insert(state_key_id, prev_event.event_id.clone());
            }
            // Do not key this collection by frame id. Forked state predecessors can
            // legitimately share a before-frame and each must remain a separate leaf.
            extremity_states.push(leaf_state);
        }

        let mut fork_states = Vec::with_capacity(extremity_states.len());
        let mut auth_chain_sets = Vec::with_capacity(extremity_states.len());

        for leaf_state in extremity_states {
            let mut state = StateMap::with_capacity(leaf_state.len());
            let mut starting_events = Vec::with_capacity(leaf_state.len());

            for (k, id) in leaf_state {
                if let Ok(DbRoomStateField {
                    event_ty,
                    state_key,
                    ..
                }) = state::get_field(k).await
                {
                    // FIXME: Undo .to_string().into() when StateMap is updated to use
                    // StateEventType
                    state.insert((event_ty.to_string().into(), state_key), id.clone());
                } else {
                    warn!("failed to get_state_key_id");
                }
                starting_events.push(id);
            }

            for starting_event in starting_events {
                auth_chain_sets.push(
                    crate::room::auth_chain::get_auth_chain_ids(
                        &incoming_pdu.room_id,
                        [&*starting_event].into_iter(),
                    )
                    .await?,
                );
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
                    .await
                    .map(|s| s.pdu)
                    .map_err(|_| StateError::other("missing pdu 5"))
            },
            |map| {
                // Snapshot the event ids synchronously so the returned future owns its
                // data (no borrow of `map` across the await, no blocking the runtime).
                let event_ids: Vec<OwnedEventId> = map.values().flatten().cloned().collect();
                async move {
                    let mut subgraph = HashSet::new();
                    for event_id in &event_ids {
                        if let Ok(pdu) = timeline::get_pdu(event_id).await {
                            subgraph.extend(pdu.auth_events.iter().cloned());
                            subgraph.extend(pdu.prev_events.iter().cloned());
                        }
                    }
                    Some(subgraph)
                }
            },
        )
        .await;
        drop(state_lock);

        match result {
            Ok(new_state) => {
                let mut resolved = IndexMap::new();
                for ((event_type, state_key), event_id) in new_state {
                    let state_key_id =
                        state::ensure_field_id(&event_type.to_string().into(), &state_key).await?;
                    resolved.insert(state_key_id, event_id);
                }
                Ok(Some(resolved))
            }
            Err(e) => {
                warn!("state resolution on prev events failed: {}", e);
                Ok(None)
            }
        }
    }
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::UnixMillis;
    use crate::core::events::StateEventType;
    use crate::event::{OutlierPdu, SnPduEvent};

    fn event(id: &str, ty: &str, state_key: Option<&str>, content: serde_json::Value) -> PduEvent {
        serde_json::from_value(serde_json::json!({
            "event_id": id, "room_id": "!auth-history:example.org", "sender": "@alice:example.org",
            "type": ty, "state_key": state_key, "origin_server_ts": 1000,
            "content": content, "depth": 1, "hashes": {"sha256": "test"}
        }))
        .unwrap()
    }

    async fn store(pdu: PduEvent) -> SnPduEvent {
        OutlierPdu {
            json_data: crate::core::serde::to_canonical_object(&pdu).unwrap(),
            room_id: pdu.room_id.clone(),
            pdu,
            soft_failed: false,
            policy_refused: false,
            remote_server: "example.org".try_into().unwrap(),
            room_version: RoomVersionId::V11,
            event_sn: None,
        }
        .save_to_database(false)
        .await
        .unwrap()
        .0
    }

    async fn snapshot(target: &SnPduEvent, members: &[&SnPduEvent]) -> i64 {
        let mut compressed = CompressedState::new();
        for pdu in members {
            let field = state::ensure_field_id(
                &pdu.event_ty.to_string().into(),
                pdu.state_key.as_deref().unwrap(),
            )
            .await
            .unwrap();
            compressed.insert(state::compress_event(&pdu.room_id, field, pdu.event_sn).unwrap());
        }
        state::set_event_state_before(&target.event_id, &target.room_id, Arc::new(compressed))
            .await
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_event_time_auth_never_substitutes_current_state() {
        crate::test_database::init();
        let rules = room::get_version_rules(&RoomVersionId::V11).unwrap();
        let create = store(event(
            "$auth-create",
            "m.room.create",
            Some(""),
            serde_json::json!({"creator": "@alice:example.org", "room_version": "11"}),
        ))
        .await;
        diesel::insert_into(rooms::table)
            .values(crate::data::room::NewDbRoom {
                id: create.room_id.clone(),
                version: "11".into(),
                is_public: false,
                min_depth: 1,
                has_auth_chain_index: false,
                created_at: UnixMillis(1000),
            })
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();
        snapshot(&create, &[]).await;
        let mut join = event(
            "$auth-alice-join",
            "m.room.member",
            Some("@alice:example.org"),
            serde_json::json!({"membership": "join"}),
        );
        join.prev_events = vec![create.event_id.clone()];
        join.auth_events = vec![create.event_id.clone()];
        let join = store(join).await;
        snapshot(&join, &[&create]).await;
        let mut rejected = event(
            "$auth-rejected",
            "m.room.member",
            Some("@alice:example.org"),
            serde_json::json!({"membership": "ban"}),
        );
        rejected.prev_events = vec![join.event_id.clone()];
        rejected.auth_events = vec![create.event_id.clone(), join.event_id.clone()];
        rejected.rejection_reason = Some("fixture rejected state event".into());
        let rejected = store(rejected).await;
        let mut bob = event(
            "$auth-bob-join",
            "m.room.member",
            Some("@bob:example.org"),
            serde_json::json!({"membership": "join"}),
        );
        bob.sender = UserId::parse("@bob:example.org").unwrap();
        let bob = store(bob).await;
        let current_frame = snapshot(&bob, &[&create, &join, &bob]).await;
        diesel::update(rooms::table.find(&create.room_id))
            .set(rooms::state_frame_id.eq(current_frame))
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();

        let mut child = event(
            "$auth-child",
            "m.room.message",
            None,
            serde_json::json!({"body": "historical", "msgtype": "m.text"}),
        );
        child.prev_events = vec![rejected.event_id.clone()];
        child.auth_events = vec![create.event_id.clone(), join.event_id.clone()];
        let old_state = resolve_state_at_incoming(&child, &rules)
            .await
            .unwrap()
            .unwrap();
        let alice_field = state::ensure_field_id(&StateEventType::RoomMember, "@alice:example.org")
            .await
            .unwrap();
        let bob_field = state::ensure_field_id(&StateEventType::RoomMember, "@bob:example.org")
            .await
            .unwrap();
        assert_eq!(old_state.get(&alice_field), Some(&join.event_id));
        assert!(
            !old_state.contains_key(&bob_field),
            "later room state must not enter the historical fork"
        );
        crate::event::handler::auth_check(&child, &rules, Some(&old_state))
            .await
            .unwrap();
        child.sender = bob.sender.clone();
        child.auth_events = vec![create.event_id.clone(), bob.event_id.clone()];
        assert!(
            crate::event::handler::auth_check(&child, &rules, Some(&old_state))
                .await
                .is_err(),
            "later joined membership cannot authorize an earlier event"
        );

        // A stored before-frame follows exactly the same rules as legacy DAG reconstruction.
        snapshot(&rejected, &[&create, &join]).await;
        assert_eq!(
            resolve_state_at_incoming(&child, &rules)
                .await
                .unwrap()
                .unwrap(),
            old_state
        );
        // If a rejected predecessor's historical state is unavailable, request recovery.
        let mut missing = event(
            "$auth-rejected-missing",
            "m.room.member",
            Some("@alice:example.org"),
            serde_json::json!({"membership": "ban"}),
        );
        missing.prev_events = vec![EventId::parse("$auth-unknown").unwrap()];
        missing.rejection_reason = Some("fixture".into());
        let missing = store(missing).await;
        child.prev_events = vec![missing.event_id.clone()];
        assert!(
            resolve_state_at_incoming(&child, &rules)
                .await
                .unwrap()
                .is_none()
        );
        // Mixed accepted/rejected forks must retain both historic branches.
        child.prev_events.push(join.event_id.clone());
        assert!(
            resolve_state_at_incoming(&child, &rules)
                .await
                .unwrap()
                .is_none()
        );
    }
}
