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
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::process::{Child, Command};
use tokio::io::AsyncBufReadExt;
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

    /// Resolves the path to the Palpo binary.
    ///
    /// Default: looks for `palpo` (or `palpo.exe` on Windows) in the same directory
    /// as the running admin-server executable.
    ///
    /// # Returns
    ///
    /// - Ok(PathBuf) with the resolved binary path
    /// - Err(AdminError::PalpoBinaryNotFound) if the binary does not exist
    fn resolve_palpo_binary() -> Result<PathBuf, AdminError> {
        let exe_dir = std::env::current_exe()
            .map_err(|e| AdminError::ServerStartFailed(format!("Cannot determine executable path: {}", e)))?
            .parent()
            .ok_or_else(|| AdminError::ServerStartFailed("Executable has no parent directory".to_string()))?
            .to_path_buf();

        let binary_name = if cfg!(windows) { "palpo.exe" } else { "palpo" };
        let palpo_path = exe_dir.join(binary_name);

        if palpo_path.exists() {
            info!("Found Palpo binary at: {}", palpo_path.display());
            Ok(palpo_path)
        } else {
            let path_str = palpo_path.display().to_string();
            error!("Palpo binary not found at: {}", path_str);
            Err(AdminError::PalpoBinaryNotFound(path_str))
        }
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

        // Ensure config file exists before starting Palpo process
        let config_path = PathBuf::from("palpo.toml");
        if !config_path.exists() {
            info!("Config file not found, creating default config with localhost settings...");
            ServerConfigAPI::save_config(&config).await?;
            info!("Default config file created successfully");
        }

        // Resolve Palpo binary path
        let palpo_binary = Self::resolve_palpo_binary().map_err(|e| {
            let mut state = self.process_state.lock().unwrap();
            state.status = ServerStatus::Error;
            e
        })?;

        // Get absolute path to config file to ensure Palpo can find it
        let absolute_config_path = std::fs::canonicalize("palpo.toml")
            .map_err(|e| {
                error!("Failed to get absolute path for config file: {}", e);
                let mut state = self.process_state.lock().unwrap();
                state.status = ServerStatus::Error;
                AdminError::ServerStartFailed(format!("Failed to resolve config path: {}", e))
            })?;

        // Start Palpo process
        info!("Starting Palpo server from: {}", palpo_binary.display());
        let mut child = Command::new(&palpo_binary)
            .arg("--config")
            .arg(absolute_config_path)
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

        // Spawn tasks to read stdout and stderr from the child process
        let stdout = child.stdout.take().expect("Child should have stdout");
        let stderr = child.stderr.take().expect("Child should have stderr");

        // Read stdout in a separate task
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout);
            let mut line = String::new();
            loop {
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        if !line.trim().is_empty() {
                            info!("[Palpo stdout] {}", line.trim());
                        }
                        line.clear();
                    }
                    Err(e) => {
                        error!("[Palpo stdout error] {}", e);
                        break;
                    }
                }
            }
        });

        // Read stderr in a separate task
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr);
            let mut line = String::new();
            loop {
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        if !line.trim().is_empty() {
                            error!("[Palpo stderr] {}", line.trim());
                        }
                        line.clear();
                    }
                    Err(e) => {
                        error!("[Palpo stderr error] {}", e);
                        break;
                    }
                }
            }
        });

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
    /// 2. Sends SIGTERM (Unix) or termination signal for graceful shutdown
    /// 3. Waits up to 10 seconds for graceful exit
    /// 4. Falls back to SIGKILL if graceful shutdown times out
    /// 5. Updates the process state
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
            // Step 1: Attempt graceful shutdown with SIGTERM (Unix only)
            #[cfg(unix)]
            {
                use nix::sys::signal::{kill, Signal};
                use nix::unistd::Pid;
                
                if let Some(pid) = child.id() {
                    info!("Sending SIGTERM to Palpo process (PID: {})", pid);
                    match kill(Pid::from_raw(pid as i32), Signal::SIGTERM) {
                        Ok(()) => info!("SIGTERM sent successfully"),
                        Err(e) => warn!("Failed to send SIGTERM: {}, falling back to kill", e),
                    }
                }
            }

            // Step 2: Wait for graceful exit with timeout (10 seconds)
            let timeout_duration = tokio::time::Duration::from_secs(10);
            match tokio::time::timeout(timeout_duration, child.wait()).await {
                Ok(Ok(status)) => {
                    info!("Palpo server stopped gracefully with status: {}", status);
                }
                Ok(Err(e)) => {
                    warn!("Error waiting for process to exit: {}", e);
                }
                Err(_) => {
                    // Step 3: Timeout - force kill
                    warn!("Graceful shutdown timeout after 10s, sending SIGKILL");
                    
                    if let Err(e) = child.kill().await {
                        error!("Failed to kill Palpo process: {}", e);
                        let mut state = self.process_state.lock().unwrap();
                        state.status = ServerStatus::Error;
                        return Err(AdminError::ServerStopFailed(format!(
                            "Failed to kill process after timeout: {}",
                            e
                        )));
                    }

                    // Wait for process to be reaped
                    match child.wait().await {
                        Ok(status) => {
                            info!("Palpo server killed with status: {}", status);
                        }
                        Err(e) => {
                            warn!("Error waiting for killed process: {}", e);
                        }
                    }
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

    #[test]
    fn test_resolve_palpo_binary_returns_path_based_on_exe_dir() {
        // The binary won't exist in test environment, so we expect PalpoBinaryNotFound
        // but the path should be relative to the current executable's directory
        let result = ServerControlAPI::resolve_palpo_binary();
        match result {
            Err(AdminError::PalpoBinaryNotFound(path)) => {
                // Path should end with "palpo" (or "palpo.exe" on Windows)
                let expected_suffix = if cfg!(windows) { "palpo.exe" } else { "palpo" };
                assert!(
                    path.ends_with(expected_suffix),
                    "Expected path to end with '{}', got: {}",
                    expected_suffix,
                    path
                );
                // Path should NOT be the old hardcoded value
                assert_ne!(path, "./target/release/palpo");
            }
            Ok(path) => {
                // If palpo binary happens to exist alongside the test binary, that's fine too
                let expected_suffix = if cfg!(windows) { "palpo.exe" } else { "palpo" };
                assert!(path.to_string_lossy().ends_with(expected_suffix));
            }
            Err(e) => panic!("Unexpected error type: {}", e),
        }
    }
}
