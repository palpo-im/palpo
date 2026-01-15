/// Endpoints for room management.
mod alias;
mod summary;
mod thread;
mod thread_msc4306;
mod thread_msc4308;
pub use alias::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
pub use summary::*;
pub use thread::*;
pub use thread_msc4306::*;
pub use thread_msc4308::*;

use crate::client::filter::RoomEventFilter;
use crate::client::membership::InviteThreepid;
use crate::events::room::create::PreviousRoom;
use crate::events::room::member::MembershipState;
use crate::events::room::power_levels::RoomPowerLevelsEventContent;
use crate::events::{
    AnyInitialStateEvent, AnyRoomAccountDataEvent, AnyStateEvent, AnyTimelineEvent,
};
use crate::room::{RoomType, Visibility};
use crate::serde::{RawJson, StringEnum};
use crate::{
    OwnedEventId, OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName, OwnedUserId, PrivOwnedStr,
    RoomVersionId,
};

// /// `POST /_matrix/client/*/createRoom`
// ///
// /// Create a new room.
//
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3createroom
//
// /// Whether or not a newly created room will be listed in the room directory.

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/createRoom",
//         1.1 => "/_matrix/client/v3/createRoom",
//     }
// };

/// Request type for the `create_room` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct CreateRoomReqBody {
    /// Extra keys to be added to the content of the `m.room.create`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_content: Option<RawJson<CreationContent>>,

    /// List of state events to send to the new room.
    ///
    /// Takes precedence over events set by preset, but gets overridden by name
    /// and topic keys.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub initial_state: Vec<RawJson<AnyInitialStateEvent>>,

    /// A list of user IDs to invite to the room.
    ///
    /// This will tell the server to invite everyone in the list to the newly
    /// created room.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub invite: Vec<OwnedUserId>,

    /// List of third party IDs of users to invite.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub invite_3pid: Vec<InviteThreepid>,

    /// If set, this sets the `is_direct` flag on room invites.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub is_direct: bool,

    /// If this is included, an `m.room.name` event will be sent into the room
    /// to indicate the name of the room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Power level content to override in the default power level event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_level_content_override: Option<RawJson<RoomPowerLevelsEventContent>>,

    /// Convenience parameter for setting various default state events based on
    /// a preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<RoomPreset>,

    /// The desired room alias local part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_alias_name: Option<String>,

    // /// The desired custom room ID, local part or fully qualified.
    // #[serde(alias = "fi.mau.room_id", skip_serializing_if = "Option::is_none")]
    // pub room_id: Option<String>,
    /// Room version to set for the room.
    ///
    /// Defaults to homeserver's default if not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_version: Option<RoomVersionId>,

    /// If this is included, an `m.room.topic` event will be sent into the room
    /// to indicate the topic for the room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,

    /// A public visibility indicates that the room will be shown in the
    /// published room list.
    ///
    /// A private visibility will hide the room from the published room list.
    /// Defaults to `Private`.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub visibility: Visibility,
}

/// Response type for the `create_room` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct CreateRoomResBody {
    /// The created room's ID.
    pub room_id: OwnedRoomId,
}

/// Extra options to be added to the `m.room.create` event.
///
/// This is the same as the event content struct for `m.room.create`, but
/// without some fields that servers are supposed to ignore.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct CreationContent {
    /// A list of user IDs to consider as additional creators, and hence grant an "infinite"
    /// immutable power level, from room version 12 onwards.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub additional_creators: Vec<OwnedUserId>,

    /// Whether users on other servers can join this room.
    ///
    /// Defaults to `true` if key does not exist.
    #[serde(
        rename = "m.federate",
        default = "crate::serde::default_true",
        skip_serializing_if = "crate::serde::is_true"
    )]
    pub federate: bool,

    /// A reference to the room this room replaces, if the previous room was
    /// upgraded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<PreviousRoom>,

    /// The room type.
    ///
    /// This is currently only used for spaces.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub room_type: Option<RoomType>,
}

impl CreationContent {
    /// Creates a new `CreationContent` with all fields defaulted.
    pub fn new() -> Self {
        Self {
            additional_creators: Vec::new(),
            federate: true,
            predecessor: None,
            room_type: None,
        }
    }

    // /// Given a `CreationContent` and the other fields that a homeserver has to
    // fill, construct /// a `RoomCreateEventContent`.
    // pub fn into_event_content(self, creator: OwnedUserId, room_version:
    // RoomVersionId) -> RoomCreateEventContent {     assign!
    // (RoomCreateEventContent::new_v1(creator), {         federate:
    // self.federate,         room_version: room_version,
    //         predecessor: self.predecessor,
    //         room_type: self.room_type
    //     })
    // }

    /// Returns whether all fields have their default value.
    pub fn is_empty(&self) -> bool {
        self.federate && self.predecessor.is_none() && self.room_type.is_none()
    }
}

impl Default for CreationContent {
    fn default() -> Self {
        Self::new()
    }
}

/// A convenience parameter for setting a few default state events.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RoomPreset {
    /// `join_rules` is set to `invite` and `history_visibility` is set to
    /// `shared`.
    PrivateChat,

    /// `join_rules` is set to `public` and `history_visibility` is set to
    /// `shared`.
    PublicChat,

    /// Same as `PrivateChat`, but all initial invitees get the same power level
    /// as the creator.
    TrustedPrivateChat,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

// /// `POST /_matrix/client/*/rooms/{room_id}/upgrade`
// ///
// /// Upgrades a room to a particular version.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3roomsroomidupgrade

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/upgrade",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/upgrade",
//     }
// };

/// Request type for the `upgrade_room` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct UpgradeRoomReqBody {
    /// A list of user IDs to consider as additional creators, and hence grant an "infinite"
    /// immutable power level, from room version 12 onwards.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub additional_creators: Vec<OwnedUserId>,

    /// ID of the room to be upgraded.
    // #[salvo(parameter(parameter_in = Path))]
    // pub room_id: OwnedRoomId,

    /// New version for the room.
    pub new_version: RoomVersionId,
}

/// Response type for the `upgrade_room` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct UpgradeRoomResBody {
    /// ID of the new room.
    pub replacement_room: OwnedRoomId,
}

// /// `GET /_matrix/client/*/rooms/{room_id}/timestamp_to_event`
// ///
// /// Get the ID of the event closest to the given timestamp.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv1roomsroomidtimestamp_to_event

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable =>
// "/_matrix/client/unstable/org.matrix.msc3030/rooms/:room_id/timestamp_to_event",         1.6 =>
// "/_matrix/client/v1/rooms/:room_id/timestamp_to_event",     }
// };

// /// `GET /_matrix/client/*/rooms/{room_id}/event/{event_id}`
// ///
// /// Get a single event based on roomId/eventId
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3roomsroomideventeventid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/event/:event_id",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/event/:event_id",
//     }
// };

/// Response type for the `get_room_event` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct RoomEventResBody(
    /// Arbitrary JSON of the event body.
    pub RawJson<AnyTimelineEvent>,
);
impl RoomEventResBody {
    /// Creates a new `Response` with the given event.
    pub fn new(event: RawJson<AnyTimelineEvent>) -> Self {
        Self(event)
    }
}

// /// `POST /_matrix/client/*/rooms/{room_id}/report/{event_id}`
// ///
// /// Report content as inappropriate.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3roomsroomidreporteventid
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/report/:event_id",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/report/:event_id",
//     }
// };

/// Request type for the `report_content` endpoint.

#[derive(ToParameters, Deserialize, Debug)]
pub struct ReportContentReqArgs {
    /// Room in which the event to be reported is located.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// Event to report.
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,
}

#[derive(ToSchema, Deserialize, Debug)]
pub struct ReportContentReqBody {
    /// Integer between -100 and 0 rating offensivness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,

    /// Reason to report content.
    ///
    /// May be blank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/read_markers",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/read_markers",
//     }
// };

/// Request type for the `set_read_marker` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct SetReadMarkerReqBody {
    // /// The room ID to set the read marker in for the user.
    // #[salvo(parameter(parameter_in = Path))]
    // pub room_id: OwnedRoomId,
    /// The event ID the fully-read marker should be located at.
    ///
    /// The event MUST belong to the room.
    ///
    /// This is equivalent to calling the [`create_receipt`] endpoint with a
    /// [`ReceiptType::FullyRead`].
    ///
    /// [`create_receipt`]: crate::receipt::create_receipt
    /// [`ReceiptType::FullyRead`]: crate::receipt::create_receipt::v3::ReceiptType::FullyRead
    #[serde(
        default,
        rename = "m.fully_read",
        skip_serializing_if = "Option::is_none"
    )]
    pub fully_read: Option<OwnedEventId>,

    /// The event ID to set the public read receipt location at.
    ///
    /// This is equivalent to calling the [`create_receipt`] endpoint with a
    /// [`ReceiptType::Read`].
    ///
    /// [`create_receipt`]: crate::receipt::create_receipt
    /// [`ReceiptType::Read`]: crate::receipt::create_receipt::v3::ReceiptType::Read
    #[serde(default, rename = "m.read", skip_serializing_if = "Option::is_none")]
    pub read_receipt: Option<OwnedEventId>,

    /// The event ID to set the private read receipt location at.
    ///
    /// This is equivalent to calling the [`create_receipt`] endpoint with a
    /// [`ReceiptType::ReadPrivate`].
    ///
    /// [`create_receipt`]: crate::receipt::create_receipt
    /// [`ReceiptType::ReadPrivate`]: crate::receipt::create_receipt::v3::ReceiptType::ReadPrivate
    #[serde(
        default,
        rename = "m.read.private",
        skip_serializing_if = "Option::is_none"
    )]
    pub private_read_receipt: Option<OwnedEventId>,
}

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/context/:event_id",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/context/:event_id",
//     }
// };

// /// Request type for the `get_context` endpoint.

#[derive(ToParameters, Deserialize, Debug)]
pub struct ContextReqArgs {
    /// The room to get events from.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The event to get context around.
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,

    /// The maximum number of context events to return.
    ///
    /// This limit applies to the sum of the `events_before` and `events_after`
    /// arrays. The requested event ID is always returned in `event` even if
    /// the limit is `0`.
    ///
    /// Defaults to 10.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default = "default_limit", skip_serializing_if = "is_default_limit")]
    pub limit: usize,

    /// A RoomEventFilter to filter returned events with.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(
        with = "crate::serde::json_string",
        default,
        skip_serializing_if = "RoomEventFilter::is_empty"
    )]
    pub filter: RoomEventFilter,
}

fn default_limit() -> usize {
    10
}
/// Response type for the `get_context` endpoint.

#[derive(ToSchema, Serialize, Default, Debug)]
pub struct ContextResBody {
    /// A token that can be used to paginate backwards with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,

    /// A token that can be used to paginate forwards with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,

    /// A list of room events that happened just before the requested event,
    /// in reverse-chronological order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub events_before: Vec<RawJson<AnyTimelineEvent>>,

    /// Details of the requested event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub event: Option<RawJson<AnyTimelineEvent>>,

    /// A list of room events that happened just after the requested event,
    /// in chronological order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub events_after: Vec<RawJson<AnyTimelineEvent>>,

    /// The state of the room at the last event returned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub state: Vec<RawJson<AnyStateEvent>>,
}

impl ContextResBody {
    /// Creates an empty `Response`.
    pub fn new() -> Self {
        Default::default()
    }
}
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable =>
// "/_matrix/client/unstable/xyz.amorgan.knock/knock/:room_id_or_alias",
//         1.1 => "/_matrix/client/v3/knock/:room_id_or_alias",
//     }
// };

/// Request type for the `knock_room` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct KnockReqArgs {
    /// The room the user should knock on.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id_or_alias: OwnedRoomOrAliasId,

    /// The servers to attempt to knock on the room through.
    ///
    /// One of the servers must be participating in the room.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub server_name: Vec<OwnedServerName>,
}
/// Request type for the `knock_room` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct KnockReqBody {
    /// The reason for joining a room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub via: Vec<OwnedServerName>,
}

/// Response type for the `knock_room` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct KnockResBody {
    /// The room that the user knocked on.
    pub room_id: OwnedRoomId,
}
impl KnockResBody {
    /// Creates a new `Response` with the given room ID.
    pub fn new(room_id: OwnedRoomId) -> Self {
        Self { room_id }
    }
}

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/initialSync",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/initialSync",
//     }
// };

/// Request type for the `get_room_event` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct InitialSyncReqArgs {
    /// The ID of the room.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// Limit messages chunks size
    #[salvo(parameter(parameter_in = Query))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Response type for the `get_room_event` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Debug)]
pub struct InitialSyncResBody {
    /// The private data that this user has attached to this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_data: Option<Vec<RawJson<AnyRoomAccountDataEvent>>>,

    /// The user’s membership state in this room. One of: [invite, join, leave,
    /// ban].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub membership: Option<MembershipState>,

    /// The pagination chunk for this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<PaginationChunk>,

    /// The ID of this room.
    pub room_id: OwnedRoomId,

    /// If the user is a member of the room this will be the current state of
    /// the room as a list of events. If the user has left the room this
    /// will be the state of the room when they left it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Vec<RawJson<AnyStateEvent>>>,

    /// Whether this room is visible to the /publicRooms API or not.
    /// One of: [private, public].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
}

/// Page of timeline events
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct PaginationChunk {
    /// If the user is a member of the room this will be a list of the most
    /// recent messages for this room. If the user has left the room this
    /// will be the messages that preceded them leaving. This array will
    /// consist of at most limit elements.
    pub chunk: Vec<RawJson<AnyTimelineEvent>>,

    /// A token which correlates to the end of chunk. Can be passed to
    /// /rooms/<room_id>/messages to retrieve later events.
    pub end: String,

    /// A token which correlates to the start of chunk. Can be passed to
    /// /rooms/<room_id>/messages to retrieve earlier events. If no earlier
    /// events are available, this property may be omitted from the
    /// response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
}
