use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::Seqnum;
use crate::core::client::filter::RoomEventFilter;
use crate::core::client::sync_events::v5::*;
use crate::core::client::sync_events::{self};
use crate::core::device::DeviceLists;
use crate::core::events::receipt::{SyncReceiptEvent, combine_receipt_event_contents};
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::direct::DirectEventContent;
use crate::core::events::{AnyRawAccountDataEvent, GlobalAccountDataEventType, StateEventType, TimelineEventType};
use crate::core::identifiers::*;
use crate::core::UnixMillis;
use crate::data::connect;
use crate::data::schema::*;
use crate::event::{BatchToken, ignored_filter};
use crate::room::{self, filter_rooms, state, timeline};
use crate::sync_v3::{DEFAULT_BUMP_TYPES, TimelineData, share_encrypted_room};
use crate::{AppResult, data, extract_variant};

/// Sort rooms by last activity (most recent first) using event sequence numbers.
fn sort_rooms_by_activity(rooms: &mut [&RoomId]) -> AppResult<()> {
    if rooms.is_empty() {
        return Ok(());
    }

    let room_strs: Vec<&str> = rooms.iter().map(|r| r.as_str()).collect();

    let results: Vec<(String, i64)> = event_points::table
        .filter(event_points::room_id.eq_any(&room_strs))
        .distinct_on(event_points::room_id)
        .order_by((event_points::room_id.asc(), event_points::event_sn.desc()))
        .select((event_points::room_id, event_points::event_sn))
        .load(&mut connect()?)?;

    let last_sn: BTreeMap<&str, i64> = results.iter().map(|(r, sn)| (r.as_str(), *sn)).collect();

    rooms.sort_by(|a, b| {
        let sn_a = last_sn.get(a.as_str()).copied().unwrap_or(0);
        let sn_b = last_sn.get(b.as_str()).copied().unwrap_or(0);
        sn_b.cmp(&sn_a)
    });

    Ok(())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SlidingSyncCache {
    lists: BTreeMap<String, sync_events::v5::ReqList>,
    subscriptions: BTreeMap<OwnedRoomId, sync_events::v5::RoomSubscription>,
    known_rooms: KnownRooms, // For every room, the room_since_sn number
    extensions: sync_events::v5::ExtensionsConfig,
    required_state: BTreeSet<Seqnum>,
}

/// In-memory cache backed by database for cross-instance persistence.
/// The local cache provides fast access; the database ensures failover.
static CONNECTIONS: LazyLock<
    Mutex<BTreeMap<(OwnedUserId, OwnedDeviceId, Option<String>), Arc<Mutex<SlidingSyncCache>>>>,
> = LazyLock::new(Default::default);

/// Load a connection cache from the database if not present in memory.
fn load_or_create_connection(
    user_id: &OwnedUserId,
    device_id: &OwnedDeviceId,
    conn_id: &Option<String>,
) -> Arc<Mutex<SlidingSyncCache>> {
    let mut cache = CONNECTIONS.lock().unwrap();
    let key = (user_id.clone(), device_id.clone(), conn_id.clone());
    if let Some(entry) = cache.get(&key) {
        return Arc::clone(entry);
    }

    // Try to load from database
    let conn_id_str = conn_id.as_deref().unwrap_or("");
    let db_cache = connect()
        .ok()
        .and_then(|mut conn| {
            sliding_sync_connections::table
                .filter(sliding_sync_connections::user_id.eq(user_id.as_str()))
                .filter(sliding_sync_connections::device_id.eq(device_id.as_str()))
                .filter(sliding_sync_connections::conn_id.eq(conn_id_str))
                .select(sliding_sync_connections::cache_data)
                .first::<serde_json::Value>(&mut conn)
                .ok()
        })
        .and_then(|json| serde_json::from_value::<SlidingSyncCache>(json).ok())
        .unwrap_or_default();

    let entry = Arc::new(Mutex::new(db_cache));
    cache.insert(key, Arc::clone(&entry));
    entry
}

/// Persist the connection cache to the database for cross-instance access.
fn persist_connection(
    user_id: &OwnedUserId,
    device_id: &OwnedDeviceId,
    conn_id: &Option<String>,
    cached: &SlidingSyncCache,
) {
    let conn_id_str = conn_id.as_deref().unwrap_or("");
    let Ok(cache_data) = serde_json::to_value(cached) else {
        return;
    };
    let now = UnixMillis::now();
    if let Ok(mut conn) = connect() {
        let _ = diesel::insert_into(sliding_sync_connections::table)
            .values((
                sliding_sync_connections::user_id.eq(user_id.as_str()),
                sliding_sync_connections::device_id.eq(device_id.as_str()),
                sliding_sync_connections::conn_id.eq(conn_id_str),
                sliding_sync_connections::cache_data.eq(&cache_data),
                sliding_sync_connections::updated_at.eq(now),
            ))
            .on_conflict((
                sliding_sync_connections::user_id,
                sliding_sync_connections::device_id,
                sliding_sync_connections::conn_id,
            ))
            .do_update()
            .set((
                sliding_sync_connections::cache_data.eq(&cache_data),
                sliding_sync_connections::updated_at.eq(now),
            ))
            .execute(&mut conn);
    }
}

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

    // Load DM room set from m.direct account data
    let dm_rooms: HashSet<OwnedRoomId> = data::user::get_data::<DirectEventContent>(
        sender_id,
        None,
        &GlobalAccountDataEventType::Direct.to_string(),
    )
    .map(|direct| direct.0.values().flatten().cloned().collect())
    .unwrap_or_default();

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
        &dm_rooms,
        &mut todo_rooms,
        known_rooms,
        &mut res_body,
    )
    .await?;

    fetch_subscriptions(sync_info, &mut todo_rooms, known_rooms)?;

    res_body.rooms = process_rooms(
        sync_info,
        &all_invited_rooms,
        &dm_rooms,
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
    dm_rooms: &HashSet<OwnedRoomId>,
    todo_rooms: &mut TodoRooms,
    known_rooms: &KnownRooms,
    res_body: &mut SyncEventsResBody,
) -> AppResult<()> {
    for (list_id, list) in &req_body.lists {
        let mut active_rooms: Vec<&RoomId> = match list.filters.as_ref().and_then(|f| f.is_invite)
        {
            Some(true) => all_invited_rooms.to_vec(),
            Some(false) => all_joined_rooms.to_vec(),
            None => all_rooms.to_vec(),
        };

        // Apply not_room_types filter
        if let Some(filter) = list.filters.as_ref().map(|f| &f.not_room_types) {
            if !filter.is_empty() {
                active_rooms = filter_rooms(&active_rooms, filter, true);
            }
        }

        // Apply room_types filter
        if let Some(filter) = list.filters.as_ref().and_then(|f| {
            if f.room_types.is_empty() {
                None
            } else {
                Some(&f.room_types)
            }
        }) {
            active_rooms = filter_rooms(&active_rooms, filter, false);
        }

        // Apply is_dm filter
        match list.filters.as_ref().and_then(|f| f.is_dm) {
            Some(true) => active_rooms.retain(|r| dm_rooms.contains(*r)),
            Some(false) => active_rooms.retain(|r| !dm_rooms.contains(*r)),
            None => {}
        }

        // Apply is_encrypted filter
        match list.filters.as_ref().and_then(|f| f.is_encrypted) {
            Some(true) => active_rooms.retain(|r| room::is_encrypted(r)),
            Some(false) => active_rooms.retain(|r| !room::is_encrypted(r)),
            None => {}
        }

        // Sort rooms by last activity (most recent first)
        let mut sorted_rooms = active_rooms;
        sort_rooms_by_activity(&mut sorted_rooms)?;
        let count = sorted_rooms.len();

        let mut new_known_rooms: BTreeSet<OwnedRoomId> = BTreeSet::new();
        let mut ops = Vec::new();
        let ranges = list.ranges.clone();

        for range in &ranges {
            // Ranges are inclusive [start, end] per MSC4186
            let start = range.0.min(count);
            let end = (range.1 + 1).min(count); // convert to exclusive for slicing

            if start >= end {
                continue;
            }

            let room_ids: Vec<&RoomId> = sorted_rooms[start..end].to_vec();
            let owned_room_ids: Vec<OwnedRoomId> =
                room_ids.iter().map(|r| (*r).to_owned()).collect();

            ops.push(sync_events::v5::SyncListOp::Sync {
                range: (start, end - 1), // back to inclusive for response
                room_ids: owned_room_ids.clone(),
            });

            new_known_rooms.extend(owned_room_ids);

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
            sync_events::v5::SyncList { count, ops },
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
    Ok(())
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
        since_sn,
    }: SyncInfo<'_>,
    all_invited_rooms: &[&RoomId],
    dm_rooms: &HashSet<OwnedRoomId>,
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

        let mut required_state_events = Vec::new();
        let mut lazy_senders_collected = false;

        for rs in required_state.iter() {
            match (rs.0.clone(), rs.1.as_str()) {
                // $LAZY: return member state for timeline event senders
                (ref event_type, "$LAZY") if *event_type == StateEventType::RoomMember => {
                    if lazy_senders_collected {
                        continue;
                    }
                    lazy_senders_collected = true;

                    let senders: HashSet<&UserId> = timeline
                        .events
                        .iter()
                        .map(|(_, pdu)| pdu.sender.as_ref())
                        .collect();

                    for sender in senders.into_iter().take(100) {
                        if let Ok(pdu) =
                            room::get_state(room_id, &StateEventType::RoomMember, sender.as_str(), None)
                        {
                            if !is_required_state_send(
                                sender_id.to_owned(),
                                device_id.to_owned(),
                                req_body.conn_id.clone(),
                                pdu.event_sn,
                            ) {
                                mark_required_state_sent(
                                    sender_id.to_owned(),
                                    device_id.to_owned(),
                                    req_body.conn_id.clone(),
                                    pdu.event_sn,
                                );
                                required_state_events.push(pdu.to_sync_state_event());
                            }
                        }
                    }
                }
                // * state key: return all state events of this type
                (ref event_type, "*") => {
                    if let Ok(frame_id) = room::get_frame_id(room_id, None) {
                        if let Ok(full_state) = state::get_full_state(frame_id) {
                            for ((ty, _sk), pdu) in &full_state {
                                if ty == event_type
                                    && !is_required_state_send(
                                        sender_id.to_owned(),
                                        device_id.to_owned(),
                                        req_body.conn_id.clone(),
                                        pdu.event_sn,
                                    )
                                {
                                    mark_required_state_sent(
                                        sender_id.to_owned(),
                                        device_id.to_owned(),
                                        req_body.conn_id.clone(),
                                        pdu.event_sn,
                                    );
                                    required_state_events.push(pdu.to_sync_state_event());
                                }
                            }
                        }
                    }
                }
                // $ME: substitute with the requester's user_id
                (ref event_type, "$ME") => {
                    if let Ok(pdu) = room::get_state(room_id, event_type, sender_id.as_str(), None)
                    {
                        if !is_required_state_send(
                            sender_id.to_owned(),
                            device_id.to_owned(),
                            req_body.conn_id.clone(),
                            pdu.event_sn,
                        ) {
                            mark_required_state_sent(
                                sender_id.to_owned(),
                                device_id.to_owned(),
                                req_body.conn_id.clone(),
                                pdu.event_sn,
                            );
                            required_state_events.push(pdu.to_sync_state_event());
                        }
                    }
                }
                // Specific state key
                (ref event_type, state_key) => {
                    if let Ok(pdu) = room::get_state(room_id, event_type, state_key, None) {
                        if !is_required_state_send(
                            sender_id.to_owned(),
                            device_id.to_owned(),
                            req_body.conn_id.clone(),
                            pdu.event_sn,
                        ) {
                            mark_required_state_sent(
                                sender_id.to_owned(),
                                device_id.to_owned(),
                                req_body.conn_id.clone(),
                                pdu.event_sn,
                            );
                            required_state_events.push(pdu.to_sync_state_event());
                        }
                    }
                }
            }
        }

        let required_state = required_state_events;

        // Heroes - limit query to 6 members (5 heroes + sender) to avoid loading all members
        let heroes: Vec<_> = room::get_members_limit(room_id, 6)?
            .into_iter()
            .filter(|member| *member != sender_id)
            .take(5)
            .filter_map(|user_id| {
                room::get_member(room_id, &user_id, None)
                    .ok()
                    .map(|member| sync_events::v5::SyncRoomHero {
                        user_id,
                        name: member.display_name,
                        avatar: member.avatar_url,
                    })
            })
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
                is_dm: Some(dm_rooms.contains(room_id)),
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
                num_live: Some(timeline.events.iter().filter(|(sn, _)| **sn > since_sn).count() as i64),
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
        .remove(&(user_id.clone(), device_id.clone(), conn_id.clone()));

    // Also remove from database
    let conn_id_str = conn_id.as_deref().unwrap_or("");
    if let Ok(mut db_conn) = connect() {
        let _ = diesel::delete(
            sliding_sync_connections::table
                .filter(sliding_sync_connections::user_id.eq(user_id.as_str()))
                .filter(sliding_sync_connections::device_id.eq(device_id.as_str()))
                .filter(sliding_sync_connections::conn_id.eq(conn_id_str)),
        )
        .execute(&mut db_conn);
    }
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
    let cached = load_or_create_connection(&user_id, &device_id, &req_body.conn_id);
    let cached = &mut cached.lock().unwrap();

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
                    some_or_sticky(&mut filters.is_dm, cached_filters.is_dm);
                    some_or_sticky(&mut filters.is_encrypted, cached_filters.is_encrypted);
                    list_or_sticky(&mut filters.room_types, &cached_filters.room_types);
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
    let known = cached.known_rooms.clone();
    // Persist to DB for cross-instance availability
    persist_connection(&user_id, &device_id, &req_body.conn_id, cached);
    known
}

pub fn update_sync_subscriptions(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    conn_id: Option<String>,
    subscriptions: BTreeMap<OwnedRoomId, sync_events::v5::RoomSubscription>,
) {
    let entry = load_or_create_connection(&user_id, &device_id, &conn_id);
    let cached = &mut entry.lock().unwrap();
    cached.subscriptions = subscriptions;
    persist_connection(&user_id, &device_id, &conn_id, cached);
}

pub fn update_sync_known_rooms(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    conn_id: Option<String>,
    list_id: String,
    new_cached_rooms: BTreeSet<OwnedRoomId>,
    since_sn: i64,
) {
    let entry = load_or_create_connection(&user_id, &device_id, &conn_id);
    let cached = &mut entry.lock().unwrap();

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
    persist_connection(&user_id, &device_id, &conn_id, cached);
}

pub fn mark_required_state_sent(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    conn_id: Option<String>,
    event_sn: Seqnum,
) {
    let entry = load_or_create_connection(&user_id, &device_id, &conn_id);
    let cached = &mut entry.lock().unwrap();
    cached.required_state.insert(event_sn);
    persist_connection(&user_id, &device_id, &conn_id, cached);
}
pub fn is_required_state_send(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    conn_id: Option<String>,
    event_sn: Seqnum,
) -> bool {
    let entry = load_or_create_connection(&user_id, &device_id, &conn_id);
    let cached = entry.lock().unwrap();
    cached.required_state.contains(&event_sn)
}
