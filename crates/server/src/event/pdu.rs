use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};
use serde_json::{json, value::to_raw_value};

use crate::core::client::filter::RoomEventFilter;
use crate::core::events::room::history_visibility::{
    HistoryVisibility, RoomHistoryVisibilityEventContent,
};
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::room::redaction::RoomRedactionEventContent;
use crate::core::events::space::child::HierarchySpaceChildEvent;
use crate::core::events::{
    AnyMessageLikeEvent, AnyStateEvent, AnyStrippedStateEvent, AnySyncStateEvent,
    AnySyncTimelineEvent, AnyTimelineEvent, MessageLikeEventContent, StateEvent, StateEventContent,
    StateEventType, TimelineEventType,
};
use crate::core::identifiers::*;
use crate::core::serde::{
    CanonicalJsonObject, CanonicalJsonValue, JsonValue, RawJson, RawJsonValue,
};
use crate::core::{Seqnum, UnixMillis, UserId};
use crate::room::state;
use crate::{AppError, AppResult, room};

/// Content hashes of a PDU.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EventHash {
    /// The SHA-256 hash.
    pub sha256: String,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct SnPduEvent {
    #[serde(flatten)]
    pub pdu: PduEvent,
    #[serde(skip_serializing)]
    pub event_sn: Seqnum,
}
impl SnPduEvent {
    pub fn new(pdu: PduEvent, event_sn: Seqnum) -> Self {
        Self { pdu, event_sn }
    }

    pub fn user_can_see(&self, user_id: &UserId) -> AppResult<bool> {
        if self.event_ty == TimelineEventType::RoomMember
            && self.state_key.as_deref() == Some(user_id.as_str())
        {
            return Ok(true);
        }
        if self.is_room_state() {
            if room::is_world_readable(&self.room_id) {
                return Ok(!room::user::is_banned(user_id, &self.room_id)?);
            } else if room::user::is_joined(user_id, &self.room_id)? {
                return Ok(true);
            }
        }
        let Ok(frame_id) = state::get_pdu_frame_id(&self.event_id) else {
            return Ok(false);
        };

        if let Some(visibility) = state::USER_VISIBILITY_CACHE
            .lock()
            .unwrap()
            .get_mut(&(user_id.to_owned(), frame_id))
        {
            return Ok(*visibility);
        }

        let history_visibility = state::get_state_content::<RoomHistoryVisibilityEventContent>(
            frame_id,
            &StateEventType::RoomHistoryVisibility,
            "",
        )
        .map_or(
            HistoryVisibility::Shared,
            |c: RoomHistoryVisibilityEventContent| c.history_visibility,
        );

        let visibility = match history_visibility {
            HistoryVisibility::WorldReadable => true,
            HistoryVisibility::Shared => {
                let Ok(membership) = state::user_membership(frame_id, user_id) else {
                    return crate::room::user::is_joined(user_id, &self.room_id);
                };
                membership == MembershipState::Join
                    || crate::room::user::is_joined(user_id, &self.room_id)?
            }
            HistoryVisibility::Invited => {
                // Allow if any member on requesting server was AT LEAST invited, else deny
                state::user_was_invited(frame_id, user_id)
            }
            HistoryVisibility::Joined => {
                // Allow if any member on requested server was joined, else deny
                state::user_was_joined(frame_id, user_id)
                    || state::user_was_joined(frame_id - 1, user_id)
            }
            _ => {
                error!("unknown history visibility {history_visibility}");
                false
            }
        };

        state::USER_VISIBILITY_CACHE
            .lock()
            .expect("should locked")
            .insert((user_id.to_owned(), frame_id), visibility);
        Ok(visibility)
    }

    pub fn add_unsigned_membership(&mut self, user_id: &UserId) -> AppResult<()> {
        #[derive(Deserialize)]
        struct ExtractMemebership {
            membership: String,
        }
        let membership = if self.event_ty == TimelineEventType::RoomMember
            && self.state_key == Some(user_id.to_string())
        {
            self.get_content::<ExtractMemebership>()
                .map(|m| m.membership)
                .ok()
        } else if let Ok(frame_id) = crate::event::get_frame_id(&self.room_id, self.event_sn) {
            state::user_membership(frame_id, user_id)
                .ok()
                .map(|m| m.to_string())
        } else {
            None
        };
        if let Some(membership) = membership {
            self.unsigned.insert(
                "membership".to_owned(),
                to_raw_value(&membership).expect("should always work"),
            );
        } else {
            self.unsigned.insert(
                "membership".to_owned(),
                to_raw_value("leave").expect("should always work"),
            );
        }
        Ok(())
    }

    pub fn from_canonical_object(
        event_id: &EventId,
        event_sn: Seqnum,
        json: CanonicalJsonObject,
    ) -> Result<Self, serde_json::Error> {
        let pdu = PduEvent::from_canonical_object(event_id, json)?;
        Ok(Self::new(pdu, event_sn))
    }

    pub fn from_json_value(
        event_id: &EventId,
        event_sn: Seqnum,
        json: JsonValue,
    ) -> AppResult<Self> {
        let pdu = PduEvent::from_json_value(event_id, json)?;
        Ok(Self::new(pdu, event_sn))
    }
}
impl AsRef<PduEvent> for SnPduEvent {
    fn as_ref(&self) -> &PduEvent {
        &self.pdu
    }
}
impl AsMut<PduEvent> for SnPduEvent {
    fn as_mut(&mut self) -> &mut PduEvent {
        &mut self.pdu
    }
}
impl DerefMut for SnPduEvent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pdu
    }
}
impl Deref for SnPduEvent {
    type Target = PduEvent;

    fn deref(&self) -> &Self::Target {
        &self.pdu
    }
}
impl TryFrom<(PduEvent, Option<Seqnum>)> for SnPduEvent {
    type Error = AppError;

    fn try_from((pdu, event_sn): (PduEvent, Option<Seqnum>)) -> Result<Self, Self::Error> {
        if let Some(sn) = event_sn {
            Ok(SnPduEvent::new(pdu, sn))
        } else {
            Err(AppError::internal(
                "Cannot convert PDU without event_sn to SnPduEvent.",
            ))
        }
    }
}
impl crate::core::state::Event for SnPduEvent {
    type Id = OwnedEventId;

    fn event_id(&self) -> &Self::Id {
        &self.event_id
    }

    fn room_id(&self) -> &RoomId {
        &self.room_id
    }

    fn sender(&self) -> &UserId {
        &self.sender
    }

    fn event_type(&self) -> &TimelineEventType {
        &self.event_ty
    }

    fn content(&self) -> &RawJsonValue {
        &self.content
    }

    fn origin_server_ts(&self) -> UnixMillis {
        self.origin_server_ts
    }

    fn state_key(&self) -> Option<&str> {
        self.state_key.as_deref()
    }

    fn prev_events(&self) -> &[Self::Id] {
        self.prev_events.deref()
    }

    fn auth_events(&self) -> &[Self::Id] {
        self.auth_events.deref()
    }

    fn redacts(&self) -> Option<&Self::Id> {
        self.redacts.as_ref()
    }

    fn rejected(&self) -> bool {
        self.rejection_reason.is_some()
    }
}

// These impl's allow us to dedup state snapshots when resolving state
// for incoming events (federation/send/{txn}).
impl Eq for SnPduEvent {}
impl PartialEq for SnPduEvent {
    fn eq(&self, other: &Self) -> bool {
        self.event_id == other.event_id
    }
}
impl PartialOrd for SnPduEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // self.event_id.partial_cmp(&other.event_id)
        Some(self.cmp(other))
    }
}
impl Ord for SnPduEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        self.event_id.cmp(&other.event_id)
    }
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct PduEvent {
    pub event_id: OwnedEventId,
    #[serde(rename = "type")]
    pub event_ty: TimelineEventType,
    pub room_id: OwnedRoomId,
    pub sender: OwnedUserId,
    pub origin_server_ts: UnixMillis,
    pub content: Box<RawJsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_key: Option<String>,
    pub prev_events: Vec<OwnedEventId>,
    pub depth: u64,
    pub auth_events: Vec<OwnedEventId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacts: Option<OwnedEventId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unsigned: BTreeMap<String, Box<RawJsonValue>>,
    pub hashes: EventHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signatures: Option<Box<RawJsonValue>>, // BTreeMap<Box<ServerName>, BTreeMap<ServerSigningKeyId, String>>
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra_data: BTreeMap<String, JsonValue>,

    #[serde(skip)]
    pub rejection_reason: Option<String>,
}

impl PduEvent {
    #[tracing::instrument]
    pub fn redact(&mut self, reason: &PduEvent) -> AppResult<()> {
        let allowed: &[&str] = match self.event_ty {
            TimelineEventType::RoomMember => &["join_authorised_via_users_server", "membership"],
            TimelineEventType::RoomCreate => &["creator"],
            TimelineEventType::RoomJoinRules => &["join_rule"],
            TimelineEventType::RoomPowerLevels => &[
                "ban",
                "events",
                "events_default",
                "kick",
                "redact",
                "state_default",
                "users",
                "users_default",
            ],
            TimelineEventType::RoomHistoryVisibility => &["history_visibility"],
            _ => &[],
        };

        let mut old_content = self
            .get_content::<BTreeMap<String, serde_json::Value>>()
            .map_err(|_| AppError::internal("PDU in db has invalid content."))?;

        let mut new_content = serde_json::Map::new();

        for key in allowed {
            if let Some(value) = old_content.remove(*key) {
                new_content.insert((*key).to_owned(), value);
            }
        }

        self.unsigned = BTreeMap::new();
        self.unsigned.insert(
            "redacted_because".to_owned(),
            to_raw_value(reason).expect("to_raw_value(PduEvent) always works"),
        );

        self.content = to_raw_value(&new_content).expect("to string always works");

        Ok(())
    }

    pub fn redacts_id(&self, room_version: &RoomVersionId) -> Option<OwnedEventId> {
        use RoomVersionId::*;

        if self.event_ty != TimelineEventType::RoomRedaction {
            return None;
        }

        match *room_version {
            V1 | V2 | V3 | V4 | V5 | V6 | V7 | V8 | V9 | V10 => self.redacts.clone(),
            _ => {
                self.get_content::<RoomRedactionEventContent>()
                    .ok()?
                    .redacts
            }
        }
    }

    pub fn remove_transaction_id(&mut self) -> AppResult<()> {
        self.unsigned.remove("transaction_id");
        Ok(())
    }

    pub fn add_age(&mut self) -> AppResult<()> {
        let now: i128 = UnixMillis::now().get().into();
        let then: i128 = self.origin_server_ts.get().into();
        let age = now.saturating_sub(then);

        self.unsigned
            .insert("age".to_owned(), to_raw_value(&age).unwrap());

        Ok(())
    }

    #[tracing::instrument]
    pub fn to_sync_room_event(&self) -> RawJson<AnySyncTimelineEvent> {
        let mut json = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
        });

        if !self.unsigned.is_empty() {
            json["unsigned"] = json!(self.unsigned);
        }
        if let Some(state_key) = &self.state_key {
            json["state_key"] = json!(state_key);
        }
        if let Some(redacts) = &self.redacts {
            json["redacts"] = json!(redacts);
        }

        serde_json::from_value(json).expect("RawJson::from_value always works")
    }

    #[tracing::instrument]
    pub fn to_room_event(&self) -> RawJson<AnyTimelineEvent> {
        let mut data = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
            "room_id": self.room_id,
        });

        if !self.unsigned.is_empty() {
            data["unsigned"] = json!(self.unsigned);
        }
        if let Some(state_key) = &self.state_key {
            data["state_key"] = json!(state_key);
        }
        if let Some(redacts) = &self.redacts {
            data["redacts"] = json!(redacts);
        }

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    #[tracing::instrument]
    pub fn to_message_like_event(&self) -> RawJson<AnyMessageLikeEvent> {
        let mut data = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
            "room_id": self.room_id,
        });

        if !self.unsigned.is_empty() {
            data["unsigned"] = json!(self.unsigned);
        }
        if let Some(state_key) = &self.state_key {
            data["state_key"] = json!(state_key);
        }
        if let Some(redacts) = &self.redacts {
            data["redacts"] = json!(redacts);
        }

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    #[tracing::instrument]
    pub fn to_state_event(&self) -> RawJson<AnyStateEvent> {
        serde_json::from_value(self.to_state_event_value())
            .expect("RawJson::from_value always works")
    }
    #[tracing::instrument]
    pub fn to_state_event_value(&self) -> JsonValue {
        let JsonValue::Object(mut data) = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
            "room_id": self.room_id,
            "state_key": self.state_key,
        }) else {
            panic!("Invalid JSON value, never happened!");
        };

        if !self.unsigned.is_empty() {
            data.insert("unsigned".into(), json!(self.unsigned));
        }

        for (key, value) in &self.extra_data {
            if !data.contains_key(key) {
                data.insert(key.clone(), value.clone());
            }
        }

        JsonValue::Object(data)
    }

    #[tracing::instrument]
    pub fn to_sync_state_event(&self) -> RawJson<AnySyncStateEvent> {
        let mut data = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
            "state_key": self.state_key,
        });

        if !self.unsigned.is_empty() {
            data["unsigned"] = json!(self.unsigned);
        }

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    #[tracing::instrument]
    pub fn to_stripped_state_event(&self) -> RawJson<AnyStrippedStateEvent> {
        let data = json!({
            "content": self.content,
            "type": self.event_ty,
            "sender": self.sender,
            "state_key": self.state_key,
        });

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    #[tracing::instrument]
    pub fn to_stripped_space_child_event(&self) -> RawJson<HierarchySpaceChildEvent> {
        let data = json!({
            "content": self.content,
            "type": self.event_ty,
            "sender": self.sender,
            "state_key": self.state_key,
            "origin_server_ts": self.origin_server_ts,
        });

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    #[tracing::instrument]
    pub fn to_member_event(&self) -> RawJson<StateEvent<RoomMemberEventContent>> {
        let mut data = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
            "redacts": self.redacts,
            "room_id": self.room_id,
            "state_key": self.state_key,
        });

        if !self.unsigned.is_empty() {
            data["unsigned"] = json!(self.unsigned);
        }

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    pub fn from_canonical_object(
        event_id: &EventId,
        mut json: CanonicalJsonObject,
    ) -> Result<Self, serde_json::Error> {
        json.insert(
            "event_id".to_owned(),
            CanonicalJsonValue::String(event_id.as_str().to_owned()),
        );

        serde_json::from_value(serde_json::to_value(json).expect("valid JSON"))
    }

    pub fn from_json_value(event_id: &EventId, json: JsonValue) -> AppResult<Self> {
        if let JsonValue::Object(mut obj) = json {
            obj.insert("event_id".to_owned(), event_id.as_str().into());

            serde_json::from_value(serde_json::Value::Object(obj)).map_err(Into::into)
        } else {
            Err(AppError::public("Invalid JSON value"))
        }
    }

    pub fn get_content<T>(&self) -> Result<T, serde_json::Error>
    where
        T: for<'de> Deserialize<'de>,
    {
        serde_json::from_str(self.content.get())
    }

    pub fn is_rejected(&self) -> bool {
        self.rejection_reason.is_some()
    }

    pub fn is_room_state(&self) -> bool {
        self.state_key.as_deref() == Some("")
    }
    pub fn is_user_state(&self) -> bool {
        self.state_key.is_some() && self.state_key.as_deref() != Some("")
    }

    pub fn can_pass_filter(&self, filter: &RoomEventFilter) -> bool {
        if filter.not_types.contains(&self.event_ty.to_string()) {
            return false;
        }
        if filter.not_rooms.contains(&self.room_id) {
            return false;
        }
        if filter.not_senders.contains(&self.sender) {
            return false;
        }

        if let Some(rooms) = &filter.rooms
            && !rooms.contains(&self.room_id)
        {
            return false;
        }
        if let Some(senders) = &filter.senders
            && !senders.contains(&self.sender)
        {
            return false;
        }
        if let Some(types) = &filter.types
            && !types.contains(&self.event_ty.to_string())
        {
            return false;
        }
        // TODO: url filter
        // if let Some(url_filter) = &filter.url_filter {
        //     match url_filter {
        //         UrlFilter::EventsWithUrl => if !self.events::contains_url.eq(true)),
        //         UrlFilter::EventsWithoutUrl => query = query.filter(events::contains_url.eq(false)),
        //     }
        // }

        true
    }
}

impl crate::core::state::Event for PduEvent {
    type Id = OwnedEventId;

    fn event_id(&self) -> &Self::Id {
        &self.event_id
    }

    fn room_id(&self) -> &RoomId {
        &self.room_id
    }

    fn sender(&self) -> &UserId {
        &self.sender
    }

    fn event_type(&self) -> &TimelineEventType {
        &self.event_ty
    }

    fn content(&self) -> &RawJsonValue {
        &self.content
    }

    fn origin_server_ts(&self) -> UnixMillis {
        self.origin_server_ts
    }

    fn state_key(&self) -> Option<&str> {
        self.state_key.as_deref()
    }

    fn prev_events(&self) -> &[Self::Id] {
        self.prev_events.deref()
    }

    fn auth_events(&self) -> &[Self::Id] {
        self.auth_events.deref()
    }

    fn redacts(&self) -> Option<&Self::Id> {
        self.redacts.as_ref()
    }

    fn rejected(&self) -> bool {
        self.rejection_reason.is_some()
    }
}

// These impl's allow us to dedup state snapshots when resolving state
// for incoming events (federation/send/{txn}).
impl Eq for PduEvent {}
impl PartialEq for PduEvent {
    fn eq(&self, other: &Self) -> bool {
        self.event_id == other.event_id
    }
}
impl PartialOrd for PduEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // self.event_id.partial_cmp(&other.event_id)
        Some(self.cmp(other))
    }
}
impl Ord for PduEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        self.event_id.cmp(&other.event_id)
    }
}

/// Build the start of a PDU in order to add it to the Database.
#[derive(Debug, Deserialize)]
pub struct PduBuilder {
    #[serde(rename = "type")]
    pub event_type: TimelineEventType,
    pub content: Box<RawJsonValue>,
    #[serde(default)]
    pub unsigned: BTreeMap<String, Box<RawJsonValue>>,
    pub state_key: Option<String>,
    pub redacts: Option<OwnedEventId>,
    pub timestamp: Option<UnixMillis>,
}

impl PduBuilder {
    pub fn state<T>(state_key: String, content: &T) -> Self
    where
        T: StateEventContent,
    {
        Self {
            event_type: content.event_type().into(),
            content: to_raw_value(content)
                .expect("Builder failed to serialize state event content to RawValue"),
            state_key: Some(state_key),
            ..Self::default()
        }
    }

    pub fn timeline<T>(content: &T) -> Self
    where
        T: MessageLikeEventContent,
    {
        Self {
            event_type: content.event_type().into(),
            content: to_raw_value(content)
                .expect("Builder failed to serialize timeline event content to RawValue"),
            ..Self::default()
        }
    }
}

impl Default for PduBuilder {
    fn default() -> Self {
        Self {
            event_type: "m.room.message".into(),
            content: Box::<RawJsonValue>::default(),
            unsigned: Default::default(),
            state_key: None,
            redacts: None,
            timestamp: None,
        }
    }
}
