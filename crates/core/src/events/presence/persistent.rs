//! User-controlled persistent presence configuration (MSC4532).
use crate::macros::EventContent;
use crate::presence::revised::Status;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, EventContent)]
#[palpo_event(type = "org.continuwuity.presence_v2.msc4532.presence.persistent", kind = GlobalAccountData)]
pub struct PresencePersistentEventContent {
    /// Unknown states are retained but ignored when resolving presence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_override: Option<String>,
    /// Shared status; an omitted object clears previous status.
    #[serde(default)]
    pub status: Status,
}
