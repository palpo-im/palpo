/// Property-based test for password change invalidation
///
/// **Property 5: Password Change Invalidates Old Password**
/// **Validates: Requirements 3.8, 3.9**
///
/// This test verifies that after a Web UI admin changes their password,
/// the old password can no longer be used for authentication.
///
/// Test strategy:
/// 1. Create admin with initial password
/// 2. Authenticate successfully with initial password
/// 3. Change password to a new password
/// 4. Verify old password fails authentication
/// 5. Verify new password succeeds authentication

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
    
    palpo_data::init(&config);
    palpo_data::DIESEL_POOL.get().expect("Pool should be initialized").clone()
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
fn test_password_change_invalidates_old_password_simple() {
    // Simple non-property-based test case
    let pool = get_or_create_pool();
    let service = WebUIAuthService::new(pool);
    
    // Initialize schema
    service.initialize_schema().expect("Failed to initialize schema");
    
    // Create admin with initial password (or reset if already exists)
    let initial_password = "InitialPass123!";
    if service.admin_exists().expect("Failed to check admin") {
        service.reset_password(initial_password).expect("Failed to reset password");
    } else {
        service.create_admin(initial_password).expect("Failed to create admin");
    }
    
    // Verify initial password works
    let result = service.authenticate("admin", initial_password);
    assert!(result.is_ok(), "Initial authentication should succeed");
    
    // Change password
    let new_password = "NewPassword456@";
    service.change_password(initial_password, new_password)
        .expect("Failed to change password");
    
    // Verify old password fails
    let result = service.authenticate("admin", initial_password);
    assert!(result.is_err(), "Old password should fail after change");
    match result {
        Err(AdminError::InvalidCredentials) => {
            // Expected error
        }
        _ => panic!("Expected InvalidCredentials error"),
    }
    
    // Verify new password works
    let result = service.authenticate("admin", new_password);
    assert!(result.is_ok(), "New password should succeed");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))] // Reduce cases for faster testing
    
    #[test]
    fn test_password_change_invalidates_old_password_property(
        initial_password in valid_password_strategy(),
        new_password in valid_password_strategy(),
    ) {
        // Ensure passwords are different
        prop_assume!(initial_password != new_password);
        
        let pool = get_or_create_pool();
        let service = WebUIAuthService::new(pool);
        
        // Initialize schema
        service.initialize_schema().expect("Failed to initialize schema");
        
        // Create admin with initial password (or reset if already exists)
        if service.admin_exists().expect("Failed to check admin") {
            // Reset password using reset_password which doesn't require current password
            service.reset_password(&initial_password).expect("Failed to reset password");
        } else {
            service.create_admin(&initial_password).expect("Failed to create admin");
        }
        
        // Verify initial password works
        let result = service.authenticate("admin", &initial_password);
        prop_assert!(result.is_ok(), "Initial authentication should succeed");
        
        // Change password
        service.change_password(&initial_password, &new_password)
            .expect("Failed to change password");
        
        // Verify old password fails
        let result = service.authenticate("admin", &initial_password);
        prop_assert!(result.is_err(), "Old password should fail after change");
        
        // Verify new password works
        let result = service.authenticate("admin", &new_password);
        prop_assert!(result.is_ok(), "New password should succeed");
    }
}

#[test]
fn test_multiple_password_changes() {
    // Test that multiple password changes work correctly
    let pool = get_or_create_pool();
    let service = WebUIAuthService::new(pool);
    
    service.initialize_schema().expect("Failed to initialize schema");
    
    let passwords = vec![
        "Password1!Aa",
        "Password2@Bb",
        "Password3#Cc",
        "Password4$Dd",
    ];
    
    // Create admin with first password (or reset if already exists)
    if service.admin_exists().expect("Failed to check admin") {
        service.reset_password(passwords[0]).expect("Failed to reset password");
    } else {
        service.create_admin(passwords[0]).expect("Failed to create admin");
    }
    
    // Change password multiple times
    for i in 0..passwords.len() - 1 {
        let current = passwords[i];
        let next = passwords[i + 1];
        
        // Verify current password works
        assert!(service.authenticate("admin", current).is_ok());
        
        // Change to next password
        service.change_password(current, next)
            .expect("Failed to change password");
        
        // Verify old password fails
        assert!(service.authenticate("admin", current).is_err());
        
        // Verify new password works
        assert!(service.authenticate("admin", next).is_ok());
    }
}
