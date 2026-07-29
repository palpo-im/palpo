//! Common types for the [presence module][presence].
//!
//! [presence]: https://spec.matrix.org/latest/client-server-api/#presence
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::serde::StringEnum;
use crate::{OwnedUserId, PrivOwnedStr};

/// A description of a user's connectivity and availability for chat.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, Default, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PresenceState {
    /// Disconnected from the service.
    Offline,

    /// Connected to the service.
    #[default]
    Online,

    /// Connected to the service but not available for chat.
    Unavailable,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

impl Default for &'_ PresenceState {
    fn default() -> Self {
        &PresenceState::Online
    }
}

/// The content for "m.presence" Edu.

#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct PresenceContent {
    /// A list of presence updates that the receiving server is likely to be
    /// interested in.
    pub push: Vec<PresenceUpdate>,
}

impl PresenceContent {
    /// Creates a new `PresenceContent`.
    pub fn new(push: Vec<PresenceUpdate>) -> Self {
        Self { push }
    }
}

/// An update to the presence of a user.

#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct PresenceUpdate {
    /// The user ID this presence EDU is for.
    pub user_id: OwnedUserId,

    /// The presence of the user.
    pub presence: PresenceState,

    /// An optional description to accompany the presence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_msg: Option<String>,

    /// The number of milliseconds that have elapsed since the user last did
    /// something.
    pub last_active_ago: u64,

    /// Whether or not the user is currently active.
    ///
    /// Defaults to false.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub currently_active: bool,

    /// Changes to the user's presence recipient list since the previous update.
    #[cfg(feature = "unstable-msc4495")]
    #[serde(
        default,
        skip_serializing_if = "PresenceRecipientListUpdates::is_empty"
    )]
    pub recipients: PresenceRecipientListUpdates,

    /// The stream ID of the user's current presence recipient list.
    #[cfg(feature = "unstable-msc4495")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<i64>,

    /// The previous stream ID for this recipient-list delta.
    #[cfg(feature = "unstable-msc4495")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_id: Option<i64>,
}

impl PresenceUpdate {
    /// Creates a new `PresenceUpdate` with the given `user_id`, `presence` and
    /// `last_activity`.
    pub fn new(user_id: OwnedUserId, presence: PresenceState, last_activity: u64) -> Self {
        Self {
            user_id,
            presence,
            last_active_ago: last_activity,
            status_msg: None,
            currently_active: false,
            #[cfg(feature = "unstable-msc4495")]
            recipients: PresenceRecipientListUpdates::default(),
            #[cfg(feature = "unstable-msc4495")]
            stream_id: None,
            #[cfg(feature = "unstable-msc4495")]
            prev_id: None,
        }
    }
}

/// Added and removed users in a presence recipient list.
#[cfg(feature = "unstable-msc4495")]
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, Default)]
pub struct PresenceRecipientListUpdates {
    /// Users added to the recipient list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add: Vec<OwnedUserId>,

    /// Users removed from the recipient list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delete: Vec<OwnedUserId>,
}

#[cfg(feature = "unstable-msc4495")]
impl PresenceRecipientListUpdates {
    /// Creates recipient-list updates from added and removed users.
    pub fn new(add: Vec<OwnedUserId>, delete: Vec<OwnedUserId>) -> Self {
        Self { add, delete }
    }

    /// Whether this update contains no changes.
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.delete.is_empty()
    }
}

#[cfg(all(test, feature = "unstable-msc4495"))]
mod msc4495_tests {
    use serde_json::json;

    use super::{PresenceRecipientListUpdates, PresenceState, PresenceUpdate};
    use crate::owned_user_id;

    #[test]
    fn presence_recipient_delta_round_trips() {
        let update = PresenceUpdate {
            user_id: owned_user_id!("@alice:example.org"),
            presence: PresenceState::Online,
            status_msg: None,
            last_active_ago: 1_000,
            currently_active: true,
            recipients: PresenceRecipientListUpdates::new(
                vec![owned_user_id!("@bob:example.org")],
                vec![owned_user_id!("@charlie:example.org")],
            ),
            stream_id: Some(321),
            prev_id: Some(123),
        };

        let json = serde_json::to_value(&update).unwrap();
        assert_eq!(
            json,
            json!({
                "user_id": "@alice:example.org",
                "presence": "online",
                "last_active_ago": 1_000,
                "currently_active": true,
                "recipients": {
                    "add": ["@bob:example.org"],
                    "delete": ["@charlie:example.org"]
                },
                "stream_id": 321,
                "prev_id": 123
            })
        );

        let parsed: PresenceUpdate = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.stream_id, Some(321));
        assert_eq!(parsed.prev_id, Some(123));
        assert_eq!(parsed.recipients.add.len(), 1);
        assert_eq!(parsed.recipients.delete.len(), 1);
    }

    #[test]
    fn legacy_presence_update_omits_msc4495_fields() {
        let update = PresenceUpdate::new(
            owned_user_id!("@alice:example.org"),
            PresenceState::Online,
            1_000,
        );

        let json = serde_json::to_value(update).unwrap();
        assert!(json.get("recipients").is_none());
        assert!(json.get("stream_id").is_none());
        assert!(json.get("prev_id").is_none());
    }
}
