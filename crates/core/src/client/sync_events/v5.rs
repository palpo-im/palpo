//! `POST /_matrix/client/unstable/org.matrix.msc3575/sync` ([MSC])
//!
//! Get all new events in a sliding window of rooms since the last sync or a
//! given point in time.
//!
//! [MSC]: https://github.com/matrix-org/matrix-doc/blob/kegan/sync-v3/proposals/3575-sync.md

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use salvo::prelude::*;
use serde::{Deserialize, Serialize, de::Error as _};

use super::UnreadNotificationsCount;
use crate::device::DeviceLists;
use crate::events::receipt::SyncReceiptEvent;
use crate::events::typing::SyncTypingEvent;
use crate::events::{AnyGlobalAccountDataEvent, AnyRoomAccountDataEvent, AnyToDeviceEvent};
use crate::{
    OwnedMxcUri, Seqnum, UnixMillis,
    directory::RoomTypeFilter,
    events::{AnyStrippedStateEvent, AnySyncStateEvent, AnySyncTimelineEvent, StateEventType},
    identifiers::*,
    serde::{RawJson, deserialize_cow_str, duration::opt_ms},
    state::TypeStateKey,
};

pub type SyncInfo<'a> = (&'a UserId, &'a DeviceId, Seqnum, &'a SyncEventsReqBody);

pub type KnownRooms = BTreeMap<String, BTreeMap<OwnedRoomId, Seqnum>>;
pub type TodoRooms = BTreeMap<OwnedRoomId, (BTreeSet<TypeStateKey>, usize, Seqnum)>;

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//        unstable =>
// "/_matrix/client/unstable/org.matrix.simplified_msc3575/sync",        // 1.4
// => "/_matrix/client/v5/sync",     }
// };

#[derive(ToParameters, Deserialize, Debug)]
pub struct SyncEventsReqArgs {
    /// A point in time to continue a sync from.
    ///
    /// Should be a token from the `pos` field of a previous `/sync`
    /// response.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<String>,

    /// The maximum time to poll before responding to this request.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(with = "opt_ms", default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Duration>,
}
/// Request type for the `sync` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct SyncEventsReqBody {
    /// A unique string identifier for this connection to the server.
    ///
    /// Optional. If this is missing, only one sliding sync connection can be
    /// made to the server at any one time. Clients need to set this to
    /// allow more than one connection concurrently, so the server can
    /// distinguish between connections. This is NOT STICKY and must be
    /// provided with every request, if your client needs more than one
    /// concurrent connection.
    ///
    /// Limitation: it must not contain more than 16 chars, due to it being
    /// required with every request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<String>,

    /// Allows clients to know what request params reached the server,
    /// functionally similar to txn IDs on /send for events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txn_id: Option<String>,

    /// The list configurations of rooms we are interested in mapped by
    /// name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lists: BTreeMap<String, ReqList>,

    /// Specific rooms and event types that we want to receive events from.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub room_subscriptions: BTreeMap<OwnedRoomId, RoomSubscription>,

    /// Specific rooms we no longer want to receive events from.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub unsubscribe_rooms: Vec<OwnedRoomId>,

    /// Extensions API.
    #[serde(default, skip_serializing_if = "ExtensionsConfig::is_empty")]
    pub extensions: ExtensionsConfig,
}

/// Response type for the `sync` endpoint.
#[derive(ToSchema, Serialize, Default, Debug)]
pub struct SyncEventsResBody {
    /// Matches the `txn_id` sent by the request. Please see
    /// [`Request::txn_id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txn_id: Option<String>,

    /// The token to supply in the `pos` param of the next `/sync` request.
    pub pos: String,

    /// Updates on the order of rooms, mapped by the names we asked for.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lists: BTreeMap<String, SyncList>,

    /// The updates on rooms.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rooms: BTreeMap<OwnedRoomId, SyncRoom>,

    /// Extensions API.
    #[serde(default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}
impl SyncEventsResBody {
    /// Creates a new `Response` with the given pos.
    pub fn new(pos: String) -> Self {
        Self {
            pos,
            ..Default::default()
        }
    }
}
/// A sliding sync response updates to joiend rooms (see
/// [`super::Response::lists`]).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SyncList {
    /// The total number of rooms found for this list.
    pub count: usize,
}

/// A slising sync response updated room (see [`super::Response::rooms`]).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SyncRoom {
    /// The name as calculated by the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The avatar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<OwnedMxcUri>,

    /// Whether it is an initial response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial: Option<bool>,

    /// Whether it is a direct room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_dm: Option<bool>,

    /// If this is `Some(_)`, this is a not-yet-accepted invite containing
    /// the given stripped state events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_state: Option<Vec<RawJson<AnyStrippedStateEvent>>>,

    /// Number of unread notifications.
    #[serde(flatten, default, skip_serializing_if = "UnreadNotificationsCount::is_empty")]
    pub unread_notifications: UnreadNotificationsCount,

    /// Message-like events and live state events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<RawJson<AnySyncTimelineEvent>>,

    /// State events as configured by the request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_state: Vec<RawJson<AnySyncStateEvent>>,

    /// The `prev_batch` allowing you to paginate through the messages
    /// before the given ones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_batch: Option<String>,

    /// True if the number of events returned was limited by the limit on
    /// the filter.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub limited: bool,

    /// The number of users with membership of `join`, including the
    /// client’s own user ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joined_count: Option<i64>,

    /// The number of users with membership of `invite`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invited_count: Option<i64>,

    /// The number of timeline events which have just occurred and are not
    /// historical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_live: Option<i64>,

    /// The bump stamp of the room.
    ///
    /// It can be interpreted as a “recency stamp” or “streaming order
    /// index”. For example, consider `roomA` with `bump_stamp = 2`, `roomB`
    /// with `bump_stamp = 1` and `roomC` with `bump_stamp = 0`. If `roomC`
    /// receives an update, its `bump_stamp` will be 3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bump_stamp: Option<i64>,

    /// Heroes of the room, if requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heroes: Option<Vec<SyncRoomHero>>,
}

/// Filter for a sliding sync list, set at request.
///
/// All fields are applied with AND operators, hence if `is_dm`  is `true` and
/// `is_encrypted` is `true` then only encrypted DM rooms will be returned. The
/// absence of fields implies no filter on that criteria: it does NOT imply
/// `false`.
///
/// Filters are considered _sticky_, meaning that the filter only has to be
/// provided once and their parameters 'sticks' for future requests until a new
/// filter overwrites them.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReqListFilters {
    /// Whether to return invited Rooms, only joined rooms or both.
    ///
    /// Flag which only returns rooms the user is currently invited to. If
    /// unset, both invited and joined rooms are returned. If false, no
    /// invited rooms are returned. If true, only invited rooms are
    /// returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_invite: Option<bool>,

    /// Only list rooms that are not of these create-types, or all.
    ///
    /// Same as "room_types" but inverted. This can be used to filter out spaces
    /// from the room list.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub not_room_types: Vec<RoomTypeFilter>,
}

/// Sliding Sync Request for each list.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReqList {
    /// The ranges of rooms we're interested in.
    pub ranges: Vec<(usize, usize)>,

    /// The details to be included per room
    #[serde(flatten)]
    pub room_details: RoomDetailsConfig,

    /// Request a stripped variant of membership events for the users used
    /// to calculate the room name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_heroes: Option<bool>,

    /// Filters to apply to the list before sorting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<ReqListFilters>,
}

/// Configuration for requesting room details.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoomDetailsConfig {
    /// Required state for each room returned. An array of event type and state
    /// key tuples.
    ///
    /// Note that elements of this array are NOT sticky so they must be
    /// specified in full when they are changed. Sticky.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_state: Vec<(StateEventType, String)>,

    /// The maximum number of timeline events to return per room. Sticky.
    pub timeline_limit: usize,
}

/// Configuration for old rooms to include
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct IncludeOldRooms {
    /// Required state for each room returned. An array of event type and state
    /// key tuples.
    ///
    /// Note that elements of this array are NOT sticky so they must be
    /// specified in full when they are changed. Sticky.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_state: Vec<(StateEventType, String)>,

    /// The maximum number of timeline events to return per room. Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline_limit: Option<usize>,
}

/// Sliding sync request room subscription (see
/// [`super::Request::room_subscriptions`]).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoomSubscription {
    /// Required state for each returned room. An array of event type and
    /// state key tuples.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_state: Vec<(StateEventType, String)>,

    /// The maximum number of timeline events to return per room.
    pub timeline_limit: i64,

    /// Include the room heroes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_heroes: Option<bool>,
}

/// Single entry for a room-related read receipt configuration in
/// [`Receipts`].
#[derive(Clone, Debug, PartialEq)]
pub enum ReceiptsRoom {
    /// Get read receipts for all the subscribed rooms.
    AllSubscribed,

    /// Get read receipts for this particular room.
    Room(OwnedRoomId),
}

impl Serialize for ReceiptsRoom {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::AllSubscribed => serializer.serialize_str("*"),
            Self::Room(r) => r.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ReceiptsRoom {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        match deserialize_cow_str(deserializer)?.as_ref() {
            "*" => Ok(Self::AllSubscribed),
            other => Ok(Self::Room(RoomId::parse(other).map_err(D::Error::custom)?.to_owned())),
        }
    }
}

/// A sliding sync room hero.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct SyncRoomHero {
    /// The user ID of the hero.
    pub user_id: OwnedUserId,

    /// The name of the hero.
    #[serde(rename = "displayname", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The avatar of the hero.
    #[serde(rename = "avatar_url", skip_serializing_if = "Option::is_none")]
    pub avatar: Option<OwnedMxcUri>,
}

impl SyncRoomHero {
    /// Creates a new `SyncRoomHero` with the given user id.
    pub fn new(user_id: OwnedUserId) -> Self {
        Self {
            user_id,
            name: None,
            avatar: None,
        }
    }
}

/// Sliding-Sync extension configuration.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ExtensionsConfig {
    /// Request to devices messages with the given config.
    #[serde(default, skip_serializing_if = "ToDeviceConfig::is_empty")]
    pub to_device: ToDeviceConfig,

    /// Configure the end-to-end-encryption extension.
    #[serde(default, skip_serializing_if = "E2eeConfig::is_empty")]
    pub e2ee: E2eeConfig,

    /// Configure the account data extension.
    #[serde(default, skip_serializing_if = "AccountDataConfig::is_empty")]
    pub account_data: AccountDataConfig,

    /// Request to receipt information with the given config.
    #[serde(default, skip_serializing_if = "ReceiptsConfig::is_empty")]
    pub receipts: ReceiptsConfig,

    /// Request to typing information with the given config.
    #[serde(default, skip_serializing_if = "TypingConfig::is_empty")]
    pub typing: TypingConfig,

    /// Extensions may add further fields to the list.
    #[serde(flatten)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    other: BTreeMap<String, serde_json::Value>,
}

impl ExtensionsConfig {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.to_device.is_empty()
            && self.e2ee.is_empty()
            && self.account_data.is_empty()
            && self.receipts.is_empty()
            && self.typing.is_empty()
            && self.other.is_empty()
    }
}

/// Extensions specific response data.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct Extensions {
    /// To-device extension in response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_device: Option<ToDevice>,

    /// E2ee extension in response.
    #[serde(default, skip_serializing_if = "E2ee::is_empty")]
    pub e2ee: E2ee,

    /// Account data extension in response.
    #[serde(default, skip_serializing_if = "AccountData::is_empty")]
    pub account_data: AccountData,

    /// Receipt data extension in response.
    #[serde(default, skip_serializing_if = "Receipts::is_empty")]
    pub receipts: Receipts,

    /// Typing data extension in response.
    #[serde(default, skip_serializing_if = "Typing::is_empty")]
    pub typing: Typing,
}

impl Extensions {
    /// Whether the extension data is empty.
    ///
    /// True if neither to-device, e2ee nor account data are to be found.
    pub fn is_empty(&self) -> bool {
        self.to_device.is_none()
            && self.e2ee.is_empty()
            && self.account_data.is_empty()
            && self.receipts.is_empty()
            && self.typing.is_empty()
    }
}

/// To-device messages extension configuration.
///
/// According to [MSC3885](https://github.com/matrix-org/matrix-spec-proposals/pull/3885).
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ToDeviceConfig {
    /// Activate or deactivate this extension. Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// Max number of to-device messages per response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,

    /// Give messages since this token only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,

    /// List of list names for which to-device events should be enabled.
    ///
    /// If not defined, will be enabled for *all* the lists appearing in the
    /// request. If defined and empty, will be disabled for all the lists.
    ///
    /// Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lists: Option<Vec<String>>,

    /// List of room names for which to-device events should be enabled.
    ///
    /// If not defined, will be enabled for *all* the rooms appearing in the
    /// `room_subscriptions`. If defined and empty, will be disabled for all
    /// the rooms.
    ///
    /// Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rooms: Option<Vec<OwnedRoomId>>,
}

impl ToDeviceConfig {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.limit.is_none() && self.since.is_none()
    }
}

/// To-device messages extension response.
///
/// According to [MSC3885](https://github.com/matrix-org/matrix-spec-proposals/pull/3885).
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToDevice {
    /// Fetch the next batch from this entry.
    pub next_batch: String,

    /// The to-device Events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<RawJson<AnyToDeviceEvent>>,
}

/// E2ee extension configuration.
///
/// According to [MSC3884](https://github.com/matrix-org/matrix-spec-proposals/pull/3884).
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct E2eeConfig {
    /// Activate or deactivate this extension. Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl E2eeConfig {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
    }
}

/// E2ee extension response data.
///
/// According to [MSC3884](https://github.com/matrix-org/matrix-spec-proposals/pull/3884).
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct E2ee {
    /// Information on E2ee device updates.
    ///
    /// Only present on an incremental sync.
    #[serde(default, skip_serializing_if = "DeviceLists::is_empty")]
    pub device_lists: DeviceLists,

    /// For each key algorithm, the number of unclaimed one-time keys
    /// currently held on the server for a device.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub device_one_time_keys_count: BTreeMap<DeviceKeyAlgorithm, u64>,

    /// For each key algorithm, the number of unclaimed one-time keys
    /// currently held on the server for a device.
    ///
    /// The presence of this field indicates that the server supports
    /// fallback keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_unused_fallback_key_types: Option<Vec<DeviceKeyAlgorithm>>,
}

impl E2ee {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.device_lists.is_empty()
            && self.device_one_time_keys_count.is_empty()
            && self.device_unused_fallback_key_types.is_none()
    }
}

/// Account-data extension configuration.
///
/// Not yet part of the spec proposal. Taken from the reference implementation
/// <https://github.com/matrix-org/sliding-sync/blob/main/sync3/extensions/account_data.go>
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AccountDataConfig {
    /// Activate or deactivate this extension. Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// List of list names for which account data should be enabled.
    ///
    /// This is specific to room account data (e.g. user-defined room tags).
    ///
    /// If not defined, will be enabled for *all* the lists appearing in the
    /// request. If defined and empty, will be disabled for all the lists.
    ///
    /// Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lists: Option<Vec<String>>,

    /// List of room names for which account data should be enabled.
    ///
    /// This is specific to room account data (e.g. user-defined room tags).
    ///
    /// If not defined, will be enabled for *all* the rooms appearing in the
    /// `room_subscriptions`. If defined and empty, will be disabled for all
    /// the rooms.
    ///
    /// Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rooms: Option<Vec<OwnedRoomId>>,
}

impl AccountDataConfig {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
    }
}

/// Account-data extension response data.
///
/// Not yet part of the spec proposal. Taken from the reference implementation
/// <https://github.com/matrix-org/sliding-sync/blob/main/sync3/extensions/account_data.go>
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountData {
    /// The global private data created by this user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global: Vec<RawJson<AnyGlobalAccountDataEvent>>,

    /// The private data that this user has attached to each room.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rooms: BTreeMap<OwnedRoomId, Vec<RawJson<AnyRoomAccountDataEvent>>>,
}

impl AccountData {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.global.is_empty() && self.rooms.is_empty()
    }
}

/// Receipt extension configuration.
///
/// According to [MSC3960](https://github.com/matrix-org/matrix-spec-proposals/pull/3960)
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ReceiptsConfig {
    /// Activate or deactivate this extension. Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// List of list names for which receipts should be enabled.
    ///
    /// If not defined, will be enabled for *all* the lists appearing in the
    /// request. If defined and empty, will be disabled for all the lists.
    ///
    /// Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lists: Option<Vec<String>>,

    /// List of room names for which receipts should be enabled.
    ///
    /// If not defined, will be enabled for *all* the rooms appearing in the
    /// `room_subscriptions`. If defined and empty, will be disabled for all
    /// the rooms.
    ///
    /// Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rooms: Option<Vec<RoomReceiptConfig>>,
}

impl ReceiptsConfig {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
    }
}

/// Single entry for a room-related read receipt configuration in
/// `ReceiptsConfig`.
#[derive(ToSchema, Clone, Debug, PartialEq)]
pub enum RoomReceiptConfig {
    /// Get read receipts for all the subscribed rooms.
    AllSubscribed,
    /// Get read receipts for this particular room.
    Room(OwnedRoomId),
}

impl Serialize for RoomReceiptConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            RoomReceiptConfig::AllSubscribed => serializer.serialize_str("*"),
            RoomReceiptConfig::Room(r) => r.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for RoomReceiptConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        match deserialize_cow_str(deserializer)?.as_ref() {
            "*" => Ok(RoomReceiptConfig::AllSubscribed),
            other => Ok(RoomReceiptConfig::Room(
                RoomId::parse(other).map_err(D::Error::custom)?.to_owned(),
            )),
        }
    }
}

/// Receipt extension response data.
///
/// According to [MSC3960](https://github.com/matrix-org/matrix-spec-proposals/pull/3960)
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct Receipts {
    /// The ephemeral receipt room event for each room
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub rooms: BTreeMap<OwnedRoomId, SyncReceiptEvent>,
}

impl Receipts {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }
}

/// Typing extension configuration.
///
/// Not yet part of the spec proposal. Taken from the reference implementation
/// <https://github.com/matrix-org/sliding-sync/blob/main/sync3/extensions/typing.go>
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TypingConfig {
    /// Activate or deactivate this extension. Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// List of list names for which typing notifications should be enabled.
    ///
    /// If not defined, will be enabled for *all* the lists appearing in the
    /// request. If defined and empty, will be disabled for all the lists.
    ///
    /// Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lists: Option<Vec<String>>,

    /// List of room names for which typing notifications should be enabled.
    ///
    /// If not defined, will be enabled for *all* the rooms appearing in the
    /// `room_subscriptions`. If defined and empty, will be disabled for all
    /// the rooms.
    ///
    /// Sticky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rooms: Option<Vec<OwnedRoomId>>,
}

impl TypingConfig {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
    }
}

/// Typing extension response data.
///
/// Not yet part of the spec proposal. Taken from the reference implementation
/// <https://github.com/matrix-org/sliding-sync/blob/main/sync3/extensions/typing.go>
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct Typing {
    /// The ephemeral typing event for each room
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub rooms: BTreeMap<OwnedRoomId, SyncTypingEvent>,
}

impl Typing {
    /// Whether all fields are empty or `None`.
    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }
}
