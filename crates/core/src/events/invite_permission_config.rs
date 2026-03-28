//! Types for the [`m.invite_permission_config`] account data.
//!
//! [`m.invite_permission_config`]: https://github.com/matrix-org/matrix-spec-proposals/pull/4380

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::{EventContent, StringEnum};
use crate::PrivOwnedStr;

/// The content of an [`m.invite_permission_config`] account data.
///
/// Controls whether invites to this account are permitted.
///
/// [`m.invite_permission_config`]: https://github.com/matrix-org/matrix-spec-proposals/pull/4380
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[non_exhaustive]
#[palpo_event(
    kind = GlobalAccountData,
    type = "m.invite_permission_config",
)]
pub struct InvitePermissionConfigEventContent {
    /// The default action chosen by the user that the homeserver should perform automatically when
    /// receiving an invitation for this account.
    ///
    /// A missing, invalid or unsupported value means that the user wants to receive invites as
    /// normal.
    #[serde(
        default,
        deserialize_with = "crate::serde::default_on_error",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_action: Option<InvitePermissionAction>,
}

impl InvitePermissionConfigEventContent {
    /// Creates a new empty `InvitePermissionConfigEventContent`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Possible actions in response to an invite.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "lowercase")]
#[non_exhaustive]
pub enum InvitePermissionAction {
    /// Reject the invite.
    Block,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}
