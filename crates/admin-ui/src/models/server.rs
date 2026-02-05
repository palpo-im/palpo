//! Server control and status models

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
use std::collections::HashMap;

/// Server status information
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServerStatus {
    pub uptime: Duration,
    pub version: String,
    pub features: Vec<String>,
    pub active_connections: u32,
    pub memory_usage: u64,
    pub config_last_modified: SystemTime,
    pub hot_reload_supported: bool,
    pub is_healthy: bool,
    pub database_status: DatabaseStatus,
    pub federation_status: FederationStatus,
}

/// Database connection status
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DatabaseStatus {
    pub connected: bool,
    pub pool_size: u32,
    pub active_connections: u32,
    pub idle_connections: u32,
    pub last_error: Option<String>,
}

/// Federation connectivity status
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FederationStatus {
    pub enabled: bool,
    pub reachable_servers: u32,
    pub unreachable_servers: u32,
    pub pending_transactions: u32,
    pub last_federation_error: Option<String>,
}

/// Server feature information
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServerFeature {
    pub name: String,
    pub enabled: bool,
    pub description: String,
    pub requires_restart: bool,
    pub config_key: Option<String>,
}
/// Configuration reload result
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConfigReloadResult {
    pub success: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub hot_reload_supported: bool,
    pub restart_required: bool,
    pub affected_services: Vec<String>,
    pub reload_time: Duration,
}

/// Admin command for execution
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AdminCommand {
    pub command: String,
    pub args: Vec<String>,
    pub require_confirmation: bool,
    pub timeout_seconds: Option<u64>,
    pub working_directory: Option<String>,
    pub environment_vars: Option<HashMap<String, String>>,
}

/// Command execution result
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommandResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
    pub execution_time: Duration,
    pub exit_code: Option<i32>,
    pub command: String,
    pub started_at: SystemTime,
}

/// Server restart request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RestartServerRequest {
    pub force: bool,
    pub graceful_timeout_seconds: Option<u64>,
    pub reason: Option<String>,
}

/// Server shutdown request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShutdownServerRequest {
    pub graceful: bool,
    pub timeout_seconds: Option<u64>,
    pub reason: Option<String>,
}
/// Admin notice request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AdminNoticeRequest {
    pub message: String,
    pub notice_type: NoticeType,
    pub target_rooms: Option<Vec<String>>,
    pub urgent: bool,
}

/// Notice type for admin messages
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NoticeType {
    Info,
    Warning,
    Error,
    Maintenance,
    Security,
}

/// Server status response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerStatusResponse {
    pub success: bool,
    pub status: Option<ServerStatus>,
    pub error: Option<String>,
}

/// Config reload response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigReloadResponse {
    pub success: bool,
    pub result: Option<ConfigReloadResult>,
    pub error: Option<String>,
}

/// Server features response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerFeaturesResponse {
    pub success: bool,
    pub features: Vec<ServerFeature>,
    pub error: Option<String>,
}

/// Command execution response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommandExecutionResponse {
    pub success: bool,
    pub result: Option<CommandResult>,
    pub error: Option<String>,
}
/// Generic operation response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OperationResponse {
    pub success: bool,
    pub message: Option<String>,
    pub error: Option<String>,
}

/// Server metrics for monitoring
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServerMetrics {
    pub cpu_usage_percent: f64,
    pub memory_usage_bytes: u64,
    pub memory_total_bytes: u64,
    pub disk_usage_bytes: u64,
    pub disk_total_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub active_rooms: u32,
    pub active_users: u32,
    pub events_per_second: f64,
}

impl ServerStatus {
    /// Check if the server is healthy
    pub fn is_healthy(&self) -> bool {
        self.is_healthy && 
        self.database_status.connected && 
        (!self.federation_status.enabled || self.federation_status.reachable_servers > 0)
    }

    /// Get uptime in human-readable format
    pub fn uptime_string(&self) -> String {
        let seconds = self.uptime.as_secs();
        let days = seconds / 86400;
        let hours = (seconds % 86400) / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;

        if days > 0 {
            format!("{}d {}h {}m {}s", days, hours, minutes, secs)
        } else if hours > 0 {
            format!("{}h {}m {}s", hours, minutes, secs)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, secs)
        } else {
            format!("{}s", secs)
        }
    }
}
impl NoticeType {
    /// Get human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            NoticeType::Info => "Information",
            NoticeType::Warning => "Warning",
            NoticeType::Error => "Error",
            NoticeType::Maintenance => "Maintenance",
            NoticeType::Security => "Security Alert",
        }
    }

    /// Get emoji representation
    pub fn emoji(&self) -> &'static str {
        match self {
            NoticeType::Info => "ℹ️",
            NoticeType::Warning => "⚠️",
            NoticeType::Error => "❌",
            NoticeType::Maintenance => "🔧",
            NoticeType::Security => "🔒",
        }
    }
}

impl AdminCommand {
    /// Create a new admin command
    pub fn new(command: String) -> Self {
        Self {
            command,
            args: Vec::new(),
            require_confirmation: false,
            timeout_seconds: Some(30),
            working_directory: None,
            environment_vars: None,
        }
    }

    /// Add an argument to the command
    pub fn with_arg(mut self, arg: String) -> Self {
        self.args.push(arg);
        self
    }

    /// Set whether confirmation is required
    pub fn with_confirmation(mut self, require: bool) -> Self {
        self.require_confirmation = require;
        self
    }

    /// Set command timeout
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }
}