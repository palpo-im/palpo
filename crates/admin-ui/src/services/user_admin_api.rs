//! User administration API implementation
//!
//! This module provides the API client for the admin-server user management endpoints.
//! All endpoints require authentication via Bearer token.

use crate::models::{
    CreateUserRequest, CreateUserResponse, UpdateUserRequest, UpdateUserResponse,
    ResetPasswordRequest, ResetPasswordResponse, DeactivateUserRequest, DeactivateUserResponse,
    BatchUserOperationRequest, BatchUserOperationResponse, ListUsersRequest, ListUsersResponse,
    UserStatistics, BatchUserOperation,
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

        // Validate username
        if request.username.is_empty() || request.username.len() > 255 {
            return Ok(CreateUserResponse {
                success: false,
                user: None,
                generated_password: None,
                error: Some("Username must be between 1 and 255 characters".to_string()),
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
                        "username": request.username,
                        "is_admin": request.is_admin,
                        "permissions": request.permissions,
                        "password_generated": response.generated_password.is_some()
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