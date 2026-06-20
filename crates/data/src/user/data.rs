use std::collections::HashMap;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
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
    // Locate the current row explicitly. Global account data is stored with
    // `room_id = NULL`, and the `user_datas_udx` unique index treats NULLs as
    // distinct (Postgres default), so `ON CONFLICT (user_id, room_id,
    // data_type)` never matches a NULL `room_id` and would insert a duplicate
    // row on every update. We therefore find the latest existing row and
    // update it in place (or insert when none exists).
    let existing = if let Some(room_id) = &room_id {
        user_datas::table
            .filter(user_datas::user_id.eq(user_id))
            .filter(user_datas::room_id.eq(room_id))
            .filter(user_datas::data_type.eq(event_type))
            .order_by(user_datas::id.desc())
            .first::<DbUserData>(&mut connect().await?)
            .await
            .optional()?
    } else {
        user_datas::table
            .filter(user_datas::user_id.eq(user_id))
            .filter(user_datas::room_id.is_null())
            .filter(user_datas::data_type.eq(event_type))
            .order_by(user_datas::id.desc())
            .first::<DbUserData>(&mut connect().await?)
            .await
            .optional()?
    };

    if let Some(existing) = &existing
        && existing.json_data == json_data
    {
        return Ok(existing.clone());
    }

    if let Some(existing) = existing {
        diesel::update(user_datas::table.find(existing.id))
            .set((
                user_datas::json_data.eq(&json_data),
                user_datas::occur_sn.eq(crate::next_sn().await?),
                user_datas::created_at.eq(UnixMillis::now()),
            ))
            .get_result::<DbUserData>(&mut connect().await?)
            .await
            .map_err(Into::into)
    } else {
        let new_data = NewDbUserData {
            user_id: user_id.to_owned(),
            room_id: room_id.clone(),
            data_type: event_type.to_owned(),
            json_data,
            occur_sn: Some(crate::next_sn().await?),
            created_at: UnixMillis::now(),
        };
        diesel::insert_into(user_datas::table)
            .values(&new_data)
            .get_result::<DbUserData>(&mut connect().await?)
            .await
            .map_err(Into::into)
    }
}

#[tracing::instrument]
pub async fn get_data<E: DeserializeOwned>(
    user_id: &UserId,
    room_id: Option<&RoomId>,
    kind: &str,
) -> DataResult<E> {
    let row = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(
            user_datas::room_id
                .eq(room_id)
                .or(user_datas::room_id.is_null()),
        )
        .filter(user_datas::data_type.eq(kind))
        .order_by(user_datas::id.desc())
        .first::<DbUserData>(&mut connect().await?)
        .await?;
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
        Ok(Some(serde_json::from_value(row.json_data)?))
    } else {
        Ok(None)
    }
}

pub async fn delete_global_data(user_id: &UserId, kind: &str) -> DataResult<()> {
    diesel::delete(
        user_datas::table
            .filter(user_datas::user_id.eq(user_id))
            .filter(user_datas::room_id.is_null())
            .filter(user_datas::data_type.eq(kind)),
    )
    .execute(&mut connect().await?)
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
        .select((user_datas::data_type, user_datas::json_data))
        .load::<(String, JsonValue)>(&mut connect().await?)
        .await
        .map(|rows| rows.into_iter().collect())
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
        ))
        .load::<(Option<OwnedRoomId>, String, JsonValue)>(&mut connect().await?)
        .await?;

    let mut result = HashMap::new();
    for (room_id, data_type, json_data) in rows {
        if let Some(room_id) = room_id {
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
