use std::collections::{BTreeMap, HashMap, HashSet, hash_map::Entry};

use palpo_core::Seqnum;
use state::DbRoomStateField;

use crate::core::UnixMillis;
use crate::core::client::filter::{FilterDefinition, LazyLoadOptions, RoomEventFilter};
use crate::core::client::sync_events::UnreadNotificationsCount;
use crate::core::client::sync_events::v3::{
    Ephemeral, Filter, GlobalAccountData, InviteState, InvitedRoom, JoinedRoom, KnockState,
    KnockedRoom, LeftRoom, Presence, RoomAccountData, RoomSummary, Rooms, State, SyncEventsReqArgs,
    SyncEventsResBody, Timeline, ToDevice,
};
use crate::core::device::DeviceLists;
use crate::core::events::receipt::ReceiptEvent;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{
    AnyRawAccountDataEvent, AnySyncEphemeralRoomEvent, StateEventType, TimelineEventType,
};
use crate::core::identifiers::*;
use crate::core::serde::RawJson;
use crate::event::{EventHash, PduEvent, SnPduEvent};
use crate::room::{state, timeline};
use crate::{AppError, AppResult, config, data, extract_variant, room};

pub const DEFAULT_BUMP_TYPES: &[TimelineEventType; 6] = &[
    TimelineEventType::CallInvite,
    TimelineEventType::PollStart,
    TimelineEventType::Beacon,
    TimelineEventType::RoomEncrypted,
    TimelineEventType::RoomMessage,
    TimelineEventType::Sticker,
];

#[tracing::instrument(skip_all)]
pub async fn sync_events(
    sender_id: &UserId,
    device_id: &DeviceId,
    args: &SyncEventsReqArgs,
) -> AppResult<SyncEventsResBody> {
    let curr_sn = data::curr_sn()?;
    crate::seqnum_reach(curr_sn).await;
    let since_sn = if let Some(since) = args.since.as_ref() {
        Some(
            since
                .parse()
                .map_err(|_| AppError::public("Invalid `since` parameter, must be a number."))?,
        )
    } else {
        None
    };
    let next_batch = curr_sn + 1;

    // Load filter
    let filter = match &args.filter {
        None => FilterDefinition::default(),
        Some(Filter::FilterDefinition(filter)) => filter.to_owned(),
        Some(Filter::FilterId(filter_id)) => {
            data::user::get_filter(sender_id, filter_id.parse::<i64>().unwrap_or_default())?
        }
    };
    let _lazy_load_enabled = filter.room.state.lazy_load_options.is_enabled()
        || filter.room.timeline.lazy_load_options.is_enabled();

    let full_state = args.full_state;

    let mut joined_rooms = BTreeMap::new();
    let mut presence_updates = HashMap::new();
    let mut joined_users = HashSet::new(); // Users that have joined any encrypted rooms the sender was in
    let mut left_users = HashSet::new(); // Users that have left any encrypted rooms the sender was in
    let mut device_list_updates = HashSet::new();
    let mut device_list_left = HashSet::new();

    // Look for device list updates of this account
    device_list_updates.extend(data::user::keys_changed_users(
        sender_id,
        since_sn.unwrap_or_default(),
        None,
    )?);

    let all_joined_rooms = data::user::joined_rooms(sender_id)?;
    for room_id in &all_joined_rooms {
        let joined_room = match load_joined_room(
            sender_id,
            device_id,
            room_id,
            since_sn,
            Some(curr_sn),
            next_batch,
            full_state,
            &filter,
            &mut device_list_updates,
            &mut joined_users,
            &mut left_users,
        )
        .await
        {
            Ok(joined_room) => joined_room,
            Err(e) => {
                tracing::error!(error = ?e, "load joined room failed");
                continue;
            }
        };
        if !joined_room.is_empty() {
            joined_rooms.insert(room_id.to_owned(), joined_room);
        }
    }

    let mut left_rooms = BTreeMap::new();
    let all_left_rooms = room::user::left_rooms(sender_id, since_sn)?;

    for room_id in all_left_rooms.keys() {
        let Ok(left_sn) = room::user::left_sn(room_id, sender_id) else {
            tracing::warn!("left room {} without a left_sn, skipping", room_id);
            continue;
        };
        // Left before last sync
        if since_sn > Some(left_sn) {
            continue;
        }
        let left_room = match load_left_room(
            sender_id,
            device_id,
            room_id,
            since_sn,
            Some(curr_sn),
            left_sn,
            next_batch,
            full_state,
            &filter,
            &mut device_list_updates,
            &mut joined_users,
            &mut left_users,
        )
        .await
        {
            Ok(left_room) => left_room,
            Err(e) => {
                tracing::error!(error = ?e, "load joined room failed");
                continue;
            }
        };
        left_rooms.insert(room_id.to_owned(), left_room);
    }

    let invited_rooms: BTreeMap<_, _> =
        data::user::invited_rooms(sender_id, since_sn.unwrap_or_default())?
            .into_iter()
            .map(|(room_id, invite_state_events)| {
                (
                    room_id,
                    InvitedRoom {
                        invite_state: InviteState {
                            events: invite_state_events,
                        },
                    },
                )
            })
            .collect();

    for left_room in left_rooms.keys() {
        for user_id in room::joined_users(left_room, None)? {
            let dont_share_encrypted_room =
                room::user::shared_rooms(vec![sender_id.to_owned(), user_id.clone()])?
                    .into_iter()
                    .map(|other_room_id| {
                        room::get_state(&other_room_id, &StateEventType::RoomEncryption, "", None)
                            .is_ok()
                    })
                    .all(|encrypted| !encrypted);
            // If the user doesn't share an encrypted room with the target anymore, we need
            // to tell them.
            if dont_share_encrypted_room {
                device_list_left.insert(user_id);
            }
        }
    }
    for user_id in left_users {
        let dont_share_encrypted_room =
            room::user::shared_rooms(vec![sender_id.to_owned(), user_id.clone()])?
                .into_iter()
                .map(|other_room_id| {
                    room::get_state(&other_room_id, &StateEventType::RoomEncryption, "", None)
                        .is_ok()
                })
                .all(|encrypted| !encrypted);
        // If the user doesn't share an encrypted room with the target anymore, we need to tell
        // them
        if dont_share_encrypted_room {
            device_list_left.insert(user_id);
        }
    }

    let knocked_rooms = data::user::knocked_rooms(sender_id, 0)?.into_iter().fold(
        BTreeMap::default(),
        |mut knocked_rooms: BTreeMap<_, _>, (room_id, knock_state)| {
            let knock_sn = room::user::knock_sn(sender_id, &room_id).ok();

            // Knocked before last sync
            if since_sn > knock_sn {
                return knocked_rooms;
            }

            let knocked_room = KnockedRoom {
                knock_state: KnockState {
                    events: knock_state,
                },
            };

            knocked_rooms.insert(room_id, knocked_room);
            knocked_rooms
        },
    );

    if config::get().presence.allow_local {
        // Take presence updates from this room
        for (user_id, presence_event) in
            crate::data::user::presences_since(since_sn.unwrap_or_default())?
        {
            if user_id == sender_id || !state::user_can_see_user(sender_id, &user_id)? {
                continue;
            }

            match presence_updates.entry(user_id) {
                Entry::Vacant(slot) => {
                    slot.insert(presence_event);
                }
                Entry::Occupied(mut slot) => {
                    let curr_event = slot.get_mut();
                    let curr_content = &mut curr_event.content;
                    let new_content = presence_event.content;

                    // Update existing presence event with more info
                    curr_content.presence = new_content.presence;
                    curr_content.status_msg =
                        new_content.status_msg.or(curr_content.status_msg.take());
                    curr_content.last_active_ago =
                        new_content.last_active_ago.or(curr_content.last_active_ago);
                    curr_content.display_name = new_content
                        .display_name
                        .or(curr_content.display_name.take());
                    curr_content.avatar_url =
                        new_content.avatar_url.or(curr_content.avatar_url.take());
                    curr_content.currently_active = new_content
                        .currently_active
                        .or(curr_content.currently_active);
                }
            }
        }
        for joined_user in &joined_users {
            if !presence_updates.contains_key(joined_user)
                && let Ok(presence) = data::user::last_presence(joined_user)
            {
                presence_updates.insert(joined_user.to_owned(), presence);
            }
        }
    }

    // Remove all to-device events the device received *last time*
    data::user::device::remove_to_device_events(
        sender_id,
        device_id,
        since_sn.unwrap_or_default() - 1,
    )?;

    let account_data = GlobalAccountData {
        events: data::user::data_changes(None, sender_id, since_sn.unwrap_or_default(), None)?
            .into_iter()
            .filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Global))
            .collect(),
    };

    let rooms = Rooms {
        leave: left_rooms,
        join: joined_rooms,
        invite: invited_rooms,
        knock: knocked_rooms,
    };
    let presence = Presence {
        events: presence_updates
            .into_values()
            .map(|v| RawJson::new(&v).expect("PresenceEvent always serializes successfully"))
            .collect(),
    };
    let device_lists = DeviceLists {
        changed: device_list_updates.into_iter().collect(),
        left: device_list_left.into_iter().collect(),
    };

    let to_device = ToDevice {
        events: data::user::device::get_to_device_events(
            sender_id,
            device_id,
            since_sn,
            Some(next_batch),
        )?,
    };

    let res_body = SyncEventsResBody {
        next_batch: next_batch.to_string(),
        rooms,
        presence,
        account_data,
        device_lists,
        device_one_time_keys_count: { data::user::count_one_time_keys(sender_id, device_id)? },
        to_device,
        // Fallback keys are not yet supported
        device_unused_fallback_key_types: None,
    };
    Ok(res_body)
}

#[tracing::instrument(skip_all)]
async fn load_joined_room(
    sender_id: &UserId,
    device_id: &DeviceId,
    room_id: &RoomId,
    since_sn: Option<Seqnum>,
    until_sn: Option<Seqnum>,
    next_batch: Seqnum,
    full_state: bool,
    filter: &FilterDefinition,
    device_list_updates: &mut HashSet<OwnedUserId>,
    joined_users: &mut HashSet<OwnedUserId>,
    left_users: &mut HashSet<OwnedUserId>,
) -> AppResult<JoinedRoom> {
    if since_sn > Some(data::curr_sn()?) {
        return Ok(JoinedRoom::default());
    }
    let lazy_load_enabled = filter.room.state.lazy_load_options.is_enabled()
        || filter.room.timeline.lazy_load_options.is_enabled();

    let lazy_load_send_redundant = match filter.room.state.lazy_load_options {
        LazyLoadOptions::Enabled {
            include_redundant_members: redundant,
        } => redundant,
        _ => false,
    };

    let current_frame_id = room::get_frame_id(room_id, None)?;
    let since_frame_id = crate::event::get_last_frame_id(room_id, since_sn).ok();

    let (timeline_pdus, limited) = load_timeline(
        sender_id,
        room_id,
        since_sn,
        Some(next_batch),
        Some(&filter.room.timeline),
    )?;

    let since_sn = if let Some(since_sn) = since_sn {
        since_sn
    } else {
        crate::room::user::join_sn(sender_id, room_id).unwrap_or_default()
    };

    let send_notification_counts = !timeline_pdus.is_empty()
        || room::user::last_read_notification(sender_id, room_id)? >= since_sn;
    let mut timeline_users = HashSet::new();
    let mut timeline_pdu_ids = HashSet::new();
    for (_, event) in &timeline_pdus {
        timeline_users.insert(event.sender.as_str().to_owned());
        timeline_pdu_ids.insert(event.event_id.clone());
    }
    room::lazy_loading::lazy_load_confirm_delivery(sender_id, device_id, room_id, since_sn)?;

    let (heroes, joined_member_count, invited_member_count, joined_since_last_sync, state_events) =
        if timeline_pdus.is_empty()
            && (since_frame_id == Some(current_frame_id) || since_frame_id.is_none())
        {
            // No state changes
            (Vec::new(), None, None, false, Vec::new())
        } else {
            // Calculates joined_member_count, invited_member_count and heroes
            let calculate_counts = || {
                let joined_member_count = room::joined_member_count(room_id).unwrap_or(0);
                let invited_member_count = room::invited_member_count(room_id).unwrap_or(0);

                // Recalculate heroes (first 5 members)
                let mut heroes = Vec::new();

                if joined_member_count + invited_member_count <= 5 {
                    // Go through all PDUs and for each member event, check if the user is still joined or
                    // invited until we have 5 or we reach the end
                    for hero in timeline::all_pdus(sender_id, room_id, until_sn)?
                        .into_iter() // Ignore all broken pdus
                        .filter(|(_, pdu)| pdu.event_ty == TimelineEventType::RoomMember)
                        .map(|(_, pdu)| {
                            let content = pdu.get_content::<RoomMemberEventContent>()?;

                            if let Some(state_key) = &pdu.state_key {
                                let user_id = UserId::parse(state_key.clone()).map_err(|_| {
                                    AppError::public("Invalid UserId in member PDU.")
                                })?;

                                // The membership was and still is invite or join
                                if matches!(
                                    content.membership,
                                    MembershipState::Join | MembershipState::Invite
                                ) && (room::user::is_joined(&user_id, room_id)?
                                    || room::user::is_invited(&user_id, room_id)?)
                                {
                                    Ok::<_, AppError>(Some(state_key.clone()))
                                } else {
                                    Ok(None)
                                }
                            } else {
                                Ok(None)
                            }
                        })
                        // Filter out buggy users
                        .filter_map(|u| u.ok())
                        // Filter for possible heroes
                        .flatten()
                    {
                        if heroes.contains(&hero) || hero == sender_id.as_str() {
                            continue;
                        }

                        heroes.push(hero);
                    }
                }
                Ok::<_, AppError>((
                    Some(joined_member_count),
                    Some(invited_member_count),
                    heroes,
                ))
            };

            let joined_since_last_sync = room::user::join_sn(sender_id, room_id)? >= since_sn;
            if since_sn == 0 || joined_since_last_sync {
                // Probably since = 0, we will do an initial sync
                let (joined_member_count, invited_member_count, heroes) = calculate_counts()?;
                let current_state_ids =
                    state::get_full_state_ids(since_frame_id.unwrap_or(current_frame_id))?;
                let mut state_events = Vec::new();
                let mut lazy_loaded = HashSet::new();

                for (state_key_id, event_id) in current_state_ids {
                    if timeline_pdu_ids.contains(&event_id) {
                        continue;
                    }
                    if let Some(state_limit) = filter.room.state.limit
                        && state_events.len() >= state_limit
                    {
                        break;
                    }
                    let DbRoomStateField {
                        event_ty,
                        state_key,
                        ..
                    } = state::get_field(state_key_id)?;

                    if event_ty != StateEventType::RoomMember {
                        let Ok(pdu) = timeline::get_pdu(&event_id) else {
                            error!("pdu in state not found: {}", event_id);
                            continue;
                        };
                        if pdu.can_pass_filter(&filter.room.state) {
                            state_events.push(pdu);
                        }
                    } else if !lazy_load_enabled
                    || full_state
                    || timeline_users.contains(&state_key)
                    // TODO: Delete the following line when this is resolved: https://github.com/vector-im/element-web/issues/22565
                    || *sender_id == state_key
                    {
                        let Ok(pdu) = timeline::get_pdu(&event_id) else {
                            error!("pdu in state not found: {}", event_id);
                            continue;
                        };

                        // This check is in case a bad user ID made it into the database
                        if let Ok(uid) = UserId::parse(&state_key) {
                            lazy_loaded.insert(uid);
                        }
                        if pdu.can_pass_filter(&filter.room.state) {
                            state_events.push(pdu);
                        }
                    }
                }

                // Reset lazy loading because this is an initial sync
                room::lazy_loading::lazy_load_reset(sender_id, device_id, room_id)?;
                // The state_events above should contain all timeline_users, let's mark them as lazy loaded.
                room::lazy_loading::lazy_load_mark_sent(
                    sender_id,
                    device_id,
                    room_id,
                    lazy_loaded,
                    next_batch,
                );

                // && encrypted_room || new_encrypted_room {
                // If the user is in a new encrypted room, give them all joined users
                *joined_users = room::joined_users(room_id, None)?.into_iter().collect();
                // .into_iter()
                // .filter(|user_id| {
                //     // Don't send key updates from the sender to the sender
                //     sender_id != user_id
                // })
                // .collect();
                device_list_updates.extend(
                    joined_users.clone().into_iter(), // .filter(|user_id| {
                                                      // Only send keys if the sender doesn't share an encrypted room with the target already
                                                      // !share_encrypted_room(sender_id, user_id, &room_id).unwrap_or(false)
                                                      // }),
                );
                (
                    heroes,
                    joined_member_count,
                    invited_member_count,
                    true,
                    state_events,
                )
            } else if let Some(since_frame_id) = since_frame_id {
                // Incremental /sync
                let mut state_events = Vec::new();
                let mut lazy_loaded = HashSet::new();

                if since_frame_id != current_frame_id {
                    let current_state_ids = state::get_full_state_ids(current_frame_id)?;
                    let since_state_ids = state::get_full_state_ids(since_frame_id)?;

                    for (key, id) in current_state_ids {
                        if let Some(state_limit) = filter.room.state.limit
                            && state_events.len() >= state_limit
                        {
                            break;
                        }
                        if full_state || since_state_ids.get(&key) != Some(&id) {
                            let pdu = match timeline::get_pdu(&id) {
                                Ok(pdu) => pdu,
                                Err(_) => {
                                    error!("pdu in state not found: {}", id);
                                    continue;
                                }
                            };
                            if pdu.event_ty == TimelineEventType::RoomMember {
                                match UserId::parse(
                                    pdu.state_key
                                        .as_ref()
                                        .expect("state event has state key")
                                        .clone(),
                                ) {
                                    Ok(state_key_user_id) => {
                                        lazy_loaded.insert(state_key_user_id);
                                    }
                                    Err(e) => error!("invalid state key for member event: {}", e),
                                }
                            }
                            if pdu.can_pass_filter(&filter.room.state) {
                                state_events.push(pdu);
                            }
                        }
                    }
                }

                for (_, event) in &timeline_pdus {
                    if let Some(state_limit) = filter.room.state.limit
                        && state_events.len() >= state_limit
                    {
                        break;
                    }
                    if lazy_loaded.contains(&event.sender) {
                        continue;
                    }
                    if (!room::lazy_loading::lazy_load_was_sent_before(
                        sender_id,
                        device_id,
                        room_id,
                        &event.sender,
                    )? || lazy_load_send_redundant)
                        && let Ok(member_event) = room::get_state(
                            room_id,
                            &StateEventType::RoomMember,
                            event.sender.as_str(),
                            None,
                        )
                    {
                        lazy_loaded.insert(event.sender.clone());
                        if member_event.can_pass_filter(&filter.room.state) {
                            state_events.push(member_event);
                        }
                    }
                }
                room::lazy_loading::lazy_load_mark_sent(
                    sender_id,
                    device_id,
                    room_id,
                    lazy_loaded,
                    next_batch,
                );

                let encrypted_room =
                    state::get_state(current_frame_id, &StateEventType::RoomEncryption, "").is_ok();
                let since_encryption =
                    state::get_state(since_frame_id, &StateEventType::RoomEncryption, "").ok();
                // Calculations:
                let _new_encrypted_room = encrypted_room && since_encryption.is_none();
                let send_member_count = state_events
                    .iter()
                    .any(|event| event.event_ty == TimelineEventType::RoomMember);

                // if encrypted_room {
                for state_event in &state_events {
                    if state_event.event_ty != TimelineEventType::RoomMember {
                        continue;
                    }

                    if let Some(state_key) = &state_event.state_key {
                        let user_id = UserId::parse(state_key.clone())
                            .map_err(|_| AppError::public("invalid UserId in member PDU."))?;

                        if user_id == sender_id {
                            continue;
                        }

                        let new_membership = state_event
                            .get_content::<RoomMemberEventContent>()
                            .map_err(|_| AppError::public("invalid PDU in database."))?
                            .membership;

                        match new_membership {
                            MembershipState::Join => {
                                // A new user joined an encrypted room
                                // if !share_encrypted_room(sender_id, &user_id, &room_id)? {
                                if since_sn <= state_event.event_sn
                                    && !room::user::shared_rooms(vec![
                                        sender_id.to_owned(),
                                        user_id.to_owned(),
                                    ])?
                                    .is_empty()
                                {
                                    // if user_id.is_local() {
                                    // check for test TestDeviceListsUpdateOverFederation
                                    // device_list_updates.insert(user_id.clone());
                                    // }
                                    joined_users.insert(user_id);
                                }
                            }
                            MembershipState::Leave => {
                                // Write down users that have left encrypted rooms we are in
                                left_users.insert(user_id);
                            }
                            _ => {}
                        }
                    }
                }
                // }

                if joined_since_last_sync {
                    // && encrypted_room || new_encrypted_room {
                    // If the user is in a new encrypted room, give them all joined users
                    device_list_updates.extend(
                        room::joined_users(room_id, None)?
                            .into_iter()
                            .filter(|user_id| {
                                // Don't send key updates from the sender to the sender
                                sender_id != user_id
                            }), // .filter(|user_id| {
                                // Only send keys if the sender doesn't share an encrypted room with the target already
                                // !share_encrypted_room(sender_id, user_id, &room_id).unwrap_or(false)
                                // }),
                    );
                }

                let (joined_member_count, invited_member_count, heroes) = if send_member_count {
                    calculate_counts()?
                } else {
                    (None, None, Vec::new())
                };

                (
                    heroes,
                    joined_member_count,
                    invited_member_count,
                    joined_since_last_sync,
                    state_events,
                )
            } else {
                (Vec::new(), None, None, false, Vec::new())
            }
        };

    // Look for device list updates in this room
    device_list_updates.extend(room::keys_changed_users(room_id, since_sn, None)?);

    let mut limited = limited || joined_since_last_sync;
    if let Some((_, first_event)) = timeline_pdus.first()
        && first_event.event_ty == TimelineEventType::RoomCreate
    {
        limited = false;
    }
    let prev_batch = if limited {
        timeline_pdus.first().map(|(sn, _)| sn.to_string())
    } else {
        timeline_pdus.last().map(|(sn, _)| sn.to_string())
    };

    let mut edus: Vec<RawJson<AnySyncEphemeralRoomEvent>> = Vec::new();
    for (_, content) in data::room::receipt::read_receipts(room_id, since_sn)? {
        let receipt = ReceiptEvent {
            room_id: room_id.to_owned(),
            content,
        };
        edus.push(RawJson::new(&receipt)?.cast());
    }
    if room::typing::last_typing_update(room_id).await? >= since_sn {
        edus.push(
            serde_json::from_str(&serde_json::to_string(
                &room::typing::all_typings(room_id).await?,
            )?)
            .expect("event is valid, we just created it"),
        );
    }

    let account_events = data::user::data_changes(Some(room_id), sender_id, since_sn, None)?
        .into_iter()
        .filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Room))
        .collect();
    let notify_summary = room::user::notify_summary(sender_id, room_id)?;
    let mut notification_count = None;
    let mut highlight_count = None;
    let mut unread_count = None;
    if send_notification_counts {
        if filter.room.timeline.unread_thread_notifications {
            notification_count = Some(notify_summary.notification_count);
            highlight_count = Some(notify_summary.highlight_count);
        } else {
            notification_count = Some(notify_summary.all_notification_count());
            highlight_count = Some(notify_summary.all_highlight_count());
        }
        unread_count = Some(notify_summary.all_unread_count())
    }

    Ok(JoinedRoom {
        account_data: RoomAccountData {
            events: account_events,
        },
        summary: RoomSummary {
            heroes,
            joined_member_count: joined_member_count.map(|n| (n as u32).into()),
            invited_member_count: invited_member_count.map(|n| (n as u32).into()),
        },
        unread_notifications: UnreadNotificationsCount {
            highlight_count,
            notification_count,
        },
        timeline: Timeline {
            limited,
            prev_batch,
            events: timeline_pdus
                .iter()
                .map(|(_, pdu)| pdu.to_sync_room_event())
                .collect(),
        },
        state: State {
            events: state_events
                .iter()
                .map(|pdu| pdu.to_sync_state_event())
                .collect(),
        },
        ephemeral: Ephemeral { events: edus },
        unread_thread_notifications: if filter.room.timeline.unread_thread_notifications {
            notify_summary
                .threads
                .iter()
                .map(|(thread_id, summary)| {
                    (
                        thread_id.to_owned(),
                        UnreadNotificationsCount {
                            notification_count: Some(summary.notification_count),
                            highlight_count: Some(summary.highlight_count),
                        },
                    )
                })
                .collect()
        } else {
            BTreeMap::new()
        },
        unread_count,
    })
}

#[tracing::instrument(skip_all)]
async fn load_left_room(
    sender_id: &UserId,
    _device_id: &DeviceId,
    room_id: &RoomId,
    since_sn: Option<Seqnum>,
    _until_sn: Option<Seqnum>,
    _left_sn: Seqnum,
    next_batch: Seqnum,
    _full_state: bool,
    filter: &FilterDefinition,
    _device_list_updates: &mut HashSet<OwnedUserId>,
    _joined_users: &mut HashSet<OwnedUserId>,
    _left_users: &mut HashSet<OwnedUserId>,
) -> AppResult<LeftRoom> {
    let conf = crate::config::get();
    if !room::room_exists(room_id)? {
        let event = PduEvent {
            event_id: EventId::new(&conf.server_name),
            sender: sender_id.to_owned(),
            origin_server_ts: UnixMillis::now(),
            event_ty: TimelineEventType::RoomMember,
            content: serde_json::from_str(r#"{"membership":"leave"}"#).expect("this is valid JSON"),
            state_key: Some(sender_id.to_string()),
            unsigned: Default::default(),
            // The following keys are dropped on conversion
            room_id: room_id.to_owned(),
            prev_events: vec![],
            depth: 1,
            auth_events: vec![],
            redacts: None,
            hashes: EventHash {
                sha256: String::new(),
            },
            signatures: None,
            extra_data: Default::default(),
            rejection_reason: None,
        };
        return Ok(LeftRoom {
            account_data: RoomAccountData::default(),
            timeline: Timeline {
                limited: false,
                prev_batch: Some(next_batch.to_string()),
                events: Vec::new(),
            },
            state: State {
                events: vec![event.to_sync_state_event()],
            },
        });
    }

    let since_frame_id = crate::event::get_last_frame_id(room_id, since_sn);
    let _since_state_ids = match since_frame_id {
        Ok(s) => state::get_full_state_ids(s)?,
        _ => HashMap::new(),
    };

    let curr_frame_id = room::get_frame_id(room_id, None)?;
    let left_event_id = state::get_state_event_id(
        curr_frame_id,
        &StateEventType::RoomMember,
        sender_id.as_str(),
    )?;
    let left_frame_id = state::get_pdu_frame_id(&left_event_id)?;
    let (timeline_pdus, mut limited) = load_timeline(
        sender_id,
        room_id,
        since_sn,
        None,
        Some(&filter.room.timeline),
    )?;

    let since_sn = since_sn.unwrap_or_default();

    let _send_notification_counts = !timeline_pdus.is_empty()
        || room::user::last_read_notification(sender_id, room_id)? >= since_sn;
    let mut timeline_users = HashSet::new();
    let mut timeline_pdu_ids = HashSet::new();
    for (_, event) in &timeline_pdus {
        timeline_users.insert(event.sender.as_str().to_owned());
        timeline_pdu_ids.insert(event.event_id.clone());
    }

    let mut state_events = Vec::new();
    let mut left_state_ids = state::get_full_state_ids(left_frame_id)?;
    let leave_state_key_id =
        state::ensure_field_id(&StateEventType::RoomMember, sender_id.as_str())?;
    left_state_ids.insert(leave_state_key_id, left_event_id.clone());

    for (key, event_id) in left_state_ids {
        if let Some(state_limit) = filter.room.state.limit
            && state_events.len() >= state_limit
        {
            break;
        }
        if timeline_pdu_ids.contains(&event_id) {
            continue;
        }
        let DbRoomStateField { state_key, .. } = state::get_field(key)?;

        if *sender_id == state_key {
            let pdu = match timeline::get_pdu(&event_id) {
                Ok(pdu) => pdu,
                _ => {
                    error!("pdu in state not found: {}", event_id);
                    continue;
                }
            };

            if pdu.can_pass_filter(&filter.room.state) {
                state_events.push(pdu);
            }
        }
    }

    if let Some((_, first_event)) = timeline_pdus.first()
        && first_event.event_ty == TimelineEventType::RoomCreate
    {
        limited = false;
    }
    let prev_batch = if limited {
        timeline_pdus.first().map(|(sn, _)| sn.to_string())
    } else {
        timeline_pdus.last().map(|(sn, _)| sn.to_string())
    };

    let _left_event = timeline::get_pdu(&left_event_id).map(|pdu| pdu.to_sync_room_event());
    Ok(LeftRoom {
        account_data: RoomAccountData { events: Vec::new() },
        timeline: Timeline {
            limited,
            prev_batch,
            events: timeline_pdus
                .iter()
                .map(|(_, pdu)| pdu.to_sync_room_event())
                .collect(),
        },
        state: State {
            events: state_events
                .iter()
                .map(|pdu| pdu.to_sync_state_event())
                .collect(),
        },
    })
}
#[tracing::instrument]
pub(crate) fn load_timeline(
    user_id: &UserId,
    room_id: &RoomId,
    since_sn: Option<Seqnum>,
    until_sn: Option<Seqnum>,
    filter: Option<&RoomEventFilter>,
) -> AppResult<(Vec<(i64, SnPduEvent)>, bool)> {
    let limit = filter.and_then(|f| f.limit).unwrap_or(10);
    let mut timeline_pdus = if let Some(since_sn) = since_sn {
        if let Some(until_sn) = until_sn {
            let (min_sn, max_sn) = if until_sn > since_sn {
                (since_sn, until_sn)
            } else {
                (until_sn, since_sn)
            };

            timeline::get_pdus_backward(user_id, room_id, max_sn, Some(min_sn), filter, limit + 1)?
        } else {
            timeline::get_pdus_backward(
                user_id,
                room_id,
                i64::MAX,
                Some(since_sn),
                filter,
                limit + 1,
            )?
        }
    } else {
        timeline::get_pdus_backward(user_id, room_id, i64::MAX, None, filter, limit + 1)?
    };

    if timeline_pdus.len() > limit {
        timeline_pdus.remove(0);
        Ok((timeline_pdus, true))
    } else {
        Ok((timeline_pdus, false))
    }
}

#[tracing::instrument]
pub(crate) fn share_encrypted_room(
    sender_id: &UserId,
    user_id: &UserId,
    ignore_room: Option<&RoomId>,
) -> AppResult<bool> {
    let shared_rooms = room::user::shared_rooms(vec![sender_id.to_owned(), user_id.to_owned()])?
        .into_iter()
        .filter(|room_id| Some(&**room_id) != ignore_room)
        .any(|other_room_id| {
            room::get_state(&other_room_id, &StateEventType::RoomEncryption, "", None).is_ok()
        });

    Ok(shared_rooms)
}
