/// Threepid Handler - HTTP handlers for third-party identifier lookup
///
/// This module implements threepid (third-party identifier) API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
///
/// Endpoints:
/// - GET /api/v1/threepid/{medium}/users/{address} - Lookup user by threepid
/// - GET /api/v1/users/{user_id}/threepids - Get user's threepids
/// - POST /api/v1/users/{user_id}/threepids - Add threepid
/// - DELETE /api/v1/users/{user_id}/threepids/{medium}/{address} - Remove threepid
/// - POST /api/v1/users/{user_id}/threepids/{medium}/{address}/validate - Validate threepid
/// - GET /api/v1/auth-providers/{provider}/users/{external_id} - Lookup user by external ID
/// - GET /api/v1/users/{user_id}/external-ids - Get user's external IDs
/// - POST /api/v1/users/{user_id}/external-ids - Add external ID
/// - DELETE /api/v1/users/{user_id}/external-ids/{provider}/{external_id} - Remove external ID

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::types::AdminError;
use crate::repositories::{ThreepidRepository, UserThreepid, UserExternalId};

use super::auth_middleware::require_auth;
use super::validation::{validate_user_id, validate_threepid_medium, ValidationError};

// ===== Request Types =====

/// Threepid lookup response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreepidLookupResponse {
    pub user_id: String,
    pub medium: String,
    pub address: String,
    pub validated: bool,
    pub validated_at: Option<i64>,
}

/// User threepids response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserThreepidsResponse {
    pub user_id: String,
    pub threepids: Vec<ThreepidInfo>,
}

/// Threepid info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreepidInfo {
    pub medium: String,
    pub address: String,
    pub validated: bool,
    pub validated_at: Option<i64>,
    pub added_at: i64,
}

/// Add threepid request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddThreepidRequest {
    pub medium: String,
    pub address: String,
}

/// External ID lookup response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIdLookupResponse {
    pub user_id: String,
    pub auth_provider: String,
    pub external_id: String,
}

/// User external IDs response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserExternalIdsResponse {
    pub user_id: String,
    pub external_ids: Vec<ExternalIdInfo>,
}

/// External ID info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIdInfo {
    pub auth_provider: String,
    pub external_id: String,
    pub created_at: i64,
}

/// Add external ID request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddExternalIdRequest {
    pub auth_provider: String,
    pub external_id: String,
}

/// Generic success response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Standard error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

/// Threepid handler state containing the repository
#[derive(Clone)]
pub struct ThreepidHandlerState {
    pub threepid_repo: Arc<dyn ThreepidRepository>,
}

impl ThreepidHandlerState {
    pub fn new(threepid_repo: Arc<dyn ThreepidRepository>) -> Self {
        Self { threepid_repo }
    }
}

/// Global threepid handler state
static THREEPID_HANDLER_STATE: std::sync::OnceLock<ThreepidHandlerState> = std::sync::OnceLock::new();

/// Initialize the global threepid handler state
pub fn init_threepid_handler_state(state: ThreepidHandlerState) {
    THREEPID_HANDLER_STATE.set(state).expect("Threepid handler state already initialized");
}

/// Get the global threepid handler state
fn get_threepid_handler_state() -> &'static ThreepidHandlerState {
    THREEPID_HANDLER_STATE.get().expect("Threepid handler state not initialized")
}

// ===== Handler Functions =====

/// GET /api/v1/threepid/{medium}/users/{address} - Lookup user by threepid
#[handler]
pub async fn lookup_user_by_threepid(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let medium = req.param::<String>("medium").unwrap_or_default();
    let address = req.param::<String>("address").unwrap_or_default();

    // Validate medium
    if let Err(e) = validate_threepid_medium(&medium) {
        tracing::warn!("Invalid threepid medium: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid medium: {}", e) }));
        return;
    }

    // Validate address is not empty
    if address.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: "Address cannot be empty".to_string() }));
        return;
    }

    let decoded_address = urlencoding::decode(&address).unwrap_or(address.clone());

    match state.threepid_repo.lookup_user_by_threepid(&medium, &decoded_address).await {
        Ok(Some(r)) => {
            res.render(Json(ThreepidLookupResponse {
                user_id: r.user_id,
                medium: r.medium,
                address: r.address,
                validated: r.validated,
                validated_at: r.validated_at,
            }));
        }
        Ok(None) => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(SuccessResponse {
                success: false,
                message: format!("No user found with threepid {}: {}", medium, decoded_address),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to lookup threepid: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to lookup threepid".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/threepids - Get user's threepids
#[handler]
pub async fn get_user_threepids(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.threepid_repo.get_user_threepids(&user_id).await {
        Ok(threepids) => {
            let threepid_info: Vec<ThreepidInfo> = threepids.iter().map(|t| ThreepidInfo {
                medium: t.medium.clone(),
                address: t.address.clone(),
                validated: t.validated_ts.is_some(),
                validated_at: t.validated_ts,
                added_at: t.added_ts,
            }).collect();
            res.render(Json(UserThreepidsResponse {
                user_id,
                threepids: threepid_info,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get user threepids: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get user threepids".to_string() }));
        }
    }
}

/// POST /api/v1/users/{user_id}/threepids - Add threepid
#[handler]
pub async fn add_threepid(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<AddThreepidRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid add threepid request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    // Validate medium
    if let Err(e) = validate_threepid_medium(&body.medium) {
        tracing::warn!("Invalid threepid medium: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid medium: {}", e) }));
        return;
    }

    // Validate address is not empty
    if body.address.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: "Address cannot be empty".to_string() }));
        return;
    }

    match state.threepid_repo.add_threepid(&user_id, &body.medium, &body.address).await {
        Ok(threepid) => {
            tracing::info!("Added threepid {}:{} for user {}", body.medium, body.address, user_id);
            res.status_code(StatusCode::CREATED);
            res.render(Json(ThreepidInfo {
                medium: threepid.medium,
                address: threepid.address,
                validated: threepid.validated_ts.is_some(),
                validated_at: threepid.validated_ts,
                added_at: threepid.added_ts,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to add threepid: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to add threepid".to_string() }));
        }
    }
}

/// DELETE /api/v1/users/{user_id}/threepids/{medium}/{address} - Remove threepid
#[handler]
pub async fn remove_threepid(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();
    let medium = req.param::<String>("medium").unwrap_or_default();
    let address = req.param::<String>("address").unwrap_or_default();

    // Validate parameters
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    if let Err(e) = validate_threepid_medium(&medium) {
        tracing::warn!("Invalid threepid medium: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid medium: {}", e) }));
        return;
    }

    let decoded_address = urlencoding::decode(&address).unwrap_or(address.clone());

    if let Err(e) = state.threepid_repo.remove_threepid(&user_id, &medium, &decoded_address).await {
        tracing::error!("Failed to remove threepid: {}", e);
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        res.render(Json(ErrorResponse { error: "Failed to remove threepid".to_string() }));
        return;
    }

    tracing::info!("Removed threepid {}:{} for user {}", medium, decoded_address, user_id);
    res.render(Json(SuccessResponse {
        success: true,
        message: format!("Threepid {}:{} removed successfully", medium, decoded_address),
    }));
}

/// GET /api/v1/auth-providers/{provider}/users/{external_id} - Lookup user by external ID
#[handler]
pub async fn lookup_user_by_external_id(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let provider = req.param::<String>("provider").unwrap_or_default();
    let external_id = req.param::<String>("external_id").unwrap_or_default();

    // Validate provider is not empty
    if provider.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: "Provider cannot be empty".to_string() }));
        return;
    }

    // Validate external_id is not empty
    if external_id.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: "External ID cannot be empty".to_string() }));
        return;
    }

    let decoded_external_id = urlencoding::decode(&external_id).unwrap_or(external_id.clone());

    match state.threepid_repo.lookup_user_by_external_id(&provider, &decoded_external_id).await {
        Ok(Some(r)) => {
            res.render(Json(ExternalIdLookupResponse {
                user_id: r.user_id,
                auth_provider: r.auth_provider,
                external_id: r.external_id,
            }));
        }
        Ok(None) => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(SuccessResponse {
                success: false,
                message: format!("No user found with external ID {}:{}", provider, decoded_external_id),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to lookup external ID: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to lookup external ID".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/external-ids - Get user's external IDs
#[handler]
pub async fn get_user_external_ids(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.threepid_repo.get_user_external_ids(&user_id).await {
        Ok(external_ids) => {
            let external_id_info: Vec<ExternalIdInfo> = external_ids.iter().map(|e| ExternalIdInfo {
                auth_provider: e.auth_provider.clone(),
                external_id: e.external_id.clone(),
                created_at: e.created_ts,
            }).collect();
            res.render(Json(UserExternalIdsResponse {
                user_id,
                external_ids: external_id_info,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get user external IDs: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get user external IDs".to_string() }));
        }
    }
}

/// POST /api/v1/users/{user_id}/external-ids - Add external ID
#[handler]
pub async fn add_external_id(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<AddExternalIdRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid add external ID request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    // Validate provider is not empty
    if body.auth_provider.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: "Auth provider cannot be empty".to_string() }));
        return;
    }

    // Validate external_id is not empty
    if body.external_id.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: "External ID cannot be empty".to_string() }));
        return;
    }

    match state.threepid_repo.add_external_id(&user_id, &body.auth_provider, &body.external_id).await {
        Ok(external_id) => {
            tracing::info!("Added external ID {}:{} for user {}", body.auth_provider, body.external_id, user_id);
            res.status_code(StatusCode::CREATED);
            res.render(Json(ExternalIdInfo {
                auth_provider: external_id.auth_provider,
                external_id: external_id.external_id,
                created_at: external_id.created_ts,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to add external ID: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to add external ID".to_string() }));
        }
    }
}

/// DELETE /api/v1/users/{user_id}/external-ids/{provider}/{external_id} - Remove external ID
#[handler]
pub async fn remove_external_id(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();
    let provider = req.param::<String>("provider").unwrap_or_default();
    let external_id = req.param::<String>("external_id").unwrap_or_default();

    // Validate parameters
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let decoded_external_id = urlencoding::decode(&external_id).unwrap_or(external_id.clone());

    if let Err(e) = state.threepid_repo.remove_external_id(&user_id, &provider, &decoded_external_id).await {
        tracing::error!("Failed to remove external ID: {}", e);
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        res.render(Json(ErrorResponse { error: "Failed to remove external ID".to_string() }));
        return;
    }

    tracing::info!("Removed external ID {}:{} for user {}", provider, decoded_external_id, user_id);
    res.render(Json(SuccessResponse {
        success: true,
        message: format!("External ID {}:{} removed successfully", provider, decoded_external_id),
    }));
}