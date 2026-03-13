//! Pusher management models

use serde::{Deserialize, Serialize};

/// Pusher information for a user
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PusherInfo {
    pub pusher_id: String,
    pub app_id: String,
    pub app_display_name: String,
    pub device_display_name: String,
    pub profile_tag: Option<String>,
    pub kind: PusherKind,
    pub lang: String,
    pub data: PusherData,
    pub state: PusherState,
    pub last_active_ts: Option<u64>,
    pub creation_ts: u64,
}

/// Pusher kind enumeration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PusherKind {
    Http,
    Email,
    Custom(String),
}

/// Pusher data (varies by pusher type)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PusherData {
    pub url: Option<String>,        // For HTTP pushers
    pub format: Option<String>,     // For HTTP pushers
    pub email: Option<String>,      // For email pushers
}

/// Pusher state enumeration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PusherState {
    Active,
    Gone,
    Unsupported,
}

/// Pusher list response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PusherListResponse {
    pub success: bool,
    pub pushers: Vec<PusherInfo>,
    pub error: Option<String>,
}

/// Pusher update request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdatePusherRequest {
    pub pusher_id: String,
    pub data: Option<PusherData>,
}

/// Pusher update response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdatePusherResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Pusher deletion request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeletePusherRequest {
    pub pusher_id: String,
}

/// Pusher deletion response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeletePusherResponse {
    pub success: bool,
    pub error: Option<String>,
}

impl PusherInfo {
    /// Get human-readable pusher kind
    pub fn kind_display(&self) -> &'static str {
        match self.kind {
            PusherKind::Http => "HTTP 推送",
            PusherKind::Email => "邮件推送",
            PusherKind::Custom(ref s) => s.as_str(),
        }
    }

    /// Get human-readable state
    pub fn state_display(&self) -> &'static str {
        match self.state {
            PusherState::Active => "活跃",
            PusherState::Gone => "已移除",
            PusherState::Unsupported => "不支持",
        }
    }

    /// Get pusher icon based on kind
    pub fn icon(&self) -> &'static str {
        match self.kind {
            PusherKind::Http => "🌐",
            PusherKind::Email => "📧",
            PusherKind::Custom(_) => "🔔",
        }
    }
}