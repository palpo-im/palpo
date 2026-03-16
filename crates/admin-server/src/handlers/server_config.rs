/// Server Configuration HTTP Handlers
///
/// REST API endpoints for managing Palpo server configuration:
/// - GET /api/v1/config/form - Get parsed configuration as form data
/// - POST /api/v1/config/form - Save configuration from form data
/// - GET /api/v1/config/metadata - Get configuration metadata
/// - POST /api/v1/config/reset - Reset configuration to last saved state
/// - POST /api/v1/config/reload - Reload configuration from file
/// - GET /api/v1/server/version - Get server version information
/// - GET /api/v1/config/search - Search configuration items
/// - GET /api/v1/config/toml - Get raw TOML file content
/// - POST /api/v1/config/toml - Save raw TOML file content
/// - POST /api/v1/config/toml/validate - Validate TOML syntax and content
/// - POST /api/v1/config/toml/parse - Parse TOML and return as JSON
/// - POST /api/v1/config/export - Export configuration
/// - POST /api/v1/config/import - Import and validate configuration

use crate::server_config::ServerConfigAPI;
use crate::types::{AdminError, ServerConfig};
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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


// ===== Form Editing Mode Endpoints =====

/// GET /api/v1/config/form
///
/// Gets parsed configuration as form data (JSON)
#[handler]
pub async fn get_config_form(res: &mut Response) {
    match ServerConfigAPI::get_config().await {
        Ok(config) => {
            let form_data = ServerConfigAPI::config_to_json(&config);
            res.render(Json(serde_json::json!({
                "success": true,
                "data": form_data
            })));
        }
        Err(err) => {
            tracing::error!("Failed to get config form: {}", err);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            })));
        }
    }
}

/// POST /api/v1/config/form
///
/// Saves configuration from form data
#[derive(Debug, Serialize, Deserialize)]
pub struct SaveFormConfigRequest {
    pub data: JsonValue,
}

#[handler]
pub async fn save_config_form(req: &mut Request, res: &mut Response) {
    let request = match req.parse_json::<SaveFormConfigRequest>().await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("Failed to parse request: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": "Invalid request body"
            })));
            return;
        }
    };

    match ServerConfigAPI::json_to_config(&request.data) {
        Ok(config) => {
            match ServerConfigAPI::save_config(&config).await {
                Ok(()) => {
                    res.render(Json(serde_json::json!({
                        "success": true,
                        "message": "Configuration saved successfully"
                    })));
                }
                Err(err) => {
                    tracing::error!("Failed to save config: {}", err);
                    res.status_code(StatusCode::BAD_REQUEST);
                    res.render(Json(serde_json::json!({
                        "success": false,
                        "error": err.to_string()
                    })));
                }
            }
        }
        Err(err) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            })));
        }
    }
}

/// GET /api/v1/config/metadata
///
/// Gets configuration metadata (field descriptions, defaults, validation rules)
#[handler]
pub async fn get_config_metadata(res: &mut Response) {
    let metadata = ServerConfigAPI::get_metadata();
    res.render(Json(serde_json::json!({
        "success": true,
        "data": metadata
    })));
}

/// POST /api/v1/config/reset
///
/// Resets configuration to last saved state
#[handler]
pub async fn reset_config_handler(res: &mut Response) {
    match ServerConfigAPI::reset_config().await {
        Ok(config) => {
            let form_data = ServerConfigAPI::config_to_json(&config);
            res.render(Json(serde_json::json!({
                "success": true,
                "data": form_data,
                "message": "Configuration reset to last saved state"
            })));
        }
        Err(err) => {
            tracing::error!("Failed to reset config: {}", err);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            })));
        }
    }
}

/// POST /api/v1/config/reload
///
/// Reloads configuration from file (without restart)
#[handler]
pub async fn reload_config_handler(res: &mut Response) {
    match ServerConfigAPI::reload_config().await {
        Ok(config) => {
            let form_data = ServerConfigAPI::config_to_json(&config);
            res.render(Json(serde_json::json!({
                "success": true,
                "data": form_data,
                "message": "Configuration reloaded from file"
            })));
        }
        Err(err) => {
            tracing::error!("Failed to reload config: {}", err);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            })));
        }
    }
}

/// GET /api/v1/server/version
///
/// Gets server version information
#[derive(Debug, Serialize)]
pub struct VersionResponse {
    pub version: String,
    pub build_date: String,
}

#[handler]
pub async fn get_server_version(res: &mut Response) {
    res.render(Json(serde_json::json!({
        "success": true,
        "data": {
            "version": env!("CARGO_PKG_VERSION"),
            "build_date": env!("CARGO_PKG_VERSION"),
        }
    })));
}

/// GET /api/v1/config/search
///
/// Searches configuration items by label/description
#[derive(Debug, Deserialize)]
pub struct SearchConfigRequest {
    pub query: String,
}

#[handler]
pub async fn search_config(req: &mut Request, res: &mut Response) {
    let query = match req.query::<String>("q") {
        Some(q) => q,
        None => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": "Missing 'q' query parameter"
            })));
            return;
        }
    };

    let results = ServerConfigAPI::search_config(&query);
    res.render(Json(serde_json::json!({
        "success": true,
        "data": results
    })));
}

// ===== TOML Editing Mode Endpoints =====

/// GET /api/v1/config/toml
///
/// Gets raw TOML file content
#[handler]
pub async fn get_config_toml(res: &mut Response) {
    match ServerConfigAPI::get_toml_content().await {
        Ok(content) => {
            res.render(Json(serde_json::json!({
                "success": true,
                "data": {
                    "content": content
                }
            })));
        }
        Err(err) => {
            tracing::error!("Failed to get TOML content: {}", err);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            })));
        }
    }
}

/// POST /api/v1/config/toml
///
/// Saves raw TOML file content
#[derive(Debug, Serialize, Deserialize)]
pub struct SaveTomlConfigRequest {
    pub content: String,
}

#[handler]
pub async fn save_config_toml(req: &mut Request, res: &mut Response) {
    let request = match req.parse_json::<SaveTomlConfigRequest>().await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("Failed to parse request: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": "Invalid request body"
            })));
            return;
        }
    };

    match ServerConfigAPI::save_toml_content(&request.content).await {
        Ok(()) => {
            res.render(Json(serde_json::json!({
                "success": true,
                "message": "TOML configuration saved successfully"
            })));
        }
        Err(err) => {
            tracing::error!("Failed to save TOML: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            })));
        }
    }
}

/// POST /api/v1/config/toml/validate
///
/// Validates TOML syntax and content
#[derive(Debug, Serialize, Deserialize)]
pub struct ValidateTomlRequest {
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct ValidateTomlResponse {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[handler]
pub async fn validate_toml(req: &mut Request, res: &mut Response) {
    let request = match req.parse_json::<ValidateTomlRequest>().await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("Failed to parse request: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": "Invalid request body"
            })));
            return;
        }
    };

    match ServerConfigAPI::validate_toml(&request.content) {
        Ok(()) => {
            res.render(Json(serde_json::json!({
                "success": true,
                "valid": true,
                "errors": []
            })));
        }
        Err(err) => {
            res.render(Json(serde_json::json!({
                "success": true,
                "valid": false,
                "errors": [err.to_string()]
            })));
        }
    }
}

/// POST /api/v1/config/toml/parse
///
/// Parses TOML and returns as JSON
#[derive(Debug, Serialize, Deserialize)]
pub struct ParseTomlRequest {
    pub content: String,
}

#[handler]
pub async fn parse_toml(req: &mut Request, res: &mut Response) {
    let request = match req.parse_json::<ParseTomlRequest>().await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("Failed to parse request: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": "Invalid request body"
            })));
            return;
        }
    };

    match ServerConfigAPI::parse_toml_to_json(&request.content) {
        Ok(json) => {
            res.render(Json(serde_json::json!({
                "success": true,
                "data": json
            })));
        }
        Err(err) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            })));
        }
    }
}

// ===== Import/Export Endpoints =====

/// POST /api/v1/config/export
///
/// Exports configuration in specified format
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportConfigRequest {
    pub format: String, // "json", "yaml", or "toml"
}

#[handler]
pub async fn export_config(req: &mut Request, res: &mut Response) {
    let request = match req.parse_json::<ExportConfigRequest>().await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("Failed to parse request: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": "Invalid request body"
            })));
            return;
        }
    };

    match ServerConfigAPI::export_config(&request.format).await {
        Ok(content) => {
            res.render(Json(serde_json::json!({
                "success": true,
                "data": {
                    "format": request.format,
                    "content": content
                }
            })));
        }
        Err(err) => {
            tracing::error!("Failed to export config: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            })));
        }
    }
}

/// POST /api/v1/config/import
///
/// Imports and validates configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportConfigRequest {
    pub format: String, // "json", "yaml", or "toml"
    pub content: String,
}

#[handler]
pub async fn import_config(req: &mut Request, res: &mut Response) {
    let request = match req.parse_json::<ImportConfigRequest>().await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("Failed to parse request: {}", err);
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": "Invalid request body"
            })));
            return;
        }
    };

    match ServerConfigAPI::import_config(&request.content, &request.format) {
        Ok(config) => {
            let form_data = ServerConfigAPI::config_to_json(&config);
            res.render(Json(serde_json::json!({
                "success": true,
                "data": form_data,
                "message": "Configuration imported and validated successfully"
            })));
        }
        Err(err) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            })));
        }
    }
}
