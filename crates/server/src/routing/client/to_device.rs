use std::collections::BTreeMap;

use salvo::oapi::extract::*;
use salvo::prelude::*;
use ulid::Ulid;

use crate::core::device::DirectDeviceContent;
use crate::core::federation::transaction::Edu;
use crate::core::to_device::{
    DeviceIdOrAllDevices, SendEventToDeviceReqArgs, SendEventToDeviceReqBody,
};
use crate::{AuthArgs, DepotExt, EmptyResult, IsRemoteOrLocal, MatrixError, data, empty_ok};

pub fn authed_router() -> Router {
    Router::with_path("sendToDevice/{event_type}/{txn_id}").put(send_to_device)
}

/// #PUT /_matrix/client/r0/sendToDevice/{event_type}/{txn_id}
/// Send a to-device event to a set of client devices.
#[endpoint]
async fn send_to_device(
    _aa: AuthArgs,
    args: SendEventToDeviceReqArgs,
    body: JsonBody<SendEventToDeviceReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    // Check if this is a new transaction id
    if crate::transaction_id::txn_id_exists(
        &args.txn_id,
        authed.user_id(),
        Some(authed.device_id()),
    )
    .await?
    {
        return empty_ok();
    }

    for (target_user_id, map) in &body.messages {
        for (target_device_id_maybe, event) in map {
            if target_user_id.server_name().is_remote() {
                let mut map = BTreeMap::new();
                map.insert(target_device_id_maybe.clone(), event.clone());
                let mut messages = BTreeMap::new();
                messages.insert(target_user_id.clone(), map);

                let message_id = Ulid::generate();
                crate::sending::send_reliable_edu(
                    target_user_id.server_name(),
                    &Edu::DirectToDevice(DirectDeviceContent {
                        sender: authed.user_id().to_owned(),
                        ev_type: args.event_type.clone(),
                        message_id: message_id.to_string().into(),
                        messages,
                    }),
                    &message_id.to_string(),
                )
                .await?;

                continue;
            }

            match target_device_id_maybe {
                DeviceIdOrAllDevices::DeviceId(target_device_id) => {
                    data::user::device::add_to_device_event(
                        authed.user_id(),
                        target_user_id,
                        target_device_id,
                        &args.event_type.to_string(),
                        event
                            .deserialize_as()
                            .map_err(|_| MatrixError::invalid_param("Event is invalid"))?,
                    )
                    .await?
                }

                DeviceIdOrAllDevices::AllDevices => {
                    for target_device_id in data::user::all_device_ids(target_user_id).await? {
                        data::user::device::add_to_device_event(
                            authed.user_id(),
                            target_user_id,
                            &target_device_id,
                            &args.event_type.to_string(),
                            event
                                .deserialize_as()
                                .map_err(|_| MatrixError::invalid_param("Event is invalid"))?,
                        )
                        .await?;
                    }
                }
            }
        }
    }

    // Save transaction id with empty data
    crate::transaction_id::add_txn_id(
        &args.txn_id,
        authed.user_id(),
        Some(authed.device_id()),
        None,
        None,
    )
    .await?;

    empty_ok()
}

#[endpoint]
pub(super) async fn for_dehydrated(_aa: AuthArgs) -> EmptyResult {
    Err(
        MatrixError::unrecognized("To-device messages for dehydrated devices are not implemented.")
            .into(),
    )
}
