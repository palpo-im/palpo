/// Web UI Admin HTTP Handlers
///
/// This module implements the REST API endpoints for Web UI admin operations.
/// These endpoints handle the first tier of the two-tier admin system,
/// providing authentication and session management independent of Palpo server.
///
/// # Endpoints
///
/// - `GET /api/v1/admin/webui-admin/status` - Check setup status and detect legacy credentials
/// - `POST /api/v1/admin/webui-admin/setup` - Create Web UI admin account
/// - `POST /api/v1/admin/webui-admin/login` - Authenticate and get session token
/// - `POST /api/v1/admin/webui-admin/change-password` - Change admin password
/// - `POST /api/v1/admin/webui-admin/logout` - Invalidate session token
/// - `POST /api/v1/admin/webui-admin/migrate` - Migrate from localStorage to database
///
/// # Requirements
///
/// Implements requirements:
/// - 1.1, 1.9: Web UI admin database authentication
/// - 3.1-3.5: Password change functionality
/// - 11.1-11.3: Migration from localStorage

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

use crate::migration_service::LegacyCredentials;
use crate::types::AdminError;
use crate::{MigrationService, SessionManager, WebUIAuthService};

/// Shared application state for handlers
///
/// Contains the services needed by the Web UI admin endpoints.
/// Services are wrapped in Arc for efficient cloning across handlers.
#[derive(Clone, Debug)]
pub struct AppState {
    pub auth_service: Arc<WebUIAuthService>,
    pub session_manager: Arc<SessionManager>,
    pub migration_service: Arc<MigrationService>,
}

/// Global application state
static APP_STATE: OnceLock<AppState> = OnceLock::new();

/// Initialize the global application state
pub fn init_app_state(state: AppState) {
    APP_STATE.set(state).expect("App state already initialized");
}

/// Get the global application state
pub fn get_app_state() -> &'static AppState {
    APP_STATE.get().expect("App state not initialized")
}

// ===== Request/Response Types =====

/// Response for the status endpoint
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    /// Whether initial setup is needed (no admin exists)
    pub needs_setup: bool,
    /// Whether legacy credentials might exist (client should check localStorage)
    pub check_legacy: bool,
}

/// Request body for setup endpoint
#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    /// Password for the admin account (username is fixed as "admin")
    pub password: String,
}

/// Response for successful setup
#[derive(Debug, Serialize)]
pub struct SetupResponse {
    /// Success message
    pub message: String,
}

/// Request body for login endpoint
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// Username (must be "admin")
    pub username: String,
    /// Password
    pub password: String,
}

/// Response for successful login
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// Success status
    pub success: bool,
    /// Session token for authenticated requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// User information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserInfo>,
    /// Error message if login failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// User information in login response
#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub user_id: String,
    pub username: String,
    pub is_admin: bool,
    pub session_id: String,
    pub expires_at: String,
    pub permissions: Vec<String>,
}

/// Request body for password change endpoint
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    /// Current password for verification
    pub current_password: String,
    /// New password to set
    pub new_password: String,
}

/// Response for successful password change
#[derive(Debug, Serialize)]
pub struct ChangePasswordResponse {
    /// Success message
    pub message: String,
}

/// Request body for logout endpoint
#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    /// Session token to invalidate
    pub token: String,
}

/// Response for successful logout
#[derive(Debug, Serialize)]
pub struct LogoutResponse {
    /// Success message
    pub message: String,
}

/// Request body for migration endpoint
#[derive(Debug, Deserialize)]
pub struct MigrateRequest {
    /// Legacy credentials from localStorage
    pub legacy_credentials: LegacyCredentials,
    /// User's password for verification
    pub password: String,
}

/// Response for successful migration
#[derive(Debug, Serialize)]
pub struct MigrateResponse {
    /// Success message
    pub message: String,
}

/// Standard error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
}

// ===== Handler Functions =====

/// GET /api/v1/admin/webui-admin/status
///
/// Checks if initial setup is needed and whether to check for legacy credentials.
///
/// # Returns
///
/// - `needs_setup: true` - No admin exists, setup wizard should be shown
/// - `needs_setup: false` - Admin exists, show login page
/// - `check_legacy: true` - Client should check localStorage for legacy credentials
///
/// # Requirements
///
/// Implements requirement 1.1: Check if admin exists
#[handler]
pub async fn status(res: &mut Response) {
    let state = get_app_state();

    match state.auth_service.admin_exists() {
        Ok(exists) => {
            let response = StatusResponse {
                needs_setup: !exists,
                check_legacy: !exists, // Only check legacy if no admin exists
            };
            res.render(Json(response));
        }
        Err(e) => {
            tracing::error!("Failed to check admin status: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Failed to check admin status".to_string(),
            }));
        }
    }
}

/// POST /api/v1/admin/webui-admin/setup
///
/// Creates the Web UI admin account in the database.
/// This is the initial setup step for new installations.
///
/// # Request Body
///
/// ```json
/// {
///   "password": "SecureP@ssw0rd123"
/// }
/// ```
///
/// # Response
///
/// - 200 OK: Admin created successfully
/// - 400 Bad Request: Password policy violation
/// - 409 Conflict: Admin already exists
/// - 500 Internal Server Error: Database error
///
/// # Requirements
///
/// Implements requirements:
/// - 1.3: Validate password policy
/// - 1.4: Use fixed username "admin"
/// - 1.5: Hash password with Argon2
/// - 1.6: Store credentials in database
#[handler]
pub async fn setup(req: &mut Request, res: &mut Response) {
    let state = get_app_state();

    let body = match req.parse_json::<SetupRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid setup request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid request body".to_string(),
            }));
            return;
        }
    };

    match state.auth_service.create_admin(&body.password) {
        Ok(()) => {
            tracing::info!("Web UI admin created successfully");
            res.render(Json(SetupResponse {
                message: "Admin account created successfully".to_string(),
            }));
        }
        Err(AdminError::WebUIAdminAlreadyExists) => {
            res.status_code(StatusCode::CONFLICT);
            res.render(Json(ErrorResponse {
                error: "Admin already exists".to_string(),
            }));
        }
        Err(AdminError::PasswordTooShort(len)) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: format!("Password too short: {} characters (minimum 12)", len),
            }));
        }
        Err(AdminError::MissingUppercase) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Password must contain at least one uppercase letter".to_string(),
            }));
        }
        Err(AdminError::MissingLowercase) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Password must contain at least one lowercase letter".to_string(),
            }));
        }
        Err(AdminError::MissingDigit) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Password must contain at least one digit".to_string(),
            }));
        }
        Err(AdminError::MissingSpecialChar) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Password must contain at least one special character".to_string(),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to create admin: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Failed to create admin account".to_string(),
            }));
        }
    }
}

/// POST /api/v1/admin/webui-admin/login
///
/// Authenticates the Web UI admin and returns a session token.
///
/// # Request Body
///
/// ```json
/// {
///   "username": "admin",
///   "password": "SecureP@ssw0rd123"
/// }
/// ```
///
/// # Response
///
/// - 200 OK: Authentication successful, returns session token
/// - 401 Unauthorized: Invalid credentials
/// - 500 Internal Server Error: Database error
///
/// # Requirements
///
/// Implements requirements:
/// - 1.7: Verify username and password
/// - 1.8: Generate session token
/// - 1.9: Independent of Palpo server
#[handler]
pub async fn login(req: &mut Request, res: &mut Response) {
    let state = get_app_state();

    let body = match req.parse_json::<LoginRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid login request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid request body".to_string(),
            }));
            return;
        }
    };

    match state.auth_service.authenticate(&body.username, &body.password) {
        Ok(_session_token) => {
            // Store session in session manager
            match state.session_manager.create_session(&body.username).await {
                Ok(token) => {
                    tracing::info!("User {} logged in successfully", body.username);
                    res.render(Json(LoginResponse {
                        success: true,
                        token: Some(token.token.clone()),
                        user: Some(UserInfo {
                            user_id: body.username.clone(),
                            username: body.username.clone(),
                            is_admin: true,
                            session_id: token.token,
                            expires_at: token.expires_at.to_rfc3339(),
                            permissions: vec![
                                "ConfigManagement".to_string(),
                                "UserManagement".to_string(),
                                "RoomManagement".to_string(),
                                "FederationManagement".to_string(),
                                "MediaManagement".to_string(),
                            ],
                        }),
                        error: None,
                    }));
                }
                Err(e) => {
                    tracing::error!("Failed to create session: {}", e);
                    res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                    res.render(Json(ErrorResponse {
                        error: "Failed to create session".to_string(),
                    }));
                }
            }
        }
        Err(AdminError::InvalidCredentials) => {
            tracing::warn!("Failed login attempt for user: {}", body.username);
            res.status_code(StatusCode::UNAUTHORIZED);
            res.render(Json(ErrorResponse {
                error: "Invalid username or password".to_string(),
            }));
        }
        Err(e) => {
            tracing::error!("Login error: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Login failed".to_string(),
            }));
        }
    }
}

/// POST /api/v1/admin/webui-admin/change-password
///
/// Changes the Web UI admin password.
/// Requires a valid session token in the Authorization header.
///
/// # Request Headers
///
/// - `Authorization: Bearer <session_token>`
///
/// # Request Body
///
/// ```json
/// {
///   "current_password": "OldP@ssw0rd123",
///   "new_password": "NewP@ssw0rd456"
/// }
/// ```
///
/// # Response
///
/// - 200 OK: Password changed successfully
/// - 400 Bad Request: Password policy violation or same password
/// - 401 Unauthorized: Invalid session or wrong current password
/// - 500 Internal Server Error: Database error
///
/// # Requirements
///
/// Implements requirements:
/// - 3.6: Verify current password
/// - 3.7: Validate new password policy
/// - 3.8: Verify new password different
/// - 3.9: Hash new password
/// - 3.10: Update database
#[handler]
pub async fn change_password(req: &mut Request, res: &mut Response) {
    let state = get_app_state();

    // Extract and validate session token from Authorization header
    let auth_header = match req.headers().get("Authorization") {
        Some(h) => h.to_str().unwrap_or(""),
        None => {
            res.status_code(StatusCode::UNAUTHORIZED);
            res.render(Json(ErrorResponse {
                error: "Missing Authorization header".to_string(),
            }));
            return;
        }
    };

    let token = if let Some(t) = auth_header.strip_prefix("Bearer ") {
        t
    } else {
        res.status_code(StatusCode::UNAUTHORIZED);
        res.render(Json(ErrorResponse {
            error: "Invalid Authorization header format".to_string(),
        }));
        return;
    };

    // Validate session
    match state.session_manager.validate_session(token).await {
        Ok(_username) => {
            // Session is valid, proceed with password change
        }
        Err(AdminError::SessionExpired) => {
            res.status_code(StatusCode::UNAUTHORIZED);
            res.render(Json(ErrorResponse {
                error: "Session expired".to_string(),
            }));
            return;
        }
        Err(AdminError::InvalidSessionToken) => {
            res.status_code(StatusCode::UNAUTHORIZED);
            res.render(Json(ErrorResponse {
                error: "Invalid session token".to_string(),
            }));
            return;
        }
        Err(e) => {
            tracing::error!("Session validation error: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Session validation failed".to_string(),
            }));
            return;
        }
    }

    // Parse request body
    let body = match req.parse_json::<ChangePasswordRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid change password request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid request body".to_string(),
            }));
            return;
        }
    };

    // Attempt password change
    match state
        .auth_service
        .change_password(&body.current_password, &body.new_password)
    {
        Ok(()) => {
            tracing::info!("Password changed successfully");
            res.render(Json(ChangePasswordResponse {
                message: "Password changed successfully".to_string(),
            }));
        }
        Err(AdminError::InvalidCredentials) => {
            res.status_code(StatusCode::UNAUTHORIZED);
            res.render(Json(ErrorResponse {
                error: "Current password is incorrect".to_string(),
            }));
        }
        Err(AdminError::PasswordNotChanged) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "New password must be different from current password".to_string(),
            }));
        }
        Err(AdminError::PasswordTooShort(len)) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: format!("Password too short: {} characters (minimum 12)", len),
            }));
        }
        Err(AdminError::MissingUppercase) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Password must contain at least one uppercase letter".to_string(),
            }));
        }
        Err(AdminError::MissingLowercase) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Password must contain at least one lowercase letter".to_string(),
            }));
        }
        Err(AdminError::MissingDigit) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Password must contain at least one digit".to_string(),
            }));
        }
        Err(AdminError::MissingSpecialChar) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Password must contain at least one special character".to_string(),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to change password: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Failed to change password".to_string(),
            }));
        }
    }
}

/// POST /api/v1/admin/webui-admin/logout
///
/// Invalidates the session token, logging out the user.
///
/// # Request Body
///
/// ```json
/// {
///   "token": "session_token_here"
/// }
/// ```
///
/// # Response
///
/// - 200 OK: Logout successful
/// - 400 Bad Request: Invalid request body
/// - 500 Internal Server Error: Session invalidation failed
///
/// # Requirements
///
/// Implements requirement 1.8: Session management
#[handler]
pub async fn logout(req: &mut Request, res: &mut Response) {
    let state = get_app_state();

    let body = match req.parse_json::<LogoutRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid logout request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid request body".to_string(),
            }));
            return;
        }
    };

    match state.session_manager.invalidate_session(&body.token).await {
        Ok(()) => {
            tracing::info!("User logged out successfully");
            res.render(Json(LogoutResponse {
                message: "Logged out successfully".to_string(),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to invalidate session: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Failed to logout".to_string(),
            }));
        }
    }
}

/// POST /api/v1/admin/webui-admin/migrate
///
/// Migrates credentials from localStorage to the database.
/// This endpoint is used when upgrading from the old localStorage-based
/// authentication to the new database-backed system.
///
/// # Request Body
///
/// ```json
/// {
///   "legacy_credentials": {
///     "username": "admin",
///     "password_hash": "...",
///     "salt": "..."
///   },
///   "password": "user_password_for_verification"
/// }
/// ```
///
/// # Response
///
/// - 200 OK: Migration successful
/// - 400 Bad Request: Invalid request body
/// - 401 Unauthorized: Password verification failed
/// - 409 Conflict: Admin already exists in database
/// - 500 Internal Server Error: Migration failed
///
/// # Requirements
///
/// Implements requirements:
/// - 11.3: Verify old password before migration
/// - 11.5: Migration must be idempotent
/// - 11.6: Failed migration must not corrupt existing data
/// - 11.7: Re-hash password with new salt
#[handler]
pub async fn migrate(req: &mut Request, res: &mut Response) {
    let state = get_app_state();

    let body = match req.parse_json::<MigrateRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid migrate request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid request body".to_string(),
            }));
            return;
        }
    };

    match state
        .migration_service
        .migrate_from_legacy(&body.legacy_credentials, &body.password)
    {
        Ok(()) => {
            tracing::info!("Credentials migrated successfully");
            res.render(Json(MigrateResponse {
                message: "Credentials migrated successfully".to_string(),
            }));
        }
        Err(AdminError::InvalidCredentials) => {
            res.status_code(StatusCode::UNAUTHORIZED);
            res.render(Json(ErrorResponse {
                error: "Password verification failed".to_string(),
            }));
        }
        Err(AdminError::WebUIAdminAlreadyExists) => {
            res.status_code(StatusCode::CONFLICT);
            res.render(Json(ErrorResponse {
                error: "Admin already exists in database".to_string(),
            }));
        }
        Err(e) => {
            tracing::error!("Migration failed: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Migration failed: {}", e),
            }));
        }
    }
}
