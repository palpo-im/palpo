//! Server discovery endpoints.

use std::collections::BTreeMap;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::serde::Base64;
use crate::{OwnedServerName, OwnedServerSigningKeyId, UnixMillis};

/// Public key of the homeserver for verifying digital signatures.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct VerifyKey {
    /// The unpadded base64-encoded key.
    #[salvo(schema(value_type = String))]
    pub key: Base64,
}

impl VerifyKey {
    /// Creates a new `VerifyKey` from the given key.
    pub fn new(key: Base64) -> Self {
        Self { key }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            key: Base64::new(bytes),
        }
    }
}

/// A key the server used to use, but stopped using.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OldVerifyKey {
    /// Timestamp when this key expired.
    pub expired_ts: UnixMillis,

    /// The unpadded base64-encoded key.
    pub key: Base64,
}

impl OldVerifyKey {
    /// Creates a new `OldVerifyKey` with the given expiry time and key.
    pub fn new(expired_ts: UnixMillis, key: Base64) -> Self {
        Self { expired_ts, key }
    }
}

// Spec is wrong, all fields are required (see https://github.com/matrix-org/matrix-spec/issues/613)
/// Queried server key, signed by the notary server.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct ServerSigningKeys {
    /// DNS name of the homeserver.
    pub server_name: OwnedServerName,

    /// Public keys of the homeserver for verifying digital signatures.
    pub verify_keys: BTreeMap<OwnedServerSigningKeyId, VerifyKey>,

    /// Public keys that the homeserver used to use and when it stopped using
    /// them.
    pub old_verify_keys: BTreeMap<OwnedServerSigningKeyId, OldVerifyKey>,

    /// Digital signatures of this object signed using the verify_keys.
    ///
    /// Map of server name to keys by key ID.
    pub signatures: BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, String>>,

    /// Timestamp when the keys should be refreshed.
    ///
    /// This field MUST be ignored in room versions 1, 2, 3, and 4.
    pub valid_until_ts: UnixMillis,
}

impl ServerSigningKeys {
    /// Creates a new `ServerSigningKeys` with the given server name and
    /// validity timestamp.
    ///
    /// All other fields will be empty.
    pub fn new(server_name: OwnedServerName, valid_until_ts: UnixMillis) -> Self {
        Self {
            server_name,
            verify_keys: BTreeMap::new(),
            old_verify_keys: BTreeMap::new(),
            signatures: BTreeMap::new(),
            valid_until_ts,
        }
    }
}

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/.well-known/matrix/server",
//     }
// };
/// Response type for the `discover_homeserver` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct ServerResBody {
    /// The server name to delegate server-server communications to, with
    /// optional port.
    #[serde(rename = "m.server")]
    pub server: OwnedServerName,
}

impl ServerResBody {
    /// Creates a new `Response` with the given homeserver.
    pub fn new(server: OwnedServerName) -> Self {
        Self { server }
    }
}
