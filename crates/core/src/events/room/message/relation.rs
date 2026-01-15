use std::borrow::Cow;

use salvo::oapi::ToSchema;
use serde::Deserialize;

use crate::events::relation::{CustomRelation, InReplyTo, RelationType, Replacement, Thread};
use crate::serde::JsonObject;

/// Message event relationship.
#[derive(ToSchema, Deserialize, Clone, Debug)]
#[allow(clippy::manual_non_exhaustive)]
pub enum Relation<C> {
    /// An `m.in_reply_to` relation indicating that the event is a reply to
    /// another event.
    Reply {
        /// Information about another message being replied to.
        in_reply_to: InReplyTo,
    },

    /// An event that replaces another event.
    Replacement(Replacement<C>),

    /// An event that belongs to a thread.
    Thread(Thread),

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(CustomRelation),
}

impl<C> Relation<C>
where
    C: ToSchema,
{
    /// The type of this `Relation`.
    ///
    /// Returns an `Option` because the `Reply` relation does not have
    /// a`rel_type` field.
    pub fn rel_type(&self) -> Option<RelationType> {
        match self {
            Relation::Reply { .. } => None,
            Relation::Replacement(_) => Some(RelationType::Replacement),
            Relation::Thread(_) => Some(RelationType::Thread),
            Relation::_Custom(c) => c.rel_type(),
        }
    }

    /// The associated data.
    ///
    /// The returned JSON object holds the contents of `m.relates_to`, including
    /// `rel_type` and `event_id` if present, but not things like
    /// `m.new_content` for `m.replace` relations that live next to
    /// `m.relates_to`.
    ///
    /// Prefer to use the public variants of `Relation` where possible; this
    /// method is meant to be used for custom relations only.
    pub fn data(&self) -> Cow<'_, JsonObject>
    where
        C: Clone,
    {
        if let Relation::_Custom(CustomRelation(data)) = self {
            Cow::Borrowed(data)
        } else {
            Cow::Owned(self.serialize_data())
        }
    }
}

/// Message event relationship, except a replacement.
#[derive(ToSchema, Clone, Debug)]
#[allow(clippy::manual_non_exhaustive)]
pub enum RelationWithoutReplacement {
    /// An `m.in_reply_to` relation indicating that the event is a reply to
    /// another event.
    Reply {
        /// Information about another message being replied to.
        in_reply_to: InReplyTo,
    },

    /// An event that belongs to a thread.
    Thread(Thread),

    #[doc(hidden)]
    _Custom(CustomRelation),
}

impl RelationWithoutReplacement {
    /// The type of this `Relation`.
    ///
    /// Returns an `Option` because the `Reply` relation does not have
    /// a`rel_type` field.
    pub fn rel_type(&self) -> Option<RelationType> {
        match self {
            Self::Reply { .. } => None,
            Self::Thread(_) => Some(RelationType::Thread),
            Self::_Custom(c) => c.rel_type(),
        }
    }

    /// The associated data.
    ///
    /// The returned JSON object holds the contents of `m.relates_to`, including
    /// `rel_type` and `event_id` if present, but not things like
    /// `m.new_content` for `m.replace` relations that live next to
    /// `m.relates_to`.
    ///
    /// Prefer to use the public variants of `Relation` where possible; this
    /// method is meant to be used for custom relations only.
    pub fn data(&self) -> Cow<'_, JsonObject> {
        if let Self::_Custom(CustomRelation(data)) = self {
            Cow::Borrowed(data)
        } else {
            Cow::Owned(self.serialize_data())
        }
    }
}

impl<C> TryFrom<Relation<C>> for RelationWithoutReplacement
where
    C: ToSchema,
{
    type Error = Replacement<C>;

    fn try_from(value: Relation<C>) -> Result<Self, Self::Error> {
        let rel = match value {
            Relation::Reply { in_reply_to } => Self::Reply { in_reply_to },
            Relation::Replacement(r) => return Err(r),
            Relation::Thread(t) => Self::Thread(t),
            Relation::_Custom(c) => Self::_Custom(c),
        };

        Ok(rel)
    }
}
