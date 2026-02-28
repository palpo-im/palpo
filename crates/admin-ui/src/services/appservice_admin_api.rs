//! Appservice administration API implementation

use crate::models::{
    Appservice, AppserviceNamespaces, AppserviceNamespace, RegisterAppserviceRequest, 
    RegisterAppserviceResponse, UpdateAppserviceRequest, UpdateAppserviceResponse,
    UnregisterAppserviceRequest, UnregisterAppserviceResponse, TestAppserviceRequest,
    TestAppserviceResponse, ListAppservicesRequest, ListAppservicesResponse,
    AppserviceStatistics, ValidateYamlConfigRequest, ValidateYamlConfigResponse,
    AppserviceDetailResponse, AppserviceActivity, AppserviceActivityType,
    AppserviceSortField, AppserviceSortOrder, AppserviceTestType, 
    WebConfigError, AuditAction, AuditTargetType, generate_token, validate_regex,
    parse_yaml_config, generate_yaml_config,
};
use crate::utils::audit_logger::AuditLogger;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Appservice administration API service
#[derive(Clone)]
pub struct AppserviceAdminAPI {
    audit_logger: AuditLogger,
    // In a real implementation, this would connect to the Matrix server's database
    // For now, we'll use in-memory storage for demonstration
    appservices: std::sync::Arc<std::sync::RwLock<HashMap<String, Appservice>>>,
    activities: std::sync::Arc<std::sync::RwLock<Vec<AppserviceActivity>>>,
}

impl AppserviceAdminAPI {
    /// Create a new AppserviceAdminAPI instance
    pub fn new(audit_logger: AuditLogger) -> Self {
        let appservices = std::sync::Arc::new(std::sync::RwLock::new(HashMap::new()));
        let activities = std::sync::Arc::new(std::sync::RwLock::new(Vec::new()));
        
        // Add some sample appservices for demonstration
        let mut appservices_map = appservices.write().unwrap();
        
        // Sample bridge appservice
        appservices_map.insert(
            "telegram-bridge".to_string(),
            Appservice {
                id: "telegram-bridge".to_string(),
                url: "http://localhost:8080".to_string(),
                as_token: "as_token_telegram_bridge_12345".to_string(),
                hs_token: "hs_token_telegram_bridge_67890".to_string(),
                sender_localpart: "telegrambot".to_string(),
                namespaces: AppserviceNamespaces {
                    users: vec![
                        AppserviceNamespace {
                            exclusive: true,
                            regex: "@telegram_.*:example.com".to_string(),
                        }
                    ],
                    aliases: vec![
                        AppserviceNamespace {
                            exclusive: true,
                            regex: "#telegram_.*:example.com".to_string(),
                        }
                    ],
                    rooms: vec![],
                },
                rate_limited: Some(false),
                protocols: Some(vec!["telegram".to_string()]),
                is_active: true,
                created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 86400,
                last_ping: Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 300),
                last_error: None,
            }
        );
        
        // Sample IRC bridge appservice
        appservices_map.insert(
            "irc-bridge".to_string(),
            Appservice {
                id: "irc-bridge".to_string(),
                url: "http://localhost:8081".to_string(),
                as_token: "as_token_irc_bridge_abcde".to_string(),
                hs_token: "hs_token_irc_bridge_fghij".to_string(),
                sender_localpart: "ircbot".to_string(),
                namespaces: AppserviceNamespaces {
                    users: vec![
                        AppserviceNamespace {
                            exclusive: true,
                            regex: "@irc_.*:example.com".to_string(),
                        }
                    ],
                    aliases: vec![
                        AppserviceNamespace {
                            exclusive: true,
                            regex: "#irc_.*:example.com".to_string(),
                        }
                    ],
                    rooms: vec![],
                },
                rate_limited: Some(true),
                protocols: Some(vec!["irc".to_string()]),
                is_active: false,
                created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 172800,
                last_ping: Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 3600),
                last_error: Some("Connection timeout".to_string()),
            }
        );
        
        drop(appservices_map);
        
        Self {
            audit_logger,
            appservices,
            activities,
        }
    }

    /// List appservices with filtering and pagination
    pub async fn list_appservices(&self, request: ListAppservicesRequest, admin_user: &str) -> Result<ListAppservicesResponse, WebConfigError> {
        // Check permissions
        if !self.has_appservice_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for appservice management"));
        }

        let appservices = self.appservices.read().map_err(|_| WebConfigError::internal("Failed to read appservices"))?;
        
        let mut filtered_appservices: Vec<Appservice> = appservices.values().cloned().collect();
        
        // Apply filters
        if let Some(search) = &request.search {
            let search_lower = search.to_lowercase();
            filtered_appservices.retain(|appservice| {
                appservice.id.to_lowercase().contains(&search_lower) ||
                appservice.url.to_lowercase().contains(&search_lower) ||
                appservice.sender_localpart.to_lowercase().contains(&search_lower)
            });
        }
        
        if let Some(filter_active) = request.filter_active {
            filtered_appservices.retain(|appservice| appservice.is_active == filter_active);
        }
        
        // Apply sorting
        if let Some(sort_by) = &request.sort_by {
            let ascending = matches!(request.sort_order, Some(AppserviceSortOrder::Ascending) | None);
            
            filtered_appservices.sort_by(|a, b| {
                let cmp = match sort_by {
                    AppserviceSortField::Id => a.id.cmp(&b.id),
                    AppserviceSortField::Url => a.url.cmp(&b.url),
                    AppserviceSortField::CreatedAt => a.created_at.cmp(&b.created_at),
                    AppserviceSortField::LastPing => {
                        a.last_ping.unwrap_or(0).cmp(&b.last_ping.unwrap_or(0))
                    },
                    AppserviceSortField::IsActive => a.is_active.cmp(&b.is_active),
                };
                
                if ascending { cmp } else { cmp.reverse() }
            });
        }
        
        let total_count = filtered_appservices.len() as u32;
        
        // Apply pagination
        let offset = request.offset.unwrap_or(0) as usize;
        let limit = request.limit.unwrap_or(50) as usize;
        
        let paginated_appservices: Vec<Appservice> = filtered_appservices
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect();
        
        let has_more = (offset + paginated_appservices.len()) < total_count as usize;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "appservice_list",
            Some(serde_json::json!({
                "filter": {
                    "search": request.search,
                    "active": request.filter_active
                },
                "pagination": {
                    "offset": request.offset,
                    "limit": request.limit
                },
                "result_count": paginated_appservices.len()
            })),
            "Listed appservices with filters",
        ).await;
        
        Ok(ListAppservicesResponse {
            success: true,
            appservices: paginated_appservices,
            total_count,
            has_more,
            error: None,
        })
    }

    /// Get appservice statistics
    pub async fn get_appservice_statistics(&self, admin_user: &str) -> Result<AppserviceStatistics, WebConfigError> {
        // Check permissions
        if !self.has_appservice_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for appservice management"));
        }

        let appservices = self.appservices.read().map_err(|_| WebConfigError::internal("Failed to read appservices"))?;
        
        let mut stats = AppserviceStatistics {
            total_appservices: 0,
            active_appservices: 0,
            inactive_appservices: 0,
            appservices_with_errors: 0,
            total_managed_users: 0,
            total_managed_aliases: 0,
            total_managed_rooms: 0,
        };
        
        for appservice in appservices.values() {
            stats.total_appservices += 1;
            
            if appservice.is_active {
                stats.active_appservices += 1;
            } else {
                stats.inactive_appservices += 1;
            }
            
            if appservice.last_error.is_some() {
                stats.appservices_with_errors += 1;
            }
            
            stats.total_managed_users += appservice.namespaces.users.len() as u32;
            stats.total_managed_aliases += appservice.namespaces.aliases.len() as u32;
            stats.total_managed_rooms += appservice.namespaces.rooms.len() as u32;
        }
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "appservice_statistics",
            Some(serde_json::json!(stats)),
            "Retrieved appservice statistics",
        ).await;
        
        Ok(stats)
    }

    /// Register a new appservice
    pub async fn register_appservice(&self, request: RegisterAppserviceRequest, admin_user: &str) -> Result<RegisterAppserviceResponse, WebConfigError> {
        // Check permissions
        if !self.has_appservice_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for appservice management"));
        }

        // Validate appservice ID
        if request.id.is_empty() || request.id.len() > 255 {
            return Ok(RegisterAppserviceResponse {
                success: false,
                appservice: None,
                generated_as_token: None,
                generated_hs_token: None,
                error: Some("Appservice ID must be between 1 and 255 characters".to_string()),
            });
        }
        
        // Validate URL
        if request.url.is_empty() || !request.url.starts_with("http") {
            return Ok(RegisterAppserviceResponse {
                success: false,
                appservice: None,
                generated_as_token: None,
                generated_hs_token: None,
                error: Some("Invalid URL format".to_string()),
            });
        }
        
        // Validate namespaces
        if let Err(error) = self.validate_namespaces(&request.namespaces) {
            return Ok(RegisterAppserviceResponse {
                success: false,
                appservice: None,
                generated_as_token: None,
                generated_hs_token: None,
                error: Some(error),
            });
        }
        
        // Check if appservice already exists
        let appservices = self.appservices.read().map_err(|_| WebConfigError::internal("Failed to read appservices"))?;
        if appservices.contains_key(&request.id) {
            return Ok(RegisterAppserviceResponse {
                success: false,
                appservice: None,
                generated_as_token: None,
                generated_hs_token: None,
                error: Some("Appservice with this ID already exists".to_string()),
            });
        }
        drop(appservices);
        
        // Generate tokens if not provided
        let generated_as_token = if request.as_token.is_none() {
            Some(generate_token())
        } else {
            None
        };
        
        let generated_hs_token = if request.hs_token.is_none() {
            Some(generate_token())
        } else {
            None
        };
        
        // Create appservice
        let appservice = Appservice {
            id: request.id.clone(),
            url: request.url.clone(),
            as_token: request.as_token.unwrap_or_else(|| generated_as_token.clone().unwrap()),
            hs_token: request.hs_token.unwrap_or_else(|| generated_hs_token.clone().unwrap()),
            sender_localpart: request.sender_localpart.clone(),
            namespaces: request.namespaces.clone(),
            rate_limited: request.rate_limited,
            protocols: request.protocols.clone(),
            is_active: true,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            last_ping: None,
            last_error: None,
        };
        
        // Store appservice
        let mut appservices = self.appservices.write().map_err(|_| WebConfigError::internal("Failed to write appservices"))?;
        appservices.insert(request.id.clone(), appservice.clone());
        drop(appservices);
        
        // Log activity
        self.log_activity(AppserviceActivity {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            activity_type: AppserviceActivityType::Registration,
            description: format!("Registered appservice {}", request.id),
            success: true,
            error: None,
        }).await;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::AppserviceRegister,
            AuditTargetType::Config,
            &request.id,
            Some(serde_json::json!({
                "id": request.id,
                "url": request.url,
                "sender_localpart": request.sender_localpart,
                "namespaces": request.namespaces,
                "tokens_generated": {
                    "as_token": generated_as_token.is_some(),
                    "hs_token": generated_hs_token.is_some()
                }
            })),
            &format!("Registered appservice {}", request.id),
        ).await;
        
        Ok(RegisterAppserviceResponse {
            success: true,
            appservice: Some(appservice),
            generated_as_token,
            generated_hs_token,
            error: None,
        })
    }

    /// Update appservice configuration
    pub async fn update_appservice(&self, appservice_id: &str, request: UpdateAppserviceRequest, admin_user: &str) -> Result<UpdateAppserviceResponse, WebConfigError> {
        // Check permissions
        if !self.has_appservice_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for appservice management"));
        }

        let mut appservices = self.appservices.write().map_err(|_| WebConfigError::internal("Failed to write appservices"))?;
        
        let appservice = appservices.get_mut(appservice_id).ok_or_else(|| {
            WebConfigError::validation(format!("Appservice {} not found", appservice_id))
        })?;
        
        // Store original values for audit log
        let original_values = serde_json::json!({
            "url": appservice.url,
            "namespaces": appservice.namespaces,
            "rate_limited": appservice.rate_limited,
            "protocols": appservice.protocols
        });
        
        // Update fields
        if let Some(url) = request.url {
            if !url.starts_with("http") {
                return Ok(UpdateAppserviceResponse {
                    success: false,
                    appservice: None,
                    error: Some("Invalid URL format".to_string()),
                });
            }
            appservice.url = url;
        }
        
        if let Some(namespaces) = request.namespaces {
            if let Err(error) = self.validate_namespaces(&namespaces) {
                return Ok(UpdateAppserviceResponse {
                    success: false,
                    appservice: None,
                    error: Some(error),
                });
            }
            appservice.namespaces = namespaces;
        }
        
        if let Some(rate_limited) = request.rate_limited {
            appservice.rate_limited = Some(rate_limited);
        }
        
        if let Some(protocols) = request.protocols {
            appservice.protocols = Some(protocols);
        }
        
        let updated_appservice = appservice.clone();
        drop(appservices);
        
        // Log activity
        self.log_activity(AppserviceActivity {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            activity_type: AppserviceActivityType::ConfigUpdate,
            description: format!("Updated appservice {}", appservice_id),
            success: true,
            error: None,
        }).await;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            appservice_id,
            Some(serde_json::json!({
                "original": original_values,
                "updated": {
                    "url": updated_appservice.url,
                    "namespaces": updated_appservice.namespaces,
                    "rate_limited": updated_appservice.rate_limited,
                    "protocols": updated_appservice.protocols
                }
            })),
            &format!("Updated appservice {}", appservice_id),
        ).await;
        
        Ok(UpdateAppserviceResponse {
            success: true,
            appservice: Some(updated_appservice),
            error: None,
        })
    }

    /// Unregister an appservice
    pub async fn unregister_appservice(&self, request: UnregisterAppserviceRequest, admin_user: &str) -> Result<UnregisterAppserviceResponse, WebConfigError> {
        // Check permissions
        if !self.has_appservice_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for appservice management"));
        }

        let mut appservices = self.appservices.write().map_err(|_| WebConfigError::internal("Failed to write appservices"))?;
        
        let appservice = appservices.get(&request.id).ok_or_else(|| {
            WebConfigError::validation(format!("Appservice {} not found", request.id))
        })?.clone();
        
        appservices.remove(&request.id);
        drop(appservices);
        
        // In a real implementation, this would:
        // - Clean up managed users, aliases, and rooms if requested
        // - Notify the appservice of unregistration
        // - Remove from homeserver configuration
        
        // Log activity
        self.log_activity(AppserviceActivity {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            activity_type: AppserviceActivityType::Unregistration,
            description: format!("Unregistered appservice {}", request.id),
            success: true,
            error: None,
        }).await;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::AppserviceUnregister,
            AuditTargetType::Config,
            &request.id,
            Some(serde_json::json!({
                "id": request.id,
                "cleanup_data": request.cleanup_data,
                "appservice": appservice
            })),
            &format!("Unregistered appservice {}", request.id),
        ).await;
        
        Ok(UnregisterAppserviceResponse {
            success: true,
            error: None,
        })
    }

    /// Test appservice connectivity and functionality
    pub async fn test_appservice(&self, request: TestAppserviceRequest, admin_user: &str) -> Result<TestAppserviceResponse, WebConfigError> {
        // Check permissions
        if !self.has_appservice_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for appservice management"));
        }

        let appservices = self.appservices.read().map_err(|_| WebConfigError::internal("Failed to read appservices"))?;
        
        let appservice = appservices.get(&request.id).ok_or_else(|| {
            WebConfigError::validation(format!("Appservice {} not found", request.id))
        })?;
        
        let start_time = SystemTime::now();
        
        // In a real implementation, this would make actual HTTP requests to the appservice
        // For now, we'll simulate the test based on the appservice's current state
        let (success, response_data, error) = match request.test_type {
            AppserviceTestType::Ping => {
                if appservice.is_active {
                    (true, Some(serde_json::json!({"status": "ok"})), None)
                } else {
                    (false, None, Some("Appservice is inactive".to_string()))
                }
            },
            AppserviceTestType::UserQuery => {
                if appservice.namespaces.users.is_empty() {
                    (false, None, Some("No user namespaces configured".to_string()))
                } else {
                    (true, Some(serde_json::json!({"user_id": "@test:example.com", "exists": false})), None)
                }
            },
            AppserviceTestType::AliasQuery => {
                if appservice.namespaces.aliases.is_empty() {
                    (false, None, Some("No alias namespaces configured".to_string()))
                } else {
                    (true, Some(serde_json::json!({"alias": "#test:example.com", "exists": false})), None)
                }
            },
            AppserviceTestType::RoomQuery => {
                if appservice.namespaces.rooms.is_empty() {
                    (false, None, Some("No room namespaces configured".to_string()))
                } else {
                    (true, Some(serde_json::json!({"room_id": "!test:example.com", "exists": false})), None)
                }
            },
        };
        
        let response_time_ms = start_time.elapsed()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        
        drop(appservices);
        
        // Update appservice last_ping and error status
        let mut appservices = self.appservices.write().map_err(|_| WebConfigError::internal("Failed to write appservices"))?;
        if let Some(appservice) = appservices.get_mut(&request.id) {
            appservice.last_ping = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
            if success {
                appservice.last_error = None;
            } else {
                appservice.last_error = error.clone();
            }
        }
        drop(appservices);
        
        // Log activity
        self.log_activity(AppserviceActivity {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            activity_type: match request.test_type {
                AppserviceTestType::Ping => AppserviceActivityType::Ping,
                AppserviceTestType::UserQuery => AppserviceActivityType::UserQuery,
                AppserviceTestType::AliasQuery => AppserviceActivityType::AliasQuery,
                AppserviceTestType::RoomQuery => AppserviceActivityType::RoomQuery,
            },
            description: format!("Tested appservice {} with {}", request.id, request.test_type.description()),
            success,
            error: error.clone(),
        }).await;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            &request.id,
            Some(serde_json::json!({
                "test_type": request.test_type,
                "success": success,
                "response_time_ms": response_time_ms,
                "error": error
            })),
            &format!("Tested appservice {} with {}", request.id, request.test_type.description()),
        ).await;
        
        Ok(TestAppserviceResponse {
            success,
            test_type: request.test_type,
            response_time_ms: Some(response_time_ms),
            response_data,
            error,
        })
    }

    /// Validate YAML configuration
    pub async fn validate_yaml_config(&self, request: ValidateYamlConfigRequest, admin_user: &str) -> Result<ValidateYamlConfigResponse, WebConfigError> {
        // Check permissions
        if !self.has_appservice_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for appservice management"));
        }

        let mut validation_errors = Vec::new();
        let mut warnings = Vec::new();
        
        // Parse YAML
        let parsed_config = match parse_yaml_config(&request.yaml_content) {
            Ok(config) => {
                // Validate parsed configuration
                if config.id.is_empty() {
                    validation_errors.push("Appservice ID cannot be empty".to_string());
                }
                
                if !config.url.starts_with("http") {
                    validation_errors.push("Invalid URL format".to_string());
                }
                
                if config.as_token.is_empty() {
                    validation_errors.push("AS token cannot be empty".to_string());
                }
                
                if config.hs_token.is_empty() {
                    validation_errors.push("HS token cannot be empty".to_string());
                }
                
                if config.sender_localpart.is_empty() {
                    validation_errors.push("Sender localpart cannot be empty".to_string());
                }
                
                // Validate namespaces
                if let Err(error) = self.validate_namespaces(&config.namespaces) {
                    validation_errors.push(error);
                }
                
                // Check for warnings
                if config.namespaces.users.is_empty() && config.namespaces.aliases.is_empty() && config.namespaces.rooms.is_empty() {
                    warnings.push("No namespaces configured - appservice will not manage any users, aliases, or rooms".to_string());
                }
                
                if config.rate_limited.unwrap_or(true) {
                    warnings.push("Rate limiting is enabled - this may affect performance".to_string());
                }
                
                Some(config)
            },
            Err(error) => {
                validation_errors.push(error);
                None
            }
        };
        
        let success = validation_errors.is_empty();
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            "yaml_validation",
            Some(serde_json::json!({
                "success": success,
                "errors_count": validation_errors.len(),
                "warnings_count": warnings.len()
            })),
            "Validated YAML configuration",
        ).await;
        
        Ok(ValidateYamlConfigResponse {
            success,
            parsed_config,
            validation_errors,
            warnings,
        })
    }

    /// Get detailed appservice information
    pub async fn get_appservice_detail(&self, appservice_id: &str, admin_user: &str) -> Result<AppserviceDetailResponse, WebConfigError> {
        // Check permissions
        if !self.has_appservice_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for appservice management"));
        }

        let appservices = self.appservices.read().map_err(|_| WebConfigError::internal("Failed to read appservices"))?;
        
        let appservice = appservices.get(appservice_id).ok_or_else(|| {
            WebConfigError::validation(format!("Appservice {} not found", appservice_id))
        })?.clone();
        
        drop(appservices);
        
        // Generate YAML configuration
        let yaml_config = generate_yaml_config(&appservice)
            .map_err(|e| WebConfigError::internal(&format!("Failed to generate YAML: {}", e)))?;
        
        // Get statistics (simplified for single appservice)
        let statistics = AppserviceStatistics {
            total_appservices: 1,
            active_appservices: if appservice.is_active { 1 } else { 0 },
            inactive_appservices: if !appservice.is_active { 1 } else { 0 },
            appservices_with_errors: if appservice.last_error.is_some() { 1 } else { 0 },
            total_managed_users: appservice.namespaces.users.len() as u32,
            total_managed_aliases: appservice.namespaces.aliases.len() as u32,
            total_managed_rooms: appservice.namespaces.rooms.len() as u32,
        };
        
        // Get recent activity
        let activities = self.activities.read().map_err(|_| WebConfigError::internal("Failed to read activities"))?;
        let recent_activity: Vec<AppserviceActivity> = activities
            .iter()
            .filter(|activity| activity.description.contains(appservice_id))
            .take(10)
            .cloned()
            .collect();
        drop(activities);
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::ConfigUpdate,
            AuditTargetType::Config,
            appservice_id,
            Some(serde_json::json!({
                "appservice_id": appservice_id
            })),
            &format!("Retrieved detailed information for appservice {}", appservice_id),
        ).await;
        
        Ok(AppserviceDetailResponse {
            appservice,
            yaml_config,
            statistics,
            recent_activity,
        })
    }

    /// Validate namespace configuration
    fn validate_namespaces(&self, namespaces: &AppserviceNamespaces) -> Result<(), String> {
        // Validate user namespaces
        for namespace in &namespaces.users {
            validate_regex(&namespace.regex)?;
        }
        
        // Validate alias namespaces
        for namespace in &namespaces.aliases {
            validate_regex(&namespace.regex)?;
        }
        
        // Validate room namespaces
        for namespace in &namespaces.rooms {
            validate_regex(&namespace.regex)?;
        }
        
        Ok(())
    }

    /// Log appservice activity
    async fn log_activity(&self, activity: AppserviceActivity) {
        if let Ok(mut activities) = self.activities.write() {
            activities.push(activity);
            
            // Keep only the last 1000 activities
            if activities.len() > 1000 {
                let len = activities.len();
                activities.drain(0..len - 1000);
            }
        }
    }

    /// Check if the admin user has appservice management permissions
    async fn has_appservice_management_permission(&self, _admin_user: &str) -> Result<bool, WebConfigError> {
        // In a real implementation, this would check the admin user's permissions
        // For now, we'll assume all admin users have appservice management permissions
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::audit_logger::AuditLogger;

    fn create_test_api() -> AppserviceAdminAPI {
        let audit_logger = AuditLogger::new(1000);
        AppserviceAdminAPI::new(audit_logger)
    }

    #[tokio::test]
    async fn test_list_appservices() {
        let api = create_test_api();
        let request = ListAppservicesRequest::default();
        
        let response = api.list_appservices(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert_eq!(response.appservices.len(), 2); // telegram-bridge and irc-bridge
        assert_eq!(response.total_count, 2);
    }

    #[tokio::test]
    async fn test_register_appservice() {
        let api = create_test_api();
        let request = RegisterAppserviceRequest {
            id: "discord-bridge".to_string(),
            url: "http://localhost:8082".to_string(),
            as_token: None, // Auto-generate token
            hs_token: None, // Auto-generate token
            sender_localpart: "discordbot".to_string(),
            namespaces: AppserviceNamespaces {
                users: vec![
                    AppserviceNamespace {
                        exclusive: true,
                        regex: "@discord_.*:example.com".to_string(),
                    }
                ],
                aliases: vec![],
                rooms: vec![],
            },
            rate_limited: Some(false),
            protocols: Some(vec!["discord".to_string()]),
            yaml_config: None,
        };
        
        let response = api.register_appservice(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.appservice.is_some());
        assert!(response.generated_as_token.is_some());
        assert!(response.generated_hs_token.is_some());
        
        let appservice = response.appservice.unwrap();
        assert_eq!(appservice.id, "discord-bridge");
        assert_eq!(appservice.url, "http://localhost:8082");
        assert!(appservice.is_active);
    }

    #[tokio::test]
    async fn test_update_appservice() {
        let api = create_test_api();
        let request = UpdateAppserviceRequest {
            url: Some("http://localhost:8083".to_string()),
            namespaces: None,
            rate_limited: Some(true),
            protocols: Some(vec!["telegram".to_string(), "telegram-bot".to_string()]),
            yaml_config: None,
        };
        
        let response = api.update_appservice("telegram-bridge", request, "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.appservice.is_some());
        
        let appservice = response.appservice.unwrap();
        assert_eq!(appservice.url, "http://localhost:8083");
        assert_eq!(appservice.rate_limited, Some(true));
        assert_eq!(appservice.protocols, Some(vec!["telegram".to_string(), "telegram-bot".to_string()]));
    }

    #[tokio::test]
    async fn test_unregister_appservice() {
        let api = create_test_api();
        let request = UnregisterAppserviceRequest {
            id: "irc-bridge".to_string(),
            cleanup_data: true,
        };
        
        let response = api.unregister_appservice(request, "admin").await.unwrap();
        
        assert!(response.success);
        
        // Verify appservice is removed
        let appservices = api.appservices.read().unwrap();
        assert!(!appservices.contains_key("irc-bridge"));
    }

    #[tokio::test]
    async fn test_test_appservice() {
        let api = create_test_api();
        let request = TestAppserviceRequest {
            id: "telegram-bridge".to_string(),
            test_type: AppserviceTestType::Ping,
        };
        
        let response = api.test_appservice(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert_eq!(response.test_type, AppserviceTestType::Ping);
        assert!(response.response_time_ms.is_some());
        assert!(response.response_data.is_some());
    }

    #[tokio::test]
    async fn test_validate_yaml_config() {
        let api = create_test_api();
        let yaml_content = r#"
id: test-bridge
url: http://localhost:8080
as_token: test_as_token
hs_token: test_hs_token
sender_localpart: testbot
namespaces:
  users:
    - exclusive: true
      regex: "@test_.*:example.com"
  aliases: []
  rooms: []
rate_limited: false
protocols:
  - test
"#;
        
        let request = ValidateYamlConfigRequest {
            yaml_content: yaml_content.to_string(),
        };
        
        let response = api.validate_yaml_config(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.parsed_config.is_some());
        assert!(response.validation_errors.is_empty());
        
        let config = response.parsed_config.unwrap();
        assert_eq!(config.id, "test-bridge");
        assert_eq!(config.url, "http://localhost:8080");
    }

    #[tokio::test]
    async fn test_get_appservice_statistics() {
        let api = create_test_api();
        
        let stats = api.get_appservice_statistics("admin").await.unwrap();
        
        assert_eq!(stats.total_appservices, 2);
        assert_eq!(stats.active_appservices, 1); // telegram-bridge is active
        assert_eq!(stats.inactive_appservices, 1); // irc-bridge is inactive
        assert_eq!(stats.appservices_with_errors, 1); // irc-bridge has error
    }

    #[tokio::test]
    async fn test_get_appservice_detail() {
        let api = create_test_api();
        
        let response = api.get_appservice_detail("telegram-bridge", "admin").await.unwrap();
        
        assert_eq!(response.appservice.id, "telegram-bridge");
        assert!(!response.yaml_config.is_empty());
        assert_eq!(response.statistics.total_appservices, 1);
    }
}