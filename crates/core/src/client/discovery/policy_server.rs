//! `GET /.well-known/matrix/policy_server`
//!
//! Discovery information for a policy server.

use std::collections::BTreeMap;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::SigningKeyAlgorithm;
use crate::serde::Base64;

/// Request type for the `policy_server` endpoint.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct PolicyServerReqBody {}

impl PolicyServerReqBody {
    /// Creates an empty `PolicyServerReqBody`.
    pub fn new() -> Self {
        Self {}
    }
}

/// Response type for the `policy_server` endpoint.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct PolicyServerResBody {
    /// Public signing keys for the policy server.
    ///
    /// The response must contain at least one `ed25519` key.
    pub public_keys: BTreeMap<SigningKeyAlgorithm, Base64>,
}

impl PolicyServerResBody {
    /// Creates a new `PolicyServerResBody` with the given Ed25519 public key.
    pub fn new(ed25519_public_key: Base64) -> Self {
        Self {
            public_keys: [(SigningKeyAlgorithm::Ed25519, ed25519_public_key)].into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::PolicyServerResBody;
    use crate::SigningKeyAlgorithm;
    use crate::serde::Base64;

    #[test]
    fn policy_server_response_serializes_ed25519_key() {
        let response = PolicyServerResBody::new(Base64::new(vec![1, 2, 3]));

        assert_eq!(
            to_json_value(&response).unwrap(),
            json!({
                "public_keys": {
                    "ed25519": "AQID"
                }
            })
        );
    }

    #[test]
    fn policy_server_response_deserializes_ed25519_key() {
        let response: PolicyServerResBody = from_json_value(json!({
            "public_keys": {
                "ed25519": "AQID"
            }
        }))
        .unwrap();

        let ed25519_key = response
            .public_keys
            .get(&SigningKeyAlgorithm::Ed25519)
            .expect("ed25519 key is present");
        assert_eq!(ed25519_key.as_bytes(), &[1, 2, 3]);
    }
}
