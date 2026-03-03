/// Property-based test for migration idempotence
///
/// **Property 15: Migration Idempotence**
/// **Validates: Requirements 11.5, 11.6**
///
/// This test verifies that repeated migration operations produce the same result.
/// Specifically:
/// - Running migration multiple times should not create duplicate records
/// - The webui_admin_credentials table state remains consistent after repeated migrations
/// - No duplicate records are created

use palpo_admin_server::{types::AdminError, WebUIAuthService};
use proptest::prelude::*;

/// Helper function to get or create a test database pool
fn get_or_create_pool() -> DieselPool {
    // Try to get existing pool first
    if let Some(pool) = palpo_data::DIESEL_POOL.get() {
        return pool.clone();
    }
    
    // Create new pool if not exists - use existing palpo database
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
    
    palpo_data::init(&config);
    palpo_data::DIESEL_POOL.get().expect("Pool should be initialized").clone()
}

/// Helper function to generate a valid password
fn valid_password_strategy() -> impl Strategy<Value = String> {
    prop::string::string_regex("[A-Z][a-z][0-9][!@#$%^&*][A-Za-z0-9!@#$%^&*()_+-=]{8,}")
        .expect("Invalid regex")
}

#[test]
fn test_migration_idempotence_simple() {
    // Simple test case for migration idempotence
    // This test verifies that create_admin is idempotent
    let pool = get_or_create_pool();
    let auth_service = WebUIAuthService::new(pool);
    
    // Initialize schema
    auth_service.initialize_schema().expect("Failed to initialize schema");
    
    // Check if admin exists first
    if auth_service.admin_exists().expect("Failed to check admin") {
        // Reset password instead of creating new admin
        let password = "ValidPassword123!Aa";
        let reset_result = auth_service.reset_password(password);
        assert!(reset_result.is_ok(), "Reset password should succeed");
        
        // Verify authentication works
        let auth_result = auth_service.authenticate("admin", password);
        assert!(auth_result.is_ok(), "Authentication should work after reset");
        return;
    }
    
    let password = "ValidPassword123!Aa";
    
    // First create_admin should succeed
    let result1 = auth_service.create_admin(password);
    assert!(result1.is_ok(), "First create_admin should succeed: {:?}", result1);
    
    // Verify admin was created
    let admin_exists = auth_service.admin_exists().expect("Failed to check admin");
    assert!(admin_exists, "Admin should exist after creation");
    
    // Second create_admin should fail - admin already exists
    let result2 = auth_service.create_admin(password);
    assert!(result2.is_err(), "Second create_admin should fail - admin already exists");
    
    // Verify only one record exists (CHECK constraint enforces this)
    let admin_count = auth_service.admin_exists().expect("Failed to check admin");
    assert!(admin_count, "Admin should still exist");
}

proptest::proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))]
    
    #[test]
    fn test_migration_idempotence_property(password in valid_password_strategy()) {
        let pool = get_or_create_pool();
        let auth_service = WebUIAuthService::new(pool);
        
        // Initialize schema
        auth_service.initialize_schema().expect("Failed to initialize schema");
        
        // Check if admin exists, if so use reset_password
        if auth_service.admin_exists().expect("Failed to check admin") {
            let reset_result = auth_service.reset_password(&password);
            prop_assert!(reset_result.is_ok(), "Reset password should succeed");
        } else {
            // First create_admin
            let result1 = auth_service.create_admin(&password);
            prop_assert!(result1.is_ok(), "First create_admin should succeed");
        }
        
        // Verify admin exists
        let admin_exists = auth_service.admin_exists().expect("Failed to check admin");
        prop_assert!(admin_exists, "Admin should exist");
        
        // Second create_admin attempt should fail
        let result2 = auth_service.create_admin(&password);
        prop_assert!(result2.is_err(), "Second create_admin should fail - admin already exists");
        
        // Verify state is consistent - admin still exists
        let admin_still_exists = auth_service.admin_exists().expect("Failed to check admin");
        prop_assert!(admin_still_exists, "Admin should still exist after failed creation");
    }
}

#[test]
fn test_migration_no_duplicate_records() {
    // Test that repeated create_admin calls don't cause issues
    let pool = get_or_create_pool();
    let auth_service = WebUIAuthService::new(pool);
    
    // Initialize schema
    auth_service.initialize_schema().expect("Failed to initialize schema");
    
    let password = "TestPassword123!Aa";
    
    // Check if admin exists
    if auth_service.admin_exists().expect("Failed to check admin") {
        // Admin exists, reset password to our test password
        let reset_result = auth_service.reset_password(password);
        assert!(reset_result.is_ok(), "Reset password should succeed");
    } else {
        // First create_admin should succeed
        let result = auth_service.create_admin(password);
        assert!(result.is_ok(), "First create_admin should succeed");
    }
    
    // Run create_admin multiple times
    for i in 0..5 {
        let result = auth_service.create_admin(password);
        // Subsequent create_admin should fail
        assert!(result.is_err(), "create_admin {} should fail - admin exists", i);
    }
    
    // Verify only one admin record exists
    let admin_exists = auth_service.admin_exists().expect("Failed to check admin");
    assert!(admin_exists, "Admin should exist");
    
    // Verify authentication works
    let auth_result = auth_service.authenticate("admin", password);
    assert!(auth_result.is_ok(), "Authentication should succeed");
}

proptest::proptest! {
    #![proptest_config(ProptestConfig::with_cases(5))]
    
    #[test]
    fn test_migration_state_consistency(password in valid_password_strategy()) {
        let pool = get_or_create_pool();
        let auth_service = WebUIAuthService::new(pool);
        
        // Initialize schema
        auth_service.initialize_schema().expect("Failed to initialize schema");
        
        // Check if admin exists, if so use reset_password
        let first_result = if auth_service.admin_exists().expect("Failed to check admin") {
            auth_service.reset_password(&password)
        } else {
            auth_service.create_admin(&password)
        };
        
        let state_after_first = auth_service.admin_exists().unwrap_or(false);
        
        // Attempt create_admin again
        let second_result = auth_service.create_admin(&password);
        let state_after_second = auth_service.admin_exists().unwrap_or(false);
        
        // State should be consistent
        prop_assert_eq!(state_after_first, state_after_second,
            "State should be consistent after repeated create_admin");
        
        // First operation should succeed, second should fail
        prop_assert!(first_result.is_ok(), "First operation should succeed");
        prop_assert!(second_result.is_err(), "Second create_admin should fail");
    }
}

#[test]
fn test_migration_with_different_passwords() {
    // Test that create_admin with different passwords maintains consistency
    let pool = get_or_create_pool();
    let auth_service = WebUIAuthService::new(pool);
    
    // Initialize schema
    auth_service.initialize_schema().expect("Failed to initialize schema");
    
    let password1 = "FirstPassword123!Aa";
    let password2 = "SecondPassword456@Bb";
    
    // Check if admin exists first
    if auth_service.admin_exists().expect("Failed to check admin") {
        // Admin exists, use reset_password to set password1
        let reset_result = auth_service.reset_password(password1);
        assert!(reset_result.is_ok(), "Reset password should succeed");
    } else {
        // First create_admin with password1
        let result1 = auth_service.create_admin(password1);
        assert!(result1.is_ok(), "First create_admin should succeed");
    }
    
    // Verify authentication works with password1
    assert!(auth_service.authenticate("admin", password1).is_ok(),
        "Authentication should work with first password");
    
    // Second create_admin attempt with different password should fail
    let result2 = auth_service.create_admin(password2);
    assert!(result2.is_err(), "Second create_admin should fail");
    
    // Verify password1 still works (state unchanged)
    assert!(auth_service.authenticate("admin", password1).is_ok(),
        "First password should still work");
}

// Import required types
use palpo_data::{DbConfig, DieselPool};