use std::collections::HashSet;

use salvo::http::StatusError;

use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::federation::membership::{MakeLeaveResBody, SendLeaveReqBody, make_leave_request};
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonObject, CanonicalJsonValue, to_raw_json_value};
use crate::core::{Seqnum, UnixMillis};
use crate::data::room::{DbEventData, NewDbEvent};
use crate::event::{PduBuilder, ensure_event_sn};
use crate::membership::federation::membership::{SendLeaveReqArgsV2, send_leave_request_v2};
use crate::room::{self, state, timeline};
use crate::{
    AppError, AppResult, GetUrlOrigin, MatrixError, PduEvent, SnPduEvent, config, data, membership,
};

// Make a user leave all their joined rooms
pub async fn leave_all_rooms(user_id: &UserId) -> AppResult<()> {
    let all_room_ids = data::user::joined_rooms(user_id)?
        .into_iter()
        .chain(
            data::user::invited_rooms(user_id, 0)?
                .into_iter()
                .map(|t| t.0),
        )
        .collect::<Vec<_>>();
    for room_id in all_room_ids {
        leave_room(user_id, &room_id, None).await.ok();
    }
    Ok(())
}

pub async fn leave_room(
    user_id: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
) -> AppResult<()> {
    // Ask a remote server if we don't have this room
    let conf = config::get();

    if room::is_server_joined(&conf.server_name, room_id)? {
        // If only this server in room, leave locally.
        if let Err(e) = leave_room_local(user_id, room_id, reason.clone()).await {
            warn!("failed to leave room {} locally: {}", user_id, e);
        } else {
            return Ok(());
        }
    }
    match leave_room_remote(user_id, room_id).await {
        Ok((event_id, event_sn)) => {
            let last_state = state::get_user_state(user_id, room_id)?;

            // We always drop the invite, we can't rely on other servers
            membership::update_membership(
                &event_id,
                event_sn,
                room_id,
                user_id,
                MembershipState::Leave,
                user_id,
                last_state,
            )?;
            Ok(())
        }
        Err(e) => {
            warn!("failed to leave room {} remotely: {}", user_id, e);
            if !room::has_any_other_server(room_id, &conf.server_name)? {
                leave_room_local(user_id, room_id, reason).await?;
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

async fn leave_room_local(
    user_id: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
) -> AppResult<(OwnedEventId, Seqnum)> {
    let member_event =
        room::get_state(room_id, &StateEventType::RoomMember, user_id.as_str(), None)?;
    let mut event_content = member_event
        .get_content::<RoomMemberEventContent>()
        .map_err(|_| AppError::public("invalid member event in database"))?;

    let just_invited = event_content.membership == MembershipState::Invite;
    event_content.membership = MembershipState::Leave;
    event_content.reason = reason;
    event_content.join_authorized_via_users_server = None;

    let pdu = timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_json_value(&event_content).expect("event is valid, we just created it"),
            state_key: Some(user_id.to_string()),
            ..Default::default()
        },
        user_id,
        room_id,
        &crate::room::get_version(room_id)?,
        &room::lock_state(room_id).await,
    )
    .await?;
    if just_invited && member_event.sender.server_name() != config::server_name() {
        let _ = crate::sending::send_pdu_room(
            room_id,
            &pdu.event_id,
            &[member_event.sender.server_name().to_owned()],
            &[],
        );
    } else {
        let _ = crate::sending::send_pdu_room(room_id, &pdu.event_id, &[], &[]);
    }
    Ok((pdu.event_id.clone(), pdu.event_sn))
}

async fn leave_room_remote(
    user_id: &UserId,
    room_id: &RoomId,
) -> AppResult<(OwnedEventId, Seqnum)> {
    let mut make_leave_response_and_server =
        Err(AppError::public("no server available to assist in leaving"));
    let invite_state = state::get_user_state(user_id, room_id)?
        .ok_or(MatrixError::bad_state("user is not invited"))?;

    let servers: HashSet<_> = invite_state
        .iter()
        .filter_map(|event| serde_json::from_str(event.as_str()).ok())
        .filter_map(|event: serde_json::Value| event.get("sender").cloned())
        .filter_map(|sender| sender.as_str().map(|s| s.to_owned()))
        .filter_map(|sender| UserId::parse(sender).ok())
        .map(|user| user.server_name().to_owned())
        .collect();

    for remote_server in servers {
        let request = make_leave_request(
            &room_id
                .server_name()
                .map_err(AppError::internal)?
                .origin()
                .await,
            room_id,
            user_id,
        )?
        .into_inner();
        let make_leave_response = crate::sending::send_federation_request(
            room_id.server_name().map_err(AppError::internal)?,
            request,
            None,
        )
        .await?
        .json::<MakeLeaveResBody>()
        .await;

        make_leave_response_and_server = make_leave_response
            .map(|r| (r, remote_server))
            .map_err(Into::into);

        if make_leave_response_and_server.is_ok() {
            break;
        }
    }

    let (make_leave_response, remote_server) = make_leave_response_and_server?;

    let room_version_id = match make_leave_response.room_version {
        Some(version) if config::supported_room_versions().contains(&version) => version,
        _ => return Err(AppError::public("room version is not supported")),
    };

    let mut leave_event_stub =
        serde_json::from_str::<CanonicalJsonObject>(make_leave_response.event.get())
            .map_err(|_| AppError::public("invalid make_leave event json received from server"))?;

    // TODO: Is origin needed?
    leave_event_stub.insert(
        "origin".to_owned(),
        CanonicalJsonValue::String(config::get().server_name.as_str().to_owned()),
    );
    if !leave_event_stub.contains_key("origin_server_ts") {
        leave_event_stub.insert(
            "origin_server_ts".to_owned(),
            CanonicalJsonValue::Integer(UnixMillis::now().get() as i64),
        );
    }
    // We don't leave the event id in the pdu because that's only allowed in v1 or v2 rooms
    leave_event_stub.remove("event_id");

    // In order to create a compatible ref hash (EventID) the `hashes` field needs to be present
    crate::server_key::hash_and_sign_event(&mut leave_event_stub, &room_version_id)
        .expect("event is valid, we just created it");

    // Generate event id
    let event_id = crate::event::gen_event_id(&leave_event_stub, &room_version_id)?;

    // Add event_id back
    leave_event_stub.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.as_str().to_owned()),
    );

    let (event_sn, event_guard) = ensure_event_sn(room_id, &event_id)?;
    NewDbEvent {
        id: event_id.to_owned(),
        sn: event_sn,
        ty: MembershipState::Leave.to_string(),
        room_id: room_id.to_owned(),
        unrecognized_keys: None,
        depth: 0,
        topological_ordering: 0,
        stream_ordering: event_sn,
        origin_server_ts: UnixMillis::now(),
        received_at: None,
        sender_id: Some(user_id.to_owned()),
        contains_url: false,
        worker_id: None,
        state_key: Some(user_id.to_string()),
        is_outlier: false,
        soft_failed: false,
        is_rejected: false,
        rejection_reason: None,
    }
    .save()?;

    DbEventData {
        event_id: event_id.clone(),
        event_sn,
        room_id: room_id.to_owned(),
        internal_metadata: None,
        json_data: serde_json::to_value(&leave_event_stub)?,
        format_version: None,
    }
    .save()?;
    let parsed_leave_pdu =
        PduEvent::from_canonical_object(room_id, &event_id, leave_event_stub.clone()).map_err(
            |e| {
                StatusError::internal_server_error()
                    .brief(format!("invalid leave event PDU: {e:?}"))
            },
        )?;
    let leave_pdu = SnPduEvent {
        pdu: parsed_leave_pdu,
        event_sn,
        is_outlier: false,
        soft_failed: false,
        is_backfill: false,
    };
    timeline::append_pdu(
        &leave_pdu,
        leave_event_stub.clone(),
        &room::lock_state(room_id).await,
    )
    .await?;
    drop(event_guard);

    // It has enough fields to be called a proper event now
    let leave_event = leave_event_stub;
    let request = send_leave_request_v2(
        &remote_server.origin().await,
        SendLeaveReqArgsV2 {
            room_id: room_id.to_owned(),
            event_id: event_id.clone(),
        },
        SendLeaveReqBody(crate::sending::convert_to_outgoing_federation_event(
            leave_event.clone(),
        )),
    )?
    .into_inner();
    crate::sending::send_federation_request(&remote_server, request, None).await?;

    Ok((event_id, event_sn))
}
