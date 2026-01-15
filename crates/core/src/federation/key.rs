/// Endpoints for handling keys for end-to-end encryption
///
/// `POST /_matrix/federation/*/user/keys/claim`
///
/// Claim one-time keys for use in pre-key messages.
/// `/v1/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/server-server-api/#post_matrixfederationv1userkeysclaim
use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::Url;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::encryption::{CrossSigningKey, DeviceKeys, OneTimeKey};
use crate::sending::{SendRequest, SendResult};
use crate::serde::Base64;
use crate::{DeviceKeyAlgorithm, OwnedDeviceId, OwnedDeviceKeyId, OwnedUserId};

pub fn get_server_key_request(origin: &str) -> SendResult<SendRequest> {
    let url = Url::parse(&format!("{origin}/_matrix/key/v2/server"))?;
    Ok(crate::sending::get(url))
}

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/user/keys/claim",
//     }
// };

// pub fn claim_keys_request(txn_id: &str, body: ClaimKeysReqBody) ->
// SendRequest {     let url = registration
//         .build_url(&format!("/app/v1/transactions/{}", txn_id))
//     crate::sending::post(url)
//         .stuff(req_body)
// }

pub fn claim_keys_request(origin: &str, body: ClaimKeysReqBody) -> SendResult<SendRequest> {
    let url = Url::parse(&format!("{origin}/_matrix/federation/v1/user/keys/claim"))?;
    crate::sending::post(url).stuff(body)
}

/// Request type for the `claim_keys` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Debug)]
pub struct ClaimKeysReqBody {
    #[serde(
        with = "crate::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,

    /// The keys to be claimed.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub one_time_keys: OneTimeKeyClaims,
}
crate::json_body_modifier!(ClaimKeysReqBody);

/// Response type for the `claim_keys` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Debug)]

pub struct ClaimKeysResBody {
    /// One-time keys for the queried devices
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub one_time_keys: OneTimeKeys,
}
impl ClaimKeysResBody {
    /// Creates a new `Response` with the given one time keys.
    pub fn new(one_time_keys: OneTimeKeys) -> Self {
        Self { one_time_keys }
    }
}

/// A claim for one time keys
pub type OneTimeKeyClaims = BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, DeviceKeyAlgorithm>>;

/// One time keys for use in pre-key messages
pub type OneTimeKeys =
    BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, BTreeMap<OwnedDeviceKeyId, OneTimeKey>>>;

/// A key and its signature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyObject {
    /// The key, encoded using unpadded base64.
    pub key: Base64,

    /// Signature of the key object.
    pub signatures: BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceKeyId, String>>,
}

impl KeyObject {
    /// Creates a new `KeyObject` with the given key and signatures.
    pub fn new(
        key: Base64,
        signatures: BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceKeyId, String>>,
    ) -> Self {
        Self { key, signatures }
    }
}

// /// `POST /_matrix/federation/*/user/keys/query`
// ///
// /// Get the current devices and identity keys for the given users.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#post_matrixfederationv1userkeysquery
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/user/keys/query",
//     }
// };

pub fn query_keys_request(origin: &str, body: QueryKeysReqBody) -> SendResult<SendRequest> {
    let url = Url::parse(&format!("{origin}/_matrix/federation/v1/user/keys/query"))?;
    crate::sending::post(url).stuff(body)
}

/// Request type for the `get_keys` endpoint.

#[derive(ToSchema, Deserialize, Serialize, Debug)]
pub struct QueryKeysReqBody {
    /// The keys to be downloaded.
    ///
    /// Gives all keys for a given user if the list of device ids is empty.
    pub device_keys: BTreeMap<OwnedUserId, Vec<OwnedDeviceId>>,
}
crate::json_body_modifier!(QueryKeysReqBody);

/// Response type for the `get_keys` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Default, Debug)]

pub struct QueryKeysResBody {
    /// Keys from the queried devices.
    pub device_keys: BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, DeviceKeys>>,

    /// Information on the master cross-signing keys of the queried users.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub master_keys: BTreeMap<OwnedUserId, CrossSigningKey>,

    /// Information on the self-signing keys of the queried users.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub self_signing_keys: BTreeMap<OwnedUserId, CrossSigningKey>,
}
impl QueryKeysResBody {
    /// Creates a new `Response` with the given device keys.
    pub fn new(device_keys: BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, DeviceKeys>>) -> Self {
        Self {
            device_keys,
            ..Default::default()
        }
    }
}
