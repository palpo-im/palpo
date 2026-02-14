//! Error handling models and utilities

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Main error type for the web configuration interface
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum WebConfigError {
    /// Configuration validation errors
    #[error("Configuration validation failed: {message}")]
    ValidationError { message: String, field: Option<String> },

    /// Authentication and authorization errors
    #[error("Authentication failed: {message}")]
    AuthError { message: String },

    /// Authorization/permission errors
    #[error("Access denied: {message}")]
    PermissionError { message: String },

    /// API communication errors
    #[error("API request failed: {message}")]
    ApiError { message: String, status_code: Option<u16> },

    /// File system errors (config file operations)
    #[error("File system error: {message}")]
    FileSystemError { message: String, path: Option<String> },

    /// Database connection/operation errors
    #[error("Database error: {message}")]
    DatabaseError { message: String },

    /// Configuration parsing errors
    #[error("Configuration parsing error: {message}")]
    ParseError { message: String, format: String },

    /// Network/connectivity errors
    #[error("Network error: {message}")]
    NetworkError { message: String },

    /// Server control errors (restart, reload, etc.)
    #[error("Server control error: {message}")]
    ServerControlError { message: String },

    /// Generic internal server errors
    #[error("Internal server error: {message}")]
    InternalError { message: String },

    /// Client-side errors (frontend specific)
    #[error("Client error: {message}")]
    ClientError { message: String },
}

/// Frontend-specific API error type for WASM compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub message: String,
    pub status_code: Option<u16>,
    pub error_code: Option<String>,
    pub details: Option<serde_json::Value>,
}

/// HTTP status code mappings for WebConfigError
impl WebConfigError {
    /// Get the appropriate HTTP status code for this error
    pub fn status_code(&self) -> u16 {
        match self {
            WebConfigError::ValidationError { .. } => 400, // Bad Request
            WebConfigError::AuthError { .. } => 401,       // Unauthorized
            WebConfigError::PermissionError { .. } => 403, // Forbidden
            WebConfigError::ApiError { status_code: Some(code), .. } => *code,
            WebConfigError::ApiError { .. } => 500,        // Internal Server Error
            WebConfigError::FileSystemError { .. } => 500, // Internal Server Error
            WebConfigError::DatabaseError { .. } => 500,   // Internal Server Error
            WebConfigError::ParseError { .. } => 400,      // Bad Request
            WebConfigError::NetworkError { .. } => 503,    // Service Unavailable
            WebConfigError::ServerControlError { .. } => 500, // Internal Server Error
            WebConfigError::InternalError { .. } => 500,   // Internal Server Error
            WebConfigError::ClientError { .. } => 400,     // Bad Request
        }
    }

    /// Get a user-friendly error message
    pub fn user_message(&self) -> String {
        match self {
            WebConfigError::ValidationError { message, field } => {
                if let Some(field) = field {
                    format!("Validation error in field '{}': {}", field, message)
                } else {
                    format!("Validation error: {}", message)
                }
            }
            WebConfigError::AuthError { message } => {
                format!("Authentication required: {}", message)
            }
            WebConfigError::PermissionError { message } => {
                format!("Permission denied: {}", message)
            }
            WebConfigError::ApiError { message, .. } => {
                format!("API error: {}", message)
            }
            WebConfigError::FileSystemError { message, path } => {
                if let Some(path) = path {
                    format!("File system error at '{}': {}", path, message)
                } else {
                    format!("File system error: {}", message)
                }
            }
            WebConfigError::DatabaseError { message } => {
                format!("Database error: {}", message)
            }
            WebConfigError::ParseError { message, format } => {
                format!("Failed to parse {} configuration: {}", format, message)
            }
            WebConfigError::NetworkError { message } => {
                format!("Network error: {}", message)
            }
            WebConfigError::ServerControlError { message } => {
                format!("Server control error: {}", message)
            }
            WebConfigError::InternalError { message } => {
                format!("Internal error: {}", message)
            }
            WebConfigError::ClientError { message } => {
                format!("Client error: {}", message)
            }
        }
    }

    /// Get error code for API responses
    pub fn error_code(&self) -> &'static str {
        match self {
            WebConfigError::ValidationError { .. } => "VALIDATION_ERROR",
            WebConfigError::AuthError { .. } => "AUTH_ERROR",
            WebConfigError::PermissionError { .. } => "PERMISSION_ERROR",
            WebConfigError::ApiError { .. } => "API_ERROR",
            WebConfigError::FileSystemError { .. } => "FILESYSTEM_ERROR",
            WebConfigError::DatabaseError { .. } => "DATABASE_ERROR",
            WebConfigError::ParseError { .. } => "PARSE_ERROR",
            WebConfigError::NetworkError { .. } => "NETWORK_ERROR",
            WebConfigError::ServerControlError { .. } => "SERVER_CONTROL_ERROR",
            WebConfigError::InternalError { .. } => "INTERNAL_ERROR",
            WebConfigError::ClientError { .. } => "CLIENT_ERROR",
        }
    }
}

/// Error response format for API endpoints
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    pub code: String,
    pub status: u16,
    pub details: Option<serde_json::Value>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl ErrorResponse {
    /// Create an error response from WebConfigError
    pub fn from_error(error: WebConfigError) -> Self {
        Self {
            error: error.to_string(),
            message: error.user_message(),
            code: error.error_code().to_string(),
            status: error.status_code(),
            details: None,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Create an error response with additional details
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Result type alias for web configuration operations
pub type WebConfigResult<T> = Result<T, WebConfigError>;

/// Conversion implementations for common error types
impl From<serde_json::Error> for WebConfigError {
    fn from(err: serde_json::Error) -> Self {
        WebConfigError::ParseError {
            message: err.to_string(),
            format: "JSON".to_string(),
        }
    }
}

/// Frontend-specific error handling utilities
impl ApiError {
    /// Create a new API error
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: None,
            error_code: None,
            details: None,
        }
    }

    /// Create an API error with status code
    pub fn with_status(message: impl Into<String>, status_code: u16) -> Self {
        Self {
            message: message.into(),
            status_code: Some(status_code),
            error_code: None,
            details: None,
        }
    }

    /// Create an API error with error code
    pub fn with_code(mut self, error_code: impl Into<String>) -> Self {
        self.error_code = Some(error_code.into());
        self
    }

    /// Add details to the error
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Check if this is a client error (4xx)
    pub fn is_client_error(&self) -> bool {
        self.status_code.map_or(false, |code| code >= 400 && code < 500)
    }

    /// Check if this is a server error (5xx)
    pub fn is_server_error(&self) -> bool {
        self.status_code.map_or(false, |code| code >= 500)
    }

    /// Check if this is an authentication error
    pub fn is_auth_error(&self) -> bool {
        self.status_code == Some(401) || 
        self.error_code.as_deref() == Some("AUTH_ERROR")
    }

    /// Check if this is a permission error
    pub fn is_permission_error(&self) -> bool {
        self.status_code == Some(403) || 
        self.error_code.as_deref() == Some("PERMISSION_ERROR")
    }
}

/// Extension trait for WebConfigError to add auth error checking
impl WebConfigError {
    /// Check if this is an authentication error
    pub fn is_auth_error(&self) -> bool {
        matches!(self, WebConfigError::AuthError { .. }) ||
        (matches!(self, WebConfigError::ApiError { status_code: Some(401), .. }))
    }

    /// Check if this is a permission error  
    pub fn is_permission_error(&self) -> bool {
        matches!(self, WebConfigError::PermissionError { .. }) ||
        (matches!(self, WebConfigError::ApiError { status_code: Some(403), .. }))
    }

    /// Check if this is a client error (4xx)
    pub fn is_client_error(&self) -> bool {
        match self {
            WebConfigError::ApiError { status_code: Some(code), .. } => *code >= 400 && *code < 500,
            WebConfigError::ValidationError { .. } |
            WebConfigError::AuthError { .. } |
            WebConfigError::PermissionError { .. } |
            WebConfigError::ClientError { .. } |
            WebConfigError::ParseError { .. } => true,
            _ => false,
        }
    }

    /// Check if this is a server error (5xx)
    pub fn is_server_error(&self) -> bool {
        match self {
            WebConfigError::ApiError { status_code: Some(code), .. } => *code >= 500,
            WebConfigError::DatabaseError { .. } |
            WebConfigError::FileSystemError { .. } |
            WebConfigError::ServerControlError { .. } |
            WebConfigError::InternalError { .. } |
            WebConfigError::NetworkError { .. } => true,
            _ => false,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ApiError {}

/// Helper functions for error creation
impl WebConfigError {
    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> Self {
        WebConfigError::ValidationError {
            message: message.into(),
            field: None,
        }
    }

    /// Create a validation error for a specific field
    pub fn validation_field(field: impl Into<String>, message: impl Into<String>) -> Self {
        WebConfigError::ValidationError {
            message: message.into(),
            field: Some(field.into()),
        }
    }

    /// Create an authentication error
    pub fn auth(message: impl Into<String>) -> Self {
        WebConfigError::AuthError {
            message: message.into(),
        }
    }

    /// Create a permission error
    pub fn permission(message: impl Into<String>) -> Self {
        WebConfigError::PermissionError {
            message: message.into(),
        }
    }

    /// Create an API error
    pub fn api(message: impl Into<String>) -> Self {
        WebConfigError::ApiError {
            message: message.into(),
            status_code: None,
        }
    }

    /// Create an API error with status code
    pub fn api_with_status(message: impl Into<String>, status_code: u16) -> Self {
        WebConfigError::ApiError {
            message: message.into(),
            status_code: Some(status_code),
        }
    }

    /// Create a file system error
    pub fn filesystem(message: impl Into<String>) -> Self {
        WebConfigError::FileSystemError {
            message: message.into(),
            path: None,
        }
    }

    /// Create a file system error with path
    pub fn filesystem_with_path(message: impl Into<String>, path: impl Into<String>) -> Self {
        WebConfigError::FileSystemError {
            message: message.into(),
            path: Some(path.into()),
        }
    }

    /// Create a database error
    pub fn database(message: impl Into<String>) -> Self {
        WebConfigError::DatabaseError {
            message: message.into(),
        }
    }

    /// Create a network error
    pub fn network(message: impl Into<String>) -> Self {
        WebConfigError::NetworkError {
            message: message.into(),
        }
    }

    /// Create a server control error
    pub fn server_control(message: impl Into<String>) -> Self {
        WebConfigError::ServerControlError {
            message: message.into(),
        }
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        WebConfigError::InternalError {
            message: message.into(),
        }
    }

    /// Create a parse error
    pub fn parse(message: impl Into<String>, format: impl Into<String>) -> Self {
        WebConfigError::ParseError {
            message: message.into(),
            format: format.into(),
        }
    }

    /// Create a client error
    pub fn client(message: impl Into<String>) -> Self {
        WebConfigError::ClientError {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    /// Test WebConfigError status code mappings
    /// Validates that each error type returns the correct HTTP status code
    #[test]
    fn test_error_status_codes() {
        // Client errors (4xx)
        assert_eq!(
            WebConfigError::validation("test").status_code(),
            400,
            "ValidationError should return 400"
        );
        assert_eq!(
            WebConfigError::auth("test").status_code(),
            401,
            "AuthError should return 401"
        );
        assert_eq!(
            WebConfigError::permission("test").status_code(),
            403,
            "PermissionError should return 403"
        );
        assert_eq!(
            WebConfigError::client("test").status_code(),
            400,
            "ClientError should return 400"
        );
        
        // Parse errors are client errors
        assert_eq!(
            WebConfigError::parse("test", "JSON").status_code(),
            400,
            "ParseError should return 400"
        );
        
        // Server errors (5xx)
        assert_eq!(
            WebConfigError::api("test").status_code(),
            500,
            "ApiError without status should return 500"
        );
        assert_eq!(
            WebConfigError::api_with_status("test", 502).status_code(),
            502,
            "ApiError with status should return that status"
        );
        assert_eq!(
            WebConfigError::filesystem("test").status_code(),
            500,
            "FileSystemError should return 500"
        );
        assert_eq!(
            WebConfigError::database("test").status_code(),
            500,
            "DatabaseError should return 500"
        );
        assert_eq!(
            WebConfigError::server_control("test").status_code(),
            500,
            "ServerControlError should return 500"
        );
        assert_eq!(
            WebConfigError::internal("test").status_code(),
            500,
            "InternalError should return 500"
        );
        
        // Network errors return 503
        assert_eq!(
            WebConfigError::network("test").status_code(),
            503,
            "NetworkError should return 503"
        );
    }

    /// Test WebConfigError user-friendly messages
    #[test]
    fn test_error_user_messages() {
        // Validation error without field
        let error = WebConfigError::validation("field is required");
        assert_eq!(
            error.user_message(),
            "Validation error: field is required"
        );

        // Validation error with field
        let error = WebConfigError::validation_field("username", "must be at least 3 characters");
        assert_eq!(
            error.user_message(),
            "Validation error in field 'username': must be at least 3 characters"
        );

        // Auth error
        let error = WebConfigError::auth("invalid credentials");
        assert_eq!(
            error.user_message(),
            "Authentication required: invalid credentials"
        );

        // Permission error
        let error = WebConfigError::permission("admin access required");
        assert_eq!(
            error.user_message(),
            "Permission denied: admin access required"
        );

        // API error
        let error = WebConfigError::api("connection refused");
        assert_eq!(
            error.user_message(),
            "API error: connection refused"
        );

        // File system error without path
        let error = WebConfigError::filesystem("permission denied");
        assert_eq!(
            error.user_message(),
            "File system error: permission denied"
        );

        // File system error with path
        let error = WebConfigError::filesystem_with_path("permission denied", "/etc/palpo/config.toml");
        assert_eq!(
            error.user_message(),
            "File system error at '/etc/palpo/config.toml': permission denied"
        );

        // Database error
        let error = WebConfigError::database("connection timeout");
        assert_eq!(
            error.user_message(),
            "Database error: connection timeout"
        );

        // Parse error
        let error = WebConfigError::parse("unexpected token", "YAML");
        assert_eq!(
            error.user_message(),
            "Failed to parse YAML configuration: unexpected token"
        );

        // Network error
        let error = WebConfigError::network("connection timeout");
        assert_eq!(
            error.user_message(),
            "Network error: connection timeout"
        );

        // Server control error
        let error = WebConfigError::server_control("restart failed");
        assert_eq!(
            error.user_message(),
            "Server control error: restart failed"
        );

        // Internal error
        let error = WebConfigError::internal("unknown error occurred");
        assert_eq!(
            error.user_message(),
            "Internal error: unknown error occurred"
        );

        // Client error
        let error = WebConfigError::client("invalid input");
        assert_eq!(
            error.user_message(),
            "Client error: invalid input"
        );
    }

    /// Test WebConfigError error codes
    #[test]
    fn test_error_codes() {
        assert_eq!(
            WebConfigError::validation("test").error_code(),
            "VALIDATION_ERROR"
        );
        assert_eq!(
            WebConfigError::auth("test").error_code(),
            "AUTH_ERROR"
        );
        assert_eq!(
            WebConfigError::permission("test").error_code(),
            "PERMISSION_ERROR"
        );
        assert_eq!(
            WebConfigError::api("test").error_code(),
            "API_ERROR"
        );
        assert_eq!(
            WebConfigError::filesystem("test").error_code(),
            "FILESYSTEM_ERROR"
        );
        assert_eq!(
            WebConfigError::database("test").error_code(),
            "DATABASE_ERROR"
        );
        assert_eq!(
            WebConfigError::parse("test", "JSON").error_code(),
            "PARSE_ERROR"
        );
        assert_eq!(
            WebConfigError::network("test").error_code(),
            "NETWORK_ERROR"
        );
        assert_eq!(
            WebConfigError::server_control("test").error_code(),
            "SERVER_CONTROL_ERROR"
        );
        assert_eq!(
            WebConfigError::internal("test").error_code(),
            "INTERNAL_ERROR"
        );
        assert_eq!(
            WebConfigError::client("test").error_code(),
            "CLIENT_ERROR"
        );
    }

    /// Test ErrorResponse creation from WebConfigError
    #[test]
    fn test_error_response_from_error() {
        let error = WebConfigError::validation_field("port", "must be between 1 and 65535");
        let response = ErrorResponse::from_error(error.clone());

        assert_eq!(response.error, error.to_string());
        assert_eq!(response.message, error.user_message());
        assert_eq!(response.code, "VALIDATION_ERROR");
        assert_eq!(response.status, 400);
        assert!(response.timestamp <= chrono::Utc::now());
    }

    /// Test ErrorResponse with details
    #[test]
    fn test_error_response_with_details() {
        let error = WebConfigError::validation("multiple errors");
        let details = serde_json::json!({
            "errors": ["port is required", "host is required"]
        });
        
        let response = ErrorResponse::from_error(error).with_details(details.clone());
        
        assert_eq!(response.details, Some(details));
    }

    /// Test ApiError creation utilities
    #[test]
    fn test_api_error_creation() {
        // Basic creation
        let error = ApiError::new("Something went wrong");
        assert_eq!(error.message, "Something went wrong");
        assert!(error.status_code.is_none());
        assert!(error.error_code.is_none());
        assert!(error.details.is_none());

        // With status code
        let error = ApiError::with_status("Bad request", 400);
        assert_eq!(error.message, "Bad request");
        assert_eq!(error.status_code, Some(400));

        // With error code
        let error = ApiError::new("Error occurred").with_code("CUSTOM_ERROR");
        assert_eq!(error.message, "Error occurred");
        assert_eq!(error.error_code, Some("CUSTOM_ERROR".to_string()));

        // With details
        let error = ApiError::new("Error").with_details(serde_json::json!({"key": "value"}));
        assert!(error.details.is_some());
    }

    /// Test ApiError error type checking
    #[test]
    fn test_api_error_type_checking() {
        // Client errors (4xx)
        let error = ApiError::with_status("Bad request", 400);
        assert!(error.is_client_error());
        assert!(!error.is_server_error());

        let error = ApiError::with_status("Not found", 404);
        assert!(error.is_client_error());
        assert!(!error.is_server_error());

        // Server errors (5xx)
        let error = ApiError::with_status("Internal error", 500);
        assert!(!error.is_client_error());
        assert!(error.is_server_error());

        let error = ApiError::with_status("Bad gateway", 502);
        assert!(!error.is_client_error());
        assert!(error.is_server_error());

        // Auth error by status code
        let error = ApiError::with_status("Unauthorized", 401);
        assert!(error.is_auth_error());
        assert!(!error.is_permission_error());

        // Auth error by error code
        let error = ApiError::new("Auth failed").with_code("AUTH_ERROR");
        assert!(error.is_auth_error());

        // Permission error by status code
        let error = ApiError::with_status("Forbidden", 403);
        assert!(error.is_permission_error());
        assert!(!error.is_auth_error());

        // Permission error by error code
        let error = ApiError::new("Access denied").with_code("PERMISSION_ERROR");
        assert!(error.is_permission_error());

        // No status code - default to not client/server error
        let error = ApiError::new("Unknown error");
        assert!(!error.is_client_error());
        assert!(!error.is_server_error());
    }

    /// Test WebConfigError error type checking
    #[test]
    fn test_web_config_error_type_checking() {
        // Auth errors
        assert!(WebConfigError::auth("test").is_auth_error());
        assert!(!WebConfigError::auth("test").is_permission_error());

        // Auth error via API error with 401
        assert!(WebConfigError::api_with_status("test", 401).is_auth_error());
        assert!(!WebConfigError::api_with_status("test", 401).is_permission_error());

        // Permission errors
        assert!(WebConfigError::permission("test").is_permission_error());
        assert!(!WebConfigError::permission("test").is_auth_error());

        // Permission error via API error with 403
        assert!(WebConfigError::api_with_status("test", 403).is_permission_error());
        assert!(!WebConfigError::api_with_status("test", 403).is_auth_error());

        // Client errors (4xx)
        assert!(WebConfigError::validation("test").is_client_error());
        assert!(WebConfigError::client("test").is_client_error());
        assert!(WebConfigError::api_with_status("test", 400).is_client_error());
        assert!(WebConfigError::api_with_status("test", 404).is_client_error());

        // Server errors (5xx)
        assert!(WebConfigError::database("test").is_server_error());
        assert!(WebConfigError::filesystem("test").is_server_error());
        assert!(WebConfigError::server_control("test").is_server_error());
        assert!(WebConfigError::internal("test").is_server_error());
        assert!(WebConfigError::api_with_status("test", 500).is_server_error());
        assert!(WebConfigError::api_with_status("test", 502).is_server_error());

        // Network error is a server error
        assert!(WebConfigError::network("test").is_server_error());

        // Parse error is a client error
        assert!(WebConfigError::parse("test", "JSON").is_client_error());
    }

    /// Test WebConfigError helper functions
    #[test]
    fn test_error_helper_functions() {
        // Validation error helpers
        let error = WebConfigError::validation("test message");
        assert!(matches!(error, WebConfigError::ValidationError { message, field: None } 
            if message == "test message"));

        let error = WebConfigError::validation_field("field", "message");
        assert!(matches!(error, WebConfigError::ValidationError { message, field: Some(f) } 
            if message == "message" && f == "field"));

        // Auth error helper
        let error = WebConfigError::auth("auth message");
        assert!(matches!(error, WebConfigError::AuthError { message } 
            if message == "auth message"));

        // Permission error helper
        let error = WebConfigError::permission("permission message");
        assert!(matches!(error, WebConfigError::PermissionError { message } 
            if message == "permission message"));

        // API error helpers
        let error = WebConfigError::api("api message");
        assert!(matches!(error, WebConfigError::ApiError { message, status_code: None } 
            if message == "api message"));

        let error = WebConfigError::api_with_status("api message", 500);
        assert!(matches!(error, WebConfigError::ApiError { message, status_code: Some(500) } 
            if message == "api message"));

        // File system error helpers
        let error = WebConfigError::filesystem("fs message");
        assert!(matches!(error, WebConfigError::FileSystemError { message, path: None } 
            if message == "fs message"));

        let error = WebConfigError::filesystem_with_path("fs message", "/path/to/file");
        assert!(matches!(error, WebConfigError::FileSystemError { message, path: Some(p) } 
            if message == "fs message" && p == "/path/to/file"));

        // Database error helper
        let error = WebConfigError::database("db message");
        assert!(matches!(error, WebConfigError::DatabaseError { message } 
            if message == "db message"));

        // Network error helper
        let error = WebConfigError::network("network message");
        assert!(matches!(error, WebConfigError::NetworkError { message } 
            if message == "network message"));

        // Server control error helper
        let error = WebConfigError::server_control("control message");
        assert!(matches!(error, WebConfigError::ServerControlError { message } 
            if message == "control message"));

        // Internal error helper
        let error = WebConfigError::internal("internal message");
        assert!(matches!(error, WebConfigError::InternalError { message } 
            if message == "internal message"));

        // Client error helper
        let error = WebConfigError::client("client message");
        assert!(matches!(error, WebConfigError::ClientError { message } 
            if message == "client message"));
    }

    /// Test serde_json::Error conversion to WebConfigError
    #[test]
    fn test_serde_json_error_conversion() {
        let json_error = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let error: WebConfigError = json_error.into();
        
        assert!(matches!(error, WebConfigError::ParseError { format, .. } 
            if format == "JSON"));
    }

    /// Test WebConfigError Display trait implementation
    #[test]
    fn test_error_display() {
        let error = WebConfigError::validation_field("port", "must be positive");
        assert_eq!(
            error.to_string(),
            "Configuration validation failed: must be positive"
        );

        let error = WebConfigError::auth("invalid token");
        assert_eq!(
            error.to_string(),
            "Authentication failed: invalid token"
        );

        let error = WebConfigError::permission("admin required");
        assert_eq!(
            error.to_string(),
            "Access denied: admin required"
        );

        let error = WebConfigError::api("connection refused");
        assert_eq!(
            error.to_string(),
            "API request failed: connection refused"
        );

        let error = WebConfigError::filesystem_with_path("not found", "/etc/config");
        assert_eq!(
            error.to_string(),
            "File system error: not found"
        );

        let error = WebConfigError::database("connection timeout");
        assert_eq!(
            error.to_string(),
            "Database error: connection timeout"
        );

        let error = WebConfigError::parse("syntax error", "YAML");
        assert_eq!(
            error.to_string(),
            "Configuration parsing error: syntax error"
        );

        let error = WebConfigError::network("timeout");
        assert_eq!(
            error.to_string(),
            "Network error: timeout"
        );

        let error = WebConfigError::server_control("restart failed");
        assert_eq!(
            error.to_string(),
            "Server control error: restart failed"
        );

        let error = WebConfigError::internal("oops");
        assert_eq!(
            error.to_string(),
            "Internal server error: oops"
        );

        let error = WebConfigError::client("invalid input");
        assert_eq!(
            error.to_string(),
            "Client error: invalid input"
        );
    }

    /// Test ApiError Display trait implementation
    #[test]
    fn test_api_error_display() {
        let error = ApiError::new("test message");
        assert_eq!(error.to_string(), "test message");

        let error = ApiError::with_status("error", 500);
        assert_eq!(error.to_string(), "error");

        let error = ApiError::new("error").with_code("ERR_CODE");
        assert_eq!(error.to_string(), "error");
    }

    /// Test ErrorResponse serialization
    #[test]
    fn test_error_response_serialization() {
        let error = WebConfigError::validation_field("email", "invalid format");
        let response = ErrorResponse::from_error(error);
        
        let serialized = serde_json::to_string(&response).expect("Failed to serialize error response");
        let deserialized: ErrorResponse = serde_json::from_str(&serialized).expect("Failed to deserialize error response");
        
        assert_eq!(response.error, deserialized.error);
        assert_eq!(response.message, deserialized.message);
        assert_eq!(response.code, deserialized.code);
        assert_eq!(response.status, deserialized.status);
    }

    /// Test ApiError serialization for frontend
    #[test]
    fn test_api_error_serialization() {
        let error = ApiError::with_status("Something went wrong", 500)
            .with_code("INTERNAL_ERROR")
            .with_details(serde_json::json!({"trace": "abc123"}));
        
        let serialized = serde_json::to_string(&error).expect("Failed to serialize API error");
        let deserialized: ApiError = serde_json::from_str(&serialized).expect("Failed to deserialize API error");
        
        assert_eq!(error.message, deserialized.message);
        assert_eq!(error.status_code, deserialized.status_code);
        assert_eq!(error.error_code, deserialized.error_code);
        assert_eq!(error.details, deserialized.details);
    }

    /// Test WebConfigError serialization for API responses
    #[test]
    fn test_web_config_error_serialization() {
        let error = WebConfigError::validation_field("port", "must be between 1-65535");
        
        let serialized = serde_json::to_string(&error).expect("Failed to serialize WebConfigError");
        let deserialized: WebConfigError = serde_json::from_str(&serialized).expect("Failed to deserialize WebConfigError");
        
        assert_eq!(error.to_string(), deserialized.to_string());
        assert_eq!(error.status_code(), deserialized.status_code());
        assert_eq!(error.user_message(), deserialized.user_message());
    }

    /// Test WebConfigError clone and debug
    #[test]
    fn test_error_clone_and_debug() {
        let error = WebConfigError::validation_field("field", "message");
        let cloned = error.clone();
        
        assert_eq!(format!("{:?}", error), format!("{:?}", cloned));
        assert_eq!(error.user_message(), cloned.user_message());
    }

    /// Test ApiError clone and debug
    #[test]
    fn test_api_error_clone_and_debug() {
        let error = ApiError::with_status("message", 400)
            .with_code("ERR")
            .with_details(serde_json::json!({"key": "value"}));
        let cloned = error.clone();
        
        assert_eq!(format!("{:?}", error), format!("{:?}", cloned));
        assert_eq!(error.message, cloned.message);
        assert_eq!(error.status_code, cloned.status_code);
    }

    /// Test WebConfigResult type alias
    #[test]
    fn test_web_config_result() {
        // Success case
        let result: WebConfigResult<String> = Ok("success".to_string());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");

        // Error case
        let result: WebConfigResult<String> = Err(WebConfigError::validation("failed"));
        assert!(result.is_err());
        assert!(matches!(result, Err(WebConfigError::ValidationError { .. })));
    }

    /// Test error with various status codes in ApiError
    #[test]
    fn test_api_error_various_status_codes() {
        // Test all common status codes
        let test_cases = [
            (400, true, false),  // Bad Request - client error
            (401, true, false),  // Unauthorized - client error (4xx)
            (403, true, false),  // Forbidden - client error (4xx)
            (404, true, false),  // Not Found - client error
            (500, false, true),  // Internal Server Error - server error
            (502, false, true),  // Bad Gateway - server error
            (503, false, true),  // Service Unavailable - server error
        ];

        for (status, expected_client, expected_server) in test_cases {
            let error = ApiError::with_status("test", status);
            assert_eq!(error.is_client_error(), expected_client, 
                "Status {} should be client error: {}", status, expected_client);
            assert_eq!(error.is_server_error(), expected_server, 
                "Status {} should be server error: {}", status, expected_server);
        }
    }

    /// Test error with no status code
    #[test]
    fn test_api_error_no_status_code() {
        let error = ApiError::new("error without status");
        assert!(!error.is_client_error());
        assert!(!error.is_server_error());
        assert!(!error.is_auth_error());
        assert!(!error.is_permission_error());
    }

    /// Test WebConfigError with various API error status codes
    #[test]
    fn test_web_config_error_api_status_codes() {
        // API error with 401 should be auth error
        let error = WebConfigError::api_with_status("unauthorized", 401);
        assert!(error.is_auth_error());
        assert!(!error.is_permission_error());

        // API error with 403 should be permission error
        let error = WebConfigError::api_with_status("forbidden", 403);
        assert!(!error.is_auth_error());
        assert!(error.is_permission_error());

        // API error with 404 should be client error
        let error = WebConfigError::api_with_status("not found", 404);
        assert!(error.is_client_error());
        assert!(!error.is_server_error());

        // API error with 500 should be server error
        let error = WebConfigError::api_with_status("internal error", 500);
        assert!(!error.is_client_error());
        assert!(error.is_server_error());
    }
}