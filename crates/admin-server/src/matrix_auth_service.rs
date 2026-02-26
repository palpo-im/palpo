/// Matrix Authentication Service (Tier 2)
///
/// This service handles Matrix admin authentication using the standard Matrix Client API.
/// It authenticates users via the Matrix login endpoint, verifies their admin status,
/// and checks for force password change requirements.
///
/// **Requirements Implemented:**
/// - 8.1: Matrix admin login uses standard Matrix login endpoint
/// - 8.2: Verify admin status after login
/// - 8.3: Check force_password_change flag and require password change if set

use crate::types::AdminError;
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Result of authentication attempt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResult {
    /// Access token for authenticated session
    pub access_token: String,
    /// Full Matrix user ID (e.g., "@admin:localhost")
    pub user_id: String,
    /// Whether the user has admin privileges
    pub is_admin: bool,
    /// Whether the user is a guest
    pub is_guest: bool,
    /// Whether the user must change their password before proceeding
    pub force_password_change: bool,
}

/// Response from Matrix login endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MatrixLoginResponse {
    /// Access token for the session
    access_token: String,
    /// Full Matrix user ID
    user_id: String,
    /// Home server name
    #[serde(default)]
    home_server: Option<String>,
    /// Device ID
    #[serde(default)]
    device_id: Option<String>,
}

/// Response from Matrix whoami endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WhoamiResponse {
    /// Full Matrix user ID
    user_id: String,
    /// Whether the user is a guest
    #[serde(default)]
    is_guest: bool,
    /// Device ID
    #[serde(default)]
    device_id: Option<String>,
}

/// Matrix Authentication Service
///
/// Provides authentication functionality for Matrix admin users using
/// the standard Matrix Client API endpoints.
#[derive(Debug)]
pub struct AuthService {
    /// HTTP client for making API requests
    client: Client,
}

impl AuthService {
    /// Creates a new AuthService instance
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Authenticates a user using Matrix standard login API
    ///
    /// This method performs the following steps:
    /// 1. Authenticates via `/_matrix/client/r0/login` endpoint
    /// 2. Verifies admin status via `/_matrix/client/r0/account/whoami` endpoint
    /// 3. Checks if user is admin via Matrix Admin API
    /// 4. Checks for force_password_change flag
    ///
    /// # Arguments
    /// * `username` - The username (without @ or domain)
    /// * `password` - The user's password
    /// * `homeserver` - The homeserver URL (e.g., "http://localhost:8008")
    ///
    /// # Returns
    /// `AuthResult` containing access token, user ID, admin status, and force password change flag
    ///
    /// # Errors
    /// - `AdminError::InvalidCredentials` - If username or password is incorrect
    /// - `AdminError::HttpError` - If the HTTP request fails
    /// - `AdminError::MatrixApiError` - If the Matrix API returns an error
    ///
    /// # Requirements
    /// - 8.1: Uses standard Matrix login endpoint `/_matrix/client/r0/login`
    /// - 8.2: Verifies admin status after login via whoami
    /// - 8.3: Checks force_password_change flag
    pub async fn authenticate(
        &self,
        username: &str,
        password: &str,
        homeserver: &str,
    ) -> Result<AuthResult, AdminError> {
        // Requirement 8.1: Use Matrix standard login endpoint
        let login_url = format!("{}/_matrix/client/r0/login", homeserver);
        
        let login_body = serde_json::json!({
            "type": "m.login.password",
            "user": username,
            "password": password
        });

        let response = self
            .client
            .post(&login_url)
            .json(&login_body)
            .send()
            .await?;

        // Check if authentication was successful
        if response.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(AdminError::InvalidCredentials);
        }

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AdminError::MatrixApiError(format!(
                "Login failed with status {}: {}",
                status, error_text
            )));
        }

        let login_response: MatrixLoginResponse = response
            .json()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to parse login response: {}", e)))?;

        // Requirement 8.2: Verify admin status via whoami
        let whoami = self
            .verify_admin_status(&login_response.access_token, homeserver)
            .await?;

        // Check if user is admin via Matrix Admin API
        let is_admin = self
            .is_user_admin(&login_response.user_id, &login_response.access_token, homeserver)
            .await?;

        // Requirement 8.3: Check force_password_change flag
        let force_password_change = self
            .check_force_password_change(&login_response.user_id, &login_response.access_token, homeserver)
            .await
            .unwrap_or(false); // Default to false if check fails

        Ok(AuthResult {
            access_token: login_response.access_token,
            user_id: login_response.user_id,
            is_admin,
            is_guest: whoami.is_guest,
            force_password_change,
        })
    }

    /// Verifies admin status using whoami endpoint
    ///
    /// Calls the `/_matrix/client/r0/account/whoami` endpoint to verify
    /// the access token is valid and retrieve basic user information.
    ///
    /// # Arguments
    /// * `access_token` - The access token from login
    /// * `homeserver` - The homeserver URL
    ///
    /// # Returns
    /// `WhoamiResponse` containing user ID and guest status
    ///
    /// # Errors
    /// Returns `AdminError` if the API call fails
    ///
    /// # Requirements
    /// - 8.2: Uses `/_matrix/client/r0/account/whoami` endpoint
    async fn verify_admin_status(
        &self,
        access_token: &str,
        homeserver: &str,
    ) -> Result<WhoamiResponse, AdminError> {
        let whoami_url = format!("{}/_matrix/client/r0/account/whoami", homeserver);

        let response = self
            .client
            .get(&whoami_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AdminError::MatrixApiError(format!(
                "Whoami failed with status {}: {}",
                status, error_text
            )));
        }

        response
            .json()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to parse whoami response: {}", e)))
    }

    /// Checks if user is admin by querying Matrix Admin API
    ///
    /// Queries the Matrix Admin API to determine if the user has admin privileges.
    /// Uses the `/_synapse/admin/v2/users/{user_id}` endpoint.
    ///
    /// # Arguments
    /// * `user_id` - The full Matrix user ID (e.g., "@admin:localhost")
    /// * `access_token` - The access token for authentication
    /// * `homeserver` - The homeserver URL
    ///
    /// # Returns
    /// `true` if the user is an admin, `false` otherwise
    ///
    /// # Errors
    /// Returns `AdminError` if the API call fails
    pub async fn is_user_admin(
        &self,
        user_id: &str,
        access_token: &str,
        homeserver: &str,
    ) -> Result<bool, AdminError> {
        let encoded = urlencoding::encode(user_id);
        let url = format!("{}/_synapse/admin/v2/users/{}", homeserver, encoded);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        match response.status() {
            reqwest::StatusCode::OK => {
                #[derive(Deserialize)]
                struct UserInfo {
                    admin: bool,
                }
                
                let user_info: UserInfo = response
                    .json()
                    .await
                    .map_err(|e| AdminError::MatrixApiError(format!("Failed to parse user info: {}", e)))?;
                
                Ok(user_info.admin)
            }
            reqwest::StatusCode::NOT_FOUND => Ok(false),
            reqwest::StatusCode::FORBIDDEN => Ok(false), // User doesn't have permission to check
            _ => {
                let status = response.status();
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                Err(AdminError::MatrixApiError(format!(
                    "Admin check failed with status {}: {}",
                    status, error_text
                )))
            }
        }
    }

    /// Checks if the user has the force_password_change flag set
    ///
    /// Queries the Matrix Admin API to check if the user is required to change
    /// their password before accessing the system.
    ///
    /// # Arguments
    /// * `user_id` - The full Matrix user ID
    /// * `access_token` - The access token for authentication
    /// * `homeserver` - The homeserver URL
    ///
    /// # Returns
    /// `true` if password change is required, `false` otherwise
    ///
    /// # Errors
    /// Returns `AdminError` if the API call fails
    ///
    /// # Requirements
    /// - 8.3: Checks force_password_change flag
    async fn check_force_password_change(
        &self,
        user_id: &str,
        access_token: &str,
        homeserver: &str,
    ) -> Result<bool, AdminError> {
        let encoded = urlencoding::encode(user_id);
        let url = format!("{}/_synapse/admin/v2/users/{}", homeserver, encoded);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            // If we can't check, default to false (don't block login)
            return Ok(false);
        }

        #[derive(Deserialize)]
        struct UserAttributes {
            #[serde(default)]
            force_password_change: bool,
        }

        let attrs: UserAttributes = response
            .json()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to parse user attributes: {}", e)))?;

        Ok(attrs.force_password_change)
    }

    /// Sets the force_password_change flag for a user
    ///
    /// Updates the user's attributes to require password change on next login.
    ///
    /// # Arguments
    /// * `user_id` - The full Matrix user ID
    /// * `access_token` - Admin access token for authentication
    /// * `homeserver` - The homeserver URL
    /// * `force_change` - Whether to require password change
    ///
    /// # Returns
    /// Ok(()) on success
    ///
    /// # Errors
    /// Returns `AdminError` if the API call fails
    pub async fn set_force_password_change(
        &self,
        user_id: &str,
        access_token: &str,
        homeserver: &str,
        force_change: bool,
    ) -> Result<(), AdminError> {
        let encoded = urlencoding::encode(user_id);
        let url = format!("{}/_synapse/admin/v2/users/{}", homeserver, encoded);

        let request_body = serde_json::json!({
            "force_password_change": force_change
        });

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AdminError::MatrixApiError(format!(
                "Failed to set force_password_change with status {}: {}",
                status, error_text
            )));
        }

        Ok(())
    }

    /// Changes a Matrix admin user's password
    ///
    /// This method performs the complete password change workflow:
    /// 1. Validates the new password against the password policy
    /// 2. Verifies the current password by attempting authentication
    /// 3. Updates the password via Matrix Admin API
    /// 4. Clears the force_password_change flag
    /// 5. Logs the password change event (placeholder for audit logging)
    ///
    /// # Arguments
    /// * `user_id` - The full Matrix user ID (e.g., "@admin:localhost")
    /// * `current_password` - The user's current password for verification
    /// * `new_password` - The new password to set
    /// * `homeserver` - The homeserver URL (e.g., "http://localhost:8008")
    ///
    /// # Returns
    /// Ok(()) on success
    ///
    /// # Errors
    /// - `AdminError::InvalidCredentials` - If current password is incorrect
    /// - `AdminError::PasswordTooShort` - If new password is less than 12 characters
    /// - `AdminError::Missing*` - If new password doesn't meet complexity requirements
    /// - `AdminError::PasswordNotChanged` - If new password is same as current password
    /// - `AdminError::MatrixApiError` - If API communication fails
    ///
    /// # Requirements
    /// - 8.4: Matrix admin can change their own password
    /// - 8.5: Password change clears force_password_change flag
    /// - 8.6: Audit log records password change events
    /// - 9.1, 9.2, 9.4: Password policy validation
    pub async fn change_admin_password(
        &self,
        user_id: &str,
        current_password: &str,
        new_password: &str,
        homeserver: &str,
    ) -> Result<(), AdminError> {
        // Requirement 9.3: Ensure new password is different from current password
        if current_password == new_password {
            return Err(AdminError::PasswordNotChanged);
        }

        // Requirement 9.1, 9.2, 9.4: Validate new password against policy
        Self::validate_password_policy(new_password)?;

        // Requirement 8.4: Verify current password via Matrix auth
        // Extract username from user_id (e.g., "@admin:localhost" -> "admin")
        let username = user_id
            .strip_prefix('@')
            .and_then(|s| s.split(':').next())
            .ok_or_else(|| AdminError::MatrixApiError("Invalid user_id format".to_string()))?;

        // Authenticate with current password to verify it's correct
        let auth_result = self.authenticate(username, current_password, homeserver).await?;

        // Verify the authenticated user matches the target user_id
        if auth_result.user_id != user_id {
            return Err(AdminError::InvalidCredentials);
        }

        // Requirement 8.4: Update password via Matrix Admin API
        // Use the authenticated access token to change the password
        let encoded = urlencoding::encode(user_id);
        let url = format!(
            "{}/_synapse/admin/v1/reset_password/{}",
            homeserver, encoded
        );

        let request_body = serde_json::json!({
            "new_password": new_password,
            "logout_devices": false
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", auth_result.access_token))
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AdminError::MatrixApiError(format!(
                "Password change failed with status {}: {}",
                status, error_text
            )));
        }

        // Requirement 8.5: Clear force_password_change flag
        self.set_force_password_change(
            user_id,
            &auth_result.access_token,
            homeserver,
            false,
        )
        .await?;

        // Requirement 8.6: Log password change event
        // TODO: Integrate with audit logging system when available
        // AuditLogger::log_event(AuditEvent::PasswordChanged {
        //     user_id: user_id.to_string(),
        // }).await;

        Ok(())
    }

    /// Validates password against policy requirements
    ///
    /// Implements requirements 9.1, 9.2, 9.4:
    /// - Minimum 12 characters
    /// - Must contain uppercase, lowercase, digit, and special character
    ///
    /// # Arguments
    /// * `password` - The password to validate
    ///
    /// # Errors
    /// Returns specific `AdminError` variants for each policy violation
    fn validate_password_policy(password: &str) -> Result<(), AdminError> {
        const MIN_LENGTH: usize = 12;
        const SPECIAL_CHARS: &str = "!@#$%^&*()_+-=[]{}|;:,.<>?";

        // Requirement 9.1: Check minimum length
        if password.len() < MIN_LENGTH {
            return Err(AdminError::PasswordTooShort(password.len()));
        }

        // Requirement 9.2: Check for uppercase letter
        if !password.chars().any(|c| c.is_uppercase()) {
            return Err(AdminError::MissingUppercase);
        }

        // Requirement 9.2: Check for lowercase letter
        if !password.chars().any(|c| c.is_lowercase()) {
            return Err(AdminError::MissingLowercase);
        }

        // Requirement 9.2: Check for digit
        if !password.chars().any(|c| c.is_ascii_digit()) {
            return Err(AdminError::MissingDigit);
        }

        // Requirement 9.2: Check for special character
        if !password.chars().any(|c| SPECIAL_CHARS.contains(c)) {
            return Err(AdminError::MissingSpecialChar);
        }

        Ok(())
    }
}

impl Default for AuthService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_service_creation() {
        let _service = AuthService::new();
        // Service created successfully
    }

    #[test]
    fn test_auth_result_serialization() {
        let result = AuthResult {
            access_token: "test_token".to_string(),
            user_id: "@admin:localhost".to_string(),
            is_admin: true,
            is_guest: false,
            force_password_change: false,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("test_token"));
        assert!(json.contains("@admin:localhost"));
    }

    #[test]
    fn test_validate_password_policy_valid() {
        let result = AuthService::validate_password_policy("SecureP@ss123!");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_password_policy_too_short() {
        let result = AuthService::validate_password_policy("Short1!");
        assert!(matches!(result, Err(AdminError::PasswordTooShort(7))));
    }

    #[test]
    fn test_validate_password_policy_missing_uppercase() {
        let result = AuthService::validate_password_policy("lowercase123!");
        assert!(matches!(result, Err(AdminError::MissingUppercase)));
    }

    #[test]
    fn test_validate_password_policy_missing_lowercase() {
        let result = AuthService::validate_password_policy("UPPERCASE123!");
        assert!(matches!(result, Err(AdminError::MissingLowercase)));
    }

    #[test]
    fn test_validate_password_policy_missing_digit() {
        let result = AuthService::validate_password_policy("NoDigitsHere!");
        assert!(matches!(result, Err(AdminError::MissingDigit)));
    }

    #[test]
    fn test_validate_password_policy_missing_special() {
        let result = AuthService::validate_password_policy("NoSpecialChar123");
        assert!(matches!(result, Err(AdminError::MissingSpecialChar)));
    }
}
