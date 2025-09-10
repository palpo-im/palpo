pub mod device;
pub use device::{DbUserDevice, NewDbUserDevice};
mod password;
pub use password::*;
mod profile;
pub use profile::*;
mod filter;
pub use filter::*;
mod access_token;
pub use access_token::*;
mod refresh_token;
pub use refresh_token::*;
mod data;
pub use data::*;
pub mod key;
pub mod pusher;
// pub mod push_rule;
pub use key::*;
pub mod key_backup;
pub use key_backup::*;
pub mod session;
pub use session::*;
pub mod presence;
use std::mem;

use diesel::dsl::count_distinct;
use diesel::prelude::*;
pub use presence::*;

use crate::core::events::AnyStrippedStateEvent;
use crate::core::identifiers::*;
use crate::core::serde::{JsonValue, RawJson};
use crate::core::{OwnedMxcUri, UnixMillis};
use crate::schema::*;
use crate::{DataError, DataResult, connect, diesel_exists};

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = users)]
pub struct DbUser {
    pub id: OwnedUserId,
    pub ty: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub appservice_id: Option<String>,
    pub shadow_banned: bool,
    pub consent_at: Option<UnixMillis>,
    pub consent_version: Option<String>,
    pub consent_server_notice_sent: Option<String>,
    pub approved_at: Option<UnixMillis>,
    pub approved_by: Option<OwnedUserId>,
    pub deactivated_at: Option<UnixMillis>,
    pub deactivated_by: Option<OwnedUserId>,
    pub locked_at: Option<UnixMillis>,
    pub locked_by: Option<OwnedUserId>,
    pub created_at: UnixMillis,
}

#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = users)]
pub struct NewDbUser {
    pub id: OwnedUserId,
    pub ty: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub appservice_id: Option<String>,
    pub created_at: UnixMillis,
}

impl DbUser {
    pub fn is_deactivated(&self) -> bool {
        self.deactivated_at.is_some()
    }
}

#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = user_ignores)]
pub struct NewDbUserIgnore {
    pub user_id: OwnedUserId,
    pub ignored_id: OwnedUserId,
    pub created_at: UnixMillis,
}

pub fn is_admin(user_id: &UserId) -> DataResult<bool> {
    users::table
        .filter(users::id.eq(user_id))
        .select(users::is_admin)
        .first::<bool>(&mut connect()?)
        .map_err(Into::into)
}

/// Returns an iterator over all rooms this user joined.
pub fn joined_rooms(user_id: &UserId) -> DataResult<Vec<OwnedRoomId>> {
    let room_memeberships = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .distinct_on(room_users::room_id)
        .select((room_users::room_id, room_users::membership))
        .order_by((room_users::room_id.desc(), room_users::id.desc()))
        .load::<(OwnedRoomId, String)>(&mut connect()?)?;
    Ok(room_memeberships
        .into_iter()
        .filter_map(|(room_id, membership)| {
            if membership == "join" {
                Some(room_id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>())
}
/// Returns an iterator over all rooms a user was invited to.
pub fn invited_rooms(
    user_id: &UserId,
    since_sn: i64,
) -> DataResult<Vec<(OwnedRoomId, Vec<RawJson<AnyStrippedStateEvent>>)>> {
    let ingored_ids = user_ignores::table
        .filter(user_ignores::user_id.eq(user_id))
        .select(user_ignores::ignored_id)
        .load::<OwnedUserId>(&mut connect()?)?;
    let list = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::membership.eq("invite"))
        .filter(room_users::event_sn.ge(since_sn))
        .filter(room_users::sender_id.ne_all(&ingored_ids))
        .select((room_users::room_id, room_users::state_data))
        .load::<(OwnedRoomId, Option<JsonValue>)>(&mut connect()?)?
        .into_iter()
        .filter_map(|(room_id, state_data)| {
            state_data
                .and_then(|state_data| serde_json::from_value(state_data).ok())
                .map(|state_data| (room_id, state_data))
        })
        .collect();
    Ok(list)
}

pub fn knocked_rooms(
    user_id: &UserId,
    since_sn: i64,
) -> DataResult<Vec<(OwnedRoomId, Vec<RawJson<AnyStrippedStateEvent>>)>> {
    let list = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::membership.eq("knock"))
        .filter(room_users::event_sn.ge(since_sn))
        .select((room_users::room_id, room_users::state_data))
        .load::<(OwnedRoomId, Option<JsonValue>)>(&mut connect()?)?
        .into_iter()
        .filter_map(|(room_id, state_data)| {
            state_data
                .and_then(|state_data| serde_json::from_value(state_data).ok())
                .map(|state_data| (room_id, state_data))
        })
        .collect();
    Ok(list)
}

/// Check if a user has an account on this homeserver.
pub fn user_exists(user_id: &UserId) -> DataResult<bool> {
    let query = users::table.find(user_id);
    diesel_exists!(query, &mut connect()?).map_err(Into::into)
}

pub fn get_user(user_id: &UserId) -> DataResult<DbUser> {
    users::table
        .find(user_id)
        .first::<DbUser>(&mut connect()?)
        .map_err(Into::into)
}

/// Returns the number of users registered on this server.
pub fn count() -> DataResult<u64> {
    let count = user_passwords::table
        .select(count_distinct(user_passwords::user_id))
        .first::<i64>(&mut connect()?)?;
    Ok(count as u64)
}

/// Returns a list of local users as list of usernames.
///
/// A user account is considered `local` if the length of it's password is greater then zero.
pub fn list_local_users() -> DataResult<Vec<OwnedUserId>> {
    user_passwords::table
        .select(user_passwords::user_id)
        .load::<OwnedUserId>(&mut connect()?)
        .map_err(Into::into)
}

/// Returns the display_name of a user on this homeserver.
pub fn display_name(user_id: &UserId) -> DataResult<Option<String>> {
    user_profiles::table
        .filter(user_profiles::user_id.eq(user_id.as_str()))
        .filter(user_profiles::room_id.is_null())
        .select(user_profiles::display_name)
        .first::<Option<String>>(&mut connect()?)
        .optional()
        .map(Option::flatten)
        .map_err(Into::into)
}
pub fn set_display_name(user_id: &UserId, display_name: &str) -> DataResult<()> {
    diesel::update(
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.is_null()),
    )
    .set(user_profiles::display_name.eq(display_name))
    .execute(&mut connect()?)
    .map(|_| ())
    .map_err(Into::into)
}
pub fn remove_display_name(user_id: &UserId) -> DataResult<()> {
    diesel::update(
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.is_null()),
    )
    .set(user_profiles::display_name.eq::<Option<String>>(None))
    .execute(&mut connect()?)
    .map(|_| ())
    .map_err(Into::into)
}

/// Get the avatar_url of a user.
pub fn avatar_url(user_id: &UserId) -> DataResult<Option<OwnedMxcUri>> {
    user_profiles::table
        .filter(user_profiles::user_id.eq(user_id.as_str()))
        .filter(user_profiles::room_id.is_null())
        .select(user_profiles::avatar_url)
        .first::<Option<OwnedMxcUri>>(&mut connect()?)
        .optional()
        .map(Option::flatten)
        .map_err(Into::into)
}
pub fn set_avatar_url(user_id: &UserId, avatar_url: &MxcUri) -> DataResult<()> {
    diesel::update(
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.is_null()),
    )
    .set(user_profiles::avatar_url.eq(avatar_url.as_str()))
    .execute(&mut connect()?)?;
    Ok(())
}

pub fn delete_profile(user_id: &UserId) -> DataResult<()> {
    diesel::delete(
        user_profiles::table
            .filter(user_profiles::user_id.eq(user_id.as_str()))
            .filter(user_profiles::room_id.is_null()),
    )
    .execute(&mut connect()?)?;
    Ok(())
}

/// Get the blurhash of a user.
pub fn blurhash(user_id: &UserId) -> DataResult<Option<String>> {
    user_profiles::table
        .filter(user_profiles::user_id.eq(user_id.as_str()))
        .filter(user_profiles::room_id.is_null())
        .select(user_profiles::blurhash)
        .first::<Option<String>>(&mut connect()?)
        .optional()
        .map(Option::flatten)
        .map_err(Into::into)
}

pub fn is_deactivated(user_id: &UserId) -> DataResult<bool> {
    let deactivated_at = users::table
        .filter(users::id.eq(user_id))
        .select(users::deactivated_at)
        .first::<Option<UnixMillis>>(&mut connect()?)
        .optional()?
        .flatten();
    Ok(deactivated_at.is_some())
}

pub fn all_device_ids(user_id: &UserId) -> DataResult<Vec<OwnedDeviceId>> {
    user_devices::table
        .filter(user_devices::user_id.eq(user_id))
        .select(user_devices::device_id)
        .load::<OwnedDeviceId>(&mut connect()?)
        .map_err(Into::into)
}

pub fn delete_access_tokens(user_id: &UserId) -> DataResult<()> {
    diesel::delete(user_access_tokens::table.filter(user_access_tokens::user_id.eq(user_id)))
        .execute(&mut connect()?)?;
    Ok(())
}

pub fn delete_refresh_tokens(user_id: &UserId) -> DataResult<()> {
    diesel::delete(user_refresh_tokens::table.filter(user_refresh_tokens::user_id.eq(user_id)))
        .execute(&mut connect()?)?;
    Ok(())
}

pub fn remove_all_devices(user_id: &UserId) -> DataResult<()> {
    delete_access_tokens(user_id)?;
    delete_refresh_tokens(user_id)?;
    pusher::delete_user_pushers(user_id)
}
pub fn delete_dehydrated_devices(user_id: &UserId) -> DataResult<()> {
    diesel::delete(
        user_dehydrated_devices::table.filter(user_dehydrated_devices::user_id.eq(user_id)),
    )
    .execute(&mut connect()?)?;
    Ok(())
}

/// Ensure that a user only sees signatures from themselves and the target user
pub fn clean_signatures<F: Fn(&UserId) -> bool>(
    cross_signing_key: &mut serde_json::Value,
    sender_id: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: F,
) -> DataResult<()> {
    if let Some(signatures) = cross_signing_key
        .get_mut("signatures")
        .and_then(|v| v.as_object_mut())
    {
        // Don't allocate for the full size of the current signatures, but require
        // at most one resize if nothing is dropped
        let new_capacity = signatures.len() / 2;
        for (user, signature) in
            mem::replace(signatures, serde_json::Map::with_capacity(new_capacity))
        {
            let sid = <&UserId>::try_from(user.as_str())
                .map_err(|_| DataError::internal("Invalid user ID in database."))?;
            if sender_id == Some(user_id) || sid == user_id || allowed_signatures(sid) {
                signatures.insert(user, signature);
            }
        }
    }

    Ok(())
}

pub fn deactivate(user_id: &UserId) -> DataResult<()> {
    diesel::update(users::table.find(user_id))
        .set((users::deactivated_at.eq(UnixMillis::now()),))
        .execute(&mut connect()?)?;

    diesel::delete(user_threepids::table.filter(user_threepids::user_id.eq(user_id)))
        .execute(&mut connect()?)?;
    diesel::delete(user_access_tokens::table.filter(user_access_tokens::user_id.eq(user_id)))
        .execute(&mut connect()?)?;

    Ok(())
}

pub fn set_ignored_users(user_id: &UserId, ignored_ids: &[OwnedUserId]) -> DataResult<()> {
    diesel::delete(user_ignores::table.filter(user_ignores::user_id.eq(user_id)))
        .execute(&mut connect()?)?;
    for ignored_id in ignored_ids {
        diesel::insert_into(user_ignores::table)
            .values(NewDbUserIgnore {
                user_id: user_id.to_owned(),
                ignored_id: ignored_id.to_owned(),
                created_at: UnixMillis::now(),
            })
            .on_conflict_do_nothing()
            .execute(&mut connect()?)?;
    }
    Ok(())
}
