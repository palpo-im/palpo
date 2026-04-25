//! Modules for events in the `m.room` namespace.
//!
//! This module also contains types shared by events in its child namespaces.

use std::{
    collections::{BTreeMap, btree_map},
    fmt,
    ops::Deref,
};

use as_variant::as_variant;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize, de};

use crate::serde::{
    Base64, JsonObject, StringEnum,
    base64::{Standard, UrlSafe},
};
use crate::{OwnedMxcUri, PrivOwnedStr};

pub mod aliases;
pub mod avatar;
pub mod canonical_alias;
pub mod create;
pub mod encrypted;
mod encrypted_file_serde;
pub mod encryption;
pub mod guest_access;
pub mod history_visibility;
pub mod join_rule;
pub mod member;
pub mod message;
pub mod name;
pub mod pinned_events;
pub mod policy;
pub mod power_levels;
pub mod redaction;
pub mod server_acl;
pub mod third_party_invite;
mod thumbnail_source_serde;
pub mod tombstone;
pub mod topic;

/// The source of a media file.
#[derive(ToSchema, Clone, Debug, Serialize)]
#[allow(clippy::exhaustive_enums)]
pub enum MediaSource {
    /// The MXC URI to the unencrypted media file.
    #[serde(rename = "url")]
    Plain(OwnedMxcUri),

    /// The encryption info of the encrypted media file.
    #[serde(rename = "file")]
    Encrypted(Box<EncryptedFile>),
}

// Custom implementation of `Deserialize`, because serde doesn't guarantee what
// variant will be deserialized for "externally tagged"¹ enums where multiple
// "tag" fields exist.
//
// ¹ https://serde.rs/enum-representations.html
impl<'de> Deserialize<'de> for MediaSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct MediaSourceJsonRepr {
            url: Option<OwnedMxcUri>,
            file: Option<Box<EncryptedFile>>,
        }

        match MediaSourceJsonRepr::deserialize(deserializer)? {
            MediaSourceJsonRepr {
                url: None,
                file: None,
            } => Err(de::Error::missing_field("url")),
            // Prefer file if it is set
            MediaSourceJsonRepr {
                file: Some(file), ..
            } => Ok(MediaSource::Encrypted(file)),
            MediaSourceJsonRepr { url: Some(url), .. } => Ok(MediaSource::Plain(url)),
        }
    }
}

/// Metadata about an image.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct ImageInfo {
    /// The height of the image in pixels.
    #[serde(rename = "h", skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,

    /// The width of the image in pixels.
    #[serde(rename = "w", skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,

    /// The MIME type of the image, e.g. "image/png."
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimetype: Option<String>,

    /// The file size of the image in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,

    /// Metadata about the image referred to in `thumbnail_source`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_info: Option<Box<ThumbnailInfo>>,

    /// The source of the thumbnail of the image.
    #[serde(
        flatten,
        with = "thumbnail_source_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub thumbnail_source: Option<MediaSource>,

    /// The [BlurHash](https://blurha.sh) for this image.
    ///
    /// This uses the unstable prefix in
    /// [MSC2448](https://github.com/matrix-org/matrix-spec-proposals/pull/2448).
    #[cfg(feature = "unstable-msc2448")]
    #[serde(
        rename = "xyz.amorgan.blurhash",
        skip_serializing_if = "Option::is_none"
    )]
    pub blurhash: Option<String>,

    /// If this flag is `true`, the original image SHOULD be assumed to be animated. If this flag
    /// is `false`, the original image SHOULD be assumed to NOT be animated.
    ///
    /// If a sending client is unable to determine whether an image is animated, it SHOULD leave
    /// the flag unset.
    ///
    /// Receiving clients MAY use this flag to optimize whether to download the original image
    /// rather than a thumbnail if it is animated, but they SHOULD NOT trust this flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_animated: Option<bool>,
}

impl ImageInfo {
    /// Creates an empty `ImageInfo`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Metadata about a thumbnail.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct ThumbnailInfo {
    /// The height of the thumbnail in pixels.
    #[serde(rename = "h", skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,

    /// The width of the thumbnail in pixels.
    #[serde(rename = "w", skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,

    /// The MIME type of the thumbnail, e.g. "image/png."
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimetype: Option<String>,

    /// The file size of the thumbnail in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

impl ThumbnailInfo {
    /// Creates an empty `ThumbnailInfo`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// A file sent to a room with end-to-end encryption enabled.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct EncryptedFile {
    /// The URL to the file.
    pub url: OwnedMxcUri,

    /// Information about the encryption of the file.
    #[serde(flatten)]
    pub info: EncryptedFileInfo,

    /// A map from an algorithm name to a hash of the ciphertext.
    ///
    /// Clients should support the SHA-256 hash.
    pub hashes: EncryptedFileHashes,
}

impl EncryptedFile {
    /// Construct a new `EncryptedFile` with the given URL, encryption info and hashes.
    pub fn new(url: OwnedMxcUri, info: EncryptedFileInfo, hashes: EncryptedFileHashes) -> Self {
        Self { url, info, hashes }
    }
}

/// Information about the encryption of a file.
#[derive(ToSchema, Debug, Clone, Serialize)]
#[serde(tag = "v", rename_all = "lowercase")]
pub enum EncryptedFileInfo {
    /// Information about a file encrypted using version 2 of the attachment encryption protocol.
    V2(V2EncryptedFileInfo),

    #[doc(hidden)]
    #[serde(untagged)]
    _Custom(CustomEncryptedFileInfo),
}

impl EncryptedFileInfo {
    /// Get the version of the attachment encryption protocol.
    ///
    /// This matches the `v` field in the serialized data.
    pub fn version(&self) -> &str {
        match self {
            Self::V2(_) => "v2",
            Self::_Custom(info) => &info.v,
        }
    }

    /// Get the data of the attachment encryption protocol, if it doesn't match one of the known
    /// variants.
    pub fn custom_data(&self) -> Option<&JsonObject> {
        as_variant!(self, Self::_Custom(info) => &info.data)
    }
}

impl<'de> Deserialize<'de> for EncryptedFileInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut data = JsonObject::deserialize(deserializer)?;
        let version = data
            .get("v")
            .and_then(|value| value.as_str())
            .ok_or_else(|| de::Error::missing_field("v"))?
            .to_owned();

        match version.as_str() {
            "v2" => serde_json::from_value(serde_json::Value::Object(data))
                .map(Self::V2)
                .map_err(de::Error::custom),
            _ => {
                data.remove("v");
                Ok(Self::_Custom(CustomEncryptedFileInfo { v: version, data }))
            }
        }
    }
}

impl From<V2EncryptedFileInfo> for EncryptedFileInfo {
    fn from(value: V2EncryptedFileInfo) -> Self {
        Self::V2(value)
    }
}

/// A file encrypted with the AES-CTR algorithm with a 256-bit key.
#[derive(ToSchema, Clone)]
pub struct V2EncryptedFileInfo {
    /// The 256-bit key used to encrypt or decrypt the file.
    pub k: Base64<UrlSafe, [u8; 32]>,

    /// The 128-bit unique counter block used by AES-CTR.
    pub iv: Base64<Standard, [u8; 16]>,
}

impl V2EncryptedFileInfo {
    /// Construct a new `V2EncryptedFileInfo` with the given encoded key and initialization vector.
    pub fn new(k: Base64<UrlSafe, [u8; 32]>, iv: Base64<Standard, [u8; 16]>) -> Self {
        Self { k, iv }
    }

    /// Construct a new `V2EncryptedFileInfo` by base64-encoding the given key and initialization
    /// vector bytes.
    pub fn encode(k: [u8; 32], iv: [u8; 16]) -> Self {
        Self::new(Base64::new(k), Base64::new(iv))
    }
}

impl fmt::Debug for V2EncryptedFileInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("V2EncryptedFileInfo")
            .finish_non_exhaustive()
    }
}

/// Information about a file encrypted using a custom version of the attachment encryption protocol.
#[doc(hidden)]
#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
pub struct CustomEncryptedFileInfo {
    /// The version of the protocol.
    v: String,

    /// Extra data about the encryption.
    #[serde(flatten)]
    data: JsonObject,
}

/// A map of [`EncryptedFileHashAlgorithm`] to the associated [`EncryptedFileHash`].
///
/// This type is used to ensure that a supported [`EncryptedFileHash`] always matches the
/// appropriate [`EncryptedFileHashAlgorithm`].
#[derive(Clone, Debug, Default)]
pub struct EncryptedFileHashes(BTreeMap<EncryptedFileHashAlgorithm, EncryptedFileHash>);

impl EncryptedFileHashes {
    /// Construct an empty `EncryptedFileHashes`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct an `EncryptedFileHashes` that includes the given SHA-256 hash.
    pub fn with_sha256(hash: [u8; 32]) -> Self {
        std::iter::once(EncryptedFileHash::Sha256(Base64::new(hash))).collect()
    }

    /// Insert the given [`EncryptedFileHash`].
    ///
    /// If a hash with the same [`EncryptedFileHashAlgorithm`] was already present, it is returned.
    pub fn insert(&mut self, hash: EncryptedFileHash) -> Option<EncryptedFileHash> {
        self.0.insert(hash.algorithm(), hash)
    }
}

impl Deref for EncryptedFileHashes {
    type Target = BTreeMap<EncryptedFileHashAlgorithm, EncryptedFileHash>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromIterator<EncryptedFileHash> for EncryptedFileHashes {
    fn from_iter<T: IntoIterator<Item = EncryptedFileHash>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|hash| (hash.algorithm(), hash))
                .collect(),
        )
    }
}

impl Extend<EncryptedFileHash> for EncryptedFileHashes {
    fn extend<T: IntoIterator<Item = EncryptedFileHash>>(&mut self, iter: T) {
        self.0
            .extend(iter.into_iter().map(|hash| (hash.algorithm(), hash)));
    }
}

impl IntoIterator for EncryptedFileHashes {
    type Item = EncryptedFileHash;
    type IntoIter = btree_map::IntoValues<EncryptedFileHashAlgorithm, EncryptedFileHash>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_values()
    }
}

impl ToSchema for EncryptedFileHashes {
    fn to_schema(
        components: &mut salvo::oapi::Components,
    ) -> salvo::oapi::RefOr<salvo::oapi::Schema> {
        <BTreeMap<String, Base64>>::to_schema(components)
    }
}

/// An algorithm used to generate the hash of an [`EncryptedFile`].
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "lowercase")]
pub enum EncryptedFileHashAlgorithm {
    /// The SHA-256 algorithm.
    Sha256,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// The hash of an encrypted file's ciphertext.
#[derive(ToSchema, Clone, Debug)]
pub enum EncryptedFileHash {
    /// A hash computed with the SHA-256 algorithm.
    Sha256(Base64<Standard, [u8; 32]>),

    #[doc(hidden)]
    _Custom(CustomEncryptedFileHash),
}

impl EncryptedFileHash {
    /// The algorithm that was used to generate this hash.
    pub fn algorithm(&self) -> EncryptedFileHashAlgorithm {
        match self {
            Self::Sha256(_) => EncryptedFileHashAlgorithm::Sha256,
            Self::_Custom(custom) => custom.algorithm.as_str().into(),
        }
    }

    /// Get a reference to the decoded bytes of the hash.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Sha256(hash) => hash.as_bytes(),
            Self::_Custom(custom) => custom.hash.as_bytes(),
        }
    }

    /// Get the decoded bytes of the hash.
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Sha256(hash) => hash.into_inner().into(),
            Self::_Custom(custom) => custom.hash.into_inner(),
        }
    }
}

/// A hash computed with a custom algorithm.
#[doc(hidden)]
#[derive(ToSchema, Clone, Debug)]
pub struct CustomEncryptedFileHash {
    /// The algorithm that was used to generate the hash.
    algorithm: String,

    /// The hash.
    hash: Base64,
}

#[cfg(test)]
mod encrypted_file_tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::{
        EncryptedFile, EncryptedFileHash, EncryptedFileHashAlgorithm, EncryptedFileInfo,
        V2EncryptedFileInfo,
    };

    fn encrypted_file_json() -> serde_json::Value {
        json!({
            "url": "mxc://notareal.hs/abcdef",
            "key": {
                "kty": "oct",
                "key_ops": ["encrypt", "decrypt"],
                "alg": "A256CTR",
                "k": "TLlG_OpX807zzQuuwv4QZGJ21_u7weemFGYJFszMn9A",
                "ext": true
            },
            "iv": "S22dq3NAX8wAAAAAAAAAAA",
            "hashes": {
                "sha256": "aWOHudBnDkJ9IwaR1Nd8XKoI7DOrqDTwt6xDPfVGN6Q"
            },
            "v": "v2",
        })
    }

    #[test]
    fn encrypted_file_round_trips_strict_types() {
        let file: EncryptedFile = from_json_value(encrypted_file_json()).unwrap();

        assert_eq!(file.info.version(), "v2");
        let EncryptedFileInfo::V2(V2EncryptedFileInfo { k, iv }) = &file.info else {
            panic!("expected v2 encrypted file info");
        };
        assert_eq!(k.as_inner().len(), 32);
        assert_eq!(iv.as_inner().len(), 16);

        let hash = file
            .hashes
            .get(&EncryptedFileHashAlgorithm::Sha256)
            .expect("sha256 hash should be present");
        let EncryptedFileHash::Sha256(hash) = hash else {
            panic!("expected sha256 encrypted file hash");
        };
        assert_eq!(hash.as_inner().len(), 32);

        let serialized = to_json_value(&file).unwrap();
        assert_eq!(serialized["v"], "v2");
        assert_eq!(serialized["key"]["kty"], "oct");
        assert_eq!(serialized["key"]["alg"], "A256CTR");
        assert_eq!(serialized["key"]["ext"], true);
        assert_eq!(
            serialized["hashes"]["sha256"],
            "aWOHudBnDkJ9IwaR1Nd8XKoI7DOrqDTwt6xDPfVGN6Q"
        );
    }

    #[test]
    fn encrypted_file_rejects_wrong_key_length() {
        let mut json = encrypted_file_json();
        json["key"]["k"] = "AA".into();

        from_json_value::<EncryptedFile>(json).unwrap_err();
    }

    #[test]
    fn encrypted_file_rejects_wrong_hash_length() {
        let mut json = encrypted_file_json();
        json["hashes"]["sha256"] = "AA".into();

        from_json_value::<EncryptedFile>(json).unwrap_err();
    }
}

// #[cfg(test)]
// mod tests {
//     use std::collections::BTreeMap;

//     use crate::{mxc_uri, serde::Base64};
//     use assert_matches2::assert_matches;
//     use serde::Deserialize;
//     use serde_json::{from_value as from_json_value, json};

//     use super::{EncryptedFile, JsonWebKey, MediaSource};

//     #[derive(Deserialize)]
//     struct MsgWithAttachment {
//         #[allow(dead_code)]
//         body: String,
//         #[serde(flatten)]
//         source: MediaSource,
//     }

//     fn dummy_jwt() -> JsonWebKey {
//         JsonWebKey {
//             kty: "oct".to_owned(),
//             key_ops: vec!["encrypt".to_owned(), "decrypt".to_owned()],
//             alg: "A256CTR".to_owned(),
//             k: Base64::new(vec![0; 64]),
//             ext: true,
//         }
//     }

//     fn encrypted_file() -> EncryptedFile {
//         EncryptedFile {
//             url: mxc_uri!("mxc://localhost/encryptedfile").to_owned(),
//             key: dummy_jwt(),
//             iv: Base64::new(vec![0; 64]),
//             hashes: BTreeMap::new(),
//             v: "v2".to_owned(),
//         }
//     }

//     #[test]
//     fn prefer_encrypted_attachment_over_plain() {
//         let msg: MsgWithAttachment = from_json_value(json!({
//             "body": "",
//             "url": "mxc://localhost/file",
//             "file": encrypted_file(),
//         }))
//         .unwrap();

//         assert_matches!(msg.source, MediaSource::Encrypted(_));

//         // As above, but with the file field before the url field
//         let msg: MsgWithAttachment = from_json_value(json!({
//             "body": "",
//             "file": encrypted_file(),
//             "url": "mxc://localhost/file",
//         }))
//         .unwrap();

//         assert_matches!(msg.source, MediaSource::Encrypted(_));
//     }
// }
