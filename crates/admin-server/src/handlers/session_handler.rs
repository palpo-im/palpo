/// Session Handler - HTTP handlers for session management API
///
/// This module implements session management API endpoints using Salvo framework.
/// All endpoints require authentication via Bearer token.
/// Uses PalpoClient to communicate with Palpo Matrix server via HTTP API.
///
/// Endpoints:
/// - GET /api/v1/users/{user_id}/whois - Get user connection info
/// - GET /api/v1/users/{user_id}/sessions - List user sessions
/// - GET /api/v1/users/{user_id}/sessions/count - Get session count
/// - GET /api/v1/users/{user_id}/last-seen - Get last seen info
/// - DELETE /api/v1/users/{user_id}/sessions - Delete user sessions

use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

use crate::palpo_client::{PalpoClient, PalpoWhoisResponse};

use super::auth_middleware::require_auth;
use super::validation::validate_user_id;

// ===== Request Types =====

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionResponse>,
    pub total_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub device_id: String,
    pub user_id: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub last_seen: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoisResponse {
    pub user_id: String,
    pub devices: Vec<DeviceWhoisInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceWhoisInfo {
    pub device_id: String,
    pub sessions: Vec<SessionWhoisInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWhoisInfo {
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub last_seen: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastSeenResponse {
    pub user_id: String,
    pub last_seen_ts: Option<i64>,
    pub last_seen_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCountResponse {
    pub user_id: String,
    pub session_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDeleteResponse {
    pub success: bool,
    pub deleted_count: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ===== Handler State =====

#[derive(Clone, Debug)]
pub struct SessionHandlerState {
    pub palpo_client: Arc<PalpoClient>,
}

impl SessionHandlerState {
    pub fn new(palpo_client: Arc<PalpoClient>) -> Self {
        Self { palpo_client }
    }
}

static SESSION_HANDLER_STATE: OnceLock<SessionHandlerState> = OnceLock::new();

pub fn init_session_handler_state(state: SessionHandlerState) {
    SESSION_HANDLER_STATE.set(state).expect("Session handler state already initialized");
}

fn get_session_handler_state() -> &'static SessionHandlerState {
    SESSION_HANDLER_STATE.get().expect("Session handler state not initialized")
}

// ===== Handler Functions =====

/// GET /api/v1/users/{user_id}/whois - Get user connection info
#[handler]
pub async fn get_whois(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_whois(&user_id).await {
        Ok(whois) => {
            let devices: Vec<DeviceWhoisInfo> = whois.devices.iter().map(|d| {
                DeviceWhoisInfo {
                    device_id: d.device_id.clone(),
                    sessions: d.sessions.iter().map(|s| {
                        SessionWhoisInfo {
                            ip_address: s.connections.first().map(|c| c.ip.clone()),
                            user_agent: s.connections.first().and_then(|c| c.user_agent.clone()),
                            last_seen: s.connections.first().and_then(|c| c.last_seen),
                        }
                    }).collect(),
                }
            }).collect();

            res.render(Json(WhoisResponse {
                user_id: whois.user_id,
                devices,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get whois: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get whois".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/sessions - List user sessions
#[handler]
pub async fn list_sessions(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // Get whois info which contains session details
    match state.palpo_client.get_whois(&user_id).await {
        Ok(whois) => {
            let mut sessions = Vec::new();
            for device in &whois.devices {
                for session in &device.sessions {
                    if let Some(conn) = session.connections.first() {
                        sessions.push(SessionResponse {
                            device_id: device.device_id.clone(),
                            user_id: user_id.clone(),
                            ip_address: Some(conn.ip.clone()),
                            user_agent: conn.user_agent.clone(),
                            last_seen: conn.last_seen,
                        });
                    }
                }
            }

            res.render(Json(SessionListResponse {
                total_count: sessions.len() as i64,
                sessions,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to list sessions: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to list sessions".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/sessions/count - Get session count
#[handler]
pub async fn get_session_count(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_whois(&user_id).await {
        Ok(whois) => {
            let count = whois.devices.iter()
                .map(|d| d.sessions.len())
                .sum::<usize>() as i64;

            res.render(Json(SessionCountResponse {
                user_id,
                session_count: count,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get session count: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get session count".to_string() }));
        }
    }
}

/// GET /api/v1/users/{user_id}/last-seen - Get last seen info
#[handler]
pub async fn get_last_seen(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    match state.palpo_client.get_whois(&user_id).await {
        Ok(whois) => {
            // Find the most recent last_seen
            let mut last_seen_ts: Option<i64> = None;
            let mut last_seen_ip: Option<String> = None;

            for device in &whois.devices {
                for session in &device.sessions {
                    for conn in &session.connections {
                        if let Some(ts) = conn.last_seen {
                            if last_seen_ts.map_or(true, |old| ts > old) {
                                last_seen_ts = Some(ts);
                                last_seen_ip = Some(conn.ip.clone());
                            }
                        }
                    }
                }
            }

            res.render(Json(LastSeenResponse {
                user_id,
                last_seen_ts,
                last_seen_ip,
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get last seen: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get last seen".to_string() }));
        }
    }
}

/// DELETE /api/v1/users/{user_id}/sessions - Delete user sessions
/// NOTE: This deletes all devices/sessions for the user
#[handler]
pub async fn delete_user_sessions(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !require_auth(depot, res) { return; }

    let state = get_session_handler_state();
    let user_id = req.param::<String>("user_id").unwrap_or_default();

    if let Err(e) = validate_user_id(&user_id) {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(ErrorResponse { error: format!("Invalid user_id: {}", e) }));
        return;
    }

    // Get all devices and delete them
    match state.palpo_client.list_user_devices(&user_id).await {
        Ok(result) => {
            if result.devices.is_empty() {
                res.render(Json(SuccessResponse {
                    success: true,
                    message: "No sessions to delete".to_string(),
                }));
                return;
            }

            let device_ids: Vec<String> = result.devices.iter()
                .map(|d| d.device_id.clone())
                .collect();

            match state.palpo_client.delete_user_devices(&user_id, &device_ids).await {
                Ok(_) => {
                    tracing::info!("Deleted {} sessions for user {}", device_ids.len(), user_id);
                    res.render(Json(BatchDeleteResponse {
                        success: true,
                        deleted_count: device_ids.len() as u64,
                        message: format!("Deleted {} sessions", device_ids.len()),
                    }));
                }
                Err(e) => {
                    tracing::error!("Failed to delete sessions: {}", e);
                    res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                    res.render(Json(ErrorResponse { error: "Failed to delete sessions".to_string() }));
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to get user devices: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Json(ErrorResponse { error: "Failed to get sessions".to_string() }));
        }
    }
}