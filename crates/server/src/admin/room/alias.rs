use std::fmt::Write;

use crate::core::{OwnedRoomAliasId, OwnedRoomId, RoomAliasId};
use clap::Subcommand;
use futures_util::StreamExt;
use palpo_core::{AppError, AppResult};

use crate::Context;

#[derive(Debug, Subcommand)]
pub(crate) enum RoomAliasCommand {
    /// - Make an alias point to a room.
    Set {
        #[arg(short, long)]
        /// Set the alias even if a room is already using it
        force: bool,

        /// The room id to set the alias on
        room_id: OwnedRoomId,

        /// The alias localpart to use (`alias`, not `#alias:servername.tld`)
        room_alias_localpart: String,
    },

    /// - Remove a local alias
    Remove {
        /// The alias localpart to remove (`alias`, not `#alias:servername.tld`)
        room_alias_localpart: String,
    },

    /// - Show which room is using an alias
    Which {
        /// The alias localpart to look up (`alias`, not
        /// `#alias:servername.tld`)
        room_alias_localpart: String,
    },

    /// - List aliases currently being used
    List {
        /// If set, only list the aliases for this room
        room_id: Option<OwnedRoomId>,
    },
}

pub(super) async fn process(command: RoomAliasCommand, context: &Context<'_>) -> AppResult<()> {
    let services = context.services;
    let server_user = &services.globals.server_user;

    match command {
        RoomAliasCommand::Set {
            ref room_alias_localpart,
            ..
        }
        | RoomAliasCommand::Remove {
            ref room_alias_localpart,
        }
        | RoomAliasCommand::Which {
            ref room_alias_localpart,
        } => {
            let room_alias_str = format!("#{}:{}", room_alias_localpart, services.globals.server_name());
            let room_alias = match RoomAliasId::parse(room_alias_str) {
                Ok(alias) => alias,
                Err(err) => {
                    return Err(AppError::public(format!("Failed to parse alias: {err}")));
                }
            };
            match command {
                RoomAliasCommand::Set { force, room_id, .. } => {
                    match (force, services.rooms.alias.resolve_local_alias(&room_alias).await) {
                        (true, Ok(id)) => match services.rooms.alias.set_alias(&room_alias, &room_id, server_user) {
                            Err(err) => Err(AppError::public(format!("Failed to remove alias: {err}"))),
                            Ok(()) => {
                                context
                                    .write_str(&format!("Successfully overwrote alias (formerly {id})"))
                                    .await
                            }
                        },
                        (false, Ok(id)) => Err(AppError::public(format!(
                            "Refusing to overwrite in use alias for {id}, use -f or --force to \
							 overwrite"
                        ))),
                        (_, Err(_)) => match services.rooms.alias.set_alias(&room_alias, &room_id, server_user) {
                            Err(err) => Err(AppError::public(format!("Failed to remove alias: {err}"))),
                            Ok(()) => context.write_str("Successfully set alias").await,
                        },
                    }
                }
                RoomAliasCommand::Remove { .. } => match services.rooms.alias.resolve_local_alias(&room_alias).await {
                    Err(_) => Err(AppError::public("Alias isn't in use.")),
                    Ok(id) => match crate::room::remove_alias(&room_alias, server_user).await {
                        Err(err) => Err(AppError::public(format!("Failed to remove alias: {err}"))),
                        Ok(()) => context.write_str(&format!("Removed alias from {id}")).await,
                    },
                },
                RoomAliasCommand::Which { .. } => match services.rooms.alias.resolve_local_alias(&room_alias).await {
                    Err(_) => Err(AppError::public("Alias isn't in use.")),
                    Ok(id) => context.write_str(&format!("Alias resolves to {id}")).await,
                },
                RoomAliasCommand::List { .. } => unreachable!(),
            }
        }
        RoomAliasCommand::List { room_id } => {
            if let Some(room_id) = room_id {
                let aliases: Vec<OwnedRoomAliasId> = services
                    .rooms
                    .alias
                    .local_aliases_for_room(&room_id)
                    .map(Into::into)
                    .collect()
                    .await;

                let plain_list = aliases.iter().fold(String::new(), |mut output, alias| {
                    writeln!(output, "- {alias}").expect("should be able to write to string buffer");
                    output
                });

                let plain = format!("Aliases for {room_id}:\n{plain_list}");
                context.write_str(&plain).await
            } else {
                let aliases = services
                    .rooms
                    .alias
                    .all_local_aliases()
                    .map(|(room_id, localpart)| (room_id.into(), localpart.into()))
                    .collect::<Vec<(OwnedRoomId, String)>>()
                    .await;

                let server_name = services.globals.server_name();
                let plain_list = aliases.iter().fold(String::new(), |mut output, (alias, id)| {
                    writeln!(output, "- `{alias}` -> #{id}:{server_name}")
                        .expect("should be able to write to string buffer");
                    output
                });

                let plain = format!("Aliases:\n{plain_list}");
                context.write_str(&plain).await
            }
        }
    }
}
