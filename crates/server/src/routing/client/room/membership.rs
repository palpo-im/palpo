use std::collections::BTreeMap;

use diesel::prelude::*;
use palpo_core::Seqnum;
use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde_json::value::to_raw_value;

use crate::core::client::membership::MembershipEventFilter;
use crate::core::client::membership::{
    BanUserReqBody, InvitationRecipient, InviteUserReqBody, JoinRoomReqBody, JoinRoomResBody, JoinedMembersResBody,
    JoinedRoomsResBody, KickUserReqBody, LeaveRoomReqBody, MembersReqArgs, MembersResBody, RoomMember,
    UnbanUserReqBody,
};
use crate::core::client::room::{KnockReqArgs, KnockReqBody};
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::federation::query::{ProfileReqArgs, profile_request};
use crate::core::identifiers::*;
use crate::core::user::ProfileResBody;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::DbProfile;
use crate::exts::*;
use crate::membership::{banned_room_check, knock_room_by_id};
use crate::room::{state, timeline};
use crate::sending::send_federation_request;
use crate::{
    AppError, AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, PduBuilder, PduEvent, data, empty_ok, json_ok,
    room, utils,
};

/// #POST /_matrix/client/r0/rooms/{room_id}/members
/// Lists all joined users in a room.
///
/// - Only works if the user is currently joined
#[endpoint]
pub(super) fn get_members(_aa: AuthArgs, args: MembersReqArgs, depot: &mut Depot) -> JsonResult<MembersResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let membership = args.membership.as_ref();
    let not_membership = args.not_membership.as_ref();

    let mut until_sn = if !state::user_can_see_events(sender_id, &args.room_id)? {
        if let Ok(leave_sn) = crate::room::user::leave_sn(sender_id, &args.room_id) {
            Some(leave_sn)
        } else {
            return Err(MatrixError::forbidden("You don't have permission to view this room.", None).into());
        }
    } else {
        None
    };

    let frame_id = if let Some(at_sn) = &args.at {
        if let Ok(at_sn) = at_sn.parse::<Seqnum>() {
            if let Some(usn) = until_sn {
                until_sn = Some(usn.min(at_sn));
            } else {
                until_sn = Some(at_sn);
            }
            event_points::table
                .filter(event_points::room_id.eq(&args.room_id))
                .filter(event_points::event_sn.le(at_sn))
                .filter(event_points::frame_id.is_not_null())
                .order(event_points::frame_id.desc())
                .select(event_points::frame_id)
                .first::<Option<i64>>(&mut connect()?)?
                .unwrap_or_default()
        } else {
            return Err(MatrixError::bad_state("Invalid at parameter.").into());
        }
    } else {
        crate::room::get_frame_id(&args.room_id, until_sn)?
    };
    let states: Vec<_> = state::get_full_state(frame_id)?
        .into_iter()
        .filter(|(key, _)| key.0 == StateEventType::RoomMember)
        .filter_map(|(_, pdu)| membership_filter(pdu, membership, not_membership, until_sn))
        .map(|pdu| pdu.to_member_event())
        .collect();

    json_ok(MembersResBody { chunk: states })
}
fn membership_filter(
    pdu: PduEvent,
    for_membership: Option<&MembershipEventFilter>,
    not_membership: Option<&MembershipEventFilter>,
    until_sn: Option<Seqnum>,
) -> Option<PduEvent> {
    if let Some(until_sn) = until_sn {
        if pdu.event_sn > until_sn {
            return None;
        }
    }
    let membership_state_filter = match for_membership {
        Some(MembershipEventFilter::Ban) => MembershipState::Ban,
        Some(MembershipEventFilter::Invite) => MembershipState::Invite,
        Some(MembershipEventFilter::Knock) => MembershipState::Knock,
        Some(MembershipEventFilter::Leave) => MembershipState::Leave,
        Some(_) | None => MembershipState::Join,
    };

    let not_membership_state_filter = match not_membership {
        Some(MembershipEventFilter::Ban) => MembershipState::Ban,
        Some(MembershipEventFilter::Invite) => MembershipState::Invite,
        Some(MembershipEventFilter::Join) => MembershipState::Join,
        Some(MembershipEventFilter::Knock) => MembershipState::Knock,
        Some(_) | None => MembershipState::Leave,
    };

    let evt_membership = pdu.get_content::<RoomMemberEventContent>().ok()?.membership;

    if for_membership.is_some() && not_membership.is_some() {
        if membership_state_filter != evt_membership || not_membership_state_filter == evt_membership {
            None
        } else {
            Some(pdu)
        }
    } else if for_membership.is_some() && not_membership.is_none() {
        if membership_state_filter != evt_membership {
            None
        } else {
            Some(pdu)
        }
    } else if not_membership.is_some() && for_membership.is_none() {
        if not_membership_state_filter == evt_membership {
            None
        } else {
            Some(pdu)
        }
    } else {
        Some(pdu)
    }
}

/// #POST /_matrix/client/r0/rooms/{room_id}/joined_members
/// Lists all members of a room.
///
/// - The sender user must be in the room
/// - TODO: An appservice just needs a puppet joined
/// https://spec.matrix.org/latest/client-server-api/#knocking-on-rooms
#[endpoint]
pub(super) fn joined_members(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    depot: &mut Depot,
) -> JsonResult<JoinedMembersResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let room_id = room_id.into_inner();

    // let until_sn = if !state::user_can_see_events(sender_id, &room_id)? {
    //     if let Ok(leave_sn) = crate::room::user::leave_sn(sender_id, &room_id) {
    //         Some(leave_sn)
    //     } else {
    //         return Err(MatrixError::forbidden("You don't have permission to view this room.", None).into());
    //     }
    // } else {
    //     None
    // };
    // the sender user must be in the room
    if !state::user_can_see_events(sender_id, &room_id)? {
        return Err(MatrixError::forbidden("You don't have permission to view this room.", None).into());
    }

    let mut joined = BTreeMap::new();
    for user_id in crate::room::get_joined_users(&room_id, None)? {
        if let Some(DbProfile {
            display_name,
            avatar_url,
            ..
        }) = data::user::get_profile(&user_id, None)?
        {
            joined.insert(user_id, RoomMember::new(display_name, avatar_url));
        }
    }

    json_ok(JoinedMembersResBody { joined })
}

/// #POST /_matrix/client/r0/joined_rooms
/// Lists all rooms the user has joined.
#[endpoint]
pub(crate) async fn joined_rooms(_aa: AuthArgs, depot: &mut Depot) -> JsonResult<JoinedRoomsResBody> {
    let authed = depot.authed_info()?;

    json_ok(JoinedRoomsResBody {
        joined_rooms: data::user::joined_rooms(authed.user_id())?,
    })
}

/// #POST /_matrix/client/r0/rooms/{room_id}/forget
/// Forgets about a room.
///
/// - If the sender user currently left the room: Stops sender user from receiving information about the room
///
/// Note: Other devices of the user have no way of knowing the room was forgotten, so this has to
/// be called from every device
#[endpoint]
pub(super) async fn forget_room(_aa: AuthArgs, room_id: PathParam<OwnedRoomId>, depot: &mut Depot) -> EmptyResult {
    let authed = depot.authed_info()?;
    let room_id = room_id.into_inner();

    crate::membership::forget_room(authed.user_id(), &room_id)?;

    empty_ok()
}

/// #POST /_matrix/client/r0/rooms/{room_id}/leave
/// Tries to leave the sender user from a room.
///
/// - This should always work if the user is currently joined.
#[endpoint]
pub(super) async fn leave_room(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<LeaveRoomReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let room_id = room_id.into_inner();
    crate::membership::leave_room(authed.user_id(), &room_id, body.reason.clone()).await?;
    empty_ok()
}

/// #POST /_matrix/client/r0/rooms/{room_id}/join
/// Tries to join the sender user into a room.
///
/// - If the server knowns about this room: creates the join event and does auth rules locally
/// - If the server does not know about the room: asks other servers over federation
#[endpoint]
pub(super) async fn join_room_by_id(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<Option<JoinRoomReqBody>>,
    depot: &mut Depot,
) -> JsonResult<JoinRoomResBody> {
    let authed = depot.authed_info()?;
    let room_id = room_id.into_inner();
    let body = body.into_inner();

    let mut servers = Vec::new(); // There is no body.server_name for /roomId/join
    servers.extend(
        state::get_user_state(authed.user_id(), &room_id)?
            .unwrap_or_default()
            .iter()
            .filter_map(|event| serde_json::from_str(event.inner().get()).ok())
            .filter_map(|event: serde_json::Value| event.get("sender").cloned())
            .filter_map(|sender| sender.as_str().map(|s| s.to_owned()))
            .filter_map(|sender| UserId::parse(sender).ok())
            .map(|user| user.server_name().to_owned()),
    );

    servers.push(room_id.server_name().map_err(AppError::public)?.to_owned());

    println!(
        "==============join_room_by_id===extra data: {:?}",
        body.as_ref().map(|body| body.extra_data.clone())
    );

    crate::membership::join_room(
        authed,
        &room_id,
        body.as_ref().map(|body| body.reason.clone()).flatten(),
        &servers,
        body.as_ref().map(|body| body.third_party_signed.as_ref()).flatten(),
        authed.appservice.as_ref(),
        body.as_ref().map(|body| body.extra_data.clone()).unwrap_or_default(),
    )
    .await?;
    json_ok(JoinRoomResBody { room_id })
}

/// #POST /_matrix/client/r0/rooms/{room_id}/invite
/// Tries to send an invite event into the room.
#[endpoint]
pub(super) async fn invite_user(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<InviteUserReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    let InvitationRecipient::UserId { user_id } = &body.recipient else {
        return Err(MatrixError::not_found("User not found.").into());
    };
    crate::membership::invite_user(
        authed.user_id(),
        user_id,
        &room_id.into_inner(),
        body.reason.clone(),
        false,
    )
    .await?;
    empty_ok()
}

/// #POST /_matrix/client/r0/join/{room_id_or_alias}
/// Tries to join the sender user into a room.
///
/// - If the server knowns about this room: creates the join event and does auth rules locally
/// - If the server does not know about the room: asks other servers over federation
#[endpoint]
pub(crate) async fn join_room_by_id_or_alias(
    _aa: AuthArgs,
    room_id_or_alias: PathParam<OwnedRoomOrAliasId>,
    server_name: QueryParam<Vec<OwnedServerName>, false>,
    via: QueryParam<Vec<OwnedServerName>, false>,
    body: JsonBody<Option<JoinRoomReqBody>>,
    req: &mut Request,
    depot: &mut Depot,
) -> JsonResult<JoinRoomResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let room_id_or_alias = room_id_or_alias.into_inner();
    let body = body.into_inner().unwrap_or_default();
    let remote_addr = req.remote_addr();

    // The servers to attempt to join the room through.
    //
    // One of the servers must be participating in the room.
    //
    // When serializing, this field is mapped to both `server_name` and `via`
    // with identical values.
    //
    // When deserializing, the value is read from `via` if it's not missing or
    // empty and `server_name` otherwise.
    let via = via
        .into_inner()
        .unwrap_or_else(|| server_name.into_inner().unwrap_or_default());

    let (servers, room_id) = match OwnedRoomId::try_from(room_id_or_alias) {
        Ok(room_id) => {
            banned_room_check(
                authed.user_id(),
                Some(&room_id),
                room_id.server_name().ok(),
                remote_addr,
            )
            .await?;
            let mut servers = via;
            servers.extend(crate::room::lookup_servers(&room_id)?);

            let state_servers = state::get_user_state(authed.user_id(), &room_id)?.unwrap_or_default();
            let state_servers = state_servers
                .iter()
                .filter_map(|event| serde_json::from_str(event.inner().get()).ok())
                .filter_map(|event: serde_json::Value| event.get("sender").cloned())
                .filter_map(|sender| sender.as_str().map(|s| s.to_owned()))
                .filter_map(|sender| UserId::parse(sender).ok())
                .map(|user| user.server_name().to_owned());

            servers.extend(state_servers);
            if let Ok(server) = room_id.server_name() {
                servers.push(server.to_owned());
            }

            servers.sort_unstable();
            servers.dedup();
            (servers, room_id)
        }
        Err(room_alias) => {
            let (room_id, mut servers) = crate::room::resolve_alias(&room_alias, Some(via)).await?;
            banned_room_check(sender_id, Some(&room_id), Some(room_alias.server_name()), remote_addr).await?;

            let addl_via_servers = crate::room::lookup_servers(&room_id)?;

            let addl_state_servers = state::get_user_state(authed.user_id(), &room_id)?.unwrap_or_default();

            let mut addl_servers: Vec<_> = addl_state_servers
                .iter()
                .filter_map(|event| serde_json::from_str(event.inner().get()).ok())
                .filter_map(|event: serde_json::Value| event.get("sender").cloned())
                .filter_map(|sender| sender.as_str().map(|s| s.to_owned()))
                .filter_map(|sender| UserId::parse(sender).ok())
                .map(|user| user.server_name().to_owned())
                .chain(addl_via_servers)
                .collect();

            addl_servers.sort_unstable();
            addl_servers.dedup();
            servers.append(&mut addl_servers);

            (servers, room_id)
        }
    };

    let join_room_body = crate::membership::join_room(
        authed,
        &room_id,
        body.reason.clone(),
        &servers,
        body.third_party_signed.as_ref(),
        authed.appservice.as_ref(),
        body.extra_data,
    )
    .await?;

    json_ok(JoinRoomResBody {
        room_id: join_room_body.room_id,
    })
}

/// #POST /_matrix/client/r0/rooms/{room_id}/ban
/// Tries to send a ban event into the room.
#[endpoint]
pub(super) async fn ban_user(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<BanUserReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let room_id = room_id.into_inner();

    let state_lock = room::lock_state(&room_id).await;
    let room_state = room::get_state(&room_id, &StateEventType::RoomMember, body.user_id.as_ref(), None).ok();

    let event = if let Some(room_state) = room_state {
        let event = serde_json::from_str::<RoomMemberEventContent>(room_state.content.get())
            .map_err(|_| AppError::internal("Invalid member event in database."))?;

        // If they are already banned and the reason is unchanged, there isn't any point in sending a new event.
        if event.membership == MembershipState::Ban && event.reason == body.reason {
            return empty_ok();
        }
        RoomMemberEventContent {
            membership: MembershipState::Ban,
            ..event
        }
    } else if body.user_id.is_remote() {
        let profile_request = profile_request(
            &body.user_id.server_name().origin().await,
            ProfileReqArgs {
                user_id: body.user_id.to_owned(),
                field: None,
            },
        )?
        .into_inner();
        let ProfileResBody {
            avatar_url,
            display_name,
            blurhash,
        } = send_federation_request(body.user_id.server_name(), profile_request)
            .await?
            .json()
            .await
            .unwrap_or_default();

        RoomMemberEventContent {
            membership: MembershipState::Ban,
            display_name,
            avatar_url,
            is_direct: None,
            third_party_invite: None,
            blurhash,
            reason: body.reason.clone(),
            join_authorized_via_users_server: None,
            extra_data: Default::default(),
        }
    } else {
        let DbProfile {
            display_name,
            avatar_url,
            blurhash,
            ..
        } = data::user::get_profile(&body.user_id, None)?.ok_or(MatrixError::not_found("User profile not found."))?;
        RoomMemberEventContent {
            membership: MembershipState::Ban,
            display_name,
            avatar_url,
            is_direct: None,
            third_party_invite: None,
            blurhash,
            reason: body.reason.clone(),
            join_authorized_via_users_server: None,
            extra_data: Default::default(),
        }
    };

    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&event).expect("event is valid, we just created it"),
            state_key: Some(body.user_id.to_string()),
            ..Default::default()
        },
        authed.user_id(),
        &room_id,
        &state_lock,
    )?;

    empty_ok()
}

/// #POST /_matrix/client/r0/rooms/{room_id}/unban
/// Tries to send an unban event into the room.
#[endpoint]
pub(super) async fn unban_user(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<UnbanUserReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let room_id = room_id.into_inner();

    let state_lock = room::lock_state(&room_id).await;
    let mut event = room::get_state_content::<RoomMemberEventContent>(
        &room_id,
        &StateEventType::RoomMember,
        body.user_id.as_ref(),
        None,
    )?;

    event.membership = MembershipState::Leave;
    event.reason = body.reason.clone();

    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&event).expect("event is valid, we just created it"),
            state_key: Some(body.user_id.to_string()),
            ..Default::default()
        },
        authed.user_id(),
        &room_id,
        &state_lock,
    )?;

    empty_ok()
}

/// #POST /_matrix/client/r0/rooms/{room_id}/kick
/// Tries to send a kick event into the room.
#[endpoint]
pub(super) async fn kick_user(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<KickUserReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let room_id = room_id.into_inner();

    let state_lock = room::lock_state(&room_id).await;
    let Ok(event) = room::get_member(&room_id, &body.user_id) else {
        return Err(MatrixError::forbidden("Users cannot kick users from a room they are not in", None).into());
    };

    if !matches!(
        event.membership,
        MembershipState::Invite | MembershipState::Knock | MembershipState::Join,
    ) {
        return Err(MatrixError::forbidden(
            format!(
                "Cannot kick a user who is not apart of the room (current membership: {})",
                event.membership
            ),
            None,
        )
        .into());
    }

    timeline::build_and_append_pdu(
        PduBuilder::state(
            body.user_id.to_string(),
            &RoomMemberEventContent {
                membership: MembershipState::Leave,
                reason: body.reason.clone(),
                is_direct: None,
                join_authorized_via_users_server: None,
                third_party_invite: None,
                ..event
            },
        ),
        authed.user_id(),
        &room_id,
        &state_lock,
    )?;

    empty_ok()
}

#[endpoint]
pub(crate) async fn knock_room(
    _aa: AuthArgs,
    args: KnockReqArgs,
    body: JsonBody<KnockReqBody>,
    req: &mut Request,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let (room_id, servers) = match OwnedRoomId::try_from(args.room_id_or_alias) {
        Ok(room_id) => {
            crate::membership::banned_room_check(
                sender_id,
                Some(&room_id),
                room_id.server_name().ok(),
                req.remote_addr(),
            )
            .await?;

            let mut servers = body.via.clone();
            servers.extend(crate::room::lookup_servers(&room_id).unwrap_or_default());
            servers.extend(
                state::get_user_state(sender_id, &room_id)
                    .unwrap_or_default()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|event| event.get_field("sender").ok().flatten())
                    .filter_map(|sender: &str| UserId::parse(sender).ok())
                    .map(|user| user.server_name().to_owned()),
            );

            if let Ok(server) = room_id.server_name() {
                servers.push(server.to_owned());
            }
            servers.dedup();
            utils::shuffle(&mut servers);

            (room_id, servers)
        }
        Err(room_alias) => {
            let (room_id, mut servers) = crate::room::resolve_alias(&room_alias, Some(body.via.clone())).await?;

            banned_room_check(
                sender_id,
                Some(&room_id),
                Some(room_alias.server_name()),
                req.remote_addr(),
            )
            .await?;

            let addl_via_servers = crate::room::lookup_servers(&room_id)?;
            let addl_state_servers = state::get_user_state(sender_id, &room_id)?.unwrap_or_default();

            let mut addl_servers: Vec<_> = addl_state_servers
                .iter()
                .filter_map(|event| serde_json::from_str(event.inner().get()).ok())
                .filter_map(|event: serde_json::Value| event.get("sender").cloned())
                .filter_map(|sender| sender.as_str().map(|s| s.to_owned()))
                .filter_map(|sender| UserId::parse(sender).ok())
                .map(|user| user.server_name().to_owned())
                .chain(addl_via_servers)
                .collect();

            addl_servers.sort_unstable();
            addl_servers.dedup();
            utils::shuffle(&mut addl_servers);
            servers.append(&mut addl_servers);

            (room_id, servers)
        }
    };

    knock_room_by_id(sender_id, &room_id, body.reason.clone(), &servers).await?;
    empty_ok()
}
