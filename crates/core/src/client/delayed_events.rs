//! Endpoints for sending and interacting with delayed events.
//!
//! Delayed events are an unstable feature added by [MSC4140].
//!
//! [MSC4140]: https://github.com/matrix-org/matrix-spec-proposals/pull/4140

use std::collections::BTreeMap;
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

    /// Additional fields carried by the Matrix standard error response.
    #[serde(flatten)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub extra: BTreeMap<String, JsonValue>,
}

/// How a delayed event was finalized.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct DelayedEventFinalization {
    /// The error that prevented the delayed event from being sent.
    ///
    /// Present only when sending failed. It is mutually exclusive with
    /// [`event_id`](Self::event_id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<DelayedEventError>,

    /// The event ID allocated when the delayed event was sent successfully.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<OwnedEventId>,

    /// The time at which the delayed event was finalized.
    pub finalised_ts: UnixMillis,
}

/// The structure returned by the delayed-event lookup endpoint.
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
    #[serde(rename = "delay_ms", with = "crate::serde::duration::ms")]
    #[salvo(schema(value_type = u64))]
    pub delay: Duration,

    /// The timestamp when the delayed event was scheduled or last restarted.
    #[serde(rename = "delayed_since_ts")]
    pub running_since: UnixMillis,

    /// How this event was finalized. Absent while it remains scheduled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalised: Option<DelayedEventFinalization>,
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
    #[serde(rename = "delay_ms", with = "crate::serde::duration::ms")]
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    #[test]
    fn deserialize_send_delayed_event_req_body() {
        let body: SendDelayedEventReqBody = serde_json::from_value(json!({
            "delay_ms": 103,
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
            "delay_ms": 9000,
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
            finalised: Some(DelayedEventFinalization {
                error: None,
                event_id: Some("$event:example.org".try_into().unwrap()),
                finalised_ts: crate::UnixMillis(70103),
            }),
        };

        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "content": {"topic": "test topic"},
                "delay_ms": 103,
                "delay_id": "a_delay_id",
                "delayed_since_ts": 70000,
                "finalised": {
                    "event_id": "$event:example.org",
                    "finalised_ts": 70103,
                },
                "room_id": "!roomid:example.org",
                "state_key": "a_state_key",
                "type": "m.room.topic",
            })
        );
    }

    #[test]
    fn scheduled_delayed_event_omits_finalisation() {
        let event = DelayedEventData {
            delay_id: "a_delay_id".to_owned(),
            room_id: "!roomid:example.org".try_into().unwrap(),
            event_type: "m.room.message".into(),
            state_key: None,
            content: json!({"msgtype": "m.text", "body": "test"}),
            delay: Duration::from_millis(103),
            running_since: crate::UnixMillis(70000),
            finalised: None,
        };

        let serialized = serde_json::to_value(&event).unwrap();
        assert!(serialized.get("finalised").is_none());
        assert!(serialized.get("state_key").is_none());
    }

    #[test]
    fn failed_finalisation_nests_the_standard_error() {
        let finalised = DelayedEventFinalization {
            error: Some(DelayedEventError {
                errcode: "M_FORBIDDEN".to_owned(),
                error: Some("you shall not pass".to_owned()),
                extra: BTreeMap::new(),
            }),
            event_id: None,
            finalised_ts: crate::UnixMillis(70103),
        };

        assert_eq!(
            serde_json::to_value(finalised).unwrap(),
            json!({
                "error": {
                    "errcode": "M_FORBIDDEN",
                    "error": "you shall not pass",
                },
                "finalised_ts": 70103,
            })
        );
    }

    #[test]
    fn delayed_event_error_preserves_standard_error_extensions() {
        let error: DelayedEventError = serde_json::from_value(json!({
            "errcode": "M_LIMIT_EXCEEDED",
            "error": "slow down",
            "retry_after_ms": 1500,
        }))
        .unwrap();

        assert_eq!(error.extra.get("retry_after_ms"), Some(&json!(1500)));
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            json!({
                "errcode": "M_LIMIT_EXCEEDED",
                "error": "slow down",
                "retry_after_ms": 1500,
            })
        );
    }

    #[test]
    fn update_action_from_str() {
        assert_eq!(UpdateAction::from("restart"), UpdateAction::Restart);
        assert_eq!(UpdateAction::from("send"), UpdateAction::Send);
        assert_eq!(UpdateAction::from("cancel"), UpdateAction::Cancel);
    }

    #[test]
    fn cancelled_finalisation_has_only_its_timestamp() {
        let finalised = DelayedEventFinalization {
            error: None,
            event_id: None,
            finalised_ts: crate::UnixMillis(70103),
        };

        assert_eq!(
            serde_json::to_value(finalised).unwrap(),
            json!({"finalised_ts": 70103})
        );
    }
}
