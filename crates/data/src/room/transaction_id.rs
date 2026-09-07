use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::identifiers::*;
use crate::core::{DeviceId, TransactionId, UnixMillis, UserId};
use crate::room::NewDbEventIdempotent;
use crate::schema::*;
use crate::{DataResult, connect};

/// Record a transaction id together with the event it produced for idempotency.
pub async fn add_txn_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
    event_id: Option<&EventId>,
) -> DataResult<()> {
    diesel::insert_into(event_idempotents::table)
        .values(&NewDbEventIdempotent {
            txn_id: txn_id.to_owned(),
            user_id: user_id.to_owned(),
            device_id: device_id.map(|d| d.to_owned()),
            room_id: room_id.map(|r| r.to_owned()),
            event_id: event_id.map(|e| e.to_owned()),
            created_at: UnixMillis::now(),
        })
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Check whether a `(user, device, txn_id)` combination has already been seen.
pub async fn txn_id_exists(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
) -> DataResult<bool> {
    if let Some(device_id) = device_id {
        let query = event_idempotents::table
            .filter(event_idempotents::user_id.eq(user_id))
            .filter(event_idempotents::device_id.eq(device_id))
            .filter(event_idempotents::txn_id.eq(txn_id))
            .select(event_idempotents::event_id);
        diesel_exists!(query, &mut connect().await?).map_err(Into::into)
    } else {
        let query = event_idempotents::table
            .filter(event_idempotents::user_id.eq(user_id))
            .filter(event_idempotents::device_id.is_null())
            .filter(event_idempotents::txn_id.eq(txn_id))
            .select(event_idempotents::event_id);
        diesel_exists!(query, &mut connect().await?).map_err(Into::into)
    }
}

/// Look up the event id previously recorded for a transaction.
pub async fn get_event_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
) -> DataResult<Option<OwnedEventId>> {
    let mut query = event_idempotents::table
        .filter(event_idempotents::user_id.eq(user_id))
        .filter(event_idempotents::txn_id.eq(txn_id))
        .into_boxed();
    if let Some(device_id) = device_id {
        query = query.filter(event_idempotents::device_id.eq(device_id));
    } else {
        query = query.filter(event_idempotents::device_id.is_null());
    }
    if let Some(room_id) = room_id {
        query = query.filter(event_idempotents::room_id.eq(room_id));
    } else {
        query = query.filter(event_idempotents::room_id.is_null());
    }
    query
        .select(event_idempotents::event_id)
        .first::<Option<OwnedEventId>>(&mut connect().await?)
        .await
        .optional()
        .map(|v| v.flatten())
        .map_err(Into::into)
}

/// Recover the originating device using the complete, locally recorded event identity.
pub async fn get_event_device(
    txn_id: &TransactionId,
    user_id: &UserId,
    room_id: &RoomId,
    event_id: &EventId,
) -> DataResult<Option<OwnedDeviceId>> {
    // New events save trusted provenance in the same transaction as their JSON,
    // before /sync can see them and before the route records idempotency completion.
    let metadata = event_datas::table
        .filter(event_datas::event_id.eq(event_id))
        .filter(event_datas::room_id.eq(room_id))
        .select(event_datas::internal_metadata)
        .first::<Option<serde_json::Value>>(&mut connect().await?)
        .await
        .optional()?
        .flatten();
    if let Some(metadata) = metadata
        && metadata
            .get("transaction_user")
            .and_then(serde_json::Value::as_str)
            == Some(user_id.as_str())
        && metadata
            .get("transaction_id")
            .and_then(serde_json::Value::as_str)
            == Some(txn_id.as_str())
        && let Some(device) = metadata
            .get("transaction_device")
            .and_then(serde_json::Value::as_str)
    {
        return Ok(Some(device.into()));
    }
    // Existing events predate the provenance field.
    event_idempotents::table
        .filter(event_idempotents::txn_id.eq(txn_id))
        .filter(event_idempotents::user_id.eq(user_id))
        .filter(event_idempotents::room_id.eq(room_id))
        .filter(event_idempotents::event_id.eq(event_id))
        .select(event_idempotents::device_id)
        .first::<Option<OwnedDeviceId>>(&mut connect().await?)
        .await
        .optional()
        .map(|v| v.flatten())
        .map_err(Into::into)
}
