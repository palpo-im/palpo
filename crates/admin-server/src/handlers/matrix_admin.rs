/// Matrix Admin HTTP Handlers
///
/// This module implements the REST API endpoints for Matrix admin operations.
/// These endpoints handle the second tier of the two-tier admin system,
/// providing Matrix admin user creation, authentication, and password management.
///
/// # Endpoints
///
/// - `POST /api/v1/admin/matrix-admin/create` - Create Matrix admin user
/// - `POST /api/v1/admin/matrix-admin/login` - Matrix admin login
/// - `POST /api/v1/admin/matrix-admin/change-password` - Change Matrix admin password
///
/// # Requirements
///
/// Implements requirements:
/// - 7.1: Matrix admin creation requires Palpo server running
/// - 8.1: Matrix admin login uses standard Matrix authentication
/// - 8.4: Matrix admin can change their password

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

use crate::matrix_admin_creation::MatrixAdminCreationService;
use crate::matrix_auth_service::AuthService;
use crate::types::AdminError;

/// Shared application state for Matrix admin handlers
#[derive(Clone, Debug)]
pub struct MatrixAdminState {
    pub creation_service: Arc<MatrixAdminCreationService>,
    pub auth_service: Arc<AuthService>,
    pub homeserver_url: String,
}

/// Global Matrix admin state
static MATRIX_ADMIN_STATE: OnceLock<MatrixAdminState> = OnceLock::new();

/// Initialize the global Matrix admin state
pub fn init_matrix_admin_state(state: MatrixAdminState) {
    MATRIX_ADMIN_STATE
        .set(state)
        .expect("Matrix admin state already initialized");
}

/// Get the global Matrix admin state
pub fn get_matrix_admin_state() -> &'static MatrixAdminState {
    MATRIX_ADMIN_STATE
        .get()
        .expect("Matrix admin state not initialized")
}

// ===== Request/Response Types =====

/// Request body for creating a Matrix admin
#[derive(Debug, Deserialize)]
pub struct CreateMatrixAdminRequest {
    /// Username for the Matrix admin (without @ or domain)
    pub username: String,
    /// Password for the Matrix admin
    pub password: String,
    /// Optional display name
    #[serde(default)]
    pub displayname: Option<String>,
}

/// Response for successful Matrix admin creation
#[derive(Debug, Serialize)]
pub struct CreateMatrixAdminResponse {
    /// Full Matrix user ID (e.g., "@admin:localhost")
    pub user_id: String,
    /// Username (without @ or domain)
    pub username: String,
    /// Temporary password (should be changed on first login)
    pub password: String,
    /// Success message
    pub message: String,
}

/// Request body for Matrix admin login
#[derive(Debug, Deserialize)]
pub struct MatrixAdminLoginRequest {
    /// Username (without @ or domain)
    pub username: String,
    /// Password
    pub password: String,
}

/// Response for successful Matrix admin login
#[derive(Debug, Serialize)]
pub struct MatrixAdminLoginResponse {
    /// Access token for Matrix API requests
    pub access_token: String,
    /// Full Matrix user ID
    pub user_id: String,
    /// Whether the user has admin privileges
    pub is_admin: bool,
    /// Whether the user must change their password
    pub force_password_change: bool,
}

/// Request body for Matrix admin password change
#[derive(Debug, Deserialize)]
pub struct MatrixAdminChangePasswordRequest {
    /// Full Matrix user ID (e.g., "@admin:localhost")
    pub user_id: String,
    /// Current password for verification
    pub current_password: String,
    /// New password to set
    pub new_password: String,
}

/// Response for successful password change
#[derive(Debug, Serialize)]
pub struct MatrixAdminChangePasswordResponse {
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

/// POST /api/v1/admin/matrix-admin/create
///
/// Creates a new Matrix admin user.
///
/// This endpoint:
/// 1. Checks if Palpo server is running (Requirement 7.1)
/// 2. Validates the password against policy
/// 3. Creates the user via Matrix Admin API with admin=true
/// 4. Verifies admin status after creation
/// 5. Returns the created user's credentials
///
/// # Request Body
///
/// ```json
/// {
///   "username": "admin",
///   "password": "SecureP@ssw0rd123",
///   "displayname": "Administrator"
/// }
/// ```
///
/// # Response
///
/// - 200 OK: Matrix admin created successfully
/// - 400 Bad Request: Password policy violation
/// - 503 Service Unavailable: Palpo server not running
/// - 500 Internal Server Error: Creation failed
///
/// # Example Response
///
/// ```json
/// {
///   "user_id": "@admin:localhost",
///   "username": "admin",
///   "password": "SecureP@ssw0rd123",
///   "message": "Matrix admin created successfully. Please save these credentials."
/// }
/// ```
///
/// # Requirements
///
/// Implements requirements:
/// - 7.1: Verify Palpo server is running before creation
/// - 7.2: Return clear error if server not running
/// - 7.3: Use Matrix Admin API endpoint
/// - 7.4: Set admin field to 1 (true)
/// - 7.5: Validate password policy
/// - 7.6: Return created username and password
/// - 7.7: Verify admin status after creation
#[handler]
pub async fn create_matrix_admin(req: &mut Request, res: &mut Response) {
    let state = get_matrix_admin_state();

    let body = match req.parse_json::<CreateMatrixAdminRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid create matrix admin request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid request body".to_string(),
            }));
            return;
        }
    };

    match state
        .creation_service
        .create_matrix_admin(
            &body.username,
            &body.password,
            body.displayname.as_deref(),
        )
        .await
    {
        Ok(result) => {
            tracing::info!("Matrix admin created: {}", result.user_id);
            res.render(Json(CreateMatrixAdminResponse {
                user_id: result.user_id,
                username: result.username,
                password: result.password,
                message: "Matrix admin created successfully. Please save these credentials."
                    .to_string(),
            }));
        }
        Err(AdminError::ServerNotRunning) => {
            tracing::warn!("Attempted to create Matrix admin while server not running");
            res.status_code(StatusCode::SERVICE_UNAVAILABLE);
            res.render(Json(ErrorResponse {
                error: "Palpo server is not running. Please start the server first.".to_string(),
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
        Err(AdminError::AdminStatusNotSet) => {
            tracing::error!("Admin status was not set correctly after user creation");
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Failed to set admin privileges for user".to_string(),
            }));
        }
        Err(AdminError::MatrixApiError(msg)) => {
            tracing::error!("Matrix API error during admin creation: {}", msg);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Failed to create Matrix admin: {}", msg),
            }));
        }
        Err(e) => {
            tracing::error!("Unexpected error creating Matrix admin: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Failed to create Matrix admin".to_string(),
            }));
        }
    }
}

/// POST /api/v1/admin/matrix-admin/login
///
/// Authenticates a Matrix admin user using standard Matrix login.
///
/// This endpoint:
/// 1. Authenticates via Matrix `/_matrix/client/r0/login` endpoint
/// 2. Verifies the user has admin privileges
/// 3. Checks for force_password_change flag
/// 4. Returns access token and user information
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
/// - 200 OK: Authentication successful
/// - 401 Unauthorized: Invalid credentials or not an admin
/// - 500 Internal Server Error: Authentication failed
///
/// # Example Response
///
/// ```json
/// {
///   "access_token": "syt_...",
///   "user_id": "@admin:localhost",
///   "is_admin": true,
///   "force_password_change": false
/// }
/// ```
///
/// # Requirements
///
/// Implements requirements:
/// - 8.1: Use standard Matrix login endpoint
/// - 8.2: Verify admin status after login
/// - 8.3: Check force_password_change flag
#[handler]
pub async fn login_matrix_admin(req: &mut Request, res: &mut Response) {
    let state = get_matrix_admin_state();

    let body = match req.parse_json::<MatrixAdminLoginRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid matrix admin login request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid request body".to_string(),
            }));
            return;
        }
    };

    match state
        .auth_service
        .authenticate(&body.username, &body.password, &state.homeserver_url)
        .await
    {
        Ok(auth_result) => {
            // Verify the user is actually an admin
            if !auth_result.is_admin {
                tracing::warn!(
                    "User {} attempted to login but is not an admin",
                    auth_result.user_id
                );
                res.status_code(StatusCode::UNAUTHORIZED);
                res.render(Json(ErrorResponse {
                    error: "User does not have admin privileges".to_string(),
                }));
                return;
            }

            tracing::info!("Matrix admin logged in: {}", auth_result.user_id);
            res.render(Json(MatrixAdminLoginResponse {
                access_token: auth_result.access_token,
                user_id: auth_result.user_id,
                is_admin: auth_result.is_admin,
                force_password_change: auth_result.force_password_change,
            }));
        }
        Err(AdminError::InvalidCredentials) => {
            tracing::warn!("Failed Matrix admin login attempt for user: {}", body.username);
            res.status_code(StatusCode::UNAUTHORIZED);
            res.render(Json(ErrorResponse {
                error: "Invalid username or password".to_string(),
            }));
        }
        Err(AdminError::MatrixApiError(msg)) => {
            tracing::error!("Matrix API error during login: {}", msg);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Authentication failed: {}", msg),
            }));
        }
        Err(e) => {
            tracing::error!("Unexpected error during Matrix admin login: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Authentication failed".to_string(),
            }));
        }
    }
}

/// POST /api/v1/admin/matrix-admin/change-password
///
/// Changes a Matrix admin user's password.
///
/// This endpoint:
/// 1. Validates the new password against policy
/// 2. Verifies the current password by authenticating
/// 3. Updates the password via Matrix Admin API
/// 4. Clears the force_password_change flag
///
/// # Request Body
///
/// ```json
/// {
///   "user_id": "@admin:localhost",
///   "current_password": "OldP@ssw0rd123",
///   "new_password": "NewP@ssw0rd456"
/// }
/// ```
///
/// # Response
///
/// - 200 OK: Password changed successfully
/// - 400 Bad Request: Password policy violation or same password
/// - 401 Unauthorized: Invalid current password
/// - 500 Internal Server Error: Password change failed
///
/// # Example Response
///
/// ```json
/// {
///   "message": "Password changed successfully"
/// }
/// ```
///
/// # Requirements
///
/// Implements requirements:
/// - 8.4: Matrix admin can change their password
/// - 8.5: Password change clears force_password_change flag
/// - 9.1, 9.2, 9.4: Password policy validation
#[handler]
pub async fn change_matrix_admin_password(req: &mut Request, res: &mut Response) {
    let state = get_matrix_admin_state();

    let body = match req.parse_json::<MatrixAdminChangePasswordRequest>().await {
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

    match state
        .auth_service
        .change_admin_password(
            &body.user_id,
            &body.current_password,
            &body.new_password,
            &state.homeserver_url,
        )
        .await
    {
        Ok(()) => {
            tracing::info!("Password changed successfully for user: {}", body.user_id);
            res.render(Json(MatrixAdminChangePasswordResponse {
                message: "Password changed successfully".to_string(),
            }));
        }
        Err(AdminError::InvalidCredentials) => {
            tracing::warn!(
                "Failed password change attempt for user {}: invalid current password",
                body.user_id
            );
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
        Err(AdminError::MatrixApiError(msg)) => {
            tracing::error!("Matrix API error during password change: {}", msg);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Failed to change password: {}", msg),
            }));
        }
        Err(e) => {
            tracing::error!("Unexpected error changing password: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: "Failed to change password".to_string(),
            }));
        }
    }
}
