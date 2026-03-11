/// Property-Based Test: Username Availability Accuracy (Property 1)
///
/// **Property 1: Username Validation Invariant**
/// For all valid usernames, validation should pass.
/// For all invalid usernames, validation should fail with appropriate error.
///
/// This test uses proptest to generate a wide range of username inputs
/// and verifies the validation logic is correct across all possible inputs.
///
/// **Validates: Requirements 1.3** (Username availability check)

use proptest::prelude::*;
use palpo_admin_server::handlers::validation::validate_username;

/// Valid username strategy
/// - 3-64 characters
/// - Contains only alphanumeric and underscore
/// - Does not start with number
fn valid_username() -> impl Strategy<Value = String> {
    // Generate strings of length 3-64 with valid characters
    "[a-zA-Z][a-zA-Z0-9_]{2,63}"
        .prop_map(|s| s)
        .prop_filter("username must not exceed 64 chars", |s| s.len() <= 64)
}

/// Invalid username strategy
/// - Empty string
/// - Too short (<3 chars) - must NOT start with letter to be invalid
/// - Too long (>64 chars)
/// - Contains invalid characters (:, @, /)
/// - Starts with number
fn invalid_username() -> impl Strategy<Value = String> {
    prop_oneof![
        // Empty string
        Just("".to_string()),
        // Too short (1-2 chars) - must start with digit or underscore to be invalid
        "[0-9_][a-zA-Z0-9_]{0,1}".prop_map(|s| s),
        // Too long (65+ chars) - explicit long string
        "[a-z]{65,100}".prop_map(|s| s),
        // Contains colon - must have at least one colon
        "[a-z]+:[a-z]+".prop_map(|s| s),
        // Contains at sign - must have at least one @
        "[a-z]+@[a-z]+".prop_map(|s| s),
        // Contains slash - must have at least one /
        "[a-z]+/[a-z]+".prop_map(|s| s),
        // Starts with number (3+ chars)
        "[0-9][a-zA-Z0-9_]{2,63}".prop_map(|s| s),
        // Only special characters
        Just("___...".to_string())
    ]
}

#[test]
fn test_valid_usernames_pass_validation() {
    // Property: All valid usernames should pass validation
    proptest!(|(username in valid_username())| {
        let result = validate_username(&username);
        assert!(
            result.is_ok(),
            "Valid username '{}' should pass validation but failed: {:?}",
            username, result
        );
    });
}

#[test]
fn test_invalid_usernames_fail_validation() {
    // Property: All invalid usernames should fail validation
    proptest!(|(username in invalid_username())| {
        let result = validate_username(&username);
        assert!(
            result.is_err(),
            "Invalid username '{}' should fail validation but passed",
            username
        );
    });
}

#[test]
fn test_username_length_boundaries() {
    // Property: Length boundaries are correctly enforced
    proptest!(|(len in 1..=128u64)| {
        let username = "a".repeat(len as usize);
        let result = validate_username(&username);
        
        if len >= 3 && len <= 64 {
            // Valid length range
            assert!(
                result.is_ok(),
                "Username of length {} should be valid but failed: {:?}",
                len, result
            );
        } else {
            // Invalid length range (too short or too long)
            assert!(
                result.is_err(),
                "Username of length {} should be invalid but passed",
                len
            );
        }
    });
}

#[test]
fn test_username_character_validation() {
    // Property: Only valid characters are accepted
    // Must generate at least 3 characters total (1 first + 2+ rest)
    proptest!(|(
        first in "[a-zA-Z]",
        rest in "[a-zA-Z0-9_]{2,62}"
    )| {
        let username = format!("{}{}", first, rest);
        let result = validate_username(&username);
        
        assert!(
            result.is_ok(),
            "Username with valid characters '{}' should pass but failed: {:?}",
            username, result
        );
    });
}

#[test]
fn test_username_rejects_matrix_chars() {
    // Property: Matrix user ID characters (:, @, /) should be rejected
    proptest!(|(
        prefix in "[a-zA-Z0-9_]{1,10}",
        special_char in "[/:@]",
        suffix in "[a-zA-Z0-9_]{0,50}"
    )| {
        let username = format!("{}{}{}", prefix, special_char, suffix);
        let result = validate_username(&username);
        
        assert!(
            result.is_err(),
            "Username containing Matrix character '{}' should fail but passed",
            username
        );
    });
}

#[test]
fn test_username_validation_idempotence() {
    // Property: Validating the same username multiple times gives same result
    proptest!(|(username in "[a-zA-Z][a-zA-Z0-9_]{2,63}")| {
        let result1 = validate_username(&username);
        let result2 = validate_username(&username);
        
        assert_eq!(
            result1.is_ok(),
            result2.is_ok(),
            "Username validation should be idempotent"
        );
    });
}

#[test]
fn test_username_validation_determinism() {
    // Property: Validation result depends only on username content, not external state
    proptest!(|(username in "[a-zA-Z][a-zA-Z0-9_]{2,63}")| {
        // Run validation multiple times
        let results: Vec<bool> = (0..10)
            .map(|_| validate_username(&username).is_ok())
            .collect();
        
        // All results should be the same (deterministic)
        assert!(
            results.iter().all(|&r| r == results[0]),
            "Username validation should be deterministic"
        );
    });
}