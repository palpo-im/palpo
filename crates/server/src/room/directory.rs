use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::AppResult;
use crate::core::RoomId;
use crate::core::room::Visibility;
use crate::data::connect;
use crate::data::schema::*;

#[tracing::instrument]
pub async fn set_public(room_id: &RoomId, value: bool) -> AppResult<()> {
    diesel::update(rooms::table.find(room_id))
        .set(rooms::is_public.eq(value))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

#[tracing::instrument]
pub async fn is_public(room_id: &RoomId) -> AppResult<bool> {
    rooms::table
        .find(room_id)
        .select(rooms::is_public)
        .first::<bool>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

#[tracing::instrument]
pub async fn visibility(room_id: &RoomId) -> Visibility {
    if is_public(room_id).await.unwrap_or(false) {
        Visibility::Public
    } else {
        Visibility::Private
    }
}
