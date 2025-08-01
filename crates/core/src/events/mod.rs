//! (De)serializable types for the events in the [Matrix](https://matrix.org) specification.
//! These types are used by other Palpo  use   crate::s.
//!
//! All data exchanged over Matrix is expressed as an event.
//! Different event types represent different actions, such as joining a room or
//! sending a message. Events are stored and transmitted as simple JSON
//! structures. While anyone can create a new event type for their own purposes,
//! the Matrix specification defines a number of event types which are
//! considered core to the protocol. This module contains Rust types for all of
//! the event types defined by the specification and facilities for extending
//! the event system for custom event types.
//!
//! # Core event types
//!
//! This module includes Rust types for all event types in the Matrix
//! specification. To better organize the crate, these types live in separate
//! modules with a hierarchy that matches the reverse domain name notation of
//! the event type. For example, the `m.room.message` event
//! lives at `palpo::events::room::message::RoomMessageEvent`. Each type's
//! module also contains a Rust type for that event type's `content` field, and
//! any other supporting types required by the event's other fields.
//!
//! # Extending Palpo with custom events
//!
//! For our examples we will start with a simple custom state event.
//! `palpo_event` specifies the state event's `type` and its `kind`.
//!
//! ```rust
//! use serde::{Deserialize, Serialize};
//!
//! use palpo_core::macros::EventContent;
//! use palpo_core::RoomVersionId;
//!
//! #[derive(Clone, Debug, Deserialize, Serialize, EventContent)]
//! #[palpo_event(type = "org.example.event", kind = State, state_key_type = String)]
//! pub struct ExampleContent {
//!     field: String,
//! }
//! ```
//!
//! This can be used with events structs, such as passing it into
//! `palpo::api::client::state::send_state_event`'s `Request`.
//!
//! As a more advanced example we create a reaction message event. For this
//! event we will use a [`OriginalSyncMessageLikeEvent`] struct but any
//! [`OriginalMessageLikeEvent`] struct would work.
//!
//! ```rust
//! use palpo_core::{RoomVersionId, OwnedEventId};
//! use palpo_core::events::{EventContent, OriginalSyncMessageLikeEvent};
//! use palpo_core::macros::EventContent;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Clone, Debug, Deserialize, Serialize)]
//! #[serde(tag = "rel_type")]
//! pub enum RelatesTo {
//!     #[serde(rename = "m.annotation")]
//!     Annotation {
//!         /// The event this reaction relates to.
//!         event_id: OwnedEventId,
//!         /// The displayable content of the reaction.
//!         key: String,
//!     },
//!
//!     /// Since this event is not fully specified in the Matrix spec
//!     /// it may change or types may be added, we are ready!
//!     #[serde(rename = "m.whatever")]
//!     Whatever,
//! }
//!
//! /// The payload for our reaction event.
//! #[derive(Clone, Debug, Deserialize, Serialize, EventContent)]
//! #[palpo_event(type = "m.reaction", kind = MessageLike)]
//! pub struct ReactionEventContent {
//!     #[serde(rename = "m.relates_to")]
//!     pub relates_to: RelatesTo,
//! }
//!
//! let json = serde_json::json!({
//!     "content": {
//!         "m.relates_to": {
//!             "event_id": "$xxxx-xxxx",
//!             "key": "👍",
//!             "rel_type": "m.annotation"
//!         }
//!     },
//!     "event_id": "$xxxx-xxxx",
//!     "origin_server_ts": 1,
//!     "sender": "@someone:example.org",
//!     "type": "m.reaction",
//!     "unsigned": {
//!         "age": 85
//!     }
//! });
//!
//! // The downside of this event is we cannot use it with event enums,
//! // but could be deserialized from a `RawJson<_>` that has failed to deserialize.
//! assert!(matches!(
//!     serde_json::from_value::<OriginalSyncMessageLikeEvent<ReactionEventContent>>(json),
//!     Ok(OriginalSyncMessageLikeEvent {
//!         content: ReactionEventContent {
//!             relates_to: RelatesTo::Annotation { key, .. },
//!         },
//!         ..
//!     }) if key == "👍"
//! ));
//! ```

// Needs to be public for trybuild tests
#[doc(hidden)]
pub mod _custom;
mod content;
mod enums;
mod kinds;
mod state_key;
mod unsigned;

#[cfg(feature = "unstable-msc3927")]
pub mod audio;
#[cfg(feature = "unstable-msc3489")]
pub mod beacon;
#[cfg(feature = "unstable-msc3489")]
pub mod beacon_info;
pub mod call;
pub mod direct;
pub mod dummy;
#[cfg(feature = "unstable-msc3954")]
pub mod emote;
#[cfg(feature = "unstable-msc3956")]
pub mod encrypted;
#[cfg(feature = "unstable-msc3551")]
pub mod file;
pub mod forwarded_room_key;
pub mod fully_read;
pub mod identity_server;
pub mod ignored_user_list;
#[cfg(feature = "unstable-msc3552")]
pub mod image;
#[cfg(feature = "unstable-msc2545")]
pub mod image_pack;
pub mod key;
#[cfg(feature = "unstable-msc3488")]
pub mod location;
pub mod marked_unread;
#[cfg(feature = "unstable-msc4171")]
pub mod member_hints;
#[cfg(feature = "unstable-msc1767")]
pub mod message;
// pub mod pdu;
pub mod policy;
#[cfg(feature = "unstable-msc3381")]
pub mod poll;
pub mod presence;
pub mod push_rules;
pub mod reaction;
pub mod receipt;
pub mod relation;
pub mod room;
pub mod room_key;
pub mod room_key_request;
pub mod secret;
pub mod secret_storage;
pub mod space;
pub mod sticker;
pub mod tag;
pub mod typing;
#[cfg(feature = "unstable-msc3553")]
pub mod video;
#[cfg(feature = "unstable-msc3245")]
pub mod voice;

use std::collections::BTreeSet;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize, Serializer, de::IgnoredAny};
use smallstr::SmallString;

pub use self::{
    content::*,
    enums::*,
    kinds::*,
    relation::{BundledMessageLikeRelations, BundledStateRelations},
    state_key::EmptyStateKey,
    unsigned::{MessageLikeUnsigned, RedactedUnsigned, StateUnsigned, UnsignedRoomRedactionEvent},
};
use crate::{EventEncryptionAlgorithm, OwnedUserId, RoomVersionId};

pub type StateKey = SmallString<[u8; INLINE_SIZE]>;
const INLINE_SIZE: usize = 48;

/// Trait to define the behavior of redact an event's content object.
pub trait RedactContent {
    /// The redacted form of the event's content.
    type Redacted;

    /// Transform `self` into a redacted form (removing most or all fields)
    /// according to the spec.
    ///
    /// A small number of events have room-version specific redaction behavior,
    /// so a version has to be specified.
    fn redact(self, version: &RoomVersionId) -> Self::Redacted;
}

/// Helper struct to determine the event kind from a
/// `serde_json::value::RawValue`.
#[doc(hidden)]
#[derive(Deserialize)]
#[allow(clippy::exhaustive_structs)]
pub struct EventTypeDeHelper<'a> {
    #[serde(borrow, rename = "type")]
    pub ev_type: std::borrow::Cow<'a, str>,
}

/// Helper struct to determine if an event has been redacted.
#[doc(hidden)]
#[derive(Deserialize)]
#[allow(clippy::exhaustive_structs)]
pub struct RedactionDeHelper {
    /// Used to check whether redacted_because exists.
    pub unsigned: Option<UnsignedDeHelper>,
}

#[doc(hidden)]
#[derive(Deserialize)]
#[allow(clippy::exhaustive_structs)]
pub struct UnsignedDeHelper {
    /// This is the field that signals an event has been redacted.
    pub redacted_because: Option<IgnoredAny>,
}

/// Helper function for erroring when trying to serialize an event enum _Custom
/// variant that can only be created by deserializing from an unknown event
/// type.
#[doc(hidden)]
#[allow(clippy::ptr_arg)]
pub fn serialize_custom_event_error<T, S: Serializer>(_: &T, _: S) -> Result<S::Ok, S::Error> {
    Err(serde::ser::Error::custom(
        "Failed to serialize event [content] enum: Unknown event type.\n\
         To send custom events, turn them into `RawJson<EnumType>` by going through
         `serde_json::value::to_raw_value` and `RawJson::from_json`.",
    ))
}

/// Describes whether the event mentions other users or the room.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Mentions {
    /// The list of mentioned users.
    ///
    /// Defaults to an empty `BTreeSet`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub user_ids: BTreeSet<OwnedUserId>,

    /// Whether the whole room is mentioned.
    ///
    /// Defaults to `false`.
    #[serde(default, skip_serializing_if = "palpo_core::serde::is_default")]
    pub room: bool,
}

impl Mentions {
    /// Create a `Mentions` with the default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a `Mentions` for the given user IDs.
    pub fn with_user_ids(user_ids: impl IntoIterator<Item = OwnedUserId>) -> Self {
        Self {
            user_ids: BTreeSet::from_iter(user_ids),
            ..Default::default()
        }
    }

    /// Create a `Mentions` for a room mention.
    pub fn with_room_mention() -> Self {
        Self {
            room: true,
            ..Default::default()
        }
    }

    fn add(&mut self, mentions: Self) {
        self.user_ids.extend(mentions.user_ids);
        self.room |= mentions.room;
    }
}
