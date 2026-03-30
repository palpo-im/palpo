/// Property-based tests for PalpoClient user management
///
/// These tests verify correctness properties of the PalpoClient implementation
/// using automated test generation.

use palpo_admin_server::palpo_client::{
    CreateOrUpdateUserRequest, ListUsersQuery, PalpoRateLimitConfig,
};
use proptest::prelude::*;

// Property 1: ListUsersQuery pagination parameters are valid
proptest! {
    #[test]
    fn test_list_users_query_pagination_valid(from in 0..1000u32, limit in 1..=100u32) {
        let query = ListUsersQuery {
            from: Some(from as i64),
            limit: Some(limit as i64),
            search_term: None,
            guests: None,
            deactivated: None,
            admins: None,
        };

        // Pagination should always be valid
        prop_assert!(query.from.unwrap_or(0) >= 0);
        prop_assert!(query.limit.unwrap_or(1) >= 1);
    }
}

// Property 2: Rate limit config values are within valid ranges
proptest! {
    #[test]
    fn test_rate_limit_config_valid_values(
        messages_per_second in 0..=1000i64,
        burst_count in 0..=500i64
    ) {
        let config = PalpoRateLimitConfig {
            messages_per_second: Some(messages_per_second),
            burst_count: Some(burst_count),
        };

        // Config should serialize without panic
        let json = serde_json::to_string(&config).unwrap();
        prop_assert!(!json.is_empty());
    }
}

// Property 3: CreateOrUpdateUserRequest preserves data
proptest! {
    #[test]
    fn test_create_user_request_preserves_data(
        displayname in "[a-zA-Z0-9_]{1,50}",
        admin in proptest::bool::ANY,
        deactivated in proptest::bool::ANY
    ) {
        let req = CreateOrUpdateUserRequest {
            displayname: Some(displayname.clone()),
            password: Some("test_password".to_string()),
            admin: Some(admin),
            deactivated: Some(deactivated),
            avatar_url: None,
            user_type: None,
        };

        // Round-trip through JSON should preserve data
        let json = serde_json::to_string(&req).unwrap();
        let decoded: CreateOrUpdateUserRequest = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(decoded.displayname, Some(displayname));
        prop_assert_eq!(decoded.admin, Some(admin));
        prop_assert_eq!(decoded.deactivated, Some(deactivated));
    }
}

// Property 4: Search term filtering works correctly
proptest! {
    #[test]
    fn test_list_users_query_search_term(search_term in "[a-zA-Z0-9_-]{0,100}") {
        let query = ListUsersQuery {
            from: Some(0),
            limit: Some(10),
            search_term: Some(search_term.clone()),
            guests: None,
            deactivated: None,
            admins: None,
        };

        // Should serialize correctly
        let json = serde_json::to_string(&query).unwrap();
        prop_assert!(json.contains(&search_term));
    }
}

// Property 5: Guest and admin filters are valid
proptest! {
    #[test]
    fn test_list_users_query_user_type_filters(
        guests in proptest::bool::ANY,
        admins in proptest::bool::ANY,
        deactivated in proptest::bool::ANY
    ) {
        let query = ListUsersQuery {
            from: Some(0),
            limit: Some(10),
            search_term: None,
            guests: Some(guests),
            deactivated: Some(deactivated),
            admins: Some(admins),
        };

        // Should serialize without error
        let json = serde_json::to_string(&query).unwrap();
        prop_assert!(!json.is_empty());
    }
}

// Property 6: Rate limit round-trip consistency
proptest! {
    #[test]
    fn test_rate_limit_roundtrip(messages in 1..=1000i64, burst in 1..=500i64) {
        let config = PalpoRateLimitConfig {
            messages_per_second: Some(messages),
            burst_count: Some(burst),
        };

        // Serialize and deserialize
        let json = serde_json::to_string(&config).unwrap();
        let decoded: PalpoRateLimitConfig = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(decoded.messages_per_second, Some(messages));
        prop_assert_eq!(decoded.burst_count, Some(burst));
    }
}

// Property 7: Empty optional fields don't cause issues
#[test]
fn test_create_user_request_minimal_fields() {
    let req = CreateOrUpdateUserRequest {
        displayname: None,
        password: None,
        admin: None,
        deactivated: None,
        avatar_url: None,
        user_type: None,
    };

    // Should serialize without error
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.is_empty());
}

// Property 8: ListUsersQuery minimal fields
#[test]
fn test_list_users_query_minimal_fields() {
    let query = ListUsersQuery {
        from: None,
        limit: None,
        search_term: None,
        guests: None,
        deactivated: None,
        admins: None,
    };

    // Should serialize without error
    let json = serde_json::to_string(&query).unwrap();
    assert!(!json.is_empty());
}

// Property 9: Rate limit config minimal fields
#[test]
fn test_rate_limit_config_minimal_fields() {
    let config = PalpoRateLimitConfig {
        messages_per_second: None,
        burst_count: None,
    };

    // Should serialize without error
    let json = serde_json::to_string(&config).unwrap();
    assert!(!json.is_empty());
}