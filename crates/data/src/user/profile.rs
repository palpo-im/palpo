use diesel::prelude::*;

use crate::core::OwnedMxcUri;
use crate::core::identifiers::*;
use crate::core::serde::{JsonObject, JsonValue};
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
        .first::<JsonValue>(&mut connect()?)?;

    Ok(fields.as_object().cloned().unwrap_or_default())
}

pub fn profile_field(user_id: &UserId, field: &str) -> DataResult<Option<JsonValue>> {
    Ok(profile_fields(user_id)?.remove(field))
}

pub fn set_profile_field(user_id: &UserId, field: &str, value: JsonValue) -> DataResult<()> {
    let mut fields = profile_fields(user_id)?;
    fields.insert(field.to_owned(), value);

    diesel::update(
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id))
            .filter(user_profiles::room_id.is_null()),
    )
    .set(user_profiles::fields.eq(JsonValue::Object(fields)))
    .execute(&mut connect()?)?;

    Ok(())
}

pub fn delete_profile_field(user_id: &UserId, field: &str) -> DataResult<()> {
    let mut fields = profile_fields(user_id)?;
    fields.remove(field);

    diesel::update(
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id))
            .filter(user_profiles::room_id.is_null()),
    )
    .set(user_profiles::fields.eq(JsonValue::Object(fields)))
    .execute(&mut connect()?)?;

    Ok(())
}
