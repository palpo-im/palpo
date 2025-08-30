use clap::Subcommand;

use crate::admin::{Context, RoomInfo, get_room_info};
use crate::core::{OwnedRoomId, OwnedServerName, OwnedUserId};
use crate::macros::admin_command_dispatch;
use crate::{AppError, AppResult, config, data, sending};

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(crate) enum FederationCommand {
    /// - Disables incoming federation handling for a room.
    DisableRoom { room_id: OwnedRoomId },

    /// - Enables incoming federation handling for a room again.
    EnableRoom { room_id: OwnedRoomId },

    /// - Fetch `/.well-known/matrix/support` from the specified server
    ///
    /// Despite the name, this is not a federation endpoint and does not go
    /// through the federation / server resolution process as per-spec this is
    /// supposed to be served at the server_name.
    ///
    /// Respecting homeservers put this file here for listing administration,
    /// moderation, and security inquiries. This command provides a way to
    /// easily fetch that information.
    FetchSupportWellKnown { server_name: OwnedServerName },

    /// - Lists all the rooms we share/track with the specified *remote* user
    RemoteUserInRooms { user_id: OwnedUserId },
}

pub(super) async fn disable_room(ctx: &Context<'_>, room_id: OwnedRoomId) -> AppResult<()> {
    crate::room::disable_room(&room_id, true)?;
    ctx.write_str("Room disabled.").await
}

pub(super) async fn enable_room(ctx: &Context<'_>, room_id: OwnedRoomId) -> AppResult<()> {
    crate::room::disable_room(&room_id, false)?;
    ctx.write_str("Room enabled.").await
}

pub(super) async fn fetch_support_well_known(
    ctx: &Context<'_>,
    server_name: OwnedServerName,
) -> AppResult<()> {
    let response = sending::default_client()
        .get(format!("https://{server_name}/.well-known/matrix/support"))
        .send()
        .await?;

    let text = response.text().await?;

    if text.is_empty() {
        return Err(AppError::public("Response text/body is empty."));
    }

    if text.len() > 1500 {
        return Err(AppError::public(
            "Response text/body is over 1500 characters, assuming no support well-known.",
        ));
    }

    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(_) => {
            return Err(AppError::public("Response text/body is not valid JSON."));
        }
    };

    let pretty_json: String = match serde_json::to_string_pretty(&json) {
        Ok(json) => json,
        Err(_) => {
            return Err(AppError::public("Response text/body is not valid JSON."));
        }
    };

    ctx.write_str(&format!(
        "Got JSON response:\n\n```json\n{pretty_json}\n```"
    ))
    .await
}

pub(super) async fn remote_user_in_rooms(ctx: &Context<'_>, user_id: OwnedUserId) -> AppResult<()> {
    if user_id.server_name() == config::server_name() {
        return Err(AppError::public(
            "User belongs to our server, please use `list-joined-rooms` user admin command \
			 instead.",
        ));
    }

    if !data::user::user_exists(&user_id)? {
        return Err(AppError::public(
            "Remote user does not exist in our database.",
        ));
    }

    let mut rooms: Vec<RoomInfo> = data::user::joined_rooms(&user_id)?
        .into_iter()
        .map(|room_id| get_room_info(&room_id))
        .collect();

    if rooms.is_empty() {
        return Err(AppError::public("User is not in any rooms."));
    }

    rooms.sort_by_key(|r| r.joined_members);
    rooms.reverse();

    let num = rooms.len();
    let body = rooms
        .iter()
        .map(
            |RoomInfo {
                 id,
                 joined_members,
                 name,
             }| format!("{id} | Members: {joined_members} | Name: {name}"),
        )
        .collect::<Vec<_>>()
        .join("\n");

    ctx.write_str(&format!(
        "Rooms {user_id} shares with us ({num}):\n```\n{body}\n```",
    ))
    .await
}
