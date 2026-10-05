//! Rust replacement slices for web-admin's store, mini-app sessions and Inbox.
//! Existing Node state is preserved; production cutover requires the remaining
//! fleet/Matrix workflow and notification integrations listed in the README.
pub mod accounts;
pub mod api;
pub mod associations;
mod connections;
mod creation;
mod engagement_setup;
mod intents;
mod lifecycle;
pub mod machine;
pub mod matrix;
mod navigation;
pub mod notifications;
pub mod outbound;
mod preferences;
mod rooms;
pub mod store;
pub mod updates;
mod views;
pub mod workflow;

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
#[error("{code}")]
pub struct Error {
    pub status: u16,
    pub code: &'static str,
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn fail(status: u16, code: &'static str) -> Error {
    Error { status, code }
}

impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        fail(503, "workflow_store_unavailable")
    }
}

impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        fail(400, "invalid_arguments")
    }
}

impl From<palpo_hagency_contract::Error> for Error {
    fn from(value: palpo_hagency_contract::Error) -> Self {
        use palpo_hagency_contract::Error as C;
        match value {
            C::Forbidden | C::SelfApproval => fail(403, "coordinator_required"),
            C::Unsupported => fail(501, "coordinator_protocol_unavailable"),
            C::BindingMismatch => fail(409, "workflow_binding_changed"),
            C::Expired => fail(409, "workflow_authority_expired"),
            C::EngagementUnavailable => fail(409, "engagement_unavailable"),
            C::ProjectUnavailable => fail(409, "project_not_ready"),
            C::ResourceNotGranted => fail(403, "resource_not_granted"),
            C::Unallocated | C::InsufficientCapacity => fail(409, "insufficient_capacity"),
            _ => fail(400, "invalid_arguments"),
        }
    }
}

pub fn digest(value: &Value) -> Result<String> {
    Ok(palpo_hagency_contract::canonical::digest(value)?)
}

pub fn secret() -> String {
    rand::random::<[u8; 32]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
