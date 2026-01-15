mod alias;
mod directory;
mod info;
mod moderation;

use clap::Subcommand;

use self::alias::RoomAliasCommand;
use self::directory::RoomDirectoryCommand;
use self::info::RoomInfoCommand;
use self::moderation::RoomModerationCommand;
use crate::admin::{Context, PAGE_SIZE, get_room_info};
use crate::core::OwnedRoomId;
use crate::macros::admin_command_dispatch;
use crate::{AppError, AppResult, data};

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(crate) enum RoomCommand {
    /// - List all rooms the server knows about
    #[clap(alias = "list")]
    ListRooms {
        page: Option<usize>,

        /// Excludes rooms that we have federation disabled with
        #[arg(long)]
        exclude_disabled: bool,

        /// Excludes rooms that we have banned
        #[arg(long)]
        exclude_banned: bool,

        #[arg(long)]
        /// Whether to only output room IDs without supplementary room
        /// information
        no_details: bool,
    },

    #[command(subcommand)]
    /// - View information about a room we know about
    Info(RoomInfoCommand),

    #[command(subcommand)]
    /// - Manage moderation of remote or local rooms
    Moderation(RoomModerationCommand),

    #[command(subcommand)]
    /// - Manage rooms' aliases
    Alias(RoomAliasCommand),

    #[command(subcommand)]
    /// - Manage the room directory
    Directory(RoomDirectoryCommand),

    /// - Check if we know about a room
    Exists { room_id: OwnedRoomId },
}

pub(super) async fn list_rooms(
    ctx: &Context<'_>,
    page: Option<usize>,
    exclude_disabled: bool,
    exclude_banned: bool,
    no_details: bool,
) -> AppResult<()> {
    // TODO: i know there's a way to do this with clap, but i can't seem to find it
    let page = page.unwrap_or(1);
    let mut rooms = crate::room::all_room_ids()?
        .iter()
        .filter(|room_id| !exclude_disabled || !crate::room::is_disabled(room_id).unwrap_or(false))
        .filter(|room_id| !exclude_banned || !data::room::is_banned(room_id).unwrap_or(true))
        .map(|room_id| get_room_info(room_id))
        .collect::<Vec<_>>();

    rooms.sort_by_key(|r| r.joined_members);
    rooms.reverse();

    let rooms = rooms
        .into_iter()
        .skip(page.saturating_sub(1).saturating_mul(PAGE_SIZE))
        .take(PAGE_SIZE)
        .collect::<Vec<_>>();

    if rooms.is_empty() {
        return Err(AppError::public("No more rooms."));
    }

    let body = rooms
        .iter()
        .map(|info| {
            if no_details {
                format!("{}", info.id)
            } else {
                format!(
                    "{}\tMembers: {}\tName: {}",
                    info.id, info.joined_members, info.name
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    ctx.write_str(&format!("Rooms ({}):\n```\n{body}\n```", rooms.len(),))
        .await
}

pub(super) async fn exists(ctx: &Context<'_>, room_id: OwnedRoomId) -> AppResult<()> {
    let result = crate::room::room_exists(&room_id)?;

    ctx.write_str(&format!("{result}")).await
}
