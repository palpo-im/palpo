/// Comprehensive validation tests for user management
///
/// This test suite provides extensive coverage of all validation functions
/// to ensure proper input validation across the user management API.

use palpo_admin_server::handlers::validation::*;

// ===== User ID Validation Tests =====

#[test]
fn test_validate_user_id_valid_formats() {
    // Standard formats
    assert!(validate_user_id("@alice:example.com").is_ok());
    assert!(validate_user_id("@bob:matrix.org").is_ok());
    assert!(validate_user_id("@user123:localhost").is_ok());
    
    // With special characters in localpart
    assert!(validate_user_id("@user-name:example.com").is_ok());
    assert!(validate_user_id("@user_name:example.com").is_ok());
    assert!(validate_user_id("@user.name:example.com").is_ok());
    assert!(validate_user_id("@user=name:example.com").is_ok());
    assert!(validate_user_id("@user/name:example.com").is_ok());
    
    // With numbers
    assert!(validate_user_id("@user123:example.com").is_ok());
    assert!(validate_user_id("@123user:example.com").is_ok());
    
    // With subdomain
    assert!(validate_user_id("@alice:sub.example.com").is_ok());
    
    // With port
    assert!(validate_user_id("@alice:example.com:8008").is_ok());
}

#[test]
fn test_validate_user_id_invalid_formats() {
    // Empty
    assert!(validate_user_id("").is_err());
    
    // Missing @
    assert!(validate_user_id("alice:example.com").is_err());
    
    // Missing server
    assert!(validate_user_id("@alice").is_err());
    
    // Missing colon
    assert!(validate_user_id("@aliceexample.com").is_err());
    
    // Empty localpart
    assert!(validate_user_id("@:example.com").is_err());
    
    // Empty server
    assert!(validate_user_id("@alice:").is_err());
    
    // Too long
    let long_user_id = format!("@{}:example.com", "a".repeat(300));
    assert!(validate_user_id(&long_user_id).is_err());
}

#[test]
fn test_validate_user_id_edge_cases() {
    // Single character localpart
    assert!(validate_user_id("@a:example.com").is_ok());
    
    // Single character server
    assert!(validate_user_id("@alice:a").is_ok());
    
    // Maximum length (just under limit)
    let max_localpart = "a".repeat(240);
    let max_user_id = format!("@{}:example.com", max_localpart);
    assert!(validate_user_id(&max_user_id).is_ok());
}

// ===== Username Validation Tests =====

#[test]
fn test_validate_username_valid() {
    assert!(validate_username("alice").is_ok());
    assert!(validate_username("alice123").is_ok());
    assert!(validate_username("alice_bob").is_ok());
    assert!(validate_username("alice-bob").is_ok());
    assert!(validate_username("alice.bob").is_ok());
    assert!(validate_username("a").is_ok());
}

#[test]
fn test_validate_username_invalid() {
    // Empty
    assert!(validate_username("").is_err());
    
    // Contains invalid characters
    assert!(validate_username("alice:bob").is_err());
    assert!(validate_username("@alice").is_err());
    assert!(validate_username("alice/bob").is_err());
    
    // Too long
    let long_username = "a".repeat(65);
    assert!(validate_username(&long_username).is_err());
}

#[test]
fn test_validate_username_edge_cases() {
    // Maximum length
    let max_username = "a".repeat(64);
    assert!(validate_username(&max_username).is_ok());
    
    // Just over maximum
    let over_max = "a".repeat(65);
    assert!(validate_username(&over_max).is_err());
}

// ===== Pagination Validation Tests =====

#[test]
fn test_validate_limit_valid() {
    assert_eq!(validate_limit(Some(1)).unwrap(), 1);
    assert_eq!(validate_limit(Some(10)).unwrap(), 10);
    assert_eq!(validate_limit(Some(50)).unwrap(), 50);
    assert_eq!(validate_limit(Some(100)).unwrap(), 100);
    assert_eq!(validate_limit(None).unwrap(), 50); // Default
}

#[test]
fn test_validate_limit_clamping() {
    // Over maximum should clamp to max
    assert_eq!(validate_limit(Some(200)).unwrap(), 100);
    assert_eq!(validate_limit(Some(1000)).unwrap(), 100);
}

#[test]
fn test_validate_limit_invalid() {
    // Zero or negative
    assert!(validate_limit(Some(0)).is_err());
    assert!(validate_limit(Some(-1)).is_err());
    assert!(validate_limit(Some(-100)).is_err());
}

#[test]
fn test_validate_offset_valid() {
    assert_eq!(validate_offset(Some(0)).unwrap(), 0);
    assert_eq!(validate_offset(Some(10)).unwrap(), 10);
    assert_eq!(validate_offset(Some(100)).unwrap(), 100);
    assert_eq!(validate_offset(Some(1000)).unwrap(), 1000);
    assert_eq!(validate_offset(None).unwrap(), 0); // Default
}

#[test]
fn test_validate_offset_invalid() {
    // Negative
    assert!(validate_offset(Some(-1)).is_err());
    assert!(validate_offset(Some(-100)).is_err());
}

// ===== Displayname Validation Tests =====

#[test]
fn test_validate_displayname_valid() {
    assert!(validate_displayname(None).is_ok());
    assert!(validate_displayname(Some("Alice")).is_ok());
    assert!(validate_displayname(Some("Alice Bob")).is_ok());
    assert!(validate_displayname(Some("Alice 123")).is_ok());
    assert!(validate_displayname(Some("")).is_ok()); // Empty is valid
    
    // Unicode characters
    assert!(validate_displayname(Some("Alice 🎉")).is_ok());
    assert!(validate_displayname(Some("爱丽丝")).is_ok());
}

#[test]
fn test_validate_displayname_invalid() {
    // Too long
    let long_name = "a".repeat(257);
    assert!(validate_displayname(Some(&long_name)).is_err());
}

#[test]
fn test_validate_displayname_edge_cases() {
    // Maximum length
    let max_name = "a".repeat(256);
    assert!(validate_displayname(Some(&max_name)).is_ok());
    
    // Just over maximum
    let over_max = "a".repeat(257);
    assert!(validate_displayname(Some(&over_max)).is_err());
}

// ===== Device ID Validation Tests =====

#[test]
fn test_validate_device_id_valid() {
    assert!(validate_device_id("ABCD1234").is_ok());
    assert!(validate_device_id("device_123").is_ok());
    assert!(validate_device_id("device-123").is_ok());
    assert!(validate_device_id("ABCDEFGHIJ").is_ok());
}

#[test]
fn test_validate_device_id_invalid() {
    // Empty
    assert!(validate_device_id("").is_err());
    
    // Too short
    assert!(validate_device_id("ABC").is_err());
    
    // Too long
    let long_id = "A".repeat(51);
    assert!(validate_device_id(&long_id).is_err());
    
    // Invalid characters
    assert!(validate_device_id("device@123").is_err());
    assert!(validate_device_id("device#123").is_err());
    assert!(validate_device_id("device 123").is_err());
}

#[test]
fn test_validate_device_id_edge_cases() {
    // Minimum length
    assert!(validate_device_id("ABCD").is_ok());
    
    // Maximum length
    let max_id = "A".repeat(50);
    assert!(validate_device_id(&max_id).is_ok());
}

// ===== Room ID Validation Tests =====

#[test]
fn test_validate_room_id_valid() {
    assert!(validate_room_id("!abc123:example.com").is_ok());
    assert!(validate_room_id("!room:matrix.org").is_ok());
    assert!(validate_room_id("!a:localhost").is_ok());
}

#[test]
fn test_validate_room_id_invalid() {
    // Empty
    assert!(validate_room_id("").is_err());
    
    // Missing !
    assert!(validate_room_id("abc123:example.com").is_err());
    
    // Too long
    let long_room_id = format!("!{}:example.com", "a".repeat(300));
    assert!(validate_room_id(&long_room_id).is_err());
}

// ===== Session Token Validation Tests =====

#[test]
fn test_validate_session_token_valid() {
    assert!(validate_session_token("abcdef1234567890").is_ok());
    assert!(validate_session_token(&"a".repeat(16)).is_ok());
    assert!(validate_session_token(&"a".repeat(100)).is_ok());
}

#[test]
fn test_validate_session_token_invalid() {
    // Empty
    assert!(validate_session_token("").is_err());
    
    // Too short
    assert!(validate_session_token("abc").is_err());
    assert!(validate_session_token(&"a".repeat(15)).is_err());
}

// ===== Rate Limit Validation Tests =====

#[test]
fn test_validate_rate_limit_params_valid() {
    assert!(validate_rate_limit_params(Some(10), Some(100)).is_ok());
    assert!(validate_rate_limit_params(Some(0), Some(0)).is_ok());
    assert!(validate_rate_limit_params(Some(10000), Some(100000)).is_ok());
    assert!(validate_rate_limit_params(None, None).is_ok());
    assert!(validate_rate_limit_params(Some(10), None).is_ok());
    assert!(validate_rate_limit_params(None, Some(100)).is_ok());
}

#[test]
fn test_validate_rate_limit_params_invalid() {
    // Negative values
    assert!(validate_rate_limit_params(Some(-1), Some(100)).is_err());
    assert!(validate_rate_limit_params(Some(10), Some(-1)).is_err());
    
    // Over maximum
    assert!(validate_rate_limit_params(Some(10001), Some(100)).is_err());
    assert!(validate_rate_limit_params(Some(10), Some(100001)).is_err());
}

#[test]
fn test_validate_rate_limit_params_edge_cases() {
    // Zero values (valid)
    assert!(validate_rate_limit_params(Some(0), Some(0)).is_ok());
    
    // Maximum values
    assert!(validate_rate_limit_params(Some(10000), Some(100000)).is_ok());
    
    // Just over maximum
    assert!(validate_rate_limit_params(Some(10001), Some(100000)).is_err());
    assert!(validate_rate_limit_params(Some(10000), Some(100001)).is_err());
}

// ===== Threepid Medium Validation Tests =====

#[test]
fn test_validate_threepid_medium_valid() {
    assert!(validate_threepid_medium("email").is_ok());
    assert!(validate_threepid_medium("phone").is_ok());
    assert!(validate_threepid_medium("msisdn").is_ok());
}

#[test]
fn test_validate_threepid_medium_invalid() {
    assert!(validate_threepid_medium("").is_err());
    assert!(validate_threepid_medium("invalid").is_err());
    assert!(validate_threepid_medium("EMAIL").is_err()); // Case sensitive
    assert!(validate_threepid_medium("sms").is_err());
}

// ===== Search Query Validation Tests =====

#[test]
fn test_validate_search_query_valid() {
    assert!(validate_search_query(None).is_ok());
    assert!(validate_search_query(Some("alice")).is_ok());
    assert!(validate_search_query(Some("alice bob")).is_ok());
    assert!(validate_search_query(Some("")).is_ok());
    
    // Maximum length
    let max_query = "a".repeat(500);
    assert!(validate_search_query(Some(&max_query)).is_ok());
}

#[test]
fn test_validate_search_query_invalid() {
    // Too long
    let long_query = "a".repeat(501);
    assert!(validate_search_query(Some(&long_query)).is_err());
}

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
