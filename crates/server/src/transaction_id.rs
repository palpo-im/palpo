use crate::core::identifiers::*;
use crate::core::{DeviceId, TransactionId, UserId};
use crate::{AppResult, data};

pub async fn add_txn_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
    event_id: Option<&EventId>,
) -> AppResult<()> {
    data::room::transaction_id::add_txn_id(txn_id, user_id, device_id, room_id, event_id).await?;
    Ok(())
}

pub async fn txn_id_exists(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
) -> AppResult<bool> {
    Ok(data::room::transaction_id::txn_id_exists(txn_id, user_id, device_id).await?)
}

pub async fn get_event_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
) -> AppResult<Option<OwnedEventId>> {
    Ok(data::room::transaction_id::get_event_id(txn_id, user_id, device_id, room_id).await?)
}
