use reqwest::Url;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::identifiers::*;
use crate::room::TimestampToEventReqArgs;
use crate::sending::{SendRequest, SendResult};
use crate::serde::RawJsonValue;
use crate::{Direction, UnixMillis};

// /// `GET /_matrix/federation/*/timestamp_to_event/{room_id}`
// ///
// /// Get the ID of the event closest to the given timestamp.
//
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1timestamp_to_eventroomid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         unstable =>
// "/_matrix/federation/unstable/org.matrix.msc3030/timestamp_to_event/:room_id",         1.6 =>
// "/_matrix/federation/v1/timestamp_to_event/:room_id",     }
// };

pub fn timestamp_to_event_request(
    origin: &str,
    args: TimestampToEventReqArgs,
) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/timestamp_to_event/{}",
        args.room_id
    ))?;
    url.query_pairs_mut()
        .append_pair(
            "dir",
            match args.dir {
                Direction::Forward => "f",
                Direction::Backward => "b",
            },
        )
        .append_pair("ts", &args.ts.to_string());
    Ok(crate::sending::get(url))
}

// /// `GET /_matrix/federation/*/event/{event_id}`
// ///
// /// Retrieves a single event.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1eventeventid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/event/:event_id",
//     }
// };

// /// Request type for the `get_event` endpoint.

// pub struct Request {
//     /// The event ID to get.
//     #[salvo(parameter(parameter_in = Path))]
//     pub event_id: OwnedEventId,
// }

pub fn event_request(origin: &str, args: EventReqArgs) -> SendResult<SendRequest> {
    let url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/event/{}",
        args.event_id
    ))?;
    Ok(crate::sending::get(url))
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct EventReqArgs {
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,

    /// Event to report.
    #[salvo(parameter(parameter_in = Path))]
    pub include_unredacted_content: Option<bool>,
}
impl EventReqArgs {
    pub fn new(event_id: impl Into<OwnedEventId>) -> Self {
        Self {
            event_id: event_id.into(),
            include_unredacted_content: None,
        }
    }
}

/// Response type for the `get_event` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct EventResBody {
    /// The `server_name` of the homeserver sending this transaction.
    pub origin: OwnedServerName,

    /// Time on originating homeserver when this transaction started.
    pub origin_server_ts: UnixMillis,

    /// The event.
    #[serde(rename = "pdus", with = "crate::serde::single_element_seq")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub pdu: Box<RawJsonValue>,
}
impl EventResBody {
    /// Creates a new `Response` with the given server name, timestamp, and
    /// event.
    pub fn new(
        origin: OwnedServerName,
        origin_server_ts: UnixMillis,
        pdu: Box<RawJsonValue>,
    ) -> Self {
        Self {
            origin,
            origin_server_ts,
            pdu,
        }
    }
}

// /// `POST /_matrix/federation/*/get_missing_events/{room_id}`
// ///
// /// Retrieves previous events that the sender is missing.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#post_matrixfederationv1get_missing_eventsroomid

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/get_missing_events/:room_id",
//     }
// };

pub fn missing_events_request(
    origin: &str,
    room_id: &RoomId,
    body: MissingEventsReqBody,
) -> SendResult<SendRequest> {
    let url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/get_missing_events/{room_id}"
    ))?;
    crate::sending::post(url).stuff(body)
}

/// Request type for the `get_missing_events` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Debug)]
pub struct MissingEventsReqBody {
    /// The room ID to search in.
    // #[salvo(parameter(parameter_in = Path))]
    // pub room_id: OwnedRoomId,

    /// The maximum number of events to retrieve.
    ///
    /// Defaults to 10.
    #[serde(default = "default_limit", skip_serializing_if = "is_default_limit")]
    pub limit: usize,

    /// The minimum depth of events to retrieve.
    ///
    /// Defaults to 0.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub min_depth: u64,

    /// The latest event IDs that the sender already has.
    ///
    /// These are skipped when retrieving the previous events of
    /// `latest_events`.
    pub earliest_events: Vec<OwnedEventId>,

    /// The event IDs to retrieve the previous events for.
    pub latest_events: Vec<OwnedEventId>,
}
crate::json_body_modifier!(MissingEventsReqBody);

/// Response type for the `get_missing_events` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct MissingEventsResBody {
    /// The missing PDUs.
    #[salvo(schema(value_type = Vec<Object>))]
    pub events: Vec<Box<RawJsonValue>>,
}
impl MissingEventsResBody {
    /// Creates a new `Response` with the given events.
    pub fn new(events: Vec<Box<RawJsonValue>>) -> Self {
        Self { events }
    }
}

fn default_limit() -> usize {
    10
}

fn is_default_limit(val: &usize) -> bool {
    *val == default_limit()
}

// /// `GET /_matrix/federation/*/state_ids/{room_id}`
// ///
// /// Retrieves a snapshot of a room's state at a given event, in the form of
// /// event IDs. `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1state_idsroomid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/state_ids/:room_id",
//     }
// };
pub fn room_state_ids_request(
    origin: &str,
    args: RoomStateAtEventReqArgs,
) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/state_ids/{}",
        args.room_id
    ))?;
    url.query_pairs_mut()
        .append_pair("event_id", args.event_id.as_str());
    Ok(crate::sending::get(url))
}

/// Request type for the `get_state_ids` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct RoomStateAtEventReqArgs {
    /// The room ID to get state for.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// An event ID in the room to retrieve the state at.
    #[salvo(parameter(parameter_in = Query))]
    pub event_id: OwnedEventId,
}

/// Response type for the `get_state_ids` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Debug)]
pub struct RoomStateIdsResBody {
    /// The full set of authorization events that make up the state of the
    /// room, and their authorization events, recursively.
    pub auth_chain_ids: Vec<OwnedEventId>,

    /// The fully resolved state of the room at the given event.
    pub pdu_ids: Vec<OwnedEventId>,
}

impl RoomStateIdsResBody {
    /// Creates a new `Response` with the given auth chain IDs and room state
    /// IDs.
    pub fn new(auth_chain_ids: Vec<OwnedEventId>, pdu_ids: Vec<OwnedEventId>) -> Self {
        Self {
            auth_chain_ids,
            pdu_ids,
        }
    }
}

// /// `GET /_matrix/federation/*/state/{room_id}`
// ///
// /// Retrieves a snapshot of a room's state at a given event.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1stateroomid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/state/:room_id",
//     }
// };

pub fn room_state_request(origin: &str, args: RoomStateReqArgs) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/state/{}",
        args.room_id
    ))?;
    url.query_pairs_mut()
        .append_pair("event_id", args.event_id.as_str());
    Ok(crate::sending::get(url))
}
/// Request type for the `get_state` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct RoomStateReqArgs {
    /// The room ID to get state for.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// An event ID in the room to retrieve the state at.
    #[salvo(parameter(parameter_in = Query))]
    pub event_id: OwnedEventId,
}

/// Response type for the `get_state` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct RoomStateResBody {
    /// The full set of authorization events that make up the state of the
    /// room, and their authorization events, recursively.
    #[salvo(schema(value_type = Object))]
    pub auth_chain: Vec<Box<RawJsonValue>>,

    /// The fully resolved state of the room at the given event.
    #[salvo(schema(value_type = Vec<Object>))]
    pub pdus: Vec<Box<RawJsonValue>>,
}

// /// `PUT /_matrix/app/*/ping`
// ///
// /// Endpoint to ping the application service.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/application-service-api/#post_matrixappv1ping

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/app/unstable/fi.mau.msc2659/ping",
//         1.7 => "/_matrix/app/v1/ping",
//     }
// };

/// Request type for the `send_ping` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct PingReqBody {
    /// A transaction ID for the ping, copied directly from the `POST
    /// /_matrix/client/v1/appservice/{appserviceId}/ping` call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<OwnedTransactionId>,
}
