use std::borrow::Borrow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use diesel::prelude::*;
use serde::Deserialize;

use crate::core::Seqnum;
use crate::core::events::push_rules::PushRulesEventContent;
use crate::core::events::room::canonical_alias::RoomCanonicalAliasEventContent;
use crate::core::events::room::encrypted::Relation;
use crate::core::events::room::member::MembershipState;
use crate::core::events::{GlobalAccountDataEventType, StateEventType, TimelineEventType};
use crate::core::identifiers::*;
use crate::core::presence::PresenceState;
use crate::core::push::{Action, Ruleset, Tweak};
use crate::core::serde::{CanonicalJsonObject, CanonicalJsonValue, JsonValue, to_canonical_object};
use crate::core::state::Event;
use crate::data::room::{DbEvent, DbEventData, NewDbEventEdge};
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::event::{PduBuilder, PduEvent};
use crate::room::{push_action, state, timeline};
use crate::{
    AppError, AppResult, MatrixError, RoomMutexGuard, SnPduEvent, config, data, membership,
};

mod backfill;
pub mod stream;
pub mod topolo;
pub use backfill::*;

pub static LAST_TIMELINE_COUNT_CACHE: LazyLock<Mutex<HashMap<OwnedRoomId, i64>>> =
    LazyLock::new(Default::default);
// pub static PDU_CACHE: LazyLock<Mutex<LruCache<OwnedRoomId, Arc<PduEvent>>>> =
// LazyLock::new(Default::default);

#[tracing::instrument]
pub fn first_pdu_in_room(room_id: &RoomId) -> AppResult<Option<PduEvent>> {
    event_datas::table
        .filter(event_datas::room_id.eq(room_id))
        .order(event_datas::event_sn.asc())
        .select((event_datas::event_id, event_datas::json_data))
        .first::<(OwnedEventId, JsonValue)>(&mut connect()?)
        .optional()?
        .map(|(event_id, json)| {
            PduEvent::from_json_value(room_id, &event_id, json)
                .map_err(|_e| AppError::internal("invalid pdu in db"))
        })
        .transpose()
}

#[tracing::instrument]
pub fn last_event_sn(user_id: &UserId, room_id: &RoomId) -> AppResult<Seqnum> {
    let event_sn = events::table
        .filter(events::room_id.eq(room_id))
        .filter(events::sn.is_not_null())
        .select(events::sn)
        .order(events::sn.desc())
        .first::<Seqnum>(&mut connect()?)?;
    Ok(event_sn)
}

/// Returns the json of a pdu.
pub fn get_pdu_json(event_id: &EventId) -> AppResult<Option<CanonicalJsonObject>> {
    event_datas::table
        .filter(event_datas::event_id.eq(event_id))
        .select(event_datas::json_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?
        .map(|json| {
            serde_json::from_value(json).map_err(|_e| AppError::internal("invalid pdu in db"))
        })
        .transpose()
}

/// Returns the pdu.
pub fn get_non_outlier_pdu(event_id: &EventId) -> AppResult<Option<SnPduEvent>> {
    let Some((event_sn, room_id, stream_ordering)) = events::table
        .filter(events::is_outlier.eq(false))
        .filter(events::id.eq(event_id))
        .select((events::sn, events::room_id, events::stream_ordering))
        .first::<(Seqnum, OwnedRoomId, i64)>(&mut connect()?)
        .optional()?
    else {
        return Ok(None);
    };
    let mut pdu = event_datas::table
        .filter(event_datas::event_id.eq(event_id))
        .select(event_datas::json_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?
        .map(|json| {
            SnPduEvent::from_json_value(
                &room_id,
                event_id,
                event_sn,
                json,
                false,
                false,
                stream_ordering < 0,
            )
            .map_err(|_e| AppError::internal("invalid pdu in db"))
        })
        .transpose()?;
    if let Some(pdu) = pdu.as_mut() {
        let event = events::table
            .filter(events::id.eq(event_id))
            .first::<DbEvent>(&mut connect()?)?;
        pdu.is_outlier = event.is_outlier;
        pdu.soft_failed = event.soft_failed;
        pdu.rejection_reason = event.rejection_reason;
    }
    Ok(pdu)
}

pub fn get_pdu(event_id: &EventId) -> AppResult<SnPduEvent> {
    let event = events::table
        .filter(events::id.eq(event_id))
        .first::<DbEvent>(&mut connect()?)?;
    let (event_sn, room_id, json) = event_datas::table
        .filter(event_datas::event_id.eq(event_id))
        .select((
            event_datas::event_sn,
            event_datas::room_id,
            event_datas::json_data,
        ))
        .first::<(Seqnum, OwnedRoomId, JsonValue)>(&mut connect()?)?;
    let mut pdu = PduEvent::from_json_value(&room_id, event_id, json)
        .map_err(|_e| AppError::internal("invalid pdu in db"))?;
    pdu.rejection_reason = event.rejection_reason;
    Ok(SnPduEvent {
        pdu,
        event_sn,
        is_outlier: event.is_outlier,
        soft_failed: event.soft_failed,
        is_backfill: event.stream_ordering < 0,
    })
}

pub fn get_pdu_and_data(event_id: &EventId) -> AppResult<(SnPduEvent, CanonicalJsonObject)> {
    let event = events::table
        .filter(events::id.eq(event_id))
        .first::<DbEvent>(&mut connect()?)?;
    let (event_sn, room_id, json) = event_datas::table
        .filter(event_datas::event_id.eq(event_id))
        .select((
            event_datas::event_sn,
            event_datas::room_id,
            event_datas::json_data,
        ))
        .first::<(Seqnum, OwnedRoomId, JsonValue)>(&mut connect()?)?;
    let data = serde_json::from_value(json.clone())
        .map_err(|_e| AppError::internal("invalid pdu in db"))?;
    let mut pdu = PduEvent::from_json_value(&room_id, event_id, json)
        .map_err(|_e| AppError::internal("invalid pdu in db"))?;
    pdu.rejection_reason = event.rejection_reason;
    Ok((
        SnPduEvent {
            pdu,
            event_sn,
            is_outlier: event.is_outlier,
            soft_failed: event.soft_failed,
            is_backfill: event.stream_ordering < 0,
        },
        data,
    ))
}

pub fn get_may_missing_pdus(
    room_id: &RoomId,
    event_ids: &[OwnedEventId],
) -> AppResult<(Vec<SnPduEvent>, Vec<OwnedEventId>)> {
    let events = event_datas::table
        .filter(event_datas::room_id.eq(room_id))
        .filter(event_datas::event_id.eq_any(event_ids))
        .select(event_datas::event_id)
        .load::<OwnedEventId>(&mut connect()?)?;

    let mut pdus = Vec::with_capacity(events.len());
    let mut missing_ids = event_ids.iter().cloned().collect::<HashSet<_>>();
    for event_id in events {
        let Ok(pdu) = timeline::get_pdu(&event_id) else {
            continue;
        };
        pdus.push(pdu);
        missing_ids.remove(&event_id);
    }
    Ok((pdus, missing_ids.into_iter().collect()))
}

pub fn has_pdu(event_id: &EventId) -> bool {
    if let Ok(mut conn) = connect() {
        diesel_exists!(
            event_datas::table.filter(event_datas::event_id.eq(event_id)),
            &mut conn
        )
        .unwrap_or(false)
    } else {
        false
    }
}

/// Removes a pdu and creates a new one with the same id.
#[tracing::instrument]
pub fn replace_pdu(event_id: &EventId, pdu_json: &CanonicalJsonObject) -> AppResult<()> {
    diesel::update(event_datas::table.filter(event_datas::event_id.eq(event_id)))
        .set(event_datas::json_data.eq(serde_json::to_value(pdu_json)?))
        .execute(&mut connect()?)?;
    // PDU_CACHE.lock().unwrap().remove(&(*pdu.event_id).to_owned());

    Ok(())
}

/// Creates a new persisted data unit and adds it to a room.
///
/// By this point the incoming event should be fully authenticated, no auth happens
/// in `append_pdu`.
///
/// Returns pdu id
#[tracing::instrument(skip_all)]
pub async fn append_pdu(
    pdu: &SnPduEvent,
    mut pdu_json: CanonicalJsonObject,
    state_lock: &RoomMutexGuard,
) -> AppResult<()> {
    let conf = crate::config::get();

    // Make unsigned fields correct. This is not properly documented in the spec, but state
    // events need to have previous content in the unsigned field, so clients can easily
    // interpret things like membership changes
    if let Some(state_key) = &pdu.state_key {
        if let CanonicalJsonValue::Object(unsigned) = pdu_json
            .entry("unsigned".to_owned())
            .or_insert_with(|| CanonicalJsonValue::Object(Default::default()))
        {
            if let Ok(state_frame_id) = state::get_pdu_frame_id(&pdu.event_id)
                && let Ok(prev_state) = state::get_state(
                    state_frame_id - 1,
                    &pdu.event_ty.to_string().into(),
                    state_key,
                )
            {
                unsigned.insert(
                    "prev_content".to_owned(),
                    CanonicalJsonValue::Object(
                        to_canonical_object(prev_state.content.clone())
                            .expect("event is valid, we just created it"),
                    ),
                );
                unsigned.insert(
                    "prev_sender".to_owned(),
                    CanonicalJsonValue::String(prev_state.sender.to_string()),
                );
                unsigned.insert(
                    "replaces_state".to_owned(),
                    CanonicalJsonValue::String(prev_state.event_id.to_string()),
                );
            }
        } else {
            error!("invalid unsigned type in pdu");
        }
    }

    let mut leaves: BTreeSet<_> = state::get_forward_extremities(&pdu.room_id)?
        .into_iter()
        .collect();
    // Remove any forward extremities that are referenced by this incoming event's prev_events
    leaves.retain(|event_id| !pdu.prev_events.contains(event_id));
    if !diesel_exists!(
        event_edges::table.filter(event_edges::prev_id.eq(&pdu.event_id)),
        &mut connect()?
    )? {
        // Only add the incoming event as a forward extremity if it is not already in the DB
        leaves.insert(pdu.event_id.clone());
    }
    state::set_forward_extremities(&pdu.room_id, leaves.iter().map(Borrow::borrow), state_lock)?;
    state::update_backward_extremities(&pdu)?;

    #[derive(Deserialize, Clone, Debug)]
    struct ExtractEventId {
        event_id: OwnedEventId,
    }
    #[derive(Deserialize, Clone, Debug)]
    struct ExtractRelatesToEventId {
        #[serde(rename = "m.relates_to")]
        relates_to: ExtractEventId,
    }
    let mut relates_added = false;
    if let Ok(content) = pdu.get_content::<ExtractRelatesTo>() {
        let rel_type = content.relates_to.rel_type();
        match content.relates_to {
            Relation::Reply { in_reply_to } => {
                // We need to do it again here, because replies don't have event_id as a top level
                // field
                super::pdu_metadata::add_relation(
                    &pdu.room_id,
                    &in_reply_to.event_id,
                    &pdu.event_id,
                    rel_type,
                )?;
                relates_added = true;
            }
            Relation::Thread(thread) => {
                super::pdu_metadata::add_relation(
                    &pdu.room_id,
                    &thread.event_id,
                    &pdu.event_id,
                    rel_type,
                )?;
                relates_added = true;
                // thread_id = Some(thread.event_id.clone());
                super::thread::add_to_thread(&thread.event_id, pdu)?;
            }
            _ => {} // TODO: Aggregate other types
        }
    }
    if !relates_added && let Ok(content) = pdu.get_content::<ExtractRelatesToEventId>() {
        super::pdu_metadata::add_relation(
            &pdu.room_id,
            &content.relates_to.event_id,
            &pdu.event_id,
            None,
        )?;
    }

    let sync_pdu = pdu.to_sync_room_event();
    let mut notifies = Vec::new();
    let mut highlights = Vec::new();

    for user_id in super::get_our_real_users(&pdu.room_id)?.iter() {
        // Don't notify the user of their own events
        if user_id == &pdu.sender {
            continue;
        }

        let rules_for_user = data::user::get_global_data::<PushRulesEventContent>(
            user_id,
            &GlobalAccountDataEventType::PushRules.to_string(),
        )?
        .map(|content: PushRulesEventContent| content.global)
        .unwrap_or_else(|| Ruleset::server_default(user_id));

        let mut highlight = false;
        let mut notify = false;

        if let Ok(power_levels) = crate::room::get_power_levels(pdu.room_id()).await {
            for action in data::user::pusher::get_actions(
                user_id,
                &rules_for_user,
                &power_levels,
                &sync_pdu,
                &pdu.room_id,
            )
            .await?
            {
                match action {
                    Action::Notify => notify = true,
                    Action::SetTweak(Tweak::Highlight(true)) => {
                        highlight = true;
                    }
                    _ => {}
                };
            }
        }

        if notify {
            notifies.push(user_id.clone());
        }
        if highlight {
            highlights.push(user_id.clone());
        }

        if let Err(e) =
            push_action::upsert_push_action(&pdu.room_id, &pdu.event_id, user_id, notify, highlight)
        {
            error!("failed to upsert event push action: {}", e);
        }
        push_action::refresh_notify_summary(&pdu.sender, &pdu.room_id)?;

        for push_key in data::user::pusher::get_push_keys(user_id)? {
            crate::sending::send_push_pdu(&pdu.event_id, user_id, push_key)?;
        }
    }

    match pdu.event_ty {
        TimelineEventType::RoomRedaction => {
            if let Some(redact_id) = &pdu.redacts {
                redact_pdu(redact_id, pdu)?;
            }
        }
        TimelineEventType::SpaceChild => {
            if let Some(_state_key) = &pdu.state_key {
                let mut cache = super::space::ROOM_ID_SPACE_CHUNK_CACHE.lock().unwrap();
                cache.remove(&(pdu.room_id.clone(), false));
                cache.remove(&(pdu.room_id.clone(), true));
            }
        }
        TimelineEventType::RoomMember => {
            if let Some(state_key) = &pdu.state_key {
                #[derive(Deserialize)]
                struct ExtractMembership {
                    membership: MembershipState,
                }

                // if the state_key fails
                let target_user_id = UserId::parse(state_key.clone())
                    .expect("This state_key was previously validated");

                let content = pdu
                    .get_content::<ExtractMembership>()
                    .map_err(|_| AppError::internal("Invalid content in pdu."))?;

                let stripped_state = match content.membership {
                    MembershipState::Invite | MembershipState::Knock => {
                        let state = state::summary_stripped(pdu)?;
                        Some(state)
                    }
                    _ => None,
                };

                if content.membership == MembershipState::Join {
                    let _ = crate::user::ping_presence(&pdu.sender, &PresenceState::Online);
                }
                // Update our membership info, we do this here incase a user is invited
                // and immediately leaves we need the DB to record the invite event for auth
                membership::update_membership(
                    &pdu.event_id,
                    pdu.event_sn,
                    &pdu.room_id,
                    &target_user_id,
                    content.membership,
                    &pdu.sender,
                    stripped_state,
                )?;
            }
        }
        TimelineEventType::RoomMessage => {
            #[derive(Deserialize)]
            struct ExtractBody {
                body: Option<String>,
            }

            let content = pdu
                .get_content::<ExtractBody>()
                .map_err(|_| AppError::internal("Invalid content in pdu."))?;

            if let Some(body) = content.body
                && let Ok(admin_room) = super::resolve_local_alias(
                    <&RoomAliasId>::try_from(format!("#admins:{}", &conf.server_name).as_str())
                        .expect("#admins:server_name is a valid room alias"),
                )
            {
                let server_user = config::server_user_id();

                let to_palpo = body.starts_with(&format!("{server_user}: "))
                    || body.starts_with(&format!("{server_user} "))
                    || body == format!("{server_user}:")
                    || body == format!("{server_user}");

                // This will evaluate to false if the emergency password is set up so that
                // the administrator can execute commands as palpo
                let from_palpo = pdu.sender == server_user && conf.emergency_password.is_none();

                if to_palpo && !from_palpo && admin_room == pdu.room_id {
                    let _ = crate::admin::executor()
                        .command(body, Some(pdu.event_id.clone()))
                        .await;
                }
            }
        }
        TimelineEventType::RoomTombstone => {
            #[derive(Deserialize)]
            struct ExtractReplacementRoom {
                replacement_room: Option<OwnedRoomId>,
            }

            let content = pdu
                .get_content::<ExtractReplacementRoom>()
                .map_err(|_| AppError::internal("invalid content in tombstone pdu"))?;

            if let Some(new_room_id) = content.replacement_room {
                let local_user_ids = super::user::local_users(&pdu.room_id)?;
                for user_id in &local_user_ids {
                    super::user::copy_room_tags_and_direct_to_room(
                        user_id,
                        &pdu.room_id,
                        &new_room_id,
                    )?;
                    super::user::copy_push_rules_from_room_to_room(
                        user_id,
                        &pdu.room_id,
                        &new_room_id,
                    )?;
                }
            }
        }
        _ => {}
    }

    DbEventData {
        event_id: pdu.event_id.clone(),
        event_sn: pdu.event_sn,
        room_id: pdu.room_id.to_owned(),
        internal_metadata: None,
        json_data: serde_json::to_value(&pdu_json)?,
        format_version: None,
    }
    .save()?;
    diesel::update(events::table.find(&*pdu.event_id))
        .set(events::is_outlier.eq(false))
        .execute(&mut connect()?)?;

    for prev_id in &pdu.prev_events {
        let new_edge = NewDbEventEdge {
            room_id: pdu.room_id.clone(),
            event_depth: pdu.depth as i64,
            event_id: pdu.event_id.clone(),
            event_sn: pdu.event_sn,
            prev_id: prev_id.clone(),
        };
        if let Err(e) = new_edge.save() {
            error!("failed to save event edge: {}", e);
        }
    }

    // Update Relationships
    #[derive(Deserialize, Clone, Debug)]
    struct ExtractRelatesTo {
        #[serde(rename = "m.relates_to")]
        relates_to: Relation,
    }

    crate::event::search::save_pdu(pdu, &pdu_json)?;

    let frame_id = state::append_to_state(pdu)?;
    // We set the room state after inserting the pdu, so that we never have a moment in time
    // where events in the current room state do not exist
    state::set_room_state(&pdu.room_id, frame_id)?;

    if let Err(e) = push_action::increment_notification_counts(&pdu.event_id, notifies, highlights)
    {
        error!("failed to increment notification counts: {}", e);
    }

    let all_appservices = crate::appservice::all()?;
    for appservice in all_appservices.values() {
        let in_room = super::appservice_in_room(&pdu.room_id, appservice)?;
        if in_room {
            crate::sending::send_pdu_appservice(appservice.registration.id.clone(), &pdu.event_id)?;
            continue;
        }

        // If the RoomMember event has a non-empty state_key, it is targeted at someone.
        // If it is our appservice user, we send this PDU to it.
        if pdu.event_ty == TimelineEventType::RoomMember
            && let Some(state_key_uid) = &pdu
                .state_key
                .as_ref()
                .and_then(|state_key| UserId::parse(state_key.as_str()).ok())
            && let Ok(appservice_uid) = UserId::parse_with_server_name(
                &*appservice.registration.sender_localpart,
                &conf.server_name,
            )
            && state_key_uid == &appservice_uid
        {
            crate::sending::send_pdu_appservice(appservice.registration.id.clone(), &pdu.event_id)?;
            continue;
        }
        let matching_users = || {
            config::get().server_name == pdu.sender.server_name()
                && appservice.is_user_match(&pdu.sender)
                || pdu.event_ty == TimelineEventType::RoomMember
                    && pdu.state_key.as_ref().is_some_and(|state_key| {
                        UserId::parse(state_key).is_ok_and(|user_id| {
                            config::get().server_name == user_id.server_name()
                                && appservice.is_user_match(&user_id)
                        })
                    })
        };
        let matching_aliases = || {
            super::local_aliases_for_room(&pdu.room_id)
                .unwrap_or_default()
                .iter()
                .any(|room_alias| appservice.aliases.is_match(room_alias.as_str()))
                || if let Ok(pdu) =
                    super::get_state(&pdu.room_id, &StateEventType::RoomCanonicalAlias, "", None)
                {
                    pdu.get_content::<RoomCanonicalAliasEventContent>()
                        .is_ok_and(|content| {
                            content
                                .alias
                                .is_some_and(|alias| appservice.aliases.is_match(alias.as_str()))
                                || content
                                    .alt_aliases
                                    .iter()
                                    .any(|alias| appservice.aliases.is_match(alias.as_str()))
                        })
                } else {
                    false
                }
        };

        if matching_aliases() || appservice.rooms.is_match(pdu.room_id.as_str()) || matching_users()
        {
            crate::sending::send_pdu_appservice(appservice.registration.id.clone(), &pdu.event_id)?;
        }
    }
    Ok(())
}

fn check_pdu_for_admin_room(pdu: &PduEvent, sender: &UserId) -> AppResult<()> {
    let conf = crate::config::get();
    match pdu.event_type() {
        TimelineEventType::RoomEncryption => {
            warn!("Encryption is not allowed in the admins room");
            return Err(MatrixError::forbidden(
                "Encryption is not allowed in the admins room.",
                None,
            )
            .into());
        }
        TimelineEventType::RoomMember => {
            #[derive(Deserialize)]
            struct ExtractMembership {
                membership: MembershipState,
            }

            let target = pdu
                .state_key
                .clone()
                .filter(|v| v.starts_with("@"))
                .unwrap_or(sender.as_str().to_owned());
            let server_name = &conf.server_name;
            let server_user = config::server_user_id();
            let content = pdu
                .get_content::<ExtractMembership>()
                .map_err(|_| AppError::internal("invalid content in pdu."))?;

            if content.membership == MembershipState::Leave {
                if target == *server_user {
                    warn!("Palpo user cannot leave from admins room");
                    return Err(MatrixError::forbidden(
                        "Palpo user cannot leave from admins room.",
                        None,
                    )
                    .into());
                }

                let count = super::joined_users(pdu.room_id(), None)?
                    .iter()
                    .filter(|m| m.server_name() == server_name)
                    .filter(|m| m.as_str() != target)
                    .count();
                if count < 2 {
                    warn!("Last admin cannot leave from admins room");
                    return Err(MatrixError::forbidden(
                        "Last admin cannot leave from admins room.",
                        None,
                    )
                    .into());
                }
            }

            if content.membership == MembershipState::Ban && pdu.state_key().is_some() {
                if target == *server_user {
                    warn!("Palpo user cannot be banned in admins room");
                    return Err(MatrixError::forbidden(
                        "Palpo user cannot be banned in admins room.",
                        None,
                    )
                    .into());
                }

                let count = super::joined_users(pdu.room_id(), None)?
                    .iter()
                    .filter(|m| m.server_name() == server_name)
                    .filter(|m| m.as_str() != target)
                    .count();
                if count < 2 {
                    warn!("Last admin cannot be banned in admins room");
                    return Err(MatrixError::forbidden(
                        "Last admin cannot be banned in admins room.",
                        None,
                    )
                    .into());
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Creates a new persisted data unit and adds it to a room.
#[tracing::instrument(skip_all)]
pub async fn build_and_append_pdu(
    pdu_builder: PduBuilder,
    sender: &UserId,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    state_lock: &RoomMutexGuard,
) -> AppResult<SnPduEvent> {
    if let Some(state_key) = &pdu_builder.state_key
        && let Ok(curr_state) = super::get_state(
            room_id,
            &pdu_builder.event_type.to_string().into(),
            state_key,
            None,
        )
        && curr_state.content.get() == pdu_builder.content.get()
    {
        return Ok(curr_state);
    }

    let (pdu, pdu_json, _event_guard) = pdu_builder
        .hash_sign_save(sender, room_id, room_version, state_lock)
        .await?;
    let room_id = &pdu.room_id;
    crate::room::ensure_room(room_id, room_version)?;

    // let conf = crate::config::get();
    // let admin_room = super::resolve_local_alias(
    //     <&RoomAliasId>::try_from(format!("#admins:{}", &conf.server_name).as_str())
    //         .expect("#admins:server_name is a valid room alias"),
    // )?;
    if crate::room::is_admin_room(room_id)? {
        check_pdu_for_admin_room(&pdu, sender)?;
    }

    append_pdu(&pdu, pdu_json, state_lock).await?;

    // In case we are kicking or banning a user, we need to inform their server of the change
    // move to append pdu
    // if pdu.event_ty == TimelineEventType::RoomMember {
    //     crate::room::update_joined_servers(&room_id)?;
    //     crate::room::update_currents(&room_id)?;
    // }

    let servers = super::participating_servers(room_id, false)?;
    crate::sending::send_pdu_servers(servers.into_iter(), &pdu.event_id)?;

    Ok(pdu)
}

/// Replace a PDU with the redacted form.
#[tracing::instrument(skip(reason))]
pub fn redact_pdu(event_id: &EventId, reason: &PduEvent) -> AppResult<()> {
    // TODO: Don't reserialize, keep original json
    if let Ok(mut pdu) = get_pdu(event_id) {
        pdu.redact(reason)?;
        replace_pdu(event_id, &to_canonical_object(&pdu)?)?;
        diesel::update(events::table.filter(events::id.eq(event_id)))
            .set(events::is_redacted.eq(true))
            .execute(&mut connect()?)?;
        diesel::delete(event_searches::table.filter(event_searches::event_id.eq(event_id)))
            .execute(&mut connect()?)?;
    }
    // If event does not exist, just noop
    Ok(())
}

pub fn is_event_next_to_backward_gap(event: &PduEvent) -> AppResult<bool> {
    let mut event_ids = event.prev_events.clone();
    event_ids.push(event.event_id().to_owned());
    let query = event_backward_extremities::table
        .filter(event_backward_extremities::room_id.eq(event.room_id()))
        .filter(event_backward_extremities::event_id.eq_any(event_ids));
    Ok(diesel_exists!(query, &mut connect()?)?)
}

pub fn is_event_next_to_forward_gap(event: &PduEvent) -> AppResult<bool> {
    let mut event_ids = event.prev_events.clone();
    event_ids.push(event.event_id().to_owned());
    let query = event_forward_extremities::table
        .filter(event_forward_extremities::room_id.eq(event.room_id()))
        .filter(event_forward_extremities::event_id.eq_any(event_ids));
    Ok(diesel_exists!(query, &mut connect()?)?)
}
