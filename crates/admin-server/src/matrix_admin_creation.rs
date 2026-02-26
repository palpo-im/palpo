/// Matrix Admin Creation Service (Tier 2)
///
/// This service handles the creation of Matrix admin users through the Matrix Admin API.
/// It requires the Palpo server to be running and validates that admin privileges are
/// correctly set after user creation.
///
/// **Requirements Implemented:**
/// - 7.1: Matrix admin creation requires Palpo server to be running
/// - 7.2: Return error if server not running with clear message
/// - 7.3: Use Matrix Admin API `/_synapse/admin/v2/register` endpoint
/// - 7.4: Set admin field to 1 in user creation
/// - 7.5: Validate password against policy before creation
/// - 7.6: Return created username and temporary password
/// - 7.7: Verify admin status after creation

use crate::types::{AdminError, CreateMatrixAdminResponse};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Matrix Admin API client for creating and managing admin users
///
/// This client communicates with the Palpo Matrix server's admin API endpoints
/// to create users with admin privileges and verify their status.
#[derive(Debug)]
pub struct MatrixAdminClient {
    /// Base URL of the Palpo server (e.g., "http://localhost:8008")
    base_url: String,
    /// HTTP client for making API requests
    client: Client,
}

impl MatrixAdminClient {
    /// Creates a new Matrix Admin API client
    ///
    /// # Arguments
    /// * `base_url` - The base URL of the Palpo server
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: Client::new(),
        }
    }

    /// Creates a new Matrix user with admin privileges
    ///
    /// Calls the Matrix Admin API `/_synapse/admin/v2/register` endpoint
    /// to create a user with the admin flag set to true.
    ///
    /// # Arguments
    /// * `username` - The username for the new admin (without @ or domain)
    /// * `password` - The password for the new admin
    /// * `displayname` - Optional display name for the user
    ///
    /// # Returns
    /// The user_id of the created admin user
    ///
    /// # Errors
    /// Returns `AdminError::MatrixApiError` if the API call fails
    pub async fn create_admin_user(
        &self,
        username: &str,
        password: &str,
        displayname: Option<&str>,
    ) -> Result<String, AdminError> {
        let url = format!("{}/_synapse/admin/v2/users/@{}:localhost", self.base_url, username);
        
        let request_body = serde_json::json!({
            "password": password,
            "admin": true,
            "displayname": displayname.unwrap_or(username)
        });

        let response = self
            .client
            .put(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to send request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AdminError::MatrixApiError(format!(
                "API returned status {}: {}",
                status, error_text
            )));
        }

        let user_info: UserInfoResponse = response
            .json()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to parse response: {}", e)))?;

        Ok(user_info.name)
    }

    /// Retrieves user information including admin status
    ///
    /// # Arguments
    /// * `user_id` - The full Matrix user ID (e.g., "@admin:localhost")
    ///
    /// # Returns
    /// User information including admin status
    ///
    /// # Errors
    /// Returns `AdminError::MatrixApiError` if the API call fails
    pub async fn get_user(&self, user_id: &str) -> Result<UserInfoResponse, AdminError> {
        let encoded = urlencoding::encode(user_id);
        let url = format!("{}/_synapse/admin/v2/users/{}", self.base_url, encoded);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to send request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AdminError::MatrixApiError(format!(
                "API returned status {}: {}",
                status, error_text
            )));
        }

        response
            .json()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to parse response: {}", e)))
    }

    /// Sets or removes admin privileges for a user
    ///
    /// # Arguments
    /// * `user_id` - The full Matrix user ID (e.g., "@user:localhost")
    /// * `is_admin` - Whether the user should have admin privileges
    ///
    /// # Returns
    /// Ok(()) on success
    ///
    /// # Errors
    /// Returns `AdminError::MatrixApiError` if the API call fails
    pub async fn set_user_admin(&self, user_id: &str, is_admin: bool) -> Result<(), AdminError> {
        let encoded = urlencoding::encode(user_id);
        let url = format!("{}/_synapse/admin/v2/users/{}", self.base_url, encoded);

        let request_body = serde_json::json!({
            "admin": is_admin
        });

        let response = self
            .client
            .put(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to send request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AdminError::MatrixApiError(format!(
                "API returned status {}: {}",
                status, error_text
            )));
        }

        Ok(())
    }

    /// Changes a user's password via the Matrix Admin API
    ///
    /// # Arguments
    /// * `user_id` - The full Matrix user ID (e.g., "@user:localhost")
    /// * `new_password` - The new password to set
    ///
    /// # Returns
    /// Ok(()) on success
    ///
    /// # Errors
    /// Returns `AdminError::MatrixApiError` if the API call fails
    pub async fn change_password(&self, user_id: &str, new_password: &str) -> Result<(), AdminError> {
        let encoded = urlencoding::encode(user_id);
        let url = format!(
            "{}/_synapse/admin/v1/reset_password/{}",
            self.base_url, encoded
        );

        let request_body = serde_json::json!({
            "new_password": new_password,
            "logout_devices": false
        });

        let response = self
            .client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to send request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AdminError::MatrixApiError(format!(
                "API returned status {}: {}",
                status, error_text
            )));
        }

        Ok(())
    }

    /// Sets an extended attribute on a user
    ///
    /// This method updates user attributes via the Matrix Admin API.
    /// Can be used to set custom flags like force_password_change.
    ///
    /// # Arguments
    /// * `user_id` - The full Matrix user ID (e.g., "@user:localhost")
    /// * `attribute` - The attribute name to set
    /// * `value` - The JSON value to set for the attribute
    ///
    /// # Returns
    /// Ok(()) on success
    ///
    /// # Errors
    /// Returns `AdminError::MatrixApiError` if the API call fails
    pub async fn set_user_attribute(
        &self,
        user_id: &str,
        attribute: &str,
        value: &serde_json::Value,
    ) -> Result<(), AdminError> {
        let encoded = urlencoding::encode(user_id);
        let url = format!("{}/_synapse/admin/v2/users/{}", self.base_url, encoded);

        // Build request body with the attribute
        let mut request_body = serde_json::Map::new();
        request_body.insert(attribute.to_string(), value.clone());

        let response = self
            .client
            .put(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| AdminError::MatrixApiError(format!("Failed to send request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AdminError::MatrixApiError(format!(
                "API returned status {}: {}",
                status, error_text
            )));
        }

        Ok(())
    }
}

/// Response from Matrix Admin API user info endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfoResponse {
    /// Full Matrix user ID
    pub name: String,
    /// Display name of the user
    #[serde(default)]
    pub displayname: Option<String>,
    /// Whether the user has admin privileges
    pub admin: bool,
    /// Whether the user is deactivated
    #[serde(default)]
    pub deactivated: bool,
}

/// Service for creating Matrix admin users (Tier 2)
///
/// This service orchestrates the creation of Matrix admin users by:
/// 1. Verifying the Palpo server is running
/// 2. Validating the password against policy
/// 3. Creating the user via Matrix Admin API
/// 4. Verifying admin status was set correctly
#[derive(Debug)]
pub struct MatrixAdminCreationService {
    /// Matrix Admin API client
    matrix_admin: MatrixAdminClient,
    /// Base URL of the Palpo server
    server_base_url: String,
}

impl MatrixAdminCreationService {
    /// Creates a new Matrix Admin Creation Service
    ///
    /// # Arguments
    /// * `server_base_url` - The base URL of the Palpo server (e.g., "http://localhost:8008")
    pub fn new(server_base_url: String) -> Self {
        let matrix_admin = MatrixAdminClient::new(server_base_url.clone());
        Self {
            matrix_admin,
            server_base_url,
        }
    }

    /// Checks if the Palpo server is running
    ///
    /// Makes a simple HTTP request to the server's version endpoint to verify it's accessible.
    ///
    /// # Returns
    /// `true` if the server is running and responding, `false` otherwise
    async fn is_server_running(&self) -> bool {
        let client = Client::new();
        let url = format!("{}/_matrix/client/versions", self.server_base_url);
        
        match client.get(&url).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
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

        // Check minimum length
        if password.len() < MIN_LENGTH {
            return Err(AdminError::PasswordTooShort(password.len()));
        }

        // Check for uppercase letter
        if !password.chars().any(|c| c.is_uppercase()) {
            return Err(AdminError::MissingUppercase);
        }

        // Check for lowercase letter
        if !password.chars().any(|c| c.is_lowercase()) {
            return Err(AdminError::MissingLowercase);
        }

        // Check for digit
        if !password.chars().any(|c| c.is_ascii_digit()) {
            return Err(AdminError::MissingDigit);
        }

        // Check for special character
        if !password.chars().any(|c| SPECIAL_CHARS.contains(c)) {
            return Err(AdminError::MissingSpecialChar);
        }

        Ok(())
    }

    /// Creates a Matrix admin user
    ///
    /// This is the main entry point for creating Matrix admin users. It performs
    /// all necessary validation and verification steps.
    ///
    /// # Arguments
    /// * `username` - The username for the new admin (without @ or domain)
    /// * `password` - The password for the new admin
    /// * `displayname` - Optional display name for the user
    ///
    /// # Returns
    /// Response containing the created user's credentials
    ///
    /// # Errors
    /// - `AdminError::ServerNotRunning` - If Palpo server is not accessible
    /// - `AdminError::PasswordTooShort` - If password is less than 12 characters
    /// - `AdminError::Missing*` - If password doesn't meet complexity requirements
    /// - `AdminError::AdminStatusNotSet` - If admin flag wasn't set correctly
    /// - `AdminError::MatrixApiError` - If API communication fails
    ///
    /// # Requirements
    /// - 7.1: Verifies Palpo server is running before creation
    /// - 7.2: Returns clear error if server not running
    /// - 7.3: Uses Matrix Admin API endpoint
    /// - 7.4: Sets admin field to 1 (true)
    /// - 7.5: Validates password policy
    /// - 7.6: Returns created username and password
    /// - 7.7: Verifies admin status after creation
    pub async fn create_matrix_admin(
        &self,
        username: &str,
        password: &str,
        displayname: Option<&str>,
    ) -> Result<CreateMatrixAdminResponse, AdminError> {
        // Requirement 7.1 & 7.2: Verify Palpo server is running
        if !self.is_server_running().await {
            return Err(AdminError::ServerNotRunning);
        }

        // Requirement 7.5: Validate password policy
        Self::validate_password_policy(password)?;

        // Requirement 7.3 & 7.4: Create admin user via Matrix Admin API with admin=true
        let user_id = self
            .matrix_admin
            .create_admin_user(username, password, displayname)
            .await?;

        // Requirement 7.7: Verify admin status after creation
        let user_info = self.matrix_admin.get_user(&user_id).await?;
        if !user_info.admin {
            return Err(AdminError::AdminStatusNotSet);
        }

        // Requirement 7.6: Return created username and temporary password
        Ok(CreateMatrixAdminResponse {
            user_id,
            username: username.to_string(),
            password: password.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_policy_valid() {
        let result = MatrixAdminCreationService::validate_password_policy("SecureP@ss123!");
        assert!(result.is_ok());
    }

    #[test]
    fn test_password_policy_too_short() {
        let result = MatrixAdminCreationService::validate_password_policy("Short1!");
        assert!(matches!(result, Err(AdminError::PasswordTooShort(7))));
    }

    #[test]
    fn test_password_policy_missing_uppercase() {
        let result = MatrixAdminCreationService::validate_password_policy("lowercase123!");
        assert!(matches!(result, Err(AdminError::MissingUppercase)));
    }

    #[test]
    fn test_password_policy_missing_lowercase() {
        let result = MatrixAdminCreationService::validate_password_policy("UPPERCASE123!");
        assert!(matches!(result, Err(AdminError::MissingLowercase)));
    }

    #[test]
    fn test_password_policy_missing_digit() {
        let result = MatrixAdminCreationService::validate_password_policy("NoDigitsHere!");
        assert!(matches!(result, Err(AdminError::MissingDigit)));
    }

    #[test]
    fn test_password_policy_missing_special() {
        let result = MatrixAdminCreationService::validate_password_policy("NoSpecialChar123");
        assert!(matches!(result, Err(AdminError::MissingSpecialChar)));
    }
}
