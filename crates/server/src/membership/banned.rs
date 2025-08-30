use salvo::conn::SocketAddr;

use crate::core::events::room::message::RoomMessageEventContent;
use crate::core::identifiers::*;
use crate::{AppResult, MatrixError, data};

/// Checks if the room is banned in any way possible and the sender user is not
/// an admin.
///
/// Performs automatic deactivation if `auto_deactivate_banned_room_attempts` is
/// enabled
#[tracing::instrument]
pub async fn banned_room_check(
    user_id: &UserId,
    room_id: Option<&RoomId>,
    server_name: Option<&ServerName>,
    client_addr: &SocketAddr,
) -> AppResult<()> {
    if data::user::is_admin(user_id)? {
        return Ok(());
    }

    let conf = crate::config::get();
    if let Some(room_id) = room_id {
        if data::room::is_disabled(room_id)?
            || conf
                .forbidden_remote_server_names
                .is_match(room_id.server_name().unwrap().host())
        {
            warn!(
                "User {user_id} who is not an admin attempted to send an invite for or \
				 attempted to join a banned room or banned room server name: {room_id}"
            );

            if conf.auto_deactivate_banned_room_attempts {
                warn!(
                    "Automatically deactivating user {user_id} due to attempted banned room join"
                );

                if conf.admin.room_notices {
                    crate::admin::send_message(RoomMessageEventContent::text_plain(format!(
                        "Automatically deactivating user {user_id} due to attempted banned \
							 room join from IP {client_addr}"
                    )))
                    .await
                    .ok();
                }

                let all_joined_rooms: Vec<OwnedRoomId> = data::user::joined_rooms(user_id)?;

                crate::user::full_user_deactivate(user_id, &all_joined_rooms).await?;
            }

            return Err(
                MatrixError::forbidden("This room is banned on this homeserver.", None).into(),
            );
        }
    } else if let Some(server_name) = server_name
        && conf
            .forbidden_remote_server_names
            .is_match(server_name.host())
    {
        warn!(
            "User {user_id} who is not an admin tried joining a room which has the server \
				 name {server_name} that is globally forbidden. Rejecting.",
        );

        if conf.auto_deactivate_banned_room_attempts {
            warn!("Automatically deactivating user {user_id} due to attempted banned room join");
            if conf.admin.room_notices {
                crate::admin::send_message(RoomMessageEventContent::text_plain(format!(
                    "Automatically deactivating user {user_id} due to attempted banned \
							 room join from IP {client_addr}"
                )))
                .await
                .ok();
            }

            let all_joined_rooms = data::user::joined_rooms(user_id)?;
            crate::user::full_user_deactivate(user_id, &all_joined_rooms).await?;
        }

        return Err(MatrixError::forbidden(
            "This remote server is banned on this homeserver.",
            None,
        )
        .into());
    }
    Ok(())
}
