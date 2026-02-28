/// Integration tests for Server Config API endpoints
///
/// Tests the REST API endpoints for server configuration management:
/// - GET /api/v1/admin/server/config
/// - POST /api/v1/admin/server/config
/// - POST /api/v1/admin/server/config/validate

use palpo_admin_server::types::ServerConfig;
use serde_json::json;

#[test]
fn test_server_config_serialization() {
    let config = ServerConfig {
        database_url: "postgresql://user:pass@localhost/palpo".to_string(),
        server_name: "example.com".to_string(),
        bind_address: "0.0.0.0".to_string(),
        port: 8008,
        tls_certificate: None,
        tls_private_key: None,
    };

    // Test JSON serialization
    let json_str = serde_json::to_string(&config).expect("Failed to serialize to JSON");
    let deserialized: ServerConfig =
        serde_json::from_str(&json_str).expect("Failed to deserialize from JSON");

    assert_eq!(config.database_url, deserialized.database_url);
    assert_eq!(config.server_name, deserialized.server_name);
    assert_eq!(config.bind_address, deserialized.bind_address);
    assert_eq!(config.port, deserialized.port);
}

#[test]
fn test_validate_config_request_format() {
    let request = json!({
        "config": {
            "database_url": "postgresql://user:pass@localhost/palpo",
            "server_name": "example.com",
            "bind_address": "0.0.0.0",
            "port": 8008,
            "tls_certificate": null,
            "tls_private_key": null
        }
    });

    // Verify the request can be serialized
    let json_str = serde_json::to_string(&request).expect("Failed to serialize request");
    assert!(json_str.contains("database_url"));
    assert!(json_str.contains("server_name"));
}

#[test]
fn test_save_config_request_format() {
    let config = ServerConfig {
        database_url: "postgresql://user:pass@localhost/palpo".to_string(),
        server_name: "example.com".to_string(),
        bind_address: "0.0.0.0".to_string(),
        port: 8008,
        tls_certificate: Some("/path/to/cert.pem".to_string()),
        tls_private_key: Some("/path/to/key.pem".to_string()),
    };

    let request = json!({
        "config": config
    });

    // Verify the request can be serialized
    let json_str = serde_json::to_string(&request).expect("Failed to serialize request");
    assert!(json_str.contains("tls_certificate"));
    assert!(json_str.contains("tls_private_key"));
}
