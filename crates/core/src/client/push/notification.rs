//! `GET /_matrix/client/*/notifications`
//!
//! Paginate through the list of events that the user has been, or would have
//! been notified about. `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3notifications
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::AnySyncTimelineEvent;
use crate::push::Action;
use crate::serde::RawJson;
use crate::{OwnedRoomId, UnixMillis};

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/notifications",
//         1.1 => "/_matrix/client/v3/notifications",
//     }
// };

#[derive(ToSchema, Default, Serialize, Deserialize)]
pub struct NotificationsReqBody {
    /// Pagination token given to retrieve the next set of events.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,

    /// Limit on the number of events to return in this request.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,

    /// Allows basic filtering of events returned.
    ///
    /// Supply "highlight" to return only events where the notification had the
    /// 'highlight' tweak set.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<String>,
}

// /// Request type for the `get_notifications` endpoint.

// #[derive(Default)]
// pub struct NotificationsReqBody {
//     /// Pagination token given to retrieve the next set of events.
//     #[salvo(parameter(parameter_in = Query))]
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub from: Option<String>,

//     /// Limit on the number of events to return in this request.
//     #[salvo(parameter(parameter_in = Query))]
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub limit: Option<usize>,

//     /// Allows basic filtering of events returned.
//     ///
//     /// Supply "highlight" to return only events where the notification had
// the 'highlight'     /// tweak set.
//     #[salvo(parameter(parameter_in = Query))]
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub only: Option<String>,
// }

/// Response type for the `get_notifications` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct NotificationsResBody {
    /// The token to supply in the from param of the next /notifications request
    /// in order to request more events.
    ///
    /// If this is absent, there are no more results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,

    /// The list of events that triggered notifications.
    pub notifications: Vec<Notification>,
}
impl NotificationsResBody {
    /// Creates a new `Response` with the given notifications.
    pub fn new(notifications: Vec<Notification>) -> Self {
        Self {
            next_token: None,
            notifications,
        }
    }
}

/// Represents a notification.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct Notification {
    /// The actions to perform when the conditions for this rule are met.
    pub actions: Vec<Action>,

    /// The event that triggered the notification.
    pub event: RawJson<AnySyncTimelineEvent>,

    /// The profile tag of the rule that matched this event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_tag: Option<String>,

    /// Indicates whether the user has sent a read receipt indicating that they
    /// have read this message.
    pub read: bool,

    /// The ID of the room in which the event was posted.
    pub room_id: OwnedRoomId,

    /// The time at which the event notification was sent.
    pub ts: UnixMillis,
}
