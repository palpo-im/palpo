/// Endpoints for exchanging transaction messages between homeservers.
///
/// `PUT /_matrix/federation/*/send/{txn_id}`
///
/// Send live activity messages to another server.
/// `/v1/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/server-server-api/#put_matrixfederationv1sendtxnid
use std::collections::BTreeMap;

use reqwest::Url;
use salvo::prelude::*;
use serde::{Deserialize, Serialize, de};

use crate::device::{DeviceListUpdateContent, DirectDeviceContent};
use crate::encryption::CrossSigningKey;
use crate::events::receipt::{Receipt, ReceiptContent};
use crate::events::typing::TypingContent;
use crate::identifiers::*;
use crate::presence::PresenceContent;
use crate::sending::{SendRequest, SendResult};
use crate::serde::{JsonValue, RawJsonValue, from_raw_json_value};
use crate::{OwnedServerName, UnixMillis};

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/send/:transaction_id",
//     }
// };

pub fn send_message_request(
    origin: &str,
    txn_id: &str,
    body: SendMessageReqBody,
) -> SendResult<SendRequest> {
    let url = Url::parse(&format!("{origin}/_matrix/federation/v1/send/{txn_id}"))?;
    crate::sending::put(url).stuff(body)
}

/// Request type for the `send_transaction_message` endpoint.

#[derive(ToSchema, Deserialize, Serialize, Debug)]
pub struct SendMessageReqBody {
    // /// A transaction ID unique between sending and receiving homeservers.
    // #[salvo(parameter(parameter_in = Path))]
    // pub transaction_id: OwnedTransactionId,
    /// The server_name of the homeserver sending this transaction.
    pub origin: OwnedServerName,

    /// POSIX timestamp in milliseconds on the originating homeserver when this
    /// transaction started.
    pub origin_server_ts: UnixMillis,

    /// List of persistent updates to rooms.
    ///
    /// Must not be more than 50 items.
    ///
    /// With the `unstable-unspecified` feature, sending `pdus` is optional.
    /// See [matrix-spec#705](https://github.com/matrix-org/matrix-spec/issues/705).
    #[cfg_attr(
        feature = "unstable-unspecified",
        serde(default, skip_serializing_if = "<[_]>::is_empty")
    )]
    #[salvo(schema(value_type = Vec<Object>))]
    pub pdus: Vec<Box<RawJsonValue>>,

    /// List of ephemeral messages.
    ///
    /// Must not be more than 100 items.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub edus: Vec<Edu>,
}
crate::json_body_modifier!(SendMessageReqBody);

/// Response type for the `send_transaction_message` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]

pub struct SendMessageResBody {
    /// Map of event IDs and response for each PDU given in the request.
    ///
    /// With the `unstable-msc3618` feature, returning `pdus` is optional.
    /// See [MSC3618](https://github.com/matrix-org/matrix-spec-proposals/pull/3618).
    #[serde(default, with = "crate::serde::pdu_process_response")]
    pub pdus: BTreeMap<OwnedEventId, Result<(), String>>,
}
crate::json_body_modifier!(SendMessageResBody);

impl SendMessageResBody {
    /// Creates a new `Response` with the given PDUs.
    pub fn new(pdus: BTreeMap<OwnedEventId, Result<(), String>>) -> Self {
        Self { pdus }
    }
}

/// Type for passing ephemeral data to homeservers.
#[derive(ToSchema, Clone, Debug, Serialize)]
#[serde(tag = "edu_type", content = "content")]
pub enum Edu {
    /// An EDU representing presence updates for users of the sending
    /// homeserver.
    #[serde(rename = "m.presence")]
    Presence(PresenceContent),

    /// An EDU representing receipt updates for users of the sending homeserver.
    #[serde(rename = "m.receipt")]
    Receipt(ReceiptContent),

    /// A typing notification EDU for a user in a room.
    #[serde(rename = "m.typing")]
    Typing(TypingContent),

    /// An EDU that lets servers push details to each other when one of their
    /// users adds a new device to their account, required for E2E
    /// encryption to correctly target the current set of devices for a
    /// given user.
    #[serde(rename = "m.device_list_update")]
    DeviceListUpdate(DeviceListUpdateContent),

    /// An EDU that lets servers push send events directly to a specific device
    /// on a remote server - for instance, to maintain an Olm E2E encrypted
    /// message channel between a local and remote device.
    #[serde(rename = "m.direct_to_device")]
    DirectToDevice(DirectDeviceContent),

    /// An EDU that lets servers push details to each other when one of their
    /// users updates their cross-signing keys.
    #[serde(rename = "m.signing_key_update")]
    #[salvo(schema(value_type = Object))]
    SigningKeyUpdate(SigningKeyUpdateContent),

    #[doc(hidden)]
    #[salvo(schema(value_type = Object))]
    _Custom(JsonValue),
}

#[derive(Debug, Deserialize)]
struct EduDeHelper {
    /// The message type field
    edu_type: String,
    content: Box<RawJsonValue>,
}

impl<'de> Deserialize<'de> for Edu {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let json = Box::<RawJsonValue>::deserialize(deserializer)?;
        let EduDeHelper { edu_type, content } = from_raw_json_value(&json)?;

        Ok(match edu_type.as_ref() {
            "m.presence" => Self::Presence(from_raw_json_value(&content)?),
            "m.receipt" => Self::Receipt(from_raw_json_value(&content)?),
            "m.typing" => Self::Typing(from_raw_json_value(&content)?),
            "m.device_list_update" => Self::DeviceListUpdate(from_raw_json_value(&content)?),
            "m.direct_to_device" => Self::DirectToDevice(from_raw_json_value(&content)?),
            "m.signing_key_update" => Self::SigningKeyUpdate(from_raw_json_value(&content)?),
            _ => Self::_Custom(from_raw_json_value(&content)?),
        })
    }
}

/// Mapping between user and `ReceiptData`.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct ReceiptMap {
    /// Read receipts for users in the room.
    #[serde(rename = "m.read")]
    pub read: BTreeMap<OwnedUserId, ReceiptData>,
}

impl ReceiptMap {
    /// Creates a new `ReceiptMap`.
    pub fn new(read: BTreeMap<OwnedUserId, ReceiptData>) -> Self {
        Self { read }
    }
}

/// Metadata about the event that was last read and when.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReceiptData {
    /// Metadata for the read receipt.
    pub data: Receipt,

    /// The extremity event ID the user has read up to.
    pub event_ids: Vec<OwnedEventId>,
}

impl ReceiptData {
    /// Creates a new `ReceiptData`.
    pub fn new(data: Receipt, event_ids: Vec<OwnedEventId>) -> Self {
        Self { data, event_ids }
    }
}

/// The content for an `m.signing_key_update` EDU.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct SigningKeyUpdateContent {
    /// The user ID whose cross-signing keys have changed.
    pub user_id: OwnedUserId,

    /// The user's master key, if it was updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_key: Option<CrossSigningKey>,

    /// The users's self-signing key, if it was updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_signing_key: Option<CrossSigningKey>,
}

impl SigningKeyUpdateContent {
    /// Creates a new `SigningKeyUpdateContent`.
    pub fn new(user_id: OwnedUserId) -> Self {
        Self {
            user_id,
            master_key: None,
            self_signing_key: None,
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::events::ToDeviceEventType;
//     use crate::{room_id, user_id};
//     use assert_matches2::assert_matches;
//     use serde_json::json;

//     use super::{DeviceListUpdateContent, Edu, ReceiptContent};

//     #[test]
//     fn device_list_update_edu() {
//         let json = json!({
//             "content": {
//                 "deleted": false,
//                 "device_display_name": "Mobile",
//                 "device_id": "QBUAZIFURK",
//                 "keys": {
//                     "algorithms": [
//                         "m.olm.v1.curve25519-aes-sha2",
//                         "m.megolm.v1.aes-sha2"
//                     ],
//                     "device_id": "JLAFKJWSCS",
//                     "keys": {
//                         "curve25519:JLAFKJWSCS":
// "3C5BFWi2Y8MaVvjM8M22DBmh24PmgR0nPvJOIArzgyI",
// "ed25519:JLAFKJWSCS": "lEuiRJBit0IG6nUf5pUzWTUEsRVVe/HJkoKuEww9ULI"
//                     },
//                     "signatures": {
//                         "@alice:example.com": {
//                             "ed25519:JLAFKJWSCS":
// "dSO80A01XiigH3uBiDVx/EjzaoycHcjq9lfQX0uWsqxl2giMIiSPR8a4d291W1ihKJL/
// a+myXS367WT6NAIcBA"                         }
//                     },
//                     "user_id": "@alice:example.com"
//                 },
//                 "stream_id": 6,
//                 "user_id": "@john:example.com"
//             },
//             "edu_type": "m.device_list_update"
//         });

//         let edu = serde_json::from_value::<Edu>(json.clone()).unwrap();
//         assert_matches!(
//             &edu,
//             Edu::DeviceListUpdate(DeviceListUpdateContent {
//                 user_id,
//                 device_id,
//                 device_display_name,
//                 stream_id,
//                 prev_id,
//                 deleted,
//                 keys,
//             })
//         );

//         assert_eq!(user_id, "@john:example.com");
//         assert_eq!(device_id, "QBUAZIFURK");
//         assert_eq!(device_display_name.as_deref(), Some("Mobile"));
//         assert_eq!(*stream_id, u6);
//         assert_eq!(*prev_id, vec![]);
//         assert_eq!(*deleted, Some(false));
//         assert_matches!(keys, Some(_));

//         assert_eq!(serde_json::to_value(&edu).unwrap(), json);
//     }

//     #[test]
//     fn minimal_device_list_update_edu() {
//         let json = json!({
//             "content": {
//                 "device_id": "QBUAZIFURK",
//                 "stream_id": 6,
//                 "user_id": "@john:example.com"
//             },
//             "edu_type": "m.device_list_update"
//         });

//         let edu = serde_json::from_value::<Edu>(json.clone()).unwrap();
//         assert_matches!(
//             &edu,
//             Edu::DeviceListUpdate(DeviceListUpdateContent {
//                 user_id,
//                 device_id,
//                 device_display_name,
//                 stream_id,
//                 prev_id,
//                 deleted,
//                 keys,
//             })
//         );

//         assert_eq!(user_id, "@john:example.com");
//         assert_eq!(device_id, "QBUAZIFURK");
//         assert_eq!(*device_display_name, None);
//         assert_eq!(*stream_id, u6);
//         assert_eq!(*prev_id, vec![]);
//         assert_eq!(*deleted, None);
//         assert_matches!(keys, None);

//         assert_eq!(serde_json::to_value(&edu).unwrap(), json);
//     }

//     #[test]
//     fn receipt_edu() {
//         let json = json!({
//             "content": {
//                 "!some_room:example.org": {
//                     "m.read": {
//                         "@john:matrix.org": {
//                             "data": {
//                                 "ts": 1_533_358
//                             },
//                             "event_ids": [
//                                 "$read_this_event:matrix.org"
//                             ]
//                         }
//                     }
//                 }
//             },
//             "edu_type": "m.receipt"
//         });

//         let edu = serde_json::from_value::<Edu>(json.clone()).unwrap();
//         assert_matches!(&edu, Edu::Receipt(ReceiptContent { receipts }));
//         assert!(receipts.get(room_id!("!some_room:example.org")).is_some());

//         assert_eq!(serde_json::to_value(&edu).unwrap(), json);
//     }

//     #[test]
//     fn typing_edu() {
//         let json = json!({
//             "content": {
//                 "room_id": "!somewhere:matrix.org",
//                 "typing": true,
//                 "user_id": "@john:matrix.org"
//             },
//             "edu_type": "m.typing"
//         });

//         let edu = serde_json::from_value::<Edu>(json.clone()).unwrap();
//         assert_matches!(&edu, Edu::Typing(content));
//         assert_eq!(content.room_id, "!somewhere:matrix.org");
//         assert_eq!(content.user_id, "@john:matrix.org");
//         assert!(content.typing);

//         assert_eq!(serde_json::to_value(&edu).unwrap(), json);
//     }

//     #[test]
//     fn direct_to_device_edu() {
//         let json = json!({
//             "content": {
//                 "message_id": "hiezohf6Hoo7kaev",
//                 "messages": {
//                     "@alice:example.org": {
//                         "IWHQUZUIAH": {
//                             "algorithm": "m.megolm.v1.aes-sha2",
//                             "room_id": "!Cuyf34gef24t:localhost",
//                             "session_id":
// "X3lUlvLELLYxeTx4yOVu6UDpasGEVO0Jbu+QFnm0cKQ",
// "session_key": "AgAAAADxKHa9uFxcXzwYoNueL5Xqi69IkD4sni8LlfJL7qNBEY..."
//                         }
//                     }
//                 },
//                 "sender": "@john:example.com",
//                 "type": "m.room_key_request"
//             },
//             "edu_type": "m.direct_to_device"
//         });

//         let edu = serde_json::from_value::<Edu>(json.clone()).unwrap();
//         assert_matches!(&edu, Edu::DirectToDevice(content));
//         assert_eq!(content.sender, "@john:example.com");
//         assert_eq!(content.ev_type, ToDeviceEventType::RoomKeyRequest);
//         assert_eq!(content.message_id, "hiezohf6Hoo7kaev");
//         assert!(content.messages.get(user_id!("@alice:example.org")).
// is_some());

//         assert_eq!(serde_json::to_value(&edu).unwrap(), json);
//     }

//     #[test]
//     fn signing_key_update_edu() {
//         let json = json!({
//             "content": {
//                 "master_key": {
//                     "keys": {
//                         "ed25519:alice+base64+public+key":
// "alice+base64+public+key",
// "ed25519:base64+master+public+key": "base64+master+public+key"
// },                     "signatures": {
//                         "@alice:example.com": {
//                             "ed25519:alice+base64+master+key":
// "signature+of+key"                         }
//                     },
//                     "usage": [
//                         "master"
//                     ],
//                     "user_id": "@alice:example.com"
//                 },
//                 "self_signing_key": {
//                     "keys": {
//                         "ed25519:alice+base64+public+key":
// "alice+base64+public+key",
// "ed25519:base64+self+signing+public+key":
// "base64+self+signing+master+public+key"                     },
//                     "signatures": {
//                         "@alice:example.com": {
//                             "ed25519:alice+base64+master+key":
// "signature+of+key",
// "ed25519:base64+master+public+key": "signature+of+self+signing+key"
//                         }
//                     },
//                     "usage": [
//                         "self_signing"
//                     ],
//                     "user_id": "@alice:example.com"
//                   },
//                 "user_id": "@alice:example.com"
//             },
//             "edu_type": "m.signing_key_update"
//         });

//         let edu = serde_json::from_value::<Edu>(json.clone()).unwrap();
//         assert_matches!(&edu, Edu::SigningKeyUpdate(content));
//         assert_eq!(content.user_id, "@alice:example.com");
//         assert!(content.master_key.is_some());
//         assert!(content.self_signing_key.is_some());

//         assert_eq!(serde_json::to_value(&edu).unwrap(), json);
//     }
// }
