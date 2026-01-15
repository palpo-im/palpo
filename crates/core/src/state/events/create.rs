//! Types to deserialize `m.room.create` events.

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Deref;

use serde::Deserialize;
use serde::de::IgnoredAny;

use super::Event;
use crate::room_version_rules::RoomVersionRules;
use crate::serde::from_raw_json_value;
use crate::state::StateResult;
use crate::{OwnedUserId, RoomVersionId, UserId};

/// A helper type for an [`Event`] of type `m.room.create`.
///
/// This is a type that deserializes each field lazily, when requested.
#[derive(Debug, Clone)]
pub struct RoomCreateEvent<E: Event>(E);

impl<E: Event> RoomCreateEvent<E> {
    /// Construct a new `RoomCreateEvent` around the given event.
    pub fn new(event: E) -> Self {
        Self(event)
    }

    /// The version of the room.
    pub fn room_version(&self) -> StateResult<RoomVersionId> {
        #[derive(Deserialize)]
        struct RoomCreateContentRoomVersion {
            room_version: Option<RoomVersionId>,
        }

        let content: RoomCreateContentRoomVersion =
            from_raw_json_value(self.content()).map_err(|err: serde_json::Error| {
                format!("invalid `room_version` field in `m.room.create` event: {err}")
            })?;
        Ok(content.room_version.unwrap_or(RoomVersionId::V1))
    }

    pub fn version_rules(&self) -> StateResult<RoomVersionRules> {
        let room_version = self.room_version()?;
        room_version.rules().ok_or_else(|| {
            format!("unsupported room version `{room_version}` in `m.room.create` event").into()
        })
    }

    /// Whether the room is federated.
    pub fn federate(&self) -> StateResult<bool> {
        #[derive(Deserialize)]
        struct RoomCreateContentFederate {
            #[serde(rename = "m.federate")]
            federate: Option<bool>,
        }

        let content: RoomCreateContentFederate =
            from_raw_json_value(self.content()).map_err(|err: serde_json::Error| {
                format!("invalid `m.federate` field in `m.room.create` event: {err}")
            })?;
        Ok(content.federate.unwrap_or(true))
    }

    /// The creator of the room.
    ///
    /// If the `use_room_create_sender` field of `AuthorizationRules` is set, the creator is the
    /// sender of this `m.room.create` event, otherwise it is deserialized from the `creator`
    /// field of this event's content.
    ///
    /// This function ignores any `content.additional_creators`, and should only be used in
    /// `check_room_member_join`. Otherwise, you should use `creators` instead.
    pub fn creator(&self) -> StateResult<Cow<'_, UserId>> {
        #[derive(Deserialize)]
        struct RoomCreateContentCreator {
            creator: OwnedUserId,
        }

        let rules = self.version_rules()?.authorization;
        if rules.use_room_create_sender {
            Ok(Cow::Borrowed(self.sender()))
        } else {
            let content: RoomCreateContentCreator =
                from_raw_json_value(self.content()).map_err(|err: serde_json::Error| {
                    format!("missing or invalid `creator` field in `m.room.create` event: {err}")
                })?;

            Ok(Cow::Owned(content.creator))
        }
    }

    /// The additional creators of the room (if any).
    ///
    /// If the `explicitly_privilege_room_creators`
    /// field of `AuthorizationRules` is set, any additional user IDs in `additional_creators`, if
    /// present, will also be considered creators.
    ///
    /// This function ignores the primary room creator, and should only be used in
    /// `check_room_member_join`. Otherwise, you should use `creators` instead.
    pub fn additional_creators(&self) -> StateResult<HashSet<OwnedUserId>> {
        #[derive(Deserialize)]
        struct RoomCreateContentAdditionalCreators {
            #[serde(default)]
            additional_creators: HashSet<OwnedUserId>,
        }

        let rules = self.version_rules()?.authorization;
        Ok(if rules.additional_room_creators {
            let content: RoomCreateContentAdditionalCreators = from_raw_json_value(self.content())
                .map_err(|err: serde_json::Error| {
                    format!("invalid `additional_creators` field in `m.room.create` event: {err}")
                })?;

            content.additional_creators
        } else {
            HashSet::new()
        })
    }

    /// The creators of the room.
    ///
    /// If the `use_room_create_sender` field of `AuthorizationRules` is set,
    /// the creator is the sender of this `m.room.create` event, otherwise it
    /// is deserialized from the `creator` field of this event's content.
    /// Additionally if the `explicitly_privilege_room_creators`
    /// field of `AuthorizationRules` is set, any additional user IDs in
    /// `additional_creators`, if present, will also be considered creators.
    pub fn creators(&self) -> StateResult<HashSet<OwnedUserId>> {
        let mut creators = self.additional_creators()?;

        creators.insert(self.creator()?.into_owned());

        Ok(creators)
    }

    /// Whether the event has a `creator` field.
    pub(crate) fn has_creator(&self) -> StateResult<bool> {
        #[derive(Deserialize)]
        struct RoomCreateContentCreator {
            creator: Option<IgnoredAny>,
        }

        let content: RoomCreateContentCreator =
            from_raw_json_value(self.content()).map_err(|err: serde_json::Error| {
                format!("invalid `creator` field in `m.room.create` event: {err}")
            })?;
        Ok(content.creator.is_some())
    }

    pub fn into_inner(self) -> E {
        self.0
    }
}

impl<E: Event> Deref for RoomCreateEvent<E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
