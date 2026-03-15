/// Device Repository - Database operations for device management
///
/// This module provides the data access layer for device management operations.
/// It implements the DeviceRepository trait with direct PostgreSQL operations
/// using Diesel ORM, optimized for performance.
///
/// Features:
/// - Device CRUD operations
/// - User device listing
/// - Batch device deletion
/// - Device session tracking

use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use chrono::Utc;

use crate::types::AdminError;
use palpo_data::DieselPool;

/// Device entity representing a Matrix device
#[derive(Debug, Clone, Queryable, Insertable, AsChangeset, Serialize, Deserialize)]
#[diesel(table_name = devices)]
pub struct Device {
    pub device_id: String,
    pub user_id: String,
    pub display_name: Option<String>,
    pub last_seen_ts: Option<i64>,
    pub last_seen_ip: Option<String>,
    pub last_seen_user_agent: Option<String>,
    pub created_ts: i64,
}

/// Device creation input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDeviceInput {
    pub device_id: String,
    pub user_id: String,
    pub display_name: Option<String>,
    pub initial_device: bool,
}

/// Device update input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDeviceInput {
    pub display_name: Option<String>,
}

/// Device list filter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceFilter {
    pub user_id: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Device list result with pagination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceListResult {
    pub devices: Vec<Device>,
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Device with IP and session info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceWithSessions {
    pub device: Device,
    pub ip_addresses: Vec<String>,
    pub last_seen: Option<i64>,
    pub user_agent: Option<String>,
}

/// Repository trait for device data access operations
#[async_trait::async_trait]
pub trait DeviceRepository {
    /// Create a new device
    async fn create_device(&self, input: &CreateDeviceInput) -> Result<Device, AdminError>;

    /// Get a device by user ID and device ID
    async fn get_device(&self, user_id: &str, device_id: &str) -> Result<Option<Device>, AdminError>;

    /// Get all devices for a user
    async fn get_user_devices(&self, user_id: &str) -> Result<Vec<Device>, AdminError>;

    /// Get devices with session info
    async fn get_user_devices_with_sessions(&self, user_id: &str) -> Result<Vec<DeviceWithSessions>, AdminError>;

    /// Update device information
    async fn update_device(&self, user_id: &str, device_id: &str, input: &UpdateDeviceInput) -> Result<Device, AdminError>;

    /// Delete a single device
    async fn delete_device(&self, user_id: &str, device_id: &str) -> Result<(), AdminError>;

    /// Delete multiple devices for a user
    async fn delete_devices(&self, user_id: &str, device_ids: &[String]) -> Result<u64, AdminError>;

    /// Delete all devices for a user
    async fn delete_all_user_devices(&self, user_id: &str) -> Result<u64, AdminError>;

    /// Update device last seen timestamp and IP
    async fn update_device_last_seen(&self, user_id: &str, device_id: &str, ip: &str, user_agent: Option<&str>) -> Result<(), AdminError>;

    /// Get device count for a user
    async fn get_user_device_count(&self, user_id: &str) -> Result<i64, AdminError>;

    /// List devices with pagination
    async fn list_devices(&self, filter: &DeviceFilter) -> Result<DeviceListResult, AdminError>;
}

/// Diesel-based DeviceRepository implementation
#[derive(Debug)]
pub struct DieselDeviceRepository {
    db_pool: DieselPool,
}

impl DieselDeviceRepository {
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }
}

#[async_trait::async_trait]
impl DeviceRepository for DieselDeviceRepository {
    async fn create_device(&self, input: &CreateDeviceInput) -> Result<Device, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = Utc::now().timestamp_millis();

        let device = Device {
            device_id: input.device_id.clone(),
            user_id: input.user_id.clone(),
            display_name: input.display_name.clone(),
            last_seen_ts: None,
            last_seen_ip: None,
            last_seen_user_agent: None,
            created_ts: now,
        };

        diesel::insert_into(devices::table)
            .values(&device)
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(device)
    }

    async fn get_device(&self, user_id: &str, device_id: &str) -> Result<Option<Device>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let device = devices::table
            .filter(devices::user_id.eq(user_id))
            .filter(devices::device_id.eq(device_id))
            .first::<Device>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(device)
    }

    async fn get_user_devices(&self, user_id: &str) -> Result<Vec<Device>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let devices = devices::table
            .filter(devices::user_id.eq(user_id))
            .order_by(devices::created_ts.desc())
            .load::<Device>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(devices)
    }

    async fn get_user_devices_with_sessions(&self, user_id: &str) -> Result<Vec<DeviceWithSessions>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let devices = self.get_user_devices(user_id).await?;
        let mut result = Vec::new();

        for device in devices {
            // Get IP addresses for this device
            let ip_addresses: Vec<String> = user_ips::table
                .filter(user_ips::user_id.eq(user_id))
                .filter(user_ips::device_id.eq(&device.device_id))
                .select(user_ips::ip)
                .load(&mut conn)
                .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

            // Get last seen info
            let last_seen_info = user_ips::table
                .filter(user_ips::user_id.eq(user_id))
                .filter(user_ips::device_id.eq(&device.device_id))
                .order_by(user_ips::last_seen_ts.desc())
                .first::<IpRecord>(&mut conn)
                .optional()
                .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

            let last_seen = last_seen_info.as_ref().map(|r| r.last_seen_ts);
            let user_agent = last_seen_info.as_ref().and_then(|r| r.user_agent.clone());

            result.push(DeviceWithSessions {
                device,
                ip_addresses,
                last_seen,
                user_agent,
            });
        }

        Ok(result)
    }

    async fn update_device(&self, user_id: &str, device_id: &str, input: &UpdateDeviceInput) -> Result<Device, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let device = diesel::update(devices::table.find((device_id, user_id)))
            .set((
                input.display_name.is_some()
                    .then(|| devices::display_name.eq(&input.display_name)),
            ))
            .filter(devices::user_id.eq(user_id))
            .filter(devices::device_id.eq(device_id))
            .get_result::<Device>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(device)
    }

    async fn delete_device(&self, user_id: &str, device_id: &str) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        diesel::delete(
            devices::table
                .filter(devices::user_id.eq(user_id))
                .filter(devices::device_id.eq(device_id))
        )
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn delete_devices(&self, user_id: &str, device_ids: &[String]) -> Result<u64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = diesel::delete(
            devices::table
                .filter(devices::user_id.eq(user_id))
                .filter(devices::device_id.eq_any(device_ids))
        )
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count as u64)
    }

    async fn delete_all_user_devices(&self, user_id: &str) -> Result<u64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = diesel::delete(
            devices::table.filter(devices::user_id.eq(user_id))
        )
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count as u64)
    }

    async fn update_device_last_seen(&self, user_id: &str, device_id: &str, ip: &str, user_agent: Option<&str>) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = Utc::now().timestamp_millis();

        diesel::update(devices::table.find((device_id, user_id)))
            .set((
                devices::last_seen_ts.eq(now),
                devices::last_seen_ip.eq(ip),
                devices::last_seen_user_agent.eq(user_agent),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn get_user_device_count(&self, user_id: &str) -> Result<i64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = devices::table
            .filter(devices::user_id.eq(user_id))
            .count()
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count)
    }

    async fn list_devices(&self, filter: &DeviceFilter) -> Result<DeviceListResult, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let limit = filter.limit.unwrap_or(50).min(100);
        let offset = filter.offset.unwrap_or(0);

        let mut query = devices::table.into_boxed();

        if let Some(user_id) = &filter.user_id {
            query = query.filter(devices::user_id.eq(user_id));
        }

        // Get total count - rebuild query to avoid clone issue
        let mut count_query = devices::table.into_boxed();
        if let Some(user_id) = &filter.user_id {
            count_query = count_query.filter(devices::user_id.eq(user_id));
        }

        let total_count = count_query
            .count()
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        let devices = query
            .order_by(devices::created_ts.desc())
            .limit(limit)
            .offset(offset)
            .load::<Device>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(DeviceListResult {
            devices,
            total_count,
            limit,
            offset,
        })
    }
}

// Helper struct for IP record query
#[derive(Queryable)]
#[allow(dead_code)]
struct IpRecord {
    pub user_id: String,
    pub ip: String,
    pub last_seen_ts: i64,
    pub device_id: Option<String>,
    pub user_agent: Option<String>,
}

// Table definitions
use crate::schema::devices;
use crate::schema::user_ips;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_create_device() {}

    #[tokio::test]
    #[ignore]
    async fn test_get_user_devices() {}

    #[tokio::test]
    #[ignore]
    async fn test_delete_devices() {}

    #[tokio::test]
    #[ignore]
    async fn test_batch_device_deletion() {}
}