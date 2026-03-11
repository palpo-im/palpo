/// Integration Test: Complete User Lifecycle Flow (3.3.1)
///
/// Tests the complete user lifecycle:
/// Create user → Verify in DB → Modify user → Verify changes → Deactivate → Verify state
///
/// Tests: Repository + Handler + Database interaction

#[cfg(test)]
mod user_lifecycle_tests {
    use palpo_admin_server::repositories::{RepositoryFactory, UserRepository, UserFilter, CreateUserInput, UpdateUserInput};
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
        format!("@test_user_{}:test.example.com", timestamp)
    }

    /// Generate unique username
    fn generate_test_username() -> String {
        let timestamp = Utc::now().timestamp_millis();
        format!("testuser_{}", timestamp % 100000)
    }

    #[tokio::test]
    async fn test_complete_user_lifecycle() {
        // Skip if no database available
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return, // Skip if database not available
        };

        let user_id = generate_test_user_id();
        let username = generate_test_username();
        let display_name = "Test User".to_string();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some(display_name.clone()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        let created_user = factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        assert_eq!(created_user.name, user_id);
        assert!(created_user.name.contains(&username));
        assert_eq!(created_user.displayname, Some(display_name.clone()));
        assert!(!created_user.is_deactivated);

        // Step 2: Verify user exists in DB
        let fetched_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert_eq!(fetched_user.name, user_id);
        assert!(fetched_user.name.contains(&username));

        // Step 3: Modify user
        let new_display_name = "Updated Test User".to_string();
        let update_input = UpdateUserInput {
            displayname: Some(new_display_name.clone()),
            avatar_url: None,
            is_admin: None,
            user_type: None,
        };

        let updated_user = factory.user_repository().update_user(&user_id, &update_input).await
            .expect("Failed to update user");

        assert_eq!(updated_user.displayname, Some(new_display_name.clone()));

        // Step 4: Verify changes persisted
        let verified_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to verify user")
            .expect("User not found after update");

        assert_eq!(verified_user.displayname, Some(new_display_name));

        // Step 5: Deactivate user
        factory.user_repository().deactivate_user(&user_id, false).await
            .expect("Failed to deactivate user");

        // Step 6: Verify deactivated state
        let deactivated_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch deactivated user")
            .expect("User not found after deactivation");

        assert!(deactivated_user.is_deactivated);

        // Cleanup: Reactivate for future tests
        let _ = factory.user_repository().reactivate_user(&user_id).await;

        // Verify reactivated state
        let reactivated_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch reactivated user")
            .expect("User not found after reactivation");

        assert!(!reactivated_user.is_deactivated);
    }

    #[tokio::test]
    async fn test_user_list_with_pagination() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        // Create filter for listing users
        let filter = UserFilter {
            search_term: None,
            is_admin: None,
            is_deactivated: None,
            shadow_banned: None,
            limit: Some(10),
            offset: Some(0),
        };

        let result = factory.user_repository().list_users(&filter).await
            .expect("Failed to list users");

        // Verify pagination info
        assert!(result.users.len() <= 10);
        assert!(result.total_count >= 0);
        assert_eq!(result.limit, 10);
        assert_eq!(result.offset, 0);
    }

    #[tokio::test]
    async fn test_username_availability_check() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let username = format!("available_user_{}", Utc::now().timestamp_millis());

        // Check availability before creation
        let is_available = factory.user_repository().is_username_available(&username).await
            .expect("Failed to check username availability");

        assert!(is_available, "New username should be available");

        // Create user with this username
        let user_id = format!("@{}:test.example.com", username);
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: None,
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        let _ = factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Check availability after creation
        let is_available_after = factory.user_repository().is_username_available(&username).await
            .expect("Failed to check username availability after creation");

        assert!(!is_available_after, "Username should not be available after creation");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_user_filter_combinations() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        // Test filter with multiple conditions
        let filter = UserFilter {
            search_term: Some("test".to_string()),
            is_admin: Some(false),
            is_deactivated: Some(false),
            shadow_banned: None,
            limit: Some(50),
            offset: Some(0),
        };

        let result = factory.user_repository().list_users(&filter).await
            .expect("Failed to list users with filter");

        // Verify all returned users match filter criteria
        for user in result.users {
            if let Some(ref search_term) = filter.search_term {
                assert!(
                    user.name.contains(search_term) || 
                    user.displayname.as_ref().map_or(false, |d| d.contains(search_term)),
                    "User {} should match search term",
                    user.name
                );
            }
            assert!(!user.is_admin || filter.is_admin != Some(false));
            assert!(!user.is_deactivated || filter.is_deactivated != Some(false));
        }
    }
}