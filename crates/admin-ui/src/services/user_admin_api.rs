//! User administration API implementation

use crate::models::{
    User, CreateUserRequest, CreateUserResponse, UpdateUserRequest, UpdateUserResponse,
    ResetPasswordRequest, ResetPasswordResponse, DeactivateUserRequest, DeactivateUserResponse,
    BatchUserOperationRequest, BatchUserOperationResponse, ListUsersRequest, ListUsersResponse,
    UserStatistics, BatchUserOperation, UserSortField, SortOrder, Permission,
    WebConfigError, AuditAction, AuditTargetType,
};
use crate::utils::audit_logger::AuditLogger;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use rand::{thread_rng, Rng};
use rand::distributions::Alphanumeric;

/// User administration API service
#[derive(Clone)]
pub struct UserAdminAPI {
    audit_logger: AuditLogger,
    // In a real implementation, this would connect to the Matrix server's database
    // For now, we'll use in-memory storage for demonstration
    users: std::sync::Arc<std::sync::RwLock<HashMap<String, User>>>,
}

impl UserAdminAPI {
    /// Create a new UserAdminAPI instance
    pub fn new(audit_logger: AuditLogger) -> Self {
        let users = std::sync::Arc::new(std::sync::RwLock::new(HashMap::new()));
        
        // Add some sample users for demonstration
        let mut users_map = users.write().unwrap();
        
        // Admin user
        users_map.insert(
            "@admin:example.com".to_string(),
            User {
                user_id: "@admin:example.com".to_string(),
                username: "admin".to_string(),
                display_name: Some("Administrator".to_string()),
                avatar_url: None,
                is_admin: true,
                is_deactivated: false,
                creation_ts: 1640995200, // 2022-01-01
                last_seen_ts: Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()),
                permissions: vec![Permission::SystemAdmin],
            }
        );
        
        // Regular user
        users_map.insert(
            "@user1:example.com".to_string(),
            User {
                user_id: "@user1:example.com".to_string(),
                username: "user1".to_string(),
                display_name: Some("User One".to_string()),
                avatar_url: None,
                is_admin: false,
                is_deactivated: false,
                creation_ts: 1641081600, // 2022-01-02
                last_seen_ts: Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 3600),
                permissions: vec![],
            }
        );
        
        drop(users_map);
        
        Self {
            audit_logger,
            users,
        }
    }

    /// List users with filtering and pagination
    pub async fn list_users(&self, request: ListUsersRequest, admin_user: &str) -> Result<ListUsersResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        let users = self.users.read().map_err(|_| WebConfigError::internal("Failed to read users"))?;
        
        let mut filtered_users: Vec<User> = users.values().cloned().collect();
        
        // Apply filters
        if let Some(search) = &request.search {
            let search_lower = search.to_lowercase();
            filtered_users.retain(|user| {
                user.username.to_lowercase().contains(&search_lower) ||
                user.display_name.as_ref().map_or(false, |name: &String| name.to_lowercase().contains(&search_lower)) ||
                user.user_id.to_lowercase().contains(&search_lower)
            });
        }
        
        if let Some(filter_admin) = request.filter_admin {
            filtered_users.retain(|user| user.is_admin == filter_admin);
        }
        
        if let Some(filter_deactivated) = request.filter_deactivated {
            filtered_users.retain(|user| user.is_deactivated == filter_deactivated);
        }
        
        // Apply sorting
        if let Some(sort_by) = &request.sort_by {
            let ascending = matches!(request.sort_order, Some(SortOrder::Ascending) | None);
            
            filtered_users.sort_by(|a, b| {
                let cmp = match sort_by {
                    UserSortField::Username => a.username.cmp(&b.username),
                    UserSortField::DisplayName => {
                        let a_name = a.display_name.as_deref().unwrap_or(&a.username);
                        let b_name = b.display_name.as_deref().unwrap_or(&b.username);
                        a_name.cmp(b_name)
                    },
                    UserSortField::CreationTime => a.creation_ts.cmp(&b.creation_ts),
                    UserSortField::LastSeen => {
                        a.last_seen_ts.unwrap_or(0).cmp(&b.last_seen_ts.unwrap_or(0))
                    },
                    UserSortField::IsAdmin => a.is_admin.cmp(&b.is_admin),
                };
                
                if ascending { cmp } else { cmp.reverse() }
            });
        }
        
        let total_count = filtered_users.len() as u32;
        
        // Apply pagination
        let offset = request.offset.unwrap_or(0) as usize;
        let limit = request.limit.unwrap_or(50) as usize;
        
        let paginated_users: Vec<User> = filtered_users
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect();
        
        let has_more = (offset + paginated_users.len()) < total_count as usize;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate, // Using existing action since UserList doesn't exist
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
                "result_count": paginated_users.len()
            })),
            "Listed users with filters",
        ).await;
        
        Ok(ListUsersResponse {
            success: true,
            users: paginated_users,
            total_count,
            has_more,
            error: None,
        })
    }

    /// Get user statistics
    pub async fn get_user_statistics(&self, admin_user: &str) -> Result<UserStatistics, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        let users = self.users.read().map_err(|_| WebConfigError::internal("Failed to read users"))?;
        
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let today_start = now - (now % 86400); // Start of today
        let week_start = today_start - (6 * 86400); // Start of this week (7 days ago)
        let month_start = today_start - (29 * 86400); // Start of this month (30 days ago)
        
        let mut stats = UserStatistics {
            total_users: 0,
            active_users: 0,
            admin_users: 0,
            deactivated_users: 0,
            users_created_today: 0,
            users_created_this_week: 0,
            users_created_this_month: 0,
        };
        
        for user in users.values() {
            stats.total_users += 1;
            
            if user.is_active() {
                stats.active_users += 1;
            }
            
            if user.is_admin {
                stats.admin_users += 1;
            }
            
            if user.is_deactivated {
                stats.deactivated_users += 1;
            }
            
            if user.creation_ts >= today_start {
                stats.users_created_today += 1;
            }
            
            if user.creation_ts >= week_start {
                stats.users_created_this_week += 1;
            }
            
            if user.creation_ts >= month_start {
                stats.users_created_this_month += 1;
            }
        }
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate, // Using existing action since UserStatistics doesn't exist
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
        
        // Generate user ID (in real implementation, this would follow Matrix user ID format)
        let user_id = format!("@{}:example.com", request.username);
        
        // Check if user already exists
        let users = self.users.read().map_err(|_| WebConfigError::internal("Failed to read users"))?;
        if users.contains_key(&user_id) {
            return Ok(CreateUserResponse {
                success: false,
                user: None,
                generated_password: None,
                error: Some("User already exists".to_string()),
            });
        }
        drop(users);
        
        // Generate password if not provided
        let generated_password = if request.password.is_none() {
            Some(self.generate_password())
        } else {
            None
        };
        
        // Create user
        let user = User {
            user_id: user_id.clone(),
            username: request.username.clone(),
            display_name: request.display_name.clone(),
            avatar_url: None,
            is_admin: request.is_admin,
            is_deactivated: false,
            creation_ts: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            last_seen_ts: None,
            permissions: request.permissions.clone(),
        };
        
        // Store user
        let mut users = self.users.write().map_err(|_| WebConfigError::internal("Failed to write users"))?;
        users.insert(user_id.clone(), user.clone());
        drop(users);
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserCreate,
            AuditTargetType::User,
            &user_id,
            Some(serde_json::json!({
                "username": request.username,
                "is_admin": request.is_admin,
                "permissions": request.permissions,
                "password_generated": generated_password.is_some()
            })),
            &format!("Created user {}", user_id),
        ).await;
        
        Ok(CreateUserResponse {
            success: true,
            user: Some(user),
            generated_password,
            error: None,
        })
    }

    /// Update user information
    pub async fn update_user(&self, user_id: &str, request: UpdateUserRequest, admin_user: &str) -> Result<UpdateUserResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        let mut users = self.users.write().map_err(|_| WebConfigError::internal("Failed to write users"))?;
        
        let user = users.get_mut(user_id).ok_or_else(|| {
            WebConfigError::validation(format!("User {} not found", user_id))
        })?;
        
        // Store original values for audit log
        let original_values = serde_json::json!({
            "display_name": user.display_name,
            "avatar_url": user.avatar_url,
            "is_admin": user.is_admin,
            "permissions": user.permissions
        });
        
        // Update fields
        if let Some(display_name) = request.display_name {
            user.display_name = Some(display_name);
        }
        
        if let Some(avatar_url) = request.avatar_url {
            user.avatar_url = Some(avatar_url);
        }
        
        if let Some(is_admin) = request.is_admin {
            user.is_admin = is_admin;
        }
        
        if let Some(permissions) = request.permissions {
            user.permissions = permissions;
        }
        
        let updated_user = user.clone();
        drop(users);
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate,
            AuditTargetType::User,
            user_id,
            Some(serde_json::json!({
                "original": original_values,
                "updated": {
                    "display_name": updated_user.display_name,
                    "avatar_url": updated_user.avatar_url,
                    "is_admin": updated_user.is_admin,
                    "permissions": updated_user.permissions
                }
            })),
            &format!("Updated user {}", user_id),
        ).await;
        
        Ok(UpdateUserResponse {
            success: true,
            user: Some(updated_user),
            error: None,
        })
    }

    /// Reset user password
    pub async fn reset_password(&self, request: ResetPasswordRequest, admin_user: &str) -> Result<ResetPasswordResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        let users = self.users.read().map_err(|_| WebConfigError::internal("Failed to read users"))?;
        
        if !users.contains_key(&request.user_id) {
            return Ok(ResetPasswordResponse {
                success: false,
                generated_password: None,
                error: Some("User not found".to_string()),
            });
        }
        drop(users);
        
        // Generate password if not provided
        let generated_password = if request.new_password.is_none() {
            Some(self.generate_password())
        } else {
            None
        };
        
        // In a real implementation, this would update the password in the database
        // and optionally logout all devices
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate, // Using existing action since UserPasswordReset doesn't exist
            AuditTargetType::User,
            &request.user_id,
            Some(serde_json::json!({
                "logout_devices": request.logout_devices,
                "password_generated": generated_password.is_some()
            })),
            &format!("Reset password for user {}", request.user_id),
        ).await;
        
        Ok(ResetPasswordResponse {
            success: true,
            generated_password,
            error: None,
        })
    }

    /// Deactivate a user
    pub async fn deactivate_user(&self, request: DeactivateUserRequest, admin_user: &str) -> Result<DeactivateUserResponse, WebConfigError> {
        // Check permissions
        if !self.has_user_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for user management"));
        }

        let mut users = self.users.write().map_err(|_| WebConfigError::internal("Failed to write users"))?;
        
        let user = users.get_mut(&request.user_id).ok_or_else(|| {
            WebConfigError::validation(format!("User {} not found", request.user_id))
        })?;
        
        if user.is_deactivated {
            return Ok(DeactivateUserResponse {
                success: false,
                error: Some("User is already deactivated".to_string()),
            });
        }
        
        user.is_deactivated = true;
        user.last_seen_ts = None; // Clear last seen timestamp
        
        drop(users);
        
        // In a real implementation, this would:
        // - Leave all rooms if requested
        // - Erase user data if requested
        // - Invalidate all access tokens
        
        // Log the action
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
        
        Ok(DeactivateUserResponse {
            success: true,
            error: None,
        })
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
            AuditAction::UserUpdate, // Using existing action since UserBatchOperation doesn't exist
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
    use crate::utils::audit_logger::AuditLogger;

    fn create_test_api() -> UserAdminAPI {
        let audit_logger = AuditLogger::new(1000);
        UserAdminAPI::new(audit_logger)
    }

    #[tokio::test]
    async fn test_list_users() {
        let api = create_test_api();
        let request = ListUsersRequest::default();
        
        let response = api.list_users(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert_eq!(response.users.len(), 2); // admin and user1
        assert_eq!(response.total_count, 2);
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
        
        let response = api.create_user(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.user.is_some());
        assert!(response.generated_password.is_some());
        
        let user = response.user.unwrap();
        assert_eq!(user.username, "newuser");
        assert_eq!(user.display_name, Some("New User".to_string()));
        assert!(!user.is_admin);
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
        
        let response = api.update_user("@user1:example.com", request, "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.user.is_some());
        
        let user = response.user.unwrap();
        assert_eq!(user.display_name, Some("Updated Name".to_string()));
        assert!(user.is_admin);
        assert!(user.permissions.contains(&Permission::UserManagement));
    }

    #[tokio::test]
    async fn test_deactivate_user() {
        let api = create_test_api();
        let request = DeactivateUserRequest {
            user_id: "@user1:example.com".to_string(),
            erase_data: false,
            leave_rooms: true,
        };
        
        let response = api.deactivate_user(request, "admin").await.unwrap();
        
        assert!(response.success);
        
        // Verify user is deactivated
        let users = api.users.read().unwrap();
        let user = users.get("@user1:example.com").unwrap();
        assert!(user.is_deactivated);
    }

    #[tokio::test]
    async fn test_user_statistics() {
        let api = create_test_api();
        
        let stats = api.get_user_statistics("admin").await.unwrap();
        
        assert_eq!(stats.total_users, 2);
        assert_eq!(stats.admin_users, 1);
        assert_eq!(stats.deactivated_users, 0);
    }

    #[tokio::test]
    async fn test_batch_operation() {
        let api = create_test_api();
        let request = BatchUserOperationRequest {
            user_ids: vec!["@user1:example.com".to_string()],
            operation: BatchUserOperation::SetAdmin { is_admin: true },
        };
        
        let response = api.batch_operation(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert_eq!(response.processed_count, 1);
        assert!(response.failed_users.is_empty());
        
        // Verify user is now admin
        let users = api.users.read().unwrap();
        let user = users.get("@user1:example.com").unwrap();
        assert!(user.is_admin);
    }
}