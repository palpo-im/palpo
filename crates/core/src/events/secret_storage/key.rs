//! Types for the [`m.secret_storage.key.*`] event.
//!
//! [`m.secret_storage.key.*`]: https://spec.matrix.org/latest/client-server-api/#key-storage

use std::borrow::Cow;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize, de};
use serde_json::Value as JsonValue;

use crate::{
    KeyDerivationAlgorithm,
    macros::EventContent,
    serde::{Base64, JsonObject},
};

/// A passphrase from which a key is to be derived.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct PassPhrase {
    /// The algorithm to use to generate the key from the passphrase.
    ///
    /// Must be `m.pbkdf2`.
    pub algorithm: KeyDerivationAlgorithm,

    /// The salt used in PBKDF2.
    pub salt: String,

    /// The number of iterations to use in PBKDF2.
    pub iterations: u64,

    /// The number of bits to generate for the key.
    ///
    /// Defaults to 256
    #[serde(default = "default_bits", skip_serializing_if = "is_default_bits")]
    pub bits: u64,
}

impl PassPhrase {
    /// Creates a new `PassPhrase` with a given salt and number of iterations.
    pub fn new(salt: String, iterations: u64) -> Self {
        Self {
            algorithm: KeyDerivationAlgorithm::Pbkfd2,
            salt,
            iterations,
            bits: default_bits(),
        }
    }
}

fn default_bits() -> u64 {
    256
}

fn is_default_bits(val: &u64) -> bool {
    *val == default_bits()
}

/// A key description encrypted using a specified algorithm.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, EventContent)]
#[palpo_event(type = "m.secret_storage.key.*", kind = GlobalAccountData)]
pub struct SecretStorageKeyEventContent {
    /// The ID of the key.
    #[palpo_event(type_fragment)]
    #[serde(skip)]
    pub key_id: String,

    /// The name of the key.
    pub name: Option<String>,

    /// The encryption algorithm used for this key.
    ///
    /// Currently, only `m.secret_storage.v1.aes-hmac-sha2` is supported.
    #[serde(flatten)]
    pub algorithm: SecretStorageEncryptionAlgorithm,

    /// The passphrase from which to generate the key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<PassPhrase>,
}

impl SecretStorageKeyEventContent {
    /// Creates a `KeyDescription` with the given name.
    pub fn new(key_id: String, algorithm: SecretStorageEncryptionAlgorithm) -> Self {
        Self {
            key_id,
            name: None,
            algorithm,
            passphrase: None,
        }
    }
}

/// An algorithm and its properties, used to encrypt a secret.
#[derive(ToSchema, Debug, Clone)]
pub enum SecretStorageEncryptionAlgorithm {
    /// Encrypted using the `m.secret_storage.v1.aes-hmac-sha2` algorithm.
    ///
    /// Secrets using this method are encrypted using AES-CTR-256 and
    /// authenticated using HMAC-SHA-256.
    V1AesHmacSha2(SecretStorageV1AesHmacSha2Properties),

    /// Encrypted using a custom algorithm.
    #[doc(hidden)]
    _Custom(CustomSecretEncryptionAlgorithm),
}

impl SecretStorageEncryptionAlgorithm {
    /// The `algorithm` string.
    pub fn algorithm(&self) -> &str {
        match self {
            Self::V1AesHmacSha2(_) => "m.secret_storage.v1.aes-hmac-sha2",
            Self::_Custom(c) => &c.algorithm,
        }
    }

    /// The algorithm-specific properties.
    ///
    /// The returned JSON object won't contain the `algorithm` field, use
    /// [`Self::algorithm()`] to access it.
    ///
    /// Prefer to use the public variants of `SecretStorageEncryptionAlgorithm`
    /// where possible; this method is meant to be used for custom
    /// algorithms only.
    pub fn properties(&self) -> Cow<'_, JsonObject> {
        fn serialize<T: Serialize>(obj: &T) -> JsonObject {
            match serde_json::to_value(obj).expect("secret properties serialization to succeed") {
                JsonValue::Object(obj) => obj,
                _ => panic!("all secret properties must serialize to objects"),
            }
        }

        match self {
            Self::V1AesHmacSha2(p) => Cow::Owned(serialize(p)),
            Self::_Custom(c) => Cow::Borrowed(&c.properties),
        }
    }
}

/// The key properties for the `m.secret_storage.v1.aes-hmac-sha2` algorithm.
#[derive(ToSchema, Debug, Clone, Deserialize, Serialize)]
pub struct SecretStorageV1AesHmacSha2Properties {
    /// The 16-byte initialization vector, encoded as base64.
    pub iv: Base64,

    /// The MAC, encoded as base64.
    pub mac: Base64,
}

impl SecretStorageV1AesHmacSha2Properties {
    /// Creates a new `SecretStorageV1AesHmacSha2Properties` with the given
    /// initialization vector and MAC.
    pub fn new(iv: Base64, mac: Base64) -> Self {
        Self { iv, mac }
    }
}

/// The payload for a custom secret encryption algorithm.
#[doc(hidden)]
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct CustomSecretEncryptionAlgorithm {
    /// The encryption algorithm to be used for the key.
    algorithm: String,

    /// Algorithm-specific properties.
    #[serde(flatten)]
    properties: JsonObject,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SecretStorageEncryptionAlgorithmDeHelper {
    Known(KnownSecretStorageEncryptionAlgorithmDeHelper),
    Unknown(CustomSecretEncryptionAlgorithm),
}

#[derive(Deserialize)]
#[serde(tag = "algorithm")]
enum KnownSecretStorageEncryptionAlgorithmDeHelper {
    #[serde(rename = "m.secret_storage.v1.aes-hmac-sha2")]
    V1AesHmacSha2(SecretStorageV1AesHmacSha2Properties),
}

impl<'de> Deserialize<'de> for SecretStorageEncryptionAlgorithm {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let helper = SecretStorageEncryptionAlgorithmDeHelper::deserialize(deserializer)?;

        Ok(match helper {
            SecretStorageEncryptionAlgorithmDeHelper::Known(k) => match k {
                KnownSecretStorageEncryptionAlgorithmDeHelper::V1AesHmacSha2(p) => {
                    Self::V1AesHmacSha2(p)
                }
            },
            SecretStorageEncryptionAlgorithmDeHelper::Unknown(c) => Self::_Custom(c),
        })
    }
}

#[derive(Debug, Serialize)]
struct SecretStorageEncryptionAlgorithmSerHelper<'a, T: Serialize> {
    algorithm: &'a str,
    #[serde(flatten)]
    properties: T,
}

impl Serialize for SecretStorageEncryptionAlgorithm {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let algorithm = self.algorithm();
        match self {
            Self::V1AesHmacSha2(properties) => SecretStorageEncryptionAlgorithmSerHelper {
                algorithm,
                properties,
            }
            .serialize(serializer),
            Self::_Custom(properties) => SecretStorageEncryptionAlgorithmSerHelper {
                algorithm,
                properties,
            }
            .serialize(serializer),
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::{serde::Base64, KeyDerivationAlgorithm};
//     use assert_matches2::assert_matches;
//     use serde_json::{
//         from_value as from_json_value, json, to_value as to_json_value,
// value::to_raw_value as to_raw_json_value,     };

//     use super::{
//         PassPhrase, SecretStorageEncryptionAlgorithm,
// SecretStorageKeyEventContent,         SecretStorageV1AesHmacSha2Properties,
//     };
//     use crate::events::{EventContentFromType, GlobalAccountDataEvent};

//     #[test]
//     fn key_description_serialization() {
//         let mut content = SecretStorageKeyEventContent::new(
//             "my_key".into(),
//
// SecretStorageEncryptionAlgorithm::V1AesHmacSha2(SecretStorageV1AesHmacSha2Properties
// {                 iv: Base64::parse("YWJjZGVmZ2hpamtsbW5vcA").unwrap(),
//                 mac:
// Base64::parse("aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U").unwrap(),
// }),         );
//         content.name = Some("my_key".to_owned());

//         let json = json!({
//             "name": "my_key",
//             "algorithm": "m.secret_storage.v1.aes-hmac-sha2",
//             "iv": "YWJjZGVmZ2hpamtsbW5vcA",
//             "mac": "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U"
//         });

//         assert_eq!(to_json_value(&content).unwrap(), json);
//     }

//     #[test]
//     fn key_description_deserialization() {
//         let json = to_raw_json_value(&json!({
//             "name": "my_key",
//             "algorithm": "m.secret_storage.v1.aes-hmac-sha2",
//             "iv": "YWJjZGVmZ2hpamtsbW5vcA",
//             "mac": "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U"
//         }))
//         .unwrap();

//         let content =
// SecretStorageKeyEventContent::from_parts("m.secret_storage.key.test",
// &json).unwrap();         assert_eq!(content.name.unwrap(), "my_key");
//         assert_matches!(content.passphrase, None);

//         assert_matches!(
//             content.algorithm,
//
// SecretStorageEncryptionAlgorithm::V1AesHmacSha2(SecretStorageV1AesHmacSha2Properties
// { iv, mac })         );
//         assert_eq!(iv.encode(), "YWJjZGVmZ2hpamtsbW5vcA");
//         assert_eq!(mac.encode(), "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U");
//     }

//     #[test]
//     fn key_description_deserialization_without_name() {
//         let json = to_raw_json_value(&json!({
//             "algorithm": "m.secret_storage.v1.aes-hmac-sha2",
//             "iv": "YWJjZGVmZ2hpamtsbW5vcA",
//             "mac": "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U"
//         }))
//         .unwrap();

//         let content =
// SecretStorageKeyEventContent::from_parts("m.secret_storage.key.test",
// &json).unwrap();         assert!(content.name.is_none());
//         assert_matches!(content.passphrase, None);

//         assert_matches!(
//             content.algorithm,
//
// SecretStorageEncryptionAlgorithm::V1AesHmacSha2(SecretStorageV1AesHmacSha2Properties
// { iv, mac })         );
//         assert_eq!(iv.encode(), "YWJjZGVmZ2hpamtsbW5vcA");
//         assert_eq!(mac.encode(), "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U");
//     }

//     #[test]
//     fn key_description_with_passphrase_serialization() {
//         let mut content = SecretStorageKeyEventContent {
//             passphrase: Some(PassPhrase::new("rocksalt".into(), u8)),
//             ..SecretStorageKeyEventContent::new(
//                 "my_key".into(),
//
// SecretStorageEncryptionAlgorithm::V1AesHmacSha2(SecretStorageV1AesHmacSha2Properties
// {                     iv: Base64::parse("YWJjZGVmZ2hpamtsbW5vcA").unwrap(),
//                     mac:
// Base64::parse("aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U").unwrap(),
// }),             )
//         };
//         content.name = Some("my_key".to_owned());

//         let json = json!({
//             "name": "my_key",
//             "algorithm": "m.secret_storage.v1.aes-hmac-sha2",
//             "iv": "YWJjZGVmZ2hpamtsbW5vcA",
//             "mac": "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U",
//             "passphrase": {
//                 "algorithm": "m.pbkdf2",
//                 "salt": "rocksalt",
//                 "iterations": 8
//             }
//         });

//         assert_eq!(to_json_value(&content).unwrap(), json);
//     }

//     #[test]
//     fn key_description_with_passphrase_deserialization() {
//         let json = to_raw_json_value(&json!({
//             "name": "my_key",
//             "algorithm": "m.secret_storage.v1.aes-hmac-sha2",
//             "iv": "YWJjZGVmZ2hpamtsbW5vcA",
//             "mac": "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U",
//             "passphrase": {
//                 "algorithm": "m.pbkdf2",
//                 "salt": "rocksalt",
//                 "iterations": 8,
//                 "bits": 256
//             }
//         }))
//         .unwrap();

//         let content =
// SecretStorageKeyEventContent::from_parts("m.secret_storage.key.test",
// &json).unwrap();         assert_eq!(content.name.unwrap(), "my_key");

//         let passphrase = content.passphrase.unwrap();
//         assert_eq!(passphrase.algorithm, KeyDerivationAlgorithm::Pbkfd2);
//         assert_eq!(passphrase.salt, "rocksalt");
//         assert_eq!(passphrase.iterations, u8);
//         assert_eq!(passphrase.bits, u256);

//         assert_matches!(
//             content.algorithm,
//
// SecretStorageEncryptionAlgorithm::V1AesHmacSha2(SecretStorageV1AesHmacSha2Properties
// { iv, mac })         );
//         assert_eq!(iv.encode(), "YWJjZGVmZ2hpamtsbW5vcA");
//         assert_eq!(mac.encode(), "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U");
//     }

//     #[test]
//     fn event_serialization() {
//         let mut content = SecretStorageKeyEventContent::new(
//             "my_key_id".into(),
//
// SecretStorageEncryptionAlgorithm::V1AesHmacSha2(SecretStorageV1AesHmacSha2Properties
// {                 iv: Base64::parse("YWJjZGVmZ2hpamtsbW5vcA").unwrap(),
//                 mac:
// Base64::parse("aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U").unwrap(),
// }),         );
//         content.name = Some("my_key".to_owned());

//         let json = json!({
//             "name": "my_key",
//             "algorithm": "m.secret_storage.v1.aes-hmac-sha2",
//             "iv": "YWJjZGVmZ2hpamtsbW5vcA",
//             "mac": "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U"
//         });

//         assert_eq!(to_json_value(&content).unwrap(), json);
//     }

//     #[test]
//     fn event_deserialization() {
//         let json = json!({
//             "type": "m.secret_storage.key.my_key_id",
//             "content": {
//                 "name": "my_key",
//                 "algorithm": "m.secret_storage.v1.aes-hmac-sha2",
//                 "iv": "YWJjZGVmZ2hpamtsbW5vcA",
//                 "mac": "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U"
//             }
//         });

//         let ev =
// from_json_value::<GlobalAccountDataEvent<SecretStorageKeyEventContent>>(json).
// unwrap();         assert_eq!(ev.content.key_id, "my_key_id");
//         assert_eq!(ev.content.name.unwrap(), "my_key");
//         assert_matches!(ev.content.passphrase, None);

//         assert_matches!(
//             ev.content.algorithm,
//
// SecretStorageEncryptionAlgorithm::V1AesHmacSha2(SecretStorageV1AesHmacSha2Properties
// { iv, mac })         );
//         assert_eq!(iv.encode(), "YWJjZGVmZ2hpamtsbW5vcA");
//         assert_eq!(mac.encode(), "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U");
//     }

//     #[test]
//     fn custom_algorithm_key_description_deserialization() {
//         let json = to_raw_json_value(&json!({
//             "name": "my_key",
//             "algorithm": "io.palpo.custom_alg",
//             "io.palpo.custom_prop1": "YWJjZGVmZ2hpamtsbW5vcA",
//             "io.palpo.custom_prop2": "aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U"
//         }))
//         .unwrap();

//         let content =
// SecretStorageKeyEventContent::from_parts("m.secret_storage.key.test",
// &json).unwrap();         assert_eq!(content.name.unwrap(), "my_key");
//         assert_matches!(content.passphrase, None);

//         let algorithm = content.algorithm;
//         assert_eq!(algorithm.algorithm(), "io.palpo.custom_alg");
//         let properties = algorithm.properties();
//         assert_eq!(properties.len(), 2);
//         assert_eq!(
//             properties.get("io.palpo.custom_prop1").unwrap().as_str(),
//             Some("YWJjZGVmZ2hpamtsbW5vcA")
//         );
//         assert_eq!(
//             properties.get("io.palpo.custom_prop2").unwrap().as_str(),
//             Some("aWRvbnRrbm93d2hhdGFtYWNsb29rc2xpa2U")
//         );
//     }
// }
