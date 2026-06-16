use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use lru_cache::LruCache;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;
use crate::user::{DbUser, DbUserDevice};
use crate::{DataResult, connect};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_access_tokens)]
pub struct DbAccessToken {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub token: String,
    pub puppets_user_id: Option<OwnedUserId>,
    pub last_validated: Option<UnixMillis>,
    pub refresh_token_id: Option<i64>,
    pub is_used: bool,
    pub expires_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_access_tokens)]
pub struct NewDbAccessToken {
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub token: String,
    pub puppets_user_id: Option<OwnedUserId>,
    pub last_validated: Option<UnixMillis>,
    pub refresh_token_id: Option<i64>,
    pub is_used: bool,
    pub expires_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
}

impl NewDbAccessToken {
    pub fn new(
        user_id: OwnedUserId,
        device_id: OwnedDeviceId,
        token: String,
        refresh_token_id: Option<i64>,
    ) -> Self {
        Self {
            user_id,
            device_id,
            token,
            puppets_user_id: None,
            last_validated: None,
            refresh_token_id,
            is_used: false,
            expires_at: None,
            created_at: UnixMillis::now(),
        }
    }
}

/// A native access token resolved to the user, device and token id it
/// authenticates. Returned by [`authenticate_token`].
#[derive(Debug, Clone)]
pub struct TokenAuth {
    pub user: DbUser,
    pub device: DbUserDevice,
    pub access_token_id: i64,
}

struct CachedAuth {
    auth: TokenAuth,
    cached_at: Instant,
}

/// Maximum number of token authentications kept in memory.
const CACHE_CAPACITY: usize = 100_000;
/// How long a cached authentication may be served before it is re-validated
/// against the database. Explicit invalidation (see [`invalidate_user`]) covers
/// logout and account-state changes immediately; this short TTL is only a
/// backstop bounding staleness for anything not explicitly invalidated (e.g. a
/// token orphaned by refresh-token rotation).
const CACHE_TTL: Duration = Duration::from_secs(60);

static CACHE: LazyLock<Mutex<LruCache<String, CachedAuth>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(CACHE_CAPACITY)));

fn cache_get(token: &str) -> Option<TokenAuth> {
    let mut cache = CACHE.lock().ok()?;
    let fresh = match cache.get_mut(token) {
        Some(entry) if entry.cached_at.elapsed() < CACHE_TTL => Some(entry.auth.clone()),
        Some(_) => None, // expired
        None => return None,
    };
    if fresh.is_none() {
        cache.remove(token);
    }
    fresh
}

fn cache_put(token: &str, auth: TokenAuth) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(
            token.to_owned(),
            CachedAuth {
                auth,
                cached_at: Instant::now(),
            },
        );
    }
}

/// Drop every cached authentication that belongs to `user_id`.
///
/// Must be called whenever a user's access tokens are revoked or any
/// account-usability state changes, so that a logged-out / deactivated /
/// locked / suspended user can never keep authenticating from the cache.
pub fn invalidate_user(user_id: &UserId) {
    if let Ok(mut cache) = CACHE.lock() {
        let stale: Vec<String> = cache
            .iter()
            .filter(|(_, v)| v.auth.user.id.as_str() == user_id.as_str())
            .map(|(k, _)| k.clone())
            .collect();
        for token in stale {
            cache.remove(&token);
        }
    }
}

/// Resolve a native access token to its user, device and token id.
///
/// Uses an in-memory cache to keep the authentication hot path off the
/// database (a cache hit avoids three round-trips per request). Returns
/// `Ok(None)` when `token` is not a known user access token, so the caller can
/// fall through to other schemes (e.g. appservice tokens).
pub async fn authenticate_token(token: &str) -> DataResult<Option<TokenAuth>> {
    if let Some(hit) = cache_get(token) {
        return Ok(Some(hit));
    }

    let access_token = match user_access_tokens::table
        .filter(user_access_tokens::token.eq(token))
        .first::<DbAccessToken>(&mut connect().await?)
        .await
    {
        Ok(access_token) => access_token,
        Err(diesel::result::Error::NotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let user = match users::table
        .find(&access_token.user_id)
        .first::<DbUser>(&mut connect().await?)
        .await
    {
        Ok(user) => user,
        Err(diesel::result::Error::NotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let device = match user_devices::table
        .filter(user_devices::device_id.eq(&access_token.device_id))
        .filter(user_devices::user_id.eq(&user.id))
        .first::<DbUserDevice>(&mut connect().await?)
        .await
    {
        Ok(device) => device,
        Err(diesel::result::Error::NotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let auth = TokenAuth {
        user,
        device,
        access_token_id: access_token.id,
    };
    cache_put(token, auth.clone());
    Ok(Some(auth))
}
