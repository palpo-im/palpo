/// Session Handler - HTTP handlers for session and whois API
///
/// This module implements session-related API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
///
/// Endpoints:
/// - GET /api/v1/users/{user_id}/whois - Get whois information
/// - GET /api/v1/users/{user_id}/sessions - List user sessions
/// - GET /api/v1/users/{user_id}/sessions/count - Get session count
/// - GET /api/v1/users/{user_id}/last-seen - Get last seen information
/// - DELETE /api/v1/users/{user_id}/sessions - Delete all user sessions

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::types::AdminError;
use crate::repositories::{SessionRepository, SessionFilter, SessionInfo, WhoisInfo};

use super::auth_middleware::require_auth;
use super::validation::{validate_user_id, validate_limit, validate_offset, ValidationError};

// ===== Request Types =====

/// Session list query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Session list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionResponse>,
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Session response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub ip: String,
    pub last_seen: i64,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub user_agent: Option<String>,
}

/// Whois response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoisResponse {
    pub user_id: String,
    pub sessions: Vec<SessionResponse>,
    pub total_session_count: i64,
    pub primary_device_id: Option<String>,
}

/// Last seen response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastSeenResponse {
    pub user_id: String,
    pub last_seen_ts: Option<i64>,
    pub last_seen_ip: Option<String>,
}

/// Session count response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCountResponse {
    pub user_id: String,
    pub ip_count: i64,
}

/// Generic success response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Batch delete response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDeleteResponse {
    pub success: bool,
    pub deleted_count: u64,
    pub message: String,
}

/// Standard error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

/// Session handler state containing the repository
#[derive(Clone)]
pub struct SessionHandlerState {
    pub session_repo: Arc<dyn SessionRepository>,
}

impl SessionHandlerState {
    pub fn new(session_repo: Arc<dyn SessionRepository>) -> Self {
        Self { session_repo }
    }
}

/// Global session handler state
static SESSION_HANDLER_STATE: std::sync::OnceLock<SessionHandlerState> = std::sync::OnceLock::new();

/// Initialize the global session handler state
pub fn init_session_handler_state(state: SessionHandlerState) {
    SESSION_HANDLER_STATE.set(state).expect("Session handler state already initialized");
}

/// Get the global session handler state
fn get_session_handler_state() -> &'static SessionHandlerState {
    SESSION_HANDLER_STATE.get().expect("Session handler state not initialized")
}

// ===== Handler Functions =====

/// GET /api/v1/users/{user_id}/whois - Get whois information
#[handler]
pub async fn get_whois(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.session_repo.get_whois(&user_id).await {
        Ok(whois) => {
            let sessions: Vec<SessionResponse> = whois.sessions.iter().map(SessionResponse::from).collect();
            res.render(Json(WhoisResponse {
                user_id: whois.user_id,
                sessions,
                total_session_count: whois.total_session_count,
                primary_device_id: whois.primary_device_id,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get whois: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get whois information".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/sessions - List user sessions
#[handler]
pub async fn list_sessions(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let query = req.parse_query::<SessionListQuery>().unwrap_or_default();

    // Validate pagination parameters
    let limit = match validate_limit(query.limit) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("Invalid limit parameter: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: format!("Invalid limit: {}", e) }));
            return;
        }
    };

    let offset = match validate_offset(query.offset) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("Invalid offset parameter: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: format!("Invalid offset: {}", e) }));
            return;
        }
    };

    let filter = SessionFilter {
        user_id: user_id.clone(),
        limit: Some(limit),
        offset: Some(offset),
    };

    match state.session_repo.list_sessions(&filter).await {
        Ok(result) => {
            let sessions: Vec<SessionResponse> = result.sessions.iter().map(SessionResponse::from).collect();
            res.render(Json(SessionListResponse {
                sessions,
                total_count: result.total_count,
                limit,
                offset,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to list sessions: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to list sessions".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/sessions/count - Get session count
#[handler]
pub async fn get_session_count(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.session_repo.get_user_ip_count(&user_id).await {
        Ok(count) => {
            res.render(Json(SessionCountResponse {
                user_id,
                ip_count: count,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get session count: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get session count".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/last-seen - Get last seen information
#[handler]
pub async fn get_last_seen(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.session_repo.get_last_seen(&user_id).await {
        Ok(last_seen_ts) => {
            let last_seen_ip = state.session_repo.get_last_seen_ip(&user_id).await.unwrap_or(None);
            res.render(Json(LastSeenResponse {
                user_id,
                last_seen_ts,
                last_seen_ip,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get last seen: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get last seen information".to_string() }));
        }
    }
}

/// DELETE /api/v1/users/{user_id}/sessions - Delete all user sessions
#[handler]
pub async fn delete_user_sessions(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.session_repo.delete_user_sessions(&user_id).await {
        Ok(count) => {
            tracing::info!("Deleted {} sessions for user {}", count, user_id);
            res.render(Json(BatchDeleteResponse {
                success: true,
                deleted_count: count,
                message: format!("Deleted {} sessions for user {}", count, user_id),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to delete sessions: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to delete sessions".to_string() }));
        }
    }
}

// ===== Conversion Implementations =====

impl From<&SessionInfo> for SessionResponse {
    fn from(session: &SessionInfo) -> Self {
        SessionResponse {
            ip: session.ip.clone(),
            last_seen: session.last_seen,
            device_id: session.device_id.clone(),
            device_name: session.device_name.clone(),
            user_agent: session.user_agent.clone(),
        }
    }
}