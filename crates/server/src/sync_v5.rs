use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use crate::core::Seqnum;
use crate::core::client::filter::RoomEventFilter;
use crate::core::client::sync_events::v5::*;
use crate::core::client::sync_events::{self};
use crate::core::device::DeviceLists;
use crate::core::events::receipt::{SyncReceiptEvent, combine_receipt_event_contents};
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{AnyRawAccountDataEvent, StateEventType, TimelineEventType};
use crate::core::identifiers::*;
use crate::event::{BatchToken, ignored_filter};
use crate::room::{self, filter_rooms, state, timeline};
use crate::sync_v3::{DEFAULT_BUMP_TYPES, TimelineData, share_encrypted_room};
use crate::{AppResult, data, extract_variant};

#[derive(Debug, Default)]
struct SlidingSyncCache {
    lists: BTreeMap<String, sync_events::v5::ReqList>,
    subscriptions: BTreeMap<OwnedRoomId, sync_events::v5::RoomSubscription>,
    known_rooms: KnownRooms, // For every room, the room_since_sn number
    extensions: sync_events::v5::ExtensionsConfig,
    required_state: BTreeSet<Seqnum>,
}

static CONNECTIONS: LazyLock<
    Mutex<BTreeMap<(OwnedUserId, OwnedDeviceId, Option<String>), Arc<Mutex<SlidingSyncCache>>>>,
> = LazyLock::new(Default::default);

#[tracing::instrument(skip_all)]
pub async fn sync_events(
    sender_id: &UserId,
    device_id: &DeviceId,
    since_sn: Seqnum,
    req_body: &SyncEventsReqBody,
    known_rooms: &KnownRooms,
) -> AppResult<SyncEventsResBody> {
    let curr_sn = data::curr_sn()?;
    crate::seqnum_reach(curr_sn).await;
    let next_batch = curr_sn + 1;
    if since_sn > curr_sn {
        return Ok(SyncEventsResBody::new(next_batch.to_string()));
    }

    let all_joined_rooms = data::user::joined_rooms(sender_id)?;

    let all_invited_rooms = data::user::invited_rooms(sender_id, 0)?;
    let all_invited_rooms: Vec<&RoomId> = all_invited_rooms.iter().map(|r| r.0.as_ref()).collect();

    let all_knocked_rooms = data::user::knocked_rooms(sender_id, 0)?;
    let all_knocked_rooms: Vec<&RoomId> = all_knocked_rooms.iter().map(|r| r.0.as_ref()).collect();

    let all_rooms: Vec<&RoomId> = all_joined_rooms
        .iter()
        .map(AsRef::as_ref)
        .chain(all_invited_rooms.iter().map(AsRef::as_ref))
        .chain(all_knocked_rooms.iter().map(AsRef::as_ref))
        .collect();

    let all_joined_rooms = all_joined_rooms.iter().map(AsRef::as_ref).collect();
    let all_invited_rooms = all_invited_rooms.iter().map(AsRef::as_ref).collect();

    let mut todo_rooms: TodoRooms = BTreeMap::new();

    let sync_info = SyncInfo {
        sender_id,
        device_id,
        since_sn,
        req_body,
    };
    let mut res_body = SyncEventsResBody {
        txn_id: req_body.txn_id.clone(),
        pos: next_batch.to_string(),
        lists: BTreeMap::new(),
        rooms: BTreeMap::new(),
        extensions: Extensions {
            account_data: collect_account_data(sync_info)?,
            e2ee: collect_e2ee(sync_info, &all_joined_rooms)?,
            to_device: collect_to_device(sync_info, next_batch),
            receipts: collect_receipts(),
            typing: collect_typing(sync_info, next_batch, all_rooms.iter().cloned()).await?,
        },
    };

    process_lists(
        sync_info,
        &all_invited_rooms,
        &all_joined_rooms,
        &all_rooms,
        &mut todo_rooms,
        known_rooms,
        &mut res_body,
    )
    .await;

    fetch_subscriptions(sync_info, &mut todo_rooms, known_rooms)?;

    res_body.rooms = process_rooms(
        sync_info,
        &all_invited_rooms,
        &todo_rooms,
        known_rooms,
        &mut res_body,
    )
    .await?;
    Ok(res_body)
}

#[allow(clippy::too_many_arguments)]
async fn process_lists(
    SyncInfo {
        sender_id,
        device_id,
        since_sn,
        req_body,
    }: SyncInfo<'_>,
    all_invited_rooms: &Vec<&RoomId>,
    all_joined_rooms: &Vec<&RoomId>,
    all_rooms: &Vec<&RoomId>,
    todo_rooms: &mut TodoRooms,
    known_rooms: &KnownRooms,
    res_body: &mut SyncEventsResBody,
) -> KnownRooms {
    for (list_id, list) in &req_body.lists {
        let active_rooms = match list.filters.clone().and_then(|f| f.is_invite) {
            Some(true) => all_invited_rooms,
            Some(false) => all_joined_rooms,
            None => all_rooms,
        };
        let active_rooms = match list.filters.clone().map(|f| f.not_room_types) {
            Some(filter) if filter.is_empty() => active_rooms,
            Some(value) => &filter_rooms(active_rooms, &value, true),
            None => active_rooms,
        };

        let mut new_known_rooms: BTreeSet<OwnedRoomId> = BTreeSet::new();
        let mut ranges = list.ranges.clone();
        if ranges.is_empty() {
            ranges.push((0, 50));
        }

        for mut range in ranges {
            if range == (0, 0) {
                range.1 = active_rooms.len().min(range.0 + 19);
            } else {
                range.1 = range.1.clamp(range.0, active_rooms.len().max(range.0));
            }

            let room_ids = active_rooms[range.0..range.1].to_vec();

            let new_rooms: BTreeSet<OwnedRoomId> =
                room_ids.clone().into_iter().map(From::from).collect();
            new_known_rooms.extend(new_rooms);
            // new_known_rooms.extend(room_ids..cloned());
            for room_id in room_ids {
                let todo_room = todo_rooms.entry(room_id.to_owned()).or_insert(TodoRoom {
                    required_state: BTreeSet::new(),
                    timeline_limit: 0_usize,
                    room_since_sn: since_sn,
                });

                let limit = list.room_details.timeline_limit.min(100);

                todo_room.required_state.extend(
                    list.room_details
                        .required_state
                        .iter()
                        .map(|(ty, sk)| (ty.clone(), sk.as_str().into())),
                );

                todo_room.timeline_limit = todo_room.timeline_limit.max(limit);
                todo_room.room_since_sn = todo_room.room_since_sn.min(
                    known_rooms
                        .get(list_id.as_str())
                        .and_then(|k| k.get(room_id))
                        .copied()
                        .unwrap_or(since_sn),
                );
            }
        }
        res_body.lists.insert(
            list_id.clone(),
            sync_events::v5::SyncList {
                count: active_rooms.len(),
            },
        );

        crate::sync_v5::update_sync_known_rooms(
            sender_id.to_owned(),
            device_id.to_owned(),
            req_body.conn_id.clone(),
            list_id.clone(),
            new_known_rooms,
            since_sn,
        );
    }
    BTreeMap::default()
}

fn fetch_subscriptions(
    SyncInfo {
        sender_id,
        device_id,
        since_sn,
        req_body,
    }: SyncInfo<'_>,
    todo_rooms: &mut TodoRooms,
    known_rooms: &KnownRooms,
) -> AppResult<()> {
    let mut known_subscription_rooms = BTreeSet::new();
    for (room_id, room) in &req_body.room_subscriptions {
        if !crate::room::room_exists(room_id)? {
            continue;
        }
        let todo_room = todo_rooms.entry(room_id.clone()).or_insert(TodoRoom::new(
            BTreeSet::new(),
            0_usize,
            i64::MAX,
        ));

        let limit = room.timeline_limit;

        todo_room.required_state.extend(
            room.required_state
                .iter()
                .map(|(ty, sk)| (ty.clone(), sk.as_str().into())),
        );
        todo_room.timeline_limit = todo_room.timeline_limit.max(limit as usize);
        todo_room.room_since_sn = todo_room.room_since_sn.min(
            known_rooms
                .get("subscriptions")
                .and_then(|k| k.get(room_id))
                .copied()
                .unwrap_or(since_sn),
        );
        known_subscription_rooms.insert(room_id.clone());
    }
    // where this went (protomsc says it was removed)
    // for r in req_body.unsubscribe_rooms {
    // 	known_subscription_rooms.remove(&r);
    // 	req_body.room_subscriptions.remove(&r);
    //}

    crate::sync_v5::update_sync_known_rooms(
        sender_id.to_owned(),
        device_id.to_owned(),
        req_body.conn_id.clone(),
        "subscriptions".to_owned(),
        known_subscription_rooms,
        since_sn,
    );
    Ok(())
}

async fn process_rooms(
    SyncInfo {
        sender_id,
        req_body,
        device_id,
        ..
    }: SyncInfo<'_>,
    all_invited_rooms: &[&RoomId],
    todo_rooms: &TodoRooms,
    known_rooms: &KnownRooms,
    response: &mut SyncEventsResBody,
) -> AppResult<BTreeMap<OwnedRoomId, sync_events::v5::SyncRoom>> {
    let mut rooms = BTreeMap::new();
    for (
        room_id,
        TodoRoom {
            required_state,
            timeline_limit,
            room_since_sn,
        },
    ) in todo_rooms
    {
        let mut timestamp: Option<_> = None;
        let mut invite_state = None;
        let new_room_id: &RoomId = (*room_id).as_ref();
        let timeline = if all_invited_rooms.contains(&new_room_id) {
            // TODO: figure out a timestamp we can use for remote invites
            invite_state = crate::room::user::invite_state(sender_id, room_id).ok();
            TimelineData {
                events: Default::default(),
                limited: false,
                prev_batch: None,
                next_batch: None,
            }
        } else {
            crate::sync_v3::load_timeline(
                sender_id,
                room_id,
                Some(BatchToken::new_live(*room_since_sn)),
                Some(BatchToken::LIVE_MAX),
                Some(&RoomEventFilter::with_limit(*timeline_limit)),
            )?
        };

        if req_body.extensions.account_data.enabled == Some(true) {
            response.extensions.account_data.rooms.insert(
                room_id.to_owned(),
                data::user::data_changes(Some(room_id), sender_id, *room_since_sn, None)?
                    .into_iter()
                    .filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Room))
                    .collect::<Vec<_>>(),
            );
        }

        let last_private_read_update =
            data::room::receipt::last_private_read_update_sn(sender_id, room_id)
                .unwrap_or_default()
                > *room_since_sn;

        let private_read_event = if last_private_read_update {
            crate::room::receipt::last_private_read(sender_id, room_id).ok()
        } else {
            None
        };

        let mut receipts = data::room::receipt::read_receipts(room_id, *room_since_sn)?
            .into_iter()
            .filter_map(|(read_user, content)| {
                if !crate::user::user_is_ignored(&read_user, sender_id) {
                    Some(content)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        if let Some(private_read_event) = private_read_event {
            receipts.push(private_read_event);
        }

        let receipt_size = receipts.len();

        if receipt_size > 0 {
            response.extensions.receipts.rooms.insert(
                room_id.clone(),
                SyncReceiptEvent {
                    content: combine_receipt_event_contents(receipts),
                },
            );
        }

        if room_since_sn != &0
            && timeline.events.is_empty()
            && invite_state.is_none()
            && receipt_size == 0
        {
            continue;
        }

        let prev_batch = timeline
            .events
            .first()
            .and_then(|(sn, _)| if *sn == 0 { None } else { Some(sn.to_string()) });

        let room_events: Vec<_> = timeline
            .events
            .iter()
            .filter(|item| ignored_filter(*item, sender_id))
            .map(|(_, pdu)| pdu.to_sync_room_event())
            .collect();

        for (_, pdu) in &timeline.events {
            let ts = pdu.origin_server_ts;
            if DEFAULT_BUMP_TYPES.binary_search(&pdu.event_ty).is_ok()
                && timestamp.is_none_or(|time| time <= ts)
            {
                timestamp = Some(ts);
            }
        }

        let required_state = required_state
            .iter()
            .filter_map(|state| {
                let state_key = match state.1.as_str() {
                    "$LAZY" | "*" => return None,
                    "$ME" => sender_id.as_str(),
                    _ => state.1.as_str(),
                };

                let pdu = room::get_state(room_id, &state.0, state_key, None);
                if let Ok(pdu) = &pdu {
                    if is_required_state_send(
                        sender_id.to_owned(),
                        device_id.to_owned(),
                        req_body.conn_id.clone(),
                        pdu.event_sn,
                    ) {
                        None
                    } else {
                        mark_required_state_sent(
                            sender_id.to_owned(),
                            device_id.to_owned(),
                            req_body.conn_id.clone(),
                            pdu.event_sn,
                        );
                        Some(pdu.to_sync_state_event())
                    }
                } else {
                    pdu.map(|s| s.to_sync_state_event()).ok()
                }
            })
            .collect::<Vec<_>>();

        // Heroes
        let heroes: Vec<_> = room::get_members(room_id)?
            .into_iter()
            .filter(|member| *member != sender_id)
            .filter_map(|user_id| {
                room::get_member(room_id, &user_id, None)
                    .ok()
                    .map(|member| sync_events::v5::SyncRoomHero {
                        user_id,
                        name: member.display_name,
                        avatar: member.avatar_url,
                    })
            })
            .take(5)
            .collect();

        let name = match heroes.len().cmp(&(1_usize)) {
            Ordering::Greater => {
                let firsts = heroes[1..]
                    .iter()
                    .map(|h: &SyncRoomHero| h.name.clone().unwrap_or_else(|| h.user_id.to_string()))
                    .collect::<Vec<_>>()
                    .join(", ");

                let last = heroes[0]
                    .name
                    .clone()
                    .unwrap_or_else(|| heroes[0].user_id.to_string());

                Some(format!("{firsts} and {last}"))
            }
            Ordering::Equal => Some(
                heroes[0]
                    .name
                    .clone()
                    .unwrap_or_else(|| heroes[0].user_id.to_string()),
            ),
            Ordering::Less => None,
        };

        let heroes_avatar = if heroes.len() == 1 {
            heroes[0].avatar.clone()
        } else {
            None
        };

        let notify_summary = room::user::notify_summary(sender_id, room_id)?;
        rooms.insert(
            room_id.clone(),
            SyncRoom {
                name: room::get_name(room_id).ok().or(name),
                avatar: match heroes_avatar {
                    Some(heroes_avatar) => Some(heroes_avatar),
                    _ => room::get_avatar_url(room_id).ok().flatten(),
                },
                initial: Some(
                    room_since_sn == &0
                        || !known_rooms
                            .values()
                            .any(|rooms| rooms.contains_key(room_id)),
                ),
                is_dm: None,
                invite_state,
                unread_notifications: sync_events::UnreadNotificationsCount {
                    notification_count: Some(notify_summary.all_notification_count()),
                    highlight_count: Some(notify_summary.all_highlight_count()),
                },
                timeline: room_events,
                required_state,
                prev_batch,
                limited: timeline.limited,
                joined_count: Some(
                    crate::room::joined_member_count(room_id)
                        .unwrap_or(0)
                        .try_into()
                        .unwrap_or(0),
                ),
                invited_count: Some(
                    crate::room::invited_member_count(room_id)
                        .unwrap_or(0)
                        .try_into()
                        .unwrap_or(0),
                ),
                num_live: None, // Count events in timeline greater than global sync counter
                bump_stamp: timestamp.map(|t| t.get() as i64),
                heroes: Some(heroes),
            },
        );
    }
    Ok(rooms)
}
fn collect_account_data(
    SyncInfo {
        sender_id,
        since_sn,
        req_body,
        ..
    }: SyncInfo<'_>,
) -> AppResult<sync_events::v5::AccountData> {
    let mut account_data = sync_events::v5::AccountData {
        global: Vec::new(),
        rooms: BTreeMap::new(),
    };

    if !req_body.extensions.account_data.enabled.unwrap_or(false) {
        return Ok(sync_events::v5::AccountData::default());
    }

    account_data.global = data::user::data_changes(None, sender_id, since_sn, None)?
        .into_iter()
        .filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Global))
        .collect();

    if let Some(rooms) = &req_body.extensions.account_data.rooms {
        for room in rooms {
            account_data.rooms.insert(
                room.clone(),
                data::user::data_changes(Some(room), sender_id, since_sn, None)?
                    .into_iter()
                    .filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Room))
                    .collect(),
            );
        }
    }

    Ok(account_data)
}

fn collect_e2ee(
    SyncInfo {
        sender_id,
        device_id,
        since_sn,
        req_body,
    }: SyncInfo<'_>,
    all_joined_rooms: &Vec<&RoomId>,
) -> AppResult<sync_events::v5::E2ee> {
    if !req_body.extensions.e2ee.enabled.unwrap_or(false) {
        return Ok(sync_events::v5::E2ee::default());
    }
    let mut left_encrypted_users = HashSet::new(); // Users that have left any encrypted rooms the sender was in
    let mut device_list_changes = HashSet::new();
    let mut device_list_left = HashSet::new();
    // Look for device list updates of this account
    device_list_changes.extend(data::user::keys_changed_users(sender_id, since_sn, None)?);

    for room_id in all_joined_rooms {
        let Ok(current_frame_id) = crate::room::get_frame_id(room_id, None) else {
            error!("Room {room_id} has no state");
            continue;
        };
        let since_frame_id = crate::event::get_last_frame_id(room_id, Some(since_sn)).ok();

        let encrypted_room =
            state::get_state(current_frame_id, &StateEventType::RoomEncryption, "").is_ok();

        if let Some(since_frame_id) = since_frame_id {
            // // Skip if there are only timeline changes
            // if since_frame_id == current_frame_id {
            //     continue;
            // }

            let since_encryption =
                state::get_state(since_frame_id, &StateEventType::RoomEncryption, "").ok();

            let joined_since_last_sync = room::user::join_sn(sender_id, room_id)? >= since_sn;

            let new_encrypted_room = encrypted_room && since_encryption.is_none();

            if encrypted_room {
                let current_state_ids = state::get_full_state_ids(current_frame_id)?;

                let since_state_ids = state::get_full_state_ids(since_frame_id)?;

                for (key, id) in current_state_ids {
                    if since_state_ids.get(&key) != Some(&id) {
                        let Ok(pdu) = timeline::get_pdu(&id) else {
                            error!("pdu in state not found: {id}");
                            continue;
                        };
                        if pdu.event_ty == TimelineEventType::RoomMember
                            && let Some(Ok(user_id)) = pdu.state_key.as_deref().map(UserId::parse)
                        {
                            if user_id == sender_id {
                                continue;
                            }

                            let content: RoomMemberEventContent = pdu.get_content()?;
                            match content.membership {
                                MembershipState::Join => {
                                    // A new user joined an encrypted room
                                    if !share_encrypted_room(sender_id, &user_id, Some(room_id))? {
                                        device_list_changes.insert(user_id.to_owned());
                                    }
                                }
                                MembershipState::Leave => {
                                    // Write down users that have left encrypted rooms we
                                    // are in
                                    left_encrypted_users.insert(user_id.to_owned());
                                }
                                _ => {}
                            }
                        }
                    }
                }
                if joined_since_last_sync || new_encrypted_room {
                    // If the user is in a new encrypted room, give them all joined users
                    device_list_changes.extend(
                        room::get_members(room_id)?
                            .into_iter()
                            // Don't send key updates from the sender to the sender
                            .filter(|user_id| sender_id != *user_id)
                            // Only send keys if the sender doesn't share an encrypted room with the target
                            // already
                            .filter_map(|user_id| {
                                if !share_encrypted_room(sender_id, &user_id, Some(room_id))
                                    .unwrap_or(false)
                                {
                                    Some(user_id.to_owned())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>(),
                    );
                }
            }
        }
        // Look for device list updates in this room
        device_list_changes.extend(crate::room::keys_changed_users(room_id, since_sn, None)?);
    }

    for user_id in left_encrypted_users {
        let Ok(share_encrypted_room) = share_encrypted_room(sender_id, &user_id, None) else {
            continue;
        };

        // If the user doesn't share an encrypted room with the target anymore, we need
        // to tell them
        if !share_encrypted_room {
            device_list_left.insert(user_id);
        }
    }

    Ok(E2ee {
        device_lists: DeviceLists {
            changed: device_list_changes.into_iter().collect(),
            left: device_list_left.into_iter().collect(),
        },
        device_one_time_keys_count: data::user::count_one_time_keys(sender_id, device_id)?,
        device_unused_fallback_key_types: None,
    })
}

fn collect_to_device(
    SyncInfo {
        sender_id,
        device_id,
        since_sn,
        req_body,
    }: SyncInfo<'_>,
    next_batch: Seqnum,
) -> Option<sync_events::v5::ToDevice> {
    if !req_body.extensions.to_device.enabled.unwrap_or(false) {
        return None;
    }

    data::user::device::remove_to_device_events(sender_id, device_id, since_sn - 1).ok()?;

    let events =
        data::user::device::get_to_device_events(sender_id, device_id, None, Some(next_batch))
            .ok()?;

    Some(sync_events::v5::ToDevice {
        next_batch: next_batch.to_string(),
        events,
    })
}

fn collect_receipts() -> sync_events::v5::Receipts {
    sync_events::v5::Receipts {
        rooms: BTreeMap::new(),
    }
    // TODO: get explicitly requested read receipts
}

async fn collect_typing<'a, Rooms>(
    SyncInfo { req_body, .. }: SyncInfo<'_>,
    _next_batch: Seqnum,
    rooms: Rooms,
) -> AppResult<sync_events::v5::Typing>
where
    Rooms: Iterator<Item = &'a RoomId> + Send + 'a,
{
    use sync_events::v5::Typing;

    if !req_body.extensions.typing.enabled.unwrap_or(false) {
        return Ok(Typing::default());
    }

    let mut typing = Typing::new();
    for room_id in rooms {
        typing.rooms.insert(
            room_id.to_owned(),
            room::typing::all_typings(room_id).await?,
        );
    }

    Ok(typing)
}

pub fn forget_sync_request_connection(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    conn_id: Option<String>,
) {
    CONNECTIONS
        .lock()
        .unwrap()
        .remove(&(user_id, device_id, conn_id));
}
/// load params from cache if body doesn't contain it, as long as it's allowed
/// in some cases we may need to allow an empty list as an actual value
fn list_or_sticky<T: Clone>(target: &mut Vec<T>, cached: &Vec<T>) {
    if target.is_empty() {
        target.clone_from(cached);
    }
}
fn some_or_sticky<T>(target: &mut Option<T>, cached: Option<T>) {
    if target.is_none() {
        *target = cached;
    }
}
pub fn update_sync_request_with_cache(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    req_body: &mut sync_events::v5::SyncEventsReqBody,
) -> BTreeMap<String, BTreeMap<OwnedRoomId, i64>> {
    let mut cache = CONNECTIONS.lock().unwrap();
    let cached = Arc::clone(
        cache
            .entry((user_id, device_id, req_body.conn_id.clone()))
            .or_default(),
    );
    let cached = &mut cached.lock().unwrap();
    drop(cache);

    for (list_id, list) in &mut req_body.lists {
        if let Some(cached_list) = cached.lists.get(list_id) {
            list_or_sticky(
                &mut list.room_details.required_state,
                &cached_list.room_details.required_state,
            );
            // some_or_sticky(&mut list.include_heroes, cached_list.include_heroes);

            match (&mut list.filters, cached_list.filters.clone()) {
                (Some(filters), Some(cached_filters)) => {
                    some_or_sticky(&mut filters.is_invite, cached_filters.is_invite);
                    // TODO (morguldir): Find out how a client can unset this, probably need
                    // to change into an option inside palpo
                    list_or_sticky(&mut filters.not_room_types, &cached_filters.not_room_types);
                }
                (_, Some(cached_filters)) => list.filters = Some(cached_filters),
                (Some(list_filters), _) => list.filters = Some(list_filters.clone()),
                (..) => {}
            }
        }
        cached.lists.insert(list_id.clone(), list.clone());
    }

    cached
        .subscriptions
        .extend(req_body.room_subscriptions.clone());
    req_body
        .room_subscriptions
        .extend(cached.subscriptions.clone());

    req_body.extensions.e2ee.enabled = req_body
        .extensions
        .e2ee
        .enabled
        .or(cached.extensions.e2ee.enabled);

    req_body.extensions.to_device.enabled = req_body
        .extensions
        .to_device
        .enabled
        .or(cached.extensions.to_device.enabled);

    req_body.extensions.account_data.enabled = req_body
        .extensions
        .account_data
        .enabled
        .or(cached.extensions.account_data.enabled);
    req_body.extensions.account_data.lists = req_body
        .extensions
        .account_data
        .lists
        .clone()
        .or(cached.extensions.account_data.lists.clone());
    req_body.extensions.account_data.rooms = req_body
        .extensions
        .account_data
        .rooms
        .clone()
        .or(cached.extensions.account_data.rooms.clone());

    some_or_sticky(
        &mut req_body.extensions.typing.enabled,
        cached.extensions.typing.enabled,
    );
    some_or_sticky(
        &mut req_body.extensions.typing.rooms,
        cached.extensions.typing.rooms.clone(),
    );
    some_or_sticky(
        &mut req_body.extensions.typing.lists,
        cached.extensions.typing.lists.clone(),
    );
    some_or_sticky(
        &mut req_body.extensions.receipts.enabled,
        cached.extensions.receipts.enabled,
    );
    some_or_sticky(
        &mut req_body.extensions.receipts.rooms,
        cached.extensions.receipts.rooms.clone(),
    );
    some_or_sticky(
        &mut req_body.extensions.receipts.lists,
        cached.extensions.receipts.lists.clone(),
    );

    cached.extensions = req_body.extensions.clone();
    cached.known_rooms.clone()
}

pub fn update_sync_subscriptions(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    conn_id: Option<String>,
    subscriptions: BTreeMap<OwnedRoomId, sync_events::v5::RoomSubscription>,
) {
    let mut cache = CONNECTIONS.lock().unwrap();
    let cached = Arc::clone(cache.entry((user_id, device_id, conn_id)).or_default());
    let cached = &mut cached.lock().unwrap();
    drop(cache);

    cached.subscriptions = subscriptions;
}

pub fn update_sync_known_rooms(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    conn_id: Option<String>,
    list_id: String,
    new_cached_rooms: BTreeSet<OwnedRoomId>,
    since_sn: i64,
) {
    let mut cache = CONNECTIONS.lock().unwrap();
    let cached = Arc::clone(cache.entry((user_id, device_id, conn_id)).or_default());
    let cached = &mut cached.lock().unwrap();
    drop(cache);

    for (roomid, last_since) in cached
        .known_rooms
        .entry(list_id.clone())
        .or_default()
        .iter_mut()
    {
        if !new_cached_rooms.contains(roomid) {
            *last_since = 0;
        }
    }
    let list = cached.known_rooms.entry(list_id).or_default();
    for room_id in new_cached_rooms {
        list.insert(room_id, since_sn);
    }
}

pub fn mark_required_state_sent(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    conn_id: Option<String>,
    event_sn: Seqnum,
) {
    let mut cache = CONNECTIONS.lock().unwrap();
    let cached = Arc::clone(cache.entry((user_id, device_id, conn_id)).or_default());
    let cached = &mut cached.lock().unwrap();
    drop(cache);
    cached.required_state.insert(event_sn);
}
pub fn is_required_state_send(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    conn_id: Option<String>,
    event_sn: Seqnum,
) -> bool {
    let cache = CONNECTIONS.lock().unwrap();
    let Some(cached) = cache.get(&(user_id, device_id, conn_id)) else {
        return false;
    };
    cached.lock().unwrap().required_state.contains(&event_sn)
}
