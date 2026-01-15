//! Types for the [`m.key.verification.request`] event.
//!
//! [`m.key.verification.request`]: https://spec.matrix.org/latest/client-server-api/#mkeyverificationrequest

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use super::VerificationMethod;
use crate::macros::EventContent;
use crate::{OwnedDeviceId, OwnedTransactionId, UnixMillis};

/// The content of an `m.key.verification.request` event.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.key.verification.request", kind = ToDevice)]
pub struct ToDeviceKeyVerificationRequestEventContent {
    /// The device ID which is initiating the request.
    pub from_device: OwnedDeviceId,

    /// An opaque identifier for the verification request.
    ///
    /// Must be unique with respect to the devices involved.
    pub transaction_id: OwnedTransactionId,

    /// The verification methods supported by the sender.
    pub methods: Vec<VerificationMethod>,

    /// The time in milliseconds for when the request was made.
    ///
    /// If the request is in the future by more than 5 minutes or more than 10
    /// minutes in the past, the message should be ignored by the receiver.
    pub timestamp: UnixMillis,
}

impl ToDeviceKeyVerificationRequestEventContent {
    /// Creates a new `ToDeviceKeyVerificationRequestEventContent` with the
    /// given device ID, transaction ID, methods and timestamp.
    pub fn new(
        from_device: OwnedDeviceId,
        transaction_id: OwnedTransactionId,
        methods: Vec<VerificationMethod>,
        timestamp: UnixMillis,
    ) -> Self {
        Self {
            from_device,
            transaction_id,
            methods,
            timestamp,
        }
    }
}
