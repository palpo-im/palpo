use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel_async::AsyncPgConnection;
use horme::{OptionalExtension as _, RunQueryDsl};

use crate::DataResult;
use crate::core::UnixMillis;
use crate::core::client::backup::{BackupAlgorithm, KeyBackupData, ModifyKeysResBody};
use crate::core::identifiers::*;
use crate::core::serde::{JsonValue, RawJson};
use crate::schema::*;

/// Version creation, retirement and key mutations share this lock, so a write
/// cannot validate the latest version and then race a new version or deletion.
async fn lock_backups(conn: &mut AsyncPgConnection, user_id: &UserId) -> DataResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 7141321))")
        .bind::<diesel::sql_types::Text, _>(user_id.as_str())
        .execute(conn)
        .await?;
    Ok(())
}

async fn mutation_version(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    version: i64,
    latest_only: bool,
) -> DataResult<DbRoomKeysVersion> {
    let mut query = e2e_room_keys_versions::table
        .filter(e2e_room_keys_versions::user_id.eq(user_id))
        .filter(e2e_room_keys_versions::is_trashed.eq(false))
        .into_boxed();
    if !latest_only {
        query = query.filter(e2e_room_keys_versions::version.eq(version));
    }
    let backup = query
        .order(e2e_room_keys_versions::version.desc())
        .first::<DbRoomKeysVersion>(conn)
        .await
        .optional()?
        .ok_or_else(|| crate::core::MatrixError::not_found("Key backup does not exist."))?;
    if latest_only && backup.version != version {
        let mut error = crate::core::MatrixError::wrong_room_keys_version(
            serde_json::Map::from_iter([
                (
                    "error".to_owned(),
                    JsonValue::from("You may only write to the current key backup."),
                ),
                (
                    "current_version".to_owned(),
                    JsonValue::from(backup.version.to_string()),
                ),
            ]),
            Some(backup.version.to_string()),
        );
        error.status_code = Some(salvo::http::StatusCode::FORBIDDEN);
        return Err(error.into());
    }
    Ok(backup)
}

async fn bump_etag(conn: &mut AsyncPgConnection, backup_id: i64) -> DataResult<()> {
    diesel::update(e2e_room_keys_versions::table.find(backup_id))
        .set(e2e_room_keys_versions::etag.eq(e2e_room_keys_versions::etag + 1))
        .execute(conn)
        .await?;
    Ok(())
}

async fn mutation_response(
    conn: &mut AsyncPgConnection,
    backup: &DbRoomKeysVersion,
) -> DataResult<ModifyKeysResBody> {
    let count = e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(&backup.user_id))
        .filter(e2e_room_keys::version.eq(backup.version))
        .count()
        .get_result::<i64>(conn)
        .await?;
    let etag = e2e_room_keys_versions::table
        .find(backup.id)
        .select(e2e_room_keys_versions::etag)
        .first::<i64>(conn)
        .await?;
    Ok(ModifyKeysResBody::new(etag.to_string(), count as u64))
}

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

pub async fn create_backup(
    user_id: &UserId,
    algorithm: &RawJson<BackupAlgorithm>,
) -> DataResult<DbRoomKeysVersion> {
    crate::db()
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_backups(conn, user_id).await?;
            let previous = e2e_room_keys_versions::table
                .filter(e2e_room_keys_versions::user_id.eq(user_id))
                .select(diesel::dsl::max(e2e_room_keys_versions::version))
                .first::<Option<i64>>(conn)
                .await?;
            let version = (UnixMillis::now().get() as i64).max(
                previous
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| crate::DataError::internal("key backup version exhausted"))?,
            );
            let new_keys_version = NewDbRoomKeysVersion {
                user_id: user_id.to_owned(),
                version,
                algorithm: serde_json::to_value(algorithm)?,
                auth_data: serde_json::to_value(BTreeMap::<String, JsonValue>::new())?,
                created_at: UnixMillis::now(),
            };
            diesel::insert_into(e2e_room_keys_versions::table)
                .values(&new_keys_version)
                .get_result(conn)
                .await
                .map_err(Into::into)
        })
        .await
}

/// Update an existing backup version's `auth_data` in place and bump its etag.
///
/// The version identifier is preserved (per the Matrix spec, `PUT
/// /room_keys/version/{version}` updates the named version rather than creating
/// a new one). Trashed versions are ignored. Returns `true` if a matching,
/// live version was updated, `false` if no such version exists.
pub async fn update_backup(
    user_id: &UserId,
    version: i64,
    algorithm: &BackupAlgorithm,
) -> DataResult<bool> {
    crate::db()
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_backups(conn, user_id).await?;
            let backup = match mutation_version(conn, user_id, version, false).await {
                Ok(backup) => backup,
                Err(error) if error.is_not_found() => return Ok(false),
                Err(error) => return Err(error),
            };
            let algorithm = serde_json::to_value(algorithm)?;
            if backup.algorithm.get("algorithm") != algorithm.get("algorithm") {
                return Err(crate::core::MatrixError::invalid_param(
                    "The backup algorithm cannot be changed.",
                )
                .into());
            }
            if backup.algorithm != algorithm {
                diesel::update(e2e_room_keys_versions::table.find(backup.id))
                    .set((
                        e2e_room_keys_versions::algorithm.eq(algorithm),
                        e2e_room_keys_versions::etag.eq(e2e_room_keys_versions::etag + 1),
                    ))
                    .execute(conn)
                    .await?;
            }
            Ok(true)
        })
        .await
}

pub async fn get_latest_room_key(user_id: &UserId) -> DataResult<Option<DbRoomKey>> {
    e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(user_id))
        .order(e2e_room_keys::version.desc())
        .first::<DbRoomKey>(&mut crate::db())
        .await
        .optional()
        .map_err(Into::into)
}

pub async fn get_room_key(
    user_id: &UserId,
    room_id: &RoomId,
    version: i64,
) -> DataResult<Option<DbRoomKey>> {
    e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(user_id))
        .filter(e2e_room_keys::room_id.eq(room_id))
        .filter(e2e_room_keys::version.eq(version))
        .first::<DbRoomKey>(&mut crate::db())
        .await
        .optional()
        .map_err(Into::into)
}

pub async fn get_latest_room_keys_version(
    user_id: &UserId,
) -> DataResult<Option<DbRoomKeysVersion>> {
    e2e_room_keys_versions::table
        .filter(e2e_room_keys_versions::user_id.eq(user_id))
        .filter(e2e_room_keys_versions::is_trashed.eq(false))
        .order(e2e_room_keys_versions::version.desc())
        .first::<DbRoomKeysVersion>(&mut crate::db())
        .await
        .optional()
        .map_err(Into::into)
}
pub async fn get_room_keys_version(
    user_id: &UserId,
    version: i64,
) -> DataResult<Option<DbRoomKeysVersion>> {
    e2e_room_keys_versions::table
        .filter(e2e_room_keys_versions::user_id.eq(user_id))
        .filter(e2e_room_keys_versions::version.eq(version))
        .filter(e2e_room_keys_versions::is_trashed.eq(false))
        .first::<DbRoomKeysVersion>(&mut crate::db())
        .await
        .optional()
        .map_err(Into::into)
}

pub async fn add_key(
    user_id: &UserId,
    version: i64,
    room_id: &RoomId,
    session_id: &String,
    key_data: &KeyBackupData,
) -> DataResult<()> {
    store_keys(
        user_id,
        version,
        &[(room_id, session_id.as_str(), key_data)],
    )
    .await?;
    Ok(())
}

/// Store one upload atomically, returning the count and etag of that same commit.
/// PostgreSQL evaluates the quality predicate against the conflicting row while
/// holding its lock; a concurrent upload can never overwrite a better key.
pub async fn store_keys(
    user_id: &UserId,
    version: i64,
    keys: &[(&RoomId, &str, &KeyBackupData)],
) -> DataResult<ModifyKeysResBody> {
    let keys = keys
        .iter()
        .map(|(room_id, session_id, key_data)| {
            Ok(NewDbRoomKey {
                user_id: user_id.to_owned(),
                room_id: (*room_id).to_owned(),
                session_id: (*session_id).to_owned(),
                version,
                first_message_index: Some(i64::try_from(key_data.first_message_index).map_err(
                    |_| {
                        crate::core::MatrixError::invalid_param(
                            "Backup message index is too large.",
                        )
                    },
                )?),
                forwarded_count: Some(i64::try_from(key_data.forwarded_count).map_err(|_| {
                    crate::core::MatrixError::invalid_param("Backup forwarded count is too large.")
                })?),
                is_verified: key_data.is_verified,
                session_data: serde_json::to_value(&key_data.session_data)?,
                created_at: UnixMillis::now(),
            })
        })
        .collect::<DataResult<Vec<_>>>()?;
    crate::db()
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_backups(conn, user_id).await?;
            let backup = mutation_version(conn, user_id, version, true).await?;
            let mut changed = false;
            for key in &keys {
                use diesel::query_dsl::methods::FilterDsl;
                let affected = diesel::insert_into(e2e_room_keys::table)
                    .values(key)
                    .on_conflict((
                        e2e_room_keys::user_id,
                        e2e_room_keys::room_id,
                        e2e_room_keys::session_id,
                        e2e_room_keys::version,
                    ))
                    .do_update()
                    .set(key)
                    .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
                        "(EXCLUDED.is_verified AND NOT e2e_room_keys.is_verified) OR \
                        (EXCLUDED.is_verified = e2e_room_keys.is_verified AND \
                        (EXCLUDED.first_message_index < COALESCE(e2e_room_keys.first_message_index, 0) OR \
                        (EXCLUDED.first_message_index = COALESCE(e2e_room_keys.first_message_index, 0) AND \
                        EXCLUDED.forwarded_count < COALESCE(e2e_room_keys.forwarded_count, 0))))",
                    ))
                    .execute(conn)
                    .await?;
                changed |= affected > 0;
            }
            if changed {
                bump_etag(conn, backup.id).await?;
            }
            mutation_response(conn, &backup).await
        })
        .await
}

pub async fn count_keys(user_id: &UserId, version: i64) -> DataResult<i64> {
    e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(user_id))
        .filter(e2e_room_keys::version.eq(version))
        .count()
        .get_result(&mut crate::db())
        .await
        .map_err(Into::into)
}

pub async fn get_etag(user_id: &UserId, version: i64) -> DataResult<String> {
    e2e_room_keys_versions::table
        .filter(e2e_room_keys_versions::user_id.eq(user_id))
        .filter(e2e_room_keys_versions::version.eq(version))
        .select(e2e_room_keys_versions::etag)
        .first(&mut crate::db())
        .await
        .map(|etag: i64| etag.to_string())
        .map_err(Into::into)
}

pub async fn get_key_for_session(
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
        .first::<DbRoomKey>(&mut crate::db())
        .await
        .optional()
        .map_err(Into::into)
}

pub async fn delete_backup(user_id: &UserId, version: i64) -> DataResult<()> {
    crate::db()
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_backups(conn, user_id).await?;
            let backup = e2e_room_keys_versions::table
                .filter(e2e_room_keys_versions::user_id.eq(user_id))
                .filter(e2e_room_keys_versions::version.eq(version))
                .first::<DbRoomKeysVersion>(conn)
                .await
                .optional()?
                .ok_or_else(|| crate::core::MatrixError::not_found("Key backup does not exist."))?;
            // Retrying a successful retirement is idempotent. The named
            // version remains unavailable to every key mutation.
            if backup.is_trashed {
                return Ok(());
            }
            diesel::delete(
                e2e_room_keys::table
                    .filter(e2e_room_keys::user_id.eq(user_id))
                    .filter(e2e_room_keys::version.eq(version)),
            )
            .execute(conn)
            .await?;
            diesel::update(e2e_room_keys_versions::table.find(backup.id))
                .set((
                    e2e_room_keys_versions::is_trashed.eq(true),
                    e2e_room_keys_versions::etag.eq(e2e_room_keys_versions::etag + 1),
                ))
                .execute(conn)
                .await?;
            Ok(())
        })
        .await
}

/// Delete keys and advance their backup's etag in one transaction. Old live
/// versions remain deletable; absent or retired versions are not writable.
pub async fn delete_keys(
    user_id: &UserId,
    version: i64,
    room_id: Option<&RoomId>,
    session_id: Option<&str>,
) -> DataResult<ModifyKeysResBody> {
    crate::db()
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_backups(conn, user_id).await?;
            let backup = mutation_version(conn, user_id, version, false).await?;
            let mut query = e2e_room_keys::table
                .filter(e2e_room_keys::user_id.eq(user_id))
                .filter(e2e_room_keys::version.eq(version))
                .into_boxed();
            if let Some(room_id) = room_id {
                query = query.filter(e2e_room_keys::room_id.eq(room_id));
            }
            if let Some(session_id) = session_id {
                query = query.filter(e2e_room_keys::session_id.eq(session_id));
            }
            // Keep selection inside SQL, including for a whole-backup deletion.
            let affected = diesel::delete(
                e2e_room_keys::table
                    .filter(e2e_room_keys::id.eq_any(query.select(e2e_room_keys::id))),
            )
            .execute(conn)
            .await?;
            if affected > 0 {
                bump_etag(conn, backup.id).await?;
            }
            mutation_response(conn, &backup).await
        })
        .await
}

pub async fn delete_all_keys(user_id: &UserId, version: i64) -> DataResult<()> {
    delete_keys(user_id, version, None, None).await?;
    Ok(())
}

pub async fn delete_room_keys(user_id: &UserId, version: i64, room_id: &RoomId) -> DataResult<()> {
    delete_keys(user_id, version, Some(room_id), None).await?;
    Ok(())
}

pub async fn delete_room_key(
    user_id: &UserId,
    version: i64,
    room_id: &RoomId,
    session_id: &String,
) -> DataResult<()> {
    delete_keys(user_id, version, Some(room_id), Some(session_id)).await?;
    Ok(())
}
