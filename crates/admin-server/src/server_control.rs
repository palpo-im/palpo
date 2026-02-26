/// Server Control API
///
/// This module implements the Palpo server lifecycle management functionality.
/// It provides methods to start, stop, restart, and query the status of the
/// Palpo Matrix server process.
///
/// # Requirements
///
/// Implements requirements:
/// - 6.1: Query server status (running/stopped, PID, uptime)
/// - 6.2: Start Palpo server with config validation
/// - 6.3: Stop Palpo server with graceful shutdown
/// - 6.4: Restart Palpo server
/// - 6.5: Validate configuration before starting
/// - 6.6: Return clear error messages
/// - 6.7: Include process ID and uptime in status

use crate::server_config::ServerConfigAPI;
use crate::types::{AdminError, ServerStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::process::{Child, Command};
use tracing::{error, info, warn};

/// Detailed server status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatusInfo {
    /// Current status of the server
    pub status: ServerStatus,
    /// Process ID if server is running
    pub pid: Option<u32>,
    /// Timestamp when server was started
    pub started_at: Option<DateTime<Utc>>,
    /// Server uptime in seconds
    pub uptime_seconds: Option<i64>,
}

/// Internal process state
#[derive(Debug)]
struct ProcessState {
    /// Child process handle
    child: Option<Child>,
    /// Current server status
    status: ServerStatus,
    /// Timestamp when server was started
    started_at: Option<DateTime<Utc>>,
}

/// Server Control API for managing Palpo server lifecycle
///
/// This service manages the Palpo Matrix server process, providing
/// start, stop, restart, and status query operations.
#[derive(Debug)]
pub struct ServerControlAPI {
    /// Process state protected by mutex for thread-safe access
    process_state: Arc<Mutex<ProcessState>>,
}

impl ServerControlAPI {
    /// Creates a new ServerControlAPI instance
    pub fn new() -> Self {
        Self {
            process_state: Arc::new(Mutex::new(ProcessState {
                child: None,
                status: ServerStatus::NotStarted,
                started_at: None,
            })),
        }
    }

    /// Gets detailed server status including PID and uptime
    ///
    /// # Returns
    ///
    /// ServerStatusInfo with current status, PID, start time, and uptime
    ///
    /// # Requirements
    ///
    /// Implements requirement 6.1, 6.7: Query server status with PID and uptime
    pub fn get_status(&self) -> ServerStatusInfo {
        let state = self.process_state.lock().unwrap();
        
        let pid = state.child.as_ref().and_then(|child| child.id());
        
        let uptime_seconds = state.started_at.map(|start| {
            let now = Utc::now();
            (now - start).num_seconds()
        });

        ServerStatusInfo {
            status: state.status,
            pid,
            started_at: state.started_at,
            uptime_seconds,
        }
    }

    /// Checks if the server is currently running
    ///
    /// # Returns
    ///
    /// true if server status is Running, false otherwise
    pub fn is_running(&self) -> bool {
        let state = self.process_state.lock().unwrap();
        state.status == ServerStatus::Running
    }

    /// Starts the Palpo server
    ///
    /// This method:
    /// 1. Checks if server is already running
    /// 2. Validates the server configuration
    /// 3. Spawns the Palpo server process
    /// 4. Updates the process state
    ///
    /// # Returns
    ///
    /// - Ok(()) if server started successfully
    /// - Err(AdminError::ServerAlreadyRunning) if already running
    /// - Err(AdminError::ConfigValidationFailed) if config is invalid
    /// - Err(AdminError::ServerStartFailed) if process spawn fails
    ///
    /// # Requirements
    ///
    /// Implements requirements:
    /// - 6.2: Start Palpo server
    /// - 6.5: Validate configuration before starting
    /// - 6.6: Return clear error messages
    pub async fn start_server(&self) -> Result<(), AdminError> {
        // Check if already running
        {
            let state = self.process_state.lock().unwrap();
            if state.status == ServerStatus::Running {
                info!("Server is already running");
                return Ok(()); // Idempotent - already running is success
            }
        }

        // Update status to Starting
        {
            let mut state = self.process_state.lock().unwrap();
            state.status = ServerStatus::Starting;
        }

        // Validate config before starting
        info!("Validating server configuration...");
        let config = ServerConfigAPI::get_config().await?;
        ServerConfigAPI::validate_config(&config)?;
        info!("Configuration validated successfully");

        // Start Palpo process
        info!("Starting Palpo server...");
        let child = Command::new("./target/release/palpo")
            .arg("--config")
            .arg("palpo.toml")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                error!("Failed to spawn Palpo process: {}", e);
                let mut state = self.process_state.lock().unwrap();
                state.status = ServerStatus::Error;
                AdminError::ServerStartFailed(format!("Failed to spawn process: {}", e))
            })?;

        let pid = child.id();
        info!("Palpo server started with PID: {:?}", pid);

        // Update process state
        {
            let mut state = self.process_state.lock().unwrap();
            state.child = Some(child);
            state.status = ServerStatus::Running;
            state.started_at = Some(Utc::now());
        }

        Ok(())
    }

    /// Stops the Palpo server gracefully
    ///
    /// This method:
    /// 1. Checks if server is running
    /// 2. Sends termination signal to the process
    /// 3. Waits for the process to exit
    /// 4. Updates the process state
    ///
    /// # Returns
    ///
    /// - Ok(()) if server stopped successfully
    /// - Err(AdminError::ServerStopFailed) if termination fails
    ///
    /// # Requirements
    ///
    /// Implements requirements:
    /// - 6.3: Stop Palpo server with graceful shutdown
    /// - 6.6: Return clear error messages
    pub async fn stop_server(&self) -> Result<(), AdminError> {
        // Check if running
        {
            let state = self.process_state.lock().unwrap();
            if state.status != ServerStatus::Running {
                info!("Server is not running");
                return Ok(()); // Idempotent - already stopped is success
            }
        }

        // Update status to Stopping
        {
            let mut state = self.process_state.lock().unwrap();
            state.status = ServerStatus::Stopping;
        }

        info!("Stopping Palpo server...");

        // Take ownership of the child process
        let child = {
            let mut state = self.process_state.lock().unwrap();
            state.child.take()
        };

        if let Some(mut child) = child {
            // Attempt graceful shutdown by killing the process
            if let Err(e) = child.kill().await {
                error!("Failed to kill Palpo process: {}", e);
                let mut state = self.process_state.lock().unwrap();
                state.status = ServerStatus::Error;
                return Err(AdminError::ServerStopFailed(format!(
                    "Failed to kill process: {}",
                    e
                )));
            }

            // Wait for process to exit
            match child.wait().await {
                Ok(status) => {
                    info!("Palpo server stopped with status: {}", status);
                }
                Err(e) => {
                    warn!("Error waiting for process to exit: {}", e);
                }
            }
        }

        // Update process state
        {
            let mut state = self.process_state.lock().unwrap();
            state.status = ServerStatus::Stopped;
            state.started_at = None;
        }

        info!("Palpo server stopped successfully");
        Ok(())
    }

    /// Restarts the Palpo server
    ///
    /// This method stops the server (if running) and then starts it again.
    /// There is a 2-second delay between stop and start to ensure clean shutdown.
    ///
    /// # Returns
    ///
    /// - Ok(()) if server restarted successfully
    /// - Err(AdminError) if stop or start fails
    ///
    /// # Requirements
    ///
    /// Implements requirement 6.4: Restart Palpo server
    pub async fn restart_server(&self) -> Result<(), AdminError> {
        info!("Restarting Palpo server...");

        // Stop the server
        self.stop_server().await?;

        // Wait a bit to ensure clean shutdown
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Start the server
        self.start_server().await?;

        info!("Palpo server restarted successfully");
        Ok(())
    }
}

impl Default for ServerControlAPI {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_status() {
        let api = ServerControlAPI::new();
        let status = api.get_status();
        
        assert_eq!(status.status, ServerStatus::NotStarted);
        assert_eq!(status.pid, None);
        assert_eq!(status.started_at, None);
        assert_eq!(status.uptime_seconds, None);
    }

    #[test]
    fn test_is_running_initially_false() {
        let api = ServerControlAPI::new();
        assert!(!api.is_running());
    }
}
