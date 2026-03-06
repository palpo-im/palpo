/// Rate Limit Repository - Database operations for per-user rate limiting
///
/// This module provides the data access layer for rate limit configuration.
/// It implements the RateLimitRepository trait with direct PostgreSQL operations.
///
/// Features:
/// - Per-user rate limit configuration
/// - Get/Set/Delete rate limit settings

use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use chrono::Utc;

use crate::types::AdminError;
use palpo_data::DieselPool;

/// User rate limit configuration
#[derive(Debug, Clone, Queryable, Insertable, AsChangeset, Serialize, Deserialize)]
#[diesel(table_name = user_rate_limit_configs)]
pub struct UserRateLimitConfig {
    pub user_id: String,
    pub messages_per_second: i32,
    pub burst_count: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Rate limit update input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRateLimitInput {
    pub messages_per_second: i32,
    pub burst_count: i32,
}

/// Repository trait for rate limit configuration
#[async_trait::async_trait]
pub trait RateLimitRepository {
    /// Get rate limit config for a user
    async fn get_rate_limit(&self, user_id: &str) -> Result<Option<UserRateLimitConfig>, AdminError>;

    /// Set rate limit config for a user
    async fn set_rate_limit(&self, user_id: &str, input: &UpdateRateLimitInput) -> Result<UserRateLimitConfig, AdminError>;

    /// Delete rate limit config for a user
    async fn delete_rate_limit(&self, user_id: &str) -> Result<(), AdminError>;

    /// Check if user has custom rate limit
    async fn has_custom_rate_limit(&self, user_id: &str) -> Result<bool, AdminError>;
}

/// Diesel-based RateLimitRepository implementation
#[derive(Debug)]
pub struct DieselRateLimitRepository {
    db_pool: DieselPool,
}

impl DieselRateLimitRepository {
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }
}

#[async_trait::async_trait]
impl RateLimitRepository for DieselRateLimitRepository {
    async fn get_rate_limit(&self, user_id: &str) -> Result<Option<UserRateLimitConfig>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let config = user_rate_limit_configs::table
            .filter(user_rate_limit_configs::user_id.eq(user_id))
            .first::<UserRateLimitConfig>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(config)
    }

    async fn set_rate_limit(&self, user_id: &str, input: &UpdateRateLimitInput) -> Result<UserRateLimitConfig, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = Utc::now().timestamp_millis();

        let config = UserRateLimitConfig {
            user_id: user_id.to_string(),
            messages_per_second: input.messages_per_second,
            burst_count: input.burst_count,
            created_at: now,
            updated_at: now,
        };

        diesel::insert_into(user_rate_limit_configs::table)
            .values(&config)
            .on_conflict(user_rate_limit_configs::user_id)
            .do_update()
            .set((
                user_rate_limit_configs::messages_per_second.eq(input.messages_per_second),
                user_rate_limit_configs::burst_count.eq(input.burst_count),
                user_rate_limit_configs::updated_at.eq(now),
            ))
            .get_result::<UserRateLimitConfig>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(config)
    }

    async fn delete_rate_limit(&self, user_id: &str) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        diesel::delete(
            user_rate_limit_configs::table.filter(user_rate_limit_configs::user_id.eq(user_id))
        )
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn has_custom_rate_limit(&self, user_id: &str) -> Result<bool, AdminError> {
        let config = self.get_rate_limit(user_id).await?;
        Ok(config.is_some())
    }
}

// Table definitions
use crate::schema::user_rate_limit_configs;

#[cfg(test)]
mod tests {
    use super::*;

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