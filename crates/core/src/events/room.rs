//! Modules for events in the `m.room` namespace.
//!
//! This module also contains types shared by events in its child namespaces.

use std::collections::BTreeMap;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize, de};

use crate::OwnedMxcUri;
use crate::serde::Base64;
use crate::serde::base64::UrlSafe;

pub mod aliases;
pub mod avatar;
pub mod canonical_alias;
pub mod create;
pub mod encrypted;
pub mod encryption;
pub mod guest_access;
pub mod history_visibility;
pub mod join_rule;
pub mod member;
pub mod message;
pub mod name;
pub mod pinned_events;
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
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct EncryptedFile {
    /// The URL to the file.
    pub url: OwnedMxcUri,

    /// A [JSON Web Key](https://tools.ietf.org/html/rfc7517#appendix-A.3) object.
    pub key: JsonWebKey,

    /// The 128-bit unique counter block used by AES-CTR, encoded as unpadded
    /// base64.
    pub iv: Base64,

    /// A map from an algorithm name to a hash of the ciphertext, encoded as
    /// unpadded base64.
    ///
    /// Clients should support the SHA-256 hash, which uses the key sha256.
    pub hashes: BTreeMap<String, Base64>,

    /// Version of the encrypted attachments protocol.
    ///
    /// Must be `v2`.
    pub v: String,
}

/// A [JSON Web Key](https://tools.ietf.org/html/rfc7517#appendix-A.3) object.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct JsonWebKey {
    /// Key type.
    ///
    /// Must be `oct`.
    pub kty: String,

    /// Key operations.
    ///
    /// Must at least contain `encrypt` and `decrypt`.
    pub key_ops: Vec<String>,

    /// Algorithm.
    ///
    /// Must be `A256CTR`.
    pub alg: String,

    /// The key, encoded as url-safe unpadded base64.
    pub k: Base64<UrlSafe>,

    /// Extractable.
    ///
    /// Must be `true`. This is a
    /// [W3C extension](https://w3c.github.io/webcrypto/#iana-section-jwk).
    pub ext: bool,
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
