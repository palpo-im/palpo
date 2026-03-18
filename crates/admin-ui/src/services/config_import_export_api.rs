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

/// Export response
#[derive(Debug, Clone, Deserialize)]
pub struct ExportResponse {
    pub content: String,
    pub format: String,
    pub size_bytes: usize,
    pub timestamp: String,
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
    pub async fn export_config(options: ExportOptions) -> Result<ExportResponse, String> {
        let client = get_api_client()
            .map_err(|e| format!("API client error: {}", e))?;

        let request = serde_json::json!({
            "format": "toml",
            "include_sensitive": options.include_sensitive,
            "include_defaults": options.include_defaults,
        });

        client
            .post_json_response::<_, ExportResponse>(
                "/api/v1/admin/config/export",
                &request,
            )
            .await
            .map_err(|e| format!("Export failed: {}", e))
    }

    /// Import configuration from TOML
    pub async fn import_config(request: ConfigImportRequest) -> Result<ImportResult, String> {
        let client = get_api_client()
            .map_err(|e| format!("API client error: {}", e))?;

        let payload = serde_json::json!({
            "content": request.content,
            "format": "toml",
            "merge_strategy": match request.merge_strategy {
                MergeStrategy::Replace => "replace",
                MergeStrategy::Merge => "merge",
            },
            "validate_only": request.validate_only,
            "backup_current": request.backup_current,
        });

        client
            .post_json_response::<_, ImportResult>(
                "/api/v1/admin/config/import",
                &payload,
            )
            .await
            .map_err(|e| format!("Import failed: {}", e))
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
