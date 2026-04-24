//! Types for the [`m.key_backup`] account data event.
//!
//! [`m.key_backup`]: https://github.com/matrix-org/matrix-spec-proposals/pull/4287

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;

/// The content of an [`m.key_backup`] event.
///
/// [`m.key_backup`]: https://github.com/matrix-org/matrix-spec-proposals/pull/4287
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize, EventContent)]
#[palpo_event(type = "m.key_backup", kind = GlobalAccountData)]
pub struct KeyBackupEventContent {
    /// Is key backup explicitly enabled or disabled by the user?
    pub enabled: bool,
}

impl KeyBackupEventContent {
    /// Creates a new `KeyBackupEventContent`.
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }
}
