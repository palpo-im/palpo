mod acquire;
#[cfg(test)]
mod history_capacity_tests;
#[cfg(test)]
mod legacy_fixture;
#[cfg(test)]
mod notary_tests;
mod request;
#[cfg(test)]
#[path = "server_key/validity/notary_fixture.rs"]
pub(crate) mod test_fixture;
mod trust;
#[cfg(test)]
pub(crate) use trust::seed as seed_verified_keys;
mod validity;
mod verify;
use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::time::Duration;

pub use acquire::*;
pub use request::*;
pub use verify::*;

use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
use crate::core::room_version_rules::RoomVersionRules;
use crate::core::serde::{Base64, CanonicalJsonObject};
use crate::core::signatures::{self, PublicKeyMap, PublicKeySet};
use crate::core::{
    OwnedServerSigningKeyId, RoomVersionId, ServerName, ServerSigningKeyId, UnixMillis,
};
use crate::exts::*;
use crate::utils::timepoint_from_now;
use crate::{AppError, AppResult, config, data};

pub type VerifyKeys = BTreeMap<OwnedServerSigningKeyId, VerifyKey>;
pub type PubKeyMap = PublicKeyMap;
pub type PubKeys = PublicKeySet;

pub async fn verify_keys_for(server: &ServerName) -> VerifyKeys {
    let mut keys = VerifyKeys::new();
    for (document, _) in trust::cached(server)
        .await
        .unwrap_or_default()
        .into_iter()
        .rev()
    {
        keys.extend(merge_old_keys(document).verify_keys);
    }

    if !server.is_remote() {
        let keypair = config::keypair();
        let verify_key = VerifyKey {
            key: Base64::new(keypair.public_key().to_vec()),
        };

        let id = format!("ed25519:{}", keypair.version());
        let verify_keys: VerifyKeys = [(id.try_into().expect("should work"), verify_key)].into();

        keys.extend(verify_keys);
    }

    keys
}

pub async fn signing_keys_for(server: &ServerName) -> AppResult<ServerSigningKeys> {
    trust::cached(server)
        .await?
        .into_iter()
        .next()
        .map(|(keys, _)| keys)
        .ok_or_else(|| AppError::public("no verified signing keys found for server"))
}

/// The verified signed body, without any projection through typed key structs.
pub(crate) async fn signing_key_document_for(server: &ServerName) -> AppResult<serde_json::Value> {
    trust::cached_raw(server)
        .await?
        .ok_or_else(|| AppError::public("no verified signing key document found"))
}

fn minimum_valid_ts() -> UnixMillis {
    let timepoint =
        timepoint_from_now(Duration::from_secs(3600)).expect("SystemTime should not overflow");
    UnixMillis::from_system_time(timepoint).expect("UInt should not overflow")
}

fn merge_old_keys(mut keys: ServerSigningKeys) -> ServerSigningKeys {
    keys.verify_keys.extend(
        keys.old_verify_keys
            .clone()
            .into_iter()
            .map(|(key_id, old)| (key_id, VerifyKey::new(old.key))),
    );

    keys
}

fn extract_key(mut keys: ServerSigningKeys, key_id: &ServerSigningKeyId) -> Option<VerifyKey> {
    // This selector is used for HTTP request authentication. Historical keys
    // are exclusively for events, even when a malformed document lists both.
    if keys.old_verify_keys.contains_key(key_id) {
        return None;
    }
    keys.verify_keys.remove(key_id)
}

fn key_exists(keys: &ServerSigningKeys, key_id: &ServerSigningKeyId) -> bool {
    keys.verify_keys.contains_key(key_id) || keys.old_verify_keys.contains_key(key_id)
}

pub async fn get_event_keys(
    object: &CanonicalJsonObject,
    version: &RoomVersionRules,
) -> AppResult<PubKeyMap> {
    let required = match signatures::required_keys(object, version) {
        Ok(required) => required,
        Err(e) => {
            return Err(AppError::public(format!(
                "failed to determine keys required to verify: {e}"
            )));
        }
    };

    let timestamp = validity::event_timestamp(object)?;
    let mut result = PubKeyMap::new();
    for (server, ids) in required {
        let server: &ServerName = server.borrow();
        let mut keys = PubKeys::new();
        for id in ids {
            let id: &ServerSigningKeyId = id.borrow();
            // A server may attach multiple signatures during key rotation. One
            // valid signature is sufficient; an unavailable/expired additional
            // key must not invalidate the other candidates.
            if let Ok(key) =
                validity::event_verify_key(server, id, timestamp, version.enforce_key_validity)
                    .await
            {
                keys.insert(id.to_string(), key.key);
            }
        }
        result.insert(server.to_string(), keys);
    }
    Ok(result)
}

pub async fn get_pubkeys<'a, S, K>(batch: S) -> PubKeyMap
where
    S: Iterator<Item = (&'a ServerName, K)> + Send,
    K: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
    let mut keys = PubKeyMap::new();
    for (server, key_ids) in batch {
        let pubkeys = get_pubkeys_for(server, key_ids).await;
        keys.insert(server.into(), pubkeys);
    }

    keys
}

pub async fn get_pubkeys_for<'a, I>(origin: &ServerName, key_ids: I) -> PubKeys
where
    I: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
    let mut keys = PubKeys::new();
    for key_id in key_ids {
        if let Ok(verify_key) = get_verify_key(origin, key_id).await {
            keys.insert(key_id.into(), verify_key.key);
        }
    }

    keys
}

pub async fn get_verify_key(
    origin: &ServerName,
    key_id: &ServerSigningKeyId,
) -> AppResult<VerifyKey> {
    let notary_first = crate::config::get().query_trusted_key_servers_first;
    let notary_only = crate::config::get().only_query_trusted_key_servers;

    if !origin.is_remote() {
        let current = config::keypair();
        if key_id.as_str() == format!("ed25519:{}", current.version()) {
            return Ok(VerifyKey::new(Base64::new(current.public_key().to_vec())));
        }
    }
    if trust::retirement(origin, key_id).await?.is_some() {
        return Err(AppError::public(
            "retired key cannot authenticate HTTP requests",
        ));
    }
    if let Some(result) = trust::latest_request_key(origin, key_id).await? {
        return Ok(result);
    }

    if notary_first && let Ok(result) = get_verify_key_from_notaries(origin, key_id).await {
        return Ok(result);
    }

    if !notary_only && let Ok(result) = get_verify_key_from_origin(origin, key_id).await {
        return Ok(result);
    }

    if !notary_first && let Ok(result) = get_verify_key_from_notaries(origin, key_id).await {
        return Ok(result);
    }

    tracing::error!(?key_id, ?origin, "failed to fetch federation signing-key");
    Err(AppError::public("failed to fetch federation signing-key"))
}

async fn get_verify_key_from_notaries(
    origin: &ServerName,
    key_id: &ServerSigningKeyId,
) -> AppResult<VerifyKey> {
    for notary in &crate::config::get().trusted_servers {
        let requested = std::collections::BTreeSet::from([key_id.to_owned()]);
        if let Ok(server_keys) =
            request::notary_request_for(notary, origin, minimum_valid_ts(), Some(&requested)).await
        {
            for server_key in server_keys {
                if server_key.server_name != origin {
                    continue;
                }
                if let Some(result) = trust::request_key(origin, key_id).await? {
                    return Ok(result);
                }
            }
        }
    }

    Err(AppError::public(
        "failed to fetch signing-key from notaries",
    ))
}

async fn get_verify_key_from_origin(
    origin: &ServerName,
    key_id: &ServerSigningKeyId,
) -> AppResult<VerifyKey> {
    let requested = std::collections::BTreeSet::from([key_id.to_owned()]);
    if let Ok(server_key) = request::server_request_for(origin, Some(&requested)).await {
        if !key_exists(&server_key, key_id) {
            return Err(AppError::public("origin response lacks requested key"));
        }
        if let Some(result) = trust::request_key(origin, key_id).await? {
            return Ok(result);
        }
    }

    Err(AppError::public("failed to fetch signing-key from origin"))
}
pub fn sign_json(object: &mut CanonicalJsonObject) -> AppResult<()> {
    signatures::sign_json(
        config::get().server_name.as_str(),
        config::keypair(),
        object,
    )
    .map_err(Into::into)
}

pub fn hash_and_sign_event(
    object: &mut CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> AppResult<()> {
    let version_rules = crate::room::get_version_rules(room_version)?;
    signatures::hash_and_sign_event(
        config::get().server_name.as_str(),
        config::keypair(),
        object,
        &version_rules.redaction,
    )?;
    Ok(())
}
