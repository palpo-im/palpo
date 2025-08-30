use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap};
use std::iter::once;
use std::sync::Arc;

use diesel::prelude::*;
use indexmap::IndexMap;
use palpo_core::serde::JsonValue;
use palpo_data::user::DbUser;
use salvo::http::StatusError;
use tokio::sync::RwLock;

use crate::appservice::RegistrationInfo;
use crate::core::UnixMillis;
use crate::core::client::membership::{JoinRoomResBody, ThirdPartySigned};
use crate::core::device::DeviceListUpdateContent;
use crate::core::events::TimelineEventType;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::federation::membership::{
    MakeJoinReqArgs, MakeJoinResBody, SendJoinArgs, SendJoinReqBody, SendJoinResBodyV2,
};
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::core::serde::{
    CanonicalJsonObject, CanonicalJsonValue, to_canonical_value, to_raw_json_value,
};
use crate::data::room::{DbEventData, NewDbEvent};
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::event::handler::{fetch_and_process_missing_prev_events, process_incoming_pdu};
use crate::event::{PduBuilder, PduEvent, ensure_event_sn, gen_event_id_canonical_json};
use crate::federation::maybe_strip_event_id;
use crate::room::state::{CompressedEvent, DeltaInfo};
use crate::room::{state, timeline};
use crate::sending::send_edu_server;
use crate::{
    AppError, AppResult, GetUrlOrigin, IsRemoteOrLocal, MatrixError, OptionalExtension, SnPduEvent,
    config, data, room,
};

pub async fn join_room(
    sender: &DbUser,
    device_id: Option<&DeviceId>,
    room_id: &RoomId,
    reason: Option<String>,
    servers: &[OwnedServerName],
    _third_party_signed: Option<&ThirdPartySigned>,
    appservice: Option<&RegistrationInfo>,
    extra_data: BTreeMap<String, JsonValue>,
) -> AppResult<JoinRoomResBody> {
    if sender.is_guest && appservice.is_none() && !room::guest_can_join(room_id) {
        return Err(
            MatrixError::forbidden("Guests are not allowed to join this room", None).into(),
        );
    }
    let sender_id = &sender.id;
    if room::user::is_joined(sender_id, room_id)? {
        return Ok(JoinRoomResBody {
            room_id: room_id.into(),
        });
    }

    if let Ok(membership) = room::get_member(room_id, sender_id)
        && membership.membership == MembershipState::Ban
    {
        tracing::warn!(
            "{} is banned from {room_id} but attempted to join",
            sender_id
        );
        return Err(MatrixError::forbidden("You are banned from the room.", None).into());
    }

    // Ask a remote server if we are not participating in this room
    let (should_remote, servers) =
        room::should_join_on_remote_servers(sender_id, room_id, servers).await?;

    if !should_remote {
        info!("We can join locally");
        let join_rule = room::get_join_rule(room_id)?;

        let event = RoomMemberEventContent {
            membership: MembershipState::Join,
            display_name: data::user::display_name(sender_id).ok().flatten(),
            avatar_url: data::user::avatar_url(sender_id).ok().flatten(),
            is_direct: None,
            third_party_invite: None,
            blurhash: data::user::blurhash(sender_id).ok().flatten(),
            reason: reason.clone(),
            join_authorized_via_users_server: get_first_user_can_issue_invite(
                room_id,
                sender_id,
                &join_rule.restriction_rooms(),
            )
            .await
            .ok(),
            extra_data: extra_data.clone(),
        };
        match timeline::build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomMember,
                content: to_raw_json_value(&event).expect("event is valid, we just created it"),
                state_key: Some(sender_id.to_string()),
                ..Default::default()
            },
            sender_id,
            room_id,
            &room::lock_state(room_id).await,
        )
        .await
        {
            Ok(_) => {
                if let Some(device_id) = device_id {
                    crate::user::mark_device_key_update_with_joined_rooms(
                        sender_id,
                        device_id,
                        &[room_id.to_owned()],
                    )?;
                }
                return Ok(JoinRoomResBody::new(room_id.to_owned()));
            }
            Err(e) => {
                tracing::error!("Failed to append join event locally: {e}");
                if servers.is_empty() || servers.iter().all(|s| s.is_local()) {
                    return Err(e);
                }
            }
        }
    }

    info!("joining {room_id} over federation");

    let (make_join_response, remote_server) =
        make_join_request(sender_id, room_id, &servers).await?;

    info!("make_join finished");

    let room_version_id = match make_join_response.room_version {
        Some(room_version) if config::supported_room_versions().contains(&room_version) => {
            room_version
        }
        _ => return Err(AppError::public("Room version is not supported")),
    };

    let mut join_event_stub: CanonicalJsonObject =
        serde_json::from_str(make_join_response.event.get())
            .map_err(|_| AppError::public("Invalid make_join event json received from server."))?;

    let join_authorized_via_users_server = join_event_stub
        .get("content")
        .map(|s| {
            s.as_object()?
                .get("join_authorised_via_users_server")?
                .as_str()
        })
        .and_then(|s| OwnedUserId::try_from(s.unwrap_or_default()).ok());

    // TODO: Is origin needed?
    join_event_stub.insert(
        "origin".to_owned(),
        CanonicalJsonValue::String(config::get().server_name.as_str().to_owned()),
    );
    join_event_stub.insert(
        "origin_server_ts".to_owned(),
        CanonicalJsonValue::Integer(UnixMillis::now().get() as i64),
    );
    join_event_stub.insert(
        "content".to_owned(),
        to_canonical_value(RoomMemberEventContent {
            membership: MembershipState::Join,
            display_name: data::user::display_name(sender_id)?,
            avatar_url: data::user::avatar_url(sender_id)?,
            is_direct: None,
            third_party_invite: None,
            blurhash: data::user::blurhash(sender_id)?,
            reason,
            join_authorized_via_users_server,
            extra_data: extra_data.clone(),
        })
        .expect("event is valid, we just created it"),
    );

    // We keep the "event_id" in the pdu only in v1 or v2 rooms
    maybe_strip_event_id(&mut join_event_stub, &room_version_id);

    // In order to create a compatible ref hash (EventID) the `hashes` field needs to be present
    crate::server_key::hash_and_sign_event(&mut join_event_stub, &room_version_id)
        .expect("event is valid, we just created it");

    // Generate event id
    let event_id = crate::event::gen_event_id(&join_event_stub, &room_version_id)?;

    // Add event_id back
    join_event_stub.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.as_str().to_owned()),
    );

    // It has enough fields to be called a proper event now
    let mut join_event = join_event_stub;
    let body = SendJoinReqBody(crate::sending::convert_to_outgoing_federation_event(
        join_event.clone(),
    ));
    info!("Asking {remote_server} for send_join");
    let send_join_request = crate::core::federation::membership::send_join_request(
        &remote_server.origin().await,
        SendJoinArgs {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
            omit_members: false,
        },
        body,
    )?
    .into_inner();

    let send_join_body = crate::sending::send_federation_request(&remote_server, send_join_request)
        .await?
        .json::<SendJoinResBodyV2>()
        .await?;

    info!("send_join finished");

    if let Some(signed_raw) = &send_join_body.0.event {
        info!(
            "There is a signed event. This room is probably using restricted joins. Adding signature to our event"
        );
        let (signed_event_id, signed_value) =
            match gen_event_id_canonical_json(signed_raw, &room_version_id) {
                Ok(t) => t,
                Err(_) => {
                    // Event could not be converted to canonical json
                    return Err(MatrixError::invalid_param(
                        "Could not convert event to canonical json.",
                    )
                    .into());
                }
            };

        if signed_event_id != event_id {
            return Err(MatrixError::invalid_param("Server sent event with wrong event id").into());
        }

        match signed_value["signatures"]
            .as_object()
            .ok_or(MatrixError::invalid_param(
                "Server sent invalid signatures type",
            ))
            .and_then(|e| {
                e.get(remote_server.as_str())
                    .ok_or(MatrixError::invalid_param(
                        "Server did not send its signature",
                    ))
            }) {
            Ok(signature) => {
                join_event
                    .get_mut("signatures")
                    .expect("we created a valid pdu")
                    .as_object_mut()
                    .expect("we created a valid pdu")
                    .insert(remote_server.to_string(), signature.clone());
            }
            Err(e) => {
                warn!(
                    "Server {remote_server} sent invalid signature in sendjoin signatures for event {signed_value:?}: {e:?}",
                );
            }
        }
    }

    room::ensure_room(room_id, &room_version_id)?;

    info!("Parsing join event");

    let parsed_join_pdu =
        PduEvent::from_canonical_object(&event_id, join_event.clone()).map_err(|e| {
            warn!("Invalid PDU in send_join response: {}", e);
            AppError::public("Invalid join event PDU.")
        })?;

    let mut state = HashMap::new();
    let pub_key_map = RwLock::new(BTreeMap::new());

    info!("Acquiring server signing keys for response events");
    let resp_events = &send_join_body.0;
    let resp_state = &resp_events.state;
    let resp_auth = &resp_events.auth_chain;
    crate::server_key::acquire_events_pubkeys(resp_auth.iter().chain(resp_state.iter())).await;

    let mut parsed_pdus = IndexMap::new();
    for auth_pdu in resp_auth {
        let (event_id, event_value, _room_id, _room_version_id) =
            crate::parse_incoming_pdu(auth_pdu)?;
        parsed_pdus.insert(event_id, event_value);
    }
    for state in resp_state {
        let (event_id, event_value, _room_id, _room_version_id) = crate::parse_incoming_pdu(state)?;
        parsed_pdus.insert(event_id, event_value);
    }
    for (event_id, event_value) in parsed_pdus {
        if let Err(e) = process_incoming_pdu(
            &remote_server,
            &event_id,
            room_id,
            &room_version_id,
            event_value,
            true,
        )
        .await
        {
            error!("Failed to fetch missing prev events for join: {e}");
        }
    }
    if let Err(e) = fetch_and_process_missing_prev_events(
        &remote_server,
        room_id,
        &room_version_id,
        &parsed_join_pdu,
        &mut Default::default(),
    )
    .await
    {
        error!("Failed to fetch missing prev events for join: {e}");
    }

    info!("Going through send_join response room_state");
    for result in send_join_body
        .0
        .state
        .iter()
        .map(|pdu| super::validate_and_add_event_id(pdu, &room_version_id, &pub_key_map))
    {
        let (event_id, value) = match result.await {
            Ok(t) => t,
            Err(_) => continue,
        };

        let pdu = if let Some(pdu) = timeline::get_pdu(&event_id).optional()? {
            pdu
        } else {
            let (event_sn, event_guard) = ensure_event_sn(room_id, &event_id)?;
            let pdu = SnPduEvent::from_canonical_object(&event_id, event_sn, value.clone())
                .map_err(|e| {
                    warn!("Invalid PDU in send_join response: {} {:?}", e, value);
                    AppError::public("Invalid PDU in send_join response.")
                })?;

            NewDbEvent::from_canonical_json(&event_id, event_sn, &value)?.save()?;
            DbEventData {
                event_id: pdu.event_id.to_owned(),
                event_sn,
                room_id: pdu.room_id.clone(),
                internal_metadata: None,
                json_data: serde_json::to_value(&value)?,
                format_version: None,
            }
            .save()?;

            drop(event_guard);
            pdu
        };

        if let Some(state_key) = &pdu.state_key {
            let state_key_id = state::ensure_field_id(&pdu.event_ty.to_string().into(), state_key)?;
            state.insert(state_key_id, (pdu.event_id.clone(), pdu.event_sn));
        }
    }

    info!("Going through send_join response auth_chain");
    for result in send_join_body
        .0
        .auth_chain
        .iter()
        .map(|pdu| super::validate_and_add_event_id(pdu, &room_version_id, &pub_key_map))
    {
        let (event_id, value) = match result.await {
            Ok(t) => t,
            Err(_) => continue,
        };

        if !timeline::has_pdu(&event_id) {
            let (event_sn, _event_guard) = ensure_event_sn(room_id, &event_id)?;
            NewDbEvent::from_canonical_json(&event_id, event_sn, &value)?.save()?;
            DbEventData {
                event_id: event_id.to_owned(),
                event_sn,
                room_id: room_id.to_owned(),
                internal_metadata: None,
                json_data: serde_json::to_value(&value)?,
                format_version: None,
            }
            .save()?;
        }
    }

    info!("Running send_join auth check");
    // TODO: Authcheck
    // if !event_auth::auth_check(
    //     &RoomVersion::new(&room_version_id)?,
    //     &parsed_join_pdu,
    //     None::<PduEvent>, // TODO: third party invite
    //     |k, s| {
    //         timeline::get_pdu(
    //             state.get(&state::ensure_field_id(&k.to_string().into(), s).ok()?)?,
    //         )
    //         .ok()?
    //     },
    // )
    // .map_err(|e| {
    //     warn!("Auth check failed when running send_json auth check: {e}");
    //     MatrixError::invalid_param("Auth check failed when running send_json auth check")
    // })? {
    //     return Err(MatrixError::invalid_param("Auth check failed when running send_json auth check").into());
    // }

    info!("Saving state from send_join");
    let DeltaInfo {
        frame_id,
        appended,
        disposed,
    } = state::save_state(
        room_id,
        Arc::new(
            state
                .into_iter()
                .map(|(k, (_event_id, event_sn))| Ok(CompressedEvent::new(k, event_sn)))
                .collect::<AppResult<_>>()?,
        ),
    )?;

    state::force_state(room_id, frame_id, appended, disposed)?;

    // info!("Updating joined counts for new room");
    // room::update_joined_servers(room_id)?;
    // room::update_currents(room_id)?;

    let state_lock = room::lock_state(room_id).await;
    info!("Appending new room join event");
    let join_event_id = parsed_join_pdu.event_id.clone();
    let (join_event_sn, event_guard) = ensure_event_sn(room_id, &join_event_id)?;
    diesel::insert_into(events::table)
        .values(NewDbEvent::from_canonical_json(
            &event_id,
            join_event_sn,
            &join_event,
        )?)
        .on_conflict_do_nothing()
        .execute(&mut connect()?)?;

    let join_pdu = SnPduEvent::new(parsed_join_pdu, join_event_sn);
    timeline::append_pdu(
        &join_pdu,
        join_event,
        once(join_event_id.borrow()),
        &state_lock,
    )
    .await?;
    let frame_id_after_join = state::append_to_state(&join_pdu)?;
    drop(event_guard);

    info!("Setting final room state for new room");
    // We set the room state after inserting the pdu, so that we never have a moment in time
    // where events in the current room state do not exist
    state::set_room_state(room_id, frame_id_after_join)?;
    drop(state_lock);

    if let Some(device_id) = device_id {
        let room_server_id = room_id
            .server_name()
            .map_err(|e| AppError::public(format!("bad server name: {e}")))?;
        let query = room_users::table
            .filter(room_users::room_id.ne(room_id))
            .filter(room_users::user_id.eq(sender_id))
            .filter(room_users::room_server_id.eq(room_server_id));
        if !diesel_exists!(query, &mut connect()?)? {
            let content = DeviceListUpdateContent::new(
                sender_id.to_owned(),
                device_id.to_owned(),
                data::next_sn()? as u64,
            );
            let edu = Edu::DeviceListUpdate(content);
            send_edu_server(room_server_id, &edu)?;
        }
    }
    Ok(JoinRoomResBody::new(room_id.to_owned()))
}

pub async fn get_first_user_can_issue_invite(
    room_id: &RoomId,
    invitee_id: &UserId,
    restriction_rooms: &[OwnedRoomId],
) -> AppResult<OwnedUserId> {
    if restriction_rooms.iter().any(|restriction_room_id| {
        room::user::is_joined(invitee_id, restriction_room_id).unwrap_or(false)
    }) {
        for joined_user in room::joined_users(room_id, None)? {
            if joined_user.server_name() == config::get().server_name
                && room::user_can_invite(room_id, &joined_user, invitee_id).await
            {
                return Ok(joined_user);
            }
        }
    }
    Err(MatrixError::not_found("No user can issue invite in this room.").into())
}
pub async fn get_users_can_issue_invite(
    room_id: &RoomId,
    invitee_id: &UserId,
    restriction_rooms: &[OwnedRoomId],
) -> AppResult<Vec<OwnedUserId>> {
    let mut users = vec![];
    if restriction_rooms.iter().any(|restriction_room_id| {
        room::user::is_joined(invitee_id, restriction_room_id).unwrap_or(false)
    }) {
        for joined_user in room::joined_users(room_id, None)? {
            if joined_user.server_name() == config::get().server_name
                && room::user_can_invite(room_id, &joined_user, invitee_id).await
            {
                users.push(joined_user);
            }
        }
    }
    Ok(users)
}

async fn make_join_request(
    user_id: &UserId,
    room_id: &RoomId,
    servers: &[OwnedServerName],
) -> AppResult<(MakeJoinResBody, OwnedServerName)> {
    let mut last_join_error = Err(StatusError::bad_request()
        .brief("No server available to assist in joining.")
        .into());

    for remote_server in servers {
        if remote_server == &config::get().server_name {
            continue;
        }
        info!("Asking {remote_server} for make_join");

        let make_join_request = crate::core::federation::membership::make_join_request(
            &remote_server.origin().await,
            MakeJoinReqArgs {
                room_id: room_id.to_owned(),
                user_id: user_id.to_owned(),
                ver: config::supported_room_versions(),
            },
        )?
        .into_inner();
        let make_join_response =
            crate::sending::send_federation_request(remote_server, make_join_request).await;
        match make_join_response {
            Ok(make_join_response) => {
                let res_body = make_join_response.json::<MakeJoinResBody>().await;
                last_join_error = res_body
                    .map(|r| (r, remote_server.clone()))
                    .map_err(Into::into);
            }
            Err(e) => {
                tracing::error!("make_join_request failed: {e:?}");
                last_join_error = Err(e);
            }
        }

        if last_join_error.is_ok() {
            break;
        }
    }

    last_join_error
}
