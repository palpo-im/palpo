//! Server control API implementation

use crate::models::{
    ServerStatus, ServerStatusResponse, ConfigReloadResult, ConfigReloadResponse,
    ServerFeature, ServerFeaturesResponse, AdminCommand, CommandResult, 
    CommandExecutionResponse, RestartServerRequest, ShutdownServerRequest,
    AdminNoticeRequest, OperationResponse, ServerMetrics,
    DatabaseStatus, FederationStatus, WebConfigError, AuditAction, AuditTargetType,
};
use crate::utils::audit_logger::AuditLogger;
use std::time::{Duration, SystemTime};
use std::sync::{Arc, RwLock};

#[cfg(target_arch = "wasm32")]
use gloo_timers::future::sleep;
#[cfg(not(target_arch = "wasm32"))]
use tokio::time::sleep;

/// Server control API service
#[derive(Clone)]
pub struct ServerControlAPI {
    audit_logger: AuditLogger,
    // In a real implementation, this would connect to the actual server
    // For now, we'll simulate server state
    server_state: Arc<RwLock<ServerState>>,
}

/// Internal server state for simulation
#[derive(Clone, Debug)]
struct ServerState {
    started_at: SystemTime,
    version: String,
    features: Vec<ServerFeature>,
    config_last_modified: SystemTime,
    is_healthy: bool,
    active_connections: u32,
    memory_usage: u64,
}

impl ServerControlAPI {
    /// Create a new ServerControlAPI instance
    pub fn new(audit_logger: AuditLogger) -> Self {
        let server_state = Arc::new(RwLock::new(ServerState {
            started_at: SystemTime::now(),
            version: "1.0.0".to_string(),
            features: Self::default_features(),
            config_last_modified: SystemTime::now(),
            is_healthy: true,
            active_connections: 42,
            memory_usage: 256 * 1024 * 1024, // 256MB
        }));

        Self {
            audit_logger,
            server_state,
        }
    }

    /// Get current server status
    pub async fn get_server_status(&self, admin_user: &str) -> Result<ServerStatusResponse, WebConfigError> {
        // Check permissions
        if !self.has_server_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for server management"));
        }

        let state = self.server_state.read()
            .map_err(|_| WebConfigError::internal("Failed to read server state"))?;

        let uptime = SystemTime::now()
            .duration_since(state.started_at)
            .unwrap_or(Duration::from_secs(0));

        let status = ServerStatus {
            uptime,
            version: state.version.clone(),
            features: state.features.iter().map(|f| f.name.clone()).collect(),
            active_connections: state.active_connections,
            memory_usage: state.memory_usage,
            config_last_modified: state.config_last_modified,
            hot_reload_supported: true,
            is_healthy: state.is_healthy,
            database_status: DatabaseStatus {
                connected: true,
                pool_size: 10,
                active_connections: 3,
                idle_connections: 7,
                last_error: None,
            },
            federation_status: FederationStatus {
                enabled: true,
                reachable_servers: 15,
                unreachable_servers: 2,
                pending_transactions: 5,
                last_federation_error: None,
            },
        };

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigReload,
            AuditTargetType::Server,
            "server_status",
            Some(serde_json::json!({
                "uptime_seconds": uptime.as_secs(),
                "version": state.version,
                "is_healthy": state.is_healthy
            })),
            "Retrieved server status",
        ).await;

        Ok(ServerStatusResponse {
            success: true,
            status: Some(status),
            error: None,
        })
    }
    /// Reload server configuration
    pub async fn reload_config(&self, admin_user: &str) -> Result<ConfigReloadResponse, WebConfigError> {
        // Check permissions
        if !self.has_server_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for server management"));
        }

        let start_time = SystemTime::now();
        
        // Simulate config reload process
        sleep(Duration::from_millis(500)).await;
        
        let reload_time = SystemTime::now()
            .duration_since(start_time)
            .unwrap_or(Duration::from_secs(0));

        // Update config last modified time
        {
            let mut state = self.server_state.write()
                .map_err(|_| WebConfigError::internal("Failed to write server state"))?;
            state.config_last_modified = SystemTime::now();
        }

        let result = ConfigReloadResult {
            success: true,
            errors: Vec::new(),
            warnings: vec!["Some deprecated configuration options detected".to_string()],
            hot_reload_supported: true,
            restart_required: false,
            affected_services: vec!["HTTP Server".to_string(), "Federation".to_string()],
            reload_time,
        };

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate,
            AuditTargetType::Server,
            "config_reload",
            Some(serde_json::json!({
                "reload_time_ms": reload_time.as_millis(),
                "warnings_count": result.warnings.len(),
                "affected_services": result.affected_services
            })),
            "Reloaded server configuration",
        ).await;

        Ok(ConfigReloadResponse {
            success: true,
            result: Some(result),
            error: None,
        })
    }

    /// Restart the server
    pub async fn restart_server(&self, request: RestartServerRequest, admin_user: &str) -> Result<OperationResponse, WebConfigError> {
        // Check permissions
        if !self.has_server_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for server management"));
        }

        // Log the action first (before potential restart)
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate, // Using existing action since ServerRestart doesn't exist
            AuditTargetType::Server,
            "server_restart",
            Some(serde_json::json!({
                "force": request.force,
                "graceful_timeout": request.graceful_timeout_seconds,
                "reason": request.reason
            })),
            &format!("Initiated server restart (force: {})", request.force),
        ).await;

        // In a real implementation, this would trigger an actual server restart
        // For simulation, we'll just update the started_at time
        {
            let mut state = self.server_state.write()
                .map_err(|_| WebConfigError::internal("Failed to write server state"))?;
            state.started_at = SystemTime::now();
        }

        // Simulate restart delay
        if !request.force {
            sleep(Duration::from_millis(1000)).await;
        }

        Ok(OperationResponse {
            success: true,
            message: Some("Server restart initiated successfully".to_string()),
            error: None,
        })
    }
    /// Shutdown the server
    pub async fn shutdown_server(&self, request: ShutdownServerRequest, admin_user: &str) -> Result<OperationResponse, WebConfigError> {
        // Check permissions
        if !self.has_server_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for server management"));
        }

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate, // Using existing action since ServerShutdown doesn't exist
            AuditTargetType::Server,
            "server_shutdown",
            Some(serde_json::json!({
                "graceful": request.graceful,
                "timeout": request.timeout_seconds,
                "reason": request.reason
            })),
            &format!("Initiated server shutdown (graceful: {})", request.graceful),
        ).await;

        // In a real implementation, this would trigger an actual server shutdown
        // For simulation, we'll just mark the server as unhealthy
        {
            let mut state = self.server_state.write()
                .map_err(|_| WebConfigError::internal("Failed to write server state"))?;
            state.is_healthy = false;
        }

        Ok(OperationResponse {
            success: true,
            message: Some("Server shutdown initiated successfully".to_string()),
            error: None,
        })
    }

    /// Get server features
    pub async fn get_server_features(&self, admin_user: &str) -> Result<ServerFeaturesResponse, WebConfigError> {
        // Check permissions
        if !self.has_server_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for server management"));
        }

        let state = self.server_state.read()
            .map_err(|_| WebConfigError::internal("Failed to read server state"))?;

        let features = state.features.clone();

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigReload,
            AuditTargetType::Server,
            "server_features",
            Some(serde_json::json!({
                "feature_count": features.len(),
                "enabled_features": features.iter().filter(|f| f.enabled).count()
            })),
            "Retrieved server features",
        ).await;

        Ok(ServerFeaturesResponse {
            success: true,
            features,
            error: None,
        })
    }
    /// Send admin notice to management rooms
    pub async fn send_admin_notice(&self, request: AdminNoticeRequest, admin_user: &str) -> Result<OperationResponse, WebConfigError> {
        // Check permissions
        if !self.has_server_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for server management"));
        }

        // Validate message
        if request.message.trim().is_empty() {
            return Ok(OperationResponse {
                success: false,
                message: None,
                error: Some("Message cannot be empty".to_string()),
            });
        }

        if request.message.len() > 4000 {
            return Ok(OperationResponse {
                success: false,
                message: None,
                error: Some("Message too long (max 4000 characters)".to_string()),
            });
        }

        // In a real implementation, this would send the message to admin rooms
        // For simulation, we'll just log the action
        
        let formatted_message = format!(
            "{} **{}**: {}",
            request.notice_type.emoji(),
            request.notice_type.description(),
            request.message
        );

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate, // Using existing action since AdminNotice doesn't exist
            AuditTargetType::Server,
            "admin_notice",
            Some(serde_json::json!({
                "notice_type": request.notice_type,
                "message_length": request.message.len(),
                "target_rooms": request.target_rooms,
                "urgent": request.urgent,
                "formatted_message": formatted_message
            })),
            &format!("Sent admin notice: {}", request.notice_type.description()),
        ).await;

        Ok(OperationResponse {
            success: true,
            message: Some("Admin notice sent successfully".to_string()),
            error: None,
        })
    }

    /// Execute admin command
    pub async fn execute_admin_command(&self, command: AdminCommand, admin_user: &str) -> Result<CommandExecutionResponse, WebConfigError> {
        // Check permissions
        if !self.has_server_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for server management"));
        }

        // Validate command
        if command.command.trim().is_empty() {
            return Ok(CommandExecutionResponse {
                success: false,
                result: None,
                error: Some("Command cannot be empty".to_string()),
            });
        }

        // Security check - only allow safe commands in simulation
        let safe_commands = ["echo", "date", "whoami", "pwd", "ls", "ps"];
        let cmd_name = command.command.split_whitespace().next().unwrap_or("");
        
        if !safe_commands.contains(&cmd_name) {
            return Ok(CommandExecutionResponse {
                success: false,
                result: None,
                error: Some(format!("Command '{}' is not allowed for security reasons", cmd_name)),
            });
        }

        let started_at = SystemTime::now();
        
        // Execute the command (simulated in this demo implementation)
        // Note: timeout_seconds parameter is ignored as this is a frontend-only demo
        let result = self.execute_system_command(&command).await;

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate, // Using existing action since CommandExecution doesn't exist
            AuditTargetType::Server,
            "admin_command",
            Some(serde_json::json!({
                "command": command.command,
                "args": command.args,
                "success": result.success,
                "execution_time_ms": result.execution_time.as_millis(),
                "exit_code": result.exit_code,
                "require_confirmation": command.require_confirmation
            })),
            &format!("Executed admin command: {}", command.command),
        ).await;

        Ok(CommandExecutionResponse {
            success: result.success,
            result: Some(result),
            error: None,
        })
    }
    /// Execute a system command (internal helper)
    async fn execute_system_command(&self, command: &AdminCommand) -> CommandResult {
        let started_at = SystemTime::now();
        
        // For simulation, we'll create mock responses for safe commands
        let (success, output, exit_code) = match command.command.as_str() {
            "echo" => {
                let output = if command.args.is_empty() {
                    String::new()
                } else {
                    command.args.join(" ")
                };
                (true, output, Some(0))
            },
            "date" => (true, "Mon Jan  1 12:00:00 UTC 2024".to_string(), Some(0)),
            "whoami" => (true, "palpo".to_string(), Some(0)),
            "pwd" => (true, "/opt/palpo".to_string(), Some(0)),
            "ls" => (true, "config.toml\nlogs/\ndata/\nstatic/".to_string(), Some(0)),
            "ps" => (true, "  PID TTY          TIME CMD\n 1234 ?        00:00:05 palpo-server".to_string(), Some(0)),
            _ => (false, String::new(), Some(127)), // Command not found
        };

        let execution_time = SystemTime::now()
            .duration_since(started_at)
            .unwrap_or(Duration::from_millis(100));

        CommandResult {
            success,
            output,
            error: if success { None } else { Some("Command failed".to_string()) },
            execution_time,
            exit_code,
            command: command.command.clone(),
            started_at,
        }
    }

    /// Get server metrics
    pub async fn get_server_metrics(&self, admin_user: &str) -> Result<ServerMetrics, WebConfigError> {
        // Check permissions
        if !self.has_server_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for server management"));
        }

        let state = self.server_state.read()
            .map_err(|_| WebConfigError::internal("Failed to read server state"))?;

        // Simulate metrics collection
        let metrics = ServerMetrics {
            cpu_usage_percent: 15.5,
            memory_usage_bytes: state.memory_usage,
            memory_total_bytes: 1024 * 1024 * 1024, // 1GB
            disk_usage_bytes: 2 * 1024 * 1024 * 1024, // 2GB
            disk_total_bytes: 10 * 1024 * 1024 * 1024, // 10GB
            network_rx_bytes: 1024 * 1024 * 100, // 100MB
            network_tx_bytes: 1024 * 1024 * 80,  // 80MB
            active_rooms: 150,
            active_users: 1200,
            events_per_second: 5.2,
        };

        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigReload,
            AuditTargetType::Server,
            "server_metrics",
            Some(serde_json::json!({
                "cpu_usage": metrics.cpu_usage_percent,
                "memory_usage_mb": metrics.memory_usage_bytes / (1024 * 1024),
                "active_users": metrics.active_users,
                "active_rooms": metrics.active_rooms
            })),
            "Retrieved server metrics",
        ).await;

        Ok(metrics)
    }
    /// Check if the admin user has server management permissions
    async fn has_server_management_permission(&self, _admin_user: &str) -> Result<bool, WebConfigError> {
        // In a real implementation, this would check the admin user's permissions
        // For now, we'll assume all admin users have server management permissions
        Ok(true)
    }

    /// Get default server features
    fn default_features() -> Vec<ServerFeature> {
        vec![
            ServerFeature {
                name: "Federation".to_string(),
                enabled: true,
                description: "Matrix federation support".to_string(),
                requires_restart: false,
                config_key: Some("federation.enabled".to_string()),
            },
            ServerFeature {
                name: "Media Repository".to_string(),
                enabled: true,
                description: "Media file storage and serving".to_string(),
                requires_restart: false,
                config_key: Some("media.enabled".to_string()),
            },
            ServerFeature {
                name: "Push Notifications".to_string(),
                enabled: true,
                description: "Push notification gateway".to_string(),
                requires_restart: false,
                config_key: Some("push.enabled".to_string()),
            },
            ServerFeature {
                name: "Registration".to_string(),
                enabled: false,
                description: "Open user registration".to_string(),
                requires_restart: false,
                config_key: Some("registration.enabled".to_string()),
            },
            ServerFeature {
                name: "Metrics".to_string(),
                enabled: true,
                description: "Prometheus metrics endpoint".to_string(),
                requires_restart: true,
                config_key: Some("metrics.enabled".to_string()),
            },
            ServerFeature {
                name: "Admin API".to_string(),
                enabled: true,
                description: "Administrative API endpoints".to_string(),
                requires_restart: true,
                config_key: Some("admin_api.enabled".to_string()),
            },
            ServerFeature {
                name: "Rate Limiting".to_string(),
                enabled: true,
                description: "Request rate limiting".to_string(),
                requires_restart: false,
                config_key: Some("rate_limiting.enabled".to_string()),
            },
            ServerFeature {
                name: "TURN Server".to_string(),
                enabled: false,
                description: "Integrated TURN server for VoIP".to_string(),
                requires_restart: true,
                config_key: Some("turn.enabled".to_string()),
            },
        ]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NoticeType;
    use crate::utils::audit_logger::AuditLogger;

    fn create_test_api() -> ServerControlAPI {
        let audit_logger = AuditLogger::new(1000);
        ServerControlAPI::new(audit_logger)
    }

    #[tokio::test]
    async fn test_get_server_status() {
        let api = create_test_api();
        
        let response = api.get_server_status("admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.status.is_some());
        
        let status = response.status.unwrap();
        assert_eq!(status.version, "1.0.0");
        assert!(status.is_healthy());
        assert!(status.hot_reload_supported);
        assert!(status.database_status.connected);
        assert!(status.federation_status.enabled);
    }

    #[tokio::test]
    async fn test_reload_config() {
        let api = create_test_api();
        
        let response = api.reload_config("admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.result.is_some());
        
        let result = response.result.unwrap();
        assert!(result.success);
        assert!(result.hot_reload_supported);
        assert!(!result.restart_required);
        assert!(!result.affected_services.is_empty());
    }

    #[tokio::test]
    async fn test_restart_server() {
        let api = create_test_api();
        let request = RestartServerRequest {
            force: false,
            graceful_timeout_seconds: Some(30),
            reason: Some("Configuration update".to_string()),
        };
        
        let response = api.restart_server(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.message.is_some());
    }

    #[tokio::test]
    async fn test_get_server_features() {
        let api = create_test_api();
        
        let response = api.get_server_features("admin").await.unwrap();
        
        assert!(response.success);
        assert!(!response.features.is_empty());
        
        // Check that we have expected features
        let feature_names: Vec<&String> = response.features.iter().map(|f| &f.name).collect();
        assert!(feature_names.contains(&&"Federation".to_string()));
        assert!(feature_names.contains(&&"Media Repository".to_string()));
        assert!(feature_names.contains(&&"Admin API".to_string()));
    }

    #[tokio::test]
    async fn test_send_admin_notice() {
        let api = create_test_api();
        let request = AdminNoticeRequest {
            message: "Server maintenance scheduled for tonight".to_string(),
            notice_type: NoticeType::Maintenance,
            target_rooms: None,
            urgent: false,
        };
        
        let response = api.send_admin_notice(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.message.is_some());
    }

    #[tokio::test]
    async fn test_execute_admin_command_safe() {
        let api = create_test_api();
        let command = AdminCommand::new("echo".to_string())
            .with_arg("Hello World".to_string());
        
        let response = api.execute_admin_command(command, "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.result.is_some());
        
        let result = response.result.unwrap();
        assert!(result.success);
        assert_eq!(result.output, "Hello World");
        assert_eq!(result.exit_code, Some(0));
    }

    #[tokio::test]
    async fn test_execute_admin_command_unsafe() {
        let api = create_test_api();
        let command = AdminCommand::new("rm".to_string())
            .with_arg("-rf".to_string())
            .with_arg("/".to_string());
        
        let response = api.execute_admin_command(command, "admin").await.unwrap();
        
        assert!(!response.success);
        assert!(response.error.is_some()); // Error should be in the error field
        assert!(response.result.is_none());
    }

    #[tokio::test]
    async fn test_get_server_metrics() {
        let api = create_test_api();
        
        let metrics = api.get_server_metrics("admin").await.unwrap();
        
        assert!(metrics.cpu_usage_percent >= 0.0);
        assert!(metrics.memory_usage_bytes > 0);
        assert!(metrics.active_users > 0);
        assert!(metrics.active_rooms > 0);
    }

    #[tokio::test]
    async fn test_server_status_uptime_string() {
        let status = ServerStatus {
            uptime: Duration::from_secs(90061), // 1 day, 1 hour, 1 minute, 1 second
            version: "1.0.0".to_string(),
            features: vec![],
            active_connections: 10,
            memory_usage: 1024,
            config_last_modified: SystemTime::now(),
            hot_reload_supported: true,
            is_healthy: true,
            database_status: DatabaseStatus {
                connected: true,
                pool_size: 10,
                active_connections: 5,
                idle_connections: 5,
                last_error: None,
            },
            federation_status: FederationStatus {
                enabled: true,
                reachable_servers: 10,
                unreachable_servers: 0,
                pending_transactions: 0,
                last_federation_error: None,
            },
        };
        
        let uptime_str = status.uptime_string();
        assert_eq!(uptime_str, "1d 1h 1m 1s");
    }
}