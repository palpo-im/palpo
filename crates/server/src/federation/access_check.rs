use crate::AppResult;
use crate::core::{EventId, MatrixError, RoomId, ServerName};
use crate::event::handler;
use crate::room::{self, state};

pub async fn access_check(
    origin: &ServerName,
    room_id: &RoomId,
    event_id: Option<&EventId>,
) -> AppResult<()> {
    if !room::is_server_joined(origin, room_id).await? {
        return Err(MatrixError::forbidden(
            format!("server `{origin}` is not in room `{room_id}`"),
            None,
        )
        .into());
    }

    handler::acl_check(origin, room_id).await?;

    // let world_readable = crate::room::is_world_readable(room_id);

    // if any user on our homeserver is trying to knock this room, we'll need to
    // acknowledge bans or leaves
    // let user_is_knocking = crate::room::members_knocked(room_id).count();

    if let Some(event_id) = event_id
        && !state::server_can_see_event(origin, room_id, event_id).await?
    {
        return Err(MatrixError::forbidden("server is not allowed to see event", None).into());
    }

    Ok(())
}
