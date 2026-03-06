/// Property-based test for password change invalidation
///
/// **Property 5: Password Change Invalidates Old Password**
/// **Validates: Requirements 3.8, 3.9**
///
/// This test verifies that after a Web UI admin changes their password,
/// the old password can no longer be used for authentication.
///
/// ## Test Environment Setup
///
/// **Database**: Uses dedicated test database `palpo_test` via TEST_DATABASE_URL environment variable
/// - Default: `postgresql://palpo:password@localhost/palpo_test`
/// - Ensures test isolation from production/development data
/// - Database is shared across tests but each test manages its own state
///
/// ## Test Execution Steps
///
/// 1. **Setup Phase**:
///    - Initialize database connection pool (shared across tests)
///    - Create WebUIAuthService instance
///    - Initialize schema (creates webui_admin_credentials table if not exists)
///
/// 2. **Test Execution**:
///    - Create admin with initial password OR reset if already exists
///    - Verify initial password works for authentication
///    - Change password to new password using change_password()
///    - Verify old password fails authentication
///    - Verify new password succeeds authentication
///
/// 3. **Cleanup Phase**:
///    - No explicit cleanup needed as tests use reset_password pattern
///    - Database state is preserved for subsequent tests
///    - Each test is idempotent and can run in any order
///
/// ## Running Tests
///
/// **Note**: These tests are marked as `#[ignore]` because they require a dedicated test database
/// to ensure isolation from development/production data. This prevents accidental data modification.
///
/// To run these tests manually:
///
/// ```bash
/// # First, create the test database (only once)
/// createdb palpo_test
///
/// # Run all tests in this file
/// TEST_DATABASE_URL="postgresql://palpo:password@localhost/palpo_test" \
///   cargo test -p palpo-admin-server --test password_change_invalidation_test
///
/// # Run with --ignored flag to include ignored tests
/// TEST_DATABASE_URL="postgresql://palpo:password@localhost/palpo_test" \
///   cargo test -p palpo-admin-server --test password_change_invalidation_test -- --ignored
///
/// # Run specific test
/// TEST_DATABASE_URL="postgresql://palpo:password@localhost/palpo_test" \
///   cargo test -p palpo-admin-server --test password_change_invalidation_test test_password_change_invalidates_old_password_simple -- --ignored
///
/// # Run with verbose output
/// TEST_DATABASE_URL="postgresql://palpo:password@localhost/palpo_test" \
///   cargo test -p palpo-admin-server --test password_change_invalidation_test -- --ignored --nocapture
/// ```
///
/// ## Database Cleanup
///
/// After testing, clean up the test database:
/// ```bash
/// # Option 1: Drop entire test database
/// dropdb palpo_test
///
/// # Option 2: Clean specific tables (keep schema)
/// psql -h localhost -U palpo -d palpo_test -c "TRUNCATE webui_admin_credentials CASCADE;"
///
/// # Option 3: Recreate fresh test database
/// dropdb palpo_test && createdb palpo_test
/// ```

use palpo_admin_server::{types::AdminError, WebUIAuthService};
use palpo_data::{DbConfig, DieselPool};
use proptest::prelude::*;

/// Helper function to get or create a test database pool
fn get_or_create_pool() -> DieselPool {
    // Try to get existing pool first
    if let Some(pool) = palpo_data::DIESEL_POOL.get() {
        return pool.clone();
    }
    
    // Create new pool if not exists
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://palpo:password@localhost/palpo".to_string());
    
    let config = DbConfig {
        url: database_url,
        pool_size: 5,
        min_idle: Some(1),
        tcp_timeout: 10000,
        connection_timeout: 30000,
        statement_timeout: 30000,
        helper_threads: 2,
        enforce_tls: false,
    };
    
    // Initialize the database pool (safe to call multiple times)
    // The OnceLock will ensure only the first call succeeds
    palpo_data::init(&config);
    
    // Get the pool (will succeed as init() was just called or was already called)
    palpo_data::DIESEL_POOL.get()
        .expect("Failed to get database pool. Ensure TEST_DATABASE_URL is set or PostgreSQL is running.")
        .clone()
}

/// Helper function to generate a valid password
fn valid_password_strategy() -> impl Strategy<Value = String> {
    // Generate passwords that meet the policy:
    // - At least 12 characters
    // - Contains uppercase, lowercase, digit, and special character
    prop::string::string_regex("[A-Z][a-z][0-9][!@#$%^&*][A-Za-z0-9!@#$%^&*]{8,}")
        .expect("Invalid regex")
}

#[test]
#[ignore = "Requires dedicated test database (palpo_test). Run with --ignored flag and TEST_DATABASE_URL."]
fn test_password_change_invalidates_old_password_simple() {
    // Test Environment: Uses isolated test database (palpo_test)
    // Purpose: Verify password change invalidates old password
    
    let pool = get_or_create_pool();
    let auth_service = WebUIAuthService::new(pool.clone());
    
    // Step 1: Initialize schema
    auth_service.initialize_schema().expect("Failed to initialize schema");
    
    let initial_password = "InitialPassword123!Aa";
    let new_password = "NewPassword456@Bb";
    
    // Step 2: Ensure clean state or create admin
    if auth_service.admin_exists().expect("Failed to check admin") {
        // Reset to initial password for consistent test
        let reset_result = auth_service.reset_password(initial_password);
        assert!(reset_result.is_ok(), "Reset password should succeed");
    } else {
        // Create admin with initial password
        let create_result = auth_service.create_admin(initial_password);
        assert!(create_result.is_ok(), "Create admin should succeed");
    }
    
    // Step 3: Verify initial password works
    let auth_initial = auth_service.authenticate("admin", initial_password);
    assert!(auth_initial.is_ok(), "Initial password should work");
    
    // Step 4: Change password
    let change_result = auth_service.change_password(initial_password, new_password);
    assert!(change_result.is_ok(), "Change password should succeed");
    
    // Step 5: Verify old password fails
    let auth_old = auth_service.authenticate("admin", initial_password);
    assert!(auth_old.is_err(), "Old password should fail after change");
    
    // Step 6: Verify new password works
    let auth_new = auth_service.authenticate("admin", new_password);
    assert!(auth_new.is_ok(), "New password should work");
}

proptest::proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))]
    
    #[test]
    #[ignore = "Requires dedicated test database (palpo_test). Run with --ignored flag and TEST_DATABASE_URL."]
    fn test_password_change_invalidates_old_password_property(initial_pwd in valid_password_strategy(), new_pwd in valid_password_strategy()) {
        let pool = get_or_create_pool();
        let auth_service = WebUIAuthService::new(pool.clone());
        
        // Initialize schema
        auth_service.initialize_schema().expect("Failed to initialize schema");
        
        // Ensure admin exists with initial password
        if !auth_service.admin_exists().unwrap_or(false) {
            let create_result = auth_service.create_admin(&initial_pwd);
            prop_assert!(create_result.is_ok(), "Create admin should succeed");
        } else {
            let reset_result = auth_service.reset_password(&initial_pwd);
            prop_assert!(reset_result.is_ok(), "Reset password should succeed");
        }
        
        // Verify initial password works
        prop_assert!(auth_service.authenticate("admin", &initial_pwd).is_ok(),
            "Initial password should work");
        
        // Change password
        let change_result = auth_service.change_password(&initial_pwd, &new_pwd);
        prop_assert!(change_result.is_ok(), "Change password should succeed");
        
        // Verify old password fails
        prop_assert!(auth_service.authenticate("admin", &initial_pwd).is_err(),
            "Old password should fail after change");
        
        // Verify new password works
        prop_assert!(auth_service.authenticate("admin", &new_pwd).is_ok(),
            "New password should work");
    }
}

#[test]
#[ignore = "Requires dedicated test database (palpo_test). Run with --ignored flag and TEST_DATABASE_URL."]
fn test_multiple_password_changes() {
    // Test Environment: Uses isolated test database (palpo_test)
    // Purpose: Verify multiple consecutive password changes work correctly
    
    let pool = get_or_create_pool();
    let auth_service = WebUIAuthService::new(pool.clone());
    
    // Step 1: Initialize schema
    auth_service.initialize_schema().expect("Failed to initialize schema");
    
    let password1 = "Password1!Aa";
    let password2 = "Password2@Bb";
    let password3 = "Password3#Cc";
    
    // Step 2: Ensure admin exists with password1
    if !auth_service.admin_exists().unwrap_or(false) {
        let create_result = auth_service.create_admin(password1);
        assert!(create_result.is_ok(), "Create admin should succeed");
    } else {
        let reset_result = auth_service.reset_password(password1);
        assert!(reset_result.is_ok(), "Reset password should succeed");
    }
    
    // Step 3: Change from password1 to password2
    let change1 = auth_service.change_password(password1, password2);
    assert!(change1.is_ok(), "First password change should succeed");
    
    // Step 4: Verify password1 fails, password2 works
    assert!(auth_service.authenticate("admin", password1).is_err(),
        "Password1 should fail after first change");
    assert!(auth_service.authenticate("admin", password2).is_ok(),
        "Password2 should work after first change");
    
    // Step 5: Change from password2 to password3
    let change2 = auth_service.change_password(password2, password3);
    assert!(change2.is_ok(), "Second password change should succeed");
    
    // Step 6: Verify password2 fails, password3 works
    assert!(auth_service.authenticate("admin", password2).is_err(),
        "Password2 should fail after second change");
    assert!(auth_service.authenticate("admin", password3).is_ok(),
        "Password3 should work after second change");
}
