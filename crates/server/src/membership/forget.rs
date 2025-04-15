use diesel::prelude::*;

use crate::core::identifiers::*;
use crate::data::connect;
use crate::data::schema::*;
use crate::{AppResult, MatrixError, data, diesel_exists};

/// Makes a user forget a room.
#[tracing::instrument]
pub fn forget_room(user_id: &UserId, room_id: &RoomId) -> AppResult<()> {
    if diesel_exists!(
        room_users::table
            .filter(room_users::user_id.eq(user_id))
            .filter(room_users::room_id.eq(room_id))
            .filter(room_users::membership.eq("join")),
        &mut connect()?
    )? {
        return Err(MatrixError::unknown("The user has not left the room.").into());
    }
    diesel::update(
        room_users::table
            .filter(room_users::user_id.eq(user_id))
            .filter(room_users::room_id.eq(room_id)),
    )
    .set(room_users::forgotten.eq(true))
    .execute(&mut connect()?)?;
    Ok(())
}
