/// Server Status HTTP Handlers
///
/// This module implements HTTP handlers for server health monitoring endpoints.

use crate::server_status::{ServerStatusAPI, HealthInfo, VersionInfo};
use salvo::prelude::*;
use serde_json::json;
use tracing::info;

/// Health check endpoint
///
/// Returns server health status for monitoring and load balancer integration.
///
/// # Response
///
/// ```json
/// {
///   "status": "healthy",
///   "response_time_ms": 5,
///   "timestamp": 1234567890,
///   "metrics": {
///     "cpu_usage": 25.5,
///     "memory_usage_mb": 512,
///     "total_memory_mb": 8192,
///     "memory_usage_percent": 6.25,
///     "active_connections": 42,
///     "uptime_seconds": 86400
///   }
/// }
/// ```
#[handler]
pub async fn get_health() -> Result<Json<HealthInfo>, StatusError> {
    info!("Health check requested");
    let health = ServerStatusAPI::get_health();
    Ok(Json(health))
}

/// Metrics endpoint
///
/// Returns detailed system metrics for monitoring.
///
/// # Response
///
/// ```json
/// {
///   "cpu_usage": 25.5,
///   "memory_usage_mb": 512,
///   "total_memory_mb": 8192,
///   "memory_usage_percent": 6.25,
///   "active_connections": 42,
///   "uptime_seconds": 86400
/// }
/// ```
#[handler]
pub async fn get_metrics() -> Result<Json<serde_json::Value>, StatusError> {
    info!("Metrics requested");
    let health = ServerStatusAPI::get_health();
    Ok(Json(json!(health.metrics)))
}

/// Version endpoint
///
/// Returns server version information.
///
/// # Response
///
/// ```json
/// {
///   "admin_server_version": "0.2.1",
///   "palpo_version": null,
///   "build_timestamp": "0.2.1"
/// }
/// ```
#[handler]
pub async fn get_version() -> Result<Json<VersionInfo>, StatusError> {
    info!("Version requested");
    let version = ServerStatusAPI::get_version();
    Ok(Json(version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_health_handler() {
        let health = ServerStatusAPI::get_health();
        assert!(health.response_time_ms >= 0);
    }

    #[tokio::test]
    async fn test_get_version_handler() {
        let version = ServerStatusAPI::get_version();
        assert!(!version.admin_server_version.is_empty());
    }
}
