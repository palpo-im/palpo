//! Authentication hook for Dioxus frontend

use dioxus::prelude::*;
use crate::models::{AdminUser, AuthState, Permission};
use crate::services::AuthService;
use wasm_bindgen_futures::spawn_local;

/// Authentication context and methods
#[derive(Clone)]
pub struct AuthContext {
    pub auth_state: Signal<AuthState>,
    pub auth_service: AuthService,
}

impl AuthContext {
    /// Login with username and password
    pub fn login(&self, username: String, password: String) {
        let auth_service = self.auth_service.clone();
        let mut auth_state = self.auth_state;
        
        spawn_local(async move {
            auth_state.set(AuthState::Authenticating);
            
            match auth_service.login(username, password).await {
                Ok(response) => {
                    if response.success {
                        if let Some(user) = response.user {
                            auth_state.set(AuthState::Authenticated(user));
                        } else {
                            auth_state.set(AuthState::Failed("No user data received".to_string()));
                        }
                    } else {
                        let error = response.error.unwrap_or_else(|| "Login failed".to_string());
                        auth_state.set(AuthState::Failed(error));
                    }
                }
                Err(error) => {
                    auth_state.set(AuthState::Failed(error.user_message()));
                }
            }
        });
    }

    /// Logout current user
    pub fn logout(&self) {
        let auth_service = self.auth_service.clone();
        let mut auth_state = self.auth_state;
        
        spawn_local(async move {
            if let AuthState::Authenticated(user) = &*auth_state.read() {
                let session_id = user.session_id.clone();
                let _ = auth_service.logout(session_id).await;
            }
            auth_state.set(AuthState::Unauthenticated);
        });
    }

    /// Validate current session
    pub fn validate_session(&self) {
        let auth_service = self.auth_service.clone();
        let mut auth_state = self.auth_state;
        
        spawn_local(async move {
            match auth_service.validate_session().await {
                Ok(response) => {
                    if response.valid {
                        if let Some(user) = response.user {
                            auth_state.set(AuthState::Authenticated(user));
                        }
                    } else {
                        auth_state.set(AuthState::Unauthenticated);
                    }
                }
                Err(_) => {
                    auth_state.set(AuthState::Unauthenticated);
                }
            }
        });
    }

    /// Check if user is authenticated
    pub fn is_authenticated(&self) -> bool {
        self.auth_state.read().is_authenticated()
    }

    /// Get current user
    pub fn current_user(&self) -> Option<AdminUser> {
        self.auth_state.read().user().cloned()
    }

    /// Check if user has specific permission
    pub fn has_permission(&self, permission: Permission) -> bool {
        if let Some(user) = self.current_user() {
            user.has_permission(&permission)
        } else {
            false
        }
    }

    /// Check if authentication is in progress
    pub fn is_authenticating(&self) -> bool {
        self.auth_state.read().is_authenticating()
    }

    /// Get authentication error if any
    pub fn auth_error(&self) -> Option<String> {
        self.auth_state.read().error().cloned()
    }
}

/// Hook for authentication management in Dioxus components
pub fn use_auth() -> AuthContext {
    // Get the auth state from context
    let auth_state = use_context::<Signal<AuthState>>();
    let auth_service = AuthService::default();

    // Validate session on component mount
    use_effect({
        let auth_service = auth_service.clone();
        let mut auth_state = auth_state;
        
        move || {
            let auth_service = auth_service.clone();
            spawn_local(async move {
                match auth_service.validate_session().await {
                    Ok(response) => {
                        if response.valid {
                            if let Some(user) = response.user {
                                auth_state.set(AuthState::Authenticated(user));
                            }
                        }
                    }
                    Err(_) => {
                        // Silently fail - user is just not authenticated
                    }
                }
            });
        }
    });

    AuthContext {
        auth_state,
        auth_service,
    }
}

/// Hook for requiring authentication - redirects to login if not authenticated
pub fn use_require_auth() -> Option<AdminUser> {
    let auth_context = use_auth();
    
    let auth_state = auth_context.auth_state.read();
    match &*auth_state {
        AuthState::Authenticated(user) => {
            if user.is_session_valid() {
                Some(user.clone())
            } else {
                // Session expired, logout
                drop(auth_state); // Release the read lock
                auth_context.logout();
                None
            }
        }
        AuthState::Unauthenticated => None,
        AuthState::Authenticating => None,
        AuthState::Failed(_) => None,
    }
}

/// Hook for requiring specific permission
pub fn use_require_permission(permission: Permission) -> Option<AdminUser> {
    let user = use_require_auth()?;
    
    if user.has_permission(&permission) {
        Some(user)
    } else {
        None
    }
}

/// Hook for session monitoring and auto-logout
pub fn use_session_monitor() {
    let auth_context = use_auth();
    
    use_effect(move || {
        let auth_context = auth_context.clone();
        
        spawn_local(async move {
            loop {
                // Check session every 30 seconds
                let timeout = js_sys::Promise::new(&mut |resolve, _| {
                    web_sys::window()
                        .unwrap()
                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 30000)
                        .unwrap();
                });
                wasm_bindgen_futures::JsFuture::from(timeout).await.unwrap();
                
                if let Some(user) = auth_context.current_user() {
                    if !user.is_session_valid() {
                        // Session expired, logout
                        auth_context.logout();
                        break;
                    }
                } else {
                    // Not authenticated, stop monitoring
                    break;
                }
            }
        });
    });
}