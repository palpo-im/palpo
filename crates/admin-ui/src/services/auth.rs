//! Authentication service for admin UI

use crate::models::{
    AdminUser, AuthState, LoginRequest, LoginResponse, LogoutRequest,
    ValidateSessionRequest, ValidateSessionResponse, WebConfigError, WebConfigResult,
};
use crate::services::api_client::{get_api_client, ApiClient};

/// Authentication service for handling login, logout, and session management
#[derive(Clone)]
pub struct AuthService {
    api_client: ApiClient,
}

impl AuthService {
    /// Create a new authentication service
    pub fn new(api_client: ApiClient) -> Self {
        Self { api_client }
    }

    /// Create authentication service using global API client
    pub fn from_global() -> WebConfigResult<Self> {
        let api_client = get_api_client()?;
        Ok(Self { api_client })
    }

    /// Authenticate user with username and password
    pub async fn login(&self, username: String, password: String) -> WebConfigResult<LoginResponse> {
        let request = LoginRequest { username, password };
        
        // Construct full URL with base URL
        let url = format!("{}/api/v1/admin/webui-admin/login", self.api_client.base_url());
        
        // Use API client without auth for login
        let mut config = crate::services::api_client::RequestConfig::new(
            crate::services::api_client::HttpMethod::Post,
            url
        ).without_auth();
        config = config.with_json_body(&request)?;
        
        let response = self.api_client.execute_request(config).await?;
        let login_response: LoginResponse = self.api_client.parse_json(response).await?;
        
        // Store token in API client if login successful
        if login_response.success {
            if let Some(token) = &login_response.token {
                self.api_client.set_token(token)?;
            }
        }
        
        Ok(login_response)
    }

    /// Logout current user
    pub async fn logout(&self, session_id: String) -> WebConfigResult<()> {
        let request = LogoutRequest { session_id };
        
        let _response = self.api_client.post_json("/api/auth/logout", &request).await?;
        
        // Clear stored token
        self.api_client.clear_token()?;
        
        Ok(())
    }

    /// Validate current session
    pub async fn validate_session(&self) -> WebConfigResult<ValidateSessionResponse> {
        let token = self.api_client.get_token()?
            .ok_or_else(|| WebConfigError::auth("No authentication token found"))?;
        
        let request = ValidateSessionRequest { token };
        let validation_response: ValidateSessionResponse = self.api_client
            .post_json_response("/api/auth/validate", &request).await?;
        
        // Clear token if session is invalid
        if !validation_response.valid {
            self.api_client.clear_token()?;
        }
        
        Ok(validation_response)
    }

    /// Get current authentication state
    pub async fn get_auth_state(&self) -> AuthState {
        match self.validate_session().await {
            Ok(response) => {
                if response.valid {
                    if let Some(user) = response.user {
                        AuthState::Authenticated(user)
                    } else {
                        AuthState::Unauthenticated
                    }
                } else {
                    AuthState::Unauthenticated
                }
            }
            Err(error) => {
                // Log error but don't expose it to prevent information leakage
                web_sys::console::log_1(&format!("Auth validation error: {}", error).into());
                AuthState::Unauthenticated
            }
        }
    }

    /// Check if user is currently authenticated
    pub async fn is_authenticated(&self) -> bool {
        matches!(self.get_auth_state().await, AuthState::Authenticated(_))
    }

    /// Get current user if authenticated
    pub async fn get_current_user(&self) -> Option<AdminUser> {
        match self.get_auth_state().await {
            AuthState::Authenticated(user) => Some(user),
            _ => None,
        }
    }
}

/// Default authentication service instance
impl Default for AuthService {
    fn default() -> Self {
        // Try to use global API client, fallback to creating a new one
        match Self::from_global() {
            Ok(service) => service,
            Err(_) => {
                // Create a default API client if global one is not available
                let api_client = crate::services::api_client::ApiClient::default();
                Self::new(api_client)
            }
        }
    }
}

/// Authentication middleware for protecting routes
pub struct AuthMiddleware;

impl AuthMiddleware {
    /// Check if user is authenticated and has admin privileges
    pub async fn require_admin(auth_service: &AuthService) -> WebConfigResult<AdminUser> {
        match auth_service.get_auth_state().await {
            AuthState::Authenticated(user) => {
                if user.is_admin && user.is_session_valid() {
                    Ok(user)
                } else if !user.is_admin {
                    Err(WebConfigError::permission("Admin privileges required"))
                } else {
                    Err(WebConfigError::auth("Session expired"))
                }
            }
            AuthState::Authenticating => {
                Err(WebConfigError::auth("Authentication in progress"))
            }
            AuthState::Failed(error) => {
                Err(WebConfigError::auth(format!("Authentication failed: {}", error)))
            }
            AuthState::Unauthenticated => {
                Err(WebConfigError::auth("Authentication required"))
            }
        }
    }

    /// Check if user has specific permission
    pub async fn require_permission(
        auth_service: &AuthService,
        permission: crate::models::Permission,
    ) -> WebConfigResult<AdminUser> {
        let user = Self::require_admin(auth_service).await?;
        
        if user.has_permission(&permission) {
            Ok(user)
        } else {
            Err(WebConfigError::permission(format!(
                "Permission required: {}",
                permission.description()
            )))
        }
    }

    /// Validate session and return user if valid
    pub async fn validate_session(auth_service: &AuthService) -> WebConfigResult<Option<AdminUser>> {
        match auth_service.get_auth_state().await {
            AuthState::Authenticated(user) => {
                if user.is_session_valid() {
                    Ok(Some(user))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }
}

/// Session manager for handling session timeouts and renewals
pub struct SessionManager {
    auth_service: AuthService,
    #[allow(dead_code)]
    check_interval: u32, // in milliseconds
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(auth_service: AuthService, check_interval: u32) -> Self {
        Self {
            auth_service,
            check_interval,
        }
    }

    /// Start session monitoring
    pub async fn start_monitoring(&self) -> WebConfigResult<()> {
        // This would typically use a timer to periodically check session validity
        // For now, we'll just validate the current session
        let _validation = self.auth_service.validate_session().await?;
        Ok(())
    }

    /// Check if session needs renewal
    pub async fn needs_renewal(&self) -> bool {
        if let Some(user) = self.auth_service.get_current_user().await {
            if let Some(remaining) = user.remaining_session_time() {
                // Renew if less than 5 minutes remaining
                remaining < 300
            } else {
                true // Session expired
            }
        } else {
            false // Not authenticated
        }
    }

    /// Get session expiry timestamp
    pub async fn get_session_expiry(&self) -> Option<String> {
        self.auth_service
            .get_current_user()
            .await
            .map(|user| user.expires_at)
    }
}