/// User Handler - HTTP handlers for user management API
///
/// This module implements all user management API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
/// Uses PalpoClient to communicate with Palpo Matrix server via HTTP API.
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
use std::sync::{Arc, OnceLock};

use crate::palpo_client::{CreateOrUpdateUserRequest, ListUsersQuery, PalpoClient, PalpoUser};

use super::auth_middleware::require_auth;
use super::validation::{
    validate_user_id, validate_username, validate_limit, validate_offset,
    validate_displayname,
};

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    pub user_id: String,
    pub username: String,
    pub displayname: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub is_local: bool,
    pub server_name: String,
    pub shadow_banned: bool,
    pub deactivated: bool,
    pub locked: bool,
}

impl UserResponse {
    pub fn from_palpo_user(user: &PalpoUser, server_name: &str) -> Self {
        let username = user.name
            .trim_start_matches('@')
            .split(':')
            .next()
            .unwrap_or(&user.name)
            .to_string();

        Self {
            user_id: user.name.clone(),
            username,
            displayname: user.displayname.clone(),
            avatar_url: user.avatar_url.clone(),
            is_admin: user.admin,
            is_guest: user.is_guest.unwrap_or(false),
            is_local: user.name.ends_with(&format!(":{}", server_name)),
            server_name: server_name.to_string(),
            shadow_banned: user.shadow_banned,
            deactivated: user.deactivated,
            locked: false,
        }
    }
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
    pub user_id: String,
    pub username: String,
    pub displayname: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub is_local: bool,
    pub server_name: String,
    pub shadow_banned: bool,
    pub deactivated: bool,
    pub locked: bool,
    pub creation_ts: Option<i64>,
    pub threepids: Vec<ThreepidInfo>,
    pub external_ids: Vec<ExternalIdInfo>,
    pub user_type: Option<String>,
    pub appservice_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreepidInfo {
    pub medium: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIdInfo {
    pub auth_provider: String,
    pub external_id: String,
}

impl UserDetailsResponse {
    pub fn from_palpo_user(user: &PalpoUser, server_name: &str) -> Self {
        let username = user.name
            .trim_start_matches('@')
            .split(':')
            .next()
            .unwrap_or(&user.name)
            .to_string();

        Self {
            user_id: user.name.clone(),
            username,
            displayname: user.displayname.clone(),
            avatar_url: user.avatar_url.clone(),
            is_admin: user.admin,
            is_guest: user.is_guest.unwrap_or(false),
            is_local: user.name.ends_with(&format!(":{}", server_name)),
            server_name: server_name.to_string(),
            shadow_banned: user.shadow_banned,
            deactivated: user.deactivated,
            locked: false,
            creation_ts: user.creation_ts,
            threepids: user.threepids.iter().map(|t| ThreepidInfo {
                medium: t.medium.clone(),
                address: t.address.clone(),
            }).collect(),
            external_ids: user.external_ids.iter().map(|e| ExternalIdInfo {
                auth_provider: e.auth_provider.clone(),
                external_id: e.external_id.clone(),
            }).collect(),
            user_type: user.user_type.clone(),
            appservice_id: user.appservice_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsernameAvailabilityResponse {
    pub available: bool,
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStatsResponse {
    pub total_users: i64,
    pub active_users: i64,
    pub inactive_users: i64,
    pub admin_users: i64,
    pub guest_users: i64,
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
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== State =====

#[derive(Clone, Debug)]
pub struct UserHandlerState {
    pub palpo_client: Arc<PalpoClient>,
    pub server_name: String,
}

impl UserHandlerState {
    pub fn new(palpo_client: Arc<PalpoClient>, server_name: String) -> Self {
        Self { palpo_client, server_name }
    }
}

static USER_HANDLER_STATE: OnceLock<UserHandlerState> = OnceLock::new();

pub fn init_user_handler_state(state: UserHandlerState) {
    USER_HANDLER_STATE.set(state).expect("User handler state already initialized");
}

fn get_user_handler_state() -> &'static UserHandlerState {
    USER_HANDLER_STATE.get().expect("User handler state not initialized")
}

// ===== Handler Functions =====

/// POST /api/v1/users - Create a new user
#[handler]
pub async fn create_user(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let body = match req.parse_json::<CreateUserRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid create user request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    if let Err(e) = validate_user_id(&body.user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    if let Err(e) = validate_displayname(body.displayname.as_deref()) {
        tracing::warn!("Invalid displayname: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid displayname: {}", e) }));
        return;
    }

    let create_req = CreateOrUpdateUserRequest {
        displayname: body.displayname,
        avatar_url: body.avatar_url,
        admin: Some(body.is_admin),
        user_type: body.user_type,
        ..Default::default()
    };

    match state.palpo_client.create_or_update_user(&body.user_id, &create_req).await {
        Ok(user) => {
            tracing::info!("Created user: {}", user.name);
            res.status_code(StatusCode::CREATED);
            res.render(Json(UserResponse::from_palpo_user(&user, &state.server_name)));
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
pub async fn list_users(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let query = req.parse_queries::<UserListQuery>().unwrap_or_default();

    let limit = match validate_limit(query.limit) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("Invalid limit parameter: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: format!("Invalid limit: {}", e) }));
            return;
        }
    };

    let offset = match validate_offset(query.offset) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("Invalid offset parameter: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: format!("Invalid offset: {}", e) }));
            return;
        }
    };

    let list_query = ListUsersQuery {
        from: Some(offset),
        limit: Some(limit),
        search_term: query.search.clone(),
        admins: query.is_admin,
        deactivated: query.is_deactivated,
        ..Default::default()
    };

    match state.palpo_client.list_users(&list_query).await {
        Ok(result) => {
            let users: Vec<UserResponse> = result.users.iter()
                .map(|u| UserResponse::from_palpo_user(&u, &state.server_name))
                .collect();
            res.render(Json(UserListResponse {
                users,
                total_count: result.total,
                limit,
                offset,
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
pub async fn get_user(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_user(&user_id).await {
        Ok(user) => {
            res.render(Json(UserResponse::from_palpo_user(&user, &state.server_name)));
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
pub async fn get_user_details(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_user(&user_id).await {
        Ok(user) => {
            res.render(Json(UserDetailsResponse::from_palpo_user(&user, &state.server_name)));
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
pub async fn update_user(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<UpdateUserRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid update user request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    if let Err(e) = validate_displayname(body.displayname.as_deref()) {
        tracing::warn!("Invalid displayname: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid displayname: {}", e) }));
        return;
    }

    let update_req = CreateOrUpdateUserRequest {
        displayname: body.displayname,
        avatar_url: body.avatar_url,
        admin: body.is_admin,
        user_type: body.user_type,
        ..Default::default()
    };

    match state.palpo_client.create_or_update_user(&user_id, &update_req).await {
        Ok(user) => {
            tracing::info!("Updated user: {}", user.name);
            res.render(Json(UserResponse::from_palpo_user(&user, &state.server_name)));
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
pub async fn deactivate_user(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = req.parse_json::<DeactivateUserRequest>().await.unwrap_or(DeactivateUserRequest { erase: false });

    if let Err(e) = state.palpo_client.deactivate_user(&user_id, body.erase).await {
        tracing::error!("Failed to deactivate user: {}", e);
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        res.render(Json(ErrorResponse { error: "Failed to deactivate user".to_string() }));
        return;
    }

    tracing::info!("Deactivated user: {} (erase: {})", user_id, body.erase);
    res.render(Json(SuccessResponse {
        message: format!("User {} deactivated", user_id),
    }));
}

/// POST /api/v1/users/{user_id}/reactivate - Reactivate deactivated user
#[handler]
pub async fn reactivate_user(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // Reactivation is done by creating/updating the user with empty password
    let reactivate_req = CreateOrUpdateUserRequest {
        displayname: None,
        avatar_url: None,
        admin: None,
        user_type: None,
        password: Some("".to_string()),
        ..Default::default()
    };

    match state.palpo_client.create_or_update_user(&user_id, &reactivate_req).await {
        Ok(_) => {
            tracing::info!("Reactivated user: {}", user_id);
            res.render(Json(SuccessResponse {
                message: format!("User {} reactivated", user_id),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to reactivate user: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to reactivate user".to_string() }));
        }
    }
}

/// GET /api/v1/users/username-available/{username} - Check username availability
#[handler]
pub async fn check_username_available(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let username = req.param::<String>("username").unwrap_or_default();

    if let Err(e) = validate_username(&username) {
        tracing::warn!("Invalid username format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid username: {}", e) }));
        return;
    }

    // Construct full user_id
    let user_id = format!("@{}:{}", username, state.server_name);

    match state.palpo_client.get_user(&user_id).await {
        Ok(_) => {
            // User exists, so username is not available
            res.render(Json(UsernameAvailabilityResponse {
                available: false,
                username,
            }));
        }
        Err(e) => {
            // Check if it's a 404 error by looking at the message
            let err_msg = e.to_string();
            if err_msg.contains("404") || err_msg.contains("not found") || err_msg.contains("User not found") {
                // User doesn't exist, username is available
                res.render(Json(UsernameAvailabilityResponse {
                    available: true,
                    username,
                }));
            } else {
                tracing::error!("Failed to check username availability: {}", e);
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                res.render(Json(ErrorResponse { error: "Failed to check username availability".to_string() }));
            }
        }
    }
}

/// GET /api/v1/users/{user_id}/admin - Get admin status
#[handler]
pub async fn get_admin_status(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_user(&user_id).await {
        Ok(user) => {
            res.render(Json(AdminStatusResponse {
                user_id: user.name,
                is_admin: user.admin,
            }));
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
pub async fn set_admin_status(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<AdminStatusRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid admin status request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    // Get current user and update admin status
    let update_req = CreateOrUpdateUserRequest {
        displayname: None,
        avatar_url: None,
        admin: Some(body.is_admin),
        user_type: None,
        password: None,
        ..Default::default()
    };

    match state.palpo_client.create_or_update_user(&user_id, &update_req).await {
        Ok(user) => {
            tracing::info!("Set admin status for user: {} -> {}", user.name, body.is_admin);
            res.render(Json(AdminStatusResponse {
                user_id: user.name,
                is_admin: user.admin,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to set admin status: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to set admin status".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/shadow-ban - Get shadow ban status
#[handler]
pub async fn get_shadow_banned(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_user(&user_id).await {
        Ok(user) => {
            res.render(Json(ShadowBanStatusResponse {
                user_id: user.name,
                shadow_banned: user.shadow_banned,
            }));
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
pub async fn set_shadow_banned(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<ShadowBanRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid shadow ban request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    let result = if body.shadow_banned {
        state.palpo_client.shadow_ban_user(&user_id).await
    } else {
        state.palpo_client.unshadow_ban_user(&user_id).await
    };

    match result {
        Ok(_) => {
            tracing::info!("Set shadow ban for user: {} -> {}", user_id, body.shadow_banned);
            res.render(Json(ShadowBanStatusResponse {
                user_id,
                shadow_banned: body.shadow_banned,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to set shadow ban: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to set shadow ban".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/locked - Get locked status
/// NOTE: Locked status is not directly available in Palpo API, always returns false
#[handler]
pub async fn get_locked(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // Palpo API doesn't support locked status, always return false
    res.render(Json(LockStatusResponse {
        user_id,
        locked: false,
    }));
}

/// PUT /api/v1/users/{user_id}/locked - Set locked status
/// NOTE: Locked status is not directly available in Palpo API
#[handler]
pub async fn set_locked(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // Palpo API doesn't support locked status
    res.status_code(StatusCode::NOT_IMPLEMENTED);
    res.render(Json(ErrorResponse { 
        error: "Locked status is not supported by Palpo server".to_string() 
    }));
}

/// GET /api/v1/users/stats - Get user statistics
#[handler]
pub async fn get_user_stats(depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_user_handler_state();

    // Get all users to calculate stats
    let list_query = ListUsersQuery {
        from: Some(0),
        limit: Some(1000),
        ..Default::default()
    };

    match state.palpo_client.list_users(&list_query).await {
        Ok(result) => {
            let total = result.total;
            let active = result.users.iter().filter(|u| !u.deactivated).count() as i64;
            let inactive = result.users.iter().filter(|u| u.deactivated).count() as i64;
            let admins = result.users.iter().filter(|u| u.admin).count() as i64;
            let guests = result.users.iter().filter(|u| u.is_guest.unwrap_or(false)).count() as i64;

            res.render(Json(UserStatsResponse {
                total_users: total,
                active_users: active,
                inactive_users: inactive,
                admin_users: admins,
                guest_users: guests,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get user stats: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get user stats".to_string() }));
        }
    }
}