use std::time::Duration;

use palpo_hagency_contract::{MatrixUserId, ServerName};
use reqwest::{Client, Method, Url};
use serde::Deserialize;
use serde_json::Value;

use crate::{Result, fail};

#[derive(Clone)]
pub struct Matrix {
    client: Client,
    origin: Url,
    server: ServerName,
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub user: MatrixUserId,
    pub admin: bool,
}

impl Matrix {
    pub fn new(origin: &str, server: ServerName) -> Result<Self> {
        let url = Url::parse(origin).map_err(|_| fail(400, "invalid_matrix_origin"))?;
        if !(url.scheme() == "https"
            || url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(fail(400, "invalid_matrix_origin"));
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| fail(503, "matrix_client_unavailable"))?;
        Ok(Self {
            client,
            origin: url,
            server,
        })
    }
    pub fn origin(&self) -> String {
        self.origin.origin().ascii_serialization()
    }
    pub fn server(&self) -> &ServerName {
        &self.server
    }

    pub async fn call(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let url = self
            .origin
            .join(path)
            .map_err(|_| fail(400, "invalid_matrix_path"))?;
        if url.origin() != self.origin.origin() || !path.starts_with('/') || path.starts_with("//")
        {
            return Err(fail(400, "invalid_matrix_path"));
        }
        self.send(method, url, token, body).await
    }

    pub async fn room_state(&self, room: &str, token: &str, user: &str) -> Result<Value> {
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .map_err(|_| fail(400, "invalid_matrix_path"))?
            .clear()
            .extend(["_matrix", "client", "v3", "rooms", room, "state"]);
        url.query_pairs_mut().append_pair("user_id", user);
        self.send(Method::GET, url, token, None).await
    }

    /// Build paths from encoded segments; Matrix IDs are never URL fragments or queries.
    pub async fn segments(
        &self,
        method: Method,
        path: &[&str],
        token: &str,
        user: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value> {
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .map_err(|_| fail(400, "invalid_matrix_path"))?
            .clear()
            .extend(path);
        if let Some(user) = user {
            url.query_pairs_mut().append_pair("user_id", user);
        }
        self.send(method, url, token, body).await
    }

    pub(crate) async fn messages(
        &self,
        room: &str,
        token: &str,
        from: Option<&str>,
        limit: u16,
    ) -> Result<Value> {
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .map_err(|_| fail(400, "invalid_matrix_path"))?
            .clear()
            .extend(["_matrix", "client", "v3", "rooms", room, "messages"]);
        url.query_pairs_mut()
            .append_pair("dir", "b")
            .append_pair("limit", &limit.to_string());
        if let Some(from) = from {
            url.query_pairs_mut().append_pair("from", from);
        }
        self.send(Method::GET, url, token, None).await
    }

    async fn send(
        &self,
        method: Method,
        url: Url,
        token: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let (status, value) = self.send_status(method, url, token, body).await?;
        if !(200..300).contains(&status) {
            return Err(fail(
                match status {
                    401 | 403 | 404 => status,
                    _ => 502,
                },
                "matrix_request_failed",
            ));
        }
        Ok(value)
    }

    /// Registration alone needs the bounded UIAA challenge from an HTTP 401.
    /// The endpoint is fixed; neither an applicant nor a challenge can redirect it.
    pub(crate) async fn register(&self, body: &Value) -> Result<(u16, Value)> {
        let url = self
            .origin
            .join("/_matrix/client/v3/register")
            .map_err(|_| fail(400, "invalid_matrix_path"))?;
        self.send_status(Method::POST, url, "", Some(body)).await
    }

    pub(crate) async fn forward_messages(
        &self,
        room: &str,
        token: &str,
        from: Option<&str>,
    ) -> Result<Value> {
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .map_err(|_| fail(400, "invalid_matrix_path"))?
            .clear()
            .extend(["_matrix", "client", "v3", "rooms", room, "messages"]);
        url.query_pairs_mut()
            .append_pair("dir", "f")
            .append_pair("limit", "100");
        if let Some(from) = from {
            url.query_pairs_mut().append_pair("from", from);
        }
        self.send(Method::GET, url, token, None).await
    }

    async fn send_status(
        &self,
        method: Method,
        url: Url,
        token: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Value)> {
        let mut request = self.client.request(method, url).bearer_auth(token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| fail(502, "palpo_unreachable"))?;
        let status = response.status();
        let mut raw = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| fail(502, "palpo_unreachable"))?
        {
            if raw.len() + chunk.len() > 1024 * 1024 {
                return Err(fail(502, "matrix_response_too_large"));
            }
            raw.extend_from_slice(&chunk);
        }
        // Ordinary authorization/404 responses need not contain JSON.
        let value = match serde_json::from_slice(&raw) {
            Ok(value) => value,
            Err(_) if !status.is_success() => Value::Null,
            Err(_) => return Err(fail(502, "matrix_response_invalid")),
        };
        Ok((status.as_u16(), value))
    }

    pub async fn authenticate(&self, token: &str) -> Result<Identity> {
        #[derive(Deserialize)]
        struct Who {
            user_id: MatrixUserId,
            #[serde(default)]
            is_guest: bool,
        }
        let raw = self
            .call(
                Method::GET,
                "/_matrix/client/v3/account/whoami",
                token,
                None,
            )
            .await?;
        let who: Who =
            serde_json::from_value(raw).map_err(|_| fail(502, "matrix_identity_invalid"))?;
        if who.is_guest || !who.user_id.belongs_to(&self.server) {
            return Err(fail(403, "local_account_required"));
        }
        let admin = match self
            .call(Method::GET, "/_palpo/admin/v1/appservices", token, None)
            .await
        {
            Ok(_) => true,
            Err(e) if e.status == 403 => false,
            Err(e) => return Err(e),
        };
        Ok(Identity {
            user: who.user_id,
            admin,
        })
    }
}
