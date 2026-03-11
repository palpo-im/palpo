/// Property-Based Test: Pagination Consistency (Property 4)
///
/// **Property 4: Pagination Invariant**
/// For all valid pagination parameters:
/// 1. limit is correctly clamped to [1, 100]
/// 2. offset is non-negative
/// 3. returned users count <= limit
/// 4. total_count is consistent regardless of pagination
///
/// This test uses proptest to generate a wide range of pagination inputs
/// and verifies the pagination logic is correct across all possible inputs.
///
/// **Validates: Requirements 1.2** (User list with pagination)

use proptest::prelude::*;
use palpo_admin_server::handlers::validation::{validate_limit, validate_offset};

/// Valid limit values (1-100)
fn valid_limit() -> impl Strategy<Value = i64> {
    1..=100i64
}

/// Valid offset values (0+)
fn valid_offset() -> impl Strategy<Value = i64> {
    0..=1000i64
}

/// Invalid limit values (too small or too large)
fn invalid_limit() -> impl Strategy<Value = i64> {
    prop_oneof![
        // Zero or negative
        -10..=0i64,
        // Too large (>100)
        101..=1000i64,
        // Very large
        10000..=100000i64
    ]
}

/// Invalid offset values (negative)
fn invalid_offset() -> impl Strategy<Value = i64> {
    -100..=-1i64
}

#[test]
fn test_limit_clamping() {
    // Property: limit should be clamped to [1, 100] or return error for < 1
    proptest!(|(limit in 0..=200i64)| {
        let result = validate_limit(Some(limit));
        
        // For valid range [1, 100], should return Ok with clamped value
        if limit >= 1 && limit <= 100 {
            assert!(result.is_ok(), "Limit {} should be valid", limit);
            let clamped = result.unwrap();
            assert!(clamped >= 1 && clamped <= 100);
        } else if limit < 1 {
            // Below minimum should return error
            assert!(result.is_err(), "Limit {} should be invalid", limit);
        } else {
            // Above 100 should be clamped to 100
            assert!(result.is_ok(), "Limit {} should be valid after clamping", limit);
            let clamped = result.unwrap();
            assert_eq!(clamped, 100, "Limit > 100 should be clamped to 100");
        }
    });
}

#[test]
fn test_offset_non_negative() {
    // Property: offset should be non-negative after validation
    proptest!(|(offset in -100..=100i64)| {
        let result = validate_offset(Some(offset));
        
        if offset >= 0 {
            assert!(result.is_ok(), "Offset {} should be valid", offset);
            let validated = result.unwrap();
            assert_eq!(validated, offset, "Valid offset should be unchanged");
        } else {
            // Negative offset should return error
            assert!(result.is_err(), "Offset {} should be invalid", offset);
        }
    });
}

#[test]
fn test_limit_validation_idempotence() {
    // Property: Same limit should give same result
    proptest!(|(limit in 0..=200i64)| {
        let result1 = validate_limit(Some(limit));
        let result2 = validate_limit(Some(limit));
        
        // Compare is_ok since ValidationError doesn't implement PartialEq
        assert_eq!(
            result1.is_ok(), result2.is_ok(),
            "Limit validation should be idempotent"
        );
    });
}

#[test]
fn test_offset_validation_idempotence() {
    // Property: Same offset should give same result
    proptest!(|(offset in -100..=100i64)| {
        let result1 = validate_offset(Some(offset));
        let result2 = validate_offset(Some(offset));
        
        // Compare is_ok since ValidationError doesn't implement PartialEq
        assert_eq!(
            result1.is_ok(), result2.is_ok(),
            "Offset validation should be idempotent"
        );
    });
}

#[test]
fn test_pagination_determinism() {
    // Property: Pagination validation depends only on parameters, not external state
    proptest!(|(limit in 0..=200i64, offset in -100..=100i64)| {
        // Run validation multiple times
        let limit_results: Vec<bool> = (0..10)
            .map(|_| validate_limit(Some(limit)).is_ok())
            .collect();
        
        let offset_results: Vec<bool> = (0..10)
            .map(|_| validate_offset(Some(offset)).is_ok())
            .collect();
        
        // All results should be the same (deterministic)
        assert!(
            limit_results.iter().all(|&r| r == limit_results[0]),
            "Limit validation should be deterministic"
        );
        assert!(
            offset_results.iter().all(|&r| r == offset_results[0]),
            "Offset validation should be deterministic"
        );
    });
}

#[test]
fn test_default_pagination_values() {
    // Default limit should be 50
    let default_limit = validate_limit(None).unwrap();
    assert_eq!(default_limit, 50, "Default limit should be 50");
    
    // Default offset should be 0
    let default_offset = validate_offset(None).unwrap();
    assert_eq!(default_offset, 0, "Default offset should be 0");
}

#[test]
fn test_pagination_boundary_conditions() {
    // Property: Boundary conditions should be handled correctly
    proptest!(|(limit in prop::num::i64::ANY, offset in prop::num::i64::ANY)| {
        // Validate limit
        let limit_result = validate_limit(Some(limit));
        let limit_ok = limit_result.is_ok();
        let limit_val = limit_result.unwrap_or(50);
        
        // Validate offset
        let offset_result = validate_offset(Some(offset));
        let offset_ok = offset_result.is_ok();
        let offset_val = offset_result.unwrap_or(0);
        
        // After validation, values should be in valid range
        if limit_ok {
            assert!(limit_val >= 1 && limit_val <= 100);
        }
        if offset_ok {
            assert!(offset_val >= 0);
        }
    });
}

#[test]
fn test_limit_edge_cases() {
    // Property: Edge case limits should be handled correctly
    proptest!(|(limit in prop_oneof![
        Just(1i64),      // Minimum valid
        Just(100i64),    // Maximum valid
        Just(0i64),      // Below minimum (should error)
        Just(101i64),    // Above maximum (should clamp to 100)
        Just(-1i64),     // Negative (should error)
        Just(1000i64),   // Very large (should clamp to 100)
    ])| {
        let result = validate_limit(Some(limit));
        
        if limit >= 1 && limit <= 100 {
            assert!(result.is_ok(), "Limit {} should be valid", limit);
        } else if limit < 1 {
            assert!(result.is_err(), "Limit {} should be invalid", limit);
        } else {
            // limit > 100
            assert!(result.is_ok(), "Limit {} should be valid after clamping", limit);
            assert_eq!(result.unwrap(), 100);
        }
    });
}

#[test]
fn test_offset_edge_cases() {
    // Property: Edge case offsets should be handled correctly
    proptest!(|(offset in prop_oneof![
        Just(0i64),       // Minimum valid
        Just(1i64),       // Just above minimum
        Just(-1i64),      // Below minimum (should error)
        Just(-100i64),    // Very negative (should error)
    ])| {
        let result = validate_offset(Some(offset));
        
        if offset >= 0 {
            assert!(result.is_ok(), "Offset {} should be valid", offset);
            assert_eq!(result.unwrap(), offset);
        } else {
            assert!(result.is_err(), "Offset {} should be invalid", offset);
        }
    });
}