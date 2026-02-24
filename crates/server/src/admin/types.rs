/// Core types for the two-tier admin system
///
/// This module defines the fundamental types used across the admin system:
/// - Tier 1: Web UI Admin (PostgreSQL-backed, independent of Palpo)
/// - Tier 2: Matrix Admin (Palpo-dependent, stored in Matrix users table)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Web UI Admin credentials stored in PostgreSQL database
///
/// This represents the first tier of the admin system. The Web UI admin
/// uses a fixed username "admin" and authenticates against PostgreSQL,
/// allowing access to the admin interface even when Palpo server is not running.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebUIAdminCredentials {
    /// Fixed username, always "admin"
    pub username: String,
    /// Argon2 or bcrypt hash of the password
    pub password_hash: String,
    /// Unique salt used for password hashing
    pub salt: String,
    /// Timestamp when the admin account was first created
    pub created_at: DateTime<Utc>,
    /// Timestamp of the last password change
    pub updated_at: DateTime<Utc>,
}

/// Session token returned after successful authentication
///
/// Used for subsequent API requests to maintain authenticated state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionToken {
    /// Cryptographically random session token
    pub token: String,
    /// Expiration timestamp for the session
    pub expires_at: DateTime<Utc>,
}

/// Palpo server configuration
///
/// Manages the configuration for the Palpo Matrix server instance.
/// Web UI admins can configure these settings before starting the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// PostgreSQL database connection URL
    pub database_url: String,
    /// Matrix server name (domain)
    pub server_name: String,
    /// IP address to bind the server to
    pub bind_address: String,
    /// Port number for the server
    pub port: u16,
    /// Optional path to TLS certificate file
    pub tls_certificate: Option<String>,
    /// Optional path to TLS private key file
    pub tls_private_key: Option<String>,
}

/// Palpo server status
///
/// Represents the current operational state of the Palpo server
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerStatus {
    /// Server has not been started yet
    NotStarted,
    /// Server is in the process of starting up
    Starting,
    /// Server is running and operational
    Running,
    /// Server is in the process of shutting down
    Stopping,
    /// Server has been stopped
    Stopped,
    /// Server encountered an error
    Error,
}

/// Response after creating a Matrix admin user
///
/// Contains the credentials for the newly created Matrix admin.
/// This is the second tier of the admin system, requiring Palpo to be running.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMatrixAdminResponse {
    /// Full Matrix user ID (@username:homeserver)
    pub user_id: String,
    /// Username portion of the Matrix ID
    pub username: String,
    /// Initial password (should be changed on first login)
    pub password: String,
}

/// Comprehensive error type for the admin system
///
/// Covers all error scenarios across both admin tiers, server control,
/// configuration management, and password policy enforcement.
#[derive(Debug, thiserror::Error)]
pub enum AdminError {
    // ===== Web UI Admin Errors =====
    /// Web UI admin account already exists in the database
    #[error("Web UI admin already exists")]
    WebUIAdminAlreadyExists,

    /// Web UI admin account not found in the database
    #[error("Web UI admin not found")]
    WebUIAdminNotFound,

    /// Provided session token is invalid
    #[error("Invalid session token")]
    InvalidSessionToken,

    /// Session token has expired
    #[error("Session expired")]
    SessionExpired,

    // ===== Database Errors =====
    /// Failed to establish connection to PostgreSQL database
    #[error("Database connection failed: {0}")]
    DatabaseConnectionFailed(String),

    /// Database query execution failed
    #[error("Database query failed: {0}")]
    DatabaseQueryFailed(String),

    /// Database is not available
    #[error("Database unavailable")]
    DatabaseUnavailable,

    /// Database migration failed to apply
    #[error("Database migration failed: {0}")]
    DatabaseMigrationFailed(String),

    // ===== Migration Errors =====
    /// Legacy credentials not found in localStorage
    #[error("Legacy credentials not found")]
    LegacyCredentialsNotFound,

    /// Migration from localStorage to database failed
    #[error("Migration failed: {0}")]
    MigrationFailed(String),

    /// Browser storage (localStorage) is unavailable
    #[error("Storage unavailable")]
    StorageUnavailable,

    // ===== Server Control Errors =====
    /// Palpo server is not currently running
    #[error("Server not running")]
    ServerNotRunning,

    /// Palpo server is already running
    #[error("Server already running")]
    ServerAlreadyRunning,

    /// Failed to start the Palpo server
    #[error("Failed to start server: {0}")]
    ServerStartFailed(String),

    /// Failed to stop the Palpo server
    #[error("Failed to stop server: {0}")]
    ServerStopFailed(String),

    // ===== Configuration Errors =====
    /// Database URL format is invalid
    #[error("Invalid database URL")]
    InvalidDatabaseUrl,

    /// Server name is invalid or empty
    #[error("Invalid server name")]
    InvalidServerName,

    /// Port number is invalid (0 or out of range)
    #[error("Invalid port")]
    InvalidPort,

    /// TLS certificate file not found at specified path
    #[error("TLS certificate not found")]
    TLSCertificateNotFound,

    /// TLS private key file not found at specified path
    #[error("TLS private key not found")]
    TLSPrivateKeyNotFound,

    /// Configuration validation failed
    #[error("Configuration validation failed: {0}")]
    ConfigValidationFailed(String),

    // ===== Matrix Admin Errors =====
    /// Matrix admin user already exists in the users table
    #[error("Matrix admin already exists")]
    MatrixAdminAlreadyExists,

    /// Matrix admin user not found in the users table
    #[error("Matrix admin not found")]
    MatrixAdminNotFound,

    /// Invalid username or password provided
    #[error("Invalid credentials")]
    InvalidCredentials,

    /// User is not an admin (admin field != 1)
    #[error("Not an admin user")]
    NotAnAdmin,

    // ===== Password Policy Errors =====
    /// Password does not meet minimum length requirement
    #[error("Password too short: {0} characters (minimum 12)")]
    PasswordTooShort(usize),

    /// Password must contain at least one uppercase letter
    #[error("Password must contain uppercase letter")]
    MissingUppercase,

    /// Password must contain at least one lowercase letter
    #[error("Password must contain lowercase letter")]
    MissingLowercase,

    /// Password must contain at least one digit
    #[error("Password must contain digit")]
    MissingDigit,

    /// Password must contain at least one special character
    #[error("Password must contain special character")]
    MissingSpecialChar,

    /// New password is the same as the current password
    #[error("New password must be different from current password")]
    PasswordNotChanged,

    /// User must change password before accessing the system
    #[error("Password change required before accessing system")]
    PasswordChangeRequired,

    // ===== Security Errors =====
    /// Too many failed login attempts
    #[error("Too many login attempts. Please try again later.")]
    RateLimitExceeded,

    /// CSRF token validation failed
    #[error("CSRF token validation failed")]
    CSRFValidationFailed,

    // ===== General Errors =====
    /// Credentials not found in secure storage
    #[error("Credentials not found in storage")]
    CredentialsNotFound,

    /// Failed to write to audit log
    #[error("Audit log write failed: {0}")]
    AuditLogFailed(String),

    /// Matrix Admin API returned an error
    #[error("Matrix Admin API error: {0}")]
    MatrixApiError(String),

    /// Admin status field was not set correctly after user creation
    #[error("Admin status not set correctly")]
    AdminStatusNotSet,

    /// Password hashing operation failed
    #[error("Password hashing failed: {0}")]
    PasswordHashError(String),

    /// I/O operation failed
    #[error("I/O error: {0}")]
    IoError(String),

    /// TOML serialization/deserialization failed
    #[error("TOML error: {0}")]
    TomlError(String),

    /// HTTP request failed
    #[error("HTTP request failed: {0}")]
    HttpError(String),
}

// Implement From conversions for common error types
impl From<sqlx::Error> for AdminError {
    fn from(err: sqlx::Error) -> Self {
        AdminError::DatabaseQueryFailed(err.to_string())
    }
}

impl From<std::io::Error> for AdminError {
    fn from(err: std::io::Error) -> Self {
        AdminError::IoError(err.to_string())
    }
}

impl From<toml::ser::Error> for AdminError {
    fn from(err: toml::ser::Error) -> Self {
        AdminError::TomlError(err.to_string())
    }
}

impl From<toml::de::Error> for AdminError {
    fn from(err: toml::de::Error) -> Self {
        AdminError::TomlError(err.to_string())
    }
}

impl From<reqwest::Error> for AdminError {
    fn from(err: reqwest::Error) -> Self {
        AdminError::HttpError(err.to_string())
    }
}
