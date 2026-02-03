//! Audit log data models

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Audit log entry
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AuditLogEntry {
    pub id: i64,
    pub timestamp: SystemTime,
    pub admin_user_id: String, // Using String instead of OwnedUserId for simplicity
    pub action: AuditAction,
    pub target_type: AuditTargetType,
    pub target_id: String,
    pub old_value: Option<serde_json::Value>,
    pub new_value: Option<serde_json::Value>,
    pub success: bool,
    pub error_message: Option<String>,
}

/// Types of audit actions
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum AuditAction {
    ConfigUpdate,
    UserCreate,
    UserUpdate,
    UserDeactivate,
    RoomDisable,
    RoomEnable,
    AppserviceRegister,
    AppserviceUnregister,
    MediaDelete,
    FederationDisable,
    ServerRestart,
    ServerShutdown,
    ConfigReload,
}

/// Types of audit targets
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum AuditTargetType {
    Config,
    User,
    Room,
    Appservice,
    Media,
    Federation,
    Server,
}

/// Audit log filter for queries
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AuditLogFilter {
    pub start_time: Option<SystemTime>,
    pub end_time: Option<SystemTime>,
    pub admin_user_id: Option<String>,
    pub action: Option<AuditAction>,
    pub target_type: Option<AuditTargetType>,
    pub success: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Audit log response for API
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AuditLogResponse {
    pub entries: Vec<AuditLogEntry>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

impl Default for AuditLogFilter {
    fn default() -> Self {
        Self {
            start_time: None,
            end_time: None,
            admin_user_id: None,
            action: None,
            target_type: None,
            success: None,
            limit: Some(50),
            offset: Some(0),
        }
    }
}

impl AuditAction {
    /// Get human-readable description of the action
    pub fn description(&self) -> &'static str {
        match self {
            AuditAction::ConfigUpdate => "Configuration Updated",
            AuditAction::UserCreate => "User Created",
            AuditAction::UserUpdate => "User Updated",
            AuditAction::UserDeactivate => "User Deactivated",
            AuditAction::RoomDisable => "Room Disabled",
            AuditAction::RoomEnable => "Room Enabled",
            AuditAction::AppserviceRegister => "Appservice Registered",
            AuditAction::AppserviceUnregister => "Appservice Unregistered",
            AuditAction::MediaDelete => "Media Deleted",
            AuditAction::FederationDisable => "Federation Disabled",
            AuditAction::ServerRestart => "Server Restarted",
            AuditAction::ServerShutdown => "Server Shutdown",
            AuditAction::ConfigReload => "Configuration Reloaded",
        }
    }
}

impl AuditTargetType {
    /// Get human-readable description of the target type
    pub fn description(&self) -> &'static str {
        match self {
            AuditTargetType::Config => "Configuration",
            AuditTargetType::User => "User",
            AuditTargetType::Room => "Room",
            AuditTargetType::Appservice => "Application Service",
            AuditTargetType::Media => "Media",
            AuditTargetType::Federation => "Federation",
            AuditTargetType::Server => "Server",
        }
    }
}