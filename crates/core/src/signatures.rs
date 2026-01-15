//! Digital signatures according to the [Matrix](https://matrix.org/) specification.
//!
//! Digital signatures are used by Matrix homeservers to verify the authenticity
//! of events in the Matrix system, as well as requests between homeservers for
//! federation. Each homeserver has one or more signing key pairs (sometimes
//! referred to as "verify keys") which it uses to sign all events and
//! federation requests. Matrix clients and other Matrix homeservers can ask the
//! homeserver for its public keys and use those keys to verify the signed data.
//!
//! Each signing key pair has an identifier, which consists of the name of the
//! digital signature algorithm it uses and a "version" string, separated by a
//! colon. The version is an arbitrary identifier used to distinguish key pairs
//! using the same algorithm from the same homeserver.
//!
//! Arbitrary JSON objects can be signed as well as JSON representations of
//! Matrix events. In both cases, the signatures are stored within the JSON
//! object itself under a `signatures` key. Events are also required to contain
//! hashes of their content, which are similarly stored within the hashed JSON
//! object under a `hashes` key.
//!
//! In JSON representations, both signatures and hashes appear as base64-encoded
//! strings, using the standard character set, without padding.
//!
//! # Signing and hashing
//!
//! To sign an arbitrary JSON object, use the `sign_json` function. See the
//! documentation of this function for more details and a full example of use.
//!
//! Signing an event uses a more complicated process than signing arbitrary
//! JSON, because events can be redacted, and signatures need to remain valid
//! even if data is removed from an event later. HomeServers are required to
//! generate hashes of event contents as well as signing events before
//! exchanging them with other homeservers. Although the algorithm for hashing
//! and signing an event is more complicated than for signing arbitrary JSON,
//! the interface to a user of palpo-signatures is the same. To hash and sign an
//! event, use the `hash_and_sign_event` function. See the documentation of this
//! function for more details and a full example of use.
//!
//! # Verifying signatures and hashes
//!
//! When a homeserver receives data from another homeserver via the federation,
//! it's necessary to verify the authenticity and integrity of the data by
//! verifying their signatures.
//!
//! To verify a signature on arbitrary JSON, use the `verify_json` function. To
//! verify the signatures and hashes on an event, use the `verify_event`
//! function. See the documentation for these respective functions for more
//! details and full examples of use.
pub use self::error::{Error, JsonError, ParseError, VerificationError};
pub use self::functions::{
    canonical_json, content_hash, hash_and_sign_event, reference_hash, required_keys, sign_json,
    verify_canonical_json_bytes, verify_event, verify_json,
};
pub use self::keys::{Ed25519KeyPair, KeyPair, PublicKeyMap, PublicKeySet};
pub use self::verification::Verified;
use crate::serde::Base64;
use crate::serde::base64::Standard;
use crate::{AnyKeyName, IdParseError, OwnedSigningKeyId, SigningKeyAlgorithm, SigningKeyId};

mod error;
mod functions;
mod keys;
mod verification;

/// A digital signature.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Signature {
    /// The ID of the key used to generate this signature.
    pub(crate) key_id: OwnedSigningKeyId<AnyKeyName>,

    /// The signature data.
    pub(crate) signature: Vec<u8>,
}

impl Signature {
    /// Creates a signature from raw bytes.
    ///
    /// This constructor will ensure that the key ID has the correct `algorithm:key_name` format.
    ///
    /// # Parameters
    ///
    /// * `id`: A key identifier, e.g. `ed25519:1`.
    /// * `bytes`: The digital signature, as a series of bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * The key ID is malformed.
    pub fn new(id: &str, bytes: &[u8]) -> Result<Self, IdParseError> {
        let key_id = SigningKeyId::<AnyKeyName>::parse(id)?;

        Ok(Self {
            key_id,
            signature: bytes.to_vec(),
        })
    }

    /// The algorithm used to generate the signature.
    pub fn algorithm(&self) -> SigningKeyAlgorithm {
        self.key_id.algorithm()
    }

    /// The raw bytes of the signature.
    pub fn as_bytes(&self) -> &[u8] {
        self.signature.as_slice()
    }

    /// A base64 encoding of the signature.
    ///
    /// Uses the standard character set with no padding.
    pub fn base64(&self) -> String {
        Base64::<Standard, _>::new(self.signature.as_slice()).encode()
    }

    /// The key identifier, a string containing the signature algorithm and the key "version"
    /// separated by a colon, e.g. `ed25519:1`.
    pub fn id(&self) -> String {
        self.key_id.to_string()
    }

    /// The "version" of the key used for this signature.
    ///
    /// Versions are used as an identifier to distinguish signatures generated from different keys
    /// but using the same algorithm on the same homeserver.
    pub fn version(&self) -> &str {
        self.key_id.key_name().as_ref()
    }

    /// Split this `Signature` into its key identifier and bytes.
    pub fn into_parts(self) -> (OwnedSigningKeyId<AnyKeyName>, Vec<u8>) {
        (self.key_id, self.signature)
    }
}

#[cfg(test)]
mod tests {
    use super::Signature;
    use crate::SigningKeyAlgorithm;

    #[test]
    fn valid_key_id() {
        let signature = Signature::new("ed25519:abcdef", &[]).unwrap();
        assert_eq!(signature.algorithm(), SigningKeyAlgorithm::Ed25519);
        assert_eq!(signature.version(), "abcdef");
    }

    #[test]
    fn unknown_key_id_algorithm() {
        let signature = Signature::new("foobar:abcdef", &[]).unwrap();
        assert_eq!(signature.algorithm().as_str(), "foobar");
        assert_eq!(signature.version(), "abcdef");
    }

    #[test]
    fn invalid_key_id_format() {
        Signature::new("ed25519", &[]).unwrap_err();
    }
}
