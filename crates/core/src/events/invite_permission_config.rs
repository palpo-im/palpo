//! Types for the [`m.invite_permission_config`] account data.
//!
//! [`m.invite_permission_config`]: https://github.com/matrix-org/matrix-spec-proposals/pull/4380

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::PrivOwnedStr;
use crate::macros::{EventContent, StringEnum};

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

    /// Reject invites unless both users are joined to a room with a non-public join rule.
    #[cfg(feature = "unstable-msc4494")]
    #[palpo_enum(rename = "uk.timedout.msc4494.deny_public")]
    DenyPublic,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn invite_actions_preserve_unknown_values_and_ignore_invalid_values() {
        for action in ["block", "unknown", "uk.timedout.msc4494.deny_public"] {
            let content: InvitePermissionConfigEventContent =
                serde_json::from_value(json!({"default_action": action})).unwrap();
            assert_eq!(content.default_action.unwrap().as_str(), action);
        }
        for value in [
            json!({}),
            json!({"default_action": 42}),
            json!({"default_action": null}),
        ] {
            let content: InvitePermissionConfigEventContent =
                serde_json::from_value(value).unwrap();
            assert!(content.default_action.is_none());
        }
    }

    #[cfg(feature = "unstable-msc4494")]
    #[test]
    fn membership_action_uses_unstable_wire_name() {
        let content: InvitePermissionConfigEventContent =
            serde_json::from_value(json!({"default_action": "uk.timedout.msc4494.deny_public"}))
                .unwrap();
        assert!(matches!(
            content.default_action,
            Some(InvitePermissionAction::DenyPublic)
        ));
        assert_eq!(
            serde_json::to_value(InvitePermissionAction::DenyPublic).unwrap(),
            json!("uk.timedout.msc4494.deny_public")
        );
    }
}
