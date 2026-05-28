use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use palpo_core::UnixMillis;

use crate::AppResult;
use crate::core::identifiers::*;
use crate::core::{DeviceId, TransactionId, UserId};
use crate::data::room::NewDbEventIdempotent;
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};

pub async fn add_txn_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
    event_id: Option<&EventId>,
) -> AppResult<()> {
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

pub async fn txn_id_exists(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
) -> AppResult<bool> {
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

pub async fn get_event_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
) -> AppResult<Option<OwnedEventId>> {
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
