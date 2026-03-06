/// Integration tests for validation functions
///
/// This test suite verifies that validation functions work together correctly.
/// Unit tests for individual validation functions are in the source files.

use palpo_admin_server::handlers::validation::*;

// ===== Integration Tests =====

#[test]
fn test_validation_error_display() {
    let err = ValidationError {
        field: "user_id".to_string(),
        message: "Invalid format".to_string(),
    };
    assert_eq!(err.to_string(), "user_id: Invalid format");
}

#[test]
fn test_multiple_validations() {
    // Test that multiple validations can be chained
    let user_id = "@alice:example.com";
    let username = "alice";
    let limit = Some(10);
    let offset = Some(0);
    
    assert!(validate_user_id(user_id).is_ok());
    assert!(validate_username(username).is_ok());
    assert!(validate_limit(limit).is_ok());
    assert!(validate_offset(offset).is_ok());
}

#[test]
fn test_validation_consistency() {
    // Ensure validation is consistent across multiple calls
    for _ in 0..100 {
        assert!(validate_user_id("@alice:example.com").is_ok());
        assert!(validate_user_id("invalid").is_err());
    }
}
