/// Endpoints for key management
pub mod claim_key;
use std::collections::{BTreeMap, btree_map};
use std::ops::Deref;
use std::time::Duration;

pub use claim_key::*;
use salvo::oapi::{ToParameters, ToSchema};
use serde::{Deserialize, Serialize};

use crate::client::uiaa::AuthData;
use crate::encryption::{CrossSigningKey, DeviceKeys, OneTimeKey};
use crate::serde::{JsonValue, RawJson, RawJsonValue, StringEnum};
use crate::{DeviceKeyAlgorithm, OwnedDeviceId, OwnedDeviceKeyId, OwnedUserId, PrivOwnedStr};

/// An iterator over signed key IDs and their associated data.
#[derive(Debug)]
pub struct SignedKeysIter<'a>(pub(super) btree_map::Iter<'a, Box<str>, Box<RawJsonValue>>);

impl<'a> Iterator for SignedKeysIter<'a> {
    type Item = (&'a str, &'a RawJsonValue);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(id, val)| (&**id, &**val))
    }
}

// /// `POST /_matrix/client/*/keys/query`
// ///
// /// Returns the current devices and identity keys for the given users.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3keysquery

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/keys/query",
//         1.1 => "/_matrix/client/v3/keys/query",
//     }
// };
/// Request type for the `get_keys` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct KeysReqBody {
    /// The time (in milliseconds) to wait when downloading keys from remote
    /// servers.
    ///
    /// 10 seconds is the recommended default.
    #[serde(
        with = "crate::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,

    /// The keys to be downloaded.
    ///
    /// An empty list indicates all devices for the corresponding user.
    pub device_keys: BTreeMap<OwnedUserId, Vec<OwnedDeviceId>>,
}

/// Response type for the `get_keys` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct KeysResBody {
    /// If any remote homeservers could not be reached, they are recorded here.
    ///
    /// The names of the properties are the names of the unreachable servers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub failures: BTreeMap<String, JsonValue>,

    /// Information on the queried devices.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub device_keys: BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, DeviceKeys>>,

    /// Information on the master cross-signing keys of the queried users.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub master_keys: BTreeMap<OwnedUserId, CrossSigningKey>,

    /// Information on the self-signing keys of the queried users.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub self_signing_keys: BTreeMap<OwnedUserId, CrossSigningKey>,

    /// Information on the user-signing keys of the queried users.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub user_signing_keys: BTreeMap<OwnedUserId, CrossSigningKey>,
}

// /// `POST /_matrix/client/*/keys/upload`
// ///
// /// Publishes end-to-end encryption keys for the device.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3keysupload

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/keys/upload",
//         1.1 => "/_matrix/client/v3/keys/upload",
//     }
// };

/// Request type for the `upload_keys` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct UploadKeysReqBody {
    /// Identity keys for the device.
    ///
    /// May be absent if no new identity keys are required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_keys: Option<DeviceKeys>,

    /// One-time public keys for "pre-key" messages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub one_time_keys: BTreeMap<OwnedDeviceKeyId, OneTimeKey>,

    /// Fallback public keys for "pre-key" messages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fallback_keys: BTreeMap<OwnedDeviceKeyId, OneTimeKey>,
}

/// Response type for the `upload_keys` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct UploadKeysResBody {
    /// For each key algorithm, the number of unclaimed one-time keys of that
    /// type currently held on the server for this device.
    pub one_time_key_counts: BTreeMap<DeviceKeyAlgorithm, u64>,
}
impl UploadKeysResBody {
    /// Creates a new `Response` with the given one time key counts.
    pub fn new(one_time_key_counts: BTreeMap<DeviceKeyAlgorithm, u64>) -> Self {
        Self {
            one_time_key_counts,
        }
    }
}

// /// `GET /_matrix/client/*/keys/changes`
// ///
// /// Gets a list of users who have updated their device identity keys since a
// /// previous sync token. `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3keyschanges
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/keys/changes",
//         1.1 => "/_matrix/client/v3/keys/changes",
//     }
// };

/// Request type for the `get_key_changes` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct KeyChangesReqArgs {
    /// The desired start point of the list.
    ///
    /// Should be the next_batch field from a response to an earlier call to
    /// /sync.
    #[salvo(parameter(parameter_in = Query))]
    pub from: String,

    /// The desired end point of the list.
    ///
    /// Should be the next_batch field from a recent call to /sync - typically
    /// the most recent such call.
    #[salvo(parameter(parameter_in = Query))]
    pub to: String,
}

/// Response type for the `get_key_changes` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct KeyChangesResBody {
    /// The Matrix User IDs of all users who updated their device identity keys.
    pub changed: Vec<OwnedUserId>,

    /// The Matrix User IDs of all users who may have left all the end-to-end
    /// encrypted rooms they previously shared with the user.
    pub left: Vec<OwnedUserId>,
}
impl KeyChangesResBody {
    /// Creates a new `Response` with the given changed and left user ID lists.
    pub fn new(changed: Vec<OwnedUserId>, left: Vec<OwnedUserId>) -> Self {
        Self { changed, left }
    }
}

// /// `POST /_matrix/client/*/keys/device_signing/upload`
// ///
// /// Publishes cross signing keys for the user.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3keysdevice_signingupload

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/keys/device_signing/upload",
//         1.1 => "/_matrix/client/v3/keys/device_signing/upload",
//     }
// };

/// Request type for the `upload_signing_keys` endpoint.
#[derive(ToSchema, Deserialize, Clone, Debug)]
pub struct UploadSigningKeysReqBody {
    /// Additional authentication information for the user-interactive
    /// authentication API.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::serde::empty_as_none"
    )]
    pub auth: Option<AuthData>,

    /// The user's master key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_key: Option<CrossSigningKey>,

    /// The user's self-signing key.
    ///
    /// Must be signed with the accompanied master, or by the user's most
    /// recently uploaded master key if no master key is included in the
    /// request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_signing_key: Option<CrossSigningKey>,

    /// The user's user-signing key.
    ///
    /// Must be signed with the accompanied master, or by the user's most
    /// recently uploaded master key if no master key is included in the
    /// request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_signing_key: Option<CrossSigningKey>,
}
// /// `POST /_matrix/client/*/keys/signatures/upload`
// ///
// /// Publishes cross-signing signatures for the user.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3keyssignaturesupload

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/keys/signatures/upload",
//         1.1 => "/_matrix/client/v3/keys/signatures/upload",
//     }
// };

/// Request type for the `upload_signatures` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct UploadSignaturesReqBody(pub BTreeMap<OwnedUserId, SignedKeys>);
impl Deref for UploadSignaturesReqBody {
    type Target = BTreeMap<OwnedUserId, SignedKeys>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Response type for the `upload_signatures` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct UploadSignaturesResBody {
    /// Signature processing failures.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub failures: BTreeMap<OwnedUserId, BTreeMap<String, Failure>>,
}
impl UploadSignaturesResBody {
    /// Creates an empty `Response`.
    pub fn new(failures: BTreeMap<OwnedUserId, BTreeMap<String, Failure>>) -> Self {
        Self { failures }
    }
}

/// A map of key IDs to signed key objects.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct SignedKeys(BTreeMap<Box<str>, Box<RawJsonValue>>);

impl SignedKeys {
    /// Creates an empty `SignedKeys` map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the given device keys.
    pub fn add_device_keys(&mut self, device_id: OwnedDeviceId, device_keys: RawJson<DeviceKeys>) {
        self.0
            .insert(device_id.as_str().into(), device_keys.into_inner());
    }

    /// Add the given cross signing keys.
    pub fn add_cross_signing_keys(
        &mut self,
        cross_signing_key_id: Box<str>,
        cross_signing_keys: RawJson<CrossSigningKey>,
    ) {
        self.0
            .insert(cross_signing_key_id, cross_signing_keys.into_inner());
    }

    /// Returns an iterator over the keys.
    pub fn iter(&self) -> SignedKeysIter<'_> {
        SignedKeysIter(self.0.iter())
    }
}

/// A failure to process a signed key.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct Failure {
    /// Machine-readable error code.
    errcode: FailureErrorCode,

    /// Human-readable error message.
    error: String,
}

/// Error code for signed key processing failures.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
#[palpo_enum(rename_all(prefix = "M_", rule = "SCREAMING_SNAKE_CASE"))]
pub enum FailureErrorCode {
    /// The signature is invalid.
    InvalidSignature,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}
