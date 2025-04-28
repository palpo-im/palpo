//! Types for the [`m.key.verification.cancel`] event.
//!
//! [`m.key.verification.cancel`]: https://spec.matrix.org/latest/client-server-api/#mkeyverificationcancel

use palpo_macros::EventContent;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::{OwnedTransactionId, PrivOwnedStr, events::relation::Reference, serde::StringEnum};

/// The content of a to-device `m.key.verification.cancel` event.
///
/// Cancels a key verification process/request.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.key.verification.cancel", kind = ToDevice)]
pub struct ToDeviceKeyVerificationCancelEventContent {
    /// The opaque identifier for the verification process/request.
    pub transaction_id: OwnedTransactionId,

    /// A human readable description of the `code`.
    ///
    /// The client should only rely on this string if it does not understand the
    /// `code`.
    pub reason: String,

    /// The error code for why the process / request was cancelled by the user.
    pub code: CancelCode,
}

impl ToDeviceKeyVerificationCancelEventContent {
    /// Creates a new `ToDeviceKeyVerificationCancelEventContent` with the given
    /// transaction ID, reason and code.
    pub fn new(transaction_id: OwnedTransactionId, reason: String, code: CancelCode) -> Self {
        Self {
            transaction_id,
            reason,
            code,
        }
    }
}

/// The content of an in-room `m.key.verification.cancel` event.
///
/// Cancels a key verification process/request.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.key.verification.cancel", kind = MessageLike)]
pub struct KeyVerificationCancelEventContent {
    /// A human readable description of the `code`.
    ///
    /// The client should only rely on this string if it does not understand the
    /// `code`.
    pub reason: String,

    /// The error code for why the process/request was cancelled by the user.
    pub code: CancelCode,

    /// Information about the related event.
    #[serde(rename = "m.relates_to")]
    pub relates_to: Reference,
}

impl KeyVerificationCancelEventContent {
    /// Creates a new `KeyVerificationCancelEventContent` with the given reason,
    /// code and reference.
    pub fn new(reason: String, code: CancelCode, relates_to: Reference) -> Self {
        Self {
            reason,
            code,
            relates_to,
        }
    }
}

/// An error code for why the process/request was cancelled by the user.
///
/// Custom error codes should use the Java package naming convention.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
// FIXME: Add `m.foo_bar` as a naming scheme in StringEnum and remove rename attributes.
#[derive(ToSchema, Clone, PartialEq, Eq, StringEnum)]
#[non_exhaustive]
pub enum CancelCode {
    /// The user cancelled the verification.
    #[palpo_enum(rename = "m.user")]
    User,

    /// The verification process timed out.
    ///
    /// Verification processes can define their own timeout parameters.
    #[palpo_enum(rename = "m.timeout")]
    Timeout,

    /// The device does not know about the given transaction ID.
    #[palpo_enum(rename = "m.unknown_transaction")]
    UnknownTransaction,

    /// The device does not know how to handle the requested method.
    ///
    /// Should be sent for `m.key.verification.start` messages and messages
    /// defined by individual verification processes.
    #[palpo_enum(rename = "m.unknown_method")]
    UnknownMethod,

    /// The device received an unexpected message.
    ///
    /// Typically raised when one of the parties is handling the verification
    /// out of order.
    #[palpo_enum(rename = "m.unexpected_message")]
    UnexpectedMessage,

    /// The key was not verified.
    #[palpo_enum(rename = "m.key_mismatch")]
    KeyMismatch,

    /// The expected user did not match the user verified.
    #[palpo_enum(rename = "m.user_mismatch")]
    UserMismatch,

    /// The message received was invalid.
    #[palpo_enum(rename = "m.invalid_message")]
    InvalidMessage,

    /// An `m.key.verification.request` was accepted by a different device.
    ///
    /// The device receiving this error can ignore the verification request.
    #[palpo_enum(rename = "m.accepted")]
    Accepted,

    /// The device receiving this error can ignore the verification request.
    #[palpo_enum(rename = "m.mismatched_commitment")]
    MismatchedCommitment,

    /// The SAS did not match.
    #[palpo_enum(rename = "m.mismatched_sas")]
    MismatchedSas,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::CancelCode;

    #[test]
    fn cancel_codes_serialize_to_display_form() {
        assert_eq!(to_json_value(&CancelCode::User).unwrap(), json!("m.user"));
    }

    #[test]
    fn custom_cancel_codes_serialize_to_display_form() {
        assert_eq!(
            to_json_value(CancelCode::from("io.palpo.test")).unwrap(),
            json!("io.palpo.test")
        );
    }

    #[test]
    fn cancel_codes_deserialize_from_display_form() {
        assert_eq!(
            from_json_value::<CancelCode>(json!("m.user")).unwrap(),
            CancelCode::User
        );
    }

    #[test]
    fn custom_cancel_codes_deserialize_from_display_form() {
        assert_eq!(
            from_json_value::<CancelCode>(json!("io.palpo.test")).unwrap(),
            "io.palpo.test".into()
        );
    }
}
