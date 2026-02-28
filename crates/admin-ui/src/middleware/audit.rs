//! # Audit Middleware
//!
//! This module provides middleware components for automatic audit logging of administrative
//! operations in the Palpo Matrix server administration interface. The middleware integrates
//! seamlessly with operation workflows to provide transparent audit trail generation.
//!
//! ## Features
//!
//! - **Automatic Logging**: Transparently logs operation results without manual intervention
//! - **Context Management**: Provides structured context for audit operations
//! - **Flexible Integration**: Can be integrated into various operation patterns
//! - **Before/After Tracking**: Supports tracking state changes with old and new values
//! - **Error Handling**: Automatically captures and logs error conditions
//!
//! ## Usage Patterns
//!
//! ### Basic Middleware Usage
//!
//! ```ignore
//! use std::rc::Rc;
//! use std::cell::RefCell;
//! use crate::services::AuditService;
//! use crate::middleware::audit::AuditMiddleware;
//!
//! let service = Rc::new(RefCell::new(AuditService::new()));
//! let middleware = AuditMiddleware::new(service);
//!
//! // Wrap operations for automatic logging
//! let result = middleware.log_operation(
//!     "admin@example.com".to_string(),
//!     AuditAction::ConfigUpdate,
//!     AuditTargetType::Config,
//!     "server_config".to_string(),
//!     None,
//!     Some(serde_json::json!({"updated": true})),
//!     perform_config_update(),
//! );
//! ```
//!
//! ### Using Audit Context
//!
//!```ignore
//! let context = AuditContext::new(
//!     "admin@example.com".to_string(),
//!     AuditAction::UserCreate,
//!     AuditTargetType::User,
//!     "new_user".to_string(),
//! ).with_new_value(Some(serde_json::json!({"username": "new_user"})));
//!
//! let result = audit_operation!(middleware, context, create_user());
//! ```
//!
//! ### Macro-Based Logging
//!
//! ```ignore
//! // Simple context creation
//! let context = audit_context!(
//!     "admin@example.com".to_string(),
//!     AuditAction::ConfigUpdate,
//!     AuditTargetType::Config,
//!     "server_config".to_string()
//! );
//!
//! // With before/after values
//! let context = audit_context!(
//!     "admin@example.com".to_string(),
//!     AuditAction::ConfigUpdate,
//!     AuditTargetType::Config,
//!     "server_config".to_string(),
//!     Some(old_value),
//!     Some(new_value)
//! );
//! ```

use crate::models::{AuditAction, AuditTargetType};
use crate::services::AuditService;
use serde_json::Value;
use std::rc::Rc;
use std::cell::RefCell;

/// Audit middleware for automatic logging of administrative operations.
///
/// This middleware provides a transparent way to add audit logging to administrative
/// operations. It wraps operation results and automatically records them in the audit
/// system based on whether they succeed or fail.
///
/// The middleware is designed for single-threaded WASM environments and uses
/// `Rc<RefCell<>>` for shared access to the audit service. For multi-threaded
/// environments, consider using `Arc<Mutex<>>` instead.
///
/// # Examples
/// #[ignore]
/// ```ignore
/// let service = Rc::new(RefCell::new(AuditService::new()));
/// let middleware = AuditMiddleware::new(service);
///
/// // Automatically log a configuration update
/// let result = middleware.log_operation(
///     "admin@example.com".to_string(),
///     AuditAction::ConfigUpdate,
///     AuditTargetType::Config,
///     "server_config".to_string(),
///     Some(serde_json::json!({"old": "value"})),
///     Some(serde_json::json!({"new": "value"})),
///     update_server_config(),
/// );
/// ```
pub struct AuditMiddleware {
    audit_service: Rc<RefCell<AuditService>>,
}

impl AuditMiddleware {
    /// Creates a new audit middleware instance.
    ///
    /// # Parameters
    ///
    /// - `audit_service`: A shared reference to an `AuditService` instance
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let service = Rc::new(RefCell::new(AuditService::new()));
    /// let middleware = AuditMiddleware::new(service);
    /// ```
    pub fn new(audit_service: Rc<RefCell<AuditService>>) -> Self {
        Self { audit_service }
    }

    /// Logs an operation with automatic success/failure detection.
    ///
    /// This method wraps an operation result and automatically determines whether
    /// to log it as a success or failure. For successful operations, it records
    /// the before and after values. For failed operations, it records the error message.
    ///
    /// # Type Parameters
    ///
    /// - `T`: The success type of the operation result
    /// - `E`: The error type of the operation result (must implement `Display` and `Clone`)
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator performing the operation
    /// - `action`: The type of administrative action being performed
    /// - `target_type`: The type of resource being targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `old_value`: Optional JSON value representing the state before the operation
    /// - `new_value`: Optional JSON value representing the state after the operation
    /// - `result`: The operation result to be logged and returned unchanged
    ///
    /// # Returns
    ///
    /// Returns the original `result` parameter unchanged, allowing transparent integration
    /// into operation chains.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Log a user creation operation
    /// let result = middleware.log_operation(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::UserCreate,
    ///     AuditTargetType::User,
    ///     "new_user".to_string(),
    ///     None,
    ///     Some(serde_json::json!({"username": "new_user", "active": true})),
    ///     create_user("new_user"), // Returns Result<User, CreateUserError>
    /// );
    ///
    /// // The result is unchanged, but the operation is now logged
    /// match result {
    ///     Ok(user) => println!("User created successfully: {}", user.id),
    ///     Err(error) => println!("User creation failed: {}", error),
    /// }
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
        E: std::fmt::Display + Clone,
    {
        let mut service = self.audit_service.borrow_mut();
        
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

    /// Logs a successful operation directly.
    ///
    /// This method bypasses the automatic success/failure detection and directly
    /// records a successful operation in the audit log. Use this when you want
    /// to explicitly log a success without wrapping a `Result` type.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator who performed the operation
    /// - `action`: The type of administrative action that was performed
    /// - `target_type`: The type of resource that was targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `old_value`: Optional JSON value representing the state before the operation
    /// - `new_value`: Optional JSON value representing the state after the operation
    ///
    /// # Returns
    ///
    /// Returns the ID of the created audit log entry, or an `ApiError` on failure.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Log a successful configuration reload
    /// let entry_id = middleware.log_success(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::ConfigReload,
    ///     AuditTargetType::Server,
    ///     "main_server".to_string(),
    ///     None,
    ///     Some(serde_json::json!({"reloaded_at": "2024-01-01T12:00:00Z"})),
    /// ).unwrap();
    /// ```
    pub fn log_success(
        &self,
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
        old_value: Option<Value>,
        new_value: Option<Value>,
    ) -> Result<i64, crate::models::error::ApiError> {
        let mut service = self.audit_service.borrow_mut();
        service.record_success(
            admin_user_id,
            action,
            target_type,
            target_id,
            old_value,
            new_value,
        )
    }

    /// Logs a failed operation directly.
    ///
    /// This method bypasses the automatic success/failure detection and directly
    /// records a failed operation in the audit log. Use this when you want to
    /// explicitly log a failure without wrapping a `Result` type.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator who attempted the operation
    /// - `action`: The type of administrative action that was attempted
    /// - `target_type`: The type of resource that was targeted
    /// - `target_id`: The specific identifier of the target resource
    /// - `error_message`: A description of why the operation failed
    ///
    /// # Returns
    ///
    /// Returns the ID of the created audit log entry, or an `ApiError` on failure.
    ///
    /// # Examples
    ///```ignore
    /// // Log a failed server restart
    /// let entry_id = middleware.log_failure(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::ServerRestart,
    ///     AuditTargetType::Server,
    ///     "main_server".to_string(),
    ///     "Insufficient privileges to restart server".to_string(),
    /// ).unwrap();
    /// ```
    pub fn log_failure(
        &self,
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
        error_message: String,
    ) -> Result<i64, crate::models::error::ApiError> {
        let mut service = self.audit_service.borrow_mut();
        service.record_failure(
            admin_user_id,
            action,
            target_type,
            target_id,
            error_message,
        )
    }
}

/// Trait for extracting audit information from request objects.
///
/// This trait provides a standardized way to extract audit-relevant information
/// from various types of request objects. Implement this trait for your request
/// types to enable automatic audit information extraction.
///
/// # Examples
///
/// ```ignore
/// struct ConfigUpdateRequest {
///     admin_id: String,
///     config_key: String,
///     old_value: serde_json::Value,
///     new_value: serde_json::Value,
/// }
///
/// impl AuditInfo for ConfigUpdateRequest {
///     fn get_admin_user_id(&self) -> Option<String> {
///         Some(self.admin_id.clone())
///     }
///
///     fn get_target_id(&self) -> Option<String> {
///         Some(self.config_key.clone())
///     }
///
///     fn get_old_value(&self) -> Option<Value> {
///         Some(self.old_value.clone())
///     }
///
///     fn get_new_value(&self) -> Option<Value> {
///         Some(self.new_value.clone())
///     }
/// }
/// ```
pub trait AuditInfo {
    /// Extracts the administrator user ID from the request.
    ///
    /// # Returns
    ///
    /// Returns `Some(String)` containing the admin user ID, or `None` if not available.
    fn get_admin_user_id(&self) -> Option<String>;
    
    /// Extracts the target resource ID from the request.
    ///
    /// # Returns
    ///
    /// Returns `Some(String)` containing the target ID, or `None` if not available.
    fn get_target_id(&self) -> Option<String>;
    
    /// Extracts the old/previous value from the request.
    ///
    /// # Returns
    ///
    /// Returns `Some(Value)` containing the old value, or `None` if not available.
    fn get_old_value(&self) -> Option<Value>;
    
    /// Extracts the new/updated value from the request.
    ///
    /// # Returns
    ///
    /// Returns `Some(Value)` containing the new value, or `None` if not available.
    fn get_new_value(&self) -> Option<Value>;
}

/// Structured context for audit operations.
///
/// This struct provides a convenient way to package all the information needed
/// for an audit log entry. It supports a builder pattern for easy construction
/// and can be used with the audit middleware for streamlined logging.
///
/// # Examples
///
/// ```ignore,ignore
/// // Basic context
/// let context = AuditContext::new(
///     "admin@example.com".to_string(),
///     AuditAction::UserUpdate,
///     AuditTargetType::User,
///     "user123".to_string(),
/// );
///
/// // Context with before/after values
/// let context = AuditContext::new(
///     "admin@example.com".to_string(),
///     AuditAction::ConfigUpdate,
///     AuditTargetType::Config,
///     "server_config".to_string(),
/// )
/// .with_old_value(Some(serde_json::json!({"enabled": false})))
/// .with_new_value(Some(serde_json::json!({"enabled": true})));
/// ```
#[derive(Clone, Debug)]
pub struct AuditContext {
    pub admin_user_id: String,
    pub action: AuditAction,
    pub target_type: AuditTargetType,
    pub target_id: String,
    pub old_value: Option<Value>,
    pub new_value: Option<Value>,
}

impl AuditContext {
    /// Creates a new audit context with the required fields.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator performing the operation
    /// - `action`: The type of administrative action being performed
    /// - `target_type`: The type of resource being targeted
    /// - `target_id`: The specific identifier of the target resource
    ///
    /// # Returns
    ///
    /// Returns a new `AuditContext` instance with the specified parameters and
    /// `None` values for `old_value` and `new_value`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let context = AuditContext::new(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::RoomDisable,
    ///     AuditTargetType::Room,
    ///     "!room123:example.com".to_string(),
    /// );
    /// ```
    pub fn new(
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
    ) -> Self {
        Self {
            admin_user_id,
            action,
            target_type,
            target_id,
            old_value: None,
            new_value: None,
        }
    }

    /// Sets the old value for the audit context using a builder pattern.
    ///
    /// # Parameters
    ///
    /// - `old_value`: Optional JSON value representing the state before the operation
    ///
    /// # Returns
    ///
    /// Returns `self` with the `old_value` field updated, enabling method chaining.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let context = AuditContext::new(...)
    ///     .with_old_value(Some(serde_json::json!({"status": "active"})));
    /// ```
    pub fn with_old_value(mut self, old_value: Option<Value>) -> Self {
        self.old_value = old_value;
        self
    }

    /// Sets the new value for the audit context using a builder pattern.
    ///
    /// # Parameters
    ///
    /// - `new_value`: Optional JSON value representing the state after the operation
    ///
    /// # Returns
    ///
    /// Returns `self` with the `new_value` field updated, enabling method chaining.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let context = AuditContext::new(...)
    ///     .with_new_value(Some(serde_json::json!({"status": "disabled"})));
    /// ```
    pub fn with_new_value(mut self, new_value: Option<Value>) -> Self {
        self.new_value = new_value;
        self
    }

    /// Sets both old and new values for the audit context using a builder pattern.
    ///
    /// This is a convenience method that combines `with_old_value` and `with_new_value`
    /// into a single call for operations that modify existing resources.
    ///
    /// # Parameters
    ///
    /// - `old_value`: Optional JSON value representing the state before the operation
    /// - `new_value`: Optional JSON value representing the state after the operation
    ///
    /// # Returns
    ///
    /// Returns `self` with both value fields updated, enabling method chaining.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let context = AuditContext::new(...)
    ///     .with_values(
    ///         Some(serde_json::json!({"enabled": false})),
    ///         Some(serde_json::json!({"enabled": true}))
    ///     );
    /// ```
    pub fn with_values(mut self, old_value: Option<Value>, new_value: Option<Value>) -> Self {
        self.old_value = old_value;
        self.new_value = new_value;
        self
    }
}

/// Convenience macro for logging operations using audit context.
///
/// This macro simplifies the process of logging operations by accepting an
/// `AuditContext` and an operation result. It extracts the necessary information
/// from the context and passes it to the middleware's `log_operation` method.
///
/// # Parameters
///
/// - `$middleware`: The `AuditMiddleware` instance to use for logging
/// - `$context`: An `AuditContext` containing the operation details
/// - `$operation`: The operation result to be logged
///
/// # Examples
///
/// ```ignore,ignore
/// let context = AuditContext::new(
///     "admin@example.com".to_string(),
///     AuditAction::UserCreate,
///     AuditTargetType::User,
///     "new_user".to_string(),
/// );
///
/// let result = audit_operation!(middleware, context, create_user());
/// ```
#[macro_export]
macro_rules! audit_operation {
    ($middleware:expr, $context:expr, $operation:expr) => {
        $middleware.log_operation(
            $context.admin_user_id,
            $context.action,
            $context.target_type,
            $context.target_id,
            $context.old_value,
            $context.new_value,
            $operation,
        )
    };
}

/// Convenience macro for creating audit contexts.
///
/// This macro provides a simplified way to create `AuditContext` instances with
/// optional before/after values. It supports two variants: basic context creation
/// and context creation with values.
///
/// # Variants
///
/// ## Basic Context Creation
/// ```ignore,ignore
/// audit_context!(user_id, action, target_type, target_id)
/// ```
///
/// ## Context Creation with Values
/// ```ignore,ignore
/// audit_context!(user_id, action, target_type, target_id, old_value, new_value)
/// ```
///
/// # Parameters
///
/// - `$user`: The administrator user ID
/// - `$action`: The `AuditAction` being performed
/// - `$target_type`: The `AuditTargetType` being targeted
/// - `$target_id`: The specific target identifier
/// - `$old`: (Optional variant) The old/previous value
/// - `$new`: (Optional variant) The new/updated value
///
/// # Examples
///
/// ```ignore,ignore
/// // Basic context
/// let context = audit_context!(
///     "admin@example.com".to_string(),
///     AuditAction::ConfigUpdate,
///     AuditTargetType::Config,
///     "server_config".to_string()
/// );
///
/// // Context with values
/// let context = audit_context!(
///     "admin@example.com".to_string(),
///     AuditAction::ConfigUpdate,
///     AuditTargetType::Config,
///     "server_config".to_string(),
///     Some(old_config),
///     Some(new_config)
/// );
/// ```
#[macro_export]
macro_rules! audit_context {
    ($user:expr, $action:expr, $target_type:expr, $target_id:expr) => {
        $crate::middleware::audit::AuditContext::new(
            $user,
            $action,
            $target_type,
            $target_id,
        )
    };
    
    ($user:expr, $action:expr, $target_type:expr, $target_id:expr, $old:expr, $new:expr) => {
        $crate::middleware::audit::AuditContext::new(
            $user,
            $action,
            $target_type,
            $target_id,
        ).with_values($old, $new)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::AuditService;

    #[test]
    fn test_audit_middleware_success() {
        let service = Rc::new(RefCell::new(AuditService::new()));
        let middleware = AuditMiddleware::new(service.clone());
        
        let context = AuditContext::new(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "test_config".to_string(),
        );
        
        let result = middleware.log_operation(
            context.admin_user_id,
            context.action,
            context.target_type,
            context.target_id,
            context.old_value,
            context.new_value,
            Ok::<_, String>("success"),
        );
        
        assert!(result.is_ok());
        
        // Check that the audit log was recorded
        let service_guard = service.borrow();
        let logs = service_guard.query_logs(Default::default()).unwrap();
        assert_eq!(logs.entries.len(), 1);
        assert!(logs.entries[0].success);
    }

    #[test]
    fn test_audit_middleware_failure() {
        let service = Rc::new(RefCell::new(AuditService::new()));
        let middleware = AuditMiddleware::new(service.clone());
        
        let context = AuditContext::new(
            "admin@example.com".to_string(),
            AuditAction::UserCreate,
            AuditTargetType::User,
            "test_user".to_string(),
        );
        
        let result = middleware.log_operation(
            context.admin_user_id,
            context.action,
            context.target_type,
            context.target_id,
            context.old_value,
            context.new_value,
            Err::<String, _>("validation failed"),
        );
        
        assert!(result.is_err());
        
        // Check that the audit log was recorded
        let service_guard = service.borrow();
        let logs = service_guard.query_logs(Default::default()).unwrap();
        assert_eq!(logs.entries.len(), 1);
        assert!(!logs.entries[0].success);
        assert_eq!(logs.entries[0].error_message, Some("validation failed".to_string()));
    }

    #[test]
    fn test_audit_context_builder() {
        let context = AuditContext::new(
            "admin@example.com".to_string(),
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "test_config".to_string(),
        )
        .with_old_value(Some(serde_json::json!({"old": "value"})))
        .with_new_value(Some(serde_json::json!({"new": "value"})));
        
        assert_eq!(context.admin_user_id, "admin@example.com");
        assert!(context.old_value.is_some());
        assert!(context.new_value.is_some());
    }
}