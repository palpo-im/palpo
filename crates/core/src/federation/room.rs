/// Endpoints for room management.
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::client::filter::RoomEventFilter;
use crate::events::{AnyStateEvent, AnyTimelineEvent};
use crate::serde::RawJson;
use crate::{OwnedEventId, OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName};

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
pub struct RoomEventResBody {
    /// Arbitrary JSON of the event body.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub event: RawJson<AnyTimelineEvent>,
}
impl RoomEventResBody {
    /// Creates a new `Response` with the given event.
    pub fn new(event: RawJson<AnyTimelineEvent>) -> Self {
        Self { event }
    }
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,

    /// A token that can be used to paginate forwards with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,

    /// A list of room events that happened just before the requested event,
    /// in reverse-chronological order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub events_before: Vec<RawJson<AnyTimelineEvent>>,

    /// Details of the requested event.
    #[serde(skip_serializing_if = "Option::is_none")]
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

    /// The reason for joining a room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// The servers to attempt to knock on the room through.
    ///
    /// One of the servers must be participating in the room.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub server_name: Vec<OwnedServerName>,
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
