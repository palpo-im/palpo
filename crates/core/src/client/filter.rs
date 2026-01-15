//! Endpoints for event filters.

// pub mod create_filter;
// pub mod get_filter;

mod lazy_load;
mod url;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

pub use self::lazy_load::LazyLoadOptions;
pub use self::url::UrlFilter;
use crate::serde::StringEnum;
use crate::{OwnedRoomId, OwnedUserId, PrivOwnedStr};

/// Format to use for returned events.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, Default, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventFormat {
    /// Client format, as described in the Client API.
    #[default]
    Client,

    /// Raw events from federation.
    Federation,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

/// Filters to be applied to room events.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct RoomEventFilter {
    /// A list of event types to exclude.
    ///
    /// If this list is absent then no event types are excluded. A matching type
    /// will be excluded even if it is listed in the 'types' filter. A '*'
    /// can be used as a wildcard to match any sequence of characters.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub not_types: Vec<String>,

    /// A list of room IDs to exclude.
    ///
    /// If this list is absent then no rooms are excluded. A matching room will
    /// be excluded even if it is listed in the 'rooms' filter.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub not_rooms: Vec<OwnedRoomId>,

    /// The maximum number of events to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,

    /// A list of room IDs to include.
    ///
    /// If this list is absent then all rooms are included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rooms: Option<Vec<OwnedRoomId>>,

    /// A list of sender IDs to exclude.
    ///
    /// If this list is absent then no senders are excluded. A matching sender
    /// will be excluded even if it is listed in the 'senders' filter.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub not_senders: Vec<OwnedUserId>,

    /// A list of senders IDs to include.
    ///
    /// If this list is absent then all senders are included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub senders: Option<Vec<OwnedUserId>>,

    /// A list of event types to include.
    ///
    /// If this list is absent then all event types are included. A '*' can be
    /// used as a wildcard to match any sequence of characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,

    /// Controls whether to include events with a URL key in their content.
    ///
    /// * `None`: No filtering
    /// * `Some(EventsWithUrl)`: Only events with a URL
    /// * `Some(EventsWithoutUrl)`: Only events without a URL
    #[serde(
        default,
        rename = "contains_url",
        skip_serializing_if = "Option::is_none"
    )]
    pub url_filter: Option<UrlFilter>,

    /// Options to control lazy-loading of membership events.
    ///
    /// Defaults to `LazyLoadOptions::Disabled`.
    #[serde(flatten)]
    pub lazy_load_options: LazyLoadOptions,

    /// Whether to enable [per-thread notification counts].
    ///
    /// Only applies to the [`sync_events`] endpoint.
    ///
    /// [per-thread notification counts]: https://spec.matrix.org/latest/client-server-api/#receiving-notifications
    /// [`sync_events`]: crate::sync::sync_events
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub unread_thread_notifications: bool,
}

impl RoomEventFilter {
    /// Creates an empty `RoomEventFilter`.
    ///
    /// You can also use the [`Default`] implementation.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Creates a new `RoomEventFilter` that can be used to ignore all room
    /// events.
    pub fn ignore_all() -> Self {
        Self {
            types: Some(vec![]),
            ..Default::default()
        }
    }

    /// Creates a new `RoomEventFilter` with [room member lazy-loading] enabled.
    ///
    /// Redundant membership events are disabled.
    ///
    /// [room member lazy-loading]: https://spec.matrix.org/latest/client-server-api/#lazy-loading-room-members
    pub fn with_lazy_loading() -> Self {
        Self {
            lazy_load_options: LazyLoadOptions::Enabled {
                include_redundant_members: false,
            },
            ..Default::default()
        }
    }

    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            ..Default::default()
        }
    }

    /// Returns `true` if all fields are empty.
    pub fn is_empty(&self) -> bool {
        self.not_types.is_empty()
            && self.not_rooms.is_empty()
            && self.limit.is_none()
            && self.rooms.is_none()
            && self.not_senders.is_empty()
            && self.senders.is_none()
            && self.types.is_none()
            && self.url_filter.is_none()
            && self.lazy_load_options.is_disabled()
            && !self.unread_thread_notifications
    }
}

/// Filters to be applied to room data.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct RoomDataFilter {
    /// Include rooms that the user has left in the sync.
    ///
    /// Defaults to `false`.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub include_leave: bool,

    /// The per user account data to include for rooms.
    #[serde(default, skip_serializing_if = "crate::serde::is_empty")]
    pub account_data: RoomEventFilter,

    /// The message and state update events to include for rooms.
    #[serde(default, skip_serializing_if = "crate::serde::is_empty")]
    pub timeline: RoomEventFilter,

    /// The events that aren't recorded in the room history, e.g. typing and
    /// receipts, to include for rooms.
    #[serde(default, skip_serializing_if = "crate::serde::is_empty")]
    pub ephemeral: RoomEventFilter,

    /// The state events to include for rooms.
    #[serde(default, skip_serializing_if = "crate::serde::is_empty")]
    pub state: RoomEventFilter,

    /// A list of room IDs to exclude.
    ///
    /// If this list is absent then no rooms are excluded. A matching room will
    /// be excluded even if it is listed in the 'rooms' filter. This filter
    /// is applied before the filters in `ephemeral`, `state`, `timeline` or
    /// `account_data`.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub not_rooms: Vec<OwnedRoomId>,

    /// A list of room IDs to include.
    ///
    /// If this list is absent then all rooms are included. This filter is
    /// applied before the filters in `ephemeral`, `state`, `timeline` or
    /// `account_data`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rooms: Option<Vec<OwnedRoomId>>,
}

impl RoomDataFilter {
    /// Creates an empty `RoomDataFilter`.
    ///
    /// You can also use the [`Default`] implementation.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Creates a new `RoomDataFilter` that can be used to ignore all room
    /// events (of any type).
    pub fn ignore_all() -> Self {
        Self {
            rooms: Some(vec![]),
            ..Default::default()
        }
    }

    /// Creates a new `RoomDataFilter` with [room member lazy-loading] enabled.
    ///
    /// Redundant membership events are disabled.
    ///
    /// [room member lazy-loading]: https://spec.matrix.org/latest/client-server-api/#lazy-loading-room-members
    pub fn with_lazy_loading() -> Self {
        Self {
            state: RoomEventFilter::with_lazy_loading(),
            ..Default::default()
        }
    }

    /// Returns `true` if all fields are empty.
    pub fn is_empty(&self) -> bool {
        !self.include_leave
            && self.account_data.is_empty()
            && self.timeline.is_empty()
            && self.ephemeral.is_empty()
            && self.state.is_empty()
            && self.not_rooms.is_empty()
            && self.rooms.is_none()
    }
}

/// Filter for non-room data.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct NonRoomDataFilter {
    /// A list of event types to exclude.
    ///
    /// If this list is absent then no event types are excluded. A matching type
    /// will be excluded even if it is listed in the 'types' filter. A '*'
    /// can be used as a wildcard to match any sequence of characters.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub not_types: Vec<String>,

    /// The maximum number of events to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,

    /// A list of senders IDs to include.
    ///
    /// If this list is absent then all senders are included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub senders: Option<Vec<OwnedUserId>>,

    /// A list of event types to include.
    ///
    /// If this list is absent then all event types are included. A '*' can be
    /// used as a wildcard to match any sequence of characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,

    /// A list of sender IDs to exclude.
    ///
    /// If this list is absent then no senders are excluded. A matching sender
    /// will be excluded even if it is listed in the 'senders' filter.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub not_senders: Vec<OwnedUserId>,
}

impl NonRoomDataFilter {
    /// Creates an empty `Filter`.
    ///
    /// You can also use the [`Default`] implementation.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Creates a new `Filter` that can be used to ignore all events.
    pub fn ignore_all() -> Self {
        Self {
            types: Some(vec![]),
            ..Default::default()
        }
    }

    /// Returns `true` if all fields are empty.
    pub fn is_empty(&self) -> bool {
        self.not_types.is_empty()
            && self.limit.is_none()
            && self.senders.is_none()
            && self.types.is_none()
            && self.not_senders.is_empty()
    }
}

/// A filter definition
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct FilterDefinition {
    /// List of event fields to include.
    ///
    /// If this list is absent then all fields are included. The entries may
    /// include '.' characters to indicate sub-fields. So ['content.body']
    /// will include the 'body' field of the 'content' object. A literal '.'
    /// or '\' character in a field name may be escaped using a '\'. A server
    /// may include more fields than were requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_fields: Option<Vec<String>>,

    /// The format to use for events.
    ///
    /// 'client' will return the events in a format suitable for clients.
    /// 'federation' will return the raw event as received over federation.
    /// The default is 'client'.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub event_format: EventFormat,

    /// The presence updates to include.
    #[serde(default, skip_serializing_if = "crate::serde::is_empty")]
    pub presence: NonRoomDataFilter,

    /// The user account data that isn't associated with rooms to include.
    #[serde(default, skip_serializing_if = "crate::serde::is_empty")]
    pub account_data: NonRoomDataFilter,

    /// Filters to be applied to room data.
    #[serde(default, skip_serializing_if = "crate::serde::is_empty")]
    pub room: RoomDataFilter,
}

impl FilterDefinition {
    /// Creates an empty `FilterDefinition`.
    ///
    /// You can also use the [`Default`] implementation.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Creates a new `FilterDefinition` that can be used to ignore all events.
    pub fn ignore_all() -> Self {
        Self {
            account_data: NonRoomDataFilter::ignore_all(),
            room: RoomDataFilter::ignore_all(),
            presence: NonRoomDataFilter::ignore_all(),
            ..Default::default()
        }
    }

    /// Creates a new `FilterDefinition` with [room member lazy-loading]
    /// enabled.
    ///
    /// Redundant membership events are disabled.
    ///
    /// [room member lazy-loading]: https://spec.matrix.org/latest/client-server-api/#lazy-loading-room-members
    pub fn with_lazy_loading() -> Self {
        Self {
            room: RoomDataFilter::with_lazy_loading(),
            ..Default::default()
        }
    }

    /// Returns `true` if all fields are empty.
    pub fn is_empty(&self) -> bool {
        self.event_fields.is_none()
            && self.event_format == EventFormat::Client
            && self.presence.is_empty()
            && self.account_data.is_empty()
            && self.room.is_empty()
    }
}
// /// `POST /_matrix/client/*/user/{user_id}/filter`
// ///
// /// Create a new filter for event retrieval.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3useruser_idfilter
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/user/:user_id/filter",
//         1.1 => "/_matrix/client/v3/user/:user_id/filter",
//     }
// };

/// Request type for the `create_filter` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct CreateFilterReqBody {
    // /// The ID of the user uploading the filter.
    // ///
    // /// The access token must be authorized to make requests for this user ID.
    // #[salvo(parameter(parameter_in = Path))]
    // pub user_id: OwnedUserId,
    /// The filter definition.
    #[serde(flatten)]
    pub filter: FilterDefinition,
}

/// Response type for the `create_filter` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct CreateFilterResBody {
    /// The ID of the filter that was created.
    pub filter_id: String,
}
impl CreateFilterResBody {
    /// Creates a new `Response` with the given filter ID.
    pub fn new(filter_id: String) -> Self {
        Self { filter_id }
    }
}

// /// `GET /_matrix/client/*/user/{user_id}/filter/{filter_id}`
// ///
// /// Retrieve a previously created filter.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3useruser_idfilterfilterid
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/user/:user_id/filter/:filter_id",
//         1.1 => "/_matrix/client/v3/user/:user_id/filter/:filter_id",
//     }
// };

/// Response type for the `get_filter` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct FilterResBody(
    /// The filter definition.
    pub FilterDefinition,
);
impl FilterResBody {
    /// Creates a new `Response` with the given filter definition.
    pub fn new(filter: FilterDefinition) -> Self {
        Self(filter)
    }
}

macro_rules! can_be_empty {
    ($ty:ident) => {
        impl crate::serde::CanBeEmpty for $ty {
            fn is_empty(&self) -> bool {
                self.is_empty()
            }
        }
    };
}

can_be_empty!(RoomDataFilter);
can_be_empty!(FilterDefinition);
can_be_empty!(RoomEventFilter);
can_be_empty!(NonRoomDataFilter);

// #[cfg(test)]
// mod tests {
//     use serde_json::{from_value as from_json_value, json, to_value as
// to_json_value};

//     use super::{Filter, FilterDefinition, LazyLoadOptions, RoomDataFilter,
// RoomEventFilter, UrlFilter};

//     #[test]
//     fn deserialize_request() {
//         crateapi::IncomingRequest as _;

//         use super::Request;

//         let req = Request::try_from_http_request(
//             http::Request::builder()
//                 .method(http::Method::POST)
//                 .uri("https://matrix.org/_matrix/client/r0/user/@foo:bar.com/filter")
//                 .body(b"{}" as &[u8])
//                 .unwrap(),
//             &["@foo:bar.com"],
//         )
//         .unwrap();

//         assert_eq!(req.user_id, "@foo:bar.com");
//         assert!(req.filter.is_empty());
//     }

//     #[test]
//     fn serialize_request() {
//         use crate::{
//             api::{MatrixVersion, OutgoingRequest, SendAccessToken},
//             owned_user_id,
//         };

//         use crate::client::filter::FilterDefinition;

//         let req = super::Request::new(owned_user_id!("@foo:bar.com"),
// FilterDefinition::default())             .try_into_http_request::<Vec<u8>>(
//                 "https://matrix.org",
//                 SendAccessToken::IfRequired("tok"),
//                 &[MatrixVersion::V1_1],
//             )
//             .unwrap();
//         assert_eq!(req.body(), b"{}");
//     }

//     #[test]
//     fn default_filters_are_empty() -> serde_json::Result<()> {
//         assert_eq!(to_json_value(NonRoomDataFilter::default())?, json!({}));
//         assert_eq!(to_json_value(FilterDefinition::default())?, json!({}));
//         assert_eq!(to_json_value(RoomEventFilter::default())?, json!({}));
//         assert_eq!(to_json_value(RoomDataFilter::default())?, json!({}));

//         Ok(())
//     }

//     #[test]
//     fn filter_definition_roundtrip() -> serde_json::Result<()> {
//         let filter = FilterDefinition::default();
//         let filter_str = to_json_value(&filter)?;

//         let incoming_filter =
// from_json_value::<FilterDefinition>(filter_str)?;         assert!
// (incoming_filter.is_empty());

//         Ok(())
//     }

//     #[test]
//     fn room_filter_definition_roundtrip() -> serde_json::Result<()> {
//         let filter = RoomDataFilter::default();
//         let room_filter = to_json_value(filter)?;

//         let incoming_room_filter =
// from_json_value::<RoomDataFilter>(room_filter)?;         assert!
// (incoming_room_filter.is_empty());

//         Ok(())
//     }

//     #[test]
//     fn issue_366() {
//         let obj = json!({
//             "lazy_load_members": true,
//             "filter_json": { "contains_url": true, "types":
// ["m.room.message"] },             "types": ["m.room.message"],
//             "not_types": [],
//             "rooms": null,
//             "not_rooms": [],
//             "senders": null,
//             "not_senders": [],
//             "contains_url": true,
//         });

//         let filter: RoomEventFilter = from_json_value(obj).unwrap();

//         assert_eq!(filter.types, Some(vec!["m.room.message".to_owned()]));
//         assert_eq!(filter.not_types, vec![""; 0]);
//         assert_eq!(filter.rooms, None);
//         assert_eq!(filter.not_rooms, vec![""; 0]);
//         assert_eq!(filter.senders, None);
//         assert_eq!(filter.not_senders, vec![""; 0]);
//         assert_eq!(filter.limit, None);
//         assert_eq!(filter.url_filter, Some(UrlFilter::EventsWithUrl));
//         assert_eq!(
//             filter.lazy_load_options,
//             LazyLoadOptions::Enabled {
//                 include_redundant_members: false
//             }
//         );
//     }
// }
