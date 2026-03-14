/// Palpo HTTP Admin API Client
///
/// This module provides a client for calling Palpo's `/_synapse/admin/` HTTP API.
/// admin-server does NOT directly connect to Palpo's database.
/// All Matrix data operations go through this client.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error};

use crate::types::AdminError;

// ===== Palpo API Response Types =====

/// User info from `/_synapse/admin/v2/users/{user_id}`
/// Field names match Palpo's API exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoUser {
    /// Full Matrix user ID, e.g. "@alice:example.com"
    pub name: String,
    #[serde(default)]
    pub displayname: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    pub admin: bool,
    #[serde(default)]
    pub deactivated: bool,
    #[serde(default)]
    pub shadow_banned: bool,
    /// Unix timestamp in milliseconds
    #[serde(default)]
    pub creation_ts: Option<i64>,
    #[serde(default)]
    pub threepids: Vec<PalpoThreepid>,
    #[serde(default)]
    pub external_ids: Vec<PalpoExternalId>,
    #[serde(default)]
    pub user_type: Option<String>,
    #[serde(default)]
    pub is_guest: Option<bool>,
    #[serde(default)]
    pub appservice_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoThreepid {
    pub medium: String,
    pub address: String,
    #[serde(default)]
    pub added_at: Option<i64>,
    #[serde(default)]
    pub validated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoExternalId {
    pub auth_provider: String,
    pub external_id: String,
}

/// Response from `GET /_synapse/admin/v2/users`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoUserListResponse {
    pub users: Vec<PalpoUser>,
    pub total: i64,
    #[serde(default)]
    pub next_token: Option<String>,
}

/// Device info from `/_synapse/admin/v2/users/{user_id}/devices`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoDevice {
    pub device_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub last_seen_ts: Option<i64>,
    #[serde(default)]
    pub last_seen_ip: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
}

/// Response from `GET /_synapse/admin/v2/users/{user_id}/devices`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoDeviceListResponse {
    pub devices: Vec<PalpoDevice>,
    pub total: i64,
}

/// Room info from `/_synapse/admin/v1/rooms`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoRoom {
    pub room_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub canonical_alias: Option<String>,
    #[serde(default)]
    pub joined_members: Option<i64>,
    #[serde(default)]
    pub joined_local_members: Option<i64>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(default)]
    pub encryption: Option<String>,
    #[serde(default)]
    pub federatable: Option<bool>,
    #[serde(default)]
    pub public: Option<bool>,
    #[serde(default)]
    pub join_rules: Option<String>,
    #[serde(default)]
    pub guest_access: Option<String>,
    #[serde(default)]
    pub history_visibility: Option<String>,
    #[serde(default)]
    pub state_events: Option<i64>,
}

/// Response from `GET /_synapse/admin/v1/rooms`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoRoomListResponse {
    pub rooms: Vec<PalpoRoom>,
    pub total_rooms: i64,
    #[serde(default)]
    pub next_batch: Option<String>,
    #[serde(default)]
    pub prev_batch: Option<String>,
}

/// Room members response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoRoomMembersResponse {
    pub members: Vec<String>,
    pub total: i64,
}

/// Server version info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalpoServerVersion {
    pub server_version: String,
    pub python_version: Option<String>,
}

// ===== Request Types =====

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateOrUpdateUserRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub displayname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deactivated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeactivateUserRequest {
    pub erase: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetPasswordRequest {
    pub new_password: String,
    #[serde(default)]
    pub logout_devices: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListUsersQuery {
    pub from: Option<i64>,
    pub limit: Option<i64>,
    pub search_term: Option<String>,
    pub guests: Option<bool>,
    pub deactivated: Option<bool>,
    pub admins: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListRoomsQuery {
    pub from: Option<i64>,
    pub limit: Option<i64>,
    pub search_term: Option<String>,
    pub order_by: Option<String>,
    pub dir: Option<String>,
}

// ===== PalpoClient =====

/// Client for Palpo's `/_synapse/admin/` HTTP API.
///
/// Holds the Matrix admin access_token and handles token refresh.
/// All methods are async and return `Result<T, AdminError>`.
#[derive(Debug, Clone)]
pub struct PalpoClient {
    inner: Arc<PalpoClientInner>,
}

#[derive(Debug)]
struct PalpoClientInner {
    base_url: String,
    admin_username: String,
    admin_password: String,
    http: Client,
    /// Current Matrix admin access_token (refreshed on 401)
    access_token: RwLock<Option<String>>,
}

impl PalpoClient {
    /// Create a new PalpoClient.
    ///
    /// `base_url` - Palpo server URL, e.g. "http://localhost:8008"
    /// `admin_username` - Matrix admin username (localpart only, e.g. "admin")
    /// `admin_password` - Matrix admin password
    pub fn new(base_url: String, admin_username: String, admin_password: String) -> Self {
        Self {
            inner: Arc::new(PalpoClientInner {
                base_url,
                admin_username,
                admin_password,
                http: Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .expect("Failed to build HTTP client"),
                access_token: RwLock::new(None),
            }),
        }
    }

    /// Login to Palpo and obtain an access_token.
    pub async fn login(&self) -> Result<(), AdminError> {
        let url = format!("{}/_matrix/client/v3/login", self.inner.base_url);
        let body = serde_json::json!({
            "type": "m.login.password",
            "user": self.inner.admin_username,
            "password": self.inner.admin_password
        });

        let resp = self.inner.http.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AdminError::MatrixApiError(format!(
                "Login failed ({}): {}", status, text
            )));
        }

        #[derive(Deserialize)]
        struct LoginResp {
            access_token: String,
        }
        let login_resp: LoginResp = resp.json().await.map_err(|e| {
            AdminError::MatrixApiError(format!("Failed to parse login response: {}", e))
        })?;

        let mut token = self.inner.access_token.write().await;
        *token = Some(login_resp.access_token);
        info!("PalpoClient: logged in as {}", self.inner.admin_username);
        Ok(())
    }

    /// Get the current access token, logging in if needed.
    async fn get_token(&self) -> Result<String, AdminError> {
        {
            let token = self.inner.access_token.read().await;
            if let Some(t) = token.as_ref() {
                return Ok(t.clone());
            }
        }
        self.login().await?;
        let token = self.inner.access_token.read().await;
        token.clone().ok_or_else(|| AdminError::MatrixApiError("No access token after login".to_string()))
    }

    /// Make an authenticated GET request, retrying once on 401.
    async fn get(&self, path: &str) -> Result<reqwest::Response, AdminError> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.inner.base_url, path);
        let resp = self.inner.http.get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            warn!("PalpoClient: 401 on GET {}, re-logging in", path);
            self.login().await?;
            let new_token = self.get_token().await?;
            let resp2 = self.inner.http.get(&url)
                .header("Authorization", format!("Bearer {}", new_token))
                .send()
                .await?;
            return Ok(resp2);
        }
        Ok(resp)
    }

    /// Make an authenticated PUT request.
    async fn put<B: Serialize>(&self, path: &str, body: &B) -> Result<reqwest::Response, AdminError> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.inner.base_url, path);
        let resp = self.inner.http.put(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(body)
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.login().await?;
            let new_token = self.get_token().await?;
            let resp2 = self.inner.http.put(&url)
                .header("Authorization", format!("Bearer {}", new_token))
                .json(body)
                .send()
                .await?;
            return Ok(resp2);
        }
        Ok(resp)
    }

    /// Make an authenticated POST request.
    async fn post<B: Serialize>(&self, path: &str, body: &B) -> Result<reqwest::Response, AdminError> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.inner.base_url, path);
        let resp = self.inner.http.post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(body)
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.login().await?;
            let new_token = self.get_token().await?;
            let resp2 = self.inner.http.post(&url)
                .header("Authorization", format!("Bearer {}", new_token))
                .json(body)
                .send()
                .await?;
            return Ok(resp2);
        }
        Ok(resp)
    }

    /// Make an authenticated DELETE request.
    async fn delete<B: Serialize>(&self, path: &str, body: Option<&B>) -> Result<reqwest::Response, AdminError> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.inner.base_url, path);
        let mut req = self.inner.http.delete(&url)
            .header("Authorization", format!("Bearer {}", token));
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.login().await?;
            let new_token = self.get_token().await?;
            let mut req2 = self.inner.http.delete(&url)
                .header("Authorization", format!("Bearer {}", new_token));
            if let Some(b) = body {
                req2 = req2.json(b);
            }
            let resp2 = req2.send().await?;
            return Ok(resp2);
        }
        Ok(resp)
    }

    /// Check API response and return error if not successful.
    async fn check_response(resp: reqwest::Response, context: &str) -> Result<reqwest::Response, AdminError> {
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(AdminError::MatrixApiError(format!("{} failed ({}): {}", context, status, text)))
    }

    // ===== User Management =====

    /// `GET /_synapse/admin/v2/users`
    pub async fn list_users(&self, query: &ListUsersQuery) -> Result<PalpoUserListResponse, AdminError> {
        let mut params = vec![];
        if let Some(from) = query.from { params.push(format!("from={}", from)); }
        if let Some(limit) = query.limit { params.push(format!("limit={}", limit)); }
        if let Some(ref s) = query.search_term { params.push(format!("search_term={}", urlencoding::encode(s))); }
        if let Some(guests) = query.guests { params.push(format!("guests={}", guests)); }
        if let Some(deactivated) = query.deactivated { params.push(format!("deactivated={}", deactivated)); }
        if let Some(admins) = query.admins { params.push(format!("admins={}", admins)); }

        let path = if params.is_empty() {
            "/_synapse/admin/v2/users".to_string()
        } else {
            format!("/_synapse/admin/v2/users?{}", params.join("&"))
        };

        let resp = self.get(&path).await?;
        let resp = Self::check_response(resp, "list_users").await?;
        resp.json().await.map_err(|e| AdminError::MatrixApiError(format!("Failed to parse users list: {}", e)))
    }

    /// `GET /_synapse/admin/v2/users/{user_id}`
    pub async fn get_user(&self, user_id: &str) -> Result<PalpoUser, AdminError> {
        let encoded = urlencoding::encode(user_id);
        let path = format!("/_synapse/admin/v2/users/{}", encoded);
        let resp = self.get(&path).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AdminError::MatrixApiError(format!("User not found: {}", user_id)));
        }
        let resp = Self::check_response(resp, "get_user").await?;
        resp.json().await.map_err(|e| AdminError::MatrixApiError(format!("Failed to parse user: {}", e)))
    }

    /// `PUT /_synapse/admin/v2/users/{user_id}` — create or update
    pub async fn create_or_update_user(&self, user_id: &str, req: &CreateOrUpdateUserRequest) -> Result<PalpoUser, AdminError> {
        let encoded = urlencoding::encode(user_id);
        let path = format!("/_synapse/admin/v2/users/{}", encoded);
        let resp = self.put(&path, req).await?;
        let resp = Self::check_response(resp, "create_or_update_user").await?;
        resp.json().await.map_err(|e| AdminError::MatrixApiError(format!("Failed to parse user: {}", e)))
    }

    /// `POST /_synapse/admin/v1/deactivate/{user_id}`
    pub async fn deactivate_user(&self, user_id: &str, erase: bool) -> Result<(), AdminError> {
        let encoded = urlencoding::encode(user_id);
        let path = format!("/_synapse/admin/v1/deactivate/{}", encoded);
        let body = DeactivateUserRequest { erase };
        let resp = self.post(&path, &body).await?;
        Self::check_response(resp, "deactivate_user").await?;
        Ok(())
    }

    /// `POST /_synapse/admin/v1/reset_password/{user_id}`
    pub async fn reset_password(&self, user_id: &str, new_password: &str, logout_devices: bool) -> Result<(), AdminError> {
        let encoded = urlencoding::encode(user_id);
        let path = format!("/_synapse/admin/v1/reset_password/{}", encoded);
        let body = ResetPasswordRequest { new_password: new_password.to_string(), logout_devices };
        let resp = self.post(&path, &body).await?;
        Self::check_response(resp, "reset_password").await?;
        Ok(())
    }

    /// `GET /_synapse/admin/v2/users/{user_id}/devices`
    pub async fn list_user_devices(&self, user_id: &str) -> Result<PalpoDeviceListResponse, AdminError> {
        let encoded = urlencoding::encode(user_id);
        let path = format!("/_synapse/admin/v2/users/{}/devices", encoded);
        let resp = self.get(&path).await?;
        let resp = Self::check_response(resp, "list_user_devices").await?;
        resp.json().await.map_err(|e| AdminError::MatrixApiError(format!("Failed to parse devices: {}", e)))
    }

    /// `DELETE /_synapse/admin/v2/users/{user_id}/devices/{device_id}`
    pub async fn delete_user_device(&self, user_id: &str, device_id: &str) -> Result<(), AdminError> {
        let encoded_user = urlencoding::encode(user_id);
        let encoded_device = urlencoding::encode(device_id);
        let path = format!("/_synapse/admin/v2/users/{}/devices/{}", encoded_user, encoded_device);
        let resp = self.delete::<serde_json::Value>(&path, None).await?;
        Self::check_response(resp, "delete_user_device").await?;
        Ok(())
    }

    /// `POST /_synapse/admin/v2/users/{user_id}/delete_devices` — bulk delete
    pub async fn delete_user_devices(&self, user_id: &str, device_ids: &[String]) -> Result<(), AdminError> {
        let encoded = urlencoding::encode(user_id);
        let path = format!("/_synapse/admin/v2/users/{}/delete_devices", encoded);
        let body = serde_json::json!({ "devices": device_ids });
        let resp = self.post(&path, &body).await?;
        Self::check_response(resp, "delete_user_devices").await?;
        Ok(())
    }

    // ===== Room Management =====

    /// `GET /_synapse/admin/v1/rooms`
    pub async fn list_rooms(&self, query: &ListRoomsQuery) -> Result<PalpoRoomListResponse, AdminError> {
        let mut params = vec![];
        if let Some(from) = query.from { params.push(format!("from={}", from)); }
        if let Some(limit) = query.limit { params.push(format!("limit={}", limit)); }
        if let Some(ref s) = query.search_term { params.push(format!("search_term={}", urlencoding::encode(s))); }
        if let Some(ref o) = query.order_by { params.push(format!("order_by={}", o)); }
        if let Some(ref d) = query.dir { params.push(format!("dir={}", d)); }

        let path = if params.is_empty() {
            "/_synapse/admin/v1/rooms".to_string()
        } else {
            format!("/_synapse/admin/v1/rooms?{}", params.join("&"))
        };

        let resp = self.get(&path).await?;
        let resp = Self::check_response(resp, "list_rooms").await?;
        resp.json().await.map_err(|e| AdminError::MatrixApiError(format!("Failed to parse rooms: {}", e)))
    }

    /// `GET /_synapse/admin/v1/rooms/{room_id}`
    pub async fn get_room(&self, room_id: &str) -> Result<PalpoRoom, AdminError> {
        let encoded = urlencoding::encode(room_id);
        let path = format!("/_synapse/admin/v1/rooms/{}", encoded);
        let resp = self.get(&path).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AdminError::MatrixApiError(format!("Room not found: {}", room_id)));
        }
        let resp = Self::check_response(resp, "get_room").await?;
        resp.json().await.map_err(|e| AdminError::MatrixApiError(format!("Failed to parse room: {}", e)))
    }

    /// `DELETE /_synapse/admin/v1/rooms/{room_id}`
    pub async fn delete_room(&self, room_id: &str, block: bool, purge: bool) -> Result<(), AdminError> {
        let encoded = urlencoding::encode(room_id);
        let path = format!("/_synapse/admin/v1/rooms/{}", encoded);
        let body = serde_json::json!({ "block": block, "purge": purge });
        let resp = self.delete(&path, Some(&body)).await?;
        Self::check_response(resp, "delete_room").await?;
        Ok(())
    }

    /// `GET /_synapse/admin/v1/rooms/{room_id}/members`
    pub async fn list_room_members(&self, room_id: &str) -> Result<PalpoRoomMembersResponse, AdminError> {
        let encoded = urlencoding::encode(room_id);
        let path = format!("/_synapse/admin/v1/rooms/{}/members", encoded);
        let resp = self.get(&path).await?;
        let resp = Self::check_response(resp, "list_room_members").await?;
        resp.json().await.map_err(|e| AdminError::MatrixApiError(format!("Failed to parse room members: {}", e)))
    }

    // ===== Server Stats =====

    /// `GET /_synapse/admin/v1/server_version`
    pub async fn get_server_version(&self) -> Result<PalpoServerVersion, AdminError> {
        let resp = self.get("/_synapse/admin/v1/server_version").await?;
        let resp = Self::check_response(resp, "get_server_version").await?;
        resp.json().await.map_err(|e| AdminError::MatrixApiError(format!("Failed to parse server version: {}", e)))
    }
}
