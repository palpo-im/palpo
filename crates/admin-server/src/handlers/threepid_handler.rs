/// Third-Party ID (Threepid) Handler - HTTP handlers for threepid operations
///
/// This module implements threepid (third-party identifier) API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
/// Uses PalpoClient to communicate with Palpo Matrix server via HTTP API.
///
/// Endpoints:
/// - GET /api/v1/threepid/email/users/{address} - Find user by email
/// - GET /api/v1/threepid/msisdn/users/{address} - Find user by phone
/// - GET /api/v1/users/{user_id}/threepids - Get user's threepids

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

use crate::palpo_client::PalpoClient;

use super::auth_middleware::require_auth;
use super::validation::validate_user_id;

// ===== Request Types =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreepidLookupResponse {
    pub medium: String,
    pub address: String,
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserThreepidsResponse {
    pub user_id: String,
    pub threepids: Vec<ThreepidInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreepidInfo {
    pub medium: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddThreepidRequest {
    pub medium: String,
    pub address: String,
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIdLookupResponse {
    pub auth_provider: String,
    pub external_id: String,
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserExternalIdsResponse {
    pub user_id: String,
    pub external_ids: Vec<ExternalIdInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIdInfo {
    pub auth_provider: String,
    pub external_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddExternalIdRequest {
    pub auth_provider: String,
    pub external_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

#[derive(Clone, Debug)]
pub struct ThreepidHandlerState {
    pub palpo_client: Arc<PalpoClient>,
}

impl ThreepidHandlerState {
    pub fn new(palpo_client: Arc<PalpoClient>) -> Self {
        Self { palpo_client }
    }
}

static THREEPID_HANDLER_STATE: OnceLock<ThreepidHandlerState> = OnceLock::new();

pub fn init_threepid_handler_state(state: ThreepidHandlerState) {
    THREEPID_HANDLER_STATE.set(state).expect("Threepid handler state already initialized");
}

fn get_threepid_handler_state() -> &'static ThreepidHandlerState {
    THREEPID_HANDLER_STATE.get().expect("Threepid handler state not initialized")
}

// ===== Handler Functions =====

/// GET /api/v1/threepid/email/users/{address} - Find user by email
#[handler]
pub async fn lookup_user_by_threepid(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let medium = req.param::<String>("medium").unwrap_or_default();
    let address = req.param::<String>("address").unwrap_or_default();

    if medium != "email" && medium != "msisdn" {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: "Invalid medium. Use 'email' or 'msisdn'".to_string() }));
        return;
    }

    match state.palpo_client.find_user_by_threepid(&medium, &address).await {
        Ok(result) => {
            if result.threepids.is_empty() {
                res.status_code(StatusCode::NOT_FOUND);
                res.render(Json(ErrorResponse { error: "User not found".to_string() }));
            } else {
                let threepid = &result.threepids[0];
                res.render(Json(ThreepidLookupResponse {
                    medium: threepid.medium.clone(),
                    address: threepid.address.clone(),
                    user_id: threepid.user_id.clone(),
                }));
            }
        }
        Err(e) => {
            tracing::error!("Failed to lookup user by threepid: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to lookup user".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/threepids - Get user's threepids
#[handler]
pub async fn get_user_threepids(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_user(&user_id).await {
        Ok(user) => {
            let threepids: Vec<ThreepidInfo> = user.threepids.iter().map(|t| ThreepidInfo {
                medium: t.medium.clone(),
                address: t.address.clone(),
            }).collect();

            res.render(Json(UserThreepidsResponse {
                user_id: user.name,
                threepids,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get user threepids: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get user threepids".to_string() }));
        }
    }
}

/// POST /api/v1/users/{user_id}/threepids - Add threepid to user
/// NOTE: Palpo admin API doesn't support adding threepids directly
#[handler]
pub async fn add_threepid(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // Palpo admin API doesn't support adding threepids
    res.status_code(StatusCode::NOT_IMPLEMENTED);
    res.render(Json(ErrorResponse { 
        error: "Adding threepids is not supported by Palpo server".to_string() 
    }));
}

/// DELETE /api/v1/users/{user_id}/threepids - Remove threepid from user
/// NOTE: Palpo admin API doesn't support removing threepids directly
#[handler]
pub async fn remove_threepid(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // Palpo admin API doesn't support removing threepids
    res.status_code(StatusCode::NOT_IMPLEMENTED);
    res.render(Json(ErrorResponse { 
        error: "Removing threepids is not supported by Palpo server".to_string() 
    }));
}

/// GET /api/v1/threepid/external_id/users/{auth_provider}/{external_id} - Find user by external ID
#[handler]
pub async fn lookup_user_by_external_id(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let auth_provider = req.param::<String>("auth_provider").unwrap_or_default();
    let external_id = req.param::<String>("external_id").unwrap_or_default();

    // There's no direct API for external ID lookup, so we need to search users
    // This is a limitation - we'll return NOT_IMPLEMENTED for now
    res.status_code(StatusCode::NOT_IMPLEMENTED);
    res.render(Json(ErrorResponse { 
        error: "External ID lookup is not supported by Palpo server".to_string() 
    }));
}

/// GET /api/v1/users/{user_id}/external_ids - Get user's external IDs
#[handler]
pub async fn get_user_external_ids(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_threepid_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_user(&user_id).await {
        Ok(user) => {
            let external_ids: Vec<ExternalIdInfo> = user.external_ids.iter().map(|e| ExternalIdInfo {
                auth_provider: e.auth_provider.clone(),
                external_id: e.external_id.clone(),
            }).collect();

            res.render(Json(UserExternalIdsResponse {
                user_id: user.name,
                external_ids,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get user external IDs: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get user external IDs".to_string() }));
        }
    }
}

/// POST /api/v1/users/{user_id}/external_ids - Add external ID to user
/// NOTE: Palpo admin API doesn't support adding external IDs directly
#[handler]
pub async fn add_external_id(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // Palpo admin API doesn't support adding external IDs
    res.status_code(StatusCode::NOT_IMPLEMENTED);
    res.render(Json(ErrorResponse { 
        error: "Adding external IDs is not supported by Palpo server".to_string() 
    }));
}

/// DELETE /api/v1/users/{user_id}/external_ids/{auth_provider}/{external_id} - Remove external ID
/// NOTE: Palpo admin API doesn't support removing external IDs directly
#[handler]
pub async fn remove_external_id(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // Palpo admin API doesn't support removing external IDs
    res.status_code(StatusCode::NOT_IMPLEMENTED);
    res.render(Json(ErrorResponse { 
        error: "Removing external IDs is not supported by Palpo server".to_string() 
    }));
}