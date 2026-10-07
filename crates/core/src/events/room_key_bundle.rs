//! Types for the `m.room_key_bundle` event defined in [MSC4268].
//!
//! [MSC4268]: https://github.com/matrix-org/matrix-spec-proposals/pull/4268
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::OwnedRoomId;
use crate::events::room::EncryptedFile;
use crate::macros::EventContent;

/// The content of an `m.room_key_bundle` event.
///
/// Typically encrypted as an `m.room.encrypted` event, then sent as a to-device event.
///
/// This event is defined in [MSC4268](https://github.com/matrix-org/matrix-spec-proposals/pull/4268)
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize, EventContent)]
#[palpo_event(type = "m.room_key_bundle", alias = "io.element.msc4268.room_key_bundle", kind = ToDevice)]
pub struct ToDeviceRoomKeyBundleEventContent {
    /// The room that these keys are for.
    pub room_id: OwnedRoomId,

    /// The location and encryption info of the key bundle.
    pub file: EncryptedFile,
}

impl ToDeviceRoomKeyBundleEventContent {
    /// Creates a new `ToDeviceRoomKeyBundleEventContent` with the given room ID, and
    /// [`EncryptedFile`] which contains the room keys from the bundle.
    pub fn new(room_id: OwnedRoomId, file: EncryptedFile) -> Self {
        Self { room_id, file }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ToDeviceRoomKeyBundleEventContent;
    use crate::events::room::{EncryptedFile, EncryptedFileHash, V2EncryptedFileInfo};
    use crate::serde::Base64;
    use crate::{owned_mxc_uri, owned_room_id};

    #[test]
    fn serialization() {
        let content = ToDeviceRoomKeyBundleEventContent {
            room_id: owned_room_id!("!testroomid:example.org"),
            file: EncryptedFile::new(
                owned_mxc_uri!("mxc://example.org/FHyPlCeYUSFFxlgbQYZmoEoe"),
                V2EncryptedFileInfo::new(
                    Base64::parse("aWF6-32KGYaC3A_FEUCk1Bt0JA37zP0wrStgmdCaW-0").unwrap(),
                    Base64::parse("w+sE15fzSc0AAAAAAAAAAA").unwrap(),
                )
                .into(),
                std::iter::once(EncryptedFileHash::Sha256(
                    Base64::parse("fdSLu/YkRx3Wyh3KQabP3rd6+SFiKg5lsJZQHtkSAYA").unwrap(),
                ))
                .collect(),
            ),
        };

        let serialized = serde_json::to_value(content).unwrap();

        assert_eq!(
            serialized,
            json!({
                "room_id": "!testroomid:example.org",
                "file": {
                    "v": "v2",
                    "url": "mxc://example.org/FHyPlCeYUSFFxlgbQYZmoEoe",
                    "key": {
                        "alg": "A256CTR",
                        "ext": true,
                        "k": "aWF6-32KGYaC3A_FEUCk1Bt0JA37zP0wrStgmdCaW-0",
                        "key_ops": ["decrypt","encrypt"],
                        "kty": "oct"
                    },
                    "iv": "w+sE15fzSc0AAAAAAAAAAA",
                    "hashes": {
                        "sha256": "fdSLu/YkRx3Wyh3KQabP3rd6+SFiKg5lsJZQHtkSAYA"
                    }
                }
            }),
            "The serialized value should match the declared JSON Value"
        );
    }

    #[test]
    fn bundle_event_uses_stable_type_for_stable_and_legacy_input() {
        use crate::events::AnyToDeviceEvent;

        for event_type in ["m.room_key_bundle", "io.element.msc4268.room_key_bundle"] {
            let json = json!({
                "type": event_type,
                "sender": "@alice:example.org",
                "content": {
                    "room_id": "!room:example.org",
                    "file": {
                        "v": "v2", "url": "mxc://example.org/key-bundle",
                        "key": { "alg": "A256CTR", "ext": true, "k": "aWF6-32KGYaC3A_FEUCk1Bt0JA37zP0wrStgmdCaW-0", "key_ops": ["decrypt", "encrypt"], "kty": "oct" },
                        "iv": "w+sE15fzSc0AAAAAAAAAAA", "hashes": { "sha256": "fdSLu/YkRx3Wyh3KQabP3rd6+SFiKg5lsJZQHtkSAYA" }
                    }
                }
            });
            let event: AnyToDeviceEvent = serde_json::from_value(json.clone()).unwrap();
            let AnyToDeviceEvent::RoomKeyBundle(event) = event else {
                panic!("room-key bundle must deserialize as the known event type");
            };
            let serialized = serde_json::to_value(event).unwrap();
            assert_eq!(serialized["type"], "m.room_key_bundle");
            assert_eq!(serialized["content"], json["content"]);
        }
    }
}
