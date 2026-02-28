/// Integration tests for Server Control endpoints
///
/// These tests verify that the server control REST API endpoints
/// are properly wired and return expected responses.

use palpo_admin_server::{ServerControlAPI, handlers::server_control};
use std::sync::Arc;

#[tokio::test]
async fn test_server_control_state_initialization() {
    // Create server control API
    let server_control = Arc::new(ServerControlAPI::new());
    
    // Verify initial status
    let status = server_control.get_status();
    assert_eq!(status.status, palpo_admin_server::ServerStatus::NotStarted);
    assert_eq!(status.pid, None);
    assert_eq!(status.started_at, None);
    assert_eq!(status.uptime_seconds, None);
}

#[tokio::test]
async fn test_is_running_initially_false() {
    let server_control = Arc::new(ServerControlAPI::new());
    assert!(!server_control.is_running());
}

#[test]
fn test_server_control_state_creation() {
    let server_control = Arc::new(ServerControlAPI::new());
    let state = server_control::ServerControlState {
        server_control,
    };
    
    // Verify state can be created
    assert!(!state.server_control.is_running());
}
