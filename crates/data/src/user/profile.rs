use diesel::prelude::*;
use diesel::sql_types::{Jsonb, Text};
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::core::identifiers::*;
use crate::core::serde::{JsonObject, JsonValue};
use crate::core::{MatrixError, OwnedMxcUri};
use crate::schema::*;
use crate::{DataError, DataResult, connect};

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

/// Restore the global profile invariant for legacy users created without a
/// `room_id IS NULL` profile row.
///
/// The user row lock serializes concurrent repairs. This is necessary because
/// PostgreSQL does not consider two NULL values equal for the existing
/// `(user_id, room_id)` unique constraint.
pub async fn ensure_profile_exists(user_id: &UserId) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, DataError, _>(async move |conn| {
            users::table
                .filter(users::id.eq(user_id))
                .select(users::id)
                .for_update()
                .first::<OwnedUserId>(conn)
                .await?;

            let profile_exists = user_profiles::table
                .filter(user_profiles::user_id.eq(user_id))
                .filter(user_profiles::room_id.is_null())
                .select(user_profiles::id)
                .first::<i64>(conn)
                .await
                .optional()?
                .is_some();
            if !profile_exists {
                diesel::insert_into(user_profiles::table)
                    .values(&NewDbProfile {
                        user_id: user_id.to_owned(),
                        room_id: None,
                        display_name: Some(user_id.localpart().to_owned()),
                        avatar_url: None,
                        blurhash: None,
                    })
                    .execute(conn)
                    .await?;
            }

            Ok(())
        })
        .await
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

pub(crate) fn ensure_profile_updated(updated: usize) -> DataResult<()> {
    if updated == 0 {
        return Err(MatrixError::not_found("Profile not found.").into());
    }

    Ok(())
}

pub async fn set_profile_field(user_id: &UserId, field: &str, value: JsonValue) -> DataResult<()> {
    async fn update(user_id: &UserId, field: &str, value: &JsonValue) -> DataResult<usize> {
        Ok(diesel::sql_query(
            "UPDATE user_profiles \
         SET fields = fields || jsonb_build_object($2, $3::jsonb) \
         WHERE user_id = $1 AND room_id IS NULL",
        )
        .bind::<Text, _>(user_id.as_str())
        .bind::<Text, _>(field)
        .bind::<Jsonb, _>(value)
        .execute(&mut connect().await?)
        .await?)
    }

    let mut updated = update(user_id, field, &value).await?;
    if updated == 0 {
        ensure_profile_exists(user_id).await?;
        updated = update(user_id, field, &value).await?;
    }

    ensure_profile_updated(updated)
}

pub async fn delete_profile_field(user_id: &UserId, field: &str) -> DataResult<()> {
    async fn update(user_id: &UserId, field: &str) -> DataResult<usize> {
        Ok(diesel::sql_query(
            "UPDATE user_profiles \
         SET fields = fields - $2 \
         WHERE user_id = $1 AND room_id IS NULL",
        )
        .bind::<Text, _>(user_id.as_str())
        .bind::<Text, _>(field)
        .execute(&mut connect().await?)
        .await?)
    }

    let mut updated = update(user_id, field).await?;
    if updated == 0 {
        ensure_profile_exists(user_id).await?;
        updated = update(user_id, field).await?;
    }

    ensure_profile_updated(updated)
}
