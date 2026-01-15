//! Types for the [`m.room.create`] event.
//!
//! [`m.room.create`]: https://spec.matrix.org/latest/client-server-api/#mroomcreate

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::events::{EmptyStateKey, RedactContent, RedactedStateEventContent, StateEventType};
use crate::macros::EventContent;
use crate::room::RoomType;
use crate::room_version_rules::RedactionRules;
use crate::{OwnedRoomId, OwnedUserId, RoomVersionId};

/// The content of an `m.room.create` event.
///
/// This is the first event in a room and cannot be changed.
///
/// It acts as the root of all other events.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.room.create", kind = State, state_key_type = EmptyStateKey, custom_redacted)]
pub struct RoomCreateEventContent {
    /// The `user_id` of the room creator.
    ///
    /// This is set by the homeserver.
    ///
    /// This is required in room versions 1 trough 10, but is removed starting
    /// from room version 11.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[deprecated = "Since Matrix 1.8. This field was removed in Room version 11, clients should use the event's sender instead"]
    pub creator: Option<OwnedUserId>,

    /// Whether or not this room's data should be transferred to other
    /// homeservers.
    #[serde(
        rename = "m.federate",
        default = "crate::serde::default_true",
        skip_serializing_if = "crate::serde::is_true"
    )]
    pub federate: bool,

    /// The version of the room.
    ///
    /// Defaults to `RoomVersionId::V1`.
    #[serde(default = "default_room_version_id")]
    pub room_version: RoomVersionId,

    /// A reference to the room this room replaces, if the previous room was
    /// upgraded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<PreviousRoom>,

    /// The room type.
    ///
    /// This is currently only used for spaces.
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub room_type: Option<RoomType>,
}

impl RoomCreateEventContent {
    /// Creates a new `RoomCreateEventContent` with the given creator, as
    /// required for room versions 1 through 10.
    pub fn new_v1(creator: OwnedUserId) -> Self {
        Self {
            creator: Some(creator),
            federate: true,
            room_version: default_room_version_id(),
            predecessor: None,
            room_type: None,
        }
    }

    /// Creates a new `RoomCreateEventContent` with the default values and no
    /// creator, as introduced in room version 11.
    ///
    /// The room version is set to [`RoomVersionId::V11`].
    pub fn new_v11() -> Self {
        Self {
            creator: None,
            federate: true,
            room_version: RoomVersionId::V11,
            predecessor: None,
            room_type: None,
        }
    }

    /// Creates a new `RoomCreateEventContent` with the default values and no
    /// creator, as introduced in room version 11.
    ///
    /// The room version is set to [`RoomVersionId::V12`].
    pub fn new_v12() -> Self {
        Self {
            creator: None,
            federate: true,
            room_version: RoomVersionId::V12,
            predecessor: None,
            room_type: None,
        }
    }
}

impl RedactContent for RoomCreateEventContent {
    type Redacted = RedactedRoomCreateEventContent;

    fn redact(self, rules: &RedactionRules) -> Self::Redacted {
        #[allow(deprecated)]
        if rules.keep_room_create_content {
            self
        } else {
            Self {
                room_version: default_room_version_id(),
                creator: self.creator,
                ..Self::new_v11()
            }
        }
    }
}

/// A reference to an old room replaced during a room version upgrade.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct PreviousRoom {
    /// The ID of the old room.
    pub room_id: OwnedRoomId,
    // /// The event ID of the last known event in the old room.
    // #[deprecated = "\
    //     This field should be sent by servers when possible for backwards compatibility \
    //     but clients should not rely on it."]
    // #[serde(skip_serializing_if = "Option::is_none")]
    // pub event_id: Option<OwnedEventId>,
}

impl PreviousRoom {
    /// Creates a new `PreviousRoom` from the given room and event IDs.
    pub fn new(room_id: OwnedRoomId) -> Self {
        Self { room_id }
    }
}

/// Used to default the `room_version` field to room version 1.
fn default_room_version_id() -> RoomVersionId {
    RoomVersionId::V1
}

/// Redacted form of [`RoomCreateEventContent`].
///
/// The redaction rules of this event changed with room version 11:
///
/// - In room versions 1 through 10, the `creator` field was preserved during redaction, starting
///   from room version 11 the field is removed.
/// - In room versions 1 through 10, all the other fields were redacted, starting from room version
///   11 all the fields are preserved.
pub type RedactedRoomCreateEventContent = RoomCreateEventContent;

impl RedactedStateEventContent for RedactedRoomCreateEventContent {
    type StateKey = EmptyStateKey;

    fn event_type(&self) -> StateEventType {
        StateEventType::RoomCreate
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::{owned_user_id, RoomVersionId};
//     use assert_matches2::assert_matches;
//     use serde_json::{from_value as from_json_value, json, to_value as
// to_json_value};

//     use super::{RoomCreateEventContent, RoomType};

//     #[test]
//     fn serialization() {
//         let content = RoomCreateEventContent {
//             federate: false,
//             room_version: RoomVersionId::V4,
//             predecessor: None,
//             room_type: None,
//         };

//         let json = json!({
//             "creator": "@carl:example.com",
//             "m.federate": false,
//             "room_version": "4"
//         });

//         assert_eq!(to_json_value(&content).unwrap(), json);
//     }

//     #[test]
//     fn space_serialization() {
//         let content = RoomCreateEventContent {
//             federate: false,
//             room_version: RoomVersionId::V4,
//             predecessor: None,
//             room_type: Some(RoomType::Space),
//         };

//         let json = json!({
//             "creator": "@carl:example.com",
//             "m.federate": false,
//             "room_version": "4",
//             "type": "m.space"
//         });

//         assert_eq!(to_json_value(&content).unwrap(), json);
//     }

//     #[test]
//     fn deserialization() {
//         let json = json!({
//             "creator": "@carl:example.com",
//             "m.federate": true,
//             "room_version": "4"
//         });

//         let content =
// from_json_value::<RoomCreateEventContent>(json).unwrap();         assert_eq!
// (content.creator.unwrap(), "@carl:example.com");         assert!(content.
// federate);         assert_eq!(content.room_version, RoomVersionId::V4);
//         assert_matches!(content.predecessor, None);
//         assert_eq!(content.room_type, None);
//     }

//     #[test]
//     fn space_deserialization() {
//         let json = json!({
//             "creator": "@carl:example.com",
//             "m.federate": true,
//             "room_version": "4",
//             "type": "m.space"
//         });

//         let content =
// from_json_value::<RoomCreateEventContent>(json).unwrap();         assert_eq!
// (content.creator.unwrap(), "@carl:example.com");         assert!(content.
// federate);         assert_eq!(content.room_version, RoomVersionId::V4);
//         assert_matches!(content.predecessor, None);
//         assert_eq!(content.room_type, Some(RoomType::Space));
//     }
// }
