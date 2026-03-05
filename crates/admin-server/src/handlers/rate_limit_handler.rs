/// Rate Limit Handler - HTTP handlers for rate limit configuration API
///
/// This module implements rate limit configuration API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
///
/// Endpoints:
/// - GET /api/v1/users/{user_id}/rate-limit - Get rate limit config
/// - POST /api/v1/users/{user_id}/rate-limit - Set rate limit config
/// - DELETE /api/v1/users/{user_id}/rate-limit - Delete rate limit config

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::types::AdminError;
use crate::repositories::{RateLimitRepository, UpdateRateLimitInput};

use super::auth_middleware::require_auth;
use super::validation::{validate_user_id, validate_rate_limit_params, ValidationError};

// ===== Request Types =====

/// Rate limit config response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfigResponse {
    pub user_id: String,
    pub messages_per_second: i32,
    pub burst_count: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Set rate limit request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetRateLimitRequest {
    pub messages_per_second: i32,
    pub burst_count: i32,
}

/// Generic success response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Custom rate limit check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRateLimitResponse {
    pub user_id: String,
    pub has_custom_rate_limit: bool,
}

/// Standard error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

/// Rate limit handler state containing the repository
#[derive(Clone)]
pub struct RateLimitHandlerState {
    pub rate_limit_repo: Arc<dyn RateLimitRepository>,
}

impl RateLimitHandlerState {
    pub fn new(rate_limit_repo: Arc<dyn RateLimitRepository>) -> Self {
        Self { rate_limit_repo }
    }
}

/// Global rate limit handler state
static RATE_LIMIT_HANDLER_STATE: std::sync::OnceLock<RateLimitHandlerState> = std::sync::OnceLock::new();

/// Initialize the global rate limit handler state
pub fn init_rate_limit_handler_state(state: RateLimitHandlerState) {
    RATE_LIMIT_HANDLER_STATE.set(state).expect("Rate limit handler state already initialized");
}

/// Get the global rate limit handler state
fn get_rate_limit_handler_state() -> &'static RateLimitHandlerState {
    RATE_LIMIT_HANDLER_STATE.get().expect("Rate limit handler state not initialized")
}

// ===== Handler Functions =====

/// GET /api/v1/users/{user_id}/rate-limit - Get rate limit config
#[handler]
pub async fn get_rate_limit(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_rate_limit_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.rate_limit_repo.get_rate_limit(&user_id).await {
        Ok(Some(config)) => {
            res.render(Json(RateLimitConfigResponse {
                user_id: config.user_id,
                messages_per_second: config.messages_per_second,
                burst_count: config.burst_count,
                created_at: config.created_at,
                updated_at: config.updated_at,
            }));
        }
        Ok(None) => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(SuccessResponse {
                success: false,
                message: format!("No custom rate limit config for user {}", user_id),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get rate limit: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get rate limit".to_string() }));
        }
    }
}

/// POST /api/v1/users/{user_id}/rate-limit - Set rate limit config
#[handler]
pub async fn set_rate_limit(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_rate_limit_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<SetRateLimitRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid rate limit request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    // Validate rate limit parameters
    if let Err(e) = validate_rate_limit_params(Some(body.messages_per_second as i64), Some(body.burst_count as i64)) {
        tracing::warn!("Invalid rate limit parameters: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid rate limit: {}", e) }));
        return;
    }

    let input = UpdateRateLimitInput {
        messages_per_second: body.messages_per_second,
        burst_count: body.burst_count,
    };

    match state.rate_limit_repo.set_rate_limit(&user_id, &input).await {
        Ok(config) => {
            tracing::info!("Set rate limit for user {}: {}/{}",
                user_id, body.messages_per_second, body.burst_count);
            res.render(Json(RateLimitConfigResponse {
                user_id: config.user_id,
                messages_per_second: config.messages_per_second,
                burst_count: config.burst_count,
                created_at: config.created_at,
                updated_at: config.updated_at,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to set rate limit: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to set rate limit".to_string() }));
        }
    }
}

/// DELETE /api/v1/users/{user_id}/rate-limit - Delete rate limit config
#[handler]
pub async fn delete_rate_limit(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_rate_limit_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    if let Err(e) = state.rate_limit_repo.delete_rate_limit(&user_id).await {
        tracing::error!("Failed to delete rate limit: {}", e);
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        res.render(Json(ErrorResponse { error: "Failed to delete rate limit".to_string() }));
        return;
    }

    tracing::info!("Deleted rate limit config for user {}", user_id);
    res.render(Json(SuccessResponse {
        success: true,
        message: format!("Rate limit config deleted for user {}", user_id),
    }));
}

/// GET /api/v1/users/{user_id}/rate-limit/custom - Check if user has custom rate limit
#[handler]
pub async fn has_custom_rate_limit(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_rate_limit_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.rate_limit_repo.has_custom_rate_limit(&user_id).await {
        Ok(has_custom) => {
            res.render(Json(CustomRateLimitResponse {
                user_id,
                has_custom_rate_limit: has_custom,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to check custom rate limit: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to check custom rate limit".to_string() }));
        }
    }
}