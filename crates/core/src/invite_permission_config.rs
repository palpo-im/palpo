//! Types for the [`m.invite_permission_config`] account data event.
//!
//! [`m.invite_permission_config`]: https://github.com/matrix-org/matrix-spec-proposals/pull/4380

use palpo_core_macros::EventContent;
use serde::{Deserialize, Serialize};

/// The content of an `m.invite_permission_config` event.
///
/// A single property: `block_all`.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[palpo_event(
    kind = GlobalAccountData,
    type = "org.matrix.msc4380.invite_permission_config",
    alias = "m.invite_permission_config",
)]
pub struct InvitePermissionConfigEventContent {
    /// When set to true, indicates that the user does not wish to receive *any* room invites, and
    /// they should be blocked.
    #[serde(default)]
    #[serde(deserialize_with = "crate::serde::default_on_error")]
    pub block_all: bool,
}

impl InvitePermissionConfigEventContent {
    /// Creates a new `InvitePermissionConfigEventContent` from the desired boolean state.
    pub fn new(block_all: bool) -> Self {
        Self { block_all }
    }
}

#[cfg(test)]
mod tests {
    use strass::assert_let;
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::InvitePermissionConfigEventContent;
    use crate::AnyGlobalAccountDataEvent;

    #[test]
    fn serialization() {
        let invite_permission_config = InvitePermissionConfigEventContent::new(true);

        let json = json!({
            "block_all": true
        });

        assert_eq!(to_json_value(invite_permission_config).unwrap(), json);
    }

    #[test]
    fn deserialization() {
        let json = json!({
            "content": {
                "block_all": true
            },
            "type": "m.invite_permission_config"
        });

        assert_let!(Ok(AnyGlobalAccountDataEvent::InvitePermissionConfig(ev)) = from_json_value::<AnyGlobalAccountDataEvent>(json));
        assert!(ev.content.block_all);
    }
}
