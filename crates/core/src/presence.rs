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
        }
    }
}
