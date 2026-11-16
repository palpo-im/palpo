//! `GET /_matrix/client/*/sync`
//!
//! Get all new events from all rooms since the last sync or a given point in
//! time.

use salvo::prelude::*;
use serde::{self, Deserialize, Serialize};

pub mod v3;
#[cfg(feature = "unstable-msc4186")]
pub mod v4;
pub mod v5;

/// Unread notifications count.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct UnreadNotificationsCount {
    /// The number of unread notifications with the highlight flag set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight_count: Option<u64>,

    /// The total number of unread notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_count: Option<u64>,
}

impl UnreadNotificationsCount {
    /// Creates an empty `UnreadNotificationsCount`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no notification count updates.
    pub fn is_empty(&self) -> bool {
        self.highlight_count.is_none() && self.notification_count.is_none()
    }
}
