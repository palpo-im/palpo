use diesel::prelude::*;
use serde::Deserialize;

use crate::core::events::StateEventType;
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonObject, JsonValue, default_false};
use crate::core::{MatrixError, Seqnum, UnixMillis};
use crate::schema::*;
use crate::{DataResult, connect};

pub mod event;
pub mod receipt;
pub mod timeline;

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = rooms)]
pub struct DbRoom {
    pub id: OwnedRoomId,
    pub sn: Seqnum,
    pub version: String,
    pub is_public: bool,
    pub min_depth: i64,
    pub state_frame_id: Option<i64>,
    pub has_auth_chain_index: bool,
    pub disabled: bool,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = rooms)]
pub struct NewDbRoom {
    pub id: OwnedRoomId,
    pub version: String,
    pub is_public: bool,
    pub min_depth: i64,
    pub has_auth_chain_index: bool,
    pub created_at: UnixMillis,
}

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = room_tags)]
pub struct DbRoomTag {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub room_id: OwnedRoomId,
    pub tag: String,
    pub content: JsonValue,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = room_tags)]
pub struct NewDbRoomTag {
    pub user_id: OwnedUserId,
    pub room_id: OwnedRoomId,
    pub tag: String,
    pub content: JsonValue,
}

#[derive(Insertable, Identifiable, Queryable, AsChangeset, Debug, Clone)]
#[diesel(table_name = stats_room_currents, primary_key(room_id))]
pub struct DbRoomCurrent {
    pub room_id: OwnedRoomId,
    pub state_events: i64,
    pub joined_members: i64,
    pub invited_members: i64,
    pub left_members: i64,
    pub banned_members: i64,
    pub knocked_members: i64,
    pub local_users_in_room: i64,
    pub completed_delta_stream_id: i64,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = event_relations)]
pub struct DbEventRelation {
    pub id: i64,

    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub event_ty: String,
    pub child_id: OwnedEventId,
    pub child_sn: i64,
    pub child_ty: String,
    pub rel_type: Option<String>,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = event_relations)]
pub struct NewDbEventRelation {
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub event_ty: String,
    pub child_id: OwnedEventId,
    pub child_sn: i64,
    pub child_ty: String,
    pub rel_type: Option<String>,
}

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = room_aliases, primary_key(alias_id))]
pub struct DbRoomAlias {
    pub alias_id: OwnedRoomAliasId,
    pub room_id: OwnedRoomId,
    pub created_by: OwnedUserId,
    pub created_at: UnixMillis,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = room_state_fields)]
pub struct DbRoomStateField {
    pub id: i64,
    pub event_ty: StateEventType,
    pub state_key: String,
}

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = room_state_deltas, primary_key(frame_id))]
pub struct DbRoomStateDelta {
    pub frame_id: i64,
    pub room_id: OwnedRoomId,
    pub parent_id: Option<i64>,
    pub appended: Vec<u8>,
    pub disposed: Vec<u8>,
}

#[derive(Identifiable, Insertable, AsChangeset, Queryable, Debug, Clone)]
#[diesel(table_name = event_receipts, primary_key(sn))]
pub struct DbReceipt {
    pub sn: Seqnum,
    pub ty: String,
    pub room_id: OwnedRoomId,
    pub user_id: OwnedUserId,
    pub event_id: OwnedEventId,
    pub event_sn: Seqnum,
    pub thread_id: Option<OwnedEventId>,
    pub json_data: JsonValue,
    pub receipt_at: UnixMillis,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = event_push_summaries)]
pub struct DbEventPushSummary {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub room_id: OwnedRoomId,
    pub notification_count: i64,
    pub highlight_count: i64,
    pub unread_count: i64,
    pub stream_ordering: i64,
    pub thread_id: Option<OwnedEventId>,
}

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = room_users)]
pub struct DbRoomUser {
    pub id: i64,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub room_id: OwnedRoomId,
    pub room_server_id: Option<OwnedServerName>,
    pub user_id: OwnedUserId,
    pub user_server_id: OwnedServerName,
    pub sender_id: OwnedUserId,
    pub membership: String,
    pub forgotten: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub state_data: Option<JsonValue>,
    pub created_at: UnixMillis,
}
#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = room_users)]
pub struct NewDbRoomUser {
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub room_id: OwnedRoomId,
    pub room_server_id: Option<OwnedServerName>,
    pub user_id: OwnedUserId,
    pub user_server_id: OwnedServerName,
    pub sender_id: OwnedUserId,
    pub membership: String,
    pub forgotten: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub state_data: Option<JsonValue>,
    pub created_at: UnixMillis,
}

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = threads, primary_key(event_id))]
pub struct DbThread {
    pub event_id: OwnedEventId,
    pub event_sn: Seqnum,
    pub room_id: OwnedRoomId,
    pub last_id: OwnedEventId,
    pub last_sn: i64,
}

#[derive(Insertable, Identifiable, AsChangeset, Queryable, Debug, Clone)]
#[diesel(table_name = event_datas, primary_key(event_id))]
pub struct DbEventData {
    pub event_id: OwnedEventId,
    pub event_sn: Seqnum,
    pub room_id: OwnedRoomId,
    pub internal_metadata: Option<JsonValue>,
    pub format_version: Option<i64>,
    pub json_data: JsonValue,
}

impl DbEventData {
    pub fn save(&self) -> DataResult<()> {
        diesel::insert_into(event_datas::table)
            .values(self)
            .on_conflict(event_datas::event_id)
            .do_update()
            .set(self)
            .execute(&mut connect()?)?;
        Ok(())
    }
}

#[derive(Identifiable, Insertable, Queryable, AsChangeset, Debug, Clone, serde::Serialize)]
#[diesel(table_name = events, primary_key(id))]
pub struct DbEvent {
    pub id: OwnedEventId,
    pub sn: Seqnum,
    pub ty: String,
    pub room_id: OwnedRoomId,
    pub depth: i64,
    pub topological_ordering: i64,
    pub stream_ordering: i64,
    pub unrecognized_keys: Option<String>,
    pub origin_server_ts: UnixMillis,
    pub received_at: Option<i64>,
    pub sender_id: Option<OwnedUserId>,
    pub contains_url: bool,
    pub worker_id: Option<String>,
    pub state_key: Option<String>,
    pub is_outlier: bool,
    pub is_redacted: bool,
    pub soft_failed: bool,
    pub is_rejected: bool,
    pub rejection_reason: Option<String>,
}
impl DbEvent {
    pub fn get_by_id(id: &EventId) -> DataResult<Self> {
        events::table
            .find(id)
            .first(&mut connect()?)
            .map_err(Into::into)
    }
}

#[derive(Insertable, AsChangeset, Deserialize, Debug, Clone)]
#[diesel(table_name = events, primary_key(id))]
pub struct NewDbEvent {
    pub id: OwnedEventId,
    pub sn: Seqnum,
    #[serde(rename = "type")]
    pub ty: String,
    pub room_id: OwnedRoomId,
    pub depth: i64,
    pub topological_ordering: i64,
    pub stream_ordering: i64,
    pub unrecognized_keys: Option<String>,
    pub origin_server_ts: UnixMillis,
    pub received_at: Option<i64>,
    pub sender_id: Option<OwnedUserId>,
    #[serde(default = "default_false")]
    pub contains_url: bool,
    pub worker_id: Option<String>,
    pub state_key: Option<String>,
    #[serde(default = "default_false")]
    pub is_outlier: bool,
    #[serde(default = "default_false")]
    pub soft_failed: bool,
    #[serde(default = "default_false")]
    pub is_rejected: bool,
    pub rejection_reason: Option<String>,
}
impl NewDbEvent {
    pub fn from_canonical_json(
        id: &EventId,
        sn: Seqnum,
        value: &CanonicalJsonObject,
        is_backfill: bool,
    ) -> DataResult<Self> {
        Self::from_json_value(id, sn, serde_json::to_value(value)?, is_backfill)
    }
    pub fn from_json_value(
        id: &EventId,
        sn: Seqnum,
        mut value: JsonValue,
        is_backfill: bool,
    ) -> DataResult<Self> {
        let depth = value.get("depth").cloned().unwrap_or(0.into());
        let ty = value
            .get("type")
            .cloned()
            .unwrap_or_else(|| "m.room.message".into());
        let obj = value
            .as_object_mut()
            .ok_or(MatrixError::bad_json("Invalid event"))?;
        obj.insert("id".into(), id.as_str().into());
        obj.insert("sn".into(), sn.into());
        obj.insert("type".into(), ty);
        obj.insert("topological_ordering".into(), depth);
        obj.insert(
            "stream_ordering".into(),
            if is_backfill { (-sn).into() } else { sn.into() },
        );
        Ok(serde_json::from_value(value)
            .map_err(|_e| MatrixError::bad_json("invalid json for event"))?)
    }

    pub fn save(&self) -> DataResult<()> {
        diesel::insert_into(events::table)
            .values(self)
            .on_conflict(events::id)
            .do_update()
            .set(self)
            .execute(&mut connect()?)?;
        Ok(())
    }
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = event_idempotents)]
pub struct NewDbEventIdempotent {
    pub txn_id: OwnedTransactionId,
    pub user_id: OwnedUserId,
    pub device_id: Option<OwnedDeviceId>,
    pub room_id: Option<OwnedRoomId>,
    pub event_id: Option<OwnedEventId>,
    pub created_at: UnixMillis,
}

#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = event_push_actions)]
pub struct NewDbEventPushAction {
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub event_sn: Seqnum,
    pub user_id: OwnedUserId,
    pub profile_tag: String,
    pub actions: JsonValue,
    pub topological_ordering: i64,
    pub stream_ordering: i64,
    pub notify: bool,
    pub highlight: bool,
    pub unread: bool,
    pub thread_id: Option<OwnedEventId>,
}

pub fn is_disabled(room_id: &RoomId) -> DataResult<bool> {
    let query = rooms::table
        .filter(rooms::id.eq(room_id))
        .filter(rooms::disabled.eq(true));
    Ok(diesel_exists!(query, &mut connect()?)?)
}

pub fn add_joined_server(room_id: &RoomId, server_name: &ServerName) -> DataResult<()> {
    let next_sn = crate::next_sn()?;
    diesel::insert_into(room_joined_servers::table)
        .values((
            room_joined_servers::room_id.eq(room_id),
            room_joined_servers::server_id.eq(server_name),
            room_joined_servers::occur_sn.eq(next_sn),
        ))
        .on_conflict_do_nothing()
        .execute(&mut connect()?)?;
    Ok(())
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = banned_rooms)]
pub struct NewDbBannedRoom {
    pub room_id: OwnedRoomId,
    pub created_by: Option<OwnedUserId>,
    pub created_at: UnixMillis,
}

pub fn is_banned(room_id: &RoomId) -> DataResult<bool> {
    let query = banned_rooms::table.filter(banned_rooms::room_id.eq(room_id));
    Ok(diesel_exists!(query, &mut connect()?)?)
}

pub fn is_public(room_id: &RoomId) -> DataResult<bool> {
    rooms::table
        .filter(rooms::id.eq(room_id))
        .select(rooms::is_public)
        .first(&mut connect()?)
        .map_err(Into::into)
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = timeline_gaps)]
pub struct NewDbTimelineGap {
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = event_missings)]
pub struct NewDbEventMissing {
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub missing_id: OwnedEventId,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = event_edges)]
pub struct NewDbEventEdge {
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub event_depth: i64,
    pub prev_id: OwnedEventId,
}

impl NewDbEventEdge {
    pub fn save(&self) -> DataResult<()> {
        diesel::insert_into(event_edges::table)
            .values(self)
            .on_conflict_do_nothing()
            .execute(&mut connect()?)?;
        Ok(())
    }
}

// >= min_sn and <= max_sn
pub fn get_timeline_gaps(
    room_id: &RoomId,
    min_sn: Seqnum,
    max_sn: Seqnum,
) -> DataResult<Vec<Seqnum>> {
    let gaps = timeline_gaps::table
        .filter(timeline_gaps::room_id.eq(room_id))
        .filter(timeline_gaps::event_sn.ge(min_sn))
        .filter(timeline_gaps::event_sn.le(max_sn))
        .order(timeline_gaps::event_sn.asc())
        .select(timeline_gaps::event_sn)
        .load::<Seqnum>(&mut connect()?)?;
    Ok(gaps)
}

// pub fn rename_room(old_room_id: &RoomId, new_room_id: &RoomId) -> DataResult<()> {
//     let conn = &mut connect()?;
//     diesel::update(rooms::table.filter(rooms::id.eq(old_room_id)))
//         .set(rooms::id.eq(new_room_id))
//         .execute(conn)?;

//     diesel::update(user_datas::table.filter(user_datas::room_id.eq(old_room_id)))
//         .set(user_datas::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(user_profiles::table.filter(user_profiles::room_id.eq(old_room_id)))
//         .set(user_profiles::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(room_aliases::table.filter(room_aliases::room_id.eq(old_room_id)))
//         .set(room_aliases::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(room_tags::table.filter(room_tags::room_id.eq(old_room_id)))
//         .set(room_tags::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(stats_room_currents::table.filter(stats_room_currents::room_id.
// eq(old_room_id)))         .set(stats_room_currents::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(events::table.filter(events::room_id.eq(old_room_id)))
//         .set(events::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(event_datas::table.filter(event_datas::room_id.eq(old_room_id)))
//         .set(event_datas::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(event_points::table.filter(event_points::room_id.eq(old_room_id)))
//         .set(event_points::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(threads::table.filter(threads::room_id.eq(old_room_id)))
//         .set(threads::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(room_state_frames::table.filter(room_state_frames::room_id.eq(old_room_id)))
//         .set(room_state_frames::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(room_state_deltas::table.filter(room_state_deltas::room_id.eq(old_room_id)))
//         .set(room_state_deltas::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(
//         event_backward_extremities::table
//             .filter(event_backward_extremities::room_id.eq(old_room_id)),
//     )
//     .set(event_backward_extremities::room_id.eq(new_room_id))
//     .execute(conn)?;
//     diesel::update(
//         event_forward_extremities::table.filter(event_forward_extremities::room_id.
// eq(old_room_id)),     )
//     .set(event_forward_extremities::room_id.eq(new_room_id))
//     .execute(conn)?;
//     diesel::update(room_users::table.filter(room_users::room_id.eq(old_room_id)))
//         .set(room_users::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(e2e_room_keys::table.filter(e2e_room_keys::room_id.eq(old_room_id)))
//         .set(e2e_room_keys::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(e2e_key_changes::table.filter(e2e_key_changes::room_id.eq(old_room_id)))
//         .set(e2e_key_changes::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(event_relations::table.filter(event_relations::room_id.eq(old_room_id)))
//         .set(event_relations::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(event_receipts::table.filter(event_receipts::room_id.eq(old_room_id)))
//         .set(event_receipts::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(event_searches::table.filter(event_searches::room_id.eq(old_room_id)))
//         .set(event_searches::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(
//         event_push_summaries::table.filter(event_push_summaries::room_id.eq(old_room_id)),
//     )
//     .set(event_push_summaries::room_id.eq(new_room_id))
//     .execute(conn)?;
//     diesel::update(event_edges::table.filter(event_edges::room_id.eq(old_room_id)))
//         .set(event_edges::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(event_idempotents::table.filter(event_idempotents::room_id.eq(old_room_id)))
//         .set(event_idempotents::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(
//         lazy_load_deliveries::table.filter(lazy_load_deliveries::room_id.eq(old_room_id)),
//     )
//     .set(lazy_load_deliveries::room_id.eq(new_room_id))
//     .execute(conn)?;
//     diesel::update(room_lookup_servers::table.filter(room_lookup_servers::room_id.
// eq(old_room_id)))         .set(room_lookup_servers::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(event_push_actions::table.filter(event_push_actions::room_id.eq(old_room_id)))
//         .set(event_push_actions::room_id.eq(new_room_id))
//         .execute(conn)?;
//     diesel::update(banned_rooms::table.filter(banned_rooms::room_id.eq(old_room_id)))
//         .set(banned_rooms::room_id.eq(new_room_id))
//         .execute(conn)?;
//     Ok(())
// }
