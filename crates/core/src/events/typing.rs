//! Types for the [`m.typing`] event.
//!
//! [`m.typing`]: https://spec.matrix.org/latest/client-server-api/#mtyping

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;
use crate::{OwnedRoomId, OwnedUserId};

/// The content of an `m.typing` event.
///
/// Informs the client who is currently typing in a given room.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[palpo_event(type = "m.typing", kind = EphemeralRoom)]
pub struct TypingEventContent {
    /// The list of user IDs typing in this room, if any.
    pub user_ids: Vec<OwnedUserId>,
}

impl TypingEventContent {
    /// Creates a new `TypingEventContent` with the given user IDs.
    pub fn new(user_ids: Vec<OwnedUserId>) -> Self {
        Self { user_ids }
    }
}

/// The content for "m.typing" Edu.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct TypingContent {
    /// The room where the user's typing status has been updated.
    pub room_id: OwnedRoomId,

    /// The user ID that has had their typing status changed.
    pub user_id: OwnedUserId,

    /// Whether the user is typing in the room or not.
    pub typing: bool,
}

impl TypingContent {
    /// Creates a new `TypingContent`.
    pub fn new(room_id: OwnedRoomId, user_id: OwnedUserId, typing: bool) -> Self {
        Self {
            room_id,
            user_id,
            typing,
        }
    }
}
