//! Modules for events in the `m.key.verification` namespace.
//!
//! This module also contains types shared by events in its child namespaces.
//!
//! The MSC for the in-room variants of the `m.key.verification.*` events can be
//! found on [MSC2241].
//!
//! [MSC2241]: https://github.com/matrix-org/matrix-spec-proposals/pull/2241

use std::time::Duration;

use salvo::oapi::ToSchema;

use crate::{PrivOwnedStr, serde::StringEnum};

pub mod accept;
pub mod cancel;
pub mod done;
pub mod key;
pub mod mac;
pub mod ready;
pub mod request;
pub mod start;

// For these two constants, see <https://spec.matrix.org/latest/client-server-api/#key-verification-framework>
/// The amount of time after which a verification request should be ignored,
/// relative to its `origin_server_ts` (for in-room events) or its `timestamp`
/// (for to-device events).
///
/// This is defined as 10 minutes.
pub const REQUEST_TIMESTAMP_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The amount of time after which a verification request should be ignored,
/// relative to the time it was received by the client.
///
/// This is defined as 2 minutes.
pub const REQUEST_RECEIVED_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// A hash algorithm.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, PartialEq, Eq, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HashAlgorithm {
    /// The SHA256 hash algorithm.
    Sha256,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// A key agreement protocol.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, PartialEq, Eq, StringEnum)]
#[palpo_enum(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum KeyAgreementProtocol {
    /// The [Curve25519](https://cr.yp.to/ecdh.html) key agreement protocol.
    Curve25519,

    /// The Curve25519 key agreement protocol with check for public keys.
    Curve25519HkdfSha256,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// A message authentication code algorithm.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, PartialEq, Eq, StringEnum)]
#[palpo_enum(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MessageAuthenticationCode {
    /// The second version of the HKDF-HMAC-SHA256 MAC.
    #[palpo_enum(rename = "hkdf-hmac-sha256.v2")]
    HkdfHmacSha256V2,

    /// The HMAC-SHA256 MAC.
    HmacSha256,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// A Short Authentication String method.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, PartialEq, Eq, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ShortAuthenticationString {
    /// The decimal method.
    Decimal,

    /// The emoji method.
    Emoji,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// A Short Authentication String (SAS) verification method.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, PartialEq, Eq, StringEnum)]
#[non_exhaustive]
pub enum VerificationMethod {
    /// The `m.sas.v1` verification method.
    #[palpo_enum(rename = "m.sas.v1")]
    SasV1,

    /// The `m.qr_code.scan.v1` verification method.
    #[palpo_enum(rename = "m.qr_code.scan.v1")]
    QrCodeScanV1,

    /// The `m.qr_code.show.v1` verification method.
    #[palpo_enum(rename = "m.qr_code.show.v1")]
    QrCodeShowV1,

    /// The `m.reciprocate.v1` verification method.
    #[palpo_enum(rename = "m.reciprocate.v1")]
    ReciprocateV1,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json};

    use super::{KeyAgreementProtocol, MessageAuthenticationCode};

    #[test]
    fn serialize_key_agreement() {
        let serialized =
            serde_json::to_string(&KeyAgreementProtocol::Curve25519HkdfSha256).unwrap();
        assert_eq!(serialized, "\"curve25519-hkdf-sha256\"");

        let deserialized: KeyAgreementProtocol = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, KeyAgreementProtocol::Curve25519HkdfSha256);
    }

    #[test]
    fn serialize_mac_method_v2() {
        let serialized =
            serde_json::to_string(&MessageAuthenticationCode::HkdfHmacSha256V2).unwrap();
        let deserialized: MessageAuthenticationCode = serde_json::from_str(&serialized).unwrap();

        assert_eq!(serialized, "\"hkdf-hmac-sha256.v2\"");
        assert_eq!(deserialized, MessageAuthenticationCode::HkdfHmacSha256V2);
    }
}
