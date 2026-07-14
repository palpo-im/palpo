use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use indexmap::IndexMap;
use lru_cache::LruCache;
use serde::Deserialize;
use serde::de::DeserializeOwned;

mod diff;
pub use diff::*;
mod field;
pub use field::*;
mod frame;
pub use frame::*;
mod graph;
pub use graph::*;

use crate::core::events::room::history_visibility::HistoryVisibility;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::room::power_levels::RoomPowerLevelsEventContent;
use crate::core::events::{AnyStrippedStateEvent, StateEventType, TimelineEventType};
use crate::core::identifiers::*;
use crate::core::room::{AllowRule, JoinRule, RoomMembership};
use crate::core::room_version_rules::AuthorizationRules;
use crate::core::serde::{JsonValue, RawJson, RawJsonValue};
use crate::core::state::StateMap;
use crate::core::{EventId, OwnedEventId, RoomId, UserId};
use crate::data::connect;
use crate::data::room::{NewDbEventMissing, NewDbTimelineGap};
use crate::data::schema::*;
use crate::event::{PduEvent, update_frame_id, update_frame_id_by_sn};
use crate::room::timeline;
use crate::{
    AppError, AppResult, MatrixError, RoomMutexGuard, SnPduEvent, membership, room, sending, utils,
};

pub static SERVER_VISIBILITY_CACHE: LazyLock<Mutex<LruCache<(OwnedServerName, i64), bool>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(100)));
pub static USER_VISIBILITY_CACHE: LazyLock<Mutex<LruCache<(OwnedUserId, i64), bool>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(100)));

pub use crate::data::room::DbRoomStateDelta;

pub async fn server_joined_rooms(server_name: &ServerName) -> AppResult<Vec<OwnedRoomId>> {
    room_joined_servers::table
        .filter(room_joined_servers::server_id.eq(server_name))
        .select(room_joined_servers::room_id)
        .load::<OwnedRoomId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Set the room to the given state_hash and update caches.
pub async fn force_state(
    room_id: &RoomId,
    frame_id: i64,
    appended: Arc<CompressedState>,
    _disposed_data: Arc<CompressedState>,
) -> AppResult<()> {
    let mut event_ids = Vec::new();
    for new in appended.iter() {
        if let Ok((_, id)) = new.split().await {
            event_ids.push(id);
        }
    }
    for event_id in &event_ids {
        let pdu = match timeline::get_pdu(event_id).await {
            Ok(pdu) => pdu,
            _ => continue,
        };

        match pdu.event_ty {
            TimelineEventType::RoomMember => {
                #[derive(Deserialize)]
                struct ExtractMembership {
                    membership: MembershipState,
                }

                let membership = match pdu.get_content::<ExtractMembership>() {
                    Ok(e) => e.membership,
                    Err(_) => continue,
                };

                let state_key = match &pdu.state_key {
                    Some(k) => k,
                    None => continue,
                };

                let user_id = match UserId::parse(state_key) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

                membership::update_membership(
                    &pdu.event_id,
                    pdu.event_sn,
                    room_id,
                    &user_id,
                    membership,
                    &pdu.sender,
                    None,
                )
                .await?;
            }
            TimelineEventType::SpaceChild => {
                let mut cache = room::space::ROOM_ID_SPACE_CHUNK_CACHE.lock().unwrap();
                cache.remove(&(pdu.room_id.clone(), false));
                cache.remove(&(pdu.room_id.clone(), true));
            }
            _ => continue,
        }
    }

    set_room_state(room_id, frame_id).await?;
    if let Err(e) = room::update_currents(room_id).await {
        error!("failed to update statistics for room {room_id}: {e}");
    }

    Ok(())
}

#[tracing::instrument]
pub async fn set_room_state(room_id: &RoomId, frame_id: i64) -> AppResult<()> {
    diesel::update(rooms::table.find(room_id))
        .set(rooms::state_frame_id.eq(frame_id))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Generates a new StateHash and associates it with the incoming event.
///
/// This adds all current state events (not including the incoming event).
#[tracing::instrument(skip(state_ids_compressed), level = "debug")]
pub async fn set_event_state(
    event_id: &EventId,
    event_sn: i64,
    room_id: &RoomId,
    state_ids_compressed: Arc<CompressedState>,
) -> AppResult<i64> {
    let prev_frame_id = get_room_frame_id(room_id, None).await.ok();
    let hash_data = utils::hash_keys(state_ids_compressed.iter().map(|s| &s[..]));
    if let Ok(frame_id) = get_frame_id(room_id, &hash_data).await {
        update_frame_id(event_id, frame_id).await?;
        Ok(frame_id)
    } else {
        let frame_id = ensure_frame(room_id, hash_data).await?;
        let states_parents = if let Some(prev_frame_id) = prev_frame_id {
            load_frame_info(prev_frame_id).await?
        } else {
            Vec::new()
        };

        let (appended, disposed) = if let Some(parent_state_info) = states_parents.last() {
            let appended: CompressedState = state_ids_compressed
                .difference(&parent_state_info.full_state)
                .copied()
                .collect();

            let disposed: CompressedState = parent_state_info
                .full_state
                .difference(&state_ids_compressed)
                .copied()
                .collect();

            (Arc::new(appended), Arc::new(disposed))
        } else {
            (state_ids_compressed, Arc::new(CompressedState::new()))
        };

        update_frame_id(event_id, frame_id).await?;
        calc_and_save_state_delta(
            room_id,
            frame_id,
            appended,
            disposed,
            1_000_000,
            states_parents,
        )
        .await?;
        Ok(frame_id)
    }
}

/// Generates a new StateHash and associates it with the incoming event.
///
/// This adds all current state events (not including the incoming event).
#[tracing::instrument(skip(new_pdu))]
pub async fn append_to_state(new_pdu: &SnPduEvent) -> AppResult<i64> {
    let prev_frame_id = get_room_frame_id(&new_pdu.room_id, None).await.ok();

    if let Some(state_key) = &new_pdu.state_key {
        let states_parents = if let Some(prev_frame_id) = prev_frame_id {
            load_frame_info(prev_frame_id).await?
        } else {
            Vec::new()
        };

        let field_id = ensure_field(&new_pdu.event_ty.to_string().into(), state_key)
            .await?
            .id;

        let new_compressed_event = CompressedEvent::new(field_id, new_pdu.event_sn);

        let replaces = states_parents
            .last()
            .map(|info| {
                info.full_state
                    .iter()
                    .find(|bytes| bytes.starts_with(&field_id.to_be_bytes()))
            })
            .unwrap_or_default();

        if Some(&new_compressed_event) == replaces {
            return prev_frame_id.ok_or_else(|| {
                MatrixError::invalid_param("Room previous point must exists.").into()
            });
        }

        // TODO: state_hash with deterministic inputs
        let mut appended = CompressedState::new();
        appended.insert(new_compressed_event);

        let mut disposed = CompressedState::new();
        if let Some(replaces) = replaces {
            disposed.insert(*replaces);
        }

        let hash_data = utils::hash_keys([new_compressed_event.as_bytes()].into_iter());
        let frame_id = ensure_frame(&new_pdu.room_id, hash_data).await?;
        update_frame_id(&new_pdu.event_id, frame_id).await?;
        calc_and_save_state_delta(
            &new_pdu.room_id,
            frame_id,
            Arc::new(appended),
            Arc::new(disposed),
            2,
            states_parents,
        )
        .await?;
        Ok(frame_id)
    } else {
        let frame_id = prev_frame_id
            .ok_or_else(|| MatrixError::invalid_param("Room previous point must exists."))?;

        update_frame_id(&new_pdu.event_id, frame_id).await?;
        Ok(frame_id)
    }
}

pub async fn summary_stripped(event: &PduEvent) -> AppResult<Vec<RawJson<AnyStrippedStateEvent>>> {
    let cells: [(&StateEventType, &str); 8] = [
        (&StateEventType::RoomCreate, ""),
        (&StateEventType::RoomJoinRules, ""),
        (&StateEventType::RoomCanonicalAlias, ""),
        (&StateEventType::RoomName, ""),
        (&StateEventType::RoomAvatar, ""),
        (&StateEventType::RoomMember, event.sender.as_str()), // Add recommended events
        (&StateEventType::RoomEncryption, ""),
        (&StateEventType::RoomTopic, ""),
    ];

    let mut state = Vec::new();
    // Add recommended events
    for (event_type, state_key) in cells {
        if let Ok(e) = super::get_state(&event.room_id, event_type, state_key, None).await {
            state.push(e.to_stripped_state_event().await);
        }
    }

    state.push(event.to_stripped_state_event().await);
    Ok(state)
}

/// Build the room-state summary used by federation invites.
///
/// Matrix v1.16 requires every entry to be a full PDU in the room version's
/// wire format and requires the room's create event to be present.
pub async fn summary_pdus(event: &PduEvent) -> AppResult<Vec<Box<RawJsonValue>>> {
    let cells: [(&StateEventType, &str); 8] = [
        (&StateEventType::RoomCreate, ""),
        (&StateEventType::RoomJoinRules, ""),
        (&StateEventType::RoomCanonicalAlias, ""),
        (&StateEventType::RoomName, ""),
        (&StateEventType::RoomAvatar, ""),
        (&StateEventType::RoomMember, event.sender.as_str()),
        (&StateEventType::RoomEncryption, ""),
        (&StateEventType::RoomTopic, ""),
    ];

    let create = super::get_state(&event.room_id, &StateEventType::RoomCreate, "", None)
        .await
        .map_err(|_| AppError::internal("room create event is missing from invite state"))?;

    let mut events = vec![create];
    for (event_type, state_key) in cells.into_iter().skip(1) {
        if let Ok(state_event) = super::get_state(&event.room_id, event_type, state_key, None).await
        {
            events.push(state_event);
        }
    }

    let mut pdus = Vec::with_capacity(events.len() + 1);
    for state_event in events {
        let json = timeline::get_pdu_json(&state_event.event_id)
            .await?
            .ok_or_else(|| {
                AppError::internal(format!(
                    "state event {} is missing its JSON data",
                    state_event.event_id
                ))
            })?;
        pdus.push(sending::convert_to_outgoing_federation_event(json).await);
    }

    let invite_json = timeline::get_pdu_json(&event.event_id)
        .await?
        .ok_or_else(|| AppError::internal("invite event is missing its JSON data"))?;
    pdus.push(sending::convert_to_outgoing_federation_event(invite_json).await);

    Ok(pdus)
}

pub async fn get_forward_extremities(room_id: &RoomId) -> AppResult<Vec<OwnedEventId>> {
    let event_ids = event_forward_extremities::table
        .filter(event_forward_extremities::room_id.eq(room_id))
        .select(event_forward_extremities::event_id)
        .distinct()
        .load::<OwnedEventId>(&mut connect().await?)
        .await?
        .into_iter()
        .collect();
    Ok(event_ids)
}

pub async fn set_forward_extremities<'a, I>(
    room_id: &RoomId,
    event_ids: I,
    _lock: &RoomMutexGuard,
) -> AppResult<()>
where
    I: Iterator<Item = &'a EventId> + Send + 'a,
{
    let event_ids = event_ids.collect::<Vec<_>>();
    diesel::delete(
        event_forward_extremities::table
            .filter(event_forward_extremities::room_id.eq(room_id))
            .filter(event_forward_extremities::event_id.ne_all(&event_ids)),
    )
    .execute(&mut connect().await?)
    .await?;
    for event_id in event_ids {
        diesel::insert_into(event_forward_extremities::table)
            .values((
                event_forward_extremities::room_id.eq(room_id),
                event_forward_extremities::event_id.eq(event_id),
            ))
            .on_conflict_do_nothing()
            .execute(&mut connect().await?)
            .await?;
    }
    Ok(())
}

pub async fn get_backward_extremities(room_id: &RoomId) -> AppResult<Vec<OwnedEventId>> {
    let event_ids = event_backward_extremities::table
        .filter(event_backward_extremities::room_id.eq(room_id))
        .select(event_backward_extremities::event_id)
        .distinct()
        .load::<OwnedEventId>(&mut connect().await?)
        .await?
        .into_iter()
        .collect();
    Ok(event_ids)
}

pub async fn update_backward_extremities(pdu: &SnPduEvent) -> AppResult<()> {
    if pdu.is_outlier {
        diesel::insert_into(event_backward_extremities::table)
            .values((
                event_backward_extremities::room_id.eq(&pdu.room_id),
                event_backward_extremities::event_id.eq(&pdu.event_id),
            ))
            .on_conflict_do_nothing()
            .execute(&mut connect().await?)
            .await?;
    } else {
        let existing_ids = events::table
            .filter(events::id.eq_any(&pdu.prev_events))
            .filter(events::is_outlier.eq(false))
            .select(events::id)
            .load::<OwnedEventId>(&mut connect().await?)
            .await?;
        let missing_ids: Vec<OwnedEventId> = pdu
            .prev_events
            .iter()
            .filter(|id| !existing_ids.contains(id))
            .cloned()
            .collect();
        if missing_ids.is_empty() {
            diesel::delete(
                event_backward_extremities::table
                    .filter(event_backward_extremities::room_id.eq(&pdu.room_id))
                    .filter(event_backward_extremities::event_id.eq(&pdu.event_id)),
            )
            .execute(&mut connect().await?)
            .await?;

            diesel::delete(
                timeline_gaps::table
                    .filter(timeline_gaps::room_id.eq(&pdu.room_id))
                    .filter(timeline_gaps::event_sn.eq(pdu.event_sn)),
            )
            .execute(&mut connect().await?)
            .await?;
        } else {
            for event_id in &missing_ids {
                diesel::insert_into(event_backward_extremities::table)
                    .values((
                        event_backward_extremities::room_id.eq(&pdu.room_id),
                        event_backward_extremities::event_id.eq(event_id),
                    ))
                    .on_conflict_do_nothing()
                    .execute(&mut connect().await?)
                    .await?;
            }
            diesel::insert_into(event_backward_extremities::table)
                .values((
                    event_backward_extremities::room_id.eq(&pdu.room_id),
                    event_backward_extremities::event_id.eq(&pdu.event_id),
                ))
                .on_conflict_do_nothing()
                .execute(&mut connect().await?)
                .await?;
            diesel::insert_into(timeline_gaps::table)
                .values(NewDbTimelineGap {
                    room_id: pdu.room_id.clone(),
                    event_id: pdu.event_id.clone(),
                    event_sn: pdu.event_sn,
                })
                .execute(&mut connect().await?)
                .await?;
        }
    }

    let existing_ids = events::table
        .filter(events::id.eq_any(&pdu.prev_events))
        .select(events::id)
        .load::<OwnedEventId>(&mut connect().await?)
        .await?;
    let missing_ids: Vec<OwnedEventId> = pdu
        .prev_events
        .iter()
        .filter(|id| !existing_ids.contains(id))
        .cloned()
        .collect();

    for missing_id in missing_ids {
        diesel::insert_into(event_missings::table)
            .values(NewDbEventMissing {
                room_id: pdu.room_id.clone(),
                event_id: pdu.event_id.clone(),
                event_sn: pdu.event_sn,
                missing_id: missing_id.clone(),
            })
            .on_conflict_do_nothing()
            .execute(&mut connect().await?)
            .await?;
    }

    Ok(())
}

/// This fetches auth events from the current state.
#[tracing::instrument]
pub async fn get_auth_events(
    room_id: &RoomId,
    kind: &TimelineEventType,
    sender: &UserId,
    state_key: Option<&str>,
    content: &serde_json::value::RawValue,
    auth_rules: &AuthorizationRules,
) -> AppResult<StateMap<SnPduEvent>> {
    let frame_id = if let Ok(current_frame_id) = get_room_frame_id(room_id, None).await {
        current_frame_id
    } else {
        return Ok(HashMap::new());
    };

    let auth_types =
        crate::core::state::auth_types_for_event(kind, sender, state_key, content, auth_rules)?;
    let mut sauth_events = HashMap::new();
    for (event_type, state_key) in auth_types {
        if let Ok(field_id) = ensure_field_id(&event_type.to_string().into(), &state_key).await {
            sauth_events.insert(field_id, (event_type, state_key));
        }
    }

    let full_state = load_frame_info(frame_id)
        .await?
        .pop()
        .expect("there is always one layer")
        .full_state;
    let mut state_map = StateMap::new();
    for state in full_state.iter() {
        let (state_key_id, event_id) = state.split().await?;
        if let Some(key) = sauth_events.remove(&state_key_id) {
            if let Ok(pdu) = timeline::get_pdu(&event_id).await {
                state_map.insert(key, pdu);
            } else {
                tracing::warn!("pdu is not found: {}", event_id);
            }
        }
    }
    Ok(state_map)
}

/// Builds a StateMap by iterating over all keys that start
/// with state_hash, this gives the full state for the given state_hash.
pub async fn get_full_state_ids(frame_id: i64) -> AppResult<IndexMap<i64, OwnedEventId>> {
    let full_state = load_frame_info(frame_id)
        .await?
        .pop()
        .expect("there is always one layer")
        .full_state;
    let mut map = IndexMap::new();
    for compressed in full_state.iter() {
        let splited = compressed.split().await?;
        map.insert(splited.0, splited.1);
    }
    Ok(map)
}

pub async fn get_full_state(
    frame_id: i64,
) -> AppResult<IndexMap<(StateEventType, String), SnPduEvent>> {
    let full_state = load_frame_info(frame_id)
        .await?
        .pop()
        .expect("there is always one layer")
        .full_state;

    let mut result = IndexMap::new();
    for compressed in full_state.iter() {
        let (_, event_id) = compressed.split().await?;
        if let Ok(pdu) = timeline::get_pdu(&event_id).await {
            result.insert(
                (
                    pdu.event_ty.to_string().into(),
                    pdu.state_key
                        .as_ref()
                        .ok_or_else(|| {
                            error!("state event has no state key: {:?}", pdu);
                            AppError::public("state event has no state key")
                        })?
                        .clone(),
                ),
                pdu,
            );
        }
    }

    Ok(result)
}

/// Returns a single PDU from `room_id` with key (`event_type`, `state_key`).
pub async fn get_state_event_id(
    frame_id: i64,
    event_type: &StateEventType,
    state_key: &str,
) -> AppResult<OwnedEventId> {
    let state_key_id = get_field_id(event_type, state_key).await?;
    let full_state = load_frame_info(frame_id)
        .await?
        .pop()
        .expect("there is always one layer")
        .full_state;
    let compressed = full_state
        .iter()
        .find(|bytes| bytes.starts_with(&state_key_id.to_be_bytes()));
    let event_id = if let Some(compressed) = compressed {
        compressed.split().await.ok().map(|(_, id)| id)
    } else {
        None
    };
    event_id.ok_or(MatrixError::not_found("state event not found").into())
}

/// Returns a single PDU from `room_id` with key (`event_type`, `state_key`).
pub async fn get_state(
    frame_id: i64,
    event_type: &StateEventType,
    state_key: &str,
) -> AppResult<SnPduEvent> {
    let event_id = get_state_event_id(frame_id, event_type, state_key).await?;
    timeline::get_pdu(&event_id).await
}
pub async fn get_state_content<T>(
    frame_id: i64,
    event_type: &StateEventType,
    state_key: &str,
) -> AppResult<T>
where
    T: DeserializeOwned,
{
    Ok(get_state(frame_id, event_type, state_key)
        .await?
        .get_content()?)
}

/// Get membership for given user in state
pub async fn user_membership(frame_id: i64, user_id: &UserId) -> AppResult<MembershipState> {
    get_state_content::<RoomMemberEventContent>(
        frame_id,
        &StateEventType::RoomMember,
        user_id.as_str(),
    )
    .await
    .map(|c: RoomMemberEventContent| c.membership)
}

/// The user was a joined member at this state (potentially in the past)
pub async fn user_was_joined(frame_id: i64, user_id: &UserId) -> bool {
    user_membership(frame_id, user_id)
        .await
        .map(|s| s == MembershipState::Join)
        .unwrap_or_default() // Return sensible default, i.e. false
}

/// The user was an invited or joined room member at this state (potentially
/// in the past)
pub async fn user_was_invited(frame_id: i64, user_id: &UserId) -> bool {
    user_membership(frame_id, user_id)
        .await
        .map(|s| s == MembershipState::Join || s == MembershipState::Invite)
        .unwrap_or_default() // Return sensible default, i.e. false
}

/// Checks if a given user can redact a given event
///
/// If federation is true, it allows redaction events from any user of the
/// same server as the original event sender
pub async fn user_can_redact(
    redacts: &EventId,
    sender: &UserId,
    room_id: &RoomId,
    federation: bool,
) -> AppResult<bool> {
    let redacting_event = timeline::get_pdu(redacts).await;

    if redacting_event
        .as_ref()
        .is_ok_and(|pdu| pdu.event_ty == TimelineEventType::RoomCreate)
    {
        return Err(MatrixError::forbidden(
            "Redacting m.room.create is not safe, forbidding.",
            None,
        )
        .into());
    }

    if redacting_event
        .as_ref()
        .is_ok_and(|pdu| pdu.event_ty == TimelineEventType::RoomServerAcl)
    {
        return Err(MatrixError::forbidden(
            "Redacting m.room.server_acl will result in the room being inaccessible for \
    			 everyone (empty allow key), forbidding.",
            None,
        )
        .into());
    }

    if let Ok(power_levels) = super::get_power_levels(room_id).await {
        Ok(power_levels.user_can_redact_event_of_other(sender)
            || power_levels.user_can_redact_own_event(sender)
                && if let Ok(redacting_event) = redacting_event {
                    if federation {
                        redacting_event.sender.server_name() == sender.server_name()
                    } else {
                        redacting_event.sender == sender
                    }
                } else {
                    false
                })
    } else {
        // Falling back on m.room.create to judge power level
        if let Ok(room_create) =
            super::get_state(room_id, &StateEventType::RoomCreate, "", None).await
        {
            Ok(room_create.sender == sender
                || redacting_event
                    .as_ref()
                    .is_ok_and(|redacting_event| redacting_event.sender == sender))
        } else {
            Err(AppError::public(
                "No m.room.power_levels or m.room.create events in database for room",
            ))
        }
    }
}

/// Whether a server is allowed to see an event through federation, based on
/// the room's history_visibility at that event's state.
#[tracing::instrument(skip(origin, room_id, event_id))]
pub async fn server_can_see_event(
    origin: &ServerName,
    room_id: &RoomId,
    event_id: &EventId,
) -> AppResult<bool> {
    let frame_id = match get_pdu_frame_id(event_id).await {
        Ok(frame_id) => frame_id,
        Err(_) => return Ok(true),
    };
    let history_visibility = super::get_history_visibility(room_id).await?;

    let visibility = match history_visibility {
        HistoryVisibility::WorldReadable | HistoryVisibility::Shared => true,
        HistoryVisibility::Invited => {
            // Allow if any member on requesting server was AT LEAST invited, else deny
            let mut allowed = false;
            for member in room::invited_users(room_id, None)
                .await?
                .into_iter()
                .filter(|member| member.server_name() == origin)
            {
                if user_was_invited(frame_id, &member).await {
                    allowed = true;
                    break;
                }
            }
            if !allowed {
                for member in room::joined_users(room_id, None)
                    .await?
                    .into_iter()
                    .filter(|member| member.server_name() == origin)
                {
                    if user_was_joined(frame_id, &member).await {
                        allowed = true;
                        break;
                    }
                }
            }
            allowed
        }
        HistoryVisibility::Joined => {
            // Allow if any member on requested server was joined, else deny
            let mut allowed = false;
            for member in room::joined_users(room_id, None)
                .await?
                .into_iter()
                .filter(|member| member.server_name() == origin)
            {
                if user_was_joined(frame_id, &member).await {
                    allowed = true;
                    break;
                }
            }
            allowed
        }
        _ => {
            error!("Unknown history visibility {history_visibility}");
            false
        }
    };

    // SERVER_VISIBILITY_CACHE
    //     .lock()
    //     .unwrap()
    //     .insert((origin.to_owned(), frame_id), visibility);

    Ok(visibility)
}

#[tracing::instrument(skip(origin, user_id))]
pub async fn server_can_see_user(origin: &ServerName, user_id: &UserId) -> AppResult<bool> {
    for room_id in server_joined_rooms(origin).await? {
        if super::user::is_joined(user_id, &room_id)
            .await
            .unwrap_or(false)
        {
            return Ok(true);
        }
    }
    Ok(false)
}
#[tracing::instrument(skip(sender_id, user_id))]
pub async fn user_can_see_user(sender_id: &UserId, user_id: &UserId) -> AppResult<bool> {
    super::user::shared_rooms(vec![sender_id.to_owned(), user_id.to_owned()])
        .await
        .map(|rooms| !rooms.is_empty())
}

/// Whether a user is allowed to see an event, based on
/// the room's history_visibility at that event's state.
#[tracing::instrument(skip(user_id, event_id))]
pub async fn user_can_see_event(user_id: &UserId, event_id: &EventId) -> AppResult<bool> {
    let pdu = timeline::get_pdu(event_id).await?;
    pdu.user_can_see(user_id).await
}

/// Whether a user is allowed to see an event, based on
/// the room's history_visibility at that event's state.
#[tracing::instrument(skip(user_id, room_id))]
pub async fn user_can_see_events(user_id: &UserId, room_id: &RoomId) -> AppResult<bool> {
    if super::user::is_joined(user_id, room_id).await? {
        return Ok(true);
    }

    let history_visibility = super::get_history_visibility(room_id).await?;
    match history_visibility {
        HistoryVisibility::Invited => super::user::is_invited(user_id, room_id).await,
        HistoryVisibility::WorldReadable => Ok(true),
        _ => Ok(false),
    }
}

/// Returns the new state_hash, and the state diff from the previous room state
pub async fn save_state(
    room_id: &RoomId,
    new_compressed_events: Arc<CompressedState>,
) -> AppResult<DeltaInfo> {
    let prev_frame_id = get_room_frame_id(room_id, None).await.ok();

    let hash_data = utils::hash_keys(new_compressed_events.iter().map(|bytes| &bytes[..]));

    let (new_frame_id, frame_existed) =
        if let Ok(frame_id) = get_frame_id(room_id, &hash_data).await {
            (frame_id, true)
        } else {
            let frame_id = ensure_frame(room_id, hash_data).await?;
            (frame_id, false)
        };

    if Some(new_frame_id) == prev_frame_id {
        return Ok(DeltaInfo {
            frame_id: new_frame_id,
            appended: Arc::new(CompressedState::new()),
            disposed: Arc::new(CompressedState::new()),
        });
    }
    for new_compressed_event in new_compressed_events.iter() {
        update_frame_id_by_sn(new_compressed_event.event_sn(), new_frame_id).await?;
    }

    let states_parents = if let Some(prev_frame_id) = prev_frame_id {
        load_frame_info(prev_frame_id).await?
    } else {
        Vec::new()
    };

    let (appended, disposed) = if let Some(parent_state_info) = states_parents.last() {
        let appended: CompressedState = new_compressed_events
            .difference(&parent_state_info.full_state)
            .copied()
            .collect();

        let disposed: CompressedState = parent_state_info
            .full_state
            .difference(&new_compressed_events)
            .copied()
            .collect();

        (Arc::new(appended), Arc::new(disposed))
    } else {
        (new_compressed_events, Arc::new(CompressedState::new()))
    };

    if !frame_existed {
        calc_and_save_state_delta(
            room_id,
            new_frame_id,
            appended.clone(),
            disposed.clone(),
            2, // every state change is 2 event changes on average
            states_parents,
        )
        .await?;
    };

    Ok(DeltaInfo {
        frame_id: new_frame_id,
        appended,
        disposed,
    })
}

#[tracing::instrument]
pub async fn get_user_state(
    user_id: &UserId,
    room_id: &RoomId,
) -> AppResult<Option<Vec<RawJson<AnyStrippedStateEvent>>>> {
    if let Some(state) = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::room_id.eq(room_id))
        .select(room_users::state_data)
        .first::<Option<JsonValue>>(&mut connect().await?)
        .await
        .optional()?
        .flatten()
    {
        Ok(Some(serde_json::from_value(state)?))
    } else {
        Ok(None)
    }
}

/// Gets up to five servers that are likely to be in the room in the
/// distant future.
///
/// See <https://spec.matrix.org/latest/appendices/#routing>
#[tracing::instrument(level = "trace")]
pub async fn servers_route_via(room_id: &RoomId) -> AppResult<Vec<OwnedServerName>> {
    let Ok(pdu) = super::get_state(room_id, &StateEventType::RoomPowerLevels, "", None).await
    else {
        return Ok(Vec::new());
    };

    let most_powerful_user_server = pdu
        .get_content::<RoomPowerLevelsEventContent>()?
        .users
        .iter()
        .max_by_key(|(_, power)| *power)
        .and_then(|x| (*x.1 >= 50).then_some(x))
        .map(|(user, _power)| user.server_name().to_owned());

    let mut servers: Vec<OwnedServerName> = super::joined_servers(room_id)
        .await?
        .into_iter()
        .take(5)
        .collect();

    if let Some(server) = most_powerful_user_server {
        servers.insert(0, server);
        servers.truncate(5);
    }

    Ok(servers)
}

/// Returns an empty vec if not a restricted room
pub fn allowed_room_ids(join_rule: JoinRule) -> Vec<OwnedRoomId> {
    let mut room_ids = Vec::with_capacity(1);
    if let JoinRule::Restricted(r) | JoinRule::KnockRestricted(r) = join_rule {
        for rule in r.allow {
            if let AllowRule::RoomMembership(RoomMembership {
                room_id: membership,
            }) = rule
            {
                room_ids.push(membership.clone());
            }
        }
    }
    room_ids
}
