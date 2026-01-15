//! Endpoints for event receipts.
//! `POST /_matrix/client/*/rooms/{room_id}/receipt/{receiptType}/{event_id}`
//!
//! Send a receipt event to a room.
//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3roomsroomidreceiptreceipttypeeventid
use std::collections::{BTreeMap, btree_map};
use std::ops::{Deref, DerefMut};

use salvo::prelude::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::macros::EventContent;
use crate::serde::StringEnum;
use crate::{
    EventId, IdParseError, OwnedEventId, OwnedRoomId, OwnedUserId, PrivOwnedStr, UnixMillis, UserId,
};

/// The content of an `m.receipt` event.
///
/// A mapping of event ID to a collection of receipts for this event ID. The
/// event ID is the ID of the event being acknowledged and *not* an ID for the
/// receipt itself.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[allow(clippy::exhaustive_structs)]
#[palpo_event(type = "m.receipt", kind = EphemeralRoom)]
pub struct ReceiptEventContent(pub BTreeMap<OwnedEventId, Receipts>);

impl ReceiptEventContent {
    /// Get the receipt for the given user ID with the given receipt type, if it
    /// exists.
    pub fn user_receipt(
        &self,
        user_id: &UserId,
        receipt_type: ReceiptType,
    ) -> Option<(&EventId, &Receipt)> {
        self.iter().find_map(|(event_id, receipts)| {
            let receipt = receipts.get(&receipt_type)?.get(user_id)?;
            Some((event_id.as_ref(), receipt))
        })
    }
}

impl Deref for ReceiptEventContent {
    type Target = BTreeMap<OwnedEventId, Receipts>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ReceiptEventContent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for ReceiptEventContent {
    type Item = (OwnedEventId, Receipts);
    type IntoIter = btree_map::IntoIter<OwnedEventId, Receipts>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(OwnedEventId, Receipts)> for ReceiptEventContent {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = (OwnedEventId, Receipts)>,
    {
        Self(BTreeMap::from_iter(iter))
    }
}

/// A collection of receipts.
pub type Receipts = BTreeMap<ReceiptType, UserReceipts>;

/// A mapping of user ID to receipt.
///
/// The user ID is the entity who sent this receipt.
pub type UserReceipts = BTreeMap<OwnedUserId, Receipt>;

/// An acknowledgement of an event.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct Receipt {
    /// The time when the receipt was sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[salvo(schema(value_type = Option<u64>))]
    pub ts: Option<UnixMillis>,

    /// The thread this receipt applies to.
    #[serde(
        rename = "thread_id",
        default,
        skip_serializing_if = "crate::serde::is_default"
    )]
    pub thread: ReceiptThread,
}

impl Receipt {
    /// Creates a new `Receipt` with the given timestamp.
    ///
    /// To create an empty receipt instead, use [`Receipt::default`].
    pub fn new(ts: UnixMillis) -> Self {
        Self {
            ts: Some(ts),
            thread: ReceiptThread::Unthreaded,
        }
    }
}
/// The [thread a receipt applies to].
///
/// This type can hold an arbitrary string. To build this with a custom value,
/// convert it from an `Option<String>` with `::from()` / `.into()`.
/// [`ReceiptThread::Unthreaded`] can be constructed from `None`.
///
/// To check for values that are not available as a documented variant here, use
/// its string representation, obtained through [`.as_str()`](Self::as_str()).
///
/// [thread a receipt applies to]: https://spec.matrix.org/latest/client-server-api/#threaded-read-receipts
#[derive(ToSchema, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReceiptThread {
    /// The receipt applies to the timeline, regardless of threads.
    ///
    /// Used by clients that are not aware of threads.
    ///
    /// This is the default.
    #[default]
    Unthreaded,

    /// The receipt applies to the main timeline.
    ///
    /// Used for events that don't belong to a thread.
    Main,

    /// The receipt applies to a thread.
    ///
    /// Used for events that belong to a thread with the given thread root.
    Thread(OwnedEventId),

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

impl ReceiptThread {
    /// Get the string representation of this `ReceiptThread`.
    ///
    /// [`ReceiptThread::Unthreaded`] returns `None`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Unthreaded => None,
            Self::Main => Some("main"),
            Self::Thread(event_id) => Some(event_id.as_str()),
            Self::_Custom(s) => Some(&s.0),
        }
    }
}

impl<T> TryFrom<Option<T>> for ReceiptThread
where
    T: AsRef<str> + Into<Box<str>>,
{
    type Error = IdParseError;

    fn try_from(s: Option<T>) -> Result<Self, Self::Error> {
        let res = match s {
            None => Self::Unthreaded,
            Some(s) => match s.as_ref() {
                "main" => Self::Main,
                s_ref if s_ref.starts_with('$') => Self::Thread(EventId::parse(s_ref)?),
                _ => Self::_Custom(PrivOwnedStr(s.into())),
            },
        };

        Ok(res)
    }
}
impl Serialize for ReceiptThread {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ReceiptThread {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = palpo_core::serde::deserialize_cow_str(deserializer)?;
        Self::try_from(Some(s)).map_err(serde::de::Error::custom)
    }
}

/// The content for "m.receipt" Edu.

#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[serde(transparent)]
pub struct ReceiptContent(pub BTreeMap<OwnedRoomId, ReceiptMap>);

impl ReceiptContent {
    /// Creates a new `ReceiptContent`.
    pub fn new(receipts: BTreeMap<OwnedRoomId, ReceiptMap>) -> Self {
        Self(receipts)
    }
}

impl Deref for ReceiptContent {
    type Target = BTreeMap<OwnedRoomId, ReceiptMap>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ReceiptContent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for ReceiptContent {
    type Item = (OwnedRoomId, ReceiptMap);
    type IntoIter = btree_map::IntoIter<OwnedRoomId, ReceiptMap>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(OwnedRoomId, ReceiptMap)> for ReceiptContent {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = (OwnedRoomId, ReceiptMap)>,
    {
        Self(BTreeMap::from_iter(iter))
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
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
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

// #[cfg(test)]
// mod tests {
//     use crate::{owned_event_id, UnixMillis};
//     use assert_matches2::assert_matches;
//     use serde_json::{from_value as from_json_value, json, to_value as
// to_json_value};

//     use super::{Receipt, ReceiptThread};

//     #[test]
//     fn serialize_receipt() {
//         let mut receipt = Receipt::default();
//         assert_eq!(to_json_value(receipt.clone()).unwrap(), json!({}));

//         receipt.thread = ReceiptThread::Main;
//         assert_eq!(to_json_value(receipt.clone()).unwrap(), json!({
// "thread_id": "main" }));

//         receipt.thread =
// ReceiptThread::Thread(owned_event_id!("$abcdef76543"));         assert_eq!
// (to_json_value(receipt).unwrap(), json!({ "thread_id": "$abcdef76543" }));

//         let mut receipt =
// Receipt::new(UnixMillis(1_664_702_144_365_u64.try_into().unwrap()));
//         assert_eq!(
//             to_json_value(receipt.clone()).unwrap(),
//             json!({ "ts": 1_664_702_144_365_u64 })
//         );

//         receipt.thread =
// ReceiptThread::try_from(Some("io.palpo.unknown")).unwrap();
//         assert_eq!(
//             to_json_value(receipt).unwrap(),
//             json!({ "ts": 1_664_702_144_365_u64, "thread_id":
// "io.palpo.unknown" })         );
//     }

//     #[test]
//     fn deserialize_receipt() {
//         let receipt = from_json_value::<Receipt>(json!({})).unwrap();
//         assert_eq!(receipt.ts, None);
//         assert_eq!(receipt.thread, ReceiptThread::Unthreaded);

//         let receipt = from_json_value::<Receipt>(json!({ "thread_id": "main"
// })).unwrap();         assert_eq!(receipt.ts, None);
//         assert_eq!(receipt.thread, ReceiptThread::Main);

//         let receipt = from_json_value::<Receipt>(json!({ "thread_id":
// "$abcdef76543" })).unwrap();         assert_eq!(receipt.ts, None);
//         assert_matches!(receipt.thread, ReceiptThread::Thread(event_id));
//         assert_eq!(event_id, "$abcdef76543");

//         let receipt = from_json_value::<Receipt>(json!({ "ts":
// 1_664_702_144_365_u64 })).unwrap();         assert_eq!(
//             receipt.ts.unwrap(),
//             UnixMillis(1_664_702_144_365_u64.try_into().unwrap())
//         );
//         assert_eq!(receipt.thread, ReceiptThread::Unthreaded);

//         let receipt =
//             from_json_value::<Receipt>(json!({ "ts": 1_664_702_144_365_u64,
// "thread_id": "io.palpo.unknown" }))                 .unwrap();
//         assert_eq!(
//             receipt.ts.unwrap(),
//             UnixMillis(1_664_702_144_365_u64.try_into().unwrap())
//         );
//         assert_matches!(&receipt.thread, ReceiptThread::_Custom(_));
//         assert_eq!(receipt.thread.as_str().unwrap(), "io.palpo.unknown");
//     }
// }

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 =>
// "/_matrix/client/r0/rooms/:room_id/receipt/:receipt_type/:event_id",
//         1.1 =>
// "/_matrix/client/v3/rooms/:room_id/receipt/:receipt_type/:event_id",     }
// };

#[derive(ToParameters, Deserialize, Debug)]
pub struct SendReceiptReqArgs {
    /// The room in which to send the event.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The type of receipt to send.
    #[salvo(parameter(parameter_in = Path))]
    pub receipt_type: ReceiptType,

    /// The event ID to acknowledge up to.
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,
}

/// Request type for the `create_receipt` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct CreateReceiptReqBody {
    /// The thread this receipt applies to.
    ///
    /// *Note* that this must be the default value if used with
    /// [`ReceiptType::FullyRead`].
    ///
    /// Defaults to [`ReceiptThread::Unthreaded`].
    #[serde(
        rename = "thread_id",
        default,
        skip_serializing_if = "crate::serde::is_default"
    )]
    pub thread: ReceiptThread,
}

/// The type of receipt.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
pub enum ReceiptType {
    /// A [public read receipt].
    ///
    /// Indicates that the given event has been presented to the user.
    ///
    /// This receipt is federated to other users.
    ///
    /// [public read receipt]: https://spec.matrix.org/latest/client-server-api/#receipts
    #[palpo_enum(rename = "m.read")]
    Read,

    /// A [private read receipt].
    ///
    /// Indicates that the given event has been presented to the user.
    ///
    /// This read receipt is not federated so only the user and their homeserver
    /// are aware of it.
    ///
    /// [private read receipt]: https://spec.matrix.org/latest/client-server-api/#private-read-receipts
    #[palpo_enum(rename = "m.read.private")]
    ReadPrivate,

    /// A [fully read marker].
    ///
    /// Indicates that the given event has been read by the user.
    ///
    /// This is actually not a receipt, but a piece of room account data. It is
    /// provided here for convenience.
    ///
    /// [fully read marker]: https://spec.matrix.org/latest/client-server-api/#fully-read-markers
    #[palpo_enum(rename = "m.fully_read")]
    FullyRead,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

pub fn combine_receipt_event_contents(receipts: Vec<ReceiptEventContent>) -> ReceiptEventContent {
    let mut combined = ReceiptEventContent(BTreeMap::new());

    for receipt in receipts {
        for (event_id, receipts) in receipt.0 {
            let type_map = combined.0.entry(event_id).or_default();
            for receipt in receipts {
                type_map.entry(receipt.0).or_default().extend(receipt.1);
            }
        }
    }

    combined
}
