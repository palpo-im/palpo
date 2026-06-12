use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::Deserialize;

use crate::core::events::StateEventType;
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonObject, JsonValue, default_false};
use crate::core::{MatrixError, Seqnum, UnixMillis};
use crate::schema::*;
use crate::{DataResult, connect};

pub mod event;
pub mod event_report;
pub mod lazy_loading;
pub mod peek;
pub mod receipt;
pub mod timeline;
pub mod transaction_id;
pub mod typing;
pub use event_report::*;

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
    pub async fn save(&self) -> DataResult<()> {
        diesel::insert_into(event_datas::table)
            .values(self)
            .on_conflict(event_datas::event_id)
            .do_update()
            .set(self)
            .execute(&mut connect().await?)
            .await?;
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
    pub async fn get_by_id(id: &EventId) -> DataResult<Self> {
        events::table
            .find(id)
            .first(&mut connect().await?)
            .await
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
        Self::from_json_value(id, sn, serde_json::to_value(value)?, is_backfill, None)
    }
    pub fn from_canonical_json_with_room_id(
        id: &EventId,
        sn: Seqnum,
        value: &CanonicalJsonObject,
        is_backfill: bool,
        room_id: &RoomId,
    ) -> DataResult<Self> {
        Self::from_json_value(
            id,
            sn,
            serde_json::to_value(value)?,
            is_backfill,
            Some(room_id),
        )
    }
    pub fn from_json_value(
        id: &EventId,
        sn: Seqnum,
        mut value: JsonValue,
        is_backfill: bool,
        room_id: Option<&RoomId>,
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
        // For V12 rooms, the create event does not contain a room_id field in its
        // content (the room_id is derived from the event_id). Inject it here so
        // deserialization into NewDbEvent succeeds.
        if !obj.contains_key("room_id")
            && let Some(rid) = room_id
        {
            obj.insert("room_id".into(), rid.as_str().into());
        }
        Ok(serde_json::from_value(value)
            .map_err(|_e| MatrixError::bad_json("invalid json for event"))?)
    }

    pub async fn save(&self) -> DataResult<()> {
        diesel::insert_into(events::table)
            .values(self)
            .on_conflict(events::id)
            .do_update()
            .set(self)
            .execute(&mut connect().await?)
            .await?;
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

pub async fn is_disabled(room_id: &RoomId) -> DataResult<bool> {
    let query = rooms::table
        .filter(rooms::id.eq(room_id))
        .filter(rooms::disabled.eq(true));
    Ok(diesel_exists!(query, &mut connect().await?)?)
}

pub async fn add_joined_server(room_id: &RoomId, server_name: &ServerName) -> DataResult<()> {
    let next_sn = crate::next_sn().await?;
    diesel::insert_into(room_joined_servers::table)
        .values((
            room_joined_servers::room_id.eq(room_id),
            room_joined_servers::server_id.eq(server_name),
            room_joined_servers::occur_sn.eq(next_sn),
        ))
        .on_conflict_do_nothing()
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Return the distinct set of servers joined to any of the given rooms.
pub async fn joined_servers_for_rooms(
    room_ids: &[OwnedRoomId],
) -> DataResult<Vec<OwnedServerName>> {
    room_joined_servers::table
        .filter(room_joined_servers::room_id.eq_any(room_ids))
        .select(room_joined_servers::server_id)
        .distinct()
        .load::<OwnedServerName>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = banned_rooms)]
pub struct NewDbBannedRoom {
    pub room_id: OwnedRoomId,
    pub created_by: Option<OwnedUserId>,
    pub created_at: UnixMillis,
}

pub async fn is_banned(room_id: &RoomId) -> DataResult<bool> {
    let query = banned_rooms::table.filter(banned_rooms::room_id.eq(room_id));
    Ok(diesel_exists!(query, &mut connect().await?)?)
}

pub async fn is_public(room_id: &RoomId) -> DataResult<bool> {
    rooms::table
        .filter(rooms::id.eq(room_id))
        .select(rooms::is_public)
        .first(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Set whether a room is published in the public room directory.
pub async fn set_public(room_id: &RoomId, value: bool) -> DataResult<()> {
    diesel::update(rooms::table.find(room_id))
        .set(rooms::is_public.eq(value))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Resolve a local alias to the room id it points at.
pub async fn get_alias_room_id(alias_id: &RoomAliasId) -> DataResult<Option<OwnedRoomId>> {
    room_aliases::table
        .filter(room_aliases::alias_id.eq(alias_id))
        .select(room_aliases::room_id)
        .first::<OwnedRoomId>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Fetch the full record for a local alias.
pub async fn get_alias(alias_id: &RoomAliasId) -> DataResult<Option<DbRoomAlias>> {
    room_aliases::table
        .find(alias_id)
        .first::<DbRoomAlias>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// All local aliases pointing at a room.
pub async fn local_aliases_for_room(room_id: &RoomId) -> DataResult<Vec<OwnedRoomAliasId>> {
    room_aliases::table
        .filter(room_aliases::room_id.eq(room_id))
        .select(room_aliases::alias_id)
        .load::<OwnedRoomAliasId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// All local `(room_id, alias_id)` pairs.
pub async fn all_local_aliases() -> DataResult<Vec<(OwnedRoomId, OwnedRoomAliasId)>> {
    room_aliases::table
        .select((room_aliases::room_id, room_aliases::alias_id))
        .load::<(OwnedRoomId, OwnedRoomAliasId)>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Whether the alias already exists but points at a different room.
pub async fn alias_exists_for_other_room(
    alias_id: &RoomAliasId,
    room_id: &RoomId,
) -> DataResult<bool> {
    let query = room_aliases::table
        .filter(room_aliases::alias_id.eq(alias_id))
        .filter(room_aliases::room_id.ne(room_id));
    Ok(diesel_exists!(query, &mut connect().await?)?)
}

/// Insert an alias, ignoring conflicts with an existing one.
pub async fn set_alias(alias: DbRoomAlias) -> DataResult<()> {
    diesel::insert_into(room_aliases::table)
        .values(alias)
        .on_conflict_do_nothing()
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Remove a local alias.
pub async fn remove_alias(alias_id: &RoomAliasId) -> DataResult<()> {
    diesel::delete(room_aliases::table.filter(room_aliases::alias_id.eq(alias_id)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// List a room's thread roots as `(event_id, event_sn)`, newest activity first,
/// optionally only those with `event_sn <= before_sn`.
pub async fn list_threads(
    room_id: &RoomId,
    before_sn: Option<i64>,
    limit: i64,
) -> DataResult<Vec<(OwnedEventId, i64)>> {
    let mut query = threads::table
        .filter(threads::room_id.eq(room_id))
        .into_boxed();
    if let Some(before_sn) = before_sn {
        query = query.filter(threads::event_sn.le(before_sn));
    }
    query
        .select((threads::event_id, threads::event_sn))
        .order_by(threads::last_sn.desc())
        .limit(limit)
        .load::<(OwnedEventId, i64)>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Set the thread id an event belongs to.
pub async fn set_event_thread_id(event_id: &EventId, thread_id: &EventId) -> DataResult<()> {
    diesel::update(event_points::table.find(event_id))
        .set(event_points::thread_id.eq(thread_id))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Insert a thread root, updating its latest event if it already exists.
pub async fn upsert_thread(thread: DbThread) -> DataResult<()> {
    let last_id = thread.last_id.clone();
    let last_sn = thread.last_sn;
    diesel::insert_into(threads::table)
        .values(thread)
        .on_conflict(threads::event_id)
        .do_update()
        .set((threads::last_id.eq(last_id), threads::last_sn.eq(last_sn)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Current per-room statistics row, if present.
pub async fn get_room_current(room_id: &RoomId) -> DataResult<Option<DbRoomCurrent>> {
    stats_room_currents::table
        .filter(stats_room_currents::room_id.eq(room_id))
        .first::<DbRoomCurrent>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Number of invited members recorded for a room.
pub async fn invited_members_count(room_id: &RoomId) -> DataResult<Option<i64>> {
    stats_room_currents::table
        .filter(stats_room_currents::room_id.eq(room_id))
        .select(stats_room_currents::invited_members)
        .first::<i64>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Number of left members recorded for a room.
pub async fn left_members_count(room_id: &RoomId) -> DataResult<Option<i64>> {
    stats_room_currents::table
        .filter(stats_room_currents::room_id.eq(room_id))
        .select(stats_room_currents::left_members)
        .first::<i64>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Insert an event relation row.
pub async fn add_event_relation(relation: &NewDbEventRelation) -> DataResult<()> {
    diesel::insert_into(event_relations::table)
        .values(relation)
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Load event relations for a target event, ordered by child sequence number in
/// the requested direction. `forward` selects ascending order from `from`,
/// otherwise descending.
#[allow(clippy::too_many_arguments)]
pub async fn get_event_relations(
    room_id: &RoomId,
    event_id: &EventId,
    child_ty: Option<&str>,
    rel_type: Option<&str>,
    from: Seqnum,
    to: Option<Seqnum>,
    forward: bool,
    limit: usize,
) -> DataResult<Vec<DbEventRelation>> {
    let mut query = event_relations::table
        .filter(event_relations::room_id.eq(room_id))
        .filter(event_relations::event_id.eq(event_id))
        .into_boxed();
    if let Some(child_ty) = child_ty {
        query = query.filter(event_relations::child_ty.eq(child_ty));
    }
    if let Some(rel_type) = rel_type {
        query = query.filter(event_relations::rel_type.eq(rel_type));
    }
    if forward {
        query = query.filter(event_relations::child_sn.ge(from));
        if let Some(to) = to {
            query = query.filter(event_relations::child_sn.le(to));
        }
        query = query.order_by(event_relations::child_sn.asc());
    } else {
        query = query.filter(event_relations::child_sn.le(from));
        if let Some(to) = to {
            query = query.filter(event_relations::child_sn.ge(to));
        }
        query = query.order_by(event_relations::child_sn.desc());
    }
    query
        .limit(limit as i64)
        .load::<DbEventRelation>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Mark an event as soft-failed.
pub async fn set_event_soft_failed(event_id: &EventId) -> DataResult<()> {
    diesel::update(events::table.filter(events::id.eq(event_id)))
        .set(events::soft_failed.eq(true))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Whether an event is recorded as soft-failed.
pub async fn is_event_soft_failed(event_id: &EventId) -> DataResult<bool> {
    events::table
        .filter(events::id.eq(event_id))
        .select(events::soft_failed)
        .first(&mut connect().await?)
        .await
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
    pub async fn save(&self) -> DataResult<()> {
        diesel::insert_into(event_edges::table)
            .values(self)
            .on_conflict_do_nothing()
            .execute(&mut connect().await?)
            .await?;
        Ok(())
    }
}

// >= min_sn and <= max_sn
pub async fn get_timeline_gaps(
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
        .load::<Seqnum>(&mut connect().await?)
        .await?;
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
