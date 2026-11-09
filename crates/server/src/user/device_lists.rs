//! Device tracking at immutable sync publication boundaries.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::device::DeviceLists;
use crate::core::events::StateEventType;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::{OwnedEventId, OwnedRoomId, OwnedUserId, Seqnum, UserId};
use crate::data::connect;
use crate::data::schema::*;
use crate::{AppResult, MatrixError, room};

#[cfg(test)]
mod regression;
#[cfg(test)]
pub(crate) use regression::assert_large_custom_state_is_not_resolved;

#[derive(Default)]
struct Sharing {
    all: BTreeSet<OwnedUserId>,
    encrypted: BTreeSet<OwnedUserId>,
    joined_rooms: BTreeMap<OwnedRoomId, Seqnum>,
    encrypted_rooms: BTreeSet<OwnedRoomId>,
    room_members: BTreeMap<OwnedRoomId, Arc<BTreeMap<OwnedUserId, MemberPoint>>>,
}

struct MemberPoint {
    event_id: OwnedEventId,
    event_sn: Seqnum,
}

struct FrameSharing {
    joined: Arc<BTreeMap<OwnedUserId, MemberPoint>>,
    encrypted: bool,
    caller_join_period: Option<Seqnum>,
}

async fn joined_period(
    user: &UserId,
    point: &MemberPoint,
    periods: &mut HashMap<OwnedEventId, Seqnum>,
) -> AppResult<Seqnum> {
    if let Some(period) = periods.get(&point.event_id) {
        return Ok(*period);
    }
    let period =
        room::user::join_sn_for_event(user, point.event_id.clone(), point.event_sn).await?;
    periods.insert(point.event_id.clone(), period);
    Ok(period)
}

async fn sharing_before(
    user: &UserId,
    boundary: Seqnum,
    frames: &mut HashMap<i64, FrameSharing>,
    join_periods: &mut HashMap<OwnedEventId, Seqnum>,
) -> AppResult<Sharing> {
    let mut sharing = Sharing::default();
    // Membership rows are retained on leave and forget. They locate candidate
    // rooms only: authorization comes from the immutable published frame.
    let rooms = room_users::table
        .filter(room_users::user_id.eq(user))
        .select(room_users::room_id)
        .distinct()
        .load::<crate::core::OwnedRoomId>(&mut connect().await?)
        .await?;
    for room_id in rooms {
        let frame = event_points::table
            .filter(event_points::room_id.eq(&room_id))
            .filter(event_points::live_published_sn.lt(boundary))
            .order(event_points::live_published_sn.desc())
            .select(event_points::live_published_frame_id)
            .first::<Option<i64>>(&mut connect().await?)
            .await
            .optional()?;
        let Some(frame) = frame else { continue };
        let frame = frame.ok_or_else(|| MatrixError::unknown("published state frame missing"))?;
        if let std::collections::hash_map::Entry::Vacant(entry) = frames.entry(frame) {
            // Decode field/event sequence numbers without resolving event IDs.
            // get_full_state_ids calls CompressedEvent::split for every entry,
            // which adds one event_points lookup per unrelated custom event.
            let layers = room::state::load_frame_info(frame).await?;
            let layer = layers
                .last()
                .ok_or_else(|| MatrixError::unknown("published state frame missing"))?;
            let state: HashMap<_, _> = layer
                .full_state
                .iter()
                .map(|event| (event.field_id(), event.event_sn()))
                .collect();
            let field_ids: Vec<_> = state.keys().copied().collect();
            let mut joined = BTreeMap::new();
            let mut encrypted = false;
            let mut caller_join_period = None;
            // Restrict field metadata before loading event bodies: unrelated
            // custom state never adds PDU reads. Batch bodies and reuse a
            // frame shared by both endpoints within this request.
            for ids in field_ids.chunks(1_000) {
                let fields = room_state_fields::table
                    .filter(room_state_fields::id.eq_any(ids))
                    .filter(
                        room_state_fields::event_ty.eq_any(["m.room.member", "m.room.encryption"]),
                    )
                    .load::<crate::data::room::DbRoomStateField>(&mut connect().await?)
                    .await?;
                let members: HashMap<_, _> = fields
                    .into_iter()
                    .filter_map(|field| {
                        if field.event_ty == StateEventType::RoomEncryption {
                            encrypted |= field.state_key.is_empty();
                            None
                        } else {
                            Some((state[&field.id], field.state_key))
                        }
                    })
                    .collect();
                if members.is_empty() {
                    continue;
                }
                // event_points has a (room_id, event_sn) index; event_datas
                // has an event_id primary key, but no event_sn-only index.
                // Resolve only selected memberships in one indexed batch join.
                let bodies = event_points::table
                    .inner_join(
                        event_datas::table.on(event_datas::event_id
                            .eq(event_points::event_id)
                            .and(event_datas::event_sn.eq(event_points::event_sn))),
                    )
                    .filter(event_points::room_id.eq(&room_id))
                    .filter(event_points::event_sn.eq_any(members.keys()))
                    .filter(event_datas::room_id.eq(&room_id))
                    .select((
                        event_points::event_sn,
                        event_points::event_id,
                        event_datas::json_data,
                    ))
                    .load::<(Seqnum, crate::core::OwnedEventId, serde_json::Value)>(
                        &mut connect().await?,
                    )
                    .await?;
                if bodies.len() != members.len() {
                    return Err(MatrixError::unknown("published member event missing").into());
                }
                for (event_sn, event_id, body) in bodies {
                    let content: RoomMemberEventContent =
                        serde_json::from_value(body["content"].clone())?;
                    if content.membership == MembershipState::Join {
                        let member = UserId::parse(&members[&event_sn])?;
                        let point = MemberPoint { event_id, event_sn };
                        if member == user {
                            // A profile-only member update retains the same
                            // joined period; leave+rejoin starts a new one even
                            // if both endpoints contain this same room.
                            let period = joined_period(user, &point, join_periods).await?;
                            caller_join_period = Some(period);
                        }
                        joined.insert(member, point);
                    }
                }
            }
            entry.insert(FrameSharing {
                joined: Arc::new(joined),
                encrypted,
                caller_join_period,
            });
        }
        let FrameSharing {
            joined,
            encrypted,
            caller_join_period,
        } = &frames[&frame];
        if joined.contains_key(user) {
            let period = caller_join_period
                .ok_or_else(|| MatrixError::unknown("caller joined period missing"))?;
            sharing.joined_rooms.insert(room_id.clone(), period);
            sharing
                .room_members
                .insert(room_id.clone(), Arc::clone(joined));
            sharing.all.extend(joined.keys().cloned());
            if *encrypted {
                sharing.encrypted_rooms.insert(room_id);
                sharing.encrypted.extend(joined.keys().cloned());
            }
        }
    }
    sharing.all.remove(user);
    sharing.encrypted.remove(user);
    Ok(sharing)
}

/// `from` and `to` are next-sync positions: changes occupy [from, to).
/// The same calculation serves keys/changes, classic sync and sliding sync.
pub(crate) async fn changes(user: &UserId, from: Seqnum, to: Seqnum) -> AppResult<DeviceLists> {
    if from < 0 || to < from {
        return Err(MatrixError::invalid_param("invalid device-list sync interval").into());
    }
    if from == to {
        return Ok(DeviceLists::default());
    }
    let mut frames = HashMap::new();
    let mut join_periods = HashMap::new();
    let before = sharing_before(user, from, &mut frames, &mut join_periods).await?;
    let after = sharing_before(user, to, &mut frames, &mut join_periods).await?;
    // Preserve the existing sync notification for newly shared plaintext
    // peers too. Enabling encryption in an existing room starts E2EE tracking
    // even when the users already shared that plaintext room.
    let mut changed: BTreeSet<_> = after.all.difference(&before.all).cloned().collect();
    changed.extend(after.encrypted.difference(&before.encrypted).cloned());
    // Endpoint user sets alone cannot detect leave+rejoin between two syncs.
    // First prove continuity from unchanged member identities in any shared
    // room. Only then walk changed peer identities that have no other proof;
    // ordinary polls and unrelated state never walk every peer's join history.
    let mut continuous = BTreeSet::new();
    let mut changed_members = Vec::new();
    for (room_id, before_members) in &before.room_members {
        if before.joined_rooms.get(room_id) != after.joined_rooms.get(room_id) {
            continue;
        }
        let Some(after_members) = after.room_members.get(room_id) else {
            continue;
        };
        for (peer, old) in before_members.iter() {
            if peer == user {
                continue;
            }
            let Some(new) = after_members.get(peer) else {
                continue;
            };
            if old.event_id == new.event_id {
                continuous.insert(peer.clone());
            } else {
                changed_members.push((peer, old, new));
            }
        }
    }
    for (peer, old, new) in changed_members {
        if continuous.contains(peer) {
            continue;
        }
        if joined_period(peer, old, &mut join_periods).await?
            == joined_period(peer, new, &mut join_periods).await?
        {
            continuous.insert(peer.clone());
        }
    }
    changed.extend(
        before
            .all
            .intersection(&after.all)
            .filter(|peer| !continuous.contains(*peer))
            .cloned(),
    );
    // An imported/federated join can publish membership without uploading a
    // device key or writing an own-user key marker. Other devices must still
    // learn about this user's newly joined room. Keep that notification apart
    // from peer tracking so the caller is never reported in `left`.
    if after
        .joined_rooms
        .iter()
        .any(|(room, period)| before.joined_rooms.get(room) != Some(period))
        || !after.encrypted_rooms.is_subset(&before.encrypted_rooms)
    {
        changed.insert(user.to_owned());
    }
    let candidates: Vec<_> = after
        .all
        .iter()
        .cloned()
        .chain(std::iter::once(user.to_owned()))
        .collect();
    for changed_user in
        crate::data::user::key::keys_changed_in_window(&candidates, from, to).await?
    {
        // Device key updates are relevant in plaintext shared rooms too. A
        // user's own devices remain tracked independently of room membership.
        if changed_user == user || after.all.contains(&changed_user) {
            changed.insert(changed_user);
        }
    }
    let mut left: BTreeSet<_> = before
        .encrypted
        .difference(&after.encrypted)
        .cloned()
        .collect();
    left.extend(before.all.difference(&after.all).cloned());
    Ok(DeviceLists {
        changed: changed.into_iter().collect(),
        left: left.into_iter().collect(),
    })
}
