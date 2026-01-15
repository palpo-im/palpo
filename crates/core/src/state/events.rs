//! Helper traits and types to work with events (aka PDUs).

mod create;
mod join_rules;
pub(crate) mod member;
pub(crate) mod power_levels;
mod third_party_invite;

use std::borrow::Borrow;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::sync::Arc;

pub use self::create::RoomCreateEvent;
pub use self::join_rules::RoomJoinRulesEvent;
pub use self::member::RoomMemberEvent;
pub use self::power_levels::{RoomPowerLevelsEvent, RoomPowerLevelsIntField};
pub use self::third_party_invite::RoomThirdPartyInviteEvent;
use crate::events::TimelineEventType;
use crate::serde::RawJsonValue;
use crate::{EventId, RoomId, UnixMillis, UserId};

/// Abstraction of a PDU so users can have their own PDU types.
pub trait Event: Debug {
    type Id: Clone + Debug + Display + Eq + Ord + Hash + Borrow<EventId>;

    /// The `EventId` of this event.
    fn event_id(&self) -> &Self::Id;

    /// The `RoomId` of this event.
    fn room_id(&self) -> &RoomId;

    /// The `UserId` of this event.
    fn sender(&self) -> &UserId;

    /// The time of creation on the originating server.
    fn origin_server_ts(&self) -> UnixMillis;

    /// The event type.
    fn event_type(&self) -> &TimelineEventType;

    /// The event's content.
    fn content(&self) -> &RawJsonValue;

    /// The state key for this event.
    fn state_key(&self) -> Option<&str>;

    /// The events before this event.
    // Requires GATs to avoid boxing (and TAIT for making it convenient).
    fn prev_events(&self) -> &[Self::Id];

    /// All the authenticating events for this event.
    // Requires GATs to avoid boxing (and TAIT for making it convenient).
    fn auth_events(&self) -> &[Self::Id];

    /// If this event is a redaction event this is the event it redacts.
    fn redacts(&self) -> Option<&Self::Id>;

    /// Whether this event was rejected for not passing the checks on reception of a PDU.
    fn rejected(&self) -> bool;
}

impl<T: Event> Event for &T {
    type Id = T::Id;

    fn event_id(&self) -> &Self::Id {
        (*self).event_id()
    }

    fn room_id(&self) -> &RoomId {
        (*self).room_id()
    }

    fn sender(&self) -> &UserId {
        (*self).sender()
    }

    fn origin_server_ts(&self) -> UnixMillis {
        (*self).origin_server_ts()
    }

    fn event_type(&self) -> &TimelineEventType {
        (*self).event_type()
    }

    fn content(&self) -> &RawJsonValue {
        (*self).content()
    }

    fn state_key(&self) -> Option<&str> {
        (*self).state_key()
    }

    fn prev_events(&self) -> &[Self::Id] {
        (*self).prev_events()
    }

    fn auth_events(&self) -> &[Self::Id] {
        (*self).auth_events()
    }

    fn redacts(&self) -> Option<&Self::Id> {
        (*self).redacts()
    }

    fn rejected(&self) -> bool {
        (*self).rejected()
    }
}

impl<T: Event> Event for Arc<T> {
    type Id = T::Id;

    fn event_id(&self) -> &Self::Id {
        (**self).event_id()
    }

    fn room_id(&self) -> &RoomId {
        (**self).room_id()
    }

    fn sender(&self) -> &UserId {
        (**self).sender()
    }

    fn origin_server_ts(&self) -> UnixMillis {
        (**self).origin_server_ts()
    }

    fn event_type(&self) -> &TimelineEventType {
        (**self).event_type()
    }

    fn content(&self) -> &RawJsonValue {
        (**self).content()
    }

    fn state_key(&self) -> Option<&str> {
        (**self).state_key()
    }

    fn prev_events(&self) -> &[Self::Id] {
        (**self).prev_events()
    }

    fn auth_events(&self) -> &[Self::Id] {
        (**self).auth_events()
    }

    fn redacts(&self) -> Option<&Self::Id> {
        (**self).redacts()
    }

    fn rejected(&self) -> bool {
        (**self).rejected()
    }
}
