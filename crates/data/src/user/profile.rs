use diesel::prelude::*;
use diesel::sql_types::{Jsonb, Text};
use diesel_async::RunQueryDsl;

use crate::core::identifiers::*;
use crate::core::serde::{JsonObject, JsonValue};
use crate::core::{MatrixError, MxcUri, OwnedMxcUri, Seqnum};
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
/// The stream position is left to the column's sequence default, so it is consumed by the
/// statement that writes the row. Taking it separately would publish a position before the
/// row it names existed, and a sync landing in that window would step past the change and
/// never deliver it.
pub async fn record_profile_change(
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
        .execute(&mut connect().await?)
        .await?;
    Ok(())
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

/// Mirrors a remote user's membership-carried display name into their profile row and the
/// change stream (MSC4262).
///
/// Remote users get a profile row so that the one place sliding sync reads profiles from
/// answers for them too; without it every remote member would be invisible to the
/// extension.
pub async fn set_remote_profile_display_name(
    user_id: &UserId,
    display_name: Option<&str>,
) -> DataResult<()> {
    upsert_remote_profile(user_id).await?;
    diesel::update(
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.is_null()),
    )
    .set(user_profiles::display_name.eq(display_name))
    .execute(&mut connect().await?)
    .await?;

    record_profile_change(user_id, "displayname", display_name.map(|name| name.into())).await
}

/// Mirrors a remote user's membership-carried avatar into their profile row and the change
/// stream (MSC4262).
pub async fn set_remote_profile_avatar_url(
    user_id: &UserId,
    avatar_url: Option<&MxcUri>,
) -> DataResult<()> {
    upsert_remote_profile(user_id).await?;
    diesel::update(
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.is_null()),
    )
    .set(user_profiles::avatar_url.eq(avatar_url.map(|url| url.as_str())))
    .execute(&mut connect().await?)
    .await?;

    record_profile_change(
        user_id,
        "avatar_url",
        avatar_url.map(|url| url.as_str().into()),
    )
    .await
}

/// Ensures a remote user has a global profile row to write into.
///
/// The conflict target is the partial index over global rows, not the table's
/// `UNIQUE (user_id, room_id)`: that constraint cannot deduplicate these, because
/// PostgreSQL treats the NULL `room_id` as distinct from itself and every insert would
/// simply add another row.
async fn upsert_remote_profile(user_id: &UserId) -> DataResult<()> {
    diesel::sql_query(
        "INSERT INTO user_profiles (user_id, room_id) VALUES ($1, NULL)          ON CONFLICT (user_id) WHERE room_id IS NULL DO NOTHING",
    )
    .bind::<Text, _>(user_id.as_str())
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}
