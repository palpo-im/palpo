/// Integration tests for Server Config API endpoints

use palpo_admin_server::server_config::ServerConfigAPI;
use palpo_admin_server::types::{ServerConfig, ListenerConfig, DatabaseConfig, WellKnownConfig};
use serde_json::json;

fn make_valid_config() -> ServerConfig {
    ServerConfig {
        server_name: "example.com".to_string(),
        allow_registration: true,
        listener_configs: vec![ListenerConfig {
            address: "0.0.0.0:8008".to_string(),
        }],
        database: DatabaseConfig {
            url: "postgresql://user:pass@localhost/palpo".to_string(),
        },
        well_known: WellKnownConfig {
            server: "example.com".to_string(),
            client: "https://example.com".to_string(),
        },
    }
}

#[test]
fn test_validate_config_valid() {
    let config = make_valid_config();
    assert!(ServerConfigAPI::validate_config(&config).is_ok());
}

#[test]
fn test_config_to_json_conversion() {
    let config = make_valid_config();
    let json = ServerConfigAPI::config_to_json(&config);
    assert_eq!(json["server_name"], "example.com");
    assert!(json["database"].is_object());
}

#[test]
fn test_json_to_config_conversion() {
    let json = json!({
        "server_name": "example.com",
        "allow_registration": true,
        "listener_configs": [{"address": "0.0.0.0:8008"}],
        "database": {"url": "postgresql://user:pass@localhost/palpo"},
        "well_known": {"server": "example.com", "client": "https://example.com"}
    });
    let config = ServerConfigAPI::json_to_config(&json).expect("Failed to convert JSON to config");
    assert_eq!(config.server_name, "example.com");
    assert_eq!(config.database.url, "postgresql://user:pass@localhost/palpo");
}

#[test]
fn test_get_metadata() {
    let metadata = ServerConfigAPI::get_metadata();
    assert!(!metadata.fields.is_empty());
    let field_names: Vec<_> = metadata.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"server_name"));
    assert!(field_names.contains(&"database.url"));
}

#[test]
fn test_search_config_by_name() {
    let results = ServerConfigAPI::search_config("database");
    assert!(!results.is_empty());
}

#[test]
fn test_search_config_case_insensitive() {
    let results_lower = ServerConfigAPI::search_config("server");
    let results_upper = ServerConfigAPI::search_config("SERVER");
    assert_eq!(results_lower.len(), results_upper.len());
}

#[test]
fn test_search_config_no_results() {
    let results = ServerConfigAPI::search_config("nonexistent_field_xyz");
    assert!(results.is_empty());
}

// ===== TOML Editing Mode Tests =====

#[test]
fn test_validate_toml_valid() {
    // validate_toml only checks TOML syntax
    let toml_content = r#"
server_name = "example.com"
allow_registration = true

[[listeners]]
address = "0.0.0.0:8008"

[db]
url = "postgresql://user:pass@localhost/palpo"
"#;
    let result = ServerConfigAPI::validate_toml(toml_content);
    assert!(result.is_ok());
}

#[test]
fn test_validate_toml_invalid_syntax() {
    let toml_content = r#"
server_name = "example.com
allow_registration = true
"#;
    let result = ServerConfigAPI::validate_toml(toml_content);
    assert!(result.is_err());
}

#[test]
fn test_validate_toml_any_valid_syntax() {
    // validate_toml only checks TOML syntax, not content semantics
    let toml_content = r#"
database_url = "mysql://user:pass@localhost/palpo"
server_name = "example.com"
port = 8008
"#;
    let result = ServerConfigAPI::validate_toml(toml_content);
    assert!(result.is_ok());
}

#[test]
fn test_parse_toml_to_json() {
    let toml_content = r#"
server_name = "example.com"
port = 8008
"#;
    let json = ServerConfigAPI::parse_toml_to_json(toml_content).expect("Failed to parse TOML");
    assert_eq!(json["server_name"], "example.com");
    assert_eq!(json["port"], 8008);
}

#[test]
fn test_parse_toml_invalid() {
    let toml_content = r#"
server_name = "example.com
"#;
    let result = ServerConfigAPI::parse_toml_to_json(toml_content);
    assert!(result.is_err());
}

// ===== Import/Export Tests =====

#[test]
fn test_import_config_toml() {
    let toml_content = r#"
server_name = "example.com"
allow_registration = true

[[listeners]]
address = "0.0.0.0:8008"

[db]
url = "postgresql://user:pass@localhost/palpo"

[well_known]
server = "example.com"
client = "https://example.com"
"#;
    let config = ServerConfigAPI::import_config(toml_content, "toml")
        .expect("Failed to import TOML config");
    assert_eq!(config.server_name, "example.com");
    assert_eq!(config.database.url, "postgresql://user:pass@localhost/palpo");
}

#[test]
fn test_import_config_invalid_json() {
    let result = ServerConfigAPI::import_config("{ invalid json }", "json");
    assert!(result.is_err());
}

#[test]
fn test_import_config_invalid_format() {
    let result = ServerConfigAPI::import_config("some content", "xml");
    assert!(result.is_err());
}

#[test]
fn test_import_config_validation_fails() {
    // Empty server_name should fail validation
    let toml_content = r#"
server_name = ""

[[listeners]]
address = "0.0.0.0:8008"

[db]
url = "postgresql://user:pass@localhost/palpo"

[well_known]
server = "example.com"
client = "https://example.com"
"#;
    let result = ServerConfigAPI::import_config(toml_content, "toml");
    assert!(result.is_err());
}
