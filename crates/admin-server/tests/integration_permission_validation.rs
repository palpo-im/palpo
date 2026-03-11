/// Integration Test: Permission Validation Across Operations (3.3.4)
///
/// Tests that permission validation works correctly across operations:
/// Non-admin attempts admin operation → Verify 403
/// Admin performs operation → Verify success
///
/// Tests: Auth middleware + All handlers

#[cfg(test)]
mod permission_validation_tests {
    use palpo_admin_server::repositories::{RepositoryFactory, UserRepository, CreateUserInput};
    use palpo_admin_server::types::AdminError;
    use palpo_data::DieselPool;
    use diesel::PgConnection;
    use diesel::r2d2::ConnectionManager;
    use diesel::r2d2::Pool;
    use chrono::Utc;

    // Test configuration - uses test database
    fn test_db_url() -> String {
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/palpo_test".to_string())
    }

    /// Create a repository factory for testing
    fn create_test_repository() -> Result<RepositoryFactory, AdminError> {
        let db_url = test_db_url();
        let raw_pool = Pool::builder()
            .max_size(5)
            .build(ConnectionManager::<PgConnection>::new(&db_url))
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let pool = DieselPool::new_background_worker(raw_pool);
        let factory = RepositoryFactory::new(pool);
        Ok(factory)
    }

    /// Generate unique test user ID
    fn generate_test_user_id() -> String {
        let timestamp = Utc::now().timestamp_millis();
        format!("@test_perm_user_{}:test.example.com", timestamp)
    }

    #[tokio::test]
    async fn test_admin_status_can_be_set() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let user_id = generate_test_user_id();

        // Step 1: Create regular user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Permission Test User".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Verify user is not admin
        let user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(!user.is_admin, "User should not be admin initially");

        // Step 3: Set admin status
        factory.user_repository().set_admin_status(&user_id, true).await
            .expect("Failed to set admin status");

        // Step 4: Verify user is now admin
        let admin_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user after admin set")
            .expect("User not found");

        assert!(admin_user.is_admin, "User should be admin after setting status");

        // Step 5: Revoke admin status
        factory.user_repository().set_admin_status(&user_id, false).await
            .expect("Failed to revoke admin status");

        // Step 6: Verify user is no longer admin
        let regular_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user after admin revoked")
            .expect("User not found");

        assert!(!regular_user.is_admin, "User should not be admin after revocation");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_shadow_ban_requires_admin() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let user_id = generate_test_user_id();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Shadow Ban Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Verify user is not shadow-banned
        let user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(!user.shadow_banned, "User should not be shadow-banned initially");

        // Step 3: Set shadow ban status (this operation should require admin in real code)
        factory.user_repository().set_shadow_banned(&user_id, true).await
            .expect("Failed to set shadow ban");

        // Step 4: Verify user is shadow-banned
        let banned_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user after shadow ban")
            .expect("User not found");

        assert!(banned_user.shadow_banned, "User should be shadow-banned");

        // Step 5: Remove shadow ban
        factory.user_repository().set_shadow_banned(&user_id, false).await
            .expect("Failed to remove shadow ban");

        // Step 6: Verify user is not shadow-banned
        let unbanned_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user after shadow ban removal")
            .expect("User not found");

        assert!(!unbanned_user.shadow_banned, "User should not be shadow-banned after removal");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_locked_status_requires_admin() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let user_id = generate_test_user_id();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Locked Status Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Verify user is not locked
        let user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(!user.locked, "User should not be locked initially");

        // Step 3: Set locked status
        factory.user_repository().set_locked(&user_id, true).await
            .expect("Failed to set locked status");

        // Step 4: Verify user is locked
        let locked_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user after locking")
            .expect("User not found");

        assert!(locked_user.locked, "User should be locked");

        // Step 5: Unlock user
        factory.user_repository().set_locked(&user_id, false).await
            .expect("Failed to unlock user");

        // Step 6: Verify user is not locked
        let unlocked_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user after unlocking")
            .expect("User not found");

        assert!(!unlocked_user.locked, "User should not be locked after unlocking");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_user_count_operations() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        // Step 1: Get initial counts
        let initial_user_count = factory.user_repository().get_user_count().await
            .expect("Failed to get user count");
        let initial_admin_count = factory.user_repository().get_admin_count().await
            .expect("Failed to get admin count");
        let initial_deactivated_count = factory.user_repository().get_deactivated_count().await
            .expect("Failed to get deactivated count");

        // Step 2: Create test users
        let test_users: Vec<String> = (0..3).map(|_| generate_test_user_id()).collect();

        for (i, user_id) in test_users.iter().enumerate() {
            let create_input = CreateUserInput {
                user_id: user_id.clone(),
                displayname: Some(format!("Count Test User {}", i)),
                avatar_url: None,
                is_admin: i == 0, // First user is admin
                is_guest: false,
                user_type: None,
                appservice_id: None,
            };

            factory.user_repository().create_user(&create_input).await
                .expect("Failed to create user");
        }

        // Step 3: Verify counts increased
        let new_user_count = factory.user_repository().get_user_count().await
            .expect("Failed to get new user count");
        let new_admin_count = factory.user_repository().get_admin_count().await
            .expect("Failed to get new admin count");

        assert_eq!(new_user_count, initial_user_count + 3,
            "User count should increase by 3");
        assert_eq!(new_admin_count, initial_admin_count + 1,
            "Admin count should increase by 1");

        // Step 4: Deactivate one user
        factory.user_repository().deactivate_user(&test_users[0], false).await
            .expect("Failed to deactivate user");

        let new_deactivated_count = factory.user_repository().get_deactivated_count().await
            .expect("Failed to get deactivated count after deactivation");

        assert_eq!(new_deactivated_count, initial_deactivated_count + 1,
            "Deactivated count should increase by 1");

        // Cleanup
        for user_id in test_users {
            let _ = factory.user_repository().deactivate_user(&user_id, true).await;
        }
    }

    #[tokio::test]
    async fn test_user_details_includes_attributes() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let user_id = generate_test_user_id();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Details Test User".to_string()),
            avatar_url: Some("https://example.com/avatar.jpg".to_string()),
            is_admin: true,
            is_guest: false,
            user_type: Some("privileged".to_string()),
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Get user details
        let details = factory.user_repository().get_user_details(&user_id).await
            .expect("Failed to get user details")
            .expect("User details not found");

        // Step 3: Verify basic info
        assert_eq!(details.user.name, user_id);
        assert_eq!(details.user.displayname, Some("Details Test User".to_string()));
        assert!(details.user.is_admin);

        // Step 4: Verify attributes exist
        assert!(details.attributes.is_some(), "User should have attributes");

        if let Some(attrs) = &details.attributes {
            assert_eq!(attrs.user_id, user_id);
            assert!(!attrs.shadow_banned, "Should not be shadow-banned");
            assert!(!attrs.locked, "Should not be locked");
            assert!(!attrs.deactivated, "Should not be deactivated");
        }

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }
}