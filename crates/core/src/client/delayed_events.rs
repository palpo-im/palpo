//! Endpoints for sending and interacting with delayed events.
//!
//! Delayed events are an unstable feature added by [MSC4140].
//!
//! [MSC4140]: https://github.com/matrix-org/matrix-spec-proposals/pull/4140

use std::time::Duration;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::TimelineEventType;
use crate::serde::{JsonValue, StringEnum};
use crate::{OwnedEventId, OwnedRoomId, OwnedTransactionId, PrivOwnedStr, UnixMillis};

/// The standard error response body stored for a delayed event that failed to
/// be sent.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct DelayedEventError {
    /// The Matrix error code of the failure, e.g. `M_FORBIDDEN`.
    pub errcode: String,

    /// A human-readable description of the failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The structure of the data for returning a delayed event from a GET endpoint.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct DelayedEventData {
    /// The ID of the delayed event.
    pub delay_id: String,

    /// The ID of the room that the delayed event was scheduled to be sent in.
    pub room_id: OwnedRoomId,

    /// The event type of the delayed event.
    #[serde(rename = "type")]
    pub event_type: TimelineEventType,

    /// The state key if the event is a state event, nothing otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_key: Option<String>,

    /// The event content to send.
    ///
    /// This is the content that was submitted to the send endpoint, not the
    /// content of the final event.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub content: JsonValue,

    /// The duration that the server should wait before sending this event.
    #[serde(with = "crate::serde::duration::ms")]
    #[salvo(schema(value_type = u64))]
    pub delay: Duration,

    /// The timestamp when the delayed event was scheduled or last restarted.
    pub running_since: UnixMillis,

    /// The error that prevented the delayed event from being sent.
    ///
    /// Present only for finalized events that were cancelled due to an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<DelayedEventError>,

    /// The `event_id` this event got when it was sent.
    ///
    /// Present only for events that were sent successfully.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<OwnedEventId>,

    /// The timestamp when the event was finalized.
    ///
    /// Present only for events that were finalized (sent, failed to send, or
    /// cancelled).
    #[serde(
        rename = "finalised_ts",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub finalized_ts: Option<UnixMillis>,
}

impl DelayedEventData {
    /// Returns the status indicated by this delayed event data.
    pub fn status(&self) -> DelayedEventStatus {
        if self.finalized_ts.is_none() {
            DelayedEventStatus::Scheduled
        } else if self.event_id.is_some() {
            DelayedEventStatus::Send
        } else if self.error.is_some() {
            DelayedEventStatus::Error
        } else {
            DelayedEventStatus::Cancel
        }
    }
}

/// The status that a delayed event stored on the server can have.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DelayedEventStatus {
    /// The event is currently scheduled to be submitted at a later date.
    /// It may be restarted, sent or cancelled via the management endpoint.
    Scheduled,

    /// The event has been sent successfully.
    Send,

    /// The event has been cancelled.
    Cancel,

    /// The event has encountered an error when trying to send.
    Error,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// The possible update actions for updating a delayed event.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "lowercase")]
#[non_exhaustive]
pub enum UpdateAction {
    /// Restart the delayed event timeout. (heartbeat ping)
    Restart,

    /// Send the delayed event immediately independent of the timeout state.
    /// (deletes all timers)
    Send,

    /// Delete the delayed event and never send it. (deletes all timers)
    Cancel,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

// /// `PUT /_matrix/client/unstable/org.matrix.msc4140/rooms/{room_id}/delayed_event/{event_type}/
// {txn_id}` ///
// /// Send a delayed event (a scheduled message) to a room.
// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable("org.matrix.msc4140") =>
// "/_matrix/client/unstable/org.matrix.msc4140/rooms/{room_id}/delayed_event/{event_type}/{txn_id}"
// ,     }
// };

/// Request args for the `send_delayed_event` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct SendDelayedEventReqArgs {
    /// The room to send the event to.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The type of event to send.
    #[salvo(parameter(parameter_in = Path))]
    pub event_type: TimelineEventType,

    /// The transaction ID for this event.
    ///
    /// Clients should generate a unique ID across requests within the
    /// same session. It will be used by the server to ensure idempotency of
    /// requests.
    #[salvo(parameter(parameter_in = Path))]
    pub txn_id: OwnedTransactionId,

    /// Timestamp to use for the `origin_server_ts` of the event when it is
    /// sent.
    ///
    /// This is called [timestamp massaging] and can only be used by
    /// Appservices.
    ///
    /// [timestamp massaging]: https://spec.matrix.org/latest/application-service-api/#timestamp-massaging
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ts")]
    pub timestamp: Option<UnixMillis>,
}

/// Request body for the `send_delayed_event` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct SendDelayedEventReqBody {
    /// The duration that the server should wait before sending this event.
    #[serde(with = "crate::serde::duration::ms")]
    #[salvo(schema(value_type = u64))]
    pub delay: Duration,

    /// The state key if the event is a state event, nothing otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_key: Option<String>,

    /// The event content to send.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub content: JsonValue,
}

/// Response type for the `send_delayed_event` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct SendDelayedEventResBody {
    /// The `delay_id` generated for this delayed event. Used to interact with
    /// delayed events.
    pub delay_id: String,
}

impl SendDelayedEventResBody {
    /// Creates a new `SendDelayedEventResBody` with the given delay id.
    pub fn new(delay_id: String) -> Self {
        Self { delay_id }
    }
}

// /// `GET /_matrix/client/unstable/org.matrix.msc4140/delayed_events`
// ///
// /// Get all of the user's scheduled delayed events.
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable("org.matrix.msc4140") =>
// "/_matrix/client/unstable/org.matrix.msc4140/delayed_events",     }
// };

/// Response type for the `get_all_delayed_events` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct DelayedEventsResBody {
    /// An array of objects describing scheduled delayed events owned by the
    /// requesting user.
    pub delayed_events: Vec<DelayedEventData>,
}

impl DelayedEventsResBody {
    /// Creates a new `DelayedEventsResBody` with the given delayed events.
    pub fn new(delayed_events: Vec<DelayedEventData>) -> Self {
        Self { delayed_events }
    }
}

// /// `GET /_matrix/client/unstable/org.matrix.msc4140/delayed_events/{delay_id}`
// ///
// /// Get the information about a delayed event. The response body is a single
// /// `DelayedEventData` object.
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable("org.matrix.msc4140") =>
// "/_matrix/client/unstable/org.matrix.msc4140/delayed_events/{delay_id}",     }
// };

// /// `POST /_matrix/client/unstable/org.matrix.msc4140/delayed_events/{delay_id}/{action}`
// ///
// /// Update a delayed event: restart its timeout, send it immediately, or
// /// cancel it.
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable("org.matrix.msc4140") =>
// "/_matrix/client/unstable/org.matrix.msc4140/delayed_events/{delay_id}/{action}",     }
// };

/// Request args for the `update_delayed_event` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct UpdateDelayedEventReqArgs {
    /// The delay id that we want to update.
    #[salvo(parameter(parameter_in = Path))]
    pub delay_id: String,

    /// Which kind of update we want to request for the delayed event.
    #[salvo(parameter(parameter_in = Path))]
    pub action: UpdateAction,
}

/// Request body for the deprecated body-action variant of the
/// `update_delayed_event` endpoint
/// (`POST /_matrix/client/unstable/org.matrix.msc4140/delayed_events/{delay_id}`).
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct UpdateDelayedEventReqBody {
    /// Which kind of update we want to request for the delayed event.
    pub action: UpdateAction,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::{Value as JsonValue, json};

    use super::*;

    #[test]
    fn deserialize_send_delayed_event_req_body() {
        let body: SendDelayedEventReqBody = serde_json::from_value(json!({
            "delay": 103,
            "content": {"msgtype": "m.text", "body": "test"},
        }))
        .unwrap();

        assert_eq!(body.delay, Duration::from_millis(103));
        assert_eq!(body.state_key, None);
        assert_eq!(body.content, json!({"msgtype": "m.text", "body": "test"}));
    }

    #[test]
    fn deserialize_send_delayed_state_event_req_body() {
        let body: SendDelayedEventReqBody = serde_json::from_value(json!({
            "delay": 9000,
            "state_key": "a_state_key",
            "content": {"topic": "test topic"},
        }))
        .unwrap();

        assert_eq!(body.delay, Duration::from_millis(9000));
        assert_eq!(body.state_key.as_deref(), Some("a_state_key"));
    }

    #[test]
    fn serialize_delayed_event_data() {
        let event = DelayedEventData {
            delay_id: "a_delay_id".to_owned(),
            room_id: "!roomid:example.org".try_into().unwrap(),
            event_type: "m.room.topic".into(),
            state_key: Some("a_state_key".to_owned()),
            content: json!({"topic": "test topic"}),
            delay: Duration::from_millis(103),
            running_since: crate::UnixMillis(70000),
            error: None,
            event_id: Some("$event:example.org".try_into().unwrap()),
            finalized_ts: Some(crate::UnixMillis(70103)),
        };
        assert_eq!(event.status(), DelayedEventStatus::Send);

        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "content": {"topic": "test topic"},
                "delay": 103,
                "delay_id": "a_delay_id",
                "event_id": "$event:example.org",
                "finalised_ts": 70103,
                "room_id": "!roomid:example.org",
                "running_since": 70000,
                "state_key": "a_state_key",
                "type": "m.room.topic",
            })
        );
    }

    #[test]
    fn scheduled_delayed_event_data_status() {
        let event = DelayedEventData {
            delay_id: "a_delay_id".to_owned(),
            room_id: "!roomid:example.org".try_into().unwrap(),
            event_type: "m.room.message".into(),
            state_key: None,
            content: json!({"msgtype": "m.text", "body": "test"}),
            delay: Duration::from_millis(103),
            running_since: crate::UnixMillis(70000),
            error: None,
            event_id: None,
            finalized_ts: None,
        };
        assert_eq!(event.status(), DelayedEventStatus::Scheduled);

        let serialized = serde_json::to_value(&event).unwrap();
        assert!(serialized.get("event_id").is_none());
        assert!(serialized.get("finalised_ts").is_none());
        assert!(serialized.get("error").is_none());
        assert!(serialized.get("state_key").is_none());
    }

    #[test]
    fn error_delayed_event_data_status() {
        let event = DelayedEventData {
            delay_id: "a_delay_id".to_owned(),
            room_id: "!roomid:example.org".try_into().unwrap(),
            event_type: "m.room.message".into(),
            state_key: None,
            content: json!({"msgtype": "m.text", "body": "test"}),
            delay: Duration::from_millis(103),
            running_since: crate::UnixMillis(70000),
            error: Some(DelayedEventError {
                errcode: "M_FORBIDDEN".to_owned(),
                error: Some("you shall not pass".to_owned()),
            }),
            event_id: None,
            finalized_ts: Some(crate::UnixMillis(70103)),
        };
        assert_eq!(event.status(), DelayedEventStatus::Error);
    }

    #[test]
    fn update_action_from_str() {
        assert_eq!(UpdateAction::from("restart"), UpdateAction::Restart);
        assert_eq!(UpdateAction::from("send"), UpdateAction::Send);
        assert_eq!(UpdateAction::from("cancel"), UpdateAction::Cancel);

        let body: UpdateDelayedEventReqBody =
            serde_json::from_value(json!({"action": "cancel"})).unwrap();
        assert_eq!(body.action, UpdateAction::Cancel);
    }

    #[test]
    fn cancelled_delayed_event_data_status() {
        let event = DelayedEventData {
            delay_id: "a_delay_id".to_owned(),
            room_id: "!roomid:example.org".try_into().unwrap(),
            event_type: "m.room.message".into(),
            state_key: None,
            content: JsonValue::Null,
            delay: Duration::from_millis(103),
            running_since: crate::UnixMillis(70000),
            error: None,
            event_id: None,
            finalized_ts: Some(crate::UnixMillis(70103)),
        };
        assert_eq!(event.status(), DelayedEventStatus::Cancel);
    }
}
