/// Shadow Ban Handler - HTTP handlers for shadow-ban operations
///
/// This module implements shadow-ban API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
///
/// Endpoints:
/// - GET /api/v1/users/{user_id}/shadow-ban - Get shadow ban status
/// - PUT /api/v1/users/{user_id}/shadow-ban - Set shadow ban status
/// - GET /api/v1/shadow-banned/users - List all shadow-banned users
/// - GET /api/v1/shadow-banned/count - Get shadow-banned user count

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::repositories::DieselShadowBanRepository;
use crate::shadow_ban_repository::ShadowBanRepository;

use super::auth_middleware::require_auth;
use super::validation::{validate_user_id, validate_limit, validate_offset};

// ===== Request Types =====

/// Shadow ban status response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowBanStatusResponse {
    pub user_id: String,
    pub is_shadow_banned: bool,
    pub shadow_banned_at: Option<i64>,
    pub updated_at: i64,
}

/// Set shadow ban request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetShadowBanRequest {
    pub shadow_banned: bool,
}

/// Shadow ban list query
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShadowBanListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Shadow ban list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowBanListResponse {
    pub users: Vec<String>,
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Shadow ban count response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowBanCountResponse {
    pub total_shadow_banned: i64,
}

/// Shadow ban check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowBanCheckResponse {
    pub user_id: String,
    pub is_shadow_banned: bool,
}

/// Generic success response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Standard error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

/// Shadow ban handler state containing the repository
#[derive(Clone, Debug)]
pub struct ShadowBanHandlerState {
    pub shadow_ban_repo: Arc<DieselShadowBanRepository>,
}

impl ShadowBanHandlerState {
    pub fn new(shadow_ban_repo: Arc<DieselShadowBanRepository>) -> Self {
        Self { shadow_ban_repo }
    }
}

/// Global shadow ban handler state
static SHADOW_BAN_HANDLER_STATE: std::sync::OnceLock<ShadowBanHandlerState> = std::sync::OnceLock::new();

/// Initialize the global shadow ban handler state
pub fn init_shadow_ban_handler_state(state: ShadowBanHandlerState) {
    SHADOW_BAN_HANDLER_STATE.set(state).expect("Shadow ban handler state already initialized");
}

/// Get the global shadow ban handler state
fn get_shadow_ban_handler_state() -> &'static ShadowBanHandlerState {
    SHADOW_BAN_HANDLER_STATE.get().expect("Shadow ban handler state not initialized")
}

// ===== Handler Functions =====

/// GET /api/v1/users/{user_id}/shadow-ban - Get shadow ban status
#[handler]
pub async fn get_shadow_ban_status(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_shadow_ban_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.shadow_ban_repo.get_shadow_ban_status(&user_id).await {
        Ok(status) => {
            res.render(Json(ShadowBanStatusResponse {
                user_id: status.user_id,
                is_shadow_banned: status.is_shadow_banned,
                shadow_banned_at: status.shadow_banned_at,
                updated_at: status.updated_at,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get shadow ban status: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get shadow ban status".to_string() }));
        }
    }
}

/// PUT /api/v1/users/{user_id}/shadow-ban - Set shadow ban status
#[handler]
pub async fn set_shadow_banned(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_shadow_ban_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<SetShadowBanRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid shadow ban request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    match state.shadow_ban_repo.set_shadow_banned(&user_id, body.shadow_banned).await {
        Ok(status) => {
            tracing::info!("Set shadow ban for {}: {}", user_id, body.shadow_banned);
            res.render(Json(ShadowBanStatusResponse {
                user_id: status.user_id,
                is_shadow_banned: status.is_shadow_banned,
                shadow_banned_at: status.shadow_banned_at,
                updated_at: status.updated_at,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to set shadow ban: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to set shadow ban".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/shadow-ban/check - Check if user is shadow-banned
#[handler]
pub async fn is_shadow_banned(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_shadow_ban_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.shadow_ban_repo.is_shadow_banned(&user_id).await {
        Ok(is_banned) => {
            res.render(Json(ShadowBanCheckResponse {
                user_id,
                is_shadow_banned: is_banned,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to check shadow ban: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to check shadow ban".to_string() }));
        }
    }
}

/// GET /api/v1/shadow-banned/users - List all shadow-banned users
#[handler]
pub async fn list_shadow_banned_users(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_shadow_ban_handler_state();
    let query = req.parse_queries::<ShadowBanListQuery>().unwrap_or_default();

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

    match state.shadow_ban_repo.get_all_shadow_banned(limit, offset).await {
        Ok(users) => {
            let total = state.shadow_ban_repo.get_shadow_banned_count().await.unwrap_or(0);
            res.render(Json(ShadowBanListResponse {
                users,
                total_count: total,
                limit,
                offset,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to list shadow-banned users: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to list shadow-banned users".to_string() }));
        }
    }
}

/// GET /api/v1/shadow-banned/count - Get shadow-banned user count
#[handler]
pub async fn get_shadow_banned_count(depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_shadow_ban_handler_state();

    match state.shadow_ban_repo.get_shadow_banned_count().await {
        Ok(count) => {
            res.render(Json(ShadowBanCountResponse {
                total_shadow_banned: count,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get shadow-banned count: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get shadow-banned count".to_string() }));
        }
    }
}