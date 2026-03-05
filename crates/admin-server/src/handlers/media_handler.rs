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

use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};

use crate::types::AdminError;
use crate::repositories::{MediaRepository, MediaFilter};

/// Media list query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Media handler configuration
pub struct MediaHandler<T: MediaRepository> {
    media_repo: T,
}

impl<T: MediaRepository> MediaHandler<T> {
    /// Create a new handler with the given repository
    pub fn new(media_repo: T) -> Self {
        Self { media_repo }
    }

    /// List media for a user
    pub async fn list_user_media(&self, query: web::Query<MediaListQuery>) -> Result<HttpResponse, AdminError> {
        let filter = MediaFilter {
            user_id: query.user_id.clone(),
            limit: query.limit,
            offset: query.offset,
        };

        let result = self.media_repo.get_user_media(&filter).await?;

        let media: Vec<MediaResponse> = result.media.iter().map(MediaResponse::from).collect();

        Ok(HttpResponse::Ok().json(MediaListResponse {
            media,
            total_count: result.total_count,
            total_size: result.total_size,
            limit: result.limit,
            offset: result.offset,
        }))
    }

    /// Get media count for a user
    pub async fn get_user_media_count(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let count = self.media_repo.get_user_media_count(&user_id).await?;

        Ok(HttpResponse::Ok().json(MediaCountResponse {
            user_id: user_id.to_string(),
            media_count: count,
        }))
    }

    /// Get total media size for a user
    pub async fn get_user_media_size(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let size = self.media_repo.get_user_media_size(&user_id).await?;

        Ok(HttpResponse::Ok().json(MediaSizeResponse {
            user_id: user_id.to_string(),
            total_size: size,
        }))
    }

    /// Delete a single media
    pub async fn delete_media(&self, media_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        self.media_repo.delete_media(&media_id).await?;

        Ok(HttpResponse::Ok().json(SuccessResponse {
            success: true,
            message: format!("Media {} deleted successfully", media_id),
        }))
    }

    /// Delete multiple media
    pub async fn delete_media_batch(
        &self,
        req: web::Json<DeleteMediaRequest>,
    ) -> Result<HttpResponse, AdminError> {
        // Note: This is a placeholder - actual implementation depends on Palpo schema
        // The MediaRepository currently returns 0 for all operations

        Ok(HttpResponse::Ok().json(BatchDeleteResponse {
            success: true,
            deleted_count: 0,
            message: "Media deletion not yet implemented - depends on Palpo media schema".to_string(),
        }))
    }

    /// Delete all media for a user
    pub async fn delete_all_user_media(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let count = self.media_repo.delete_user_media(&user_id).await?;

        Ok(HttpResponse::Ok().json(BatchDeleteResponse {
            success: true,
            deleted_count: count,
            message: format!("Deleted {} media for user {}", count, user_id),
        }))
    }
}

/// Conversion implementation
impl From<&crate::repositories::MediaMetadata> for MediaResponse {
    fn from(media: &crate::repositories::MediaMetadata) -> Self {
        MediaResponse {
            media_id: media.media_id.clone(),
            user_id: media.user_id.clone(),
            media_type: media.media_type.clone(),
            file_path: media.file_path.clone(),
            file_size: media.file_size,
            created_ts: media.created_ts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::DieselMediaRepository;
    use palpo_data::DieselPool;

    #[tokio::test]
    #[ignore]
    async fn test_list_user_media() {}

    #[tokio::test]
    #[ignore]
    async fn test_get_user_media_count() {}

    #[tokio::test]
    #[ignore]
    async fn test_delete_media() {}
}