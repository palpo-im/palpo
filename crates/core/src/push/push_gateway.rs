//! `GET /_matrix/client/*/notifications`
//!
//! Paginate through the list of events that the user has been, or would have
//! been notified about. `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3notifications
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::TimelineEventType;
use crate::identifiers::*;
use crate::push::{PusherData, Tweak};
use crate::serde::{RawJsonValue, StringEnum};
use crate::{PrivOwnedStr, UnixSeconds};

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/push/v1/notify",
//     }
// };

#[derive(ToSchema, Serialize, Debug)]
pub struct SendEventNotificationReqBody {
    /// Information about the push notification
    pub notification: Notification,
}
crate::json_body_modifier!(SendEventNotificationReqBody);
impl SendEventNotificationReqBody {
    pub fn new(notification: Notification) -> Self {
        Self { notification }
    }
}

/// Response type for the `send_event_notification` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct SendEventNotificationResBody {
    /// A list of all pushkeys given in the notification request that are not
    /// valid.
    ///
    /// These could have been rejected by an upstream gateway because they have
    /// expired or have never been valid. Homeservers must cease sending
    /// notification requests for these pushkeys and remove the associated
    /// pushers. It may not necessarily be the notification in the request
    /// that failed: it could be that a previous notification to the same
    /// pushkey failed. May be empty.
    pub rejected: Vec<String>,
}
impl SendEventNotificationResBody {
    /// Creates a new `Response` with the given list of rejected pushkeys.
    pub fn new(rejected: Vec<String>) -> Self {
        Self { rejected }
    }
}

/// Represents a notification.
#[derive(ToSchema, Default, Deserialize, Serialize, Clone, Debug)]
pub struct Notification {
    /// The Matrix event ID of the event being notified about.
    ///
    /// Required if the notification is about a particular Matrix event. May be
    /// omitted for notifications that only contain updated badge counts.
    /// This ID can and should be used to detect duplicate notification
    /// requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<OwnedEventId>,

    /// The ID of the room in which this event occurred.
    ///
    /// Required if the notification relates to a specific Matrix event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_id: Option<OwnedRoomId>,

    /// The type of the event as in the event's `type` field.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub event_type: Option<TimelineEventType>,

    /// The sender of the event as in the corresponding event field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<OwnedUserId>,

    /// The current display name of the sender in the room in which the event
    /// occurred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_display_name: Option<String>,

    /// The name of the room in which the event occurred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_name: Option<String>,

    /// An alias to display for the room in which the event occurred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_alias: Option<OwnedRoomAliasId>,

    /// Whether the user receiving the notification is the subject of a member
    /// event (i.e. the `state_key` of the member event is equal to the
    /// user's Matrix ID).
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub user_is_target: bool,

    /// The priority of the notification.
    ///
    /// If omitted, `high` is assumed. This may be used by push gateways to
    /// deliver less time-sensitive notifications in a way that will
    /// preserve battery power on mobile devices.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub prio: NotificationPriority,

    /// The `content` field from the event, if present.
    ///
    /// The pusher may omit this if the event had no content or for any other
    /// reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[salvo(schema(value_type = Object))]
    pub content: Option<Box<RawJsonValue>>,

    /// Current number of unacknowledged communications for the recipient user.
    ///
    /// Counts whose value is zero should be omitted.
    #[serde(default, skip_serializing_if = "NotificationCounts::is_default")]
    pub counts: NotificationCounts,

    /// An array of devices that the notification should be sent to.
    pub devices: Vec<Device>,
}
impl Notification {
    /// Create a new notification for the given devices.
    pub fn new(devices: Vec<Device>) -> Self {
        Notification {
            devices,
            ..Default::default()
        }
    }
}

/// Type for passing information about notification priority.
///
/// This may be used by push gateways to deliver less time-sensitive
/// notifications in a way that will preserve battery power on mobile devices.
///
/// This type can hold an arbitrary string. To build this with a custom value,
/// convert it from a string with `::from()` / `.into()`. To check for values
/// that are not available as a documented variant here, use its string
/// representation, obtained through `.as_str()`.
#[derive(ToSchema, Clone, Default, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NotificationPriority {
    /// A high priority notification
    #[default]
    High,

    /// A low priority notification
    Low,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// Type for passing information about notification counts.
#[derive(ToSchema, Deserialize, Serialize, Default, Clone, Debug)]
pub struct NotificationCounts {
    /// The number of unread messages a user has across all of the rooms they
    /// are a member of.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub unread: u64,

    /// The number of unacknowledged missed calls a user has across all rooms of
    /// which they are a member.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub missed_calls: u64,
}

impl NotificationCounts {
    /// Create new notification counts from the given unread and missed call
    /// counts.
    pub fn new(unread: u64, missed_calls: u64) -> Self {
        NotificationCounts {
            unread,
            missed_calls,
        }
    }

    fn is_default(&self) -> bool {
        self.unread == 0 && self.missed_calls == 0
    }
}

/// Type for passing information about devices.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct Device {
    /// The `app_id` given when the pusher was created.
    ///
    /// Max length: 64 chars.
    pub app_id: String,

    /// The `pushkey` given when the pusher was created.
    ///
    /// Max length: 512 bytes.
    pub pushkey: String,

    /// The unix timestamp (in seconds) when the pushkey was last updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pushkey_ts: Option<UnixSeconds>,

    /// A dictionary of additional pusher-specific data.
    #[serde(default, skip_serializing_if = "PusherData::is_empty")]
    pub data: PusherData,

    /// A dictionary of customisations made to the way this notification is to
    /// be presented.
    ///
    /// These are added by push rules.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tweaks: Vec<Tweak>,
}

impl Device {
    /// Create a new device with the given app id and pushkey
    pub fn new(app_id: String, pushkey: String) -> Self {
        Device {
            app_id,
            pushkey,
            pushkey_ts: None,
            data: PusherData::new(),
            tweaks: Vec::new(),
        }
    }
}
