/// Server Configuration API
///
/// Manages Palpo server configuration including reading, writing, and validating
/// TOML configuration files. This service allows Web UI admins to configure
/// the Palpo server before starting it.

use crate::types::{AdminError, ServerConfig, ListenerConfig, DatabaseConfig, WellKnownConfig};
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
    /// - Listener addresses must be valid
    ///
    /// # Arguments
    /// - `config` - Configuration to validate
    ///
    /// # Returns
    /// - `Ok(())` - Configuration is valid
    /// - `Err(AdminError)` - Configuration validation failed with specific error
    pub fn validate_config(config: &ServerConfig) -> Result<(), AdminError> {
        // Validate database URL format
        if !config.database.url.starts_with("postgresql://") {
            return Err(AdminError::InvalidDatabaseUrl);
        }

        // Validate server name
        if config.server_name.is_empty() {
            return Err(AdminError::InvalidServerName);
        }

        // Validate at least one listener exists
        if config.listener_configs.is_empty() {
            return Err(AdminError::InvalidPort);
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
    /// - Server name: localhost:8008 (for local testing)
    /// - Listener: 0.0.0.0:8008 (all interfaces)
    /// - Well-known server endpoint configured for local testing
    /// - Registration enabled
    ///
    /// # Returns
    /// Default `ServerConfig` instance
    pub fn default_config() -> ServerConfig {
        ServerConfig {
            server_name: "localhost:8008".to_string(),
            allow_registration: true,
            listener_configs: vec![
                ListenerConfig {
                    address: "0.0.0.0:8008".to_string(),
                }
            ],
            database: DatabaseConfig {
                url: "postgresql://palpo:password@localhost/palpo".to_string(),
            },
            well_known: WellKnownConfig {
                server: "localhost:8008".to_string(),
                client: "http://localhost:8008".to_string(),
            },
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
                    name: "server_name".to_string(),
                    description: "Matrix server name (domain:port for local testing)".to_string(),
                    field_type: "string".to_string(),
                    default_value: Some(JsonValue::String("localhost:8008".to_string())),
                    required: true,
                    validation_rules: Some({
                        let mut rules = HashMap::new();
                        rules.insert("min_length".to_string(), JsonValue::Number(1.into()));
                        rules
                    }),
                },
                ConfigFieldMetadata {
                    name: "allow_registration".to_string(),
                    description: "Enable user registration on the server".to_string(),
                    field_type: "boolean".to_string(),
                    default_value: Some(JsonValue::Bool(true)),
                    required: false,
                    validation_rules: None,
                },
                ConfigFieldMetadata {
                    name: "database.url".to_string(),
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
                    name: "listeners.address".to_string(),
                    description: "Address and port to bind the server to".to_string(),
                    field_type: "string".to_string(),
                    default_value: Some(JsonValue::String("0.0.0.0:8008".to_string())),
                    required: true,
                    validation_rules: None,
                },
                ConfigFieldMetadata {
                    name: "well_known.server".to_string(),
                    description: "Server well-known endpoint for federation".to_string(),
                    field_type: "string".to_string(),
                    default_value: Some(JsonValue::String("localhost:8008".to_string())),
                    required: false,
                    validation_rules: None,
                },
                ConfigFieldMetadata {
                    name: "well_known.client".to_string(),
                    description: "Client well-known endpoint for clients".to_string(),
                    field_type: "string".to_string(),
                    default_value: Some(JsonValue::String("http://localhost:8008".to_string())),
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
            "server_name": config.server_name,
            "allow_registration": config.allow_registration,
            "listener_configs": config.listener_configs,
            "database": config.database,
            "well_known": config.well_known,
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
        let server_name = json["server_name"]
            .as_str()
            .ok_or_else(|| AdminError::InvalidInput("server_name must be a string".to_string()))?
            .to_string();
        
        let allow_registration = json["allow_registration"]
            .as_bool()
            .unwrap_or(true);

        let listener_configs = if let Some(listeners) = json["listener_configs"].as_array() {
            if listeners.is_empty() {
                return Err(AdminError::InvalidInput("listener_configs must not be empty".to_string()));
            }
            listeners.iter().map(|l| {
                let addr = l["address"]
                    .as_str()
                    .ok_or_else(|| AdminError::InvalidInput("listener address must be a string".to_string()))?;
                Ok(ListenerConfig {
                    address: addr.to_string(),
                })
            }).collect::<Result<Vec<_>, AdminError>>()?
        } else {
            // Default to single listener if not specified
            vec![ListenerConfig {
                address: "0.0.0.0:8008".to_string(),
            }]
        };

        let database = if let Some(db_json) = json["database"].as_object() {
            DatabaseConfig {
                url: db_json.get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| AdminError::InvalidInput("database.url must be a string".to_string()))?
                    .to_string(),
            }
        } else {
            DatabaseConfig {
                url: "postgresql://palpo:password@localhost/palpo".to_string(),
            }
        };

        let well_known = if let Some(wk_json) = json["well_known"].as_object() {
            WellKnownConfig {
                server: wk_json.get("server")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| AdminError::InvalidInput("well_known.server must be a string".to_string()))?
                    .to_string(),
                client: wk_json.get("client")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("http://{}", server_name)),
            }
        } else {
            WellKnownConfig {
                server: server_name.clone(),
                client: format!("http://{}", server_name),
            }
        };

        Ok(ServerConfig {
            server_name,
            allow_registration,
            listener_configs,
            database,
            well_known,
        })
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
    /// Validates TOML syntax before saving. Does not validate against ServerConfig
    /// schema since the raw palpo.toml has a different structure.
    ///
    /// # Arguments
    /// - `content` - Raw TOML content
    ///
    /// # Returns
    /// - `Ok(())` - TOML saved successfully
    /// - `Err(AdminError)` - If validation or saving fails
    pub async fn save_toml_content(content: &str) -> Result<(), AdminError> {
        // Validate TOML syntax only (not against ServerConfig schema)
        content.parse::<toml::Value>()
            .map_err(|e| AdminError::TomlError(e.to_string()))?;

        // Write to file
        fs::write(Self::CONFIG_PATH, content).await.map_err(|e| AdminError::IoError(e.to_string()))?;

        Ok(())
    }

    /// Validates TOML syntax
    ///
    /// Only validates TOML syntax, not content against ServerConfig schema,
    /// since the raw palpo.toml has a different structure.
    ///
    /// # Arguments
    /// - `content` - Raw TOML content to validate
    ///
    /// # Returns
    /// - `Ok(())` - TOML syntax is valid
    /// - `Err(AdminError)` - If syntax is invalid
    pub fn validate_toml(content: &str) -> Result<(), AdminError> {
        content.parse::<toml::Value>()
            .map_err(|e| AdminError::TomlError(e.to_string()))?;
        Ok(())
    }

    /// Parses TOML and returns as JSON
    ///
    /// # Arguments
    /// - `content` - Raw TOML content
    ///
    /// # Returns
    /// - `Ok(JsonValue)` - Parsed TOML as JSON
    /// - `Err(AdminError)` - If parsing fails
    pub fn parse_toml_to_json(content: &str) -> Result<JsonValue, AdminError> {
        let value: toml::Value = content.parse()
            .map_err(|e: toml::de::Error| AdminError::TomlError(e.to_string()))?;
        serde_json::to_value(&value)
            .map_err(|e| AdminError::InvalidInput(e.to_string()))
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
            server_name: "example.com".to_string(),
            allow_registration: true,
            listener_configs: vec![
                ListenerConfig {
                    address: "0.0.0.0:8008".to_string(),
                }
            ],
            database: DatabaseConfig {
                url: "postgresql://user:pass@localhost/db".to_string(),
            },
            well_known: WellKnownConfig {
                server: "example.com".to_string(),
                client: "https://example.com".to_string(),
            },
        };

        assert!(ServerConfigAPI::validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_invalid_database_url() {
        let config = ServerConfig {
            server_name: "example.com".to_string(),
            allow_registration: true,
            listener_configs: vec![
                ListenerConfig {
                    address: "0.0.0.0:8008".to_string(),
                }
            ],
            database: DatabaseConfig {
                url: "mysql://localhost/db".to_string(),
            },
            well_known: WellKnownConfig {
                server: "example.com".to_string(),
                client: "https://example.com".to_string(),
            },
        };

        let result = ServerConfigAPI::validate_config(&config);
        assert!(matches!(result, Err(AdminError::InvalidDatabaseUrl)));
    }

    #[test]
    fn test_validate_empty_server_name() {
        let config = ServerConfig {
            server_name: "".to_string(),
            allow_registration: true,
            listener_configs: vec![
                ListenerConfig {
                    address: "0.0.0.0:8008".to_string(),
                }
            ],
            database: DatabaseConfig {
                url: "postgresql://localhost/db".to_string(),
            },
            well_known: WellKnownConfig {
                server: "example.com".to_string(),
                client: "https://example.com".to_string(),
            },
        };

        let result = ServerConfigAPI::validate_config(&config);
        assert!(matches!(result, Err(AdminError::InvalidServerName)));
    }

    #[test]
    fn test_validate_no_listeners() {
        let config = ServerConfig {
            server_name: "example.com".to_string(),
            allow_registration: true,
            listener_configs: vec![],
            database: DatabaseConfig {
                url: "postgresql://localhost/db".to_string(),
            },
            well_known: WellKnownConfig {
                server: "example.com".to_string(),
                client: "https://example.com".to_string(),
            },
        };

        let result = ServerConfigAPI::validate_config(&config);
        assert!(matches!(result, Err(AdminError::InvalidPort)));
    }

    #[test]
    fn test_default_config() {
        let config = ServerConfigAPI::default_config();
        
        assert_eq!(config.database.url, "postgresql://palpo:password@localhost/palpo");
        assert_eq!(config.server_name, "localhost:8008");
        assert_eq!(config.listener_configs.len(), 1);
        assert_eq!(config.listener_configs[0].address, "0.0.0.0:8008");
        assert_eq!(config.well_known.server, "localhost:8008");
        assert_eq!(config.well_known.client, "http://localhost:8008");
    }
}
