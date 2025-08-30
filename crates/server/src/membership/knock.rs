use std::borrow::Borrow;
use std::collections::HashMap;
use std::iter::once;
use std::sync::Arc;

use salvo::http::StatusError;

use crate::core::UnixMillis;
use crate::core::events::StateEventType;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::federation::event::{EventReqArgs, EventResBody, event_request};
use crate::core::federation::knock::{
    MakeKnockReqArgs, MakeKnockResBody, SendKnockReqArgs, SendKnockReqBody, SendKnockResBody,
    send_knock_request,
};
use crate::core::identifiers::*;
use crate::core::room::JoinRule;
use crate::core::serde::{CanonicalJsonObject, CanonicalJsonValue, to_canonical_value};
use crate::data::room::NewDbEvent;
use crate::event::{PduBuilder, PduEvent, ensure_event_sn, gen_event_id, handler};
use crate::room::state::{CompressedEvent, DeltaInfo};
use crate::room::{self, state, timeline};
use crate::{
    AppError, AppResult, GetUrlOrigin, IsRemoteOrLocal, MatrixError, OptionalExtension, SnPduEvent,
    config,
};

pub async fn knock_room(
    sender_id: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    servers: &[OwnedServerName],
) -> AppResult<()> {
    if room::user::is_invited(sender_id, room_id)? {
        warn!("{sender_id} is already invited in {room_id} but attempted to knock");
        return Err(MatrixError::forbidden(
            "You cannot knock on a room you are already invited/accepted to.",
            None,
        )
        .into());
    }

    if room::user::is_joined(sender_id, room_id)? {
        warn!("{sender_id} is already joined in {room_id} but attempted to knock");
        return Err(MatrixError::forbidden(
            "You cannot knock on a room you are already joined in.",
            None,
        )
        .into());
    }

    if room::user::is_knocked(sender_id, room_id)? {
        warn!("{sender_id} is already knocked in {room_id}");
        return Ok(());
    }

    if let Ok(memeber) = room::get_member(room_id, sender_id)
        && memeber.membership == MembershipState::Ban
    {
        warn!("{sender_id} is banned from {room_id} but attempted to knock");
        return Err(MatrixError::forbidden(
            "You cannot knock on a room you are banned from.",
            None,
        )
        .into());
    }

    if room::is_server_joined(&config::get().server_name, room_id).unwrap_or(false) {
        use RoomVersionId::*;
        info!("We can knock locally");
        let room_version_id = room::get_version(room_id)?;
        if matches!(room_version_id, V1 | V2 | V3 | V4 | V5 | V6) {
            return Err(MatrixError::forbidden(
                "This room version does not support knocking.",
                None,
            )
            .into());
        }

        let join_rule = room::get_join_rule(room_id)?;
        if !matches!(
            join_rule,
            JoinRule::Invite | JoinRule::Knock | JoinRule::KnockRestricted(..)
        ) {
            return Err(
                MatrixError::forbidden("This room does not support knocking.", None).into(),
            );
        }

        let content = RoomMemberEventContent {
            display_name: crate::data::user::display_name(sender_id).ok().flatten(),
            avatar_url: crate::data::user::avatar_url(sender_id).ok().flatten(),
            blurhash: crate::data::user::blurhash(sender_id).ok().flatten(),
            reason: reason.clone(),
            ..RoomMemberEventContent::new(MembershipState::Knock)
        };

        // Try normal knock first
        match timeline::build_and_append_pdu(
            PduBuilder::state(sender_id.to_string(), &content),
            sender_id,
            room_id,
            &room::lock_state(room_id).await,
        )
        .await
        {
            Ok(_) => {
                return Ok(());
            }
            Err(e) => {
                tracing::error!("Failed to knock room {room_id} with conflict error: {e}");
                if servers.is_empty() || servers.iter().all(|s| s.is_local()) {
                    return Err(e);
                }
            }
        }
    }
    info!("Knocking {room_id} over federation.");

    let (make_knock_response, remote_server) =
        make_knock_request(sender_id, room_id, servers).await?;

    info!("make_knock finished");

    let room_version_id = make_knock_response.room_version;

    if !config::supports_room_version(&room_version_id) {
        return Err(StatusError::internal_server_error()
            .brief("Remote room version {room_version_id} is not supported by palpo")
            .into());
    }
    crate::room::ensure_room(room_id, &room_version_id)?;

    let mut knock_event_stub: CanonicalJsonObject =
        serde_json::from_str(make_knock_response.event.get()).map_err(|e| {
            StatusError::internal_server_error().brief(format!(
                "Invalid make_knock event json received from server: {e:?}"
            ))
        })?;

    knock_event_stub.insert(
        "origin".to_owned(),
        CanonicalJsonValue::String(config::get().server_name.as_str().to_owned()),
    );
    knock_event_stub.insert(
        "origin_server_ts".to_owned(),
        CanonicalJsonValue::Integer(UnixMillis::now().get() as i64),
    );
    knock_event_stub.insert(
        "content".to_owned(),
        to_canonical_value(RoomMemberEventContent {
            display_name: crate::data::user::display_name(sender_id).ok().flatten(),
            avatar_url: crate::data::user::avatar_url(sender_id).ok().flatten(),
            blurhash: crate::data::user::blurhash(sender_id).ok().flatten(),
            reason,
            ..RoomMemberEventContent::new(MembershipState::Knock)
        })
        .expect("event is valid, we just created it"),
    );

    // In order to create a compatible ref hash (EventID) the `hashes` field needs
    // to be present
    crate::server_key::hash_and_sign_event(&mut knock_event_stub, &room_version_id)?;

    // Generate event id
    let event_id = gen_event_id(&knock_event_stub, &room_version_id)?;

    // Add event_id
    knock_event_stub.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.clone().into()),
    );

    // It has enough fields to be called a proper event now
    let knock_event = knock_event_stub;

    info!("Asking {remote_server} for send_knock in room {room_id}");
    let send_knock_request = send_knock_request(
        &remote_server.origin().await,
        SendKnockReqArgs {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
        },
        SendKnockReqBody::new(crate::sending::convert_to_outgoing_federation_event(
            knock_event.clone(),
        )),
    )?
    .into_inner();

    let send_knock_body =
        crate::sending::send_federation_request(&remote_server, send_knock_request)
            .await?
            .json::<SendKnockResBody>()
            .await?;

    info!("send_knock finished");

    info!("parsing knock event");
    let parsed_knock_pdu = PduEvent::from_canonical_object(&event_id, knock_event.clone())
        .map_err(|e| {
            StatusError::internal_server_error().brief(format!("Invalid knock event PDU: {e:?}"))
        })?;

    info!("going through send_knock response knock state events");

    // TODO: how to handle this? snpase save this state to unsigned field.
    let knock_state = send_knock_body
        .knock_room_state
        .iter()
        .map(|event| serde_json::from_str::<CanonicalJsonObject>(event.clone().into_inner().get()))
        .filter_map(Result::ok);

    let mut state_map = HashMap::new();

    for value in knock_state {
        let Some(state_key) = value.get("state_key") else {
            warn!("send_knock stripped state event missing state_key: {value:?}");
            continue;
        };
        let Some(event_type) = value.get("type") else {
            warn!("send_knock stripped state event missing event type: {value:?}");
            continue;
        };

        let Ok(_state_key) = serde_json::from_value::<String>(state_key.clone().into()) else {
            warn!("send_knock stripped state event has invalid state_key: {value:?}");
            continue;
        };
        let Ok(_event_type) = serde_json::from_value::<StateEventType>(event_type.clone().into())
        else {
            warn!("send_knock stripped state event has invalid event type: {value:?}");
            continue;
        };

        let pdu = if let Some(pdu) = timeline::get_pdu(&event_id).optional()? {
            pdu
        } else {
            let request =
                event_request(&remote_server.origin().await, EventReqArgs::new(&event_id))?
                    .into_inner();
            let res_body = crate::sending::send_federation_request(&remote_server, request)
                .await?
                .json::<EventResBody>()
                .await?;
            let _ = handler::process_incoming_pdu(
                &remote_server,
                &event_id,
                room_id,
                &room_version_id,
                serde_json::from_str(res_body.pdu.get())?,
                true,
            )
            .await;
            timeline::get_pdu(&event_id)?
        };

        if let Some(state_key) = &pdu.state_key {
            let state_key_id = state::ensure_field_id(&pdu.event_ty.to_string().into(), state_key)?;
            state_map.insert(state_key_id, (pdu.event_id.clone(), pdu.event_sn));
        }
    }

    info!("Appending room knock event locally");
    let event_id = parsed_knock_pdu.event_id.clone();
    let (event_sn, event_guard) = ensure_event_sn(room_id, &event_id)?;
    NewDbEvent {
        id: event_id.to_owned(),
        sn: event_sn,
        ty: MembershipState::Knock.to_string(),
        room_id: room_id.to_owned(),
        unrecognized_keys: None,
        depth: parsed_knock_pdu.depth as i64,
        topological_ordering: parsed_knock_pdu.depth as i64,
        stream_ordering: 0,
        origin_server_ts: UnixMillis::now(),
        received_at: None,
        sender_id: Some(sender_id.to_owned()),
        contains_url: false,
        worker_id: None,
        state_key: Some(sender_id.to_string()),
        is_outlier: true,
        soft_failed: false,
        rejection_reason: None,
    }
    .save()?;
    let knock_pdu = SnPduEvent::new(parsed_knock_pdu, event_sn);
    timeline::append_pdu(
        &knock_pdu,
        knock_event,
        once(event_id.borrow()),
        &room::lock_state(room_id).await,
    )
    .await?;

    info!("Compressing state from send_knock");
    let compressed = state_map
        .into_iter()
        .map(|(k, (_event_id, event_sn))| Ok(CompressedEvent::new(k, event_sn)))
        .collect::<AppResult<_>>()?;

    debug!("Saving compressed state");
    let DeltaInfo {
        frame_id,
        appended,
        disposed,
    } = state::save_state(room_id, Arc::new(compressed))?;

    debug!("Forcing state for new room");
    state::force_state(room_id, frame_id, appended, disposed)?;

    let frame_id = state::append_to_state(&knock_pdu)?;

    info!("Updating membership locally to knock state with provided stripped state events");
    crate::membership::update_membership(
        &event_id,
        knock_pdu.event_sn,
        room_id,
        sender_id,
        MembershipState::Knock,
        sender_id,
        Some(send_knock_body.knock_room_state),
    )?;

    info!("Setting final room state for new room");
    // We set the room state after inserting the pdu, so that we never have a moment
    // in time where events in the current room state do not exist
    let _ = state::set_room_state(room_id, frame_id);

    drop(event_guard);
    Ok(())
}

async fn make_knock_request(
    sender_id: &UserId,
    room_id: &RoomId,
    servers: &[OwnedServerName],
) -> AppResult<(MakeKnockResBody, OwnedServerName)> {
    let mut make_knock_response_and_server = Err(AppError::HttpStatus(
        StatusError::internal_server_error().brief("No server available to assist in knocking."),
    ));

    let mut make_knock_counter: usize = 0;

    for remote_server in servers {
        if remote_server.is_local() {
            continue;
        }

        info!("Asking {remote_server} for make_knock ({make_knock_counter})");

        let request = crate::core::federation::knock::make_knock_request(
            &remote_server.origin().await,
            MakeKnockReqArgs {
                room_id: room_id.to_owned(),
                user_id: sender_id.to_owned(),
                ver: config::supported_room_versions(),
            },
        )?
        .into_inner();

        let make_knock_response = crate::sending::send_federation_request(remote_server, request)
            .await?
            .json::<MakeKnockResBody>()
            .await
            .map_err(Into::into);

        trace!("make_knock response: {make_knock_response:?}");
        make_knock_counter = make_knock_counter.saturating_add(1);

        make_knock_response_and_server = make_knock_response.map(|r| (r, remote_server.clone()));

        if make_knock_response_and_server.is_ok() {
            break;
        }

        if make_knock_counter > 40 {
            warn!(
                "50 servers failed to provide valid make_knock response, assuming no server can \
				 assist in knocking."
            );
            make_knock_response_and_server = Err(StatusError::internal_server_error()
                .brief("No server available to assist in knocking.")
                .into());

            return make_knock_response_and_server;
        }
    }

    make_knock_response_and_server
}
