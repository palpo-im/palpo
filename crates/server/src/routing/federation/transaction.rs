use std::collections::BTreeMap;
use std::time::Instant;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::UnixMillis;
use crate::core::device::{DeviceListUpdateContent, DirectDeviceContent};
use crate::core::events::receipt::{
    ReceiptContent, ReceiptEvent, ReceiptEventContent, ReceiptType,
};
use crate::core::events::typing::TypingContent;
use crate::core::federation::transaction::{
    Edu, SendMessageReqBody, SendMessageResBody, SigningKeyUpdateContent,
};
use crate::core::identifiers::*;
use crate::core::presence::PresenceContent;
use crate::core::serde::RawJsonValue;
use crate::core::to_device::DeviceIdOrAllDevices;
use crate::data::user::NewDbPresence;
use crate::event::handler;
use crate::sending::{EDU_LIMIT, PDU_LIMIT};
use crate::{AppError, AppResult, DepotExt, JsonResult, MatrixError, data, json_ok, room};

pub fn router() -> Router {
    Router::with_path("send/{txn_id}").put(send_message)
}

/// #PUT /_matrix/federation/v1/send/{txn_id}
/// Push EDUs and PDUs to this server.
#[endpoint]
async fn send_message(
    depot: &mut Depot,
    _txn_id: PathParam<OwnedTransactionId>,
    body: JsonBody<SendMessageReqBody>,
) -> JsonResult<SendMessageResBody> {
    let origin = depot.origin()?;
    let body = body.into_inner();
    if &body.origin != origin {
        return Err(MatrixError::forbidden(
            "not allowed to send transactions on behalf of other servers",
            None,
        )
        .into());
    }

    if body.pdus.len() > PDU_LIMIT {
        return Err(MatrixError::forbidden(
            "not allowed to send more than {PDU_LIMIT} PDUs in one transaction",
            None,
        )
        .into());
    }

    if body.edus.len() > EDU_LIMIT {
        return Err(MatrixError::forbidden(
            "not allowed to send more than {EDU_LIMIT} EDUs in one transaction",
            None,
        )
        .into());
    }

    let txn_start_time = Instant::now();
    let resolved_map = process_pdus(&body.pdus, &body.origin, &txn_start_time).await?;

    process_edus(body.edus, &body.origin).await;

    json_ok(SendMessageResBody {
        pdus: resolved_map
            .into_iter()
            .map(|(e, r)| (e, r.map_err(|e| e.to_string())))
            .collect(),
    })
}

async fn process_pdus(
    pdus: &[Box<RawJsonValue>],
    origin: &ServerName,
    txn_start_time: &Instant,
) -> AppResult<BTreeMap<OwnedEventId, AppResult<()>>> {
    let mut parsed_pdus = Vec::with_capacity(pdus.len());
    for pdu in pdus {
        parsed_pdus.push(match crate::parse_incoming_pdu(pdu) {
            Ok(t) => t,
            Err(e) => {
                warn!("could not parse PDU: {e}");
                continue;
            }
        });

        // We do not add the event_id field to the pdu here because of signature
        // and hashes checks
    }
    let mut resolved_map = BTreeMap::new();
    for (event_id, value, room_id, room_version_id) in parsed_pdus {
        // crate::server::check_running()?;
        let pdu_start_time = Instant::now();
        let result = handler::process_incoming_pdu(
            origin,
            &event_id,
            &room_id,
            &room_version_id,
            value,
            true,
        )
        .await;
        debug!(
            pdu_elapsed = ?pdu_start_time.elapsed(),
            txn_elapsed = ?txn_start_time.elapsed(),
            "Finished PDU {event_id}",
        );

        // if result.is_ok() {
        //     resolved_map.insert(event_id, Ok(()));
        // }
        resolved_map.insert(event_id, result);
    }

    for (id, result) in &resolved_map {
        if let Err(e) = result
            && matches!(e, AppError::Matrix(_))
        {
            warn!("incoming PDU failed {id}: {e:?}");
        }
    }

    Ok(resolved_map)
}

async fn process_edus(edus: Vec<Edu>, origin: &ServerName) {
    for edu in edus {
        match edu {
            Edu::Presence(presence) => process_edu_presence(origin, presence).await,
            Edu::Receipt(receipt) => process_edu_receipt(origin, receipt).await,
            Edu::Typing(typing) => process_edu_typing(origin, typing).await,
            Edu::DeviceListUpdate(content) => process_edu_device_list_update(origin, content).await,
            Edu::DirectToDevice(content) => process_edu_direct_to_device(origin, content).await,
            Edu::SigningKeyUpdate(content) => process_edu_signing_key_update(origin, content).await,
            Edu::_Custom(ref _custom) => {
                warn!("received custom/unknown EDU");
            }
        }
    }
}

async fn process_edu_presence(origin: &ServerName, presence: PresenceContent) {
    if !crate::config::get().presence.allow_incoming {
        return;
    }

    for update in presence.push {
        if update.user_id.server_name() != origin {
            warn!(
                %update.user_id, %origin,
                "received presence EDU for user not belonging to origin"
            );
            continue;
        }

        crate::data::user::set_presence(
            NewDbPresence {
                user_id: update.user_id.clone(),
                stream_id: None,
                state: Some(update.presence.to_string()),
                last_active_at: Some(UnixMillis::now()),
                last_federation_update_at: None,
                last_user_sync_at: None,
                currently_active: None,
                occur_sn: None,
                status_msg: update.status_msg.clone(),
            },
            true,
        )
        .ok();
    }
}

async fn process_edu_receipt(origin: &ServerName, receipt: ReceiptContent) {
    // if !crate::config::get().allow_incoming_read_receipts() {
    // 	return;
    // }

    for (room_id, room_updates) in receipt {
        if handler::acl_check(origin, &room_id).is_err() {
            warn!(
                %origin, %room_id,
                "received read receipt EDU from ACL'd server"
            );
            continue;
        }

        for (user_id, user_updates) in room_updates.read {
            if user_id.server_name() != origin {
                warn!(
                    %user_id, %origin,
                    "received read receipt EDU for user not belonging to origin"
                );
                continue;
            }

            if room::joined_users(&room_id, None)
                .unwrap_or_default()
                .iter()
                .any(|member| member.server_name() == user_id.server_name())
            {
                for event_id in &user_updates.event_ids {
                    let user_receipts =
                        BTreeMap::from([(user_id.clone(), user_updates.data.clone())]);
                    let receipts = BTreeMap::from([(ReceiptType::Read, user_receipts)]);
                    let receipt_content = BTreeMap::from([(event_id.to_owned(), receipts)]);
                    let event = ReceiptEvent {
                        content: ReceiptEventContent(receipt_content),
                        room_id: room_id.clone(),
                    };

                    let _ = room::receipt::update_read(&user_id, &room_id, &event);
                }
            } else {
                warn!(
                    %user_id, %room_id, %origin,
                    "received read receipt EDU from server who does not have a member in the room",
                );
                continue;
            }
        }
    }
}

async fn process_edu_typing(origin: &ServerName, typing: TypingContent) {
    // if !crate::config::get().allow_incoming_typing {
    //     return;
    // }

    if typing.user_id.server_name() != origin {
        warn!(
            %typing.user_id, %origin,
            "received typing EDU for user not belonging to origin"
        );
        return;
    }

    if handler::acl_check(typing.user_id.server_name(), &typing.room_id).is_err() {
        warn!(
            %typing.user_id, %typing.room_id, %origin,
            "received typing EDU for ACL'd user's server"
        );
        return;
    }

    if room::user::is_joined(&typing.user_id, &typing.room_id).unwrap_or(false) {
        if typing.typing {
            let timeout = UnixMillis::now().get().saturating_add(
                crate::config::get()
                    .typing
                    .federation_timeout
                    .saturating_mul(1000),
            );
            let _ = room::typing::add_typing(&typing.user_id, &typing.room_id, timeout).await;
        } else {
            let _ = room::typing::remove_typing(&typing.user_id, &typing.room_id).await;
        }
    } else {
        warn!(
            %typing.user_id, %typing.room_id, %origin,
            "received typing EDU for user not in room"
        );
    }
}

async fn process_edu_device_list_update(origin: &ServerName, content: DeviceListUpdateContent) {
    let DeviceListUpdateContent {
        user_id, device_id, ..
    } = content;

    if user_id.server_name() != origin {
        warn!(
            %user_id, %origin,
            "received device list update EDU for user not belonging to origin"
        );
        return;
    }

    let _ = crate::user::mark_device_key_update(&user_id, &device_id);
}

async fn process_edu_direct_to_device(origin: &ServerName, content: DirectDeviceContent) {
    let DirectDeviceContent {
        sender,
        ev_type,
        message_id,
        messages,
    } = content;

    if sender.server_name() != origin {
        warn!(
            %sender, %origin,
            "received direct to device EDU for user not belonging to origin"
        );
        return;
    }

    // Check if this is a new transaction id
    if crate::transaction_id::txn_id_exists(&message_id, &sender, None).unwrap_or_default() {
        return;
    }

    for (target_user_id, map) in &messages {
        for (target_device_id_maybe, event) in map {
            let Ok(event) = event
                .deserialize_as()
                .map_err(|e| error!("To-Device event is invalid: {e}"))
            else {
                continue;
            };

            let ev_type = ev_type.to_string();
            match target_device_id_maybe {
                DeviceIdOrAllDevices::DeviceId(target_device_id) => {
                    let _ = data::user::device::add_to_device_event(
                        &sender,
                        target_user_id,
                        target_device_id,
                        &ev_type,
                        event,
                    );
                }

                DeviceIdOrAllDevices::AllDevices => {
                    let (sender, ev_type, event) = (&sender, &ev_type, &event);
                    data::user::all_device_ids(target_user_id)
                        .unwrap_or_default()
                        .iter()
                        .for_each(|target_device_id| {
                            let _ = data::user::device::add_to_device_event(
                                sender,
                                target_user_id,
                                target_device_id,
                                ev_type,
                                event.clone(),
                            );
                        });
                }
            }
        }
    }

    // Save transaction id with empty data
    let _ = crate::transaction_id::add_txn_id(&message_id, &sender, None, None, None);
}

async fn process_edu_signing_key_update(origin: &ServerName, content: SigningKeyUpdateContent) {
    let SigningKeyUpdateContent {
        user_id,
        master_key,
        self_signing_key,
    } = content;

    if user_id.server_name() != origin {
        warn!(
            %user_id, %origin,
            "received signing key update EDU from server that does not belong to user's server"
        );
        return;
    }

    if let Some(master_key) = master_key {
        let _ = crate::user::add_cross_signing_keys(
            &user_id,
            &master_key,
            &self_signing_key,
            &None,
            true,
        );
    }
}
