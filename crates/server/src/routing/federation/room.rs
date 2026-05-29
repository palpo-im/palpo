use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde_json::value::to_raw_value;

use crate::core::client::directory::{PublicRoomsFilteredReqBody, PublicRoomsReqArgs};
use crate::core::directory::{PublicRoomFilter, PublicRoomsResBody, RoomNetwork};
use crate::core::events::StateEventType;
use crate::core::events::room::history_visibility::{
    HistoryVisibility, RoomHistoryVisibilityEventContent,
};
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::federation::event::{
    RoomStateAtEventReqArgs, RoomStateIdsResBody, RoomStateReqArgs, RoomStateResBody,
};
use crate::core::federation::knock::{
    MakeKnockReqArgs, MakeKnockResBody, SendKnockReqArgs, SendKnockReqBody, SendKnockResBody,
};
use crate::core::federation::peek::{
    PeekReqArgs, PeekResBody, PeekStartResBody, PeekSubReqArgs,
};
use crate::core::identifiers::*;
use crate::core::serde::{JsonObject, RawJsonValue};
use crate::event::{gen_event_id_canonical_json, handler};
use crate::federation::peek as fed_peek;
use crate::room::{state, timeline};
use crate::{
    AppResult, AuthArgs, DepotExt, EmptyResult, IsRemoteOrLocal, JsonResult, MatrixError,
    PduBuilder, PduEvent, data, empty_ok, json_ok, room, sending,
};

pub fn router() -> Router {
    Router::new()
        .push(Router::with_path("state/{room_id}").get(get_state))
        .push(
            Router::with_path("publicRooms")
                .get(get_public_rooms)
                .post(get_filtered_public_rooms),
        )
        .push(Router::with_path("send_knock/{room_id}/{event_id}").put(send_knock))
        .push(Router::with_path("make_knock/{room_id}/{user_id}").get(make_knock))
        .push(Router::with_path("state_ids/{room_id}").get(get_state_at_event))
        .push(Router::with_path("peek/{room_id}").get(peek))
        .push(
            Router::with_path("peek/{room_id}/{peek_id}")
                .put(peek_start)
                .delete(peek_cancel),
        )
}

/// #GET /_matrix/federation/v1/peek/{room_id}
/// Returns a snapshot (current state + auth chain + recent messages) of a
/// world-readable room so a remote server can render a preview for a user who
/// has not joined it. Lightweight federated peek in the spirit of MSC2444.
#[endpoint]
async fn peek(_aa: AuthArgs, args: PeekReqArgs, depot: &mut Depot) -> JsonResult<PeekResBody> {
    // Authenticated as a federating server by the auth hoop.
    let origin = depot.origin()?;
    let room_id = &args.room_id;

    if !room::room_exists(room_id).await? {
        return Err(MatrixError::not_found("Room not found on this server.").into());
    }

    // Honour `m.room.server_acl`: a server denied by the room's ACL must not be
    // able to read room data through peeking either.
    handler::acl_check(origin, room_id).await?;

    // Only world-readable rooms may be peeked without membership; otherwise the
    // requesting server has no business reading the state.
    if !room::is_world_readable(room_id).await {
        return Err(MatrixError::forbidden("Room is not world-readable; peeking is not permitted.", None).into());
    }

    let room_version = room::get_version(room_id).await?;

    // Return only the room's public "description" state (stripped state), never
    // the full state graph. Membership, power levels and custom state can have
    // been written while the room was private; the spec's stripped-state set is
    // exactly the surface meant to be shared with non-members for a preview, so
    // we expose that and nothing more (also avoids leaking the auth chain).
    let mut pdus = Vec::with_capacity(PEEK_STATE_TYPES.len());
    for ty in PEEK_STATE_TYPES {
        let Ok(pdu) = room::get_state(room_id, ty, "", None).await else {
            continue;
        };
        match timeline::get_pdu_json(&pdu.event_id).await {
            Ok(Some(json)) => pdus.push(sending::convert_to_outgoing_federation_event(json).await),
            Ok(None) => {}
            Err(e) => error!("peek: failed to load state event {}: {e}", pdu.event_id),
        }
    }

    // A page of recent timeline events for the preview.
    //
    // History visibility is evaluated at the point each event was sent (per the
    // spec), not from the room's *current* state. The room is world-readable
    // now, but older messages may have been sent while it was joined/shared. A
    // peeking server is an unaffiliated non-member, so it may only receive
    // events whose history visibility *at that event's own state* was
    // world-readable; everything else is excluded.
    //
    // Because that filter runs outside the loader, a run of recent
    // non-readable events could otherwise yield an empty page while older
    // readable events still exist. So we grow the scan window (doubling, capped)
    // until the visible page is filled or the room history is exhausted.
    let messages = recent_world_readable_messages(room_id, args.limit.clamp(1, 100)).await?;

    json_ok(PeekResBody {
        room_version,
        pdus,
        messages,
    })
}

/// #PUT /_matrix/federation/v1/peek/{room_id}/{peek_id}
/// Start or renew an ongoing peek subscription (MSC2444). Records the requesting
/// server so it receives new room events, and returns the full current state +
/// auth chain + recent messages so the peer can build a local copy of the room.
#[endpoint]
async fn peek_start(
    _aa: AuthArgs,
    args: PeekSubReqArgs,
    depot: &mut Depot,
) -> JsonResult<PeekStartResBody> {
    let origin = depot.origin()?.clone();
    let room_id = &args.room_id;

    if !room::room_exists(room_id).await? {
        return Err(MatrixError::not_found("Room not found on this server.").into());
    }
    handler::acl_check(&origin, room_id).await?;

    let room_version = room::get_version(room_id).await?;

    // Pin the frame we'll serve, then verify world-readability *at that exact
    // frame* (not "current"). This closes the TOCTOU where an admin makes the
    // room private between a coarse is_world_readable() check and reading the
    // state: whatever frame we return, we've confirmed it was world-readable.
    let frame_id = room::get_frame_id(room_id, None).await?;
    let frame_world_readable = state::get_state_content::<RoomHistoryVisibilityEventContent>(
        frame_id,
        &StateEventType::RoomHistoryVisibility,
        "",
    )
    .await
    .map(|c| c.history_visibility == HistoryVisibility::WorldReadable)
    .unwrap_or(false);
    if !frame_world_readable {
        return Err(MatrixError::forbidden(
            "Room is not world-readable; peeking is not permitted.",
            None,
        )
        .into());
    }

    // Register the peer before capturing the snapshot, so any event created while
    // we read state/messages is forwarded to it (rather than falling into a gap
    // between the snapshot and registration). A forwarded event the peer can't
    // place yet is recovered via prev_events backfill, and duplicates with the
    // snapshot are harmless/idempotent on its side. (Ongoing relay is itself
    // re-guarded by world-readability + ACL in append_pdu.)
    fed_peek::register_peeking_server(room_id, &origin, &args.peek_id).await?;

    // Full current state + backing auth chain (like a send_join response) so the
    // peer can construct a complete, verifiable local copy of this world-readable
    // room, built from the same `frame_id` we verified above.
    let state_ids: Vec<OwnedEventId> = state::get_full_state_ids(frame_id)
        .await?
        .into_values()
        .collect();

    let mut state = Vec::with_capacity(state_ids.len());
    for id in &state_ids {
        if let Ok(Some(json)) = timeline::get_pdu_json(id).await {
            state.push(sending::convert_to_outgoing_federation_event(json).await);
        }
    }

    let auth_chain_ids =
        room::auth_chain::get_auth_chain_ids(room_id, state_ids.iter().map(AsRef::as_ref)).await?;
    let mut auth_chain = Vec::with_capacity(auth_chain_ids.len());
    for id in &auth_chain_ids {
        if let Ok(Some(json)) = timeline::get_pdu_json(id).await {
            auth_chain.push(sending::convert_to_outgoing_federation_event(json).await);
        }
    }

    let messages = recent_world_readable_messages(room_id, 50).await?;

    json_ok(PeekStartResBody {
        room_version,
        state,
        auth_chain,
        messages,
        renewal_interval: fed_peek::PEEK_RENEWAL_INTERVAL_MS,
    })
}

/// #DELETE /_matrix/federation/v1/peek/{room_id}/{peek_id}
/// Cancel an ongoing peek subscription (MSC2444).
#[endpoint]
async fn peek_cancel(_aa: AuthArgs, args: PeekSubReqArgs, depot: &mut Depot) -> EmptyResult {
    let origin = depot.origin()?.clone();
    fed_peek::unregister_peeking_server(&args.room_id, &origin, &args.peek_id).await?;
    empty_ok()
}

/// Collect up to `limit` of the most recent timeline events that were
/// world-readable at the point they were sent, oldest-to-newest. Grows the scan
/// window (doubling, capped) so a run of recent non-readable events doesn't
/// yield an empty page while older readable events still exist.
async fn recent_world_readable_messages(
    room_id: &RoomId,
    limit: usize,
) -> AppResult<Vec<Box<RawJsonValue>>> {
    let mut messages = Vec::with_capacity(limit);
    let mut scan = limit.saturating_mul(2).clamp(limit, 100);
    loop {
        let recent =
            timeline::stream::load_pdus_backward(None, room_id, None, None, None, scan).await?;
        let raw = recent.len();

        messages.clear();
        for (_sn, pdu) in &recent {
            if messages.len() >= limit {
                break;
            }
            if !event_world_readable(&pdu.event_id).await {
                continue;
            }
            if let Ok(Some(json)) = timeline::get_pdu_json(&pdu.event_id).await {
                messages.push(sending::convert_to_outgoing_federation_event(json).await);
            }
        }

        if messages.len() >= limit || raw < scan || scan >= PEEK_SCAN_CAP {
            break;
        }
        scan = scan.saturating_mul(2).min(PEEK_SCAN_CAP);
    }
    messages.reverse();
    Ok(messages)
}

/// Upper bound on how many recent events a single peek will scan while looking
/// for world-readable messages, so the responder stays cheap even if a long run
/// of recent events is not world-readable.
const PEEK_SCAN_CAP: usize = 1000;

/// Whether an event was world-readable at the point it was sent, i.e. the
/// `m.room.history_visibility` in the room state *at that event's own frame* was
/// `world_readable`. This is the only visibility a non-member peeking server is
/// allowed to see; anything else (or an unresolvable frame/visibility) is false.
async fn event_world_readable(event_id: &EventId) -> bool {
    let Ok(frame_id) = state::get_pdu_frame_id(event_id).await else {
        return false;
    };
    state::get_state_content::<RoomHistoryVisibilityEventContent>(
        frame_id,
        &StateEventType::RoomHistoryVisibility,
        "",
    )
    .await
    .map(|c| c.history_visibility == HistoryVisibility::WorldReadable)
    .unwrap_or(false)
}

/// The room "description" state shared in a peek preview — the recommended
/// stripped-state types plus history visibility. Deliberately excludes
/// membership, power levels and arbitrary state so a peek cannot leak more than
/// a non-member is meant to see.
const PEEK_STATE_TYPES: &[StateEventType] = &[
    StateEventType::RoomCreate,
    StateEventType::RoomName,
    StateEventType::RoomTopic,
    StateEventType::RoomAvatar,
    StateEventType::RoomJoinRules,
    StateEventType::RoomCanonicalAlias,
    StateEventType::RoomEncryption,
    StateEventType::RoomHistoryVisibility,
];

/// #GET /_matrix/federation/v1/state/{room_id}
/// Retrieves the current state of the room.
#[endpoint]
async fn get_state(
    _aa: AuthArgs,
    args: RoomStateReqArgs,
    depot: &mut Depot,
) -> JsonResult<RoomStateResBody> {
    let origin = depot.origin()?;
    crate::federation::access_check(origin, &args.room_id, None).await?;

    let state_hash = state::get_pdu_frame_id(&args.event_id).await?;

    let mut pdus = Vec::new();
    for id in state::get_full_state_ids(state_hash).await?.into_values() {
        match timeline::get_pdu_json(&id).await {
            Ok(Some(json)) => {
                pdus.push(sending::convert_to_outgoing_federation_event(json).await);
            }
            Ok(None) => {
                error!("Could not find event json for state event {id} in db");
            }
            Err(e) => {
                error!("Failed to load state event {id} from db: {e}");
            }
        }
    }

    let auth_chain_ids =
        room::auth_chain::get_auth_chain_ids(&args.room_id, [&*args.event_id].into_iter()).await?;

    let mut auth_chain = Vec::new();
    for id in auth_chain_ids.into_iter() {
        match timeline::get_pdu_json(&id).await {
            Ok(Some(json)) => {
                auth_chain
                    .push(crate::sending::convert_to_outgoing_federation_event(json).await);
            }
            Ok(None) => {
                error!("Could not find event json for {id} in db::");
            }
            Err(_) => {}
        }
    }

    json_ok(RoomStateResBody { auth_chain, pdus })
}

/// #GET /_matrix/federation/v1/publicRooms
/// Lists the public rooms on this server.
#[endpoint]
async fn get_public_rooms(
    _aa: AuthArgs,
    args: PublicRoomsReqArgs,
) -> JsonResult<PublicRoomsResBody> {
    let body = crate::directory::get_public_rooms(
        None,
        args.limit,
        args.since.as_deref(),
        &PublicRoomFilter::default(),
        &RoomNetwork::Matrix,
    )
    .await?;
    json_ok(body)
}

/// #POST /_matrix/federation/v1/publicRooms
/// Lists the public rooms on this server.
#[endpoint]
async fn get_filtered_public_rooms(
    _aa: AuthArgs,
    args: JsonBody<PublicRoomsFilteredReqBody>,
) -> JsonResult<PublicRoomsResBody> {
    let body = crate::directory::get_public_rooms(
        args.server.as_deref(),
        args.limit,
        args.since.as_deref(),
        &args.filter,
        &args.room_network,
    )
    .await?;
    json_ok(body)
}

/// # `PUT /_matrix/federation/v1/send_knock/{roomId}/{eventId}`
///
/// Submits a signed knock event.
#[endpoint]
async fn send_knock(
    _aa: AuthArgs,
    args: SendKnockReqArgs,
    _req: &mut Request,
    body: JsonBody<SendKnockReqBody>,
    depot: &mut Depot,
) -> JsonResult<SendKnockResBody> {
    use crate::core::RoomVersionId::*;

    let origin = depot.origin()?;
    let body: SendKnockReqBody = body.into_inner();

    if args.room_id.is_remote() {
        return Err(MatrixError::not_found("room is unknown to this server").into());
    }

    // ACL check origin server
    handler::acl_check(origin, &args.room_id).await?;

    let room_version = crate::room::get_version(&args.room_id).await?;

    if matches!(room_version, V1 | V2 | V3 | V4 | V5 | V6) {
        return Err(MatrixError::forbidden("room version does not support knocking", None).into());
    }

    let Ok((event_id, value)) = gen_event_id_canonical_json(&body.0, &room_version) else {
        // Event could not be converted to canonical json
        return Err(MatrixError::invalid_param("could not convert event to canonical json").into());
    };

    let event_type: StateEventType = serde_json::from_value(
        value
            .get("type")
            .ok_or_else(|| MatrixError::invalid_param("event has no event type"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::invalid_param(format!("event has invalid event type: {e}")))?;

    if event_type != StateEventType::RoomMember {
        return Err(MatrixError::invalid_param(
            "not allowed to send non-membership state event to knock endpoint",
        )
        .into());
    }

    let content: RoomMemberEventContent = serde_json::from_value(
        value
            .get("content")
            .ok_or_else(|| MatrixError::invalid_param("membership event has no content"))?
            .clone()
            .into(),
    )
    .map_err(|e| {
        MatrixError::invalid_param(format!("event has invalid membership content: {e}"))
    })?;

    if content.membership != MembershipState::Knock {
        return Err(MatrixError::invalid_param(
            "not allowed to send a non-knock membership event to knock endpoint",
        )
        .into());
    }

    // ACL check sender server name
    let sender: OwnedUserId = serde_json::from_value(
        value
            .get("sender")
            .ok_or_else(|| MatrixError::invalid_param("event has no sender user id"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::invalid_param(format!("event sender is not a valid user id: {e}")))?;

    handler::acl_check(sender.server_name(), &args.room_id).await?;

    // check if origin server is trying to send for another server
    if sender.server_name() != origin {
        return Err(MatrixError::bad_json(
            "Not allowed to knock on behalf of another server/user.",
        )
        .into());
    }

    let state_key: OwnedUserId = serde_json::from_value(
        value
            .get("state_key")
            .ok_or_else(|| MatrixError::invalid_param("event does not have a state_key"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json(format!("event does not have a valid state_key: {e}")))?;

    if state_key != sender {
        return Err(
            MatrixError::invalid_param("state_key does not match sender user of event.").into(),
        );
    };

    let origin: OwnedServerName = serde_json::from_value(
        value
            .get("origin")
            .ok_or_else(|| MatrixError::bad_json("event does not have an origin server name"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json(format!("event has an invalid origin server name: {e}")))?;

    let event: JsonObject = serde_json::from_str(body.0.get())
        .map_err(|e| MatrixError::invalid_param(format!("invalid knock event PDU: {e}")))?;

    let pdu: PduEvent = PduEvent::from_json_value(&args.room_id, &event_id, event.into())
        .map_err(|e| MatrixError::invalid_param(format!("invalid knock event pdu: {e}")))?;

    handler::process_incoming_pdu(
        &origin,
        &event_id,
        &args.room_id,
        &room_version,
        value.clone(),
        true,
        false,
    )
    .await
    .map_err(|e| {
        error!(
            error = %e,
            room_id = %args.room_id, "could not accept as timeline event {}", event_id
        );
        MatrixError::invalid_param("could not accept as timeline event".to_string())
    })?;

    data::room::add_joined_server(&args.room_id, &origin).await?;

    let knock_room_state = state::summary_stripped(&pdu).await?;
    if let Err(e) = crate::sending::send_pdu_room(&args.room_id, &event_id, &[], &[]).await {
        error!("failed to notify knock event: {e}");
    }
    json_ok(SendKnockResBody { knock_room_state })
}

/// # `GET /_matrix/federation/v1/make_knock/{room_id}/{user_id}`
///
/// Creates a knock template.
#[endpoint]
async fn make_knock(
    _aa: AuthArgs,
    args: MakeKnockReqArgs,
    depot: &mut Depot,
) -> JsonResult<MakeKnockResBody> {
    use crate::core::RoomVersionId::*;

    let origin = depot.origin()?;
    if !crate::room::room_exists(&args.room_id).await? {
        return Err(MatrixError::not_found("room is unknown to this server").into());
    }

    if args.user_id.server_name() != origin {
        return Err(
            MatrixError::bad_json("not allowed to knock on behalf of another server/user").into(),
        );
    }

    // ACL check origin server
    handler::acl_check(origin, &args.room_id).await?;

    let room_version_id = crate::room::get_version(&args.room_id).await?;

    if matches!(room_version_id, V1 | V2 | V3 | V4 | V5 | V6) {
        return Err(MatrixError::incompatible_room_version(
            "room version does not support knocking",
            room_version_id,
        )
        .into());
    }

    // if !args.ver.contains(&room_version_id) {
    //     return Err(MatrixError::incompatible_room_version(
    //         room_version_id,
    //         "Your homeserver does not support the features required to knock on this room.",
    //     ));
    // }

    let state_lock = room::lock_state(&args.room_id).await;
    if let Ok(member) = room::get_member(&args.room_id, &args.user_id, None).await
        && member.membership == MembershipState::Ban
    {
        warn!(
            "remote user {} is banned from {} but attempted to knock",
            &args.user_id, &args.room_id
        );
        return Err(
            MatrixError::forbidden("you cannot knock on a room you are banned from", None).into(),
        );
    }

    let (_pdu, mut pdu_json) = PduBuilder::state(
        args.user_id.to_string(),
        &RoomMemberEventContent::new(MembershipState::Knock),
    )
    .hash_sign(&args.user_id, &args.room_id, &room_version_id)
    .await?;
    drop(state_lock);

    // room v3 and above removed the "event_id" field from remote PDU format
    crate::federation::maybe_strip_event_id(&mut pdu_json, &room_version_id);
    json_ok(MakeKnockResBody {
        room_version: room_version_id,
        event: to_raw_value(&pdu_json).expect("CanonicalJson can be serialized to json"),
    })
}

/// #GET /_matrix/federation/v1/state_ids/{room_id}
/// Retrieves the current state of the room.
#[endpoint]
async fn get_state_at_event(
    depot: &mut Depot,
    args: RoomStateAtEventReqArgs,
) -> JsonResult<RoomStateIdsResBody> {
    let origin = depot.origin()?;

    crate::federation::access_check(origin, &args.room_id, Some(&args.event_id)).await?;

    let frame_id = state::get_pdu_frame_id(&args.event_id).await?;

    let pdu_ids = state::get_full_state_ids(frame_id)
        .await?
        .into_values()
        .map(|id| (*id).to_owned())
        .collect();

    let auth_chain_ids =
        crate::room::auth_chain::get_auth_chain_ids(&args.room_id, [&*args.event_id].into_iter())
            .await?;

    json_ok(RoomStateIdsResBody {
        auth_chain_ids: auth_chain_ids
            .into_iter()
            .map(|id| (*id).to_owned())
            .collect(),
        pdu_ids,
    })
}
