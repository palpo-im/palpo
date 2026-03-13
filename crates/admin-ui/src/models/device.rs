//! Device and session management models

use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use crate::models::room::SortOrder;

/// Device information for a user
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub device_id: String,
    pub user_id: String,
    pub display_name: Option<String>,
    pub device_type: DeviceType,
    pub last_seen_ip: Option<String>,
    pub last_seen_ts: u64,
    pub last_seen_user_agent: Option<String>,
    pub is_suspended: bool,
    pub created_ts: u64,
    pub session_id: Option<String>,
}

/// Device type enumeration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum DeviceType {
    Mobile,
    Desktop,
    Web,
    Bot,
    Other,
}

/// Device list request with filtering and pagination
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeviceListRequest {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub search: Option<String>,
    pub filter_suspended: Option<bool>,
    pub sort_by: Option<DeviceSortField>,
    pub sort_order: Option<SortOrder>,
}

/// Device list response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeviceListResponse {
    pub success: bool,
    pub devices: Vec<DeviceInfo>,
    pub total_count: u32,
    pub has_more: bool,
    pub error: Option<String>,
}

/// Device sort fields
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum DeviceSortField {
    DeviceId,
    DisplayName,
    LastSeen,
    Created,
}

/// Device deletion request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeleteDeviceRequest {
    pub device_id: String,
    pub reason: Option<String>,
}

/// Device deletion response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeleteDeviceResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Batch device deletion request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BatchDeleteDeviceRequest {
    pub device_ids: Vec<String>,
    pub reason: Option<String>,
}

/// Batch device deletion response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BatchDeleteDeviceResponse {
    pub success: bool,
    pub deleted_count: u32,
    pub failed_count: u32,
    pub failed_devices: Vec<String>,
    pub errors: Vec<String>,
}

/// Device suspension request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SuspendDeviceRequest {
    pub device_id: String,
    pub reason: Option<String>,
}

/// Device suspension response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SuspendDeviceResponse {
    pub success: bool,
    pub error: Option<String>,
}

impl DeviceInfo {
    /// Check if the device is currently active
    pub fn is_active(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // Device is considered active if seen within the last 24 hours
        now - self.last_seen_ts < 86400
    }

    /// Get human-readable last seen time
    pub fn last_seen_readable(&self) -> String {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let diff = now - self.last_seen_ts;
        
        if diff < 60 {
            format!("{} 秒前", diff)
        } else if diff < 3600 {
            format!("{} 分钟前", diff / 60)
        } else if diff < 86400 {
            format!("{} 小时前", diff / 3600)
        } else if diff < 86400 * 7 {
            format!("{} 天前", diff / 86400)
        } else {
            let dt = chrono::Utc.timestamp_opt(self.last_seen_ts as i64, 0).single();
            match dt {
                Some(d) => d.format("%Y-%m-%d").to_string(),
                None => "未知".to_string(),
            }
        }
    }

    /// Get device icon based on type
    pub fn device_icon(&self) -> &'static str {
        match self.device_type {
            DeviceType::Mobile => "📱",
            DeviceType::Desktop => "🖥️",
            DeviceType::Web => "🌐",
            DeviceType::Bot => "🤖",
            DeviceType::Other => "📟",
        }
    }
}

impl Default for DeviceListRequest {
    fn default() -> Self {
        Self {
            limit: Some(50),
            offset: Some(0),
            search: None,
            filter_suspended: None,
            sort_by: Some(DeviceSortField::LastSeen),
            sort_order: Some(SortOrder::Descending),
        }
    }
}

impl DeviceSortField {
    /// Get human-readable description of the sort field
    pub fn description(&self) -> &'static str {
        match self {
            DeviceSortField::DeviceId => "Device ID",
            DeviceSortField::DisplayName => "Display Name",
            DeviceSortField::LastSeen => "Last Seen",
            DeviceSortField::Created => "Created",
        }
    }
}