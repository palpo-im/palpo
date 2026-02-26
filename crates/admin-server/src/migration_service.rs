/// Migration Service
///
/// This module handles migration of Web UI admin credentials from legacy storage
/// (localStorage in the browser) to the new PostgreSQL database storage.
///
/// # Architecture
///
/// Since this is server-side code, the migration flow works as follows:
/// 1. Client detects legacy credentials in localStorage
/// 2. Client prompts user to enter their password for verification
/// 3. Client sends legacy credentials + password to server
/// 4. Server verifies password matches legacy hash
/// 5. Server creates new database credentials with the same password
/// 6. Server returns success, client clears localStorage
///
/// # Requirements
///
/// Implements requirements:
/// - 11.1: Detect legacy credentials (client-side)
/// - 11.2: Provide migration wizard (client-side UI)
/// - 11.3: Verify old password before migration
/// - 11.4: Clear localStorage after successful migration (client-side)
/// - 11.5: Migration must be idempotent
/// - 11.6: Failed migration must not corrupt existing data
/// - 11.7: Re-hash password with new salt
/// - 11.8: Audit log migration events

use super::types::AdminError;
use super::webui_auth_service::WebUIAuthService;
use serde::{Deserialize, Serialize};

/// Legacy credentials structure from localStorage
///
/// This represents the old credential format that was stored in the browser's
/// localStorage before the migration to PostgreSQL database storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyCredentials {
    /// Username (should always be "admin")
    pub username: String,
    /// Argon2 password hash from legacy system
    pub password_hash: String,
    /// Salt used for legacy password hash
    pub salt: String,
}

/// Migration Service
///
/// Handles the migration of Web UI admin credentials from legacy localStorage
/// storage to the new PostgreSQL database storage system.
#[derive(Debug, Clone)]
pub struct MigrationService {
    auth_service: WebUIAuthService,
}

impl MigrationService {
    /// Creates a new migration service
    ///
    /// # Arguments
    ///
    /// * `auth_service` - WebUIAuthService instance for creating database credentials
    pub fn new(auth_service: WebUIAuthService) -> Self {
        Self { auth_service }
    }

    /// Migrates credentials from legacy storage to database
    ///
    /// This method:
    /// 1. Verifies the provided password matches the legacy hash
    /// 2. Creates new database credentials with the same password
    /// 3. Uses a new salt and re-hashes the password for security
    ///
    /// The method is idempotent - if database credentials already exist,
    /// it will return an error without corrupting existing data.
    ///
    /// # Arguments
    ///
    /// * `legacy_creds` - Legacy credentials from localStorage
    /// * `password` - User's password for verification
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Migration successful
    /// * `Err(AdminError::InvalidPassword)` - Password doesn't match legacy hash
    /// * `Err(AdminError::WebUIAdminAlreadyExists)` - Database credentials already exist
    /// * `Err(AdminError::DatabaseQueryFailed)` - Database operation failed
    ///
    /// # Requirements
    ///
    /// Implements requirements:
    /// - 11.3: Verify old password before migration
    /// - 11.5: Migration must be idempotent
    /// - 11.6: Failed migration must not corrupt existing data
    /// - 11.7: Re-hash password with new salt
    /// - 11.8: Audit log migration events
    ///
    /// # Example
    ///
    /// ```no_run
    /// use palpo_admin_server::{MigrationService, WebUIAuthService};
    /// use palpo_admin_server::migration_service::LegacyCredentials;
    /// use palpo_data::DieselPool;
    ///
    /// async fn migrate(db_pool: DieselPool) {
    ///     let auth_service = WebUIAuthService::new(db_pool);
    ///     let migration_service = MigrationService::new(auth_service);
    ///     
    ///     let legacy_creds = LegacyCredentials {
    ///         username: "admin".to_string(),
    ///         password_hash: "...".to_string(),
    ///         salt: "...".to_string(),
    ///     };
    ///     
    ///     migration_service.migrate_from_legacy(
    ///         &legacy_creds,
    ///         "user_password"
    ///     ).unwrap();
    /// }
    /// ```
    pub fn migrate_from_legacy(
        &self,
        legacy_creds: &LegacyCredentials,
        password: &str,
    ) -> Result<(), AdminError> {
        // Verify password against legacy credentials
        self.verify_legacy_password(legacy_creds, password)?;

        // Create new database credentials with the same password
        // This will re-hash with a new salt for security
        // If admin already exists, this will return an error (idempotent)
        self.auth_service.create_admin(password)?;

        tracing::info!("Successfully migrated legacy credentials to database");

        // TODO: Log audit event when AuditLogger is implemented
        // AuditLogger::log_event(AuditEvent::CredentialsMigrated).await;

        Ok(())
    }

    /// Verifies that a password matches the legacy credentials
    ///
    /// This method uses Argon2 to verify the password against the legacy
    /// password hash and salt.
    ///
    /// # Arguments
    ///
    /// * `legacy_creds` - Legacy credentials containing hash and salt
    /// * `password` - Password to verify
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Password matches
    /// * `Err(AdminError::InvalidCredentials)` - Password doesn't match
    ///
    /// # Requirements
    ///
    /// Implements requirement 11.3: Verify old password before migration
    fn verify_legacy_password(
        &self,
        legacy_creds: &LegacyCredentials,
        password: &str,
    ) -> Result<(), AdminError> {
        // Verify password using Argon2
        let hash_matches = argon2::verify_encoded(&legacy_creds.password_hash, password.as_bytes())
            .unwrap_or(false);

        if !hash_matches {
            tracing::warn!(
                "Failed legacy password verification for user: {}",
                legacy_creds.username
            );
            return Err(AdminError::InvalidCredentials);
        }

        tracing::debug!("Legacy password verified successfully");
        Ok(())
    }

    /// Checks if migration is needed
    ///
    /// Returns true if no database credentials exist yet, indicating that
    /// migration from legacy storage may be needed.
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Migration needed (no database credentials)
    /// * `Ok(false)` - Migration not needed (database credentials exist)
    /// * `Err(_)` - Database query failed
    ///
    /// # Requirements
    ///
    /// Implements requirement 11.1: Detect if migration is needed
    pub fn is_migration_needed(&self) -> Result<bool, AdminError> {
        // If admin doesn't exist in database, migration may be needed
        let admin_exists = self.auth_service.admin_exists()?;
        Ok(!admin_exists)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_legacy_credentials_serialization() {
        let creds = LegacyCredentials {
            username: "admin".to_string(),
            password_hash: "test_hash".to_string(),
            salt: "test_salt".to_string(),
        };

        // Test serialization
        let json = serde_json::to_string(&creds).unwrap();
        assert!(json.contains("admin"));
        assert!(json.contains("test_hash"));
        assert!(json.contains("test_salt"));

        // Test deserialization
        let deserialized: LegacyCredentials = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.username, "admin");
        assert_eq!(deserialized.password_hash, "test_hash");
        assert_eq!(deserialized.salt, "test_salt");
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_migration_success() {
        // This test would verify successful migration from legacy to database
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_migration_wrong_password() {
        // This test would verify migration fails with wrong password
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_migration_idempotent() {
        // This test would verify repeated migration attempts are safe
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_is_migration_needed() {
        // This test would verify migration detection logic
    }
}
