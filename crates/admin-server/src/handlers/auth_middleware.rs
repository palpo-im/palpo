/// Authentication Middleware for User Management API
///
/// This module provides authentication middleware for user management endpoints.
/// All user management endpoints require a valid session token.

use std::sync::Arc;

use salvo::prelude::*;
use crate::types::AdminError;
use crate::SessionManager;
use crate::handlers::user_handler::ErrorResponse;

/// Authentication middleware that validates session tokens
///
/// Extracts the Bearer token from the Authorization header and validates it
/// against the session manager. If valid, adds the username to request extensions.
pub struct AuthMiddleware {
    session_manager: Arc<SessionManager>,
}

impl AuthMiddleware {
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait]
impl Handler for AuthMiddleware {
    async fn handle(&self, req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
        // Skip auth for health check and status endpoints
        let path = req.uri().path();
        if path.ends_with("/health") || path.ends_with("/status") {
            ctrl.call_next(req, depot, res).await;
            return;
        }

        // Extract Authorization header
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

        // Extract Bearer token
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
        match self.session_manager.validate_session(token).await {
            Ok(username) => {
                // Add username to depot for handlers to access
                depot.insert("username", username);
                ctrl.call_next(req, depot, res).await;
            }
            Err(AdminError::SessionExpired) => {
                res.status_code(StatusCode::UNAUTHORIZED);
                res.render(Json(ErrorResponse {
                    error: "Session expired".to_string(),
                }));
            }
            Err(AdminError::InvalidSessionToken) => {
                res.status_code(StatusCode::UNAUTHORIZED);
                res.render(Json(ErrorResponse {
                    error: "Invalid session token".to_string(),
                }));
            }
            Err(e) => {
                tracing::error!("Session validation error: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse {
                    error: "Session validation failed".to_string(),
                }));
            }
        }
    }
}

/// Optional auth middleware - allows unauthenticated access but adds user info if available
pub struct OptionalAuthMiddleware {
    session_manager: Arc<SessionManager>,
}

impl OptionalAuthMiddleware {
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait]
impl Handler for OptionalAuthMiddleware {
    async fn handle(&self, req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
        // Check for Authorization header
        if let Some(auth_header) = req.headers().get("Authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    if let Ok(username) = self.session_manager.validate_session(token).await {
                        depot.insert("username", username);
                    }
                }
            }
        }
        ctrl.call_next(req, depot, res).await;
    }
}

/// Get the authenticated username from depot
pub fn get_authenticated_username(depot: &Depot) -> Option<String> {
    depot.get::<String>("username").ok().map(|v| v.clone())
}

/// Require authentication - returns error response if not authenticated
pub fn require_auth(depot: &Depot, res: &mut Response) -> bool {
    if let Some(_) = get_authenticated_username(depot) {
        true
    } else {
        res.status_code(StatusCode::UNAUTHORIZED);
        res.render(Json(ErrorResponse {
            error: "Authentication required".to_string(),
        }));
        false
    }
}