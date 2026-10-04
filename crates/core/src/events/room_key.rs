//! Types for the [`m.room_key`] event.
//!
//! [`m.room_key`]: https://spec.matrix.org/latest/client-server-api/#mroom_key

pub mod withheld;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;
use crate::{EventEncryptionAlgorithm, OwnedRoomId};

/// The content of an `m.room_key` event.
///
/// Typically encrypted as an `m.room.encrypted` event, then sent as a to-device
/// event.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.room_key", kind = ToDevice)]
pub struct ToDeviceRoomKeyEventContent {
    /// The encryption algorithm the key in this event is to be used with.
    ///
    /// Must be `m.megolm.v1.aes-sha2`.
    pub algorithm: EventEncryptionAlgorithm,

    /// The room where the key is used.
    pub room_id: OwnedRoomId,

    /// The ID of the session that the key is for.
    pub session_id: String,

    /// The key to be exchanged.
    pub session_key: String,

    /// Used to mark key if allowed for shared history.
    ///
    /// Defaults to `false`.
    #[serde(
        default,
        alias = "org.matrix.msc3061.shared_history",
        skip_serializing_if = "palpo_core::serde::is_default"
    )]
    pub shared_history: bool,
}

impl ToDeviceRoomKeyEventContent {
    /// Creates a new `ToDeviceRoomKeyEventContent` with the given algorithm,
    /// room ID, session ID and session key.
    pub fn new(
        algorithm: EventEncryptionAlgorithm,
        room_id: OwnedRoomId,
        session_id: String,
        session_key: String,
    ) -> Self {
        Self {
            algorithm,
            room_id,
            session_id,
            session_key,
            shared_history: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, to_value as to_json_value};

    use super::ToDeviceRoomKeyEventContent;
    use crate::{EventEncryptionAlgorithm, owned_room_id};

    #[test]
    fn serialization() {
        let content = ToDeviceRoomKeyEventContent {
            algorithm: EventEncryptionAlgorithm::MegolmV1AesSha2,
            room_id: owned_room_id!("!testroomid:example.org"),
            session_id: "SessId".into(),
            session_key: "SessKey".into(),
            shared_history: true,
        };

        assert_eq!(
            to_json_value(content).unwrap(),
            json!({
                "algorithm": "m.megolm.v1.aes-sha2",
                "room_id": "!testroomid:example.org",
                "session_id": "SessId",
                "session_key": "SessKey",
                "shared_history": true,
            })
        );
    }

    #[test]
    fn shared_history_accepts_stable_and_legacy_input() {
        for field in ["shared_history", "org.matrix.msc3061.shared_history"] {
            let mut json = json!({
                "algorithm": "m.megolm.v1.aes-sha2",
                "room_id": "!testroomid:example.org",
                "session_id": "SessId",
                "session_key": "SessKey"
            });
            json[field] = true.into();
            let content: ToDeviceRoomKeyEventContent = serde_json::from_value(json).unwrap();
            assert!(content.shared_history);
            let serialized = to_json_value(content).unwrap();
            assert_eq!(serialized["shared_history"], true);
            assert!(
                serialized
                    .get("org.matrix.msc3061.shared_history")
                    .is_none()
            );
        }
    }

    #[test]
    fn shared_history_defaults_to_false_and_is_omitted() {
        let json = json!({
            "algorithm": "m.megolm.v1.aes-sha2",
            "room_id": "!testroomid:example.org",
            "session_id": "SessId",
            "session_key": "SessKey"
        });
        let content: ToDeviceRoomKeyEventContent = serde_json::from_value(json.clone()).unwrap();
        assert!(!content.shared_history);
        assert_eq!(to_json_value(content).unwrap(), json);
    }
}
