use std::collections::BTreeMap;

use diesel::prelude::*;
use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde_json::value::to_raw_value;

use crate::bl::exts::*;
use crate::core::client::membership::{
    BanUserReqBody, InvitationRecipient, InviteUserReqBody, JoinRoomReqBody, JoinRoomResBody, JoinedMembersResBody,
    JoinedRoomsResBody, KickUserReqBody, LeaveRoomReqBody, MembersReqArgs, MembersResBody, RoomMember,
    UnbanUserReqBody,
};
use crate::core::client::room::{KnockReqArgs, KnockReqBody};
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::federation::query::{profile_request, ProfileReqArgs};
use crate::core::identifiers::*;
use crate::core::user::ProfileResBody;
use crate::room::state;
use crate::room::state::UserCanSeeEvent;
use crate::schema::*;
use crate::sending::send_federation_request;
use crate::user::DbProfile;
use crate::{
    db, diesel_exists, empty_ok, json_ok, AppError, AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError,
    PduBuilder,
};

// #POST /_matrix/client/r0/rooms/{room_id}/members
/// Lists all joined users in a room.
///
/// - Only works if the user is currently joined
#[endpoint]
pub(super) fn get_members(_aa: AuthArgs, args: MembersReqArgs, depot: &mut Depot) -> JsonResult<MembersResBody> {
    let authed = depot.authed_info()?;

    let can_see = state::user_can_see_state_events(&authed.user_id(), &args.room_id)?;
    if can_see == UserCanSeeEvent::Never {
        return Err(MatrixError::forbidden("You don't have permission to view this room.").into());
    }

    let frame_id = if let Some(at_sn) = &args.at {
        if let Ok(at_sn) = at_sn.parse::<i64>() {
            room_state_points::table
                .filter(room_state_points::room_id.eq(&args.room_id))
                .filter(room_state_points::event_sn.le(at_sn))
                .filter(room_state_points::event_sn.le(can_see.as_until_sn()))
                .filter(room_state_points::frame_id.is_not_null())
                .order(room_state_points::frame_id.desc())
                .select(room_state_points::frame_id)
                .first::<Option<i64>>(&mut db::connect()?)?
                .unwrap_or_default()
        } else {
            return Err(MatrixError::bad_state("Invalid at parameter.").into());
        }
    } else {
        state::get_room_frame_id(&args.room_id, Some(can_see.as_until_sn()))?
            .ok_or_else(|| AppError::public("state delta not found"))?
    };
    let mut states: Vec<_> = state::get_full_state(frame_id)?
        .into_iter()
        .filter(|(key, _)| key.0 == StateEventType::RoomMember)
        .map(|(_, pdu)| pdu.to_member_event())
        .collect();
    if let Some(membership) = &args.membership {
        states = states
            .into_iter()
            .filter(|event| membership.to_string() == event.deserialize().unwrap().membership().to_string())
            .collect();
    }
    if let Some(not_membership) = &args.not_membership {
        states = states
            .into_iter()
            .filter(|event| not_membership.to_string() != event.deserialize().unwrap().membership().to_string())
            .collect();
    }

    json_ok(MembersResBody { chunk: states })
}

// #POST /_matrix/client/r0/rooms/{room_id}/joined_members
/// Lists all members of a room.
///
/// - The sender user must be in the room
/// - TODO: An appservice just needs a puppet joined
#[endpoint]
pub(super) fn joined_members(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    depot: &mut Depot,
) -> JsonResult<JoinedMembersResBody> {
    let authed = depot.authed_info()?;

    let can_see = state::user_can_see_state_events(&authed.user_id(), &room_id)?;
    if can_see == UserCanSeeEvent::Never {
        return Err(MatrixError::forbidden("You don't have permission to view this room.").into());
    }

    let mut joined = BTreeMap::new();
    for user_id in crate::room::get_joined_users(&room_id, Some(can_see.as_until_sn()))? {
        if let Some(DbProfile {
            display_name,
            avatar_url,
            ..
        }) = crate::user::get_profile(&user_id, None)?
        {
            joined.insert(user_id, RoomMember::new(display_name, avatar_url));
        }
    }

    json_ok(JoinedMembersResBody { joined })
}

// #POST /_matrix/client/r0/joined_rooms
/// Lists all rooms the user has joined.
#[endpoint]
pub(crate) async fn joined_rooms(_aa: AuthArgs, depot: &mut Depot) -> JsonResult<JoinedRoomsResBody> {
    let authed = depot.authed_info()?;

    json_ok(JoinedRoomsResBody {
        joined_rooms: crate::user::joined_rooms(authed.user_id(), 0)?,
    })
}

// #POST /_matrix/client/r0/rooms/{room_id}/forget
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

// #POST /_matrix/client/r0/rooms/{room_id}/leave
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
    crate::membership::leave_room(authed.user_id(), &room_id, body.reason.clone())?;
    empty_ok()
}

// #POST /_matrix/client/r0/rooms/{room_id}/join
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
        state::get_invite_state(authed.user_id(), &room_id)?
            .unwrap_or_default()
            .iter()
            .filter_map(|event| serde_json::from_str(event.inner().get()).ok())
            .filter_map(|event: serde_json::Value| event.get("sender").cloned())
            .filter_map(|sender| sender.as_str().map(|s| s.to_owned()))
            .filter_map(|sender| UserId::parse(sender).ok())
            .map(|user| user.server_name().to_owned()),
    );

    servers.push(room_id.server_name().map_err(AppError::public)?.to_owned());

    crate::membership::join_room(
        &authed.user_id(),
        &room_id,
        body.as_ref().map(|body| body.reason.clone()).flatten(),
        &servers,
        body.as_ref().map(|body| body.third_party_signed.as_ref()).flatten(),
    )
    .await?;
    json_ok(JoinRoomResBody { room_id })
}

// #POST /_matrix/client/r0/rooms/{room_id}/invite
/// Tries to send an invite event into the room.
#[endpoint]
pub(super) async fn invite_user(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<InviteUserReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    println!("====vvvvvvvvvvvvvv  0");
    let InvitationRecipient::UserId { user_id } = &body.recipient else {
        return Err(MatrixError::not_found("User not found.").into());
    };
    println!("====vvvvvvvvvvvvvv  1");
    crate::membership::invite_user(
        authed.user_id(),
        user_id,
        &room_id.into_inner(),
        body.reason.clone(),
        false,
    )
    .await?;
    println!("====vvvvvvvvvvvvvv  2");
    empty_ok()
}

// #POST /_matrix/client/r0/join/{room_id_or_alias}
/// Tries to join the sender user into a room.
///
/// - If the server knowns about this room: creates the join event and does auth rules locally
/// - If the server does not know about the room: asks other servers over federation
#[endpoint]
pub(crate) async fn join_room_by_id_or_alias(
    _aa: AuthArgs,
    room_id_or_alias: PathParam<OwnedRoomOrAliasId>,
    server_name: QueryParam<Vec<OwnedServerName>, false>,
    body: JsonBody<Option<JoinRoomReqBody>>,
    depot: &mut Depot,
) -> JsonResult<JoinRoomResBody> {
    let authed = depot.authed_info()?;
    let room_id_or_alias = room_id_or_alias.into_inner();
    let body = body.into_inner();
    let mut servers = server_name.into_inner().unwrap_or_default();

    let (servers, room_id) = match OwnedRoomId::try_from(room_id_or_alias) {
        Ok(room_id) => {
            servers.extend(
                crate::room::state::get_invite_state(authed.user_id(), &room_id)?
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|event| serde_json::from_str(event.inner().get()).ok())
                    .filter_map(|event: serde_json::Value| event.get("sender").cloned())
                    .filter_map(|sender| sender.as_str().map(|s| s.to_owned()))
                    .filter_map(|sender| UserId::parse(sender).ok())
                    .map(|user| user.server_name().to_owned()),
            );
            servers.push(room_id.server_name().map_err(AppError::public)?.to_owned());

            (servers, room_id)
        }
        Err(room_alias) => {
            let response = crate::room::get_alias_response(room_alias).await?;
            (response.servers, response.room_id)
        }
    };

    let join_room_response = crate::membership::join_room(
        authed.user_id(),
        &room_id,
        body.as_ref().map(|body| body.reason.clone()).flatten(),
        &servers,
        body.as_ref().map(|body| body.third_party_signed.as_ref()).flatten(),
    )
    .await?;

    json_ok(JoinRoomResBody {
        room_id: join_room_response.room_id,
    })
}

// #POST /_matrix/client/r0/rooms/{room_id}/ban
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

    let room_state = state::get_state(&room_id, &StateEventType::RoomMember, body.user_id.as_ref(), None)?;

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
        let profile_request = profile_request(ProfileReqArgs {
            user_id: body.user_id.to_owned(),
            field: None,
        })?
        .into_inner();
        let ProfileResBody {
            avatar_url,
            display_name,
            blurhash,
        } = send_federation_request(body.user_id.server_name(), profile_request)
            .await?
            .json()
            .await?;

        RoomMemberEventContent {
            membership: MembershipState::Ban,
            display_name,
            avatar_url,
            is_direct: None,
            third_party_invite: None,
            blurhash,
            reason: body.reason.clone(),
            join_authorized_via_users_server: None,
        }
    } else {
        let DbProfile {
            display_name,
            avatar_url,
            blurhash,
            ..
        } = crate::user::get_profile(&body.user_id, None)?.ok_or(MatrixError::not_found("User profile not found."))?;
        RoomMemberEventContent {
            membership: MembershipState::Ban,
            display_name,
            avatar_url,
            is_direct: None,
            third_party_invite: None,
            blurhash,
            reason: body.reason.clone(),
            join_authorized_via_users_server: None,
        }
    };

    crate::room::timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&event).expect("event is valid, we just created it"),
            unsigned: None,
            state_key: Some(body.user_id.to_string()),
            redacts: None,
        },
        authed.user_id(),
        &room_id,
    )?;

    empty_ok()
}

// #POST /_matrix/client/r0/rooms/{room_id}/unban
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

    let mut event: RoomMemberEventContent = serde_json::from_str(
        crate::room::state::get_state(&room_id, &StateEventType::RoomMember, body.user_id.as_ref(), None)?
            .ok_or(MatrixError::bad_state("Cannot unban a user who is not banned."))?
            .content
            .get(),
    )
    .map_err(|_| AppError::internal("Invalid member event in database."))?;

    event.membership = MembershipState::Leave;
    event.reason = body.reason.clone();

    crate::room::timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&event).expect("event is valid, we just created it"),
            unsigned: None,
            state_key: Some(body.user_id.to_string()),
            redacts: None,
        },
        authed.user_id(),
        &room_id,
    )?;

    empty_ok()
}
// #POST /_matrix/client/r0/rooms/{room_id}/kick
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

    if !diesel_exists!(
        room_users::table
            .filter(room_users::user_id.eq(&body.user_id))
            .filter(room_users::membership.eq_any(["join", "invite"])),
        &mut db::connect()?
    )? {
        return Err(MatrixError::forbidden("User are not in the room.").into());
    }

    let mut event: RoomMemberEventContent = serde_json::from_str(
        crate::room::state::get_state(&room_id, &StateEventType::RoomMember, body.user_id.as_ref(), None)?
            .ok_or(MatrixError::bad_state("Cannot kick member that's not in the room."))?
            .content
            .get(),
    )
    .map_err(|_| AppError::internal("Invalid member event in database."))?;

    event.membership = MembershipState::Leave;
    event.reason = body.reason.clone();

    crate::room::timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&event).expect("event is valid, we just created it"),
            unsigned: None,
            state_key: Some(body.user_id.to_string()),
            redacts: None,
        },
        authed.user_id(),
        &room_id,
    )?;

    empty_ok()
}

#[endpoint]
pub(crate) async fn knock_room(
    _aa: AuthArgs,
    args: KnockReqArgs,
    body: JsonBody<KnockReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let room_id = match OwnedRoomId::try_from(args.room_id_or_alias) {
        Ok(room_id) => room_id,
        Err(room_alias) => {
            let response = crate::room::get_alias_response(room_alias).await?;
            response.room_id
        }
    };

    let mut event: RoomMemberEventContent = RoomMemberEventContent::new(MembershipState::Knock);
    event.reason = body.into_inner().reason;

    let pdu = crate::room::timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&event).expect("event is valid, we just created it"),
            unsigned: None,
            state_key: Some(authed.user_id().to_string()),
            redacts: None,
        },
        authed.user_id(),
        &room_id,
    )?;
    crate::room::update_membership(
        &pdu.event_id,
        pdu.event_sn,
        &room_id,
        authed.user_id(),
        MembershipState::Knock,
        authed.user_id(),
        None,
    )?;
    empty_ok()
}
