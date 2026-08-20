use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel::sql_types::{Jsonb, Text};
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use crate::core::identifiers::*;
use crate::core::serde::{JsonObject, JsonValue};
use crate::core::{MatrixError, MxcUri, OwnedMxcUri, Seqnum};
use crate::schema::*;
use crate::{DataResult, connect};

const PROFILE_STREAM_LOCK_ID: i64 = 1_346_426_200;

async fn lock_profile_stream(conn: &mut AsyncPgConnection) -> Result<(), DieselError> {
    diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
        .bind::<diesel::sql_types::BigInt, _>(PROFILE_STREAM_LOCK_ID)
        .execute(conn)
        .await?;
    Ok(())
}

async fn lock_profile_stream_shared(conn: &mut AsyncPgConnection) -> Result<(), DieselError> {
    diesel::sql_query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind::<diesel::sql_types::BigInt, _>(PROFILE_STREAM_LOCK_ID)
        .execute(conn)
        .await?;
    Ok(())
}

/// Read the global stream position after every earlier profile mutation and write to this
/// device inbox commits.
///
/// Both streams allocate from `occur_sn_seq`. Holding both advisory locks in one
/// transaction prevents the sequence read from observing a position allocated by an
/// uncommitted writer from the other stream.
pub async fn curr_sn_after_profile_and_inbox_writes(
    user_id: &UserId,
    device_id: &DeviceId,
) -> DataResult<Seqnum> {
    let curr_sn = connect()
        .await?
        .transaction::<_, DieselError, _>(async |conn| {
            // Readers may run together, but an exclusive writer cannot publish a stream
            // position until its profile row and change row have both committed.
            lock_profile_stream_shared(conn).await?;
            super::device::lock_inbox_stream(conn, user_id, device_id).await?;
            diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT last_value FROM occur_sn_seq")
                .get_result::<Seqnum>(conn)
                .await
        })
        .await?;
    Ok(curr_sn)
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_profiles)]
pub struct DbProfile {
    pub id: i64,
    pub user_id: OwnedUserId,
    // pub server_name: Option<OwnedServerName>,
    pub room_id: Option<OwnedRoomId>,
    pub display_name: Option<String>,
    pub avatar_url: Option<OwnedMxcUri>,
    pub blurhash: Option<String>,
    pub fields: JsonValue,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_profiles)]
pub struct NewDbProfile {
    pub user_id: OwnedUserId,
    // pub server_name: Option<OwnedServerName>,
    pub room_id: Option<OwnedRoomId>,
    pub display_name: Option<String>,
    pub avatar_url: Option<OwnedMxcUri>,
    pub blurhash: Option<String>,
}

/// Insert a profile row.
pub async fn create_profile(profile: &NewDbProfile) -> DataResult<()> {
    diesel::insert_into(user_profiles::table)
        .values(profile)
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

pub async fn get_profile(
    user_id: &UserId,
    room_id: Option<&RoomId>,
) -> DataResult<Option<DbProfile>> {
    let profile = if let Some(room_id) = room_id {
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.eq(room_id))
            .first::<DbProfile>(&mut connect().await?)
            .await
            .optional()?
    } else {
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.is_null())
            .first::<DbProfile>(&mut connect().await?)
            .await
            .optional()?
    };
    Ok(profile)
}

pub async fn profile_fields(user_id: &UserId) -> DataResult<JsonObject> {
    let fields = user_profiles::table
        .filter(user_profiles::user_id.eq(user_id))
        .filter(user_profiles::room_id.is_null())
        .select(user_profiles::fields)
        .first::<JsonValue>(&mut connect().await?)
        .await
        .optional()?;

    Ok(fields
        .as_ref()
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default())
}

pub async fn profile_field(user_id: &UserId, field: &str) -> DataResult<Option<JsonValue>> {
    Ok(profile_fields(user_id).await?.remove(field))
}

fn ensure_profile_updated(updated: usize) -> DataResult<()> {
    if updated == 0 {
        return Err(MatrixError::not_found("Profile not found.").into());
    }

    Ok(())
}

pub async fn set_profile_field(user_id: &UserId, field: &str, value: JsonValue) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_profile_stream(conn).await?;
            let updated = diesel::sql_query(
                "UPDATE user_profiles \
                 SET fields = fields || jsonb_build_object($2, $3::jsonb) \
                 WHERE user_id = $1 AND room_id IS NULL",
            )
            .bind::<Text, _>(user_id.as_str())
            .bind::<Text, _>(field)
            .bind::<Jsonb, _>(value.clone())
            .execute(conn)
            .await?;
            ensure_profile_updated(updated)?;
            record_profile_change_on_conn(conn, user_id, field, Some(value.clone())).await
        })
        .await
}

pub async fn delete_profile_field(user_id: &UserId, field: &str) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_profile_stream(conn).await?;
            let updated = diesel::sql_query(
                "UPDATE user_profiles \
                 SET fields = fields - $2 \
                 WHERE user_id = $1 AND room_id IS NULL",
            )
            .bind::<Text, _>(user_id.as_str())
            .bind::<Text, _>(field)
            .execute(conn)
            .await?;
            ensure_profile_updated(updated)?;
            record_profile_change_on_conn(conn, user_id, field, None).await
        })
        .await
}

/// A single profile field change, as recorded on the profile change stream.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileChange {
    pub user_id: OwnedUserId,
    pub field: String,
    /// The field's new value, or `None` when the field was cleared.
    ///
    /// A stored JSON `null` is `Some(JsonValue::Null)`: the spec lets servers keep `null`
    /// as a value, so it must stay distinguishable from a removal.
    pub value: Option<JsonValue>,
}

/// Appends a profile field change to the stream read by sliding sync ([MSC4262]).
///
/// Called from every profile write so that a client can be told what changed since its
/// last sync position without the server having to diff whole profiles.
///
/// [MSC4262]: https://github.com/matrix-org/matrix-spec-proposals/pull/4262
/// The stream position is left to the column's sequence default. All callers hold the
/// profile-stream advisory lock until the row commits, and sliding sync takes the same lock
/// before publishing its upper bound, so a cursor cannot step past an uncommitted change.
pub async fn record_profile_change(
    user_id: &UserId,
    field: &str,
    value: Option<JsonValue>,
) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_profile_stream(conn).await?;
            record_profile_change_on_conn(conn, user_id, field, value.clone()).await
        })
        .await
}

async fn record_profile_change_on_conn(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    field: &str,
    value: Option<JsonValue>,
) -> DataResult<()> {
    diesel::insert_into(user_profile_changes::table)
        .values((
            user_profile_changes::user_id.eq(user_id.as_str()),
            user_profile_changes::field.eq(field),
            user_profile_changes::value.eq(&value),
            user_profile_changes::removed.eq(value.is_none()),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn set_global_display_name(
    user_id: &UserId,
    display_name: Option<&str>,
) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_profile_stream(conn).await?;
            let updated = diesel::update(
                user_profiles::table
                    .filter(user_profiles::user_id.eq(user_id.as_str()))
                    .filter(user_profiles::room_id.is_null()),
            )
            .set(user_profiles::display_name.eq(display_name))
            .execute(conn)
            .await?;
            ensure_profile_updated(updated)?;
            record_profile_change_on_conn(
                conn,
                user_id,
                "displayname",
                display_name.map(Into::into),
            )
            .await
        })
        .await
}

pub async fn set_global_avatar_url(
    user_id: &UserId,
    avatar_url: Option<&MxcUri>,
) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_profile_stream(conn).await?;
            let updated = diesel::update(
                user_profiles::table
                    .filter(user_profiles::user_id.eq(user_id.as_str()))
                    .filter(user_profiles::room_id.is_null()),
            )
            .set(user_profiles::avatar_url.eq(avatar_url.map(MxcUri::as_str)))
            .execute(conn)
            .await?;
            ensure_profile_updated(updated)?;
            record_profile_change_on_conn(
                conn,
                user_id,
                "avatar_url",
                avatar_url.map(|url| url.as_str().into()),
            )
            .await
        })
        .await
}

pub async fn set_global_avatar_and_blurhash(
    user_id: &UserId,
    avatar_url: Option<&MxcUri>,
    blurhash: Option<&str>,
) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_profile_stream(conn).await?;
            let updated = diesel::update(
                user_profiles::table
                    .filter(user_profiles::user_id.eq(user_id.as_str()))
                    .filter(user_profiles::room_id.is_null()),
            )
            .set((
                user_profiles::avatar_url.eq(avatar_url.map(MxcUri::as_str)),
                user_profiles::blurhash.eq(blurhash),
            ))
            .execute(conn)
            .await?;
            ensure_profile_updated(updated)?;
            record_profile_change_on_conn(
                conn,
                user_id,
                "avatar_url",
                avatar_url.map(|url| url.as_str().into()),
            )
            .await?;
            record_profile_change_on_conn(
                conn,
                user_id,
                "xyz.amorgan.blurhash",
                blurhash.map(Into::into),
            )
            .await
        })
        .await
}

pub async fn delete_global_profile(user_id: &UserId) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_profile_stream(conn).await?;
            let profile = user_profiles::table
                .filter(user_profiles::user_id.eq(user_id.as_str()))
                .filter(user_profiles::room_id.is_null())
                .first::<DbProfile>(conn)
                .await
                .optional()?;
            let Some(profile) = profile else {
                return Ok(());
            };

            diesel::delete(user_profiles::table.find(profile.id))
                .execute(conn)
                .await?;
            if profile.display_name.is_some() {
                record_profile_change_on_conn(conn, user_id, "displayname", None).await?;
            }
            if profile.avatar_url.is_some() {
                record_profile_change_on_conn(conn, user_id, "avatar_url", None).await?;
            }
            if profile.blurhash.is_some() {
                record_profile_change_on_conn(conn, user_id, "xyz.amorgan.blurhash", None).await?;
            }
            if let Some(fields) = profile.fields.as_object() {
                for field in fields.keys() {
                    record_profile_change_on_conn(conn, user_id, field, None).await?;
                }
            }
            Ok(())
        })
        .await
}

/// Profile changes in `[since_sn, until_sn)`, oldest first.
///
/// The bounds are half-open to match sync tokens: `since_sn` is the first position the
/// client has not seen, and the token it gets back is the first position it will not see
/// this time. An exclusive lower bound here would skip a change written at exactly the
/// position the client was handed last time.
///
/// Restricted to `users` when given, which is how sliding sync limits updates to the people
/// the syncing user shares a room with. An empty `users` slice yields nothing rather than
/// everything, since "no shared users" is a real answer.
pub async fn profile_changes_since(
    users: Option<&[OwnedUserId]>,
    since_sn: Seqnum,
    until_sn: Seqnum,
) -> DataResult<Vec<ProfileChange>> {
    if users.is_some_and(<[OwnedUserId]>::is_empty) {
        return Ok(Vec::new());
    }

    let mut query = user_profile_changes::table
        .filter(user_profile_changes::occur_sn.ge(since_sn))
        .filter(user_profile_changes::occur_sn.lt(until_sn))
        .into_boxed();
    if let Some(users) = users {
        query = query.filter(user_profile_changes::user_id.eq_any(users));
    }

    let rows = query
        .order(user_profile_changes::occur_sn.asc())
        .select((
            user_profile_changes::user_id,
            user_profile_changes::field,
            user_profile_changes::value,
            user_profile_changes::removed,
        ))
        .load::<(OwnedUserId, String, Option<JsonValue>, bool)>(&mut connect().await?)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(user_id, field, value, removed)| ProfileChange {
            user_id,
            field,
            value: if removed {
                None
            } else {
                Some(value.unwrap_or(JsonValue::Null))
            },
        })
        .collect())
}

/// Mirrors a remote user's membership-carried profile fields into its row and change stream.
///
/// Remote users get a profile row so that the one place sliding sync reads profiles from
/// answers for them too; without it every remote member would be invisible to the
/// extension.
pub async fn set_remote_profile_fields(
    user_id: &UserId,
    display_name: Option<Option<&str>>,
    avatar_url: Option<Option<&MxcUri>>,
    blurhash: Option<Option<&str>>,
) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_profile_stream(conn).await?;
            upsert_remote_profile(conn, user_id).await?;
            if let Some(display_name) = display_name {
                diesel::update(
                    user_profiles::table
                        .filter(user_profiles::user_id.eq(user_id.as_str()))
                        .filter(user_profiles::room_id.is_null()),
                )
                .set(user_profiles::display_name.eq(display_name))
                .execute(conn)
                .await?;
                record_profile_change_on_conn(
                    conn,
                    user_id,
                    "displayname",
                    display_name.map(Into::into),
                )
                .await?;
            }
            if let Some(avatar_url) = avatar_url {
                diesel::update(
                    user_profiles::table
                        .filter(user_profiles::user_id.eq(user_id.as_str()))
                        .filter(user_profiles::room_id.is_null()),
                )
                .set(user_profiles::avatar_url.eq(avatar_url.map(MxcUri::as_str)))
                .execute(conn)
                .await?;
                record_profile_change_on_conn(
                    conn,
                    user_id,
                    "avatar_url",
                    avatar_url.map(|url| url.as_str().into()),
                )
                .await?;
            }
            if let Some(blurhash) = blurhash {
                diesel::update(
                    user_profiles::table
                        .filter(user_profiles::user_id.eq(user_id.as_str()))
                        .filter(user_profiles::room_id.is_null()),
                )
                .set(user_profiles::blurhash.eq(blurhash))
                .execute(conn)
                .await?;
                record_profile_change_on_conn(
                    conn,
                    user_id,
                    "xyz.amorgan.blurhash",
                    blurhash.map(Into::into),
                )
                .await?;
            }
            Ok(())
        })
        .await
}

/// Ensures a remote user has a global profile row to write into.
///
/// The conflict target is the partial index over global rows, not the table's
/// `UNIQUE (user_id, room_id)`: that constraint cannot deduplicate these, because
/// PostgreSQL treats the NULL `room_id` as distinct from itself and every insert would
/// simply add another row.
async fn upsert_remote_profile(conn: &mut AsyncPgConnection, user_id: &UserId) -> DataResult<()> {
    diesel::sql_query(
        "INSERT INTO user_profiles (user_id, room_id) VALUES ($1, NULL)          ON CONFLICT (user_id) WHERE room_id IS NULL DO NOTHING",
    )
    .bind::<Text, _>(user_id.as_str())
    .execute(conn)
    .await?;
    Ok(())
}
