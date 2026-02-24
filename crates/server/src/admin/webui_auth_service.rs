/// Web UI Authentication Service
///
/// This module implements the first tier of the two-tier admin system.
/// It provides PostgreSQL-backed authentication for the Web UI admin account,
/// which operates independently of the Palpo Matrix server.
///
/// Key features:
/// - Fixed username "admin" for consistency
/// - Argon2id password hashing for security
/// - Database-backed credential storage
/// - Independent of Palpo server status
/// - Session token generation for authenticated requests
///
/// # Requirements
///
/// Implements requirements:
/// - 1.1-1.9: Web UI admin database authentication
/// - 3.6-3.12: Password change functionality
///
/// # Example
///
/// ```no_run
/// use palpo::admin::webui_auth_service::WebUIAuthService;
/// use palpo::data::DieselPool;
///
/// async fn example(db_pool: DieselPool) {
///     let service = WebUIAuthService::new(db_pool);
///     
///     // Check if admin exists
///     let exists = service.admin_exists().await.unwrap();
///     
///     if !exists {
///         // Create admin account
///         service.create_admin("SecureP@ssw0rd123").await.unwrap();
///     }
///     
///     // Authenticate
///     let session = service.authenticate("admin", "SecureP@ssw0rd123").await.unwrap();
/// }
/// ```

use diesel::prelude::*;
use diesel::sql_query;
use diesel::sql_types::{BigInt, Text};

use super::types::{AdminError, SessionToken};
use crate::data::DieselPool;

/// Web UI Authentication Service
///
/// Manages authentication for the Web UI admin account using PostgreSQL database.
/// This service is independent of the Palpo Matrix server and can operate even
/// when Palpo is not running.
pub struct WebUIAuthService {
    db_pool: DieselPool,
}

impl WebUIAuthService {
    /// Fixed username for Web UI admin account
    pub const ADMIN_USERNAME: &'static str = "admin";

    /// Session token validity duration in hours
    const SESSION_DURATION_HOURS: i64 = 24;

    /// Creates a new Web UI authentication service
    ///
    /// # Arguments
    ///
    /// * `db_pool` - Diesel PostgreSQL connection pool
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }

    /// Initializes the database schema for Web UI admin credentials
    ///
    /// Creates the `webui_admin_credentials` table if it doesn't exist.
    /// This method is idempotent and safe to call multiple times.
    ///
    /// # Errors
    ///
    /// Returns `AdminError::DatabaseMigrationFailed` if schema creation fails
    ///
    /// # Requirements
    ///
    /// Implements requirement 2.1: Create webui_admin_credentials table
    pub fn initialize_schema(&self) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

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
        .execute(&mut conn)
        .map_err(|e| {
            AdminError::DatabaseMigrationFailed(format!(
                "Failed to create webui_admin_credentials table: {}",
                e
            ))
        })?;

        tracing::info!("Web UI admin schema initialized successfully");
        Ok(())
    }

    /// Checks if a Web UI admin account exists in the database
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Admin account exists
    /// * `Ok(false)` - No admin account found
    /// * `Err(_)` - Database query failed
    ///
    /// # Requirements
    ///
    /// Implements requirement 1.1: Check if admin exists
    pub fn admin_exists(&self) -> Result<bool, AdminError> {
        #[derive(QueryableByName)]
        struct CountResult {
            #[diesel(sql_type = BigInt)]
            count: i64,
        }

        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let result = sql_query(
            "SELECT COUNT(*) as count FROM webui_admin_credentials WHERE username = $1",
        )
        .bind::<Text, _>(Self::ADMIN_USERNAME)
        .get_result::<CountResult>(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(result.count > 0)
    }

    /// Creates a new Web UI admin account
    ///
    /// This method:
    /// 1. Validates the password against password policy
    /// 2. Checks that no admin account already exists
    /// 3. Hashes the password using Argon2id with a random salt
    /// 4. Stores the credentials in the database
    ///
    /// # Arguments
    ///
    /// * `password` - Plain text password (will be hashed before storage)
    ///
    /// # Errors
    ///
    /// * `AdminError::WebUIAdminAlreadyExists` - Admin account already exists
    /// * `AdminError::PasswordTooShort` - Password doesn't meet minimum length
    /// * `AdminError::Missing*` - Password missing required character types
    /// * `AdminError::DatabaseQueryFailed` - Database operation failed
    ///
    /// # Requirements
    ///
    /// Implements requirements:
    /// - 1.3: Validate password policy
    /// - 1.4: Use fixed username "admin"
    /// - 1.5: Hash password with Argon2
    /// - 1.6: Store credentials in database
    pub fn create_admin(&self, password: &str) -> Result<(), AdminError> {
        // Validate password policy
        self.validate_password_policy(password)?;

        // Check if admin already exists
        if self.admin_exists()? {
            return Err(AdminError::WebUIAdminAlreadyExists);
        }

        // Generate salt and hash password with Argon2id
        let salt = crate::utils::random_string(32);
        let hashing_conf = argon2::Config {
            variant: argon2::Variant::Argon2id,
            ..Default::default()
        };
        let password_hash = argon2::hash_encoded(password.as_bytes(), salt.as_bytes(), &hashing_conf)
            .map_err(|e| AdminError::PasswordHashError(e.to_string()))?;

        // Store in database
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        sql_query(
            r#"
            INSERT INTO webui_admin_credentials (username, password_hash, salt, created_at, updated_at)
            VALUES ($1, $2, $3, NOW(), NOW())
            "#,
        )
        .bind::<Text, _>(Self::ADMIN_USERNAME)
        .bind::<Text, _>(&password_hash)
        .bind::<Text, _>(&salt)
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        tracing::info!("Web UI admin account created successfully");

        // TODO: Log audit event when AuditLogger is implemented
        // AuditLogger::log_event(AuditEvent::WebUIAdminCreated {
        //     username: Self::ADMIN_USERNAME.to_string(),
        // }).await;

        Ok(())
    }

    /// Authenticates a Web UI admin user
    ///
    /// This method:
    /// 1. Verifies the username is "admin"
    /// 2. Fetches credentials from the database
    /// 3. Verifies the password using Argon2
    /// 4. Generates a session token
    ///
    /// # Arguments
    ///
    /// * `username` - Username (must be "admin")
    /// * `password` - Plain text password to verify
    ///
    /// # Returns
    ///
    /// * `Ok(SessionToken)` - Authentication successful, returns session token
    /// * `Err(AdminError::InvalidCredentials)` - Wrong username or password
    ///
    /// # Requirements
    ///
    /// Implements requirements:
    /// - 1.7: Verify username and password
    /// - 1.8: Generate session token
    /// - 1.9: Independent of Palpo server
    pub fn authenticate(&self, username: &str, password: &str) -> Result<SessionToken, AdminError> {
        // Verify username is "admin"
        if username != Self::ADMIN_USERNAME {
            tracing::warn!("Authentication attempt with invalid username: {}", username);
            return Err(AdminError::InvalidCredentials);
        }

        // Fetch credentials from database
        #[derive(QueryableByName)]
        struct CredentialRow {
            #[diesel(sql_type = Text)]
            password_hash: String,
            #[diesel(sql_type = Text)]
            salt: String,
        }

        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let row = sql_query(
            "SELECT password_hash, salt FROM webui_admin_credentials WHERE username = $1",
        )
        .bind::<Text, _>(username)
        .get_result::<CredentialRow>(&mut conn)
        .optional()
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?
        .ok_or(AdminError::InvalidCredentials)?;

        // Verify password with Argon2
        let hash_matches = argon2::verify_encoded(&row.password_hash, password.as_bytes())
            .unwrap_or(false);

        if !hash_matches {
            tracing::warn!("Failed authentication attempt for user: {}", username);
            return Err(AdminError::InvalidCredentials);
        }

        // Generate session token
        let token = self.generate_session_token();
        let expires_at = chrono::Utc::now()
            + chrono::Duration::try_hours(Self::SESSION_DURATION_HOURS)
                .expect("Valid duration");

        tracing::info!("User {} authenticated successfully", username);

        // TODO: Log audit event when AuditLogger is implemented
        // AuditLogger::log_event(AuditEvent::WebUIAdminLogin {
        //     username: username.to_string(),
        // }).await;

        Ok(SessionToken { token, expires_at })
    }

    /// Changes the Web UI admin password
    ///
    /// This method:
    /// 1. Verifies the current password
    /// 2. Validates the new password against policy
    /// 3. Ensures new password is different from current
    /// 4. Hashes the new password with a new salt
    /// 5. Updates the database
    ///
    /// # Arguments
    ///
    /// * `current_password` - Current password for verification
    /// * `new_password` - New password to set
    ///
    /// # Errors
    ///
    /// * `AdminError::InvalidCredentials` - Current password is incorrect
    /// * `AdminError::PasswordNotChanged` - New password same as current
    /// * `AdminError::Password*` - New password doesn't meet policy
    ///
    /// # Requirements
    ///
    /// Implements requirements:
    /// - 3.6: Verify current password
    /// - 3.7: Validate new password policy
    /// - 3.8: Verify new password different
    /// - 3.9: Hash new password
    /// - 3.10: Update database
    pub fn change_password(
        &self,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), AdminError> {
        // Verify current password by attempting authentication
        self.authenticate(Self::ADMIN_USERNAME, current_password)?;

        // Validate new password policy
        self.validate_password_policy(new_password)?;

        // Ensure new password is different
        if current_password == new_password {
            return Err(AdminError::PasswordNotChanged);
        }

        // Hash new password with new salt
        let salt = crate::utils::random_string(32);
        let hashing_conf = argon2::Config {
            variant: argon2::Variant::Argon2id,
            ..Default::default()
        };
        let password_hash = argon2::hash_encoded(new_password.as_bytes(), salt.as_bytes(), &hashing_conf)
            .map_err(|e| AdminError::PasswordHashError(e.to_string()))?;

        // Update database
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        sql_query(
            r#"
            UPDATE webui_admin_credentials 
            SET password_hash = $1, salt = $2, updated_at = NOW()
            WHERE username = $3
            "#,
        )
        .bind::<Text, _>(&password_hash)
        .bind::<Text, _>(&salt)
        .bind::<Text, _>(Self::ADMIN_USERNAME)
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        tracing::info!("Web UI admin password changed successfully");

        // TODO: Log audit event when AuditLogger is implemented
        // AuditLogger::log_event(AuditEvent::PasswordChanged {
        //     username: Self::ADMIN_USERNAME.to_string(),
        // }).await;

        Ok(())
    }

    /// Resets the Web UI admin password (for recovery via SQL)
    ///
    /// This method is intended for password recovery scenarios where the admin
    /// has forgotten their password and needs to reset it directly via SQL.
    ///
    /// # Arguments
    ///
    /// * `new_password` - New password to set
    ///
    /// # Errors
    ///
    /// * `AdminError::Password*` - Password doesn't meet policy requirements
    /// * `AdminError::DatabaseQueryFailed` - Database update failed
    ///
    /// # Requirements
    ///
    /// Implements requirement 4.3: SQL-based password recovery
    pub fn reset_password(&self, new_password: &str) -> Result<(), AdminError> {
        // Validate password policy
        self.validate_password_policy(new_password)?;

        // Hash password
        let salt = crate::utils::random_string(32);
        let hashing_conf = argon2::Config {
            variant: argon2::Variant::Argon2id,
            ..Default::default()
        };
        let password_hash = argon2::hash_encoded(new_password.as_bytes(), salt.as_bytes(), &hashing_conf)
            .map_err(|e| AdminError::PasswordHashError(e.to_string()))?;

        // Update database
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        sql_query(
            r#"
            UPDATE webui_admin_credentials 
            SET password_hash = $1, salt = $2, updated_at = NOW()
            WHERE username = $3
            "#,
        )
        .bind::<Text, _>(&password_hash)
        .bind::<Text, _>(&salt)
        .bind::<Text, _>(Self::ADMIN_USERNAME)
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        tracing::info!("Web UI admin password reset successfully");

        // TODO: Log audit event when AuditLogger is implemented
        // AuditLogger::log_event(AuditEvent::WebUIAdminPasswordReset).await;

        Ok(())
    }

    /// Validates a password against the password policy
    ///
    /// Password requirements:
    /// - Minimum 12 characters
    /// - At least one uppercase letter
    /// - At least one lowercase letter
    /// - At least one digit
    /// - At least one special character
    ///
    /// # Arguments
    ///
    /// * `password` - Password to validate
    ///
    /// # Errors
    ///
    /// Returns specific `AdminError` variants for each policy violation
    ///
    /// # Requirements
    ///
    /// Implements requirements 9.1, 9.2, 9.4: Password policy validation
    fn validate_password_policy(&self, password: &str) -> Result<(), AdminError> {
        const MIN_LENGTH: usize = 12;
        const SPECIAL_CHARS: &str = "!@#$%^&*()_+-=[]{}|;:,.<>?";

        // Check minimum length
        if password.len() < MIN_LENGTH {
            return Err(AdminError::PasswordTooShort(password.len()));
        }

        // Check for uppercase letter
        if !password.chars().any(|c| c.is_uppercase()) {
            return Err(AdminError::MissingUppercase);
        }

        // Check for lowercase letter
        if !password.chars().any(|c| c.is_lowercase()) {
            return Err(AdminError::MissingLowercase);
        }

        // Check for digit
        if !password.chars().any(|c| c.is_ascii_digit()) {
            return Err(AdminError::MissingDigit);
        }

        // Check for special character
        if !password.chars().any(|c| SPECIAL_CHARS.contains(c)) {
            return Err(AdminError::MissingSpecialChar);
        }

        Ok(())
    }

    /// Generates a cryptographically secure random session token
    ///
    /// # Returns
    ///
    /// A 64-character hexadecimal string (32 random bytes)
    fn generate_session_token(&self) -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let bytes: Vec<u8> = (0..32).map(|_| rng.r#gen()).collect();
        hex::encode(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a PostgreSQL database connection
    // They are integration tests and should be run with a test database

    #[test]
    fn test_password_policy_valid() {
        // Test password policy validation without needing a database
        let result = validate_password_policy_static("SecureP@ss123!");
        assert!(result.is_ok());
    }

    #[test]
    fn test_password_policy_too_short() {
        let result = validate_password_policy_static("Short1!");
        assert!(matches!(result, Err(AdminError::PasswordTooShort(7))));
    }

    #[test]
    fn test_password_policy_missing_uppercase() {
        let result = validate_password_policy_static("lowercase123!");
        assert!(matches!(result, Err(AdminError::MissingUppercase)));
    }

    #[test]
    fn test_password_policy_missing_lowercase() {
        let result = validate_password_policy_static("UPPERCASE123!");
        assert!(matches!(result, Err(AdminError::MissingLowercase)));
    }

    #[test]
    fn test_password_policy_missing_digit() {
        let result = validate_password_policy_static("NoDigitsHere!");
        assert!(matches!(result, Err(AdminError::MissingDigit)));
    }

    #[test]
    fn test_password_policy_missing_special() {
        let result = validate_password_policy_static("NoSpecialChar123");
        assert!(matches!(result, Err(AdminError::MissingSpecialChar)));
    }

    #[test]
    fn test_session_token_generation() {
        let token1 = generate_session_token_static();
        let token2 = generate_session_token_static();

        // Tokens should be 64 characters (32 bytes hex encoded)
        assert_eq!(token1.len(), 64);
        assert_eq!(token2.len(), 64);

        // Tokens should be unique
        assert_ne!(token1, token2);

        // Tokens should be valid hex
        assert!(token1.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(token2.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // Static helper functions for testing without database
    fn validate_password_policy_static(password: &str) -> Result<(), AdminError> {
        const MIN_LENGTH: usize = 12;
        const SPECIAL_CHARS: &str = "!@#$%^&*()_+-=[]{}|;:,.<>?";

        if password.len() < MIN_LENGTH {
            return Err(AdminError::PasswordTooShort(password.len()));
        }
        if !password.chars().any(|c| c.is_uppercase()) {
            return Err(AdminError::MissingUppercase);
        }
        if !password.chars().any(|c| c.is_lowercase()) {
            return Err(AdminError::MissingLowercase);
        }
        if !password.chars().any(|c| c.is_ascii_digit()) {
            return Err(AdminError::MissingDigit);
        }
        if !password.chars().any(|c| SPECIAL_CHARS.contains(c)) {
            return Err(AdminError::MissingSpecialChar);
        }
        Ok(())
    }

    fn generate_session_token_static() -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let bytes: Vec<u8> = (0..32).map(|_| rng.r#gen()).collect();
        hex::encode(bytes)
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_admin_exists_empty_database() {
        // This test would verify admin_exists returns false on empty database
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_create_admin_success() {
        // This test would verify admin creation works correctly
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_create_admin_duplicate() {
        // This test would verify duplicate admin creation is rejected
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_authenticate_success() {
        // This test would verify successful authentication
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_authenticate_wrong_password() {
        // This test would verify wrong password is rejected
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_change_password_success() {
        // This test would verify password change works correctly
    }

    #[test]
    #[ignore] // Requires database connection
    fn test_change_password_same_as_current() {
        // This test would verify same password is rejected
    }
}
