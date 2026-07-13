use crate::core::events::StateEventType;
pub use crate::data::room::DbRoomStateField;
use crate::{AppResult, data};

pub async fn get_field(field_id: i64) -> AppResult<DbRoomStateField> {
    Ok(data::room::get_state_field(field_id).await?)
}
pub async fn get_field_id(event_ty: &StateEventType, state_key: &str) -> AppResult<i64> {
    Ok(data::room::get_state_field_id(event_ty, state_key).await?)
}
pub async fn ensure_field_id(event_ty: &StateEventType, state_key: &str) -> AppResult<i64> {
    Ok(data::room::ensure_state_field_id(event_ty, state_key).await?)
}
pub async fn ensure_field(
    event_ty: &StateEventType,
    state_key: &str,
) -> AppResult<DbRoomStateField> {
    Ok(data::room::ensure_state_field(event_ty, state_key).await?)
}
