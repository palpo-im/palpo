//! Key algorithms used in Matrix spec.

use crate::macros::StringEnum;
use salvo::prelude::*;

use crate::PrivOwnedStr;

/// The basic key algorithms in the specification.
#[derive(ToSchema, Clone, PartialEq, Eq, PartialOrd, Ord, StringEnum)]
#[non_exhaustive]
#[palpo_enum(rename_all = "snake_case")]
pub enum DeviceKeyAlgorithm {
    /// The Ed25519 signature algorithm.
    Ed25519,

    /// The Curve25519 ECDH algorithm.
    Curve25519,

    /// The Curve25519 ECDH algorithm, but the key also contains signatures
    SignedCurve25519,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

/// The signing key algorithms defined in the Matrix spec.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, StringEnum)]
#[non_exhaustive]
#[palpo_enum(rename_all = "snake_case")]
pub enum SigningKeyAlgorithm {
    /// The Ed25519 signature algorithm.
    Ed25519,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// An encryption algorithm to be used to encrypt messages sent to a room.
#[derive(ToSchema, Clone, PartialEq, Eq, PartialOrd, Ord, StringEnum)]
#[non_exhaustive]
pub enum EventEncryptionAlgorithm {
    /// Olm version 1 using Curve25519, AES-256, and SHA-256.
    #[palpo_enum(rename = "m.olm.v1.curve25519-aes-sha2")]
    OlmV1Curve25519AesSha2,

    /// Megolm version 1 using AES-256 and SHA-256.
    #[palpo_enum(rename = "m.megolm.v1.aes-sha2")]
    MegolmV1AesSha2,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// A key algorithm to be used to generate a key from a passphrase.
#[derive(ToSchema, Clone, PartialEq, Eq, PartialOrd, Ord, StringEnum)]
#[non_exhaustive]
pub enum KeyDerivationAlgorithm {
    /// PBKDF2
    #[palpo_enum(rename = "m.pbkdf2")]
    Pbkfd2,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

#[cfg(test)]
mod tests {
    use super::{DeviceKeyAlgorithm, SigningKeyAlgorithm};

    #[test]
    fn parse_device_key_algorithm() {
        assert_eq!(
            DeviceKeyAlgorithm::from("ed25519"),
            DeviceKeyAlgorithm::Ed25519
        );
        assert_eq!(
            DeviceKeyAlgorithm::from("curve25519"),
            DeviceKeyAlgorithm::Curve25519
        );
        assert_eq!(
            DeviceKeyAlgorithm::from("signed_curve25519"),
            DeviceKeyAlgorithm::SignedCurve25519
        );
    }

    #[test]
    fn parse_signing_key_algorithm() {
        assert_eq!(
            SigningKeyAlgorithm::from("ed25519"),
            SigningKeyAlgorithm::Ed25519
        );
    }

    #[test]
    fn event_encryption_algorithm_serde() {
        use serde_json::json;

        use super::EventEncryptionAlgorithm;
        use crate::serde::test::serde_json_eq;

        serde_json_eq(
            EventEncryptionAlgorithm::MegolmV1AesSha2,
            json!("m.megolm.v1.aes-sha2"),
        );
        serde_json_eq(
            EventEncryptionAlgorithm::OlmV1Curve25519AesSha2,
            json!("m.olm.v1.curve25519-aes-sha2"),
        );
        serde_json_eq(
            EventEncryptionAlgorithm::from("io.palpo.test"),
            json!("io.palpo.test"),
        );
    }

    #[test]
    fn key_derivation_algorithm_serde() {
        use serde_json::json;

        use super::KeyDerivationAlgorithm;
        use crate::serde::test::serde_json_eq;

        serde_json_eq(KeyDerivationAlgorithm::Pbkfd2, json!("m.pbkdf2"));
    }
}
