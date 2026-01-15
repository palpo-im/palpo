//! Types for the [`m.room.tombstone`] event.
//!
//! [`m.room.tombstone`]: https://spec.matrix.org/latest/client-server-api/#mroomtombstone

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::OwnedRoomId;
use crate::events::{
    EmptyStateKey, PossiblyRedactedStateEventContent, StateEventType, StaticEventContent,
};
use crate::macros::EventContent;

/// The content of an `m.room.tombstone` event.
///
/// A state event signifying that a room has been upgraded to a different room
/// version, and that clients should go there.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(
    type = "m.room.tombstone",
    kind = State,
    state_key_type = EmptyStateKey,
    custom_possibly_redacted,
)]
pub struct RoomTombstoneEventContent {
    /// A server-defined message.
    #[serde(default)]
    pub body: String,

    /// The new room the client should be visiting.
    pub replacement_room: OwnedRoomId,
}

impl RoomTombstoneEventContent {
    /// Creates a new `RoomTombstoneEventContent` with the given body and
    /// replacement room ID.
    pub fn new(body: String, replacement_room: OwnedRoomId) -> Self {
        Self {
            body,
            replacement_room,
        }
    }
}

/// The possibly redacted form of [`RoomTombstoneEventContent`].
///
/// This type is used when it's not obvious whether the content is redacted or
/// not.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct PossiblyRedactedRoomTombstoneEventContent {
    /// A server-defined message.
    pub body: Option<String>,

    /// The new room the client should be visiting.
    pub replacement_room: Option<OwnedRoomId>,
}

impl PossiblyRedactedStateEventContent for PossiblyRedactedRoomTombstoneEventContent {
    type StateKey = EmptyStateKey;

    fn event_type(&self) -> StateEventType {
        StateEventType::RoomTombstone
    }
}

impl StaticEventContent for PossiblyRedactedRoomTombstoneEventContent {
    const TYPE: &'static str = RoomTombstoneEventContent::TYPE;
    type IsPrefix = <RoomTombstoneEventContent as StaticEventContent>::IsPrefix;
}
