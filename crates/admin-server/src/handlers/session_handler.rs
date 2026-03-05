/// Session Handler - HTTP handlers for session and whois API
///
/// This module implements session-related API endpoints including:
/// - Whois queries (user session information)
/// - Session listing
/// - Session count
/// - Last seen information

use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};

use crate::types::AdminError;
use crate::repositories::{SessionRepository, SessionFilter, SessionInfo, WhoisInfo};

/// Session list query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Session list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionResponse>,
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Session response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub ip: String,
    pub last_seen: i64,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub user_agent: Option<String>,
}

/// Whois response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoisResponse {
    pub user_id: String,
    pub sessions: Vec<SessionResponse>,
    pub total_session_count: i64,
    pub primary_device_id: Option<String>,
}

/// Last seen response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastSeenResponse {
    pub user_id: String,
    pub last_seen_ts: Option<i64>,
    pub last_seen_ip: Option<String>,
}

/// Session count response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCountResponse {
    pub user_id: String,
    pub ip_count: i64,
}

/// Generic success response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Session handler configuration
pub struct SessionHandler<T: SessionRepository> {
    session_repo: T,
}

impl<T: SessionRepository> SessionHandler<T> {
    /// Create a new handler with the given repository
    pub fn new(session_repo: T) -> Self {
        Self { session_repo }
    }

    /// Get whois information for a user
    pub async fn get_whois(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let whois = self.session_repo.get_whois(&user_id).await?;

        let sessions: Vec<SessionResponse> = whois.sessions.iter().map(SessionResponse::from).collect();

        Ok(HttpResponse::Ok().json(WhoisResponse {
            user_id: whois.user_id,
            sessions,
            total_session_count: whois.total_session_count,
            primary_device_id: whois.primary_device_id,
        }))
    }

    /// List all sessions for a user
    pub async fn list_sessions(
        &self,
        user_id: web::Path<String>,
        query: web::Query<SessionListQuery>,
    ) -> Result<HttpResponse, AdminError> {
        let filter = SessionFilter {
            user_id: user_id.to_string(),
            limit: query.limit,
            offset: query.offset,
        };

        let result = self.session_repo.list_sessions(&filter).await?;

        let sessions: Vec<SessionResponse> = result.sessions.iter().map(SessionResponse::from).collect();

        Ok(HttpResponse::Ok().json(SessionListResponse {
            sessions,
            total_count: result.total_count,
            limit: result.limit,
            offset: result.offset,
        }))
    }

    /// Get all sessions for a user (without pagination)
    pub async fn get_user_sessions(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let sessions = self.session_repo.get_user_sessions(&user_id).await?;

        let response: Vec<SessionResponse> = sessions.iter().map(SessionResponse::from).collect();

        Ok(HttpResponse::Ok().json(response))
    }

    /// Get session count for a user
    pub async fn get_session_count(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let count = self.session_repo.get_user_ip_count(&user_id).await?;

        Ok(HttpResponse::Ok().json(SessionCountResponse {
            user_id: user_id.to_string(),
            ip_count: count,
        }))
    }

    /// Get last seen information for a user
    pub async fn get_last_seen(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let last_seen_ts = self.session_repo.get_last_seen(&user_id).await?;
        let last_seen_ip = self.session_repo.get_last_seen_ip(&user_id).await?;

        Ok(HttpResponse::Ok().json(LastSeenResponse {
            user_id: user_id.to_string(),
            last_seen_ts,
            last_seen_ip,
        }))
    }

    /// Record a session (for internal use)
    pub async fn record_session(
        &self,
        req: web::Json<RecordSessionRequest>,
    ) -> Result<HttpResponse, AdminError> {
        self.session_repo.record_session(
            &req.user_id,
            &req.ip,
            req.device_id.as_deref(),
            req.user_agent.as_deref(),
        ).await?;

        Ok(HttpResponse::Ok().json(SuccessResponse {
            success: true,
            message: "Session recorded successfully".to_string(),
        }))
    }

    /// Delete old sessions (cleanup)
    pub async fn delete_old_sessions(&self, before_ts: web::Path<i64>) -> Result<HttpResponse, AdminError> {
        let count = self.session_repo.delete_old_sessions(before_ts.into_inner()).await?;

        Ok(HttpResponse::Ok().json(BatchDeleteResponse {
            success: true,
            deleted_count: count,
            message: format!("Deleted {} old sessions", count),
        }))
    }

    /// Delete all sessions for a user
    pub async fn delete_user_sessions(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let count = self.session_repo.delete_user_sessions(&user_id).await?;

        tracing::info!("Deleted {} sessions for user {}", count, user_id);

        Ok(HttpResponse::Ok().json(BatchDeleteResponse {
            success: true,
            deleted_count: count,
            message: format!("Deleted {} sessions for user {}", count, user_id),
        }))
    }
}

/// Record session request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordSessionRequest {
    pub user_id: String,
    pub ip: String,
    pub device_id: Option<String>,
    pub user_agent: Option<String>,
}

/// Batch delete response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDeleteResponse {
    pub success: bool,
    pub deleted_count: u64,
    pub message: String,
}

// Conversion implementations
impl From<&SessionInfo> for SessionResponse {
    fn from(session: &SessionInfo) -> Self {
        SessionResponse {
            ip: session.ip.clone(),
            last_seen: session.last_seen,
            device_id: session.device_id.clone(),
            device_name: session.device_name.clone(),
            user_agent: session.user_agent.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::DieselSessionRepository;
    use palpo_data::DieselPool;

    #[tokio::test]
    #[ignore]
    async fn test_get_whois() {}

    #[tokio::test]
    #[ignore]
    async fn test_list_sessions() {}

    #[tokio::test]
    #[ignore]
    async fn test_get_last_seen() {}
}