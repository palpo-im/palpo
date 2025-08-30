use crate::AppResult;
use crate::core::{EventId, MatrixError, RoomId, ServerName};
use crate::event::handler;
use crate::room::{self, state};

pub fn access_check(
    origin: &ServerName,
    room_id: &RoomId,
    event_id: Option<&EventId>,
) -> AppResult<()> {
    if !room::is_server_joined(origin, room_id)? {
        return Err(MatrixError::forbidden("Server is not in room.", None).into());
    }

    handler::acl_check(origin, room_id)?;

    // let world_readable = crate::room::is_world_readable(room_id);

    // if any user on our homeserver is trying to knock this room, we'll need to
    // acknowledge bans or leaves
    // let user_is_knocking = crate::room::members_knocked(room_id).count();

    if let Some(event_id) = event_id
        && !state::server_can_see_event(origin, room_id, event_id)?
    {
        return Err(MatrixError::forbidden("Server is not allowed to see event.", None).into());
    }

    Ok(())
}
