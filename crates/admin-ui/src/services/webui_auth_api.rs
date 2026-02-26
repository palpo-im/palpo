/// Web UI Admin Authentication API Client
///
/// This module provides client-side API calls for Web UI admin authentication.
/// The Web UI admin is the first tier of the two-tier admin system, using
/// PostgreSQL database authentication with a fixed username "admin".

use crate::models::error::WebConfigResult;
use crate::services::api_client::{get_api_client, RequestConfig, HttpMethod};
use serde::{Deserialize, Serialize};

/// Status response for checking if setup is needed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStatusResponse {
    pub needs_setup: bool,
    pub has_legacy_credentials: bool,
}

/// Request to create Web UI admin account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupRequest {
    pub password: String,
}

/// Response after successful setup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupResponse {
    pub success: bool,
    pub message: String,
}

/// Request to migrate from localStorage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateRequest {
    pub password: String,
}

/// Response after successful migration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateResponse {
    pub success: bool,
    pub message: String,
}

/// Request to login
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Response after successful login
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub success: bool,
    pub token: String,
    pub message: Option<String>,
}

/// Request to change password
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
    pub confirm_password: String,
}

/// Response after successful password change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordResponse {
    pub success: bool,
    pub message: String,
}

/// Web UI Auth API client
pub struct WebUIAuthAPI;

impl WebUIAuthAPI {
    /// Check if setup is needed and detect legacy credentials
    pub async fn get_status() -> WebConfigResult<SetupStatusResponse> {
        let client = get_api_client()?;
        let config = RequestConfig::new(HttpMethod::Get, "/api/v1/admin/webui-admin/status")
            .without_auth();
        
        let response = client.execute_request(config).await?;
        client.parse_json(response).await
    }

    /// Create Web UI admin account (initial setup)
    pub async fn setup(password: String) -> WebConfigResult<SetupResponse> {
        let client = get_api_client()?;
        let request = SetupRequest { password };
        
        let config = RequestConfig::new(HttpMethod::Post, "/api/v1/admin/webui-admin/setup")
            .with_json_body(&request)?
            .without_auth();
        
        let response = client.execute_request(config).await?;
        client.parse_json(response).await
    }

    /// Migrate from localStorage to database
    pub async fn migrate(password: String) -> WebConfigResult<MigrateResponse> {
        let client = get_api_client()?;
        let request = MigrateRequest { password };
        
        let config = RequestConfig::new(HttpMethod::Post, "/api/v1/admin/webui-admin/migrate")
            .with_json_body(&request)?
            .without_auth();
        
        let response = client.execute_request(config).await?;
        client.parse_json(response).await
    }

    /// Login with Web UI admin credentials
    pub async fn login(username: String, password: String) -> WebConfigResult<LoginResponse> {
        let client = get_api_client()?;
        let request = LoginRequest { username, password };
        
        let config = RequestConfig::new(HttpMethod::Post, "/api/v1/admin/webui-admin/login")
            .with_json_body(&request)?
            .without_auth();
        
        let response = client.execute_request(config).await?;
        client.parse_json(response).await
    }

    /// Change Web UI admin password
    pub async fn change_password(
        current_password: String,
        new_password: String,
        confirm_password: String,
    ) -> WebConfigResult<ChangePasswordResponse> {
        let client = get_api_client()?;
        let request = ChangePasswordRequest {
            current_password,
            new_password,
            confirm_password,
        };
        
        let config = RequestConfig::new(HttpMethod::Post, "/api/v1/admin/webui-admin/change-password")
            .with_json_body(&request)?;
        
        let response = client.execute_request(config).await?;
        client.parse_json(response).await
    }
}
