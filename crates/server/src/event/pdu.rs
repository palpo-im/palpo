use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::value::to_raw_value;
use ulid::Ulid;

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
use crate::core::room_version_rules::RoomIdFormatVersion;
use crate::core::serde::{
    CanonicalJsonObject, CanonicalJsonValue, JsonValue, RawJson, RawJsonValue, default_false,
    to_canonical_object, to_canonical_value, validate_canonical_json,
};
use crate::core::state::{StateError, event_auth};
use crate::core::{Seqnum, UnixMillis, UserId};
use crate::data::connect;
use crate::data::room::{DbEventData, NewDbEvent};
use crate::data::schema::*;
use crate::event::{BatchToken, SeqnumQueueGuard};
use crate::room::state;
use crate::room::timeline::get_pdu;
use crate::{AppError, AppResult, MatrixError, RoomMutexGuard, room};

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

    #[serde(skip, default)]
    pub is_outlier: bool,
    #[serde(skip, default = "default_false")]
    pub soft_failed: bool,
    #[serde(skip, default = "default_false")]
    pub is_backfill: bool,
}
impl SnPduEvent {
    pub fn new(
        pdu: PduEvent,
        event_sn: Seqnum,
        is_outlier: bool,
        soft_failed: bool,
        is_backfill: bool,
    ) -> Self {
        Self {
            pdu,
            event_sn,
            is_outlier,
            soft_failed,
            is_backfill,
        }
    }

    pub async fn user_can_see(&self, user_id: &UserId) -> AppResult<bool> {
        // Clients must always be able to observe their own membership transitions.
        // In particular, a `knock` -> `leave` transition would otherwise be hidden
        // by shared history visibility because neither side is a joined membership.
        // The event only describes the requesting user's own membership/profile.
        if self.event_ty == TimelineEventType::RoomMember
            && self.state_key.as_deref() == Some(user_id.as_str())
        {
            return Ok(true);
        }

        let frame_id = match state::get_pdu_before_frame_id(&self.event_id).await {
            Ok(frame_id) => frame_id,
            // Non-state event frames have always been immutable because `save_state`
            // only rewrites events present in the state map. They are a safe fallback
            // for data written before `before_frame_id` existed. Legacy state events
            // deliberately fail closed instead of risking future-state disclosure.
            Err(e) if e.is_not_found() && self.state_key.is_none() => {
                match state::get_pdu_frame_id(&self.event_id).await {
                    Ok(frame_id) => frame_id,
                    Err(e) if e.is_not_found() => return Ok(false),
                    Err(e) => return Err(e),
                }
            }
            Err(e) if e.is_not_found() => return Ok(false),
            Err(e) => return Err(e),
        };
        let state::StateBefore::Resolved(history_visibility) =
            state::history_visibility_before(self, frame_id).await?
        else {
            return Ok(false);
        };
        let after_history_visibility = (self.event_ty == TimelineEventType::RoomHistoryVisibility)
            .then(|| {
                self.get_content::<RoomHistoryVisibilityEventContent>()
                    .map(|content| content.history_visibility)
                    .unwrap_or(HistoryVisibility::Shared)
            });
        if history_visibility == HistoryVisibility::WorldReadable
            || after_history_visibility == Some(HistoryVisibility::WorldReadable)
        {
            return Ok(true);
        }
        let after_membership = (self.event_ty == TimelineEventType::RoomMember
            && self.state_key.as_deref() == Some(user_id.as_str()))
        .then(|| {
            self.get_content::<RoomMemberEventContent>()
                .ok()
                .map(|content| content.membership)
        })
        .flatten();
        let uses_shared_visibility = state::uses_shared_history_visibility(&history_visibility)
            || after_history_visibility
                .as_ref()
                .is_some_and(state::uses_shared_history_visibility);
        let state::StateBefore::Resolved(membership) =
            state::user_membership_before(self, frame_id, user_id).await?
        else {
            return Ok(false);
        };
        // A user joined at the event already satisfies every non-world-readable
        // visibility rule. Avoid the considerably more expensive ancestry lookup on
        // this overwhelmingly common path.
        if membership.as_ref() == Some(&MembershipState::Join) {
            return Ok(true);
        }
        let joined_after = uses_shared_visibility
            && room::user::joined_after(user_id, &self.room_id, &self.event_id, self.depth).await?;

        Ok(
            state::history_visibility_allows(
                &history_visibility,
                membership.as_ref(),
                joined_after,
            ) || after_history_visibility.as_ref().is_some_and(|visibility| {
                state::history_visibility_allows(visibility, membership.as_ref(), joined_after)
            }) || after_membership.as_ref().is_some_and(|membership| {
                state::history_visibility_allows(
                    &history_visibility,
                    Some(membership),
                    joined_after,
                )
            }),
        )
    }

    pub async fn add_unsigned_membership(&mut self, user_id: &UserId) -> AppResult<()> {
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
        } else if let Ok(frame_id) = crate::event::get_frame_id(&self.room_id, self.event_sn).await
        {
            state::user_membership(frame_id, user_id)
                .await
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
        room_id: &RoomId,
        event_id: &EventId,
        event_sn: Seqnum,
        json: CanonicalJsonObject,
        is_outlier: bool,
        soft_failed: bool,
        is_backfill: bool,
    ) -> Result<Self, serde_json::Error> {
        let pdu = PduEvent::from_canonical_object(room_id, event_id, json)?;
        Ok(Self::new(
            pdu,
            event_sn,
            is_outlier,
            soft_failed,
            is_backfill,
        ))
    }

    pub fn from_json_value(
        room_id: &RoomId,
        event_id: &EventId,
        event_sn: Seqnum,
        json: JsonValue,
        is_outlier: bool,
        soft_failed: bool,
        is_backfill: bool,
    ) -> AppResult<Self> {
        let pdu = PduEvent::from_json_value(room_id, event_id, json)?;
        Ok(Self::new(
            pdu,
            event_sn,
            is_outlier,
            soft_failed,
            is_backfill,
        ))
    }

    pub fn into_inner(self) -> PduEvent {
        self.pdu
    }

    pub fn live_token(&self) -> BatchToken {
        BatchToken::Live {
            stream_ordering: self.event_sn,
        }
    }
    pub fn historic_token(&self) -> BatchToken {
        BatchToken::Historic {
            stream_ordering: if self.is_backfill {
                -self.event_sn
            } else {
                self.event_sn
            },
            topological_ordering: self.depth as i64,
        }
    }
    pub fn prev_historic_token(&self) -> BatchToken {
        BatchToken::Historic {
            stream_ordering: if self.is_backfill {
                -self.event_sn - 1
            } else {
                self.event_sn - 1
            },
            topological_ordering: self.depth as i64,
        }
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
// impl TryFrom<(PduEvent, Option<Seqnum>)> for SnPduEvent {
//     type Error = AppError;

//     fn try_from((pdu, event_sn): (PduEvent, Option<Seqnum>)) -> Result<Self, Self::Error> {
//         if let Some(sn) = event_sn {
//             Ok(SnPduEvent::new(pdu, sn))
//         } else {
//             Err(AppError::internal(
//                 "Cannot convert PDU without event_sn to SnPduEvent.",
//             ))
//         }
//     }
// }
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
        self.pdu.rejected()
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
    #[serde(default)]
    pub prev_events: Vec<OwnedEventId>,
    pub depth: u64,
    #[serde(default)]
    pub auth_events: Vec<OwnedEventId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacts: Option<OwnedEventId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unsigned: BTreeMap<String, Box<RawJsonValue>>,
    pub hashes: EventHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signatures: Option<Box<RawJsonValue>>, /* BTreeMap<Box<ServerName>,
                                                * BTreeMap<ServerSigningKeyId, String>> */
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra_data: BTreeMap<String, JsonValue>,

    #[serde(skip, default)]
    pub rejection_reason: Option<String>,

    // Trusted local provenance, never accepted from or emitted into event JSON.
    #[serde(skip)]
    pub(crate) transaction_device: Option<OwnedDeviceId>,
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

    fn unsigned_for_recipient(
        &self,
        recipient: &UserId,
        device_id: Option<&DeviceId>,
    ) -> Cow<'_, BTreeMap<String, Box<RawJsonValue>>> {
        let originating_device = self.sender == recipient
            && device_id.is_some()
            && self.transaction_device.as_deref() == device_id;
        if originating_device
            && !self.unsigned.contains_key("redacted_because")
            && !self.unsigned.contains_key("m.relations")
        {
            Cow::Borrowed(&self.unsigned)
        } else {
            let mut unsigned = self.unsigned_without_transaction_id();
            if originating_device && let Some(txn) = self.unsigned.get("transaction_id") {
                unsigned.insert("transaction_id".into(), txn.clone());
            }
            Cow::Owned(unsigned)
        }
    }

    fn unsigned_without_transaction_id(&self) -> BTreeMap<String, Box<RawJsonValue>> {
        let mut unsigned = self.unsigned.clone();
        unsigned.remove("transaction_id");
        // Embedded events have independent senders and devices. Old stored bundles
        // may predate the privacy filtering at their creation sites.
        for key in ["redacted_because", "m.relations"] {
            if let Some(raw) = unsigned.remove(key)
                && let Ok(mut value) = serde_json::from_str::<JsonValue>(raw.get())
            {
                strip_embedded_transaction_ids(&mut value);
                unsigned.insert(key.into(), to_raw_value(&value).expect("valid JSON"));
            }
        }
        unsigned
    }

    fn transaction_metadata(&self) -> Option<JsonValue> {
        let device = self.transaction_device.as_ref()?;
        let txn: OwnedTransactionId =
            serde_json::from_str(self.unsigned.get("transaction_id")?.get()).ok()?;
        Some(
            json!({"transaction_device": device, "transaction_id": txn, "transaction_user": self.sender}),
        )
    }

    /// Hydrate device provenance only from trusted local metadata or a legacy idempotency record.
    pub(crate) async fn load_transaction_device(&mut self) -> AppResult<()> {
        self.transaction_device = None;
        if let Some(txn_id) = self.unsigned.get("transaction_id")
            && let Ok(txn_id) = serde_json::from_str::<OwnedTransactionId>(txn_id.get())
        {
            self.transaction_device = crate::data::room::transaction_id::get_event_device(
                &txn_id,
                &self.sender,
                &self.room_id,
                &self.event_id,
            )
            .await?;
        }
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

    fn to_sync_room_event_with_unsigned(
        &self,
        unsigned: &BTreeMap<String, Box<RawJsonValue>>,
    ) -> RawJson<AnySyncTimelineEvent> {
        let mut json = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
        });

        if !unsigned.is_empty() {
            json["unsigned"] = json!(unsigned);
        }
        if let Some(state_key) = &self.state_key {
            json["state_key"] = json!(state_key);
        }
        if let Some(redacts) = &self.redacts {
            json["redacts"] = json!(redacts);
        }

        serde_json::from_value(json).expect("RawJson::from_value always works")
    }

    pub fn to_sync_room_event_for(
        &self,
        recipient: &UserId,
        device_id: Option<&DeviceId>,
    ) -> RawJson<AnySyncTimelineEvent> {
        self.to_sync_room_event_with_unsigned(
            self.unsigned_for_recipient(recipient, device_id).as_ref(),
        )
    }

    pub fn to_sync_room_event_without_transaction_id(&self) -> RawJson<AnySyncTimelineEvent> {
        self.to_sync_room_event_with_unsigned(&self.unsigned_without_transaction_id())
    }

    fn to_room_event_with_unsigned(
        &self,
        unsigned: &BTreeMap<String, Box<RawJsonValue>>,
    ) -> RawJson<AnyTimelineEvent> {
        let age = UnixMillis::now()
            .get()
            .saturating_sub(self.origin_server_ts.get());
        let mut data = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
            "room_id": self.room_id,
        });

        if unsigned.is_empty() {
            data["unsigned"] = json!({ "age": age });
        } else {
            let mut unsigned_json = json!(unsigned);
            unsigned_json["age"] = json!(age);
            data["unsigned"] = unsigned_json;
        }
        if let Some(state_key) = &self.state_key {
            data["state_key"] = json!(state_key);
        }
        if let Some(redacts) = &self.redacts {
            data["redacts"] = json!(redacts);
        }

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    pub fn to_room_event_for(
        &self,
        recipient: &UserId,
        device_id: Option<&DeviceId>,
    ) -> RawJson<AnyTimelineEvent> {
        self.to_room_event_with_unsigned(self.unsigned_for_recipient(recipient, device_id).as_ref())
    }

    pub fn to_room_event_without_transaction_id(&self) -> RawJson<AnyTimelineEvent> {
        self.to_room_event_with_unsigned(&self.unsigned_without_transaction_id())
    }

    fn to_message_like_event_with_unsigned(
        &self,
        unsigned: &BTreeMap<String, Box<RawJsonValue>>,
    ) -> RawJson<AnyMessageLikeEvent> {
        let mut data = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
            "room_id": self.room_id,
        });

        if !unsigned.is_empty() {
            data["unsigned"] = json!(unsigned);
        }
        if let Some(state_key) = &self.state_key {
            data["state_key"] = json!(state_key);
        }
        if let Some(redacts) = &self.redacts {
            data["redacts"] = json!(redacts);
        }

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    pub fn to_message_like_event_for(
        &self,
        recipient: &UserId,
        device_id: Option<&DeviceId>,
    ) -> RawJson<AnyMessageLikeEvent> {
        self.to_message_like_event_with_unsigned(
            self.unsigned_for_recipient(recipient, device_id).as_ref(),
        )
    }

    pub fn to_message_like_event_without_transaction_id(&self) -> RawJson<AnyMessageLikeEvent> {
        self.to_message_like_event_with_unsigned(&self.unsigned_without_transaction_id())
    }

    #[tracing::instrument]
    pub fn to_state_event_with_sender_only_unsigned(&self) -> RawJson<AnyStateEvent> {
        serde_json::from_value(self.to_state_event_value_with_unsigned(&self.unsigned))
            .expect("RawJson::from_value always works")
    }

    pub fn to_state_event_for(
        &self,
        recipient: &UserId,
        device_id: Option<&DeviceId>,
    ) -> RawJson<AnyStateEvent> {
        serde_json::from_value(self.to_state_event_value_with_unsigned(
            self.unsigned_for_recipient(recipient, device_id).as_ref(),
        ))
        .expect("RawJson::from_value always works")
    }

    fn to_state_event_value_with_unsigned(
        &self,
        unsigned: &BTreeMap<String, Box<RawJsonValue>>,
    ) -> JsonValue {
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

        if !unsigned.is_empty() {
            data.insert("unsigned".into(), json!(unsigned));
        }

        for (key, value) in &self.extra_data {
            if !data.contains_key(key) {
                data.insert(key.clone(), value.clone());
            }
        }

        JsonValue::Object(data)
    }

    pub fn to_state_event_value_for(
        &self,
        recipient: &UserId,
        device_id: Option<&DeviceId>,
    ) -> JsonValue {
        self.to_state_event_value_with_unsigned(
            self.unsigned_for_recipient(recipient, device_id).as_ref(),
        )
    }

    fn to_sync_state_event_with_unsigned(
        &self,
        unsigned: &BTreeMap<String, Box<RawJsonValue>>,
    ) -> RawJson<AnySyncStateEvent> {
        let mut data = json!({
            "content": self.content,
            "type": self.event_ty,
            "event_id": *self.event_id,
            "sender": self.sender,
            "origin_server_ts": self.origin_server_ts,
            "state_key": self.state_key,
        });

        if !unsigned.is_empty() {
            data["unsigned"] = json!(unsigned);
        }

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    pub fn to_sync_state_event_for(
        &self,
        recipient: &UserId,
        device_id: Option<&DeviceId>,
    ) -> RawJson<AnySyncStateEvent> {
        self.to_sync_state_event_with_unsigned(
            self.unsigned_for_recipient(recipient, device_id).as_ref(),
        )
    }

    #[tracing::instrument]
    pub async fn to_stripped_state_event(&self) -> RawJson<AnyStrippedStateEvent> {
        if self.event_ty == TimelineEventType::RoomCreate {
            let version_rules = crate::room::get_version(&self.room_id)
                .await
                .and_then(|version| crate::room::get_version_rules(&version));
            if let Ok(version_rules) = version_rules
                && version_rules.authorization.room_create_event_id_as_room_id
            {
                return serde_json::from_value(json!(self))
                    .expect("RawJson::from_value always works");
            }
        }
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
    pub fn to_member_event_for(
        &self,
        recipient: &UserId,
        device_id: Option<&DeviceId>,
    ) -> RawJson<StateEvent<RoomMemberEventContent>> {
        let unsigned = self.unsigned_for_recipient(recipient, device_id);
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

        if !unsigned.is_empty() {
            data["unsigned"] = json!(unsigned);
        }

        serde_json::from_value(data).expect("RawJson::from_value always works")
    }

    pub fn from_canonical_object(
        room_id: &RoomId,
        event_id: &EventId,
        mut json: CanonicalJsonObject,
    ) -> Result<Self, serde_json::Error> {
        json.insert("room_id".to_owned(), room_id.as_str().into());
        json.insert(
            "event_id".to_owned(),
            CanonicalJsonValue::String(event_id.as_str().to_owned()),
        );

        serde_json::from_value(serde_json::to_value(json).expect("valid JSON"))
    }

    pub fn from_json_value(
        room_id: &RoomId,
        event_id: &EventId,
        json: JsonValue,
    ) -> AppResult<Self> {
        if let JsonValue::Object(mut obj) = json {
            obj.insert("event_id".to_owned(), event_id.as_str().into());
            obj.insert("room_id".to_owned(), room_id.as_str().into());

            serde_json::from_value(serde_json::Value::Object(obj)).map_err(Into::into)
        } else {
            Err(AppError::public("invalid json value"))
        }
    }

    pub fn get_content<T>(&self) -> Result<T, serde_json::Error>
    where
        T: for<'de> Deserialize<'de>,
    {
        serde_json::from_str(self.content.get())
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
        //         UrlFilter::EventsWithoutUrl => query =
        // query.filter(events::contains_url.eq(false)),     }
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
    /// Authenticated local provenance; never accept this field from JSON.
    #[serde(skip)]
    pub transaction_device: Option<OwnedDeviceId>,
}

impl PduBuilder {
    pub fn state<T>(state_key: String, content: &T) -> Self
    where
        T: StateEventContent,
    {
        Self {
            event_type: content.event_type().into(),
            content: to_raw_value(content)
                .expect("builder failed to serialize state event content to RawValue"),
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
                .expect("builder failed to serialize timeline event content to RawValue"),
            ..Self::default()
        }
    }

    pub async fn hash_sign_save(
        self,
        sender_id: &UserId,
        room_id: &RoomId,
        room_version: &RoomVersionId,
        _state_lock: &RoomMutexGuard,
    ) -> AppResult<(SnPduEvent, CanonicalJsonObject, Option<SeqnumQueueGuard>)> {
        let (pdu, pdu_json) = self.hash_sign(sender_id, room_id, room_version).await?;
        let (event_sn, event_guard) = crate::event::ensure_event_sn(room_id, &pdu.event_id).await?;
        let content_value: JsonValue = serde_json::from_str(pdu.content.get())?;
        let db_event = NewDbEvent {
            id: pdu.event_id.to_owned(),
            sn: event_sn,
            ty: pdu.event_ty.to_string(),
            room_id: pdu.room_id.to_owned(),
            unrecognized_keys: None,
            depth: pdu.depth as i64,
            topological_ordering: pdu.depth as i64,
            stream_ordering: event_sn,
            origin_server_ts: pdu.origin_server_ts,
            received_at: None,
            sender_id: Some(sender_id.to_owned()),
            contains_url: content_value.get("url").is_some(),
            worker_id: None,
            state_key: pdu.state_key.clone(),
            is_outlier: true,
            soft_failed: false,
            is_rejected: false,
            rejection_reason: None,
        };
        let event_data = DbEventData {
            event_id: pdu.event_id.clone(),
            event_sn,
            room_id: pdu.room_id.to_owned(),
            internal_metadata: pdu.transaction_metadata(),
            json_data: serde_json::to_value(&pdu_json)?,
            format_version: None,
        };
        // Store the event metadata and JSON as one unit. Feature-specific indexes which
        // make an outlier intentionally observable can join this transaction rather than
        // racing a separately committed event row.
        connect()
            .await?
            .transaction::<_, AppError, _>(async |conn| {
                db_event.save_with_conn(conn).await?;
                event_data.save_with_conn(conn).await?;
                Ok(())
            })
            .await?;

        Ok((
            SnPduEvent {
                pdu,
                event_sn,
                is_outlier: true,
                soft_failed: false,
                is_backfill: false,
            },
            pdu_json,
            event_guard,
        ))
    }

    pub async fn hash_sign(
        self,
        sender_id: &UserId,
        room_id: &RoomId,
        room_version: &RoomVersionId,
    ) -> AppResult<(PduEvent, CanonicalJsonObject)> {
        let PduBuilder {
            event_type,
            content,
            mut unsigned,
            state_key,
            redacts,
            timestamp,
            transaction_device,
            ..
        } = self;

        let prev_events: Vec<_> = state::get_forward_extremities(room_id)
            .await?
            .into_iter()
            .take(20)
            .collect();

        let conf = crate::config::get();
        // If there was no create event yet, assume we are creating a room with the default
        // version right now
        // let room_version = if let Ok(room_version) = super::get_version(room_id) {
        //     room_version
        // } else if event_type == TimelineEventType::RoomCreate {
        //     let content: RoomCreateEventContent = serde_json::from_str(content.get())?;
        //     content.room_version
        // } else {
        //     return Err(AppError::public(format!(
        //         "non-create event for room `{room_id}` of unknown version"
        //     )));
        // };
        let version_rules = crate::room::get_version_rules(room_version)?;
        let auth_rules = &version_rules.authorization;

        let auth_events = state::get_auth_events(
            room_id,
            &event_type,
            sender_id,
            state_key.as_deref(),
            &content,
            auth_rules,
        )
        .await?;

        // Our depth is the maximum depth of prev_events + 1
        let mut max_depth = 0;
        for event_id in &prev_events {
            if let Ok(prev_pdu) = get_pdu(event_id).await {
                max_depth = max_depth.max(prev_pdu.depth);
            }
        }
        let depth = max_depth + 1;

        if let Some(state_key) = &state_key
            && let Ok(prev_pdu) =
                crate::room::get_state(room_id, &event_type.to_string().into(), state_key, None)
                    .await
        {
            unsigned.insert("prev_content".to_owned(), prev_pdu.content.clone());
            unsigned.insert(
                "prev_sender".to_owned(),
                to_raw_value(&prev_pdu.sender).expect("UserId::to_value always works"),
            );
            unsigned.insert(
                "replaces_state".to_owned(),
                to_raw_value(&prev_pdu.event_id).expect("EventId is valid json"),
            );
        }

        let temp_event_id =
            OwnedEventId::try_from(format!("$backfill_{}", Ulid::generate())).unwrap();

        let mut pdu = PduEvent {
            event_id: temp_event_id.clone(),
            event_ty: event_type,
            room_id: room_id.to_owned(),
            sender: sender_id.to_owned(),
            origin_server_ts: timestamp.unwrap_or_else(UnixMillis::now),
            content,
            state_key,
            prev_events,
            depth,
            auth_events: auth_events
                .values()
                .map(|pdu| pdu.event_id.clone())
                .collect(),
            redacts,
            unsigned,
            hashes: EventHash {
                sha256: "aaa".to_owned(),
            },
            signatures: None,
            extra_data: Default::default(),
            rejection_reason: None,
            transaction_device,
        };

        let fetch_event = async |event_id: OwnedEventId| {
            get_pdu(&event_id)
                .await
                .map(|s| s.pdu)
                .map_err(|_| StateError::other("missing PDU 6"))
        };
        let fetch_state = async |k: StateEventType, s: String| {
            if let Some(pdu) = auth_events
                .get(&(k.clone(), s.to_owned()))
                .map(|s| s.pdu.clone())
            {
                return Ok(pdu);
            }
            if auth_rules.room_create_event_id_as_room_id && k == StateEventType::RoomCreate {
                let pdu = crate::room::get_create(room_id)
                    .await
                    .map_err(|_| StateError::other("missing create event"))?
                    .into_inner();
                if pdu.room_id != *room_id {
                    Err(StateError::other("mismatched room id in create event"))
                } else {
                    Ok(pdu.into_inner())
                }
            } else {
                // If the state event is not found in auth_events, try to look it up
                // directly from room state as a fallback.
                if let Ok(state_pdu) = crate::room::get_state(room_id, &k, &s, None).await {
                    return Ok(state_pdu.pdu);
                }
                warn!(
                    "hash_sign: missing state event in auth_events for room {room_id}, event_type: {k}, state_key: {s}, auth_events keys: {:?}",
                    auth_events.keys().collect::<Vec<_>>()
                );
                Err(StateError::other(format!(
                    "failed hash and sign event, missing state event, event_type: {k}, state_key:{s}"
                )))
            }
        };
        event_auth::auth_check(auth_rules, &pdu, &fetch_event, &fetch_state).await?;

        // Hash and sign
        // NOTE: `pdu.content` originates from the client request (via `PduBuilder`),
        // so it may legitimately fail canonical-JSON serialization — e.g. if the
        // client submitted a float, which the Matrix canonical-JSON spec forbids.
        // Return a 400 `M_BAD_JSON` rather than panicking the server thread.
        let mut pdu_json = to_canonical_object(&pdu).map_err(|e| {
            tracing::warn!(error = ?e, "event content is not valid canonical JSON");
            MatrixError::bad_json(format!("event content is not valid canonical JSON: {e}"))
        })?;

        pdu_json.remove("event_id");

        if version_rules.room_id_format == RoomIdFormatVersion::V2
            && pdu.event_ty == TimelineEventType::RoomCreate
        {
            pdu_json.remove("room_id");
        }

        // Add origin because synapse likes that (and it's required in the spec)
        pdu_json.insert(
            "origin".to_owned(),
            to_canonical_value(&conf.server_name)
                .expect("server name is a valid CanonicalJsonValue"),
        );

        match crate::server_key::hash_and_sign_event(&mut pdu_json, room_version) {
            Ok(_) => {}
            Err(e) => {
                return match e {
                    AppError::Signatures(crate::core::signatures::Error::PduSize) => {
                        Err(MatrixError::too_large("message is too long").into())
                    }
                    _ => Err(MatrixError::unknown("signing event failed").into()),
                };
            }
        }

        // Generate event id
        pdu.event_id = crate::event::gen_event_id(&pdu_json, room_version)?;
        if version_rules.room_id_format == RoomIdFormatVersion::V2
            && pdu.event_ty == TimelineEventType::RoomCreate
        {
            pdu.room_id = RoomId::new_v2(pdu.event_id.localpart())?;
            diesel::update(
                event_forward_extremities::table
                    .filter(event_forward_extremities::room_id.eq(room_id)),
            )
            .set(event_forward_extremities::room_id.eq(&pdu.room_id))
            .execute(&mut connect().await?)
            .await?;
        }

        pdu_json.insert(
            "event_id".to_owned(),
            CanonicalJsonValue::String(pdu.event_id.as_str().to_owned()),
        );

        if let Err(e) = validate_canonical_json(&pdu_json) {
            error!("invalid event json: {}", e);
            return Err(MatrixError::bad_json(e.to_string()).into());
        }

        Ok((pdu, pdu_json))
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
            transaction_device: None,
        }
    }
}

/// Only event metadata is private; similarly named fields inside content are user data.
fn strip_embedded_transaction_ids(value: &mut JsonValue) {
    match value {
        JsonValue::Object(object) => {
            if let Some(JsonValue::Object(unsigned)) = object.get_mut("unsigned") {
                unsigned.remove("transaction_id");
            }
            for (key, value) in object {
                if key != "content" {
                    strip_embedded_transaction_ids(value);
                }
            }
        }
        JsonValue::Array(values) => {
            for value in values {
                strip_embedded_transaction_ids(value);
            }
        }
        _ => {}
    }
}

/// Federation has no originating-device context, including for embedded events.
pub(crate) fn sanitize_federation_unsigned(pdu: &mut CanonicalJsonObject) {
    let Some(CanonicalJsonValue::Object(unsigned)) = pdu.get_mut("unsigned") else {
        return;
    };
    unsigned.remove("transaction_id");
    for key in ["redacted_because", "m.relations"] {
        if let Some(value) = unsigned.get_mut(key) {
            let mut json = serde_json::to_value(&*value).expect("valid canonical JSON");
            strip_embedded_transaction_ids(&mut json);
            *value =
                serde_json::from_value(json).expect("removing fields preserves canonical JSON");
        }
    }
}

#[cfg(test)]
mod sender_only_unsigned_tests {
    use serde_json::value::to_raw_value;

    use super::*;

    fn event_with_transaction_id() -> PduEvent {
        let mut unsigned = BTreeMap::new();
        unsigned.insert("transaction_id".to_owned(), to_raw_value("txn").unwrap());
        unsigned.insert("age".to_owned(), to_raw_value(&10_u64).unwrap());

        PduEvent {
            event_id: "$event:example.org".try_into().unwrap(),
            sender: "@alice:example.org".try_into().unwrap(),
            origin_server_ts: UnixMillis(1),
            event_ty: TimelineEventType::RoomMessage,
            content: to_raw_value(&json!({"body": "hi", "msgtype": "m.text"})).unwrap(),
            state_key: None,
            room_id: "!room:example.org".try_into().unwrap(),
            prev_events: Vec::new(),
            depth: 1,
            auth_events: Vec::new(),
            redacts: None,
            hashes: EventHash {
                sha256: String::new(),
            },
            signatures: None,
            unsigned,
            extra_data: Default::default(),
            rejection_reason: None,
            transaction_device: None,
        }
    }

    #[test]
    fn originating_device_keeps_its_transaction_id() {
        let mut event = event_with_transaction_id();
        event.transaction_device = Some("PHONE".into());
        let sender: OwnedUserId = "@alice:example.org".try_into().unwrap();

        let converted = event.to_room_event_for(&sender, Some("PHONE".into()));
        let json: JsonValue = serde_json::from_str(converted.as_str()).unwrap();

        assert_eq!(
            json.pointer("/unsigned/transaction_id"),
            Some(&json!("txn"))
        );
    }

    #[test]
    fn other_users_do_not_receive_the_transaction_id() {
        let event = event_with_transaction_id();
        let recipient: OwnedUserId = "@bob:example.org".try_into().unwrap();

        let converted = event.to_room_event_for(&recipient, Some("PHONE".into()));
        let json: JsonValue = serde_json::from_str(converted.as_str()).unwrap();

        assert!(json.pointer("/unsigned/transaction_id").is_none());
        assert!(json.pointer("/unsigned/age").is_some());
    }

    #[test]
    fn member_listing_does_not_leak_the_transaction_id() {
        let mut event = event_with_transaction_id();
        event.event_ty = TimelineEventType::RoomMember;
        event.state_key = Some(event.sender.to_string());
        event.content = to_raw_value(&json!({"membership": "join"})).unwrap();
        let recipient: OwnedUserId = "@bob:example.org".try_into().unwrap();

        let converted = event.to_member_event_for(&recipient, Some("PHONE".into()));
        let json: JsonValue = serde_json::from_str(converted.as_str()).unwrap();

        assert!(json.pointer("/unsigned/transaction_id").is_none());
        assert_eq!(json.pointer("/unsigned/age"), Some(&json!(10)));
    }

    #[test]
    fn another_device_and_device_less_consumers_do_not_receive_transaction_ids() {
        let mut event = event_with_transaction_id();
        event.transaction_device = Some("PHONE".into());
        let laptop: &DeviceId = "LAPTOP".into();
        for device in [Some(laptop), None] {
            let converted = event.to_room_event_for(&event.sender, device);
            let json: JsonValue = serde_json::from_str(converted.as_str()).unwrap();
            assert!(json.pointer("/unsigned/transaction_id").is_none());
        }
    }

    #[test]
    fn event_json_cannot_supply_trusted_device_provenance() {
        let event = event_with_transaction_id();
        let mut json = serde_json::to_value(&event).unwrap();
        json["transaction_device"] = json!("PHONE");
        let parsed: PduEvent = serde_json::from_value(json).unwrap();
        assert!(parsed.transaction_device.is_none());
        let builder: PduBuilder = serde_json::from_value(json!({
            "type": "m.room.message", "content": {}, "transaction_device": "PHONE"
        }))
        .unwrap();
        assert!(builder.transaction_device.is_none());
        let converted = parsed.to_room_event_for(&parsed.sender, Some("PHONE".into()));
        let json: JsonValue = serde_json::from_str(converted.as_str()).unwrap();
        assert!(json.pointer("/unsigned/transaction_id").is_none());
    }

    #[test]
    fn nested_events_never_expose_a_transaction_id() {
        let event = event_with_transaction_id();

        let converted = event.to_message_like_event_without_transaction_id();
        let json: JsonValue = serde_json::from_str(converted.as_str()).unwrap();

        assert!(json.pointer("/unsigned/transaction_id").is_none());
        assert_eq!(json.pointer("/unsigned/age"), Some(&json!(10)));
    }

    #[test]
    fn stored_redactions_and_relation_bundles_do_not_leak_device_metadata() {
        let mut event = event_with_transaction_id();
        event.transaction_device = Some("PHONE".into());
        let embedded = json!({
            "type": "m.room.message", "sender": "@other:example.org",
            "content": {"unsigned": {"transaction_id": "user content"}},
            "unsigned": {"transaction_id": "private nested transaction", "age": 12}
        });
        event
            .unsigned
            .insert("redacted_because".into(), to_raw_value(&embedded).unwrap());
        event.unsigned.insert(
            "m.relations".into(),
            to_raw_value(&json!({
                "m.thread": {"latest_event": embedded}, "m.replace": embedded
            }))
            .unwrap(),
        );
        for device in [Some("PHONE".into()), Some("LAPTOP".into()), None] {
            let converted = event.to_room_event_for(&event.sender, device);
            let json: JsonValue = serde_json::from_str(converted.as_str()).unwrap();
            assert_eq!(
                json.pointer("/unsigned/transaction_id").is_some(),
                device == Some("PHONE".into())
            );
            for path in [
                "/unsigned/redacted_because",
                "/unsigned/m.relations/m.thread/latest_event",
                "/unsigned/m.relations/m.replace",
            ] {
                assert!(
                    json.pointer(&format!("{path}/unsigned/transaction_id"))
                        .is_none()
                );
                assert_eq!(
                    json.pointer(&format!("{path}/unsigned/age")),
                    Some(&json!(12))
                );
                assert_eq!(
                    json.pointer(&format!("{path}/content/unsigned/transaction_id")),
                    Some(&json!("user content"))
                );
            }
        }
        let mut federation = to_canonical_object(&event).unwrap();
        sanitize_federation_unsigned(&mut federation);
        let federation = serde_json::to_value(federation).unwrap();
        assert!(federation.pointer("/unsigned/transaction_id").is_none());
        for path in [
            "/unsigned/redacted_because",
            "/unsigned/m.relations/m.thread/latest_event",
            "/unsigned/m.relations/m.replace",
        ] {
            assert!(
                federation
                    .pointer(&format!("{path}/unsigned/transaction_id"))
                    .is_none()
            );
            assert_eq!(
                federation.pointer(&format!("{path}/content/unsigned/transaction_id")),
                Some(&json!("user content"))
            );
        }
    }
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_transaction_device_is_scoped_to_the_event() {
        crate::test_database::init();
        let mut event = event_with_transaction_id();
        let phone: &DeviceId = "PHONE".into();
        let laptop: &DeviceId = "LAPTOP".into();
        crate::data::room::transaction_id::add_txn_id(
            "txn".into(),
            &event.sender,
            Some(phone),
            Some(&event.room_id),
            Some(&event.event_id),
        )
        .await
        .unwrap();
        // Another device can reuse the transaction string for a different event.
        let other = EventId::parse("$other:example.org").unwrap();
        crate::data::room::transaction_id::add_txn_id(
            "txn".into(),
            &event.sender,
            Some(laptop),
            Some(&event.room_id),
            Some(&other),
        )
        .await
        .unwrap();
        event.load_transaction_device().await.unwrap();
        assert_eq!(event.transaction_device.as_deref(), Some(phone));
        for (device, expected) in [(phone, true), (laptop, false)] {
            let json: JsonValue = serde_json::from_str(
                event
                    .to_room_event_for(&event.sender, Some(device))
                    .as_str(),
            )
            .unwrap();
            assert_eq!(json.pointer("/unsigned/transaction_id").is_some(), expected);
        }
        event.event_id = other;
        event.load_transaction_device().await.unwrap();
        assert_eq!(event.transaction_device.as_deref(), Some(laptop));
        event.room_id = "!other:example.org".try_into().unwrap();
        event.load_transaction_device().await.unwrap();
        assert!(event.transaction_device.is_none());

        // A newly visible event already has device provenance even while its route
        // has not yet written the idempotency-completion row.
        event.event_id = "$before-idempotency:example.org".try_into().unwrap();
        event.transaction_device = Some(phone.to_owned());
        let metadata = event.transaction_metadata();
        let event_data = DbEventData {
            event_id: event.event_id.clone(),
            event_sn: 500,
            room_id: event.room_id.clone(),
            json_data: serde_json::to_value(&event).unwrap(),
            internal_metadata: metadata,
            format_version: None,
        };
        event_data.save().await.unwrap();
        assert!(
            crate::data::room::transaction_id::get_event_id(
                "txn".into(),
                &event.sender,
                Some(phone),
                Some(&event.room_id)
            )
            .await
            .unwrap()
            .is_none()
        );
        event.transaction_device = None;
        event.load_transaction_device().await.unwrap();
        assert_eq!(event.transaction_device.as_deref(), Some(phone));
        // Later JSON updates must not clear trusted metadata.
        let mut update = event_data;
        update.internal_metadata = None;
        update.save().await.unwrap();
        event.load_transaction_device().await.unwrap();
        assert_eq!(event.transaction_device.as_deref(), Some(phone));
    }
}
