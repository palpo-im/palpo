//! Federation management data models
//!
//! This module contains all data structures and types used for managing Matrix federation
//! in the Palpo admin interface. It provides models for federation destinations, connection
//! testing, support information, and various request/response types.
//!
//! # Key Components
//!
//! - [`FederationDestination`] - Represents a federated Matrix server
//! - [`DestinationInfo`] - Detailed information about a federation destination
//! - [`FederationTestResult`] - Results from federation connectivity tests
//! - [`SupportInfo`] - Server support contact information
//!
//! # Usage
//!
//! These models are primarily used by the [`FederationAdminAPI`](crate::services::FederationAdminAPI)
//! to manage federation connections and provide administrative functionality.

use serde::{Deserialize, Serialize};
use crate::models::room::SortOrder;

/// Federation destination information
///
/// Represents a Matrix server that this homeserver federates with.
/// Contains connection status, failure tracking, and shared room information.
///
/// # Examples
///
/// ```rust
/// use palpo_admin_ui::models::FederationDestination;
///
/// let destination = FederationDestination {
///     server_name: "matrix.org".to_string(),
///     is_reachable: true,
///     last_successful_send: Some(1640995200),
///     failure_count: 0,
///     shared_rooms: vec!["!room:example.com".to_string()],
///     is_disabled: false,
///     last_failure_reason: None,
///     avg_response_time: Some(150),
/// };
///
/// assert!(destination.is_healthy());
/// assert_eq!(destination.status_description(), "Healthy");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FederationDestination {
    /// Server name (e.g., "matrix.org")
    pub server_name: String,
    /// Whether the server is currently reachable
    pub is_reachable: bool,
    /// Last successful send timestamp
    pub last_successful_send: Option<u64>,
    /// Number of consecutive failures
    pub failure_count: u32,
    /// List of shared rooms with this server
    pub shared_rooms: Vec<String>,
    /// Whether federation is disabled for this destination
    pub is_disabled: bool,
    /// Last failure reason if any
    pub last_failure_reason: Option<String>,
    /// Average response time in milliseconds
    pub avg_response_time: Option<u32>,
}

/// Detailed destination information
///
/// Extended information about a federation destination including server capabilities,
/// connection statistics, and recent federation events.
///
/// This is typically returned by [`FederationAdminAPI::get_destination_info`](crate::services::FederationAdminAPI::get_destination_info)
/// and provides comprehensive details for administrative monitoring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DestinationInfo {
    /// Basic destination information
    pub destination: FederationDestination,
    /// Server version information
    pub server_version: Option<String>,
    /// Supported Matrix versions
    pub supported_versions: Vec<String>,
    /// Server features
    pub features: Vec<String>,
    /// Connection statistics
    pub connection_stats: ConnectionStats,
    /// Recent events sent/received
    pub recent_events: Vec<FederationEvent>,
}

/// Connection statistics for a federation destination
///
/// Tracks various metrics about the federation connection including
/// event counts, data transfer, and uptime statistics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectionStats {
    /// Total events sent
    pub events_sent: u64,
    /// Total events received
    pub events_received: u64,
    /// Total bytes sent
    pub bytes_sent: u64,
    /// Total bytes received
    pub bytes_received: u64,
    /// Connection uptime percentage
    pub uptime_percentage: f32,
    /// Last connection attempt timestamp
    pub last_connection_attempt: Option<u64>,
}

/// Federation event information
///
/// Represents a Matrix event that was sent or received via federation.
/// Used for tracking recent federation activity and debugging connectivity issues.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FederationEvent {
    /// Event ID
    pub event_id: String,
    /// Room ID
    pub room_id: String,
    /// Event type
    pub event_type: String,
    /// Sender
    pub sender: String,
    /// Timestamp
    pub timestamp: u64,
    /// Direction (sent/received)
    pub direction: EventDirection,
    /// Processing status
    pub status: EventStatus,
}

/// Event direction
///
/// Indicates whether a federation event was sent to or received from a remote server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EventDirection {
    Sent,
    Received,
}

/// Event processing status
///
/// Tracks the current state of federation event processing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EventStatus {
    Success,
    Failed,
    Pending,
    Retrying,
}

/// Federation test result
///
/// Contains the results of testing federation connectivity to a remote server.
/// Includes overall success status, timing information, server details, and
/// individual test results for different aspects of the federation connection.
///
/// # Test Components
///
/// Federation tests typically include:
/// - DNS resolution
/// - TLS handshake
/// - Matrix version discovery
/// - Server key verification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FederationTestResult {
    /// Whether the test was successful
    pub success: bool,
    /// Test duration in milliseconds
    pub duration_ms: u32,
    /// Server response information
    pub server_info: Option<ServerInfo>,
    /// Error message if test failed
    pub error: Option<String>,
    /// Test details
    pub test_details: Vec<TestDetail>,
}

/// Server information from federation test
///
/// Basic information about a remote Matrix server discovered during federation testing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerInfo {
    /// Server name
    pub server_name: String,
    /// Server version
    pub version: Option<String>,
    /// Supported Matrix versions
    pub supported_versions: Vec<String>,
    /// Server features
    pub features: Vec<String>,
}

/// Individual test detail
///
/// Represents the result of a single component of the federation test suite.
/// Each test has a name, success status, duration, and optional error message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestDetail {
    /// Test name
    pub test_name: String,
    /// Test result
    pub success: bool,
    /// Test duration in milliseconds
    pub duration_ms: u32,
    /// Error message if test failed
    pub error: Option<String>,
}

/// Support information from well-known endpoint
///
/// Contact and support information for a Matrix server, typically retrieved
/// from the `/.well-known/matrix/support` endpoint as defined in MSC1929.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SupportInfo {
    /// Server name
    pub server_name: String,
    /// Support contacts
    pub contacts: Vec<SupportContact>,
    /// Support room
    pub support_room: Option<String>,
    /// Support page URL
    pub support_page: Option<String>,
    /// Additional information
    pub additional_info: Option<String>,
}

/// Support contact information
///
/// Represents a single contact method for server support (email, Matrix ID, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SupportContact {
    /// Contact type (email, matrix_id, etc.)
    pub contact_type: String,
    /// Contact value
    pub contact_value: String,
    /// Contact role
    pub role: Option<String>,
}

/// Request to list federation destinations
///
/// Supports filtering, sorting, and pagination of federation destinations.
///
/// # Examples
///
/// ```rust
/// use palpo_admin_ui::models::ListDestinationsRequest;
///
/// // List all reachable destinations
/// let request = ListDestinationsRequest {
///     filter_reachable: Some(true),
///     limit: Some(20),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListDestinationsRequest {
    /// Search filter
    pub search: Option<String>,
    /// Filter by reachability status
    pub filter_reachable: Option<bool>,
    /// Filter by disabled status
    pub filter_disabled: Option<bool>,
    /// Sort field
    pub sort_by: Option<DestinationSortField>,
    /// Sort order
    pub sort_order: Option<SortOrder>,
    /// Pagination offset
    pub offset: Option<u32>,
    /// Pagination limit
    pub limit: Option<u32>,
}

/// Fields to sort destinations by
///
/// Defines the available sorting options for federation destination lists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DestinationSortField {
    ServerName,
    LastSuccessfulSend,
    FailureCount,
    SharedRooms,
    ResponseTime,
}



/// Response for listing destinations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDestinationsResponse {
    /// Whether the operation was successful
    pub success: bool,
    /// List of destinations
    pub destinations: Vec<FederationDestination>,
    /// Total count of destinations
    pub total_count: u32,
    /// Whether there are more results
    pub has_more: bool,
    /// Error message if operation failed
    pub error: Option<String>,
}

/// Request to get destination information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDestinationInfoRequest {
    /// Server name
    pub server_name: String,
}

/// Response for getting destination information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDestinationInfoResponse {
    /// Whether the operation was successful
    pub success: bool,
    /// Destination information
    pub destination_info: Option<DestinationInfo>,
    /// Error message if operation failed
    pub error: Option<String>,
}

/// Request to disable/enable destination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToggleDestinationRequest {
    /// Server name
    pub server_name: String,
    /// Reason for the action
    pub reason: Option<String>,
}

/// Response for destination toggle operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToggleDestinationResponse {
    /// Whether the operation was successful
    pub success: bool,
    /// Error message if operation failed
    pub error: Option<String>,
}

/// Request to test federation with a server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFederationRequest {
    /// Server name to test
    pub server_name: String,
    /// Test timeout in seconds
    pub timeout_seconds: Option<u32>,
}

/// Response for federation test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFederationResponse {
    /// Whether the operation was successful
    pub success: bool,
    /// Test result
    pub test_result: Option<FederationTestResult>,
    /// Error message if operation failed
    pub error: Option<String>,
}

/// Request to fetch support information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchSupportInfoRequest {
    /// Server name
    pub server_name: String,
}

/// Response for fetching support information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchSupportInfoResponse {
    /// Whether the operation was successful
    pub success: bool,
    /// Support information
    pub support_info: Option<SupportInfo>,
    /// Error message if operation failed
    pub error: Option<String>,
}

impl FederationDestination {
    /// Get display name for the destination
    ///
    /// Returns the server name as the display name for UI purposes.
    pub fn display_name(&self) -> &str {
        &self.server_name
    }

    /// Check if the destination is healthy
    ///
    /// A destination is considered healthy if it is:
    /// - Reachable (can be contacted)
    /// - Has fewer than 5 consecutive failures
    /// - Is not administratively disabled
    ///
    /// # Returns
    ///
    /// `true` if the destination is healthy, `false` otherwise
    pub fn is_healthy(&self) -> bool {
        self.is_reachable && self.failure_count < 5 && !self.is_disabled
    }

    /// Get status description
    ///
    /// Returns a human-readable status description based on the destination's current state.
    ///
    /// # Returns
    ///
    /// - "Disabled" if administratively disabled
    /// - "Unreachable" if connection attempts are failing
    /// - "Degraded" if reachable but has some failures
    /// - "Healthy" if fully operational
    pub fn status_description(&self) -> &'static str {
        if self.is_disabled {
            "Disabled"
        } else if !self.is_reachable {
            "Unreachable"
        } else if self.failure_count > 0 {
            "Degraded"
        } else {
            "Healthy"
        }
    }
}

impl Default for ConnectionStats {
    fn default() -> Self {
        Self {
            events_sent: 0,
            events_received: 0,
            bytes_sent: 0,
            bytes_received: 0,
            uptime_percentage: 0.0,
            last_connection_attempt: None,
        }
    }
}

impl TestDetail {
    /// Create a successful test detail
    ///
    /// Convenience constructor for creating a test detail representing a successful test.
    ///
    /// # Arguments
    ///
    /// * `test_name` - Name of the test that was performed
    /// * `duration_ms` - How long the test took to complete in milliseconds
    ///
    /// # Returns
    ///
    /// A `TestDetail` with success status and no error message
    pub fn success(test_name: String, duration_ms: u32) -> Self {
        Self {
            test_name,
            success: true,
            duration_ms,
            error: None,
        }
    }

    /// Create a failed test detail
    ///
    /// Convenience constructor for creating a test detail representing a failed test.
    ///
    /// # Arguments
    ///
    /// * `test_name` - Name of the test that was performed
    /// * `duration_ms` - How long the test took before failing in milliseconds
    /// * `error` - Description of what went wrong
    ///
    /// # Returns
    ///
    /// A `TestDetail` with failure status and error message
    pub fn failure(test_name: String, duration_ms: u32, error: String) -> Self {
        Self {
            test_name,
            success: false,
            duration_ms,
            error: Some(error),
        }
    }
}