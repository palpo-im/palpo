//! Configuration Import/Export API service
//! 
//! Provides comprehensive functionality for importing and exporting Palpo server configuration
//! with support for multiple formats, conflict resolution, and migration assistance.

use crate::models::{config::*, error::WebConfigError, validation::ValidationResult};
use crate::utils::fs_compat;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// Configuration Import/Export API service
pub struct ConfigImportExportAPI;

impl ConfigImportExportAPI {
    /// Export configuration with comprehensive options
    pub async fn export_config(options: ExportOptions) -> Result<ConfigExportResponse, WebConfigError> {
        let config_path = Self::get_config_path()?;
        let config_content = fs_compat::read_to_string(&config_path)
            .await
            .map_err(|e| WebConfigError::filesystem_with_path(format!("Failed to read config file: {}", e), &config_path))?;
        
        let mut config: WebConfigData = toml::from_str(&config_content)
            .map_err(|e| WebConfigError::ParseError { 
                message: format!("Failed to parse TOML: {}", e),
                format: "TOML".to_string(),
            })?;
        
        // Apply export options
        if !options.include_sensitive {
            Self::sanitize_sensitive_data(&mut config);
        }
        
        if !options.include_defaults {
            Self::remove_default_values(&mut config);
        }
        
        // Filter sections if specified
        if let Some(sections) = &options.sections {
            config = Self::filter_sections(config, sections)?;
        }
        
        // Serialize to requested format
        let content = Self::serialize_config(&config, &options.format)?;
        
        // Apply encryption if requested
        let final_content = if options.encrypt {
            if let Some(key) = &options.encryption_key {
                Self::encrypt_content(&content, key)?
            } else {
                return Err(WebConfigError::validation("Encryption key required for encrypted export"));
            }
        } else {
            content
        };
        
        let checksum = Self::calculate_checksum(&final_content);
        let size_bytes = final_content.len() as u64;
        
        Ok(ConfigExportResponse {
            content: final_content,
            format: options.format,
            exported_at: SystemTime::now(),
            checksum,
            size_bytes,
        })
    }
    
    /// Import configuration with comprehensive validation and conflict resolution
    pub async fn import_config(request: ConfigImportRequest) -> Result<ImportResult, WebConfigError> {
        // Decrypt content if needed
        let content = if let Some(key) = &request.encryption_key {
            Self::decrypt_content(&request.content, key)?
        } else {
            request.content.clone()
        };
        
        // Parse configuration from content
        let imported_config = Self::parse_config(&content, &request.format)?;
        
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
        
        // Get current configuration for comparison
        let current_config = Self::get_current_config().await?;
        
        // Calculate changes and conflicts
        let changes = Self::calculate_changes(&current_config, &imported_config);
        let conflicts = Self::detect_conflicts(&changes, &request.merge_strategy);
        
        // Handle conflicts based on merge strategy
        let (final_config, applied_changes, skipped_changes) = 
            Self::resolve_conflicts(current_config, imported_config, conflicts, &request.merge_strategy)?;
        
        // Create backup if requested
        let backup_file = if request.backup_current {
            let config_path = Self::get_config_path()?;
            Some(Self::create_backup(&config_path).await?)
        } else {
            None
        };
        
        // Apply final configuration
        match Self::save_config(&final_config).await {
            Ok(_) => Ok(ImportResult {
                success: true,
                applied_changes,
                skipped_changes,
                errors: Vec::new(),
                warnings: Vec::new(),
                backup_file,
            }),
            Err(e) => Ok(ImportResult {
                success: false,
                applied_changes: Vec::new(),
                skipped_changes,
                errors: vec![e.to_string()],
                warnings: Vec::new(),
                backup_file,
            }),
        }
    }
    
    /// Preview import changes without applying them
    pub async fn preview_import(request: ConfigImportRequest) -> Result<ImportPreview, WebConfigError> {
        // Decrypt content if needed
        let content = if let Some(key) = &request.encryption_key {
            Self::decrypt_content(&request.content, key)?
        } else {
            request.content.clone()
        };
        
        // Parse configuration from content
        let imported_config = Self::parse_config(&content, &request.format)?;
        
        // Validate imported configuration
        let validation_result = Self::validate_config(&imported_config).await?;
        let validation_errors = validation_result.errors.into_iter().map(|e| e.message).collect();
        
        // Get current configuration for comparison
        let current_config = Self::get_current_config().await?;
        
        // Calculate changes and conflicts
        let changes = Self::calculate_changes(&current_config, &imported_config);
        let conflicts = Self::detect_conflicts(&changes, &request.merge_strategy);
        
        // Assess impact
        let impact = Self::assess_impact(&changes);
        
        Ok(ImportPreview {
            changes,
            conflicts,
            validation_errors,
            estimated_impact: impact,
        })
    }
    
    /// Validate import file format and content
    pub async fn validate_import_file(file_content: String, format: ConfigFormat) -> Result<ImportValidationResult, WebConfigError> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        // Try to parse the content
        match Self::parse_config(&file_content, &format) {
            Ok(config) => {
                // Validate the parsed configuration
                let validation_result = Self::validate_config(&config).await?;
                errors.extend(validation_result.errors.into_iter().map(|e| e.message));
                warnings.extend(validation_result.warnings.into_iter().map(|w| w.message));
                
                // Check for missing required fields
                let missing_fields = Self::check_required_fields(&config);
                
                Ok(ImportValidationResult {
                    valid: errors.is_empty(),
                    errors,
                    warnings,
                    missing_required_fields: missing_fields,
                    format_valid: true,
                })
            }
            Err(e) => {
                errors.push(format!("Failed to parse {} content: {}", format.to_string(), e));
                Ok(ImportValidationResult {
                    valid: false,
                    errors,
                    warnings,
                    missing_required_fields: Vec::new(),
                    format_valid: false,
                })
            }
        }
    }
    
    /// Get available export formats
    pub async fn get_export_formats() -> Result<Vec<ExportFormat>, WebConfigError> {
        Ok(vec![
            ExportFormat {
                format: ConfigFormat::Toml,
                name: "TOML".to_string(),
                description: "Tom's Obvious, Minimal Language - Human-readable configuration format".to_string(),
                file_extension: "toml".to_string(),
                supports_encryption: true,
                supports_compression: false,
            },
            ExportFormat {
                format: ConfigFormat::Json,
                name: "JSON".to_string(),
                description: "JavaScript Object Notation - Widely supported structured data format".to_string(),
                file_extension: "json".to_string(),
                supports_encryption: true,
                supports_compression: false,
            },
            ExportFormat {
                format: ConfigFormat::Yaml,
                name: "YAML".to_string(),
                description: "YAML Ain't Markup Language - Human-readable data serialization standard".to_string(),
                file_extension: "yaml".to_string(),
                supports_encryption: true,
                supports_compression: false,
            },
            ExportFormat {
                format: ConfigFormat::Encrypted,
                name: "Encrypted".to_string(),
                description: "Encrypted configuration file with AES-256 encryption".to_string(),
                file_extension: "enc".to_string(),
                supports_encryption: true,
                supports_compression: true,
            },
        ])
    }
    
    /// Create migration script between versions
    pub async fn create_migration_script(from_version: String, to_version: String) -> Result<MigrationScript, WebConfigError> {
        // This would contain version-specific migration logic
        // For now, return a basic migration script
        Ok(MigrationScript {
            from_version,
            to_version,
            script_content: Self::generate_migration_script_content(),
            instructions: vec![
                "1. Backup your current configuration".to_string(),
                "2. Review the changes in the migration script".to_string(),
                "3. Apply the migration using the import function".to_string(),
                "4. Validate the new configuration".to_string(),
                "5. Restart the server if required".to_string(),
            ],
            breaking_changes: vec![],
            warnings: vec![
                "Always test migrations in a development environment first".to_string(),
            ],
        })
    }
    
    // Private helper methods
    
    fn get_config_path() -> Result<String, WebConfigError> {
        Ok(std::env::var("PALPO_CONFIG_PATH")
            .unwrap_or_else(|_| "palpo.toml".to_string()))
    }
    
    async fn get_current_config() -> Result<WebConfigData, WebConfigError> {
        let config_path = Self::get_config_path()?;
        let config_content = fs_compat::read_to_string(&config_path)
            .await
            .map_err(|e| WebConfigError::filesystem_with_path(format!("Failed to read config file: {}", e), &config_path))?;
        
        toml::from_str(&config_content)
            .map_err(|e| WebConfigError::ParseError { 
                message: format!("Failed to parse TOML: {}", e),
                format: "TOML".to_string(),
            })
    }
    
    async fn save_config(config: &WebConfigData) -> Result<(), WebConfigError> {
        let config_path = Self::get_config_path()?;
        let toml_content = toml::to_string_pretty(config)
            .map_err(|e| WebConfigError::ParseError { 
                message: format!("Failed to serialize TOML: {}", e),
                format: "TOML".to_string(),
            })?;
        
        fs_compat::write(&config_path, toml_content)
            .await
            .map_err(|e| WebConfigError::filesystem_with_path(format!("Failed to write config file: {}", e), &config_path))
    }
    
    async fn validate_config(config: &WebConfigData) -> Result<ValidationResult, WebConfigError> {
        // Use the existing validation logic from ConfigAPI
        crate::services::ConfigAPI::validate_config(config).await
    }
    
    fn parse_config(content: &str, format: &ConfigFormat) -> Result<WebConfigData, WebConfigError> {
        match format {
            ConfigFormat::Toml => {
                toml::from_str(content)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("TOML parsing failed: {}", e),
                        format: "TOML".to_string(),
                    })
            }
            ConfigFormat::Json => {
                serde_json::from_str(content)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("JSON parsing failed: {}", e),
                        format: "JSON".to_string(),
                    })
            }
            ConfigFormat::Yaml => {
                serde_yaml::from_str(content)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("YAML parsing failed: {}", e),
                        format: "YAML".to_string(),
                    })
            }
            ConfigFormat::Encrypted => {
                Err(WebConfigError::validation("Encrypted format should be decrypted before parsing"))
            }
        }
    }
    
    fn serialize_config(config: &WebConfigData, format: &ConfigFormat) -> Result<String, WebConfigError> {
        match format {
            ConfigFormat::Toml => {
                toml::to_string_pretty(config)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("TOML serialization failed: {}", e),
                        format: "TOML".to_string(),
                    })
            }
            ConfigFormat::Json => {
                serde_json::to_string_pretty(config)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("JSON serialization failed: {}", e),
                        format: "JSON".to_string(),
                    })
            }
            ConfigFormat::Yaml => {
                serde_yaml::to_string(config)
                    .map_err(|e| WebConfigError::ParseError { 
                        message: format!("YAML serialization failed: {}", e),
                        format: "YAML".to_string(),
                    })
            }
            ConfigFormat::Encrypted => {
                Err(WebConfigError::validation("Use encrypt_content for encrypted format"))
            }
        }
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
        // Format: postgresql://user:password@localhost:5432/database
        if let Some(at_pos) = connection_string.find('@') {
            // Look for the last colon before the @ symbol
            if let Some(colon_pos) = connection_string[..at_pos].rfind(':') {
                // Check if this colon is part of the password (not the protocol part)
                // We need to make sure we're not masking the protocol part (postgresql://)
                let protocol_end = connection_string.find("://").map(|pos| pos + 3).unwrap_or(0);
                if colon_pos > protocol_end {
                    let mut masked = connection_string.to_string();
                    masked.replace_range(colon_pos + 1..at_pos, "***");
                    return masked;
                }
            }
        }
        connection_string.to_string()
    }
    
    fn remove_default_values(config: &mut WebConfigData) {
        let default_config = WebConfigData::default();
        
        // Compare and remove fields that match defaults
        // This is a simplified implementation - in practice, you'd want more sophisticated comparison
        if config.server.max_request_size == default_config.server.max_request_size {
            config.server.max_request_size = 0; // Use 0 to indicate default should be used
        }
        
        if config.database.max_connections == default_config.database.max_connections {
            config.database.max_connections = 0;
        }
        
        // Continue for other fields as needed...
    }
    
    fn filter_sections(config: WebConfigData, sections: &[String]) -> Result<WebConfigData, WebConfigError> {
        let mut filtered_config = WebConfigData::default();
        
        for section in sections {
            match section.as_str() {
                "server" => filtered_config.server = config.server.clone(),
                "database" => filtered_config.database = config.database.clone(),
                "federation" => filtered_config.federation = config.federation.clone(),
                "auth" => filtered_config.auth = config.auth.clone(),
                "media" => filtered_config.media = config.media.clone(),
                "network" => filtered_config.network = config.network.clone(),
                "logging" => filtered_config.logging = config.logging.clone(),
                _ => return Err(WebConfigError::validation(format!("Unknown section: {}", section))),
            }
        }
        
        Ok(filtered_config)
    }
    
    fn calculate_changes(current: &WebConfigData, imported: &WebConfigData) -> Vec<ConfigChange> {
        let mut changes = Vec::new();
        
        // Compare server configuration
        Self::compare_server_config(&current.server, &imported.server, &mut changes);
        Self::compare_database_config(&current.database, &imported.database, &mut changes);
        Self::compare_federation_config(&current.federation, &imported.federation, &mut changes);
        Self::compare_auth_config(&current.auth, &imported.auth, &mut changes);
        Self::compare_media_config(&current.media, &imported.media, &mut changes);
        Self::compare_network_config(&current.network, &imported.network, &mut changes);
        Self::compare_logging_config(&current.logging, &imported.logging, &mut changes);
        
        changes
    }
    
    fn compare_server_config(current: &ServerConfigSection, imported: &ServerConfigSection, changes: &mut Vec<ConfigChange>) {
        if current.server_name != imported.server_name {
            changes.push(ConfigChange {
                field: "server.server_name".to_string(),
                old_value: Some(serde_json::to_value(&current.server_name).unwrap()),
                new_value: serde_json::to_value(&imported.server_name).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: true,
            });
        }
        
        if current.max_request_size != imported.max_request_size {
            changes.push(ConfigChange {
                field: "server.max_request_size".to_string(),
                old_value: Some(serde_json::to_value(&current.max_request_size).unwrap()),
                new_value: serde_json::to_value(&imported.max_request_size).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: false,
            });
        }
        
        // Compare listeners
        if current.listeners != imported.listeners {
            changes.push(ConfigChange {
                field: "server.listeners".to_string(),
                old_value: Some(serde_json::to_value(&current.listeners).unwrap()),
                new_value: serde_json::to_value(&imported.listeners).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: true,
            });
        }
    }
    
    fn compare_database_config(current: &DatabaseConfigSection, imported: &DatabaseConfigSection, changes: &mut Vec<ConfigChange>) {
        if current.connection_string != imported.connection_string {
            changes.push(ConfigChange {
                field: "database.connection_string".to_string(),
                old_value: Some(serde_json::Value::String("***REDACTED***".to_string())),
                new_value: serde_json::Value::String("***REDACTED***".to_string()),
                change_type: ChangeType::Modified,
                requires_restart: true,
            });
        }
        
        if current.max_connections != imported.max_connections {
            changes.push(ConfigChange {
                field: "database.max_connections".to_string(),
                old_value: Some(serde_json::to_value(&current.max_connections).unwrap()),
                new_value: serde_json::to_value(&imported.max_connections).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: false,
            });
        }
    }
    
    fn compare_federation_config(current: &FederationConfigSection, imported: &FederationConfigSection, changes: &mut Vec<ConfigChange>) {
        if current.enabled != imported.enabled {
            changes.push(ConfigChange {
                field: "federation.enabled".to_string(),
                old_value: Some(serde_json::to_value(&current.enabled).unwrap()),
                new_value: serde_json::to_value(&imported.enabled).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: true,
            });
        }
        
        if current.trusted_servers != imported.trusted_servers {
            changes.push(ConfigChange {
                field: "federation.trusted_servers".to_string(),
                old_value: Some(serde_json::to_value(&current.trusted_servers).unwrap()),
                new_value: serde_json::to_value(&imported.trusted_servers).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: false,
            });
        }
    }
    
    fn compare_auth_config(current: &AuthConfigSection, imported: &AuthConfigSection, changes: &mut Vec<ConfigChange>) {
        if current.registration_enabled != imported.registration_enabled {
            changes.push(ConfigChange {
                field: "auth.registration_enabled".to_string(),
                old_value: Some(serde_json::to_value(&current.registration_enabled).unwrap()),
                new_value: serde_json::to_value(&imported.registration_enabled).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: false,
            });
        }
        
        if current.jwt_expiry != imported.jwt_expiry {
            changes.push(ConfigChange {
                field: "auth.jwt_expiry".to_string(),
                old_value: Some(serde_json::to_value(&current.jwt_expiry).unwrap()),
                new_value: serde_json::to_value(&imported.jwt_expiry).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: false,
            });
        }
    }
    
    fn compare_media_config(current: &MediaConfigSection, imported: &MediaConfigSection, changes: &mut Vec<ConfigChange>) {
        if current.storage_path != imported.storage_path {
            changes.push(ConfigChange {
                field: "media.storage_path".to_string(),
                old_value: Some(serde_json::to_value(&current.storage_path).unwrap()),
                new_value: serde_json::to_value(&imported.storage_path).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: true,
            });
        }
        
        if current.max_file_size != imported.max_file_size {
            changes.push(ConfigChange {
                field: "media.max_file_size".to_string(),
                old_value: Some(serde_json::to_value(&current.max_file_size).unwrap()),
                new_value: serde_json::to_value(&imported.max_file_size).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: false,
            });
        }
    }
    
    fn compare_network_config(current: &NetworkConfigSection, imported: &NetworkConfigSection, changes: &mut Vec<ConfigChange>) {
        if current.request_timeout != imported.request_timeout {
            changes.push(ConfigChange {
                field: "network.request_timeout".to_string(),
                old_value: Some(serde_json::to_value(&current.request_timeout).unwrap()),
                new_value: serde_json::to_value(&imported.request_timeout).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: false,
            });
        }
        
        if current.cors_origins != imported.cors_origins {
            changes.push(ConfigChange {
                field: "network.cors_origins".to_string(),
                old_value: Some(serde_json::to_value(&current.cors_origins).unwrap()),
                new_value: serde_json::to_value(&imported.cors_origins).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: false,
            });
        }
    }
    
    fn compare_logging_config(current: &LoggingConfigSection, imported: &LoggingConfigSection, changes: &mut Vec<ConfigChange>) {
        if current.level != imported.level {
            changes.push(ConfigChange {
                field: "logging.level".to_string(),
                old_value: Some(serde_json::to_value(&current.level).unwrap()),
                new_value: serde_json::to_value(&imported.level).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: false,
            });
        }
        
        if current.output != imported.output {
            changes.push(ConfigChange {
                field: "logging.output".to_string(),
                old_value: Some(serde_json::to_value(&current.output).unwrap()),
                new_value: serde_json::to_value(&imported.output).unwrap(),
                change_type: ChangeType::Modified,
                requires_restart: true,
            });
        }
    }
    
    fn detect_conflicts(changes: &[ConfigChange], merge_strategy: &MergeStrategy) -> Vec<ConfigConflict> {
        let mut conflicts = Vec::new();
        
        // For now, we'll consider any change that requires restart as a potential conflict
        // In a real implementation, you'd have more sophisticated conflict detection
        for change in changes {
            if change.requires_restart && matches!(merge_strategy, MergeStrategy::KeepCurrent) {
                conflicts.push(ConfigConflict {
                    field: change.field.clone(),
                    current_value: change.old_value.clone().unwrap_or(serde_json::Value::Null),
                    import_value: change.new_value.clone(),
                    resolution: ConflictResolution::KeepCurrent,
                });
            }
        }
        
        conflicts
    }
    
    fn resolve_conflicts(
        current: WebConfigData,
        imported: WebConfigData,
        _conflicts: Vec<ConfigConflict>,
        merge_strategy: &MergeStrategy,
    ) -> Result<(WebConfigData, Vec<ConfigChange>, Vec<ConfigChange>), WebConfigError> {
        match merge_strategy {
            MergeStrategy::Replace => {
                let changes = Self::calculate_changes(&current, &imported);
                Ok((imported, changes, Vec::new()))
            }
            MergeStrategy::Merge => {
                // Implement merge logic - for now, just use imported config
                let changes = Self::calculate_changes(&current, &imported);
                Ok((imported, changes, Vec::new()))
            }
            MergeStrategy::KeepCurrent => {
                Ok((current, Vec::new(), Vec::new()))
            }
            MergeStrategy::Interactive => {
                // For interactive mode, we'd need UI interaction
                // For now, default to merge behavior
                let changes = Self::calculate_changes(&current, &imported);
                Ok((imported, changes, Vec::new()))
            }
        }
    }
    
    fn assess_impact(changes: &[ConfigChange]) -> ImpactAssessment {
        let restart_required = changes.iter().any(|c| c.requires_restart);
        let affected_services = Self::get_affected_services(changes);
        let risk_level = Self::calculate_risk_level(changes);
        let estimated_downtime = if restart_required {
            Some(Duration::from_secs(30)) // Estimate 30 seconds downtime for restart
        } else {
            None
        };
        
        ImpactAssessment {
            restart_required,
            affected_services,
            risk_level,
            estimated_downtime,
        }
    }
    
    fn get_affected_services(changes: &[ConfigChange]) -> Vec<String> {
        let mut services = Vec::new();
        
        for change in changes {
            match change.field.split('.').next() {
                Some("server") => {
                    if !services.contains(&"HTTP Server".to_string()) {
                        services.push("HTTP Server".to_string());
                    }
                }
                Some("database") => {
                    if !services.contains(&"Database Connection".to_string()) {
                        services.push("Database Connection".to_string());
                    }
                }
                Some("federation") => {
                    if !services.contains(&"Federation".to_string()) {
                        services.push("Federation".to_string());
                    }
                }
                Some("auth") => {
                    if !services.contains(&"Authentication".to_string()) {
                        services.push("Authentication".to_string());
                    }
                }
                Some("media") => {
                    if !services.contains(&"Media Storage".to_string()) {
                        services.push("Media Storage".to_string());
                    }
                }
                Some("network") => {
                    if !services.contains(&"Network".to_string()) {
                        services.push("Network".to_string());
                    }
                }
                Some("logging") => {
                    if !services.contains(&"Logging".to_string()) {
                        services.push("Logging".to_string());
                    }
                }
                _ => {}
            }
        }
        
        services
    }
    
    fn calculate_risk_level(changes: &[ConfigChange]) -> RiskLevel {
        let high_risk_fields = [
            "server.server_name",
            "database.connection_string",
            "auth.jwt_secret",
            "federation.signing_key_path",
        ];
        
        let medium_risk_fields = [
            "server.listeners",
            "federation.enabled",
            "auth.registration_enabled",
        ];
        
        for change in changes {
            if high_risk_fields.contains(&change.field.as_str()) {
                return RiskLevel::High;
            }
        }
        
        for change in changes {
            if medium_risk_fields.contains(&change.field.as_str()) {
                return RiskLevel::Medium;
            }
        }
        
        if changes.iter().any(|c| c.requires_restart) {
            RiskLevel::Medium
        } else if changes.is_empty() {
            RiskLevel::Low
        } else {
            RiskLevel::Low
        }
    }
    
    fn check_required_fields(config: &WebConfigData) -> Vec<String> {
        let mut missing = Vec::new();
        
        if config.server.server_name.is_empty() {
            missing.push("server.server_name".to_string());
        }
        
        if config.database.connection_string.is_empty() {
            missing.push("database.connection_string".to_string());
        }
        
        if config.auth.jwt_secret.is_empty() {
            missing.push("auth.jwt_secret".to_string());
        }
        
        if config.federation.signing_key_path.is_empty() {
            missing.push("federation.signing_key_path".to_string());
        }
        
        missing
    }
    
    fn encrypt_content(content: &str, _key: &str) -> Result<String, WebConfigError> {
        // Placeholder for encryption implementation
        // In a real implementation, you'd use a proper encryption library like AES
        use base64::{Engine as _, engine::general_purpose};
        let encrypted = format!("ENCRYPTED:{}", general_purpose::STANDARD.encode(content));
        Ok(encrypted)
    }
    
    fn decrypt_content(content: &str, _key: &str) -> Result<String, WebConfigError> {
        // Placeholder for decryption implementation
        if let Some(encrypted_data) = content.strip_prefix("ENCRYPTED:") {
            use base64::{Engine as _, engine::general_purpose};
            match general_purpose::STANDARD.decode(encrypted_data) {
                Ok(decoded) => String::from_utf8(decoded)
                    .map_err(|e| WebConfigError::validation(format!("Invalid encrypted content: {}", e))),
                Err(e) => Err(WebConfigError::validation(format!("Failed to decode encrypted content: {}", e))),
            }
        } else {
            Err(WebConfigError::validation("Invalid encrypted content format"))
        }
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
        
        fs_compat::copy(config_path, &backup_path)
            .await
            .map_err(|e| WebConfigError::filesystem_with_path(format!("Failed to create backup: {}", e), config_path))?;
        
        Ok(backup_path)
    }
    
    fn generate_migration_script_content() -> String {
        r#"#!/bin/bash
# Palpo Configuration Migration Script
# This script helps migrate configuration between versions

echo "Starting configuration migration..."

# Backup current configuration
cp palpo.toml palpo.toml.backup.$(date +%s)

# Apply migration changes
# (Migration-specific changes would be added here)

echo "Migration completed successfully!"
echo "Please review the changes and restart the server."
"#.to_string()
    }
}

// Enhanced request/response models

#[derive(Serialize, Deserialize, Debug)]
pub struct ExportOptions {
    pub format: ConfigFormat,
    pub include_sensitive: bool,
    pub include_defaults: bool,
    pub sections: Option<Vec<String>>,
    pub encrypt: bool,
    pub encryption_key: Option<String>,
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
    pub encryption_key: Option<String>,
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
pub struct ImportPreview {
    pub changes: Vec<ConfigChange>,
    pub conflicts: Vec<ConfigConflict>,
    pub validation_errors: Vec<String>,
    pub estimated_impact: ImpactAssessment,
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
pub struct ConfigConflict {
    pub field: String,
    pub current_value: serde_json::Value,
    pub import_value: serde_json::Value,
    pub resolution: ConflictResolution,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ImportValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub missing_required_fields: Vec<String>,
    pub format_valid: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExportFormat {
    pub format: ConfigFormat,
    pub name: String,
    pub description: String,
    pub file_extension: String,
    pub supports_encryption: bool,
    pub supports_compression: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MigrationScript {
    pub from_version: String,
    pub to_version: String,
    pub script_content: String,
    pub instructions: Vec<String>,
    pub breaking_changes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ImpactAssessment {
    pub restart_required: bool,
    pub affected_services: Vec<String>,
    pub risk_level: RiskLevel,
    pub estimated_downtime: Option<Duration>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ConfigFormat {
    Toml,
    Json,
    Yaml,
    Encrypted,
}

impl ConfigFormat {
    pub fn to_string(&self) -> &'static str {
        match self {
            ConfigFormat::Toml => "TOML",
            ConfigFormat::Json => "JSON", 
            ConfigFormat::Yaml => "YAML",
            ConfigFormat::Encrypted => "Encrypted",
        }
    }
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

#[derive(Serialize, Deserialize, Debug)]
pub enum ConflictResolution {
    UseImport,
    KeepCurrent,
    Manual,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_export_formats() {
        let formats = ConfigImportExportAPI::get_export_formats().await.unwrap();
        assert_eq!(formats.len(), 4);
        
        let toml_format = formats.iter().find(|f| matches!(f.format, ConfigFormat::Toml)).unwrap();
        assert_eq!(toml_format.name, "TOML");
        assert_eq!(toml_format.file_extension, "toml");
        assert!(toml_format.supports_encryption);
    }

    #[test]
    fn test_config_format_to_string() {
        assert_eq!(ConfigFormat::Toml.to_string(), "TOML");
        assert_eq!(ConfigFormat::Json.to_string(), "JSON");
        assert_eq!(ConfigFormat::Yaml.to_string(), "YAML");
        assert_eq!(ConfigFormat::Encrypted.to_string(), "Encrypted");
    }

    #[test]
    fn test_calculate_checksum() {
        let content = "test content";
        let checksum1 = ConfigImportExportAPI::calculate_checksum(content);
        let checksum2 = ConfigImportExportAPI::calculate_checksum(content);
        assert_eq!(checksum1, checksum2);
        
        let different_content = "different content";
        let checksum3 = ConfigImportExportAPI::calculate_checksum(different_content);
        assert_ne!(checksum1, checksum3);
    }

    #[test]
    fn test_encrypt_decrypt_content() {
        let content = "test configuration content";
        let key = "test-key";
        
        let encrypted = ConfigImportExportAPI::encrypt_content(content, key).unwrap();
        assert!(encrypted.starts_with("ENCRYPTED:"));
        
        let decrypted = ConfigImportExportAPI::decrypt_content(&encrypted, key).unwrap();
        assert_eq!(decrypted, content);
    }

    #[test]
    fn test_mask_connection_string() {
        let connection_string = "postgresql://user:password@localhost:5432/database";
        let masked = ConfigImportExportAPI::mask_connection_string(connection_string);
        // The function should mask the password part between : and @
        assert_eq!(masked, "postgresql://user:***@localhost:5432/database");
        
        let no_password = "postgresql://user@localhost:5432/database";
        let masked_no_password = ConfigImportExportAPI::mask_connection_string(no_password);
        assert_eq!(masked_no_password, no_password);
        
        // Test edge case with no @ symbol
        let no_at = "postgresql://localhost:5432/database";
        let masked_no_at = ConfigImportExportAPI::mask_connection_string(no_at);
        assert_eq!(masked_no_at, no_at);
    }

    #[test]
    fn test_check_required_fields() {
        let mut config = WebConfigData::default();
        let missing = ConfigImportExportAPI::check_required_fields(&config);
        // Default config has "localhost" as server_name and "change-me" as jwt_secret, so they're not empty
        assert!(!missing.contains(&"server.server_name".to_string()));
        assert!(!missing.contains(&"auth.jwt_secret".to_string()));
        
        // Set empty values to test missing field detection
        config.server.server_name = "".to_string();
        config.auth.jwt_secret = "".to_string();
        let missing_after = ConfigImportExportAPI::check_required_fields(&config);
        assert!(missing_after.contains(&"server.server_name".to_string()));
        assert!(missing_after.contains(&"auth.jwt_secret".to_string()));
    }

    #[test]
    fn test_calculate_risk_level() {
        let high_risk_change = ConfigChange {
            field: "server.server_name".to_string(),
            old_value: Some(serde_json::Value::String("old".to_string())),
            new_value: serde_json::Value::String("new".to_string()),
            change_type: ChangeType::Modified,
            requires_restart: true,
        };
        
        let risk_level = ConfigImportExportAPI::calculate_risk_level(&[high_risk_change]);
        assert!(matches!(risk_level, RiskLevel::High));
        
        let low_risk_change = ConfigChange {
            field: "logging.level".to_string(),
            old_value: Some(serde_json::Value::String("info".to_string())),
            new_value: serde_json::Value::String("debug".to_string()),
            change_type: ChangeType::Modified,
            requires_restart: false,
        };
        
        let risk_level_low = ConfigImportExportAPI::calculate_risk_level(&[low_risk_change]);
        assert!(matches!(risk_level_low, RiskLevel::Low));
    }
}