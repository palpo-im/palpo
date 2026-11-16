//! Shared wire models for experimental [MSC4560] stripped-state queries.
//!
//! These types do not implement room visibility, authentication, routing or
//! operational limits. A server must validate requests before alias resolution
//! or federation lookup and authorize access before returning any state/counts.
//!
//! [MSC4560]: https://github.com/matrix-org/matrix-spec-proposals/pull/4560

use std::collections::BTreeSet;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::{ALLOWED_STRIPPED_STATE_EVENTS, StateEventType};
use crate::{MatrixError, MatrixResult};

/// The client query's maximum number of `via` entries, before deduplication.
pub const MAX_VIAS: usize = 32;

/// JSON request body shared by the client and federation endpoints.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct StrippedStateMsc4560ReqBody {
    /// Types to select, across all their state keys. Defaults to no state.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub state: BTreeSet<StateEventType>,

    /// Whether to include the joined-member count. Defaults to `false`.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub num_joined_members: bool,
}
crate::json_body_modifier!(StrippedStateMsc4560ReqBody);

impl StrippedStateMsc4560ReqBody {
    /// Whether neither state nor a member count was requested.
    pub fn is_empty(&self) -> bool {
        self.state.is_empty() && !self.num_joined_members
    }

    /// Check the proposal's allowlist before any alias/network operation.
    ///
    /// Custom/unknown types remain representable so a handler can return the
    /// required `M_INVALID_PARAM`, rather than a JSON deserialization error.
    pub fn validate(&self) -> MatrixResult<()> {
        if self
            .state
            .iter()
            .any(|event_type| !ALLOWED_STRIPPED_STATE_EVENTS.contains(event_type))
        {
            return Err(MatrixError::invalid_param(
                "Requested state type is not allowed by MSC4560",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
