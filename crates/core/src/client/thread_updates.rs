//! Experimental MSC4360 cross-room thread-update wire contracts.
//!
//! Adapted from Ruma ea7c5471cf46bb04be3efeac4d9ee4c937842446; its MIT notice is
//! retained in `sync_events/v4/LICENSE-ruma`. These models do not register routes,
//! advertise support, implement cursors, or authorize access to thread roots.

use std::collections::BTreeMap;

use js_int::UInt;
use serde::{Deserialize, Serialize};

use crate::events::AnySyncTimelineEvent;
use crate::serde::RawJson;
use crate::{Direction, MatrixError, MatrixResult, OwnedEventId, OwnedRoomId};

/// Authenticated GET endpoint, pending server implementation.
pub const PATH: &str = "/_matrix/client/unstable/io.element.msc4360/thread_updates";
/// Name of the optional sync extension.
pub const EXTENSION: &str = "io.element.msc4360.threads";

/// Per-request thread extension configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ThreadsConfig {
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub enabled: bool,
    /// Include roots even for updates already presented in a room timeline.
    pub include_roots: bool,
    /// Requested maximum; omission means the protocol default of 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<UInt>,
}

impl ThreadsConfig {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }
}

/// Thread updates in a sync response. An omitted root is meaningful when
/// `include_roots` was false; a pagination-only response is also meaningful.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Threads {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub updates: BTreeMap<OwnedRoomId, BTreeMap<OwnedEventId, ThreadUpdate>>,
    /// Cross-room pagination cursor for updates omitted from this sync batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_batch: Option<String>,
}

impl Threads {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.updates.is_empty() && self.prev_batch.is_none()
    }
}

/// A sync update, including an optional root with bundled `m.thread` aggregation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ThreadUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_root: Option<RawJson<AnySyncTimelineEvent>>,
    /// Cursor for this thread's `/relations?dir=b`, not `/thread_updates`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_batch: Option<String>,
}

impl ThreadUpdate {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Query parameters for the cross-room pagination endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadUpdatesReqArgs {
    // Retained on deserialization so a handler can reject unsupported directions.
    dir: Direction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<UInt>,
}

impl Default for ThreadUpdatesReqArgs {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadUpdatesReqArgs {
    pub fn new() -> Self {
        Self {
            dir: Direction::Backward,
            from: None,
            to: None,
            limit: None,
        }
    }

    /// A handler must call this before reading or advancing a cursor.
    pub fn validate(&self) -> MatrixResult<()> {
        if self.dir != Direction::Backward {
            return Err(MatrixError::invalid_param(
                "Thread updates support only dir=b",
            ));
        }
        if self.limit.is_some_and(|limit| limit == UInt::MIN) {
            return Err(MatrixError::invalid_param(
                "Thread updates limit must be greater than zero",
            ));
        }
        Ok(())
    }
}

/// Unlike sync updates, pagination always includes each root and its aggregation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PaginatedThreadUpdate {
    pub thread_root: RawJson<AnySyncTimelineEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_batch: Option<String>,
}

impl PaginatedThreadUpdate {
    pub fn new(thread_root: RawJson<AnySyncTimelineEvent>) -> Self {
        Self {
            thread_root,
            prev_batch: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThreadUpdatesResBody {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chunk: BTreeMap<OwnedRoomId, BTreeMap<OwnedEventId, PaginatedThreadUpdate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_batch: Option<String>,
}

#[cfg(test)]
mod tests;
