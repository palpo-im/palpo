use diesel::prelude::*;
use diesel::sql_types::{Jsonb, Text};
use diesel_async::RunQueryDsl;

use crate::core::identifiers::*;
use crate::core::serde::{JsonObject, JsonValue};
use crate::core::{MatrixError, OwnedMxcUri, Seqnum};
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
    let updated = diesel::sql_query(
        "UPDATE user_profiles \
         SET fields = fields || jsonb_build_object($2, $3::jsonb) \
         WHERE user_id = $1 AND room_id IS NULL",
    )
    .bind::<Text, _>(user_id.as_str())
    .bind::<Text, _>(field)
    .bind::<Jsonb, _>(value.clone())
    .execute(&mut connect().await?)
    .await?;

    ensure_profile_updated(updated)?;
    record_profile_change(user_id, field, Some(value)).await
}

pub async fn delete_profile_field(user_id: &UserId, field: &str) -> DataResult<()> {
    let updated = diesel::sql_query(
        "UPDATE user_profiles \
         SET fields = fields - $2 \
         WHERE user_id = $1 AND room_id IS NULL",
    )
    .bind::<Text, _>(user_id.as_str())
    .bind::<Text, _>(field)
    .execute(&mut connect().await?)
    .await?;

    ensure_profile_updated(updated)?;
    record_profile_change(user_id, field, None).await
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
pub async fn record_profile_change(
    user_id: &UserId,
    field: &str,
    value: Option<JsonValue>,
) -> DataResult<()> {
    let occur_sn = crate::next_sn().await?;
    diesel::insert_into(user_profile_changes::table)
        .values((
            user_profile_changes::occur_sn.eq(occur_sn),
            user_profile_changes::user_id.eq(user_id.as_str()),
            user_profile_changes::field.eq(field),
            user_profile_changes::value.eq(&value),
            user_profile_changes::removed.eq(value.is_none()),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Profile changes in `(since_sn, until_sn]`, oldest first.
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
        .filter(user_profile_changes::occur_sn.gt(since_sn))
        .filter(user_profile_changes::occur_sn.le(until_sn))
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
