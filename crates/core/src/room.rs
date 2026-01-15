//! Common types for rooms.
use std::borrow::{Borrow, Cow};
use std::collections::BTreeMap;

use as_variant::as_variant;
use salvo::prelude::*;
use serde::de::Error as _;
use serde::{Deserialize, Serialize, de};
use serde_json::Value as JsonValue;
use serde_json::value::RawValue as RawJsonValue;

use crate::events::StateEventType;
use crate::serde::{JsonObject, StringEnum, from_raw_json_value};
use crate::{
    Direction, EventEncryptionAlgorithm, OwnedEventId, OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId,
    OwnedUserId, PrivOwnedStr, RoomId, RoomVersionId, UnixMillis,
};

/// An enum of possible room types.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
pub enum RoomType {
    /// Defines the room as a space.
    #[palpo_enum(rename = "m.space")]
    Space,

    /// Defines the room as a custom type.
    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, Default, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Visibility {
    /// Indicates that the room will be shown in the published room list.
    Public,

    /// Indicates that the room will not be shown in the published room list.
    #[default]
    Private,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct RoomEventReqArgs {
    /// Room in which the event to be reported is located.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// Event to report.
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,
}
#[derive(ToParameters, Deserialize, Debug)]
pub struct RoomTypingReqArgs {
    /// Room in which the event to be reported is located.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// Event to report.
    #[salvo(parameter(parameter_in = Path))]
    pub user_id: OwnedUserId,
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct RoomUserReqArgs {
    /// Room in which the event to be reported is located.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// Event to report.
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct RoomEventTypeReqArgs {
    /// The ID of the room the event is in.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The ID of the event.
    #[salvo(parameter(parameter_in = Path))]
    pub event_type: StateEventType,
}

/// The rule used for users wishing to join this room.
///
/// This type can hold an arbitrary join rule. To check for values that are not available as a
/// documented variant here, get its kind with [`.kind()`](Self::kind) or its string representation
/// with [`.as_str()`](Self::as_str), and its associated data with [`.data()`](Self::data).
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
    #[salvo(schema(value_type = Object))]
    _Custom(CustomJoinRule),
}

impl JoinRule {
    /// Returns the kind of this `JoinRule`.
    pub fn kind(&self) -> JoinRuleKind {
        match self {
            Self::Invite => JoinRuleKind::Invite,
            Self::Knock => JoinRuleKind::Knock,
            Self::Private => JoinRuleKind::Private,
            Self::Restricted(_) => JoinRuleKind::Restricted,
            Self::KnockRestricted(_) => JoinRuleKind::KnockRestricted,
            Self::Public => JoinRuleKind::Public,
            Self::_Custom(CustomJoinRule { join_rule, .. }) => {
                JoinRuleKind::_Custom(PrivOwnedStr(join_rule.as_str().into()))
            }
        }
    }
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
            JoinRule::_Custom(CustomJoinRule { join_rule, .. }) => join_rule,
        }
    }

    /// Returns the associated data of this `JoinRule`.
    ///
    /// The returned JSON object won't contain the `join_rule` field, use
    /// [`.kind()`](Self::kind) or [`.as_str()`](Self::as_str) to access those.
    ///
    /// Prefer to use the public variants of `JoinRule` where possible; this method is meant to
    /// be used for custom join rules only.
    pub fn data(&self) -> Cow<'_, JsonObject> {
        fn serialize<T: Serialize>(obj: &T) -> JsonObject {
            match serde_json::to_value(obj).expect("join rule serialization should succeed") {
                JsonValue::Object(mut obj) => {
                    obj.remove("body");
                    obj
                }
                _ => panic!("all message types should serialize to objects"),
            }
        }

        match self {
            JoinRule::Invite | JoinRule::Knock | JoinRule::Private | JoinRule::Public => {
                Cow::Owned(JsonObject::new())
            }
            JoinRule::Restricted(restricted) | JoinRule::KnockRestricted(restricted) => {
                Cow::Owned(serialize(restricted))
            }
            Self::_Custom(c) => Cow::Borrowed(&c.data),
        }
    }
}

impl<'de> Deserialize<'de> for JoinRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
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
            _ => from_raw_json_value(&json).map(Self::_Custom),
        }
    }
}

/// The payload for an unsupported join rule.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CustomJoinRule {
    /// The kind of join rule.
    join_rule: String,

    /// The remaining data.
    #[serde(flatten)]
    data: JsonObject,
}

/// Configuration of the `Restricted` join rule.
#[derive(ToSchema, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restricted {
    /// Allow rules which describe conditions that allow joining a room.
    #[serde(default, deserialize_with = "crate::serde::ignore_invalid_vec_items")]
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
        D: de::Deserializer<'de>,
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

/// The kind of rule used for users wishing to join this room.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(Clone, Default, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
pub enum JoinRuleKind {
    /// A user who wishes to join the room must first receive an invite to the room from someone
    /// already inside of the room.
    Invite,

    /// Users can join the room if they are invited, or they can request an invite to the room.
    ///
    /// They can be allowed (invited) or denied (kicked/banned) access.
    Knock,

    /// Reserved but not yet implemented by the Matrix specification.
    Private,

    /// Users can join the room if they are invited, or if they meet any of the conditions
    /// described in a set of rules.
    Restricted,

    /// Users can join the room if they are invited, or if they meet any of the conditions
    /// described in a set of rules, or they can request an invite to the room.
    KnockRestricted,

    /// Anyone can join the room without any prior action.
    #[default]
    Public,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

impl From<JoinRuleKind> for JoinRuleSummary {
    fn from(value: JoinRuleKind) -> Self {
        match value {
            JoinRuleKind::Invite => Self::Invite,
            JoinRuleKind::Knock => Self::Knock,
            JoinRuleKind::Private => Self::Private,
            JoinRuleKind::Restricted => Self::Restricted(Default::default()),
            JoinRuleKind::KnockRestricted => Self::KnockRestricted(Default::default()),
            JoinRuleKind::Public => Self::Public,
            JoinRuleKind::_Custom(s) => Self::_Custom(s),
        }
    }
}

/// The summary of a room's state.
#[derive(Debug, Clone, Serialize)]
pub struct RoomSummary {
    /// The ID of the room.
    pub room_id: OwnedRoomId,

    /// The canonical alias of the room, if any.
    ///
    /// If the `compat-empty-string-null` cargo feature is enabled, this field being an empty
    /// string in JSON will result in `None` here during deserialization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_alias: Option<OwnedRoomAliasId>,

    /// The name of the room, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The topic of the room, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,

    /// The URL for the room's avatar, if one is set.
    ///
    /// If you activate the `compat-empty-string-null` feature, this field being an empty string in
    /// JSON will result in `None` here during deserialization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<OwnedMxcUri>,

    /// The type of room from `m.room.create`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_type: Option<RoomType>,

    /// The number of members joined to the room.
    pub num_joined_members: u64,

    /// The join rule of the room.
    #[serde(flatten, skip_serializing_if = "crate::serde::is_default")]
    pub join_rule: JoinRuleSummary,

    /// Whether the room may be viewed by users without joining.
    pub world_readable: bool,

    /// Whether guest users may join the room and participate in it.
    ///
    /// If they can, they will be subject to ordinary power level rules like any other user.
    pub guest_can_join: bool,

    /// If the room is encrypted, the algorithm used for this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EventEncryptionAlgorithm>,

    /// The version of the room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_version: Option<RoomVersionId>,
}

impl RoomSummary {
    /// Construct a new `RoomSummary` with the given required fields.
    pub fn new(
        room_id: OwnedRoomId,
        join_rule: JoinRuleSummary,
        guest_can_join: bool,
        num_joined_members: u64,
        world_readable: bool,
    ) -> Self {
        Self {
            room_id,
            canonical_alias: None,
            name: None,
            topic: None,
            avatar_url: None,
            room_type: None,
            num_joined_members,
            join_rule,
            world_readable,
            guest_can_join,
            encryption: None,
            room_version: None,
        }
    }
}

impl<'de> Deserialize<'de> for RoomSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        /// Helper type to deserialize [`RoomSummary`] because using `flatten` on `join_rule`
        /// returns an error.
        #[derive(Deserialize)]
        struct RoomSummaryDeHelper {
            room_id: OwnedRoomId,
            #[cfg_attr(
                feature = "compat-empty-string-null",
                serde(default, deserialize_with = "crate::serde::empty_string_as_none")
            )]
            canonical_alias: Option<OwnedRoomAliasId>,
            name: Option<String>,
            topic: Option<String>,
            #[cfg_attr(
                feature = "compat-empty-string-null",
                serde(default, deserialize_with = "crate::serde::empty_string_as_none")
            )]
            avatar_url: Option<OwnedMxcUri>,
            room_type: Option<RoomType>,
            num_joined_members: u64,
            world_readable: bool,
            guest_can_join: bool,
            encryption: Option<EventEncryptionAlgorithm>,
            room_version: Option<RoomVersionId>,
        }

        let json = Box::<RawJsonValue>::deserialize(deserializer)?;
        let RoomSummaryDeHelper {
            room_id,
            canonical_alias,
            name,
            topic,
            avatar_url,
            room_type,
            num_joined_members,
            world_readable,
            guest_can_join,
            encryption,
            room_version,
        } = from_raw_json_value(&json)?;
        let join_rule: JoinRuleSummary = from_raw_json_value(&json)?;

        Ok(Self {
            room_id,
            canonical_alias,
            name,
            topic,
            avatar_url,
            room_type,
            num_joined_members,
            join_rule,
            world_readable,
            guest_can_join,
            encryption,
            room_version,
        })
    }
}

/// The rule used for users wishing to join a room.
///
/// In contrast to the regular [`JoinRule`], this enum holds only simplified conditions for joining
/// restricted rooms.
///
/// This type can hold an arbitrary join rule. To check for values that are not available as a
/// documented variant here, get its kind with `.kind()` or use its string representation, obtained
/// through `.as_str()`.
///
/// Because this type contains a few neighbouring fields instead of a whole object, and it is not
/// possible to know which fields to parse for unknown variants, this type will fail to serialize if
/// it doesn't match one of the documented variants. It is only possible to construct an
/// undocumented variant by deserializing it, so do not re-serialize this type.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "join_rule", rename_all = "snake_case")]
pub enum JoinRuleSummary {
    /// A user who wishes to join the room must first receive an invite to the room from someone
    /// already inside of the room.
    Invite,

    /// Users can join the room if they are invited, or they can request an invite to the room.
    ///
    /// They can be allowed (invited) or denied (kicked/banned) access.
    Knock,

    /// Reserved but not yet implemented by the Matrix specification.
    Private,

    /// Users can join the room if they are invited, or if they meet one of the conditions
    /// described in the [`RestrictedSummary`].
    Restricted(RestrictedSummary),

    /// Users can join the room if they are invited, or if they meet one of the conditions
    /// described in the [`RestrictedSummary`], or they can request an invite to the room.
    KnockRestricted(RestrictedSummary),

    /// Anyone can join the room without any prior action.
    #[default]
    Public,

    #[doc(hidden)]
    #[serde(skip_serializing)]
    _Custom(PrivOwnedStr),
}

impl JoinRuleSummary {
    /// Returns the kind of this `JoinRuleSummary`.
    pub fn kind(&self) -> JoinRuleKind {
        match self {
            Self::Invite => JoinRuleKind::Invite,
            Self::Knock => JoinRuleKind::Knock,
            Self::Private => JoinRuleKind::Private,
            Self::Restricted(_) => JoinRuleKind::Restricted,
            Self::KnockRestricted(_) => JoinRuleKind::KnockRestricted,
            Self::Public => JoinRuleKind::Public,
            Self::_Custom(rule) => JoinRuleKind::_Custom(rule.clone()),
        }
    }

    /// Returns the string name of this `JoinRuleSummary`.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Invite => "invite",
            Self::Knock => "knock",
            Self::Private => "private",
            Self::Restricted(_) => "restricted",
            Self::KnockRestricted(_) => "knock_restricted",
            Self::Public => "public",
            Self::_Custom(rule) => &rule.0,
        }
    }
}

impl From<JoinRule> for JoinRuleSummary {
    fn from(value: JoinRule) -> Self {
        match value {
            JoinRule::Invite => Self::Invite,
            JoinRule::Knock => Self::Knock,
            JoinRule::Private => Self::Private,
            JoinRule::Restricted(restricted) => Self::Restricted(restricted.into()),
            JoinRule::KnockRestricted(restricted) => Self::KnockRestricted(restricted.into()),
            JoinRule::Public => Self::Public,
            JoinRule::_Custom(CustomJoinRule { join_rule, .. }) => {
                Self::_Custom(PrivOwnedStr(join_rule.into()))
            }
        }
    }
}

impl<'de> Deserialize<'de> for JoinRuleSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let json: Box<RawJsonValue> = Box::deserialize(deserializer)?;

        #[derive(Deserialize)]
        struct ExtractType<'a> {
            #[serde(borrow)]
            join_rule: Option<Cow<'a, str>>,
        }

        let Some(join_rule) = serde_json::from_str::<ExtractType<'_>>(json.get())
            .map_err(de::Error::custom)?
            .join_rule
        else {
            return Ok(Self::default());
        };

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

/// A summary of the conditions for joining a restricted room.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestrictedSummary {
    /// The room IDs which are specified by the join rules.
    #[serde(default)]
    pub allowed_room_ids: Vec<OwnedRoomId>,
}

impl RestrictedSummary {
    /// Constructs a new `RestrictedSummary` with the given room IDs.
    pub fn new(allowed_room_ids: Vec<OwnedRoomId>) -> Self {
        Self { allowed_room_ids }
    }
}

impl From<Restricted> for RestrictedSummary {
    fn from(value: Restricted) -> Self {
        let allowed_room_ids = value
            .allow
            .into_iter()
            .filter_map(|allow_rule| {
                let membership = as_variant!(allow_rule, AllowRule::RoomMembership)?;
                Some(membership.room_id)
            })
            .collect();

        Self::new(allowed_room_ids)
    }
}

/// Request type for the `get_event_by_timestamp` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct TimestampToEventReqArgs {
    /// The ID of the room the event is in.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The timestamp to search from, inclusively.
    #[salvo(parameter(parameter_in = Query))]
    pub ts: UnixMillis,

    /// The direction in which to search.
    #[salvo(parameter(parameter_in = Query))]
    pub dir: Direction,
}

/// Response type for the `get_event_by_timestamp` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Debug)]
pub struct TimestampToEventResBody {
    /// The ID of the event found.
    pub event_id: OwnedEventId,

    /// The event's timestamp.
    pub origin_server_ts: UnixMillis,
}

impl TimestampToEventResBody {
    /// Creates a new `Response` with the given event ID and timestamp.
    pub fn new(event_id: OwnedEventId, origin_server_ts: UnixMillis) -> Self {
        Self {
            event_id,
            origin_server_ts,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use assert_matches2::assert_matches;
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::{
        AllowRule, CustomAllowRule, JoinRule, JoinRuleSummary, Restricted, RestrictedSummary,
        RoomMembership, RoomSummary,
    };
    use crate::{OwnedRoomId, owned_room_id};

    #[test]
    fn deserialize_summary_no_join_rule() {
        let json = json!({
            "room_id": "!room:localhost",
            "num_joined_members": 5,
            "world_readable": false,
            "guest_can_join": false,
        });

        let summary: RoomSummary = from_json_value(json).unwrap();
        assert_eq!(summary.room_id, "!room:localhost");
        assert_eq!(summary.num_joined_members, 5);
        assert!(!summary.world_readable);
        assert!(!summary.guest_can_join);
        assert_matches!(summary.join_rule, JoinRuleSummary::Public);
    }

    #[test]
    fn deserialize_summary_private_join_rule() {
        let json = json!({
            "room_id": "!room:localhost",
            "num_joined_members": 5,
            "world_readable": false,
            "guest_can_join": false,
            "join_rule": "private",
        });

        let summary: RoomSummary = from_json_value(json).unwrap();
        assert_eq!(summary.room_id, "!room:localhost");
        assert_eq!(summary.num_joined_members, 5);
        assert!(!summary.world_readable);
        assert!(!summary.guest_can_join);
        assert_matches!(summary.join_rule, JoinRuleSummary::Private);
    }

    #[test]
    fn deserialize_summary_restricted_join_rule() {
        let json = json!({
            "room_id": "!room:localhost",
            "num_joined_members": 5,
            "world_readable": false,
            "guest_can_join": false,
            "join_rule": "restricted",
            "allowed_room_ids": ["!otherroom:localhost"],
        });

        let summary: RoomSummary = from_json_value(json).unwrap();
        assert_eq!(summary.room_id, "!room:localhost");
        assert_eq!(summary.num_joined_members, 5);
        assert!(!summary.world_readable);
        assert!(!summary.guest_can_join);
        assert_matches!(summary.join_rule, JoinRuleSummary::Restricted(restricted));
        assert_eq!(restricted.allowed_room_ids.len(), 1);
    }

    #[test]
    fn deserialize_summary_restricted_join_rule_no_allowed_room_ids() {
        let json = json!({
            "room_id": "!room:localhost",
            "num_joined_members": 5,
            "world_readable": false,
            "guest_can_join": false,
            "join_rule": "restricted",
        });

        let summary: RoomSummary = from_json_value(json).unwrap();
        assert_eq!(summary.room_id, "!room:localhost");
        assert_eq!(summary.num_joined_members, 5);
        assert!(!summary.world_readable);
        assert!(!summary.guest_can_join);
        assert_matches!(summary.join_rule, JoinRuleSummary::Restricted(restricted));
        assert_eq!(restricted.allowed_room_ids.len(), 0);
    }

    #[test]
    fn serialize_summary_knock_join_rule() {
        let summary = RoomSummary::new(
            owned_room_id!("!room:localhost"),
            JoinRuleSummary::Knock,
            false,
            5,
            false,
        );

        assert_eq!(
            to_json_value(&summary).unwrap(),
            json!({
                "room_id": "!room:localhost",
                "num_joined_members": 5,
                "world_readable": false,
                "guest_can_join": false,
                "join_rule": "knock",
            })
        );
    }

    #[test]
    fn serialize_summary_restricted_join_rule() {
        let summary = RoomSummary::new(
            owned_room_id!("!room:localhost"),
            JoinRuleSummary::Restricted(RestrictedSummary::new(vec![owned_room_id!(
                "!otherroom:localhost"
            )])),
            false,
            5,
            false,
        );

        assert_eq!(
            to_json_value(&summary).unwrap(),
            json!({
                "room_id": "!room:localhost",
                "num_joined_members": 5,
                "world_readable": false,
                "guest_can_join": false,
                "join_rule": "restricted",
                "allowed_room_ids": ["!otherroom:localhost"],
            })
        );
    }

    #[test]
    fn join_rule_to_join_rule_summary() {
        assert_eq!(JoinRuleSummary::Invite, JoinRule::Invite.into());
        assert_eq!(JoinRuleSummary::Knock, JoinRule::Knock.into());
        assert_eq!(JoinRuleSummary::Public, JoinRule::Public.into());
        assert_eq!(JoinRuleSummary::Private, JoinRule::Private.into());

        assert_matches!(
            JoinRule::KnockRestricted(Restricted::default()).into(),
            JoinRuleSummary::KnockRestricted(restricted)
        );
        assert_eq!(restricted.allowed_room_ids, &[] as &[OwnedRoomId]);

        let room_id = owned_room_id!("!room:localhost");
        assert_matches!(
            JoinRule::Restricted(Restricted::new(vec![AllowRule::RoomMembership(
                RoomMembership::new(room_id.clone())
            )]))
            .into(),
            JoinRuleSummary::Restricted(restricted)
        );
        assert_eq!(restricted.allowed_room_ids, [room_id]);
    }

    #[test]
    fn roundtrip_custom_allow_rule() {
        let json = r#"{"type":"org.msc9000.something","foo":"bar"}"#;
        let allow_rule: AllowRule = serde_json::from_str(json).unwrap();
        assert_matches!(&allow_rule, AllowRule::_Custom(_));
        assert_eq!(serde_json::to_string(&allow_rule).unwrap(), json);
    }

    #[test]
    fn invalid_allow_items() {
        let json = r#"{
            "join_rule": "restricted",
            "allow": [
                {
                    "type": "m.room_membership",
                    "room_id": "!mods:example.org"
                },
                {
                    "type": "m.room_membership",
                    "room_id": ""
                },
                {
                    "type": "m.room_membership",
                    "room_id": "not a room id"
                },
                {
                    "type": "org.example.custom",
                    "org.example.minimum_role": "developer"
                },
                {
                    "not even close": "to being correct",
                    "any object": "passes this test",
                    "only non-objects in this array": "cause deserialization to fail"
                }
            ]
        }"#;
        let join_rule: JoinRule = serde_json::from_str(json).unwrap();

        assert_matches!(join_rule, JoinRule::Restricted(restricted));
        assert_eq!(
            restricted.allow,
            &[
                AllowRule::room_membership(owned_room_id!("!mods:example.org")),
                AllowRule::_Custom(Box::new(CustomAllowRule {
                    rule_type: "org.example.custom".into(),
                    extra: BTreeMap::from([(
                        "org.example.minimum_role".into(),
                        "developer".into()
                    )])
                }))
            ]
        );
    }
}
