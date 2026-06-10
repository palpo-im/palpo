use std::collections::BTreeMap;

use crate::core::UnixMillis;
use crate::core::events::receipt::{
    Receipt, ReceiptContent, ReceiptData, ReceiptEvent, ReceiptEventContent, ReceiptMap, Receipts,
};
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::data::next_sn;
use crate::data::room::DbReceipt;
use crate::{AppResult, MatrixError, data, sending};

/// Replaces the previous read receipt.
#[tracing::instrument]
pub async fn update_read(
    user_id: &UserId,
    room_id: &RoomId,
    event: &ReceiptEvent,
    broadcast: bool,
) -> AppResult<()> {
    for (event_id, receipts) in event.content.clone() {
        let Ok(event_sn) = crate::event::get_event_sn(&event_id).await else {
            continue;
        };
        for (receipt_ty, user_receipts) in receipts {
            if let Some(receipt) = user_receipts.get(user_id) {
                let thread_id = match &receipt.thread {
                    crate::core::events::receipt::ReceiptThread::Thread(id) => Some(id.clone()),
                    _ => None,
                };
                let receipt_at = receipt.ts.unwrap_or_else(UnixMillis::now);
                let receipt = DbReceipt {
                    sn: next_sn().await?,
                    ty: receipt_ty.to_string(),
                    room_id: room_id.to_owned(),
                    user_id: user_id.to_owned(),
                    event_id: event_id.clone(),
                    event_sn,
                    thread_id,
                    json_data: serde_json::to_value(receipt)?,
                    receipt_at,
                };
                if let Err(e) = data::room::receipt::insert_receipt(&receipt).await {
                    error!("failed to insert receipt: {}", e);
                }
            }
        }
    }

    let receipts = BTreeMap::from_iter([(
        room_id.to_owned(),
        ReceiptMap::new(BTreeMap::from_iter([(
            user_id.to_owned(),
            ReceiptData::new(
                Receipt::new(UnixMillis::now()),
                event.content.0.keys().cloned().collect(),
            ),
        )])),
    )]);
    let edu = Edu::Receipt(ReceiptContent::new(receipts));
    if broadcast {
        sending::send_edu_room(room_id, &edu).await?;
    }
    Ok(())
}

/// Gets the latest private read receipt from the user in the room
pub async fn last_private_read(
    user_id: &UserId,
    room_id: &RoomId,
) -> AppResult<ReceiptEventContent> {
    let Some(event_id) = data::room::receipt::last_private_read_event_id(user_id, room_id).await?
    else {
        return Err(MatrixError::not_found("No private read receipt.").into());
    };

    // let room_sn = crate::room::get_room_sn(room_id)
    //     .map_err(|e| MatrixError::bad_state(format!("room does not exist in database for
    // {room_id}: {e}")))?;

    // let pdu = timeline::get_pdu(&event_id)?;

    let user_id: OwnedUserId = user_id.to_owned();
    let content: BTreeMap<OwnedEventId, Receipts> = BTreeMap::from_iter([(
        event_id,
        BTreeMap::from_iter([(
            crate::core::events::receipt::ReceiptType::ReadPrivate,
            BTreeMap::from_iter([(
                user_id,
                crate::core::events::receipt::Receipt {
                    ts: None, // TODO: start storing the timestamp so we can return one
                    thread: crate::core::events::receipt::ReceiptThread::Unthreaded,
                },
            )]),
        )]),
    )]);
    Ok(ReceiptEventContent(content))
}
