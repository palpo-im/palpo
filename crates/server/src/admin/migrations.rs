/// Database migration runner for Web UI admin credentials
///
/// This module manages the database schema migrations for the Web UI admin system.
/// It tracks applied migrations in a `schema_migrations` table and ensures
/// migrations are applied idempotently (safe to run multiple times).
///
/// The migration system is designed to be simple and focused on the Web UI admin
/// credentials table, which is independent of the main Palpo/Matrix database schema.

use diesel::prelude::*;
use diesel::sql_query;
use diesel::sql_types::Text;

use super::types::AdminError;
use crate::data::DieselPool;

/// Migration runner for Web UI admin database schema
///
/// Manages the creation and tracking of database migrations for the
/// webui_admin_credentials table and related schema.
pub struct MigrationRunner {
    db_pool: DieselPool,
}

impl MigrationRunner {
    /// Creates a new migration runner with the given database connection pool
    ///
    /// # Arguments
    ///
    /// * `db_pool` - Diesel PostgreSQL connection pool
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }

    /// Runs all pending migrations
    ///
    /// This method:
    /// 1. Creates the schema_migrations tracking table if it doesn't exist
    /// 2. Checks which migrations have already been applied
    /// 3. Applies any pending migrations
    /// 4. Records successful migrations in the tracking table
    ///
    /// This operation is idempotent - running it multiple times is safe.
    ///
    /// # Errors
    ///
    /// Returns `AdminError::DatabaseMigrationFailed` if any migration step fails
    pub fn run_migrations(&self) -> Result<(), AdminError> {
        tracing::info!("Running database migrations for Web UI admin");

        // Get a connection from the pool
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        // Create migrations tracking table if it doesn't exist
        self.create_migrations_table(&mut conn)?;

        // Check if migration 001 already applied
        if self.is_migration_applied(&mut conn, "001_create_webui_admin_credentials")? {
            tracing::info!("Migration 001_create_webui_admin_credentials already applied");
            return Ok(());
        }

        // Run migration 001
        self.run_migration_001(&mut conn)?;

        // Record migration as applied
        self.record_migration(&mut conn, "001_create_webui_admin_credentials")?;

        tracing::info!("Database migrations completed successfully");
        Ok(())
    }

    /// Creates the schema_migrations table for tracking applied migrations
    ///
    /// The table has two columns:
    /// - `version`: TEXT PRIMARY KEY - unique migration identifier
    /// - `applied_at`: TIMESTAMP - when the migration was applied
    ///
    /// # Errors
    ///
    /// Returns `AdminError` if table creation fails
    fn create_migrations_table(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version TEXT PRIMARY KEY,
                applied_at TIMESTAMP NOT NULL DEFAULT NOW()
            )
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!(
                "Failed to create schema_migrations table: {}",
                e
            ))
        })?;

        Ok(())
    }

    /// Checks if a specific migration has already been applied
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection
    /// * `version` - Migration version identifier (e.g., "001_create_webui_admin_credentials")
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Migration has been applied
    /// * `Ok(false)` - Migration has not been applied
    /// * `Err(_)` - Database query failed
    fn is_migration_applied(
        &self,
        conn: &mut PgConnection,
        version: &str,
    ) -> Result<bool, AdminError> {
        #[derive(QueryableByName)]
        struct CountResult {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        let result = sql_query("SELECT COUNT(*) as count FROM schema_migrations WHERE version = $1")
            .bind::<Text, _>(version)
            .get_result::<CountResult>(conn)
            .map_err(|e| {
                AdminError::DatabaseQueryFailed(format!("Failed to check migration status: {}", e))
            })?;

        Ok(result.count > 0)
    }

    /// Runs migration 001: Create webui_admin_credentials table
    ///
    /// This migration creates:
    /// - `webui_admin_credentials` table with username, password_hash, salt, timestamps
    /// - CHECK constraint ensuring username is always "admin"
    /// - Unique index ensuring only one row can exist in the table
    ///
    /// The migration is idempotent - it uses CREATE TABLE IF NOT EXISTS.
    ///
    /// # Errors
    ///
    /// Returns `AdminError::DatabaseMigrationFailed` if table creation fails
    fn run_migration_001(&self, conn: &mut PgConnection) -> Result<(), AdminError> {
        tracing::info!("Running migration 001: Create webui_admin_credentials table");

        sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS webui_admin_credentials (
                username TEXT PRIMARY KEY CHECK (username = 'admin'),
                password_hash TEXT NOT NULL,
                salt TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMP NOT NULL DEFAULT NOW()
            );
            
            CREATE UNIQUE INDEX IF NOT EXISTS idx_webui_admin_single 
            ON webui_admin_credentials ((1));
            "#,
        )
        .execute(conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!(
                "Failed to create webui_admin_credentials table: {}",
                e
            ))
        })?;

        tracing::info!("Migration 001 completed successfully");
        Ok(())
    }

    /// Records a migration as applied in the schema_migrations table
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection
    /// * `version` - Migration version identifier to record
    ///
    /// # Errors
    ///
    /// Returns `AdminError::DatabaseMigrationFailed` if recording fails
    fn record_migration(&self, conn: &mut PgConnection, version: &str) -> Result<(), AdminError> {
        sql_query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, NOW())")
            .bind::<Text, _>(version)
            .execute(conn)
            .map_err(|e| {
                AdminError::DatabaseMigrationFailed(format!("Failed to record migration: {}", e))
            })?;

        tracing::info!("Recorded migration: {}", version);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a PostgreSQL database connection
    // They are integration tests and should be run with a test database

    #[tokio::test]
    #[ignore] // Requires database connection
    async fn test_create_migrations_table() {
        // This test would require setting up a test database
        // Implementation would verify that schema_migrations table is created
    }

    #[tokio::test]
    #[ignore] // Requires database connection
    async fn test_migration_idempotence() {
        // This test would verify that running migrations multiple times
        // produces the same result (idempotence)
    }

    #[tokio::test]
    #[ignore] // Requires database connection
    async fn test_migration_tracking() {
        // This test would verify that applied migrations are correctly
        // tracked in the schema_migrations table
    }
}
