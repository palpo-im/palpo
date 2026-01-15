use salvo::oapi::{ToParameters, ToSchema};
use serde::{Deserialize, Serialize};

/// `GET /_matrix/client/*/rooms/{room_id}/relations/{event_id}/{rel_type}/
/// {event_type}`
///
/// Get the child events for a given parent event which relate to the parent
/// using the given `rel_type` and having the given `event_type`.
use crate::events::{AnyMessageLikeEvent, TimelineEventType, relation::RelationType};
use crate::serde::RawJson;
use crate::{Direction, OwnedEventId, OwnedRoomId};
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv1roomsroomidrelationseventidreltypeeventtype

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         unstable =>
// "/_matrix/client/unstable/rooms/:room_id/relations/:event_id/:rel_type/:event_type",         1.3
// => "/_matrix/client/v1/rooms/:room_id/relations/:event_id/:rel_type/:event_type",     }
// };

/// Request type for the `get_relating_events_with_rel_type_and_event_type`
/// endpoint.

#[derive(ToParameters, Deserialize, Debug)]
pub struct RelatingEventsWithRelTypeAndEventTypeReqArgs {
    /// The ID of the room containing the parent event.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The ID of the parent event whose child events are to be returned.
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,

    /// The relationship type to search for.
    #[salvo(parameter(parameter_in = Path))]
    pub rel_type: RelationType,

    /// The event type of child events to search for.
    ///
    /// Note that in encrypted rooms this will typically always be
    /// `m.room.encrypted` regardless of the event type contained within the
    /// encrypted payload.
    #[salvo(parameter(parameter_in = Path))]
    pub event_type: TimelineEventType,

    /// The pagination token to start returning results from.
    ///
    /// If `None`, results start at the most recent topological event known to
    /// the server.
    ///
    /// Can be a `next_batch` token from a previous call, or a returned  `start`
    /// token from `/messages` or a `next_batch` token from `/sync`.
    ///
    /// Note that when paginating the `from` token should be "after" the `to`
    /// token in terms of topological ordering, because it is only possible
    /// to paginate "backwards" through events, starting at `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub from: Option<String>,

    /// The direction to return events from.
    ///
    /// Defaults to [`Direction::Backward`].
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    #[salvo(parameter(parameter_in = Query))]
    pub dir: Direction,

    /// The pagination token to stop returning results at.
    ///
    /// If `None`, results continue up to `limit` or until there are no more
    /// events.
    ///
    /// Like `from`, this can be a previous token from a prior call to this
    /// endpoint or from `/messages` or `/sync`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub to: Option<String>,

    /// The maximum number of results to return in a single `chunk`.
    ///
    /// The server can and should apply a maximum value to this parameter to
    /// avoid large responses.
    ///
    /// Similarly, the server should apply a default value when not supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub limit: Option<usize>,

    /// Whether to include events which relate indirectly to the given event.
    ///
    /// These are events related to the given event via two or more direct
    /// relationships.
    ///
    /// It is recommended that homeservers traverse at least 3 levels of
    /// relationships. Implementations may perform more but should be
    /// careful to not infinitely recurse.
    ///
    /// Default to `false`.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    #[salvo(parameter(parameter_in = Query))]
    pub recurse: bool,
}

// /// `GET /_matrix/client/*/rooms/{room_id}/relations/{event_id}/{relType}`
// ///
// /// Get the child events for a given parent event which relate to the parent
// /// using the given `rel_type`.
//
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv1roomsroomidrelationseventidreltype

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/rooms/:room_id/relations/:event_id/:rel_type",
//         1.3 => "/_matrix/client/v1/rooms/:room_id/relations/:event_id/:rel_type",
//     }
// };

/// Request type for the `get_relating_events_with_rel_type` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct RelatingEventsWithRelTypeReqArgs {
    /// The ID of the room containing the parent event.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The ID of the parent event whose child events are to be returned.
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,

    /// The relationship type to search for.
    #[salvo(parameter(parameter_in = Path))]
    pub rel_type: RelationType,

    /// The pagination token to start returning results from.
    ///
    /// If `None`, results start at the most recent topological event known to
    /// the server.
    ///
    /// Can be a `next_batch` token from a previous call, or a returned  `start`
    /// token from `/messages` or a `next_batch` token from `/sync`.
    ///
    /// Note that when paginating the `from` token should be "after" the `to`
    /// token in terms of topological ordering, because it is only possible
    /// to paginate "backwards" through events, starting at `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub from: Option<String>,

    /// The direction to return events from.
    ///
    /// Defaults to [`Direction::Backward`].
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    #[salvo(parameter(parameter_in = Query))]
    pub dir: Direction,

    /// The pagination token to stop returning results at.
    ///
    /// If `None`, results continue up to `limit` or until there are no more
    /// events.
    ///
    /// Like `from`, this can be a previous token from a prior call to this
    /// endpoint or from `/messages` or `/sync`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub to: Option<String>,

    /// The maximum number of results to return in a single `chunk`.
    ///
    /// The server can and should apply a maximum value to this parameter to
    /// avoid large responses.
    ///
    /// Similarly, the server should apply a default value when not supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub limit: Option<usize>,

    /// Whether to include events which relate indirectly to the given event.
    ///
    /// These are events related to the given event via two or more direct
    /// relationships.
    ///
    /// It is recommended that homeservers traverse at least 3 levels of
    /// relationships. Implementations may perform more but should be
    /// careful to not infinitely recurse.
    ///
    /// Default to `false`.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    #[salvo(parameter(parameter_in = Query))]
    pub recurse: bool,
}

// /// `GET /_matrix/client/*/rooms/{room_id}/relations/{event_id}`
// ///
// /// Get the child events for a given parent event.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv1roomsroomidrelationseventid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/rooms/:room_id/relations/:event_id",
//         1.3 => "/_matrix/client/v1/rooms/:room_id/relations/:event_id",
//     }
// };

/// Request type for the `get_relating_events` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct RelatingEventsReqArgs {
    /// The ID of the room containing the parent event.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The ID of the parent event whose child events are to be returned.
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,

    /// The pagination token to start returning results from.
    ///
    /// If `None`, results start at the most recent topological event known to
    /// the server.
    ///
    /// Can be a `next_batch` or `prev_batch` token from a previous call, or a
    /// returned `start` token from `/messages` or a `next_batch` token from
    /// `/sync`.
    ///
    /// Note that when paginating the `from` token should be "after" the `to`
    /// token in terms of topological ordering, because it is only possible
    /// to paginate "backwards" through events, starting at `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub from: Option<String>,

    /// The direction to return events from.
    ///
    /// Defaults to [`Direction::Backward`].
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    #[salvo(parameter(parameter_in = Query))]
    pub dir: Direction,

    /// The pagination token to stop returning results at.
    ///
    /// If `None`, results continue up to `limit` or until there are no more
    /// events.
    ///
    /// Like `from`, this can be a previous token from a prior call to this
    /// endpoint or from `/messages` or `/sync`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub to: Option<String>,

    /// The maximum number of results to return in a single `chunk`.
    ///
    /// The server can and should apply a maximum value to this parameter to
    /// avoid large responses.
    ///
    /// Similarly, the server should apply a default value when not supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub limit: Option<usize>,

    /// Whether to include events which relate indirectly to the given event.
    ///
    /// These are events related to the given event via two or more direct
    /// relationships.
    ///
    /// It is recommended that homeservers traverse at least 3 levels of
    /// relationships. Implementations may perform more but should be
    /// careful to not infinitely recurse.
    ///
    /// Default to `false`.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    #[salvo(parameter(parameter_in = Query))]
    pub recurse: bool,
}

/// Response type for the `get_relating_events` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct RelationEventsResBody {
    /// The paginated child events which point to the parent.
    ///
    /// The events returned are ordered topologically, most-recent first.
    ///
    /// If no events are related to the parent or the pagination yields no
    /// results, an empty `chunk` is returned.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub chunk: Vec<RawJson<AnyMessageLikeEvent>>,

    /// An opaque string representing a pagination token.
    ///
    /// If this is `None`, there are no more results to fetch and the client
    /// should stop paginating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_batch: Option<String>,

    /// An opaque string representing a pagination token.
    ///
    /// If this is `None`, this is the start of the result set, i.e. this is the
    /// first batch/page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_batch: Option<String>,

    /// If `recurse` was set on the request, the depth to which the server
    /// recursed.
    ///
    /// If `recurse` was not set, this field must be absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recursion_depth: Option<u64>,
}
