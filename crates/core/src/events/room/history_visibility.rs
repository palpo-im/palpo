//! Types for the [`m.room.history_visibility`] event.
//!
//! [`m.room.history_visibility`]: https://spec.matrix.org/latest/client-server-api/#mroomhistory_visibility

use crate::macros::EventContent;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::{PrivOwnedStr, events::EmptyStateKey, serde::StringEnum};

/// The content of an `m.room.history_visibility` event.
///
/// This event controls whether a member of a room can see the events that
/// happened in a room from before they joined.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.room.history_visibility", kind = State, state_key_type = EmptyStateKey)]
pub struct RoomHistoryVisibilityEventContent {
    /// Who can see the room history.
    #[palpo_event(skip_redaction)]
    pub history_visibility: HistoryVisibility,
}

impl RoomHistoryVisibilityEventContent {
    /// Creates a new `RoomHistoryVisibilityEventContent` with the given policy.
    pub fn new(history_visibility: HistoryVisibility) -> Self {
        Self { history_visibility }
    }
}

impl RoomHistoryVisibilityEvent {
    /// Obtain the history visibility, regardless of whether this event is
    /// redacted.
    pub fn history_visibility(&self) -> &HistoryVisibility {
        match self {
            Self::Original(ev) => &ev.content.history_visibility,
            Self::Redacted(ev) => &ev.content.history_visibility,
        }
    }
}

impl SyncRoomHistoryVisibilityEvent {
    /// Obtain the history visibility, regardless of whether this event is
    /// redacted.
    pub fn history_visibility(&self) -> &HistoryVisibility {
        match self {
            Self::Original(ev) => &ev.content.history_visibility,
            Self::Redacted(ev) => &ev.content.history_visibility,
        }
    }
}

/// Who can see a room's history.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, PartialEq, Eq, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HistoryVisibility {
    /// Previous events are accessible to newly joined members from the point
    /// they were invited onwards.
    ///
    /// Events stop being accessible when the member's state changes to
    /// something other than *invite* or *join*.
    Invited,

    /// Previous events are accessible to newly joined members from the point
    /// they joined the room onwards.
    /// Events stop being accessible when the member's state changes to
    /// something other than *join*.
    Joined,

    /// Previous events are always accessible to newly joined members.
    ///
    /// All events in the room are accessible, even those sent when the member
    /// was not a part of the room.
    Shared,

    /// All events while this is the `HistoryVisibility` value may be shared by
    /// any participating homeserver with anyone, regardless of whether they
    /// have ever joined the room.
    WorldReadable,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}
