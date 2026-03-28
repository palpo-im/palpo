//! Configuration data models

use serde::{Deserialize, Serialize};

/// Main configuration data structure
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WebConfigData {
    pub server: ServerConfigSection,
    pub database: DatabaseConfigSection,
    pub federation: FederationConfigSection,
    pub auth: AuthConfigSection,
    pub media: MediaConfigSection,
    pub network: NetworkConfigSection,
    pub logging: LoggingConfigSection,
}

/// Server configuration section
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServerConfigSection {
    pub server_name: String,
    pub listeners: Vec<ListenerConfig>,
    pub max_request_size: u64,
    pub enable_metrics: bool,
    pub home_page: Option<String>,
    pub new_user_displayname_suffix: String,
}

/// Listener configuration for HTTP/HTTPS endpoints
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ListenerConfig {
    pub bind: String,
    pub port: u16,
    pub tls: Option<TlsConfig>,
    pub resources: Vec<ListenerResource>,
}

/// TLS configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TlsConfig {
    pub certificate_path: String,
    pub private_key_path: String,
    pub min_version: Option<String>,
}

/// Listener resource types
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ListenerResource {
    Client,
    Federation,
    Media,
    Admin,
}

/// Database configuration section
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DatabaseConfigSection {
    pub connection_string: String,
    pub max_connections: u32,
    pub connection_timeout: u64,
    pub auto_migrate: bool,
    pub pool_size: Option<u32>,
    pub min_idle: Option<u32>,
}

/// Federation configuration section
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FederationConfigSection {
    pub enabled: bool,
    pub trusted_servers: Vec<String>,
    pub signing_key_path: String,
    pub verify_keys: bool,
    pub allow_device_name: bool,
    pub allow_inbound_profile_lookup: bool,
}

/// Authentication configuration section
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AuthConfigSection {
    pub registration_enabled: bool,
    pub registration_kind: RegistrationKind,
    pub jwt_secret: String,
    pub jwt_expiry: u64,
    pub oidc_providers: Vec<OidcProvider>,
    pub allow_guest_registration: bool,
    pub require_auth_for_profile_requests: bool,
}

/// Registration types
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum RegistrationKind {
    Open,
    Token,
    Invite,
    Disabled,
}

/// OIDC provider configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OidcProvider {
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Vec<String>,
}

/// Media configuration section
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MediaConfigSection {
    pub storage_path: String,
    pub max_file_size: u64,
    pub thumbnail_sizes: Vec<ThumbnailSize>,
    pub enable_url_previews: bool,
    pub allow_legacy: bool,
    pub startup_check: bool,
}

/// Thumbnail size configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ThumbnailSize {
    pub width: u32,
    pub height: u32,
    pub method: ThumbnailMethod,
}

/// Thumbnail generation methods
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ThumbnailMethod {
    Crop,
    Scale,
}

/// Network configuration section
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetworkConfigSection {
    pub request_timeout: u64,
    pub connection_timeout: u64,
    pub ip_range_denylist: Vec<String>,
    pub cors_origins: Vec<String>,
    pub rate_limits: RateLimitConfig,
}

/// Rate limiting configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
    pub burst_size: u32,
    pub enabled: bool,
}

/// Logging configuration section
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LoggingConfigSection {
    pub level: LogLevel,
    pub format: LogFormat,
    pub output: Vec<LogOutput>,
    pub rotation: LogRotationConfig,
    pub prometheus_metrics: bool,
}

/// Log levels
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Log formats
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LogFormat {
    Json,
    Pretty,
    Compact,
    Text,
}

/// Log output destinations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LogOutput {
    Console,
    File(String),
    Syslog,
}

/// Log rotation configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LogRotationConfig {
    pub max_size_mb: u64,
    pub max_files: u32,
    pub max_age_days: u32,
}

// =============================================================================
// Backend API Response Structure (for parsing backend responses)
// =============================================================================

/// Backend ServerConfig structure (flat format)
/// This matches the backend's types::ServerConfig
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BackendServerConfig {
    pub server_name: String,
    #[serde(default = "default_allow_registration")]
    pub allow_registration: bool,
    // Backend returns "listener_configs", not "listeners"
    #[serde(rename = "listener_configs")]
    pub listener_configs: Vec<BackendListenerConfig>,
    // Backend returns "database", not "db"
    #[serde(rename = "database")]
    pub database: BackendDatabaseConfig,
    #[serde(rename = "well_known")]
    pub well_known: BackendWellKnownConfig,
}

fn default_allow_registration() -> bool {
    true
}

/// Backend listener configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BackendListenerConfig {
    pub address: String,
}

/// Backend database configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BackendDatabaseConfig {
    pub url: String,
}

/// Backend well-known configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BackendWellKnownConfig {
    pub server: String,
    pub client: String,
}

impl From<BackendServerConfig> for WebConfigData {
    fn from(backend: BackendServerConfig) -> Self {
        // Parse address string (e.g., "0.0.0.0:8008") into bind and port
        let listeners: Vec<ListenerConfig> = backend.listener_configs
            .into_iter()
            .map(|l| {
                let parts: Vec<&str> = l.address.split(':').collect();
                let bind = parts.get(0).unwrap_or(&"0.0.0.0").to_string();
                let port: u16 = parts.get(1)
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(8008);
                
                ListenerConfig {
                    bind,
                    port,
                    tls: None,
                    resources: vec![ListenerResource::Client],
                }
            })
            .collect();
        
        WebConfigData {
            server: ServerConfigSection {
                server_name: backend.server_name,
                listeners,
                max_request_size: 20 * 1024 * 1024,
                enable_metrics: false,
                home_page: None,
                new_user_displayname_suffix: "".to_string(),
            },
            database: DatabaseConfigSection {
                connection_string: backend.database.url,
                max_connections: 10,
                connection_timeout: 30,
                auto_migrate: true,
                pool_size: None,
                min_idle: None,
            },
            federation: FederationConfigSection::default(),
            auth: AuthConfigSection {
                registration_enabled: backend.allow_registration,
                ..AuthConfigSection::default()
            },
            media: MediaConfigSection::default(),
            network: NetworkConfigSection::default(),
            logging: LoggingConfigSection::default(),
        }
    }
}

impl From<WebConfigData> for BackendServerConfig {
    fn from(frontend: WebConfigData) -> Self {
        let server_name = frontend.server.server_name.clone();
        let well_known_server = server_name.clone();
        let well_known_client = format!("http://{}", server_name);
        
        let listener_configs = frontend.server.listeners
            .into_iter()
            .map(|l| BackendListenerConfig {
                address: format!("{}:{}", l.bind, l.port),
            })
            .collect();
        
        BackendServerConfig {
            server_name,
            allow_registration: frontend.auth.registration_enabled,
            listener_configs,
            database: BackendDatabaseConfig {
                url: frontend.database.connection_string,
            },
            well_known: BackendWellKnownConfig {
                server: well_known_server,
                client: well_known_client,
            },
        }
    }
}

impl Default for WebConfigData {
    fn default() -> Self {
        Self {
            server: ServerConfigSection::default(),
            database: DatabaseConfigSection::default(),
            federation: FederationConfigSection::default(),
            auth: AuthConfigSection::default(),
            media: MediaConfigSection::default(),
            network: NetworkConfigSection::default(),
            logging: LoggingConfigSection::default(),
        }
    }
}

impl Default for ServerConfigSection {
    fn default() -> Self {
        Self {
            server_name: "localhost".to_string(),
            listeners: vec![ListenerConfig::default()],
            max_request_size: 20 * 1024 * 1024, // 20MB
            enable_metrics: false,
            home_page: None,
            new_user_displayname_suffix: "".to_string(),
        }
    }
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0".to_string(),
            port: 8008,
            tls: None,
            resources: vec![ListenerResource::Client],
        }
    }
}

impl Default for DatabaseConfigSection {
    fn default() -> Self {
        Self {
            connection_string: "postgresql://palpo:password@localhost/palpo".to_string(),
            max_connections: 10,
            connection_timeout: 30,
            auto_migrate: true,
            pool_size: None,
            min_idle: None,
        }
    }
}

impl Default for FederationConfigSection {
    fn default() -> Self {
        Self {
            enabled: true,
            trusted_servers: vec![],
            signing_key_path: "signing.key".to_string(),
            verify_keys: true,
            allow_device_name: true,
            allow_inbound_profile_lookup: true,
        }
    }
}

impl Default for AuthConfigSection {
    fn default() -> Self {
        Self {
            registration_enabled: false,
            registration_kind: RegistrationKind::Disabled,
            jwt_secret: "change-me".to_string(),
            jwt_expiry: 3600, // 1 hour
            oidc_providers: vec![],
            allow_guest_registration: false,
            require_auth_for_profile_requests: true,
        }
    }
}

impl Default for MediaConfigSection {
    fn default() -> Self {
        Self {
            storage_path: "./media".to_string(),
            max_file_size: 50 * 1024 * 1024, // 50MB
            thumbnail_sizes: vec![
                ThumbnailSize {
                    width: 32,
                    height: 32,
                    method: ThumbnailMethod::Crop,
                },
                ThumbnailSize {
                    width: 96,
                    height: 96,
                    method: ThumbnailMethod::Crop,
                },
            ],
            enable_url_previews: true,
            allow_legacy: false,
            startup_check: true,
        }
    }
}

impl Default for NetworkConfigSection {
    fn default() -> Self {
        Self {
            request_timeout: 60,
            connection_timeout: 30,
            ip_range_denylist: vec![],
            cors_origins: vec!["*".to_string()],
            rate_limits: RateLimitConfig::default(),
        }
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
            burst_size: 10,
            enabled: true,
        }
    }
}

impl Default for LoggingConfigSection {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: LogFormat::Pretty,
            output: vec![LogOutput::Console],
            rotation: LogRotationConfig::default(),
            prometheus_metrics: false,
        }
    }
}

impl Default for LogRotationConfig {
    fn default() -> Self {
        Self {
            max_size_mb: 100,
            max_files: 10,
            max_age_days: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    /// Test configuration serialization and deserialization roundtrip
    #[test]
    fn test_config_serialization_roundtrip() {
        let config = WebConfigData::default();
        let serialized = serde_json::to_string(&config).expect("Failed to serialize config");
        let deserialized: WebConfigData = serde_json::from_str(&serialized).expect("Failed to deserialize config");
        assert_eq!(config, deserialized);
    }

    /// Test backend to frontend conversion
    #[test]
    fn test_backend_to_frontend_conversion() {
        let backend = BackendServerConfig {
            server_name: "localhost:8008".to_string(),
            allow_registration: true,
            listener_configs: vec![BackendListenerConfig {
                address: "0.0.0.0:8008".to_string(),
            }],
            database: BackendDatabaseConfig {
                url: "postgresql://palpo:password@localhost/palpo".to_string(),
            },
            well_known: BackendWellKnownConfig {
                server: "localhost:8008".to_string(),
                client: "http://localhost:8008".to_string(),
            },
        };
        
        let frontend: WebConfigData = backend.into();
        
        assert_eq!(frontend.server.server_name, "localhost:8008");
        assert_eq!(frontend.server.listeners.len(), 1);
        assert_eq!(frontend.server.listeners[0].bind, "0.0.0.0");
        assert_eq!(frontend.server.listeners[0].port, 8008);
        assert_eq!(frontend.database.connection_string, "postgresql://palpo:password@localhost/palpo");
        assert!(frontend.auth.registration_enabled);
    }

    /// Test frontend to backend conversion
    #[test]
    fn test_frontend_to_backend_conversion() {
        let mut frontend = WebConfigData::default();
        frontend.server.server_name = "test.example.com".to_string();
        frontend.server.listeners = vec![ListenerConfig {
            bind: "1.2.3.4".to_string(),
            port: 8448,
            tls: None,
            resources: vec![ListenerResource::Client],
        }];
        frontend.database.connection_string = "postgresql://test:test@localhost/test".to_string();
        frontend.auth.registration_enabled = true;
        
        let backend: BackendServerConfig = frontend.into();
        
        assert_eq!(backend.server_name, "test.example.com");
        assert_eq!(backend.listener_configs.len(), 1);
        assert_eq!(backend.listener_configs[0].address, "1.2.3.4:8448");
        assert_eq!(backend.database.url, "postgresql://test:test@localhost/test");
        assert!(backend.allow_registration);
    }
}
