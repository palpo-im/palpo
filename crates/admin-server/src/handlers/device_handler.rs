/// Device Handler - HTTP handlers for device management API
///
/// This module implements device management API endpoints including:
/// - List user devices
/// - Get device details
/// - Update device
/// - Delete single device
/// - Batch delete devices
/// - Delete all user devices

use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};

use crate::types::AdminError;
use crate::repositories::{DeviceRepository, Device, DeviceFilter, DeviceWithSessions};

/// Device list query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceListQuery {
    pub user_id: Option<String>,
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

/// Device handler configuration
pub struct DeviceHandler<T: DeviceRepository> {
    device_repo: T,
}

impl<T: DeviceRepository> DeviceHandler<T> {
    /// Create a new handler with the given repository
    pub fn new(device_repo: T) -> Self {
        Self { device_repo }
    }

    /// List all devices for a user
    pub async fn list_user_devices(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let devices = self.device_repo.get_user_devices(&user_id).await?;

        let response: Vec<DeviceResponse> = devices.iter().map(DeviceResponse::from).collect();

        Ok(HttpResponse::Ok().json(DeviceListResponse {
            devices: response,
            total_count: devices.len() as i64,
            limit: devices.len() as i64,
            offset: 0,
        }))
    }

    /// Get devices with session info for a user
    pub async fn list_user_devices_with_sessions(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let devices = self.device_repo.get_user_devices_with_sessions(&user_id).await?;

        let response: Vec<DeviceWithSessionsResponse> = devices.iter().map(DeviceWithSessionsResponse::from).collect();

        Ok(HttpResponse::Ok().json(response))
    }

    /// Get a specific device
    pub async fn get_device(
        &self,
        path: web::Path<(String, String)>,
    ) -> Result<HttpResponse, AdminError> {
        let (user_id, device_id) = path.into_inner();

        let device = self.device_repo.get_device(&user_id, &device_id).await?;

        match device {
            Some(d) => Ok(HttpResponse::Ok().json(DeviceResponse::from(&d))),
            None => Ok(HttpResponse::NotFound().json(SuccessResponse {
                success: false,
                message: format!("Device {} not found for user {}", device_id, user_id),
            })),
        }
    }

    /// Update a device
    pub async fn update_device(
        &self,
        path: web::Path<(String, String)>,
        req: web::Json<UpdateDeviceRequest>,
    ) -> Result<HttpResponse, AdminError> {
        let (user_id, device_id) = path.into_inner();

        let input = crate::repositories::UpdateDeviceInput {
            display_name: req.display_name.clone(),
        };

        let device = self.device_repo.update_device(&user_id, &device_id, &input).await?;

        tracing::info!("Updated device {} for user {}", device_id, user_id);

        Ok(HttpResponse::Ok().json(DeviceResponse::from(&device)))
    }

    /// Delete a single device
    pub async fn delete_device(
        &self,
        path: web::Path<(String, String)>,
    ) -> Result<HttpResponse, AdminError> {
        let (user_id, device_id) = path.into_inner();

        self.device_repo.delete_device(&user_id, &device_id).await?;

        tracing::info!("Deleted device {} for user {}", device_id, user_id);

        Ok(HttpResponse::Ok().json(SuccessResponse {
            success: true,
            message: format!("Device {} deleted successfully", device_id),
        }))
    }

    /// Batch delete devices
    pub async fn delete_devices(
        &self,
        user_id: web::Path<String>,
        req: web::Json<DeleteDeviceRequest>,
    ) -> Result<HttpResponse, AdminError> {
        let count = self.device_repo.delete_devices(&user_id, &req.device_ids).await?;

        tracing::info!("Deleted {} devices for user {}", count, user_id);

        Ok(HttpResponse::Ok().json(BatchDeleteResponse {
            success: true,
            deleted_count: count,
            message: format!("Deleted {} devices successfully", count),
        }))
    }

    /// Delete all devices for a user
    pub async fn delete_all_user_devices(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let count = self.device_repo.delete_all_user_devices(&user_id).await?;

        tracing::info!("Deleted all {} devices for user {}", count, user_id);

        Ok(HttpResponse::Ok().json(BatchDeleteResponse {
            success: true,
            deleted_count: count,
            message: format!("Deleted all {} devices successfully", count),
        }))
    }

    /// Get device count for a user
    pub async fn get_user_device_count(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let count = self.device_repo.get_user_device_count(&user_id).await?;

        Ok(HttpResponse::Ok().json(DeviceCountResponse {
            user_id: user_id.to_string(),
            device_count: count,
        }))
    }

    /// List devices with pagination (admin view)
    pub async fn list_devices(&self, query: web::Query<DeviceListQuery>) -> Result<HttpResponse, AdminError> {
        let filter = DeviceFilter {
            user_id: query.user_id.clone(),
            limit: query.limit,
            offset: query.offset,
        };

        let result = self.device_repo.list_devices(&filter).await?;

        let devices: Vec<DeviceResponse> = result.devices.iter().map(DeviceResponse::from).collect();

        Ok(HttpResponse::Ok().json(DeviceListResponse {
            devices,
            total_count: result.total_count,
            limit: result.limit,
            offset: result.offset,
        }))
    }
}

/// Device count response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCountResponse {
    pub user_id: String,
    pub device_count: i64,
}

// Conversion implementations
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::DieselDeviceRepository;
    use palpo_data::DieselPool;

    #[tokio::test]
    #[ignore]
    async fn test_list_user_devices() {}

    #[tokio::test]
    #[ignore]
    async fn test_delete_device() {}

    #[tokio::test]
    #[ignore]
    async fn test_batch_delete_devices() {}
}