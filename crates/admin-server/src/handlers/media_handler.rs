/// Media Handler - HTTP handlers for media management API
///
/// This module implements media management API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
/// Uses PalpoClient to communicate with Palpo Matrix server via HTTP API.
///
/// Endpoints:
/// - GET /api/v1/users/{user_id}/media - List user media
/// - GET /api/v1/users/{user_id}/media/count - Get media count
/// - GET /api/v1/users/{user_id}/media/size - Get total media size
/// - DELETE /api/v1/users/{user_id}/media - Delete all user media

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

use crate::palpo_client::{PalpoClient, PalpoMedia};

use super::auth_middleware::require_auth;
use super::validation::validate_user_id;

// ===== Request Types =====

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MediaListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaListResponse {
    pub media: Vec<MediaResponse>,
    pub total_count: i64,
    pub total_size: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaResponse {
    pub media_id: String,
    pub user_id: String,
    pub media_type: Option<String>,
    pub upload_name: Option<String>,
    pub file_size: i64,
    pub created_ts: Option<i64>,
}

impl MediaResponse {
    pub fn from_palpo_media(media: &PalpoMedia, user_id: &str) -> Self {
        Self {
            media_id: media.media_id.clone(),
            user_id: user_id.to_string(),
            media_type: media.media_type.clone(),
            upload_name: media.upload_name.clone(),
            file_size: media.size.unwrap_or(0),
            created_ts: media.created_ts,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaCountResponse {
    pub user_id: String,
    pub media_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSizeResponse {
    pub user_id: String,
    pub total_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteMediaRequest {
    pub media_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDeleteResponse {
    pub success: bool,
    pub deleted_count: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

#[derive(Clone, Debug)]
pub struct MediaHandlerState {
    pub palpo_client: Arc<PalpoClient>,
}

impl MediaHandlerState {
    pub fn new(palpo_client: Arc<PalpoClient>) -> Self {
        Self { palpo_client }
    }
}

static MEDIA_HANDLER_STATE: OnceLock<MediaHandlerState> = OnceLock::new();

pub fn init_media_handler_state(state: MediaHandlerState) {
    MEDIA_HANDLER_STATE.set(state).expect("Media handler state already initialized");
}

fn get_media_handler_state() -> &'static MediaHandlerState {
    MEDIA_HANDLER_STATE.get().expect("Media handler state not initialized")
}

// ===== Handler Functions =====

/// GET /api/v1/users/{user_id}/media - List user media
#[handler]
pub async fn list_user_media(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_media_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let query = req.parse_queries::<MediaListQuery>().unwrap_or_default();
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);

    match state.palpo_client.list_user_media(&user_id).await {
        Ok(result) => {
            let media: Vec<MediaResponse> = result.media.iter()
                .map(|m| MediaResponse::from_palpo_media(&m, &user_id))
                .collect();
            let total_size: i64 = result.media.iter().map(|m| m.size.unwrap_or(0)).sum();

            res.render(Json(MediaListResponse {
                media,
                total_count: result.total,
                total_size,
                limit,
                offset,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to list media: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to list media".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/media/count - Get media count
#[handler]
pub async fn get_user_media_count(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_media_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.list_user_media(&user_id).await {
        Ok(result) => {
            res.render(Json(MediaCountResponse {
                user_id,
                media_count: result.total,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get media count: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get media count".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/media/size - Get total media size
#[handler]
pub async fn get_user_media_size(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_media_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.list_user_media(&user_id).await {
        Ok(result) => {
            let total_size: i64 = result.media.iter().map(|m| m.size.unwrap_or(0)).sum();
            res.render(Json(MediaSizeResponse {
                user_id,
                total_size,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get media size: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get media size".to_string() }));
        }
    }
}

/// DELETE /api/v1/users/{user_id}/media - Delete all user media
#[handler]
pub async fn delete_all_user_media(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_media_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.delete_user_media(&user_id).await {
        Ok(result) => {
            tracing::info!("Deleted {} media items for user {}", result.total_deleted, user_id);
            res.render(Json(BatchDeleteResponse {
                success: true,
                deleted_count: result.total_deleted as u64,
                message: format!("Deleted {} media items", result.total_deleted),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to delete user media: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to delete user media".to_string() }));
        }
    }
}