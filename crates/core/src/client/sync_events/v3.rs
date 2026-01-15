//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3sync

use std::collections::BTreeMap;
use std::time::Duration;

use as_variant::as_variant;
use salvo::oapi::{ToParameters, ToSchema};
use serde::{Deserialize, Serialize};

use super::UnreadNotificationsCount;
use crate::client::filter::FilterDefinition;
use crate::device::DeviceLists;
use crate::events::presence::PresenceEvent;
use crate::events::{
    AnyGlobalAccountDataEvent, AnyRoomAccountDataEvent, AnyStrippedStateEvent,
    AnySyncEphemeralRoomEvent, AnySyncStateEvent, AnySyncTimelineEvent, AnyToDeviceEvent,
};
use crate::presence::PresenceState;
use crate::serde::RawJson;
use crate::{DeviceKeyAlgorithm, OwnedEventId, OwnedRoomId};

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/sync",
//         1.1 => "/_matrix/client/v3/sync",
//     }
// };

/// Request type for the `sync` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct SyncEventsReqArgs {
    /// A filter represented either as its full JSON definition or the ID of a
    /// saved filter.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,

    /// A point in time to continue a sync from.
    ///
    /// Should be a token from the `next_batch` field of a previous `/sync`
    /// request.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,

    /// Controls whether to include the full state for all rooms the user is a
    /// member of.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub full_state: bool,

    /// Controls whether the client is automatically marked as online by polling
    /// this API.
    ///
    /// Defaults to `PresenceState::Online`.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub set_presence: PresenceState,

    /// The maximum time to poll in milliseconds before returning this request.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(
        with = "crate::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,

    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    #[salvo(parameter(parameter_in = Query))]
    pub use_state_after: bool,
}

/// Response type for the `sync` endpoint.
#[derive(ToSchema, Serialize, Clone, Debug)]
pub struct SyncEventsResBody {
    /// The batch token to supply in the `since` param of the next `/sync`
    /// request.
    pub next_batch: String,

    /// Updates to rooms.
    #[serde(default, skip_serializing_if = "Rooms::is_empty")]
    pub rooms: Rooms,

    /// Updates to the presence status of other users.
    #[serde(default, skip_serializing_if = "Presence::is_empty")]
    pub presence: Presence,

    /// The global private data created by this user.
    #[serde(default, skip_serializing_if = "GlobalAccountData::is_empty")]
    pub account_data: GlobalAccountData,

    /// Messages sent directly between devices.
    #[serde(default, skip_serializing_if = "ToDevice::is_empty")]
    pub to_device: ToDevice,

    /// Information on E2E device updates.
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
impl SyncEventsResBody {
    /// Creates a new `Response` with the given batch token.
    pub fn new(next_batch: String) -> Self {
        Self {
            next_batch,
            rooms: Default::default(),
            presence: Default::default(),
            account_data: Default::default(),
            to_device: Default::default(),
            device_lists: Default::default(),
            device_one_time_keys_count: BTreeMap::new(),
            device_unused_fallback_key_types: None,
        }
    }
}

/// A filter represented either as its full JSON definition or the ID of a saved
/// filter.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[allow(clippy::large_enum_variant)]
#[serde(untagged)]
pub enum Filter {
    // The filter definition needs to be (de)serialized twice because it is a URL-encoded JSON
    // string. Since only does the latter and this is a very uncommon
    // setup, we implement it through custom serde logic for this specific enum variant rather
    // than adding another palpo_api attribute.
    //
    // On the deserialization side, because this is an enum with #[serde(untagged)], serde
    // will try the variants in order (https://serde.rs/enum-representations.html). That means because
    // FilterDefinition is the first variant, JSON decoding is attempted first which is almost
    // functionally equivalent to looking at whether the first symbol is a '{' as the spec
    // says. (there are probably some corner cases like leading whitespace)
    /// A complete filter definition serialized to JSON.
    #[serde(with = "crate::serde::json_string")]
    FilterDefinition(FilterDefinition),

    /// The ID of a filter saved on the server.
    FilterId(String),
}

impl From<FilterDefinition> for Filter {
    fn from(def: FilterDefinition) -> Self {
        Self::FilterDefinition(def)
    }
}

impl From<String> for Filter {
    fn from(id: String) -> Self {
        Self::FilterId(id)
    }
}

/// Updates to rooms.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct Rooms {
    /// The rooms that the user has left or been banned from.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub leave: BTreeMap<OwnedRoomId, LeftRoom>,

    /// The rooms that the user has joined.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub join: BTreeMap<OwnedRoomId, JoinedRoom>,

    /// The rooms that the user has been invited to.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub invite: BTreeMap<OwnedRoomId, InvitedRoom>,

    /// The rooms that the user has knocked on.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub knock: BTreeMap<OwnedRoomId, KnockedRoom>,
}

impl Rooms {
    /// Creates an empty `Rooms`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there is no update in any room.
    pub fn is_empty(&self) -> bool {
        self.leave.is_empty()
            && self.join.is_empty()
            && self.invite.is_empty()
            && self.knock.is_empty()
    }
}

/// Historical updates to left rooms.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct LeftRoom {
    /// The timeline of messages and state changes in the room up to the point
    /// when the user left.
    #[serde(default)]
    pub timeline: Timeline,

    /// The state updates for the room up to the start of the timeline.
    #[serde(flatten, skip_serializing_if = "State::is_empty")]
    pub state: State,

    /// The private data that this user has attached to this room.
    #[serde(default, skip_serializing_if = "RoomAccountData::is_empty")]
    pub account_data: RoomAccountData,
}

impl LeftRoom {
    /// Creates an empty `LeftRoom`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are updates in the room.
    pub fn is_empty(&self) -> bool {
        self.timeline.is_empty() && self.state.is_empty() && self.account_data.is_empty()
    }
}

/// Updates to joined rooms.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct JoinedRoom {
    /// Information about the room which clients may need to correctly render it
    /// to users.
    #[serde(default, skip_serializing_if = "RoomSummary::is_empty")]
    pub summary: RoomSummary,

    /// Counts of [unread notifications] for this room.
    ///
    /// If `unread_thread_notifications` was set to `true` in the
    /// [`RoomEventFilter`], these include only the unread notifications for
    /// the main timeline.
    ///
    /// [unread notifications]: https://spec.matrix.org/latest/client-server-api/#receiving-notifications
    /// [`RoomEventFilter`]: crate::filter::RoomEventFilter
    #[serde(default, skip_serializing_if = "UnreadNotificationsCount::is_empty")]
    pub unread_notifications: UnreadNotificationsCount,

    /// Counts of [unread notifications] for threads in this room.
    ///
    /// This is a map from thread root ID to unread notifications in the thread.
    ///
    /// Only set if `unread_thread_notifications` was set to `true` in the
    /// [`RoomEventFilter`].
    ///
    /// [unread notifications]: https://spec.matrix.org/latest/client-server-api/#receiving-notifications
    /// [`RoomEventFilter`]: crate::filter::RoomEventFilter
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unread_thread_notifications: BTreeMap<OwnedEventId, UnreadNotificationsCount>,

    /// The timeline of messages and state changes in the room.
    #[serde(default, skip_serializing_if = "Timeline::is_empty")]
    pub timeline: Timeline,

    /// Updates to the state, between the time indicated by the `since`
    /// parameter, and the start of the `timeline` (or all state up to the
    /// start of the `timeline`, if `since` is not given, or `full_state` is
    /// true).
    #[serde(flatten, skip_serializing_if = "State::is_before_and_empty")]
    pub state: State,

    /// The private data that this user has attached to this room.
    #[serde(default, skip_serializing_if = "RoomAccountData::is_empty")]
    pub account_data: RoomAccountData,

    /// The ephemeral events in the room that aren't recorded in the timeline or
    /// state of the room.
    #[serde(default, skip_serializing_if = "Ephemeral::is_empty")]
    pub ephemeral: Ephemeral,

    /// The number of unread events since the latest read receipt.
    ///
    /// This uses the unstable prefix in [MSC2654].
    ///
    /// [MSC2654]: https://github.com/matrix-org/matrix-spec-proposals/pull/2654
    #[serde(
        rename = "org.matrix.msc2654.unread_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub unread_count: Option<u64>,
}

impl JoinedRoom {
    /// Creates an empty `JoinedRoom`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no updates in the room.
    pub fn is_empty(&self) -> bool {
        let is_empty = self.summary.is_empty()
            && self.unread_notifications.is_empty()
            && self.unread_thread_notifications.is_empty()
            && self.timeline.is_empty()
            && self.state.is_empty()
            && self.account_data.is_empty()
            && self.ephemeral.is_empty();

        #[cfg(not(feature = "unstable-msc2654"))]
        return is_empty;

        #[cfg(feature = "unstable-msc2654")]
        return is_empty && self.unread_count.is_none();
    }
}

/// Updates to knocked rooms.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct KnockedRoom {
    /// The knock state.
    pub knock_state: KnockState,
}

/// A mapping from a key `events` to a list of `StrippedStateEvent`.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct KnockState {
    /// The list of events.
    pub events: Vec<RawJson<AnyStrippedStateEvent>>,
}

/// Events in the room.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct Timeline {
    /// True if the number of events returned was limited by the `limit` on the
    /// filter.
    ///
    /// Default to `false`.
    #[serde(default)]
    pub limited: bool,

    /// A token that can be supplied to to the `from` parameter of the
    /// `/rooms/{room_id}/messages` endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_batch: Option<String>,

    /// A list of events.
    #[serde(default)]
    pub events: Vec<RawJson<AnySyncTimelineEvent>>,
}

impl Timeline {
    /// Creates an empty `Timeline`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no timeline updates.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// State events in the room.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub enum State {
    /// The state changes between the previous sync and the **start** of the timeline.
    ///
    /// To get the full list of state changes since the previous sync, the state events in
    /// [`Timeline`] must be added to these events to update the local state.
    ///
    /// To get this variant, `use_state_after` must be set to `false` in the [`Request`], which is
    /// the default.
    #[serde(rename = "state")]
    Before(StateEvents),

    /// The state changes between the previous sync and the **end** of the timeline.
    ///
    /// This contains the full list of state changes since the previous sync. State events in
    /// [`Timeline`] must be ignored to update the local state.
    ///
    /// To get this variant, `use_state_after` must be set to `true` in the [`Request`].
    #[serde(rename = "state_after")]
    After(StateEvents),
}

impl State {
    /// Returns true if this is the `Before` variant and there are no state updates.
    fn is_before_and_empty(&self) -> bool {
        as_variant!(self, Self::Before).is_some_and(|state| state.is_empty())
    }

    /// Returns true if there are no state updates.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Before(state) => state.is_empty(),
            Self::After(state) => state.is_empty(),
        }
    }
}
impl Default for State {
    fn default() -> Self {
        Self::Before(Default::default())
    }
}

/// State events in the room.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct StateEvents {
    /// A list of state events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<RawJson<AnySyncStateEvent>>,
}

impl StateEvents {
    /// Creates an empty `State`.
    pub fn new(events: Vec<RawJson<AnySyncStateEvent>>) -> Self {
        Self { events }
    }

    /// Returns true if there are no state updates.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl From<Vec<RawJson<AnySyncStateEvent>>> for StateEvents {
    fn from(events: Vec<RawJson<AnySyncStateEvent>>) -> Self {
        Self::new(events)
    }
}

/// The global private data created by this user.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct GlobalAccountData {
    /// A list of events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<RawJson<AnyGlobalAccountDataEvent>>,
}

impl GlobalAccountData {
    /// Creates an empty `GlobalAccountData`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no global account data updates.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// The private data that this user has attached to this room.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct RoomAccountData {
    /// A list of events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<RawJson<AnyRoomAccountDataEvent>>,
}

impl RoomAccountData {
    /// Creates an empty `RoomAccountData`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no room account data updates.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Ephemeral events not recorded in the timeline or state of the room.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct Ephemeral {
    /// A list of events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<RawJson<AnySyncEphemeralRoomEvent>>,
}

impl Ephemeral {
    /// Creates an empty `Ephemeral`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no ephemeral event updates.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Information about room for rendering to clients.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct RoomSummary {
    /// Users which can be used to generate a room name if the room does not
    /// have one.
    ///
    /// Required if room name or canonical aliases are not set or empty.
    #[serde(rename = "m.heroes", default, skip_serializing_if = "Vec::is_empty")]
    pub heroes: Vec<String>,

    /// Number of users whose membership status is `join`.
    /// Required if field has changed since last sync; otherwise, it may be
    /// omitted.
    #[serde(
        default,
        rename = "m.joined_member_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub joined_member_count: Option<u64>,

    /// Number of users whose membership status is `invite`.
    /// Required if field has changed since last sync; otherwise, it may be
    /// omitted.
    #[serde(
        default,
        rename = "m.invited_member_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub invited_member_count: Option<u64>,
}

impl RoomSummary {
    /// Creates an empty `RoomSummary`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no room summary updates.
    pub fn is_empty(&self) -> bool {
        self.heroes.is_empty()
            && self.joined_member_count.is_none()
            && self.invited_member_count.is_none()
    }
}

/// Updates to the rooms that the user has been invited to.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct InvitedRoom {
    /// The state of a room that the user has been invited to.
    #[serde(default, skip_serializing_if = "InviteState::is_empty")]
    pub invite_state: InviteState,
}

impl InvitedRoom {
    /// Creates an empty `InvitedRoom`.
    pub fn new(invite_state: InviteState) -> Self {
        Self { invite_state }
    }

    /// Returns true if there are no updates to this room.
    pub fn is_empty(&self) -> bool {
        self.invite_state.is_empty()
    }
}

impl From<InviteState> for InvitedRoom {
    fn from(invite_state: InviteState) -> Self {
        InvitedRoom { invite_state }
    }
}

/// The state of a room that the user has been invited to.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct InviteState {
    /// A list of state events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<RawJson<AnyStrippedStateEvent>>,
}

impl InviteState {
    /// Creates an empty `InviteState`.
    pub fn new(events: Vec<RawJson<AnyStrippedStateEvent>>) -> Self {
        Self { events }
    }

    /// Returns true if there are no state updates.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl From<Vec<RawJson<AnyStrippedStateEvent>>> for InviteState {
    fn from(events: Vec<RawJson<AnyStrippedStateEvent>>) -> Self {
        Self { events }
    }
}

/// Updates to the presence status of other users.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct Presence {
    /// A list of events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<RawJson<PresenceEvent>>,
}

impl Presence {
    /// Creates an empty `Presence`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no presence updates.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Messages sent directly between devices.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct ToDevice {
    /// A list of to-device events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<RawJson<AnyToDeviceEvent>>,
}

impl ToDevice {
    /// Creates an empty `ToDevice`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no to-device events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

// #[cfg(test)]
// mod tests {
//     use assign::assign;
//     use serde_json::{from_value as from_json_value, json, to_value as
// to_json_value};

//     use super::Timeline;

//     #[test]
//     fn timeline_serde() {
//         let timeline = assign!(Timeline::new(), { limited: true });
//         let timeline_serialized = json!({ "limited": true });
//         assert_eq!(to_json_value(timeline).unwrap(), timeline_serialized);

//         let timeline_deserialized =
// from_json_value::<Timeline>(timeline_serialized).unwrap();         assert!
// (timeline_deserialized.limited);

//         let timeline_default = Timeline::default();
//         assert_eq!(to_json_value(timeline_default).unwrap(), json!({}));

//         let timeline_default_deserialized =
// from_json_value::<Timeline>(json!({})).unwrap();         assert!(!
// timeline_default_deserialized.limited);     }
// }

// #[cfg(all(test))]
// mod server_tests {
//     use std::time::Duration;

//     use crate::{api::IncomingRequest as _, presence::PresenceState};
//     use assert_matches2::assert_matches;

//     use super::{Filter, Request};

//     #[test]
//     fn deserialize_all_query_params() {
//         let uri = http::Uri::builder()
//             .scheme("https")
//             .authority("matrix.org")
//             .path_and_query(
//                 "/_matrix/client/r0/sync\
//                 ?filter=myfilter\
//                 &since=myts\
//                 &full_state=false\
//                 &set_presence=offline\
//                 &timeout=5000",
//             )
//             .build()
//             .unwrap();

//         let req = Request::try_from_http_request(
//             http::Request::builder().uri(uri).body(&[] as &[u8]).unwrap(),
//             &[] as &[String],
//         )
//         .unwrap();

//         assert_matches!(req.filter, Some(Filter::FilterId(id)));
//         assert_eq!(id, "myfilter");
//         assert_eq!(req.since.as_deref(), Some("myts"));
//         assert!(!req.full_state);
//         assert_eq!(req.set_presence, PresenceState::Offline);
//         assert_eq!(req.timeout, Some(Duration::from_millis(5000)));
//     }

//     #[test]
//     fn deserialize_no_query_params() {
//         let uri = http::Uri::builder()
//             .scheme("https")
//             .authority("matrix.org")
//             .path_and_query("/_matrix/client/r0/sync")
//             .build()
//             .unwrap();

//         let req = Request::try_from_http_request(
//             http::Request::builder().uri(uri).body(&[] as &[u8]).unwrap(),
//             &[] as &[String],
//         )
//         .unwrap();

//         assert_matches!(req.filter, None);
//         assert_eq!(req.since, None);
//         assert!(!req.full_state);
//         assert_eq!(req.set_presence, PresenceState::Online);
//         assert_eq!(req.timeout, None);
//     }

//     #[test]
//     fn deserialize_some_query_params() {
//         let uri = http::Uri::builder()
//             .scheme("https")
//             .authority("matrix.org")
//             .path_and_query(
//                 "/_matrix/client/r0/sync\
//                 ?filter=EOKFFmdZYF\
//                 &timeout=0",
//             )
//             .build()
//             .unwrap();

//         let req = Request::try_from_http_request(
//             http::Request::builder().uri(uri).body(&[] as &[u8]).unwrap(),
//             &[] as &[String],
//         )
//         .unwrap();

//         assert_matches!(req.filter, Some(Filter::FilterId(id)));
//         assert_eq!(id, "EOKFFmdZYF");
//         assert_eq!(req.since, None);
//         assert!(!req.full_state);
//         assert_eq!(req.set_presence, PresenceState::Online);
//         assert_eq!(req.timeout, Some(Duration::from_millis(0)));
//     }
// }
