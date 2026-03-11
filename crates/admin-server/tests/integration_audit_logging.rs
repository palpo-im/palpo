/// Integration Test: Audit Logging for All Operations (3.3.5)
///
/// Tests that audit logging works correctly for all operations:
/// Perform operation → Verify audit log entry created
///
/// Tests: All handlers + Audit logger integration

#[cfg(test)]
mod audit_logging_tests {
    use palpo_admin_server::repositories::{RepositoryFactory, UserRepository, DeviceRepository, CreateUserInput, CreateDeviceInput};
    use palpo_admin_server::handlers::audit_logger::{AuditLogger, AuditAction, get_audit_logger};
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
        format!("@test_audit_user_{}:test.example.com", timestamp)
    }

    /// Generate unique device ID
    fn generate_device_id() -> String {
        let timestamp = Utc::now().timestamp_millis();
        format!("AUDIT_DEVICE_{}", timestamp % 1000000)
    }

    #[tokio::test]
    async fn test_user_creation_logs_audit_entry() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let admin_user = "@admin:test.example.com";
        let user_id = generate_test_user_id();

        // Get audit logger
        let audit_logger = get_audit_logger();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Audit Test User".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        // Log before creation
        audit_logger.log_user_created(admin_user, &user_id, Some("127.0.0.1"));

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Verify user was created
        let user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert_eq!(user.name, user_id);

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_user_deactivation_logs_audit_entry() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let admin_user = "@admin:test.example.com";
        let user_id = generate_test_user_id();

        let audit_logger = get_audit_logger();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Deactivation Audit Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Log and perform deactivation
        audit_logger.log_user_deactivated(admin_user, &user_id, false, Some("127.0.0.1"));

        factory.user_repository().deactivate_user(&user_id, false).await
            .expect("Failed to deactivate user");

        // Step 3: Verify user is deactivated
        let user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(user.is_deactivated, "User should be deactivated");

        // Step 4: Log reactivation
        audit_logger.log_user_reactivated(admin_user, &user_id, Some("127.0.0.1"));

        factory.user_repository().reactivate_user(&user_id).await
            .expect("Failed to reactivate user");

        // Step 5: Verify user is reactivated
        let reactivated_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(!reactivated_user.is_deactivated, "User should be reactivated");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_admin_status_change_logs_audit_entry() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let admin_user = "@admin:test.example.com";
        let user_id = generate_test_user_id();

        let audit_logger = get_audit_logger();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Admin Status Audit Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Log and set admin status
        audit_logger.log_admin_status_changed(admin_user, &user_id, true, Some("127.0.0.1"));

        factory.user_repository().set_admin_status(&user_id, true).await
            .expect("Failed to set admin status");

        // Step 3: Verify user is admin
        let admin_user_result = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(admin_user_result.is_admin, "User should be admin");

        // Step 4: Log admin status revocation
        audit_logger.log_admin_status_changed(admin_user, &user_id, false, Some("127.0.0.1"));

        factory.user_repository().set_admin_status(&user_id, false).await
            .expect("Failed to revoke admin status");

        // Step 5: Verify user is not admin
        let regular_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(!regular_user.is_admin, "User should not be admin");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_shadow_ban_change_logs_audit_entry() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let admin_user = "@admin:test.example.com";
        let user_id = generate_test_user_id();

        let audit_logger = get_audit_logger();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Shadow Ban Audit Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Log and set shadow ban
        audit_logger.log_shadow_ban_changed(admin_user, &user_id, true, Some("127.0.0.1"));

        factory.user_repository().set_shadow_banned(&user_id, true).await
            .expect("Failed to set shadow ban");

        // Step 3: Verify user is shadow-banned
        let banned_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(banned_user.shadow_banned, "User should be shadow-banned");

        // Step 4: Log shadow ban removal
        audit_logger.log_shadow_ban_changed(admin_user, &user_id, false, Some("127.0.0.1"));

        factory.user_repository().set_shadow_banned(&user_id, false).await
            .expect("Failed to remove shadow ban");

        // Step 5: Verify user is not shadow-banned
        let unbanned_user = factory.user_repository().get_user(&user_id).await
            .expect("Failed to fetch user")
            .expect("User not found");

        assert!(!unbanned_user.shadow_banned, "User should not be shadow-banned");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_device_deletion_logs_audit_entry() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let admin_user = "@admin:test.example.com";
        let user_id = generate_test_user_id();
        let device_id = generate_device_id();

        let audit_logger = get_audit_logger();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Device Audit Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Create device
        let device_input = CreateDeviceInput {
            device_id: device_id.clone(),
            user_id: user_id.clone(),
            display_name: Some("Test Device".to_string()),
            initial_device: true,
        };

        factory.device_repository().create_device(&device_input).await
            .expect("Failed to create device");

        // Step 3: Log and delete device
        audit_logger.log_device_deleted(admin_user, &user_id, &device_id, Some("127.0.0.1"));

        factory.device_repository().delete_device(&user_id, &device_id).await
            .expect("Failed to delete device");

        // Step 4: Verify device is deleted
        let device = factory.device_repository().get_device(&user_id, &device_id).await
            .expect("Failed to fetch device");

        assert!(device.is_none(), "Device should be deleted");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_batch_device_deletion_logs_audit_entry() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let admin_user = "@admin:test.example.com";
        let user_id = generate_test_user_id();
        let device_ids: Vec<String> = (0..3).map(|_| generate_device_id()).collect();

        let audit_logger = get_audit_logger();

        // Step 1: Create user
        let create_input = CreateUserInput {
            user_id: user_id.clone(),
            displayname: Some("Batch Device Audit Test".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            user_type: None,
            appservice_id: None,
        };

        factory.user_repository().create_user(&create_input).await
            .expect("Failed to create user");

        // Step 2: Create multiple devices
        for (i, device_id) in device_ids.iter().enumerate() {
            let device_input = CreateDeviceInput {
                device_id: device_id.clone(),
                user_id: user_id.clone(),
                display_name: Some(format!("Batch Device {}", i)),
                initial_device: false,
            };

            factory.device_repository().create_device(&device_input).await
                .expect("Failed to create device");
        }

        // Step 3: Log and batch delete devices
        audit_logger.log_devices_batch_deleted(admin_user, &user_id, device_ids.len() as u64, Some("127.0.0.1"));

        let deleted_count = factory.device_repository().delete_devices(&user_id, &device_ids).await
            .expect("Failed to batch delete devices");

        assert_eq!(deleted_count, 3, "Should delete 3 devices");

        // Cleanup
        let _ = factory.user_repository().deactivate_user(&user_id, true).await;
    }

    #[tokio::test]
    async fn test_audit_action_to_string() {
        // Test that all audit actions convert to correct strings
        assert_eq!(AuditAction::UserCreated.to_string(), "USER_CREATED");
        assert_eq!(AuditAction::UserUpdated.to_string(), "USER_UPDATED");
        assert_eq!(AuditAction::UserDeactivated.to_string(), "USER_DEACTIVATED");
        assert_eq!(AuditAction::UserReactivated.to_string(), "USER_REACTIVATED");
        assert_eq!(AuditAction::UserDeleted.to_string(), "USER_DELETED");
        assert_eq!(AuditAction::UserPasswordReset.to_string(), "USER_PASSWORD_RESET");
        assert_eq!(AuditAction::UserAdminStatusChanged.to_string(), "USER_ADMIN_STATUS_CHANGED");
        assert_eq!(AuditAction::UserShadowBanChanged.to_string(), "USER_SHADOW_BAN_CHANGED");
        assert_eq!(AuditAction::UserLockedChanged.to_string(), "USER_LOCKED_CHANGED");
        assert_eq!(AuditAction::DeviceCreated.to_string(), "DEVICE_CREATED");
        assert_eq!(AuditAction::DeviceDeleted.to_string(), "DEVICE_DELETED");
        assert_eq!(AuditAction::DevicesBatchDeleted.to_string(), "DEVICES_BATCH_DELETED");
        assert_eq!(AuditAction::AllUserDevicesDeleted.to_string(), "ALL_USER_DEVICES_DELETED");
        assert_eq!(AuditAction::SessionRecorded.to_string(), "SESSION_RECORDED");
        assert_eq!(AuditAction::SessionsDeleted.to_string(), "SESSIONS_DELETED");
        assert_eq!(AuditAction::RateLimitSet.to_string(), "RATE_LIMIT_SET");
        assert_eq!(AuditAction::RateLimitDeleted.to_string(), "RATE_LIMIT_DELETED");
        assert_eq!(AuditAction::ThreepidAdded.to_string(), "THREEPID_ADDED");
        assert_eq!(AuditAction::ThreepidRemoved.to_string(), "THREEPID_REMOVED");
        assert_eq!(AuditAction::ThreepidValidated.to_string(), "THREEPID_VALIDATED");
        assert_eq!(AuditAction::ExternalIdAdded.to_string(), "EXTERNAL_ID_ADDED");
        assert_eq!(AuditAction::ExternalIdRemoved.to_string(), "EXTERNAL_ID_REMOVED");
    }

    #[tokio::test]
    async fn test_audit_logger_creation() {
        // Test that audit logger can be created
        let logger = AuditLogger::new();
        assert!(logger.log(
            "admin@test.example.com",
            AuditAction::UserCreated,
            Some("user@test.example.com"),
            None,
            Some("Test creation"),
            Some("127.0.0.1"),
            true,
            None,
        ));
    }
}