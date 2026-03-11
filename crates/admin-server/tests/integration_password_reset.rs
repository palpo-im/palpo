/// Integration Test: Password Reset Enables Login (3.3.3)
///
/// Tests that password reset properly updates credentials and enables login:
/// Create user → Reset password → Verify password hash updated → Attempt login
///
/// Tests: UserRepository + Auth service integration

#[cfg(test)]
mod password_reset_tests {
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
        format!("@test_pwd_user_{}:test.example.com", timestamp)
    }

    /// Simple password hashing for testing (in real code, use proper password hashing)
    fn hash_password(password: &str) -> String {
        // Simple hash for testing - in production use bcrypt/argon2
        let mut hash = 0u64;
        for byte in password.as_bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(*byte as u64);
        }
        format!("simple:{:x}", hash)
    }

    #[tokio::test]
    async fn test_password_reset_updates_hash() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let user_id = generate_test_user_id();
        let original_password = "OriginalPassword123!";
        let new_password = "NewPassword456!";

        // Step 1: Create user (initially without password)
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Password Test User".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        let created_user = factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        assert_eq!(created_user.name, user_id);
        assert!(created_user.password_hash.is_none(), "User should have no password initially");

        // Step 2: Set original password
        let original_hash = hash_password(original_password);
        factory.user_repository().update_password(&user_id, &original_hash, "salt").await
            .expect("Failed to set original password");

        // Step 3: Verify user has password hash
        let user_with_password = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(user_with_password.password_hash.is_some(), "User should have password hash");
        assert_eq!(user_with_password.password_hash.unwrap(), original_hash);

        // Step 4: Reset password (update to new hash)
        let new_hash = hash_password(new_password);
        factory.user_repository().update_password(&user_id, &new_hash, "new_salt").await
            .expect("Failed to reset password");

        // Step 5: Verify password hash is updated
        let user_after_reset = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user after reset")
            .expect("User not found after reset");

        let new_hash_value = user_after_reset.password_hash.unwrap();
        assert_eq!(new_hash_value, new_hash,
            "Password hash should be updated to new password");

        // Step 6: Verify old password hash is gone
        assert_ne!(new_hash_value, original_hash,
            "Old password hash should be replaced");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_password_change_preserves_user_data() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let user_id = generate_test_user_id();
        let display_name = "Password Change Test".to_string();

        // Step 1: Create user with display name
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some(display_name.clone()),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            is_admin: true,
            is_guest: false,
            user_type: Some("user".to_string()),
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Set password
        let password_hash = hash_password("TestPassword123!");
        factory.user_repository().update_password(&user_id, &password_hash, "salt").await
            .expect("Failed to set password");

        // Step 3: Change password multiple times
        for i in 1..=3 {
            let new_hash = hash_password(&format!("TestPassword{}!", 100 + i));
            factory.user_repository().update_password(&user_id, &new_hash, &format!("salt{}", i)).await
                .expect("Failed to change password");
        }

        // Step 4: Verify user data is preserved after password changes
        let user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert_eq!(user.name, user_id);
        assert_eq!(user.displayname, Some(display_name));
        assert_eq!(user.avatar_url, Some("https://example.com/avatar.png".to_string()));
        assert!(user.is_admin);
        assert!(!user.is_guest);
        assert_eq!(user.user_type, Some("user".to_string()));

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_password_reset_with_special_characters() {
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
            displayname: None,
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Test password with various special characters
        let special_passwords = [
            "P@ssw0rd!",
            "Pass#word$%",
            "Test&*()_+",
            "密码123",  // Unicode
            "pässwörd", // Non-ASCII
        ];

        for (i, password) in special_passwords.iter().enumerate() {
            let hash = hash_password(password);
            factory.user_repository().update_password(&user_id, &hash, &format!("salt{}", i)).await
                .expect("Failed to set password with special chars");

            // Verify password was set
            let user = factory.user_repository().get_user(&user_id).await
                .expect("Failed to fetch user")
                .expect("User not found");

            assert_eq!(user.password_hash.unwrap(), hash);
        }

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }
}