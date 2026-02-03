//! Configuration validation utilities

use crate::models::{ConfigError, ConfigWarning, ValidationResult, WebConfigData};
// use regex::Regex;
use std::net::{IpAddr, SocketAddr};
// use url::Url;

/// Validate server name format (Matrix server name)
pub fn validate_server_name(server_name: &str) -> Result<(), ConfigError> {
    if server_name.is_empty() {
        return Err(ConfigError::required_field("server_name"));
    }

    // Basic Matrix server name validation - simple character check
    if !server_name.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-') {
        return Err(ConfigError::invalid_format(
            "server_name",
            "Valid Matrix server name (alphanumeric, dots, and hyphens only)",
        ));
    }

    if server_name.len() > 255 {
        return Err(ConfigError::invalid_value(
            "server_name",
            server_name,
            "Server name too long (max 255 characters)",
        ));
    }

    Ok(())
}

/// Validate IP address format
pub fn validate_ip_address(ip: &str) -> Result<(), ConfigError> {
    ip.parse::<IpAddr>()
        .map_err(|_| ConfigError::invalid_format("ip_address", "Valid IP address"))?;
    Ok(())
}

/// Validate port number
pub fn validate_port(port: u16) -> Result<(), ConfigError> {
    if port == 0 {
        return Err(ConfigError::invalid_value(
            "port",
            port.to_string(),
            "Port cannot be 0",
        ));
    }
    Ok(())
}

/// Validate socket address (IP:port)
pub fn validate_socket_address(addr: &str) -> Result<(), ConfigError> {
    addr.parse::<SocketAddr>()
        .map_err(|_| ConfigError::invalid_format("socket_address", "Valid socket address (IP:port)"))?;
    Ok(())
}

/// Validate database connection string
pub fn validate_database_connection_string(conn_str: &str) -> Result<(), ConfigError> {
    if conn_str.is_empty() {
        return Err(ConfigError::required_field("connection_string"));
    }

    // Basic PostgreSQL connection string validation
    if !conn_str.starts_with("postgresql://") && !conn_str.starts_with("postgres://") {
        return Err(ConfigError::invalid_format(
            "connection_string",
            "PostgreSQL connection string (postgresql://...)",
        ));
    }

    Ok(())
}

/// Validate file path exists and is accessible
pub fn validate_file_path(path: &str, field_name: &str) -> Result<(), ConfigError> {
    if path.is_empty() {
        return Err(ConfigError::required_field(field_name));
    }

    // Basic path validation (more thorough validation would require filesystem access)
    if path.contains('\0') {
        return Err(ConfigError::invalid_value(
            field_name,
            path,
            "Path contains null character",
        ));
    }

    Ok(())
}

/// Validate URL format
pub fn validate_url(url_str: &str, field_name: &str) -> Result<(), ConfigError> {
    if url_str.is_empty() {
        return Err(ConfigError::required_field(field_name));
    }

    // Simple URL validation - check for basic URL structure
    if !url_str.starts_with("http://") && !url_str.starts_with("https://") {
        return Err(ConfigError::invalid_format(field_name, "Valid URL (must start with http:// or https://)"));
    }

    Ok(())
}

/// Validate JWT secret strength
pub fn validate_jwt_secret(secret: &str) -> Result<Vec<ConfigWarning>, ConfigError> {
    if secret.is_empty() {
        return Err(ConfigError::required_field("jwt_secret"));
    }

    let mut warnings = Vec::new();

    if secret == "change-me" || secret == "default" || secret == "secret" {
        warnings.push(ConfigWarning::security_warning(
            "jwt_secret",
            "Using default or weak JWT secret. Please use a strong, random secret.",
        ));
    }

    if secret.len() < 32 {
        warnings.push(ConfigWarning::security_warning(
            "jwt_secret",
            "JWT secret is shorter than recommended 32 characters.",
        ));
    }

    Ok(warnings)
}

/// Validate entire configuration
pub fn validate_config(config: &WebConfigData) -> ValidationResult {
    let mut result = ValidationResult::success();

    // Validate server configuration
    if let Err(error) = validate_server_name(&config.server.server_name) {
        result.add_error(error);
    }

    for (i, listener) in config.server.listeners.iter().enumerate() {
        if let Err(error) = validate_ip_address(&listener.bind) {
            result.add_error(ConfigError::new(
                format!("server.listeners[{}].bind", i),
                error.message,
                error.code,
            ));
        }

        if let Err(error) = validate_port(listener.port) {
            result.add_error(ConfigError::new(
                format!("server.listeners[{}].port", i),
                error.message,
                error.code,
            ));
        }
    }

    // Validate database configuration
    if let Err(error) = validate_database_connection_string(&config.database.connection_string) {
        result.add_error(error);
    }

    if config.database.max_connections == 0 {
        result.add_error(ConfigError::invalid_value(
            "database.max_connections",
            "0",
            "Must be greater than 0",
        ));
    }

    // Validate federation configuration
    if let Err(error) = validate_file_path(&config.federation.signing_key_path, "federation.signing_key_path") {
        result.add_error(error);
    }

    // Validate auth configuration
    if let Ok(warnings) = validate_jwt_secret(&config.auth.jwt_secret) {
        for warning in warnings {
            result.add_warning(warning);
        }
    } else if let Err(error) = validate_jwt_secret(&config.auth.jwt_secret) {
        result.add_error(error);
    }

    // Validate media configuration
    if let Err(error) = validate_file_path(&config.media.storage_path, "media.storage_path") {
        result.add_error(error);
    }

    if config.media.max_file_size == 0 {
        result.add_warning(ConfigWarning::performance_warning(
            "media.max_file_size",
            "Max file size is 0, which may cause issues with media uploads",
        ));
    }

    // Validate network configuration
    for (i, ip_range) in config.network.ip_range_denylist.iter().enumerate() {
        // Basic IP range validation (could be more sophisticated)
        if !ip_range.contains('/') && validate_ip_address(ip_range).is_err() {
            result.add_error(ConfigError::invalid_format(
                &format!("network.ip_range_denylist[{}]", i),
                "Valid IP address or CIDR range",
            ));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_server_name() {
        assert!(validate_server_name("example.com").is_ok());
        assert!(validate_server_name("matrix.example.com").is_ok());
        assert!(validate_server_name("localhost").is_ok());
        
        assert!(validate_server_name("").is_err());
        assert!(validate_server_name("invalid_server!").is_err());
    }

    #[test]
    fn test_validate_ip_address() {
        assert!(validate_ip_address("127.0.0.1").is_ok());
        assert!(validate_ip_address("0.0.0.0").is_ok());
        assert!(validate_ip_address("::1").is_ok());
        
        assert!(validate_ip_address("invalid").is_err());
        assert!(validate_ip_address("256.256.256.256").is_err());
    }

    #[test]
    fn test_validate_database_connection_string() {
        assert!(validate_database_connection_string("postgresql://user:pass@localhost/db").is_ok());
        assert!(validate_database_connection_string("postgres://user:pass@localhost/db").is_ok());
        
        assert!(validate_database_connection_string("").is_err());
        assert!(validate_database_connection_string("mysql://user:pass@localhost/db").is_err());
    }

    #[test]
    fn test_validate_jwt_secret() {
        let warnings = validate_jwt_secret("very-long-and-secure-secret-key-here").unwrap();
        assert!(warnings.is_empty());
        
        let warnings = validate_jwt_secret("change-me").unwrap();
        assert!(!warnings.is_empty());
        
        assert!(validate_jwt_secret("").is_err());
    }
}