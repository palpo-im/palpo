/// Endpoints for server-side key backups.
use std::collections::BTreeMap;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::OwnedRoomId;
use crate::identifiers::CrossSigningOrDeviceSignatures;
use crate::serde::{Base64, RawJson, RawJsonValue};

/// A wrapper around a mapping of session IDs to key data.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct RoomKeyBackup {
    /// A map of session IDs to key data.
    pub sessions: BTreeMap<String, KeyBackupData>,
}

impl RoomKeyBackup {
    /// Creates a new `RoomKeyBackup` with the given sessions.
    pub fn new(sessions: BTreeMap<String, KeyBackupData>) -> Self {
        Self { sessions }
    }
}

/// The algorithm used for storing backups.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "algorithm", content = "auth_data")]
pub enum BackupAlgorithm {
    /// `m.megolm_backup.v1.curve25519-aes-sha2` backup algorithm.
    #[serde(rename = "m.megolm_backup.v1.curve25519-aes-sha2")]
    MegolmBackupV1Curve25519AesSha2 {
        /// The curve25519 public key used to encrypt the backups, encoded in
        /// unpadded base64.
        #[salvo(schema(value_type = String))]
        public_key: Base64,

        /// Signatures of the auth_data as Signed JSON.
        #[salvo(schema(value_type = Object, additional_properties = true))]
        signatures: CrossSigningOrDeviceSignatures,
    },
}

/// Information about the backup key.
///
/// To create an instance of this type.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct KeyBackupData {
    /// The index of the first message in the session that the key can decrypt.
    pub first_message_index: u64,

    /// The number of times this key has been forwarded via key-sharing between
    /// devices.
    pub forwarded_count: u64,

    /// Whether the device backing up the key verified the device that the key
    /// is from.
    pub is_verified: bool,

    /// Encrypted data about the session.
    pub session_data: RawJson<EncryptedSessionData>,
}

/// The encrypted algorithm-dependent data for backups.
///
/// To create an instance of this type.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct EncryptedSessionData {
    /// Unpadded base64-encoded public half of the ephemeral key.
    #[salvo(schema(value_type = String))]
    pub ephemeral: Base64,

    /// Ciphertext, encrypted using AES-CBC-256 with PKCS#7 padding, encoded in
    /// base64.
    #[salvo(schema(value_type = String))]
    pub ciphertext: Base64,

    /// First 8 bytes of MAC key, encoded in base64.
    #[salvo(schema(value_type = String))]
    pub mac: Base64,
}

// /// `PUT /_matrix/client/*/room_keys/keys/{room_id}`
// ///
// /// Store keys in the backup for a room.
//
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3room_keyskeysroomid

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/keys/:room_id",
//         1.0 => "/_matrix/client/r0/room_keys/keys/:room_id",
//         1.1 => "/_matrix/client/v3/room_keys/keys/:room_id",
//     }
// };
/// Response type for the `add_backup_keys_for_room` endpoint.

#[derive(ToSchema, Serialize, Debug)]
pub struct ModifyKeysResBody {
    /// An opaque string representing stored keys in the backup.
    ///
    /// Clients can compare it with the etag value they received in the request
    /// of their last key storage request.
    pub etag: String,

    /// The number of keys stored in the backup.
    pub count: u64,
}

impl ModifyKeysResBody {
    /// Creates an new `Response` with the given etag and count.
    pub fn new(etag: String, count: u64) -> Self {
        Self { etag, count }
    }
}

// /// `PUT /_matrix/client/*/room_keys/keys/{room_id}/{sessionId}`
// ///
// /// Store keys in the backup for a session.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3room_keyskeysroomidsessionid

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/keys/:room_id/:session_id",
//         1.0 => "/_matrix/client/r0/room_keys/keys/:room_id/:session_id",
//         1.1 => "/_matrix/client/v3/room_keys/keys/:room_id/:session_id",
//     }
// };

/// Request type for the `add_backup_keys_for_session` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct AddKeysForSessionReqBody(
    /// The key information to store.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub KeyBackupData,
);

// /// `POST /_matrix/client/*/room_keys/version`
// ///
// /// Create a new backup version.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3room_keysversion

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/version",
//         1.1 => "/_matrix/client/v3/room_keys/version",
//     }
// };

/// Request type for the `create_backup_version` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct CreateVersionReqBody(
    /// The algorithm used for storing backups.
    pub RawJson<BackupAlgorithm>,
);

/// Response type for the `create_backup_version` endpoint.

#[derive(ToSchema, Serialize, Debug)]
pub struct CreateVersionResBody {
    /// The backup version.
    pub version: String,
}
impl CreateVersionResBody {
    /// Creates a new `Response` with the given version.
    pub fn new(version: String) -> Self {
        Self { version }
    }
}

// /// `PUT /_matrix/client/*/room_keys/keys`
// ///
// /// Store keys in the backup.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3room_keyskeys
// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/keys",
//         1.1 => "/_matrix/client/v3/room_keys/keys",
//     }
// };

/// Request type for the `add_backup_keys` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct AddKeysReqBody {
    /// A map of room IDs to session IDs to key data to store.
    pub rooms: BTreeMap<OwnedRoomId, RoomKeyBackup>,
}

// /// `DELETE /_matrix/client/*/room_keys/keys/{room_id}`
// ///
// /// Delete keys from a backup for a given room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#delete_matrixclientv3room_keyskeysroomid
// const METADATA: Metadata = metadata! {
//     method: DELETE,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/keys/:room_id",
//         1.0 => "/_matrix/client/r0/room_keys/keys/:room_id",
//         1.1 => "/_matrix/client/v3/room_keys/keys/:room_id",
//     }
// };

/// Request type for the `delete_backup_keys_for_room` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct KeysForRoomReqArgs {
    /// The backup version from which to delete keys.
    #[salvo(parameter(parameter_in = Query))]
    pub version: i64,

    /// The ID of the room to delete keys from.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,
}

// /// `DELETE /_matrix/client/*/room_keys/keys/{room_id}/{session_id}`
// ///
// /// Delete keys from a backup for a given session.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#delete_matrixclientv3room_keyskeysroomidsessionid

// const METADATA: Metadata = metadata! {
//     method: DELETE,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/keys/:room_id/:session_id",
//         1.0 => "/_matrix/client/r0/room_keys/keys/:room_id/:session_id",
//         1.1 => "/_matrix/client/v3/room_keys/keys/:room_id/:session_id",
//     }
// };

// /// `DELETE /_matrix/client/*/room_keys/keys`
// ///
// /// Delete all keys from a backup.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#delete_matrixclientv3room_keyskeys
// ///
// /// This deletes keys from a backup version, but not the version itself.

// const METADATA: Metadata = metadata! {
//     method: DELETE,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/keys",
//         1.0 => "/_matrix/client/r0/room_keys/keys",
//         1.1 => "/_matrix/client/v3/room_keys/keys",
//     }
// };

// /// Request type for the `delete_backup_keys` endpoint.
// #[derive(ToSchema, Deserialize, Debug)]
// pub struct DeleteKeysReqBody {
//     /// The backup version from which to delete keys.
//     #[salvo(parameter(parameter_in = Query))]
//     pub version: String,
// }

// /// `GET /_matrix/client/*/room_keys/version/{version}`
// ///
// /// Get information about a specific backup.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3room_keysversionversion

// https://github.com/rust-lang/rust/issues/112615

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/version/:version",
//         1.1 => "/_matrix/client/v3/room_keys/version/:version",
//     }
// };

/// Response type for the `get_backup_info` endpoint.

#[derive(ToSchema, Serialize, Debug)]
pub struct VersionResBody {
    /// The algorithm used for storing backups.
    #[serde(flatten)]
    pub algorithm: BackupAlgorithm,

    /// The number of keys stored in the backup.
    pub count: u64,

    /// An opaque string representing stored keys in the backup.
    ///
    /// Clients can compare it with the etag value they received in the request
    /// of their last key storage request.
    pub etag: String,

    /// The backup version.
    pub version: String,
}
impl VersionResBody {
    /// Creates a new `Response` with the given algorithm, key count, etag and
    /// version.
    pub fn new(algorithm: BackupAlgorithm, count: u64, etag: String, version: String) -> Self {
        Self {
            algorithm,
            count,
            etag,
            version,
        }
    }
}

// #[derive(Deserialize)]
// pub(crate) struct ResponseBodyRepr {
//     pub algorithm: Box<RawJsonValue>,
//     pub auth_data: Box<RawJsonValue>,
//     pub count: u64,
//     pub etag: String,
//     pub version: String,
// }

// #[derive(Serialize)]
// pub(crate) struct RefResponseBodyRepr<'a> {
//     pub algorithm: &'a RawJsonValue,
//     pub auth_data: &'a RawJsonValue,
//     pub count: u64,
//     pub etag: &'a str,
//     pub version: &'a str,
// }

#[derive(Deserialize, Serialize)]
pub(crate) struct AlgorithmWithData {
    pub algorithm: Box<RawJsonValue>,
    pub auth_data: Box<RawJsonValue>,
}

// impl<'de> Deserialize<'de> for ResponseBody {
//     fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
//     where
//         D: Deserializer<'de>,
//     {
//         let ResponseBodyRepr {
//             algorithm,
//             auth_data,
//             count,
//             etag,
//             version,
//         } = ResponseBodyRepr::deserialize(deserializer)?;

//         let algorithm =
// RawJson::from_json(to_raw_json_value(&AlgorithmWithData { algorithm,
// auth_data }).unwrap());

//         Ok(Self {
//             algorithm,
//             count,
//             etag,
//             version,
//         })
//     }
// }

// impl Serialize for ResponseBody {
//     fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
//     where
//         S: serde::Serializer,
//     {
//         let ResponseBody {
//             algorithm,
//             count,
//             etag,
//             version,
//         } = self;
//         let AlgorithmWithData { algorithm, auth_data } =
// algorithm.deserialize_as().map_err(ser::Error::custom)?;

//         let repr = RefResponseBodyRepr {
//             algorithm: &algorithm,
//             auth_data: &auth_data,
//             count: *count,
//             etag,
//             version,
//         };

//         repr.serialize(serializer)
//     }
// }

// /// `GET /_matrix/client/*/room_keys/keys/{room_id}`
// ///
// /// Retrieve sessions from the backup for a given room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3room_keyskeysroomid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/keys/:room_id",
//         1.0 => "/_matrix/client/r0/room_keys/keys/:room_id",
//         1.1 => "/_matrix/client/v3/room_keys/keys/:room_id",
//     }
// };

/// Request type for the `get_backup_keys_for_room` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct AddKeysForRoomReqBody {
    /// A map of session IDs to key data.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub sessions: BTreeMap<String, KeyBackupData>,
}

impl AddKeysForRoomReqBody {
    /// Creates a new `Response` with the given sessions.
    pub fn new(sessions: BTreeMap<String, KeyBackupData>) -> Self {
        Self { sessions }
    }
}

// /// `GET /_matrix/client/*/room_keys/keys/{room_id}/{session_id}`
// ///
// /// Retrieve a key from the backup for a given session.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3room_keyskeysroomidsessionid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/keys/:room_id/:session_id",
//         1.0 => "/_matrix/client/r0/room_keys/keys/:room_id/:session_id",
//         1.1 => "/_matrix/client/v3/room_keys/keys/:room_id/:session_id",
//     }
// };

/// Request type for the `get_backup_keys_for_session` endpoint.

#[derive(ToParameters, Deserialize, Debug)]
pub struct KeysForSessionReqArgs {
    /// The backup version to retrieve keys from.
    #[salvo(parameter(parameter_in = Query))]
    pub version: i64,

    /// The ID of the room that the requested key is for.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The ID of the megolm session whose key is requested.
    #[salvo(parameter(parameter_in = Path))]
    pub session_id: String,
}

// /// Response type for the `get_backup_keys_for_session` endpoint.
// #[derive(ToSchema, Serialize, Debug)]
// pub struct KeysForSessionResBody (
//     /// Information about the requested backup key.
//     pub RawJson<KeyBackupData>,
// );
// impl KeysForSessionResBody {
//     /// Creates a new `Response` with the given key_data.
//     pub fn new(key_data: RawJson<KeyBackupData>) -> Self {
//         Self (key_data)
//     }
// }

// /// `GET /_matrix/client/*/room_keys/keys`
// ///
// /// Retrieve all keys from a backup version.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3room_keyskeys

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/keys",
//         1.0 => "/_matrix/client/r0/room_keys/keys",
//         1.1 => "/_matrix/client/v3/room_keys/keys",
//     }
// };

// /// Request type for the `get_backup_keys` endpoint.

// pub struct Requxest {
//     /// The backup version to retrieve keys from.
//     #[salvo(parameter(parameter_in = Query))]
//     pub version: String,
// }

/// Response type for the `get_backup_keys` endpoint.

#[derive(ToSchema, Serialize, Debug)]
pub struct KeysResBody {
    /// A map from room IDs to session IDs to key data.
    pub rooms: BTreeMap<OwnedRoomId, RawJson<RoomKeyBackup>>,
}
impl KeysResBody {
    /// Creates a new `Response` with the given room key backups.
    pub fn new(rooms: BTreeMap<OwnedRoomId, RawJson<RoomKeyBackup>>) -> Self {
        Self { rooms }
    }
}

#[derive(ToSchema, Serialize, Debug)]
pub struct KeysForRoomResBody {
    /// A map from room IDs to session IDs to key data.
    pub sessions: BTreeMap<OwnedRoomId, RawJson<RoomKeyBackup>>,
}
impl KeysForRoomResBody {
    /// Creates a new `Response` with the given room key backups.
    pub fn new(sessions: BTreeMap<OwnedRoomId, RawJson<RoomKeyBackup>>) -> Self {
        Self { sessions }
    }
}

// /// `PUT /_matrix/client/*/room_keys/version/{version}`
// ///
// /// Update information about an existing backup.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3room_keysversionversion

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/room_keys/version/:version",
//         1.1 => "/_matrix/client/v3/room_keys/version/:version",
//     }
// };

/// Request type for the `update_backup_version` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct UpdateVersionReqBody(
    /// The algorithm used for storing backups.
    pub BackupAlgorithm,
);
