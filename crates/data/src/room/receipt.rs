use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel_async::AsyncPgConnection;
use horme::{OptionalExtension as _, RunQueryDsl};

use crate::DataResult;
use crate::core::events::receipt::{Receipt, ReceiptEventContent, ReceiptThread, ReceiptType};
use crate::core::identifiers::*;
use crate::core::{Seqnum, UnixMillis};
use crate::room::DbReceipt;
use crate::schema::*;

const RECEIPT_STREAM_LOCK: i64 = 0x50414c52435054;

/// Wait for receipt publications immediately before capturing a sync token,
/// after the other stream locks. Writers acquire summary locks before this fence.
pub async fn lock_receipt_snapshot(conn: &mut AsyncPgConnection) -> DataResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
        .bind::<diesel::sql_types::BigInt, _>(RECEIPT_STREAM_LOCK)
        .execute(conn)
        .await?;
    Ok(())
}

/// Publish inside the caller's transaction, retaining the shared stream lock
/// through commit. Acquire any summary locks first, and no other stream locks
/// afterward. Concurrent writers are permitted; snapshots wait for every writer.
/// The supplied `sn` is ignored.
pub async fn publish_receipt_with_conn(
    conn: &mut AsyncPgConnection,
    receipt: &DbReceipt,
) -> DataResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind::<diesel::sql_types::BigInt, _>(RECEIPT_STREAM_LOCK)
        .execute(conn)
        .await?;
    let mut receipt = receipt.clone();
    receipt.sn = diesel::select(diesel::dsl::sql::<diesel::sql_types::BigInt>(
        "nextval('occur_sn_seq')",
    ))
    .get_result(conn)
    .await?;
    diesel::insert_into(event_receipts::table)
        .values(&receipt)
        .execute(conn)
        .await?;
    Ok(())
}

/// Returns an iterator over the most recent read_receipts in a room that happened after the event
/// with id `since`.
pub async fn read_receipts(
    room_id: &RoomId,
    since_sn: Seqnum,
    viewer: Option<&UserId>,
) -> DataResult<Vec<(OwnedUserId, ReceiptEventContent)>> {
    // Keep the append-only log for federation cursors, but project only each
    // changed receipt key's latest value in SQL. Do not materialize its history.
    let thread =
        diesel::dsl::sql::<diesel::sql_types::Text>("COALESCE(json_data->>'thread_id', '')");
    let mut rows = event_receipts::table
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
    // Consumers which must collapse receipts into a single wire map (MSC3960)
    // resolve unrepresentable same-target collisions by stream recency, never
    // by the thread key's alphabetical position in DISTINCT ON ordering.
    rows.sort_by_key(|row| row.sn);
    // Separate events preserve different thread receipts for the same event,
    // type and user: a single nested receipt map has only one value at that key.
    let mut receipts = Vec::with_capacity(rows.len());
    for item in rows {
        let content = ReceiptEventContent(BTreeMap::from([(
            item.event_id,
            BTreeMap::from([(
                ReceiptType::from(item.ty),
                BTreeMap::from([(
                    item.user_id.clone(),
                    serde_json::from_value(item.json_data).unwrap_or_default(),
                )]),
            )]),
        )]));
        receipts.push((item.user_id, content));
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
    set_private_read_receipt(
        room_id,
        user_id,
        event_id,
        event_sn,
        &Receipt::new(UnixMillis::now()),
    )
    .await
}

/// Persist the original private receipt, including its independent thread scope.
pub async fn set_private_read_receipt(
    room_id: &RoomId,
    user_id: &UserId,
    event_id: &EventId,
    event_sn: Seqnum,
    receipt: &Receipt,
) -> DataResult<()> {
    insert_receipt(&DbReceipt {
        sn: 0,
        ty: ReceiptType::ReadPrivate.to_string(),
        room_id: room_id.to_owned(),
        user_id: user_id.to_owned(),
        event_id: event_id.to_owned(),
        event_sn,
        thread_id: match &receipt.thread {
            ReceiptThread::Thread(id) => Some(id.clone()),
            _ => None,
        },
        json_data: serde_json::to_value(receipt)?,
        receipt_at: receipt.ts.unwrap_or_else(UnixMillis::now),
    })
    .await
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

/// Publish a fully-built receipt row, allocating its position inside the transaction.
pub async fn insert_receipt(receipt: &DbReceipt) -> DataResult<()> {
    crate::db()
        .transaction::<_, crate::DataError, _>(async |conn| {
            publish_receipt_with_conn(conn, receipt).await
        })
        .await
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

pub async fn last_private_read_receipt(
    user_id: &UserId,
    room_id: &RoomId,
) -> DataResult<Option<DbReceipt>> {
    event_receipts::table
        .filter(event_receipts::room_id.eq(room_id))
        .filter(event_receipts::user_id.eq(user_id))
        .filter(event_receipts::ty.eq(ReceiptType::ReadPrivate.to_string()))
        .order_by(event_receipts::sn.desc())
        .first::<DbReceipt>(&mut crate::db())
        .await
        .optional()
        .map_err(Into::into)
}
