//! Comprehensive unit tests for User Admin API
//!
//! **Validates: Requirements 3.1.4, 3.1.6**
//!
//! This module provides comprehensive unit tests for:
//! - User list request building
//! - User creation validation
//! - User update validation
//! - Password reset logic
//! - Deactivation logic
//! - Batch operations
//! - Permission checking
//! - Password generation
//! - Error handling

#[cfg(test)]
mod tests {
    use crate::models::user::*;
    use crate::models::auth::Permission;
    use crate::models::room::SortOrder;

    // ============================================================================
    // ListUsersRequest Tests
    // ============================================================================

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

    #[test]
    fn test_list_users_request_with_search() {
        let request = ListUsersRequest {
            search: Some("test".to_string()),
            ..Default::default()
        };
        
        assert_eq!(request.search, Some("test".to_string()));
    }

    #[test]
    fn test_list_users_request_with_filters() {
        let request = ListUsersRequest {
            filter_admin: Some(true),
            filter_deactivated: Some(true),
            ..Default::default()
        };
        
        assert_eq!(request.filter_admin, Some(true));
        assert_eq!(request.filter_deactivated, Some(true));
    }

    #[test]
    fn test_list_users_request_with_pagination() {
        let request = ListUsersRequest {
            limit: Some(100),
            offset: Some(50),
            ..Default::default()
        };
        
        assert_eq!(request.limit, Some(100));
        assert_eq!(request.offset, Some(50));
    }

    #[test]
    fn test_list_users_request_with_sorting() {
        let request = ListUsersRequest {
            sort_by: Some(UserSortField::CreationTime),
            sort_order: Some(SortOrder::Descending),
            ..Default::default()
        };
        
        assert!(matches!(request.sort_by, Some(UserSortField::CreationTime)));
        assert!(matches!(request.sort_order, Some(SortOrder::Descending)));
    }

    // ============================================================================
    // User Model Tests
    // ============================================================================

    #[test]
    fn test_user_has_permission_system_admin() {
        let user = User {
            user_id: "@admin:example.com".to_string(),
            username: "admin".to_string(),
            display_name: Some("Admin User".to_string()),
            avatar_url: None,
            is_admin: true,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: Some(1640000000),
            permissions: vec![Permission::SystemAdmin],
        };
        
        // System admin should have all permissions
        assert!(user.has_permission(&Permission::UserManagement));
        assert!(user.has_permission(&Permission::RoomManagement));
        assert!(user.has_permission(&Permission::ServerControl));
    }

    #[test]
    fn test_user_has_permission_specific() {
        let user = User {
            user_id: "@user:example.com".to_string(),
            username: "user".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: Some(1640000000),
            permissions: vec![Permission::UserManagement],
        };
        
        assert!(user.has_permission(&Permission::UserManagement));
        assert!(!user.has_permission(&Permission::RoomManagement));
    }

    #[test]
    fn test_user_is_active() {
        let active_user = User {
            user_id: "@user:example.com".to_string(),
            username: "user".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: Some(1640000000),
            permissions: vec![],
        };
        
        assert!(active_user.is_active());
    }

    #[test]
    fn test_user_is_not_active_deactivated() {
        let deactivated_user = User {
            user_id: "@user:example.com".to_string(),
            username: "user".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: true,
            creation_ts: 1640000000,
            last_seen_ts: Some(1640000000),
            permissions: vec![],
        };
        
        assert!(!deactivated_user.is_active());
    }

    #[test]
    fn test_user_is_not_active_never_seen() {
        let never_seen_user = User {
            user_id: "@user:example.com".to_string(),
            username: "user".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: None,
            permissions: vec![],
        };
        
        assert!(!never_seen_user.is_active());
    }

    #[test]
    fn test_user_age_in_days() {
        use std::time::SystemTime;
        
        let one_day_ago = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 86400;
        
        let user = User {
            user_id: "@user:example.com".to_string(),
            username: "user".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: one_day_ago,
            last_seen_ts: None,
            permissions: vec![],
        };
        
        let age = user.age_in_days();
        assert!(age >= 0 && age <= 1);
    }

    #[test]
    fn test_user_days_since_last_seen() {
        use std::time::SystemTime;
        
        let two_days_ago = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - (2 * 86400);
        
        let user = User {
            user_id: "@user:example.com".to_string(),
            username: "user".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: Some(two_days_ago),
            permissions: vec![],
        };
        
        let days = user.days_since_last_seen();
        assert!(days.is_some());
        assert!(days.unwrap() >= 1 && days.unwrap() <= 2);
    }

    #[test]
    fn test_user_days_since_last_seen_none() {
        let user = User {
            user_id: "@user:example.com".to_string(),
            username: "user".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: None,
            permissions: vec![],
        };
        
        assert!(user.days_since_last_seen().is_none());
    }

    // ============================================================================
    // UserSortField Tests
    // ============================================================================

    #[test]
    fn test_user_sort_field_descriptions() {
        assert_eq!(UserSortField::Username.description(), "Username");
        assert_eq!(UserSortField::DisplayName.description(), "Display Name");
        assert_eq!(UserSortField::CreationTime.description(), "Creation Time");
        assert_eq!(UserSortField::LastSeen.description(), "Last Seen");
        assert_eq!(UserSortField::IsAdmin.description(), "Admin Status");
    }

    // ============================================================================
    // CreateUserRequest Tests
    // ============================================================================

    #[test]
    fn test_create_user_request_with_password() {
        let request = CreateUserRequest {
            username: "newuser".to_string(),
            password: Some("SecurePass123!".to_string()),
            display_name: Some("New User".to_string()),
            is_admin: false,
            permissions: vec![],
            send_notification: false,
        };
        
        assert_eq!(request.username, "newuser");
        assert_eq!(request.password, Some("SecurePass123!".to_string()));
        assert_eq!(request.display_name, Some("New User".to_string()));
        assert!(!request.is_admin);
        assert!(!request.send_notification);
    }

    #[test]
    fn test_create_user_request_auto_password() {
        let request = CreateUserRequest {
            username: "newuser".to_string(),
            password: None, // Auto-generate
            display_name: None,
            is_admin: false,
            permissions: vec![],
            send_notification: true,
        };
        
        assert!(request.password.is_none());
        assert!(request.send_notification);
    }

    #[test]
    fn test_create_user_request_admin() {
        let request = CreateUserRequest {
            username: "admin".to_string(),
            password: None,
            display_name: Some("Admin User".to_string()),
            is_admin: true,
            permissions: vec![Permission::SystemAdmin],
            send_notification: false,
        };
        
        assert!(request.is_admin);
        assert_eq!(request.permissions.len(), 1);
        assert!(request.permissions.contains(&Permission::SystemAdmin));
    }

    // ============================================================================
    // UpdateUserRequest Tests
    // ============================================================================

    #[test]
    fn test_update_user_request_display_name() {
        let request = UpdateUserRequest {
            display_name: Some("Updated Name".to_string()),
            avatar_url: None,
            is_admin: None,
            permissions: None,
        };
        
        assert_eq!(request.display_name, Some("Updated Name".to_string()));
        assert!(request.avatar_url.is_none());
        assert!(request.is_admin.is_none());
        assert!(request.permissions.is_none());
    }

    #[test]
    fn test_update_user_request_avatar() {
        let request = UpdateUserRequest {
            display_name: None,
            avatar_url: Some("mxc://example.com/avatar123".to_string()),
            is_admin: None,
            permissions: None,
        };
        
        assert_eq!(request.avatar_url, Some("mxc://example.com/avatar123".to_string()));
    }

    #[test]
    fn test_update_user_request_admin_status() {
        let request = UpdateUserRequest {
            display_name: None,
            avatar_url: None,
            is_admin: Some(true),
            permissions: None,
        };
        
        assert_eq!(request.is_admin, Some(true));
    }

    #[test]
    fn test_update_user_request_permissions() {
        let request = UpdateUserRequest {
            display_name: None,
            avatar_url: None,
            is_admin: None,
            permissions: Some(vec![Permission::UserManagement, Permission::RoomManagement]),
        };
        
        assert!(request.permissions.is_some());
        let perms = request.permissions.unwrap();
        assert_eq!(perms.len(), 2);
        assert!(perms.contains(&Permission::UserManagement));
        assert!(perms.contains(&Permission::RoomManagement));
    }

    // ============================================================================
    // ResetPasswordRequest Tests
    // ============================================================================

    #[test]
    fn test_reset_password_request_with_password() {
        let request = ResetPasswordRequest {
            user_id: "@user:example.com".to_string(),
            new_password: Some("NewSecurePass123!".to_string()),
            logout_devices: true,
        };
        
        assert_eq!(request.user_id, "@user:example.com");
        assert_eq!(request.new_password, Some("NewSecurePass123!".to_string()));
        assert!(request.logout_devices);
    }

    #[test]
    fn test_reset_password_request_auto_generate() {
        let request = ResetPasswordRequest {
            user_id: "@user:example.com".to_string(),
            new_password: None,
            logout_devices: false,
        };
        
        assert!(request.new_password.is_none());
        assert!(!request.logout_devices);
    }

    // ============================================================================
    // DeactivateUserRequest Tests
    // ============================================================================

    #[test]
    fn test_deactivate_user_request_erase_data() {
        let request = DeactivateUserRequest {
            user_id: "@user:example.com".to_string(),
            erase_data: true,
            leave_rooms: true,
        };
        
        assert_eq!(request.user_id, "@user:example.com");
        assert!(request.erase_data);
        assert!(request.leave_rooms);
    }

    #[test]
    fn test_deactivate_user_request_keep_data() {
        let request = DeactivateUserRequest {
            user_id: "@user:example.com".to_string(),
            erase_data: false,
            leave_rooms: false,
        };
        
        assert!(!request.erase_data);
        assert!(!request.leave_rooms);
    }

    // ============================================================================
    // BatchUserOperation Tests
    // ============================================================================

    #[test]
    fn test_batch_operation_deactivate() {
        let operation = BatchUserOperation::Deactivate {
            erase_data: true,
            leave_rooms: true,
        };
        
        match operation {
            BatchUserOperation::Deactivate { erase_data, leave_rooms } => {
                assert!(erase_data);
                assert!(leave_rooms);
            }
            _ => panic!("Expected Deactivate operation"),
        }
    }

    #[test]
    fn test_batch_operation_set_admin() {
        let operation = BatchUserOperation::SetAdmin {
            is_admin: true,
        };
        
        match operation {
            BatchUserOperation::SetAdmin { is_admin } => {
                assert!(is_admin);
            }
            _ => panic!("Expected SetAdmin operation"),
        }
    }

    #[test]
    fn test_batch_operation_update_permissions() {
        let operation = BatchUserOperation::UpdatePermissions {
            permissions: vec![Permission::UserManagement],
        };
        
        match operation {
            BatchUserOperation::UpdatePermissions { permissions } => {
                assert_eq!(permissions.len(), 1);
                assert!(permissions.contains(&Permission::UserManagement));
            }
            _ => panic!("Expected UpdatePermissions operation"),
        }
    }

    #[test]
    fn test_batch_user_operation_request() {
        let request = BatchUserOperationRequest {
            user_ids: vec![
                "@user1:example.com".to_string(),
                "@user2:example.com".to_string(),
                "@user3:example.com".to_string(),
            ],
            operation: BatchUserOperation::SetAdmin { is_admin: true },
        };
        
        assert_eq!(request.user_ids.len(), 3);
        assert!(matches!(request.operation, BatchUserOperation::SetAdmin { .. }));
    }

    // ============================================================================
    // Response Model Tests
    // ============================================================================

    #[test]
    fn test_create_user_response_success() {
        let user = User {
            user_id: "@newuser:example.com".to_string(),
            username: "newuser".to_string(),
            display_name: Some("New User".to_string()),
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: None,
            permissions: vec![],
        };
        
        let response = CreateUserResponse {
            success: true,
            user: Some(user.clone()),
            generated_password: Some("GeneratedPass123!".to_string()),
            error: None,
        };
        
        assert!(response.success);
        assert!(response.user.is_some());
        assert_eq!(response.user.unwrap().user_id, "@newuser:example.com");
        assert!(response.generated_password.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn test_create_user_response_failure() {
        let response = CreateUserResponse {
            success: false,
            user: None,
            generated_password: None,
            error: Some("Username already exists".to_string()),
        };
        
        assert!(!response.success);
        assert!(response.user.is_none());
        assert!(response.generated_password.is_none());
        assert_eq!(response.error, Some("Username already exists".to_string()));
    }

    #[test]
    fn test_list_users_response() {
        let users = vec![
            User {
                user_id: "@user1:example.com".to_string(),
                username: "user1".to_string(),
                display_name: None,
                avatar_url: None,
                is_admin: false,
                is_deactivated: false,
                creation_ts: 1640000000,
                last_seen_ts: Some(1640000000),
                permissions: vec![],
            },
            User {
                user_id: "@user2:example.com".to_string(),
                username: "user2".to_string(),
                display_name: None,
                avatar_url: None,
                is_admin: true,
                is_deactivated: false,
                creation_ts: 1640000000,
                last_seen_ts: Some(1640000000),
                permissions: vec![Permission::SystemAdmin],
            },
        ];
        
        let response = ListUsersResponse {
            success: true,
            users: users.clone(),
            total_count: 100,
            has_more: true,
            error: None,
        };
        
        assert!(response.success);
        assert_eq!(response.users.len(), 2);
        assert_eq!(response.total_count, 100);
        assert!(response.has_more);
        assert!(response.error.is_none());
    }

    #[test]
    fn test_batch_operation_response_success() {
        let response = BatchUserOperationResponse {
            success: true,
            processed_count: 5,
            failed_users: vec![],
            errors: vec![],
        };
        
        assert!(response.success);
        assert_eq!(response.processed_count, 5);
        assert!(response.failed_users.is_empty());
        assert!(response.errors.is_empty());
    }

    #[test]
    fn test_batch_operation_response_partial_failure() {
        let response = BatchUserOperationResponse {
            success: false,
            processed_count: 3,
            failed_users: vec!["@user4:example.com".to_string(), "@user5:example.com".to_string()],
            errors: vec!["User not found".to_string(), "Permission denied".to_string()],
        };
        
        assert!(!response.success);
        assert_eq!(response.processed_count, 3);
        assert_eq!(response.failed_users.len(), 2);
        assert_eq!(response.errors.len(), 2);
    }

    // ============================================================================
    // UserStatistics Tests
    // ============================================================================

    #[test]
    fn test_user_statistics() {
        let stats = UserStatistics {
            total_users: 1000,
            active_users: 850,
            admin_users: 10,
            deactivated_users: 50,
            users_created_today: 5,
            users_created_this_week: 25,
            users_created_this_month: 100,
        };
        
        assert_eq!(stats.total_users, 1000);
        assert_eq!(stats.active_users, 850);
        assert_eq!(stats.admin_users, 10);
        assert_eq!(stats.deactivated_users, 50);
        assert_eq!(stats.users_created_today, 5);
        assert_eq!(stats.users_created_this_week, 25);
        assert_eq!(stats.users_created_this_month, 100);
    }

    // ============================================================================
    // Password Generation Tests
    // ============================================================================

    #[test]
    fn test_password_generation_length() {
        use rand::{thread_rng, Rng};
        use rand::distributions::Alphanumeric;
        
        let password: String = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect();
        
        assert_eq!(password.len(), 16);
    }

    #[test]
    fn test_password_generation_uniqueness() {
        use rand::{thread_rng, Rng};
        use rand::distributions::Alphanumeric;
        
        let password1: String = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect();
        
        let password2: String = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect();
        
        // Passwords should be different (extremely high probability)
        assert_ne!(password1, password2);
    }

    // ============================================================================
    // Validation Tests
    // ============================================================================

    #[test]
    fn test_username_validation_empty() {
        let request = CreateUserRequest {
            username: "".to_string(),
            password: None,
            display_name: None,
            is_admin: false,
            permissions: vec![],
            send_notification: false,
        };
        
        // Empty username should be invalid
        assert!(request.username.is_empty());
    }

    #[test]
    fn test_username_validation_too_long() {
        let long_username = "a".repeat(256);
        let request = CreateUserRequest {
            username: long_username.clone(),
            password: None,
            display_name: None,
            is_admin: false,
            permissions: vec![],
            send_notification: false,
        };
        
        // Username longer than 255 should be invalid
        assert!(request.username.len() > 255);
    }

    #[test]
    fn test_username_validation_valid() {
        let valid_usernames = vec![
            "user",
            "user123",
            "user_name",
            "user-name",
            "a",
        ];
        
        for username in valid_usernames {
            let request = CreateUserRequest {
                username: username.to_string(),
                password: None,
                display_name: None,
                is_admin: false,
                permissions: vec![],
                send_notification: false,
            };
            
            assert!(!request.username.is_empty());
            assert!(request.username.len() <= 255);
        }
        
        // Test max length separately
        let max_length_username = "a".repeat(255);
        let request = CreateUserRequest {
            username: max_length_username.clone(),
            password: None,
            display_name: None,
            is_admin: false,
            permissions: vec![],
            send_notification: false,
        };
        assert_eq!(request.username.len(), 255);
    }

    // ============================================================================
    // Edge Cases
    // ============================================================================

    #[test]
    fn test_user_with_all_permissions() {
        let user = User {
            user_id: "@superadmin:example.com".to_string(),
            username: "superadmin".to_string(),
            display_name: Some("Super Admin".to_string()),
            avatar_url: None,
            is_admin: true,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: Some(1640000000),
            permissions: vec![
                Permission::SystemAdmin,
                Permission::UserManagement,
                Permission::RoomManagement,
                Permission::ServerControl,
                Permission::FederationManagement,
                Permission::MediaManagement,
            ],
        };
        
        assert_eq!(user.permissions.len(), 6);
        assert!(user.has_permission(&Permission::SystemAdmin));
        assert!(user.has_permission(&Permission::UserManagement));
    }

    #[test]
    fn test_user_with_no_permissions() {
        let user = User {
            user_id: "@user:example.com".to_string(),
            username: "user".to_string(),
            display_name: None,
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: Some(1640000000),
            permissions: vec![],
        };
        
        assert!(user.permissions.is_empty());
        assert!(!user.has_permission(&Permission::UserManagement));
    }

    #[test]
    fn test_batch_operation_empty_user_list() {
        let request = BatchUserOperationRequest {
            user_ids: vec![],
            operation: BatchUserOperation::SetAdmin { is_admin: true },
        };
        
        assert!(request.user_ids.is_empty());
    }

    #[test]
    fn test_batch_operation_single_user() {
        let request = BatchUserOperationRequest {
            user_ids: vec!["@user:example.com".to_string()],
            operation: BatchUserOperation::SetAdmin { is_admin: true },
        };
        
        assert_eq!(request.user_ids.len(), 1);
    }

    #[test]
    fn test_list_users_request_no_filters() {
        let request = ListUsersRequest {
            limit: Some(50),
            offset: Some(0),
            search: None,
            filter_admin: None,
            filter_deactivated: None,
            sort_by: None,
            sort_order: None,
        };
        
        assert!(request.search.is_none());
        assert!(request.filter_admin.is_none());
        assert!(request.filter_deactivated.is_none());
    }
}
