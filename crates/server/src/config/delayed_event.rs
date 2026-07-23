use serde::Deserialize;

use crate::core::serde::default_true;
use crate::macros::config_example;

#[config_example(filename = "palpo-example.toml", section = "delayed_events")]
#[derive(Clone, Debug, Deserialize)]
pub struct DelayedEventsConfig {
    /// Allow scheduling MSC4140 delayed events.
    ///
    /// Delayed events let clients schedule message or state events that the
    /// server sends into a room after a delay, e.g. reliable MatrixRTC
    /// "hang up" events. When disabled the endpoints are not registered and
    /// the feature is not advertised.
    #[serde(default = "default_true")]
    pub enable: bool,

    /// The maximum delay in milliseconds a client may request for a delayed
    /// event. Requests above this limit are rejected with `M_FORBIDDEN`.
    /// Defaults to 24 hours.
    ///
    /// default: 86400_000
    #[serde(default = "default_max_delay_ms")]
    pub max_delay_ms: u64,

    /// How many delayed events a user may have scheduled at once. Requests
    /// above this limit are rejected with `M_LIMIT_EXCEEDED`.
    /// Defaults to 100.
    ///
    /// default: 100
    #[serde(default = "default_max_scheduled")]
    pub max_scheduled: u64,

    /// How long finalized (sent, cancelled, or errored) delayed events are
    /// retained for lookup before they are pruned, in milliseconds.
    /// Defaults to 7 days.
    ///
    /// default: 604800_000
    #[serde(default = "default_retention_ms")]
    pub retention_ms: u64,
}

impl Default for DelayedEventsConfig {
    fn default() -> Self {
        Self {
            enable: true,
            max_delay_ms: default_max_delay_ms(),
            max_scheduled: default_max_scheduled(),
            retention_ms: default_retention_ms(),
        }
    }
}

fn default_max_delay_ms() -> u64 {
    24 * 60 * 60_000
}

fn default_max_scheduled() -> u64 {
    100
}

fn default_retention_ms() -> u64 {
    7 * 24 * 60 * 60_000
}
