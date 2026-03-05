/// Rate Limit Handler - HTTP handlers for rate limit configuration API
///
/// This module implements rate limit configuration API endpoints:
/// - Get rate limit config for a user
/// - Set rate limit config for a user
/// - Delete rate limit config for a user

use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};

use crate::types::AdminError;
use crate::repositories::{RateLimitRepository, UpdateRateLimitInput};

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

/// Rate limit handler configuration
pub struct RateLimitHandler<T: RateLimitRepository> {
    rate_limit_repo: T,
}

impl<T: RateLimitRepository> RateLimitHandler<T> {
    /// Create a new handler with the given repository
    pub fn new(rate_limit_repo: T) -> Self {
        Self { rate_limit_repo }
    }

    /// Get rate limit config for a user
    pub async fn get_rate_limit(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let config = self.rate_limit_repo.get_rate_limit(&user_id).await?;

        match config {
            Some(c) => Ok(HttpResponse::Ok().json(RateLimitConfigResponse {
                user_id: c.user_id,
                messages_per_second: c.messages_per_second,
                burst_count: c.burst_count,
                created_at: c.created_at,
                updated_at: c.updated_at,
            })),
            None => Ok(HttpResponse::NotFound().json(SuccessResponse {
                success: false,
                message: format!("No custom rate limit config for user {}", user_id),
            })),
        }
    }

    /// Set rate limit config for a user
    pub async fn set_rate_limit(
        &self,
        user_id: web::Path<String>,
        req: web::Json<SetRateLimitRequest>,
    ) -> Result<HttpResponse, AdminError> {
        let input = UpdateRateLimitInput {
            messages_per_second: req.messages_per_second,
            burst_count: req.burst_count,
        };

        let config = self.rate_limit_repo.set_rate_limit(&user_id, &input).await?;

        tracing::info!("Set rate limit for user {}: {}/{}",
            user_id, req.messages_per_second, req.burst_count);

        Ok(HttpResponse::Ok().json(RateLimitConfigResponse {
            user_id: config.user_id,
            messages_per_second: config.messages_per_second,
            burst_count: config.burst_count,
            created_at: config.created_at,
            updated_at: config.updated_at,
        }))
    }

    /// Delete rate limit config for a user (revert to default)
    pub async fn delete_rate_limit(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        self.rate_limit_repo.delete_rate_limit(&user_id).await?;

        tracing::info!("Deleted rate limit config for user {}", user_id);

        Ok(HttpResponse::Ok().json(SuccessResponse {
            success: true,
            message: format!("Rate limit config deleted for user {}", user_id),
        }))
    }

    /// Check if user has custom rate limit
    pub async fn has_custom_rate_limit(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let has_custom = self.rate_limit_repo.has_custom_rate_limit(&user_id).await?;

        Ok(HttpResponse::Ok().json(CustomRateLimitResponse {
            user_id: user_id.to_string(),
            has_custom_rate_limit: has_custom,
        }))
    }
}

/// Custom rate limit check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRateLimitResponse {
    pub user_id: String,
    pub has_custom_rate_limit: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::DieselRateLimitRepository;
    use palpo_data::DieselPool;

    #[tokio::test]
    #[ignore]
    async fn test_get_rate_limit() {}

    #[tokio::test]
    #[ignore]
    async fn test_set_rate_limit() {}

    #[tokio::test]
    #[ignore]
    async fn test_delete_rate_limit() {}
}