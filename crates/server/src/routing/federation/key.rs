//! Endpoints for handling keys for end-to-end encryption
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::federation::directory::{
    RemoteServerKeysBatchReqBody, RemoteServerKeysBatchResBody, RemoteServerKeysReqArgs,
    RemoteServerKeysResBody, ServerKeysResBody,
};
use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
use crate::core::signatures::Ed25519KeyPair;
use crate::core::serde::{Base64, CanonicalJsonObject};
use crate::core::{OwnedServerName, OwnedServerSigningKeyId, ServerName, UnixMillis};
use crate::{AppResult, AuthArgs, JsonResult, config, json_ok};

pub fn router() -> Router {
    Router::with_path("key").oapi_tag("federation").push(
        Router::with_path("v2")
            .push(
                Router::with_path("query")
                    .post(query_keys_batch)
                    .push(
                        Router::with_path("{server_name}")
                            .get(query_keys_from_server)
                            // Deprecated: /_matrix/key/v2/query/{serverName}/{keyId}
                            .push(Router::with_path("{key_id}").get(query_keys_from_server)),
                    ),
            )
            .push(
                Router::with_path("server")
                    .get(server_signing_keys)
                    // Deprecated: /_matrix/key/v2/server/{keyId}
                    .push(Router::with_path("{key_id}").get(server_signing_keys)),
            ),
    )
}

/// Fetch signing keys for a server, using cache when available.
/// Acts as a notary: returns cached keys or fetches from the origin server.
async fn fetch_signing_keys(
    server: &OwnedServerName,
    minimum_valid_until_ts: UnixMillis,
) -> Option<ServerSigningKeys> {
    // Try local cache first
    if let Ok(cached) = crate::server_key::signing_keys_for(server)
        && cached.valid_until_ts >= minimum_valid_until_ts {
            return Some(cached);
        }

    // Cache miss or expired — fetch from origin server
    match crate::server_key::server_request(server).await {
        Ok(keys) => {
            if let Err(e) = crate::server_key::add_signing_keys(keys.clone()) {
                warn!("failed to cache signing keys for {server}: {e}");
            }
            Some(keys)
        }
        Err(e) => {
            warn!("failed to fetch signing keys from {server}: {e}");
            // Fall back to whatever we have cached, even if expired
            crate::server_key::signing_keys_for(server).ok()
        }
    }
}

fn sign_server_keys_for_notary(
    notary: &ServerName,
    keypair: &Ed25519KeyPair,
    server_keys: ServerSigningKeys,
) -> AppResult<ServerSigningKeys> {
    let buf: Vec<u8> = crate::core::serde::json_to_buf(&server_keys)?;
    let mut object: CanonicalJsonObject = serde_json::from_slice(&buf)?;
    crate::core::signatures::sign_json(notary.as_str(), keypair, &mut object)?;
    Ok(serde_json::from_slice(&serde_json::to_vec(&object)?)?)
}

/// `POST /_matrix/key/v2/query`
///
/// Batch query for signing keys of multiple servers.
/// This server acts as a notary, returning keys it has cached or fetching from origin.
#[endpoint]
async fn query_keys_batch(
    _aa: AuthArgs,
    body: JsonBody<RemoteServerKeysBatchReqBody>,
) -> JsonResult<RemoteServerKeysBatchResBody> {
    let conf = config::get();
    let mut result_keys = Vec::new();

    for (server, key_criteria) in &body.server_keys {
        // Determine the most restrictive minimum_valid_until_ts across all requested keys
        let min_valid = key_criteria
            .values()
            .filter_map(|c| c.minimum_valid_until_ts)
            .max()
            .unwrap_or_else(UnixMillis::now);

        if let Some(keys) = fetch_signing_keys(server, min_valid).await {
            result_keys.push(sign_server_keys_for_notary(
                &conf.server_name,
                config::keypair(),
                keys,
            )?);
        }
    }

    json_ok(RemoteServerKeysBatchResBody::new(result_keys))
}

/// `GET /_matrix/key/v2/query/{server_name}`
///
/// Query signing keys for a specific server.
/// This server acts as a notary, returning keys it has cached or fetching from origin.
#[endpoint]
async fn query_keys_from_server(
    _aa: AuthArgs,
    args: RemoteServerKeysReqArgs,
) -> JsonResult<RemoteServerKeysResBody> {
    let conf = config::get();
    let mut result_keys = Vec::new();

    if let Some(keys) = fetch_signing_keys(&args.server_name, args.minimum_valid_until_ts).await {
        result_keys.push(sign_server_keys_for_notary(
            &conf.server_name,
            config::keypair(),
            keys,
        )?);
    }

    json_ok(RemoteServerKeysResBody::new(result_keys))
}

/// #GET /_matrix/key/v2/server
/// Gets the public signing keys of this server.
///
/// - Matrix does not support invalidating public keys, so the key returned by this will be valid
/// forever.
// Response type for this endpoint is Json because we need to calculate a signature for the response
#[endpoint]
async fn server_signing_keys(_aa: AuthArgs) -> JsonResult<ServerKeysResBody> {
    let conf = crate::config::get();
    let mut verify_keys: BTreeMap<OwnedServerSigningKeyId, VerifyKey> = BTreeMap::new();
    verify_keys.insert(
        format!("ed25519:{}", config::keypair().version())
            .try_into()
            .expect("found invalid server signing keys in DB"),
        VerifyKey {
            key: Base64::new(config::keypair().public_key().to_vec()),
        },
    );
    let server_keys = ServerSigningKeys {
        server_name: conf.server_name.clone(),
        verify_keys,
        old_verify_keys: BTreeMap::new(),
        signatures: BTreeMap::new(),
        valid_until_ts: UnixMillis::from_system_time(
            SystemTime::now() + Duration::from_secs(86400 * 7),
        )
        .expect("time is valid"),
    };
    let buf: Vec<u8> = crate::core::serde::json_to_buf(&server_keys)?;
    let mut server_keys: CanonicalJsonObject = serde_json::from_slice(&buf)?;

    crate::core::signatures::sign_json(
        conf.server_name.as_str(),
        config::keypair(),
        &mut server_keys,
    )?;
    let server_keys: ServerSigningKeys =
        serde_json::from_slice(&serde_json::to_vec(&server_keys).unwrap())?;

    json_ok(ServerKeysResBody::new(server_keys))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::core::signatures::Ed25519KeyPair;

    fn generate_key_pair(version: &str) -> Ed25519KeyPair {
        let key_content = Ed25519KeyPair::generate().unwrap();
        Ed25519KeyPair::from_der(&key_content, version.to_owned()).unwrap()
    }

    #[test]
    fn notary_signing_preserves_origin_signature_and_adds_our_own() {
        let origin = OwnedServerName::try_from("origin.example").unwrap();
        let notary = OwnedServerName::try_from("notary.example").unwrap();
        let origin_keypair = generate_key_pair("origin");
        let notary_keypair = generate_key_pair("notary");

        let mut server_keys = ServerSigningKeys::new(
            origin.clone(),
            UnixMillis::from_system_time(SystemTime::now() + Duration::from_secs(60)).unwrap(),
        );
        server_keys.verify_keys.insert(
            format!("ed25519:{}", origin_keypair.version())
                .try_into()
                .unwrap(),
            VerifyKey::from_bytes(origin_keypair.public_key().to_vec()),
        );

        let buf: Vec<u8> = crate::core::serde::json_to_buf(&server_keys).unwrap();
        let mut object: CanonicalJsonObject = serde_json::from_slice(&buf).unwrap();
        crate::core::signatures::sign_json(origin.as_str(), &origin_keypair, &mut object).unwrap();
        let server_keys: ServerSigningKeys =
            serde_json::from_slice(&serde_json::to_vec(&object).unwrap()).unwrap();

        let signed = sign_server_keys_for_notary(&notary, &notary_keypair, server_keys).unwrap();
        let notary_key_id: OwnedServerSigningKeyId =
            format!("ed25519:{}", notary_keypair.version()).try_into().unwrap();

        assert!(signed.signatures.contains_key(&origin));
        assert!(signed.signatures.contains_key(&notary));
        assert!(signed.signatures[&notary].contains_key(&notary_key_id));
    }
}
