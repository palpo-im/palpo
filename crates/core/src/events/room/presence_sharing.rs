//! Types for the MSC4495 room presence-sharing state event.

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::PrivOwnedStr;
use crate::events::EmptyStateKey;
use crate::macros::EventContent;
use crate::serde::StringEnum;

/// A room's presence-sharing hint.
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
#[palpo_enum(rename_all = "snake_case")]
pub enum PresenceSharingHint {
    /// Presence sharing in this room is prohibited.
    Forbid,

    /// Clients should suggest enabling presence sharing for this room.
    Suggest,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// The content of the MSC4495 room presence-sharing state event.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize, EventContent)]
#[non_exhaustive]
#[palpo_event(
    type = "org.continuwuity.presence_v2.msc4495.room.presence_sharing",
    kind = State,
    state_key_type = EmptyStateKey
)]
pub struct RoomPresenceSharingEventContent {
    /// The room's presence-sharing hint.
    pub presence_sharing: PresenceSharingHint,
}

impl RoomPresenceSharingEventContent {
    /// Creates room presence-sharing state.
    pub fn new(presence_sharing: PresenceSharingHint) -> Self {
        Self { presence_sharing }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{PresenceSharingHint, RoomPresenceSharingEventContent};

    #[test]
    fn serde() {
        let content = RoomPresenceSharingEventContent::new(PresenceSharingHint::Suggest);

        assert_eq!(
            serde_json::to_value(content).unwrap(),
            json!({ "presence_sharing": "suggest" })
        );
    }
}
