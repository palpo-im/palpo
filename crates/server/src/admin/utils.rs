#![allow(dead_code)]

use crate::core::{OwnedUserId, UserId};
use crate::{AppError, AppResult, IsRemoteOrLocal, config, data};

pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Parses user ID
pub(crate) fn parse_user_id(user_id: &str) -> AppResult<OwnedUserId> {
    UserId::parse_with_server_name(user_id.to_lowercase(), config::server_name()).map_err(|e| {
        AppError::public(format!(
            "the supplied username is not a valid username: {e}"
        ))
    })
}

/// Parses user ID as our local user
pub(crate) fn parse_local_user_id(user_id: &str) -> AppResult<OwnedUserId> {
    let user_id = parse_user_id(user_id)?;

    if !user_id.is_local() {
        return Err(AppError::public(
            "user {user_id:?} does not belong to our server.",
        ));
    }

    Ok(user_id)
}

/// Parses user ID that is an active (not guest or deactivated) local user
pub(crate) async fn parse_active_local_user_id(user_id: &str) -> AppResult<OwnedUserId> {
    let user_id = parse_local_user_id(user_id)?;

    if !data::user::user_exists(&user_id).await? {
        return Err(AppError::public(
            "user {user_id:?} does not exist on this server.",
        ));
    }

    if data::user::is_deactivated(&user_id).await? {
        return Err(AppError::public("user {user_id:?} is deactivated."));
    }

    Ok(user_id)
}
