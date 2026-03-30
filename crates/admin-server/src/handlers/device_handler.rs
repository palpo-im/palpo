/// Device Handler - HTTP handlers for device management API
///
/// This module implements device management API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
/// Uses PalpoClient to communicate with Palpo Matrix server via HTTP API.
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
use std::sync::{Arc, OnceLock};

use crate::palpo_client::{PalpoClient, PalpoDevice};

use super::auth_middleware::require_auth;
use super::validation::{validate_user_id, validate_device_id, validate_limit, validate_offset};

// ===== Request Types =====

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceListResponse {
    pub devices: Vec<DeviceResponse>,
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
}

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

impl DeviceResponse {
    pub fn from_palpo_device(device: &PalpoDevice, user_id: &str) -> Self {
        Self {
            device_id: device.device_id.clone(),
            user_id: user_id.to_string(),
            display_name: device.display_name.clone(),
            last_seen_ts: device.last_seen_ts,
            last_seen_ip: device.last_seen_ip.clone(),
            last_seen_user_agent: None,
            created_ts: device.last_seen_ts.unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceWithSessionsResponse {
    pub device: DeviceResponse,
    pub ip_addresses: Vec<String>,
    pub last_seen: Option<i64>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDeviceRequest {
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteDeviceRequest {
    pub device_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDeleteResponse {
    pub success: bool,
    pub deleted_count: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCountResponse {
    pub user_id: String,
    pub device_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

#[derive(Clone, Debug)]
pub struct DeviceHandlerState {
    pub palpo_client: Arc<PalpoClient>,
}

impl DeviceHandlerState {
    pub fn new(palpo_client: Arc<PalpoClient>) -> Self {
        Self { palpo_client }
    }
}

static DEVICE_HANDLER_STATE: OnceLock<DeviceHandlerState> = OnceLock::new();

pub fn init_device_handler_state(state: DeviceHandlerState) {
    DEVICE_HANDLER_STATE.set(state).expect("Device handler state already initialized");
}

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

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let query = req.parse_queries::<DeviceListQuery>().unwrap_or_default();
    let limit = validate_limit(query.limit).unwrap_or(50);
    let offset = validate_offset(query.offset).unwrap_or(0);

    match state.palpo_client.list_user_devices(&user_id).await {
        Ok(result) => {
            let devices: Vec<DeviceResponse> = result.devices.iter()
                .map(|d| DeviceResponse::from_palpo_device(&d, &user_id))
                .collect();
            res.render(Json(DeviceListResponse {
                devices,
                total_count: result.total,
                limit,
                offset,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to list user devices: {}", e);
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

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    if let Err(e) = validate_device_id(&device_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid device_id: {}", e) }));
        return;
    }

    // Get all devices and find the specific one
    match state.palpo_client.list_user_devices(&user_id).await {
        Ok(result) => {
            if let Some(device) = result.devices.iter().find(|d| d.device_id == device_id) {
                res.render(Json(DeviceResponse::from_palpo_device(device, &user_id)));
            } else {
                res.status_code(StatusCode::NOT_FOUND);
                res.render(Json(ErrorResponse { error: "Device not found".to_string() }));
            }
        }
        Err(e) => {
            tracing::error!("Failed to get device: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get device".to_string() }));
        }
    }
}

/// PUT /api/v1/users/{user_id}/devices/{device_id} - Update device
/// NOTE: Palpo API doesn't support updating device display_name via admin API
#[handler]
pub async fn update_device(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let user_id = req.param::<String>("user_id").unwrap_or_default();
    let device_id = req.param::<String>("device_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    if let Err(e) = validate_device_id(&device_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid device_id: {}", e) }));
        return;
    }

    // Palpo admin API doesn't support updating device
    res.status_code(StatusCode::NOT_IMPLEMENTED);
    res.render(Json(ErrorResponse { 
        error: "Device update is not supported by Palpo server".to_string() 
    }));
}

/// DELETE /api/v1/users/{user_id}/devices/{device_id} - Delete single device
#[handler]
pub async fn delete_device(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();
    let device_id = req.param::<String>("device_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    if let Err(e) = validate_device_id(&device_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid device_id: {}", e) }));
        return;
    }

    match state.palpo_client.delete_user_device(&user_id, &device_id).await {
        Ok(_) => {
            tracing::info!("Deleted device {} for user {}", device_id, user_id);
            res.render(Json(SuccessResponse {
                success: true,
                message: format!("Device {} deleted", device_id),
            }));
        }
        Err(e) => {
            tracing::error!("Failed to delete device: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to delete device".to_string() }));
        }
    }
}

/// POST /api/v1/users/{user_id}/devices/delete - Batch delete devices
#[handler]
pub async fn delete_devices(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    let body = match req.parse_json::<DeleteDeviceRequest>().await {
        Ok(b) => b,
        Err(e) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse { error: "Invalid request body".to_string() }));
            return;
        }
    };

    if body.device_ids.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: "No device IDs provided".to_string() }));
        return;
    }

    match state.palpo_client.delete_user_devices(&user_id, &body.device_ids).await {
        Ok(_) => {
            tracing::info!("Deleted {} devices for user {}", body.device_ids.len(), user_id);
            res.render(Json(BatchDeleteResponse {
                success: true,
                deleted_count: body.device_ids.len() as u64,
                message: format!("Deleted {} devices", body.device_ids.len()),
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

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // First get all devices, then delete them
    match state.palpo_client.list_user_devices(&user_id).await {
        Ok(result) => {
            if result.devices.is_empty() {
                res.render(Json(SuccessResponse {
                    success: true,
                    message: "No devices to delete".to_string(),
                }));
                return;
            }

            let device_ids: Vec<String> = result.devices.iter()
                .map(|d| d.device_id.clone())
                .collect();

            match state.palpo_client.delete_user_devices(&user_id, &device_ids).await {
                Ok(_) => {
                    tracing::info!("Deleted all {} devices for user {}", device_ids.len(), user_id);
                    res.render(Json(BatchDeleteResponse {
                        success: true,
                        deleted_count: device_ids.len() as u64,
                        message: format!("Deleted {} devices", device_ids.len()),
                    }));
                }
                Err(e) => {
                    tracing::error!("Failed to delete all devices: {}", e);
                    res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                    res.render(Json(ErrorResponse { error: "Failed to delete devices".to_string() }));
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to get user devices: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get devices".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/devices/count - Get user device count
#[handler]
pub async fn get_user_device_count(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_device_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.list_user_devices(&user_id).await {
        Ok(result) => {
            res.render(Json(DeviceCountResponse {
                user_id,
                device_count: result.total,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get device count: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get device count".to_string() }));
        }
    }
}