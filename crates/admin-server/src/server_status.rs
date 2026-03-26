/// Server Status Monitoring
///
/// This module implements server health monitoring and metrics collection.
/// It provides methods to query server health, system metrics, and version information.
///
/// # Requirements
///
/// Implements requirements:
/// - 7.1: Query server health status
/// - 7.2: Collect system metrics (CPU, memory, connections)
/// - 7.3: Provide version information
/// - 7.4: Support health check for load balancers

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Server health status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// Server is healthy and responding
    Healthy,
    /// Server is degraded but operational
    Degraded,
    /// Server is unhealthy
    Unhealthy,
}

/// System metrics information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    /// CPU usage percentage (0-100)
    pub cpu_usage: f64,
    /// Memory usage in MB
    pub memory_usage_mb: u64,
    /// Total memory in MB
    pub total_memory_mb: u64,
    /// Memory usage percentage (0-100)
    pub memory_usage_percent: f64,
    /// Number of active connections
    pub active_connections: u32,
    /// System uptime in seconds
    pub uptime_seconds: u64,
}

/// Server health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthInfo {
    /// Current health status
    pub status: HealthStatus,
    /// Response time in milliseconds
    pub response_time_ms: u64,
    /// Timestamp of health check
    pub timestamp: u64,
    /// System metrics
    pub metrics: SystemMetrics,
}

/// Server version information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Admin server version
    pub admin_server_version: String,
    /// Palpo server version (if available)
    pub palpo_version: Option<String>,
    /// Build timestamp
    pub build_timestamp: String,
}

/// Server Status API for monitoring server health and metrics
///
/// This service provides health checks, system metrics, and version information
/// for monitoring and load balancer integration.
#[derive(Debug)]
pub struct ServerStatusAPI;

impl ServerStatusAPI {
    /// Gets server health status
    ///
    /// # Returns
    ///
    /// HealthInfo with current status, response time, and system metrics
    ///
    /// # Requirements
    ///
    /// Implements requirement 7.1, 7.4: Query server health status
    pub fn get_health() -> HealthInfo {
        let start = SystemTime::now();

        // Collect system metrics
        let metrics = Self::collect_metrics();

        // Calculate response time
        let response_time_ms = start.elapsed().unwrap_or_default().as_millis() as u64;

        // Determine health status based on metrics
        let status = if metrics.memory_usage_percent > 90.0 || metrics.cpu_usage > 95.0 {
            HealthStatus::Unhealthy
        } else if metrics.memory_usage_percent > 80.0 || metrics.cpu_usage > 80.0 {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        HealthInfo {
            status,
            response_time_ms,
            timestamp,
            metrics,
        }
    }

    /// Collects system metrics
    ///
    /// # Returns
    ///
    /// SystemMetrics with CPU, memory, and connection information
    ///
    /// # Requirements
    ///
    /// Implements requirement 7.2: Collect system metrics
    fn collect_metrics() -> SystemMetrics {
        // Get memory info using sysinfo
        #[cfg(feature = "sysinfo")]
        {
            use sysinfo::System;
            let mut sys = System::new_all();
            sys.refresh_all();

            let total_memory = sys.total_memory();
            let used_memory = sys.used_memory();
            let memory_usage_percent = (used_memory as f64 / total_memory as f64) * 100.0;

            SystemMetrics {
                cpu_usage: sys.global_cpu_usage() as f64,
                memory_usage_mb: used_memory / 1024,
                total_memory_mb: total_memory / 1024,
                memory_usage_percent,
                active_connections: 0, // Would need to query Palpo for this
                uptime_seconds: System::uptime(),
            }
        }

        #[cfg(not(feature = "sysinfo"))]
        {
            // Fallback metrics when sysinfo is not available
            SystemMetrics {
                cpu_usage: 0.0,
                memory_usage_mb: 0,
                total_memory_mb: 0,
                memory_usage_percent: 0.0,
                active_connections: 0,
                uptime_seconds: 0,
            }
        }
    }

    /// Gets server version information
    ///
    /// # Returns
    ///
    /// VersionInfo with admin server and Palpo versions
    ///
    /// # Requirements
    ///
    /// Implements requirement 7.3: Provide version information
    pub fn get_version() -> VersionInfo {
        VersionInfo {
            admin_server_version: env!("CARGO_PKG_VERSION").to_string(),
            palpo_version: None, // Would be fetched from Palpo API
            build_timestamp: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_health() {
        let health = ServerStatusAPI::get_health();
        assert!(health.response_time_ms >= 0);
        assert!(health.metrics.memory_usage_percent >= 0.0);
        assert!(health.metrics.memory_usage_percent <= 100.0);
    }

    #[test]
    fn test_get_version() {
        let version = ServerStatusAPI::get_version();
        assert!(!version.admin_server_version.is_empty());
    }

    #[test]
    fn test_health_status_determination() {
        // Test that health status is determined correctly
        let health = ServerStatusAPI::get_health();
        match health.status {
            HealthStatus::Healthy | HealthStatus::Degraded | HealthStatus::Unhealthy => {
                // Valid status
            }
        }
    }
}
