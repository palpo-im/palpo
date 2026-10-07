use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use lru_cache::LruCache;
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

/// Upper bound on cached introspection results. Without a bound the cache grew
/// without limit, since rotated/expired tokens were only ever evicted when the
/// exact same token hash was looked up again. The LRU bound keeps memory capped
/// regardless of how many distinct tokens are seen.
const CACHE_CAPACITY: usize = 100_000;

static CACHE: LazyLock<Mutex<LruCache<[u8; 32], CachedEntry>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(CACHE_CAPACITY)));

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
            let hit = match cache.get_mut(&key) {
                Some(entry) if entry.cached_at.elapsed() < Duration::from_secs(ttl) => {
                    Some(entry.result.clone())
                }
                Some(_) => None, // expired
                None => None,
            };
            match hit {
                Some(result) => return Ok(result),
                None => {
                    cache.remove(&key);
                }
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

/// Validated, space-delimited OAuth scopes (RFC 6749 section 3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthScopes(std::collections::BTreeSet<String>);

impl OAuthScopes {
    pub fn parse(scope: &str) -> Option<Self> {
        let mut scopes = std::collections::BTreeSet::new();
        for part in scope.split(' ') {
            if part.is_empty()
                || !part
                    .bytes()
                    .all(|b| matches!(b, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
            {
                return None;
            }
            scopes.insert(part.to_owned());
        }
        Some(Self(scopes))
    }

    pub fn contains(&self, scope: &str) -> bool {
        self.0.contains(scope)
    }

    pub fn has_matrix_api_scope(&self) -> bool {
        self.contains("urn:matrix:client:api:*")
            || self.contains("urn:matrix:org.matrix.msc2967.client:api:*")
    }

    /// Require one unambiguous device, allowing stable and unstable aliases.
    pub fn device_id(&self) -> Option<&str> {
        let mut device_id = None;
        for part in &self.0 {
            if let Some(id) = part
                .strip_prefix("urn:matrix:client:device:")
                .or_else(|| part.strip_prefix("urn:matrix:org.matrix.msc2967.client:device:"))
            {
                if id.is_empty() || device_id.is_some_and(|previous| previous != id) {
                    return None;
                }
                device_id = Some(id);
            }
        }
        device_id
    }
}

pub fn device_id_from_scope(scope: &str) -> Option<String> {
    OAuthScopes::parse(scope)?
        .device_id()
        .map(ToOwned::to_owned)
}

pub fn has_matrix_api_scope(scope: &str) -> bool {
    OAuthScopes::parse(scope).is_some_and(|scopes| scopes.has_matrix_api_scope())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_scope_syntax_is_strict_and_matches_exact_tokens() {
        for invalid in [
            "",
            " openid",
            "openid ",
            "openid  email",
            "openid\temail",
            "openid\nemail",
            "é",
            "a\\b",
            "a\"b",
            "a\0b",
        ] {
            assert!(OAuthScopes::parse(invalid).is_none(), "{invalid:?}");
            assert!(!has_matrix_api_scope(invalid));
        }
        let scopes = OAuthScopes::parse("openid openid ! # [ ] ~").unwrap();
        assert!(scopes.contains("openid"));
        assert!(!scopes.contains("open"));
        assert!(!has_matrix_api_scope("urn:matrix:client:api:*:extra"));
        assert_eq!(device_id_from_scope("urn:matrix:client:device:"), None);
        assert_eq!(
            device_id_from_scope("urn:matrix:client:device:DEV\topenid"),
            None
        );
    }

    #[test]
    fn matrix_scope_requires_api_access_and_unambiguous_device() {
        assert!(!has_matrix_api_scope("openid email"));
        assert!(has_matrix_api_scope("openid urn:matrix:client:api:*"));
        assert!(has_matrix_api_scope(
            "urn:matrix:org.matrix.msc2967.client:api:*"
        ));
        assert_eq!(
            device_id_from_scope(
                "urn:matrix:client:device:DEV urn:matrix:org.matrix.msc2967.client:device:DEV"
            ),
            Some("DEV".to_owned())
        );
        assert_eq!(
            device_id_from_scope("urn:matrix:client:device:DEV urn:matrix:client:device:OTHER"),
            None
        );
    }
}
