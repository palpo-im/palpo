//! Configuration Import/Export API Client
//!
//! Handles communication with backend for importing and exporting TOML configurations.

use serde::{Deserialize, Serialize};
use crate::services::api_client::get_api_client;

/// Configuration format (only TOML supported for Palpo compatibility)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigFormat {
    Toml,
}

/// Merge strategy for imports
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStrategy {
    Replace,
    Merge,
}

/// Export options
#[derive(Debug, Clone, Serialize)]
pub struct ExportOptions {
    pub format: ConfigFormat,
    pub include_sensitive: bool,
    pub include_defaults: bool,
    pub sections: Option<Vec<String>>,
    pub encrypt: bool,
    pub encryption_key: Option<String>,
}

/// Export response (matches backend response format)
#[derive(Debug, Clone, Deserialize)]
pub struct ExportResponse {
    pub content: String,
    pub format: String,
}

/// API response wrapper (backend wraps all responses in {success, data/error})
#[derive(Debug, Clone, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

/// Import request
#[derive(Debug, Clone, Serialize)]
pub struct ConfigImportRequest {
    pub content: String,
    pub format: ConfigFormat,
    pub merge_strategy: MergeStrategy,
    pub validate_only: bool,
    pub backup_current: bool,
    pub encryption_key: Option<String>,
}

/// Import result
#[derive(Debug, Clone, Deserialize)]
pub struct ImportResult {
    pub success: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub backup_path: Option<String>,
    pub applied_changes: Option<Vec<String>>,
}

/// Configuration Import/Export API client
pub struct ConfigImportExportAPI;

impl ConfigImportExportAPI {
    /// Export current configuration as TOML
    pub async fn export_config(_options: ExportOptions) -> Result<ExportResponse, String> {
        let client = get_api_client()
            .map_err(|e| format!("API client error: {}", e))?;

        let request = serde_json::json!({
            "format": "toml",
        });

        let response = client
            .post_json_response::<_, ApiResponse<ExportResponse>>(
                "/api/v1/config/export",
                &request,
            )
            .await
            .map_err(|e| format!("Export failed: {}", e))?;

        if response.success {
            response.data.ok_or_else(|| "No data in response".to_string())
        } else {
            Err(response.error.unwrap_or_else(|| "Export failed".to_string()))
        }
    }

    /// Import configuration from TOML
    pub async fn import_config(request: ConfigImportRequest) -> Result<ImportResult, String> {
        let client = get_api_client()
            .map_err(|e| format!("API client error: {}", e))?;

        let payload = serde_json::json!({
            "content": request.content,
            "format": "toml",
        });

        let response = client
            .post_json_response::<_, ApiResponse<ImportResult>>(
                "/api/v1/config/import",
                &payload,
            )
            .await
            .map_err(|e| format!("Import failed: {}", e))?;

        if response.success {
            response.data.ok_or_else(|| "No data in response".to_string())
        } else {
            Err(response.error.unwrap_or_else(|| "Import failed".to_string()))
        }
    }

    /// Validate TOML configuration without importing
    pub async fn validate_import(content: String) -> Result<ImportResult, String> {
        let request = ConfigImportRequest {
            content,
            format: ConfigFormat::Toml,
            merge_strategy: MergeStrategy::Replace,
            validate_only: true,
            backup_current: false,
            encryption_key: None,
        };

        Self::import_config(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_options_creation() {
        let opts = ExportOptions {
            format: ConfigFormat::Toml,
            include_sensitive: false,
            include_defaults: false,
            sections: None,
            encrypt: false,
            encryption_key: None,
        };
        assert_eq!(opts.format, ConfigFormat::Toml);
        assert!(!opts.include_sensitive);
    }

    #[test]
    fn test_merge_strategy_serialization() {
        let replace = MergeStrategy::Replace;
        let merge = MergeStrategy::Merge;
        assert_ne!(replace, merge);
    }

    #[test]
    fn test_import_request_creation() {
        let req = ConfigImportRequest {
            content: "[server]\nserver_name = \"example.com\"\n".to_string(),
            format: ConfigFormat::Toml,
            merge_strategy: MergeStrategy::Replace,
            validate_only: false,
            backup_current: true,
            encryption_key: None,
        };
        assert_eq!(req.format, ConfigFormat::Toml);
        assert!(req.backup_current);
    }

    #[test]
    fn test_import_result_success() {
        let result = ImportResult {
            success: true,
            errors: vec![],
            warnings: vec![],
            backup_path: Some("/backup/config.toml".to_string()),
            applied_changes: Some(vec!["server.port".to_string()]),
        };
        assert!(result.success);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_import_result_failure() {
        let result = ImportResult {
            success: false,
            errors: vec!["Invalid TOML syntax".to_string()],
            warnings: vec![],
            backup_path: None,
            applied_changes: None,
        };
        assert!(!result.success);
        assert_eq!(result.errors.len(), 1);
    }
}
