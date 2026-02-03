//! Authentication service for admin UI

use crate::models::{
    AdminUser, AuthState, LoginRequest, LoginResponse, LogoutRequest,
    ValidateSessionRequest, ValidateSessionResponse, WebConfigError, WebConfigResult,
};
use serde_json;
use std::time::SystemTime;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{window, Request, RequestInit, RequestMode, Response};

/// Authentication service for handling login, logout, and session management
#[derive(Clone)]
pub struct AuthService {
    base_url: String,
}

impl AuthService {
    /// Create a new authentication service
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    /// Authenticate user with username and password
    pub async fn login(&self, username: String, password: String) -> WebConfigResult<LoginResponse> {
        let request = LoginRequest { username, password };
        
        let url = format!("{}/api/auth/login", self.base_url);
        let response = self.post_json(&url, &request).await?;
        
        let login_response: LoginResponse = self.parse_json_response(response).await?;
        
        // Store token in local storage if login successful
        if login_response.success {
            if let Some(token) = &login_response.token {
                self.store_token(token)?;
            }
        }
        
        Ok(login_response)
    }

    /// Logout current user
    pub async fn logout(&self, session_id: String) -> WebConfigResult<()> {
        let request = LogoutRequest { session_id };
        
        let url = format!("{}/api/auth/logout", self.base_url);
        let _response = self.post_json(&url, &request).await?;
        
        // Clear stored token
        self.clear_token()?;
        
        Ok(())
    }

    /// Validate current session
    pub async fn validate_session(&self) -> WebConfigResult<ValidateSessionResponse> {
        let token = self.get_stored_token()?;
        
        let request = ValidateSessionRequest { token };
        let url = format!("{}/api/auth/validate", self.base_url);
        let response = self.post_json(&url, &request).await?;
        
        let validation_response: ValidateSessionResponse = self.parse_json_response(response).await?;
        
        // Clear token if session is invalid
        if !validation_response.valid {
            self.clear_token()?;
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

    /// Store authentication token in local storage
    fn store_token(&self, token: &str) -> WebConfigResult<()> {
        let window = window().ok_or_else(|| WebConfigError::client("No window object available"))?;
        let storage = window
            .local_storage()
            .map_err(|_| WebConfigError::client("Failed to access local storage"))?
            .ok_or_else(|| WebConfigError::client("Local storage not available"))?;

        storage
            .set_item("auth_token", token)
            .map_err(|_| WebConfigError::client("Failed to store auth token"))?;

        Ok(())
    }

    /// Get stored authentication token from local storage
    fn get_stored_token(&self) -> WebConfigResult<String> {
        let window = window().ok_or_else(|| WebConfigError::client("No window object available"))?;
        let storage = window
            .local_storage()
            .map_err(|_| WebConfigError::client("Failed to access local storage"))?
            .ok_or_else(|| WebConfigError::client("Local storage not available"))?;

        let token = storage
            .get_item("auth_token")
            .map_err(|_| WebConfigError::client("Failed to retrieve auth token"))?
            .ok_or_else(|| WebConfigError::auth("No authentication token found"))?;

        Ok(token)
    }

    /// Clear stored authentication token
    fn clear_token(&self) -> WebConfigResult<()> {
        let window = window().ok_or_else(|| WebConfigError::client("No window object available"))?;
        let storage = window
            .local_storage()
            .map_err(|_| WebConfigError::client("Failed to access local storage"))?
            .ok_or_else(|| WebConfigError::client("Local storage not available"))?;

        storage
            .remove_item("auth_token")
            .map_err(|_| WebConfigError::client("Failed to clear auth token"))?;

        Ok(())
    }

    /// Make a POST request with JSON payload
    async fn post_json<T: serde::Serialize>(&self, url: &str, data: &T) -> WebConfigResult<Response> {
        let json_data = serde_json::to_string(data)
            .map_err(|e| WebConfigError::client(format!("Failed to serialize request: {}", e)))?;

        let mut opts = RequestInit::new();
        opts.set_method("POST");
        opts.set_mode(RequestMode::Cors);
        opts.set_body(Some(&wasm_bindgen::JsValue::from_str(&json_data)));

        let request = Request::new_with_str_and_init(url, &opts)
            .map_err(|_| WebConfigError::client("Failed to create request"))?;

        // Set headers
        request
            .headers()
            .set("Content-Type", "application/json")
            .map_err(|_| WebConfigError::client("Failed to set content type header"))?;

        // Add authorization header if token is available
        if let Ok(token) = self.get_stored_token() {
            request
                .headers()
                .set("Authorization", &format!("Bearer {}", token))
                .map_err(|_| WebConfigError::client("Failed to set authorization header"))?;
        }

        let window = window().ok_or_else(|| WebConfigError::client("No window object available"))?;
        let resp_value = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|_| WebConfigError::network("Network request failed"))?;

        let resp: Response = resp_value
            .dyn_into()
            .map_err(|_| WebConfigError::client("Invalid response type"))?;

        if !resp.ok() {
            return Err(WebConfigError::api_with_status(
                format!("HTTP error: {}", resp.status_text()),
                resp.status(),
            ));
        }

        Ok(resp)
    }

    /// Parse JSON response from HTTP response
    async fn parse_json_response<T: serde::de::DeserializeOwned>(&self, response: Response) -> WebConfigResult<T> {
        let json_promise = response
            .json()
            .map_err(|_| WebConfigError::client("Failed to get JSON from response"))?;

        let json_value = JsFuture::from(json_promise)
            .await
            .map_err(|_| WebConfigError::client("Failed to parse JSON response"))?;

        let json_string = js_sys::JSON::stringify(&json_value)
            .map_err(|_| WebConfigError::client("Failed to stringify JSON"))?;

        let json_str = json_string
            .as_string()
            .ok_or_else(|| WebConfigError::client("Invalid JSON string"))?;

        serde_json::from_str(&json_str)
            .map_err(|e| WebConfigError::client(format!("Failed to deserialize JSON: {}", e)))
    }
}

/// Default authentication service instance
impl Default for AuthService {
    fn default() -> Self {
        Self::new("http://localhost:8008")
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
    pub async fn get_session_expiry(&self) -> Option<SystemTime> {
        self.auth_service
            .get_current_user()
            .await
            .map(|user| user.expires_at)
    }
}