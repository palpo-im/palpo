//! Device tracking at immutable sync publication boundaries.
use std::collections::{BTreeSet, HashMap};

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::device::DeviceLists;
use crate::core::events::StateEventType;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::{OwnedUserId, Seqnum, UserId};
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
}

async fn sharing_before(
    user: &UserId,
    boundary: Seqnum,
    frames: &mut HashMap<i64, (BTreeSet<OwnedUserId>, bool)>,
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
            let mut joined = BTreeSet::new();
            let mut encrypted = false;
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
                    .select((event_points::event_sn, event_datas::json_data))
                    .load::<(Seqnum, serde_json::Value)>(&mut connect().await?)
                    .await?;
                if bodies.len() != members.len() {
                    return Err(MatrixError::unknown("published member event missing").into());
                }
                for (event_sn, body) in bodies {
                    let content: RoomMemberEventContent =
                        serde_json::from_value(body["content"].clone())?;
                    if content.membership == MembershipState::Join {
                        joined.insert(UserId::parse(&members[&event_sn])?);
                    }
                }
            }
            entry.insert((joined, encrypted));
        }
        let (joined, encrypted) = &frames[&frame];
        if joined.contains(user) {
            sharing.all.extend(joined.iter().cloned());
            if *encrypted {
                sharing.encrypted.extend(joined.iter().cloned());
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
    let before = sharing_before(user, from, &mut frames).await?;
    let after = sharing_before(user, to, &mut frames).await?;
    // Preserve the existing sync notification for newly shared plaintext
    // peers too. Enabling encryption in an existing room starts E2EE tracking
    // even when the users already shared that plaintext room.
    let mut changed: BTreeSet<_> = after.all.difference(&before.all).cloned().collect();
    changed.extend(after.encrypted.difference(&before.encrypted).cloned());
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
    Ok(DeviceLists {
        changed: changed.into_iter().collect(),
        left: before
            .encrypted
            .difference(&after.encrypted)
            .cloned()
            .collect(),
    })
}
