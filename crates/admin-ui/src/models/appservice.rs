//! Appservice management models

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

/// Appservice information for management
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Appservice {
    pub id: String,
    pub url: String,
    pub as_token: String,
    pub hs_token: String,
    pub sender_localpart: String,
    pub namespaces: AppserviceNamespaces,
    pub rate_limited: Option<bool>,
    pub protocols: Option<Vec<String>>,
    pub is_active: bool,
    pub created_at: u64,
    pub last_ping: Option<u64>,
    pub last_error: Option<String>,
}

/// Appservice namespaces configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AppserviceNamespaces {
    pub users: Vec<AppserviceNamespace>,
    pub aliases: Vec<AppserviceNamespace>,
    pub rooms: Vec<AppserviceNamespace>,
}

/// Individual namespace configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AppserviceNamespace {
    pub exclusive: bool,
    pub regex: String,
}

/// Appservice registration request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegisterAppserviceRequest {
    pub id: String,
    pub url: String,
    pub as_token: Option<String>, // None for auto-generated token
    pub hs_token: Option<String>, // None for auto-generated token
    pub sender_localpart: String,
    pub namespaces: AppserviceNamespaces,
    pub rate_limited: Option<bool>,
    pub protocols: Option<Vec<String>>,
    pub yaml_config: Option<String>, // Raw YAML configuration
}

/// Appservice registration response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegisterAppserviceResponse {
    pub success: bool,
    pub appservice: Option<Appservice>,
    pub generated_as_token: Option<String>,
    pub generated_hs_token: Option<String>,
    pub error: Option<String>,
}

/// Appservice update request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateAppserviceRequest {
    pub url: Option<String>,
    pub namespaces: Option<AppserviceNamespaces>,
    pub rate_limited: Option<bool>,
    pub protocols: Option<Vec<String>>,
    pub yaml_config: Option<String>,
}

/// Appservice update response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateAppserviceResponse {
    pub success: bool,
    pub appservice: Option<Appservice>,
    pub error: Option<String>,
}

/// Appservice unregistration request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UnregisterAppserviceRequest {
    pub id: String,
    pub cleanup_data: bool,
}

/// Appservice unregistration response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UnregisterAppserviceResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Appservice test request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestAppserviceRequest {
    pub id: String,
    pub test_type: AppserviceTestType,
}

/// Appservice test types
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum AppserviceTestType {
    Ping,
    UserQuery,
    AliasQuery,
    RoomQuery,
}

/// Appservice test response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestAppserviceResponse {
    pub success: bool,
    pub test_type: AppserviceTestType,
    pub response_time_ms: Option<u64>,
    pub response_data: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Appservice list request with filtering and pagination
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ListAppservicesRequest {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub search: Option<String>,
    pub filter_active: Option<bool>,
    pub sort_by: Option<AppserviceSortField>,
    pub sort_order: Option<AppserviceSortOrder>,
}

/// Appservice list response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ListAppservicesResponse {
    pub success: bool,
    pub appservices: Vec<Appservice>,
    pub total_count: u32,
    pub has_more: bool,
    pub error: Option<String>,
}

/// Appservice sort fields
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum AppserviceSortField {
    Id,
    Url,
    CreatedAt,
    LastPing,
    IsActive,
}

/// Sort order for appservices
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum AppserviceSortOrder {
    Ascending,
    Descending,
}

/// Appservice statistics
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppserviceStatistics {
    pub total_appservices: u32,
    pub active_appservices: u32,
    pub inactive_appservices: u32,
    pub appservices_with_errors: u32,
    pub total_managed_users: u32,
    pub total_managed_aliases: u32,
    pub total_managed_rooms: u32,
}

/// YAML configuration validation request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValidateYamlConfigRequest {
    pub yaml_content: String,
}

/// YAML configuration validation response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValidateYamlConfigResponse {
    pub success: bool,
    pub parsed_config: Option<AppserviceYamlConfig>,
    pub validation_errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Parsed YAML configuration structure
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppserviceYamlConfig {
    pub id: String,
    pub url: String,
    pub as_token: String,
    pub hs_token: String,
    pub sender_localpart: String,
    pub namespaces: AppserviceNamespaces,
    pub rate_limited: Option<bool>,
    pub protocols: Option<Vec<String>>,
    #[serde(default)]
    pub additional_fields: HashMap<String, serde_json::Value>,
}

/// Appservice detailed information response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppserviceDetailResponse {
    pub appservice: Appservice,
    pub yaml_config: String,
    pub statistics: AppserviceStatistics,
    pub recent_activity: Vec<AppserviceActivity>,
}

/// Appservice activity log entry
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppserviceActivity {
    pub timestamp: u64,
    pub activity_type: AppserviceActivityType,
    pub description: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Appservice activity types
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum AppserviceActivityType {
    Registration,
    Unregistration,
    Ping,
    UserQuery,
    AliasQuery,
    RoomQuery,
    ConfigUpdate,
    Error,
}

impl Appservice {
    /// Check if appservice is healthy (active and no recent errors)
    pub fn is_healthy(&self) -> bool {
        self.is_active && self.last_error.is_none()
    }

    /// Get age in days since creation
    pub fn age_in_days(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        (now - self.created_at) / 86400 // 86400 seconds in a day
    }

    /// Get days since last ping
    pub fn days_since_last_ping(&self) -> Option<u64> {
        self.last_ping.map(|last_ping| {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            
            (now - last_ping) / 86400
        })
    }

    /// Get total number of managed namespaces
    pub fn total_namespaces(&self) -> usize {
        self.namespaces.users.len() + 
        self.namespaces.aliases.len() + 
        self.namespaces.rooms.len()
    }

    /// Check if appservice manages a specific user
    pub fn manages_user(&self, user_id: &str) -> bool {
        self.namespaces.users.iter().any(|ns| {
            // In a real implementation, this would use proper regex matching
            user_id.contains(&ns.regex.replace(".*", ""))
        })
    }

    /// Check if appservice manages a specific alias
    pub fn manages_alias(&self, alias: &str) -> bool {
        self.namespaces.aliases.iter().any(|ns| {
            // In a real implementation, this would use proper regex matching
            alias.contains(&ns.regex.replace(".*", ""))
        })
    }

    /// Check if appservice manages a specific room
    pub fn manages_room(&self, room_id: &str) -> bool {
        self.namespaces.rooms.iter().any(|ns| {
            // In a real implementation, this would use proper regex matching
            room_id.contains(&ns.regex.replace(".*", ""))
        })
    }
}

impl Default for ListAppservicesRequest {
    fn default() -> Self {
        Self {
            limit: Some(50),
            offset: Some(0),
            search: None,
            filter_active: None,
            sort_by: Some(AppserviceSortField::Id),
            sort_order: Some(AppserviceSortOrder::Ascending),
        }
    }
}

impl AppserviceSortField {
    /// Get human-readable description of the sort field
    pub fn description(&self) -> &'static str {
        match self {
            AppserviceSortField::Id => "ID",
            AppserviceSortField::Url => "URL",
            AppserviceSortField::CreatedAt => "Created At",
            AppserviceSortField::LastPing => "Last Ping",
            AppserviceSortField::IsActive => "Active Status",
        }
    }
}

impl AppserviceTestType {
    /// Get human-readable description of the test type
    pub fn description(&self) -> &'static str {
        match self {
            AppserviceTestType::Ping => "Ping Test",
            AppserviceTestType::UserQuery => "User Query Test",
            AppserviceTestType::AliasQuery => "Alias Query Test",
            AppserviceTestType::RoomQuery => "Room Query Test",
        }
    }
}

impl AppserviceActivityType {
    /// Get human-readable description of the activity type
    pub fn description(&self) -> &'static str {
        match self {
            AppserviceActivityType::Registration => "Registration",
            AppserviceActivityType::Unregistration => "Unregistration",
            AppserviceActivityType::Ping => "Ping",
            AppserviceActivityType::UserQuery => "User Query",
            AppserviceActivityType::AliasQuery => "Alias Query",
            AppserviceActivityType::RoomQuery => "Room Query",
            AppserviceActivityType::ConfigUpdate => "Configuration Update",
            AppserviceActivityType::Error => "Error",
        }
    }
}

/// Helper function to generate secure tokens
pub fn generate_token() -> String {
    use rand::{thread_rng, Rng};
    use rand::distributions::Alphanumeric;
    
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}

/// Helper function to validate regex patterns
pub fn validate_regex(pattern: &str) -> Result<(), String> {
    // In a real implementation, this would use the regex crate
    // For now, we'll do basic validation
    if pattern.is_empty() {
        return Err("Regex pattern cannot be empty".to_string());
    }
    
    // Check for common regex syntax errors
    if pattern.contains("**") {
        return Err("Invalid regex pattern: consecutive wildcards".to_string());
    }
    
    Ok(())
}

/// Helper function to parse YAML configuration
pub fn parse_yaml_config(yaml_content: &str) -> Result<AppserviceYamlConfig, String> {
    serde_yaml::from_str(yaml_content)
        .map_err(|e| format!("YAML parsing error: {}", e))
}

/// Helper function to generate YAML configuration from Appservice
pub fn generate_yaml_config(appservice: &Appservice) -> Result<String, String> {
    let config = AppserviceYamlConfig {
        id: appservice.id.clone(),
        url: appservice.url.clone(),
        as_token: appservice.as_token.clone(),
        hs_token: appservice.hs_token.clone(),
        sender_localpart: appservice.sender_localpart.clone(),
        namespaces: appservice.namespaces.clone(),
        rate_limited: appservice.rate_limited,
        protocols: appservice.protocols.clone(),
        additional_fields: HashMap::new(),
    };
    
    serde_yaml::to_string(&config)
        .map_err(|e| format!("YAML generation error: {}", e))
}