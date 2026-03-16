/// Server Configuration API
///
/// Manages Palpo server configuration including reading, writing, and validating
/// TOML configuration files. This service allows Web UI admins to configure
/// the Palpo server before starting it.

use crate::types::{AdminError, ServerConfig};
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Configuration metadata for a single field
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfigFieldMetadata {
    pub name: String,
    pub description: String,
    pub field_type: String,
    pub default_value: Option<JsonValue>,
    pub required: bool,
    pub validation_rules: Option<HashMap<String, JsonValue>>,
}

/// Configuration metadata structure
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfigMetadata {
    pub fields: Vec<ConfigFieldMetadata>,
}

/// Server Configuration API service
///
/// Provides methods to manage Palpo server configuration:
/// - Read current configuration from TOML file
/// - Validate configuration parameters
/// - Save configuration to TOML file
/// - Provide default configuration
pub struct ServerConfigAPI;

impl ServerConfigAPI {
    /// Default configuration file path
    const CONFIG_PATH: &'static str = "palpo.toml";

    /// Gets the current server configuration
    ///
    /// If the configuration file doesn't exist, returns the default configuration.
    ///
    /// # Returns
    /// - `Ok(ServerConfig)` - Current or default configuration
    /// - `Err(AdminError)` - If file reading or parsing fails
    pub async fn get_config() -> Result<ServerConfig, AdminError> {
        let config_path = PathBuf::from(Self::CONFIG_PATH);
        
        if !config_path.exists() {
            return Ok(Self::default_config());
        }

        let content = fs::read_to_string(&config_path).await?;
        let config: ServerConfig = toml::from_str(&content)?;
        
        Ok(config)
    }

    /// Validates server configuration
    ///
    /// Checks that all configuration parameters are valid:
    /// - Database URL must start with "postgresql://"
    /// - Server name must not be empty
    /// - Port must not be 0
    /// - TLS certificate and key files must exist if specified
    ///
    /// # Arguments
    /// - `config` - Configuration to validate
    ///
    /// # Returns
    /// - `Ok(())` - Configuration is valid
    /// - `Err(AdminError)` - Configuration validation failed with specific error
    pub fn validate_config(config: &ServerConfig) -> Result<(), AdminError> {
        // Validate database URL format
        if !config.database_url.starts_with("postgresql://") {
            return Err(AdminError::InvalidDatabaseUrl);
        }

        // Validate server name
        if config.server_name.is_empty() {
            return Err(AdminError::InvalidServerName);
        }

        // Validate port
        if config.port == 0 {
            return Err(AdminError::InvalidPort);
        }

        // Validate TLS config if provided
        if let (Some(cert), Some(key)) = (&config.tls_certificate, &config.tls_private_key) {
            let cert_path = Path::new(cert);
            let key_path = Path::new(key);
            
            if !cert_path.exists() {
                return Err(AdminError::TLSCertificateNotFound);
            }
            if !key_path.exists() {
                return Err(AdminError::TLSPrivateKeyNotFound);
            }
        }

        Ok(())
    }

    /// Saves server configuration to file
    ///
    /// Validates the configuration before saving. Writes the configuration
    /// to a TOML file at the default path.
    ///
    /// # Arguments
    /// - `config` - Configuration to save
    ///
    /// # Returns
    /// - `Ok(())` - Configuration saved successfully
    /// - `Err(AdminError)` - Validation or file writing failed
    pub async fn save_config(config: &ServerConfig) -> Result<(), AdminError> {
        // Validate before saving
        Self::validate_config(config)?;

        // Serialize to TOML
        let toml_content = toml::to_string_pretty(config)?;

        // Write to file
        fs::write(Self::CONFIG_PATH, toml_content).await?;

        Ok(())
    }

    /// Returns the default server configuration
    ///
    /// Provides sensible defaults for a new Palpo server installation:
    /// - Database: localhost PostgreSQL with default credentials
    /// - Server name: localhost
    /// - Bind address: 0.0.0.0 (all interfaces)
    /// - Port: 8008 (standard Matrix port)
    /// - No TLS (suitable for development)
    ///
    /// # Returns
    /// Default `ServerConfig` instance
    pub fn default_config() -> ServerConfig {
        ServerConfig {
            database_url: "postgresql://palpo:password@localhost/palpo".to_string(),
            server_name: "localhost".to_string(),
            bind_address: "0.0.0.0".to_string(),
            port: 8008,
            tls_certificate: None,
            tls_private_key: None,
        }
    }

    /// Gets configuration metadata for form editing
    ///
    /// Returns metadata about all configuration fields including descriptions,
    /// default values, and validation rules.
    ///
    /// # Returns
    /// `ConfigMetadata` with field descriptions and validation rules
    pub fn get_metadata() -> ConfigMetadata {
        ConfigMetadata {
            fields: vec![
                ConfigFieldMetadata {
                    name: "database_url".to_string(),
                    description: "PostgreSQL database connection URL".to_string(),
                    field_type: "string".to_string(),
                    default_value: Some(JsonValue::String("postgresql://palpo:password@localhost/palpo".to_string())),
                    required: true,
                    validation_rules: Some({
                        let mut rules = HashMap::new();
                        rules.insert("pattern".to_string(), JsonValue::String("^postgresql://".to_string()));
                        rules
                    }),
                },
                ConfigFieldMetadata {
                    name: "server_name".to_string(),
                    description: "Matrix server name (domain)".to_string(),
                    field_type: "string".to_string(),
                    default_value: Some(JsonValue::String("localhost".to_string())),
                    required: true,
                    validation_rules: Some({
                        let mut rules = HashMap::new();
                        rules.insert("min_length".to_string(), JsonValue::Number(1.into()));
                        rules
                    }),
                },
                ConfigFieldMetadata {
                    name: "bind_address".to_string(),
                    description: "IP address to bind the server to".to_string(),
                    field_type: "string".to_string(),
                    default_value: Some(JsonValue::String("0.0.0.0".to_string())),
                    required: true,
                    validation_rules: None,
                },
                ConfigFieldMetadata {
                    name: "port".to_string(),
                    description: "Port number for the server".to_string(),
                    field_type: "integer".to_string(),
                    default_value: Some(JsonValue::Number(8008.into())),
                    required: true,
                    validation_rules: Some({
                        let mut rules = HashMap::new();
                        rules.insert("min".to_string(), JsonValue::Number(1.into()));
                        rules.insert("max".to_string(), JsonValue::Number(65535.into()));
                        rules
                    }),
                },
                ConfigFieldMetadata {
                    name: "tls_certificate".to_string(),
                    description: "Path to TLS certificate file (optional)".to_string(),
                    field_type: "string".to_string(),
                    default_value: None,
                    required: false,
                    validation_rules: None,
                },
                ConfigFieldMetadata {
                    name: "tls_private_key".to_string(),
                    description: "Path to TLS private key file (optional)".to_string(),
                    field_type: "string".to_string(),
                    default_value: None,
                    required: false,
                    validation_rules: None,
                },
            ],
        }
    }

    /// Converts ServerConfig to JSON for form display
    ///
    /// # Arguments
    /// - `config` - Configuration to convert
    ///
    /// # Returns
    /// JSON representation of the configuration
    pub fn config_to_json(config: &ServerConfig) -> JsonValue {
        json!({
            "database_url": config.database_url,
            "server_name": config.server_name,
            "bind_address": config.bind_address,
            "port": config.port,
            "tls_certificate": config.tls_certificate,
            "tls_private_key": config.tls_private_key,
        })
    }

    /// Converts JSON to ServerConfig
    ///
    /// # Arguments
    /// - `json` - JSON representation of configuration
    ///
    /// # Returns
    /// - `Ok(ServerConfig)` - Parsed configuration
    /// - `Err(AdminError)` - If JSON is invalid
    pub fn json_to_config(json: &JsonValue) -> Result<ServerConfig, AdminError> {
        let config = ServerConfig {
            database_url: json["database_url"]
                .as_str()
                .ok_or_else(|| AdminError::InvalidInput("database_url must be a string".to_string()))?
                .to_string(),
            server_name: json["server_name"]
                .as_str()
                .ok_or_else(|| AdminError::InvalidInput("server_name must be a string".to_string()))?
                .to_string(),
            bind_address: json["bind_address"]
                .as_str()
                .ok_or_else(|| AdminError::InvalidInput("bind_address must be a string".to_string()))?
                .to_string(),
            port: json["port"]
                .as_u64()
                .ok_or_else(|| AdminError::InvalidInput("port must be a number".to_string()))?
                as u16,
            tls_certificate: json["tls_certificate"].as_str().map(|s| s.to_string()),
            tls_private_key: json["tls_private_key"].as_str().map(|s| s.to_string()),
        };
        Ok(config)
    }

    /// Gets raw TOML file content
    ///
    /// # Returns
    /// - `Ok(String)` - Raw TOML content
    /// - `Err(AdminError)` - If file reading fails
    pub async fn get_toml_content() -> Result<String, AdminError> {
        let config_path = PathBuf::from(Self::CONFIG_PATH);
        
        if !config_path.exists() {
            let default_config = Self::default_config();
            let toml_content = toml::to_string_pretty(&default_config)?;
            return Ok(toml_content);
        }

        fs::read_to_string(&config_path).await.map_err(|e| AdminError::IoError(e.to_string()))
    }

    /// Saves raw TOML content to file
    ///
    /// Validates TOML syntax and content before saving.
    ///
    /// # Arguments
    /// - `content` - Raw TOML content
    ///
    /// # Returns
    /// - `Ok(())` - TOML saved successfully
    /// - `Err(AdminError)` - If validation or saving fails
    pub async fn save_toml_content(content: &str) -> Result<(), AdminError> {
        // Parse TOML to validate syntax
        let config: ServerConfig = toml::from_str(content)?;
        
        // Validate configuration content
        Self::validate_config(&config)?;

        // Write to file
        fs::write(Self::CONFIG_PATH, content).await.map_err(|e| AdminError::IoError(e.to_string()))?;

        Ok(())
    }

    /// Validates TOML syntax and content
    ///
    /// # Arguments
    /// - `content` - Raw TOML content to validate
    ///
    /// # Returns
    /// - `Ok(())` - TOML is valid
    /// - `Err(AdminError)` - If validation fails
    pub fn validate_toml(content: &str) -> Result<(), AdminError> {
        // Parse TOML to validate syntax
        let config: ServerConfig = toml::from_str(content)?;
        
        // Validate configuration content
        Self::validate_config(&config)
    }

    /// Parses TOML and returns as JSON
    ///
    /// # Arguments
    /// - `content` - Raw TOML content
    ///
    /// # Returns
    /// - `Ok(JsonValue)` - Parsed configuration as JSON
    /// - `Err(AdminError)` - If parsing fails
    pub fn parse_toml_to_json(content: &str) -> Result<JsonValue, AdminError> {
        let config: ServerConfig = toml::from_str(content)?;
        Ok(Self::config_to_json(&config))
    }

    /// Searches configuration items by label or description
    ///
    /// # Arguments
    /// - `query` - Search query (case-insensitive)
    ///
    /// # Returns
    /// Vector of matching field metadata
    pub fn search_config(query: &str) -> Vec<ConfigFieldMetadata> {
        let metadata = Self::get_metadata();
        let query_lower = query.to_lowercase();
        
        metadata.fields.into_iter()
            .filter(|field| {
                field.name.to_lowercase().contains(&query_lower)
                    || field.description.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    /// Resets configuration to last saved state
    ///
    /// Reloads configuration from file, discarding any unsaved changes.
    ///
    /// # Returns
    /// - `Ok(ServerConfig)` - Reloaded configuration
    /// - `Err(AdminError)` - If reading fails
    pub async fn reset_config() -> Result<ServerConfig, AdminError> {
        Self::get_config().await
    }

    /// Reloads configuration from file without restart
    ///
    /// # Returns
    /// - `Ok(ServerConfig)` - Reloaded configuration
    /// - `Err(AdminError)` - If reading fails
    pub async fn reload_config() -> Result<ServerConfig, AdminError> {
        Self::get_config().await
    }

    /// Exports configuration in specified format
    ///
    /// # Arguments
    /// - `format` - Export format: "json", "yaml", or "toml"
    ///
    /// # Returns
    /// - `Ok(String)` - Configuration in specified format
    /// - `Err(AdminError)` - If export fails
    pub async fn export_config(format: &str) -> Result<String, AdminError> {
        let config = Self::get_config().await?;
        
        match format.to_lowercase().as_str() {
            "json" => {
                let json = Self::config_to_json(&config);
                serde_json::to_string_pretty(&json)
                    .map_err(|e| AdminError::InvalidInput(e.to_string()))
            }
            "yaml" => {
                serde_yaml::to_string(&config)
                    .map_err(|e| AdminError::InvalidInput(e.to_string()))
            }
            "toml" => {
                toml::to_string_pretty(&config)
                    .map_err(|e| AdminError::TomlError(e.to_string()))
            }
            _ => Err(AdminError::InvalidInput(format!("Unsupported format: {}", format))),
        }
    }

    /// Imports configuration from specified format
    ///
    /// # Arguments
    /// - `content` - Configuration content
    /// - `format` - Import format: "json", "yaml", or "toml"
    ///
    /// # Returns
    /// - `Ok(ServerConfig)` - Imported configuration
    /// - `Err(AdminError)` - If import or validation fails
    pub fn import_config(content: &str, format: &str) -> Result<ServerConfig, AdminError> {
        let config = match format.to_lowercase().as_str() {
            "json" => {
                let json: JsonValue = serde_json::from_str(content)
                    .map_err(|e| AdminError::InvalidInput(format!("Invalid JSON: {}", e)))?;
                Self::json_to_config(&json)?
            }
            "yaml" => {
                serde_yaml::from_str(content)
                    .map_err(|e| AdminError::InvalidInput(format!("Invalid YAML: {}", e)))?
            }
            "toml" => {
                toml::from_str(content)
                    .map_err(|e| AdminError::TomlError(e.to_string()))?
            }
            _ => return Err(AdminError::InvalidInput(format!("Unsupported format: {}", format))),
        };
        
        // Validate imported configuration
        Self::validate_config(&config)?;
        
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_config() {
        let config = ServerConfig {
            database_url: "postgresql://user:pass@localhost/db".to_string(),
            server_name: "example.com".to_string(),
            bind_address: "0.0.0.0".to_string(),
            port: 8008,
            tls_certificate: None,
            tls_private_key: None,
        };

        assert!(ServerConfigAPI::validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_invalid_database_url() {
        let config = ServerConfig {
            database_url: "mysql://localhost/db".to_string(),
            server_name: "example.com".to_string(),
            bind_address: "0.0.0.0".to_string(),
            port: 8008,
            tls_certificate: None,
            tls_private_key: None,
        };

        let result = ServerConfigAPI::validate_config(&config);
        assert!(matches!(result, Err(AdminError::InvalidDatabaseUrl)));
    }

    #[test]
    fn test_validate_empty_server_name() {
        let config = ServerConfig {
            database_url: "postgresql://localhost/db".to_string(),
            server_name: "".to_string(),
            bind_address: "0.0.0.0".to_string(),
            port: 8008,
            tls_certificate: None,
            tls_private_key: None,
        };

        let result = ServerConfigAPI::validate_config(&config);
        assert!(matches!(result, Err(AdminError::InvalidServerName)));
    }

    #[test]
    fn test_validate_invalid_port() {
        let config = ServerConfig {
            database_url: "postgresql://localhost/db".to_string(),
            server_name: "example.com".to_string(),
            bind_address: "0.0.0.0".to_string(),
            port: 0,
            tls_certificate: None,
            tls_private_key: None,
        };

        let result = ServerConfigAPI::validate_config(&config);
        assert!(matches!(result, Err(AdminError::InvalidPort)));
    }

    #[test]
    fn test_default_config() {
        let config = ServerConfigAPI::default_config();
        
        assert_eq!(config.database_url, "postgresql://palpo:password@localhost/palpo");
        assert_eq!(config.server_name, "localhost");
        assert_eq!(config.bind_address, "0.0.0.0");
        assert_eq!(config.port, 8008);
        assert!(config.tls_certificate.is_none());
        assert!(config.tls_private_key.is_none());
    }
}
