//! `POST /_matrix/client/*/keys/claim`
//!
//! Claims one-time keys for use in pre-key messages.

use std::collections::BTreeMap;
use std::time::Duration;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::DeviceKeyAlgorithm;
use crate::client::key::{SignedKeys, SignedKeysIter};
use crate::encryption::OneTimeKey;
use crate::identifiers::*;
use crate::serde::{JsonValue, RawJsonValue};

impl<'a> IntoIterator for &'a SignedKeys {
    type Item = (&'a str, &'a RawJsonValue);
    type IntoIter = SignedKeysIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/keys/claim",
//         1.1 => "/_matrix/client/v3/keys/claim",
//     }
// };

/// Request type for the `claim_keys` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct ClaimKeysReqBody {
    /// The time (in milliseconds) to wait when downloading keys from remote
    /// servers. 10 seconds is the recommended default.
    #[serde(
        with = "crate::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,

    /// The keys to be claimed.
    pub one_time_keys: BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, DeviceKeyAlgorithm>>,
}

impl ClaimKeysReqBody {
    /// Creates a new `Request` with the given key claims and the recommended 10
    /// second timeout.
    pub fn new(
        one_time_keys: BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, DeviceKeyAlgorithm>>,
    ) -> Self {
        Self {
            timeout: Some(Duration::from_secs(10)),
            one_time_keys,
        }
    }
}

/// Response type for the `claim_keys` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct ClaimKeysResBody {
    /// If any remote homeservers could not be reached, they are recorded here.
    ///
    /// The names of the properties are the names of the unreachable servers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub failures: BTreeMap<String, JsonValue>,

    /// One-time keys for the queried devices.
    pub one_time_keys: BTreeMap<OwnedUserId, OneTimeKeys>,
}

impl ClaimKeysResBody {
    /// Creates a new `Response` with the given keys and no failures.
    pub fn new(one_time_keys: BTreeMap<OwnedUserId, OneTimeKeys>) -> Self {
        Self {
            failures: BTreeMap::new(),
            one_time_keys,
        }
    }
}

/// The one-time keys for a given device.
pub type OneTimeKeys = BTreeMap<OwnedDeviceId, BTreeMap<OwnedDeviceKeyId, OneTimeKey>>;
