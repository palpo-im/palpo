use clap::Subcommand;

use crate::admin::{Context, admin_command_dispatch};
use crate::core::OwnedRoomId;
use crate::{AppError, AppResult, IsRemoteOrLocal, data};

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(crate) enum RoomInfoCommand {
    /// - List joined members in a room
    ListJoinedMembers {
        room_id: OwnedRoomId,

        /// Lists only our local users in the specified room
        #[arg(long)]
        local_only: bool,
    },

    /// - Displays room topic
    ///
    /// Room topics can be huge, so this is in its
    /// own separate command
    ViewRoomTopic { room_id: OwnedRoomId },
}

async fn list_joined_members(
    ctx: &Context<'_>,
    room_id: OwnedRoomId,
    local_only: bool,
) -> AppResult<()> {
    let room_name = crate::room::get_name(&room_id)
        .await
        .unwrap_or_else(|_| room_id.to_string());

    let mut member_info: Vec<_> = Vec::new();
    for user_id in crate::room::joined_users(&room_id, None).await?.into_iter() {
        if local_only && !user_id.is_local() {
            continue;
        }
        let displayname = data::user::display_name(&user_id)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| user_id.to_string());
        member_info.push((displayname, user_id));
    }

    let num = member_info.len();
    let body = member_info
        .into_iter()
        .map(|(displayname, mxid)| format!("{mxid} | {displayname}"))
        .collect::<Vec<_>>()
        .join("\n");

    ctx.write_str(&format!(
        "{num} Members in Room \"{room_name}\":\n```\n{body}\n```",
    ))
    .await
}

async fn view_room_topic(ctx: &Context<'_>, room_id: OwnedRoomId) -> AppResult<()> {
    let Ok(room_topic) = crate::room::get_topic(&room_id).await else {
        return Err(AppError::public("Room does not have a room topic set."));
    };

    ctx.write_str(&format!("Room topic:\n```\n{room_topic}\n```"))
        .await
}
