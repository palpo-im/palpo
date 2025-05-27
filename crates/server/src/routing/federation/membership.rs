use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde_json::value::to_raw_value;
use ulid::Ulid;

use crate::core::events::room::join_rules::JoinRule;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::federation::membership::*;
use crate::core::identifiers::*;
use crate::core::room::RoomEventReqArgs;
use crate::core::serde::{CanonicalJsonValue, JsonObject};
use crate::federation::maybe_strip_event_id;
use crate::room::timeline;
use crate::{
    DepotExt, EmptyResult, IsRemoteOrLocal, JsonResult, MatrixError, PduBuilder, PduEvent, config, empty_ok, json_ok,
    room, utils,
};

pub fn router_v1() -> Router {
    Router::new()
        .push(Router::with_path("make_join/{room_id}/{user_id}").get(make_join))
        .push(Router::with_path("invite/{room_id}/{event_id}").put(invite_user))
        .push(Router::with_path("make_leave/{room_id}/{user_id}").get(make_leave))
        .push(Router::with_path("send_join/{room_id}/{event_id}").put(send_join_v1))
        .push(Router::with_path("send_leave/{room_id}/{event_id}").put(send_leave))
}
pub fn router_v2() -> Router {
    Router::new()
        .push(Router::with_path("make_join/{room_id}/{user_id}").get(make_join))
        .push(Router::with_path("invite/{room_id}/{event_id}").put(invite_user))
        .push(Router::with_path("make_leave/{room_id}/{user_id}").get(make_leave))
        .push(Router::with_path("send_join/{room_id}/{event_id}").put(send_join_v2))
        .push(Router::with_path("send_leave/{room_id}/{event_id}").put(send_leave))
}

/// #GET /_matrix/federation/v1/make_join/{room_id}/{user_id}
/// Creates a join template.
#[endpoint]
async fn make_join(args: MakeJoinReqArgs, depot: &mut Depot) -> JsonResult<MakeJoinResBody> {
    println!("MMMMMMMMMMMMMake join  {} {} {}", crate::config::server_name(), args.room_id, args.user_id);
    if !room::room_exists(&args.room_id)? {
        return Err(MatrixError::not_found("Room is unknown to this server.").into());
    }

    let origin = depot.origin()?;
    if args.user_id.server_name() != origin {
        return Err(MatrixError::bad_json("Not allowed to join on behalf of another server/user.").into());
    }

    crate::event::handler::acl_check(args.user_id.server_name(), &args.room_id)?;

    let room_version_id = room::get_version(&args.room_id)?;
    if !args.ver.contains(&room_version_id) {
        return Err(MatrixError::incompatible_room_version("Room version not supported.", room_version_id).into());
    }

    println!(
        "MMMMMMMMMMMM {} {}, {}",
        crate::config::server_name(),
        args.room_id,
        args.user_id
    );
    let join_authorized_via_users_server: Option<OwnedUserId> = {
        use RoomVersionId::*;
        if matches!(room_version_id, V1 | V2 | V3 | V4 | V5 | V6 | V7) {
            // room version does not support restricted join rules
            println!("==================1");
            None
        } else {
            let join_rule = room::get_join_rule(&args.room_id)?;
            let guest_can_join = room::guest_can_join(&args.room_id);
            if join_rule == JoinRule::Public || guest_can_join {
                println!("==================2");
                None
            } else if crate::federation::user_can_perform_restricted_join(
                &args.user_id,
                &args.room_id,
                &room_version_id,
            )
            .await?
            {
                let Some(auth_user) = room::local_users_in_room(&args.room_id)?
                    .into_iter()
                    .filter(|user| room::user_can_invite(&args.room_id, user, &args.user_id))
                    .next()
                else {
                    println!("==================3");
                    return Err(MatrixError::unable_to_grant_join(
                        "No user on this server is able to assist in joining.",
                    )
                    .into());
                };
                println!("==================4");
                Some(auth_user)
            } else {
                println!("==================5");
                None
            }
        }
    };
    println!(
        "jjjjjjjjjoin_authorized_via_users_server: {:?}",
        join_authorized_via_users_server
    );

    let content = to_raw_value(&RoomMemberEventContent {
        avatar_url: None,
        blurhash: None,
        display_name: None,
        is_direct: None,
        membership: MembershipState::Join,
        third_party_invite: None,
        reason: None,
        join_authorized_via_users_server,
        extra_data: Default::default(),
    })
    .expect("member event is valid value");
    let (_pdu, mut pdu_json) = timeline::create_hash_and_sign_event(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content,
            state_key: Some(args.user_id.to_string()),
            ..Default::default()
        },
        &args.user_id,
        &args.room_id,
    )?;
    maybe_strip_event_id(&mut pdu_json, &room_version_id);
    let body = MakeJoinResBody {
        room_version: Some(room_version_id),
        event: to_raw_value(&pdu_json).expect("CanonicalJson can be serialized to JSON"),
    };
    json_ok(body)
}

/// #PUT /_matrix/federation/v2/invite/{room_id}/{event_id}
/// Invites a remote user to a room.
#[endpoint]
async fn invite_user(
    args: RoomEventReqArgs,
    body: JsonBody<InviteUserReqBodyV2>,
    depot: &mut Depot,
) -> JsonResult<InviteUserResBodyV2> {
    let body = body.into_inner();
    let origin = depot.origin()?;
    crate::event::handler::acl_check(origin, &args.room_id)?;

    if !config::supported_room_versions().contains(&body.room_version) {
        return Err(MatrixError::incompatible_room_version(
            "Server does not support this room version.",
            body.room_version.clone(),
        )
        .into());
    }

    let mut signed_event =
        utils::to_canonical_object(&body.event).map_err(|_| MatrixError::invalid_param("Invite event is invalid."))?;

    let invitee_id: OwnedUserId = serde_json::from_value(
        signed_event
            .get("state_key")
            .ok_or(MatrixError::invalid_param("Event had no state_key field."))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::invalid_param("state_key is not a user id."))?;
    if invitee_id.server_name().is_remote() {
        return Err(MatrixError::invalid_param("Cannot invite remote users.").into());
    }

    crate::event::handler::acl_check(invitee_id.server_name(), &args.room_id)?;

    crate::server_key::hash_and_sign_event(&mut signed_event, &body.room_version)
        .map_err(|e| MatrixError::invalid_param(format!("Failed to sign event: {e}.")))?;

    // Generate event id
    let event_id = crate::event::gen_event_id(&signed_event, &body.room_version)?;

    // Add event_id back
    signed_event.insert("event_id".to_owned(), CanonicalJsonValue::String(event_id.to_string()));

    let sender: OwnedUserId = serde_json::from_value(
        signed_event
            .get("sender")
            .ok_or(MatrixError::invalid_param("Event had no sender field."))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::invalid_param("sender is not a user id."))?;

    let mut invite_state = body.invite_room_state.clone();

    let mut event: JsonObject = serde_json::from_str(body.event.get())
        .map_err(|_| MatrixError::invalid_param("Invalid invite event bytes."))?;

    let event_id: OwnedEventId = format!("$dummy_{}", Ulid::new().to_string()).try_into()?;
    event.insert("event_id".to_owned(), event_id.to_string().into());

    let pdu: PduEvent = PduEvent::from_json_value(
        &event_id,
        crate::event::ensure_event_sn(&args.room_id, &event_id)?,
        event.into(),
    )
    .map_err(|e| {
        warn!("Invalid invite event: {}", e);
        MatrixError::invalid_param("Invalid invite event.")
    })?;
    invite_state.push(pdu.to_stripped_state_event());

    // If we are active in the room, the remote server will notify us about the join via /send.
    // If we are not in the room, we need to manually
    // record the invited state for client /sync through update_membership(), and
    // send the invite PDU to the relevant appservices.
    if !room::is_server_joined_room(config::server_name(), &args.room_id)? {
        crate::membership::update_membership(
            &pdu.event_id,
            pdu.event_sn,
            &args.room_id,
            &invitee_id,
            MembershipState::Invite,
            &sender,
            Some(invite_state),
        )?;
    }

    json_ok(InviteUserResBodyV2 {
        event: crate::sending::convert_to_outgoing_federation_event(signed_event),
    })
}

/// # `GET /_matrix/federation/v1/make_leave/{roomId}/userId}`
#[endpoint]
async fn make_leave(args: MakeLeaveReqArgs, depot: &mut Depot) -> JsonResult<MakeLeaveResBody> {
    let origin = depot.origin()?;
    if args.user_id.server_name() != origin {
        return Err(MatrixError::bad_json("Not allowed to leave on behalf of another server.").into());
    }
    if !room::is_room_exists(&args.room_id)? {
        return Err(MatrixError::forbidden("Room is unknown to this server.", None).into());
    }

    // ACL check origin
    crate::event::handler::acl_check(origin, &args.room_id)?;

    let room_version_id = room::get_version(&args.room_id)?;
    // let state_lock = services.rooms.state.mutex.lock(&body.room_id).await;

    let (_pdu, mut pdu_json) = timeline::create_hash_and_sign_event(
        PduBuilder::state(
            args.user_id.to_string(),
            &RoomMemberEventContent::new(MembershipState::Leave),
        ),
        &args.user_id,
        &args.room_id,
    )?;

    // drop(state_lock);

    // room v3 and above removed the "event_id" field from remote PDU format
    maybe_strip_event_id(&mut pdu_json, &room_version_id);

    json_ok(MakeLeaveResBody {
        room_version: Some(room_version_id),
        event: to_raw_value(&pdu_json).expect("CanonicalJson can be serialized to JSON"),
    })
}

/// #PUT /_matrix/federation/v2/send_join/{room_id}/{event_id}
/// Invites a remote user to a room.
#[endpoint]
async fn send_join_v2(
    depot: &mut Depot,
    args: RoomEventReqArgs,
    body: JsonBody<SendJoinReqBody>,
) -> JsonResult<SendJoinResBodyV2> {
    let body = body.into_inner();
    // let server_name = args.room_id.server_name().map_err(AppError::public)?;
    // crate::event::handler::acl_check(&server_name, &args.room_id)?;

    let room_state = crate::federation::membership::send_join_v2(depot.origin()?, &args.room_id, &body.0).await?;

    json_ok(SendJoinResBodyV2(room_state))
}

/// #PUT /_matrix/federation/v1/send_join/{room_id}/{event_id}
/// Submits a signed join event.
#[endpoint]
async fn send_join_v1(
    depot: &mut Depot,
    args: RoomEventReqArgs,
    body: JsonBody<SendJoinReqBody>,
) -> JsonResult<SendJoinResBodyV1> {
    let body = body.into_inner();
    let room_state = crate::federation::membership::send_join_v1(depot.origin()?, &args.room_id, &body.0).await?;
    json_ok(SendJoinResBodyV1(room_state))
}

/// #PUT /_matrix/federation/v2/send_leave/{roomId}/{eventId}
///
/// Submits a signed leave event.
#[endpoint]
async fn send_leave(depot: &mut Depot, args: SendLeaveReqArgsV2, body: JsonBody<SendLeaveReqBody>) -> EmptyResult {
    let origin = depot.origin()?;
    let body = body.into_inner();

    if !room::is_room_exists(&args.room_id)? {
        return Err(MatrixError::forbidden("Room is unknown to this server.", None).into());
    }
    crate::event::handler::acl_check(origin, &args.room_id)?;

    // We do not add the event_id field to the pdu here because of signature and hashes checks
    let room_version_id = room::get_version(&args.room_id)?;

    let Ok((event_id, value)) = crate::event::gen_event_id_canonical_json(&body.0, &room_version_id) else {
        // Event could not be converted to canonical json
        return Err(MatrixError::invalid_param("Could not convert event to canonical json.").into());
    };

    let event_room_id: OwnedRoomId = serde_json::from_value(
        serde_json::to_value(
            value
                .get("room_id")
                .ok_or_else(|| MatrixError::bad_json("Event missing room_id property."))?,
        )
        .expect("CanonicalJson is valid json value"),
    )
    .map_err(|e| MatrixError::bad_json(format!("room_id field is not a valid room ID: {e}")))?;

    if event_room_id != args.room_id {
        return Err(MatrixError::bad_json("Event room_id does not match request path room ID.").into());
    }

    let content: RoomMemberEventContent = serde_json::from_value(
        value
            .get("content")
            .ok_or_else(|| MatrixError::bad_json("Event missing content property"))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("Event content is empty or invalid"))?;

    if content.membership != MembershipState::Leave {
        return Err(
            MatrixError::bad_json("Not allowed to send a non-leave membership event to leave endpoint.").into(),
        );
    }

    let event_type: StateEventType = serde_json::from_value(
        value
            .get("type")
            .ok_or_else(|| MatrixError::bad_json("Event missing type property."))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("Event does not have a valid state event type."))?;

    if event_type != StateEventType::RoomMember {
        return Err(
            MatrixError::invalid_param("Not allowed to send non-membership state event to leave endpoint.").into(),
        );
    }

    // ACL check sender server name
    let sender: OwnedUserId = serde_json::from_value(
        value
            .get("sender")
            .ok_or_else(|| MatrixError::bad_json("Event missing sender property."))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("User ID in sender is invalid."))?;

    crate::event::handler::acl_check(sender.server_name(), &args.room_id)?;

    if sender.server_name() != origin {
        return Err(MatrixError::bad_json("Not allowed to leave on behalf of another server.").into());
    }

    let state_key: OwnedUserId = serde_json::from_value(
        value
            .get("state_key")
            .ok_or_else(|| MatrixError::invalid_param("Event missing state_key property."))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("state_key is invalid or not a user ID"))?;

    if state_key != sender {
        return Err(MatrixError::bad_json("state_key does not match sender user.").into());
    }

    let state_lock = crate::room::lock_state(&args.room_id).await;
    crate::event::handler::process_incoming_pdu(origin, &event_id, &args.room_id, &room_version_id, value, true)
        .await?;
    drop(state_lock);

    crate::sending::send_pdu_room(&args.room_id, &event_id).unwrap();
    empty_ok()
}
