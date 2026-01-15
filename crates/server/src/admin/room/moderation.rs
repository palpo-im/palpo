use clap::Subcommand;

use crate::admin::{Context, get_room_info};
use crate::core::{OwnedRoomId, OwnedRoomOrAliasId, RoomAliasId, RoomId, RoomOrAliasId};
use crate::macros::admin_command_dispatch;
use crate::{AppError, AppResult, IsRemoteOrLocal, config, membership};

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(crate) enum RoomModerationCommand {
    /// - Bans a room from local users joining and evicts all our local users (including server
    /// admins)
    ///   from the room. Also blocks any invites (local and remote) for the
    ///   banned room, and disables federation entirely with it.
    BanRoom {
        /// The room in the format of `!roomid:example.com` or a room alias in
        /// the format of `#roomalias:example.com`
        room: OwnedRoomOrAliasId,
    },

    /// - Bans a list of rooms (room IDs and room aliases) from a newline delimited codeblock
    ///   similar to `user deactivate-all`. Applies the same steps as ban-room
    BanListOfRooms,

    /// - Unbans a room to allow local users to join again
    UnbanRoom {
        /// The room in the format of `!roomid:example.com` or a room alias in
        /// the format of `#roomalias:example.com`
        room: OwnedRoomOrAliasId,
    },

    /// - List of all rooms we have banned
    ListBannedRooms {
        #[arg(long)]
        /// Whether to only output room IDs without supplementary room
        /// information
        no_details: bool,
    },
}

async fn ban_room(ctx: &Context<'_>, room: OwnedRoomOrAliasId) -> AppResult<()> {
    debug!("Got room alias or ID: {}", room);

    let admin_room_alias = config::admin_alias();

    if let Ok(admin_room_id) = crate::room::get_admin_room()
        && (room.to_string().eq(&admin_room_id) || room.to_string().eq(admin_room_alias))
    {
        return Err(AppError::public("Not allowed to ban the admin room."));
    }

    let room_id = if room.is_room_id() {
        let room_id = match RoomId::parse(&room) {
            Ok(room_id) => room_id,
            Err(e) => {
                return Err(AppError::public(format!(
                    "Failed to parse room ID {room}. Please note that this requires a full room \
					 ID (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias \
					 (`#roomalias:example.com`): {e}"
                )));
            }
        };

        debug!("Room specified is a room ID, banning room ID");
        crate::room::ban_room(&room_id, true)?;

        room_id.to_owned()
    } else if room.is_room_alias_id() {
        let room_alias = match RoomAliasId::parse(&room) {
            Ok(room_alias) => room_alias,
            Err(e) => {
                return Err(AppError::public(format!(
                    "Failed to parse room ID {room}. Please note that this requires a full room \
					 ID (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias \
					 (`#roomalias:example.com`): {e}"
                )));
            }
        };

        debug!(
            "Room specified is not a room ID, attempting to resolve room alias to a room ID \
			 locally, if not using get_alias_helper to fetch room ID remotely"
        );

        let room_id = match crate::room::alias::resolve_local_alias(&room_alias) {
            Ok(room_id) => room_id,
            _ => {
                debug!(
                    "We don't have this room alias to a room ID locally, attempting to fetch \
					 room ID over federation"
                );

                match crate::room::alias::resolve_alias(&room_alias, None).await {
                    Ok((room_id, servers)) => {
                        debug!(
                            ?room_id,
                            ?servers,
                            "Got federation response fetching room ID for {room_id}"
                        );
                        room_id
                    }
                    Err(e) => {
                        return Err(AppError::public(format!(
                            "Failed to resolve room alias {room_alias} to a room ID: {e}"
                        )));
                    }
                }
            }
        };

        crate::room::ban_room(&room_id, true)?;

        room_id
    } else {
        return Err(AppError::public(
            "Room specified is not a room ID or room alias. Please note that this requires a \
			 full room ID (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias \
			 (`#roomalias:example.com`)",
        ));
    };

    // TODO: this should be done
    debug!("Making all users leave the room {room_id} and forgetting it");
    // let mut users = self
    //     .services
    //     .rooms
    //     .state_cache
    //     .room_members(&room_id)
    //     .map(ToOwned::to_owned)
    //     .ready_filter(|user| user.is_local())
    //     .boxed();

    // while let Some(ref user_id) = users.next().await {
    //     debug!(
    //         "Attempting leave for user {user_id} in room {room_id} (ignoring all errors, \
    // 		 evicting admins too)",
    //     );

    //     if let Err(e) = membership::leave_room(user_id, &room_id, None).boxed().await {
    //         warn!("Failed to leave room: {e}");
    //     }

    //     crate::membership::forget_room(user_id, &room_id)?;
    // }

    // self.services
    //     .rooms
    //     .alias
    //     .local_aliases_for_room(&room_id)
    //     .map(ToOwned::to_owned)
    //     .for_each(async |local_alias| {
    //         if let Ok(server_user) = crate::data::user::get_user(config::server_user()) {
    //             crate::room::remove_alias(&local_alias, &server_user).await.ok();
    //         }
    //     })
    //     .await;

    // unpublish from room directory
    crate::room::directory::set_public(&room_id, false)?;

    crate::room::disable_room(&room_id, true)?;

    ctx.write_str(
        "Room banned, removed all our local users, and disabled incoming federation with room.",
    )
    .await
}

async fn ban_list_of_rooms(ctx: &Context<'_>) -> AppResult<()> {
    if ctx.body.len() < 2
        || !ctx.body[0].trim().starts_with("```")
        || ctx.body.last().unwrap_or(&"").trim() != "```"
    {
        return Err(AppError::public(
            "Expected code block in command body. Add --help for details.",
        ));
    }

    let rooms_s = ctx
        .body
        .to_vec()
        .drain(1..ctx.body.len().saturating_sub(1))
        .collect::<Vec<_>>();

    let admin_room_alias = config::admin_alias();

    let mut room_ban_count: usize = 0;
    let mut room_ids: Vec<OwnedRoomId> = Vec::new();

    for &room in &rooms_s {
        match <&RoomOrAliasId>::try_from(room) {
            Ok(room_alias_or_id) => {
                if let Ok(admin_room_id) = crate::room::get_admin_room()
                    && (room.to_owned().eq(&admin_room_id) || room.to_owned().eq(admin_room_alias))
                {
                    warn!("User specified admin room in bulk ban list, ignoring");
                    continue;
                }

                if room_alias_or_id.is_room_id() {
                    let room_id = match RoomId::parse(room_alias_or_id) {
                        Ok(room_id) => room_id,
                        Err(e) => {
                            // ignore rooms we failed to parse
                            warn!(
                                "Error parsing room \"{room}\" during bulk room banning, \
								 ignoring error and logging here: {e}"
                            );
                            continue;
                        }
                    };

                    room_ids.push(room_id.to_owned());
                }

                if room_alias_or_id.is_room_alias_id() {
                    match RoomAliasId::parse(room_alias_or_id) {
                        Ok(room_alias) => {
                            let room_id = match crate::room::alias::resolve_local_alias(&room_alias)
                            {
                                Ok(room_id) => room_id,
                                _ => {
                                    debug!(
                                        "We don't have this room alias to a room ID locally, \
										 attempting to fetch room ID over federation"
                                    );

                                    match crate::room::alias::resolve_alias(&room_alias, None).await
                                    {
                                        Ok((room_id, servers)) => {
                                            debug!(
                                                ?room_id,
                                                ?servers,
                                                "Got federation response fetching room ID for \
												 {room}",
                                            );
                                            room_id
                                        }
                                        Err(e) => {
                                            warn!(
                                                "Failed to resolve room alias {room} to a room \
												 ID: {e}"
                                            );
                                            continue;
                                        }
                                    }
                                }
                            };

                            room_ids.push(room_id);
                        }
                        Err(e) => {
                            warn!(
                                "Error parsing room \"{room}\" during bulk room banning, \
								 ignoring error and logging here: {e}"
                            );
                            continue;
                        }
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Error parsing room \"{room}\" during bulk room banning, ignoring error and \
					 logging here: {e}"
                );
                continue;
            }
        }
    }

    for room_id in room_ids {
        crate::room::ban_room(&room_id, true)?;

        debug!("Banned {room_id} successfully");
        room_ban_count = room_ban_count.saturating_add(1);

        debug!("Making all users leave the room {room_id} and forgetting it");
        let users = crate::room::joined_users(&room_id, None)?
            .into_iter()
            .filter(|user| user.is_local())
            .collect::<Vec<_>>();

        for user_id in &users {
            debug!(
                "Attempting leave for user {user_id} in room {room_id} (ignoring all errors, \
				 evicting admins too)",
            );

            if let Err(e) = membership::leave_room(user_id, &room_id, None).await {
                warn!("Failed to leave room: {e}");
            }

            membership::forget_room(user_id, &room_id)?;
        }

        // TODO: admin
        // // remove any local aliases, ignore errors
        // self.services
        //     .rooms
        //     .alias
        //     .local_aliases_for_room(&room_id)
        //     .map(ToOwned::to_owned)
        //     .for_each(async |local_alias| {
        //         if let Ok(server_user) = crate::data::user::get_user(config::server_user()) {
        //             crate::room::remove_alias(&local_alias, &server_user).await.ok();
        //         }
        //     })
        //     .await;

        // unpublish from room directory, ignore errors
        crate::room::directory::set_public(&room_id, false)?;
        crate::room::disable_room(&room_id, true)?;
    }

    ctx.write_str(&format!(
        "Finished bulk room ban, banned {room_ban_count} total rooms, evicted all users, and \
		 disabled incoming federation with the room."
    ))
    .await
}

async fn unban_room(ctx: &Context<'_>, room: OwnedRoomOrAliasId) -> AppResult<()> {
    let room_id = if room.is_room_id() {
        let room_id = match RoomId::parse(&room) {
            Ok(room_id) => room_id,
            Err(e) => {
                return Err(AppError::public(format!(
                    "Failed to parse room ID {room}. Please note that this requires a full room \
					 ID (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias \
					 (`#roomalias:example.com`): {e}"
                )));
            }
        };

        debug!("Room specified is a room ID, unbanning room ID");
        crate::room::ban_room(&room_id, false)?;

        room_id.to_owned()
    } else if room.is_room_alias_id() {
        let room_alias = match RoomAliasId::parse(&room) {
            Ok(room_alias) => room_alias,
            Err(e) => {
                return Err(AppError::public(format!(
                    "Failed to parse room ID {room}. Please note that this requires a full room \
					 ID (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias \
					 (`#roomalias:example.com`): {e}"
                )));
            }
        };

        debug!(
            "Room specified is not a room ID, attempting to resolve room alias to a room ID \
			 locally, if not using get_alias_helper to fetch room ID remotely"
        );

        let room_id = match crate::room::alias::resolve_local_alias(&room_alias) {
            Ok(room_id) => room_id,
            _ => {
                debug!(
                    "We don't have this room alias to a room ID locally, attempting to fetch \
					 room ID over federation"
                );

                match crate::room::alias::resolve_alias(&room_alias, None).await {
                    Ok((room_id, servers)) => {
                        debug!(
                            ?room_id,
                            ?servers,
                            "Got federation response fetching room ID for room {room}"
                        );
                        room_id
                    }
                    Err(e) => {
                        return Err(AppError::public(format!(
                            "Failed to resolve room alias {room} to a room ID: {e}"
                        )));
                    }
                }
            }
        };

        crate::room::ban_room(&room_id, false)?;

        room_id
    } else {
        return Err(AppError::public(
            "Room specified is not a room ID or room alias. Please note that this requires a \
			 full room ID (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias \
			 (`#roomalias:example.com`)",
        ));
    };

    crate::room::disable_room(&room_id, false)?;
    ctx.write_str("Room unbanned and federation re-enabled.")
        .await
}

async fn list_banned_rooms(ctx: &Context<'_>, no_details: bool) -> AppResult<()> {
    let room_ids: Vec<OwnedRoomId> = crate::room::list_banned_rooms()?;

    if room_ids.is_empty() {
        return Err(AppError::public("No rooms are banned."));
    }

    let mut rooms = room_ids
        .iter()
        .map(|room_id| get_room_info(room_id))
        .collect::<Vec<_>>();

    rooms.sort_by_key(|r| r.joined_members);
    rooms.reverse();

    let num = rooms.len();

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

    ctx.write_str(&format!("Rooms Banned ({num}):\n```\n{body}\n```",))
        .await
}
