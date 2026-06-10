use crate::AppResult;
use crate::core::RoomId;
use crate::core::room::Visibility;
use crate::data;

#[tracing::instrument]
pub async fn set_public(room_id: &RoomId, value: bool) -> AppResult<()> {
    data::room::set_public(room_id, value).await?;
    Ok(())
}

#[tracing::instrument]
pub async fn is_public(room_id: &RoomId) -> AppResult<bool> {
    Ok(data::room::is_public(room_id).await?)
}

#[tracing::instrument]
pub async fn visibility(room_id: &RoomId) -> Visibility {
    if is_public(room_id).await.unwrap_or(false) {
        Visibility::Public
    } else {
        Visibility::Private
    }
}
