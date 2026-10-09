//! MSC1763 retention configuration discovery.
use std::collections::BTreeMap;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::events::room::retention::RoomRetentionEventContent;

/// Bounds applied to a room's lifetime property, in milliseconds.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct LifetimeLimits {
    /// Minimum permitted value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<u64>,
    /// Maximum permitted value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<u64>,
}

/// Server limits on room retention policies.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct RetentionLimits {
    /// Bounds for min_lifetime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_lifetime: Option<LifetimeLimits>,
    /// Bounds for max_lifetime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_lifetime: Option<LifetimeLimits>,
}

/// Authenticated retention configuration, using `*` for the operator default.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct RetentionConfigurationResBody {
    /// Operator policies, including per-room overrides visible to the caller.
    pub policies: BTreeMap<String, RoomRetentionEventContent>,
    /// Bounds applied to policies supplied by rooms.
    pub limits: RetentionLimits,
}
