use diesel::prelude::*;
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
pub fn set_data(
    user_id: &UserId,
    room_id: Option<OwnedRoomId>,
    event_type: &str,
    json_data: JsonValue,
) -> DataResult<DbUserData> {
    if let Some(room_id) = &room_id {
        let user_data = user_datas::table
            .filter(user_datas::user_id.eq(user_id))
            .filter(user_datas::room_id.eq(room_id))
            .filter(user_datas::data_type.eq(event_type))
            .first::<DbUserData>(&mut connect()?)
            .optional()?;
        if let Some(user_data) = user_data
            && user_data.json_data == json_data
        {
            return Ok(user_data);
        }
    } else {
        let user_data = user_datas::table
            .filter(user_datas::user_id.eq(user_id))
            .filter(user_datas::room_id.is_null())
            .filter(user_datas::data_type.eq(event_type))
            .first::<DbUserData>(&mut connect()?)
            .optional()?;
        if let Some(user_data) = user_data
            && user_data.json_data == json_data
        {
            return Ok(user_data);
        }
    }

    let new_data = NewDbUserData {
        user_id: user_id.to_owned(),
        room_id: room_id.clone(),
        data_type: event_type.to_owned(),
        json_data,
        occur_sn: None,
        created_at: UnixMillis::now(),
    };
    diesel::insert_into(user_datas::table)
        .values(&new_data)
        .on_conflict((
            user_datas::user_id,
            user_datas::room_id,
            user_datas::data_type,
        ))
        .do_update()
        .set(&new_data)
        .get_result::<DbUserData>(&mut connect()?)
        .map_err(Into::into)
}

#[tracing::instrument]
pub fn get_data<E: DeserializeOwned>(
    user_id: &UserId,
    room_id: Option<&RoomId>,
    kind: &str,
) -> DataResult<Option<E>> {
    let row = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(
            user_datas::room_id
                .eq(room_id)
                .or(user_datas::room_id.is_null()),
        )
        .filter(user_datas::data_type.eq(kind))
        .order_by(user_datas::id.desc())
        .first::<DbUserData>(&mut connect()?)
        .optional()?;
    if let Some(row) = row {
        Ok(Some(serde_json::from_value(row.json_data)?))
    } else {
        Ok(None)
    }
}

/// Searches the account data for a specific kind.
#[tracing::instrument]
pub fn get_room_data<E: DeserializeOwned>(
    user_id: &UserId,
    room_id: &RoomId,
    kind: &str,
) -> DataResult<Option<E>> {
    let row = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(user_datas::room_id.eq(room_id))
        .filter(user_datas::data_type.eq(kind))
        .order_by(user_datas::id.desc())
        .first::<DbUserData>(&mut connect()?)
        .optional()?;
    if let Some(row) = row {
        Ok(Some(serde_json::from_value(row.json_data)?))
    } else {
        Ok(None)
    }
}

#[tracing::instrument]
pub fn get_global_data<E: DeserializeOwned>(user_id: &UserId, kind: &str) -> DataResult<Option<E>> {
    let row = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(user_datas::room_id.is_null())
        .filter(user_datas::data_type.eq(kind))
        .order_by(user_datas::id.desc())
        .first::<DbUserData>(&mut connect()?)
        .optional()?;
    if let Some(row) = row {
        Ok(Some(serde_json::from_value(row.json_data)?))
    } else {
        Ok(None)
    }
}

/// Returns all changes to the account data that happened after `since`.
#[tracing::instrument(skip(room_id, user_id, since_sn))]
pub fn data_changes(
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
            .load::<DbUserData>(&mut connect()?)?
    } else {
        query
            .order_by(user_datas::occur_sn.asc())
            .load::<DbUserData>(&mut connect()?)?
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
