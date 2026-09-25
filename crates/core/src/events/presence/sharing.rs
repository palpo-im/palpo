//! Types for the MSC4495 presence-sharing account data.

use std::collections::BTreeMap;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;
use crate::serde::StringEnum;
use crate::{OwnedRoomId, OwnedServerName, OwnedUserId, PrivOwnedStr};

/// Whether a user may receive presence updates.
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
#[palpo_enum(rename_all = "snake_case")]
pub enum UserPresenceSharingState {
    /// Allow presence sharing with the user.
    Allow,

    /// Deny presence sharing with the user.
    Deny,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// Whether a room may contribute presence recipients.
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
#[palpo_enum(rename_all = "snake_case")]
pub enum RoomPresenceSharingState {
    /// Allow presence sharing with members of the room.
    Allow,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// Whether a server may receive presence updates.
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
#[palpo_enum(rename_all = "snake_case")]
pub enum ServerPresenceSharingState {
    /// Deny presence sharing with the server.
    Deny,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// A user's selective-presence sharing configuration.
#[derive(ToSchema, Clone, Default, Debug, Deserialize, Serialize, EventContent)]
#[non_exhaustive]
#[palpo_event(
    type = "org.continuwuity.presence_v2.msc4495.presence.sharing",
    kind = GlobalAccountData
)]
pub struct PresenceSharingEventContent {
    /// Whether all users on the local homeserver may receive presence.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub share_locally: bool,

    /// Per-user sharing rules.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub users: BTreeMap<OwnedUserId, UserPresenceSharingState>,

    /// Per-room sharing rules.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rooms: BTreeMap<OwnedRoomId, RoomPresenceSharingState>,

    /// Per-server sharing rules.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub servers: BTreeMap<OwnedServerName, ServerPresenceSharingState>,
}

impl PresenceSharingEventContent {
    /// Creates a selective-presence sharing configuration.
    pub fn new(
        share_locally: bool,
        users: BTreeMap<OwnedUserId, UserPresenceSharingState>,
        rooms: BTreeMap<OwnedRoomId, RoomPresenceSharingState>,
        servers: BTreeMap<OwnedServerName, ServerPresenceSharingState>,
    ) -> Self {
        Self {
            share_locally,
            users,
            rooms,
            servers,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{PresenceSharingEventContent, UserPresenceSharingState};
    use crate::{owned_user_id, user_id};

    #[test]
    fn serde() {
        let mut content = PresenceSharingEventContent {
            share_locally: true,
            ..Default::default()
        };
        content.users.insert(
            owned_user_id!("@alice:example.org"),
            UserPresenceSharingState::Allow,
        );

        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(
            json,
            json!({
                "share_locally": true,
                "users": { "@alice:example.org": "allow" }
            })
        );

        let parsed: PresenceSharingEventContent = serde_json::from_value(json).unwrap();
        assert_eq!(
            parsed.users.get(user_id!("@alice:example.org")),
            Some(&UserPresenceSharingState::Allow)
        );
    }
}
