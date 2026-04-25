use diesel::prelude::*;
use diesel::sql_types::{Jsonb, Text};

use crate::core::identifiers::*;
use crate::core::serde::{JsonObject, JsonValue};
use crate::core::{MatrixError, OwnedMxcUri};
use crate::schema::*;
use crate::{DataResult, connect};

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

pub fn get_profile(user_id: &UserId, room_id: Option<&RoomId>) -> DataResult<Option<DbProfile>> {
    let profile = if let Some(room_id) = room_id {
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.eq(room_id))
            .first::<DbProfile>(&mut connect()?)
            .optional()?
    } else {
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.is_null())
            .first::<DbProfile>(&mut connect()?)
            .optional()?
    };
    Ok(profile)
}

pub fn profile_fields(user_id: &UserId) -> DataResult<JsonObject> {
    let fields = user_profiles::table
        .filter(user_profiles::user_id.eq(user_id))
        .filter(user_profiles::room_id.is_null())
        .select(user_profiles::fields)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;

    Ok(fields
        .as_ref()
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default())
}

pub fn profile_field(user_id: &UserId, field: &str) -> DataResult<Option<JsonValue>> {
    Ok(profile_fields(user_id)?.remove(field))
}

fn ensure_profile_updated(updated: usize) -> DataResult<()> {
    if updated == 0 {
        return Err(MatrixError::not_found("Profile not found.").into());
    }

    Ok(())
}

pub fn set_profile_field(user_id: &UserId, field: &str, value: JsonValue) -> DataResult<()> {
    let updated = diesel::sql_query(
        "UPDATE user_profiles \
         SET fields = fields || jsonb_build_object($2, $3::jsonb) \
         WHERE user_id = $1 AND room_id IS NULL",
    )
    .bind::<Text, _>(user_id.as_str())
    .bind::<Text, _>(field)
    .bind::<Jsonb, _>(value)
    .execute(&mut connect()?)?;

    ensure_profile_updated(updated)
}

pub fn delete_profile_field(user_id: &UserId, field: &str) -> DataResult<()> {
    let updated = diesel::sql_query(
        "UPDATE user_profiles \
         SET fields = fields - $2 \
         WHERE user_id = $1 AND room_id IS NULL",
    )
    .bind::<Text, _>(user_id.as_str())
    .bind::<Text, _>(field)
    .execute(&mut connect()?)?;

    ensure_profile_updated(updated)
}
