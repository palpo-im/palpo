/// Shadow Ban Repository - Database operations for shadow ban management
///
/// This module provides the data access layer for shadow ban operations.
/// Shadow-banned users can continue to use the service but their messages
/// are silently filtered/ignored by other users.

use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use chrono::Utc;

use crate::types::AdminError;
use palpo_data::DieselPool;

/// Shadow ban status for a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowBanStatus {
    pub user_id: String,
    pub is_shadow_banned: bool,
    pub shadow_banned_at: Option<i64>,
    pub updated_at: i64,
}

/// Repository trait for shadow ban operations
#[async_trait::async_trait]
pub trait ShadowBanRepository {
    /// Get shadow ban status for a user
    async fn get_shadow_ban_status(&self, user_id: &str) -> Result<ShadowBanStatus, AdminError>;

    /// Set shadow ban status for a user
    async fn set_shadow_banned(&self, user_id: &str, shadow_banned: bool) -> Result<ShadowBanStatus, AdminError>;

    /// Check if user is shadow-banned
    async fn is_shadow_banned(&self, user_id: &str) -> Result<bool, AdminError>;

    /// Get all shadow-banned users
    async fn get_all_shadow_banned(&self, limit: i64, offset: i64) -> Result<Vec<String>, AdminError>;

    /// Get shadow-banned user count
    async fn get_shadow_banned_count(&self) -> Result<i64, AdminError>;
}

/// Diesel-based ShadowBanRepository implementation
#[derive(Debug)]
pub struct DieselShadowBanRepository {
    db_pool: DieselPool,
}

impl DieselShadowBanRepository {
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }
}

#[async_trait::async_trait]
impl ShadowBanRepository for DieselShadowBanRepository {
    async fn get_shadow_ban_status(&self, user_id: &str) -> Result<ShadowBanStatus, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let attributes = user_attributes::table
            .filter(user_attributes::user_id.eq(user_id))
            .first::<UserAttributes>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        if let Some(attrs) = attributes {
            Ok(ShadowBanStatus {
                user_id: user_id.to_string(),
                is_shadow_banned: attrs.shadow_banned,
                shadow_banned_at: None, // Could track this if needed
                updated_at: attrs.updated_at,
            })
        } else {
            // User doesn't exist, return not shadow-banned
            Ok(ShadowBanStatus {
                user_id: user_id.to_string(),
                is_shadow_banned: false,
                shadow_banned_at: None,
                updated_at: Utc::now().timestamp_millis(),
            })
        }
    }

    async fn set_shadow_banned(&self, user_id: &str, shadow_banned: bool) -> Result<ShadowBanStatus, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = Utc::now().timestamp_millis();

        // Update users table
        diesel::update(users::table.find(user_id))
            .set(users::shadow_banned.eq(shadow_banned))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        // Update attributes table
        diesel::update(user_attributes::table.find(user_id))
            .set((
                user_attributes::shadow_banned.eq(shadow_banned),
                user_attributes::updated_at.eq(now),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(ShadowBanStatus {
            user_id: user_id.to_string(),
            is_shadow_banned: shadow_banned,
            shadow_banned_at: if shadow_banned { Some(now) } else { None },
            updated_at: now,
        })
    }

    async fn is_shadow_banned(&self, user_id: &str) -> Result<bool, AdminError> {
        let status = self.get_shadow_ban_status(user_id).await?;
        Ok(status.is_shadow_banned)
    }

    async fn get_all_shadow_banned(&self, limit: i64, offset: i64) -> Result<Vec<String>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let user_ids: Vec<String> = users::table
            .filter(users::shadow_banned.eq(true))
            .select(users::name)
            .limit(limit)
            .offset(offset)
            .load(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(user_ids)
    }

    async fn get_shadow_banned_count(&self) -> Result<i64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = users::table
            .filter(users::shadow_banned.eq(true))
            .count()
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count)
    }
}

// Helper struct
#[derive(Queryable, Insertable, AsChangeset)]
#[diesel(table_name = user_attributes)]
struct UserAttributes {
    pub user_id: String,
    pub shadow_banned: bool,
    pub locked: bool,
    pub deactivated: bool,
    pub erased: bool,
    pub password_changed_ts: Option<i64>,
    pub last_force_reset_ts: Option<i64>,
    pub expiry_ts: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

// Table definitions
use crate::schema::users;
use crate::schema::user_attributes;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_get_shadow_ban_status() {}

    #[tokio::test]
    #[ignore]
    async fn test_set_shadow_banned() {}

    #[tokio::test]
    #[ignore]
    async fn test_get_all_shadow_banned() {}
}