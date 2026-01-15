use std::collections::BTreeSet;
use std::mem::size_of;
use std::ops::Deref;
use std::sync::Arc;

use diesel::prelude::*;

use super::{DbRoomStateDelta, FrameInfo, room_state_deltas};
use crate::core::{OwnedEventId, RoomId, Seqnum};
use crate::data::connect;
use crate::{AppResult, utils};

pub struct StateDiff {
    pub parent_id: Option<i64>,
    pub appended: Arc<CompressedState>,
    pub disposed: Arc<CompressedState>,
}

#[derive(Clone, Default)]
pub struct DeltaInfo {
    pub frame_id: i64,
    pub appended: Arc<CompressedState>,
    pub disposed: Arc<CompressedState>,
}

pub type CompressedState = BTreeSet<CompressedEvent>;

#[derive(Eq, Ord, Hash, PartialEq, PartialOrd, Copy, Debug, Clone)]
pub struct CompressedEvent([u8; 2 * size_of::<i64>()]);
impl CompressedEvent {
    pub fn new(field_id: i64, event_sn: Seqnum) -> Self {
        let mut v = field_id.to_be_bytes().to_vec();
        v.extend_from_slice(&event_sn.to_be_bytes());
        Self(v.try_into().expect("we checked the size above"))
    }
    pub fn field_id(&self) -> i64 {
        utils::i64_from_bytes(&self.0[0..size_of::<i64>()]).expect("bytes have right length")
    }
    pub fn event_sn(&self) -> Seqnum {
        utils::i64_from_bytes(&self.0[size_of::<i64>()..]).expect("bytes have right length")
    }
    /// Returns state_key_id, event id
    pub fn split(&self) -> AppResult<(i64, OwnedEventId)> {
        Ok((
            utils::i64_from_bytes(&self[0..size_of::<i64>()]).expect("bytes have right length"),
            crate::event::get_event_id_by_sn(
                utils::i64_from_bytes(&self[size_of::<i64>()..]).expect("bytes have right length"),
            )?,
        ))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl Deref for CompressedEvent {
    type Target = [u8; 2 * size_of::<i64>()];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub fn compress_events(
    room_id: &RoomId,
    events: impl Iterator<Item = (i64, Seqnum)>,
) -> AppResult<CompressedState> {
    let mut compressed = BTreeSet::new();
    for (field_id, event_sn) in events {
        compressed.insert(compress_event(room_id, field_id, event_sn)?);
    }
    Ok(compressed)
}

pub fn compress_event(
    _room_id: &RoomId,
    field_id: i64,
    event_sn: Seqnum,
) -> AppResult<CompressedEvent> {
    Ok(CompressedEvent::new(field_id, event_sn))
}

pub fn get_detla(frame_id: i64) -> AppResult<DbRoomStateDelta> {
    room_state_deltas::table
        .find(frame_id)
        .first::<DbRoomStateDelta>(&mut connect()?)
        .map_err(Into::into)
}
pub fn load_state_diff(frame_id: i64) -> AppResult<StateDiff> {
    let DbRoomStateDelta {
        parent_id,
        appended,
        disposed,
        ..
    } = room_state_deltas::table
        .find(frame_id)
        .first::<DbRoomStateDelta>(&mut connect()?)?;
    Ok(StateDiff {
        parent_id,
        appended: Arc::new(
            appended
                .chunks_exact(size_of::<CompressedEvent>())
                .map(|chunk| CompressedEvent(chunk.try_into().expect("we checked the size above")))
                .collect(),
        ),
        disposed: Arc::new(
            disposed
                .chunks_exact(size_of::<CompressedEvent>())
                .map(|chunk| CompressedEvent(chunk.try_into().expect("we checked the size above")))
                .collect(),
        ),
    })
}

pub fn save_state_delta(room_id: &RoomId, frame_id: i64, diff: StateDiff) -> AppResult<()> {
    let StateDiff {
        parent_id,
        appended,
        disposed,
    } = diff;
    diesel::insert_into(room_state_deltas::table)
        .values(DbRoomStateDelta {
            frame_id,
            room_id: room_id.to_owned(),
            parent_id,
            appended: appended
                .iter()
                .flat_map(|event| event.as_bytes())
                .cloned()
                .collect::<Vec<_>>(),
            disposed: disposed
                .iter()
                .flat_map(|event| event.as_bytes())
                .cloned()
                .collect::<Vec<_>>(),
        })
        .on_conflict_do_nothing()
        .execute(&mut connect()?)?;
    Ok(())
}
/// Creates a new state_hash that often is just a diff to an already existing
/// state_hash and therefore very efficient.
///
/// There are multiple layers of diffs. The bottom layer 0 always contains the full state. Layer
/// 1 contains diffs to states of layer 0, layer 2 diffs to layer 1 and so on. If layer n > 0
/// grows too big, it will be combined with layer n-1 to create a new diff on layer n-1 that's
/// based on layer n-2. If that layer is also too big, it will recursively fix above layers too.
///
/// * `point_id` - Shortstate_hash of this state
/// * `appended` - Added to base. Each vec is state_key_id+shorteventid
/// * `disposed` - Removed from base. Each vec is state_key_id+shorteventid
/// * `diff_to_sibling` - Approximately how much the diff grows each time for this layer
/// * `parent_states` - A stack with info on state_hash, full state, added diff and removed diff for
///   each parent layer
#[tracing::instrument(skip(appended, disposed, diff_to_sibling, parent_states))]
pub fn calc_and_save_state_delta(
    room_id: &RoomId,
    frame_id: i64,
    appended: Arc<CompressedState>,
    disposed: Arc<CompressedState>,
    diff_to_sibling: usize,
    mut parent_states: Vec<FrameInfo>,
) -> AppResult<()> {
    let diff_sum = appended.len() + disposed.len();

    if parent_states.len() > 3 {
        // Number of layers
        // To many layers, we have to go deeper
        let parent = parent_states.pop().unwrap();

        let mut parent_appended = (*parent.appended).clone();
        let mut parent_disposed = (*parent.disposed).clone();

        for item in disposed.iter() {
            if !parent_appended.remove(item) {
                // It was not added in the parent and we removed it
                parent_disposed.insert(*item);
            }
            // Else it was added in the parent and we removed it again. We can forget this change
        }

        for item in appended.iter() {
            if !parent_disposed.remove(item) {
                // It was not touched in the parent and we added it
                parent_appended.insert(*item);
            }
            // Else it was removed in the parent and we added it again. We can forget this change
        }

        return calc_and_save_state_delta(
            room_id,
            frame_id,
            Arc::new(parent_appended),
            Arc::new(parent_disposed),
            diff_sum,
            parent_states,
        );
    }

    if parent_states.is_empty() {
        // There is no parent layer, create a new state
        return save_state_delta(
            room_id,
            frame_id,
            StateDiff {
                parent_id: None,
                appended,
                disposed,
            },
        );
    }

    // Else we have two options.
    // 1. We add the current diff on top of the parent layer.
    // 2. We replace a layer above
    let parent = parent_states.pop().unwrap();
    let parent_diff = parent.appended.len() + parent.disposed.len();

    if diff_sum * diff_sum >= 2 * diff_to_sibling * parent_diff {
        // Diff too big, we replace above layer(s)
        let mut parent_appended = (*parent.appended).clone();
        let mut parent_disposed = (*parent.disposed).clone();

        for item in disposed.iter() {
            if !parent_appended.remove(item) {
                // It was not added in the parent and we removed it
                parent_disposed.insert(*item);
            }
            // Else it was added in the parent and we removed it again. We can forget this change
        }

        for item in appended.iter() {
            if !parent_disposed.remove(item) {
                // It was not touched in the parent and we added it
                parent_appended.insert(*item);
            }
            // Else it was removed in the parent and we added it again. We can forget this change
        }

        calc_and_save_state_delta(
            room_id,
            frame_id,
            Arc::new(parent_appended),
            Arc::new(parent_disposed),
            diff_sum,
            parent_states,
        )
    } else {
        // Diff small enough, we add diff as layer on top of parent
        save_state_delta(
            room_id,
            frame_id,
            StateDiff {
                parent_id: Some(parent.frame_id),
                appended,
                disposed,
            },
        )
    }
}
