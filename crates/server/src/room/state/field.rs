use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::AppResult;
use crate::core::events::StateEventType;
use crate::data::connect;
use crate::data::schema::*;

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = room_state_fields)]
pub struct DbRoomStateField {
    pub id: i64,
    pub event_ty: StateEventType,
    pub state_key: String,
}

pub async fn get_field(field_id: i64) -> AppResult<DbRoomStateField> {
    room_state_fields::table
        .find(field_id)
        .first::<DbRoomStateField>(&mut connect().await?)
        .await
        .map_err(Into::into)
}
pub async fn get_field_id(event_ty: &StateEventType, state_key: &str) -> AppResult<i64> {
    room_state_fields::table
        .filter(room_state_fields::event_ty.eq(event_ty))
        .filter(room_state_fields::state_key.eq(state_key))
        .select(room_state_fields::id)
        .first::<i64>(&mut connect().await?)
        .await
        .map_err(Into::into)
}
pub async fn ensure_field_id(event_ty: &StateEventType, state_key: &str) -> AppResult<i64> {
    let id = diesel::insert_into(room_state_fields::table)
        .values((
            room_state_fields::event_ty.eq(event_ty),
            room_state_fields::state_key.eq(state_key),
        ))
        .on_conflict_do_nothing()
        .returning(room_state_fields::id)
        .get_result::<i64>(&mut connect().await?)
        .await
        .optional()?;
    if let Some(id) = id {
        Ok(id)
    } else {
        room_state_fields::table
            .filter(room_state_fields::event_ty.eq(event_ty))
            .filter(room_state_fields::state_key.eq(state_key))
            .select(room_state_fields::id)
            .first::<i64>(&mut connect().await?)
            .await
            .map_err(Into::into)
    }
}
pub async fn ensure_field(
    event_ty: &StateEventType,
    state_key: &str,
) -> AppResult<DbRoomStateField> {
    let id = diesel::insert_into(room_state_fields::table)
        .values((
            room_state_fields::event_ty.eq(event_ty),
            room_state_fields::state_key.eq(state_key),
        ))
        .on_conflict_do_nothing()
        .returning(room_state_fields::id)
        .get_result::<i64>(&mut connect().await?)
        .await
        .optional()?;
    if let Some(id) = id {
        room_state_fields::table
            .find(id)
            .first::<DbRoomStateField>(&mut connect().await?)
            .await
            .map_err(Into::into)
    } else {
        room_state_fields::table
            .filter(room_state_fields::event_ty.eq(event_ty))
            .filter(room_state_fields::state_key.eq(state_key))
            .first::<DbRoomStateField>(&mut connect().await?)
            .await
            .map_err(Into::into)
    }
}
