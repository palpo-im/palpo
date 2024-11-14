use std::collections::BTreeMap;

use diesel::prelude::*;
use palpo_core::JsonValue;

use crate::core::events::receipt::{ReceiptEvent, ReceiptEventContent, Receipt,ReceiptType};
use crate::core::events::{AnySyncEphemeralRoomEvent, SyncEphemeralRoomEvent};
use crate::core::identifiers::*;
use crate::core::serde::RawJson;
use crate::core::UnixMillis;
use crate::schema::*;
use crate::{db, AppResult};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = event_receipts)]
pub struct DbReceipt {
    pub id: i64,

    pub room_id: OwnedRoomId,
    pub receipt_type: String,
    pub user_id: OwnedUserId,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub json_data: JsonValue,
    pub receipt_at: UnixMillis,
}
#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = event_receipts)]
pub struct NewDbReceipt {
    pub room_id: OwnedRoomId,
    pub receipt_type: String,
    pub user_id: OwnedUserId,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub json_data: JsonValue,
    pub receipt_at: UnixMillis,
}

/// Replaces the previous read receipt.
#[tracing::instrument]
pub fn update_read(user_id: &UserId, room_id: &RoomId, event: ReceiptEvent) -> AppResult<()> {
    println!(
        "============update_read  user_id: {:?}, room_id: {:?}, event: {:#?}",
        user_id, room_id, event
    );
    for (event_id, receipts) in event.content {
        if let Ok(event_sn) = crate::event::get_event_sn(&event_id) {
            for (receipt_type, user_receipts) in receipts {
                if let Some(receipt) = user_receipts.get(user_id) {
                    println!(
                        "====receipt_type: {:?}, event_id:{event_id:?} user_receipts: {:#?}",
                        receipt_type, user_receipts
                    );
                    let receipt_at = receipt.ts.unwrap_or_else(|| UnixMillis::now());
                    let receipt = NewDbReceipt {
                        room_id: room_id.to_owned(),
                        receipt_type: receipt_type.to_string(),
                        user_id: user_id.to_owned(),
                        event_id: event_id.clone(),
                        event_sn,
                        json_data: serde_json::to_value(receipt)?,
                        receipt_at,
                    };
                    diesel::insert_into(event_receipts::table)
                        .values(&receipt)
                        .on_conflict((
                            event_receipts::room_id,
                            event_receipts::receipt_type,
                            event_receipts::user_id,
                        ))
                        .do_update()
                        .set(&receipt)
                        .execute(&mut *db::connect()?)?;
                }
            }
        }
    }
    Ok(())
}

/// Returns an iterator over the most recent read_receipts in a room that happened after the event with id `since`.
pub fn read_receipts(room_id: &RoomId, event_sn: i64) -> AppResult<SyncEphemeralRoomEvent<ReceiptEventContent>> {
    let mut event_content: BTreeMap<OwnedEventId, BTreeMap<ReceiptType, BTreeMap<OwnedUserId, Receipt>>> = BTreeMap::new();
    let receipts = event_receipts::table
        .filter(event_receipts::room_id.eq(room_id))
        .filter(event_receipts::event_sn.ge(event_sn))
        .load::<DbReceipt>(&mut *db::connect()?)?;
    for receipt in receipts {
        let DbReceipt {
            user_id,
            event_id,
            receipt_type,
            json_data,
            ..
        } = receipt;
        let mut event_map     = event_content.entry(event_id).or_default();
        let receipt_type = ReceiptType::from(receipt_type);
        let mut type_map = event_map.entry(receipt_type).or_default();
        type_map.insert(user_id, serde_json::from_value(json_data).unwrap_or_default());
    }

    Ok(SyncEphemeralRoomEvent {
        content: ReceiptEventContent(event_content),
    })
}
/// Sets a private read marker at `count`.
#[tracing::instrument]
pub fn set_private_read(room_id: &RoomId, user_id: &UserId, event_id: &EventId, event_sn: i64) -> AppResult<()> {
    diesel::insert_into(event_receipts::table)
        .values(&NewDbReceipt {
            room_id: room_id.to_owned(),
            user_id: user_id.to_owned(),
            event_id: event_id.to_owned(),
            event_sn,
            receipt_type: ReceiptType::ReadPrivate.to_string(),
            json_data: JsonValue::default(),
            receipt_at: UnixMillis::now(),
        })
        .on_conflict_do_nothing()
        .execute(&mut db::connect()?)?;
    Ok(())
}

/// Returns the private read marker.
pub fn get_private_read(room_id: &RoomId, user_id: &UserId) -> AppResult<u64> {
    let count: i64 = event_receipts::table
        .filter(event_receipts::room_id.eq(room_id))
        .filter(event_receipts::user_id.eq(user_id))
        .filter(event_receipts::receipt_type.eq(ReceiptType::ReadPrivate.to_string()))
        .count()
        .get_result(&mut *db::connect()?)?;
    Ok(count as u64)
}

// /// Returns the count of the last typing update in this room.
// #[tracing::instrument]
// pub fn update_last_private_read(user_id: &UserId, room_id: &RoomId) -> AppResult<u64> {
//     let mut key = room_id.as_bytes().to_vec();
//     key.push(0xff);
//     key.extend_from_slice(user_id.as_bytes());

//     Ok(self
//         .roomuser_id_lastprivatereadupdate
//         .get(&key)?
//         .map(|bytes| {
//             utils::u64_from_bytes(&bytes)
//                 .map_err(|_| AppError::public("Count in roomuser_id_lastprivatereadupdate is invalid."))
//         })
//         .transpose()?
//         .unwrap_or(0))
// }
