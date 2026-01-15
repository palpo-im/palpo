use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use crate::config;
use crate::core::UnixMillis;
use crate::core::federation::discovery::{OldVerifyKey, ServerSigningKeys, VerifyKey};
use crate::core::serde::Base64;

/// Similar to ServerSigningKeys, but drops a few unnecessary fields we don't require
/// post-validation
#[derive(Deserialize, Debug, Clone)]
pub struct SigningKeys {
    pub verify_keys: BTreeMap<String, VerifyKey>,
    pub old_verify_keys: BTreeMap<String, OldVerifyKey>,
    pub valid_until_ts: UnixMillis,
}

impl SigningKeys {
    /// Creates the SigningKeys struct, using the keys of the current server
    pub fn load_own_keys() -> Self {
        let mut keys = Self {
            verify_keys: BTreeMap::new(),
            old_verify_keys: BTreeMap::new(),
            valid_until_ts: UnixMillis::from_system_time(
                SystemTime::now() + Duration::from_secs(7 * 86400),
            )
            .expect("Should be valid until year 500,000,000"),
        };

        keys.verify_keys.insert(
            format!("ed25519:{}", config::keypair().version()),
            VerifyKey {
                key: Base64::new(config::keypair().public_key().to_vec()),
            },
        );

        keys
    }
}

impl From<ServerSigningKeys> for SigningKeys {
    fn from(value: ServerSigningKeys) -> Self {
        let ServerSigningKeys {
            verify_keys,
            old_verify_keys,
            valid_until_ts,
            ..
        } = value;

        Self {
            verify_keys: verify_keys
                .into_iter()
                .map(|(id, key)| (id.to_string(), key))
                .collect(),
            old_verify_keys: old_verify_keys
                .into_iter()
                .map(|(id, key)| (id.to_string(), key))
                .collect(),
            valid_until_ts,
        }
    }
}
