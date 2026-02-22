//! WASM-compatible API client with request interceptors and token management

use crate::models::{ApiError, WebConfigError, WebConfigResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{window, Request, RequestInit, RequestMode, Response};

/// HTTP methods supported by the API client
#[derive(Debug, Clone, PartialEq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
        }
    }
}

/// Request configuration for API calls
#[derive(Debug, Clone)]
pub struct RequestConfig {
    pub method: HttpMethod,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub timeout: Option<u32>,
    pub retry_count: u32,
    pub require_auth: bool,
}

impl RequestConfig {
    pub fn new(method: HttpMethod, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: HashMap::new(),
            body: None,
            timeout: None,
            retry_count: 0,
            require_auth: true,
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn with_json_body<T: Serialize>(mut self, data: &T) -> WebConfigResult<Self> {
        let json_data = serde_json::to_string(data)
            .map_err(|e| WebConfigError::client(format!("Failed to serialize request body: {}", e)))?;
        self.body = Some(json_data);
        self.headers.insert("Content-Type".to_string(), "application/json".to_string());
        Ok(self)
    }

    pub fn with_timeout(mut self, timeout_ms: u32) -> Self {
        self.timeout = Some(timeout_ms);
        self
    }

    pub fn with_retry(mut self, count: u32) -> Self {
        self.retry_count = count;
        self
    }

    pub fn without_auth(mut self) -> Self {
        self.require_auth = false;
        self
    }
}

/// Request interceptor trait for modifying requests before sending
pub trait RequestInterceptor {
    fn intercept(&self, config: &mut RequestConfig) -> WebConfigResult<()>;
}

/// Response interceptor trait for processing responses
pub trait ResponseInterceptor {
    fn intercept(&self, response: &Response, config: &RequestConfig) -> WebConfigResult<()>;
}

/// Token manager for handling authentication tokens
#[derive(Clone)]
pub struct TokenManager {
    storage_key: String,
}

impl TokenManager {
    pub fn new(storage_key: impl Into<String>) -> Self {
        Self {
            storage_key: storage_key.into(),
        }
    }

    /// Store authentication token
    pub fn store_token(&self, token: &str) -> WebConfigResult<()> {
        let window = window().ok_or_else(|| WebConfigError::client("No window object available"))?;
        let storage = window
            .local_storage()
            .map_err(|_| WebConfigError::client("Failed to access local storage"))?
            .ok_or_else(|| WebConfigError::client("Local storage not available"))?;

        storage
            .set_item(&self.storage_key, token)
            .map_err(|_| WebConfigError::client("Failed to store auth token"))?;

        Ok(())
    }

    /// Get stored authentication token
    pub fn get_token(&self) -> WebConfigResult<Option<String>> {
        let window = window().ok_or_else(|| WebConfigError::client("No window object available"))?;
        let storage = window
            .local_storage()
            .map_err(|_| WebConfigError::client("Failed to access local storage"))?
            .ok_or_else(|| WebConfigError::client("Local storage not available"))?;

        let token = storage
            .get_item(&self.storage_key)
            .map_err(|_| WebConfigError::client("Failed to retrieve auth token"))?;

        Ok(token)
    }

    /// Clear stored authentication token
    pub fn clear_token(&self) -> WebConfigResult<()> {
        let window = window().ok_or_else(|| WebConfigError::client("No window object available"))?;
        let storage = window
            .local_storage()
            .map_err(|_| WebConfigError::client("Failed to access local storage"))?
            .ok_or_else(|| WebConfigError::client("Local storage not available"))?;

        storage
            .remove_item(&self.storage_key)
            .map_err(|_| WebConfigError::client("Failed to clear auth token"))?;

        Ok(())
    }

    /// Check if token exists
    pub fn has_token(&self) -> bool {
        self.get_token().map(|t| t.is_some()).unwrap_or(false)
    }
}

/// Authentication interceptor that adds Bearer token to requests
pub struct AuthInterceptor {
    token_manager: TokenManager,
}

impl AuthInterceptor {
    pub fn new(token_manager: TokenManager) -> Self {
        Self { token_manager }
    }
}

impl RequestInterceptor for AuthInterceptor {
    fn intercept(&self, config: &mut RequestConfig) -> WebConfigResult<()> {
        if config.require_auth {
            if let Some(token) = self.token_manager.get_token()? {
                config.headers.insert("Authorization".to_string(), format!("Bearer {}", token));
            } else {
                return Err(WebConfigError::auth("No authentication token available"));
            }
        }
        Ok(())
    }
}

/// Error handling interceptor for processing error responses
pub struct ErrorInterceptor;

impl ResponseInterceptor for ErrorInterceptor {
    fn intercept(&self, response: &Response, _config: &RequestConfig) -> WebConfigResult<()> {
        if !response.ok() {
            let status = response.status();
            let status_text = response.status_text();
            
            let error = match status {
                401 => WebConfigError::auth(format!("Unauthorized: {}", status_text)),
                403 => WebConfigError::permission(format!("Forbidden: {}", status_text)),
                404 => WebConfigError::api_with_status(format!("Not found: {}", status_text), status),
                422 => WebConfigError::validation(format!("Validation error: {}", status_text)),
                500..=599 => WebConfigError::api_with_status(format!("Server error: {}", status_text), status),
                _ => WebConfigError::api_with_status(format!("HTTP error: {}", status_text), status),
            };
            
            return Err(error);
        }
        Ok(())
    }
}

/// Main API client with WASM compatibility
#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
    token_manager: TokenManager,
    request_interceptors: Rc<RefCell<Vec<Box<dyn RequestInterceptor>>>>,
    response_interceptors: Rc<RefCell<Vec<Box<dyn ResponseInterceptor>>>>,
    default_timeout: u32,
    default_retry_count: u32,
}

impl ApiClient {
    /// Create a new API client
    pub fn new(base_url: impl Into<String>) -> Self {
        let token_manager = TokenManager::new("auth_token");
        let mut client = Self {
            base_url: base_url.into(),
            token_manager: token_manager.clone(),
            request_interceptors: Rc::new(RefCell::new(Vec::new())),
            response_interceptors: Rc::new(RefCell::new(Vec::new())),
            default_timeout: 30000, // 30 seconds
            default_retry_count: 2,
        };

        // Add default interceptors
        client.add_request_interceptor(Box::new(AuthInterceptor::new(token_manager)));
        client.add_response_interceptor(Box::new(ErrorInterceptor));

        client
    }

    /// Get the base URL
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Add a request interceptor
    pub fn add_request_interceptor(&mut self, interceptor: Box<dyn RequestInterceptor>) {
        self.request_interceptors.borrow_mut().push(interceptor);
    }

    /// Add a response interceptor
    pub fn add_response_interceptor(&mut self, interceptor: Box<dyn ResponseInterceptor>) {
        self.response_interceptors.borrow_mut().push(interceptor);
    }

    /// Set authentication token
    pub fn set_token(&self, token: &str) -> WebConfigResult<()> {
        self.token_manager.store_token(token)
    }

    /// Get authentication token
    pub fn get_token(&self) -> WebConfigResult<Option<String>> {
        self.token_manager.get_token()
    }

    /// Clear authentication token
    pub fn clear_token(&self) -> WebConfigResult<()> {
        self.token_manager.clear_token()
    }

    /// Check if client has authentication token
    pub fn has_token(&self) -> bool {
        self.token_manager.has_token()
    }

    /// Make a GET request
    pub async fn get(&self, path: &str) -> WebConfigResult<Response> {
        let url = format!("{}{}", self.base_url, path);
        let config = RequestConfig::new(HttpMethod::Get, url);
        self.execute_request(config).await
    }

    /// Make a POST request with JSON body
    pub async fn post_json<T: Serialize>(&self, path: &str, data: &T) -> WebConfigResult<Response> {
        let url = format!("{}{}", self.base_url, path);
        let config = RequestConfig::new(HttpMethod::Post, url)
            .with_json_body(data)?;
        self.execute_request(config).await
    }

    /// Make a PUT request with JSON body
    pub async fn put_json<T: Serialize>(&self, path: &str, data: &T) -> WebConfigResult<Response> {
        let url = format!("{}{}", self.base_url, path);
        let config = RequestConfig::new(HttpMethod::Put, url)
            .with_json_body(data)?;
        self.execute_request(config).await
    }

    /// Make a DELETE request
    pub async fn delete(&self, path: &str) -> WebConfigResult<Response> {
        let url = format!("{}{}", self.base_url, path);
        let config = RequestConfig::new(HttpMethod::Delete, url);
        self.execute_request(config).await
    }

    /// Make a PATCH request with JSON body
    pub async fn patch_json<T: Serialize>(&self, path: &str, data: &T) -> WebConfigResult<Response> {
        let url = format!("{}{}", self.base_url, path);
        let config = RequestConfig::new(HttpMethod::Patch, url)
            .with_json_body(data)?;
        self.execute_request(config).await
    }

    /// Execute a custom request configuration
    pub async fn execute_request(&self, mut config: RequestConfig) -> WebConfigResult<Response> {
        // Apply default settings
        if config.timeout.is_none() {
            config.timeout = Some(self.default_timeout);
        }
        if config.retry_count == 0 {
            config.retry_count = self.default_retry_count;
        }

        // Apply request interceptors
        for interceptor in self.request_interceptors.borrow().iter() {
            interceptor.intercept(&mut config)?;
        }

        // Execute request with retry logic
        let mut last_error = None;
        for attempt in 0..=config.retry_count {
            match self.send_request(&config).await {
                Ok(response) => {
                    // Apply response interceptors
                    for interceptor in self.response_interceptors.borrow().iter() {
                        if let Err(e) = interceptor.intercept(&response, &config) {
                            // If it's an auth error and we have retries left, try token refresh
                            if e.is_auth_error() && attempt < config.retry_count {
                                if let Err(refresh_error) = self.try_refresh_token().await {
                                    web_sys::console::log_1(&format!("Token refresh failed: {}", refresh_error).into());
                                }
                                last_error = Some(e);
                                break; // Break inner loop to retry
                            }
                            return Err(e);
                        }
                    }
                    return Ok(response);
                }
                Err(e) => {
                    last_error = Some(e);
                    if attempt < config.retry_count {
                        // Wait before retry (exponential backoff)
                        let delay = 1000 * (2_u32.pow(attempt));
                        self.sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| WebConfigError::network("Request failed after all retries")))
    }

    /// Send HTTP request
    async fn send_request(&self, config: &RequestConfig) -> WebConfigResult<Response> {
        let opts = RequestInit::new();
        opts.set_method(config.method.as_str());
        opts.set_mode(RequestMode::Cors);

        // Set body if present
        if let Some(body) = &config.body {
            opts.set_body(&wasm_bindgen::JsValue::from_str(body));
        }

        let request = Request::new_with_str_and_init(&config.url, &opts)
            .map_err(|_| WebConfigError::client("Failed to create request"))?;

        // Set headers
        let headers = request.headers();
        for (key, value) in &config.headers {
            headers
                .set(key, value)
                .map_err(|_| WebConfigError::client(format!("Failed to set header: {}", key)))?;
        }

        let window = window().ok_or_else(|| WebConfigError::client("No window object available"))?;
        
        // Create fetch promise with timeout if specified
        let fetch_promise = if let Some(timeout) = config.timeout {
            self.fetch_with_timeout(window.fetch_with_request(&request), timeout)
        } else {
            window.fetch_with_request(&request)
        };

        let resp_value = JsFuture::from(fetch_promise)
            .await
            .map_err(|e| {
                let error_msg = if let Some(error) = e.as_string() {
                    error
                } else {
                    "Network request failed".to_string()
                };
                WebConfigError::network(error_msg)
            })?;

        let response: Response = resp_value
            .dyn_into()
            .map_err(|_| WebConfigError::client("Invalid response type"))?;

        Ok(response)
    }

    /// Create a fetch promise with timeout
    fn fetch_with_timeout(&self, fetch_promise: js_sys::Promise, timeout_ms: u32) -> js_sys::Promise {
        let timeout_promise = js_sys::Promise::new(&mut |_resolve, reject| {
            let reject_clone = reject.clone();
            let callback = Closure::once_into_js(move || {
                let _ = reject_clone.call1(&JsValue::NULL, &JsValue::from_str("Request timeout"));
            });
            
            let _ = window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    callback.unchecked_ref(),
                    timeout_ms as i32,
                );
        });

        js_sys::Promise::race(&js_sys::Array::of2(&fetch_promise, &timeout_promise))
    }

    /// Sleep for specified milliseconds
    async fn sleep(&self, ms: u32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32)
                .unwrap();
        });
        let _ = JsFuture::from(promise).await;
    }

    /// Try to refresh authentication token
    async fn try_refresh_token(&self) -> WebConfigResult<()> {
        // This would typically call a refresh endpoint
        // For now, we'll just clear the token to force re-authentication
        self.clear_token()?;
        Ok(())
    }

    /// Parse JSON response
    pub async fn parse_json<T: for<'de> Deserialize<'de>>(&self, response: Response) -> WebConfigResult<T> {
        let json_promise = response
            .json()
            .map_err(|_| WebConfigError::client("Failed to get JSON from response"))?;

        let json_value = JsFuture::from(json_promise)
            .await
            .map_err(|_| WebConfigError::client("Failed to parse JSON response"))?;

        let json_string = js_sys::JSON::stringify(&json_value)
            .map_err(|_| WebConfigError::client("Failed to stringify JSON"))?;

        let json_str = json_string
            .as_string()
            .ok_or_else(|| WebConfigError::client("Invalid JSON string"))?;

        serde_json::from_str(&json_str)
            .map_err(|e| WebConfigError::client(format!("Failed to deserialize JSON: {}", e)))
    }

    /// Parse error response
    pub async fn parse_error(&self, response: Response) -> ApiError {
        match self.parse_json::<serde_json::Value>(response).await {
            Ok(json) => {
                let message = json.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error")
                    .to_string();
                
                let error_code = json.get("code")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let status_code = json.get("status")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u16);

                ApiError {
                    message,
                    status_code,
                    error_code,
                    details: Some(json),
                }
            }
            Err(_) => ApiError::new("Failed to parse error response"),
        }
    }

    /// Get request with JSON response parsing
    pub async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> WebConfigResult<T> {
        let response = self.get(path).await?;
        self.parse_json(response).await
    }

    /// POST request with JSON response parsing
    pub async fn post_json_response<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        data: &T,
    ) -> WebConfigResult<R> {
        let response = self.post_json(path, data).await?;
        self.parse_json(response).await
    }

    /// PUT request with JSON response parsing
    pub async fn put_json_response<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        data: &T,
    ) -> WebConfigResult<R> {
        let response = self.put_json(path, data).await?;
        self.parse_json(response).await
    }

    /// PATCH request with JSON response parsing
    pub async fn patch_json_response<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        data: &T,
    ) -> WebConfigResult<R> {
        let response = self.patch_json(path, data).await?;
        self.parse_json(response).await
    }
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new("http://localhost:8008")
    }
}

thread_local! {
    static API_CLIENT: RefCell<Option<ApiClient>> = RefCell::new(None);
}

/// Initialize the global API client
pub fn init_api_client(base_url: impl Into<String>) {
    let client = ApiClient::new(base_url);
    API_CLIENT.with(|api| {
        *api.borrow_mut() = Some(client);
    });
}

/// Get the global API client instance
pub fn get_api_client() -> WebConfigResult<ApiClient> {
    API_CLIENT.with(|api| {
        api.borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| WebConfigError::client("API client not initialized. Call init_api_client() first."))
    })
}

/// Convenience functions using the global API client
pub async fn api_get(path: &str) -> WebConfigResult<Response> {
    get_api_client()?.get(path).await
}

pub async fn api_get_json<T: for<'de> Deserialize<'de>>(path: &str) -> WebConfigResult<T> {
    get_api_client()?.get_json(path).await
}

pub async fn api_post_json<T: Serialize>(path: &str, data: &T) -> WebConfigResult<Response> {
    get_api_client()?.post_json(path, data).await
}

pub async fn api_post_json_response<T: Serialize, R: for<'de> Deserialize<'de>>(
    path: &str,
    data: &T,
) -> WebConfigResult<R> {
    get_api_client()?.post_json_response(path, data).await
}

pub async fn api_put_json<T: Serialize>(path: &str, data: &T) -> WebConfigResult<Response> {
    get_api_client()?.put_json(path, data).await
}

pub async fn api_put_json_response<T: Serialize, R: for<'de> Deserialize<'de>>(
    path: &str,
    data: &T,
) -> WebConfigResult<R> {
    get_api_client()?.put_json_response(path, data).await
}

pub async fn api_delete(path: &str) -> WebConfigResult<Response> {
    get_api_client()?.delete(path).await
}

pub async fn api_patch_json<T: Serialize>(path: &str, data: &T) -> WebConfigResult<Response> {
    get_api_client()?.patch_json(path, data).await
}

pub async fn api_patch_json_response<T: Serialize, R: for<'de> Deserialize<'de>>(
    path: &str,
    data: &T,
) -> WebConfigResult<R> {
    get_api_client()?.patch_json_response(path, data).await
}

/// Set authentication token on global client
pub fn set_auth_token(token: &str) -> WebConfigResult<()> {
    get_api_client()?.set_token(token)
}

/// Clear authentication token from global client
pub fn clear_auth_token() -> WebConfigResult<()> {
    get_api_client()?.clear_token()
}

/// Check if global client has authentication token
pub fn has_auth_token() -> bool {
    get_api_client().map(|client| client.has_token()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_request_config_with_header() {
        let config = RequestConfig::new(HttpMethod::Post, "http://example.com/api")
            .with_header("X-Custom", "value");
        
        assert_eq!(config.headers.get("X-Custom"), Some(&"value".to_string()));
    }

    #[wasm_bindgen_test]
    fn test_token_manager() {
        let token_manager = TokenManager::new("test_token");
        
        // Initially no token
        assert!(!token_manager.has_token());
        
        // Store token
        token_manager.store_token("test123").unwrap();
        assert!(token_manager.has_token());
        
        // Retrieve token
        let token = token_manager.get_token().unwrap();
        assert_eq!(token, Some("test123".to_string()));
        
        // Clear token
        token_manager.clear_token().unwrap();
        assert!(!token_manager.has_token());
    }

    #[wasm_bindgen_test]
    fn test_global_api_client() {
        init_api_client("http://test.example.com");
        let client = get_api_client().unwrap();
        assert_eq!(client.base_url, "http://test.example.com");
    }
}