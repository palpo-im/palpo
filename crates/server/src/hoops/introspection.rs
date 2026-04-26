use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{AppResult, MatrixError, config, sending};

/// Subset of RFC 7662 introspection response.
#[derive(Debug, Clone, Deserialize)]
pub struct IntrospectionResult {
    pub active: bool,
    pub scope: Option<String>,
    pub username: Option<String>,
    pub sub: Option<String>,
    pub device_id: Option<String>,
    /// RFC 7519 `aud` — string or array of strings. Validated by the
    /// caller against `delegated_auth.expected_aud` to prevent
    /// cross-resource-server token reuse.
    pub aud: Option<serde_json::Value>,
}

struct CachedEntry {
    result: IntrospectionResult,
    cached_at: Instant,
}

static CACHE: LazyLock<Mutex<HashMap<[u8; 32], CachedEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn token_cache_key(token: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().into()
}

pub async fn introspect_token(token: &str) -> AppResult<IntrospectionResult> {
    let conf = config::get();
    let ttl = conf
        .delegated_auth
        .as_ref()
        .map(|da| da.introspection_cache_ttl)
        .unwrap_or(300);

    // Check cache
    if ttl > 0 {
        let key = token_cache_key(token);
        if let Ok(mut cache) = CACHE.lock()
            && let Some(entry) = cache.get(&key)
        {
            if entry.cached_at.elapsed() < Duration::from_secs(ttl) {
                return Ok(entry.result.clone());
            }
            cache.remove(&key);
        }
    }

    // Call introspection endpoint
    let introspection_url = conf
        .introspection_endpoint()
        .ok_or_else(|| MatrixError::unknown("Delegated auth not configured"))?;
    let da = conf
        .enabled_delegated_auth()
        .ok_or_else(|| MatrixError::unknown("Delegated auth not enabled"))?;

    let client = sending::default_client();
    // Three-tier client authentication for the introspection endpoint:
    //  1. (client_id + client_secret) — RFC 7662 §2.1 / RFC 6749 §2.3.1
    //     `client_secret_basic`. The right thing for a confidential client.
    //  2. (client_id only) — RFC 7009 §2.1 public client: send client_id
    //     in the form body, no Authorization header.
    //  3. (neither) — legacy fallback to `Authorization: Bearer
    //     <admin.mas_secret>`. Only safe when the upstream accepts the
    //     homeserver-admin shared bearer at this endpoint.
    let mut request = client.post(&introspection_url);
    let mut form_params: Vec<(&str, &str)> = vec![("token", token)];
    match (da.client_id.as_deref(), da.client_secret.as_deref()) {
        (Some(id), Some(secret)) => {
            request = request.basic_auth(id, Some(secret));
        }
        (Some(id), None) => {
            form_params.push(("client_id", id));
        }
        _ => {
            let mas_secret = conf
                .admin
                .mas_secret
                .as_ref()
                .ok_or_else(|| MatrixError::unknown("admin.mas_secret not configured"))?;
            request = request.bearer_auth(mas_secret);
        }
    }
    let response = request
        .form(&form_params)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Introspection request failed: {e}");
            MatrixError::unknown("Authentication service unavailable")
        })?;

    if !response.status().is_success() {
        tracing::error!("Introspection returned status: {}", response.status());
        return Err(MatrixError::unknown("Authentication service error").into());
    }

    let result: IntrospectionResult = response.json().await.map_err(|e| {
        tracing::error!("Failed to parse introspection response: {e}");
        MatrixError::unknown("Invalid introspection response")
    })?;

    // Cache the result
    if ttl > 0 {
        let key = token_cache_key(token);
        if let Ok(mut cache) = CACHE.lock() {
            cache.insert(
                key,
                CachedEntry {
                    result: result.clone(),
                    cached_at: Instant::now(),
                },
            );
        }
    }

    Ok(result)
}

/// Extract device_id from OAuth scope string.
/// Looks for `urn:matrix:client:device:<id>` or the unstable variant.
pub fn device_id_from_scope(scope: &str) -> Option<String> {
    for part in scope.split_whitespace() {
        if let Some(id) = part.strip_prefix("urn:matrix:client:device:")
            && !id.is_empty()
        {
            return Some(id.to_owned());
        }
        if let Some(id) = part.strip_prefix("urn:matrix:org.matrix.msc2967.client:device:")
            && !id.is_empty()
        {
            return Some(id.to_owned());
        }
    }
    None
}
