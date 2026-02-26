/// Server Configuration API
///
/// Manages Palpo server configuration including reading, writing, and validating
/// TOML configuration files. This service allows Web UI admins to configure
/// the Palpo server before starting it.

use crate::types::{AdminError, ServerConfig};
use std::path::{Path, PathBuf};
use tokio::fs;

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
