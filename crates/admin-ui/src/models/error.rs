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
    pub fn with_code(message: impl Into<String>, error_code: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: None,
            error_code: Some(error_code.into()),
            details: None,
        }
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
            WebConfigError::ClientError { .. } => true,
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
            WebConfigError::InternalError { .. } => true,
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

    /// Create a client error
    pub fn client(message: impl Into<String>) -> Self {
        WebConfigError::ClientError {
            message: message.into(),
        }
    }
}