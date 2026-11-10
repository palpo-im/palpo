//! Event-signature key selection. HTTP request signatures deliberately use the
//! separate, time-independent lookup in the parent module.
use super::*;
use crate::core::MatrixError;

const MAX_KEY_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1000;

pub(super) fn event_timestamp(event: &CanonicalJsonObject) -> AppResult<UnixMillis> {
    let timestamp = event
        .get("origin_server_ts")
        .and_then(|v| v.as_integer())
        .and_then(|n| u64::try_from(n).ok())
        .ok_or_else(|| {
            MatrixError::invalid_param("origin_server_ts must be a non-negative integer")
        })?;
    Ok(UnixMillis(timestamp))
}

pub(super) fn bound_cached_keys(keys: &mut ServerSigningKeys, fetched_at: UnixMillis) {
    let ceiling = UnixMillis(fetched_at.0.saturating_add(MAX_KEY_LIFETIME_MS));
    keys.valid_until_ts = keys.valid_until_ts.min(ceiling);
    for key in keys.old_verify_keys.values_mut() {
        key.expired_ts = key.expired_ts.min(ceiling);
    }
}

fn select_key(
    keys: &ServerSigningKeys,
    id: &ServerSigningKeyId,
    timestamp: UnixMillis,
    enforce: bool,
) -> Option<VerifyKey> {
    // Explicit retirement wins even if an older cache document listed the same
    // key as current. Old keys remain useful for events before retirement.
    if let Some(key) = keys.old_verify_keys.get(id) {
        return (!enforce || timestamp < key.expired_ts).then(|| VerifyKey::new(key.key.clone()));
    }
    if !enforce || timestamp <= keys.valid_until_ts {
        return keys.verify_keys.get(id).cloned();
    }
    None
}

async fn cached_event_key(
    origin: &ServerName,
    id: &ServerSigningKeyId,
    timestamp: UnixMillis,
    enforce: bool,
) -> AppResult<Option<VerifyKey>> {
    let retired_at = trust::retirement(origin, id).await?;
    if enforce && retired_at.is_some_and(|expiry| timestamp >= expiry) {
        return Ok(None);
    }
    let mut documents = trust::event_documents(origin, id, timestamp, enforce).await?;
    for (keys, fetched) in &mut documents {
        bound_cached_keys(keys, *fetched);
    }
    // An authenticated retirement applies to every historical copy, including
    // a later replay of an earlier document which still called the key current.
    Ok(documents
        .iter()
        .find_map(|(keys, _)| select_key(keys, id, timestamp, enforce)))
}

pub(super) async fn event_verify_key(
    origin: &ServerName,
    id: &ServerSigningKeyId,
    timestamp: UnixMillis,
    enforce: bool,
) -> AppResult<VerifyKey> {
    let now = UnixMillis::now();
    if enforce && timestamp.0 > now.0.saturating_add(MAX_KEY_LIFETIME_MS) {
        return Err(MatrixError::invalid_param(
            "event timestamp exceeds signing key validity horizon",
        )
        .into());
    }
    let local = !origin.is_remote();
    if local {
        let current = config::keypair();
        if id.as_str() == format!("ed25519:{}", current.version()) {
            return Ok(VerifyKey::new(Base64::new(current.public_key().to_vec())));
        }
    }
    if enforce
        && trust::retirement(origin, id)
            .await?
            .is_some_and(|expiry| timestamp >= expiry)
    {
        return Err(AppError::public(
            "signing key was retired before the event timestamp",
        ));
    }
    if let Some(key) = cached_event_key(origin, id, timestamp, enforce).await? {
        return Ok(key);
    }

    // Only the actively configured local key has an implicit current lifetime.
    // Retired local keys must pass the same cache filter as remote keys, and
    // unknown/expired local keys must never cause a request to ourselves.
    if local {
        return Err(AppError::public(
            "no local signing key valid at the event timestamp",
        ));
    }

    let conf = config::get();
    let minimum = timestamp.max(now);
    // An expired cache entry is a miss: always try refresh before rejecting the
    // event. Never fall back to an expired key after a network/notary failure.
    if conf.query_trusted_key_servers_first
        && let Some(key) = refresh_notaries(origin, id, timestamp, enforce, minimum).await?
    {
        return Ok(key);
    }
    let requested = std::collections::BTreeSet::from([id.to_owned()]);
    if !conf.only_query_trusted_key_servers
        && let Ok(keys) = super::request::server_request_for(origin, Some(&requested)).await
    {
        add_signing_keys(keys.clone()).await?;
        if let Some(key) = cached_event_key(origin, id, timestamp, enforce).await? {
            return Ok(key);
        }
    }
    if !conf.query_trusted_key_servers_first
        && let Some(key) = refresh_notaries(origin, id, timestamp, enforce, minimum).await?
    {
        return Ok(key);
    }
    Err(AppError::public(
        "no signing key valid at the event timestamp",
    ))
}

async fn refresh_notaries(
    origin: &ServerName,
    id: &ServerSigningKeyId,
    timestamp: UnixMillis,
    enforce: bool,
    minimum: UnixMillis,
) -> AppResult<Option<VerifyKey>> {
    for notary in &config::get().trusted_servers {
        let requested = std::collections::BTreeSet::from([id.to_owned()]);
        if let Ok(documents) =
            super::request::notary_request_for(notary, origin, minimum, Some(&requested)).await
        {
            for keys in documents {
                if keys.server_name != origin {
                    continue;
                }
                add_signing_keys(keys.clone()).await?;
                if let Some(key) = cached_event_key(origin, id, timestamp, enforce).await? {
                    return Ok(Some(key));
                }
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
