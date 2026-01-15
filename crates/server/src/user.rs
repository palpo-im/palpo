mod password;
use palpo_data::user::set_display_name;
pub use password::*;
pub mod key;
pub mod pusher;
pub use key::*;
pub mod presence;
// mod ldap;
// pub use ldap::*;
pub mod session;
use std::mem;

use diesel::prelude::*;
pub use presence::*;
use serde::de::DeserializeOwned;

use crate::core::UnixMillis;
use crate::core::events::GlobalAccountDataEventType;
use crate::core::events::ignored_user_list::IgnoredUserListEvent;
use crate::core::events::room::power_levels::RoomPowerLevelsEventContent;
use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::data::schema::*;
use crate::data::user::{DbUser, DbUserData, NewDbPassword, NewDbUser};
use crate::data::{DataResult, connect};
use crate::room::timeline;
use crate::{AppError, AppResult, IsRemoteOrLocal, MatrixError, PduBuilder, config, data, room};

pub fn is_username_available(username: &str) -> AppResult<bool> {
    let user_id = UserId::parse(format!("@{}:{}", username, config::server_name()))
        .map_err(|_| AppError::internal("invalid username format"))?;
    user_id.validate_strict()?;
    let available = !data::user::user_exists(&user_id)?;
    Ok(available)
}

pub fn create_user(user_id: impl Into<OwnedUserId>, password: Option<&str>) -> AppResult<DbUser> {
    let user_id = user_id.into();
    let new_user = NewDbUser {
        id: user_id.clone(),
        ty: None,
        is_admin: false,
        is_guest: password.is_none(),
        is_local: user_id.is_local(),
        localpart: user_id.localpart().to_owned(),
        server_name: user_id.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    };
    let user = diesel::insert_into(users::table)
        .values(&new_user)
        .on_conflict(users::id)
        .do_update()
        .set(&new_user)
        .get_result::<DbUser>(&mut connect()?)?;
    let display_name = user_id.localpart().to_owned();
    if let Some(password) = password {
        crate::user::set_password(&user.id, password)?;
    }
    if let Err(e) = set_display_name(&user.id, &display_name) {
        tracing::warn!("failed to set profile for new user (non-fatal): {}", e);
    }
    Ok(user)
}

pub fn list_local_users() -> AppResult<Vec<OwnedUserId>> {
    let users = user_passwords::table
        .select(user_passwords::user_id)
        .load::<OwnedUserId>(&mut connect()?)?;
    Ok(users)
}
/// Ensure that a user only sees signatures from themselves and the target user
pub fn clean_signatures<F: Fn(&UserId) -> bool>(
    cross_signing_key: &mut serde_json::Value,
    sender_id: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: F,
) -> AppResult<()> {
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
                .map_err(|_| AppError::internal("Invalid user ID in database."))?;
            if sender_id == Some(user_id) || sid == user_id || allowed_signatures(sid) {
                signatures.insert(user, signature);
            }
        }
    }

    Ok(())
}

/// Returns true/false based on whether the recipient/receiving user has
/// blocked the sender
pub fn user_is_ignored(sender_id: &UserId, recipient_id: &UserId) -> bool {
    if let Ok(Some(ignored)) = data::user::get_global_data::<IgnoredUserListEvent>(
        recipient_id,
        &GlobalAccountDataEventType::IgnoredUserList.to_string(),
    ) {
        ignored
            .content
            .ignored_users
            .keys()
            .any(|blocked_user| blocked_user == sender_id)
    } else {
        false
    }
}

/// Runs through all the deactivation steps:
///
/// - Mark as deactivated
/// - Removing display name
/// - Removing avatar URL and blurhash
/// - Removing all profile data
/// - Leaving all rooms (and forgets all of them)
pub async fn full_user_deactivate(
    user_id: &UserId,
    all_joined_rooms: &[OwnedRoomId],
) -> AppResult<()> {
    data::user::deactivate(user_id).ok();
    data::user::delete_profile(user_id).ok();

    // TODO: remove all user data
    // for profile_key in data::user::all_profile_keys(user_id) {
    //     data::user::set_profile_key(user_id, &profile_key, None);
    // }

    for room_id in all_joined_rooms {
        let state_lock = room::lock_state(room_id).await;

        let room_version = room::get_version(room_id)?;
        let version_rules = room::get_version_rules(&room_version)?;
        let room_power_levels = room::get_power_levels(room_id).await.ok();

        let user_can_change_self = room_power_levels.as_ref().is_some_and(|power_levels| {
            power_levels.user_can_change_user_power_level(user_id, user_id)
        });

        let user_can_demote_self = user_can_change_self
            || room::get_create(room_id).is_ok_and(|event| event.sender == user_id);

        if user_can_demote_self {
            let mut power_levels_content: RoomPowerLevelsEventContent = room_power_levels
                .map(TryInto::try_into)
                .transpose()?
                .unwrap_or_else(|| RoomPowerLevelsEventContent::new(&version_rules.authorization));
            power_levels_content.users.remove(user_id);

            // ignore errors so deactivation doesn't fail
            match timeline::build_and_append_pdu(
                PduBuilder::state(String::new(), &power_levels_content),
                user_id,
                room_id,
                &room_version,
                &state_lock,
            )
            .await
            {
                Err(e) => {
                    warn!(%room_id, %user_id, "Failed to demote user's own power level: {e}");
                }
                _ => {
                    info!("Demoted {user_id} in {room_id} as part of account deactivation");
                }
            }
        }
    }

    if let Err(e) = crate::membership::leave_all_rooms(user_id).await {
        tracing::warn!(%user_id, "failed to leave all rooms during deactivation: {e}");
    }

    Ok(())
}

/// Find out which user an OpenID access token belongs to.
pub async fn find_from_openid_token(token: &str) -> AppResult<OwnedUserId> {
    let Ok((user_id, expires_at)) = user_openid_tokens::table
        .filter(user_openid_tokens::token.eq(token))
        .select((user_openid_tokens::user_id, user_openid_tokens::expires_at))
        .first::<(OwnedUserId, UnixMillis)>(&mut connect()?)
    else {
        return Err(MatrixError::unauthorized("OpenID token is unrecognised").into());
    };
    if expires_at < UnixMillis::now() {
        tracing::warn!("OpenID token is expired, removing");
        diesel::delete(user_openid_tokens::table.filter(user_openid_tokens::token.eq(token)))
            .execute(&mut connect()?)?;

        return Err(MatrixError::unauthorized("OpenID token is expired").into());
    }

    Ok(user_id)
}

/// Creates a short-lived login token, which can be used to log in using the
/// `m.login.token` mechanism.
pub fn create_login_token(user_id: &UserId, token: &str) -> AppResult<u64> {
    use std::num::Saturating as Sat;

    let expires_in = crate::config::get().login_token_ttl;
    let expires_at = (Sat(UnixMillis::now().get()) + Sat(expires_in)).0 as i64;

    diesel::insert_into(user_login_tokens::table)
        .values((
            user_login_tokens::user_id.eq(user_id),
            user_login_tokens::token.eq(token),
            user_login_tokens::expires_at.eq(expires_at),
        ))
        .on_conflict(user_login_tokens::token)
        .do_update()
        .set(user_login_tokens::expires_at.eq(expires_at))
        .execute(&mut connect()?)?;

    Ok(expires_in)
}

/// Find out which user a login token belongs to.
/// Removes the token to prevent double-use attacks.
pub fn take_login_token(token: &str) -> AppResult<OwnedUserId> {
    let Ok((user_id, expires_at)) = user_login_tokens::table
        .filter(user_login_tokens::token.eq(token))
        .select((user_login_tokens::user_id, user_login_tokens::expires_at))
        .first::<(OwnedUserId, UnixMillis)>(&mut connect()?)
    else {
        return Err(MatrixError::forbidden("Login token is unrecognised.", None).into());
    };

    if expires_at < UnixMillis::now() {
        trace!(?user_id, ?token, "Removing expired login token");
        diesel::delete(user_login_tokens::table.filter(user_login_tokens::token.eq(token)))
            .execute(&mut connect()?)?;
        return Err(MatrixError::forbidden("Login token is expired.", None).into());
    }

    diesel::delete(user_login_tokens::table.filter(user_login_tokens::token.eq(token)))
        .execute(&mut connect()?)?;

    Ok(user_id)
}

pub fn valid_refresh_token(user_id: &UserId, device_id: &DeviceId, token: &str) -> AppResult<()> {
    let Ok(expires_at) = user_refresh_tokens::table
        .filter(user_refresh_tokens::user_id.eq(user_id))
        .filter(user_refresh_tokens::device_id.eq(device_id))
        .filter(user_refresh_tokens::token.eq(token))
        .select(user_refresh_tokens::expires_at)
        .first::<i64>(&mut connect()?)
    else {
        return Err(MatrixError::unauthorized("Invalid refresh token.").into());
    };
    if expires_at < UnixMillis::now().get() as i64 {
        return Err(MatrixError::unauthorized("Refresh token expired.").into());
    }
    Ok(())
}

pub fn make_user_admin(user_id: &UserId) -> AppResult<()> {
    let user_id = user_id.to_owned();
    diesel::update(users::table.filter(users::id.eq(&user_id)))
        .set(users::is_admin.eq(true))
        .execute(&mut connect()?)?;
    Ok(())
}

/// Places one event in the account data of the user and removes the previous entry.
#[tracing::instrument(skip(room_id, user_id, event_type, json_data))]
pub fn set_data(
    user_id: &UserId,
    room_id: Option<OwnedRoomId>,
    event_type: &str,
    json_data: JsonValue,
) -> DataResult<DbUserData> {
    let user_data = data::user::set_data(user_id, room_id, event_type, json_data)?;
    Ok(user_data)
}

pub fn get_data<E: DeserializeOwned>(
    user_id: &UserId,
    room_id: Option<&RoomId>,
    kind: &str,
) -> DataResult<E> {
    let data = data::user::get_data::<E>(user_id, room_id, kind)?;
    Ok(data)
}

pub fn get_global_datas(user_id: &UserId) -> DataResult<Vec<DbUserData>> {
    let datas = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(user_datas::room_id.is_null())
        .load::<DbUserData>(&mut connect()?)?;
    Ok(datas)
}

pub async fn delete_all_media(user_id: &UserId) -> AppResult<i64> {
    let medias = media_metadatas::table
        .filter(media_metadatas::created_by.eq(user_id))
        .select((media_metadatas::origin_server, media_metadatas::media_id))
        .load::<(OwnedServerName, String)>(&mut connect()?)?;

    for (origin_server, media_id) in &medias {
        if let Err(e) = crate::media::delete_media(origin_server, media_id).await {
            tracing::error!("failed to delete media file: {e}");
        }
    }
    Ok(0)
}

pub async fn deactivate_account(user_id: &UserId) -> AppResult<()> {
    diesel::update(users::table.find(user_id))
        .set(users::deactivated_at.eq(UnixMillis::now()))
        .execute(&mut connect()?)?;
    Ok(())
}
