//! # Audit Logging Utilities
//!
//! This module provides utility components for audit logging in the frontend application.
//! It includes an in-memory audit logger that can buffer audit events locally before
//! sending them to the backend, as well as convenience functions for common logging operations.
//!
//! ## Features
//!
//! - **In-Memory Buffering**: Maintains a local buffer of recent audit events
//! - **Automatic Cleanup**: Limits the number of stored entries to prevent memory issues
//! - **Backend Integration**: Placeholder for sending events to the backend API
//! - **Thread Safety**: Uses `Arc<Mutex<>>` for safe concurrent access
//! - **Global Instance**: Provides a global logger instance for convenience
//!
//! ## Usage
//!
//! ### Direct Logger Usage
//!
//! ```rust
//! let logger = AuditLogger::new(500); // Keep last 500 entries
//!
//! logger.log_success(
//!     "admin@example.com".to_string(),
//!     AuditAction::ConfigUpdate,
//!     AuditTargetType::Config,
//!     "server_config".to_string(),
//!     Some(serde_json::json!({"old": "value"})),
//!     Some(serde_json::json!({"new": "value"})),
//! );
//!
//! let recent_entries = logger.get_recent_entries(10);
//! ```
//!
//! ### Global Logger Usage
//!
//! ```rust
//! // Use convenience functions with the global logger
//! log_success(
//!     "admin@example.com".to_string(),
//!     AuditAction::UserCreate,
//!     AuditTargetType::User,
//!     "new_user".to_string(),
//!     None,
//!     Some(serde_json::json!({"username": "new_user"})),
//! );
//!
//! let recent = get_recent_entries(5);
//! ```
//!
//! ## Backend Integration
//!
//! The logger includes a placeholder `send_to_backend` method that can be implemented
//! to automatically forward audit events to the backend API. Currently, it logs to
//! the browser console for debugging purposes.

use crate::models::{AuditLogEntry, AuditAction, AuditTargetType};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// In-memory audit logger for frontend applications.
///
/// This logger provides local buffering of audit events before they are sent to
/// the backend. It maintains a fixed-size buffer of recent entries and provides
/// methods for logging both successful and failed operations.
///
/// The logger is thread-safe and can be shared across multiple components using
/// `Arc<Mutex<>>`. It automatically manages memory by limiting the number of
/// stored entries and removing old entries when the limit is exceeded.
///
/// # Examples
///
/// ```rust
/// let logger = AuditLogger::new(1000); // Keep last 1000 entries
///
/// // Log a successful operation
/// logger.log_success(
///     "admin@example.com".to_string(),
///     AuditAction::ConfigUpdate,
///     AuditTargetType::Config,
///     "server_config".to_string(),
///     Some(serde_json::json!({"enabled": false})),
///     Some(serde_json::json!({"enabled": true})),
/// );
///
/// // Log a failed operation
/// logger.log_failure(
///     "admin@example.com".to_string(),
///     AuditAction::UserCreate,
///     AuditTargetType::User,
///     "invalid_user".to_string(),
///     "Username already exists".to_string(),
/// );
///
/// // Retrieve recent entries
/// let recent = logger.get_recent_entries(10);
/// println!("Recent audit events: {}", recent.len());
/// ```
#[derive(Clone)]
pub struct AuditLogger {
    entries: Arc<Mutex<VecDeque<AuditLogEntry>>>,
    max_entries: usize,
}

impl AuditLogger {
    /// Creates a new audit logger with the specified maximum entry count.
    ///
    /// # Parameters
    ///
    /// - `max_entries`: The maximum number of audit entries to keep in memory
    ///
    /// # Returns
    ///
    /// Returns a new `AuditLogger` instance with an empty entry buffer.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let logger = AuditLogger::new(500); // Keep last 500 entries
    /// ```
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(VecDeque::new())),
            max_entries,
        }
    }

    /// Logs an administrative action with optional details.
    ///
    /// This method provides a unified interface for logging both successful and failed
    /// operations. It determines success/failure based on whether an error message is provided.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator who performed the action
    /// - `action`: The type of administrative action
    /// - `target_type`: The type of resource that was targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `details`: Optional JSON value with additional operation details
    /// - `description`: Human-readable description of the operation
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Log a successful action
    /// logger.log_action(
    ///     "admin@example.com",
    ///     AuditAction::UserCreate,
    ///     AuditTargetType::User,
    ///     "@newuser:example.com",
    ///     Some(serde_json::json!({"username": "newuser"})),
    ///     "Created new user",
    /// ).await;
    /// ```
    pub async fn log_action(
        &self,
        admin_user_id: &str,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: &str,
        details: Option<serde_json::Value>,
        _description: &str,
    ) {
        self.log_success(
            admin_user_id.to_string(),
            action,
            target_type,
            target_id.to_string(),
            None, // old_value
            details, // new_value (details)
        );
    }

    /// Logs a failed administrative action.
    ///
    /// This method is a convenience wrapper for logging failed operations with
    /// an error message.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator who attempted the action
    /// - `action`: The type of administrative action that was attempted
    /// - `target_type`: The type of resource that was targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `error_message`: A description of why the operation failed
    ///
    /// # Examples
    ///
    /// ```rust
    /// logger.log_action_failure(
    ///     "admin@example.com",
    ///     AuditAction::UserCreate,
    ///     AuditTargetType::User,
    ///     "invalid_user",
    ///     "Username already exists",
    /// ).await;
    /// ```
    pub async fn log_action_failure(
        &self,
        admin_user_id: &str,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: &str,
        error_message: &str,
    ) {
        self.log_failure(
            admin_user_id.to_string(),
            action,
            target_type,
            target_id.to_string(),
            error_message.to_string(),
        );
    }

    /// Logs a successful administrative action.
    ///
    /// This method creates an audit log entry for a successful operation and
    /// adds it to the internal buffer. The entry includes optional before and
    /// after values to track state changes.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator who performed the action
    /// - `action`: The type of administrative action that was performed
    /// - `target_type`: The type of resource that was targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `old_value`: Optional JSON value representing the state before the action
    /// - `new_value`: Optional JSON value representing the state after the action
    ///
    /// # Examples
    ///
    /// ```rust
    /// logger.log_success(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::RoomEnable,
    ///     AuditTargetType::Room,
    ///     "!room123:example.com".to_string(),
    ///     Some(serde_json::json!({"enabled": false})),
    ///     Some(serde_json::json!({"enabled": true})),
    /// );
    /// ```
    pub fn log_success(
        &self,
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
    ) {
        let entry = AuditLogEntry::success_with_values(
            admin_user_id,
            action,
            target_type,
            target_id,
            old_value,
            new_value,
        );
        
        self.add_entry(entry);
    }

    /// Logs a failed administrative action.
    ///
    /// This method creates an audit log entry for a failed operation and
    /// adds it to the internal buffer. The entry includes the error message
    /// that describes why the operation failed.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator who attempted the action
    /// - `action`: The type of administrative action that was attempted
    /// - `target_type`: The type of resource that was targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `error_message`: A description of why the operation failed
    ///
    /// # Examples
    ///
    /// ```rust
    /// logger.log_failure(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::UserDeactivate,
    ///     AuditTargetType::User,
    ///     "@user123:example.com".to_string(),
    ///     "User not found in database".to_string(),
    /// );
    /// ```
    pub fn log_failure(
        &self,
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
        error_message: String,
    ) {
        let entry = AuditLogEntry::failure(
            admin_user_id,
            action,
            target_type,
            target_id,
            error_message,
        );
        
        self.add_entry(entry);
    }

    /// Adds an audit log entry to the internal buffer.
    ///
    /// This private method handles the actual storage of audit entries. It assigns
    /// a sequential ID to the entry, adds it to the buffer, and ensures the buffer
    /// doesn't exceed the maximum size by removing old entries.
    ///
    /// # Parameters
    ///
    /// - `entry`: The audit log entry to add (ID will be assigned automatically)
    fn add_entry(&self, mut entry: AuditLogEntry) {
        if let Ok(mut entries) = self.entries.lock() {
            // Assign a simple ID based on current length
            entry.id = entries.len() as i64 + 1;
            
            entries.push_back(entry);
            
            // Keep only the most recent entries
            while entries.len() > self.max_entries {
                entries.pop_front();
            }
            
            // In a real application, you would also send this to the backend
            self.send_to_backend(&entries.back().unwrap());
        }
    }

    /// Retrieves all logged audit entries.
    ///
    /// This method returns a copy of all audit entries currently stored in the buffer.
    /// The entries are returned in chronological order (oldest first).
    ///
    /// # Returns
    ///
    /// Returns a `Vec<AuditLogEntry>` containing all stored entries, or an empty
    /// vector if the lock cannot be acquired.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let all_entries = logger.get_entries();
    /// println!("Total logged entries: {}", all_entries.len());
    /// ```
    pub fn get_entries(&self) -> Vec<AuditLogEntry> {
        if let Ok(entries) = self.entries.lock() {
            entries.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    /// Retrieves the most recent audit entries.
    ///
    /// This method returns up to `count` of the most recent audit entries,
    /// ordered chronologically (oldest first within the returned set).
    ///
    /// # Parameters
    ///
    /// - `count`: The maximum number of recent entries to return
    ///
    /// # Returns
    ///
    /// Returns a `Vec<AuditLogEntry>` containing up to `count` recent entries,
    /// or an empty vector if the lock cannot be acquired.
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Get the 5 most recent audit entries
    /// let recent = logger.get_recent_entries(5);
    /// for entry in recent {
    ///     println!("{}: {}", entry.timestamp, entry.description());
    /// }
    /// ```
    pub fn get_recent_entries(&self, count: usize) -> Vec<AuditLogEntry> {
        if let Ok(entries) = self.entries.lock() {
            entries
                .iter()
                .rev()
                .take(count)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Clears all stored audit entries.
    ///
    /// This method removes all audit entries from the internal buffer.
    /// Use with caution as this will permanently delete the local audit history.
    ///
    /// # Examples
    ///
    /// ```rust
    /// logger.clear();
    /// assert_eq!(logger.count(), 0);
    /// ```
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    /// Returns the current number of stored audit entries.
    ///
    /// # Returns
    ///
    /// Returns the count of audit entries currently in the buffer, or 0 if
    /// the lock cannot be acquired.
    ///
    /// # Examples
    ///
    /// ```rust
    /// println!("Current audit entries: {}", logger.count());
    /// ```
    pub fn count(&self) -> usize {
        if let Ok(entries) = self.entries.lock() {
            entries.len()
        } else {
            0
        }
    }

    /// Sends an audit entry to the backend (placeholder implementation).
    ///
    /// This method represents where backend integration would occur in a real
    /// application. Currently, it logs the entry to the browser console for
    /// debugging purposes.
    ///
    /// In a production implementation, this would:
    /// - Serialize the entry to JSON
    /// - Send an HTTP request to the backend audit API
    /// - Handle network errors and retry logic
    /// - Potentially queue entries for batch sending
    ///
    /// # Parameters
    ///
    /// - `entry`: The audit log entry to send to the backend
    fn send_to_backend(&self, entry: &AuditLogEntry) {
        // In a real implementation, this would make an HTTP request to the backend
        // For now, we just log to console in WASM environments
        #[cfg(target_arch = "wasm32")]
        {
            web_sys::console::log_1(&format!("Audit Log: {}", entry.description()).into());
        }
        
        #[cfg(not(target_arch = "wasm32"))]
        {
            // In non-WASM environments (like tests), just print to stdout
            println!("Audit Log: {}", entry.description());
        }
    }
}

impl Default for AuditLogger {
    /// Creates a default audit logger with a capacity of 1000 entries.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let logger = AuditLogger::default();
    /// assert_eq!(logger.max_entries, 1000);
    /// ```
    fn default() -> Self {
        Self::new(1000) // Keep last 1000 entries
    }
}

/// Global audit logger instance for application-wide use.
///
/// This static variable holds a single instance of `AuditLogger` that can be
/// shared across the entire application. It's initialized once using lazy
/// initialization and then accessed through convenience functions.
static mut GLOBAL_LOGGER: Option<AuditLogger> = None;
static INIT: std::sync::Once = std::sync::Once::new();

/// Retrieves the global audit logger instance.
///
/// This function provides access to a shared audit logger instance that is
/// initialized on first use with default settings (1000 entry capacity).
/// Subsequent calls return the same instance.
///
/// # Returns
///
/// Returns a reference to the global `AuditLogger` instance.
///
/// # Examples
///
/// ```rust
/// let logger = get_audit_logger();
/// logger.log_success(...);
/// ```
///
/// # Safety
///
/// This function uses unsafe static access, which is acceptable in single-threaded
/// WASM environments. The `Once` guard ensures thread-safe initialization.
#[allow(static_mut_refs)]
pub fn get_audit_logger() -> &'static AuditLogger {
    unsafe {
        INIT.call_once(|| {
            GLOBAL_LOGGER = Some(AuditLogger::default());
        });
        GLOBAL_LOGGER.as_ref().unwrap()
    }
}

/// Convenience function to log a successful action using the global logger.
///
/// This function provides a simplified interface for logging successful operations
/// without needing to access the global logger directly.
///
/// # Parameters
///
/// - `admin_user_id`: The ID of the administrator who performed the action
/// - `action`: The type of administrative action that was performed
/// - `target_type`: The type of resource that was targeted
/// - `target_id`: The specific identifier of the target resource
/// - `old_value`: Optional JSON value representing the state before the action
/// - `new_value`: Optional JSON value representing the state after the action
///
/// # Examples
///
/// ```rust
/// log_success(
///     "admin@example.com".to_string(),
///     AuditAction::ConfigUpdate,
///     AuditTargetType::Config,
///     "server_config".to_string(),
///     Some(serde_json::json!({"old": "value"})),
///     Some(serde_json::json!({"new": "value"})),
/// );
/// ```
pub fn log_success(
    admin_user_id: String,
    action: AuditAction,
    target_type: AuditTargetType,
    target_id: String,
    old_value: Option<serde_json::Value>,
    new_value: Option<serde_json::Value>,
) {
    get_audit_logger().log_success(admin_user_id, action, target_type, target_id, old_value, new_value);
}

/// Convenience function to log a failed action using the global logger.
///
/// This function provides a simplified interface for logging failed operations
/// without needing to access the global logger directly.
///
/// # Parameters
///
/// - `admin_user_id`: The ID of the administrator who attempted the action
/// - `action`: The type of administrative action that was attempted
/// - `target_type`: The type of resource that was targeted
/// - `target_id`: The specific identifier of the target resource
/// - `error_message`: A description of why the operation failed
///
/// # Examples
///
/// ```rust
/// log_failure(
///     "admin@example.com".to_string(),
///     AuditAction::UserCreate,
///     AuditTargetType::User,
///     "invalid_user".to_string(),
///     "Username contains invalid characters".to_string(),
/// );
/// ```
pub fn log_failure(
    admin_user_id: String,
    action: AuditAction,
    target_type: AuditTargetType,
    target_id: String,
    error_message: String,
) {
    get_audit_logger().log_failure(admin_user_id, action, target_type, target_id, error_message);
}

/// Convenience function to get recent audit entries using the global logger.
///
/// This function provides a simplified interface for retrieving recent audit
/// entries without needing to access the global logger directly.
///
/// # Parameters
///
/// - `count`: The maximum number of recent entries to return
///
/// # Returns
///
/// Returns a `Vec<AuditLogEntry>` containing up to `count` recent entries.
///
/// # Examples
///
/// ```rust
/// let recent_entries = get_recent_entries(10);
/// for entry in recent_entries {
///     println!("{}: {}", entry.admin_user_id, entry.description());
/// }
/// ```
pub fn get_recent_entries(count: usize) -> Vec<AuditLogEntry> {
    get_audit_logger().get_recent_entries(count)
}

/// Convenience function to get all audit entries using the global logger.
///
/// This function provides a simplified interface for retrieving all stored
/// audit entries without needing to access the global logger directly.
///
/// # Returns
///
/// Returns a `Vec<AuditLogEntry>` containing all stored audit entries.
///
/// # Examples
///
/// ```rust
/// let all_entries = get_all_entries();
/// println!("Total audit entries: {}", all_entries.len());
/// ```
pub fn get_all_entries() -> Vec<AuditLogEntry> {
    get_audit_logger().get_entries()
}