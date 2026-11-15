use salvo::oapi::extract::JsonBody;
use salvo::prelude::*;

use crate::core::UnixMillis;
use crate::core::events::RoomAccountDataEventType;
use crate::core::events::fully_read::{FullyReadEvent, FullyReadEventContent};
use crate::core::events::receipt::{
    CreateReceiptReqBody, Receipt, ReceiptThread, ReceiptType, SendReceiptReqArgs,
};
use crate::core::presence::PresenceState;
use crate::{AuthArgs, DepotExt, EmptyResult, MatrixError, empty_ok, room};

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
    if !matches!(
        args.receipt_type,
        ReceiptType::Read | ReceiptType::ReadPrivate | ReceiptType::FullyRead
    ) {
        return Err(MatrixError::invalid_param("Unsupported receipt type.").into());
    }
    if args.receipt_type == ReceiptType::FullyRead && body.thread != ReceiptThread::Unthreaded {
        return Err(
            MatrixError::invalid_param("Fully-read markers cannot have a thread ID.").into(),
        );
    }
    let event_sn = room::receipt::validate_client_event(
        sender_id,
        &args.room_id,
        &args.event_id,
        Some(&body.thread),
    )
    .await?;
    crate::user::ping_presence(sender_id, &PresenceState::Online).await?;
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
        }
        ReceiptType::Read | ReceiptType::ReadPrivate => {
            room::receipt::record_client_receipt(
                sender_id,
                &args.room_id,
                &args.event_id,
                event_sn,
                args.receipt_type,
                &Receipt {
                    ts: Some(UnixMillis::now()),
                    thread: body.thread.clone(),
                },
            )
            .await?;
        }
        _ => unreachable!("receipt type was validated before any writes"),
    }
    empty_ok()
}

#[cfg(test)]
#[path = "receipt_review_tests.rs"]
mod review_tests;
