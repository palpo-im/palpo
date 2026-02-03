//! Configuration data models

use serde::{Deserialize, Serialize};

/// Main configuration data structure
#[derive(Serialize, Deserialize, Clone, Debug)]
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
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerConfigSection {
    pub server_name: String,
    pub listeners: Vec<ListenerConfig>,
    pub max_request_size: u64,
    pub enable_metrics: bool,
    pub home_page: Option<String>,
    pub new_user_displayname_suffix: String,
}

/// Listener configuration for HTTP/HTTPS endpoints
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ListenerConfig {
    pub bind: String,
    pub port: u16,
    pub tls: Option<TlsConfig>,
    pub resources: Vec<ListenerResource>,
}

/// TLS configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TlsConfig {
    pub certificate_path: String,
    pub private_key_path: String,
    pub min_version: Option<String>,
}

/// Listener resource types
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ListenerResource {
    Client,
    Federation,
    Media,
    Admin,
}

/// Database configuration section
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DatabaseConfigSection {
    pub connection_string: String,
    pub max_connections: u32,
    pub connection_timeout: u64,
    pub auto_migrate: bool,
    pub pool_size: Option<u32>,
    pub min_idle: Option<u32>,
}

/// Federation configuration section
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FederationConfigSection {
    pub enabled: bool,
    pub trusted_servers: Vec<String>,
    pub signing_key_path: String,
    pub verify_keys: bool,
    pub allow_device_name: bool,
    pub allow_inbound_profile_lookup: bool,
}

/// Authentication configuration section
#[derive(Serialize, Deserialize, Clone, Debug)]
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
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum RegistrationKind {
    Open,
    Token,
    Invite,
    Disabled,
}

/// OIDC provider configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OidcProvider {
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Vec<String>,
}

/// Media configuration section
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaConfigSection {
    pub storage_path: String,
    pub max_file_size: u64,
    pub thumbnail_sizes: Vec<ThumbnailSize>,
    pub enable_url_previews: bool,
    pub allow_legacy: bool,
    pub startup_check: bool,
}

/// Thumbnail size configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ThumbnailSize {
    pub width: u32,
    pub height: u32,
    pub method: ThumbnailMethod,
}

/// Thumbnail generation methods
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ThumbnailMethod {
    Crop,
    Scale,
}

/// Network configuration section
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetworkConfigSection {
    pub request_timeout: u64,
    pub connection_timeout: u64,
    pub ip_range_denylist: Vec<String>,
    pub cors_origins: Vec<String>,
    pub rate_limits: RateLimitConfig,
}

/// Rate limiting configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
    pub burst_size: u32,
    pub enabled: bool,
}

/// Logging configuration section
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoggingConfigSection {
    pub level: LogLevel,
    pub format: LogFormat,
    pub output: Vec<LogOutput>,
    pub rotation: LogRotationConfig,
    pub prometheus_metrics: bool,
}

/// Log levels
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Log formats
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum LogFormat {
    Json,
    Pretty,
    Compact,
}

/// Log output destinations
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum LogOutput {
    Console,
    File(String),
    Syslog,
}

/// Log rotation configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LogRotationConfig {
    pub max_size_mb: u64,
    pub max_files: u32,
    pub max_age_days: u32,
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