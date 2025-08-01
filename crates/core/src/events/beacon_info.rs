//! Types for the `org.matrix.msc3489.beacon_info` state event, the unstable
//! version of `m.beacon_info` ([MSC3489]).
//!
//! [MSC3489]: https://github.com/matrix-org/matrix-spec-proposals/pull/3489

use std::time::{Duration, SystemTime};

use crate::macros::EventContent;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::{OwnedUserId, UnixMillis, events::location::AssetContent};

/// The content of a beacon_info state.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize, EventContent)]
#[palpo_event(
    type = "org.matrix.msc3672.beacon_info", alias = "m.beacon_info", kind = State, state_key_type = OwnedUserId
)]
pub struct BeaconInfoEventContent {
    /// The description of the location.
    ///
    /// It should be used to label the location on a map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Whether the user starts sharing their location.
    pub live: bool,

    /// The time when location sharing started.
    #[serde(rename = "org.matrix.msc3488.ts")]
    pub ts: UnixMillis,

    /// The duration that the location sharing will be live.
    ///
    /// Meaning that the location will stop being shared at `ts + timeout`.
    #[serde(default, with = "crate::serde::duration::ms")]
    pub timeout: Duration,

    /// The asset that this message refers to.
    #[serde(default, rename = "org.matrix.msc3488.asset")]
    pub asset: AssetContent,
}

impl BeaconInfoEventContent {
    /// Creates a new `BeaconInfoEventContent` with the given description, live,
    /// timeout and optional ts. If ts is None, the current time will be
    /// used.
    pub fn new(
        description: Option<String>,
        timeout: Duration,
        live: bool,
        ts: Option<UnixMillis>,
    ) -> Self {
        Self {
            description,
            live,
            ts: ts.unwrap_or_else(UnixMillis::now),
            timeout,
            asset: Default::default(),
        }
    }

    /// Starts the beacon_info being live.
    pub fn start(&mut self) {
        self.live = true;
    }

    /// Stops the beacon_info from being live.
    pub fn stop(&mut self) {
        self.live = false;
    }

    /// Start time plus its timeout, it returns `false`, indicating that the
    /// beacon is not live. Otherwise, it returns `true`.
    pub fn is_live(&self) -> bool {
        self.live
            && self
                .ts
                .to_system_time()
                .and_then(|t| t.checked_add(self.timeout))
                .is_some_and(|t| t > SystemTime::now())
    }
}
