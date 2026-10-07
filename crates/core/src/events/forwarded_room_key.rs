//! Types for the [`m.forwarded_room_key`] event.
//!
//! [`m.forwarded_room_key`]: https://spec.matrix.org/latest/client-server-api/#mforwarded_room_key

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;
use crate::{EventEncryptionAlgorithm, OwnedRoomId};

/// The content of an `m.forwarded_room_key` event.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.forwarded_room_key", kind = ToDevice)]
pub struct ToDeviceForwardedRoomKeyEventContent {
    /// The encryption algorithm the key in this event is to be used with.
    pub algorithm: EventEncryptionAlgorithm,

    /// The room where the key is used.
    pub room_id: OwnedRoomId,

    /// The Curve25519 key of the device which initiated the session originally.
    pub sender_key: String,

    /// The ID of the session that the key is for.
    pub session_id: String,

    /// The key to be exchanged.
    pub session_key: String,

    /// The Ed25519 key of the device which initiated the session originally.
    ///
    /// It is "claimed" because the receiving device has no way to tell that the
    /// original room_key actually came from a device which owns the private
    /// part of this key unless they have done device verification.
    pub sender_claimed_ed25519_key: String,

    /// Chain of Curve25519 keys.
    ///
    /// It starts out empty, but each time the key is forwarded to another
    /// device, the previous sender in the chain is added to the end of the
    /// list. For example, if the key is forwarded from A to B to C, this
    /// field is empty between A and B, and contains A's Curve25519 key
    /// between B and C.
    pub forwarding_curve25519_key_chain: Vec<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ToDeviceForwardedRoomKeyEventContent;

    #[test]
    fn forwarded_room_key_does_not_emit_non_spec_history_flags() {
        let expected = json!({
            "algorithm": "m.megolm.v1.aes-sha2",
            "room_id": "!room:example.org",
            "sender_key": "sender",
            "session_id": "session",
            "session_key": "key",
            "sender_claimed_ed25519_key": "signing-key",
            "forwarding_curve25519_key_chain": []
        });
        let mut legacy = expected.clone();
        legacy["org.matrix.msc3061.shared_history"] = true.into();
        let content: ToDeviceForwardedRoomKeyEventContent = serde_json::from_value(legacy).unwrap();
        assert_eq!(serde_json::to_value(content).unwrap(), expected);
    }
}
