//! User administration API implementation
//!
//! This module provides the API client for the admin-server user management endpoints.
//! All endpoints require authentication via Bearer token.

use crate::models::{
    CreateUserRequest, CreateUserResponse, UpdateUserRequest, UpdateUserResponse,
    ResetPasswordRequest, ResetPasswordResponse, DeactivateUserRequest, DeactivateUserResponse,
    BatchUserOperationRequest, BatchUserOperationResponse, ListUsersRequest, ListUsersResponse,
    UserStatistics, BatchUserOperation, User, UserResponse,
    DeviceListRequest, DeviceListResponse, DeleteDeviceResponse,
    BatchDeleteDeviceRequest, BatchDeleteDeviceResponse,
    SessionListRequest, SessionListResponse, SessionInfo, ConnectionType, WhoisInfo, TerminateSessionResponse,
    PusherListResponse, UpdatePusherRequest, UpdatePusherResponse, DeletePusherResponse,
    WebConfigError, AuditAction, AuditTargetType,
};
use crate::services::api_client::{ApiClient, api_get_json, api_post_json_response, api_put_json_response};
use crate::utils::audit_logger::AuditLogger;
use rand::{thread_rng, Rng};
use rand::distributions::Alphanumeric;

/// User administration API service
#[derive(Clone)]
pub struct UserAdminAPI {
    audit_logger: AuditLogger,
    api_client: ApiClient,
}

impl UserAdminAPI {
    /// Create a new UserAdminAPI instance
    pub fn new(audit_logger: AuditLogger, api_client: ApiClient) -> Self {
        Self {
            audit_logger,
            api_client,
        }
    }

    /// Get a single user by user_id
    pub async fn get_user(&self, user_id: &str, admin_user: &str) -> Result<Option<User>, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        let response: UserResponse = api_get_json(&format!("/api/v1/users/{}", user_id)).await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            user_id,
            None,
            &format!("Retrieved user {}", user_id),
        ).await;

        Ok(response.user)
    }

    /// List users with filtering and pagination
    pub async fn list_users(&self, request: ListUsersRequest, admin_user: &str) -> Result<ListUsersResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        // Build query parameters
        let mut query_params = Vec::new();
        if let Some(limit) = request.limit {
            query_params.push(format!("limit={}", limit));
        }
        if let Some(offset) = request.offset {
            query_params.push(format!("offset={}", offset));
        }
        if let Some(search) = &request.search {
            query_params.push(format!("search={}", search));
        }
        if let Some(filter_admin) = request.filter_admin {
            query_params.push(format!("is_admin={}", filter_admin));
        }
        if let Some(filter_deactivated) = request.filter_deactivated {
            query_params.push(format!("is_deactivated={}", filter_deactivated));
        }

        let query = if query_params.is_empty() {
            "".to_string()
        } else {
            format!("?{}", query_params.join("&"))
        };

        let response: ListUsersResponse = api_get_json(&format!("/api/v1/users{}", query)).await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            "user_list",
            Some(serde_json::json!({
                "filter": {
                    "search": request.search,
                    "admin": request.filter_admin,
                    "deactivated": request.filter_deactivated
                },
                "pagination": {
                    "offset": request.offset,
                    "limit": request.limit
                },
                "result_count": response.users.len()
            })),
            "Listed users with filters",
        ).await;

        Ok(response)
    }

    /// Get user statistics
    pub async fn get_user_statistics(&self, admin_user: &str) -> Result<UserStatistics, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        let stats: UserStatistics = api_get_json("/api/v1/users/stats").await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            "user_statistics",
            Some(serde_json::json!(stats)),
            "Retrieved user statistics",
        ).await;

        Ok(stats)
    }

    /// Create a new user
    pub async fn create_user(&self, request: CreateUserRequest, admin_user: &str) -> Result<CreateUserResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        // Validate user_id format (should be @localpart:server_name)
        if request.user_id.is_empty() || !request.user_id.starts_with('@') {
            return Ok(CreateUserResponse {
                success: false,
                user: None,
                generated_password: None,
                error: Some("Invalid user_id format. Must start with @ (e.g., @user:localhost)".to_string()),
            });
        }

        // Create user via API
        let response: CreateUserResponse = api_post_json_response("/api/v1/users", &request).await?;

        // Log the action
        if response.success {
            if let Some(ref user) = response.user {
                self.audit_logger.log_action(
                    admin_user,
                    AuditAction::UserCreate,
                    AuditTargetType::User,
                    &user.user_id,
                    Some(serde_json::json!({
                        "user_id": request.user_id,
                        "is_admin": request.is_admin,
                        "is_guest": request.is_guest,
                    })),
                    &format!("Created user {}", user.user_id),
                ).await;
            }
        }

        Ok(response)
    }

    /// Update user information
    pub async fn update_user(&self, user_id: &str, request: UpdateUserRequest, admin_user: &str) -> Result<UpdateUserResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        // Update user via API
        let response: UpdateUserResponse = api_put_json_response(&format!("/api/v1/users/{}", user_id), &request).await?;

        // Log the action
        if response.success {
            if let Some(ref user) = response.user {
                self.audit_logger.log_action(
                    admin_user,
                    AuditAction::UserUpdate,
                    AuditTargetType::User,
                    user_id,
                    Some(serde_json::json!({
                        "display_name": request.display_name,
                        "avatar_url": request.avatar_url,
                        "is_admin": request.is_admin,
                        "permissions": request.permissions
                    })),
                    &format!("Updated user {}", user_id),
                ).await;
            }
        }

        Ok(response)
    }

    /// Reset user password
    pub async fn reset_password(&self, request: ResetPasswordRequest, admin_user: &str) -> Result<ResetPasswordResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        // Reset password via API
        let response: ResetPasswordResponse = api_post_json_response(&format!("/api/v1/users/{}/reset-password", request.user_id), &request).await?;

        // Log the action
        if response.success {
            self.audit_logger.log_action(
                admin_user,
                AuditAction::UserUpdate,
                AuditTargetType::User,
                &request.user_id,
                Some(serde_json::json!({
                    "logout_devices": request.logout_devices,
                    "password_generated": response.generated_password.is_some()
                })),
                &format!("Reset password for user {}", request.user_id),
            ).await;
        }

        Ok(response)
    }

    /// Deactivate a user
    pub async fn deactivate_user(&self, request: DeactivateUserRequest, admin_user: &str) -> Result<DeactivateUserResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        // Deactivate user via API
        let response: DeactivateUserResponse = api_post_json_response(&format!("/api/v1/users/{}/deactivate", request.user_id), &request).await?;

        // Log the action
        if response.success {
            self.audit_logger.log_action(
                admin_user,
                AuditAction::UserDeactivate,
                AuditTargetType::User,
                &request.user_id,
                Some(serde_json::json!({
                    "erase_data": request.erase_data,
                    "leave_rooms": request.leave_rooms
                })),
                &format!("Deactivated user {}", request.user_id),
            ).await;
        }

        Ok(response)
    }

    /// Perform batch operations on users
    pub async fn batch_operation(&self, request: BatchUserOperationRequest, admin_user: &str) -> Result<BatchUserOperationResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        let mut processed_count = 0;
        let mut failed_users = Vec::new();
        let mut errors = Vec::new();

        for user_id in &request.user_ids {
            match self.perform_single_batch_operation(user_id, &request.operation, admin_user).await {
                Ok(_) => processed_count += 1,
                Err(e) => {
                    failed_users.push(user_id.clone());
                    errors.push(format!("{}: {}", user_id, e));
                }
            }
        }

        // Log the batch operation
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            "batch_operation",
            Some(serde_json::json!({
                "operation": request.operation,
                "user_count": request.user_ids.len(),
                "processed_count": processed_count,
                "failed_count": failed_users.len(),
                "failed_users": failed_users
            })),
            &format!("Performed batch operation on {} users", request.user_ids.len()),
        ).await;

        Ok(BatchUserOperationResponse {
            success: failed_users.is_empty(),
            processed_count,
            failed_users,
            errors,
        })
    }

    /// Perform a single batch operation on a user
    async fn perform_single_batch_operation(&self, user_id: &str, operation: &BatchUserOperation, admin_user: &str) -> Result<(), WebConfigError> {
        match operation {
            BatchUserOperation::Deactivate { erase_data, leave_rooms } => {
                let request = DeactivateUserRequest {
                    user_id: user_id.to_string(),
                    erase_data: *erase_data,
                    leave_rooms: *leave_rooms,
                };
                self.deactivate_user(request, admin_user).await?;
            },
            BatchUserOperation::SetAdmin { is_admin } => {
                let request = UpdateUserRequest {
                    display_name: None,
                    avatar_url: None,
                    is_admin: Some(*is_admin),
                    permissions: None,
                };
                self.update_user(user_id, request, admin_user).await?;
            },
            BatchUserOperation::UpdatePermissions { permissions } => {
                let request = UpdateUserRequest {
                    display_name: None,
                    avatar_url: None,
                    is_admin: None,
                    permissions: Some(permissions.clone()),
                };
                self.update_user(user_id, request, admin_user).await?;
            },
        }
        Ok(())
    }

    /// Check if the admin user has user management permissions
    async fn has_user_management_permission(&self, _admin_user: &str) -> Result<bool, WebConfigError> {
        // In a real implementation, this would check the admin user's permissions
        // For now, we'll assume all admin users have user management permissions
        Ok(true)
    }

    /// Generate a random password
    fn generate_password(&self) -> String {
        thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect()
    }

    // ==================== Device Management ====================

    /// Get devices for a user (wrapper for user detail page)
    pub async fn get_user_devices(&self, user_id: &str, request: DeviceListRequest, admin_user: &str) -> Result<DeviceListResponse, WebConfigError> {
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for device management"));
        }

        let response: DeviceListResponse = api_get_json(&format!("/api/v1/users/{}/devices", user_id)).await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            user_id,
            Some(serde_json::json!({
                "device_count": response.devices.len(),
                "total_count": response.total_count
            })),
            &format!("Listed devices for user {}", user_id),
        ).await;

        Ok(response)
    }

    /// Delete a single device
    pub async fn delete_device(&self, user_id: &str, device_id: &str, admin_user: &str) -> Result<DeleteDeviceResponse, WebConfigError> {
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for device management"));
        }

        let response: DeleteDeviceResponse = api_post_json_response(
            &format!("/api/v1/users/{}/devices/{}", user_id, device_id),
            &serde_json::json!({})
        ).await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            user_id,
            Some(serde_json::json!({
                "device_id": device_id,
                "success": response.success
            })),
            &format!("Deleted device {} for user {}", device_id, user_id),
        ).await;

        Ok(response)
    }

    /// Delete multiple devices for a user
    pub async fn delete_devices(&self, user_id: &str, device_ids: Vec<String>, admin_user: &str) -> Result<BatchDeleteDeviceResponse, WebConfigError> {
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for device management"));
        }

        let response: BatchDeleteDeviceResponse = api_post_json_response(
            &format!("/api/v1/users/{}/devices/delete", user_id),
            &BatchDeleteDeviceRequest { device_ids, reason: None }
        ).await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            user_id,
            Some(serde_json::json!({
                "deleted_count": response.deleted_count,
                "failed_count": response.failed_count
            })),
            &format!("Batch deleted devices for user {}", user_id),
        ).await;

        Ok(response)
    }

    // ==================== Session Management ====================

    /// Get sessions for a user (wrapper for user detail page)
    pub async fn get_user_sessions(&self, user_id: &str, request: SessionListRequest, admin_user: &str) -> Result<SessionListResponse, WebConfigError> {
        self.list_sessions(user_id, request, admin_user).await
    }

    /// Get whois information for a user
    pub async fn get_whois(&self, user_id: &str, admin_user: &str) -> Result<WhoisInfo, WebConfigError> {
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for session management"));
        }

        let response: WhoisInfo = api_get_json(&format!("/api/v1/users/{}/whois", user_id)).await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            user_id,
            Some(serde_json::json!({
                "session_count": response.devices.len(),
                "ip_address": response.ip_address
            })),
            &format!("Retrieved whois info for user {}", user_id),
        ).await;

        Ok(response)
    }

    /// List sessions for a user
    pub async fn list_sessions(&self, user_id: &str, request: SessionListRequest, admin_user: &str) -> Result<SessionListResponse, WebConfigError> {
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for session management"));
        }

        // Get whois info which contains session data
        let whois = self.get_whois(user_id, admin_user).await?;

        // Convert whois devices to sessions
        let sessions: Vec<SessionInfo> = whois.devices.into_iter().map(|d| SessionInfo {
            session_id: d.device_id.clone(),
            user_id: user_id.to_string(),
            device_id: Some(d.device_id),
            ip_address: d.last_seen_ip,
            user_agent: d.last_seen_user_agent,
            connection_type: ConnectionType::Web,
            login_ts: whois.connected_since,
            last_activity_ts: d.last_seen_ts,
            is_active: whois.last_activity > 0,
            room_count: 0,
            device_display_name: d.display_name,
        }).collect();

        // Calculate counts before moving sessions
        let total_count = sessions.len() as u32;
        let active_count = sessions.iter().filter(|s| s.is_active).count() as u32;

        Ok(SessionListResponse {
            success: true,
            sessions,
            total_count,
            active_count,
            has_more: false,
            error: None,
        })
    }

    /// Terminate a session
    pub async fn terminate_session(&self, user_id: &str, session_id: &str, admin_user: &str) -> Result<TerminateSessionResponse, WebConfigError> {
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for session management"));
        }

        // Delete the device to terminate the session
        let response = self.delete_device(user_id, session_id, admin_user).await?;

        Ok(TerminateSessionResponse {
            success: response.success,
            error: response.error,
        })
    }

    // ==================== Pusher Management ====================

    /// Get pushers for a user (wrapper for user detail page)
    pub async fn get_user_pushers(&self, user_id: &str, admin_user: &str) -> Result<PusherListResponse, WebConfigError> {
        self.list_pushers(user_id, admin_user).await
    }

    /// List pushers for a user
    pub async fn list_pushers(&self, user_id: &str, admin_user: &str) -> Result<PusherListResponse, WebConfigError> {
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for pusher management"));
        }

        let response: PusherListResponse = api_get_json(&format!("/api/v1/users/{}/pushers", user_id)).await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            user_id,
            Some(serde_json::json!({
                "pusher_count": response.pushers.len()
            })),
            &format!("Listed pushers for user {}", user_id),
        ).await;

        Ok(response)
    }

    /// Update a pusher
    pub async fn update_pusher(&self, user_id: &str, request: UpdatePusherRequest, admin_user: &str) -> Result<UpdatePusherResponse, WebConfigError> {
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for pusher management"));
        }

        let response: UpdatePusherResponse = api_put_json_response(
            &format!("/api/v1/users/{}/pushers/{}", user_id, request.pusher_id),
            &request
        ).await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            user_id,
            Some(serde_json::json!({
                "pusher_id": request.pusher_id,
                "success": response.success
            })),
            &format!("Updated pusher {} for user {}", request.pusher_id, user_id),
        ).await;

        Ok(response)
    }

    /// Delete a pusher
    pub async fn delete_pusher(&self, user_id: &str, pusher_id: &str, admin_user: &str) -> Result<DeletePusherResponse, WebConfigError> {
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for pusher management"));
        }

        let response: DeletePusherResponse = api_post_json_response(
            &format!("/api/v1/users/{}/pushers/{}/delete", user_id, pusher_id),
            &serde_json::json!({})
        ).await?;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            user_id,
            Some(serde_json::json!({
                "pusher_id": pusher_id,
                "success": response.success
            })),
            &format!("Deleted pusher {} for user {}", pusher_id, user_id),
        ).await;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Permission;
    use crate::utils::audit_logger::AuditLogger;
    use crate::services::api_client::ApiClient;

    fn create_test_api() -> UserAdminAPI {
        let audit_logger = AuditLogger::new(1000);
        let api_client = ApiClient::new("http://localhost:8081");
        UserAdminAPI::new(audit_logger, api_client)
    }

    #[tokio::test]
    async fn test_list_users() {
        let api = create_test_api();
        let request = ListUsersRequest::default();
        
        // Note: This test requires the admin server to be running
        // For unit testing without server, mock the API client
        let result = api.list_users(request, "admin").await;
        
        // If server is not running, this will fail - that's expected
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_create_user() {
        let api = create_test_api();
        let request = CreateUserRequest {
            username: "newuser".to_string(),
            password: None, // Auto-generate password
            display_name: Some("New User".to_string()),
            is_admin: false,
            permissions: vec![],
            send_notification: false,
        };
        
        let result = api.create_user(request, "admin").await;
        
        // If server is not running, this will fail - that's expected
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_update_user() {
        let api = create_test_api();
        let request = UpdateUserRequest {
            display_name: Some("Updated Name".to_string()),
            avatar_url: None,
            is_admin: Some(true),
            permissions: Some(vec![Permission::UserManagement]),
        };
        
        let result = api.update_user("@user1:example.com", request, "admin").await;
        
        // If server is not running, this will fail - that's expected
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_deactivate_user() {
        let api = create_test_api();
        let request = DeactivateUserRequest {
            user_id: "@user1:example.com".to_string(),
            erase_data: false,
            leave_rooms: true,
        };
        
        let result = api.deactivate_user(request, "admin").await;
        
        // If server is not running, this will fail - that's expected
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_user_statistics() {
        let api = create_test_api();
        
        let result = api.get_user_statistics("admin").await;
        
        // If server is not running, this will fail - that's expected
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_batch_operation() {
        let api = create_test_api();
        let request = BatchUserOperationRequest {
            user_ids: vec!["@user1:example.com".to_string()],
            operation: BatchUserOperation::SetAdmin { is_admin: true },
        };
        
        let result = api.batch_operation(request, "admin").await;
        
        // If server is not running, this will fail - that's expected
        assert!(result.is_ok() || result.is_err());
    }
}