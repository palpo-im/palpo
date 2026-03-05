/// User Handler - HTTP handlers for user management API
///
/// This module implements all user management API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
///
/// Endpoints:
/// - POST /api/v1/users - Create a new user
/// - GET /api/v1/users - List users with filtering and pagination
/// - GET /api/v1/users/{user_id} - Get user details
/// - PUT /api/v1/users/{user_id} - Update user
/// - DELETE /api/v1/users/{user_id} - Deactivate user
/// - POST /api/v1/users/{user_id}/reactivate - Reactivate deactivated user
/// - GET /api/v1/users/{user_id}/details - Get extended user details
/// - GET /api/v1/users/username-available/{username} - Check username availability
/// - GET /api/v1/users/{user_id}/admin - Get admin status
/// - PUT /api/v1/users/{user_id}/admin - Set admin status
/// - GET /api/v1/users/{user_id}/shadow-ban - Get shadow ban status
/// - PUT /api/v1/users/{user_id}/shadow-ban - Set shadow ban status
/// - GET /api/v1/users/{user_id}/locked - Get locked status
/// - PUT /api/v1/users/{user_id}/locked - Set locked status
/// - GET /api/v1/users/stats - Get user statistics

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use crate::types::AdminError;
use crate::repositories::{UserRepository, User, CreateUserInput, UpdateUserInput, UserFilter, UserDetails};

use super::auth_middleware::{require_auth, get_authenticated_username};

// ===== Request Types =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub user_id: String,
    pub displayname: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub user_type: Option<String>,
    pub appservice_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserRequest {
    pub displayname: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: Option<bool>,
    pub user_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserListQuery {
    pub is_admin: Option<bool>,
    pub is_deactivated: Option<bool>,
    pub shadow_banned: Option<bool>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeactivateUserRequest {
    pub erase: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowBanRequest {
    pub shadow_banned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminStatusRequest {
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockStatusRequest {
    pub locked: bool,
}

// ===== Response Types =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserResponse {
    pub name: String,
    pub displayname: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub is_deactivated: bool,
    pub is_erased: bool,
    pub shadow_banned: bool,
    pub locked: bool,
    pub creation_ts: i64,
    pub last_seen_ts: Option<i64>,
    pub user_type: Option<String>,
    pub appservice_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserListResponse {
    pub users: Vec<UserResponse>,
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDetailsResponse {
    pub user: UserResponse,
    pub device_count: i64,
    pub session_count: i64,
    pub joined_room_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsernameAvailabilityResponse {
    pub username: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStatsResponse {
    pub total_users: i64,
    pub admin_count: i64,
    pub deactivated_count: i64,
    pub active_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminStatusResponse {
    pub user_id: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowBanStatusResponse {
    pub user_id: String,
    pub shadow_banned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockStatusResponse {
    pub user_id: String,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

// ===== User Handler =====

pub struct UserHandler<T: UserRepository> {
    user_repo: T,
}

impl<T: UserRepository> UserHandler<T> {
    pub fn new(user_repo: T) -> Self {
        Self { user_repo }
    }

    /// POST /api/v1/users - Create a new user
    #[handler]
    pub async fn create_user(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let body = match req.parse_json::<CreateUserRequest>().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("Invalid create user request: {}", e);
                res.status_code(StatusCode::BAD_REQUEST);
                res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
                return;
            }
        };

        let input = CreateUserInput {
            user_id: body.user_id,
            displayname: body.displayname,
            avatar_url: body.avatar_url,
            is_admin: body.is_admin,
            is_guest: body.is_guest,
            user_type: body.user_type,
            appservice_id: body.appservice_id,
        };

        match self.user_repo.create_user(&input).await {
            Ok(user) => {
                tracing::info!("Created user: {}", user.name);
                res.status_code(StatusCode::CREATED);
                res.render(Json(UserResponse::from(&user)));
            }
            Err(e) => {
                tracing::error!("Failed to create user: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to create user".to_string() }));
            }
        }
    }

    /// GET /api/v1/users - List users with filtering and pagination
    #[handler]
    pub async fn list_users(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let query = req.parse_query::<UserListQuery>().unwrap_or_default();

        let filter = UserFilter {
            is_admin: query.is_admin,
            is_deactivated: query.is_deactivated,
            shadow_banned: query.shadow_banned,
            search_term: query.search,
            limit: query.limit,
            offset: query.offset,
        };

        match self.user_repo.list_users(&filter).await {
            Ok(result) => {
                let users: Vec<UserResponse> = result.users.iter().map(UserResponse::from).collect();
                res.render(Json(UserListResponse {
                    users,
                    total_count: result.total_count,
                    limit: result.limit,
                    offset: result.offset,
                }));
            }
            Err(e) => {
                tracing::error!("Failed to list users: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to list users".to_string() }));
            }
        }
    }

    /// GET /api/v1/users/{user_id} - Get user by ID
    #[handler]
    pub async fn get_user(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();

        match self.user_repo.get_user(&user_id).await {
            Ok(Some(user)) => {
                res.render(Json(UserResponse::from(&user)));
            }
            Ok(None) => {
                res.status_code(StatusCode::NOT_FOUND);
                res.render(Json(ErrorResponse { error: format!("User not found: {}", user_id) }));
            }
            Err(e) => {
                tracing::error!("Failed to get user: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to get user".to_string() }));
            }
        }
    }

    /// GET /api/v1/users/{user_id}/details - Get extended user details
    #[handler]
    pub async fn get_user_details(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();

        match self.user_repo.get_user_details(&user_id).await {
            Ok(Some(details)) => {
                res.render(Json(UserDetailsResponse::from(&details)));
            }
            Ok(None) => {
                res.status_code(StatusCode::NOT_FOUND);
                res.render(Json(ErrorResponse { error: format!("User not found: {}", user_id) }));
            }
            Err(e) => {
                tracing::error!("Failed to get user details: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to get user details".to_string() }));
            }
        }
    }

    /// PUT /api/v1/users/{user_id} - Update user
    #[handler]
    pub async fn update_user(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();
        let body = match req.parse_json::<UpdateUserRequest>().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("Invalid update user request: {}", e);
                res.status_code(StatusCode::BAD_REQUEST);
                res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
                return;
            }
        };

        let input = UpdateUserInput {
            displayname: body.displayname,
            avatar_url: body.avatar_url,
            is_admin: body.is_admin,
            user_type: body.user_type,
        };

        match self.user_repo.update_user(&user_id, &input).await {
            Ok(user) => {
                tracing::info!("Updated user: {}", user.name);
                res.render(Json(UserResponse::from(&user)));
            }
            Err(e) => {
                tracing::error!("Failed to update user: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to update user".to_string() }));
            }
        }
    }

    /// DELETE /api/v1/users/{user_id} - Deactivate user
    #[handler]
    pub async fn deactivate_user(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();
        let body = req.parse_json::<DeactivateUserRequest>().await.unwrap_or(DeactivateUserRequest { erase: false });

        if let Err(e) = self.user_repo.deactivate_user(&user_id, body.erase).await {
            tracing::error!("Failed to deactivate user: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to deactivate user".to_string() }));
            return;
        }

        tracing::info!("Deactivated user: {} (erase: {})", user_id, body.erase);
        res.render(Json(SuccessResponse {
            success: true,
            message: format!("User {} deactivated successfully", user_id),
        }));
    }

    /// POST /api/v1/users/{user_id}/reactivate - Reactivate user
    #[handler]
    pub async fn reactivate_user(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();

        if let Err(e) = self.user_repo.reactivate_user(&user_id).await {
            tracing::error!("Failed to reactivate user: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to reactivate user".to_string() }));
            return;
        }

        tracing::info!("Reactivated user: {}", user_id);
        res.render(Json(SuccessResponse {
            success: true,
            message: format!("User {} reactivated successfully", user_id),
        }));
    }

    /// GET /api/v1/users/username-available/{username} - Check username availability
    #[handler]
    pub async fn check_username_available(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let username = req.param::<String>("username").unwrap_or_default();

        match self.user_repo.is_username_available(&username).await {
            Ok(available) => {
                res.render(Json(UsernameAvailabilityResponse { username, available }));
            }
            Err(e) => {
                tracing::error!("Failed to check username: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to check username".to_string() }));
            }
        }
    }

    /// GET /api/v1/users/{user_id}/admin - Get admin status
    #[handler]
    pub async fn get_admin_status(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();

        match self.user_repo.get_user(&user_id).await {
            Ok(Some(user)) => {
                res.render(Json(AdminStatusResponse { user_id: user.name, is_admin: user.is_admin }));
            }
            Ok(None) => {
                res.status_code(StatusCode::NOT_FOUND);
                res.render(Json(ErrorResponse { error: format!("User not found: {}", user_id) }));
            }
            Err(e) => {
                tracing::error!("Failed to get admin status: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to get admin status".to_string() }));
            }
        }
    }

    /// PUT /api/v1/users/{user_id}/admin - Set admin status
    #[handler]
    pub async fn set_admin_status(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();
        let body = req.parse_json::<AdminStatusRequest>().await.unwrap_or(AdminStatusRequest { is_admin: false });

        if let Err(e) = self.user_repo.set_admin_status(&user_id, body.is_admin).await {
            tracing::error!("Failed to set admin status: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to set admin status".to_string() }));
            return;
        }

        tracing::info!("Set admin status for {}: {}", user_id, body.is_admin);
        res.render(Json(SuccessResponse {
            success: true,
            message: format!("Admin status set to {} for user {}", body.is_admin, user_id),
        }));
    }

    /// GET /api/v1/users/{user_id}/shadow-ban - Get shadow ban status
    #[handler]
    pub async fn get_shadow_banned(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();

        match self.user_repo.get_user_attributes(&user_id).await {
            Ok(Some(attrs)) => {
                res.render(Json(ShadowBanStatusResponse { user_id, shadow_banned: attrs.shadow_banned }));
            }
            Ok(None) => {
                res.status_code(StatusCode::NOT_FOUND);
                res.render(Json(ErrorResponse { error: format!("User not found: {}", user_id) }));
            }
            Err(e) => {
                tracing::error!("Failed to get shadow ban status: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to get shadow ban status".to_string() }));
            }
        }
    }

    /// PUT /api/v1/users/{user_id}/shadow-ban - Set shadow ban status
    #[handler]
    pub async fn set_shadow_banned(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();
        let body = req.parse_json::<ShadowBanRequest>().await.unwrap_or(ShadowBanRequest { shadow_banned: false });

        if let Err(e) = self.user_repo.set_shadow_banned(&user_id, body.shadow_banned).await {
            tracing::error!("Failed to set shadow ban: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to set shadow ban".to_string() }));
            return;
        }

        tracing::info!("Set shadow ban for {}: {}", user_id, body.shadow_banned);
        res.render(Json(SuccessResponse {
            success: true,
            message: format!("Shadow ban set to {} for user {}", body.shadow_banned, user_id),
        }));
    }

    /// GET /api/v1/users/{user_id}/locked - Get locked status
    #[handler]
    pub async fn get_locked(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();

        match self.user_repo.get_user_attributes(&user_id).await {
            Ok(Some(attrs)) => {
                res.render(Json(LockStatusResponse { user_id, locked: attrs.locked }));
            }
            Ok(None) => {
                res.status_code(StatusCode::NOT_FOUND);
                res.render(Json(ErrorResponse { error: format!("User not found: {}", user_id) }));
            }
            Err(e) => {
                tracing::error!("Failed to get locked status: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to get locked status".to_string() }));
            }
        }
    }

    /// PUT /api/v1/users/{user_id}/locked - Set locked status
    #[handler]
    pub async fn set_locked(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let user_id = req.param::<String>("user_id").unwrap_or_default();
        let body = req.parse_json::<LockStatusRequest>().await.unwrap_or(LockStatusRequest { locked: false });

        if let Err(e) = self.user_repo.set_locked(&user_id, body.locked).await {
            tracing::error!("Failed to set locked status: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to set locked status".to_string() }));
            return;
        }

        tracing::info!("Set locked status for {}: {}", user_id, body.locked);
        res.render(Json(SuccessResponse {
            success: true,
            message: format!("Locked status set to {} for user {}", body.locked, user_id),
        }));
    }

    /// GET /api/v1/users/stats - Get user statistics
    #[handler]
    pub async fn get_user_stats(
        &self,
        depot: &mut Depot,
        res: &mut Response,
    ) {
        if !require_auth(depot, res) { return; }

        let total = match self.user_repo.get_user_count().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to get user count: {}", e);
                0
            }
        };
        let admins = match self.user_repo.get_admin_count().await {
            Ok(c) => c,
            Err(_) => 0
        };
        let deactivated = match self.user_repo.get_deactivated_count().await {
            Ok(c) => c,
            Err(_) => 0
        };

        res.render(Json(UserStatsResponse {
            total_users: total,
            admin_count: admins,
            deactivated_count: deactivated,
            active_count: total - deactivated,
        }));
    }
}

// ===== Conversion Implementations =====

impl From<&User> for UserResponse {
    fn from(user: &User) -> Self {
        UserResponse {
            name: user.name.clone(),
            displayname: user.displayname.clone(),
            avatar_url: user.avatar_url.clone(),
            is_admin: user.is_admin,
            is_guest: user.is_guest,
            is_deactivated: user.is_deactivated,
            is_erased: user.is_erased,
            shadow_banned: user.shadow_banned,
            locked: user.locked,
            creation_ts: user.creation_ts,
            last_seen_ts: user.last_seen_ts,
            user_type: user.user_type.clone(),
            appservice_id: user.appservice_id.clone(),
        }
    }
}

impl From<&UserDetails> for UserDetailsResponse {
    fn from(details: &UserDetails) -> Self {
        UserDetailsResponse {
            user: UserResponse::from(&details.user),
            device_count: details.device_count,
            session_count: details.session_count,
            joined_room_count: details.joined_room_count,
        }
    }
}