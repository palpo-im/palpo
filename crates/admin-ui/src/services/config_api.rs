//! Configuration API service
//! 
//! Provides core functionality for managing Palpo server configuration
//! including reading, writing, validating, and reloading configuration files.

use crate::models::{config::*, error::WebConfigError, validation::{ConfigError, ConfigWarning, ValidationResult}};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::SystemTime;

/// Configuration API service
pub struct ConfigAPI;

impl ConfigAPI {
    // Helper method to create ConfigError with default code
    fn config_error(field: impl Into<String>, message: impl Into<String>) -> ConfigError {
        ConfigError::new(field, message, "VALIDATION_ERROR")
    }
    
    // Helper method to create ConfigWarning with default code
    fn config_warning(field: impl Into<String>, message: impl Into<String>) -> ConfigWarning {
        ConfigWarning::new(field, message, "VALIDATION_WARNING")
    }
    
    /// Get current server configuration
    pub async fn get_config() -> Result<ServerConfigResponse, WebConfigError> {
        let config_path = Self::get_config_path()?;
        let config_content = tokio::fs::read_to_string(&config_path)
            .await
            .map_err(|e| WebConfigError::filesystem_with_path(format!("Failed to read config file: {}", e), &config_path))?;
        
        let config: WebConfigData = toml::from_str(&config_content)
            .map_err(|e| WebConfigError::ParseError { 
                message: format!("Failed to parse TOML: {}", e),
                format: "TOML".to_string(),
            })?;
        
        Ok(ServerConfigResponse {
            config,
            last_modified: Self::get_file_modified_time(&config_path).await?,
            checksum: Self::calculate_checksum(&config_content),
        })
    }
    
    /// Update server configuration
    pub async fn update_config(request: UpdateConfigRequest) -> Result<(), WebConfigError> {
        // Validate configuration before saving
        let validation_result = Self::validate_config(&request.config).await?;
        if !validation_result.valid {
            return Err(WebConfigError::validation(
                validation_result.errors.into_iter()
                    .map(|e| e.message)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        
        let config_path = Self::get_config_path()?;
        
        // Create backup before updating
        if request.create_backup {
            Self::create_backup(&config_path).await?;
        }
        
        // Serialize configuration to TOML
        let toml_content = toml::to_string_pretty(&request.config)
            .map_err(|e| WebConfigError::ParseError { 
                message: format!("Failed to serialize TOML: {}", e),
                format: "TOML".to_string(),
            })?;
        
        // Write configuration to file
        tokio::fs::write(&config_path, toml_content)
            .await
            .map_err(|e| WebConfigError::filesystem_with_path(format!("Failed to write config file: {}", e), &config_path))?;
        
        Ok(())
    }
    
    /// Validate configuration
    pub async fn validate_config(config: &WebConfigData) -> Result<ValidationResult, WebConfigError> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        // Validate server configuration
        Self::validate_server_config(&config.server, &mut errors, &mut warnings);
        
        // Validate database configuration
        Self::validate_database_config(&config.database, &mut errors, &mut warnings);
        
        // Validate federation configuration
        Self::validate_federation_config(&config.federation, &mut errors, &mut warnings);
        
        // Validate auth configuration
        Self::validate_auth_config(&config.auth, &mut errors, &mut warnings);
        
        // Validate media configuration
        Self::validate_media_config(&config.media, &mut errors, &mut warnings);
        
        // Validate network configuration
        Self::validate_network_config(&config.network, &mut errors, &mut warnings);
        
        // Validate logging configuration
        Self::validate_logging_config(&config.logging, &mut errors, &mut warnings);
        
        // Validate cross-section dependencies
        Self::validate_config_dependencies(config, &mut errors, &mut warnings);
        
        Ok(ValidationResult {
            valid: errors.is_empty(),
            errors,
            warnings,
        })
    }
    
    /// Validate a single configuration field
    pub async fn validate_field(field: &str, value: &str) -> Result<FieldValidationResult, WebConfigError> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        match field {
            "server.server_name" => {
                Self::validate_server_name(value, &mut errors, &mut warnings);
            }
            "database.connection_string" => {
                Self::validate_database_connection_string(value, &mut errors, &mut warnings);
            }
            "federation.signing_key_path" => {
                Self::validate_file_path(value, &mut errors, &mut warnings);
            }
            "auth.jwt_secret" => {
                Self::validate_jwt_secret(value, &mut errors, &mut warnings);
            }
            "media.storage_path" => {
                Self::validate_directory_path(value, &mut errors, &mut warnings);
            }
            _ => {
                warnings.push(Self::config_warning(
                    field,
                    "Unknown field, validation skipped"
                ));
            }
        }
        
        Ok(FieldValidationResult {
            valid: errors.is_empty(),
            errors,
            warnings,
        })
    }
    
    /// Reload configuration (if hot reload is supported)
    pub async fn reload_config() -> Result<ConfigReloadResult, WebConfigError> {
        // Check if hot reload is supported
        let hot_reload_supported = Self::check_hot_reload_support().await;
        
        if hot_reload_supported {
            // Attempt hot reload
            match Self::perform_hot_reload().await {
                Ok(_) => Ok(ConfigReloadResult {
                    success: true,
                    errors: Vec::new(),
                    warnings: Vec::new(),
                    hot_reload_supported: true,
                    restart_required: false,
                }),
                Err(e) => Ok(ConfigReloadResult {
                    success: false,
                    errors: vec![e.to_string()],
                    warnings: Vec::new(),
                    hot_reload_supported: true,
                    restart_required: true,
                }),
            }
        } else {
            Ok(ConfigReloadResult {
                success: false,
                errors: Vec::new(),
                warnings: vec!["Hot reload not supported, server restart required".to_string()],
                hot_reload_supported: false,
                restart_required: true,
            })
        }
    }
    
    /// Export configuration
    pub async fn export_config(options: ExportOptions) -> Result<ConfigExportResponse, WebConfigError> {
        let config_response = Self::get_config().await?;
        let mut config = config_response.config;
        
        // Remove sensitive information if requested
        if !options.include_sensitive {
            Self::sanitize_sensitive_data(&mut config);
        }
        
        // Serialize to requested format
        let content = match options.format {
            ConfigFormat::Toml => {
                toml::to_string_pretty(&config)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("TOML serialization failed: {}", e),
                        format: "TOML".to_string(),
                    })?
            }
            ConfigFormat::Json => {
                serde_json::to_string_pretty(&config)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("JSON serialization failed: {}", e),
                        format: "JSON".to_string(),
                    })?
            }
            ConfigFormat::Yaml => {
                serde_yaml::to_string(&config)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("YAML serialization failed: {}", e),
                        format: "YAML".to_string(),
                    })?
            }
            ConfigFormat::Encrypted => {
                return Err(WebConfigError::validation("Encrypted export not yet implemented"));
            }
        };
        
        let checksum = Self::calculate_checksum(&content);
        let size_bytes = content.len() as u64;
        
        Ok(ConfigExportResponse {
            content,
            format: options.format,
            exported_at: SystemTime::now(),
            checksum,
            size_bytes,
        })
    }
    
    /// Import configuration
    pub async fn import_config(request: ConfigImportRequest) -> Result<ImportResult, WebConfigError> {
        // Parse configuration from content
        let imported_config: WebConfigData = match request.format {
            ConfigFormat::Toml => {
                toml::from_str(&request.content)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("TOML parsing failed: {}", e),
                        format: "TOML".to_string(),
                    })?
            }
            ConfigFormat::Json => {
                serde_json::from_str(&request.content)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("JSON parsing failed: {}", e),
                        format: "JSON".to_string(),
                    })?
            }
            ConfigFormat::Yaml => {
                serde_yaml::from_str(&request.content)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("YAML parsing failed: {}", e),
                        format: "YAML".to_string(),
                    })?
            }
            ConfigFormat::Encrypted => {
                return Err(WebConfigError::validation("Encrypted import not yet implemented"));
            }
        };
        
        // Validate imported configuration
        let validation_result = Self::validate_config(&imported_config).await?;
        if !validation_result.valid {
            return Ok(ImportResult {
                success: false,
                applied_changes: Vec::new(),
                skipped_changes: Vec::new(),
                errors: validation_result.errors.into_iter().map(|e| e.message).collect(),
                warnings: validation_result.warnings.into_iter().map(|w| w.message).collect(),
                backup_file: None,
            });
        }
        
        if request.validate_only {
            return Ok(ImportResult {
                success: true,
                applied_changes: Vec::new(),
                skipped_changes: Vec::new(),
                errors: Vec::new(),
                warnings: vec!["Validation only - no changes applied".to_string()],
                backup_file: None,
            });
        }
        
        // Create backup if requested
        let backup_file = if request.backup_current {
            let config_path = Self::get_config_path()?;
            Some(Self::create_backup(&config_path).await?)
        } else {
            None
        };
        
        // Apply configuration
        let update_request = UpdateConfigRequest {
            config: imported_config,
            create_backup: false, // Already created above
        };
        
        match Self::update_config(update_request).await {
            Ok(_) => Ok(ImportResult {
                success: true,
                applied_changes: vec![], // TODO: Calculate actual changes
                skipped_changes: Vec::new(),
                errors: Vec::new(),
                warnings: Vec::new(),
                backup_file,
            }),
            Err(e) => Ok(ImportResult {
                success: false,
                applied_changes: Vec::new(),
                skipped_changes: Vec::new(),
                errors: vec![e.to_string()],
                warnings: Vec::new(),
                backup_file,
            }),
        }
    }
    
    // Private helper methods
    
    fn get_config_path() -> Result<String, WebConfigError> {
        // Try to get config path from environment variable or use default
        Ok(std::env::var("PALPO_CONFIG_PATH")
            .unwrap_or_else(|_| "palpo.toml".to_string()))
    }
    
    async fn get_file_modified_time(path: &str) -> Result<SystemTime, WebConfigError> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| WebConfigError::filesystem_with_path(format!("Failed to get file metadata: {}", e), path))?;
        
        metadata.modified()
            .map_err(|e| WebConfigError::filesystem_with_path(format!("Failed to get modification time: {}", e), path))
    }
    
    fn calculate_checksum(content: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
    
    async fn create_backup(config_path: &str) -> Result<String, WebConfigError> {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let backup_path = format!("{}.backup.{}", config_path, timestamp);
        
        tokio::fs::copy(config_path, &backup_path)
            .await
            .map_err(|e| WebConfigError::filesystem_with_path(format!("Failed to create backup: {}", e), config_path))?;
        
        Ok(backup_path)
    }
    
    async fn check_hot_reload_support() -> bool {
        // TODO: Implement actual hot reload detection
        // For now, assume hot reload is not supported
        false
    }
    
    async fn perform_hot_reload() -> Result<(), WebConfigError> {
        // TODO: Implement actual hot reload mechanism
        // This would typically involve sending a signal to the main server process
        Err(WebConfigError::server_control("Hot reload not implemented"))
    }
    
    fn sanitize_sensitive_data(config: &mut WebConfigData) {
        // Remove or mask sensitive information
        config.auth.jwt_secret = "***REDACTED***".to_string();
        config.database.connection_string = Self::mask_connection_string(&config.database.connection_string);
        
        for provider in &mut config.auth.oidc_providers {
            provider.client_secret = "***REDACTED***".to_string();
        }
    }
    
    fn mask_connection_string(connection_string: &str) -> String {
        // Mask password in connection string
        if let Some(at_pos) = connection_string.find('@') {
            if let Some(colon_pos) = connection_string[..at_pos].rfind(':') {
                let mut masked = connection_string.to_string();
                masked.replace_range(colon_pos + 1..at_pos, "***");
                return masked;
            }
        }
        connection_string.to_string()
    }
    
    // Validation methods
    
    fn validate_server_config(
        config: &ServerConfigSection,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        // Validate server name
        Self::validate_server_name(&config.server_name, errors, warnings);
        
        // Validate listeners
        if config.listeners.is_empty() {
            errors.push(ConfigError::new(
                "server.listeners",
                "At least one listener must be configured",
                "REQUIRED_FIELD"
            ));
        }
        
        for (i, listener) in config.listeners.iter().enumerate() {
            Self::validate_listener_config(listener, i, errors, warnings);
        }
        
        // Validate max request size
        if config.max_request_size == 0 {
            errors.push(Self::config_error(
                "server.max_request_size",
                "Max request size must be greater than 0"
            ));
        } else if config.max_request_size > 100 * 1024 * 1024 {
            warnings.push(Self::config_warning("server.max_request_size".to_string(), "Max request size is very large (>100MB), consider reducing for security".to_string(),
            ));
        }
    }
    
    fn validate_listener_config(
        listener: &ListenerConfig,
        index: usize,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        let field_prefix = format!("server.listeners[{}]", index);
        
        // Validate bind address
        if listener.bind.is_empty() {
            errors.push(Self::config_error(
                format!("{}.bind", field_prefix),
                "Bind address cannot be empty"
            ));
        }
        
        // Validate port
        if listener.port == 0 {
            errors.push(Self::config_error(
                format!("{}.port", field_prefix),
                "Port must be greater than 0"
            ));
        } else if listener.port < 1024 {
            warnings.push(Self::config_warning(
                format!("{}.port", field_prefix),
                "Using privileged port (<1024), ensure proper permissions"
            ));
        }
        
        // Validate TLS configuration
        if let Some(tls) = &listener.tls {
            Self::validate_tls_config(tls, &field_prefix, errors, warnings);
        }
        
        // Validate resources
        if listener.resources.is_empty() {
            warnings.push(Self::config_warning(
                format!("{}.resources", field_prefix),
                "No resources configured for listener"
            ));
        }
    }
    
    fn validate_tls_config(
        tls: &TlsConfig,
        field_prefix: &str,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        // Validate certificate path
        if tls.certificate_path.is_empty() {
            errors.push(Self::config_error(
                format!("{}.tls.certificate_path", field_prefix),
                "Certificate path cannot be empty"
            ));
        } else if !Path::new(&tls.certificate_path).exists() {
            warnings.push(Self::config_warning(
                format!("{}.tls.certificate_path", field_prefix),
                "Certificate file does not exist"
            ));
        }
        
        // Validate private key path
        if tls.private_key_path.is_empty() {
            errors.push(Self::config_error(
                format!("{}.tls.private_key_path", field_prefix),
                "Private key path cannot be empty"
            ));
        } else if !Path::new(&tls.private_key_path).exists() {
            warnings.push(Self::config_warning(
                format!("{}.tls.private_key_path", field_prefix),
                "Private key file does not exist"
            ));
        }
    }
    
    fn validate_database_config(
        config: &DatabaseConfigSection,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        // Validate connection string
        Self::validate_database_connection_string(&config.connection_string, errors, warnings);
        
        // Validate max connections
        if config.max_connections == 0 {
            errors.push(Self::config_error(
                "database.max_connections",
                "Max connections must be greater than 0"
            ));
        } else if config.max_connections > 100 {
            warnings.push(Self::config_warning(
                "database.max_connections",
                "Very high max connections, ensure database can handle the load"
            ));
        }
        
        // Validate connection timeout
        if config.connection_timeout == 0 {
            errors.push(Self::config_error(
                "database.connection_timeout",
                "Connection timeout must be greater than 0"
            ));
        }
        
        // Validate pool configuration
        if let Some(pool_size) = config.pool_size {
            if pool_size > config.max_connections {
                errors.push(Self::config_error(
                    "database.pool_size",
                    "Pool size cannot be greater than max connections"
                ));
            }
        }
        
        if let Some(min_idle) = config.min_idle {
            if let Some(pool_size) = config.pool_size {
                if min_idle > pool_size {
                    errors.push(Self::config_error(
                        "database.min_idle",
                        "Min idle cannot be greater than pool size"
                    ));
                }
            }
        }
    }
    
    fn validate_federation_config(
        config: &FederationConfigSection,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        // Validate signing key path
        Self::validate_file_path(&config.signing_key_path, errors, warnings);
        
        // Validate trusted servers
        for (i, server) in config.trusted_servers.iter().enumerate() {
            if server.is_empty() {
                errors.push(Self::config_error(
                    format!("federation.trusted_servers[{}]", i),
                    "Server name cannot be empty"
                ));
            } else if !Self::is_valid_server_name(server) {
                errors.push(Self::config_error(
                    format!("federation.trusted_servers[{}]", i),
                    "Invalid server name format"
                ));
            }
        }
        
        if config.enabled && config.trusted_servers.is_empty() {
            warnings.push(Self::config_warning(
                "federation.trusted_servers",
                "Federation enabled but no trusted servers configured"
            ));
        }
    }
    
    fn validate_auth_config(
        config: &AuthConfigSection,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        // Validate JWT secret
        Self::validate_jwt_secret(&config.jwt_secret, errors, warnings);
        
        // Validate JWT expiry
        if config.jwt_expiry == 0 {
            errors.push(Self::config_error(
                "auth.jwt_expiry",
                "JWT expiry must be greater than 0"
            ));
        } else if config.jwt_expiry < 300 {
            warnings.push(Self::config_warning(
                "auth.jwt_expiry",
                "Very short JWT expiry (<5 minutes), may cause frequent re-authentication"
            ));
        } else if config.jwt_expiry > 86400 {
            warnings.push(Self::config_warning(
                "auth.jwt_expiry",
                "Very long JWT expiry (>24 hours), consider security implications"
            ));
        }
        
        // Validate OIDC providers
        for (i, provider) in config.oidc_providers.iter().enumerate() {
            Self::validate_oidc_provider(provider, i, errors, warnings);
        }
        
        // Validate registration configuration
        if config.registration_enabled && matches!(config.registration_kind, RegistrationKind::Disabled) {
            errors.push(Self::config_error(
                "auth.registration_kind",
                "Registration enabled but kind is set to Disabled"
            ));
        }
    }
    
    fn validate_oidc_provider(
        provider: &OidcProvider,
        index: usize,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        let field_prefix = format!("auth.oidc_providers[{}]", index);
        
        if provider.name.is_empty() {
            errors.push(Self::config_error(
                format!("{}.name", field_prefix),
                "OIDC provider name cannot be empty"
            ));
        }

        if provider.issuer.is_empty() {
            errors.push(Self::config_error(
                format!("{}.issuer", field_prefix),
                "OIDC issuer cannot be empty"
            ));
        }
        
        if provider.client_id.is_empty() {
            errors.push(Self::config_error(
                format!("{}.client_id", field_prefix),
                "OIDC client ID cannot be empty"
            ));
        }
        
        if provider.client_secret.is_empty() {
            errors.push(Self::config_error(
                format!("{}.client_secret", field_prefix),
                "OIDC client secret cannot be empty"
            ));
        }
        
        if provider.scopes.is_empty() {
            warnings.push(Self::config_warning(
                format!("{}.scopes", field_prefix),
                "No OIDC scopes configured, may limit functionality"
            ));
        }
    }
    
    fn validate_media_config(
        config: &MediaConfigSection,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        // Validate storage path
        Self::validate_directory_path(&config.storage_path, errors, warnings);
        
        // Validate max file size
        if config.max_file_size == 0 {
            errors.push(Self::config_error(
                "media.max_file_size",
                "Max file size must be greater than 0"
            ));
        } else if config.max_file_size > 1024 * 1024 * 1024 {
            warnings.push(Self::config_warning(
                "media.max_file_size",
                "Very large max file size (>1GB), consider storage implications"
            ));
        }
        
        // Validate thumbnail sizes
        if config.thumbnail_sizes.is_empty() {
            warnings.push(Self::config_warning(
                "media.thumbnail_sizes",
                "No thumbnail sizes configured, thumbnails will not be generated"
            ));
        }
        
        for (i, thumbnail) in config.thumbnail_sizes.iter().enumerate() {
            if thumbnail.width == 0 || thumbnail.height == 0 {
                errors.push(Self::config_error(
                    format!("media.thumbnail_sizes[{}]", i),
                    "Thumbnail dimensions must be greater than 0"
                ));
            }
        }
    }
    
    fn validate_network_config(
        config: &NetworkConfigSection,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        // Validate timeouts
        if config.request_timeout == 0 {
            errors.push(Self::config_error(
                "network.request_timeout",
                "Request timeout must be greater than 0"
            ));
        }
        
        if config.connection_timeout == 0 {
            errors.push(Self::config_error(
                "network.connection_timeout",
                "Connection timeout must be greater than 0"
            ));
        }
        
        // Validate IP range denylist
        for (i, ip_range) in config.ip_range_denylist.iter().enumerate() {
            if !Self::is_valid_ip_range(ip_range) {
                errors.push(Self::config_error(
                    format!("network.ip_range_denylist[{}]", i),
                    "Invalid IP range format"
                ));
            }
        }
        
        // Validate CORS origins
        for (i, origin) in config.cors_origins.iter().enumerate() {
            if origin != "*" && !Self::is_valid_origin(origin) {
                warnings.push(Self::config_warning(
                    format!("network.cors_origins[{}]", i),
                    "Potentially invalid CORS origin format"
                ));
            }
        }
        
        if config.cors_origins.contains(&"*".to_string()) && config.cors_origins.len() > 1 {
            warnings.push(Self::config_warning(
                "network.cors_origins",
                "Wildcard CORS origin (*) makes other origins redundant"
            ));
        }
        
        // Validate rate limits
        if config.rate_limits.enabled {
            if config.rate_limits.requests_per_minute == 0 {
                errors.push(Self::config_error(
                    "network.rate_limits.requests_per_minute",
                    "Requests per minute must be greater than 0 when rate limiting is enabled"
                ));
            }
            
            if config.rate_limits.burst_size == 0 {
                errors.push(Self::config_error(
                    "network.rate_limits.burst_size",
                    "Burst size must be greater than 0 when rate limiting is enabled"
                ));
            }
        }
    }
    
    fn validate_logging_config(
        config: &LoggingConfigSection,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        // Validate log outputs
        if config.output.is_empty() {
            errors.push(Self::config_error(
                "logging.output",
                "At least one log output must be configured"
            ));
        }
        
        for (i, output) in config.output.iter().enumerate() {
            if let LogOutput::File(path) = output {
                if path.is_empty() {
                    errors.push(Self::config_error(
                        format!("logging.output[{}]", i),
                        "Log file path cannot be empty"
                    ));
                }
            }
        }
        
        // Validate rotation config
        if config.rotation.max_size_mb == 0 {
            warnings.push(Self::config_warning(
                "logging.rotation.max_size_mb",
                "Log rotation disabled (max_size_mb = 0), logs may grow indefinitely"
            ));
        }
        
        if config.rotation.max_files == 0 {
            warnings.push(Self::config_warning(
                "logging.rotation.max_files",
                "No log file retention (max_files = 0), old logs will not be kept"
            ));
        }
    }
    
    fn validate_config_dependencies(
        config: &WebConfigData,
        _errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        // Check if federation is enabled but no federation listener is configured
        if config.federation.enabled {
            let has_federation_listener = config.server.listeners.iter()
                .any(|l| l.resources.iter().any(|r| matches!(r, ListenerResource::Federation)));
            
            if !has_federation_listener {
                warnings.push(Self::config_warning(
                    "federation.enabled",
                    "Federation enabled but no federation listener configured"
                ));
            }
        }
        
        // Check if media storage path is accessible
        if !Path::new(&config.media.storage_path).exists() {
            warnings.push(Self::config_warning(
                "media.storage_path",
                "Media storage directory does not exist, it will be created on startup"
            ));
        }
        
        // Check if TLS is configured for production
        let has_tls = config.server.listeners.iter().any(|l| l.tls.is_some());
        if !has_tls && config.server.server_name != "localhost" {
            warnings.push(Self::config_warning(
                "server.listeners",
                "No TLS configured for production server, consider enabling HTTPS"
            ));
        }
    }
    
    // Field-specific validation methods
    
    fn validate_server_name(
        server_name: &str,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        if server_name.is_empty() {
            errors.push(Self::config_error(
                "server.server_name",
                "Server name cannot be empty"
            ));
        } else if !Self::is_valid_server_name(server_name) {
            errors.push(Self::config_error(
                "server.server_name",
                "Invalid server name format. Must be a valid domain name"
            ));
        } else if server_name == "localhost" {
            warnings.push(Self::config_warning(
                "server.server_name",
                "Using localhost as server name, not suitable for production"
            ));
        }
    }
    
    fn validate_database_connection_string(
        connection_string: &str,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        if connection_string.is_empty() {
            errors.push(Self::config_error(
                "database.connection_string",
                "Database connection string cannot be empty"
            ));
        } else if !connection_string.starts_with("postgresql://") {
            errors.push(Self::config_error(
                "database.connection_string",
                "Only PostgreSQL databases are supported"
            ));
        } else if connection_string.contains("password") && !connection_string.contains("***") {
            warnings.push(Self::config_warning(
                "database.connection_string",
                "Database password visible in connection string, consider using environment variables"
            ));
        }
    }
    
    fn validate_file_path(
        path: &str,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        if path.is_empty() {
            errors.push(Self::config_error(
                "file_path",
                "File path cannot be empty"
            ));
        } else if !Path::new(path).exists() {
            warnings.push(Self::config_warning(
                "file_path",
                "File does not exist"
            ));
        }
    }
    
    fn validate_directory_path(
        path: &str,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        if path.is_empty() {
            errors.push(Self::config_error(
                "directory_path",
                "Directory path cannot be empty"
            ));
        } else {
            let path_obj = Path::new(path);
            if path_obj.exists() && !path_obj.is_dir() {
                errors.push(Self::config_error(
                    "directory_path",
                    "Path exists but is not a directory"
                ));
            } else if !path_obj.exists() {
                warnings.push(Self::config_warning(
                    "directory_path",
                    "Directory does not exist, it will be created if needed"
                ));
            }
        }
    }
    
    fn validate_jwt_secret(
        secret: &str,
        errors: &mut Vec<ConfigError>,
        warnings: &mut Vec<ConfigWarning>,
    ) {
        if secret.is_empty() {
            errors.push(Self::config_error(
                "auth.jwt_secret",
                "JWT secret cannot be empty"
            ));
        } else if secret.len() < 32 {
            warnings.push(Self::config_warning(
                "auth.jwt_secret",
                "JWT secret is short (<32 characters), consider using a longer secret for better security"
            ));
        } else if secret == "change-me" || secret.contains("password") || secret.contains("secret") {
            errors.push(Self::config_error(
                "auth.jwt_secret",
                "JWT secret appears to be a default or weak value, please use a strong random secret"
            ));
        }
    }
    
    // Utility validation methods
    
    fn is_valid_server_name(server_name: &str) -> bool {
        // Basic domain name validation
        if server_name.is_empty() || server_name.len() > 253 {
            return false;
        }
        
        // Check for valid characters and structure
        server_name.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-') &&
        !server_name.starts_with('-') &&
        !server_name.ends_with('-') &&
        !server_name.starts_with('.') &&
        !server_name.ends_with('.') &&
        server_name.contains('.')
    }
    
    fn is_valid_ip_range(ip_range: &str) -> bool {
        // Basic IP range validation (supports CIDR notation)
        if ip_range.contains('/') {
            let parts: Vec<&str> = ip_range.split('/').collect();
            if parts.len() != 2 {
                return false;
            }
            
            // Validate IP part
            if !Self::is_valid_ip(parts[0]) {
                return false;
            }
            
            // Validate CIDR part
            if let Ok(cidr) = parts[1].parse::<u8>() {
                cidr <= 32 // IPv4 max
            } else {
                false
            }
        } else {
            Self::is_valid_ip(ip_range)
        }
    }
    
    fn is_valid_ip(ip: &str) -> bool {
        // Basic IP validation
        use std::net::IpAddr;
        ip.parse::<IpAddr>().is_ok()
    }
    
    fn is_valid_origin(origin: &str) -> bool {
        // Basic origin validation
        origin.starts_with("http://") || origin.starts_with("https://")
    }
}

// Request/Response models

#[derive(Serialize, Deserialize, Debug)]
pub struct ServerConfigResponse {
    pub config: WebConfigData,
    pub last_modified: SystemTime,
    pub checksum: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UpdateConfigRequest {
    pub config: WebConfigData,
    pub create_backup: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FieldValidationResult {
    pub valid: bool,
    pub errors: Vec<ConfigError>,
    pub warnings: Vec<ConfigWarning>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ConfigReloadResult {
    pub success: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub hot_reload_supported: bool,
    pub restart_required: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExportOptions {
    pub format: ConfigFormat,
    pub include_sensitive: bool,
    pub include_defaults: bool,
    pub sections: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ConfigExportResponse {
    pub content: String,
    pub format: ConfigFormat,
    pub exported_at: SystemTime,
    pub checksum: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ConfigImportRequest {
    pub content: String,
    pub format: ConfigFormat,
    pub merge_strategy: MergeStrategy,
    pub validate_only: bool,
    pub backup_current: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ImportResult {
    pub success: bool,
    pub applied_changes: Vec<ConfigChange>,
    pub skipped_changes: Vec<ConfigChange>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub backup_file: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ConfigChange {
    pub field: String,
    pub old_value: Option<serde_json::Value>,
    pub new_value: serde_json::Value,
    pub change_type: ChangeType,
    pub requires_restart: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ConfigFormat {
    Toml,
    Json,
    Yaml,
    Encrypted,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum MergeStrategy {
    Replace,
    Merge,
    KeepCurrent,
    Interactive,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ChangeType {
    Added,
    Modified,
    Removed,
}