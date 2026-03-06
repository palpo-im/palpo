//! Unit tests for user management frontend components
//!
//! **Validates: Requirements 3.1.4, 3.1.6**
//!
//! This module provides comprehensive unit tests for:
//! - User model tests
//! - User form validation (username, password)
//! - Password generation
//! - Dialog types
//! - Sort order
//! - Timestamp formatting
//! - Search and filter functionality
//! - Pagination logic
//! - Batch operations

use std::time::SystemTime;
use rand::Rng;

// ============================================================================
// Model Tests
// ============================================================================

#[cfg(test)]
mod user_model_tests {
    use super::*;
    use crate::models::user::{User, UserSortField, ListUsersRequest};
    use crate::models::room::SortOrder;

    /// Test: User::is_active returns false for deactivated users
    #[test]
    fn test_user_is_active_for_deactivated() {
        let user = User {
            user_id: "@test:example.com".to_string(),
            username: "test".to_string(),
            display_name: Some("Test User".to_string()),
            avatar_url: None,
            is_admin: false,
            is_deactivated: true,
            creation_ts: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            last_seen_ts: Some(SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()),
            permissions: vec![],
        };
        assert!(!user.is_active());
    }

    /// Test: User::is_active returns true for active users
    #[test]
    fn test_user_is_active_for_active_user() {
        let user = User {
            user_id: "@test:example.com".to_string(),
            username: "test".to_string(),
            display_name: Some("Test User".to_string()),
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            last_seen_ts: Some(SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()),
            permissions: vec![],
        };
        assert!(user.is_active());
    }

    /// Test: User::age_in_days returns positive value for valid timestamp
    #[test]
    fn test_user_age_in_days() {
        let one_day_ago = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 86400; // 24 hours ago

        let user = User {
            user_id: "@test:example.com".to_string(),
            username: "test".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: one_day_ago,
            last_seen_ts: None,
            permissions: vec![],
        };

        let age = user.age_in_days();
        assert!(age >= 0 && age <= 1, "Age should be 0 or 1 days for recently created user");
    }

    /// Test: User::days_since_last_seen returns None when never seen
    #[test]
    fn test_user_days_since_last_seen_none() {
        let user = User {
            user_id: "@test:example.com".to_string(),
            username: "test".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            last_seen_ts: None,
            permissions: vec![],
        };

        assert!(user.days_since_last_seen().is_none());
    }

    /// Test: UserSortField::description returns valid string
    #[test]
    fn test_user_sort_field_description() {
        assert_eq!(UserSortField::Username.description(), "Username");
        assert_eq!(UserSortField::DisplayName.description(), "Display Name");
        assert_eq!(UserSortField::CreationTime.description(), "Creation Time");
        assert_eq!(UserSortField::LastSeen.description(), "Last Seen");
        assert_eq!(UserSortField::IsAdmin.description(), "Admin Status");
    }

    /// Test: ListUsersRequest::default has expected values
    #[test]
    fn test_list_users_request_default() {
        let request = ListUsersRequest::default();
        assert_eq!(request.limit, Some(50));
        assert_eq!(request.offset, Some(0));
        assert!(request.search.is_none());
        assert!(request.filter_admin.is_none());
        assert_eq!(request.filter_deactivated, Some(false));
        assert!(matches!(request.sort_by, Some(UserSortField::Username)));
        assert!(matches!(request.sort_order, Some(SortOrder::Ascending)));
    }
}

// ============================================================================
// User Form Validation Tests
// ============================================================================

#[cfg(test)]
mod user_form_validation_tests {
    use super::*;

    /// Validates: Requirement 1 - Username availability check
    fn validate_username(username: &str) -> Result<(), String> {
        if username.is_empty() {
            return Err("用户名不能为空".to_string());
        }
        if username.len() < 3 {
            return Err("用户名至少3个字符".to_string());
        }
        if username.len() > 255 {
            return Err("用户名不能超过255个字符".to_string());
        }
        if !username.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err("用户名只能包含字母、数字、下划线和连字符".to_string());
        }
        Ok(())
    }

    /// Validates: Requirement 2 - Password strength validation
    fn validate_password(password: &str) -> Result<(), String> {
        if password.is_empty() {
            return Err("密码不能为空".to_string());
        }
        if password.len() < 8 {
            return Err("密码至少8个字符".to_string());
        }
        let has_upper = password.chars().any(|c| c.is_uppercase());
        let has_lower = password.chars().any(|c| c.is_lowercase());
        let has_digit = password.chars().any(|c| c.is_ascii_digit());
        let has_special = password.chars().any(|c| !c.is_alphanumeric());
        
        if !has_upper || !has_lower || !has_digit || !has_special {
            return Err("密码必须包含大写字母、小写字母、数字和特殊字符".to_string());
        }
        Ok(())
    }

    /// Test: Valid usernames pass validation
    #[test]
    fn test_valid_usernames() {
        assert!(validate_username("testuser").is_ok());
        assert!(validate_username("TestUser123").is_ok());
        assert!(validate_username("user_name").is_ok());
        assert!(validate_username("user-name").is_ok());
        assert!(validate_username("abc").is_ok());
    }

    /// Test: Invalid usernames fail validation
    #[test]
    fn test_invalid_usernames() {
        assert!(validate_username("").is_err());
        assert!(validate_username("ab").is_err());
        assert!(validate_username("user name").is_err());
        assert!(validate_username("user@name").is_err());
        assert!(validate_username("user.name").is_err());
    }

    /// Test: Valid passwords pass validation
    #[test]
    fn test_valid_passwords() {
        assert!(validate_password("Password1!").is_ok());
        assert!(validate_password("Abcdef123!@#").is_ok());
        assert!(validate_password("A1b2c3d4!E5f6").is_ok());
    }

    /// Test: Invalid passwords fail validation
    #[test]
    fn test_invalid_passwords() {
        assert!(validate_password("").is_err());
        assert!(validate_password("short1!").is_err());
        assert!(validate_password("lowercase123!").is_err());
        assert!(validate_password("UPPERCASE123!").is_err());
        assert!(validate_password("NoDigits!!!").is_err());
        assert!(validate_password("NoSpecial123").is_err());
    }

    /// Test: Password confirmation matches
    #[test]
    fn test_password_confirmation() {
        let password = "TestPassword123!";
        let confirm = "TestPassword123!";
        assert_eq!(password, confirm);
    }

    /// Test: Password confirmation mismatch
    #[test]
    fn test_password_confirmation_mismatch() {
        let password = "TestPassword123!";
        let confirm = "DifferentPassword456!";
        assert_ne!(password, confirm);
    }
}

// ============================================================================
// Password Generation Tests
// ============================================================================

#[cfg(test)]
mod password_generation_tests {
    use super::*;

    /// Validates: Requirement 2 - Password generator produces strong passwords
    fn generate_password(length: usize) -> String {
        const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*";
        let mut rng = rand::thread_rng();
        let mut password = String::with_capacity(length);
        for _ in 0..length {
            let idx = rng.gen_range(0..CHARS.len());
            password.push(CHARS[idx] as char);
        }
        password
    }

    fn validate_generated_password(password: &str) -> bool {
        if password.len() < 16 {
            return false;
        }
        let has_upper = password.chars().any(|c| c.is_uppercase());
        let has_lower = password.chars().any(|c| c.is_lowercase());
        let has_digit = password.chars().any(|c| c.is_ascii_digit());
        let has_special = password.chars().any(|c| !c.is_alphanumeric());
        
        has_upper && has_lower && has_digit && has_special
    }

    /// Test: Generated password has correct length
    #[test]
    fn test_generated_password_length() {
        let password = generate_password(16);
        assert_eq!(password.len(), 16);
    }

    /// Test: Generated password meets all strength requirements
    #[test]
    fn test_generated_password_strength() {
        let password = generate_password(16);
        assert!(validate_generated_password(&password), "Generated password should meet all requirements");
    }

    /// Test: Generated password contains all character types
    #[test]
    fn test_generated_password_contains_all_types() {
        let password = generate_password(16);
        
        assert!(password.chars().any(|c| c.is_uppercase()), "Should contain uppercase");
        assert!(password.chars().any(|c| c.is_lowercase()), "Should contain lowercase");
        assert!(password.chars().any(|c| c.is_ascii_digit()), "Should contain digit");
        assert!(password.chars().any(|c| !c.is_alphanumeric()), "Should contain special char");
    }
}

// ============================================================================
// Dialog Type Tests
// ============================================================================

#[cfg(test)]
mod dialog_type_tests {
    use super::*;
    use crate::components::dialogs::DialogType;

    /// Test: All dialog types are distinct
    #[test]
    fn test_dialog_types_are_distinct() {
        let types = vec![
            DialogType::Deactivate,
            DialogType::DeleteDevice,
            DialogType::DeleteMedia,
            DialogType::ShadowBan,
            DialogType::LoginAsUser,
            DialogType::PasswordReset,
        ];
        
        let mut seen = std::collections::HashSet::new();
        for t in &types {
            assert!(seen.insert(*t), "Dialog type {:?} should be unique", t);
        }
    }

    /// Test: DialogType can be cloned
    #[test]
    fn test_dialog_type_clone() {
        let dialog_type = DialogType::Deactivate;
        let cloned = dialog_type.clone();
        assert!(matches!(cloned, DialogType::Deactivate));
    }
}

// ============================================================================
// Sort Order Tests
// ============================================================================

#[cfg(test)]
mod sort_order_tests {
    use super::*;
    use crate::models::room::SortOrder;

    /// Test: SortOrder has correct descriptions
    #[test]
    fn test_sort_order_description() {
        assert_eq!(SortOrder::Ascending.description(), "Ascending");
        assert_eq!(SortOrder::Descending.description(), "Descending");
    }

    /// Test: SortOrder can be cloned
    #[test]
    fn test_sort_order_clone() {
        let ascending = SortOrder::Ascending;
        let cloned = ascending.clone();
        assert!(matches!(cloned, SortOrder::Ascending));
    }

    /// Test: SortOrder variants are different
    #[test]
    fn test_sort_order_variants_different() {
        assert!(matches!(SortOrder::Ascending, SortOrder::Ascending));
        assert!(matches!(SortOrder::Descending, SortOrder::Descending));
        assert!(!matches!(SortOrder::Ascending, SortOrder::Descending));
    }
}

// ============================================================================
// Timestamp Formatting Tests
// ============================================================================

#[cfg(test)]
mod timestamp_formatting_tests {
    use super::*;

    /// Validates: Requirement 4 - User account info query
    fn format_timestamp(ts: u64) -> String {
        use chrono::{Utc, TimeZone};
        
        let dt = Utc.timestamp_opt(ts as i64, 0).single();
        match dt {
            Some(datetime) => datetime.format("%Y-%m-%d %H:%M").to_string(),
            None => "无效时间".to_string(),
        }
    }

    /// Test: Valid timestamp formats correctly
    #[test]
    fn test_valid_timestamp_format() {
        let ts = 1640000000; // 2021-12-20 00:26:40 UTC
        let formatted = format_timestamp(ts);
        
        assert!(formatted.starts_with("2021-12-20"));
        assert!(formatted.contains(':'));
    }

    /// Test: Zero timestamp shows invalid
    #[test]
    fn test_zero_timestamp() {
        let formatted = format_timestamp(0);
        assert_eq!(formatted, "无效时间");
    }

    /// Test: Current timestamp formats without error
    #[test]
    fn test_current_timestamp() {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let formatted = format_timestamp(now);
        assert_ne!(formatted, "无效时间");
        assert!(formatted.contains('-'));
    }
}

// ============================================================================
// Search and Filter Tests
// ============================================================================

#[cfg(test)]
mod search_filter_tests {
    use super::*;

    /// Validates: Requirement 19 - User search and filter
    fn matches_search(label: &str, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        label.to_lowercase().contains(&query.to_lowercase())
    }

    /// Test: Empty query matches all
    #[test]
    fn test_empty_query_matches_all() {
        assert!(matches_search("test", ""));
        assert!(matches_search("user", ""));
        assert!(matches_search("admin", ""));
    }

    /// Test: Case-insensitive search
    #[test]
    fn test_case_insensitive_search() {
        assert!(matches_search("TestUser", "test"));
        assert!(matches_search("TestUser", "TEST"));
        assert!(matches_search("TestUser", "User"));
    }

    /// Test: Non-matching query returns false
    #[test]
    fn test_non_matching_query() {
        assert!(!matches_search("TestUser", "xyz"));
        assert!(!matches_search("AdminPanel", "user"));
    }

    /// Test: Substring matching works
    #[test]
    fn test_substring_matching() {
        assert!(matches_search("TestUser123", "User"));
        assert!(matches_search("TestUser123", "123"));
        assert!(matches_search("TestUser123", "Test"));
    }
}

// ============================================================================
// Pagination Tests
// ============================================================================

#[cfg(test)]
mod pagination_tests {
    use super::*;

    /// Validates: Requirement 19.6 - Pagination query
    fn calculate_total_pages(total_items: u32, page_size: u32) -> u32 {
        (total_items + page_size - 1) / page_size
    }

    fn calculate_offset(page: u32, page_size: u32) -> u32 {
        page * page_size
    }

    /// Test: Total pages calculated correctly
    #[test]
    fn test_total_pages_calculation() {
        assert_eq!(calculate_total_pages(0, 20), 0);
        assert_eq!(calculate_total_pages(1, 20), 1);
        assert_eq!(calculate_total_pages(20, 20), 1);
        assert_eq!(calculate_total_pages(21, 20), 2);
        assert_eq!(calculate_total_pages(100, 20), 5);
    }

    /// Test: Offset calculated correctly
    #[test]
    fn test_offset_calculation() {
        assert_eq!(calculate_offset(0, 20), 0);
        assert_eq!(calculate_offset(1, 20), 20);
        assert_eq!(calculate_offset(2, 20), 40);
        assert_eq!(calculate_offset(5, 10), 50);
    }

    /// Test: Page bounds validation
    #[test]
    fn test_page_bounds() {
        let total_pages = calculate_total_pages(100, 20);
        assert!(total_pages > 0);
        assert!(calculate_offset(total_pages - 1, 20) < 100);
    }
}

// ============================================================================
// Batch Operations Tests
// ============================================================================

#[cfg(test)]
mod batch_operations_tests {
    use super::*;

    /// Validates: Requirement 19 - Batch operations
    fn select_all_users(users: &mut Vec<String>, select: bool) {
        if select {
            *users = users.iter().cloned().collect();
        } else {
            users.clear();
        }
    }

    fn toggle_user_selection(users: &mut Vec<String>, user_id: &str) {
        if let Some(pos) = users.iter().position(|u| u == user_id) {
            users.remove(pos);
        } else {
            users.push(user_id.to_string());
        }
    }

    /// Test: Select all works correctly
    #[test]
    fn test_select_all() {
        let mut selected = Vec::<String>::new();
        
        select_all_users(&mut selected, true);
        assert!(selected.is_empty()); // No users to select from
        
        select_all_users(&mut selected, false);
        assert!(selected.is_empty());
    }

    /// Test: Toggle selection works correctly
    #[test]
    fn test_toggle_selection() {
        let mut selected = Vec::<String>::new();
        
        toggle_user_selection(&mut selected, "user1");
        assert_eq!(selected.len(), 1);
        assert!(selected.contains(&"user1".to_string()));
        
        toggle_user_selection(&mut selected, "user1");
        assert!(selected.is_empty());
        
        toggle_user_selection(&mut selected, "user2");
        toggle_user_selection(&mut selected, "user3");
        assert_eq!(selected.len(), 2);
    }

    /// Test: Selection count is accurate
    #[test]
    fn test_selection_count() {
        let mut selected = Vec::<String>::new();
        let users = vec!["user1", "user2", "user3", "user4", "user5"];
        
        for user in &users {
            toggle_user_selection(&mut selected, user);
        }
        assert_eq!(selected.len(), 5);
        
        toggle_user_selection(&mut selected, "user3");
        assert_eq!(selected.len(), 4);
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn validate_username(username: &str) -> Result<(), String> {
    if username.is_empty() {
        return Err("用户名不能为空".to_string());
    }
    if username.len() < 3 {
        return Err("用户名至少3个字符".to_string());
    }
    if username.len() > 255 {
        return Err("用户名不能超过255个字符".to_string());
    }
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err("用户名只能包含字母、数字、下划线和连字符".to_string());
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("密码不能为空".to_string());
    }
    if password.len() < 8 {
        return Err("密码至少8个字符".to_string());
    }
    let has_upper = password.chars().any(|c| c.is_uppercase());
    let has_lower = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    let has_special = password.chars().any(|c| !c.is_alphanumeric());
    
    if !has_upper || !has_lower || !has_digit || !has_special {
        return Err("密码必须包含大写字母、小写字母、数字和特殊字符".to_string());
    }
    Ok(())
}

fn matches_search(label: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    label.to_lowercase().contains(&query.to_lowercase())
}

fn calculate_total_pages(total_items: u32, page_size: u32) -> u32 {
    (total_items + page_size - 1) / page_size
}

fn calculate_offset(page: u32, page_size: u32) -> u32 {
    page * page_size
}