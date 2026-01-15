//! Endpoints for handling keys for end-to-end encryption
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use salvo::prelude::*;

use crate::core::federation::directory::ServerKeysResBody;
use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
use crate::core::serde::{Base64, CanonicalJsonObject};
use crate::core::{OwnedServerSigningKeyId, UnixMillis};
use crate::{AuthArgs, EmptyResult, JsonResult, config, empty_ok, json_ok};

pub fn router() -> Router {
    Router::with_path("key").oapi_tag("federation").push(
        Router::with_path("v2")
            .push(
                Router::with_path("query")
                    .post(query_keys)
                    .push(Router::with_path("{server_name}").get(query_keys_from_server)),
            )
            .push(Router::with_path("server").get(server_signing_keys)),
    )
}

#[endpoint]
async fn query_keys(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
}

#[endpoint]
async fn query_keys_from_server(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
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
