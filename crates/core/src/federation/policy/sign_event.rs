//! `POST /_matrix/policy/v1/sign`
//!
//! Ask the Policy Server to sign an event.

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::serde::RawJsonValue;
use crate::{
    OwnedServerName, OwnedServerSigningKeyId, ServerName, ServerSignatures, ServerSigningKeyId,
};

/// Request body for the `sign_event` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
#[salvo(schema(value_type = Object))]
pub struct PolicySignEventReqBody(pub Box<RawJsonValue>);

impl PolicySignEventReqBody {
    /// Creates a new `PolicySignEventReqBody` with the given PDU.
    pub fn new(pdu: Box<RawJsonValue>) -> Self {
        Self(pdu)
    }
}

crate::json_body_modifier!(PolicySignEventReqBody);

/// Response body for the `sign_event` endpoint.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct PolicySignEventResBody {
    /// A map containing the Policy Server's signature of the event.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub signatures: ServerSignatures,
}

impl PolicySignEventResBody {
    /// The signing key ID that must be used by the Policy Server for the Ed25519 signature.
    pub const POLICY_SERVER_ED25519_SIGNING_KEY_ID: &str = "ed25519:policy_server";

    /// Creates a new `PolicySignEventResBody` with the given Policy Server name and signature.
    pub fn new(server_name: OwnedServerName, ed25519_signature: String) -> Self {
        let key_id: OwnedServerSigningKeyId = Self::POLICY_SERVER_ED25519_SIGNING_KEY_ID
            .try_into()
            .expect("Policy Server default ed25519 signing key ID should be valid");

        Self {
            signatures: ServerSignatures::from_iter([(server_name, key_id, ed25519_signature)]),
        }
    }

    /// Get the signature of the event for the given Policy Server name, if any.
    pub fn ed25519_signature(&self, server_name: &ServerName) -> Option<&str> {
        let key_id = <&ServerSigningKeyId>::try_from(Self::POLICY_SERVER_ED25519_SIGNING_KEY_ID)
            .expect("Policy Server default ed25519 signing key ID should be valid");

        self.signatures
            .get(server_name)?
            .get(key_id)
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::{PolicySignEventReqBody, PolicySignEventResBody};
    use crate::serde::to_raw_json_value;
    use crate::{owned_server_name, server_name};

    const SIGNATURE: &str =
        "zLFxllD0pbBuBpfHh8NuHNaICpReF/PAOpUQTsw+bFGKiGfDNAsnhcP7pbrmhhpfbOAxIdLraQLeeiXBryLmBw";

    #[test]
    fn request_body_serializes_as_raw_pdu() {
        let request = PolicySignEventReqBody::new(
            to_raw_json_value(&json!({
                "type": "m.room.message",
                "content": { "body": "hello" }
            }))
            .unwrap(),
        );

        assert_eq!(
            to_json_value(&request).unwrap(),
            json!({
                "type": "m.room.message",
                "content": { "body": "hello" }
            })
        );
    }

    #[test]
    fn response_serializes_as_signature_map() {
        let response =
            PolicySignEventResBody::new(owned_server_name!("policy.example.org"), SIGNATURE.into());

        assert_eq!(
            to_json_value(&response).unwrap(),
            json!({
                "policy.example.org": {
                    "ed25519:policy_server": SIGNATURE,
                },
            })
        );
    }

    #[test]
    fn response_deserializes_and_finds_ed25519_signature() {
        let response: PolicySignEventResBody = from_json_value(json!({
            "policy.example.org": {
                "ed25519:policy_server": SIGNATURE,
            },
        }))
        .unwrap();

        assert_eq!(
            response.ed25519_signature(server_name!("policy.example.org")),
            Some(SIGNATURE)
        );
    }
}
