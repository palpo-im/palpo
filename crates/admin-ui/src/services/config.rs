//! Configuration service for frontend

use crate::models::{WebConfigData, WebConfigResult};
use crate::models::validation::ValidationResult;
use crate::services::config_api::{ConfigAPI, UpdateConfigRequest, FieldValidationResult};
use serde::{Deserialize, Serialize};

/// Configuration service for handling config operations in the frontend
#[derive(Clone)]
pub struct ConfigService {
    base_url: String,
}

impl ConfigService {
    /// Create a new configuration service
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    /// Get current server configuration
    pub async fn get_config(&self) -> WebConfigResult<WebConfigData> {
        // In a real implementation, this would make an HTTP request to the backend
        // For now, we'll use the ConfigAPI directly (this would be replaced with HTTP calls)
        match ConfigAPI::get_config().await {
            Ok(response) => Ok(response.config),
            Err(e) => Err(e),
        }
    }

    /// Update server configuration
    pub async fn update_config(&self, config: WebConfigData) -> WebConfigResult<()> {
        let request = UpdateConfigRequest {
            config,
            create_backup: true,
        };

        // In a real implementation, this would make an HTTP request to the backend
        ConfigAPI::update_config(request).await
    }

    /// Validate configuration
    pub async fn validate_config(&self, config: WebConfigData) -> WebConfigResult<ValidationResult> {
        // In a real implementation, this would make an HTTP request to the backend
        ConfigAPI::validate_config(&config).await
    }

    /// Validate a single configuration field
    pub async fn validate_field(&self, field: &str, value: &str) -> WebConfigResult<FieldValidationResult> {
        // In a real implementation, this would make an HTTP request to the backend
        ConfigAPI::validate_field(field, value).await
    }

    /// Reload configuration
    pub async fn reload_config(&self) -> WebConfigResult<crate::services::config_api::ConfigReloadResult> {
        // In a real implementation, this would make an HTTP request to the backend
        ConfigAPI::reload_config().await
    }

    /// Export configuration
    pub async fn export_config(&self, format: ConfigFormat) -> WebConfigResult<String> {
        // Placeholder implementation
        match format {
            ConfigFormat::Toml => {
                let config = self.get_config().await?;
                toml::to_string_pretty(&config)
                    .map_err(|e| crate::models::WebConfigError::client(format!("Failed to serialize TOML: {}", e)))
            }
            ConfigFormat::Json => {
                let config = self.get_config().await?;
                serde_json::to_string_pretty(&config)
                    .map_err(|e| crate::models::WebConfigError::client(format!("Failed to serialize JSON: {}", e)))
            }
            ConfigFormat::Yaml => {
                let config = self.get_config().await?;
                serde_yaml::to_string(&config)
                    .map_err(|e| crate::models::WebConfigError::client(format!("Failed to serialize YAML: {}", e)))
            }
        }
    }

    /// Import configuration from string
    pub async fn import_config(&self, content: &str, format: ConfigFormat) -> WebConfigResult<WebConfigData> {
        let config: WebConfigData = match format {
            ConfigFormat::Toml => {
                toml::from_str(content)
                    .map_err(|e| crate::models::WebConfigError::client(format!("Failed to parse TOML: {}", e)))?
            }
            ConfigFormat::Json => {
                serde_json::from_str(content)
                    .map_err(|e| crate::models::WebConfigError::client(format!("Failed to parse JSON: {}", e)))?
            }
            ConfigFormat::Yaml => {
                serde_yaml::from_str(content)
                    .map_err(|e| crate::models::WebConfigError::client(format!("Failed to parse YAML: {}", e)))?
            }
        };

        // Validate the imported config
        let validation = self.validate_config(config.clone()).await?;
        if !validation.valid {
            let error_messages: Vec<String> = validation.errors
                .iter()
                .map(|e| e.message.clone())
                .collect();
            return Err(crate::models::WebConfigError::validation(format!(
                "Imported configuration is invalid: {}",
                error_messages.join(", ")
            )));
        }

        Ok(config)
    }
}

/// Default configuration service instance
impl Default for ConfigService {
    fn default() -> Self {
        Self::new("http://localhost:8008")
    }
}

/// Configuration export/import formats
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConfigFormat {
    Toml,
    Json,
    Yaml,
}

impl ConfigFormat {
    /// Get all available formats
    pub fn all() -> Vec<ConfigFormat> {
        vec![
            ConfigFormat::Toml,
            ConfigFormat::Json,
            ConfigFormat::Yaml,
        ]
    }

    /// Get format display name
    pub fn display_name(&self) -> &'static str {
        match self {
            ConfigFormat::Toml => "TOML",
            ConfigFormat::Json => "JSON",
            ConfigFormat::Yaml => "YAML",
        }
    }

    /// Get file extension
    pub fn extension(&self) -> &'static str {
        match self {
            ConfigFormat::Toml => "toml",
            ConfigFormat::Json => "json",
            ConfigFormat::Yaml => "yaml",
        }
    }

    /// Get MIME type
    pub fn mime_type(&self) -> &'static str {
        match self {
            ConfigFormat::Toml => "application/toml",
            ConfigFormat::Json => "application/json",
            ConfigFormat::Yaml => "application/yaml",
        }
    }
}