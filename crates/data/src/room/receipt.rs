use std::collections::BTreeMap;

use diesel::prelude::*;
use horme::{OptionalExtension as _, RunQueryDsl};

use crate::core::events::receipt::{ReceiptEventContent, ReceiptType};
use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::core::{Seqnum, UnixMillis};
use crate::room::DbReceipt;
use crate::schema::*;
use crate::{DataResult, next_sn};

/// Returns an iterator over the most recent read_receipts in a room that happened after the event
/// with id `since`.
pub async fn read_receipts(
    room_id: &RoomId,
    since_sn: Seqnum,
    viewer: Option<&UserId>,
) -> DataResult<BTreeMap<OwnedUserId, ReceiptEventContent>> {
    // Keep the append-only log for federation cursors, but project only each
    // changed receipt key's latest value in SQL. Do not materialize its history.
    let thread =
        diesel::dsl::sql::<diesel::sql_types::Text>("COALESCE(json_data->>'thread_id', '')");
    let rows = event_receipts::table
        .filter(event_receipts::sn.ge(since_sn))
        .filter(event_receipts::room_id.eq(room_id))
        .filter(
            event_receipts::ty
                .ne(ReceiptType::ReadPrivate.to_string())
                .or(event_receipts::user_id.nullable().eq(viewer)),
        )
        .distinct_on((event_receipts::user_id, event_receipts::ty, thread.clone()))
        .order((
            event_receipts::user_id,
            event_receipts::ty,
            thread,
            event_receipts::sn.desc(),
        ))
        .load::<DbReceipt>(&mut crate::db())
        .await?;
    let mut receipts: BTreeMap<OwnedUserId, ReceiptEventContent> = BTreeMap::new();
    for item in rows {
        receipts
            .entry(item.user_id.clone())
            .or_default()
            .0
            .entry(item.event_id)
            .or_default()
            .insert(
                ReceiptType::from(item.ty),
                BTreeMap::from([(
                    item.user_id,
                    serde_json::from_value(item.json_data).unwrap_or_default(),
                )]),
            );
    }

    Ok(receipts)
}

/// Sets a private read marker at `count`.
#[tracing::instrument]
pub async fn set_private_read(
    room_id: &RoomId,
    user_id: &UserId,
    event_id: &EventId,
    event_sn: Seqnum,
) -> DataResult<()> {
    diesel::insert_into(event_receipts::table)
        .values(&DbReceipt {
            sn: next_sn().await?,
            ty: ReceiptType::ReadPrivate.to_string(),
            room_id: room_id.to_owned(),
            user_id: user_id.to_owned(),
            event_id: event_id.to_owned(),
            event_sn,
            thread_id: None,
            json_data: JsonValue::default(),
            receipt_at: UnixMillis::now(),
        })
        .execute(&mut crate::db())
        .await?;
    Ok(())
}

pub async fn last_private_read_update_sn(user_id: &UserId, room_id: &RoomId) -> DataResult<Seqnum> {
    let event_sn = event_receipts::table
        .filter(event_receipts::room_id.eq(room_id))
        .filter(event_receipts::user_id.eq(user_id))
        .filter(event_receipts::ty.eq(ReceiptType::ReadPrivate.to_string()))
        .order_by(event_receipts::event_sn.desc())
        .select(event_receipts::event_sn)
        .first::<Seqnum>(&mut crate::db())
        .await?;

    Ok(event_sn)
}

/// Insert a fully-built receipt row.
pub async fn insert_receipt(receipt: &DbReceipt) -> DataResult<()> {
    diesel::insert_into(event_receipts::table)
        .values(receipt)
        .execute(&mut crate::db())
        .await?;
    Ok(())
}

/// Event id of the user's most recent private read receipt in a room.
pub async fn last_private_read_event_id(
    user_id: &UserId,
    room_id: &RoomId,
) -> DataResult<Option<OwnedEventId>> {
    event_receipts::table
        .filter(event_receipts::room_id.eq(room_id))
        .filter(event_receipts::user_id.eq(user_id))
        .filter(event_receipts::ty.eq(ReceiptType::ReadPrivate.to_string()))
        .order_by(event_receipts::sn.desc())
        .select(event_receipts::event_id)
        .first::<OwnedEventId>(&mut crate::db())
        .await
        .optional()
        .map_err(Into::into)
}
