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
    /// Validates that config can be serialized to JSON and deserialized back correctly
    #[test]
    fn test_config_serialization_roundtrip() {
        let config = WebConfigData::default();
        let serialized = serde_json::to_string(&config).expect("Failed to serialize config");
        let deserialized: WebConfigData = serde_json::from_str(&serialized).expect("Failed to deserialize config");
        assert_eq!(config, deserialized);
    }

    /// Test server configuration serialization
    #[test]
    fn test_server_config_serialization() {
        let server_config = ServerConfigSection {
            server_name: "test.example.com".to_string(),
            listeners: vec![ListenerConfig::default()],
            max_request_size: 10 * 1024 * 1024,
            enable_metrics: true,
            home_page: Some("/home".to_string()),
            new_user_displayname_suffix: "_test".to_string(),
        };
        
        let serialized = serde_json::to_string(&server_config).expect("Failed to serialize server config");
        let deserialized: ServerConfigSection = serde_json::from_str(&serialized).expect("Failed to deserialize server config");
        assert_eq!(server_config, deserialized);
    }

    /// Test database configuration serialization
    #[test]
    fn test_database_config_serialization() {
        let db_config = DatabaseConfigSection {
            connection_string: "postgresql://user:pass@localhost/dbname".to_string(),
            max_connections: 50,
            connection_timeout: 60,
            auto_migrate: false,
            pool_size: Some(25),
            min_idle: Some(5),
        };
        
        let serialized = serde_json::to_string(&db_config).expect("Failed to serialize database config");
        let deserialized: DatabaseConfigSection = serde_json::from_str(&serialized).expect("Failed to deserialize database config");
        assert_eq!(db_config, deserialized);
    }

    /// Test listener configuration with TLS
    #[test]
    fn test_listener_config_with_tls() {
        let listener = ListenerConfig {
            bind: "0.0.0.0".to_string(),
            port: 8448,
            tls: Some(TlsConfig {
                certificate_path: "/etc/palpo/tls/cert.pem".to_string(),
                private_key_path: "/etc/palpo/tls/key.pem".to_string(),
                min_version: Some("1.2".to_string()),
            }),
            resources: vec![ListenerResource::Client, ListenerResource::Federation],
        };
        
        let serialized = serde_json::to_string(&listener).expect("Failed to serialize listener config");
        let deserialized: ListenerConfig = serde_json::from_str(&serialized).expect("Failed to deserialize listener config");
        assert_eq!(listener, deserialized);
    }

    /// Test listener configuration without TLS
    #[test]
    fn test_listener_config_without_tls() {
        let listener = ListenerConfig {
            bind: "127.0.0.1".to_string(),
            port: 8008,
            tls: None,
            resources: vec![ListenerResource::Client],
        };
        
        let serialized = serde_json::to_string(&listener).expect("Failed to serialize listener config");
        let deserialized: ListenerConfig = serde_json::from_str(&serialized).expect("Failed to deserialize listener config");
        assert_eq!(listener, deserialized);
    }

    /// Test media configuration with thumbnail sizes
    #[test]
    fn test_media_config_with_thumbnails() {
        let media_config = MediaConfigSection {
            storage_path: "/var/lib/palpo/media".to_string(),
            max_file_size: 100 * 1024 * 1024,
            thumbnail_sizes: vec![
                ThumbnailSize {
                    width: 32,
                    height: 32,
                    method: ThumbnailMethod::Crop,
                },
                ThumbnailSize {
                    width: 96,
                    height: 96,
                    method: ThumbnailMethod::Scale,
                },
                ThumbnailSize {
                    width: 256,
                    height: 256,
                    method: ThumbnailMethod::Crop,
                },
            ],
            enable_url_previews: true,
            allow_legacy: true,
            startup_check: false,
        };
        
        let serialized = serde_json::to_string(&media_config).expect("Failed to serialize media config");
        let deserialized: MediaConfigSection = serde_json::from_str(&serialized).expect("Failed to deserialize media config");
        assert_eq!(media_config, deserialized);
    }

    /// Test authentication configuration with OIDC providers
    #[test]
    fn test_auth_config_with_oidc() {
        let auth_config = AuthConfigSection {
            registration_enabled: true,
            registration_kind: RegistrationKind::Open,
            jwt_secret: "super-secret-jwt-key".to_string(),
            jwt_expiry: 7200,
            oidc_providers: vec![
                OidcProvider {
                    name: "google".to_string(),
                    issuer: "https://accounts.google.com".to_string(),
                    client_id: "client-id-123".to_string(),
                    client_secret: "client-secret-456".to_string(),
                    scopes: vec!["openid".to_string(), "profile".to_string(), "email".to_string()],
                },
            ],
            allow_guest_registration: true,
            require_auth_for_profile_requests: false,
        };
        
        let serialized = serde_json::to_string(&auth_config).expect("Failed to serialize auth config");
        let deserialized: AuthConfigSection = serde_json::from_str(&serialized).expect("Failed to deserialize auth config");
        assert_eq!(auth_config, deserialized);
    }

    /// Test federation configuration
    #[test]
    fn test_federation_config_serialization() {
        let fed_config = FederationConfigSection {
            enabled: true,
            trusted_servers: vec![
                "matrix.org".to_string(),
                "vector.im".to_string(),
            ],
            signing_key_path: "/etc/palpo/signing.key".to_string(),
            verify_keys: true,
            allow_device_name: false,
            allow_inbound_profile_lookup: true,
        };
        
        let serialized = serde_json::to_string(&fed_config).expect("Failed to serialize federation config");
        let deserialized: FederationConfigSection = serde_json::from_str(&serialized).expect("Failed to deserialize federation config");
        assert_eq!(fed_config, deserialized);
    }

    /// Test network configuration with rate limits
    #[test]
    fn test_network_config_serialization() {
        let network_config = NetworkConfigSection {
            request_timeout: 120,
            connection_timeout: 60,
            ip_range_denylist: vec!["10.0.0.0/8".to_string(), "192.168.0.0/16".to_string()],
            cors_origins: vec!["https://app.example.com".to_string()],
            rate_limits: RateLimitConfig {
                requests_per_minute: 100,
                burst_size: 20,
                enabled: true,
            },
        };
        
        let serialized = serde_json::to_string(&network_config).expect("Failed to serialize network config");
        let deserialized: NetworkConfigSection = serde_json::from_str(&serialized).expect("Failed to deserialize network config");
        assert_eq!(network_config, deserialized);
    }

    /// Test logging configuration with file output
    #[test]
    fn test_logging_config_with_file_output() {
        let logging_config = LoggingConfigSection {
            level: LogLevel::Debug,
            format: LogFormat::Json,
            output: vec![
                LogOutput::Console,
                LogOutput::File("/var/log/palpo/palpo.log".to_string()),
            ],
            rotation: LogRotationConfig {
                max_size_mb: 200,
                max_files: 20,
                max_age_days: 60,
            },
            prometheus_metrics: true,
        };
        
        let serialized = serde_json::to_string(&logging_config).expect("Failed to serialize logging config");
        let deserialized: LoggingConfigSection = serde_json::from_str(&serialized).expect("Failed to deserialize logging config");
        assert_eq!(logging_config, deserialized);
    }

    /// Test default values are consistent
    #[test]
    fn test_default_values_consistency() {
        let config = WebConfigData::default();
        
        // Verify server defaults
        assert_eq!(config.server.server_name, "localhost");
        assert_eq!(config.server.listeners.len(), 1);
        assert_eq!(config.server.max_request_size, 20 * 1024 * 1024);
        
        // Verify database defaults
        assert_eq!(config.database.max_connections, 10);
        assert_eq!(config.database.connection_timeout, 30);
        assert!(config.database.auto_migrate);
        
        // Verify federation defaults
        assert!(config.federation.enabled);
        assert!(config.federation.verify_keys);
        
        // Verify auth defaults
        assert!(!config.auth.registration_enabled);
        assert_eq!(config.auth.registration_kind, RegistrationKind::Disabled);
        assert_eq!(config.auth.jwt_expiry, 3600);
        
        // Verify media defaults
        assert_eq!(config.media.max_file_size, 50 * 1024 * 1024);
        assert!(config.media.enable_url_previews);
        
        // Verify network defaults
        assert_eq!(config.network.request_timeout, 60);
        assert_eq!(config.network.cors_origins, vec!["*".to_string()]);
        
        // Verify logging defaults
        assert_eq!(LogLevel::Info, config.logging.level);
        assert!(!config.logging.prometheus_metrics);
    }

    /// Test registration kind enum serialization
    #[test]
    fn test_registration_kind_serialization() {
        for kind in [
            RegistrationKind::Open,
            RegistrationKind::Token,
            RegistrationKind::Invite,
            RegistrationKind::Disabled,
        ] {
            let serialized = serde_json::to_string(&kind).expect("Failed to serialize registration kind");
            let deserialized: RegistrationKind = serde_json::from_str(&serialized).expect("Failed to deserialize registration kind");
            assert_eq!(kind, deserialized);
        }
    }

    /// Test listener resource enum serialization
    #[test]
    fn test_listener_resource_serialization() {
        for resource in [
            ListenerResource::Client,
            ListenerResource::Federation,
            ListenerResource::Media,
            ListenerResource::Admin,
        ] {
            let serialized = serde_json::to_string(&resource).expect("Failed to serialize listener resource");
            let deserialized: ListenerResource = serde_json::from_str(&serialized).expect("Failed to deserialize listener resource");
            assert_eq!(resource, deserialized);
        }
    }

    /// Test thumbnail method enum serialization
    #[test]
    fn test_thumbnail_method_serialization() {
        for method in [ThumbnailMethod::Crop, ThumbnailMethod::Scale] {
            let serialized = serde_json::to_string(&method).expect("Failed to serialize thumbnail method");
            let deserialized: ThumbnailMethod = serde_json::from_str(&serialized).expect("Failed to deserialize thumbnail method");
            assert_eq!(method, deserialized);
        }
    }

    /// Test log level enum serialization
    #[test]
    fn test_log_level_serialization() {
        for level in [LogLevel::Debug, LogLevel::Info, LogLevel::Warn, LogLevel::Error] {
            let serialized = serde_json::to_string(&level).expect("Failed to serialize log level");
            let deserialized: LogLevel = serde_json::from_str(&serialized).expect("Failed to deserialize log level");
            assert_eq!(level, deserialized);
        }
    }

    /// Test log format enum serialization
    #[test]
    fn test_log_format_serialization() {
        for format in [LogFormat::Json, LogFormat::Pretty, LogFormat::Compact, LogFormat::Text] {
            let serialized = serde_json::to_string(&format).expect("Failed to serialize log format");
            let deserialized: LogFormat = serde_json::from_str(&serialized).expect("Failed to deserialize log format");
            assert_eq!(format, deserialized);
        }
    }

    /// Test log output enum serialization
    #[test]
    fn test_log_output_serialization() {
        // Test Console variant
        let console = LogOutput::Console;
        let serialized = serde_json::to_string(&console).expect("Failed to serialize console log output");
        let deserialized: LogOutput = serde_json::from_str(&serialized).expect("Failed to deserialize console log output");
        assert_eq!(console, deserialized);

        // Test File variant
        let file = LogOutput::File("/var/log/app.log".to_string());
        let serialized = serde_json::to_string(&file).expect("Failed to serialize file log output");
        let deserialized: LogOutput = serde_json::from_str(&serialized).expect("Failed to deserialize file log output");
        assert_eq!(file, deserialized);

        // Test Syslog variant
        let syslog = LogOutput::Syslog;
        let serialized = serde_json::to_string(&syslog).expect("Failed to serialize syslog log output");
        let deserialized: LogOutput = serde_json::from_str(&serialized).expect("Failed to deserialize syslog log output");
        assert_eq!(syslog, deserialized);
    }

    /// Test OIDC provider configuration
    #[test]
    fn test_oidc_provider_serialization() {
        let provider = OidcProvider {
            name: "azure-ad".to_string(),
            issuer: "https://login.microsoftonline.com/tenant-id".to_string(),
            client_id: "application-id".to_string(),
            client_secret: "client-secret".to_string(),
            scopes: vec!["openid".to_string(), "profile".to_string()],
        };
        
        let serialized = serde_json::to_string(&provider).expect("Failed to serialize OIDC provider");
        let deserialized: OidcProvider = serde_json::from_str(&serialized).expect("Failed to deserialize OIDC provider");
        assert_eq!(provider, deserialized);
    }

    /// Test thumbnail size configuration
    #[test]
    fn test_thumbnail_size_serialization() {
        let thumbnail = ThumbnailSize {
            width: 128,
            height: 128,
            method: ThumbnailMethod::Scale,
        };
        
        let serialized = serde_json::to_string(&thumbnail).expect("Failed to serialize thumbnail size");
        let deserialized: ThumbnailSize = serde_json::from_str(&serialized).expect("Failed to deserialize thumbnail size");
        assert_eq!(thumbnail, deserialized);
    }

    /// Test rate limit configuration
    #[test]
    fn test_rate_limit_config_serialization() {
        let rate_limit = RateLimitConfig {
            requests_per_minute: 200,
            burst_size: 50,
            enabled: true,
        };
        
        let serialized = serde_json::to_string(&rate_limit).expect("Failed to serialize rate limit config");
        let deserialized: RateLimitConfig = serde_json::from_str(&serialized).expect("Failed to deserialize rate limit config");
        assert_eq!(rate_limit, deserialized);
    }

    /// Test log rotation configuration
    #[test]
    fn test_log_rotation_config_serialization() {
        let rotation = LogRotationConfig {
            max_size_mb: 500,
            max_files: 30,
            max_age_days: 90,
        };
        
        let serialized = serde_json::to_string(&rotation).expect("Failed to serialize log rotation config");
        let deserialized: LogRotationConfig = serde_json::from_str(&serialized).expect("Failed to deserialize log rotation config");
        assert_eq!(rotation, deserialized);
    }

    /// Test TLS configuration
    #[test]
    fn test_tls_config_serialization() {
        let tls = TlsConfig {
            certificate_path: "/etc/ssl/certs/cert.pem".to_string(),
            private_key_path: "/etc/ssl/private/key.pem".to_string(),
            min_version: Some("TLSv1.2".to_string()),
        };
        
        let serialized = serde_json::to_string(&tls).expect("Failed to serialize TLS config");
        let deserialized: TlsConfig = serde_json::from_str(&serialized).expect("Failed to deserialize TLS config");
        assert_eq!(tls, deserialized);
    }

    /// Test TLS configuration without min version
    #[test]
    fn test_tls_config_without_min_version() {
        let tls = TlsConfig {
            certificate_path: "/etc/ssl/certs/cert.pem".to_string(),
            private_key_path: "/etc/ssl/private/key.pem".to_string(),
            min_version: None,
        };
        
        let serialized = serde_json::to_string(&tls).expect("Failed to serialize TLS config");
        let deserialized: TlsConfig = serde_json::from_str(&serialized).expect("Failed to deserialize TLS config");
        assert_eq!(tls, deserialized);
    }

    /// Test configuration with multiple listeners
    #[test]
    fn test_config_with_multiple_listeners() {
        let config = WebConfigData {
            server: ServerConfigSection {
                server_name: "example.com".to_string(),
                listeners: vec![
                    ListenerConfig {
                        bind: "0.0.0.0".to_string(),
                        port: 8008,
                        tls: None,
                        resources: vec![ListenerResource::Client],
                    },
                    ListenerConfig {
                        bind: "0.0.0.0".to_string(),
                        port: 8448,
                        tls: Some(TlsConfig {
                            certificate_path: "/etc/palpo/tls/cert.pem".to_string(),
                            private_key_path: "/etc/palpo/tls/key.pem".to_string(),
                            min_version: None,
                        }),
                        resources: vec![ListenerResource::Client, ListenerResource::Federation],
                    },
                ],
                max_request_size: 50 * 1024 * 1024,
                enable_metrics: true,
                home_page: None,
                new_user_displayname_suffix: "".to_string(),
            },
            ..WebConfigData::default()
        };
        
        let serialized = serde_json::to_string(&config).expect("Failed to serialize config with multiple listeners");
        let deserialized: WebConfigData = serde_json::from_str(&serialized).expect("Failed to deserialize config with multiple listeners");
        assert_eq!(config, deserialized);
    }

    /// Test configuration with multiple OIDC providers
    #[test]
    fn test_config_with_multiple_oidc_providers() {
        let config = WebConfigData {
            auth: AuthConfigSection {
                registration_enabled: true,
                registration_kind: RegistrationKind::Open,
                jwt_secret: "secret".to_string(),
                jwt_expiry: 3600,
                oidc_providers: vec![
                    OidcProvider {
                        name: "google".to_string(),
                        issuer: "https://accounts.google.com".to_string(),
                        client_id: "google-client-id".to_string(),
                        client_secret: "google-client-secret".to_string(),
                        scopes: vec!["openid".to_string(), "profile".to_string()],
                    },
                    OidcProvider {
                        name: "github".to_string(),
                        issuer: "https://github.com".to_string(),
                        client_id: "github-client-id".to_string(),
                        client_secret: "github-client-secret".to_string(),
                        scopes: vec!["read:user".to_string(), "user:email".to_string()],
                    },
                ],
                allow_guest_registration: false,
                require_auth_for_profile_requests: true,
            },
            ..WebConfigData::default()
        };
        
        let serialized = serde_json::to_string(&config).expect("Failed to serialize config with multiple OIDC providers");
        let deserialized: WebConfigData = serde_json::from_str(&serialized).expect("Failed to deserialize config with multiple OIDC providers");
        assert_eq!(config, deserialized);
    }

    /// Test configuration with multiple trusted servers
    #[test]
    fn test_config_with_multiple_trusted_servers() {
        let config = WebConfigData {
            federation: FederationConfigSection {
                enabled: true,
                trusted_servers: vec![
                    "matrix.org".to_string(),
                    "vector.im".to_string(),
                    "element.io".to_string(),
                ],
                signing_key_path: "signing.key".to_string(),
                verify_keys: true,
                allow_device_name: true,
                allow_inbound_profile_lookup: true,
            },
            ..WebConfigData::default()
        };
        
        let serialized = serde_json::to_string(&config).expect("Failed to serialize config with multiple trusted servers");
        let deserialized: WebConfigData = serde_json::from_str(&serialized).expect("Failed to deserialize config with multiple trusted servers");
        assert_eq!(config, deserialized);
    }

    /// Test configuration with multiple log outputs
    #[test]
    fn test_config_with_multiple_log_outputs() {
        let config = WebConfigData {
            logging: LoggingConfigSection {
                level: LogLevel::Debug,
                format: LogFormat::Json,
                output: vec![
                    LogOutput::Console,
                    LogOutput::File("/var/log/palpo/app.log".to_string()),
                    LogOutput::Syslog,
                ],
                rotation: LogRotationConfig {
                    max_size_mb: 100,
                    max_files: 10,
                    max_age_days: 30,
                },
                prometheus_metrics: true,
            },
            ..WebConfigData::default()
        };
        
        let serialized = serde_json::to_string(&config).expect("Failed to serialize config with multiple log outputs");
        let deserialized: WebConfigData = serde_json::from_str(&serialized).expect("Failed to deserialize config with multiple log outputs");
        assert_eq!(config, deserialized);
    }

    /// Test configuration with IP deny list
    #[test]
    fn test_config_with_ip_deny_list() {
        let config = WebConfigData {
            network: NetworkConfigSection {
                request_timeout: 60,
                connection_timeout: 30,
                ip_range_denylist: vec![
                    "10.0.0.0/8".to_string(),
                    "172.16.0.0/12".to_string(),
                    "192.168.0.0/16".to_string(),
                ],
                cors_origins: vec!["https://example.com".to_string()],
                rate_limits: RateLimitConfig::default(),
            },
            ..WebConfigData::default()
        };
        
        let serialized = serde_json::to_string(&config).expect("Failed to serialize config with IP deny list");
        let deserialized: WebConfigData = serde_json::from_str(&serialized).expect("Failed to deserialize config with IP deny list");
        assert_eq!(config, deserialized);
    }

    /// Test configuration with custom CORS origins
    #[test]
    fn test_config_with_cors_origins() {
        let config = WebConfigData {
            network: NetworkConfigSection {
                request_timeout: 60,
                connection_timeout: 30,
                ip_range_denylist: vec![],
                cors_origins: vec![
                    "https://app.example.com".to_string(),
                    "https://admin.example.com".to_string(),
                    "http://localhost:3000".to_string(),
                ],
                rate_limits: RateLimitConfig::default(),
            },
            ..WebConfigData::default()
        };
        
        let serialized = serde_json::to_string(&config).expect("Failed to serialize config with CORS origins");
        let deserialized: WebConfigData = serde_json::from_str(&serialized).expect("Failed to deserialize config with CORS origins");
        assert_eq!(config, deserialized);
    }

    /// Test configuration with custom thumbnail sizes
    #[test]
    fn test_config_with_custom_thumbnail_sizes() {
        let config = WebConfigData {
            media: MediaConfigSection {
                storage_path: "/data/media".to_string(),
                max_file_size: 25 * 1024 * 1024,
                thumbnail_sizes: vec![
                    ThumbnailSize {
                        width: 64,
                        height: 64,
                        method: ThumbnailMethod::Crop,
                    },
                    ThumbnailSize {
                        width: 128,
                        height: 128,
                        method: ThumbnailMethod::Scale,
                    },
                    ThumbnailSize {
                        width: 256,
                        height: 256,
                        method: ThumbnailMethod::Crop,
                    },
                    ThumbnailSize {
                        width: 512,
                        height: 512,
                        method: ThumbnailMethod::Scale,
                    },
                ],
                enable_url_previews: false,
                allow_legacy: true,
                startup_check: true,
            },
            ..WebConfigData::default()
        };
        
        let serialized = serde_json::to_string(&config).expect("Failed to serialize config with custom thumbnail sizes");
        let deserialized: WebConfigData = serde_json::from_str(&serialized).expect("Failed to deserialize config with custom thumbnail sizes");
        assert_eq!(config, deserialized);
    }
}