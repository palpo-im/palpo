//! # Audit Log Service
//!
//! This module provides comprehensive audit logging functionality for the Palpo Matrix server
//! administration interface. It enables recording, querying, and managing audit events for
//! all administrative operations.
//!
//! ## Features
//!
//! - **Event Recording**: Record both successful and failed administrative operations
//! - **Flexible Querying**: Query audit logs with multiple filter criteria
//! - **Statistics**: Generate audit statistics for dashboard display
//! - **Export**: Export audit logs in JSON format
//! - **Cleanup**: Automatic cleanup of old audit entries
//! - **WASM Compatible**: Designed to work in WebAssembly environments
//!
//! ## Usage
//!
//! ```ignore
//! use crate::services::audit::AuditService;
//! use crate::models::{AuditAction, AuditTargetType};
//!
//! let mut service = AuditService::new();
//!
//! // Record a successful operation
//! service.record_success(
//!     "admin@example.com".to_string(),
//!     AuditAction::ConfigUpdate,
//!     AuditTargetType::Config,
//!     "server_config".to_string(),
//!     None,
//!     Some(serde_json::json!({"server_name": "matrix.example.com"})),
//! ).unwrap();
//!
//! // Query logs with filters
//! let filter = AuditLogFilter {
//!     success: Some(true),
//!     ..Default::default()
//! };
//! let response = service.query_logs(filter).unwrap();
//! ```

use crate::models::{AuditLogEntry, AuditLogFilter, AuditLogResponse, AuditAction, AuditTargetType};
use crate::models::error::ApiError;
use serde_json::Value;
use std::time::SystemTime;

/// Core audit service for managing audit logs in the frontend.
///
/// This service provides in-memory storage and management of audit log entries.
/// In a production environment, this would typically interface with a backend
/// database, but for the frontend it maintains a local cache of recent audit events.
///
/// ## Thread Safety
///
/// This service is designed for single-threaded use in WASM environments.
/// For multi-threaded scenarios, wrap in appropriate synchronization primitives.
pub struct AuditService {
    /// In-memory storage of audit log entries
    /// 
    /// In a real implementation, this would contain database connection
    /// For now, we'll use in-memory storage for demonstration
    logs: Vec<AuditLogEntry>,
    
    /// Auto-incrementing ID counter for new entries
    next_id: i64,
}

impl AuditService {
    /// Creates a new audit service instance.
    ///
    /// Initializes an empty audit service with no stored entries.
    /// The service starts with an ID counter of 1.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let service = AuditService::new();
    /// ```
    pub fn new() -> Self {
        Self {
            logs: Vec::new(),
            next_id: 1,
        }
    }

    /// Records a successful audit event.
    ///
    /// This method creates and stores an audit log entry for a successful administrative
    /// operation. The entry includes the admin user, action type, target information,
    /// and optional before/after values.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator who performed the action
    /// - `action`: The type of action that was performed
    /// - `target_type`: The type of resource that was targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `old_value`: Optional JSON value representing the state before the action
    /// - `new_value`: Optional JSON value representing the state after the action
    ///
    /// # Returns
    ///
    /// Returns the ID of the created audit log entry on success, or an `ApiError` on failure.
    ///
    /// # Examples
    /// 
    /// ```ignore
    /// let mut service = AuditService::new();
    /// let entry_id = service.record_success(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::ConfigUpdate,
    ///     AuditTargetType::Config,
    ///     "server_config".to_string(),
    ///     Some(serde_json::json!({"old": "value"})),
    ///     Some(serde_json::json!({"new": "value"})),
    /// ).unwrap();
    /// ```
    pub fn record_success(
        &mut self,
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
        old_value: Option<Value>,
        new_value: Option<Value>,
    ) -> Result<i64, ApiError> {
        let mut entry = AuditLogEntry::success_with_values(
            admin_user_id,
            action,
            target_type,
            target_id,
            old_value,
            new_value,
        );
        
        entry.id = self.next_id;
        self.next_id += 1;
        
        self.logs.push(entry.clone());
        
        // Log to console for frontend
        #[cfg(target_arch = "wasm32")]
        web_sys::console::info_1(&format!("Audit log recorded: {}", entry.description()).into());
        
        #[cfg(not(target_arch = "wasm32"))]
        println!("Audit log recorded: {}", entry.description());
        
        Ok(entry.id)
    }

    /// Records a failed audit event.
    ///
    /// This method creates and stores an audit log entry for a failed administrative
    /// operation. The entry includes the admin user, action type, target information,
    /// and the error message that caused the failure.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator who attempted the action
    /// - `action`: The type of action that was attempted
    /// - `target_type`: The type of resource that was targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `error_message`: A description of why the operation failed
    ///
    /// # Returns
    ///
    /// Returns the ID of the created audit log entry on success, or an `ApiError` on failure.
    ///
    /// # Examples
    /// 
    /// ```ignore
    /// let mut service = AuditService::new();
    /// let entry_id = service.record_failure(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::UserCreate,
    ///     AuditTargetType::User,
    ///     "new_user".to_string(),
    ///     "User already exists".to_string(),
    /// ).unwrap();
    /// ```
    pub fn record_failure(
        &mut self,
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
        error_message: String,
    ) -> Result<i64, ApiError> {
        let mut entry = AuditLogEntry::failure(
            admin_user_id,
            action,
            target_type,
            target_id,
            error_message,
        );
        
        entry.id = self.next_id;
        self.next_id += 1;
        
        self.logs.push(entry.clone());
        
        // Log to console for frontend
        #[cfg(target_arch = "wasm32")]
        web_sys::console::warn_1(&format!("Audit log recorded (failure): {}", entry.description()).into());
        
        #[cfg(not(target_arch = "wasm32"))]
        println!("Audit log recorded (failure): {}", entry.description());
        
        Ok(entry.id)
    }

    /// Queries audit logs with flexible filtering options.
    ///
    /// This method allows querying the audit log storage with various filter criteria
    /// including time ranges, user IDs, action types, target types, success status,
    /// and pagination parameters.
    ///
    /// # Parameters
    ///
    /// - `filter`: An `AuditLogFilter` struct containing the query criteria
    ///
    /// # Returns
    ///
    /// Returns an `AuditLogResponse` containing the matching entries and pagination
    /// metadata, or an `ApiError` on failure.
    ///
    /// # Filter Options
    ///
    /// - `start_time` / `end_time`: Filter by timestamp range
    /// - `admin_user_id`: Filter by specific administrator
    /// - `action`: Filter by action type (e.g., ConfigUpdate, UserCreate)
    /// - `target_type`: Filter by target type (e.g., Config, User, Room)
    /// - `success`: Filter by operation success status
    /// - `limit` / `offset`: Pagination parameters
    ///
    /// # Examples
    /// 
    /// ```ignore
    /// // Query failed operations only
    /// let filter = AuditLogFilter {
    ///     success: Some(false),
    ///     limit: Some(10),
    ///     ..Default::default()
    /// };
    /// let response = service.query_logs(filter).unwrap();
    ///
    /// // Query specific user's config changes
    /// let filter = AuditLogFilter {
    ///     admin_user_id: Some("admin@example.com".to_string()),
    ///     action: Some(AuditAction::ConfigUpdate),
    ///     ..Default::default()
    /// };
    /// let response = service.query_logs(filter).unwrap();
    /// ```
    pub fn query_logs(&self, filter: AuditLogFilter) -> Result<AuditLogResponse, ApiError> {
        let mut filtered_logs: Vec<&AuditLogEntry> = self.logs.iter().collect();

        // Apply filters
        if let Some(start_time) = filter.start_time {
            filtered_logs.retain(|log| log.timestamp >= start_time);
        }

        if let Some(end_time) = filter.end_time {
            filtered_logs.retain(|log| log.timestamp <= end_time);
        }

        if let Some(ref admin_user_id) = filter.admin_user_id {
            filtered_logs.retain(|log| log.admin_user_id == *admin_user_id);
        }

        if let Some(ref action) = filter.action {
            filtered_logs.retain(|log| std::mem::discriminant(&log.action) == std::mem::discriminant(action));
        }

        if let Some(ref target_type) = filter.target_type {
            filtered_logs.retain(|log| std::mem::discriminant(&log.target_type) == std::mem::discriminant(target_type));
        }

        if let Some(success) = filter.success {
            filtered_logs.retain(|log| log.success == success);
        }

        let total = filtered_logs.len() as u64;

        // Apply pagination
        let offset = filter.offset.unwrap_or(0) as usize;
        let limit = filter.limit.unwrap_or(50) as usize;
        
        let paginated_logs: Vec<AuditLogEntry> = filtered_logs
            .into_iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();

        Ok(AuditLogResponse {
            entries: paginated_logs,
            total,
            page: (offset / limit) as u32,
            per_page: limit as u32,
        })
    }

    /// Retrieves a specific audit log entry by its unique ID.
    ///
    /// This method searches through the stored audit logs to find an entry
    /// with the specified ID. Returns `None` if no entry is found.
    ///
    /// # Parameters
    ///
    /// - `id`: The unique identifier of the audit log entry to retrieve
    ///
    /// # Returns
    ///
    /// Returns `Some(AuditLogEntry)` if found, `None` if not found, or an `ApiError` on failure.
    ///
    /// # Examples
    /// 
    /// ```ignore
    /// let mut service = AuditService::new();
    /// let entry_id = service.record_success(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::ConfigUpdate,
    ///     AuditTargetType::Config,
    ///     "server_config".to_string(),
    ///     None,
    ///     None,
    /// ).unwrap();
    ///
    /// let retrieved_entry = service.get_log_by_id(entry_id).unwrap();
    /// assert!(retrieved_entry.is_some());
    /// ```
    pub fn get_log_by_id(&self, id: i64) -> Result<Option<AuditLogEntry>, ApiError> {
        Ok(self.logs.iter().find(|log| log.id == id).cloned())
    }

    /// Removes old audit log entries from storage.
    ///
    /// This method performs cleanup by removing all audit log entries that are
    /// older than the specified timestamp. This is useful for maintaining storage
    /// limits and removing outdated audit information.
    ///
    /// # Parameters
    ///
    /// - `before`: A `SystemTime` timestamp; all entries older than this will be removed
    ///
    /// # Returns
    ///
    /// Returns the number of entries that were deleted, or an `ApiError` on failure.
    ///
    /// # Examples
    /// 
    /// ```ignore
    /// use std::time::{SystemTime, Duration};
    ///
    /// let mut service = AuditService::new();
    /// // ... add some entries ...
    ///
    /// // Clean up entries older than 30 days
    /// let thirty_days_ago = SystemTime::now() - Duration::from_secs(30 * 24 * 3600);
    /// let deleted_count = service.cleanup_old_logs(thirty_days_ago).unwrap();
    /// println!("Deleted {} old audit entries", deleted_count);
    /// ```
    pub fn cleanup_old_logs(&mut self, before: SystemTime) -> Result<u64, ApiError> {
        let initial_count = self.logs.len();
        self.logs.retain(|log| log.timestamp >= before);
        let deleted_count = initial_count - self.logs.len();
        
        #[cfg(target_arch = "wasm32")]
        web_sys::console::info_1(&format!("Cleaned up {} old audit log entries", deleted_count).into());
        
        #[cfg(not(target_arch = "wasm32"))]
        println!("Cleaned up {} old audit log entries", deleted_count);
        Ok(deleted_count as u64)
    }

    /// Exports audit logs to JSON format.
    ///
    /// This method queries the audit logs using the provided filter and exports
    /// the matching entries as a pretty-printed JSON string. This is useful for
    /// creating audit reports or backing up audit data.
    ///
    /// # Parameters
    ///
    /// - `filter`: An `AuditLogFilter` specifying which entries to export
    ///
    /// # Returns
    ///
    /// Returns a JSON string containing the exported audit entries, or an `ApiError` on failure.
    ///
    /// # Examples
    /// 
    /// ```ignore
    /// // Export all failed operations from the last week
    /// let filter = AuditLogFilter {
    ///     success: Some(false),
    ///     start_time: Some(SystemTime::now() - Duration::from_secs(7 * 24 * 3600)),
    ///     ..Default::default()
    /// };
    /// let json_export = service.export_logs(filter).unwrap();
    /// ```
    pub fn export_logs(&self, filter: AuditLogFilter) -> Result<String, ApiError> {
        let response = self.query_logs(filter)?;
        serde_json::to_string_pretty(&response.entries)
            .map_err(|e| ApiError::new(format!("Failed to serialize audit logs: {}", e)))
    }

    /// Generates comprehensive audit statistics.
    ///
    /// This method analyzes all stored audit logs to generate statistical information
    /// useful for dashboard displays and audit reporting. The statistics include
    /// total counts, success/failure ratios, breakdowns by action and target type,
    /// and recent activity metrics.
    ///
    /// # Returns
    ///
    /// Returns an `AuditStatistics` struct containing comprehensive metrics, or an `ApiError` on failure.
    ///
    /// # Statistics Included
    ///
    /// - **Total entries**: Overall count of all audit log entries
    /// - **Success/failure counts**: Breakdown of successful vs failed operations
    /// - **Recent activity**: Count of entries from the last 24 hours
    /// - **Action counts**: Frequency of each action type
    /// - **Target counts**: Frequency of each target type
    ///
    /// # Examples
    /// 
    /// ```ignore
    /// let stats = service.get_statistics().unwrap();
    /// println!("Total audit entries: {}", stats.total_entries);
    /// println!("Success rate: {:.1}%",
    ///     (stats.successful_entries as f64 / stats.total_entries as f64) * 100.0);
    /// println!("Recent activity (24h): {}", stats.recent_entries);
    /// ```
    pub fn get_statistics(&self) -> Result<AuditStatistics, ApiError> {
        let total_entries = self.logs.len() as u64;
        let successful_entries = self.logs.iter().filter(|log| log.success).count() as u64;
        let failed_entries = total_entries - successful_entries;

        // Count by action type
        let mut action_counts = std::collections::HashMap::new();
        for log in &self.logs {
            let action_name = log.action.description();
            *action_counts.entry(action_name.to_string()).or_insert(0) += 1;
        }

        // Count by target type
        let mut target_counts = std::collections::HashMap::new();
        for log in &self.logs {
            let target_name = log.target_type.description();
            *target_counts.entry(target_name.to_string()).or_insert(0) += 1;
        }

        // Get recent activity (last 24 hours)
        let twenty_four_hours_ago = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() - 86400;
        let recent_cutoff = std::time::UNIX_EPOCH + std::time::Duration::from_secs(twenty_four_hours_ago);
        
        let recent_entries = self.logs.iter()
            .filter(|log| log.timestamp >= recent_cutoff)
            .count() as u64;

        Ok(AuditStatistics {
            total_entries,
            successful_entries,
            failed_entries,
            recent_entries,
            action_counts,
            target_counts,
        })
    }
}

impl Default for AuditService {
    fn default() -> Self {
        Self::new()
    }
}

/// Comprehensive audit statistics for dashboard and reporting.
///
/// This struct contains various metrics and counts derived from audit log analysis.
/// It provides insights into system usage patterns, operation success rates, and
/// activity trends that are useful for administrative dashboards and audit reports.
///
/// # Fields
///
/// - `total_entries`: Total number of audit log entries
/// - `successful_entries`: Number of successful operations
/// - `failed_entries`: Number of failed operations  
/// - `recent_entries`: Number of entries from the last 24 hours
/// - `action_counts`: Frequency count for each action type
/// - `target_counts`: Frequency count for each target type
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AuditStatistics {
    pub total_entries: u64,
    pub successful_entries: u64,
    pub failed_entries: u64,
    pub recent_entries: u64,
    pub action_counts: std::collections::HashMap<String, u64>,
    pub target_counts: std::collections::HashMap<String, u64>,
}

/// Audit middleware for automatic operation logging.
///
/// This middleware provides automatic audit logging capabilities that can be
/// integrated into operation workflows. It wraps operations and automatically
/// records their success or failure in the audit log system.
///
/// The middleware uses a shared reference to an `AuditService` instance,
/// allowing multiple middleware instances to log to the same audit system.
///
/// # Thread Safety
///
/// This middleware uses `Rc<RefCell<>>` for interior mutability in single-threaded
/// WASM environments. For multi-threaded use, consider using `Arc<Mutex<>>` instead.
///
/// # Examples
/// 
/// ```ignore
/// use std::rc::Rc;
/// use std::cell::RefCell;
///
/// let service = Rc::new(RefCell::new(AuditService::new()));
/// let middleware = AuditMiddleware::new(service);
///
/// // The middleware will automatically log the result
/// let result = middleware.log_operation(
///     "admin@example.com".to_string(),
///     AuditAction::ConfigUpdate,
///     AuditTargetType::Config,
///     "server_config".to_string(),
///     None,
///     Some(serde_json::json!({"new": "value"})),
///     Ok::<_, String>("Operation successful"),
/// );
/// ```
pub struct AuditMiddleware {
    service: std::rc::Rc<std::cell::RefCell<AuditService>>,
}

impl AuditMiddleware {
    /// Creates a new audit middleware instance.
    ///
    /// # Parameters
    ///
    /// - `service`: A shared reference to an `AuditService` instance
    ///
    /// # Examples
    /// 
    /// ```ignore
    /// let service = Rc::new(RefCell::new(AuditService::new()));
    /// let middleware = AuditMiddleware::new(service);
    /// ```
    pub fn new(service: std::rc::Rc<std::cell::RefCell<AuditService>>) -> Self {
        Self { service }
    }

    /// Logs an operation result automatically based on success or failure.
    ///
    /// This method wraps an operation result and automatically records it in the
    /// audit log. If the result is `Ok`, it records a successful operation with
    /// the provided before/after values. If the result is `Err`, it records a
    /// failed operation with the error message.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator performing the operation
    /// - `action`: The type of action being performed
    /// - `target_type`: The type of resource being targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `old_value`: Optional JSON value representing the state before the operation
    /// - `new_value`: Optional JSON value representing the state after the operation
    /// - `result`: The operation result to be logged
    ///
    /// # Returns
    ///
    /// Returns the original result unchanged, allowing this method to be used
    /// transparently in operation chains.
    ///
    /// # Examples
    /// 
    /// ```ignore
    /// let result = middleware.log_operation(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::UserCreate,
    ///     AuditTargetType::User,
    ///     "new_user".to_string(),
    ///     None,
    ///     Some(serde_json::json!({"username": "new_user"})),
    ///     create_user_operation(), // This could return Ok(user) or Err(error)
    /// );
    /// ```
    pub fn log_operation<T, E>(
        &self,
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
        old_value: Option<Value>,
        new_value: Option<Value>,
        result: Result<T, E>,
    ) -> Result<T, E>
    where
        E: std::fmt::Display,
    {
        let mut service = self.service.borrow_mut();
        
        match &result {
            Ok(_) => {
                let _ = service.record_success(
                    admin_user_id,
                    action,
                    target_type,
                    target_id,
                    old_value,
                    new_value,
                );
            }
            Err(error) => {
                let _ = service.record_failure(
                    admin_user_id,
                    action,
                    target_type,
                    target_id,
                    error.to_string(),
                );
            }
        }
        
        result
    }
}

/// Convenience macro for easy audit logging with minimal parameters.
///
/// This macro provides a simplified interface for audit logging operations.
/// It automatically creates an `AuditMiddleware` instance and logs the operation
/// result with the provided parameters.
///
/// # Variants
///
/// ## Basic Usage (without before/after values)
/// ```ignore
/// audit_log!(service, user_id, action, target_type, target_id, result)
/// ```
///
/// ## With Before/After Values
/// ```ignore
/// audit_log!(service, user_id, action, target_type, target_id, old_value, new_value, result)
/// ```
///
/// # Parameters
///
/// - `service`: Shared reference to an `AuditService` instance
/// - `user_id`: Administrator user ID performing the operation
/// - `action`: The `AuditAction` being performed
/// - `target_type`: The `AuditTargetType` being targeted
/// - `target_id`: String identifier of the specific target
/// - `old_value`: (Optional variant) JSON value before the operation
/// - `new_value`: (Optional variant) JSON value after the operation
/// - `result`: The operation result to be logged
///
/// # Examples
/// ```ignore
/// // Basic audit logging
/// let result = audit_log!(
///     service_ref,
///     "admin@example.com".to_string(),
///     AuditAction::UserCreate,
///     AuditTargetType::User,
///     "new_user".to_string(),
///     create_user()
/// );
///
/// // With before/after values
/// let result = audit_log!(
///     service_ref,
///     "admin@example.com".to_string(),
///     AuditAction::ConfigUpdate,
///     AuditTargetType::Config,
///     "server_config".to_string(),
///     Some(old_config_json),
///     Some(new_config_json),
///     update_config()
/// );
/// ```
#[macro_export]
macro_rules! audit_log {
    ($service:expr, $user:expr, $action:expr, $target_type:expr, $target_id:expr, $result:expr) => {
        $crate::services::audit::AuditMiddleware::new($service)
            .log_operation($user, $action, $target_type, $target_id, None, None, $result)
    };
    
    ($service:expr, $user:expr, $action:expr, $target_type:expr, $target_id:expr, $old:expr, $new:expr, $result:expr) => {
        $crate::services::audit::AuditMiddleware::new($service)
            .log_operation($user, $action, $target_type, $target_id, $old, $new, $result)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AuditAction, AuditTargetType};

    #[test]
    fn test_audit_service_record_success() {
        let mut service = AuditService::new();
        
        let result = service.record_success(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "server_config".to_string(),
            None,
            Some(serde_json::json!({"server_name": "test.example.com"})),
        );
        
        assert!(result.is_ok());
        assert_eq!(service.logs.len(), 1);
        assert!(service.logs[0].success);
    }

    #[test]
    fn test_audit_service_record_failure() {
        let mut service = AuditService::new();
        
        let result = service.record_failure(
            "admin@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "test_user".to_string(),
            "Validation failed".to_string(),
        );
        
        assert!(result.is_ok());
        assert_eq!(service.logs.len(), 1);
        assert!(!service.logs[0].success);
        assert_eq!(service.logs[0].error_message, Some("Validation failed".to_string()));
    }

    #[test]
    fn test_audit_service_query_logs() {
        let mut service = AuditService::new();
        
        // Add some test logs
        service.record_success(
            "admin1@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_failure(
            "admin2@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "user1".to_string(),
            "Error".to_string(),
        ).unwrap();
        
        // Query all logs
        let filter = AuditLogFilter::default();
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 2);
        assert_eq!(response.total, 2);
    }

    #[test]
    fn test_audit_service_filter_by_success() {
        let mut service = AuditService::new();
        
        // Add successful and failed logs
        service.record_success(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_failure(
            "admin@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "user1".to_string(),
            "Error".to_string(),
        ).unwrap();
        
        // Query only successful logs
        let filter = AuditLogFilter {
            success: Some(true),
            ..Default::default()
        };
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 1);
        assert!(response.entries[0].success);
    }

    #[test]
    fn test_audit_service_filter_by_failure() {
        let mut service = AuditService::new();
        
        // Add successful and failed logs
        service.record_success(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_failure(
            "admin@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "user1".to_string(),
            "Validation failed".to_string(),
        ).unwrap();
        
        service.record_failure(
            "admin@example.com".to_string(),
            AuditAction::ServerRestart,
            AuditTargetType::Server,
            "main_server".to_string(),
            "Permission denied".to_string(),
        ).unwrap();
        
        // Query only failed logs
        let filter = AuditLogFilter {
            success: Some(false),
            ..Default::default()
        };
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 2);
        assert!(response.entries.iter().all(|entry| !entry.success));
        assert!(response.entries.iter().any(|entry| entry.error_message == Some("Validation failed".to_string())));
        assert!(response.entries.iter().any(|entry| entry.error_message == Some("Permission denied".to_string())));
    }

    #[test]
    fn test_audit_service_filter_by_action() {
        let mut service = AuditService::new();
        
        // Add logs with different actions
        service.record_success(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_success(
            "admin@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "user1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_failure(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config2".to_string(),
            "Invalid configuration".to_string(),
        ).unwrap();
        
        // Query only ConfigUpdate actions
        let filter = AuditLogFilter {
            action: Some(AuditAction::ConfigUpdate),
            ..Default::default()
        };
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 2);
        assert!(response.entries.iter().all(|entry| matches!(entry.action, AuditAction::ConfigUpdate)));
        // Should have one success and one failure
        assert_eq!(response.entries.iter().filter(|entry| entry.success).count(), 1);
        assert_eq!(response.entries.iter().filter(|entry| !entry.success).count(), 1);
    }

    #[test]
    fn test_audit_service_filter_by_target_type() {
        let mut service = AuditService::new();
        
        // Add logs with different target types
        service.record_success(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_success(
            "admin@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "user1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_failure(
            "admin@example.com".to_string(),
            AuditAction::RoomDisable,
            AuditTargetType::Room,
            "room1".to_string(),
            "Room not found".to_string(),
        ).unwrap();
        
        // Query only User target type
        let filter = AuditLogFilter {
            target_type: Some(AuditTargetType::User),
            ..Default::default()
        };
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 1);
        assert!(matches!(response.entries[0].target_type, AuditTargetType::User));
        assert_eq!(response.entries[0].target_id, "user1");
    }

    #[test]
    fn test_audit_service_filter_by_admin_user() {
        let mut service = AuditService::new();
        
        // Add logs from different admin users
        service.record_success(
            "admin1@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_success(
            "admin2@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "user1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_failure(
            "admin1@example.com".to_string(),
            AuditAction::ServerRestart,
            AuditTargetType::Server,
            "server1".to_string(),
            "Insufficient privileges".to_string(),
        ).unwrap();
        
        // Query only admin1's actions
        let filter = AuditLogFilter {
            admin_user_id: Some("admin1@example.com".to_string()),
            ..Default::default()
        };
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 2);
        assert!(response.entries.iter().all(|entry| entry.admin_user_id == "admin1@example.com"));
        // Should have one success and one failure
        assert_eq!(response.entries.iter().filter(|entry| entry.success).count(), 1);
        assert_eq!(response.entries.iter().filter(|entry| !entry.success).count(), 1);
    }

    #[test]
    fn test_audit_service_filter_by_time_range() {
        let mut service = AuditService::new();
        
        let now = std::time::SystemTime::now();
        let one_hour_ago = now - std::time::Duration::from_secs(3600);
        let two_hours_ago = now - std::time::Duration::from_secs(7200);
        
        // Add an old log entry manually
        let mut old_entry = AuditLogEntry::new(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "old_config".to_string(),
        );
        old_entry.id = 1;
        old_entry.timestamp = two_hours_ago;
        service.logs.push(old_entry);
        service.next_id = 2;
        
        // Add a recent log
        service.record_success(
            "admin@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "new_user".to_string(),
            None,
            None,
        ).unwrap();
        
        // Query only logs from the last hour
        let filter = AuditLogFilter {
            start_time: Some(one_hour_ago),
            ..Default::default()
        };
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].target_id, "new_user");
        assert!(response.entries[0].timestamp >= one_hour_ago);
    }

    #[test]
    fn test_audit_service_complex_filter() {
        let mut service = AuditService::new();
        
        // Add various logs
        service.record_success(
            "admin1@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_failure(
            "admin1@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config2".to_string(),
            "Validation error".to_string(),
        ).unwrap();
        
        service.record_success(
            "admin2@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config3".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_failure(
            "admin1@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "user1".to_string(),
            "User already exists".to_string(),
        ).unwrap();
        
        // Query failed ConfigUpdate actions by admin1
        let filter = AuditLogFilter {
            admin_user_id: Some("admin1@example.com".to_string()),
            action: Some(AuditAction::ConfigUpdate),
            success: Some(false),
            ..Default::default()
        };
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].admin_user_id, "admin1@example.com");
        assert!(matches!(response.entries[0].action, AuditAction::ConfigUpdate));
        assert!(!response.entries[0].success);
        assert_eq!(response.entries[0].error_message, Some("Validation error".to_string()));
    }

    #[test]
    fn test_audit_service_pagination() {
        let mut service = AuditService::new();
        
        // Add multiple logs
        for i in 1..=10 {
            service.record_success(
                "admin@example.com".to_string(),
                AuditAction::ConfigUpdate,
                AuditTargetType::Config,
                format!("config{}", i),
                None,
                None,
            ).unwrap();
        }
        
        // Test pagination - first page
        let filter = AuditLogFilter {
            limit: Some(3),
            offset: Some(0),
            ..Default::default()
        };
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 3);
        assert_eq!(response.total, 10);
        assert_eq!(response.per_page, 3);
        assert_eq!(response.page, 0);
        
        // Test pagination - second page
        let filter = AuditLogFilter {
            limit: Some(3),
            offset: Some(3),
            ..Default::default()
        };
        let response = service.query_logs(filter).unwrap();
        
        assert_eq!(response.entries.len(), 3);
        assert_eq!(response.total, 10);
        assert_eq!(response.per_page, 3);
        assert_eq!(response.page, 1);
    }

    #[test]
    fn test_audit_statistics() {
        let mut service = AuditService::new();
        
        // Add some test data
        service.record_success(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config1".to_string(),
            None,
            None,
        ).unwrap();
        
        service.record_failure(
            "admin@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "user1".to_string(),
            "Error".to_string(),
        ).unwrap();
        
        service.record_success(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "config2".to_string(),
            None,
            None,
        ).unwrap();
        
        let stats = service.get_statistics().unwrap();
        
        assert_eq!(stats.total_entries, 3);
        assert_eq!(stats.successful_entries, 2);
        assert_eq!(stats.failed_entries, 1);
        assert!(stats.action_counts.contains_key("Configuration Updated"));
        assert!(stats.action_counts.contains_key("User Created"));
        assert_eq!(stats.action_counts.get("Configuration Updated"), Some(&2));
        assert_eq!(stats.action_counts.get("User Created"), Some(&1));
        assert!(stats.target_counts.contains_key("Configuration"));
        assert!(stats.target_counts.contains_key("User"));
        assert_eq!(stats.target_counts.get("Configuration"), Some(&2));
        assert_eq!(stats.target_counts.get("User"), Some(&1));
    }
}