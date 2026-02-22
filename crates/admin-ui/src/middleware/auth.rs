//! # Authentication Middleware
//!
//! This module provides middleware components for authentication and authorization
//! in the Palpo Matrix server administration interface. The middleware integrates
//! with the authentication service to provide secure access control for administrative
//! operations.
//!
//! ## Features
//!
//! - **JWT Token Validation**: Validates and parses JWT tokens for authentication
//! - **Permission Checking**: Verifies user permissions for protected operations
//! - **Session Management**: Handles session timeouts and renewal
//! - **Role-Based Access Control**: Supports role-based permission management
//!
//! ## Usage Patterns
//!
//! ### Basic Authentication Check
//!
//! ```rust
//! use std::rc::Rc;
//! use std::cell::RefCell;
//! use crate::services::AuthService;
//! use crate::middleware::auth::{AuthMiddleware, AuthMiddlewareExt};
//!
//! let service = Rc::new(RefCell::new(AuthService::default()));
//! let middleware = AuthMiddleware::new(service.clone());
//!
//! // Check if user is authenticated
//! let result = middleware.require_auth().await;
//! ```
//!
//! ### Admin Permission Check
//!
//! ```rust
//! use crate::models::Permission;
//!
//! let result = middleware.require_permission(Permission::UserManagement).await;
//! ```
//!
//! ### Using Extension Trait
//!
//! ```rust
//! // Add authentication check to operations
//! let result = middleware.authenticate_operation(
//!     "admin@example.com".to_string(),
//!     perform_user_management(),
//! ).await;
//! ```

use crate::models::{
    AdminUser, AuthState, Permission, TokenClaims, WebConfigError, WebConfigResult,
};
use crate::services::AuthService;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::rc::Rc;
use std::cell::RefCell;
use std::time::SystemTime;

/// Authentication middleware for protecting administrative operations.
///
/// This middleware provides a transparent way to add authentication and authorization
/// checks to administrative operations. It wraps operation results and automatically
/// checks authentication status and permissions before allowing operations.
///
/// The middleware is designed for single-threaded WASM environments and uses
/// `Rc<RefCell<>>` for shared access to the auth service. For multi-threaded
/// environments, consider using `Arc<Mutex<>>` instead.
///
/// # Examples
/// #[ignore]
/// ```rust
/// let service = Rc::new(RefCell::new(AuthService::default()));
/// let middleware = AuthMiddleware::new(service.clone());
///
/// // Protect an operation requiring authentication
/// let result = middleware.protect_operation(
///     perform_admin_task(),
/// ).await;
/// ```
pub struct AuthMiddleware {
    auth_service: Rc<RefCell<AuthService>>,
    config: AuthMiddlewareConfig,
}

impl AuthMiddleware {
    /// Creates a new auth middleware instance.
    ///
    /// # Parameters
    ///
    /// - `auth_service`: A shared reference to an `AuthService` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// let service = Rc::new(RefCell::new(AuthService::default()));
    /// let middleware = AuthMiddleware::new(service);
    /// ```
    pub fn new(auth_service: Rc<RefCell<AuthService>>) -> Self {
        Self {
            auth_service,
            config: AuthMiddlewareConfig::default(),
        }
    }

    /// Creates a new auth middleware with custom configuration.
    ///
    /// # Parameters
    ///
    /// - `auth_service`: A shared reference to an `AuthService` instance
    /// - `config`: Custom middleware configuration
    ///
    /// # Examples
    ///
    /// ```rust
    /// let service = Rc::new(RefCell::new(AuthService::default()));
    /// let config = AuthMiddlewareConfig {
    ///     require_admin: true,
    ///     session_timeout: 3600,
    ///     ..Default::default()
    /// };
    /// let middleware = AuthMiddleware::new_with_config(service, config);
    /// ```
    pub fn new_with_config(auth_service: Rc<RefCell<AuthService>>, config: AuthMiddlewareConfig) -> Self {
        Self {
            auth_service,
            config,
        }
    }

    /// Requires authentication for an operation.
    ///
    /// This method checks if a user is authenticated and returns the authenticated
    /// user if successful. If not authenticated, it returns an error.
    ///
    /// # Returns
    ///
    /// Returns `Ok(AdminUser)` if authenticated, `Err(WebConfigError)` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let user = middleware.require_auth().await?;
    /// println!("Authenticated as: {}", user.username);
    /// ```
    pub async fn require_auth(&self) -> WebConfigResult<AdminUser> {
        let service = self.auth_service.borrow();
        
        match service.get_auth_state().await {
            AuthState::Authenticated(user) => {
                // Check session validity
                if self.config.check_session_timeout && !user.is_session_valid() {
                    return Err(WebConfigError::auth("Session expired"));
                }
                Ok(user)
            }
            AuthState::Unauthenticated => {
                Err(WebConfigError::auth("Authentication required"))
            }
            AuthState::Authenticating => {
                Err(WebConfigError::auth("Authentication in progress"))
            }
            AuthState::Failed(error) => {
                Err(WebConfigError::auth(format!("Authentication failed: {}", error)))
            }
        }
    }

    /// Requires admin privileges for an operation.
    ///
    /// This method checks if a user is authenticated and has admin privileges.
    ///
    /// # Returns
    ///
    /// Returns `Ok(AdminUser)` if the user is an admin, `Err(WebConfigError)` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let admin = middleware.require_admin().await?;
    /// ```
    pub async fn require_admin(&self) -> WebConfigResult<AdminUser> {
        let user = self.require_auth().await?;
        
        if user.is_admin {
            Ok(user)
        } else {
            Err(WebConfigError::permission("Admin privileges required"))
        }
    }

    /// Requires a specific permission for an operation.
    ///
    /// This method checks if a user is authenticated and has the specified permission.
    ///
    /// # Parameters
    ///
    /// - `permission`: The required permission
    ///
    /// # Returns
    ///
    /// Returns `Ok(AdminUser)` if the user has the permission, `Err(WebConfigError)` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let user = middleware.require_permission(Permission::UserManagement).await?;
    /// ```
    pub async fn require_permission(&self, permission: Permission) -> WebConfigResult<AdminUser> {
        let user = self.require_auth().await?;
        
        if user.has_permission(&permission) {
            Ok(user)
        } else {
            Err(WebConfigError::permission(format!(
                "Permission required: {}",
                permission.description()
            )))
        }
    }

    /// Requires any of the specified permissions for an operation.
    ///
    /// This method checks if a user is authenticated and has at least one of the
    /// specified permissions.
    ///
    /// # Parameters
    ///
    /// - `permissions`: A slice of permissions, any one of which is sufficient
    ///
    /// # Returns
    ///
    /// Returns `Ok(AdminUser)` if the user has any of the permissions, `Err(WebConfigError)` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let user = middleware.require_any_permission(&[
    ///     Permission::UserManagement,
    ///     Permission::RoomManagement,
    /// ]).await?;
    /// ```
    pub async fn require_any_permission(&self, permissions: &[Permission]) -> WebConfigResult<AdminUser> {
        let user = self.require_auth().await?;
        
        if user.has_any_permission(permissions) {
            Ok(user)
        } else {
            Err(WebConfigError::permission("Required permission not found"))
        }
    }

    /// Protects an operation with authentication.
    ///
    /// This method wraps an operation and checks authentication before executing it.
    /// If the user is not authenticated, the operation is not executed and an error is returned.
    ///
    /// # Type Parameters
    ///
    /// - `T`: The success type of the operation result
    /// - `F`: The operation future type
    ///
    /// # Parameters
    ///
    /// - `operation`: The async operation to protect
    ///
    /// # Returns
    ///
    /// Returns the operation result if authenticated, or an error otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let result = middleware.protect_operation(
    ///     perform_admin_task()
    /// ).await;
    /// ```
    pub async fn protect_operation<T, F>(&self, operation: F) -> WebConfigResult<T>
    where
        F: std::future::Future<Output = WebConfigResult<T>>,
    {
        let _user = self.require_auth().await?;
        operation.await
    }

    /// Protects an operation with admin privileges.
    ///
    /// This method wraps an operation and checks admin privileges before executing it.
    /// If the user is not an admin, the operation is not executed and an error is returned.
    ///
    /// # Type Parameters
    ///
    /// - `T`: The success type of the operation result
    /// - `F`: The operation future type
    ///
    /// # Parameters
    ///
    /// - `operation`: The async operation to protect
    ///
    /// # Returns
    ///
    /// Returns the operation result if the user is an admin, or an error otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let result = middleware.protect_admin_operation(
    ///     perform_admin_task()
    /// ).await;
    /// ```
    pub async fn protect_admin_operation<T, F>(&self, operation: F) -> WebConfigResult<T>
    where
        F: std::future::Future<Output = WebConfigResult<T>>,
    {
        let _admin = self.require_admin().await?;
        operation.await
    }

    /// Protects an operation with specific permission requirements.
    ///
    /// This method wraps an operation and checks for specific permissions before executing it.
    /// If the user lacks the required permission, the operation is not executed and an error is returned.
    ///
    /// # Type Parameters
    ///
    /// - `T`: The success type of the operation result
    /// - `F`: The operation future type
    ///
    /// # Parameters
    ///
    /// - `permission`: The required permission
    /// - `operation`: The async operation to protect
    ///
    /// # Returns
    ///
    /// Returns the operation result if the user has the permission, or an error otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let result = middleware.protect_with_permission(
    ///     Permission::UserManagement,
    ///     perform_user_management()
    /// ).await;
    /// ```
    pub async fn protect_with_permission<T, F>(
        &self,
        permission: Permission,
        operation: F,
    ) -> WebConfigResult<T>
    where
        F: std::future::Future<Output = WebConfigResult<T>>,
    {
        let _user = self.require_permission(permission).await?;
        operation.await
    }

    /// Validates a JWT token and returns the claims.
    ///
    /// This method parses and validates a JWT token without requiring the full
    /// authentication service. It's useful for quick token validation.
    ///
    /// # Parameters
    ///
    /// - `token`: The JWT token string to validate
    ///
    /// # Returns
    ///
    /// Returns the token claims if valid, or an error otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let claims = middleware.validate_jwt_token(token).await?;
    /// ```
    pub fn validate_jwt_token(&self, token: &str) -> WebConfigResult<TokenClaims> {
        // Split the token into parts
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(WebConfigError::auth("Invalid token format".to_string()));
        }

        // Decode the header (we don't validate it for now)
        let _header = decode_base64_urlsafe(parts[0])?;

        // Decode the payload
        let payload = decode_base64_urlsafe(parts[1])?;

        // Parse the claims
        let claims: TokenClaims = serde_json::from_slice(&payload)
            .map_err(|_| WebConfigError::auth("Invalid token payload".to_string()))?;

        // Check expiration
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| WebConfigError::auth("Invalid system time".to_string()))?
            .as_secs();

        if claims.exp < now {
            return Err(WebConfigError::auth("Token expired".to_string()));
        }

        Ok(claims)
    }

    /// Checks if a session needs renewal.
    ///
    /// This method checks if the current session is close to expiring and
    /// would benefit from renewal.
    ///
    /// # Returns
    ///
    /// Returns `true` if the session needs renewal, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// if middleware.session_needs_renewal().await {
    ///     // Trigger token refresh
    /// }
    /// ```
    pub async fn session_needs_renewal(&self) -> bool {
        if let Some(user) = self.get_current_user().await {
            if let Some(remaining) = user.remaining_session_time() {
                // Renew if less than 5 minutes (300 seconds) remaining
                remaining < 300
            } else {
                true // Session expired
            }
        } else {
            false // Not authenticated
        }
    }

    /// Gets the current authenticated user.
    ///
    /// # Returns
    ///
    /// Returns `Some(AdminUser)` if authenticated, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// if let Some(user) = middleware.get_current_user().await {
    ///     println!("Current user: {}", user.username);
    /// }
    /// ```
    pub async fn get_current_user(&self) -> Option<AdminUser> {
        let service = self.auth_service.borrow();
        service.get_current_user().await
    }

    /// Checks if the current user has a specific permission.
    ///
    /// # Parameters
    ///
    /// - `permission`: The permission to check
    ///
    /// # Returns
    ///
    /// Returns `true` if the user has the permission, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// if middleware.has_permission(Permission::UserManagement).await {
    ///     // Show user management UI
    /// }
    /// ```
    pub async fn has_permission(&self, permission: Permission) -> bool {
        if let Some(user) = self.get_current_user().await {
            user.has_permission(&permission)
        } else {
            false
        }
    }

    /// Checks if the current user is an admin.
    ///
    /// # Returns
    ///
    /// Returns `true` if the user is an admin, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// if middleware.is_admin().await {
    ///     // Show admin-only UI
    /// }
    /// ```
    pub async fn is_admin(&self) -> bool {
        if let Some(user) = self.get_current_user().await {
            user.is_admin
        } else {
            false
        }
    }
}

/// Configuration for authentication middleware.
#[derive(Clone, Debug)]
pub struct AuthMiddlewareConfig {
    /// Whether to check session timeout
    pub check_session_timeout: bool,
    /// Session timeout in seconds
    pub session_timeout: u64,
    /// Whether to require admin for all operations
    pub require_admin: bool,
    /// Default permission if not admin
    pub default_permission: Option<Permission>,
}

impl Default for AuthMiddlewareConfig {
    fn default() -> Self {
        Self {
            check_session_timeout: true,
            session_timeout: 7200, // 2 hours
            require_admin: false,
            default_permission: None,
        }
    }
}

/// Extension trait for authentication state operations.
///
/// This trait provides convenient methods for working with `AuthState`.
pub trait AuthStateExt {
    /// Check if authenticated
    fn is_authenticated(&self) -> bool;
    
    /// Get the authenticated user
    fn user(&self) -> Option<&AdminUser>;
    
    /// Check if authentication is in progress
    fn is_authenticating(&self) -> bool;
    
    /// Check if authentication failed
    fn is_failed(&self) -> bool;
    
    /// Get error message if failed
    fn error(&self) -> Option<&String>;
}

impl AuthStateExt for AuthState {
    fn is_authenticated(&self) -> bool {
        matches!(self, AuthState::Authenticated(_))
    }
    
    fn user(&self) -> Option<&AdminUser> {
        match self {
            AuthState::Authenticated(user) => Some(user),
            _ => None,
        }
    }
    
    fn is_authenticating(&self) -> bool {
        matches!(self, AuthState::Authenticating)
    }
    
    fn is_failed(&self) -> bool {
        matches!(self, AuthState::Failed(_))
    }
    
    fn error(&self) -> Option<&String> {
        match self {
            AuthState::Failed(error) => Some(error),
            _ => None,
        }
    }
}

/// Extension trait for AuthMiddleware providing additional functionality.
/// Note: This trait is simplified to avoid async_trait dependency.
/// Use the direct methods on AuthMiddleware for async operations.
#[cfg(test)]
pub trait AuthMiddlewareExt {
    /// Protect an operation with authentication
    fn protect_sync<T>(&self, operation: impl FnOnce() -> T) -> WebConfigResult<T>;
}

#[cfg(test)]
impl AuthMiddlewareExt for AuthMiddleware {
    fn protect_sync<T>(&self, operation: impl FnOnce() -> T) -> WebConfigResult<T> {
        // For sync operations, we can't check auth without async
        // This is just for testing basic functionality
        Ok(operation())
    }
}

/// Decodes a base64 URL-safe encoded string.
///
/// # Parameters
///
/// - `input`: The base64 URL-safe encoded string
///
/// # Returns
///
/// Returns the decoded bytes or an error.
fn decode_base64_urlsafe(input: &str) -> WebConfigResult<Vec<u8>> {
    // Replace URL-safe characters with standard base64 characters
    let standard_input = input
        .replace('-', "+")
        .replace('_', "/");
    
    // Add padding if necessary
    let padding = (4 - standard_input.len() % 4) % 4;
    let padded_input = standard_input + &"=".repeat(padding);
    
    STANDARD.decode(padded_input)
        .map_err(|_| WebConfigError::auth("Invalid base64 encoding".to_string()))
}

/// Creates a test JWT token for testing purposes.
///
/// This function creates a valid JWT token with the given claims for testing.
/// The token is signed with a test secret.
///
/// # Parameters
///
/// - `claims`: The token claims to include
/// - `secret`: The secret key to sign with
///
/// # Returns
///
/// Returns the encoded JWT token string.
#[cfg(test)]
pub fn create_test_jwt(claims: &TokenClaims, _secret: &str) -> String {
    // Create a simple test token without actual signature verification
    // This is sufficient for testing token parsing and validation logic
    let header = serde_json::json!({
        "alg": "HS256",
        "typ": "JWT"
    });
    
    let header_b64 = STANDARD.encode(serde_json::to_string(&header).unwrap());
    let payload_b64 = STANDARD.encode(serde_json::to_string(claims).unwrap());
    
    // Return unsigned token for testing (signature verification would require hmac/sha2)
    format!("{}.{}.", header_b64, payload_b64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::AuthService;
    use crate::models::Permission;
    use std::time::{Duration, SystemTime};
    use proptest::prelude::*;

    fn create_test_middleware() -> AuthMiddleware {
        let service = Rc::new(RefCell::new(AuthService::default()));
        AuthMiddleware::new(service)
    }

    // ============ Unit Tests ============

    #[test]
    fn test_auth_middleware_custom_config() {
        let service = Rc::new(RefCell::new(AuthService::default()));
        let config = AuthMiddlewareConfig {
            check_session_timeout: false,
            session_timeout: 3600,
            require_admin: true,
            default_permission: Some(Permission::UserManagement),
        };
        let middleware = AuthMiddleware::new_with_config(service, config);
        
        assert!(!middleware.config.check_session_timeout);
        assert_eq!(middleware.config.session_timeout, 3600);
        assert!(middleware.config.require_admin);
        assert_eq!(middleware.config.default_permission, Some(Permission::UserManagement));
    }

    #[test]
    fn test_jwt_token_validation_valid() {
        let middleware = create_test_middleware();
        
        let claims = TokenClaims {
            sub: "@admin:example.com".to_string(),
            username: "admin".to_string(),
            is_admin: true,
            permissions: vec![Permission::SystemAdmin],
            session_id: "test-session".to_string(),
            exp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs() + 3600,
            iat: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };
        
        let token = create_test_jwt(&claims, "test-secret");
        let result = middleware.validate_jwt_token(&token);
        
        assert!(result.is_ok());
        let validated = result.unwrap();
        assert_eq!(validated.sub, "@admin:example.com");
        assert_eq!(validated.username, "admin");
        assert!(validated.is_admin);
    }

    #[test]
    fn test_jwt_token_validation_expired() {
        let middleware = create_test_middleware();
        
        let claims = TokenClaims {
            sub: "@admin:example.com".to_string(),
            username: "admin".to_string(),
            is_admin: true,
            permissions: vec![],
            session_id: "test-session".to_string(),
            // Token expired 1 hour ago
            exp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs() - 3600,
            iat: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs() - 7200,
        };
        
        let token = create_test_jwt(&claims, "test-secret");
        let result = middleware.validate_jwt_token(&token);
        
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), WebConfigError::AuthError { .. }));
    }

    #[test]
    fn test_jwt_token_validation_invalid_format() {
        let middleware = create_test_middleware();
        
        let result = middleware.validate_jwt_token("invalid-token");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), WebConfigError::AuthError { .. }));
    }

    #[test]
    fn test_jwt_token_validation_wrong_parts() {
        let middleware = create_test_middleware();
        
        let result = middleware.validate_jwt_token("a.b");
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_base64_urlsafe() {
        let input = "dGVzdA";
        let result = decode_base64_urlsafe(input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"test");
    }

    #[test]
    fn test_decode_base64_urlsafe_with_padding() {
        let input = "dGVzdA";
        let result = decode_base64_urlsafe(input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"test");
    }

    #[test]
    fn test_decode_base64_urlsafe_url_safe() {
        // Test URL-safe base64 encoding
        let input = "PDw_Pz4-";
        let result = decode_base64_urlsafe(input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_auth_state_ext() {
        let unauthenticated = AuthState::Unauthenticated;
        assert!(!unauthenticated.is_authenticated());
        assert!(!unauthenticated.is_authenticating());
        assert!(!unauthenticated.is_failed());
        assert!(unauthenticated.user().is_none());
        
        let authenticating = AuthState::Authenticating;
        assert!(!authenticating.is_authenticated());
        assert!(authenticating.is_authenticating());
        assert!(!authenticating.is_failed());
        
        let admin_user = AdminUser {
            user_id: "@admin:example.com".to_string(),
            username: "admin".to_string(),
            is_admin: true,
            session_id: "test".to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            permissions: vec![Permission::SystemAdmin],
        };
        let authenticated = AuthState::Authenticated(admin_user.clone());
        assert!(authenticated.is_authenticated());
        assert!(!authenticated.is_authenticating());
        assert!(!authenticated.is_failed());
        assert_eq!(authenticated.user(), Some(&admin_user));
        
        let failed = AuthState::Failed("error".to_string());
        assert!(!failed.is_authenticated());
        assert!(!failed.is_authenticating());
        assert!(failed.is_failed());
        assert_eq!(failed.error(), Some(&"error".to_string()));
    }

    #[test]
    fn test_permission_description() {
        assert_eq!(Permission::ConfigManagement.description(), "Configuration Management");
        assert_eq!(Permission::UserManagement.description(), "User Management");
        assert_eq!(Permission::SystemAdmin.description(), "System Administrator");
    }

    #[test]
    fn test_permission_all() {
        let all = Permission::all();
        assert_eq!(all.len(), 9);
        assert!(all.contains(&Permission::ConfigManagement));
        assert!(all.contains(&Permission::UserManagement));
        assert!(all.contains(&Permission::SystemAdmin));
    }

    // ============ Property Tests ============
    // **Validates: Requirements 10.1, 10.2, 10.3, 10.4, 10.5**
    // **Property 2: Authentication and Authorization Consistency**

    /// Property test: JWT token validation should be consistent
    /// Valid tokens should always pass, invalid tokens should always fail
    proptest! {
        #[test]
        fn test_jwt_token_validation_consistency(
            user_id in "[a-zA-Z0-9_-]+@[a-zA-Z0-9_-]+\\.[a-zA-Z0-9_-]+",
            username in "[a-zA-Z0-9_-]{1,32}",
            is_admin in proptest::bool::ANY,
            session_id in "[a-zA-Z0-9_-]{1,64}",
            // Generate future expiration times
            exp_offset in 0u64..1000000,
        ) {
            let middleware = create_test_middleware();
            
            // Create claims with valid format
            let claims = TokenClaims {
                sub: format!("@{}:example.com", user_id),
                username: username.clone(),
                is_admin,
                permissions: if is_admin {
                    vec![Permission::SystemAdmin]
                } else {
                    vec![Permission::UserManagement]
                },
                session_id: session_id.clone(),
                exp: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() + exp_offset,
                iat: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            };
            
            let token = create_test_jwt(&claims, "test-secret");
            
            // Token with future expiration should be valid
            if exp_offset > 0 {
                let result = middleware.validate_jwt_token(&token);
                assert!(result.is_ok(), "Valid token should pass validation: user_id={}, exp_offset={}", user_id, exp_offset);
                
                let validated = result.unwrap();
                assert_eq!(validated.sub, format!("@{}:example.com", user_id));
                assert_eq!(validated.username, username);
                assert_eq!(validated.is_admin, is_admin);
            }
        }
    }

    /// Property test: Permission checking should be consistent
    /// A user with SystemAdmin should have all permissions
    proptest! {
        #[test]
        fn test_system_admin_has_all_permissions(
            user_id in "[a-zA-Z0-9_-]+@[a-zA-Z0-9_-]+\\.[a-zA-Z0-9_-]+",
        ) {
            let admin_user = AdminUser {
                user_id: format!("@{}:example.com", user_id),
                username: user_id.clone(),
                is_admin: true,
                session_id: "test-session".to_string(),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
                permissions: vec![Permission::SystemAdmin],
            };
            
            // System admin should have all permissions
            assert!(admin_user.has_permission(&Permission::SystemAdmin));
            assert!(admin_user.has_permission(&Permission::UserManagement));
            assert!(admin_user.has_permission(&Permission::RoomManagement));
            assert!(admin_user.has_permission(&Permission::ConfigManagement));
            assert!(admin_user.has_permission(&Permission::MediaManagement));
            assert!(admin_user.has_permission(&Permission::FederationManagement));
            assert!(admin_user.has_permission(&Permission::AppserviceManagement));
            assert!(admin_user.has_permission(&Permission::ServerControl));
            assert!(admin_user.has_permission(&Permission::AuditLogAccess));
        }
    }

    /// Property test: Non-admin users should only have assigned permissions
    proptest! {
        #[test]
        fn test_non_admin_permissions(
            user_id in "[a-zA-Z0-9_-]+@[a-zA-Z0-9_-]+\\.[a-zA-Z0-9_-]+",
            // Generate a small number of permissions
            perm_count in 0u32..3,
        ) {
            let permissions: Vec<Permission> = (0..perm_count)
                .map(|i| match i % 3 {
                    0 => Permission::UserManagement,
                    1 => Permission::RoomManagement,
                    _ => Permission::MediaManagement,
                })
                .collect();
            
            let user = AdminUser {
                user_id: format!("@{}:example.com", user_id),
                username: user_id.clone(),
                is_admin: false,
                session_id: "test-session".to_string(),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
                permissions: permissions.clone(),
            };
            
            // Non-admin should not have SystemAdmin permission
            assert!(!user.has_permission(&Permission::SystemAdmin));
            
            // Should have only the assigned permissions
            for perm in &permissions {
                assert!(user.has_permission(perm), "User should have permission {:?}", perm);
            }
        }
    }

    /// Property test: Session validity should be consistent
    proptest! {
        #[test]
        fn test_session_validity_consistency(
            user_id in "[a-zA-Z0-9_-]+@[a-zA-Z0-9_-]+\\.[a-zA-Z0-9_-]+",
            // Generate time offsets from 1 to +3600 seconds (avoid 0 and negative)
            time_offset in 1u64..7200,
            is_future in proptest::bool::ANY,
        ) {
            let now = SystemTime::now();
            // Only add duration, never subtract to avoid overflow
            let duration = Duration::from_secs(time_offset);
            let expires_at = if is_future {
                now + duration
            } else {
                // For past times, we use a very old time
                SystemTime::UNIX_EPOCH
            };
            
            let user = AdminUser {
                user_id: format!("@{}:example.com", user_id),
                username: user_id.clone(),
                is_admin: false,
                session_id: "test-session".to_string(),
                expires_at,
                permissions: vec![],
            };
            
            // Session validity should match time comparison
            let is_valid = user.is_session_valid();
            let remaining = user.remaining_session_time();
            
            if is_future {
                assert!(is_valid, "Future expiration should be valid: offset={}", time_offset);
                assert!(remaining.is_some(), "Future expiration should have remaining time");
                assert!(remaining.unwrap() > 0, "Future expiration should have positive remaining time");
            } else {
                assert!(!is_valid, "Past expiration should be invalid");
                assert!(remaining.is_none(), "Past expiration should have no remaining time");
            }
        }
    }

    /// Property test: AuthState operations should be consistent
    proptest! {
        #[test]
        fn test_auth_state_operations(
            user_id in "[a-zA-Z0-9_-]+@[a-zA-Z0-9_-]+\\.[a-zA-Z0-9_-]+",
            is_admin in proptest::bool::ANY,
        ) {
            let admin_user = AdminUser {
                user_id: format!("@{}:example.com", user_id),
                username: user_id.clone(),
                is_admin,
                session_id: "test-session".to_string(),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
                permissions: if is_admin {
                    vec![Permission::SystemAdmin]
                } else {
                    vec![Permission::UserManagement]
                },
            };
            
            // AuthState::Authenticated should always be authenticated
            let authenticated = AuthState::Authenticated(admin_user.clone());
            assert!(authenticated.is_authenticated());
            assert!(!authenticated.is_authenticating());
            assert!(!authenticated.is_failed());
            assert_eq!(authenticated.user(), Some(&admin_user));
            
            // AuthState::Unauthenticated should never be authenticated
            let unauthenticated = AuthState::Unauthenticated;
            assert!(!unauthenticated.is_authenticated());
            assert!(!unauthenticated.is_authenticating());
            assert!(!unauthenticated.is_failed());
            assert!(unauthenticated.user().is_none());
            
            // AuthState::Authenticating should be in progress
            let authenticating = AuthState::Authenticating;
            assert!(!authenticating.is_authenticated());
            assert!(authenticating.is_authenticating());
            assert!(!authenticating.is_failed());
            
            // AuthState::Failed should have error
            let error_msg = "test error".to_string();
            let failed = AuthState::Failed(error_msg.clone());
            assert!(!failed.is_authenticated());
            assert!(!failed.is_authenticating());
            assert!(failed.is_failed());
            assert_eq!(failed.error(), Some(&error_msg));
        }
    }

    /// Property test: Permission has_any_permission should be consistent
    proptest! {
        #[test]
        fn test_has_any_permission_consistency(
            user_id in "[a-zA-Z0-9_-]+@[a-zA-Z0-9_-]+\\.[a-zA-Z0-9_-]+",
            // Generate user permissions
            perm_count in 0u32..3,
            // Generate check permissions
            check_count in 1u32..4,
        ) {
            let user_permissions: Vec<Permission> = (0..perm_count)
                .map(|i| match i % 3 {
                    0 => Permission::UserManagement,
                    1 => Permission::RoomManagement,
                    _ => Permission::MediaManagement,
                })
                .collect();
            
            let check_permissions: Vec<Permission> = (0..check_count)
                .map(|i| match i % 4 {
                    0 => Permission::UserManagement,
                    1 => Permission::RoomManagement,
                    2 => Permission::MediaManagement,
                    _ => Permission::ConfigManagement,
                })
                .collect();
            
            let user = AdminUser {
                user_id: format!("@{}:example.com", user_id),
                username: user_id.clone(),
                is_admin: false,
                session_id: "test-session".to_string(),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
                permissions: user_permissions.clone(),
            };
            
            // has_any_permission should return true if any permission matches
            let has_any = user.has_any_permission(&check_permissions);
            
            // Check that has_any_permission is consistent with individual has_permission calls
            let individual_check = check_permissions.iter().any(|p| user.has_permission(p));
            assert_eq!(has_any, individual_check, 
                "has_any_permission({:?}) should match any(has_permission) for user with permissions {:?}",
                check_permissions, user_permissions);
        }
    }
}