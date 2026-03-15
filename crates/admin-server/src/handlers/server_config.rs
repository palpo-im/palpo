/// Server Configuration HTTP Handlers
///
/// REST API endpoints for managing Palpo server configuration:
/// - GET /api/v1/admin/server/config - Get current configuration
/// - POST /api/v1/admin/server/config - Save configuration
/// - POST /api/v1/admin/server/config/validate - Validate configuration

use crate::server_config::ServerConfigAPI;
use crate::types::{AdminError, ServerConfig};
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

/// Response for GET /api/v1/admin/server/config
#[derive(Debug, Serialize, Deserialize)]
pub struct GetConfigResponse {
    pub config: ServerConfig,
}

/// Request body for POST /api/v1/admin/server/config
#[derive(Debug, Serialize, Deserialize)]
pub struct SaveConfigRequest {
    pub config: ServerConfig,
}

/// Response for POST /api/v1/admin/server/config
#[derive(Debug, Serialize, Deserialize)]
pub struct SaveConfigResponse {
    pub success: bool,
    pub message: String,
}

/// Request body for POST /api/v1/admin/server/config/validate
#[derive(Debug, Serialize, Deserialize)]
pub struct ValidateConfigRequest {
    pub config: ServerConfig,
}

/// Response for POST /api/v1/admin/server/config/validate
#[derive(Debug, Serialize, Deserialize)]
pub struct ValidateConfigResponse {
    pub valid: bool,
    pub errors: Vec<String>,
}

/// GET /api/v1/admin/server/config
///
/// Retrieves the current server configuration. If no configuration file exists,
/// returns the default configuration.
///
/// # Returns
/// - 200 OK with configuration
/// - 500 Internal Server Error if reading fails
#[handler]
pub async fn get_config(res: &mut Response) {
    match ServerConfigAPI::get_config().await {
        Ok(config) => {
            res.render(Json(GetConfigResponse { config }));
        }
        Err(err) => {
            tracing::error!("Failed to get config: {}", err);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(serde_json::json!({
                "error": err.to_string()
            })));
        }
    }
}

/// POST /api/v1/admin/server/config
///
/// Saves the provided server configuration to file. Validates the configuration
/// before saving.
///
/// # Request Body
/// ```json
/// {
///   "config": {
///     "database_url": "postgresql://user:pass@localhost/palpo",
///     "server_name": "example.com",
///     "bind_address": "0.0.0.0",
///     "port": 8008,
///     "tls_certificate": null,
///     "tls_private_key": null
///   }
/// }
/// ```
///
/// # Returns
/// - 200 OK if saved successfully
/// - 400 Bad Request if validation fails
/// - 500 Internal Server Error if saving fails
#[handler]
pub async fn save_config(req: &mut Request, res: &mut Response) {
    let request = match req.parse_json::<SaveConfigRequest>().await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("Failed to parse request: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "error": "Invalid request body"
            })));
            return;
        }
    };

    match ServerConfigAPI::save_config(&request.config).await {
        Ok(()) => {
            res.render(Json(SaveConfigResponse {
                success: true,
                message: "Configuration saved successfully".to_string(),
            }));
        }
        Err(err @ AdminError::InvalidDatabaseUrl)
        | Err(err @ AdminError::InvalidServerName)
        | Err(err @ AdminError::InvalidPort)
        | Err(err @ AdminError::TLSCertificateNotFound)
        | Err(err @ AdminError::TLSPrivateKeyNotFound) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(SaveConfigResponse {
                success: false,
                message: format!("Validation failed: {}", err),
            }));
        }
        Err(err) => {
            tracing::error!("Failed to save config: {}", err);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(SaveConfigResponse {
                success: false,
                message: format!("Failed to save configuration: {}", err),
            }));
        }
    }
}

/// POST /api/v1/admin/server/config/validate
///
/// Validates the provided server configuration without saving it.
/// Useful for real-time validation in the UI.
///
/// # Request Body
/// ```json
/// {
///   "config": {
///     "database_url": "postgresql://user:pass@localhost/palpo",
///     "server_name": "example.com",
///     "bind_address": "0.0.0.0",
///     "port": 8008,
///     "tls_certificate": null,
///     "tls_private_key": null
///   }
/// }
/// ```
///
/// # Returns
/// - 200 OK with validation result
/// - 400 Bad Request if request body is invalid
#[handler]
pub async fn validate_config(req: &mut Request, res: &mut Response) {
    let request = match req.parse_json::<ValidateConfigRequest>().await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("Failed to parse request: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "error": "Invalid request body"
            })));
            return;
        }
    };

    match ServerConfigAPI::validate_config(&request.config) {
        Ok(()) => {
            res.render(Json(ValidateConfigResponse {
                valid: true,
                errors: vec![],
            }));
        }
        Err(err) => {
            res.render(Json(ValidateConfigResponse {
                valid: false,
                errors: vec![err.to_string()],
            }));
        }
    }
}
