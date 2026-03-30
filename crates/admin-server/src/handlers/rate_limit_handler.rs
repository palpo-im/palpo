/// Rate Limit Handler - HTTP handlers for rate limit configuration API
///
/// This module implements rate limit configuration API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
/// Uses PalpoClient to communicate with Palpo Matrix server via HTTP API.
///
/// Endpoints:
/// - GET /api/v1/users/{user_id}/rate-limit - Get rate limit config
/// - POST /api/v1/users/{user_id}/rate-limit - Set rate limit config
/// - DELETE /api/v1/users/{user_id}/rate-limit - Delete rate limit config

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

use crate::palpo_client::{PalpoClient, PalpoRateLimitConfig};

use super::auth_middleware::require_auth;
use super::validation::{validate_user_id, validate_rate_limit_params};

// ===== Request Types =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfigResponse {
    pub user_id: String,
    pub messages_per_second: i64,
    pub burst_count: i64,
}

impl From<PalpoRateLimitConfig> for RateLimitConfigResponse {
    fn from(config: PalpoRateLimitConfig) -> Self {
        Self {
            user_id: String::new(), // Will be set by caller
            messages_per_second: config.messages_per_second.unwrap_or(0),
            burst_count: config.burst_count.unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetRateLimitRequest {
    pub messages_per_second: i32,
    pub burst_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRateLimitResponse {
    pub user_id: String,
    pub has_custom_rate_limit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

#[derive(Clone, Debug)]
pub struct RateLimitHandlerState {
    pub palpo_client: Arc<PalpoClient>,
}

impl RateLimitHandlerState {
    pub fn new(palpo_client: Arc<PalpoClient>) -> Self {
        Self { palpo_client }
    }
}

static RATE_LIMIT_HANDLER_STATE: OnceLock<RateLimitHandlerState> = OnceLock::new();

pub fn init_rate_limit_handler_state(state: RateLimitHandlerState) {
    RATE_LIMIT_HANDLER_STATE.set(state).expect("Rate limit handler state already initialized");
}

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

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_user_rate_limit(&user_id).await {
        Ok(Some(config)) => {
            res.render(Json(RateLimitConfigResponse {
                user_id,
                messages_per_second: config.messages_per_second.unwrap_or(0),
                burst_count: config.burst_count.unwrap_or(0),
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

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<SetRateLimitRequest>().await {
        Ok(b) => b,
        Err(e) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    if let Err(e) = validate_rate_limit_params(Some(body.messages_per_second as i64), Some(body.burst_count as i64)) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid rate limit: {}", e) }));
        return;
    }

    let config = PalpoRateLimitConfig {
        messages_per_second: Some(body.messages_per_second as i64),
        burst_count: Some(body.burst_count as i64),
    };

    match state.palpo_client.set_user_rate_limit(&user_id, &config).await {
        Ok(config) => {
            tracing::info!("Set rate limit for user {}: {}/{}", user_id, body.messages_per_second, body.burst_count);
            res.render(Json(RateLimitConfigResponse {
                user_id,
                messages_per_second: config.messages_per_second.unwrap_or(0),
                burst_count: config.burst_count.unwrap_or(0),
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

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.delete_user_rate_limit(&user_id).await {
        Ok(_) => {
            tracing::info!("Deleted rate limit config for user {}", user_id);
            res.render(Json(SuccessResponse {
                success: true,
                message: format!("Rate limit config deleted for user {}", user_id),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to delete rate limit: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to delete rate limit".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/rate-limit/custom - Check if user has custom rate limit
#[handler]
pub async fn has_custom_rate_limit(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_rate_limit_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_user_rate_limit(&user_id).await {
        Ok(config) => {
            res.render(Json(CustomRateLimitResponse {
                user_id,
                has_custom_rate_limit: config.is_some(),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to check custom rate limit: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to check custom rate limit".to_string() }));
        }
    }
}