mod acquire;
mod request;
mod verify;
pub use acquire::*;
pub use request::*;
pub use verify::*;

use std::borrow::Borrow;
use std::{collections::BTreeMap, time::Duration};

use diesel::prelude::*;
use serde_json::value::RawValue as RawJsonValue;

use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
use crate::core::serde::{Base64, CanonicalJsonObject, JsonValue, RawJson};
use crate::core::signatures::{self, PublicKeyMap, PublicKeySet};
use crate::core::{OwnedServerSigningKeyId, RoomVersionId, ServerName, ServerSigningKeyId, UnixMillis};
use crate::data::connect;
use crate::data::misc::DbServerSigningKeys;
use crate::data::schema::*;
use crate::utils::timepoint_from_now;
use crate::{AppError, AppResult, exts::*};

pub type VerifyKeys = BTreeMap<OwnedServerSigningKeyId, VerifyKey>;
pub type PubKeyMap = PublicKeyMap;
pub type PubKeys = PublicKeySet;

fn add_signing_keys(new_keys: ServerSigningKeys) -> AppResult<()> {
    let server: &palpo_core::OwnedServerName = &new_keys.server_name;

    // (timo) Not atomic, but this is not critical
    let keys = server_signing_keys::table
        .find(server)
        .select(server_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;
    let mut keys = if let Some(keys) = keys {
        serde_json::from_value::<ServerSigningKeys>(keys)?
    } else {
        // Just insert "now", it doesn't matter
        ServerSigningKeys::new(server.to_owned(), UnixMillis::now())
    };

    keys.verify_keys.extend(new_keys.verify_keys);
    keys.old_verify_keys.extend(new_keys.old_verify_keys);
    diesel::insert_into(server_signing_keys::table)
        .values(DbServerSigningKeys {
            server_id: server.to_owned(),
            key_data: serde_json::to_value(&keys)?,
            updated_at: UnixMillis::now(),
            created_at: UnixMillis::now(),
        })
        .on_conflict(server_signing_keys::server_id)
        .do_update()
        .set((
            server_signing_keys::key_data.eq(serde_json::to_value(&keys)?),
            server_signing_keys::updated_at.eq(UnixMillis::now()),
        ))
        .execute(&mut connect()?)?;
    Ok(())
}

pub fn required_keys_exist(object: &CanonicalJsonObject, version: &RoomVersionId) -> bool {
    use signatures::required_keys;

    let Ok(required_keys) = required_keys(object, version) else {
        return false;
    };

    let key_ids = required_keys
        .iter()
        .flat_map(|(server, key_ids)| key_ids.iter().map(move |key_id| (server, key_id)));
    for (server, key_id) in key_ids {
        if !verify_key_exists(server, key_id).unwrap_or(false) {
            return false;
        }
    }
    true
}

pub fn verify_key_exists(server: &ServerName, key_id: &ServerSigningKeyId) -> AppResult<bool> {
    type KeysMap<'a> = BTreeMap<&'a str, &'a RawJsonValue>;

    let key_data = server_signing_keys::table
        .filter(server_signing_keys::server_id.eq(server))
        .select(server_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;

    let Some(keys) = key_data else {
        return Ok(false);
    };
    let keys: RawJson<ServerSigningKeys> = RawJson::from_value(&keys)?;

    if let Ok(Some(verify_keys)) = keys.get_field::<KeysMap<'_>>("verify_keys") {
        if verify_keys.contains_key(&key_id.as_str()) {
            return Ok(true);
        }
    }

    if let Ok(Some(old_verify_keys)) = keys.get_field::<KeysMap<'_>>("old_verify_keys") {
        if old_verify_keys.contains_key(&key_id.as_str()) {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn verify_keys_for(server: &ServerName) -> VerifyKeys {
    let mut keys = signing_keys_for(server)
        .map(|keys| merge_old_keys(keys).verify_keys)
        .unwrap_or(BTreeMap::new());

    if !server.is_remote() {
        let keypair = crate::keypair();
        let verify_key = VerifyKey {
            key: Base64::new(keypair.public_key().to_vec()),
        };

        let id = format!("ed25519:{}", keypair.version());
        let verify_keys: VerifyKeys = [(id.try_into().expect("should work"), verify_key)].into();

        keys.extend(verify_keys.clone().into_iter());
    }

    keys
}

pub fn signing_keys_for(server: &ServerName) -> AppResult<ServerSigningKeys> {
    let key_data = server_signing_keys::table
        .filter(server_signing_keys::server_id.eq(server))
        .select(server_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)?;
    Ok(serde_json::from_value(key_data)?)
}

fn minimum_valid_ts() -> UnixMillis {
    let timepoint = timepoint_from_now(Duration::from_secs(3600)).expect("SystemTime should not overflow");
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
    keys.verify_keys
        .remove(key_id)
        .or_else(|| keys.old_verify_keys.remove(key_id).map(|old| VerifyKey::new(old.key)))
}

fn key_exists(keys: &ServerSigningKeys, key_id: &ServerSigningKeyId) -> bool {
    keys.verify_keys.contains_key(key_id) || keys.old_verify_keys.contains_key(key_id)
}

pub async fn get_event_keys(object: &CanonicalJsonObject, version: &RoomVersionId) -> AppResult<PubKeyMap> {
    let required = match signatures::required_keys(object, version) {
        Ok(required) => required,
        Err(e) => {
            return Err(AppError::public(format!(
                "Failed to determine keys required to verify: {e}"
            )));
        }
    };

    let batch = required
        .iter()
        .map(|(s, ids)| (s.borrow(), ids.iter().map(Borrow::borrow)));

    Ok(get_pubkeys(batch).await)
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

pub async fn get_verify_key(origin: &ServerName, key_id: &ServerSigningKeyId) -> AppResult<VerifyKey> {
    let notary_first = crate::config().query_trusted_key_servers_first;
    let notary_only = crate::config().only_query_trusted_key_servers;

    if let Some(result) = verify_keys_for(origin).remove(key_id) {
        return Ok(result);
    }

    if notary_first {
        if let Ok(result) = get_verify_key_from_notaries(origin, key_id).await {
            return Ok(result);
        }
    }

    if !notary_only {
        if let Ok(result) = get_verify_key_from_origin(origin, key_id).await {
            return Ok(result);
        }
    }

    if !notary_first {
        if let Ok(result) = get_verify_key_from_notaries(origin, key_id).await {
            return Ok(result);
        }
    }

    tracing::error!(?key_id, ?origin, "Failed to fetch federation signing-key");
    Err(AppError::public("Failed to fetch federation signing-key"))
}

async fn get_verify_key_from_notaries(origin: &ServerName, key_id: &ServerSigningKeyId) -> AppResult<VerifyKey> {
    for notary in &crate::config().trusted_servers {
        if let Ok(server_keys) = notary_request(notary, origin).await {
            for server_key in server_keys.clone() {
                add_signing_keys(server_key)?;
            }

            for server_key in server_keys {
                if let Some(result) = extract_key(server_key, key_id) {
                    return Ok(result);
                }
            }
        }
    }

    Err(AppError::public("Failed to fetch signing-key from notaries"))
}

async fn get_verify_key_from_origin(origin: &ServerName, key_id: &ServerSigningKeyId) -> AppResult<VerifyKey> {
    if let Ok(server_key) = server_request(origin).await {
        add_signing_keys(server_key.clone())?;
        if let Some(result) = extract_key(server_key, key_id) {
            return Ok(result);
        }
    }

    Err(AppError::public("Failed to fetch signing-key from origin"))
}
pub fn sign_json(object: &mut CanonicalJsonObject) -> AppResult<()> {
    signatures::sign_json(crate::server_name().as_str(), crate::keypair(), object).map_err(Into::into)
}

pub fn hash_and_sign_event(
    object: &mut CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> Result<(), crate::core::signatures::Error> {
    signatures::hash_and_sign_event(crate::server_name().as_str(), crate::keypair(), object, room_version)
}
