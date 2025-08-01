//! Types for the [`m.room.join_rules`] event.
//!
//! [`m.room.join_rules`]: https://spec.matrix.org/latest/client-server-api/#mroomjoin_rules

use std::{
    borrow::{Borrow, Cow},
    collections::BTreeMap,
};

use crate::macros::EventContent;
use salvo::oapi::ToSchema;
use serde::{
    Deserialize, Serialize,
    de::{Deserializer, Error},
};

use crate::{
    OwnedRoomId, PrivOwnedStr, RoomId,
    events::EmptyStateKey,
    serde::{JsonValue, RawJsonValue, from_raw_json_value},
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

/// The rule used for users wishing to join this room.
///
/// This type can hold an arbitrary string. To check for values that are not
/// available as a documented variant here, use its string representation,
/// obtained through `.as_str()`.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "join_rule", rename_all = "snake_case")]
pub enum JoinRule {
    /// A user who wishes to join the room must first receive an invite to the
    /// room from someone already inside of the room.
    Invite,

    /// Users can join the room if they are invited, or they can request an
    /// invite to the room.
    ///
    /// They can be allowed (invited) or denied (kicked/banned) access.
    Knock,

    /// Reserved but not yet implemented by the Matrix specification.
    Private,

    /// Users can join the room if they are invited, or if they meet any of the
    /// conditions described in a set of [`AllowRule`]s.
    Restricted(Restricted),

    /// Users can join the room if they are invited, or if they meet any of the
    /// conditions described in a set of [`AllowRule`]s, or they can request
    /// an invite to the room.
    KnockRestricted(Restricted),

    /// Anyone can join the room without any prior action.
    Public,

    #[doc(hidden)]
    #[serde(skip_serializing)]
    _Custom(PrivOwnedStr),
}

impl JoinRule {
    pub fn is_restricted(&self) -> bool {
        matches!(self, JoinRule::Restricted(_) | JoinRule::KnockRestricted(_))
    }
    pub fn restriction_rooms(&self) -> Vec<OwnedRoomId> {
        match self {
            JoinRule::Restricted(restricted) | JoinRule::KnockRestricted(restricted) => restricted
                .allow
                .iter()
                .filter_map(|a| match a {
                    AllowRule::RoomMembership(r) => Some(r.room_id.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    /// Returns allowed room_id's for restricted rooms; empty for other variants
    pub fn allowed_rooms(&self) -> impl Iterator<Item = &RoomId> + Send {
        let rules = match self {
            JoinRule::Restricted(rules) | JoinRule::KnockRestricted(rules) => Some(rules),
            _ => None,
        };

        rules
            .into_iter()
            .flat_map(|rules| rules.allow.iter())
            .filter_map(|rule| {
                if let AllowRule::RoomMembership(RoomMembership {
                    room_id: membership,
                }) = rule
                {
                    Some(membership.borrow())
                } else {
                    None
                }
            })
    }
    /// Returns the string name of this `JoinRule`
    pub fn as_str(&self) -> &str {
        match self {
            JoinRule::Invite => "invite",
            JoinRule::Knock => "knock",
            JoinRule::Private => "private",
            JoinRule::Restricted(_) => "restricted",
            JoinRule::KnockRestricted(_) => "knock_restricted",
            JoinRule::Public => "public",
            JoinRule::_Custom(rule) => &rule.0,
        }
    }
}

impl<'de> Deserialize<'de> for JoinRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let json: Box<RawJsonValue> = Box::deserialize(deserializer)?;

        #[derive(Deserialize)]
        struct ExtractType<'a> {
            #[serde(borrow)]
            join_rule: Option<Cow<'a, str>>,
        }

        let join_rule = serde_json::from_str::<ExtractType<'_>>(json.get())
            .map_err(serde::de::Error::custom)?
            .join_rule
            .ok_or_else(|| D::Error::missing_field("join_rule"))?;

        match join_rule.as_ref() {
            "invite" => Ok(Self::Invite),
            "knock" => Ok(Self::Knock),
            "private" => Ok(Self::Private),
            "restricted" => from_raw_json_value(&json).map(Self::Restricted),
            "knock_restricted" => from_raw_json_value(&json).map(Self::KnockRestricted),
            "public" => Ok(Self::Public),
            _ => Ok(Self::_Custom(PrivOwnedStr(join_rule.into()))),
        }
    }
}

/// Configuration of the `Restricted` join rule.
#[derive(ToSchema, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restricted {
    /// Allow rules which describe conditions that allow joining a room.
    #[serde(default)]
    pub allow: Vec<AllowRule>,
}

impl Restricted {
    /// Constructs a new rule set for restricted rooms with the given rules.
    pub fn new(allow: Vec<AllowRule>) -> Self {
        Self { allow }
    }
}

/// An allow rule which defines a condition that allows joining a room.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum AllowRule {
    /// Joining is allowed if a user is already a member of the room with the id
    /// `room_id`.
    RoomMembership(RoomMembership),

    #[doc(hidden)]
    _Custom(Box<CustomAllowRule>),
}

impl AllowRule {
    /// Constructs an `AllowRule` with membership of the room with the given id
    /// as its predicate.
    pub fn room_membership(room_id: OwnedRoomId) -> Self {
        Self::RoomMembership(RoomMembership::new(room_id))
    }
}

/// Allow rule which grants permission to join based on the membership of
/// another room.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "m.room_membership")]
pub struct RoomMembership {
    /// The id of the room which being a member of grants permission to join
    /// another room.
    pub room_id: OwnedRoomId,
}

impl RoomMembership {
    /// Constructs a new room membership rule for the given room id.
    pub fn new(room_id: OwnedRoomId) -> Self {
        Self { room_id }
    }
}

#[doc(hidden)]
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomAllowRule {
    #[serde(rename = "type")]
    rule_type: String,
    #[serde(flatten)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    extra: BTreeMap<String, JsonValue>,
}

impl<'de> Deserialize<'de> for AllowRule {
    fn deserialize<D>(deserializer: D) -> Result<AllowRule, D::Error>
    where
        D: Deserializer<'de>,
    {
        let json: Box<RawJsonValue> = Box::deserialize(deserializer)?;

        // Extracts the `type` value.
        #[derive(Deserialize)]
        struct ExtractType<'a> {
            #[serde(borrow, rename = "type")]
            rule_type: Option<Cow<'a, str>>,
        }

        // Get the value of `type` if present.
        let rule_type = serde_json::from_str::<ExtractType<'_>>(json.get())
            .map_err(serde::de::Error::custom)?
            .rule_type;

        match rule_type.as_deref() {
            Some("m.room_membership") => from_raw_json_value(&json).map(Self::RoomMembership),
            Some(_) => from_raw_json_value(&json).map(Self::_Custom),
            None => Err(D::Error::missing_field("type")),
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
