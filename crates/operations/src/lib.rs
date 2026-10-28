//! Optional account signup approvals using ordinary Matrix accounts and rooms.
pub mod accounts;
pub mod api;
pub mod matrix;
pub mod store;

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

pub fn digest(value: &Value) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
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
