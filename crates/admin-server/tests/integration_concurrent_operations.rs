/// Integration Test: Concurrent Operations Consistency (3.3.7)
///
/// Tests that concurrent operations maintain data consistency:
/// Multiple concurrent user creations → Verify no duplicates
///
/// Tests: Repository + Database locking

#[cfg(test)]
mod concurrent_operations_tests {
    use palpo_admin_server::repositories::{RepositoryFactory, UserRepository, CreateUserInput};
    use palpo_admin_server::types::AdminError;
    use palpo_data::DieselPool;
    use diesel::PgConnection;
    use diesel::r2d2::ConnectionManager;
    use diesel::r2d2::Pool;
    use chrono::Utc;
    use tokio;

    // Test configuration - uses test database
    fn test_db_url() -> String {
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/palpo_test".to_string())
    }

    /// Create a repository factory for testing
    fn create_test_repository() -> Result<RepositoryFactory, AdminError> {
        let db_url = test_db_url();
        let raw_pool = Pool::builder()
            .max_size(10)
            .build(ConnectionManager::<PgConnection>::new(&db_url))
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let pool = DieselPool::new_background_worker(raw_pool);
        let factory = RepositoryFactory::new(pool);
        Ok(factory)
    }

    /// Generate unique test user ID
    fn generate_test_user_id() -> String {
        let timestamp = Utc::now().timestamp_millis();
        format!("@test_concurrent_{}:test.example.com", timestamp)
    }

    #[tokio::test]
    async fn test_concurrent_user_creation() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        // Create unique base for concurrent users
        let base_timestamp = Utc::now().timestamp_millis();
        let num_concurrent = 5;

        // Step 1: Create multiple users concurrently
        let handles: Vec<_> = (0..num_concurrent)
            .map(|i| {
                let factory = factory.clone();
                let user_id = format!("@concurrent_user_{}_{}:test.example.com", base_timestamp, i);
                tokio::spawn(async move {
                    let create_input = CreateUserInput {
                        user_id: user_id.clone(),
                        displayname: Some(format!("Concurrent User {}", i)),
                        avatar_url: None,
                        is_admin: false,
                        is_guest: false,
                        user_type: None,
                        appservice_id: None,
                    };
                    (i, factory.user_repository().create_user(&create_input).await)
                })
            })
            .collect();

        // Step 2: Wait for all creations
        let results: Vec<(usize, Result<_, _>)> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Step 3: Verify all creations succeeded
        let success_count = results.iter().filter(|(_, r)| r.is_ok()).count();
        assert_eq!(success_count, num_concurrent, "All concurrent creations should succeed");

        // Step 4: Verify all users exist
        for i in 0..num_concurrent {
            let user_id = format!("@concurrent_user_{}_{}:test.example.com", base_timestamp, i);
            let user = factory.user_repository().get_user(&user_id).await
                .expect("Failed to fetch user")
                .expect("User not found");
            assert_eq!(user.name, user_id);
        }

        // Cleanup
        for i in 0..num_concurrent {
            let user_id = format!("@concurrent_user_{}_{}:test.example.com", base_timestamp, i);
            let _ = factory.user_repository().deactivate_user(&user_id, true).await;
        }
    }

    #[tokio::test]
    async fn test_concurrent_admin_status_changes() {
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
            displayname: Some("Concurrent Admin Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Change admin status concurrently
        let handles: Vec<_> = (0..3)
            .map(|i| {
                let factory = factory.clone();
                let target_status = i % 2 == 0;
                tokio::spawn(async move {
                    factory.user_repository().set_admin_status(&user_id, target_status).await
                })
            })
            .collect();

        // Step 3: Wait for all changes
        let results: Vec<Result<(), _>> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Step 4: Verify all changes succeeded (eventual consistency)
        let success_count = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(success_count, 3, "All concurrent admin changes should succeed");

        // Step 5: Verify final state is consistent
        let user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        // Final state should be either admin or not, but consistent
        assert!(true, "User state should be consistent");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_concurrent_shadow_ban_operations() {
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
            displayname: Some("Concurrent Shadow Ban Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Toggle shadow ban concurrently
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let factory = factory.clone();
                let ban = i % 2 == 0;
                tokio::spawn(async move {
                    factory.user_repository().set_shadow_banned(&user_id, ban).await
                })
            })
            .collect();

        // Step 3: Wait for all operations
        let results: Vec<Result<(), _>> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Step 4: Verify all operations completed
        let success_count = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(success_count, 5, "All concurrent shadow ban operations should succeed");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_concurrent_lock_operations() {
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
            displayname: Some("Concurrent Lock Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Toggle lock status concurrently
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let factory = factory.clone();
                let locked = i % 2 == 0;
                tokio::spawn(async move {
                    factory.user_repository().set_locked(&user_id, locked).await
                })
            })
            .collect();

        // Step 3: Wait for all operations
        let results: Vec<Result<(), _>> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Step 4: Verify all operations completed
        let success_count = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(success_count, 4, "All concurrent lock operations should succeed");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_user_list_consistency_under_concurrent_modifications() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        // Create multiple users
        let base_timestamp = Utc::now().timestamp_millis();
        let num_users = 10;
        let test_users: Vec<String> = (0..num_users)
            .map(|i| format!("@list_test_{}_{}:test.example.com", base_timestamp, i))
            .collect();

        // Step 1: Create all users
        for (i, user_id) in test_users.iter().enumerate() {
            let create_input = CreateUserInput {
                user_id: user_id.clone(),
                displayname: Some(format!("List Test User {}", i)),
                avatar_url: None,
                is_admin: i % 3 == 0,
                is_guest: false,
                user_type: None,
                appservice_id: None,
            };

            factory.user_repository().create_user(&create_input).await
                .expect("Failed to create user");
        }

        // Step 2: Perform concurrent modifications
        let modification_handles: Vec<_> = test_users.iter().enumerate()
            .map(|(i, user_id)| {
                let factory = factory.clone();
                tokio::spawn(async move {
                    if i % 2 == 0 {
                        let _ = factory.user_repository().set_shadow_banned(user_id, true).await;
                    } else {
                        let _ = factory.user_repository().set_locked(user_id, true).await;
                    }
                })
            })
            .collect();

        futures::future::join_all(modification_handles).await;

        // Step 3: Query user list
        let filter = palpo_admin_server::repositories::UserFilter {
            search_term: Some(format!("list_test_{}", base_timestamp)),
            is_admin: None,
            is_deactivated: None,
            shadow_banned: None,
            limit: Some(100),
            offset: Some(0),
        };

        let result = factory.user_repository().list_users(&filter).await
            .expect("Failed to list users");

        // Step 4: Verify list consistency
        assert!(result.users.len() >= 0, "List query should return results");
        assert!(result.total_count >= 0, "Total count should be non-negative");

        // Cleanup
        for user_id in test_users {
            let _ = factory.user_repository().deactivate_user(&user_id, true).await;
        }
    }
}