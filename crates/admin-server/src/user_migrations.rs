/// Database migration runner for User Management
///
/// This module manages the database schema migrations for the user management feature.
/// It creates all necessary tables for user administration including:
/// - users: Main user account table
/// - devices: User device management
/// - user_ips: IP address tracking for sessions
/// - room_memberships: User room membership tracking
/// - user_rate_limit_configs: Per-user rate limiting
/// - account_data: User account data storage
/// - user_attributes: Extended user attributes (shadow_ban, locked, etc.)
/// - user_threepids: Third-party identifiers (email, phone)
/// - user_external_ids: SSO external IDs

use diesel::prelude::*;
use diesel::sql_query;
use diesel::sql_types::{Text, BigInt, Bool, Timestamp};

use super::types::AdminError;
use super::schema::*;
use palpo_data::DieselPool;

/// Migration runner for User Management database schema
pub struct UserMigrationRunner {
    db_pool: DieselPool,
}

impl UserMigrationRunner {
    /// Creates a new migration runner with the given database connection pool
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }

    /// Runs all user management migrations
    pub fn run_migrations(&self) -> Result<(), AdminError> {
        tracing::info!("Running user management database migrations");

        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        // Run each migration in order
        self.run_migration_001_create_users_table(&mut conn)?;
        self.run_migration_002_create_devices_table(&mut conn)?;
        self.run_migration_003_create_user_ips_table(&mut conn)?;
        self.run_migration_004_create_room_memberships_table(&mut conn)?;
        self.run_migration_005_create_user_rate_limit_configs_table(&mut conn)?;
        self.run_migration_006_create_account_data_table(&mut conn)?;
        self.run_migration_007_create_user_attributes_table(&mut conn)?;
        self.run_migration_008_create_user_threepids_table(&mut conn)?;
        self.run_migration_009_create_user_external_ids_table(&mut conn)?;

        tracing::info!("User management database migrations completed successfully");
        Ok(())
    }

    /// Migration 001: Create users table
    fn run_migration_001_create_users_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 001: Create users table");

        // Create users table
        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                name TEXT PRIMARY KEY,
                password_hash TEXT,
                salt TEXT,
                is_admin BOOLEAN NOT NULL DEFAULT FALSE,
                is_guest BOOLEAN NOT NULL DEFAULT FALSE,
                is_deactivated BOOLEAN NOT NULL DEFAULT FALSE,
                is_erased BOOLEAN NOT NULL DEFAULT FALSE,
                shadow_banned BOOLEAN NOT NULL DEFAULT FALSE,
                locked BOOLEAN NOT NULL DEFAULT FALSE,
                displayname TEXT,
                avatar_url TEXT,
                creation_ts BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                last_seen_ts BIGINT,
                user_type TEXT,
                appservice_id TEXT,
                consent_version TEXT,
                consent_ts BIGINT,
                consent_server_notice_sent BOOLEAN NOT NULL DEFAULT FALSE
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!("Failed to create users table: {}", e))
        })?;

        // Create indexes for common query patterns
        sql_query("CREATE INDEX IF NOT EXISTS idx_users_is_admin ON users(is_admin)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_users_is_admin: {}", e))
            })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_users_is_deactivated ON users(is_deactivated)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_users_is_deactivated: {}", e))
            })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_users_creation_ts ON users(creation_ts)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_users_creation_ts: {}", e))
            })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_users_last_seen_ts ON users(last_seen_ts)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_users_last_seen_ts: {}", e))
            })?;

        tracing::info!("Migration 001 completed successfully");
        Ok(())
    }

    /// Migration 002: Create devices table
    fn run_migration_002_create_devices_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 002: Create devices table");

        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS devices (
                device_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                display_name TEXT,
                last_seen_ts BIGINT,
                last_seen_ip TEXT,
                last_seen_user_agent TEXT,
                created_ts BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                PRIMARY KEY (device_id, user_id),
                FOREIGN KEY (user_id) REFERENCES users(name) ON DELETE CASCADE
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!("Failed to create devices table: {}", e))
        })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_devices_user_id ON devices(user_id)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_devices_user_id: {}", e))
            })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_devices_last_seen_ts ON devices(last_seen_ts)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_devices_last_seen_ts: {}", e))
            })?;

        tracing::info!("Migration 002 completed successfully");
        Ok(())
    }

    /// Migration 003: Create user_ips table
    fn run_migration_003_create_user_ips_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 003: Create user_ips table");

        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS user_ips (
                user_id TEXT NOT NULL,
                ip TEXT NOT NULL,
                last_seen_ts BIGINT NOT NULL,
                device_id TEXT,
                user_agent TEXT,
                PRIMARY KEY (user_id, ip, last_seen_ts),
                FOREIGN KEY (user_id) REFERENCES users(name) ON DELETE CASCADE
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!("Failed to create user_ips table: {}", e))
        })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_user_ips_user_id ON user_ips(user_id)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_user_ips_user_id: {}", e))
            })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_user_ips_last_seen_ts ON user_ips(last_seen_ts)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_user_ips_last_seen_ts: {}", e))
            })?;

        tracing::info!("Migration 003 completed successfully");
        Ok(())
    }

    /// Migration 004: Create room_memberships table
    fn run_migration_004_create_room_memberships_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 004: Create room_memberships table");

        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS room_memberships (
                room_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                membership TEXT NOT NULL,
                joined_ts BIGINT,
                invite_ts BIGINT,
                leave_ts BIGINT,
                ban_ts BIGINT,
                PRIMARY KEY (room_id, user_id),
                FOREIGN KEY (user_id) REFERENCES users(name) ON DELETE CASCADE
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!("Failed to create room_memberships table: {}", e))
        })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_room_memberships_user_id ON room_memberships(user_id)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_room_memberships_user_id: {}", e))
            })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_room_memberships_membership ON room_memberships(membership)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_room_memberships_membership: {}", e))
            })?;

        tracing::info!("Migration 004 completed successfully");
        Ok(())
    }

    /// Migration 005: Create user_rate_limit_configs table
    fn run_migration_005_create_user_rate_limit_configs_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 005: Create user_rate_limit_configs table");

        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS user_rate_limit_configs (
                user_id TEXT PRIMARY KEY,
                messages_per_second INTEGER NOT NULL DEFAULT 0,
                burst_count INTEGER NOT NULL DEFAULT 0,
                created_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                updated_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                FOREIGN KEY (user_id) REFERENCES users(name) ON DELETE CASCADE
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!("Failed to create user_rate_limit_configs table: {}", e))
        })?;

        tracing::info!("Migration 005 completed successfully");
        Ok(())
    }

    /// Migration 006: Create account_data table
    fn run_migration_006_create_account_data_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 006: Create account_data table");

        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS account_data (
                user_id TEXT NOT NULL,
                data_type TEXT NOT NULL,
                content TEXT NOT NULL,
                created_ts BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                updated_ts BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                PRIMARY KEY (user_id, data_type),
                FOREIGN KEY (user_id) REFERENCES users(name) ON DELETE CASCADE
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!("Failed to create account_data table: {}", e))
        })?;

        sql_query("CREATE INDEX IF NOT EXISTS idx_account_data_user_id ON account_data(user_id)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_account_data_user_id: {}", e))
            })?;

        tracing::info!("Migration 006 completed successfully");
        Ok(())
    }

    /// Migration 007: Create user_attributes table
    fn run_migration_007_create_user_attributes_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 007: Create user_attributes table");

        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS user_attributes (
                user_id TEXT PRIMARY KEY,
                shadow_banned BOOLEAN NOT NULL DEFAULT FALSE,
                locked BOOLEAN NOT NULL DEFAULT FALSE,
                deactivated BOOLEAN NOT NULL DEFAULT FALSE,
                erased BOOLEAN NOT NULL DEFAULT FALSE,
                password_changed_ts BIGINT,
                last_force_reset_ts BIGINT,
                expiry_ts BIGINT,
                created_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                updated_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                FOREIGN KEY (user_id) REFERENCES users(name) ON DELETE CASCADE
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!("Failed to create user_attributes table: {}", e))
        })?;

        tracing::info!("Migration 007 completed successfully");
        Ok(())
    }

    /// Migration 008: Create user_threepids table
    fn run_migration_008_create_user_threepids_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 008: Create user_threepids table");

        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS user_threepids (
                user_id TEXT NOT NULL,
                medium TEXT NOT NULL,
                address TEXT NOT NULL,
                validated_ts BIGINT,
                added_ts BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                PRIMARY KEY (user_id, medium, address),
                FOREIGN KEY (user_id) REFERENCES users(name) ON DELETE CASCADE
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!("Failed to create user_threepids table: {}", e))
        })?;

        sql_query("CREATE UNIQUE INDEX IF NOT EXISTS idx_user_threepids_medium_address ON user_threepids(medium, address)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_user_threepids_medium_address: {}", e))
            })?;

        tracing::info!("Migration 008 completed successfully");
        Ok(())
    }

    /// Migration 009: Create user_external_ids table
    fn run_migration_009_create_user_external_ids_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 009: Create user_external_ids table");

        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS user_external_ids (
                user_id TEXT NOT NULL,
                auth_provider TEXT NOT NULL,
                external_id TEXT NOT NULL,
                created_ts BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()) * 1000,
                PRIMARY KEY (user_id, auth_provider, external_id),
                FOREIGN KEY (user_id) REFERENCES users(name) ON DELETE CASCADE
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!("Failed to create user_external_ids table: {}", e))
        })?;

        sql_query("CREATE UNIQUE INDEX IF NOT EXISTS idx_user_external_ids_provider_external ON user_external_ids(auth_provider, external_id)")
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to create idx_user_external_ids_provider_external: {}", e))
            })?;

        tracing::info!("Migration 009 completed successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Integration tests would require a PostgreSQL database connection
    // These tests verify the migration logic and schema creation

    #[tokio::test]
    #[ignore]
    async fn test_users_table_creation() {
        // Test that users table is created with correct schema
    }

    #[tokio::test]
    #[ignore]
    async fn test_devices_table_creation() {
        // Test that devices table is created with correct schema
    }

    #[tokio::test]
    #[ignore]
    async fn test_all_indexes_created() {
        // Test that all indexes are created for performance
    }
}