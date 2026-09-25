use std::collections::HashMap;

use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::de::DeserializeOwned;

use crate::core::events::{AnyRawAccountDataEvent, RoomAccountDataEventType};
use crate::core::identifiers::*;
use crate::core::serde::{JsonValue, RawJson, json};
use crate::core::{Seqnum, UnixMillis};
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_datas)]
pub struct DbUserData {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub room_id: Option<OwnedRoomId>,
    pub data_type: String,
    pub json_data: JsonValue,
    pub is_deleted: bool,
    pub occur_sn: i64,
    pub created_at: UnixMillis,
}
#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = user_datas)]
pub struct NewDbUserData {
    pub user_id: OwnedUserId,
    pub room_id: Option<OwnedRoomId>,
    pub data_type: String,
    pub json_data: JsonValue,
    pub is_deleted: bool,
    pub occur_sn: Option<i64>,
    pub created_at: UnixMillis,
}

/// Places one event in the account data of the user and removes the previous entry.
#[tracing::instrument(skip(room_id, user_id, event_type, json_data))]
pub async fn set_data(
    user_id: &UserId,
    room_id: Option<OwnedRoomId>,
    event_type: &str,
    json_data: JsonValue,
) -> DataResult<DbUserData> {
    let mut conn = connect().await?;
    conn.transaction::<_, crate::DataError, _>(async |conn| {
        lock_data_key(conn, user_id, room_id.as_deref(), event_type).await?;
        let existing = get_latest_data(conn, user_id, room_id.as_deref(), event_type).await?;
        write_data_locked(conn, existing, user_id, room_id, event_type, json_data).await
    })
    .await
}

/// Replace account data only if its current value is still `expected`.
///
/// This is used for derived/cache-style rewrites such as refreshing server
/// default push rules. Every normal [`set_data`] for the same key takes the
/// same transaction-scoped advisory lock, so the comparison and write cannot
/// overwrite an edit that committed after the caller read `expected`.
pub async fn set_data_if_unchanged(
    user_id: &UserId,
    room_id: Option<OwnedRoomId>,
    event_type: &str,
    expected: Option<&JsonValue>,
    json_data: JsonValue,
) -> DataResult<bool> {
    let mut conn = connect().await?;
    conn.transaction::<_, crate::DataError, _>(async |conn| {
        lock_data_key(conn, user_id, room_id.as_deref(), event_type).await?;
        let existing = get_latest_data(conn, user_id, room_id.as_deref(), event_type).await?;
        let current = existing
            .as_ref()
            .filter(|row| !row.is_deleted)
            .map(|row| &row.json_data);
        if current != expected {
            return Ok(false);
        }

        write_data_locked(conn, existing, user_id, room_id, event_type, json_data).await?;
        Ok(true)
    })
    .await
}

/// Serialize mutations of one account-data key across server processes.
async fn lock_data_key(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    room_id: Option<&RoomId>,
    event_type: &str,
) -> DataResult<()> {
    let room_id = room_id.map_or("", RoomId::as_str);
    // PostgreSQL text cannot contain NUL, so make the pair unambiguous with a
    // length prefix instead of a separator that either component might use.
    let scope = format!("{}:{room_id}{event_type}", room_id.len());
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtext($1), hashtext($2))")
        .bind::<diesel::sql_types::Text, _>(user_id.as_str())
        .bind::<diesel::sql_types::Text, _>(scope)
        .execute(conn)
        .await?;
    Ok(())
}

async fn get_latest_data(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    room_id: Option<&RoomId>,
    event_type: &str,
) -> DataResult<Option<DbUserData>> {
    // Locate the current row explicitly. Global account data is stored with
    // `room_id = NULL`, and the `user_datas_udx` unique index treats NULLs as
    // distinct (Postgres default), so `ON CONFLICT (user_id, room_id,
    // data_type)` never matches a NULL `room_id` and would insert a duplicate
    // row on every update. We therefore find the latest existing row and
    // update it in place (or insert when none exists).
    let existing = if let Some(room_id) = room_id {
        user_datas::table
            .filter(user_datas::user_id.eq(user_id))
            .filter(user_datas::room_id.eq(room_id))
            .filter(user_datas::data_type.eq(event_type))
            .order_by(user_datas::id.desc())
            .first::<DbUserData>(conn)
            .await
            .optional()?
    } else {
        user_datas::table
            .filter(user_datas::user_id.eq(user_id))
            .filter(user_datas::room_id.is_null())
            .filter(user_datas::data_type.eq(event_type))
            .order_by(user_datas::id.desc())
            .first::<DbUserData>(conn)
            .await
            .optional()?
    };
    Ok(existing)
}

async fn write_data_locked(
    conn: &mut AsyncPgConnection,
    existing: Option<DbUserData>,
    user_id: &UserId,
    room_id: Option<OwnedRoomId>,
    event_type: &str,
    json_data: JsonValue,
) -> DataResult<DbUserData> {
    if let Some(existing) = &existing
        && !existing.is_deleted
        && existing.json_data == json_data
    {
        return Ok(existing.clone());
    }

    if let Some(existing) = existing {
        diesel::update(user_datas::table.find(existing.id))
            .set((
                user_datas::json_data.eq(&json_data),
                user_datas::is_deleted.eq(false),
                user_datas::occur_sn.eq(next_sn_locked(conn).await?),
                user_datas::created_at.eq(UnixMillis::now()),
            ))
            .get_result::<DbUserData>(conn)
            .await
            .map_err(Into::into)
    } else {
        let new_data = NewDbUserData {
            user_id: user_id.to_owned(),
            room_id: room_id.clone(),
            data_type: event_type.to_owned(),
            json_data,
            is_deleted: false,
            occur_sn: Some(next_sn_locked(conn).await?),
            created_at: UnixMillis::now(),
        };
        diesel::insert_into(user_datas::table)
            .values(&new_data)
            .get_result::<DbUserData>(conn)
            .await
            .map_err(Into::into)
    }
}

async fn next_sn_locked(conn: &mut AsyncPgConnection) -> DataResult<i64> {
    diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT nextval('occur_sn_seq')")
        .get_result(conn)
        .await
        .map_err(Into::into)
}

#[tracing::instrument]
pub async fn get_data<E: DeserializeOwned>(
    user_id: &UserId,
    room_id: Option<&RoomId>,
    kind: &str,
) -> DataResult<E> {
    let mut conn = connect().await?;
    let row = if let Some(room_id) = room_id {
        let room_row = user_datas::table
            .filter(user_datas::user_id.eq(user_id))
            .filter(user_datas::room_id.eq(room_id))
            .filter(user_datas::data_type.eq(kind))
            .order_by(user_datas::id.desc())
            .first::<DbUserData>(&mut conn)
            .await
            .optional()?;

        if let Some(row) = room_row {
            row
        } else {
            user_datas::table
                .filter(user_datas::user_id.eq(user_id))
                .filter(user_datas::room_id.is_null())
                .filter(user_datas::data_type.eq(kind))
                .order_by(user_datas::id.desc())
                .first::<DbUserData>(&mut conn)
                .await?
        }
    } else {
        user_datas::table
            .filter(user_datas::user_id.eq(user_id))
            .filter(user_datas::room_id.is_null())
            .filter(user_datas::data_type.eq(kind))
            .order_by(user_datas::id.desc())
            .first::<DbUserData>(&mut conn)
            .await?
    };

    if row.is_deleted {
        return Err(diesel::result::Error::NotFound.into());
    }
    Ok(serde_json::from_value(row.json_data)?)
}

/// Searches the account data for a specific kind.
#[tracing::instrument]
pub async fn get_room_data<E: DeserializeOwned>(
    user_id: &UserId,
    room_id: &RoomId,
    kind: &str,
) -> DataResult<Option<E>> {
    let row = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(user_datas::room_id.eq(room_id))
        .filter(user_datas::data_type.eq(kind))
        .order_by(user_datas::id.desc())
        .first::<DbUserData>(&mut connect().await?)
        .await
        .optional()?;
    if let Some(row) = row {
        if row.is_deleted {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(row.json_data)?))
    } else {
        Ok(None)
    }
}

#[tracing::instrument]
pub async fn get_global_data<E: DeserializeOwned>(
    user_id: &UserId,
    kind: &str,
) -> DataResult<Option<E>> {
    let row = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(user_datas::room_id.is_null())
        .filter(user_datas::data_type.eq(kind))
        .order_by(user_datas::id.desc())
        .first::<DbUserData>(&mut connect().await?)
        .await
        .optional()?;
    if let Some(row) = row {
        if row.is_deleted {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(row.json_data)?))
    } else {
        Ok(None)
    }
}

pub async fn delete_global_data(user_id: &UserId, kind: &str) -> DataResult<()> {
    let mut conn = connect().await?;
    conn.transaction::<_, crate::DataError, _>(async |conn| {
        lock_data_key(conn, user_id, None, kind).await?;
        delete_data_locked(conn, user_id, None, kind).await
    })
    .await
}

/// Delete a room-scoped account-data entry, keeping a tombstone row so the
/// deletion propagates through sync (MSC3391), mirroring `delete_global_data`.
pub async fn delete_room_data(user_id: &UserId, room_id: &RoomId, kind: &str) -> DataResult<()> {
    let mut conn = connect().await?;
    conn.transaction::<_, crate::DataError, _>(async |conn| {
        lock_data_key(conn, user_id, Some(room_id), kind).await?;
        delete_data_locked(conn, user_id, Some(room_id), kind).await
    })
    .await
}

async fn delete_data_locked(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    room_id: Option<&RoomId>,
    kind: &str,
) -> DataResult<()> {
    let existing = get_latest_data(conn, user_id, room_id, kind).await?;
    let Some(existing) = existing else {
        return Ok(());
    };

    if let Some(room_id) = room_id {
        diesel::delete(
            user_datas::table
                .filter(user_datas::user_id.eq(user_id))
                .filter(user_datas::room_id.eq(room_id))
                .filter(user_datas::data_type.eq(kind))
                .filter(user_datas::id.ne(existing.id)),
        )
        .execute(&mut *conn)
        .await?;
    } else {
        diesel::delete(
            user_datas::table
                .filter(user_datas::user_id.eq(user_id))
                .filter(user_datas::room_id.is_null())
                .filter(user_datas::data_type.eq(kind))
                .filter(user_datas::id.ne(existing.id)),
        )
        .execute(&mut *conn)
        .await?;
    }

    if existing.is_deleted {
        return Ok(());
    }

    diesel::update(user_datas::table.find(existing.id))
        .set((
            user_datas::json_data.eq(json!({})),
            user_datas::is_deleted.eq(true),
            user_datas::occur_sn.eq(next_sn_locked(conn).await?),
            user_datas::created_at.eq(UnixMillis::now()),
        ))
        .execute(conn)
        .await?;

    Ok(())
}

/// Load all global account-data rows for a user.
pub async fn get_global_datas(user_id: &UserId) -> DataResult<Vec<DbUserData>> {
    user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(user_datas::room_id.is_null())
        .load::<DbUserData>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Get all global account data for a user
pub async fn get_global_account_data(user_id: &UserId) -> DataResult<HashMap<String, JsonValue>> {
    user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(user_datas::room_id.is_null())
        .select((
            user_datas::data_type,
            user_datas::json_data,
            user_datas::is_deleted,
        ))
        .load::<(String, JsonValue, bool)>(&mut connect().await?)
        .await
        .map(|rows| {
            rows.into_iter()
                .filter_map(|(data_type, json_data, is_deleted)| {
                    (!is_deleted).then_some((data_type, json_data))
                })
                .collect()
        })
        .map_err(Into::into)
}

/// Get all room-specific account data for a user
pub async fn get_room_account_data(
    user_id: &UserId,
) -> DataResult<HashMap<String, HashMap<String, JsonValue>>> {
    let rows = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(user_datas::room_id.is_not_null())
        .select((
            user_datas::room_id,
            user_datas::data_type,
            user_datas::json_data,
            user_datas::is_deleted,
        ))
        .load::<(Option<OwnedRoomId>, String, JsonValue, bool)>(&mut connect().await?)
        .await?;

    let mut result = HashMap::new();
    for (room_id, data_type, json_data, is_deleted) in rows {
        if let Some(room_id) = room_id {
            if is_deleted {
                continue;
            }
            result
                .entry(room_id.to_string())
                .or_insert_with(HashMap::new)
                .insert(data_type, json_data);
        }
    }
    Ok(result)
}

/// Returns all changes to the account data that happened after `since`.
#[tracing::instrument(skip(room_id, user_id, since_sn))]
pub async fn data_changes(
    room_id: Option<&RoomId>,
    user_id: &UserId,
    since_sn: Seqnum,
    until_sn: Option<Seqnum>,
) -> DataResult<Vec<AnyRawAccountDataEvent>> {
    let mut user_datas = Vec::new();

    let query = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(
            user_datas::room_id
                .eq(room_id)
                .or(user_datas::room_id.is_null()),
        )
        .filter(user_datas::occur_sn.ge(since_sn))
        .into_boxed();
    let db_datas = if let Some(until_sn) = until_sn {
        query
            .filter(user_datas::occur_sn.le(until_sn))
            .order_by(user_datas::occur_sn.asc())
            .load::<DbUserData>(&mut connect().await?)
            .await?
    } else {
        query
            .order_by(user_datas::occur_sn.asc())
            .load::<DbUserData>(&mut connect().await?)
            .await?
    };

    for db_data in db_datas {
        if since_sn == 0 && db_data.is_deleted {
            continue;
        }
        let kind = RoomAccountDataEventType::from(&*db_data.data_type);
        let account_data = json!({
            "type": kind,
            "content": db_data.json_data
        });
        if db_data.room_id.is_none() {
            user_datas.push(AnyRawAccountDataEvent::Global(RawJson::from_value(
                &account_data,
            )?));
        } else {
            user_datas.push(AnyRawAccountDataEvent::Room(RawJson::from_value(
                &account_data,
            )?));
        }
    }

    Ok(user_datas)
}
