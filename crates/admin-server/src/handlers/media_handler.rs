/// Media Handler - HTTP handlers for media management API
///
/// This module implements media management API endpoints:
/// - List user media
/// - Get media count
/// - Get media size
/// - Delete media
/// - Delete all user media
///
/// Note: Actual media files are stored on disk, this tracks metadata.

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::repositories::DieselMediaRepository;
use crate::media_repository::MediaRepository;

/// Media list query parameters
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MediaListQuery {
    pub user_id: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Media list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaListResponse {
    pub media: Vec<MediaResponse>,
    pub total_count: i64,
    pub total_size: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Media response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaResponse {
    pub media_id: String,
    pub user_id: String,
    pub media_type: String,
    pub file_path: String,
    pub file_size: i64,
    pub created_ts: i64,
}

/// Media count response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaCountResponse {
    pub user_id: String,
    pub media_count: i64,
}

/// Media size response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSizeResponse {
    pub user_id: String,
    pub total_size: i64,
}

/// Delete media request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteMediaRequest {
    pub media_ids: Vec<String>,
}

/// Batch delete response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDeleteResponse {
    pub success: bool,
    pub deleted_count: u64,
    pub message: String,
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

/// Media handler state
#[derive(Clone, Debug)]
pub struct MediaHandlerState {
    pub media_repo: Arc<DieselMediaRepository>,
}

impl MediaHandlerState {
    pub fn new(media_repo: Arc<DieselMediaRepository>) -> Self {
        Self { media_repo }
    }
}

/// Global media handler state
static MEDIA_HANDLER_STATE: std::sync::OnceLock<MediaHandlerState> = std::sync::OnceLock::new();

/// Initialize the global media handler state
pub fn init_media_handler_state(state: MediaHandlerState) {
    MEDIA_HANDLER_STATE.set(state).expect("Media handler state already initialized");
}

/// Get the global media handler state
fn get_media_handler_state() -> &'static MediaHandlerState {
    MEDIA_HANDLER_STATE.get().expect("Media handler state not initialized")
}

/// List media for a user
#[handler]
pub async fn list_user_media(req: &mut Request, res: &mut Response) {
    let state = get_media_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    let query = req.parse_queries::<MediaListQuery>().unwrap_or_default();

    let filter = crate::repositories::MediaFilter {
        user_id,
        limit: query.limit,
        offset: query.offset,
    };

    match state.media_repo.get_user_media(&filter).await {
        Ok(result) => {
            let media: Vec<MediaResponse> = result.media.iter().map(MediaResponse::from).collect();
            res.render(Json(MediaListResponse {
                media,
                total_count: result.total_count,
                total_size: result.total_size,
                limit: result.limit,
                offset: result.offset,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to list media: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to list media".to_string() }));
        }
    }
}

/// Get media count for a user
#[handler]
pub async fn get_user_media_count(req: &mut Request, res: &mut Response) {
    let state = get_media_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    match state.media_repo.get_user_media_count(&user_id).await {
        Ok(count) => {
            res.render(Json(MediaCountResponse {
                user_id,
                media_count: count,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get media count: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get media count".to_string() }));
        }
    }
}

/// Get total media size for a user
#[handler]
pub async fn get_user_media_size(req: &mut Request, res: &mut Response) {
    let state = get_media_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    match state.media_repo.get_user_media_size(&user_id).await {
        Ok(size) => {
            res.render(Json(MediaSizeResponse {
                user_id,
                total_size: size,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get media size: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get media size".to_string() }));
        }
    }
}

/// Delete a single media
#[handler]
pub async fn delete_media(req: &mut Request, res: &mut Response) {
    let state = get_media_handler_state();
    let media_id = req.param::<String>("media_id").unwrap_or_default();

    if let Err(e) = state.media_repo.delete_media(&media_id).await {
        tracing::error!("Failed to delete media: {}", e);
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        res.render(Json(ErrorResponse { error: "Failed to delete media".to_string() }));
        return;
    }

    res.render(Json(SuccessResponse {
        success: true,
        message: format!("Media {} deleted successfully", media_id),
    }));
}

/// Delete multiple media
#[handler]
pub async fn delete_media_batch(req: &mut Request, res: &mut Response) {
    let state = get_media_handler_state();
    let body = req.parse_json::<DeleteMediaRequest>().await.unwrap_or(DeleteMediaRequest { media_ids: vec![] });

    let mut deleted_count = 0;
    for media_id in &body.media_ids {
        if state.media_repo.delete_media(media_id).await.is_ok() {
            deleted_count += 1;
        }
    }

    res.render(Json(BatchDeleteResponse {
        success: true,
        deleted_count,
        message: format!("Deleted {} media items", deleted_count),
    }));
}

/// Delete all media for a user
#[handler]
pub async fn delete_all_user_media(req: &mut Request, res: &mut Response) {
    let state = get_media_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    match state.media_repo.delete_all_user_media(&user_id).await {
        Ok(count) => {
            res.render(Json(BatchDeleteResponse {
                success: true,
                deleted_count: count,
                message: format!("Deleted {} media items for user {}", count, user_id),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to delete user media: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to delete user media".to_string() }));
        }
    }
}

// Conversion implementations
impl From<&crate::repositories::MediaMetadata> for MediaResponse {
    fn from(m: &crate::repositories::MediaMetadata) -> Self {
        MediaResponse {
            media_id: m.media_id.clone(),
            user_id: m.user_id.clone(),
            media_type: m.media_type.clone(),
            file_path: m.file_path.clone(),
            file_size: m.file_size,
            created_ts: m.created_ts,
        }
    }
}