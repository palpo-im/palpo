//! # Audit Log Data Models
//!
//! This module defines the core data structures used throughout the audit logging system.
//! It includes models for audit log entries, filters, responses, and enumerations for
//! actions and target types.
//!
//! ## Core Models
//!
//! - **`AuditLogEntry`**: Represents a single audit log record
//! - **`AuditLogFilter`**: Specifies criteria for querying audit logs
//! - **`AuditLogResponse`**: Contains query results with pagination metadata
//! - **`AuditAction`**: Enumeration of administrative actions that can be audited
//! - **`AuditTargetType`**: Enumeration of resource types that can be targeted
//! - **`AuditSeverity`**: Severity levels for UI styling and categorization
//!
//! ## Usage Examples
//!
//! ### Creating Audit Entries
//!
//! ```rust
//! // Create a successful operation entry
//! let entry = AuditLogEntry::success_with_values(
//!     "admin@example.com".to_string(),
//!     AuditAction::ConfigUpdate,
//!     AuditTargetType::Config,
//!     "server_config".to_string(),
//!     Some(serde_json::json!({"old": "value"})),
//!     Some(serde_json::json!({"new": "value"})),
//! );
//!
//! // Create a failed operation entry
//! let entry = AuditLogEntry::failure(
//!     "admin@example.com".to_string(),
//!     AuditAction::UserCreate,
//!     AuditTargetType::User,
//!     "invalid_user".to_string(),
//!     "Username already exists".to_string(),
//! );
//! ```
//!
//! ### Filtering Audit Logs
//!
//! ```rust
//! let filter = AuditLogFilter {
//!     success: Some(false), // Only failed operations
//!     action: Some(AuditAction::ConfigUpdate),
//!     start_time: Some(SystemTime::now() - Duration::from_secs(3600)),
//!     limit: Some(50),
//!     ..Default::default()
//! };
//! ```

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Represents a single audit log entry recording an administrative operation.
///
/// This struct contains all the information needed to track what administrative
/// action was performed, by whom, when, and whether it succeeded or failed.
/// It supports tracking before/after values for operations that modify existing
/// resources.
///
/// # Fields
///
/// - `id`: Unique identifier for the audit entry (typically assigned by database)
/// - `timestamp`: When the operation was performed
/// - `admin_user_id`: ID of the administrator who performed the operation
/// - `action`: The type of administrative action that was performed
/// - `target_type`: The type of resource that was targeted
/// - `target_id`: Specific identifier of the target resource
/// - `old_value`: Optional JSON value representing the state before the operation
/// - `new_value`: Optional JSON value representing the state after the operation
/// - `success`: Whether the operation completed successfully
/// - `error_message`: Error description if the operation failed
///
/// # Examples
///
/// ```rust
/// // Successful configuration update
/// let entry = AuditLogEntry::success_with_values(
///     "admin@example.com".to_string(),
///     AuditAction::ConfigUpdate,
///     AuditTargetType::Config,
///     "server_config".to_string(),
///     Some(serde_json::json!({"enabled": false})),
///     Some(serde_json::json!({"enabled": true})),
/// );
///
/// // Failed user creation
/// let entry = AuditLogEntry::failure(
///     "admin@example.com".to_string(),
///     AuditAction::UserCreate,
///     AuditTargetType::User,
///     "duplicate_user".to_string(),
///     "User already exists".to_string(),
/// );
/// ```
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

/// Severity levels for audit entries, used for UI styling and categorization.
///
/// These severity levels help categorize audit entries based on their potential
/// impact and provide appropriate visual styling in the user interface.
///
/// # Variants
///
/// - `Info`: Informational operations (e.g., configuration reads, queries)
/// - `Success`: Successful creation or positive actions
/// - `Warning`: Potentially risky operations or modifications
/// - `Critical`: High-impact operations like server restarts or shutdowns
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum AuditSeverity {
    Info,
    Success,
    Warning,
    Critical,
}

/// Enumeration of administrative actions that can be audited.
///
/// This enum defines all the types of administrative operations that the audit
/// system can track. Each action has an associated severity level and human-readable
/// description for display purposes.
///
/// # Examples
///
/// ```rust
/// let action = AuditAction::ConfigUpdate;
/// println!("Action: {}", action.description());
/// println!("Severity: {:?}", action.severity());
/// ```
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

/// Enumeration of resource types that can be targeted by administrative actions.
///
/// This enum defines the different types of resources that can be the target
/// of administrative operations. Each target type has a human-readable description
/// for display purposes.
///
/// # Examples
///
/// ```rust
/// let target_type = AuditTargetType::User;
/// println!("Target type: {}", target_type.description());
/// ```
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

/// Filter criteria for querying audit logs.
///
/// This struct allows specifying various criteria to filter audit log queries.
/// All fields are optional, allowing for flexible query construction. When multiple
/// criteria are specified, they are combined with AND logic.
///
/// # Fields
///
/// - `start_time`: Filter entries after this timestamp
/// - `end_time`: Filter entries before this timestamp
/// - `admin_user_id`: Filter by specific administrator
/// - `action`: Filter by specific action type
/// - `target_type`: Filter by specific target type
/// - `success`: Filter by operation success status
/// - `limit`: Maximum number of entries to return
/// - `offset`: Number of entries to skip (for pagination)
///
/// # Examples
///
/// ```rust
/// // Query failed operations from the last hour
/// let filter = AuditLogFilter {
///     success: Some(false),
///     start_time: Some(SystemTime::now() - Duration::from_secs(3600)),
///     limit: Some(20),
///     ..Default::default()
/// };
///
/// // Query all config updates by a specific admin
/// let filter = AuditLogFilter {
///     admin_user_id: Some("admin@example.com".to_string()),
///     action: Some(AuditAction::ConfigUpdate),
///     ..Default::default()
/// };
/// ```
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

/// Response structure for audit log queries.
///
/// This struct contains the results of an audit log query along with pagination
/// metadata. It's used by API endpoints to return query results in a structured format.
///
/// # Fields
///
/// - `entries`: The audit log entries that match the query criteria
/// - `total`: Total number of entries that match the criteria (before pagination)
/// - `page`: Current page number (calculated from offset and limit)
/// - `per_page`: Number of entries per page (same as limit)
///
/// # Examples
///
/// ```rust
/// let response = AuditLogResponse {
///     entries: vec![entry1, entry2, entry3],
///     total: 150,
///     page: 2,
///     per_page: 25,
/// };
/// println!("Showing {} of {} entries", response.entries.len(), response.total);
/// ```
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AuditLogResponse {
    pub entries: Vec<AuditLogEntry>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

impl Default for AuditLogFilter {
    /// Creates a default audit log filter with standard pagination settings.
    ///
    /// The default filter has no criteria specified (returns all entries) with
    /// a limit of 50 entries starting from offset 0.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let filter = AuditLogFilter::default();
    /// assert_eq!(filter.limit, Some(50));
    /// assert_eq!(filter.offset, Some(0));
    /// ```
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

impl AuditLogEntry {
    /// Creates a new basic audit log entry.
    ///
    /// This constructor creates a minimal audit log entry with the required fields.
    /// The entry is marked as successful by default, and the timestamp is set to
    /// the current time. The ID is set to 0 and should be assigned by the storage system.
    ///
    /// # Parameters
    ///
    /// - `admin_user_id`: The ID of the administrator performing the action
    /// - `action`: The type of administrative action being performed
    /// - `target_type`: The type of resource being targeted
    /// - `target_id`: The specific identifier of the target resource
    ///
    /// # Returns
    ///
    /// Returns a new `AuditLogEntry` with the specified parameters and default values
    /// for optional fields.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let entry = AuditLogEntry::new(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::ConfigReload,
    ///     AuditTargetType::Server,
    ///     "main_server".to_string(),
    /// );
    /// assert!(entry.success);
    /// assert!(entry.error_message.is_none());
    /// ```
    pub fn new(
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
    ) -> Self {
        Self {
            id: 0, // Will be set by the database
            timestamp: SystemTime::now(),
            admin_user_id,
            action,
            target_type,
            target_id,
            old_value: None,
            new_value: None,
            success: true,
            error_message: None,
        }
    }

    /// Creates a successful audit log entry with before/after values.
    ///
    /// This constructor creates an audit log entry for a successful operation
    /// that includes optional before and after values to track state changes.
    /// This is particularly useful for update operations where you want to
    /// record what changed.
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
    /// # Returns
    ///
    /// Returns a new `AuditLogEntry` marked as successful with the specified values.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let entry = AuditLogEntry::success_with_values(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::ConfigUpdate,
    ///     AuditTargetType::Config,
    ///     "server_config".to_string(),
    ///     Some(serde_json::json!({"max_users": 100})),
    ///     Some(serde_json::json!({"max_users": 200})),
    /// );
    /// assert!(entry.success);
    /// assert!(entry.old_value.is_some());
    /// assert!(entry.new_value.is_some());
    /// ```
    pub fn success_with_values(
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
    ) -> Self {
        Self {
            id: 0,
            timestamp: SystemTime::now(),
            admin_user_id,
            action,
            target_type,
            target_id,
            old_value,
            new_value,
            success: true,
            error_message: None,
        }
    }

    /// Creates a failed audit log entry.
    ///
    /// This constructor creates an audit log entry for a failed operation.
    /// The entry is marked as unsuccessful and includes the error message
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
    /// # Returns
    ///
    /// Returns a new `AuditLogEntry` marked as failed with the specified error message.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let entry = AuditLogEntry::failure(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::UserDeactivate,
    ///     AuditTargetType::User,
    ///     "@nonexistent:example.com".to_string(),
    ///     "User not found in database".to_string(),
    /// );
    /// assert!(!entry.success);
    /// assert_eq!(entry.error_message, Some("User not found in database".to_string()));
    /// ```
    pub fn failure(
        admin_user_id: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: String,
        error_message: String,
    ) -> Self {
        Self {
            id: 0,
            timestamp: SystemTime::now(),
            admin_user_id,
            action,
            target_type,
            target_id,
            old_value: None,
            new_value: None,
            success: false,
            error_message: Some(error_message),
        }
    }

    /// Checks if this audit entry represents a successful operation.
    ///
    /// # Returns
    ///
    /// Returns `true` if the operation was successful, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let success_entry = AuditLogEntry::success_with_values(...);
    /// assert!(success_entry.is_success());
    ///
    /// let failure_entry = AuditLogEntry::failure(...);
    /// assert!(!failure_entry.is_success());
    /// ```
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Checks if this audit entry represents a failed operation.
    ///
    /// # Returns
    ///
    /// Returns `true` if the operation failed, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let success_entry = AuditLogEntry::success_with_values(...);
    /// assert!(!success_entry.is_failure());
    ///
    /// let failure_entry = AuditLogEntry::failure(...);
    /// assert!(failure_entry.is_failure());
    /// ```
    pub fn is_failure(&self) -> bool {
        !self.success
    }

    /// Generates a human-readable description of the audit entry.
    ///
    /// This method creates a descriptive string that summarizes what happened
    /// in the audit entry, including the success/failure status, action type,
    /// target type, and target identifier.
    ///
    /// # Returns
    ///
    /// Returns a `String` containing a human-readable description of the audit entry.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let entry = AuditLogEntry::success_with_values(
    ///     "admin@example.com".to_string(),
    ///     AuditAction::ConfigUpdate,
    ///     AuditTargetType::Config,
    ///     "server_config".to_string(),
    ///     None,
    ///     None,
    /// );
    /// let description = entry.description();
    /// // Returns something like: "successfully configuration updated configuration server_config"
    /// ```
    pub fn description(&self) -> String {
        let action_desc = self.action.description();
        let target_desc = self.target_type.description();
        let status = if self.success { "successfully" } else { "failed to" };
        
        format!("{} {} {} {}", status, action_desc.to_lowercase(), target_desc.to_lowercase(), self.target_id)
    }
}

impl AuditAction {
    /// Returns a human-readable description of the audit action.
    ///
    /// This method provides a user-friendly string representation of the action
    /// that can be displayed in user interfaces and reports.
    ///
    /// # Returns
    ///
    /// Returns a static string describing the action.
    ///
    /// # Examples
    ///
    /// ```rust
    /// assert_eq!(AuditAction::ConfigUpdate.description(), "Configuration Updated");
    /// assert_eq!(AuditAction::UserCreate.description(), "User Created");
    /// assert_eq!(AuditAction::ServerRestart.description(), "Server Restarted");
    /// ```
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

    /// Returns a list of all available audit actions.
    ///
    /// This method is useful for generating UI elements like dropdown menus
    /// or for validation purposes.
    ///
    /// # Returns
    ///
    /// Returns a `Vec<AuditAction>` containing all possible audit actions.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let all_actions = AuditAction::all();
    /// assert!(all_actions.contains(&AuditAction::ConfigUpdate));
    /// assert!(all_actions.contains(&AuditAction::UserCreate));
    /// ```
    pub fn all() -> Vec<AuditAction> {
        vec![
            AuditAction::ConfigUpdate,
            AuditAction::UserCreate,
            AuditAction::UserUpdate,
            AuditAction::UserDeactivate,
            AuditAction::RoomDisable,
            AuditAction::RoomEnable,
            AuditAction::AppserviceRegister,
            AuditAction::AppserviceUnregister,
            AuditAction::MediaDelete,
            AuditAction::FederationDisable,
            AuditAction::ServerRestart,
            AuditAction::ServerShutdown,
            AuditAction::ConfigReload,
        ]
    }

    /// Returns the severity level of the audit action for UI styling.
    ///
    /// This method categorizes actions by their potential impact and risk level,
    /// which can be used to apply appropriate visual styling in the user interface.
    ///
    /// # Returns
    ///
    /// Returns an `AuditSeverity` enum value indicating the severity level.
    ///
    /// # Severity Categories
    ///
    /// - **Info**: Low-impact informational operations
    /// - **Success**: Positive actions like creation or enabling
    /// - **Warning**: Potentially risky operations or modifications
    /// - **Critical**: High-impact operations that could affect system availability
    ///
    /// # Examples
    ///
    /// ```rust
    /// assert_eq!(AuditAction::ConfigUpdate.severity(), AuditSeverity::Info);
    /// assert_eq!(AuditAction::UserCreate.severity(), AuditSeverity::Success);
    /// assert_eq!(AuditAction::UserDeactivate.severity(), AuditSeverity::Warning);
    /// assert_eq!(AuditAction::ServerShutdown.severity(), AuditSeverity::Critical);
    /// ```
    pub fn severity(&self) -> AuditSeverity {
        match self {
            AuditAction::ConfigUpdate | AuditAction::ConfigReload => AuditSeverity::Info,
            AuditAction::UserCreate | AuditAction::RoomEnable | AuditAction::AppserviceRegister => AuditSeverity::Success,
            AuditAction::UserUpdate => AuditSeverity::Warning,
            AuditAction::UserDeactivate | AuditAction::RoomDisable | AuditAction::AppserviceUnregister | AuditAction::MediaDelete | AuditAction::FederationDisable => AuditSeverity::Warning,
            AuditAction::ServerRestart | AuditAction::ServerShutdown => AuditSeverity::Critical,
        }
    }
}

impl AuditTargetType {
    /// Returns a human-readable description of the target type.
    ///
    /// This method provides a user-friendly string representation of the target
    /// type that can be displayed in user interfaces and reports.
    ///
    /// # Returns
    ///
    /// Returns a static string describing the target type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// assert_eq!(AuditTargetType::Config.description(), "Configuration");
    /// assert_eq!(AuditTargetType::User.description(), "User");
    /// assert_eq!(AuditTargetType::Appservice.description(), "Application Service");
    /// ```
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

    /// Returns a list of all available target types.
    ///
    /// This method is useful for generating UI elements like dropdown menus
    /// or for validation purposes.
    ///
    /// # Returns
    ///
    /// Returns a `Vec<AuditTargetType>` containing all possible target types.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let all_targets = AuditTargetType::all();
    /// assert!(all_targets.contains(&AuditTargetType::Config));
    /// assert!(all_targets.contains(&AuditTargetType::User));
    /// ```
    pub fn all() -> Vec<AuditTargetType> {
        vec![
            AuditTargetType::Config,
            AuditTargetType::User,
            AuditTargetType::Room,
            AuditTargetType::Appservice,
            AuditTargetType::Media,
            AuditTargetType::Federation,
            AuditTargetType::Server,
        ]
    }
}

impl AuditSeverity {
    /// Returns CSS classes for styling based on severity level.
    ///
    /// This method provides Tailwind CSS classes that can be applied to UI
    /// elements to visually represent the severity level of audit entries.
    ///
    /// # Returns
    ///
    /// Returns a static string containing CSS classes for text color and background.
    ///
    /// # Examples
    ///
    /// ```rust
    /// assert_eq!(AuditSeverity::Info.css_class(), "text-blue-600 bg-blue-50");
    /// assert_eq!(AuditSeverity::Critical.css_class(), "text-red-600 bg-red-50");
    /// ```
    pub fn css_class(&self) -> &'static str {
        match self {
            AuditSeverity::Info => "text-blue-600 bg-blue-50",
            AuditSeverity::Success => "text-green-600 bg-green-50",
            AuditSeverity::Warning => "text-yellow-600 bg-yellow-50",
            AuditSeverity::Critical => "text-red-600 bg-red-50",
        }
    }

    /// Returns an emoji icon representing the severity level.
    ///
    /// This method provides emoji icons that can be used in UI elements to
    /// quickly convey the severity level of audit entries.
    ///
    /// # Returns
    ///
    /// Returns a static string containing an emoji icon.
    ///
    /// # Examples
    ///
    /// ```rust
    /// assert_eq!(AuditSeverity::Info.icon(), "ℹ️");
    /// assert_eq!(AuditSeverity::Success.icon(), "✅");
    /// assert_eq!(AuditSeverity::Warning.icon(), "⚠️");
    /// assert_eq!(AuditSeverity::Critical.icon(), "🚨");
    /// ```
    pub fn icon(&self) -> &'static str {
        match self {
            AuditSeverity::Info => "ℹ️",
            AuditSeverity::Success => "✅",
            AuditSeverity::Warning => "⚠️",
            AuditSeverity::Critical => "🚨",
        }
    }
}