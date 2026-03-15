//! Session and connection management models

use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use crate::models::room::SortOrder;

/// Session information for a user connection
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SessionInfo {
    pub session_id: String,
    pub user_id: String,
    pub device_id: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub connection_type: ConnectionType,
    pub login_ts: u64,
    pub last_activity_ts: u64,
    pub is_active: bool,
    pub room_count: u32,
    pub device_display_name: Option<String>,
}

/// Connection type enumeration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ConnectionType {
    Web,
    Mobile,
    Desktop,
    Bot,
    Other,
}

/// Session list request with filtering and pagination
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionListRequest {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub filter_active: Option<bool>,
    pub filter_connection_type: Option<ConnectionType>,
    pub sort_by: Option<SessionSortField>,
    pub sort_order: Option<SortOrder>,
}

/// Session list response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionListResponse {
    pub success: bool,
    pub sessions: Vec<SessionInfo>,
    pub total_count: u32,
    pub active_count: u32,
    pub has_more: bool,
    pub error: Option<String>,
}

/// Session sort fields
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SessionSortField {
    SessionId,
    LoginTime,
    LastActivity,
    IpAddress,
}

/// Whois information response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WhoisInfo {
    pub user_id: String,
    pub devices: Vec<WhoisDevice>,
    pub connected_since: u64,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub last_activity: u64,
}

/// Whois device information
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WhoisDevice {
    pub device_id: String,
    pub display_name: Option<String>,
    pub last_seen_ip: Option<String>,
    pub last_seen_ts: u64,
    pub last_seen_user_agent: Option<String>,
}

/// Session termination request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TerminateSessionRequest {
    pub session_id: String,
    pub reason: Option<String>,
}

/// Session termination response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TerminateSessionResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Batch session termination request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BatchTerminateSessionRequest {
    pub session_ids: Vec<String>,
    pub reason: Option<String>,
}

/// Batch session termination response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BatchTerminateSessionResponse {
    pub success: bool,
    pub terminated_count: u32,
    pub failed_count: u32,
    pub failed_sessions: Vec<String>,
    pub errors: Vec<String>,
}

impl SessionInfo {
    /// Get session duration in seconds
    pub fn duration_seconds(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        now - self.login_ts
    }

    /// Get human-readable session duration
    pub fn duration_readable(&self) -> String {
        let seconds = self.duration_seconds();
        
        if seconds < 60 {
            format!("{} 秒", seconds)
        } else if seconds < 3600 {
            format!("{} 分钟", seconds / 60)
        } else if seconds < 86400 {
            format!("{} 小时", seconds / 3600)
        } else {
            format!("{} 天", seconds / 86400)
        }
    }

    /// Get human-readable last activity time
    pub fn last_activity_readable(&self) -> String {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let diff = now - self.last_activity_ts;
        
        if diff < 60 {
            "刚刚".to_string()
        } else if diff < 3600 {
            format!("{} 分钟前", diff / 60)
        } else if diff < 86400 {
            format!("{} 小时前", diff / 3600)
        } else {
            format!("{} 天前", diff / 86400)
        }
    }

    /// Get connection type icon
    pub fn connection_icon(&self) -> &'static str {
        match self.connection_type {
            ConnectionType::Web => "🌐",
            ConnectionType::Mobile => "📱",
            ConnectionType::Desktop => "🖥️",
            ConnectionType::Bot => "🤖",
            ConnectionType::Other => "📟",
        }
    }
}

impl Default for SessionListRequest {
    fn default() -> Self {
        Self {
            limit: Some(50),
            offset: Some(0),
            filter_active: None,
            filter_connection_type: None,
            sort_by: Some(SessionSortField::LastActivity),
            sort_order: Some(SortOrder::Descending),
        }
    }
}

impl SessionSortField {
    /// Get human-readable description of the sort field
    pub fn description(&self) -> &'static str {
        match self {
            SessionSortField::SessionId => "Session ID",
            SessionSortField::LoginTime => "Login Time",
            SessionSortField::LastActivity => "Last Activity",
            SessionSortField::IpAddress => "IP Address",
        }
    }
}