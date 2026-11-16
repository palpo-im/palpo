use std::collections::BTreeMap;

use diesel::prelude::*;
use horme::RunQueryDsl;

use crate::core::UnixMillis;
use crate::core::events::receipt::{
    ReceiptContent, ReceiptData, ReceiptEvent, ReceiptEventContent, ReceiptMap, ReceiptThread,
    ReceiptType,
};
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::data::diesel_exists;
use crate::data::room::DbReceipt;
use crate::data::schema::{events, threads};
use crate::{AppResult, MatrixError, data, room, sending};

/// Resolve the notification/receipt scope once from a bounded relation chain.
/// Roots stay on the main timeline. Missing or cross-room ancestry does not
/// prove a thread; transient storage errors are still returned to the caller.
pub(crate) async fn event_thread_root(
    event: &crate::event::PduEvent,
) -> AppResult<Option<OwnedEventId>> {
    related_thread_root(event, None).await
}

/// A receipt may target a root through non-thread relationships as well. Such
/// events retain their notification classification on the main timeline.
async fn related_thread_root(
    event: &crate::event::PduEvent,
    receipt_root: Option<&EventId>,
) -> AppResult<Option<OwnedEventId>> {
    if receipt_root == Some(&event.event_id)
        && is_known_thread_root(&event.room_id, &event.event_id).await?
    {
        return Ok(Some(event.event_id.clone()));
    }
    let mut related = event.clone();
    let mut visited = std::collections::HashSet::new();
    for _ in 0..3 {
        if !visited.insert(related.event_id.clone()) {
            return Ok(None);
        }
        let content: serde_json::Value = related.get_content()?;
        let Some(relation) = content.get("m.relates_to") else {
            return Ok(None);
        };
        let Some(target) = relation
            .get("event_id")
            .or_else(|| {
                relation
                    .get("m.in_reply_to")
                    .and_then(|reply| reply.get("event_id"))
            })
            .and_then(|id| id.as_str())
        else {
            return Ok(None);
        };
        let Ok(target) = EventId::parse(target) else {
            return Ok(None);
        };
        if receipt_root == Some(&target) && is_known_thread_root(&event.room_id, &target).await? {
            return Ok(Some(target));
        }
        if relation.get("rel_type").and_then(|ty| ty.as_str()) == Some("m.thread") {
            let root = match data::room::DbEvent::get_by_id(&target).await {
                Ok(root) => root,
                Err(error) if error.is_not_found() => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            return Ok((root.room_id == event.room_id
                && !root.is_outlier
                && !root.soft_failed
                && !root.is_rejected)
                .then_some(target));
        }
        related = match room::timeline::get_pdu(&target).await {
            Ok(pdu) if pdu.room_id == event.room_id && !pdu.is_outlier && !pdu.soft_failed => {
                pdu.pdu
            }
            Ok(_) => return Ok(None),
            Err(error) if error.is_not_found() => return Ok(None),
            Err(error) => return Err(error),
        };
    }
    Ok(None)
}

/// A related intermediary is not necessarily a thread root. The thread index
/// proves a published thread child, while the join rejects stale invalid roots.
async fn is_known_thread_root(room_id: &RoomId, event_id: &EventId) -> AppResult<bool> {
    diesel_exists!(
        threads::table
            .inner_join(events::table.on(events::id.eq(threads::event_id)))
            .filter(threads::room_id.eq(room_id))
            .filter(threads::event_id.eq(event_id))
            .filter(events::room_id.eq(room_id))
            .filter(events::is_outlier.eq(false))
            .filter(events::soft_failed.eq(false))
            .filter(events::is_rejected.eq(false)),
        &mut data::db()
    )
    .map_err(Into::into)
}

/// Validate every target before any client receipt, marker or federation write.
pub async fn validate_client_event(
    user_id: &UserId,
    room_id: &RoomId,
    event_id: &EventId,
    thread: Option<&ReceiptThread>,
) -> AppResult<crate::core::Seqnum> {
    if !room::user::is_joined(user_id, room_id).await? {
        return Err(MatrixError::forbidden("You are not joined to this room.", None).into());
    }
    let pdu = match room::timeline::get_pdu(event_id).await {
        Ok(pdu) => pdu,
        Err(error) if error.is_not_found() => {
            return Err(MatrixError::not_found("Receipt target was not found.").into());
        }
        Err(error) => return Err(error),
    };
    if pdu.room_id != room_id
        || pdu.is_outlier
        || pdu.soft_failed
        || !pdu.user_can_see(user_id).await?
    {
        return Err(MatrixError::forbidden(
            "You cannot acknowledge this event in this room.",
            None,
        )
        .into());
    }
    if let Some(thread) = thread {
        match thread {
            ReceiptThread::Unthreaded => {}
            ReceiptThread::Main | ReceiptThread::Thread(_) => {
                if let ReceiptThread::Thread(root) = thread {
                    match data::room::DbEvent::get_by_id(root).await {
                        Ok(root) if root.room_id == room_id => {}
                        Ok(_) => {
                            return Err(MatrixError::invalid_param(
                                "Thread belongs to another room.",
                            )
                            .into());
                        }
                        Err(error) if error.is_not_found() => {
                            return Err(
                                MatrixError::invalid_param("Unknown receipt thread ID.").into()
                            );
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                let valid = match thread {
                    ReceiptThread::Main => event_thread_root(&pdu).await?.is_none(),
                    ReceiptThread::Thread(root) => {
                        related_thread_root(&pdu, Some(root)).await?.as_deref() == Some(&**root)
                    }
                    _ => unreachable!(),
                };
                if !valid {
                    return Err(MatrixError::invalid_param(
                        "Event does not belong to the receipt thread.",
                    )
                    .into());
                }
            }
            _ => return Err(MatrixError::invalid_param("Invalid receipt thread ID.").into()),
        }
    }
    Ok(pdu.event_sn)
}

/// Commit the receipt and notification bookkeeping together before external
/// delivery. A failed gateway enqueue may be retried without leaving a local
/// read watermark ahead of stale notification actions.
pub async fn record_client_receipt(
    user_id: &UserId,
    room_id: &RoomId,
    event_id: &EventId,
    event_sn: crate::core::Seqnum,
    receipt_type: ReceiptType,
    receipt: &crate::core::events::receipt::Receipt,
) -> AppResult<()> {
    if !matches!(receipt_type, ReceiptType::Read | ReceiptType::ReadPrivate) {
        return Err(MatrixError::invalid_param("Unsupported read receipt type.").into());
    }
    let row = DbReceipt {
        sn: 0,
        ty: receipt_type.to_string(),
        room_id: room_id.to_owned(),
        user_id: user_id.to_owned(),
        event_id: event_id.to_owned(),
        event_sn,
        thread_id: match &receipt.thread {
            ReceiptThread::Thread(root) => Some(root.clone()),
            _ => None,
        },
        json_data: serde_json::to_value(receipt)?,
        receipt_at: receipt.ts.unwrap_or_else(UnixMillis::now),
    };
    room::push_action::record_read_and_clear(&row, &receipt.thread).await?;
    if receipt_type == ReceiptType::Read {
        let edu = Edu::Receipt(ReceiptContent::new(BTreeMap::from([(
            room_id.to_owned(),
            ReceiptMap::new(BTreeMap::from([(
                user_id.to_owned(),
                ReceiptData::new(receipt.clone(), vec![event_id.to_owned()]),
            )])),
        )])));
        sending::send_edu_room(room_id, &edu).await?;
    }
    Ok(())
}

/// Replaces the previous read receipt.
#[tracing::instrument]
pub async fn update_read(
    user_id: &UserId,
    room_id: &RoomId,
    event: &ReceiptEvent,
    broadcast: bool,
) -> AppResult<()> {
    let mut broadcasts = Vec::new();
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
                    sn: 0,
                    ty: receipt_ty.to_string(),
                    room_id: room_id.to_owned(),
                    user_id: user_id.to_owned(),
                    event_id: event_id.clone(),
                    event_sn,
                    thread_id,
                    json_data: serde_json::to_value(receipt)?,
                    receipt_at,
                };
                data::room::receipt::insert_receipt(&receipt).await?;
                if broadcast && receipt_ty == ReceiptType::Read {
                    // The federation wire map has one value per (room,user).
                    // Keep each original scope in its own EDU rather than inventing
                    // an unthreaded receipt or overwriting another scope.
                    broadcasts.push(Edu::Receipt(ReceiptContent::new(BTreeMap::from([(
                        room_id.to_owned(),
                        ReceiptMap::new(BTreeMap::from([(
                            user_id.to_owned(),
                            ReceiptData::new(
                                serde_json::from_value(receipt.json_data)?,
                                vec![event_id.clone()],
                            ),
                        )])),
                    )]))));
                }
            }
        }
    }

    for edu in broadcasts {
        sending::send_edu_room(room_id, &edu).await?;
    }
    Ok(())
}

/// Gets the latest private read receipt from the user in the room
pub async fn last_private_read(
    user_id: &UserId,
    room_id: &RoomId,
) -> AppResult<ReceiptEventContent> {
    let Some(receipt) = data::room::receipt::last_private_read_receipt(user_id, room_id).await?
    else {
        return Err(MatrixError::not_found("No private read receipt.").into());
    };

    let content = BTreeMap::from([(
        receipt.event_id,
        BTreeMap::from([(
            ReceiptType::ReadPrivate,
            BTreeMap::from([(
                user_id.to_owned(),
                serde_json::from_value(receipt.json_data).unwrap_or_default(),
            )]),
        )]),
    )]);
    Ok(ReceiptEventContent(content))
}
