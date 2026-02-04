//! User management models

use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use crate::models::auth::Permission;

/// User information for management
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct User {
    pub user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub is_deactivated: bool,
    pub creation_ts: u64,
    pub last_seen_ts: Option<u64>,
    pub permissions: Vec<Permission>,
}

/// User creation request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: Option<String>, // None for auto-generated password
    pub display_name: Option<String>,
    pub is_admin: bool,
    pub permissions: Vec<Permission>,
    pub send_notification: bool,
}

/// User creation response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreateUserResponse {
    pub success: bool,
    pub user: Option<User>,
    pub generated_password: Option<String>,
    pub error: Option<String>,
}

/// User update request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: Option<bool>,
    pub permissions: Option<Vec<Permission>>,
}

/// User update response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateUserResponse {
    pub success: bool,
    pub user: Option<User>,
    pub error: Option<String>,
}

/// Password reset request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResetPasswordRequest {
    pub user_id: String,
    pub new_password: Option<String>, // None for auto-generated password
    pub logout_devices: bool,
}

/// Password reset response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResetPasswordResponse {
    pub success: bool,
    pub generated_password: Option<String>,
    pub error: Option<String>,
}

/// User deactivation request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeactivateUserRequest {
    pub user_id: String,
    pub erase_data: bool,
    pub leave_rooms: bool,
}

/// User deactivation response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeactivateUserResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Batch user operation request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BatchUserOperationRequest {
    pub user_ids: Vec<String>,
    pub operation: BatchUserOperation,
}

/// Batch user operation types
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum BatchUserOperation {
    Deactivate {
        erase_data: bool,
        leave_rooms: bool,
    },
    SetAdmin {
        is_admin: bool,
    },
    UpdatePermissions {
        permissions: Vec<Permission>,
    },
}

/// Batch user operation response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BatchUserOperationResponse {
    pub success: bool,
    pub processed_count: usize,
    pub failed_users: Vec<String>,
    pub errors: Vec<String>,
}

/// User list request with filtering and pagination
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ListUsersRequest {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub search: Option<String>,
    pub filter_admin: Option<bool>,
    pub filter_deactivated: Option<bool>,
    pub sort_by: Option<UserSortField>,
    pub sort_order: Option<SortOrder>,
}

/// User list response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ListUsersResponse {
    pub success: bool,
    pub users: Vec<User>,
    pub total_count: u32,
    pub has_more: bool,
    pub error: Option<String>,
}

/// User sort fields
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum UserSortField {
    Username,
    DisplayName,
    CreationTime,
    LastSeen,
    IsAdmin,
}

use crate::models::room::SortOrder;

/// User statistics
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserStatistics {
    pub total_users: u32,
    pub active_users: u32,
    pub admin_users: u32,
    pub deactivated_users: u32,
    pub users_created_today: u32,
    pub users_created_this_week: u32,
    pub users_created_this_month: u32,
}

impl User {
    /// Check if user has a specific permission
    pub fn has_permission(&self, permission: &Permission) -> bool {
        // System admin has all permissions
        if self.permissions.contains(&Permission::SystemAdmin) {
            return true;
        }
        
        self.permissions.contains(permission)
    }

    /// Check if user is currently active (not deactivated and seen recently)
    pub fn is_active(&self) -> bool {
        !self.is_deactivated && self.last_seen_ts.is_some()
    }

    /// Get user age in days since creation
    pub fn age_in_days(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        (now - self.creation_ts) / 86400 // 86400 seconds in a day
    }

    /// Get days since last seen
    pub fn days_since_last_seen(&self) -> Option<u64> {
        self.last_seen_ts.map(|last_seen| {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            
            (now - last_seen) / 86400
        })
    }
}

impl Default for ListUsersRequest {
    fn default() -> Self {
        Self {
            limit: Some(50),
            offset: Some(0),
            search: None,
            filter_admin: None,
            filter_deactivated: Some(false), // By default, don't show deactivated users
            sort_by: Some(UserSortField::Username),
            sort_order: Some(SortOrder::Ascending),
        }
    }
}

impl UserSortField {
    /// Get human-readable description of the sort field
    pub fn description(&self) -> &'static str {
        match self {
            UserSortField::Username => "Username",
            UserSortField::DisplayName => "Display Name",
            UserSortField::CreationTime => "Creation Time",
            UserSortField::LastSeen => "Last Seen",
            UserSortField::IsAdmin => "Admin Status",
        }
    }
}

