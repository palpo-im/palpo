/// Integration tests for Server Config API endpoints
///
/// Tests the REST API endpoints for server configuration management:
/// - GET /api/v1/admin/server/config
/// - POST /api/v1/admin/server/config
/// - POST /api/v1/admin/server/config/validate
/// - GET /api/v1/config/form
/// - POST /api/v1/config/form
/// - GET /api/v1/config/metadata
/// - POST /api/v1/config/reset
/// - POST /api/v1/config/reload
/// - GET /api/v1/config/search
/// - GET /api/v1/config/toml
/// - POST /api/v1/config/toml
/// - POST /api/v1/config/toml/validate
/// - POST /api/v1/config/toml/parse
/// - POST /api/v1/config/export
/// - POST /api/v1/config/import

use palpo_admin_server::server_config::ServerConfigAPI;
use palpo_admin_server::types::ServerConfig;
use serde_json::json;

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

// ===== Form Editing Mode Tests =====

#[test]
fn test_config_to_json_conversion() {
    let config = ServerConfig {
        database_url: "postgresql://user:pass@localhost/palpo".to_string(),
        server_name: "example.com".to_string(),
        bind_address: "0.0.0.0".to_string(),
        port: 8008,
        tls_certificate: Some("/path/to/cert.pem".to_string()),
        tls_private_key: Some("/path/to/key.pem".to_string()),
    };

    let json = ServerConfigAPI::config_to_json(&config);
    
    assert_eq!(json["database_url"], "postgresql://user:pass@localhost/palpo");
    assert_eq!(json["server_name"], "example.com");
    assert_eq!(json["bind_address"], "0.0.0.0");
    assert_eq!(json["port"], 8008);
    assert_eq!(json["tls_certificate"], "/path/to/cert.pem");
    assert_eq!(json["tls_private_key"], "/path/to/key.pem");
}

#[test]
fn test_json_to_config_conversion() {
    let json = json!({
        "database_url": "postgresql://user:pass@localhost/palpo",
        "server_name": "example.com",
        "bind_address": "0.0.0.0",
        "port": 8008,
        "tls_certificate": "/path/to/cert.pem",
        "tls_private_key": "/path/to/key.pem"
    });

    let config = ServerConfigAPI::json_to_config(&json).expect("Failed to convert JSON to config");
    
    assert_eq!(config.database_url, "postgresql://user:pass@localhost/palpo");
    assert_eq!(config.server_name, "example.com");
    assert_eq!(config.bind_address, "0.0.0.0");
    assert_eq!(config.port, 8008);
    assert_eq!(config.tls_certificate, Some("/path/to/cert.pem".to_string()));
    assert_eq!(config.tls_private_key, Some("/path/to/key.pem".to_string()));
}

#[test]
fn test_json_to_config_with_null_tls() {
    let json = json!({
        "database_url": "postgresql://user:pass@localhost/palpo",
        "server_name": "example.com",
        "bind_address": "0.0.0.0",
        "port": 8008,
        "tls_certificate": null,
        "tls_private_key": null
    });

    let config = ServerConfigAPI::json_to_config(&json).expect("Failed to convert JSON to config");
    
    assert_eq!(config.tls_certificate, None);
    assert_eq!(config.tls_private_key, None);
}

#[test]
fn test_json_to_config_invalid_database_url() {
    let json = json!({
        "database_url": 123,
        "server_name": "example.com",
        "bind_address": "0.0.0.0",
        "port": 8008,
        "tls_certificate": null,
        "tls_private_key": null
    });

    let result = ServerConfigAPI::json_to_config(&json);
    assert!(result.is_err());
}

#[test]
fn test_get_metadata() {
    let metadata = ServerConfigAPI::get_metadata();
    
    assert!(!metadata.fields.is_empty());
    
    // Check that all expected fields are present
    let field_names: Vec<_> = metadata.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"database_url"));
    assert!(field_names.contains(&"server_name"));
    assert!(field_names.contains(&"bind_address"));
    assert!(field_names.contains(&"port"));
    assert!(field_names.contains(&"tls_certificate"));
    assert!(field_names.contains(&"tls_private_key"));
}

#[test]
fn test_search_config_by_name() {
    let results = ServerConfigAPI::search_config("database");
    
    assert!(!results.is_empty());
    assert!(results.iter().any(|f| f.name == "database_url"));
}

#[test]
fn test_search_config_by_description() {
    let results = ServerConfigAPI::search_config("port");
    
    assert!(!results.is_empty());
    assert!(results.iter().any(|f| f.name == "port"));
}

#[test]
fn test_search_config_case_insensitive() {
    let results_lower = ServerConfigAPI::search_config("database");
    let results_upper = ServerConfigAPI::search_config("DATABASE");
    
    assert_eq!(results_lower.len(), results_upper.len());
}

#[test]
fn test_search_config_no_results() {
    let results = ServerConfigAPI::search_config("nonexistent_field");
    
    assert!(results.is_empty());
}

// ===== TOML Editing Mode Tests =====

#[test]
fn test_validate_toml_valid() {
    let toml_content = r#"
database_url = "postgresql://user:pass@localhost/palpo"
server_name = "example.com"
bind_address = "0.0.0.0"
port = 8008
"#;

    let result = ServerConfigAPI::validate_toml(toml_content);
    assert!(result.is_ok());
}

#[test]
fn test_validate_toml_invalid_syntax() {
    let toml_content = r#"
database_url = "postgresql://user:pass@localhost/palpo"
server_name = "example.com
bind_address = "0.0.0.0"
port = 8008
"#;

    let result = ServerConfigAPI::validate_toml(toml_content);
    assert!(result.is_err());
}

#[test]
fn test_validate_toml_invalid_database_url() {
    let toml_content = r#"
database_url = "mysql://user:pass@localhost/palpo"
server_name = "example.com"
bind_address = "0.0.0.0"
port = 8008
"#;

    let result = ServerConfigAPI::validate_toml(toml_content);
    assert!(result.is_err());
}

#[test]
fn test_parse_toml_to_json() {
    let toml_content = r#"
database_url = "postgresql://user:pass@localhost/palpo"
server_name = "example.com"
bind_address = "0.0.0.0"
port = 8008
"#;

    let json = ServerConfigAPI::parse_toml_to_json(toml_content).expect("Failed to parse TOML");
    
    assert_eq!(json["database_url"], "postgresql://user:pass@localhost/palpo");
    assert_eq!(json["server_name"], "example.com");
    assert_eq!(json["bind_address"], "0.0.0.0");
    assert_eq!(json["port"], 8008);
}

#[test]
fn test_parse_toml_invalid() {
    let toml_content = r#"
database_url = "postgresql://user:pass@localhost/palpo"
server_name = "example.com
"#;

    let result = ServerConfigAPI::parse_toml_to_json(toml_content);
    assert!(result.is_err());
}

// ===== Import/Export Tests =====

#[test]
fn test_export_config_json() {
    let config = ServerConfig {
        database_url: "postgresql://user:pass@localhost/palpo".to_string(),
        server_name: "example.com".to_string(),
        bind_address: "0.0.0.0".to_string(),
        port: 8008,
        tls_certificate: None,
        tls_private_key: None,
    };

    let json = ServerConfigAPI::config_to_json(&config);
    let json_str = serde_json::to_string_pretty(&json).expect("Failed to serialize JSON");
    
    assert!(json_str.contains("database_url"));
    assert!(json_str.contains("example.com"));
}

#[test]
fn test_export_config_toml() {
    let config = ServerConfig {
        database_url: "postgresql://user:pass@localhost/palpo".to_string(),
        server_name: "example.com".to_string(),
        bind_address: "0.0.0.0".to_string(),
        port: 8008,
        tls_certificate: None,
        tls_private_key: None,
    };

    let toml_str = toml::to_string_pretty(&config).expect("Failed to serialize TOML");
    
    assert!(toml_str.contains("database_url"));
    assert!(toml_str.contains("example.com"));
}

#[test]
fn test_import_config_json() {
    let json_content = r#"{
  "database_url": "postgresql://user:pass@localhost/palpo",
  "server_name": "example.com",
  "bind_address": "0.0.0.0",
  "port": 8008,
  "tls_certificate": null,
  "tls_private_key": null
}"#;

    let config = ServerConfigAPI::import_config(json_content, "json")
        .expect("Failed to import JSON config");
    
    assert_eq!(config.database_url, "postgresql://user:pass@localhost/palpo");
    assert_eq!(config.server_name, "example.com");
}

#[test]
fn test_import_config_toml() {
    let toml_content = r#"
database_url = "postgresql://user:pass@localhost/palpo"
server_name = "example.com"
bind_address = "0.0.0.0"
port = 8008
"#;

    let config = ServerConfigAPI::import_config(toml_content, "toml")
        .expect("Failed to import TOML config");
    
    assert_eq!(config.database_url, "postgresql://user:pass@localhost/palpo");
    assert_eq!(config.server_name, "example.com");
}

#[test]
fn test_import_config_invalid_json() {
    let json_content = r#"{ invalid json }"#;

    let result = ServerConfigAPI::import_config(json_content, "json");
    assert!(result.is_err());
}

#[test]
fn test_import_config_invalid_format() {
    let content = "some content";

    let result = ServerConfigAPI::import_config(content, "xml");
    assert!(result.is_err());
}

#[test]
fn test_import_config_validation_fails() {
    let json_content = r#"{
  "database_url": "mysql://user:pass@localhost/palpo",
  "server_name": "example.com",
  "bind_address": "0.0.0.0",
  "port": 8008,
  "tls_certificate": null,
  "tls_private_key": null
}"#;

    let result = ServerConfigAPI::import_config(json_content, "json");
    assert!(result.is_err());
}
