use std::collections::BTreeMap;

use salvo::oapi::extract::JsonBody;
use salvo::prelude::*;

use crate::core::UnixMillis;
use crate::core::events::RoomAccountDataEventType;
use crate::core::events::fully_read::{FullyReadEvent, FullyReadEventContent};
use crate::core::events::receipt::{
    CreateReceiptReqBody, Receipt, ReceiptEvent, ReceiptEventContent, ReceiptThread, ReceiptType,
    SendReceiptReqArgs,
};
use crate::core::presence::PresenceState;
use crate::room::push_action;
use crate::{AppError, AuthArgs, DepotExt, EmptyResult, empty_ok, room};

/// #POST /_matrix/client/r0/rooms/{room_id}/receipt/{receipt_type}/{event_id}
/// Sets private read marker and public read receipt EDU.
#[endpoint]
pub(super) async fn send_receipt(
    _aa: AuthArgs,
    args: SendReceiptReqArgs,
    body: JsonBody<CreateReceiptReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let body = body.into_inner();
    let thread_id = match &body.thread {
        ReceiptThread::Thread(id) => Some(&**id),
        _ => None,
    };

    crate::user::ping_presence(sender_id, &PresenceState::Online).await?;
    let event_sn = crate::event::get_event_sn(&args.event_id).await?;
    match args.receipt_type {
        ReceiptType::FullyRead => {
            let fully_read_event = FullyReadEvent {
                content: FullyReadEventContent {
                    event_id: args.event_id.clone(),
                },
            };
            crate::user::set_data(
                sender_id,
                Some(args.room_id.clone()),
                &RoomAccountDataEventType::FullyRead.to_string(),
                serde_json::to_value(fully_read_event.content).expect("to json value always works"),
            )
            .await?;
            push_action::remove_actions_for_room(sender_id, &args.room_id).await?;
        }
        ReceiptType::Read => {
            let mut user_receipts = BTreeMap::new();
            user_receipts.insert(
                sender_id.to_owned(),
                Receipt {
                    ts: Some(UnixMillis::now()),
                    thread: body.thread.clone(),
                },
            );
            let mut receipts = BTreeMap::new();
            receipts.insert(ReceiptType::Read, user_receipts);

            let mut receipt_content = BTreeMap::new();
            receipt_content.insert(args.event_id.clone(), receipts);

            room::receipt::update_read(
                sender_id,
                &args.room_id,
                &ReceiptEvent {
                    content: ReceiptEventContent(receipt_content),
                    room_id: args.room_id.clone(),
                },
                true,
            )
            .await?;
            push_action::remove_actions_until(sender_id, &args.room_id, event_sn, thread_id)
                .await?;
        }
        ReceiptType::ReadPrivate => {
            // let count = timeline::get_event_sn(&args.event_id)?
            //     .ok_or(MatrixError::invalid_param("Event does not exist."))?;
            crate::data::room::receipt::set_private_read(
                &args.room_id,
                sender_id,
                &args.event_id,
                event_sn,
            )
            .await?;
            push_action::remove_actions_until(sender_id, &args.room_id, event_sn, thread_id)
                .await?;
        }
        _ => return Err(AppError::internal("unsupported receipt type")),
    }
    if matches!(
        &args.receipt_type,
        ReceiptType::Read | ReceiptType::ReadPrivate
    ) {
        push_action::refresh_notify_summary(sender_id, &args.room_id).await?;
    }
    empty_ok()
}
