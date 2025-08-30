//! Types for the [`m.room.join_rules`] event.
//!
//! [`m.room.join_rules`]: https://spec.matrix.org/latest/client-server-api/#mroomjoin_rules
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize, de::Deserializer};

use crate::macros::EventContent;
use crate::{
    events::EmptyStateKey,
    room::{AllowRule, JoinRule, Restricted},
};

/// The content of an `m.room.join_rules` event.
///
/// Describes how users are allowed to join the room.
#[derive(ToSchema, Clone, Debug, Serialize, EventContent)]
#[palpo_event(type = "m.room.join_rules", kind = State, state_key_type = EmptyStateKey)]
pub struct RoomJoinRulesEventContent {
    /// The type of rules used for users wishing to join this room.
    #[palpo_event(skip_redaction)]
    #[serde(flatten)]
    pub join_rule: JoinRule,
}

impl RoomJoinRulesEventContent {
    /// Creates a new `RoomJoinRulesEventContent` with the given rule.
    pub fn new(join_rule: JoinRule) -> Self {
        Self { join_rule }
    }

    /// Creates a new `RoomJoinRulesEventContent` with the restricted rule and
    /// the given set of allow rules.
    pub fn restricted(allow: Vec<AllowRule>) -> Self {
        Self {
            join_rule: JoinRule::Restricted(Restricted::new(allow)),
        }
    }

    /// Creates a new `RoomJoinRulesEventContent` with the knock restricted rule
    /// and the given set of allow rules.
    pub fn knock_restricted(allow: Vec<AllowRule>) -> Self {
        Self {
            join_rule: JoinRule::KnockRestricted(Restricted::new(allow)),
        }
    }
}

impl<'de> Deserialize<'de> for RoomJoinRulesEventContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let join_rule = JoinRule::deserialize(deserializer)?;
        Ok(RoomJoinRulesEventContent { join_rule })
    }
}

impl RoomJoinRulesEvent {
    /// Obtain the join rule, regardless of whether this event is redacted.
    pub fn join_rule(&self) -> &JoinRule {
        match self {
            Self::Original(ev) => &ev.content.join_rule,
            Self::Redacted(ev) => &ev.content.join_rule,
        }
    }
}

impl SyncRoomJoinRulesEvent {
    /// Obtain the join rule, regardless of whether this event is redacted.
    pub fn join_rule(&self) -> &JoinRule {
        match self {
            Self::Original(ev) => &ev.content.join_rule,
            Self::Redacted(ev) => &ev.content.join_rule,
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::owned_room_id;
//     use assert_matches2::assert_matches;

//     use super::{AllowRule, JoinRule, OriginalSyncRoomJoinRulesEvent,
// RoomJoinRulesEventContent};

//     #[test]
//     fn deserialize() {
//         let json = r#"{"join_rule": "public"}"#;
//         let event: RoomJoinRulesEventContent =
// serde_json::from_str(json).unwrap();         assert_matches!(
//             event,
//             RoomJoinRulesEventContent {
//                 join_rule: JoinRule::Public
//             }
//         );
//     }

//     #[test]
//     fn deserialize_restricted() {
//         let json = r#"{
//             "join_rule": "restricted",
//             "allow": [
//                 {
//                     "type": "m.room_membership",
//                     "room_id": "!mods:example.org"
//                 },
//                 {
//                     "type": "m.room_membership",
//                     "room_id": "!users:example.org"
//                 }
//             ]
//         }"#;
//         let event: RoomJoinRulesEventContent =
// serde_json::from_str(json).unwrap();         match event.join_rule {
//             JoinRule::Restricted(restricted) => assert_eq!(
//                 restricted.allow,
//                 &[
//
// AllowRule::room_membership(owned_room_id!("!mods:example.org")),
// AllowRule::room_membership(owned_room_id!("!users:example.org"))
// ]             ),
//             rule => panic!("Deserialized to wrong variant: {rule:?}"),
//         }
//     }

//     #[test]
//     fn deserialize_restricted_event() {
//         let json = r#"{
//             "type": "m.room.join_rules",
//             "sender": "@admin:community.rs",
//             "content": {
//                 "join_rule": "restricted",
//                 "allow": [
//                     { "type": "m.room_membership","room_id":
// "!KqeUnzmXPIhHRaWMTs:mccarty.io" }                 ]
//             },
//             "state_key": "",
//             "origin_server_ts":1630508835342,
//             "unsigned": {
//                 "age":4165521871
//             },
//             "event_id": "$0ACb9KSPlT3al3kikyRYvFhMqXPP9ZcQOBrsdIuh58U"
//         }"#;

//         assert_matches!
// (serde_json::from_str::<OriginalSyncRoomJoinRulesEvent>(json), Ok(_));     }

//     #[test]
//     fn roundtrip_custom_allow_rule() {
//         let json = r#"{"type":"org.msc9000.something","foo":"bar"}"#;
//         let allow_rule: AllowRule = serde_json::from_str(json).unwrap();
//         assert_matches!(&allow_rule, AllowRule::_Custom(_));
//         assert_eq!(serde_json::to_string(&allow_rule).unwrap(), json);
//     }
// }
