/// Threepid Repository - Database operations for third-party identifiers
///
/// This module provides the data access layer for third-party identifier (threepid)
/// management. Threepids include email addresses, phone numbers, and other
/// identifiers used for user identity and password reset.
///
/// Features:
/// - Threepid lookup by medium and address
/// - User threepid listing
/// - External ID lookup for SSO

use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use chrono::Utc;

use crate::types::AdminError;
use palpo_data::DieselPool;

/// Third-party identifier (email, phone, etc.)
#[derive(Debug, Clone, Queryable, Insertable, AsChangeset, Serialize, Deserialize)]
#[diesel(table_name = user_threepids)]
pub struct UserThreepid {
    pub user_id: String,
    pub medium: String,        // "email", "phone", etc.
    pub address: String,       // email address, phone number, etc.
    pub validated_ts: Option<i64>,
    pub added_ts: i64,
}

/// External ID for SSO providers
#[derive(Debug, Clone, Queryable, Insertable, AsChangeset, Serialize, Deserialize)]
#[diesel(table_name = user_external_ids)]
pub struct UserExternalId {
    pub user_id: String,
    pub auth_provider: String, // "saml", "oidc", etc.
    pub external_id: String,
    pub created_ts: i64,
}

/// Threepid lookup result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreepidLookupResult {
    pub user_id: String,
    pub medium: String,
    pub address: String,
    pub validated: bool,
    pub validated_at: Option<i64>,
}

/// External ID lookup result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIdLookupResult {
    pub user_id: String,
    pub auth_provider: String,
    pub external_id: String,
}

/// Repository trait for threepid data access operations
#[async_trait::async_trait]
pub trait ThreepidRepository {
    /// Lookup user by threepid (medium + address)
    async fn lookup_user_by_threepid(&self, medium: &str, address: &str) -> Result<Option<ThreepidLookupResult>, AdminError>;

    /// Get all threepids for a user
    async fn get_user_threepids(&self, user_id: &str) -> Result<Vec<UserThreepid>, AdminError>;

    /// Add a threepid for a user
    async fn add_threepid(&self, user_id: &str, medium: &str, address: &str) -> Result<UserThreepid, AdminError>;

    /// Remove a threepid from a user
    async fn remove_threepid(&self, user_id: &str, medium: &str, address: &str) -> Result<(), AdminError>;

    /// Validate a threepid
    async fn validate_threepid(&self, user_id: &str, medium: &str, address: &str) -> Result<(), AdminError>;

    /// Lookup user by external ID
    async fn lookup_user_by_external_id(&self, provider: &str, external_id: &str) -> Result<Option<ExternalIdLookupResult>, AdminError>;

    /// Get all external IDs for a user
    async fn get_user_external_ids(&self, user_id: &str) -> Result<Vec<UserExternalId>, AdminError>;

    /// Add an external ID for a user
    async fn add_external_id(&self, user_id: &str, provider: &str, external_id: &str) -> Result<UserExternalId, AdminError>;

    /// Remove an external ID
    async fn remove_external_id(&self, user_id: &str, provider: &str, external_id: &str) -> Result<(), AdminError>;
}

/// Diesel-based ThreepidRepository implementation
#[derive(Debug)]
pub struct DieselThreepidRepository {
    db_pool: DieselPool,
}

impl DieselThreepidRepository {
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }
}

#[async_trait::async_trait]
impl ThreepidRepository for DieselThreepidRepository {
    async fn lookup_user_by_threepid(&self, medium: &str, address: &str) -> Result<Option<ThreepidLookupResult>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let threepid = user_threepids::table
            .filter(user_threepids::medium.eq(medium))
            .filter(user_threepids::address.eq(address))
            .first::<UserThreepid>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(threepid.map(|t| ThreepidLookupResult {
            user_id: t.user_id,
            medium: t.medium,
            address: t.address,
            validated: t.validated_ts.is_some(),
            validated_at: t.validated_ts,
        }))
    }

    async fn get_user_threepids(&self, user_id: &str) -> Result<Vec<UserThreepid>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let threepids = user_threepids::table
            .filter(user_threepids::user_id.eq(user_id))
            .order_by(user_threepids::added_ts.desc())
            .load::<UserThreepid>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(threepids)
    }

    async fn add_threepid(&self, user_id: &str, medium: &str, address: &str) -> Result<UserThreepid, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = Utc::now().timestamp_millis();

        let threepid = UserThreepid {
            user_id: user_id.to_string(),
            medium: medium.to_string(),
            address: address.to_string(),
            validated_ts: None,
            added_ts: now,
        };

        diesel::insert_into(user_threepids::table)
            .values(&threepid)
            .on_conflict((user_threepids::user_id, user_threepids::medium, user_threepids::address))
            .do_nothing()
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(threepid)
    }

    async fn remove_threepid(&self, user_id: &str, medium: &str, address: &str) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        diesel::delete(
            user_threepids::table
                .filter(user_threepids::user_id.eq(user_id))
                .filter(user_threepids::medium.eq(medium))
                .filter(user_threepids::address.eq(address))
        )
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn validate_threepid(&self, user_id: &str, medium: &str, address: &str) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = Utc::now().timestamp_millis();

        diesel::update(
            user_threepids::table
                .filter(user_threepids::user_id.eq(user_id))
                .filter(user_threepids::medium.eq(medium))
                .filter(user_threepids::address.eq(address))
        )
        .set(user_threepids::validated_ts.eq(now))
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn lookup_user_by_external_id(&self, provider: &str, external_id: &str) -> Result<Option<ExternalIdLookupResult>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let external_id_record = user_external_ids::table
            .filter(user_external_ids::auth_provider.eq(provider))
            .filter(user_external_ids::external_id.eq(external_id))
            .first::<UserExternalId>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(external_id_record.map(|e| ExternalIdLookupResult {
            user_id: e.user_id,
            auth_provider: e.auth_provider,
            external_id: e.external_id,
        }))
    }

    async fn get_user_external_ids(&self, user_id: &str) -> Result<Vec<UserExternalId>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let external_ids = user_external_ids::table
            .filter(user_external_ids::user_id.eq(user_id))
            .order_by(user_external_ids::created_ts.desc())
            .load::<UserExternalId>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(external_ids)
    }

    async fn add_external_id(&self, user_id: &str, provider: &str, external_id: &str) -> Result<UserExternalId, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = Utc::now().timestamp_millis();

        let external_id = UserExternalId {
            user_id: user_id.to_string(),
            auth_provider: provider.to_string(),
            external_id: external_id.to_string(),
            created_ts: now,
        };

        diesel::insert_into(user_external_ids::table)
            .values(&external_id)
            .on_conflict((user_external_ids::user_id, user_external_ids::auth_provider, user_external_ids::external_id))
            .do_nothing()
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(external_id)
    }

    async fn remove_external_id(&self, user_id: &str, provider: &str, external_id: &str) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        diesel::delete(
            user_external_ids::table
                .filter(user_external_ids::user_id.eq(user_id))
                .filter(user_external_ids::auth_provider.eq(provider))
                .filter(user_external_ids::external_id.eq(external_id))
        )
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }
}

// Table definitions
use crate::schema::user_threepids;
use crate::schema::user_external_ids;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_lookup_user_by_threepid() {}

    #[tokio::test]
    #[ignore]
    async fn test_get_user_threepids() {}

    #[tokio::test]
    #[ignore]
    async fn test_lookup_user_by_external_id() {}
}