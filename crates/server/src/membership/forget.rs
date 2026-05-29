use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::identifiers::*;
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::{AppResult, MatrixError};

/// Makes a user forget a room.
#[tracing::instrument]
pub async fn forget_room(user_id: &UserId, room_id: &RoomId) -> AppResult<()> {
    if diesel_exists!(
        room_users::table
            .filter(room_users::user_id.eq(user_id))
            .filter(room_users::room_id.eq(room_id))
            .filter(room_users::membership.eq("join")),
        &mut connect().await?
    )? {
        return Err(MatrixError::unknown("The user has not left the room.").into());
    }
    diesel::update(
        room_users::table
            .filter(room_users::user_id.eq(user_id))
            .filter(room_users::room_id.eq(room_id)),
    )
    .set(room_users::forgotten.eq(true))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}
