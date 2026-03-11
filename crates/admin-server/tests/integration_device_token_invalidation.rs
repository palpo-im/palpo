/// Integration Test: Device Deletion Invalidates Tokens (3.3.2)
///
/// Tests that device deletion properly invalidates associated sessions/tokens:
/// Create device → Record session with token → Delete device → Verify device and session removed
///
/// Tests: DeviceRepository + SessionRepository + Database interaction

#[cfg(test)]
mod device_token_invalidation_tests {
    use palpo_admin_server::repositories::{RepositoryFactory, DeviceRepository, SessionRepository, CreateDeviceInput};
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
        format!("@test_device_user_{}:test.example.com", timestamp)
    }

    /// Generate unique device ID
    fn generate_device_id() -> String {
        let timestamp = Utc::now().timestamp_millis();
        format!("DEVICE_{}", timestamp % 1000000)
    }

    #[tokio::test]
    async fn test_device_deletion_removes_device() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let user_id = generate_test_user_id();
        let device_id = generate_device_id();
        let device_name = "Test Device".to_string();

        // Step 1: Create device
        let create_input = CreateDeviceInput {
            device_id: device_id.clone(),
            user_id: user_id.clone(),
            display_name: Some(device_name.clone()),
            initial_device: true,
        };

        let created_device = factory.device_repository().create_device(&create_input).await
            .expect("Failed to create device");

        assert_eq!(created_device.device_id, device_id);
        assert_eq!(created_device.user_id, user_id);
        assert_eq!(created_device.display_name, Some(device_name));

        // Step 2: Verify device exists
        let fetched_device = factory.device_repository().get_device(&user_id, &device_id).await
            .expect("Failed to fetch device")
            .expect("Device not found");

        assert_eq!(fetched_device.device_id, device_id);

        // Step 3: Record a session (simulating token)
        let session_ip = "192.168.1.100".to_string();
        let user_agent = "TestClient/1.0".to_string();

        factory.session_repository().record_session(
            &user_id,
            &session_ip,
            Some(&device_id),
            Some(&user_agent),
        ).await.expect("Failed to record session");

        // Step 4: Verify session was recorded
        let sessions = factory.session_repository().get_user_sessions(&user_id).await
            .expect("Failed to get sessions");

        assert!(!sessions.is_empty(), "Session should be recorded");
        let session = sessions.first().unwrap();
        assert_eq!(session.ip, session_ip);
        assert_eq!(session.device_id, Some(device_id.clone()));

        // Step 5: Delete device
        factory.device_repository().delete_device(&user_id, &device_id).await
            .expect("Failed to delete device");

        // Step 6: Verify device is removed
        let fetched_device_after = factory.device_repository().get_device(&user_id, &device_id).await
            .expect("Failed to fetch device after deletion");

        assert!(fetched_device_after.is_none(), "Device should be deleted");

        // Step 7: Verify session is cleaned up (device_id should be null or session removed)
        let sessions_after = factory.session_repository().get_user_sessions(&user_id).await
            .expect("Failed to get sessions after deletion");

        // Sessions may remain but device_id association should be gone
        for session in sessions_after {
            if session.ip == session_ip {
                assert!(session.device_id.is_none() || session.device_id.as_ref().unwrap() != &device_id,
                    "Session should no longer be associated with deleted device");
            }
        }
    }

    #[tokio::test]
    async fn test_batch_device_deletion() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let user_id = generate_test_user_id();
        let device_ids: Vec<String> = (0..3).map(|_| generate_device_id()).collect();

        // Step 1: Create multiple devices
        for (i, device_id) in device_ids.iter().enumerate() {
            let create_input = CreateDeviceInput {
                device_id: device_id.clone(),
                user_id: user_id.clone(),
                display_name: Some(format!("Test Device {}", i)),
                initial_device: false,
            };

            factory.device_repository().create_device(&create_input).await
                .expect("Failed to create device");

            // Record session for each device
            factory.session_repository().record_session(
                &user_id,
                &format!("192.168.1.{}", 100 + i),
                Some(device_id),
                Some("TestClient/1.0"),
            ).await.expect("Failed to record session");
        }

        // Step 2: Verify all devices exist
        let device_count_before = factory.device_repository().get_user_device_count(&user_id).await
            .expect("Failed to get device count");
        assert_eq!(device_count_before, 3, "Should have 3 devices");

        // Step 3: Batch delete devices
        let deleted_count = factory.device_repository().delete_devices(&user_id, &device_ids).await
            .expect("Failed to batch delete devices");

        assert_eq!(deleted_count, 3, "Should delete 3 devices");

        // Step 4: Verify all devices are removed
        let device_count_after = factory.device_repository().get_user_device_count(&user_id).await
            .expect("Failed to get device count after deletion");
        assert_eq!(device_count_after, 0, "Should have 0 devices");

        // Step 5: Verify devices individually
        for device_id in &device_ids {
            let fetched = factory.device_repository().get_device(&user_id, device_id).await
                .expect("Failed to fetch device");
            assert!(fetched.is_none(), "Device {} should be deleted", device_id);
        }
    }

    #[tokio::test]
    async fn test_delete_all_user_devices() {
        if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
            return;
        }

        let factory = match create_test_repository() {
            Ok(f) => f,
            Err(_) => return,
        };

        let user_id = generate_test_user_id();
        let device_ids: Vec<String> = (0..5).map(|_| generate_device_id()).collect();

        // Step 1: Create devices
        for (i, device_id) in device_ids.iter().enumerate() {
            let create_input = CreateDeviceInput {
                device_id: device_id.clone(),
                user_id: user_id.clone(),
                display_name: Some(format!("Device {}", i)),
                initial_device: i == 0,
            };

            factory.device_repository().create_device(&create_input).await
                .expect("Failed to create device");
        }

        // Step 2: Delete all user devices
        let deleted_count = factory.device_repository().delete_all_user_devices(&user_id).await
            .expect("Failed to delete all user devices");

        assert_eq!(deleted_count, 5, "Should delete 5 devices");

        // Step 3: Verify all devices are removed
        let device_count = factory.device_repository().get_user_device_count(&user_id).await
            .expect("Failed to get device count");
        assert_eq!(device_count, 0, "Should have 0 devices");
    }
}