/// Device Handler - HTTP handlers for device management API
///
/// This module implements device management API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
///
/// Endpoints:
/// - GET /api/v1/users/{user_id}/devices - List user devices
/// - GET /api/v1/users/{user_id}/devices/{device_id} - Get device details
/// - PUT /api/v1/users/{user_id}/devices/{device_id} - Update device
/// - DELETE /api/v1/users/{user_id}/devices/{device_id} - Delete single device
/// - POST /api/v1/users/{user_id}/devices/delete - Batch delete devices
/// - DELETE /api/v1/users/{user_id}/devices - Delete all user devices

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::repositories::{DieselDeviceRepository, Device, DeviceFilter, DeviceWithSessions};
use crate::device_repository::DeviceRepository;

use super::auth_middleware::require_auth;
use super::validation::{validate_user_id, validate_device_id, validate_limit, validate_offset};

// ===== Request Types =====

/// Device list query parameters
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Device list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceListResponse {
    pub devices: Vec<DeviceResponse>,
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Device response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceResponse {
    pub device_id: String,
    pub user_id: String,
    pub display_name: Option<String>,
    pub last_seen_ts: Option<i64>,
    pub last_seen_ip: Option<String>,
    pub last_seen_user_agent: Option<String>,
    pub created_ts: i64,
}

/// Device with sessions response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceWithSessionsResponse {
    pub device: DeviceResponse,
    pub ip_addresses: Vec<String>,
    pub last_seen: Option<i64>,
    pub user_agent: Option<String>,
}

/// Update device request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDeviceRequest {
    pub display_name: Option<String>,
}

/// Delete device request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteDeviceRequest {
    pub device_ids: Vec<String>,
}

/// Batch delete response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDeleteResponse {
    pub success: bool,
    pub deleted_count: u64,
    pub message: String,
}

/// Generic success response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Device count response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCountResponse {
    pub user_id: String,
    pub device_count: i64,
}

/// Standard error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

/// Device handler state containing the repository
#[derive(Clone, Debug)]
pub struct DeviceHandlerState {
    pub device_repo: Arc<DieselDeviceRepository>,
}

impl DeviceHandlerState {
    pub fn new(device_repo: Arc<DieselDeviceRepository>) -> Self {
        Self { device_repo }
    }
}

/// Global device handler state
static DEVICE_HANDLER_STATE: std::sync::OnceLock<DeviceHandlerState> = std::sync::OnceLock::new();

/// Initialize the global device handler state
pub fn init_device_handler_state(state: DeviceHandlerState) {
    DEVICE_HANDLER_STATE.set(state).expect("Device handler state already initialized");
}

/// Get the global device handler state
fn get_device_handler_state() -> &'static DeviceHandlerState {
    DEVICE_HANDLER_STATE.get().expect("Device handler state not initialized")
}

// ===== Handler Functions =====

/// GET /api/v1/users/{user_id}/devices - List user devices
#[handler]
pub async fn list_user_devices(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let query = req.parse_queries::<DeviceListQuery>().unwrap_or_default();

    // Validate pagination parameters
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

    let filter = DeviceFilter {
        user_id: Some(user_id.clone()),
        limit: Some(limit),
        offset: Some(offset),
    };

    match state.device_repo.list_devices(&filter).await {
        Ok(result) => {
            let devices: Vec<DeviceResponse> = result.devices.iter().map(DeviceResponse::from).collect();
            res.render(Json(DeviceListResponse {
                devices,
                total_count: result.total_count,
                limit,
                offset,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to list devices: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to list devices".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/devices/{device_id} - Get device details
#[handler]
pub async fn get_device(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();
    let device_id = req.param::<String>("device_id").unwrap_or_default();

    // Validate parameters
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    if let Err(e) = validate_device_id(&device_id) {
        tracing::warn!("Invalid device_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid device_id: {}", e) }));
        return;
    }

    match state.device_repo.get_device(&user_id, &device_id).await {
        Ok(Some(device)) => {
            res.render(Json(DeviceResponse::from(&device)));
        }
        Ok(None) => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(ErrorResponse { error: format!("Device {} not found for user {}", device_id, user_id) }));
        }
        Err(e) => {
            tracing::error!("Failed to get device: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get device".to_string() }));
        }
    }
}

/// PUT /api/v1/users/{user_id}/devices/{device_id} - Update device
#[handler]
pub async fn update_device(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();
    let device_id = req.param::<String>("device_id").unwrap_or_default();

    // Validate parameters
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    if let Err(e) = validate_device_id(&device_id) {
        tracing::warn!("Invalid device_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid device_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<UpdateDeviceRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid update device request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    let input = crate::repositories::UpdateDeviceInput {
        display_name: body.display_name,
    };

    match state.device_repo.update_device(&user_id, &device_id, &input).await {
        Ok(device) => {
            tracing::info!("Updated device {} for user {}", device_id, user_id);
            res.render(Json(DeviceResponse::from(&device)));
        }
        Err(e) => {
            tracing::error!("Failed to update device: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to update device".to_string() }));
        }
    }
}

/// DELETE /api/v1/users/{user_id}/devices/{device_id} - Delete single device
#[handler]
pub async fn delete_device(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();
    let device_id = req.param::<String>("device_id").unwrap_or_default();

    // Validate parameters
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    if let Err(e) = validate_device_id(&device_id) {
        tracing::warn!("Invalid device_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid device_id: {}", e) }));
        return;
    }

    if let Err(e) = state.device_repo.delete_device(&user_id, &device_id).await {
        tracing::error!("Failed to delete device: {}", e);
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        res.render(Json(ErrorResponse { error: "Failed to delete device".to_string() }));
        return;
    }

    tracing::info!("Deleted device {} for user {}", device_id, user_id);
    res.render(Json(SuccessResponse {
        success: true,
        message: format!("Device {} deleted successfully", device_id),
    }));
}

/// POST /api/v1/users/{user_id}/devices/delete - Batch delete devices
#[handler]
pub async fn delete_devices(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<DeleteDeviceRequest>().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Invalid delete devices request: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    // Validate device IDs
    for device_id in &body.device_ids {
        if let Err(e) = validate_device_id(device_id) {
            tracing::warn!("Invalid device_id in batch delete: {}", e);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: format!("Invalid device_id: {}", e) }));
            return;
        }
    }

    // Limit batch size
    if body.device_ids.len() > 100 {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: "Cannot delete more than 100 devices at once".to_string() }));
        return;
    }

    match state.device_repo.delete_devices(&user_id, &body.device_ids).await {
        Ok(count) => {
            tracing::info!("Deleted {} devices for user {}", count, user_id);
            res.render(Json(BatchDeleteResponse {
                success: true,
                deleted_count: count,
                message: format!("Deleted {} devices successfully", count),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to delete devices: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to delete devices".to_string() }));
        }
    }
}

/// DELETE /api/v1/users/{user_id}/devices - Delete all user devices
#[handler]
pub async fn delete_all_user_devices(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.device_repo.delete_all_user_devices(&user_id).await {
        Ok(count) => {
            tracing::info!("Deleted all {} devices for user {}", count, user_id);
            res.render(Json(BatchDeleteResponse {
                success: true,
                deleted_count: count,
                message: format!("Deleted all {} devices successfully", count),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to delete all devices: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to delete devices".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/devices/count - Get device count
#[handler]
pub async fn get_user_device_count(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    // Validate user_id format
    if let Err(e) = validate_user_id(&user_id) {
        tracing::warn!("Invalid user_id format: {}", e);
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.device_repo.get_user_device_count(&user_id).await {
        Ok(count) => {
            res.render(Json(DeviceCountResponse {
                user_id,
                device_count: count,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get device count: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get device count".to_string() }));
        }
    }
}

// ===== Conversion Implementations =====

impl From<&Device> for DeviceResponse {
    fn from(device: &Device) -> Self {
        DeviceResponse {
            device_id: device.device_id.clone(),
            user_id: device.user_id.clone(),
            display_name: device.display_name.clone(),
            last_seen_ts: device.last_seen_ts,
            last_seen_ip: device.last_seen_ip.clone(),
            last_seen_user_agent: device.last_seen_user_agent.clone(),
            created_ts: device.created_ts,
        }
    }
}

impl From<&DeviceWithSessions> for DeviceWithSessionsResponse {
    fn from(d: &DeviceWithSessions) -> Self {
        DeviceWithSessionsResponse {
            device: DeviceResponse::from(&d.device),
            ip_addresses: d.ip_addresses.clone(),
            last_seen: d.last_seen,
            user_agent: d.user_agent.clone(),
        }
    }
}