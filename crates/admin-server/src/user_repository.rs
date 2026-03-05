/// User Repository - Database operations for user management
///
/// This module provides the data access layer for user management operations.
/// It implements the UserRepository trait with direct PostgreSQL operations
/// using Diesel ORM, optimized for performance (targeting 2x Synapse speed).
///
/// Features:
/// - Full CRUD operations for users
/// - User list with pagination and filtering
/// - Username availability checking
/// - Admin status management
/// - Shadow ban operations
/// - Device management integration

use diesel::prelude::*;
use diesel::sql_query;
use diesel::sql_types::{Text, BigInt, Bool, Integer, Nullable, Timestamp};

use crate::types::AdminError;
use palpo_data::DieselPool;

/// User entity representing a Matrix user account
#[derive(Debug, Clone, Queryable, Insertable, AsChangeset, Serialize, Deserialize)]
#[diesel(table_name = users)]
pub struct User {
    pub name: String,                    // @username:homeserver
    pub password_hash: Option<String>,
    pub salt: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub is_deactivated: bool,
    pub is_erased: bool,
    pub shadow_banned: bool,
    pub locked: bool,
    pub displayname: Option<String>,
    pub avatar_url: Option<String>,
    pub creation_ts: i64,
    pub last_seen_ts: Option<i64>,
    pub user_type: Option<String>,
    pub appservice_id: Option<String>,
    pub consent_version: Option<String>,
    pub consent_ts: Option<i64>,
    pub consent_server_notice_sent: bool,
}

/// User attributes for shadow-ban, locked, deactivated status
#[derive(Debug, Clone, Queryable, Insertable, AsChangeset, Serialize, Deserialize)]
#[diesel(table_name = user_attributes)]
pub struct UserAttributes {
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

/// User creation input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserInput {
    pub user_id: String,                 // @username:homeserver
    pub displayname: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub user_type: Option<String>,
    pub appservice_id: Option<String>,
}

/// User update input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserInput {
    pub displayname: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: Option<bool>,
    pub user_type: Option<String>,
}

/// User list filter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFilter {
    pub is_admin: Option<bool>,
    pub is_deactivated: Option<bool>,
    pub shadow_banned: Option<bool>,
    pub search_term: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// User list result with pagination info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserListResult {
    pub users: Vec<User>,
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
}

/// User details with extended information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDetails {
    pub user: User,
    pub attributes: Option<UserAttributes>,
    pub device_count: i64,
    pub session_count: i64,
    pub joined_room_count: i64,
}

/// Repository trait for user data access operations
///
/// This trait defines all user-related database operations.
/// Implementations should use parameterized queries to prevent SQL injection
/// and optimize for performance with proper indexing.
#[async_trait::async_trait]
pub trait UserRepository {
    /// Create a new user account
    async fn create_user(&self, input: &CreateUserInput) -> Result<User, AdminError>;

    /// Get a user by their Matrix ID
    async fn get_user(&self, user_id: &str) -> Result<Option<User>, AdminError>;

    /// Get user with all details including attributes
    async fn get_user_details(&self, user_id: &str) -> Result<Option<UserDetails>, AdminError>;

    /// Update user information
    async fn update_user(&self, user_id: &str, input: &UpdateUserInput) -> Result<User, AdminError>;

    /// Delete (deactivate) a user
    async fn deactivate_user(&self, user_id: &str, erase: bool) -> Result<(), AdminError>;

    /// Reactivate a deactivated user
    async fn reactivate_user(&self, user_id: &str) -> Result<(), AdminError>;

    /// List users with filtering and pagination
    async fn list_users(&self, filter: &UserFilter) -> Result<UserListResult, AdminError>;

    /// Check if a username is available
    async fn is_username_available(&self, username: &str) -> Result<bool, AdminError>;

    /// Set admin status for a user
    async fn set_admin_status(&self, user_id: &str, is_admin: bool) -> Result<(), AdminError>;

    /// Set shadow ban status for a user
    async fn set_shadow_banned(&self, user_id: &str, shadow_banned: bool) -> Result<(), AdminError>;

    /// Set locked status for a user
    async fn set_locked(&self, user_id: &str, locked: bool) -> Result<(), AdminError>;

    /// Get user attributes
    async fn get_user_attributes(&self, user_id: &str) -> Result<Option<UserAttributes>, AdminError>;

    /// Update user password hash
    async fn update_password(&self, user_id: &str, password_hash: &str, salt: &str) -> Result<(), AdminError>;

    /// Get count of all users
    async fn get_user_count(&self) -> Result<i64, AdminError>;

    /// Get count of admin users
    async fn get_admin_count(&self) -> Result<i64, AdminError>;

    /// Get count of deactivated users
    async fn get_deactivated_count(&self) -> Result<i64, AdminError>;
}

/// Diesel-based UserRepository implementation
///
/// Uses parameterized queries and proper indexing for optimal performance.
/// Targets 2x performance improvement over Synapse Admin API.
pub struct DieselUserRepository {
    db_pool: DieselPool,
}

impl DieselUserRepository {
    /// Creates a new repository instance with the given database pool
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }
}

#[async_trait::async_trait]
impl UserRepository for DieselUserRepository {
    async fn create_user(&self, input: &CreateUserInput) -> Result<User, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = chrono::Utc::now().timestamp_millis();

        let user = User {
            name: input.user_id.clone(),
            password_hash: None,
            salt: None,
            is_admin: input.is_admin,
            is_guest: input.is_guest,
            is_deactivated: false,
            is_erased: false,
            shadow_banned: false,
            locked: false,
            displayname: input.displayname.clone(),
            avatar_url: input.avatar_url.clone(),
            creation_ts: now,
            last_seen_ts: None,
            user_type: input.user_type.clone(),
            appservice_id: input.appservice_id.clone(),
            consent_version: None,
            consent_ts: None,
            consent_server_notice_sent: false,
        };

        diesel::insert_into(users::table)
            .values(&user)
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        // Create user attributes record
        let attributes = UserAttributes {
            user_id: input.user_id.clone(),
            shadow_banned: false,
            locked: false,
            deactivated: false,
            erased: false,
            password_changed_ts: None,
            last_force_reset_ts: None,
            expiry_ts: None,
            created_at: now,
            updated_at: now,
        };

        diesel::insert_into(user_attributes::table)
            .values(&attributes)
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(user)
    }

    async fn get_user(&self, user_id: &str) -> Result<Option<User>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let user = users::table
            .filter(users::name.eq(user_id))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(user)
    }

    async fn get_user_details(&self, user_id: &str) -> Result<Option<UserDetails>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        // Get user
        let user = match users::table
            .filter(users::name.eq(user_id))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?
        {
            Some(u) => u,
            None => return Ok(None),
        };

        // Get attributes
        let attributes = user_attributes::table
            .filter(user_attributes::user_id.eq(user_id))
            .first::<UserAttributes>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        // Get device count
        let device_count = diesel::select(diesel::dsl::count(devices::device_id))
            .filter(devices::user_id.eq(user_id))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        // Get session count (unique IPs)
        let session_count = diesel::select(diesel::dsl::count_distinct(user_ips::ip))
            .filter(user_ips::user_id.eq(user_id))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        // Get joined room count
        let joined_room_count = diesel::select(diesel::dsl::count(room_memberships::room_id))
            .filter(room_memberships::user_id.eq(user_id))
            .filter(room_memberships::membership.eq("join"))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(Some(UserDetails {
            user,
            attributes,
            device_count,
            session_count,
            joined_room_count,
        }))
    }

    async fn update_user(&self, user_id: &str, input: &UpdateUserInput) -> Result<User, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let user = diesel::update(users::table.find(user_id))
            .set((
                input.displayname.is_some().then(|| users::displayname.eq(&input.displayname)),
                input.avatar_url.is_some().then(|| users::avatar_url.eq(&input.avatar_url)),
                input.is_admin.is_some().then(|| users::is_admin.eq(input.is_admin.unwrap())),
                input.user_type.is_some().then(|| users::user_type.eq(&input.user_type)),
            ).filter(users::name.eq(user_id))
            .get_result::<User>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(user)
    }

    async fn deactivate_user(&self, user_id: &str, erase: bool) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        // Update users table
        diesel::update(users::table.find(user_id))
            .set((
                users::is_deactivated.eq(true),
                users::is_erased.eq(erase),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        // Update attributes
        diesel::update(user_attributes::table.find(user_id))
            .set((
                user_attributes::deactivated.eq(true),
                user_attributes::erased.eq(erase),
                user_attributes::updated_at.eq(chrono::Utc::now().timestamp_millis()),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn reactivate_user(&self, user_id: &str) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        diesel::update(users::table.find(user_id))
            .set((
                users::is_deactivated.eq(false),
                users::is_erased.eq(false),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        diesel::update(user_attributes::table.find(user_id))
            .set((
                user_attributes::deactivated.eq(false),
                user_attributes::erased.eq(false),
                user_attributes::updated_at.eq(chrono::Utc::now().timestamp_millis()),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn list_users(&self, filter: &UserFilter) -> Result<UserListResult, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let limit = filter.limit.unwrap_or(50).min(100);
        let offset = filter.offset.unwrap_or(0);

        // Build query with filters
        let mut query = users::table.into_boxed();

        if let Some(is_admin) = filter.is_admin {
            query = query.filter(users::is_admin.eq(is_admin));
        }

        if let Some(is_deactivated) = filter.is_deactivated {
            query = query.filter(users::is_deactivated.eq(is_deactivated));
        }

        if let Some(shadow_banned) = filter.shadow_banned {
            query = query.filter(users::shadow_banned.eq(shadow_banned));
        }

        if let Some(search_term) = &filter.search_term {
            if !search_term.is_empty() {
                query = query.filter(
                    users::name.ilike(format!("%{}%", search_term))
                    .or(users::displayname.ilike(format!("%{}%", search_term)))
                );
            }
        }

        // Get total count
        let total_count = query.clone()
            .count()
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        // Get users with pagination
        let users = query
            .order_by(users::creation_ts.desc())
            .limit(limit)
            .offset(offset)
            .load::<User>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(UserListResult {
            users,
            total_count,
            limit,
            offset,
        })
    }

    async fn is_username_available(&self, username: &str) -> Result<bool, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = diesel::select(diesel::dsl::count(users::name))
            .filter(users::name.eq(format!("@{}:localhost", username)))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count == 0)
    }

    async fn set_admin_status(&self, user_id: &str, is_admin: bool) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        diesel::update(users::table.find(user_id))
            .set(users::is_admin.eq(is_admin))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn set_shadow_banned(&self, user_id: &str, shadow_banned: bool) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = chrono::Utc::now().timestamp_millis();

        diesel::update(users::table.find(user_id))
            .set(users::shadow_banned.eq(shadow_banned))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        diesel::update(user_attributes::table.find(user_id))
            .set((
                user_attributes::shadow_banned.eq(shadow_banned),
                user_attributes::updated_at.eq(now),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn set_locked(&self, user_id: &str, locked: bool) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = chrono::Utc::now().timestamp_millis();

        diesel::update(users::table.find(user_id))
            .set(users::locked.eq(locked))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        diesel::update(user_attributes::table.find(user_id))
            .set((
                user_attributes::locked.eq(locked),
                user_attributes::updated_at.eq(now),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn get_user_attributes(&self, user_id: &str) -> Result<Option<UserAttributes>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let attributes = user_attributes::table
            .filter(user_attributes::user_id.eq(user_id))
            .first::<UserAttributes>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(attributes)
    }

    async fn update_password(&self, user_id: &str, password_hash: &str, salt: &str) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = chrono::Utc::now().timestamp_millis();

        diesel::update(users::table.find(user_id))
            .set((
                users::password_hash.eq(password_hash),
                users::salt.eq(salt),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        diesel::update(user_attributes::table.find(user_id))
            .set((
                user_attributes::password_changed_ts.eq(now),
                user_attributes::updated_at.eq(now),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn get_user_count(&self) -> Result<i64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = diesel::select(diesel::dsl::count(users::name))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count)
    }

    async fn get_admin_count(&self) -> Result<i64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = diesel::select(diesel::dsl::count(users::name))
            .filter(users::is_admin.eq(true))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count)
    }

    async fn get_deactivated_count(&self) -> Result<i64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = diesel::select(diesel::dsl::count(users::name))
            .filter(users::is_deactivated.eq(true))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count)
    }
}

// Table definitions for Diesel
use crate::schema::*;
use crate::schema::users;
use crate::schema::devices;
use crate::schema::user_ips;
use crate::schema::room_memberships;
use crate::schema::user_attributes;

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a PostgreSQL database connection
    // They are integration tests and should be run with a test database

    #[tokio::test]
    #[ignore]
    async fn test_create_user() {
        // Test user creation
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_user() {
        // Test getting a user by ID
    }

    #[tokio::test]
    #[ignore]
    async fn test_list_users_with_filter() {
        // Test listing users with various filters
    }

    #[tokio::test]
    #[ignore]
    async fn test_username_availability() {
        // Test username availability checking
    }

    #[tokio::test]
    #[ignore]
    async fn test_shadow_ban() {
        // Test shadow ban operations
    }
}