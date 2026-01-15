//! Types for the [`m.room.join_rules`] event.
//!
//! [`m.room.join_rules`]: https://spec.matrix.org/latest/client-server-api/#mroomjoin_rules
use salvo::oapi::ToSchema;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};

use crate::events::EmptyStateKey;
use crate::macros::EventContent;
use crate::room::{AllowRule, JoinRule, Restricted};

/// The content of an `m.room.join_rules` event.
///
/// Describes how users are allowed to join the room.
#[derive(ToSchema, Clone, Debug, Serialize, EventContent)]
#[palpo_event(type = "m.room.join_rules", kind = State, state_key_type = EmptyStateKey)]
#[serde(transparent)]
pub struct RoomJoinRulesEventContent {
    /// The rule used for users wishing to join this room.
    #[palpo_event(skip_redaction)]
    #[serde(flatten)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
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
//     use assert_matches2::assert_matches;
//     use crate::owned_room_id;
//     use serde_json::json;

//     use super::{
//         AllowRule, JoinRule, OriginalSyncRoomJoinRulesEvent, RedactedRoomJoinRulesEventContent,
//         RoomJoinRulesEventContent,
//     };
//     use crate::room::join_rules::RedactedSyncRoomJoinRulesEvent;

//     #[test]
//     fn deserialize_content() {
//         let json = r#"{"join_rule": "public"}"#;

//         let event: RoomJoinRulesEventContent = serde_json::from_str(json).unwrap();
//         assert_matches!(
//             event,
//             RoomJoinRulesEventContent {
//                 join_rule: JoinRule::Public
//             }
//         );

//         let event: RedactedRoomJoinRulesEventContent = serde_json::from_str(json).unwrap();
//         assert_matches!(
//             event,
//             RedactedRoomJoinRulesEventContent {
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

//         let event: RoomJoinRulesEventContent = serde_json::from_str(json).unwrap();
//         assert_matches!(event.join_rule, JoinRule::Restricted(restricted));
//         assert_eq!(
//             restricted.allow,
//             &[
//                 AllowRule::room_membership(owned_room_id!("!mods:example.org")),
//                 AllowRule::room_membership(owned_room_id!("!users:example.org"))
//             ]
//         );

//         let event: RedactedRoomJoinRulesEventContent = serde_json::from_str(json).unwrap();
//         assert_matches!(event.join_rule, JoinRule::Restricted(restricted));
//         assert_eq!(
//             restricted.allow,
//             &[
//                 AllowRule::room_membership(owned_room_id!("!mods:example.org")),
//                 AllowRule::room_membership(owned_room_id!("!users:example.org"))
//             ]
//         );
//     }

//     #[test]
//     fn deserialize_restricted_event() {
//         let json = r#"{
//             "type": "m.room.join_rules",
//             "sender": "@admin:community.rs",
//             "content": {
//                 "join_rule": "restricted",
//                 "allow": [
//                     { "type": "m.room_membership","room_id": "!KqeUnzmXPIhHRaWMTs:mccarty.io" }
//                 ]
//             },
//             "state_key": "",
//             "origin_server_ts":1630508835342,
//             "unsigned": {
//                 "age":4165521871
//             },
//             "event_id": "$0ACb9KSPlT3al3kikyRYvFhMqXPP9ZcQOBrsdIuh58U"
//         }"#;

//         assert_matches!(
//             serde_json::from_str::<OriginalSyncRoomJoinRulesEvent>(json),
//             Ok(_)
//         );
//     }

//     #[test]
//     fn deserialize_redacted_restricted_event() {
//         let json = r#"{
//             "type": "m.room.join_rules",
//             "sender": "@admin:community.rs",
//             "content": {
//                 "join_rule": "restricted",
//                 "allow": [
//                     { "type": "m.room_membership","room_id": "!KqeUnzmXPIhHRaWMTs:mccarty.io" }
//                 ]
//             },
//             "state_key": "",
//             "origin_server_ts":1630508835342,
//             "unsigned": {
//                 "age":4165521871,
//                 "redacted_because": {
//                     "type": "m.room.redaction",
//                     "content": {
//                         "redacts": "$0ACb9KSPlT3al3kikyRYvFhMqXPP9ZcQOBrsdIuh58U"
//                     },
//                     "event_id": "$h29iv0s8",
//                     "origin_server_ts": 1,
//                     "sender": "@carl:example.com"
//                 }
//             },
//             "event_id": "$0ACb9KSPlT3al3kikyRYvFhMqXPP9ZcQOBrsdIuh58U"
//         }"#;

//         assert_matches!(
//             serde_json::from_str::<RedactedSyncRoomJoinRulesEvent>(json),
//             Ok(_)
//         );
//     }

//     #[test]
//     fn restricted_room_no_allow_field() {
//         let json = r#"{"join_rule":"restricted"}"#;
//         let join_rules: RoomJoinRulesEventContent = serde_json::from_str(json).unwrap();
//         assert_matches!(
//             join_rules,
//             RoomJoinRulesEventContent {
//                 join_rule: JoinRule::Restricted(_)
//             }
//         );
//     }

//     #[test]
//     fn reserialize_unsupported_join_rule() {
//         let json = json!({"join_rule": "local.matrix.custom", "foo": "bar"});

//         let content = serde_json::from_value::<RoomJoinRulesEventContent>(json.clone()).unwrap();
//         assert_eq!(content.join_rule.as_str(), "local.matrix.custom");
//         let data = content.join_rule.data();
//         assert_eq!(data.get("foo").unwrap().as_str(), Some("bar"));

//         assert_eq!(serde_json::to_value(&content).unwrap(), json);
//     }
// }
