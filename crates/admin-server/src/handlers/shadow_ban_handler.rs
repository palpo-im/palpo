/// Shadow Ban Handler - HTTP handlers for shadow-ban operations
///
/// This module implements shadow-ban API endpoints:
/// - Get shadow ban status for a user
/// - Set shadow ban status for a user
/// - List all shadow-banned users
/// - Get shadow-banned user count

use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};

use crate::types::AdminError;
use crate::repositories::ShadowBanRepository;

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Generic success response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Shadow ban handler configuration
pub struct ShadowBanHandler<T: ShadowBanRepository> {
    shadow_ban_repo: T,
}

impl<T: ShadowBanRepository> ShadowBanHandler<T> {
    /// Create a new handler with the given repository
    pub fn new(shadow_ban_repo: T) -> Self {
        Self { shadow_ban_repo }
    }

    /// Get shadow ban status for a user
    pub async fn get_shadow_ban_status(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let status = self.shadow_ban_repo.get_shadow_ban_status(&user_id).await?;

        Ok(HttpResponse::Ok().json(ShadowBanStatusResponse {
            user_id: status.user_id,
            is_shadow_banned: status.is_shadow_banned,
            shadow_banned_at: status.shadow_banned_at,
            updated_at: status.updated_at,
        }))
    }

    /// Set shadow ban status for a user
    pub async fn set_shadow_banned(
        &self,
        user_id: web::Path<String>,
        req: web::Json<SetShadowBanRequest>,
    ) -> Result<HttpResponse, AdminError> {
        let status = self.shadow_ban_repo.set_shadow_banned(&user_id, req.shadow_banned).await?;

        tracing::info!("Set shadow ban for {}: {}", user_id, req.shadow_banned);

        Ok(HttpResponse::Ok().json(ShadowBanStatusResponse {
            user_id: status.user_id,
            is_shadow_banned: status.is_shadow_banned,
            shadow_banned_at: status.shadow_banned_at,
            updated_at: status.updated_at,
        }))
    }

    /// Check if user is shadow-banned
    pub async fn is_shadow_banned(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let is_banned = self.shadow_ban_repo.is_shadow_banned(&user_id).await?;

        Ok(HttpResponse::Ok().json(ShadowBanCheckResponse {
            user_id: user_id.to_string(),
            is_shadow_banned: is_banned,
        }))
    }

    /// List all shadow-banned users
    pub async fn list_shadow_banned_users(
        &self,
        query: web::Query<ShadowBanListQuery>,
    ) -> Result<HttpResponse, AdminError> {
        let limit = query.limit.unwrap_or(50).min(100);
        let offset = query.offset.unwrap_or(0);

        let users = self.shadow_ban_repo.get_all_shadow_banned(limit, offset).await?;
        let total = self.shadow_ban_repo.get_shadow_banned_count().await?;

        Ok(HttpResponse::Ok().json(ShadowBanListResponse {
            users,
            total_count: total,
            limit,
            offset,
        }))
    }

    /// Get shadow-banned user count
    pub async fn get_shadow_banned_count(&self) -> Result<HttpResponse, AdminError> {
        let count = self.shadow_ban_repo.get_shadow_banned_count().await?;

        Ok(HttpResponse::Ok().json(ShadowBanCountResponse {
            total_shadow_banned: count,
        }))
    }
}

/// Shadow ban check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowBanCheckResponse {
    pub user_id: String,
    pub is_shadow_banned: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::DieselShadowBanRepository;
    use palpo_data::DieselPool;

    #[tokio::test]
    #[ignore]
    async fn test_get_shadow_ban_status() {}

    #[tokio::test]
    #[ignore]
    async fn test_set_shadow_banned() {}

    #[tokio::test]
    #[ignore]
    async fn test_list_shadow_banned_users() {}
}