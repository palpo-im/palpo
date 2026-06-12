use crate::AppResult;
use crate::core::identifiers::*;
use crate::data;
use crate::data::room::DbRoomCurrent;

#[tracing::instrument]
pub async fn get_current(room_id: &RoomId) -> AppResult<Option<DbRoomCurrent>> {
    Ok(data::room::get_room_current(room_id).await?)
}

#[tracing::instrument]
pub async fn invite_count(room_id: &RoomId, user_id: &UserId) -> AppResult<Option<u64>> {
    Ok(data::room::invited_members_count(room_id).await?.map(|c| c as u64))
}

#[tracing::instrument]
pub async fn left_count(room_id: &RoomId, user_id: &UserId) -> AppResult<Option<u64>> {
    Ok(data::room::left_members_count(room_id).await?.map(|c| c as u64))
}
