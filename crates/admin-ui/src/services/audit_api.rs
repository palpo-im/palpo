//! # Audit Log API Client
//!
//! This module provides a comprehensive HTTP client for communicating with the backend
//! audit log API. It handles all audit-related network operations including querying
//! logs, retrieving statistics, exporting data, and fetching individual entries.
//!
//! ## Features
//!
//! - **Async Operations**: All API calls are asynchronous and WASM-compatible
//! - **Authentication**: Supports Bearer token authentication
//! - **Error Handling**: Comprehensive error handling with detailed error messages
//! - **JSON Serialization**: Automatic serialization/deserialization of request/response data
//! - **CORS Support**: Configured for cross-origin requests
//! - **Global Instance**: Provides a global client instance for convenience
//!
//! ## Usage
//!
//! ### Basic Client Usage
//!
//! ```rust
//! let mut client = AuditApiClient::new("https://api.example.com".to_string());
//! client.set_auth_token("your_auth_token".to_string());
//!
//! // Query audit logs
//! let filter = AuditLogFilter {
//!     success: Some(false),
//!     limit: Some(10),
//!     ..Default::default()
//! };
//! let response = client.query_logs(filter).await?;
//! ```
//!
//! ### Global Client Usage
//!
//! ```rust
//! // Initialize the global client
//! init_audit_api_client("https://api.example.com".to_string());
//! set_audit_api_auth_token("your_auth_token".to_string());
//!
//! // Use convenience functions
//! let logs = query_audit_logs(filter).await?;
//! let stats = get_audit_statistics().await?;
//! ```
//!
//! ## Error Handling
//!
//! All methods return `Result<T, ApiError>` where `ApiError` contains detailed
//! information about what went wrong, including HTTP status codes for server errors.

use crate::models::{AuditLogFilter, AuditLogResponse, AuditLogEntry};
use crate::models::error::ApiError;
use crate::services::audit::AuditStatistics;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, RequestMode, Response};

/// HTTP client for audit log API operations.
///
/// This client handles all communication with the backend audit log API endpoints.
/// It provides methods for querying logs, retrieving statistics, exporting data,
/// and fetching individual entries. The client is designed to work in WASM
/// environments and supports Bearer token authentication.
///
/// # Examples
///
/// ```rust
/// let mut client = AuditApiClient::new("https://api.example.com".to_string());
/// client.set_auth_token("your_bearer_token".to_string());
///
/// // Query recent failed operations
/// let filter = AuditLogFilter {
///     success: Some(false),
///     limit: Some(20),
///     ..Default::default()
/// };
/// let response = client.query_logs(filter).await?;
/// println!("Found {} failed operations", response.entries.len());
/// ```
pub struct AuditApiClient {
    base_url: String,
    auth_token: Option<String>,
}

impl AuditApiClient {
    /// Creates a new audit API client instance.
    ///
    /// # Parameters
    ///
    /// - `base_url`: The base URL of the API server (e.g., "https://api.example.com")
    ///
    /// # Returns
    ///
    /// Returns a new `AuditApiClient` instance with no authentication token set.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let client = AuditApiClient::new("https://matrix.example.com".to_string());
    /// ```
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            auth_token: None,
        }
    }

    /// Sets the authentication token for API requests.
    ///
    /// This token will be included as a Bearer token in the Authorization header
    /// of all subsequent API requests.
    ///
    /// # Parameters
    ///
    /// - `token`: The authentication token to use for API requests
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut client = AuditApiClient::new("https://api.example.com".to_string());
    /// client.set_auth_token("eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9...".to_string());
    /// ```
    pub fn set_auth_token(&mut self, token: String) {
        self.auth_token = Some(token);
    }

    /// Queries audit logs from the backend API.
    ///
    /// This method sends a POST request to the `/api/admin/audit/logs` endpoint
    /// with the provided filter criteria. The backend will return matching audit
    /// log entries along with pagination metadata.
    ///
    /// # Parameters
    ///
    /// - `filter`: An `AuditLogFilter` specifying the query criteria
    ///
    /// # Returns
    ///
    /// Returns an `AuditLogResponse` containing the matching entries and pagination
    /// information, or an `ApiError` if the request fails.
    ///
    /// # Errors
    ///
    /// - Network errors (connection failures, timeouts)
    /// - Authentication errors (401 Unauthorized)
    /// - Authorization errors (403 Forbidden)
    /// - Server errors (5xx status codes)
    /// - JSON parsing errors
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Query all config updates from the last week
    /// let filter = AuditLogFilter {
    ///     action: Some(AuditAction::ConfigUpdate),
    ///     start_time: Some(SystemTime::now() - Duration::from_secs(7 * 24 * 3600)),
    ///     ..Default::default()
    /// };
    /// let response = client.query_logs(filter).await?;
    /// println!("Found {} config updates", response.entries.len());
    /// ```
    /// Queries audit logs from the backend API.
    ///
    /// This method sends a POST request to the `/api/admin/audit/logs` endpoint
    /// with the provided filter criteria. The backend will return matching audit
    /// log entries along with pagination metadata.
    ///
    /// # Parameters
    ///
    /// - `filter`: An `AuditLogFilter` specifying the query criteria
    ///
    /// # Returns
    ///
    /// Returns an `AuditLogResponse` containing the matching entries and pagination
    /// information, or an `ApiError` if the request fails.
    ///
    /// # Errors
    ///
    /// - Network errors (connection failures, timeouts)
    /// - Authentication errors (401 Unauthorized)
    /// - Authorization errors (403 Forbidden)
    /// - Server errors (5xx status codes)
    /// - JSON parsing errors
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Query all config updates from the last week
    /// let filter = AuditLogFilter {
    ///     action: Some(AuditAction::ConfigUpdate),
    ///     start_time: Some(SystemTime::now() - Duration::from_secs(7 * 24 * 3600)),
    ///     ..Default::default()
    /// };
    /// let response = client.query_logs(filter).await?;
    /// println!("Found {} config updates", response.entries.len());
    /// ```
    pub async fn query_logs(&self, filter: AuditLogFilter) -> Result<AuditLogResponse, ApiError> {
        let url = format!("{}/api/admin/audit/logs", self.base_url);
        
        let request_body = serde_json::to_string(&filter)
            .map_err(|e| ApiError::new(format!("Failed to serialize filter: {}", e)))?;

        let opts = RequestInit::new();
        opts.set_method("POST");
        opts.set_mode(RequestMode::Cors);
        opts.set_body(&wasm_bindgen::JsValue::from_str(&request_body));

        let request = Request::new_with_str_and_init(&url, &opts)
            .map_err(|_| ApiError::new("Failed to create request"))?;

        // Add authorization header if token is available
        if let Some(ref token) = self.auth_token {
            request.headers().set("Authorization", &format!("Bearer {}", token))
                .map_err(|_| ApiError::new("Failed to set authorization header"))?;
        }

        request.headers().set("Content-Type", "application/json")
            .map_err(|_| ApiError::new("Failed to set content type header"))?;

        let window = web_sys::window().ok_or_else(|| ApiError::new("No window object"))?;
        let response_value = JsFuture::from(window.fetch_with_request(&request)).await
            .map_err(|_| ApiError::new("Network request failed"))?;

        let response: Response = response_value.dyn_into()
            .map_err(|_| ApiError::new("Invalid response type"))?;

        if !response.ok() {
            let status = response.status();
            let error_text = JsFuture::from(response.text().unwrap()).await
                .map_err(|_| ApiError::new("Failed to read error response"))?
                .as_string()
                .unwrap_or_else(|| "Unknown error".to_string());
            
            return Err(ApiError::with_status(error_text, status));
        }

        let json_value = JsFuture::from(response.json().unwrap()).await
            .map_err(|_| ApiError::new("Failed to parse JSON response"))?;

        let json_string = js_sys::JSON::stringify(&json_value)
            .map_err(|_| ApiError::new("Failed to stringify JSON"))?
            .as_string()
            .ok_or_else(|| ApiError::new("Invalid JSON string"))?;

        serde_json::from_str(&json_string)
            .map_err(|e| ApiError::new(format!("Failed to deserialize response: {}", e)))
    }

    /// Retrieves audit statistics from the backend API.
    ///
    /// This method sends a GET request to the `/api/admin/audit/statistics` endpoint
    /// to retrieve comprehensive audit statistics including total counts, success rates,
    /// and breakdowns by action and target type.
    ///
    /// # Returns
    ///
    /// Returns an `AuditStatistics` struct containing comprehensive metrics, or an
    /// `ApiError` if the request fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let stats = client.get_statistics().await?;
    /// println!("Total entries: {}", stats.total_entries);
    /// println!("Success rate: {:.1}%", 
    ///     (stats.successful_entries as f64 / stats.total_entries as f64) * 100.0);
    /// ```
    pub async fn get_statistics(&self) -> Result<AuditStatistics, ApiError> {
        let url = format!("{}/api/admin/audit/statistics", self.base_url);

        let opts = RequestInit::new();
        opts.set_method("GET");
        opts.set_mode(RequestMode::Cors);

        let request = Request::new_with_str_and_init(&url, &opts)
            .map_err(|_| ApiError::new("Failed to create request"))?;

        // Add authorization header if token is available
        if let Some(ref token) = self.auth_token {
            request.headers().set("Authorization", &format!("Bearer {}", token))
                .map_err(|_| ApiError::new("Failed to set authorization header"))?;
        }

        let window = web_sys::window().ok_or_else(|| ApiError::new("No window object"))?;
        let response_value = JsFuture::from(window.fetch_with_request(&request)).await
            .map_err(|_| ApiError::new("Network request failed"))?;

        let response: Response = response_value.dyn_into()
            .map_err(|_| ApiError::new("Invalid response type"))?;

        if !response.ok() {
            let status = response.status();
            let error_text = JsFuture::from(response.text().unwrap()).await
                .map_err(|_| ApiError::new("Failed to read error response"))?
                .as_string()
                .unwrap_or_else(|| "Unknown error".to_string());
            
            return Err(ApiError::with_status(error_text, status));
        }

        let json_value = JsFuture::from(response.json().unwrap()).await
            .map_err(|_| ApiError::new("Failed to parse JSON response"))?;

        let json_string = js_sys::JSON::stringify(&json_value)
            .map_err(|_| ApiError::new("Failed to stringify JSON"))?
            .as_string()
            .ok_or_else(|| ApiError::new("Invalid JSON string"))?;

        serde_json::from_str(&json_string)
            .map_err(|e| ApiError::new(format!("Failed to deserialize response: {}", e)))
    }

    /// Exports audit logs from the backend API in JSON format.
    ///
    /// This method sends a POST request to the `/api/admin/audit/export` endpoint
    /// with the provided filter criteria. The backend will return the matching
    /// audit log entries as a JSON string suitable for download or further processing.
    ///
    /// # Parameters
    ///
    /// - `filter`: An `AuditLogFilter` specifying which entries to export
    ///
    /// # Returns
    ///
    /// Returns a JSON string containing the exported audit entries, or an `ApiError`
    /// if the request fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Export all failed operations from the last month
    /// let filter = AuditLogFilter {
    ///     success: Some(false),
    ///     start_time: Some(SystemTime::now() - Duration::from_secs(30 * 24 * 3600)),
    ///     ..Default::default()
    /// };
    /// let json_data = client.export_logs(filter).await?;
    /// // Save to file or process further
    /// ```
    pub async fn export_logs(&self, filter: AuditLogFilter) -> Result<String, ApiError> {
        let url = format!("{}/api/admin/audit/export", self.base_url);
        
        let request_body = serde_json::to_string(&filter)
            .map_err(|e| ApiError::new(format!("Failed to serialize filter: {}", e)))?;

        let opts = RequestInit::new();
        opts.set_method("POST");
        opts.set_mode(RequestMode::Cors);
        opts.set_body(&wasm_bindgen::JsValue::from_str(&request_body));

        let request = Request::new_with_str_and_init(&url, &opts)
            .map_err(|_| ApiError::new("Failed to create request"))?;

        // Add authorization header if token is available
        if let Some(ref token) = self.auth_token {
            request.headers().set("Authorization", &format!("Bearer {}", token))
                .map_err(|_| ApiError::new("Failed to set authorization header"))?;
        }

        request.headers().set("Content-Type", "application/json")
            .map_err(|_| ApiError::new("Failed to set content type header"))?;

        let window = web_sys::window().ok_or_else(|| ApiError::new("No window object"))?;
        let response_value = JsFuture::from(window.fetch_with_request(&request)).await
            .map_err(|_| ApiError::new("Network request failed"))?;

        let response: Response = response_value.dyn_into()
            .map_err(|_| ApiError::new("Invalid response type"))?;

        if !response.ok() {
            let status = response.status();
            let error_text = JsFuture::from(response.text().unwrap()).await
                .map_err(|_| ApiError::new("Failed to read error response"))?
                .as_string()
                .unwrap_or_else(|| "Unknown error".to_string());
            
            return Err(ApiError::with_status(error_text, status));
        }

        JsFuture::from(response.text().unwrap()).await
            .map_err(|_| ApiError::new("Failed to read response text"))?
            .as_string()
            .ok_or_else(|| ApiError::new("Invalid response text"))
    }

    /// Retrieves a specific audit log entry by its ID.
    ///
    /// This method sends a GET request to the `/api/admin/audit/logs/{id}` endpoint
    /// to retrieve a single audit log entry. Returns `None` if the entry is not found.
    ///
    /// # Parameters
    ///
    /// - `id`: The unique identifier of the audit log entry to retrieve
    ///
    /// # Returns
    ///
    /// Returns `Some(AuditLogEntry)` if found, `None` if not found (404), or an
    /// `ApiError` for other failures.
    ///
    /// # Examples
    ///
    /// ```rust
    /// match client.get_log_by_id(12345).await? {
    ///     Some(entry) => println!("Found entry: {}", entry.description()),
    ///     None => println!("Entry not found"),
    /// }
    /// ```
    pub async fn get_log_by_id(&self, id: i64) -> Result<Option<AuditLogEntry>, ApiError> {
        let url = format!("{}/api/admin/audit/logs/{}", self.base_url, id);

        let opts = RequestInit::new();
        opts.set_method("GET");
        opts.set_mode(RequestMode::Cors);

        let request = Request::new_with_str_and_init(&url, &opts)
            .map_err(|_| ApiError::new("Failed to create request"))?;

        // Add authorization header if token is available
        if let Some(ref token) = self.auth_token {
            request.headers().set("Authorization", &format!("Bearer {}", token))
                .map_err(|_| ApiError::new("Failed to set authorization header"))?;
        }

        let window = web_sys::window().ok_or_else(|| ApiError::new("No window object"))?;
        let response_value = JsFuture::from(window.fetch_with_request(&request)).await
            .map_err(|_| ApiError::new("Network request failed"))?;

        let response: Response = response_value.dyn_into()
            .map_err(|_| ApiError::new("Invalid response type"))?;

        if response.status() == 404 {
            return Ok(None);
        }

        if !response.ok() {
            let status = response.status();
            let error_text = JsFuture::from(response.text().unwrap()).await
                .map_err(|_| ApiError::new("Failed to read error response"))?
                .as_string()
                .unwrap_or_else(|| "Unknown error".to_string());
            
            return Err(ApiError::with_status(error_text, status));
        }

        let json_value = JsFuture::from(response.json().unwrap()).await
            .map_err(|_| ApiError::new("Failed to parse JSON response"))?;

        let json_string = js_sys::JSON::stringify(&json_value)
            .map_err(|_| ApiError::new("Failed to stringify JSON"))?
            .as_string()
            .ok_or_else(|| ApiError::new("Invalid JSON string"))?;

        let entry: AuditLogEntry = serde_json::from_str(&json_string)
            .map_err(|e| ApiError::new(format!("Failed to deserialize response: {}", e)))?;

        Ok(Some(entry))
    }
}

/// Global audit API client instance for application-wide use.
///
/// This static variable holds a single instance of `AuditApiClient` that can be
/// shared across the entire application. It's initialized once using `init_audit_api_client`
/// and then accessed through `get_audit_api_client`.
///
/// # Safety
///
/// This uses `unsafe` static mutable access, which is acceptable in single-threaded
/// WASM environments but should be used with caution. The `std::sync::Once` ensures
/// thread-safe initialization.
static mut GLOBAL_AUDIT_CLIENT: Option<AuditApiClient> = None;
static AUDIT_CLIENT_INIT: std::sync::Once = std::sync::Once::new();

/// Initializes the global audit API client instance.
///
/// This function should be called once at application startup to set up the
/// global audit API client with the appropriate base URL. Subsequent calls
/// will be ignored due to the `Once` guard.
///
/// # Parameters
///
/// - `base_url`: The base URL of the audit API server
///
/// # Examples
///
/// ```rust
/// // Initialize at application startup
/// init_audit_api_client("https://matrix.example.com".to_string());
/// ```
///
/// # Safety
///
/// This function is safe to call from single-threaded WASM environments.
/// The `Once` guard ensures it only executes once even if called multiple times.
pub fn init_audit_api_client(base_url: String) {
    unsafe {
        AUDIT_CLIENT_INIT.call_once(|| {
            GLOBAL_AUDIT_CLIENT = Some(AuditApiClient::new(base_url));
        });
    }
}

/// Retrieves a mutable reference to the global audit API client.
///
/// This function provides access to the global audit API client instance.
/// The client must be initialized with `init_audit_api_client` before calling
/// this function, or it will panic.
///
/// # Returns
///
/// Returns a mutable reference to the global `AuditApiClient` instance.
///
/// # Panics
///
/// Panics if the audit API client has not been initialized with `init_audit_api_client`.
///
/// # Examples
///
/// ```rust
/// let client = get_audit_api_client();
/// client.set_auth_token("new_token".to_string());
/// ```
///
/// # Safety
///
/// This function uses unsafe static mutable access, which is acceptable in
/// single-threaded WASM environments but should be used with caution.
pub fn get_audit_api_client() -> &'static mut AuditApiClient {
    unsafe {
        GLOBAL_AUDIT_CLIENT.as_mut().expect("Audit API client not initialized")
    }
}

/// Sets the authentication token for the global audit API client.
///
/// This is a convenience function that sets the authentication token on the
/// global client instance. The client must be initialized before calling this function.
///
/// # Parameters
///
/// - `token`: The authentication token to set
///
/// # Examples
///
/// ```rust
/// set_audit_api_auth_token("eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9...".to_string());
/// ```
pub fn set_audit_api_auth_token(token: String) {
    get_audit_api_client().set_auth_token(token);
}

/// Convenience function to query audit logs using the global client.
///
/// This function provides a simplified interface for querying audit logs without
/// needing to access the global client directly. The global client must be
/// initialized and configured with an authentication token before use.
///
/// # Parameters
///
/// - `filter`: An `AuditLogFilter` specifying the query criteria
///
/// # Returns
///
/// Returns an `AuditLogResponse` containing matching entries and pagination metadata.
///
/// # Examples
///
/// ```rust
/// let filter = AuditLogFilter {
///     success: Some(false),
///     limit: Some(10),
///     ..Default::default()
/// };
/// let response = query_audit_logs(filter).await?;
/// ```
pub async fn query_audit_logs(filter: AuditLogFilter) -> Result<AuditLogResponse, ApiError> {
    get_audit_api_client().query_logs(filter).await
}

/// Convenience function to get audit statistics using the global client.
///
/// This function provides a simplified interface for retrieving audit statistics
/// without needing to access the global client directly.
///
/// # Returns
///
/// Returns an `AuditStatistics` struct containing comprehensive audit metrics.
///
/// # Examples
///
/// ```rust
/// let stats = get_audit_statistics().await?;
/// println!("Total audit entries: {}", stats.total_entries);
/// ```
pub async fn get_audit_statistics() -> Result<AuditStatistics, ApiError> {
    get_audit_api_client().get_statistics().await
}

/// Convenience function to export audit logs using the global client.
///
/// This function provides a simplified interface for exporting audit logs
/// without needing to access the global client directly.
///
/// # Parameters
///
/// - `filter`: An `AuditLogFilter` specifying which entries to export
///
/// # Returns
///
/// Returns a JSON string containing the exported audit entries.
///
/// # Examples
///
/// ```rust
/// let filter = AuditLogFilter::default();
/// let json_export = export_audit_logs(filter).await?;
/// ```
pub async fn export_audit_logs(filter: AuditLogFilter) -> Result<String, ApiError> {
    get_audit_api_client().export_logs(filter).await
}

/// Convenience function to get an audit log entry by ID using the global client.
///
/// This function provides a simplified interface for retrieving a specific
/// audit log entry without needing to access the global client directly.
///
/// # Parameters
///
/// - `id`: The unique identifier of the audit log entry to retrieve
///
/// # Returns
///
/// Returns `Some(AuditLogEntry)` if found, `None` if not found.
///
/// # Examples
///
/// ```rust
/// match get_audit_log_by_id(12345).await? {
///     Some(entry) => println!("Found: {}", entry.description()),
///     None => println!("Entry not found"),
/// }
/// ```
pub async fn get_audit_log_by_id(id: i64) -> Result<Option<AuditLogEntry>, ApiError> {
    get_audit_api_client().get_log_by_id(id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_api_client_creation() {
        let client = AuditApiClient::new("http://localhost:8080".to_string());
        assert_eq!(client.base_url, "http://localhost:8080");
        assert!(client.auth_token.is_none());
    }

    #[test]
    fn test_audit_api_client_set_token() {
        let mut client = AuditApiClient::new("http://localhost:8080".to_string());
        client.set_auth_token("test_token".to_string());
        assert_eq!(client.auth_token, Some("test_token".to_string()));
    }
}