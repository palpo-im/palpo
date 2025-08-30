use std::cmp::{self, Ordering};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::Seqnum;
use crate::core::client::filter::RoomEventFilter;
use crate::core::client::sync_events::{self, v5::*};
use crate::core::device::DeviceLists;
use crate::core::events::receipt::{SyncReceiptEvent, combine_receipt_event_contents};
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{AnyRawAccountDataEvent, StateEventType, TimelineEventType};
use crate::core::identifiers::*;
use crate::data;
use crate::event::ignored_filter;
use crate::extract_variant;
use crate::room::filter_rooms;
use crate::room::{self, state, timeline};
use crate::routing::prelude::*;
use crate::sync_v3::{DEFAULT_BUMP_TYPES, share_encrypted_room};

/// `POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync`
/// ([MSC4186])
///
/// A simplified version of sliding sync ([MSC3575]).
///
/// Get all new events in a sliding window of rooms since the last sync or a
/// given point in time.
///
/// [MSC3575]: https://github.com/matrix-org/matrix-spec-proposals/pull/3575
/// [MSC4186]: https://github.com/matrix-org/matrix-spec-proposals/pull/4186
#[handler]
pub(super) async fn sync_events_v5(
    _aa: AuthArgs,
    args: SyncEventsReqArgs,
    body: JsonBody<SyncEventsReqBody>,
    depot: &mut Depot,
) -> JsonResult<SyncEventsResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let mut body = body.into_inner();
    // Setup watchers, so if there's no response, we can wait for them
    let watcher = crate::watcher::watch(sender_id, authed.device_id());

    let next_batch = data::curr_sn()? + 1;

    let _conn_id = body.conn_id.clone();

    let global_since_sn: i64 = args
        .pos
        .as_ref()
        .and_then(|string| string.parse().ok())
        .unwrap_or_default();

    if global_since_sn == 0
        && let Some(conn_id) = &body.conn_id
    {
        crate::sync_v5::forget_sync_request_connection(
            sender_id.to_owned(),
            authed.device_id().to_owned(),
            conn_id.to_owned(),
        )
    }

    // Get sticky parameters from cache
    let known_rooms = crate::sync_v5::update_sync_request_with_cache(
        sender_id.to_owned(),
        authed.device_id().to_owned(),
        &mut body,
    );

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

    let pos = next_batch.clone().to_string();

    let mut todo_rooms: TodoRooms = BTreeMap::new();

    let sync_info: SyncInfo<'_> = (sender_id, authed.device_id(), global_since_sn, &body);
    let mut res_body = SyncEventsResBody {
        txn_id: body.txn_id.clone(),
        pos,
        lists: BTreeMap::new(),
        rooms: BTreeMap::new(),
        extensions: Extensions {
            account_data: collect_account_data(sync_info)?,
            e2ee: collect_e2ee(sync_info, &all_joined_rooms)?,
            to_device: collect_to_device(sync_info, next_batch),
            receipts: collect_receipts(),
            typing: Typing::default(),
        },
    };

    process_lists(
        sync_info,
        &all_invited_rooms,
        &all_joined_rooms,
        &all_rooms,
        &mut todo_rooms,
        &known_rooms,
        &mut res_body,
    )
    .await;

    fetch_subscriptions(sync_info, &known_rooms, &mut todo_rooms)?;

    res_body.rooms = process_rooms(
        authed.user_id(),
        next_batch,
        &all_invited_rooms,
        &todo_rooms,
        &mut res_body,
        &body,
    )
    .await?;

    if res_body.rooms.iter().all(|(id, r)| {
        r.timeline.is_empty()
            && r.required_state.is_empty()
            && !res_body.extensions.receipts.rooms.contains_key(id)
    }) && res_body
        .extensions
        .to_device
        .clone()
        .is_none_or(|to| to.events.is_empty())
    {
        // Hang a few seconds so requests are not spammed
        // Stop hanging if new info arrives
        let default = Duration::from_secs(30);
        let duration = cmp::min(args.timeout.unwrap_or(default), default);
        _ = tokio::time::timeout(duration, watcher).await;
    }

    trace!(
        rooms=?res_body.rooms.len(),
        account_data=?res_body.extensions.account_data.rooms.len(),
        receipts=?res_body.extensions.receipts.rooms.len(),
        "responding to request with"
    );
    json_ok(res_body)
}

#[allow(clippy::too_many_arguments)]
async fn process_lists<'a>(
    (sender_id, sender_device, global_since_sn, body): SyncInfo<'_>,
    all_invited_rooms: &Vec<&'a RoomId>,
    all_joined_rooms: &Vec<&'a RoomId>,
    all_rooms: &Vec<&'a RoomId>,
    todo_rooms: &'a mut TodoRooms,
    known_rooms: &'a KnownRooms,
    res_body: &'_ mut SyncEventsResBody,
) -> KnownRooms {
    for (list_id, list) in &body.lists {
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

        let ranges = list.ranges.clone();

        for mut range in ranges {
            range.0 = 0;
            range.1 = range.1.clamp(range.0, active_rooms.len());

            let room_ids = active_rooms[range.0..range.1].to_vec();

            let new_rooms: BTreeSet<OwnedRoomId> =
                room_ids.clone().into_iter().map(From::from).collect();
            new_known_rooms.extend(new_rooms);
            //new_known_rooms.extend(room_ids..cloned());
            for room_id in room_ids {
                let todo_room = todo_rooms.entry(room_id.to_owned()).or_insert((
                    BTreeSet::new(),
                    0_usize,
                    i64::MAX,
                ));

                let limit = list.room_details.timeline_limit.min(100);

                todo_room.0.extend(
                    list.room_details
                        .required_state
                        .iter()
                        .map(|(ty, sk)| (ty.clone(), sk.as_str().into())),
                );

                todo_room.1 = todo_room.1.max(limit);
                // 0 means unknown because it got out of date
                todo_room.2 = todo_room.2.min(
                    known_rooms
                        .get(list_id.as_str())
                        .and_then(|k| k.get(room_id))
                        .copied()
                        .unwrap_or(0),
                );
            }
        }
        res_body.lists.insert(
            list_id.clone(),
            sync_events::v5::SyncList {
                count: active_rooms.len(),
            },
        );

        if let Some(conn_id) = &body.conn_id {
            crate::sync_v5::update_sync_known_rooms(
                sender_id.to_owned(),
                sender_device.to_owned(),
                conn_id.clone(),
                list_id.clone(),
                new_known_rooms,
                global_since_sn,
            );
        }
    }
    BTreeMap::default()
}

fn fetch_subscriptions(
    (sender_user, sender_device, global_since_sn, body): SyncInfo<'_>,
    known_rooms: &KnownRooms,
    todo_rooms: &mut TodoRooms,
) -> AppResult<()> {
    let mut known_subscription_rooms = BTreeSet::new();
    for (room_id, room) in &body.room_subscriptions {
        if !crate::room::room_exists(room_id)? {
            continue;
        }
        let todo_room =
            todo_rooms
                .entry(room_id.clone())
                .or_insert((BTreeSet::new(), 0_usize, i64::MAX));

        let limit = room.timeline_limit;

        todo_room.0.extend(
            room.required_state
                .iter()
                .map(|(ty, sk)| (ty.clone(), sk.as_str().into())),
        );
        todo_room.1 = todo_room.1.max(limit as usize);
        // 0 means unknown because it got out of date
        todo_room.2 = todo_room.2.min(
            known_rooms
                .get("subscriptions")
                .and_then(|k| k.get(room_id))
                .copied()
                .unwrap_or(0),
        );
        known_subscription_rooms.insert(room_id.clone());
    }
    // where this went (protomsc says it was removed)
    //for r in body.unsubscribe_rooms {
    //	known_subscription_rooms.remove(&r);
    //	body.room_subscriptions.remove(&r);
    //}

    if let Some(conn_id) = &body.conn_id {
        crate::sync_v5::update_sync_known_rooms(
            sender_user.to_owned(),
            sender_device.to_owned(),
            conn_id.clone(),
            "subscriptions".to_owned(),
            known_subscription_rooms,
            global_since_sn,
        );
    }
    Ok(())
}

async fn process_rooms(
    sender_id: &UserId,
    next_batch: Seqnum,
    all_invited_rooms: &[&RoomId],
    todo_rooms: &TodoRooms,
    response: &mut SyncEventsResBody,
    body: &SyncEventsReqBody,
) -> AppResult<BTreeMap<OwnedRoomId, sync_events::v5::SyncRoom>> {
    let mut rooms = BTreeMap::new();
    for (room_id, (required_state_request, timeline_limit, room_since_sn)) in todo_rooms {
        let mut timestamp: Option<_> = None;
        let mut invite_state = None;
        let new_room_id: &RoomId = (*room_id).as_ref();
        let (timeline_pdus, limited) = if all_invited_rooms.contains(&new_room_id) {
            // TODO: figure out a timestamp we can use for remote invites
            invite_state = crate::room::user::invite_state(sender_id, room_id).ok();

            (Vec::new(), true)
        } else {
            crate::sync_v3::load_timeline(
                sender_id,
                room_id,
                Some(*room_since_sn),
                Some(Seqnum::MAX),
                Some(&RoomEventFilter::with_limit(*timeline_limit)),
            )?
        };

        if body.extensions.account_data.enabled == Some(true) {
            response.extensions.account_data.rooms.insert(
                room_id.to_owned(),
                data::user::data_changes(
                    Some(room_id),
                    sender_id,
                    *room_since_sn,
                    Some(next_batch),
                )?
                .into_iter()
                .filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Room))
                .collect::<Vec<_>>(),
            );
        }

        let last_private_read_update =
            data::room::receipt::last_private_read_update_sn(sender_id, room_id)? > *room_since_sn;

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
            && timeline_pdus.is_empty()
            && response
                .extensions
                .account_data
                .rooms
                .get(room_id)
                .is_none_or(Vec::is_empty)
            && receipt_size == 0
        {
            continue;
        }

        let prev_batch = timeline_pdus
            .first()
            .and_then(|(sn, _)| if *sn == 0 { None } else { Some(sn.to_string()) });

        let room_events: Vec<_> = timeline_pdus
            .iter()
            .filter_map(|item| ignored_filter(item.clone(), sender_id))
            .map(|(_, pdu)| pdu.to_sync_room_event())
            .collect();

        for (_, pdu) in timeline_pdus {
            let ts = pdu.origin_server_ts;
            if DEFAULT_BUMP_TYPES.binary_search(&pdu.event_ty).is_ok()
                && timestamp.is_none_or(|time| time <= ts)
            {
                timestamp = Some(ts);
            }
        }

        let required_state = required_state_request
            .iter()
            .filter_map(|state| {
                room::get_state(room_id, &state.0, &state.1, None)
                    .map(|s| s.to_sync_state_event())
                    .ok()
            })
            .collect::<Vec<_>>();

        // Heroes
        let heroes: Vec<_> = room::get_members(room_id)?
            .into_iter()
            .filter(|member| *member != sender_id)
            .filter_map(|user_id| {
                room::get_member(room_id, &user_id).ok().map(|member| {
                    sync_events::v5::SyncRoomHero {
                        user_id,
                        name: member.display_name,
                        avatar: member.avatar_url,
                    }
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
                initial: Some(room_since_sn == &0),
                is_dm: None,
                invite_state,
                unread_notifications: sync_events::UnreadNotificationsCount {
                    notification_count: Some(notify_summary.all_notification_count()),
                    highlight_count: Some(notify_summary.all_highlight_count()),
                },
                timeline: room_events,
                required_state,
                prev_batch,
                limited,
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
    (sender_id, _, global_since_sn, body): (&UserId, &DeviceId, Seqnum, &SyncEventsReqBody),
) -> AppResult<sync_events::v5::AccountData> {
    let mut account_data = sync_events::v5::AccountData {
        global: Vec::new(),
        rooms: BTreeMap::new(),
    };

    if !body.extensions.account_data.enabled.unwrap_or(false) {
        return Ok(sync_events::v5::AccountData::default());
    }

    account_data.global = data::user::data_changes(None, sender_id, global_since_sn, None)?
        .into_iter()
        .filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Global))
        .collect();

    if let Some(rooms) = &body.extensions.account_data.rooms {
        for room in rooms {
            account_data.rooms.insert(
                room.clone(),
                data::user::data_changes(Some(room), sender_id, global_since_sn, None)?
                    .into_iter()
                    .filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Room))
                    .collect(),
            );
        }
    }

    Ok(account_data)
}

fn collect_e2ee<'a>(
    (sender_id, sender_device, global_since_sn, body): (
        &UserId,
        &DeviceId,
        Seqnum,
        &SyncEventsReqBody,
    ),
    all_joined_rooms: &'a Vec<&'a RoomId>,
) -> AppResult<sync_events::v5::E2ee> {
    if !body.extensions.e2ee.enabled.unwrap_or(false) {
        return Ok(sync_events::v5::E2ee::default());
    }
    let mut left_encrypted_users = HashSet::new(); // Users that have left any encrypted rooms the sender was in
    let mut device_list_changes = HashSet::new();
    let mut device_list_left = HashSet::new();
    // Look for device list updates of this account
    device_list_changes.extend(data::user::keys_changed_users(
        sender_id,
        global_since_sn,
        None,
    )?);

    for room_id in all_joined_rooms {
        let Ok(current_frame_id) = crate::room::get_frame_id(room_id, None) else {
            error!("Room {room_id} has no state");
            continue;
        };

        let since_frame_id = crate::event::get_frame_id(room_id, global_since_sn).ok();

        let encrypted_room =
            state::get_state(current_frame_id, &StateEventType::RoomEncryption, "").is_ok();

        if let Some(since_frame_id) = since_frame_id {
            // Skip if there are only timeline changes
            if since_frame_id == current_frame_id {
                continue;
            }

            let since_encryption =
                state::get_state(since_frame_id, &StateEventType::RoomEncryption, "").ok();

            let since_sender_member = state::get_state_content::<RoomMemberEventContent>(
                since_frame_id,
                &StateEventType::RoomMember,
                sender_id.as_str(),
            )
            .ok();

            let joined_since_last_sync = since_sender_member
                .as_ref()
                .is_none_or(|member| member.membership != MembershipState::Join);

            let new_encrypted_room = encrypted_room && since_encryption.is_none();

            if encrypted_room {
                let current_state_ids = state::get_full_state_ids(current_frame_id)?;

                let since_state_ids: HashMap<_, _> = state::get_full_state_ids(since_frame_id)?;

                for (key, id) in current_state_ids {
                    if since_state_ids.get(&key) != Some(&id) {
                        let Ok(pdu) = timeline::get_pdu(&id) else {
                            error!("Pdu in state not found: {id}");
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
                                if let Ok(true) =
                                    share_encrypted_room(sender_id, &user_id, Some(room_id))
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
        device_list_changes.extend(crate::room::keys_changed_users(
            room_id,
            global_since_sn,
            None,
        )?);
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
        device_one_time_keys_count: data::user::count_one_time_keys(sender_id, sender_device)?,
        device_unused_fallback_key_types: None,
    })
}

fn collect_to_device(
    (sender_id, sender_device, global_since_sn, body): SyncInfo<'_>,
    next_batch: Seqnum,
) -> Option<sync_events::v5::ToDevice> {
    if !body.extensions.to_device.enabled.unwrap_or(false) {
        return None;
    }

    data::user::device::remove_to_device_events(sender_id, sender_device, global_since_sn).ok()?;

    Some(sync_events::v5::ToDevice {
        next_batch: next_batch.to_string(),
        events: data::user::device::get_to_device_events(
            sender_id,
            sender_device,
            None,
            Some(next_batch),
        )
        .ok()?,
    })
}

fn collect_receipts() -> sync_events::v5::Receipts {
    sync_events::v5::Receipts {
        rooms: BTreeMap::new(),
    }
    // TODO: get explicitly requested read receipts
}
