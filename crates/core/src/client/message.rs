use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::client::filter::RoomEventFilter;
use crate::events::{AnyStateEvent, AnyTimelineEvent, MessageLikeEventType};
use crate::serde::RawJson;
use crate::{Direction, OwnedEventId, OwnedRoomId, OwnedTransactionId, UnixMillis};

// /// `GET /_matrix/client/*/rooms/{room_id}/messages`
// ///
// /// Get message events for a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3roomsroomidmessages
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/messages",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/messages",
//     }
// };

/// Request type for the `get_message_events` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct MessagesReqArgs {
    /// The room to get events from.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The token to start returning events from.
    ///
    /// This token can be obtained from a `prev_batch` token returned for each
    /// room by the sync endpoint, or from a `start` or `end` token returned
    /// by a previous request to this endpoint.
    ///
    /// If this is `None`, the server will return messages from the start or end
    /// of the history visible to the user, depending on the value of
    /// [`dir`][Self::dir].
    #[serde(default)]
    #[salvo(parameter(parameter_in = Query))]
    pub from: Option<String>,

    /// The token to stop returning events at.
    ///
    /// This token can be obtained from a `prev_batch` token returned for each
    /// room by the sync endpoint, or from a `start` or `end` token returned
    /// by a previous request to this endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub to: Option<String>,

    /// The direction to return events from.
    #[serde(default)]
    #[salvo(parameter(parameter_in = Query))]
    pub dir: Direction,

    /// The maximum number of events to return.
    ///
    /// Default: `10`.
    #[serde(default = "default_limit", skip_serializing_if = "is_default_limit")]
    #[salvo(parameter(parameter_in = Query))]
    pub limit: usize,

    /// A [`RoomEventFilter`] to filter returned events with.
    #[serde(
        with = "crate::serde::json_string",
        default,
        skip_serializing_if = "RoomEventFilter::is_empty"
    )]
    #[salvo(parameter(parameter_in = Query))]
    pub filter: RoomEventFilter,
}

/// Response type for the `get_message_events` endpoint.
#[derive(ToSchema, Default, Serialize, Debug)]
pub struct MessagesResBody {
    /// The token the pagination starts from.
    pub start: String,

    /// The token the pagination ends at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,

    /// A list of room events.
    #[serde(default)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub chunk: Vec<RawJson<AnyTimelineEvent>>,

    /// A list of state events relevant to showing the `chunk`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub state: Vec<RawJson<AnyStateEvent>>,
}

fn default_limit() -> usize {
    10
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_default_limit(val: &usize) -> bool {
    *val == default_limit()
}

// /// `PUT /_matrix/client/*/rooms/{room_id}/send/{eventType}/{txn_id}`
// ///
// /// Send a message event to a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3roomsroomidsendeventtypetxnid

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/send/:event_type/:txn_id",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/send/:event_type/:txn_id",
//     }
// };

/// Request type for the `create_message_event` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct CreateMessageWithTxnReqArgs {
    /// The room to send the event to.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The type of event to send.
    #[salvo(parameter(parameter_in = Path))]
    pub event_type: MessageLikeEventType,

    /// The transaction ID for this event.
    ///
    /// Clients should generate a unique ID across requests within the
    /// same session. A session is identified by an access token, and
    /// persists when the [access token is refreshed].
    ///
    /// It will be used by the server to ensure idempotency of requests.
    ///
    /// [access token is refreshed]: https://spec.matrix.org/latest/client-server-api/#refreshing-access-tokens
    #[salvo(parameter(parameter_in = Path))]
    pub txn_id: OwnedTransactionId,

    // /// The event content to send.
    // #[salvo(schema(value_type = Object, additional_properties = true))]
    // pub body: RawJson<AnyMessageLikeEventContent>,
    /// Timestamp to use for the `origin_server_ts` of the event.
    ///
    /// This is called [timestamp massaging] and can only be used by
    /// Appservices.
    ///
    /// Note that this does not change the position of the event in the
    /// timeline.
    ///
    /// [timestamp massaging]: https://spec.matrix.org/latest/application-service-api/#timestamp-massaging
    #[salvo(parameter(parameter_in = Query))]
    #[serde(skip_serializing_if = "Option::is_none", rename = "ts")]
    pub timestamp: Option<UnixMillis>,
}

/// Request type for the `create_message_event` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct CreateMessageReqArgs {
    /// The room to send the event to.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The type of event to send.
    #[salvo(parameter(parameter_in = Path))]
    pub event_type: MessageLikeEventType,

    // /// The event content to send.
    // #[salvo(schema(value_type = Object, additional_properties = true))]
    // pub body: RawJson<AnyMessageLikeEventContent>,
    /// Timestamp to use for the `origin_server_ts` of the event.
    ///
    /// This is called [timestamp massaging] and can only be used by
    /// Appservices.
    ///
    /// Note that this does not change the position of the event in the
    /// timeline.
    ///
    /// [timestamp massaging]: https://spec.matrix.org/latest/application-service-api/#timestamp-massaging
    #[salvo(parameter(parameter_in = Query))]
    #[serde(skip_serializing_if = "Option::is_none", rename = "ts")]
    pub timestamp: Option<UnixMillis>,
}

/// Response type for the `create_message_event` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct SendMessageResBody {
    /// A unique identifier for the event.
    pub event_id: OwnedEventId,
}
impl SendMessageResBody {
    /// Creates a new `Response` with the given event id.
    pub fn new(event_id: OwnedEventId) -> Self {
        Self { event_id }
    }
}
