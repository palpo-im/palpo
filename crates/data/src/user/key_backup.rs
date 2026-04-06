use std::collections::BTreeMap;

use diesel::prelude::*;

use crate::core::UnixMillis;
use crate::core::client::backup::{BackupAlgorithm, KeyBackupData};
use crate::core::identifiers::*;
use crate::core::serde::{JsonValue, RawJson};
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = e2e_room_keys)]
pub struct DbRoomKey {
    pub id: i64,

    pub user_id: OwnedUserId,
    pub room_id: OwnedRoomId,
    pub session_id: String,

    pub version: i64,

    pub first_message_index: Option<i64>,
    pub forwarded_count: Option<i64>,
    pub is_verified: bool,
    pub session_data: JsonValue,
    pub created_at: UnixMillis,
}
#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = e2e_room_keys)]
pub struct NewDbRoomKey {
    pub user_id: OwnedUserId,
    pub room_id: OwnedRoomId,
    pub session_id: String,

    pub version: i64,

    pub first_message_index: Option<i64>,
    pub forwarded_count: Option<i64>,
    pub is_verified: bool,
    pub session_data: JsonValue,
    pub created_at: UnixMillis,
}
impl From<DbRoomKey> for KeyBackupData {
    fn from(val: DbRoomKey) -> Self {
        KeyBackupData {
            first_message_index: val.first_message_index.unwrap_or(0) as u64,
            forwarded_count: val.forwarded_count.unwrap_or(0) as u64,
            is_verified: val.is_verified,
            session_data: serde_json::from_value(val.session_data).unwrap(),
        }
    }
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = e2e_room_keys_versions)]
pub struct DbRoomKeysVersion {
    pub id: i64,

    pub user_id: OwnedUserId,
    pub version: i64,
    pub algorithm: JsonValue,
    pub auth_data: JsonValue,
    pub is_trashed: bool,
    pub etag: i64,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = e2e_room_keys_versions)]
pub struct NewDbRoomKeysVersion {
    pub user_id: OwnedUserId,
    pub version: i64,
    pub algorithm: JsonValue,
    pub auth_data: JsonValue,
    pub created_at: UnixMillis,
}

pub fn create_backup(
    user_id: &UserId,
    algorithm: &RawJson<BackupAlgorithm>,
) -> DataResult<DbRoomKeysVersion> {
    let version = UnixMillis::now().get() as i64;
    let new_keys_version = NewDbRoomKeysVersion {
        user_id: user_id.to_owned(),
        version,
        algorithm: serde_json::to_value(algorithm)?,
        auth_data: serde_json::to_value(BTreeMap::<String, JsonValue>::new())?,
        created_at: UnixMillis::now(),
    };
    diesel::insert_into(e2e_room_keys_versions::table)
        .values(&new_keys_version)
        .get_result(&mut connect()?)
        .map_err(Into::into)
}

pub fn update_backup(
    user_id: &UserId,
    version: i64,
    algorithm: &BackupAlgorithm,
) -> DataResult<()> {
    diesel::update(
        e2e_room_keys_versions::table
            .filter(e2e_room_keys_versions::user_id.eq(user_id))
            .filter(e2e_room_keys_versions::version.eq(version)),
    )
    .set((
        e2e_room_keys_versions::algorithm.eq(serde_json::to_value(algorithm)?),
        e2e_room_keys_versions::etag.eq(UnixMillis::now().get() as i64),
    ))
    .execute(&mut connect()?)?;
    Ok(())
}

pub fn get_latest_room_key(user_id: &UserId) -> DataResult<Option<DbRoomKey>> {
    e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(user_id))
        .order(e2e_room_keys::version.desc())
        .first::<DbRoomKey>(&mut connect()?)
        .optional()
        .map_err(Into::into)
}

pub fn get_room_key(
    user_id: &UserId,
    room_id: &RoomId,
    version: i64,
) -> DataResult<Option<DbRoomKey>> {
    e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(user_id))
        .filter(e2e_room_keys::room_id.eq(room_id))
        .filter(e2e_room_keys::version.eq(version))
        .first::<DbRoomKey>(&mut connect()?)
        .optional()
        .map_err(Into::into)
}

pub fn get_latest_room_keys_version(user_id: &UserId) -> DataResult<Option<DbRoomKeysVersion>> {
    e2e_room_keys_versions::table
        .filter(e2e_room_keys_versions::user_id.eq(user_id))
        .filter(e2e_room_keys_versions::is_trashed.eq(false))
        .order(e2e_room_keys_versions::version.desc())
        .first::<DbRoomKeysVersion>(&mut connect()?)
        .optional()
        .map_err(Into::into)
}
pub fn get_room_keys_version(
    user_id: &UserId,
    version: i64,
) -> DataResult<Option<DbRoomKeysVersion>> {
    e2e_room_keys_versions::table
        .filter(e2e_room_keys_versions::user_id.eq(user_id))
        .filter(e2e_room_keys_versions::version.eq(version))
        .filter(e2e_room_keys_versions::is_trashed.eq(false))
        .first::<DbRoomKeysVersion>(&mut connect()?)
        .optional()
        .map_err(Into::into)
}

pub fn add_key(
    user_id: &UserId,
    version: i64,
    room_id: &RoomId,
    session_id: &String,
    key_data: &KeyBackupData,
) -> DataResult<()> {
    let new_key = NewDbRoomKey {
        user_id: user_id.to_owned(),
        room_id: room_id.to_owned(),
        session_id: session_id.to_owned(),
        version: version.to_owned(),
        first_message_index: Some(key_data.first_message_index as i64),
        forwarded_count: Some(key_data.forwarded_count as i64),
        is_verified: key_data.is_verified,
        session_data: serde_json::to_value(&key_data.session_data)?,
        created_at: UnixMillis::now(),
    };

    let exist_key = get_key_for_session(user_id, version, room_id, session_id)?;
    let replace = if let Some(exist_key) = exist_key {
        if (new_key.is_verified && !exist_key.is_verified)
            || new_key.first_message_index < exist_key.first_message_index
        {
            true
        } else if new_key.first_message_index == exist_key.first_message_index {
            new_key.forwarded_count < exist_key.forwarded_count
        } else {
            false
        }
    } else {
        true
    };
    if replace {
        diesel::insert_into(e2e_room_keys::table)
            .values(&new_key)
            .on_conflict((
                e2e_room_keys::user_id,
                e2e_room_keys::room_id,
                e2e_room_keys::session_id,
                e2e_room_keys::version,
            ))
            .do_update()
            .set(&new_key)
            .execute(&mut connect()?)?;
    }
    Ok(())
}

pub fn count_keys(user_id: &UserId, version: i64) -> DataResult<i64> {
    e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(user_id))
        .filter(e2e_room_keys::version.eq(version))
        .count()
        .get_result(&mut connect()?)
        .map_err(Into::into)
}

pub fn get_etag(user_id: &UserId, version: i64) -> DataResult<String> {
    e2e_room_keys_versions::table
        .filter(e2e_room_keys_versions::user_id.eq(user_id))
        .filter(e2e_room_keys_versions::version.eq(version))
        .select(e2e_room_keys_versions::etag)
        .first(&mut connect()?)
        .map(|etag: i64| etag.to_string())
        .map_err(Into::into)
}

pub fn get_key_for_session(
    user_id: &UserId,
    version: i64,
    room_id: &RoomId,
    session_id: &String,
) -> DataResult<Option<DbRoomKey>> {
    e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(user_id))
        .filter(e2e_room_keys::version.eq(version))
        .filter(e2e_room_keys::room_id.eq(room_id))
        .filter(e2e_room_keys::session_id.eq(session_id))
        .first::<DbRoomKey>(&mut connect()?)
        .optional()
        .map_err(Into::into)
}

pub fn delete_backup(user_id: &UserId, version: i64) -> DataResult<()> {
    delete_all_keys(user_id, version)?;
    diesel::update(
        e2e_room_keys_versions::table
            .filter(e2e_room_keys_versions::user_id.eq(user_id))
            .filter(e2e_room_keys_versions::version.eq(version)),
    )
    .set(e2e_room_keys_versions::is_trashed.eq(true))
    .execute(&mut connect()?)?;
    Ok(())
}

pub fn delete_all_keys(user_id: &UserId, version: i64) -> DataResult<()> {
    diesel::delete(
        e2e_room_keys::table
            .filter(e2e_room_keys::user_id.eq(user_id))
            .filter(e2e_room_keys::version.eq(version)),
    )
    .execute(&mut connect()?)?;
    Ok(())
}

pub fn delete_room_keys(user_id: &UserId, version: i64, room_id: &RoomId) -> DataResult<()> {
    diesel::delete(
        e2e_room_keys::table
            .filter(e2e_room_keys::user_id.eq(user_id))
            .filter(e2e_room_keys::version.eq(version))
            .filter(e2e_room_keys::room_id.eq(room_id)),
    )
    .execute(&mut connect()?)?;
    Ok(())
}

pub fn delete_room_key(
    user_id: &UserId,
    version: i64,
    room_id: &RoomId,
    session_id: &String,
) -> DataResult<()> {
    diesel::delete(
        e2e_room_keys::table
            .filter(e2e_room_keys::user_id.eq(user_id))
            .filter(e2e_room_keys::version.eq(version))
            .filter(e2e_room_keys::room_id.eq(room_id))
            .filter(e2e_room_keys::session_id.eq(session_id)),
    )
    .execute(&mut connect()?)?;
    Ok(())
}
