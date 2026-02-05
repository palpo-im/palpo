//! Tests for the API client service

#[cfg(test)]
mod tests {
    use super::super::api_client::*;
    use crate::models::WebConfigError;
    use wasm_bindgen_test::*;
    use serde::{Deserialize, Serialize};

    wasm_bindgen_test_configure!(run_in_browser);

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestData {
        message: String,
        value: i32,
    }

    #[wasm_bindgen_test]
    fn test_request_config_creation() {
        let config = RequestConfig::new(HttpMethod::Get, "http://example.com/api");
        assert_eq!(config.method, HttpMethod::Get);
        assert_eq!(config.url, "http://example.com/api");
        assert!(config.require_auth);
        assert_eq!(config.retry_count, 0);
        assert!(config.timeout.is_none());
    }

    #[wasm_bindgen_test]
    fn test_request_config_builder() {
        let test_data = TestData {
            message: "test".to_string(),
            value: 42,
        };

        let config = RequestConfig::new(HttpMethod::Post, "http://example.com/api")
            .with_header("X-Custom", "value")
            .with_json_body(&test_data)
            .unwrap()
            .with_timeout(5000)
            .with_retry(3)
            .without_auth();

        assert_eq!(config.method, HttpMethod::Post);
        assert_eq!(config.headers.get("X-Custom"), Some(&"value".to_string()));
        assert_eq!(config.headers.get("Content-Type"), Some(&"application/json".to_string()));
        assert!(config.body.is_some());
        assert_eq!(config.timeout, Some(5000));
        assert_eq!(config.retry_count, 3);
        assert!(!config.require_auth);
    }

    #[wasm_bindgen_test]
    fn test_http_method_as_str() {
        assert_eq!(HttpMethod::Get.as_str(), "GET");
        assert_eq!(HttpMethod::Post.as_str(), "POST");
        assert_eq!(HttpMethod::Put.as_str(), "PUT");
        assert_eq!(HttpMethod::Delete.as_str(), "DELETE");
        assert_eq!(HttpMethod::Patch.as_str(), "PATCH");
    }

    #[wasm_bindgen_test]
    fn test_token_manager() {
        let token_manager = TokenManager::new("test_token_key");
        
        // Initially no token
        assert!(!token_manager.has_token());
        assert_eq!(token_manager.get_token().unwrap(), None);
        
        // Store token
        token_manager.store_token("test123").unwrap();
        assert!(token_manager.has_token());
        
        // Retrieve token
        let token = token_manager.get_token().unwrap();
        assert_eq!(token, Some("test123".to_string()));
        
        // Clear token
        token_manager.clear_token().unwrap();
        assert!(!token_manager.has_token());
        assert_eq!(token_manager.get_token().unwrap(), None);
    }

    #[wasm_bindgen_test]
    fn test_api_client_creation() {
        let client = ApiClient::new("http://localhost:8008");
        assert_eq!(client.base_url, "http://localhost:8008");
        assert!(!client.has_token());
        assert_eq!(client.default_timeout, 30000);
        assert_eq!(client.default_retry_count, 2);
    }

    #[wasm_bindgen_test]
    fn test_api_client_token_management() {
        let client = ApiClient::new("http://test.example.com");
        
        // Initially no token
        assert!(!client.has_token());
        assert_eq!(client.get_token().unwrap(), None);
        
        // Set token
        client.set_token("test_token_123").unwrap();
        assert!(client.has_token());
        assert_eq!(client.get_token().unwrap(), Some("test_token_123".to_string()));
        
        // Clear token
        client.clear_token().unwrap();
        assert!(!client.has_token());
        assert_eq!(client.get_token().unwrap(), None);
    }

    #[wasm_bindgen_test]
    fn test_global_api_client() {
        // Initialize global client
        init_api_client("http://test.example.com");
        
        // Get global client
        let client = get_api_client().unwrap();
        assert_eq!(client.base_url, "http://test.example.com");
        
        // Test global convenience functions
        assert!(!has_auth_token());
        
        set_auth_token("global_test_token").unwrap();
        assert!(has_auth_token());
        
        clear_auth_token().unwrap();
        assert!(!has_auth_token());
    }

    #[wasm_bindgen_test]
    fn test_auth_interceptor() {
        let token_manager = TokenManager::new("test_auth_interceptor");
        let interceptor = AuthInterceptor::new(token_manager.clone());
        
        // Test without token - should fail for auth-required requests
        let mut config = RequestConfig::new(HttpMethod::Get, "http://example.com/api");
        let result = interceptor.intercept(&mut config);
        assert!(result.is_err());
        
        // Test with token - should add Authorization header
        token_manager.store_token("test_auth_token").unwrap();
        let mut config = RequestConfig::new(HttpMethod::Get, "http://example.com/api");
        interceptor.intercept(&mut config).unwrap();
        assert_eq!(
            config.headers.get("Authorization"),
            Some(&"Bearer test_auth_token".to_string())
        );
        
        // Test without auth requirement - should not fail
        let mut config = RequestConfig::new(HttpMethod::Get, "http://example.com/api")
            .without_auth();
        token_manager.clear_token().unwrap();
        let result = interceptor.intercept(&mut config);
        assert!(result.is_ok());
        assert!(!config.headers.contains_key("Authorization"));
    }

    #[wasm_bindgen_test]
    fn test_request_config_json_body() {
        let test_data = TestData {
            message: "Hello, World!".to_string(),
            value: 123,
        };

        let config = RequestConfig::new(HttpMethod::Post, "http://example.com/api")
            .with_json_body(&test_data)
            .unwrap();

        assert!(config.body.is_some());
        assert_eq!(
            config.headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );

        // Verify JSON serialization
        let body = config.body.unwrap();
        let parsed: TestData = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed, test_data);
    }

    #[wasm_bindgen_test]
    fn test_error_handling() {
        // Test WebConfigError auth error detection
        let auth_error = WebConfigError::auth("Authentication failed");
        assert!(auth_error.is_auth_error());
        assert!(!auth_error.is_permission_error());
        assert!(auth_error.is_client_error());
        assert!(!auth_error.is_server_error());

        // Test WebConfigError permission error detection
        let perm_error = WebConfigError::permission("Access denied");
        assert!(!perm_error.is_auth_error());
        assert!(perm_error.is_permission_error());
        assert!(perm_error.is_client_error());
        assert!(!perm_error.is_server_error());

        // Test WebConfigError API error with status codes
        let api_error_401 = WebConfigError::api_with_status("Unauthorized", 401);
        assert!(api_error_401.is_auth_error());
        assert!(api_error_401.is_client_error());

        let api_error_403 = WebConfigError::api_with_status("Forbidden", 403);
        assert!(api_error_403.is_permission_error());
        assert!(api_error_403.is_client_error());

        let api_error_500 = WebConfigError::api_with_status("Internal Server Error", 500);
        assert!(!api_error_500.is_auth_error());
        assert!(!api_error_500.is_permission_error());
        assert!(!api_error_500.is_client_error());
        assert!(api_error_500.is_server_error());
    }

    #[wasm_bindgen_test]
    fn test_api_error() {
        let api_error = crate::models::ApiError::new("Test error");
        assert_eq!(api_error.message, "Test error");
        assert_eq!(api_error.status_code, None);
        assert_eq!(api_error.error_code, None);

        let api_error_with_status = crate::models::ApiError::with_status("Bad request", 400);
        assert_eq!(api_error_with_status.message, "Bad request");
        assert_eq!(api_error_with_status.status_code, Some(400));
        assert!(api_error_with_status.is_client_error());
        assert!(!api_error_with_status.is_server_error());

        let api_error_with_code = crate::models::ApiError::with_code("Validation failed", "VALIDATION_ERROR");
        assert_eq!(api_error_with_code.message, "Validation failed");
        assert_eq!(api_error_with_code.error_code, Some("VALIDATION_ERROR".to_string()));
    }
}