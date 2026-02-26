/// Server Control HTTP Handlers
///
/// This module implements the REST API endpoints for Palpo server lifecycle control.
/// These endpoints allow Web UI admins to start, stop, restart, and query the status
/// of the Palpo Matrix server.
///
/// # Endpoints
///
/// - `GET /api/v1/admin/server/status` - Get server status with PID and uptime
/// - `POST /api/v1/admin/server/start` - Start the Palpo server
/// - `POST /api/v1/admin/server/stop` - Stop the Palpo server
/// - `POST /api/v1/admin/server/restart` - Restart the Palpo server
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

use salvo::prelude::*;
use serde::Serialize;
use std::sync::{Arc, OnceLock};

use crate::server_control::ServerControlAPI;
use crate::types::AdminError;

/// Shared application state for server control handlers
#[derive(Clone, Debug)]
pub struct ServerControlState {
    pub server_control: Arc<ServerControlAPI>,
}

/// Global server control state
static SERVER_CONTROL_STATE: OnceLock<ServerControlState> = OnceLock::new();

/// Initialize the global server control state
pub fn init_server_control_state(state: ServerControlState) {
    SERVER_CONTROL_STATE
        .set(state)
        .expect("Server control state already initialized");
}

/// Get the global server control state
pub fn get_server_control_state() -> &'static ServerControlState {
    SERVER_CONTROL_STATE
        .get()
        .expect("Server control state not initialized")
}

// ===== Response Types =====

/// Response for successful operations
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    /// Success message
    pub message: String,
}

/// Standard error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
}

// ===== Handler Functions =====

/// GET /api/v1/admin/server/status
///
/// Gets the current status of the Palpo server including:
/// - Current status (NotStarted, Starting, Running, Stopping, Stopped, Error)
/// - Process ID (if running)
/// - Start timestamp (if running)
/// - Uptime in seconds (if running)
///
/// # Response
///
/// - 200 OK: Returns ServerStatusInfo
///
/// # Example Response
///
/// ```json
/// {
///   "status": "Running",
///   "pid": 12345,
///   "started_at": "2024-01-15T10:30:00Z",
///   "uptime_seconds": 3600
/// }
/// ```
///
/// # Requirements
///
/// Implements requirements:
/// - 6.1: Query server status
/// - 6.7: Include process ID and uptime
#[handler]
pub async fn get_status(res: &mut Response) {
    let state = get_server_control_state();
    let status = state.server_control.get_status();
    res.render(Json(status));
}

/// POST /api/v1/admin/server/start
///
/// Starts the Palpo server.
///
/// This endpoint:
/// 1. Checks if server is already running (idempotent)
/// 2. Validates the server configuration
/// 3. Spawns the Palpo server process
/// 4. Returns success or error
///
/// # Response
///
/// - 200 OK: Server started successfully (or already running)
/// - 400 Bad Request: Configuration validation failed
/// - 500 Internal Server Error: Failed to start server
///
/// # Example Response
///
/// ```json
/// {
///   "message": "Server started successfully"
/// }
/// ```
///
/// # Requirements
///
/// Implements requirements:
/// - 6.2: Start Palpo server
/// - 6.5: Validate configuration before starting
/// - 6.6: Return clear error messages
#[handler]
pub async fn start_server(res: &mut Response) {
    let state = get_server_control_state();

    match state.server_control.start_server().await {
        Ok(()) => {
            tracing::info!("Server start request successful");
            res.render(Json(SuccessResponse {
                message: "Server started successfully".to_string(),
            }));
        }
        Err(AdminError::ConfigValidationFailed(msg)) => {
            tracing::warn!("Configuration validation failed: {}", msg);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: format!("Configuration validation failed: {}", msg),
            }));
        }
        Err(AdminError::InvalidDatabaseUrl) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid database URL in configuration".to_string(),
            }));
        }
        Err(AdminError::InvalidServerName) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid server name in configuration".to_string(),
            }));
        }
        Err(AdminError::InvalidPort) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "Invalid port in configuration".to_string(),
            }));
        }
        Err(AdminError::TLSCertificateNotFound) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "TLS certificate file not found".to_string(),
            }));
        }
        Err(AdminError::TLSPrivateKeyNotFound) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: "TLS private key file not found".to_string(),
            }));
        }
        Err(AdminError::ServerStartFailed(msg)) => {
            tracing::error!("Failed to start server: {}", msg);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Failed to start server: {}", msg),
            }));
        }
        Err(e) => {
            tracing::error!("Unexpected error starting server: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Failed to start server: {}", e),
            }));
        }
    }
}

/// POST /api/v1/admin/server/stop
///
/// Stops the Palpo server gracefully.
///
/// This endpoint:
/// 1. Checks if server is running (idempotent)
/// 2. Sends termination signal to the process
/// 3. Waits for the process to exit
/// 4. Returns success or error
///
/// # Response
///
/// - 200 OK: Server stopped successfully (or already stopped)
/// - 500 Internal Server Error: Failed to stop server
///
/// # Example Response
///
/// ```json
/// {
///   "message": "Server stopped successfully"
/// }
/// ```
///
/// # Requirements
///
/// Implements requirements:
/// - 6.3: Stop Palpo server with graceful shutdown
/// - 6.6: Return clear error messages
#[handler]
pub async fn stop_server(res: &mut Response) {
    let state = get_server_control_state();

    match state.server_control.stop_server().await {
        Ok(()) => {
            tracing::info!("Server stop request successful");
            res.render(Json(SuccessResponse {
                message: "Server stopped successfully".to_string(),
            }));
        }
        Err(AdminError::ServerStopFailed(msg)) => {
            tracing::error!("Failed to stop server: {}", msg);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Failed to stop server: {}", msg),
            }));
        }
        Err(e) => {
            tracing::error!("Unexpected error stopping server: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Failed to stop server: {}", e),
            }));
        }
    }
}

/// POST /api/v1/admin/server/restart
///
/// Restarts the Palpo server.
///
/// This endpoint:
/// 1. Stops the server (if running)
/// 2. Waits 2 seconds for clean shutdown
/// 3. Starts the server
/// 4. Returns success or error
///
/// # Response
///
/// - 200 OK: Server restarted successfully
/// - 400 Bad Request: Configuration validation failed
/// - 500 Internal Server Error: Failed to restart server
///
/// # Example Response
///
/// ```json
/// {
///   "message": "Server restarted successfully"
/// }
/// ```
///
/// # Requirements
///
/// Implements requirements:
/// - 6.4: Restart Palpo server
/// - 6.6: Return clear error messages
#[handler]
pub async fn restart_server(res: &mut Response) {
    let state = get_server_control_state();

    match state.server_control.restart_server().await {
        Ok(()) => {
            tracing::info!("Server restart request successful");
            res.render(Json(SuccessResponse {
                message: "Server restarted successfully".to_string(),
            }));
        }
        Err(AdminError::ConfigValidationFailed(msg)) => {
            tracing::warn!("Configuration validation failed during restart: {}", msg);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(ErrorResponse {
                error: format!("Configuration validation failed: {}", msg),
            }));
        }
        Err(AdminError::ServerStartFailed(msg)) => {
            tracing::error!("Failed to start server during restart: {}", msg);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Failed to start server: {}", msg),
            }));
        }
        Err(AdminError::ServerStopFailed(msg)) => {
            tracing::error!("Failed to stop server during restart: {}", msg);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Failed to stop server: {}", msg),
            }));
        }
        Err(e) => {
            tracing::error!("Unexpected error restarting server: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse {
                error: format!("Failed to restart server: {}", e),
            }));
        }
    }
}
