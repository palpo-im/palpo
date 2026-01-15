//! Types describing [relationships between events].
//!
//! [relationships between events]: https://spec.matrix.org/latest/client-server-api/#forming-relationships-between-events

use std::fmt::Debug;

use salvo::oapi::ToSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use super::AnyMessageLikeEvent;
use crate::serde::{JsonObject, RawJson, StringEnum};
use crate::{OwnedEventId, PrivOwnedStr};

/// Information about the event a [rich reply] is replying to.
///
/// [rich reply]: https://spec.matrix.org/latest/client-server-api/#rich-replies
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct InReplyTo {
    /// The event being replied to.
    pub event_id: OwnedEventId,
}

impl InReplyTo {
    /// Creates a new `InReplyTo` with the given event ID.
    pub fn new(event_id: OwnedEventId) -> Self {
        Self { event_id }
    }
}

/// An [annotation] for an event.
///
/// [annotation]: https://spec.matrix.org/latest/client-server-api/#event-annotations-and-reactions
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "rel_type", rename = "m.annotation")]
pub struct Annotation {
    /// The event that is being annotated.
    pub event_id: OwnedEventId,

    /// A string that indicates the annotation being applied.
    ///
    /// When sending emoji reactions, this field should include the colourful
    /// variation-16 when applicable.
    ///
    /// Clients should render reactions that have a long `key` field in a
    /// sensible manner.
    pub key: String,
}

impl Annotation {
    /// Creates a new `Annotation` with the given event ID and key.
    pub fn new(event_id: OwnedEventId, key: String) -> Self {
        Self { event_id, key }
    }
}

/// The content of a [replacement] relation.
///
/// [replacement]: https://spec.matrix.org/latest/client-server-api/#event-replacements
#[derive(ToSchema, Deserialize, Clone, Debug)]
pub struct Replacement<C> {
    /// The ID of the event being replaced.
    pub event_id: OwnedEventId,

    /// New content.
    pub new_content: C,
}

impl<C> Replacement<C>
where
    C: ToSchema,
{
    /// Creates a new `Replacement` with the given event ID and new content.
    pub fn new(event_id: OwnedEventId, new_content: C) -> Self {
        Self {
            event_id,
            new_content,
        }
    }
}

/// The content of a [thread] relation.
///
/// [thread]: https://spec.matrix.org/latest/client-server-api/#threading
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "rel_type", rename = "m.thread")]
pub struct Thread {
    /// The ID of the root message in the thread.
    pub event_id: OwnedEventId,

    /// A reply relation.
    ///
    /// If this event is a reply and belongs to a thread, this points to the
    /// message that is being replied to, and `is_falling_back` must be set
    /// to `false`.
    ///
    /// If this event is not a reply, this is used as a fallback mechanism for
    /// clients that do not support threads. This should point to the latest
    /// message-like event in the thread and `is_falling_back` must be set
    /// to `true`.
    #[serde(rename = "m.in_reply_to", skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<InReplyTo>,

    /// Whether the `m.in_reply_to` field is a fallback for older clients or a
    /// genuine reply in a thread.
    #[serde(default, skip_serializing_if = "palpo_core::serde::is_default")]
    pub is_falling_back: bool,
}

impl Thread {
    /// Convenience method to create a regular `Thread` relation with the given
    /// root event ID and latest message-like event ID.
    pub fn plain(event_id: OwnedEventId, latest_event_id: OwnedEventId) -> Self {
        Self {
            event_id,
            in_reply_to: Some(InReplyTo::new(latest_event_id)),
            is_falling_back: true,
        }
    }

    /// Convenience method to create a regular `Thread` relation with the given
    /// root event ID and *without* the recommended reply fallback.
    pub fn without_fallback(event_id: OwnedEventId) -> Self {
        Self {
            event_id,
            in_reply_to: None,
            is_falling_back: false,
        }
    }

    /// Convenience method to create a reply `Thread` relation with the given
    /// root event ID and replied-to event ID.
    pub fn reply(event_id: OwnedEventId, reply_to_event_id: OwnedEventId) -> Self {
        Self {
            event_id,
            in_reply_to: Some(InReplyTo::new(reply_to_event_id)),
            is_falling_back: false,
        }
    }
}

/// A bundled thread.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct BundledThread {
    /// The latest event in the thread.
    pub latest_event: RawJson<AnyMessageLikeEvent>,

    /// The number of events in the thread.
    pub count: u64,

    /// Whether the current logged in user has participated in the thread.
    pub current_user_participated: bool,
}

impl BundledThread {
    /// Creates a new `BundledThread` with the given event, count and user
    /// participated flag.
    pub fn new(
        latest_event: RawJson<AnyMessageLikeEvent>,
        count: u64,
        current_user_participated: bool,
    ) -> Self {
        Self {
            latest_event,
            count,
            current_user_participated,
        }
    }
}

/// A [reference] to another event.
///
/// [reference]: https://spec.matrix.org/latest/client-server-api/#reference-relations
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "rel_type", rename = "m.reference")]
pub struct Reference {
    /// The ID of the event being referenced.
    pub event_id: OwnedEventId,
}

impl Reference {
    /// Creates a new `Reference` with the given event ID.
    pub fn new(event_id: OwnedEventId) -> Self {
        Self { event_id }
    }
}

/// A bundled reference.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct BundledReference {
    /// The ID of the event referencing this event.
    pub event_id: OwnedEventId,
}

impl BundledReference {
    /// Creates a new `BundledThread` with the given event ID.
    pub fn new(event_id: OwnedEventId) -> Self {
        Self { event_id }
    }
}

/// A chunk of references.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct ReferenceChunk {
    /// A batch of bundled references.
    pub chunk: Vec<BundledReference>,
}

impl ReferenceChunk {
    /// Creates a new `ReferenceChunk` with the given chunk.
    pub fn new(chunk: Vec<BundledReference>) -> Self {
        Self { chunk }
    }
}

/// [Bundled aggregations] of related child events of a message-like event.
///
/// [Bundled aggregations]: https://spec.matrix.org/latest/client-server-api/#aggregations-of-child-events
#[derive(ToSchema, Clone, Debug, Serialize)]
pub struct BundledMessageLikeRelations<E> {
    /// Replacement relation.
    #[serde(rename = "m.replace", skip_serializing_if = "Option::is_none")]
    pub replace: Option<Box<E>>,

    /// Set when the above fails to deserialize.
    ///
    /// Intentionally *not* public.
    #[serde(skip_serializing)]
    has_invalid_replacement: bool,

    /// Thread relation.
    #[serde(rename = "m.thread", skip_serializing_if = "Option::is_none")]
    pub thread: Option<Box<BundledThread>>,

    /// Reference relations.
    #[serde(rename = "m.reference", skip_serializing_if = "Option::is_none")]
    pub reference: Option<Box<ReferenceChunk>>,
}

impl<E> BundledMessageLikeRelations<E> {
    /// Creates a new empty `BundledMessageLikeRelations`.
    pub const fn new() -> Self {
        Self {
            replace: None,
            has_invalid_replacement: false,
            thread: None,
            reference: None,
        }
    }

    /// Whether this bundle contains a replacement relation.
    ///
    /// This may be `true` even if the `replace` field is `None`, because Matrix
    /// versions prior to 1.7 had a different incompatible format for
    /// bundled replacements. Use this method to check whether an event was
    /// replaced. If this returns `true` but `replace` is `None`, use one of
    /// the endpoints from `palpo::api::client::relations` to fetch the relation
    /// details.
    pub fn has_replacement(&self) -> bool {
        self.replace.is_some() || self.has_invalid_replacement
    }

    /// Returns `true` if all fields are empty.
    pub fn is_empty(&self) -> bool {
        self.replace.is_none() && self.thread.is_none() && self.reference.is_none()
    }

    /// Transform `BundledMessageLikeRelations<E>` to
    /// `BundledMessageLikeRelations<T>` using the given closure to convert
    /// the `replace` field if it is `Some(_)`.
    pub(crate) fn map_replace<T>(self, f: impl FnOnce(E) -> T) -> BundledMessageLikeRelations<T> {
        let Self {
            replace,
            has_invalid_replacement,
            thread,
            reference,
        } = self;
        let replace = replace.map(|r| Box::new(f(*r)));
        BundledMessageLikeRelations {
            replace,
            has_invalid_replacement,
            thread,
            reference,
        }
    }
}

impl<E> Default for BundledMessageLikeRelations<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// [Bundled aggregations] of related child events of a state event.
///
/// [Bundled aggregations]: https://spec.matrix.org/latest/client-server-api/#aggregations-of-child-events
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct BundledStateRelations {
    /// Thread relation.
    #[serde(rename = "m.thread", skip_serializing_if = "Option::is_none")]
    pub thread: Option<Box<BundledThread>>,

    /// Reference relations.
    #[serde(rename = "m.reference", skip_serializing_if = "Option::is_none")]
    pub reference: Option<Box<ReferenceChunk>>,
}

impl BundledStateRelations {
    /// Creates a new empty `BundledStateRelations`.
    pub const fn new() -> Self {
        Self {
            thread: None,
            reference: None,
        }
    }

    /// Returns `true` if all fields are empty.
    pub fn is_empty(&self) -> bool {
        self.thread.is_none() && self.reference.is_none()
    }
}

/// Relation types as defined in `rel_type` of an `m.relates_to` field.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all(prefix = "m.", rule = "snake_case"))]
#[non_exhaustive]
pub enum RelationType {
    /// `m.annotation`, an annotation, principally used by reactions.
    Annotation,

    /// `m.replace`, a replacement.
    Replacement,

    /// `m.thread`, a participant to a thread.
    Thread,

    /// `m.reference`, a reference to another event.
    Reference,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// The payload for a custom relation.
#[doc(hidden)]
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct CustomRelation(pub(super) JsonObject);

impl CustomRelation {
    pub(super) fn rel_type(&self) -> Option<RelationType> {
        Some(self.0.get("rel_type")?.as_str()?.into())
    }
}

#[derive(Deserialize)]
struct BundledMessageLikeRelationsJsonRepr<E> {
    #[serde(rename = "m.replace")]
    replace: Option<RawJson<Box<E>>>,
    #[serde(rename = "m.thread")]
    thread: Option<Box<BundledThread>>,
    #[serde(rename = "m.reference")]
    reference: Option<Box<ReferenceChunk>>,
}

impl<'de, E> Deserialize<'de> for BundledMessageLikeRelations<E>
where
    E: DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let BundledMessageLikeRelationsJsonRepr {
            replace,
            thread,
            reference,
        } = BundledMessageLikeRelationsJsonRepr::deserialize(deserializer)?;

        let (replace, has_invalid_replacement) =
            match replace.as_ref().map(RawJson::deserialize).transpose() {
                Ok(replace) => (replace, false),
                Err(_) => (None, true),
            };

        Ok(BundledMessageLikeRelations {
            replace,
            has_invalid_replacement,
            thread,
            reference,
        })
    }
}
