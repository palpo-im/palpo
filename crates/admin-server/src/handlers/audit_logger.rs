/// Audit Logging for User Management Operations
///
/// This module provides audit logging functionality for all user management operations.
/// All administrative actions are logged with user, action, target, and timestamp.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Audit action types for user management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditAction {
    // User actions
    UserCreated,
    UserUpdated,
    UserDeactivated,
    UserReactivated,
    UserDeleted,
    UserPasswordReset,
    UserAdminStatusChanged,
    UserShadowBanChanged,
    UserLockedChanged,

    // Device actions
    DeviceCreated,
    DeviceDeleted,
    DevicesBatchDeleted,
    AllUserDevicesDeleted,

    // Session actions
    SessionRecorded,
    SessionsDeleted,

    // Rate limit actions
    RateLimitSet,
    RateLimitDeleted,

    // Threepid actions
    ThreepidAdded,
    ThreepidRemoved,
    ThreepidValidated,
    ExternalIdAdded,
    ExternalIdRemoved,
}

impl AuditAction {
    pub fn to_string(&self) -> String {
        match self {
            AuditAction::UserCreated => "USER_CREATED".to_string(),
            AuditAction::UserUpdated => "USER_UPDATED".to_string(),
            AuditAction::UserDeactivated => "USER_DEACTIVATED".to_string(),
            AuditAction::UserReactivated => "USER_REACTIVATED".to_string(),
            AuditAction::UserDeleted => "USER_DELETED".to_string(),
            AuditAction::UserPasswordReset => "USER_PASSWORD_RESET".to_string(),
            AuditAction::UserAdminStatusChanged => "USER_ADMIN_STATUS_CHANGED".to_string(),
            AuditAction::UserShadowBanChanged => "USER_SHADOW_BAN_CHANGED".to_string(),
            AuditAction::UserLockedChanged => "USER_LOCKED_CHANGED".to_string(),
            AuditAction::DeviceCreated => "DEVICE_CREATED".to_string(),
            AuditAction::DeviceDeleted => "DEVICE_DELETED".to_string(),
            AuditAction::DevicesBatchDeleted => "DEVICES_BATCH_DELETED".to_string(),
            AuditAction::AllUserDevicesDeleted => "ALL_USER_DEVICES_DELETED".to_string(),
            AuditAction::SessionRecorded => "SESSION_RECORDED".to_string(),
            AuditAction::SessionsDeleted => "SESSIONS_DELETED".to_string(),
            AuditAction::RateLimitSet => "RATE_LIMIT_SET".to_string(),
            AuditAction::RateLimitDeleted => "RATE_LIMIT_DELETED".to_string(),
            AuditAction::ThreepidAdded => "THREEPID_ADDED".to_string(),
            AuditAction::ThreepidRemoved => "THREEPID_REMOVED".to_string(),
            AuditAction::ThreepidValidated => "THREEPID_VALIDATED".to_string(),
            AuditAction::ExternalIdAdded => "EXTERNAL_ID_ADDED".to_string(),
            AuditAction::ExternalIdRemoved => "EXTERNAL_ID_REMOVED".to_string(),
        }
    }
}

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub id: String,
    pub timestamp: i64,
    pub admin_user: String,
    pub action: String,
    pub target_user: Option<String>,
    pub target_resource: Option<String>,
    pub details: Option<String>,
    pub ip_address: Option<String>,
    pub success: bool,
    pub error_message: Option<String>,
}

/// Audit logger configuration
#[derive(Clone)]
pub struct AuditLogger {
    // In production, this would write to a database or logging service
    // For now, we use tracing for output
}

impl AuditLogger {
    pub fn new() -> Self {
        Self {}
    }

    /// Log an audit event
    pub fn log(
        &self,
        admin_user: &str,
        action: AuditAction,
        target_user: Option<&str>,
        target_resource: Option<&str>,
        details: Option<&str>,
        ip_address: Option<&str>,
        success: bool,
        error_message: Option<&str>,
    ) {
        let entry = AuditLogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now().timestamp_millis(),
            admin_user: admin_user.to_string(),
            action: action.to_string(),
            target_user: target_user.map(|s| s.to_string()),
            target_resource: target_resource.map(|s| s.to_string()),
            details: details.map(|s| s.to_string()),
            ip_address: ip_address.map(|s| s.to_string()),
            success,
            error_message: error_message.map(|s| s.to_string()),
        };

        // Log to tracing (in production, also write to database)
        if success {
            tracing::info!(audit = ?entry, "Audit log");
        } else {
            tracing::warn!(audit = ?entry, "Audit log - failed operation");
        }
    }

    /// Log user creation
    pub fn log_user_created(
        &self,
        admin_user: &str,
        new_user_id: &str,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            AuditAction::UserCreated,
            Some(new_user_id),
            None,
            Some("New user account created"),
            ip_address,
            true,
            None,
        );
    }

    /// Log user update
    pub fn log_user_updated(
        &self,
        admin_user: &str,
        target_user: &str,
        changes: &str,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            AuditAction::UserUpdated,
            Some(target_user),
            None,
            Some(changes),
            ip_address,
            true,
            None,
        );
    }

    /// Log user deactivation
    pub fn log_user_deactivated(
        &self,
        admin_user: &str,
        target_user: &str,
        erase: bool,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            AuditAction::UserDeactivated,
            Some(target_user),
            None,
            Some(&format!("erase_data: {}", erase)),
            ip_address,
            true,
            None,
        );
    }

    /// Log user reactivation
    pub fn log_user_reactivated(
        &self,
        admin_user: &str,
        target_user: &str,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            AuditAction::UserReactivated,
            Some(target_user),
            None,
            Some("User reactivated"),
            ip_address,
            true,
            None,
        );
    }

    /// Log admin status change
    pub fn log_admin_status_changed(
        &self,
        admin_user: &str,
        target_user: &str,
        is_admin: bool,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            AuditAction::UserAdminStatusChanged,
            Some(target_user),
            None,
            Some(&format!("is_admin: {}", is_admin)),
            ip_address,
            true,
            None,
        );
    }

    /// Log shadow ban change
    pub fn log_shadow_ban_changed(
        &self,
        admin_user: &str,
        target_user: &str,
        shadow_banned: bool,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            AuditAction::UserShadowBanChanged,
            Some(target_user),
            None,
            Some(&format!("shadow_banned: {}", shadow_banned)),
            ip_address,
            true,
            None,
        );
    }

    /// Log device deletion
    pub fn log_device_deleted(
        &self,
        admin_user: &str,
        target_user: &str,
        device_id: &str,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            AuditAction::DeviceDeleted,
            Some(target_user),
            Some(device_id),
            Some("Device deleted"),
            ip_address,
            true,
            None,
        );
    }

    /// Log batch device deletion
    pub fn log_devices_batch_deleted(
        &self,
        admin_user: &str,
        target_user: &str,
        count: u64,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            AuditAction::DevicesBatchDeleted,
            Some(target_user),
            None,
            Some(&format!("deleted_count: {}", count)),
            ip_address,
            true,
            None,
        );
    }

    /// Log rate limit change
    pub fn log_rate_limit_changed(
        &self,
        admin_user: &str,
        target_user: &str,
        messages_per_second: i32,
        burst_count: i32,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            AuditAction::RateLimitSet,
            Some(target_user),
            None,
            Some(&format!("mps: {}, burst: {}", messages_per_second, burst_count)),
            ip_address,
            true,
            None,
        );
    }

    /// Log failed operation
    pub fn log_failure(
        &self,
        admin_user: &str,
        action: AuditAction,
        target_user: Option<&str>,
        error: &str,
        ip_address: Option<&str>,
    ) {
        self.log(
            admin_user,
            action,
            target_user,
            None,
            None,
            ip_address,
            false,
            Some(error),
        );
    }
}

/// Global audit logger instance
pub static AUDIT_LOGGER: once_cell::sync::Lazy<Arc<AuditLogger>> =
    once_cell::sync::Lazy::new(|| Arc::new(AuditLogger::new()));

/// Get the global audit logger
pub fn get_audit_logger() -> Arc<AuditLogger> {
    AUDIT_LOGGER.clone()
}