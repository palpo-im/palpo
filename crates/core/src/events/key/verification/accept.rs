//! Types for the [`m.key.verification.accept`] event.
//!
//! [`m.key.verification.accept`]: https://spec.matrix.org/latest/client-server-api/#mkeyverificationaccept

use std::collections::BTreeMap;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use super::{
    HashAlgorithm, KeyAgreementProtocol, MessageAuthenticationCode, ShortAuthenticationString,
};
use crate::OwnedTransactionId;
use crate::events::relation::Reference;
use crate::macros::EventContent;
use crate::serde::Base64;

/// The content of a to-device `m.key.verification.accept` event.
///
/// Accepts a previously sent `m.key.verification.start` message.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.key.verification.accept", kind = ToDevice)]
pub struct ToDeviceKeyVerificationAcceptEventContent {
    /// An opaque identifier for the verification process.
    ///
    /// Must be the same as the one used for the `m.key.verification.start`
    /// message.
    pub transaction_id: OwnedTransactionId,

    /// The method specific content.
    #[serde(flatten)]
    pub method: AcceptMethod,
}

impl ToDeviceKeyVerificationAcceptEventContent {
    /// Creates a new `ToDeviceKeyVerificationAcceptEventContent` with the given
    /// transaction ID and method-specific content.
    pub fn new(transaction_id: OwnedTransactionId, method: AcceptMethod) -> Self {
        Self {
            transaction_id,
            method,
        }
    }
}

/// The content of an in-room `m.key.verification.accept` event.
///
/// Accepts a previously sent `m.key.verification.start` message.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.key.verification.accept", kind = MessageLike)]
pub struct KeyVerificationAcceptEventContent {
    /// The method specific content.
    #[serde(flatten)]
    pub method: AcceptMethod,

    /// Information about the related event.
    #[serde(rename = "m.relates_to")]
    pub relates_to: Reference,
}

impl KeyVerificationAcceptEventContent {
    /// Creates a new `KeyVerificationAcceptEventContent` with the given
    /// method-specific content and reference.
    pub fn new(method: AcceptMethod, relates_to: Reference) -> Self {
        Self { method, relates_to }
    }
}

/// An enum representing the different method-specific
/// `m.key.verification.accept` content.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum AcceptMethod {
    /// The `m.sas.v1` verification method.
    SasV1(SasV1Content),

    /// Any unknown accept method.
    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(_CustomContent),
}

/// Method-specific content of an unknown key verification method.
#[doc(hidden)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[allow(clippy::exhaustive_structs)]
pub struct _CustomContent {
    /// The fields supplied by the unknown method.
    #[serde(flatten)]
    pub data: BTreeMap<String, JsonValue>,
}

/// The payload of an `m.key.verification.accept` event using the `m.sas.v1`
/// method.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct SasV1Content {
    /// The key agreement protocol the device is choosing to use, out of the
    /// options in the `m.key.verification.start` message.
    pub key_agreement_protocol: KeyAgreementProtocol,

    /// The hash method the device is choosing to use, out of the options in the
    /// `m.key.verification.start` message.
    pub hash: HashAlgorithm,

    /// The message authentication code the device is choosing to use, out of
    /// the options in the `m.key.verification.start` message.
    pub message_authentication_code: MessageAuthenticationCode,

    /// The SAS methods both devices involved in the verification process
    /// understand.
    ///
    /// Must be a subset of the options in the `m.key.verification.start`
    /// message.
    pub short_authentication_string: Vec<ShortAuthenticationString>,

    /// The hash (encoded as unpadded base64) of the concatenation of the
    /// device's ephemeral public key (encoded as unpadded base64) and the
    /// canonical JSON representation of the `m.key.verification.start` message.
    pub commitment: Base64,
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::{
        AcceptMethod, KeyVerificationAcceptEventContent, ToDeviceKeyVerificationAcceptEventContent,
    };

    #[test]
    fn to_device_sas_v1_round_trips_without_method() {
        let json = json!({
            "transaction_id": "456",
            "commitment": "aGVsbG8",
            "key_agreement_protocol": "curve25519",
            "hash": "sha256",
            "message_authentication_code": "hkdf-hmac-sha256.v2",
            "short_authentication_string": ["decimal"]
        });

        let content: ToDeviceKeyVerificationAcceptEventContent =
            from_json_value(json.clone()).unwrap();

        assert!(matches!(&content.method, AcceptMethod::SasV1(_)));
        assert_eq!(to_json_value(content).unwrap(), json);
    }

    #[test]
    fn in_room_sas_v1_round_trips_without_method() {
        let json = json!({
            "commitment": "aGVsbG8",
            "key_agreement_protocol": "curve25519",
            "hash": "sha256",
            "message_authentication_code": "hkdf-hmac-sha256.v2",
            "short_authentication_string": ["decimal"],
            "m.relates_to": {
                "rel_type": "m.reference",
                "event_id": "$1598361704261elfgc:localhost"
            }
        });

        let content: KeyVerificationAcceptEventContent = from_json_value(json.clone()).unwrap();

        assert!(matches!(&content.method, AcceptMethod::SasV1(_)));
        assert_eq!(to_json_value(content).unwrap(), json);
    }

    #[test]
    fn unknown_accept_content_round_trips_without_method() {
        let json = json!({
            "transaction_id": "456",
            "com.example.custom": "field"
        });

        let content: ToDeviceKeyVerificationAcceptEventContent =
            from_json_value(json.clone()).unwrap();

        assert!(matches!(&content.method, AcceptMethod::_Custom(_)));
        assert_eq!(to_json_value(content).unwrap(), json);
    }
}
