// Unit tests for TOML validation functionality
// These tests verify TOML parsing, validation, and error handling

#[test]
fn test_toml_basic_parsing() {
    let toml_content = r#"
[server]
server_name = "example.com"
port = 8008
"#;
    
    let result: Result<toml::Table, _> = toml::from_str(toml_content);
    assert!(result.is_ok(), "Basic TOML should parse successfully");
}

#[test]
fn test_toml_invalid_syntax() {
    let invalid_toml = r#"
[server
server_name = "example.com"
"#;
    
    let result: Result<toml::Table, _> = toml::from_str(invalid_toml);
    assert!(result.is_err(), "Invalid TOML syntax should fail");
}

#[test]
fn test_toml_string_values() {
    let toml_content = r#"
[server]
server_name = "example.com"
description = "A test server"
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert_eq!(
        table["server"]["server_name"].as_str().unwrap(),
        "example.com"
    );
}

#[test]
fn test_toml_integer_values() {
    let toml_content = r#"
[server]
port = 8008
max_connections = 1000
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert_eq!(table["server"]["port"].as_integer().unwrap(), 8008);
    assert_eq!(table["server"]["max_connections"].as_integer().unwrap(), 1000);
}

#[test]
fn test_toml_boolean_values() {
    let toml_content = r#"
[server]
enable_federation = true
enable_tls = false
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert_eq!(table["server"]["enable_federation"].as_bool().unwrap(), true);
    assert_eq!(table["server"]["enable_tls"].as_bool().unwrap(), false);
}

#[test]
fn test_toml_float_values() {
    let toml_content = r#"
[server]
timeout = 30.5
retry_delay = 1.5
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert_eq!(table["server"]["timeout"].as_float().unwrap(), 30.5);
    assert_eq!(table["server"]["retry_delay"].as_float().unwrap(), 1.5);
}

#[test]
fn test_toml_array_values() {
    let toml_content = r#"
[server]
allowed_origins = ["http://localhost:3000", "https://example.com"]
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    let origins = table["server"]["allowed_origins"].as_array().unwrap();
    assert_eq!(origins.len(), 2);
    assert_eq!(origins[0].as_str().unwrap(), "http://localhost:3000");
}

#[test]
fn test_toml_nested_tables() {
    let toml_content = r#"
[server]
name = "example.com"

[server.tls]
enabled = true
cert_path = "/etc/certs/cert.pem"
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert!(table["server"]["tls"].is_table());
    assert_eq!(table["server"]["tls"]["enabled"].as_bool().unwrap(), true);
}

#[test]
fn test_toml_array_of_tables() {
    let toml_content = r#"
[[listeners]]
address = "0.0.0.0"
port = 8008

[[listeners]]
address = "127.0.0.1"
port = 8009
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    let listeners = table["listeners"].as_array().unwrap();
    assert_eq!(listeners.len(), 2);
}

#[test]
fn test_toml_with_comments() {
    let toml_content = r#"
# Server configuration
[server]
server_name = "example.com"  # The server name
port = 8008  # The listening port
"#;
    
    let result: Result<toml::Table, _> = toml::from_str(toml_content);
    assert!(result.is_ok(), "TOML with comments should parse");
}

#[test]
fn test_toml_serialization() {
    let toml_content = r#"
[server]
server_name = "example.com"
port = 8008
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    let serialized = toml::to_string_pretty(&table).unwrap();
    
    // Verify serialized content contains expected values
    assert!(serialized.contains("server_name"));
    assert!(serialized.contains("example.com"));
    assert!(serialized.contains("port"));
    assert!(serialized.contains("8008"));
}

#[test]
fn test_toml_round_trip() {
    let original = r#"
[server]
server_name = "example.com"
port = 8008

[database]
connection_string = "postgresql://localhost/palpo"
"#;
    
    // Parse
    let table: toml::Table = toml::from_str(original).unwrap();
    
    // Serialize
    let serialized = toml::to_string_pretty(&table).unwrap();
    
    // Parse again
    let reparsed: toml::Table = toml::from_str(&serialized).unwrap();
    
    // Verify values are preserved
    assert_eq!(
        table["server"]["server_name"],
        reparsed["server"]["server_name"]
    );
    assert_eq!(
        table["database"]["connection_string"],
        reparsed["database"]["connection_string"]
    );
}

#[test]
fn test_toml_missing_required_field() {
    let toml_content = r#"
[server]
port = 8008
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    
    // Verify missing field returns None
    assert!(table["server"]["server_name"].is_none());
}

#[test]
fn test_toml_type_mismatch() {
    let toml_content = r#"
[server]
port = "8008"
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    
    // Port is a string, not an integer
    assert!(table["server"]["port"].as_str().is_some());
    assert!(table["server"]["port"].as_integer().is_none());
}

#[test]
fn test_toml_duplicate_keys() {
    let invalid_toml = r#"
[server]
port = 8008
port = 9009
"#;
    
    let result: Result<toml::Table, _> = toml::from_str(invalid_toml);
    assert!(result.is_err(), "Duplicate keys should cause parse error");
}

#[test]
fn test_toml_empty_content() {
    let empty_toml = "";
    
    let result: Result<toml::Table, _> = toml::from_str(empty_toml);
    assert!(result.is_ok(), "Empty TOML should parse as empty table");
    
    let table = result.unwrap();
    assert!(table.is_empty());
}

#[test]
fn test_toml_only_comments() {
    let comments_only = r#"
# This is a comment
# Another comment
"#;
    
    let result: Result<toml::Table, _> = toml::from_str(comments_only);
    assert!(result.is_ok(), "TOML with only comments should parse");
    
    let table = result.unwrap();
    assert!(table.is_empty());
}

#[test]
fn test_toml_multiline_string() {
    let toml_content = r#"
[server]
description = """
This is a
multiline
description
"""
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    let description = table["server"]["description"].as_str().unwrap();
    assert!(description.contains("multiline"));
}

#[test]
fn test_toml_special_characters() {
    let toml_content = r#"
[server]
server_name = "example.com"
special_chars = "!@#$%^&*()"
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert_eq!(
        table["server"]["special_chars"].as_str().unwrap(),
        "!@#$%^&*()"
    );
}

#[test]
fn test_toml_unicode_characters() {
    let toml_content = r#"
[server]
server_name = "例え.com"
description = "日本語のテスト"
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert_eq!(table["server"]["server_name"].as_str().unwrap(), "例え.com");
}

#[test]
fn test_toml_error_message_format() {
    let invalid_toml = r#"
[server
server_name = "example.com"
"#;
    
    let result: Result<toml::Table, _> = toml::from_str(invalid_toml);
    assert!(result.is_err());
    
    let error = result.unwrap_err();
    let error_msg = error.to_string();
    
    // Error message should not be empty
    assert!(!error_msg.is_empty());
}

#[test]
fn test_toml_large_numbers() {
    let toml_content = r#"
[server]
max_file_size = 1073741824
timeout_ms = 30000
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert_eq!(table["server"]["max_file_size"].as_integer().unwrap(), 1073741824);
    assert_eq!(table["server"]["timeout_ms"].as_integer().unwrap(), 30000);
}

#[test]
fn test_toml_negative_numbers() {
    let toml_content = r#"
[server]
retry_count = -1
timeout = -30.5
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert_eq!(table["server"]["retry_count"].as_integer().unwrap(), -1);
    assert_eq!(table["server"]["timeout"].as_float().unwrap(), -30.5);
}

#[test]
fn test_toml_scientific_notation() {
    let toml_content = r#"
[server]
timeout = 1e-3
large_number = 1.5e10
"#;
    
    let table: toml::Table = toml::from_str(toml_content).unwrap();
    assert_eq!(table["server"]["timeout"].as_float().unwrap(), 0.001);
    assert_eq!(table["server"]["large_number"].as_float().unwrap(), 1.5e10);
}
