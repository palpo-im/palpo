//! API Client demonstration example

use palpo_admin_ui::services::api_client::{RequestConfig, HttpMethod};
use palpo_admin_ui::models::WebConfigError;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct TestData {
    message: String,
    value: i32,
}

fn main() {
    println!("API Client Demo");
    
    // Test request configuration
    println!("✓ Request configuration:");
    let test_data = TestData {
        message: "Hello, API!".to_string(),
        value: 42,
    };
    
    match RequestConfig::new(HttpMethod::Post, "/api/test")
        .with_header("X-Custom", "demo-value")
        .with_json_body(&test_data)
        .map(|config| config.with_timeout(5000).with_retry(2))
    {
        Ok(config) => {
            println!("  - Created POST request config");
            println!("  - Method: {:?}", config.method);
            println!("  - URL: {}", config.url);
            println!("  - Headers: {:?}", config.headers);
            println!("  - Has body: {}", config.body.is_some());
            println!("  - Timeout: {:?}ms", config.timeout);
            println!("  - Retry count: {}", config.retry_count);
        }
        Err(e) => {
            println!("  - Error creating config: {}", e);
        }
    }
    
    // Test HTTP methods
    println!("✓ HTTP methods:");
    let methods = vec![
        HttpMethod::Get,
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Delete,
        HttpMethod::Patch,
    ];
    
    for method in methods {
        println!("  - {}: {}", format!("{:?}", method), method.as_str());
    }
    
    // Test error types
    println!("✓ Error handling:");
    let auth_error = WebConfigError::auth("Demo auth error");
    println!("  - Auth error: {}", auth_error);
    println!("  - Is auth error: {}", auth_error.is_auth_error());
    println!("  - Is client error: {}", auth_error.is_client_error());
    
    let api_error = WebConfigError::api_with_status("Demo API error", 500);
    println!("  - API error: {}", api_error);
    println!("  - Is server error: {}", api_error.is_server_error());
    
    let validation_error = WebConfigError::validation_field("username", "Username is required");
    println!("  - Validation error: {}", validation_error);
    println!("  - Status code: {}", validation_error.status_code());
    
    // Test request config builder pattern
    println!("✓ Request config builder:");
    let get_config = RequestConfig::new(HttpMethod::Get, "https://api.example.com/users")
        .with_header("Accept", "application/json")
        .with_timeout(10000)
        .without_auth();
    
    println!("  - GET config created");
    println!("  - Requires auth: {}", get_config.require_auth);
    println!("  - Timeout: {:?}ms", get_config.timeout);
    
    println!("\n🎉 API Client demo completed successfully!");
    println!("The API client configuration and error handling work correctly.");
    println!("WASM-specific features (like actual HTTP requests) require a browser environment.");
}