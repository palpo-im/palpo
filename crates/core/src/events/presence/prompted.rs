//! Types for the MSC4495 presence-prompt account data.

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;
use crate::{OwnedRoomId, OwnedUserId};

/// Users and rooms for which the presence-sharing prompt was already shown.
#[derive(ToSchema, Clone, Default, Debug, Deserialize, Serialize, EventContent)]
#[non_exhaustive]
#[palpo_event(
    type = "org.continuwuity.presence_v2.msc4495.presence.prompted",
    kind = GlobalAccountData
)]
pub struct PresencePromptedEventContent {
    /// Users for which the prompt was already shown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<OwnedUserId>,

    /// Rooms for which the prompt was already shown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<OwnedRoomId>,
}

impl PresencePromptedEventContent {
    /// Creates presence-prompt state.
    pub fn new(users: Vec<OwnedUserId>, rooms: Vec<OwnedRoomId>) -> Self {
        Self { users, rooms }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::PresencePromptedEventContent;
    use crate::{owned_room_id, owned_user_id};

    #[test]
    fn serde() {
        let content = PresencePromptedEventContent::new(
            vec![owned_user_id!("@alice:example.org")],
            vec![owned_room_id!("!room:example.org")],
        );

        assert_eq!(
            serde_json::to_value(content).unwrap(),
            json!({
                "users": ["@alice:example.org"],
                "rooms": ["!room:example.org"]
            })
        );
    }
}
