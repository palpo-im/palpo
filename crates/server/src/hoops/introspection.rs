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
        if let Ok(mut cache) = CACHE.lock() {
            if let Some(entry) = cache.get(&key) {
                if entry.cached_at.elapsed() < Duration::from_secs(ttl) {
                    return Ok(entry.result.clone());
                }
                cache.remove(&key);
            }
        }
    }

    // Call introspection endpoint
    let introspection_url = conf
        .introspection_endpoint()
        .ok_or_else(|| MatrixError::unknown("Delegated auth not configured"))?;
    let mas_secret = conf
        .admin
        .mas_secret
        .as_ref()
        .ok_or_else(|| MatrixError::unknown("admin.mas_secret not configured"))?;

    let client = sending::default_client();
    let response = client
        .post(&introspection_url)
        .bearer_auth(mas_secret)
        .form(&[("token", token)])
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
        if let Some(id) = part.strip_prefix("urn:matrix:client:device:") {
            if !id.is_empty() {
                return Some(id.to_owned());
            }
        }
        if let Some(id) = part.strip_prefix("urn:matrix:org.matrix.msc2967.client:device:") {
            if !id.is_empty() {
                return Some(id.to_owned());
            }
        }
    }
    None
}
