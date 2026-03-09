//! Authentication and authorization models

use serde::{Deserialize, Serialize};

/// Admin user information
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AdminUser {
    pub user_id: String, // Using String instead of OwnedUserId for simplicity
    pub username: String,
    pub is_admin: bool,
    pub session_id: String,
    pub expires_at: String, // RFC3339 timestamp string
    pub permissions: Vec<Permission>,
}

impl PartialEq for AdminUser {
    fn eq(&self, other: &Self) -> bool {
        self.user_id == other.user_id
            && self.username == other.username
            && self.is_admin == other.is_admin
            && self.session_id == other.session_id
            && self.permissions == other.permissions
            // Note: We don't compare expires_at as SystemTime doesn't implement PartialEq
    }
}

/// User permissions for different admin functions
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Permission {
    /// Full configuration management access
    ConfigManagement,
    /// User management access
    UserManagement,
    /// Room management access
    RoomManagement,
    /// Federation management access
    FederationManagement,
    /// Media management access
    MediaManagement,
    /// Appservice management access
    AppserviceManagement,
    /// Server control access (restart, reload)
    ServerControl,
    /// Audit log access
    AuditLogAccess,
    /// System administration (all permissions)
    SystemAdmin,
}

/// Authentication state for the frontend
#[derive(Clone, Debug, PartialEq)]
pub enum AuthState {
    /// Not authenticated
    Unauthenticated,
    /// Authentication in progress
    Authenticating,
    /// Successfully authenticated
    Authenticated(AdminUser),
    /// Authentication failed
    Failed(String),
}

/// Login request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Login response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoginResponse {
    pub success: bool,
    pub token: Option<String>,
    pub user: Option<AdminUser>,
    pub error: Option<String>,
}

/// JWT token claims
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TokenClaims {
    pub sub: String, // user_id
    pub username: String,
    pub is_admin: bool,
    pub permissions: Vec<Permission>,
    pub session_id: String,
    pub exp: u64, // expiration timestamp
    pub iat: u64, // issued at timestamp
}

/// Session validation request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValidateSessionRequest {
    pub token: String,
}

/// Session validation response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValidateSessionResponse {
    pub valid: bool,
    pub user: Option<AdminUser>,
    pub error: Option<String>,
}

/// Logout request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LogoutRequest {
    pub session_id: String,
}

impl AdminUser {
    /// Check if user has a specific permission
    pub fn has_permission(&self, permission: &Permission) -> bool {
        // System admin has all permissions
        if self.permissions.contains(&Permission::SystemAdmin) {
            return true;
        }
        
        self.permissions.contains(permission)
    }

    /// Check if user has any of the specified permissions
    pub fn has_any_permission(&self, permissions: &[Permission]) -> bool {
        // System admin has all permissions
        if self.permissions.contains(&Permission::SystemAdmin) {
            return true;
        }

        permissions.iter().any(|p| self.permissions.contains(p))
    }

    /// Check if the session is still valid (not expired)
    pub fn is_session_valid(&self) -> bool {
        // Parse RFC3339 timestamp
        if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&self.expires_at) {
            chrono::Utc::now() < expires
        } else {
            false
        }
    }

    /// Get remaining session time in seconds
    pub fn remaining_session_time(&self) -> Option<u64> {
        if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&self.expires_at) {
            let now = chrono::Utc::now();
            let expires_utc = expires.with_timezone(&chrono::Utc);
            if expires_utc > now {
                Some((expires_utc.timestamp() - now.timestamp()) as u64)
            } else {
                None
            }
        } else {
            None
        }
    }
}

impl AuthState {
    /// Check if the user is authenticated
    pub fn is_authenticated(&self) -> bool {
        matches!(self, AuthState::Authenticated(_))
    }

    /// Get the authenticated user if available
    pub fn user(&self) -> Option<&AdminUser> {
        match self {
            AuthState::Authenticated(user) => Some(user),
            _ => None,
        }
    }

    /// Check if authentication is in progress
    pub fn is_authenticating(&self) -> bool {
        matches!(self, AuthState::Authenticating)
    }

    /// Check if authentication failed
    pub fn is_failed(&self) -> bool {
        matches!(self, AuthState::Failed(_))
    }

    /// Get the error message if authentication failed
    pub fn error(&self) -> Option<&String> {
        match self {
            AuthState::Failed(error) => Some(error),
            _ => None,
        }
    }
}

impl Default for AuthState {
    fn default() -> Self {
        AuthState::Unauthenticated
    }
}

impl Permission {
    /// Get human-readable description of the permission
    pub fn description(&self) -> &'static str {
        match self {
            Permission::ConfigManagement => "Configuration Management",
            Permission::UserManagement => "User Management",
            Permission::RoomManagement => "Room Management",
            Permission::FederationManagement => "Federation Management",
            Permission::MediaManagement => "Media Management",
            Permission::AppserviceManagement => "Appservice Management",
            Permission::ServerControl => "Server Control",
            Permission::AuditLogAccess => "Audit Log Access",
            Permission::SystemAdmin => "System Administrator",
        }
    }

    /// Get all available permissions
    pub fn all() -> Vec<Permission> {
        vec![
            Permission::ConfigManagement,
            Permission::UserManagement,
            Permission::RoomManagement,
            Permission::FederationManagement,
            Permission::MediaManagement,
            Permission::AppserviceManagement,
            Permission::ServerControl,
            Permission::AuditLogAccess,
            Permission::SystemAdmin,
        ]
    }
}

/// Authentication middleware configuration
#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// JWT secret key
    pub jwt_secret: String,
    /// Token expiration time in seconds
    pub token_expiry: u64,
    /// Session timeout in seconds
    pub session_timeout: u64,
    /// Whether to require HTTPS for authentication
    pub require_https: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            jwt_secret: "change-me-in-production".to_string(),
            token_expiry: 3600, // 1 hour
            session_timeout: 7200, // 2 hours
            require_https: true,
        }
    }
}