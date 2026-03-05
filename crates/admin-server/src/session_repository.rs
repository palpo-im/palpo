/// Session Repository - Database operations for session and IP tracking
///
/// This module provides the data access layer for session management operations.
/// It implements the SessionRepository trait with direct PostgreSQL operations
/// using Diesel ORM.
///
/// Features:
/// - Session (IP) tracking for users
/// - Whois query support
/// - Session history with device association

use diesel::prelude::*;
use chrono::Utc;

use crate::types::AdminError;
use palpo_data::DieselPool;

/// User IP session record
#[derive(Debug, Clone, Queryable, Insertable, AsChangeset, Serialize, Deserialize)]
#[diesel(table_name = user_ips)]
pub struct UserIp {
    pub user_id: String,
    pub ip: String,
    pub last_seen_ts: i64,
    pub device_id: Option<String>,
    pub user_agent: Option<String>,
}

/// Session with device info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub ip: String,
    pub last_seen: i64,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub user_agent: Option<String>,
}

/// Whois information for a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoisInfo {
    pub user_id: String,
    pub sessions: Vec<SessionInfo>,
    pub total_session_count: i64,
    pub primary_device_id: Option<String>,
}

/// Session filter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFilter {
    pub user_id: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Session list result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListResult {
    pub sessions: Vec<SessionInfo>,
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Repository trait for session data access operations
#[async_trait::async_trait]
pub trait SessionRepository {
    /// Record a user session (IP login)
    async fn record_session(&self, user_id: &str, ip: &str, device_id: Option<&str>, user_agent: Option<&str>) -> Result<(), AdminError>;

    /// Get whois information for a user
    async fn get_whois(&self, user_id: &str) -> Result<WhoisInfo, AdminError>;

    /// Get all sessions for a user
    async fn get_user_sessions(&self, user_id: &str) -> Result<Vec<SessionInfo>, AdminError>;

    /// Get sessions with pagination
    async fn list_sessions(&self, filter: &SessionFilter) -> Result<SessionListResult, AdminError>;

    /// Get unique IP count for a user
    async fn get_user_ip_count(&self, user_id: &str) -> Result<i64, AdminError>;

    /// Get last seen timestamp for a user
    async fn get_last_seen(&self, user_id: &str) -> Result<Option<i64>, AdminError>;

    /// Get last seen IP for a user
    async fn get_last_seen_ip(&self, user_id: &str) -> Result<Option<String>, AdminError>;

    /// Delete old sessions (cleanup)
    async fn delete_old_sessions(&self, before_ts: i64) -> Result<u64, AdminError>;

    /// Delete all sessions for a user
    async fn delete_user_sessions(&self, user_id: &str) -> Result<u64, AdminError>;
}

/// Diesel-based SessionRepository implementation
pub struct DieselSessionRepository {
    db_pool: DieselPool,
}

impl DieselSessionRepository {
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }
}

#[async_trait::async_trait]
impl SessionRepository for DieselSessionRepository {
    async fn record_session(&self, user_id: &str, ip: &str, device_id: Option<&str>, user_agent: Option<&str>) -> Result<(), AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let now = Utc::now().timestamp_millis();

        // Upsert session record
        diesel::insert_into(user_ips::table)
            .values((
                user_ips::user_id.eq(user_id),
                user_ips::ip.eq(ip),
                user_ips::last_seen_ts.eq(now),
                user_ips::device_id.eq(device_id),
                user_ips::user_agent.eq(user_agent),
            ))
            .on_conflict((user_ips::user_id, user_ips::ip, user_ips::last_seen_ts))
            .do_update()
            .set((
                user_ips::last_seen_ts.eq(now),
                user_ips::device_id.eq(device_id),
                user_ips::user_agent.eq(user_agent),
            ))
            .execute(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(())
    }

    async fn get_whois(&self, user_id: &str) -> Result<WhoisInfo, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        // Get all sessions grouped by IP
        let sessions: Vec<SessionInfo> = user_ips::table
            .filter(user_ips::user_id.eq(user_id))
            .order_by(user_ips::last_seen_ts.desc())
            .load::<UserIp>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?
            .into_iter()
            .map(|ip_record| {
                // Get device name if device_id exists
                let device_name = ip_record.device_id.as_ref().and_then(|device_id| {
                    devices::table
                        .filter(devices::device_id.eq(device_id))
                        .filter(devices::user_id.eq(user_id))
                        .select(devices::display_name)
                        .first::<Option<String>>(&mut conn)
                        .ok()
                        .flatten()
                });

                SessionInfo {
                    ip: ip_record.ip,
                    last_seen: ip_record.last_seen_ts,
                    device_id: ip_record.device_id,
                    device_name,
                    user_agent: ip_record.user_agent,
                }
            })
            .collect();

        // Get total unique IP count
        let total_session_count = diesel::select(diesel::dsl::count_distinct(user_ips::ip))
            .filter(user_ips::user_id.eq(user_id))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        // Get primary device (most recently used)
        let primary_device = devices::table
            .filter(devices::user_id.eq(user_id))
            .order_by(devices::last_seen_ts.desc())
            .first::<Device>(&mut conn)
            .optional()
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(WhoisInfo {
            user_id: user_id.to_string(),
            sessions,
            total_session_count,
            primary_device_id: primary_device.map(|d| d.device_id),
        })
    }

    async fn get_user_sessions(&self, user_id: &str) -> Result<Vec<SessionInfo>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let sessions: Vec<SessionInfo> = user_ips::table
            .filter(user_ips::user_id.eq(user_id))
            .order_by(user_ips::last_seen_ts.desc())
            .load::<UserIp>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?
            .into_iter()
            .map(|ip_record| {
                let device_name = ip_record.device_id.as_ref().and_then(|device_id| {
                    devices::table
                        .filter(devices::device_id.eq(device_id))
                        .filter(devices::user_id.eq(user_id))
                        .select(devices::display_name)
                        .first::<Option<String>>(&mut conn)
                        .ok()
                        .flatten()
                });

                SessionInfo {
                    ip: ip_record.ip,
                    last_seen: ip_record.last_seen_ts,
                    device_id: ip_record.device_id,
                    device_name,
                    user_agent: ip_record.user_agent,
                }
            })
            .collect();

        Ok(sessions)
    }

    async fn list_sessions(&self, filter: &SessionFilter) -> Result<SessionListResult, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let limit = filter.limit.unwrap_or(50).min(100);
        let offset = filter.offset.unwrap_or(0);

        let total_count = diesel::select(diesel::dsl::count_distinct(user_ips::ip))
            .filter(user_ips::user_id.eq(&filter.user_id))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        let sessions: Vec<SessionInfo> = user_ips::table
            .filter(user_ips::user_id.eq(&filter.user_id))
            .order_by(user_ips::last_seen_ts.desc())
            .limit(limit)
            .offset(offset)
            .load::<UserIp>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?
            .into_iter()
            .map(|ip_record| {
                let device_name = ip_record.device_id.as_ref().and_then(|device_id| {
                    devices::table
                        .filter(devices::device_id.eq(device_id))
                        .filter(devices::user_id.eq(&filter.user_id))
                        .select(devices::display_name)
                        .first::<Option<String>>(&mut conn)
                        .ok()
                        .flatten()
                });

                SessionInfo {
                    ip: ip_record.ip,
                    last_seen: ip_record.last_seen_ts,
                    device_id: ip_record.device_id,
                    device_name,
                    user_agent: ip_record.user_agent,
                }
            })
            .collect();

        Ok(SessionListResult {
            sessions,
            total_count,
            limit,
            offset,
        })
    }

    async fn get_user_ip_count(&self, user_id: &str) -> Result<i64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = diesel::select(diesel::dsl::count_distinct(user_ips::ip))
            .filter(user_ips::user_id.eq(user_id))
            .get_result::<i64>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count)
    }

    async fn get_last_seen(&self, user_id: &str) -> Result<Option<i64>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let last_seen = diesel::select(diesel::dsl::max(user_ips::last_seen_ts))
            .filter(user_ips::user_id.eq(user_id))
            .get_result::<Option<i64>>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(last_seen)
    }

    async fn get_last_seen_ip(&self, user_id: &str) -> Result<Option<String>, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let last_seen_ip = diesel::select(diesel::dsl::max(user_ips::last_seen_ts))
            .filter(user_ips::user_id.eq(user_id))
            .get_result::<Option<i64>>(&mut conn)
            .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        if let Some(ts) = last_seen_ip {
            let ip = user_ips::table
                .filter(user_ips::user_id.eq(user_id))
                .filter(user_ips::last_seen_ts.eq(ts))
                .select(user_ips::ip)
                .first::<String>(&mut conn)
                .optional()
                .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;
            Ok(ip)
        } else {
            Ok(None)
        }
    }

    async fn delete_old_sessions(&self, before_ts: i64) -> Result<u64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = diesel::delete(
            user_ips::table.filter(user_ips::last_seen_ts.lt(before_ts))
        )
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count as u64)
    }

    async fn delete_user_sessions(&self, user_id: &str) -> Result<u64, AdminError> {
        let mut conn = self
            .db_pool
            .get()
            .map_err(|e| AdminError::DatabaseConnectionFailed(e.to_string()))?;

        let count = diesel::delete(
            user_ips::table.filter(user_ips::user_id.eq(user_id))
        )
        .execute(&mut conn)
        .map_err(|e| AdminError::DatabaseQueryFailed(e.to_string()))?;

        Ok(count as u64)
    }
}

// Helper struct for device query
#[derive(Queryable)]
struct Device {
    pub device_id: String,
    pub user_id: String,
    pub display_name: Option<String>,
    pub last_seen_ts: Option<i64>,
}

// Table definitions
use crate::schema::*;
use crate::schema::user_ips;
use crate::schema::devices;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_record_session() {}

    #[tokio::test]
    #[ignore]
    async fn test_get_whois() {}

    #[tokio::test]
    #[ignore]
    async fn test_get_user_sessions() {}
}