use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde_json::json;
use serde_json::value::to_raw_value;

use crate::core::UnixMillis;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{AnyStrippedStateEvent, StateEventType, TimelineEventType};
use crate::core::federation::membership::*;
use crate::core::identifiers::*;
use crate::core::room::{JoinRule, RoomEventReqArgs};
use crate::core::room_version_rules::{EventIdFormatVersion, RoomVersionRules};
use crate::core::serde::{
    CanonicalJsonObject, CanonicalJsonValue, JsonValue, RawJson, RawJsonValue, canonical_json,
    to_canonical_object,
};
use crate::core::signatures::Verified;
use crate::data::connect;
use crate::data::room::NewDbEvent;
use crate::data::schema::*;
use crate::event::{PduEvent, handler};
use crate::federation::maybe_strip_event_id;
use crate::room::{ensure_room, timeline};
use crate::{
    AppResult, DepotExt, EmptyResult, IsRemoteOrLocal, JsonResult, MatrixError, PduBuilder,
    SnPduEvent, config, data, empty_ok, json_ok, membership, room,
};

pub fn router_v1() -> Router {
    // Keep the v1 send_join / send_leave routes for older remote servers. New outgoing membership
    // requests are built with the v2 helpers in palpo-core.
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
    if !room::room_exists(&args.room_id).await? {
        return Err(MatrixError::not_found("Room is unknown to this server.").into());
    }

    let origin = depot.origin()?;
    if args.user_id.server_name() != origin {
        return Err(
            MatrixError::bad_json("Not allowed to join on behalf of another server/user.").into(),
        );
    }

    handler::acl_check(args.user_id.server_name(), &args.room_id).await?;

    let room_version_id = room::get_version(&args.room_id).await?;
    if !args.ver.contains(&room_version_id) {
        return Err(MatrixError::incompatible_room_version(
            "Room version not supported.",
            room_version_id,
        )
        .into());
    }

    let state_lock = crate::room::lock_state(&args.room_id).await;

    if args.user_id.is_remote()
        && args.room_id.is_remote()
        && !room::is_server_joined(&config::get().server_name, &args.room_id).await?
    {
        return Err(MatrixError::bad_json("Not allowed to join on unkonwn remote server.").into());
    }
    let join_authorized_via_users_server: Option<OwnedUserId> = {
        use RoomVersionId::*;
        if matches!(room_version_id, V1 | V2 | V3 | V4 | V5 | V6 | V7) {
            // room version does not support restricted join rules
            None
        } else {
            let join_rule = room::get_join_rule(&args.room_id).await?;
            let guest_can_join = room::guest_can_join(&args.room_id).await;
            if join_rule == JoinRule::Public || guest_can_join {
                None
            } else if crate::federation::user_can_perform_restricted_join(
                &args.user_id,
                &args.room_id,
                &room_version_id,
                Some(&join_rule),
            )
            .await?
            {
                membership::get_first_user_can_issue_invite(
                    &args.room_id,
                    &args.user_id,
                    &join_rule.restriction_rooms(),
                )
                .await
                .ok()
            } else {
                return Err(MatrixError::unable_to_grant_join(
                    "no user on this server is able to assist in joining",
                )
                .into());
            }
        }
    };

    let content = to_raw_value(&RoomMemberEventContent {
        avatar_url: None,
        blurhash: None,
        display_name: None,
        is_direct: None,
        membership: MembershipState::Join,
        third_party_invite: None,
        reason: None,
        join_authorized_via_users_server,
        #[cfg(feature = "unstable-msc4293")]
        redact_events: false,
        extra_data: Default::default(),
    })
    .expect("member event is valid value");
    let (_pdu, mut pdu_json) = PduBuilder {
        event_type: TimelineEventType::RoomMember,
        content,
        state_key: Some(args.user_id.to_string()),
        ..Default::default()
    }
    .hash_sign(&args.user_id, &args.room_id, &room_version_id)
    .await?;
    drop(state_lock);
    maybe_strip_event_id(&mut pdu_json, &room_version_id);
    let body = MakeJoinResBody {
        room_version: Some(room_version_id),
        event: to_raw_value(&pdu_json).expect("CanonicalJson can be serialized to JSON"),
    };
    json_ok(body)
}

/// Read or recompute an event ID from a PDU in Palpo's stored form.
///
/// Room versions 1 and 2 carry an explicit event ID. Later versions derive it from a
/// reference hash and do not carry `event_id` on the wire, but Palpo adds the field back
/// before policy processing and persistence. That stored field must not become part of the
/// reference-hash input when checking that supplementary signatures preserved the ID.
fn event_id_for_pdu(
    event: &CanonicalJsonObject,
    room_version: &RoomVersionId,
    rules: &RoomVersionRules,
) -> Result<OwnedEventId, crate::AppError> {
    if rules.event_id_format == EventIdFormatVersion::V1 {
        return event
            .get("event_id")
            .and_then(CanonicalJsonValue::as_str)
            .ok_or_else(|| {
                crate::AppError::from(MatrixError::invalid_param(
                    "event has no valid event_id field",
                ))
            })?
            .try_into()
            .map_err(|_| crate::AppError::from(MatrixError::invalid_param("event_id is invalid")));
    }

    let mut event = event.clone();
    event.remove("event_id");
    crate::event::gen_event_id(&event, room_version)
}

fn requires_full_invite_state(rules: &RoomVersionRules) -> bool {
    // Room version 12 is the first stable version covered by the mandatory MSC4311
    // validation. The same authorization flag also identifies its domainless room IDs.
    rules.authorization.room_create_event_id_as_room_id
}

/// Check that an incoming federation invite is a membership invite and, when this server
/// participates in the room, that it passes room authorization against trusted state.
///
/// A server which is not yet in the room has no trusted state to authorise against.
/// `invite_room_state` is not an auth snapshot (it carries no auth chain), so it is only
/// validated for format and integrity, by `verified_v12_invite_state` for room versions
/// that require it.
async fn authenticate_invite_event(
    room_id: &RoomId,
    event_id: &EventId,
    event: &CanonicalJsonObject,
    rules: &RoomVersionRules,
) -> AppResult<PduEvent> {
    let incoming = PduEvent::from_canonical_object(room_id, event_id, event.clone())
        .map_err(|_| MatrixError::invalid_param("invalid invite event"))?;
    if incoming.event_ty != TimelineEventType::RoomMember
        || incoming.state_key.as_deref().is_none()
        || incoming
            .get_content::<RoomMemberEventContent>()
            .map_err(|_| MatrixError::invalid_param("invite has invalid member content"))?
            .membership
            != MembershipState::Invite
    {
        return Err(MatrixError::invalid_param("event is not a membership invite").into());
    }

    // When we participate in the room, authorise against our trusted event-time state.
    if room::is_server_joined(config::server_name(), room_id).await? {
        handler::auth_check(&incoming, rules, None).await?;
    }
    Ok(incoming)
}

#[endpoint]
async fn invite_user(
    args: RoomEventReqArgs,
    body: JsonBody<InviteUserReqBodyV2>,
    depot: &mut Depot,
) -> JsonResult<InviteUserResBodyV2> {
    let body = body.into_inner();
    let origin = depot.origin()?;
    let conf = config::get();
    handler::acl_check(origin, &args.room_id).await?;

    if !config::supported_room_versions().contains(&body.room_version) {
        return Err(MatrixError::incompatible_room_version(
            "server does not support this room version",
            body.room_version.clone(),
        )
        .into());
    }
    let mut signed_event = to_canonical_object(&body.event)
        .map_err(|_| MatrixError::invalid_param("invite event is invalid"))?;

    let invitee_id: OwnedUserId = serde_json::from_value(
        signed_event
            .get("state_key")
            .ok_or(MatrixError::invalid_param("event had no state_key field"))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::invalid_param("state_key is not a user id"))?;
    if invitee_id.server_name().is_remote() {
        return Err(MatrixError::invalid_param("cannot invite remote users").into());
    }
    let invitee = data::user::get_user(&invitee_id)
        .await
        .map_err(|_| MatrixError::not_found("invitee user not found"))?;
    handler::acl_check(invitee_id.server_name(), &args.room_id).await?;

    let sender_id: OwnedUserId = serde_json::from_value(
        signed_event
            .get("sender")
            .ok_or(MatrixError::invalid_param("event had no sender field"))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::invalid_param("sender is not a user id"))?;
    if sender_id.server_name() != origin {
        return Err(MatrixError::forbidden(
            "cannot send an invite on behalf of another server",
            None,
        )
        .into());
    }
    if let Some(CanonicalJsonValue::String(event_room_id)) = signed_event.get("room_id")
        && event_room_id != args.room_id.as_str()
    {
        return Err(MatrixError::bad_json("event room ID does not match the request path").into());
    }

    // Authenticate the sender's original event before either this server or a Policy
    // Server adds a supplementary signature.
    let version_rules = room::get_version_rules(&body.room_version)?;
    let event_id = event_id_for_pdu(&signed_event, &body.room_version, &version_rules)?;
    let content_was_redacted =
        match crate::server_key::verify_event(&signed_event, &body.room_version).await {
            Ok(Verified::All) => false,
            Ok(Verified::Signatures) => {
                signed_event = crate::core::serde::canonical_json::redact(
                    signed_event,
                    &version_rules.redaction,
                    None,
                )
                .map_err(|e| {
                    MatrixError::invalid_param(format!("invite event redaction failed: {e}"))
                })?;
                true
            }
            Err(e) => {
                return Err(MatrixError::invalid_param(format!(
                    "signature verification failed: {e}"
                ))
                .into());
            }
        };
    if event_id != args.event_id {
        return Err(MatrixError::bad_json("event ID does not match the request path").into());
    }
    let mut auth_event = signed_event.clone();
    auth_event.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.to_string()),
    );

    let verified_invite_state = if requires_full_invite_state(&version_rules) {
        Some(
            verified_v12_invite_state(&body.invite_room_state, &args.room_id, &body.room_version)
                .await?,
        )
    } else {
        None
    };
    ensure_room(&args.room_id, &body.room_version).await?;
    if data::room::is_banned(&args.room_id).await? {
        return Err(MatrixError::forbidden("this room is banned on this homeserver", None).into());
    }

    if conf.block_non_admin_invites && !invitee.is_admin {
        return Err(MatrixError::forbidden("this server does not allow room invites", None).into());
    }

    authenticate_invite_event(&args.room_id, &event_id, &auth_event, &version_rules).await?;

    // `auth_check` resolves the event-time state and takes the room state lock internally.
    // Acquire our write-side lock only after that read-only validation, otherwise invites
    // to a server which is already participating in the room deadlock on the same mutex.
    let state_lock = room::lock_state(&args.room_id).await;

    if content_was_redacted {
        // Keep the sender's original content hash. Re-hashing a redacted copy would make
        // the sender's otherwise-valid signature cover a different `hashes` block.
        crate::server_key::sign_json(&mut signed_event)
            .map_err(|e| MatrixError::invalid_param(format!("failed to sign event: {e}")))?;
    } else {
        crate::server_key::hash_and_sign_event(&mut signed_event, &body.room_version)
            .map_err(|e| MatrixError::invalid_param(format!("failed to sign event: {e}")))?;
    }
    signed_event.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.to_string()),
    );

    // Only contact the Policy Server after room authorization. For a first invite, use
    // the signed policy state supplied alongside the event because no local state exists.
    crate::room::policy::check_invite_event(
        &args.room_id,
        &mut signed_event,
        &body.room_version,
        &body.invite_room_state,
    )
    .await?;
    if event_id_for_pdu(&signed_event, &body.room_version, &version_rules)? != event_id {
        return Err(
            MatrixError::bad_json("supplementary invite signatures changed the event ID").into(),
        );
    }

    let mut invite_state = match &verified_invite_state {
        Some(pdus) => pdus
            .iter()
            .map(|pdu| {
                let raw = to_raw_value(pdu)
                    .map_err(|_| MatrixError::invalid_param("invite state event is invalid"))?;
                stripped_invite_state_event(&raw)
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => body
            .invite_room_state
            .iter()
            .map(|event| stripped_invite_state_event(event))
            .collect::<Result<Vec<_>, _>>()?,
    };

    // If we are active in the room, the remote server will notify us about the join via /send.
    // If we are not in the room, we need to manually
    // record the invited state for client /sync through update_membership(), and
    // send the invite PDU to the relevant appservices.
    // if !room::is_server_joined(&config::get().server_name, &args.room_id)? {
    // Store the same event that is returned to the inviting server. This includes this
    // server's signature and any Policy Server signature added above.
    let event = signed_event.clone();

    let (event_sn, event_guard) = crate::event::ensure_event_sn(&args.room_id, &event_id).await?;
    let pdu = SnPduEvent::from_canonical_object(
        &args.room_id,
        &event_id,
        event_sn,
        event.clone(),
        false,
        false,
        false,
    )
    .map_err(|e| {
        warn!("invalid invite event: {}", e);
        MatrixError::invalid_param("invalid invite event")
    })?;
    invite_state.push(pdu.to_stripped_state_event().await);

    NewDbEvent {
        id: pdu.event_id.to_owned(),
        sn: pdu.event_sn,
        ty: pdu.event_ty.to_string(),
        room_id: pdu.room_id.to_owned(),
        unrecognized_keys: None,
        depth: pdu.depth as i64,
        topological_ordering: pdu.depth as i64,
        stream_ordering: pdu.event_sn,
        origin_server_ts: UnixMillis::now(),
        received_at: None,
        sender_id: Some(pdu.sender.clone()),
        contains_url: false,
        worker_id: None,
        state_key: pdu.state_key.clone(),
        is_outlier: false,
        soft_failed: false,
        is_rejected: false,
        rejection_reason: None,
    }
    .save()
    .await?;
    timeline::append_pdu(&pdu, event, &state_lock).await?;

    // let sender_id: OwnedUserId = serde_json::from_value(
    //     signed_event
    //         .get("sender")
    //         .ok_or(MatrixError::invalid_param("event had no sender field"))?
    //         .clone()
    //         .into(),
    // )
    // .map_err(|_| MatrixError::invalid_param("sender is not a user id"))?;

    diesel::update(
        room_users::table.filter(
            room_users::room_id
                .eq(&args.room_id)
                .and(room_users::user_id.eq(&invitee_id))
                .and(room_users::membership.eq(MembershipState::Invite.to_string())),
        ),
    )
    .set(room_users::state_data.eq(json!(invite_state)))
    .execute(&mut connect().await?)
    .await
    .ok();

    drop(event_guard);
    // }
    drop(state_lock);

    json_ok(InviteUserResBodyV2 {
        event: crate::sending::convert_to_outgoing_federation_event(signed_event).await,
    })
}

/// Verify the full PDUs a version 12 invite must carry as room state.
///
/// Each event must be correctly signed. An event whose signatures are valid but
/// whose content hash no longer matches has been redacted, so it is kept in its
/// redacted form rather than rejecting the invite.
async fn verified_v12_invite_state(
    events: &[Box<RawJsonValue>],
    room_id: &RoomId,
    room_version: &RoomVersionId,
) -> AppResult<Vec<CanonicalJsonObject>> {
    let version_rules = crate::room::get_version_rules(room_version)?;
    let mut has_create = false;
    let mut verified = Vec::with_capacity(events.len());
    for event in events {
        let (pdu, is_create) = parse_v12_invite_state_event(event, room_id, room_version)?;
        if is_create {
            if has_create {
                return Err(MatrixError::missing_param(
                    "invite_room_state contains multiple m.room.create events",
                )
                .into());
            }
            has_create = true;
        }
        let event_type = pdu
            .get("type")
            .and_then(CanonicalJsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        let pdu = match crate::server_key::verify_event(&pdu, room_version).await {
            Ok(Verified::All) => pdu,
            Ok(Verified::Signatures) => {
                warn!(
                    "invite state event {event_type} in {room_id} failed its hash check, redacting"
                );
                canonical_json::redact(pdu, &version_rules.redaction, None).map_err(|_| {
                    MatrixError::missing_param("invite_room_state contains an invalid PDU")
                })?
            }
            Err(e) => {
                warn!(
                    "rejecting invite to {room_id}: invite state event {event_type} failed signature verification: {e}"
                );
                return Err(MatrixError::missing_param(
                    "invite_room_state contains an event with an invalid signature",
                )
                .into());
            }
        };
        verified.push(pdu);
    }
    if !has_create {
        return Err(MatrixError::missing_param("invite_room_state lacks m.room.create").into());
    }
    Ok(verified)
}

/// Version 12 derives the room ID from the create event. Validate the full PDU
/// before stripping it for clients; a matching `type` alone is not evidence of
/// the room's create event.
fn parse_v12_invite_state_event(
    event: &RawJsonValue,
    room_id: &RoomId,
    room_version: &RoomVersionId,
) -> Result<(CanonicalJsonObject, bool), MatrixError> {
    let pdu: CanonicalJsonObject = serde_json::from_str(event.get())
        .map_err(|_| MatrixError::missing_param("invite_room_state contains an invalid PDU"))?;
    if ["auth_events", "prev_events", "signatures"]
        .into_iter()
        .any(|field| !pdu.contains_key(field))
    {
        return Err(MatrixError::missing_param(
            "invite_room_state contains an incomplete PDU",
        ));
    }
    let is_create = pdu.get("type").and_then(CanonicalJsonValue::as_str) == Some("m.room.create");
    if pdu
        .get("state_key")
        .and_then(CanonicalJsonValue::as_str)
        .is_none()
    {
        return Err(MatrixError::missing_param(
            "invite_room_state contains a non-state event",
        ));
    }
    if is_create {
        if pdu.get("state_key").and_then(CanonicalJsonValue::as_str) != Some("")
            || pdu.contains_key("room_id")
        {
            return Err(MatrixError::missing_param(
                "invite_room_state contains an invalid m.room.create event",
            ));
        }
    } else if pdu.get("room_id").and_then(CanonicalJsonValue::as_str) != Some(room_id.as_str()) {
        return Err(MatrixError::missing_param(
            "invite_room_state contains an event from another room",
        ));
    }
    let event_id = crate::event::gen_event_id(&pdu, room_version)
        .map_err(|_| MatrixError::missing_param("invite_room_state contains an invalid PDU"))?;
    crate::event::PduEvent::from_canonical_object(room_id, &event_id, pdu.clone())
        .map_err(|_| MatrixError::missing_param("invite_room_state contains an invalid PDU"))?;
    if is_create && RoomId::new_v2(event_id.localpart()).ok().as_deref() != Some(room_id) {
        return Err(MatrixError::missing_param(
            "invite_room_state create event does not match the room ID",
        ));
    }
    Ok((pdu, is_create))
}

/// Convert federation invite state to the stripped form exposed to clients.
///
/// Current senders provide full PDUs, while pre-v1.16 senders may still send
/// stripped state. Both forms contain these four common fields.
fn stripped_invite_state_event(
    event: &RawJsonValue,
) -> Result<RawJson<AnyStrippedStateEvent>, MatrixError> {
    let event_value: JsonValue = serde_json::from_str(event.get())
        .map_err(|_| MatrixError::invalid_param("invite state event is invalid JSON"))?;
    let event = event_value
        .as_object()
        .ok_or_else(|| MatrixError::invalid_param("invite state event is not an object"))?;

    let field = |name| {
        event.get(name).cloned().ok_or_else(|| {
            MatrixError::invalid_param(format!("invite state event is missing {name}"))
        })
    };
    let stripped = json!({
        "content": field("content")?,
        "sender": field("sender")?,
        "state_key": field("state_key")?,
        "type": field("type")?,
    });

    RawJson::from_value(&stripped)
        .map_err(|_| MatrixError::invalid_param("invite state event is invalid"))
}

/// # `GET /_matrix/federation/v1/make_leave/{roomId}/userId}`
#[endpoint]
async fn make_leave(args: MakeLeaveReqArgs, depot: &mut Depot) -> JsonResult<MakeLeaveResBody> {
    let origin = depot.origin()?;
    if args.user_id.server_name() != origin {
        return Err(
            MatrixError::bad_json("not allowed to leave on behalf of another server").into(),
        );
    }
    if !room::is_room_exists(&args.room_id).await? {
        return Err(MatrixError::forbidden("room is unknown to this server", None).into());
    }

    // ACL check origin
    handler::acl_check(origin, &args.room_id).await?;

    let room_version_id = room::get_version(&args.room_id).await?;
    let state_lock = crate::room::lock_state(&args.room_id).await;

    let (_pdu, mut pdu_json) = PduBuilder::state(
        args.user_id.to_string(),
        &RoomMemberEventContent::new(MembershipState::Leave),
    )
    .hash_sign(&args.user_id, &args.room_id, &room_version_id)
    .await?;
    drop(state_lock);

    // room v3 and above removed the "event_id" field from remote PDU format
    maybe_strip_event_id(&mut pdu_json, &room_version_id);

    json_ok(MakeLeaveResBody {
        room_version: Some(room_version_id),
        event: to_raw_value(&pdu_json).expect("canonicalJson can be serialized to JSON"),
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
    // handler::acl_check(&server_name, &args.room_id)?;

    let room_state =
        crate::federation::membership::send_join_v2(depot.origin()?, &args.room_id, &body.0)
            .await?;

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
    let room_state =
        crate::federation::membership::send_join_v1(depot.origin()?, &args.room_id, &body.0)
            .await?;
    json_ok(SendJoinResBodyV1(room_state))
}

/// #PUT /_matrix/federation/v2/send_leave/{roomId}/{eventId}
///
/// Submits a signed leave event.
#[endpoint]
async fn send_leave(
    depot: &mut Depot,
    args: SendLeaveReqArgsV2,
    body: JsonBody<SendLeaveReqBody>,
) -> EmptyResult {
    let origin = depot.origin()?;
    let body = body.into_inner();

    if !room::is_room_exists(&args.room_id).await? {
        return Err(MatrixError::forbidden("Room is unknown to this server.", None).into());
    }
    handler::acl_check(origin, &args.room_id).await?;

    // We do not add the event_id field to the pdu here because of signature and hashes checks
    let room_version_id = room::get_version(&args.room_id).await?;

    let Ok((event_id, mut value)) =
        crate::event::gen_event_id_canonical_json(&body.0, &room_version_id)
    else {
        // Event could not be converted to canonical json
        return Err(
            MatrixError::invalid_param("could not convert event to canonical json.").into(),
        );
    };

    let event_room_id: OwnedRoomId = serde_json::from_value(
        serde_json::to_value(
            value
                .get("room_id")
                .ok_or_else(|| MatrixError::bad_json("event missing room_id property."))?,
        )
        .expect("CanonicalJson is valid json value"),
    )
    .map_err(|e| MatrixError::bad_json(format!("room_id field is not a valid room id: {e}")))?;

    if event_room_id != args.room_id {
        return Err(
            MatrixError::bad_json("event room_id does not match request path room id").into(),
        );
    }

    let content: RoomMemberEventContent = serde_json::from_value(
        value
            .get("content")
            .ok_or_else(|| MatrixError::bad_json("event missing content property"))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("event content is empty or invalid"))?;

    if content.membership != MembershipState::Leave {
        return Err(MatrixError::bad_json(
            "not allowed to send a non-leave membership event to leave endpoint",
        )
        .into());
    }

    let event_type: StateEventType = serde_json::from_value(
        value
            .get("type")
            .ok_or_else(|| MatrixError::bad_json("event missing type property."))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("event does not have a valid state event type"))?;

    if event_type != StateEventType::RoomMember {
        return Err(MatrixError::invalid_param(
            "not allowed to send non-membership state event to leave endpoint",
        )
        .into());
    }

    // ACL check sender server name
    let sender: OwnedUserId = serde_json::from_value(
        value
            .get("sender")
            .ok_or_else(|| MatrixError::bad_json("event missing sender property"))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("user in sender is invalid"))?;

    handler::acl_check(sender.server_name(), &args.room_id).await?;

    if sender.server_name() != origin {
        return Err(
            MatrixError::bad_json("not allowed to leave on behalf of another server.").into(),
        );
    }

    let state_key: OwnedUserId = serde_json::from_value(
        value
            .get("state_key")
            .ok_or_else(|| MatrixError::invalid_param("event missing state_key property"))?
            .clone()
            .into(),
    )
    .map_err(|_| MatrixError::bad_json("state_key is invalid or not a user id"))?;

    if state_key != sender {
        return Err(MatrixError::bad_json("state_key does not match sender user").into());
    }

    // A synchronous send_leave must report a Policy Server refusal to the sender. The
    // normal incoming-PDU path intentionally turns the same refusal into a soft failure
    // for transaction traffic, which would incorrectly make this endpoint return success.
    crate::room::policy::check_federation_event(
        &args.room_id,
        &event_id,
        &mut value,
        &room_version_id,
    )
    .await?;

    handler::process_incoming_pdu(
        origin,
        &event_id,
        &args.room_id,
        &room_version_id,
        value,
        true,
        false,
    )
    .await?;
    if let Err(e) = crate::sending::send_pdu_room(&args.room_id, &event_id, &[], &[]).await {
        error!("failed to notify leave event: {e}");
    }
    empty_ok()
}

#[cfg(test)]
mod tests {
    use serde_json::value::to_raw_value;
    use serde_json::{Value, json};

    use super::{
        event_id_for_pdu, parse_v12_invite_state_event, requires_full_invite_state,
        stripped_invite_state_event,
    };
    use crate::core::identifiers::{RoomId, RoomVersionId};
    use crate::core::room_version_rules::RoomVersionRules;
    use crate::core::serde::CanonicalJsonObject;

    #[test]
    fn v12_invite_state_requires_the_real_full_create_event() {
        let mut create = json!({
            "auth_events": [],
            "content": { "room_version": "12" },
            "depth": 1,
            "hashes": { "sha256": "hash" },
            "origin_server_ts": 1,
            "prev_events": [],
            "sender": "@alice:example.org",
            "signatures": { "example.org": { "ed25519:key": "sig" } },
            "state_key": "",
            "type": "m.room.create"
        });
        let canonical = serde_json::from_value(create.clone()).unwrap();
        let event_id = crate::event::gen_event_id(&canonical, &RoomVersionId::V12).unwrap();
        let room_id = RoomId::new_v2(event_id.localpart()).unwrap();
        let raw = to_raw_value(&create).unwrap();
        assert!(
            parse_v12_invite_state_event(&raw, &room_id, &RoomVersionId::V12)
                .unwrap()
                .1
        );
        let other_room = RoomId::new_v2("other").unwrap();
        assert!(parse_v12_invite_state_event(&raw, &other_room, &RoomVersionId::V12).is_err());

        create["state_key"] = json!("@alice:example.org");
        let raw = to_raw_value(&create).unwrap();
        assert!(parse_v12_invite_state_event(&raw, &room_id, &RoomVersionId::V12).is_err());

        create["state_key"] = json!("");
        create["room_id"] = json!(room_id);
        let raw = to_raw_value(&create).unwrap();
        assert!(parse_v12_invite_state_event(&raw, &room_id, &RoomVersionId::V12).is_err());

        create.as_object_mut().unwrap().remove("room_id");
        create.as_object_mut().unwrap().remove("signatures");
        let raw = to_raw_value(&create).unwrap();
        assert!(parse_v12_invite_state_event(&raw, &room_id, &RoomVersionId::V12).is_err());

        let stripped = to_raw_value(&json!({
            "content": { "room_version": "12" },
            "sender": "@alice:example.org",
            "state_key": "",
            "type": "m.room.create"
        }))
        .unwrap();
        assert!(parse_v12_invite_state_event(&stripped, &room_id, &RoomVersionId::V12).is_err());
    }

    #[test]
    fn supplementary_invite_signatures_preserve_reference_hash_event_id() {
        let room_version = RoomVersionId::V11;
        let mut event: CanonicalJsonObject = serde_json::from_value(json!({
            "auth_events": [],
            "content": { "membership": "invite" },
            "depth": 1,
            "hashes": { "sha256": "hash" },
            "origin_server_ts": 1,
            "prev_events": [],
            "room_id": "!room:example.org",
            "sender": "@alice:example.org",
            "signatures": { "example.org": { "ed25519:one": "first" } },
            "state_key": "@bob:remote.example",
            "type": "m.room.member"
        }))
        .unwrap();
        let event_id = crate::event::gen_event_id(&event, &room_version).unwrap();

        event.insert("event_id".to_owned(), event_id.to_string().into());
        event
            .get_mut("signatures")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(
                "remote.example".to_owned(),
                serde_json::from_value(json!({ "ed25519:two": "second" })).unwrap(),
            );

        assert_eq!(
            event_id_for_pdu(&event, &room_version, &RoomVersionRules::V11).unwrap(),
            event_id
        );
    }

    #[test]
    fn legacy_room_versions_use_the_explicit_event_id() {
        let event_id = "$opaque:example.org";
        let event: CanonicalJsonObject = serde_json::from_value(json!({
            "event_id": event_id,
            "signatures": { "example.org": { "ed25519:one": "first" } },
            "type": "m.room.member"
        }))
        .unwrap();

        assert_eq!(
            event_id_for_pdu(&event, &RoomVersionId::V1, &RoomVersionRules::V1)
                .unwrap()
                .as_str(),
            event_id
        );
    }

    #[test]
    fn full_invite_state_becomes_mandatory_in_room_version_12() {
        assert!(!requires_full_invite_state(&RoomVersionRules::V11));
        assert!(requires_full_invite_state(&RoomVersionRules::V12));
    }

    #[test]
    fn strips_full_federation_pdu_for_client_state() {
        let pdu = to_raw_value(&json!({
            "auth_events": ["$auth"],
            "content": { "name": "Federated room" },
            "depth": 7,
            "hashes": { "sha256": "hash" },
            "origin_server_ts": 1,
            "prev_events": ["$prev"],
            "room_id": "!room:example.org",
            "sender": "@alice:example.org",
            "signatures": { "example.org": { "ed25519:key": "sig" } },
            "state_key": "",
            "type": "m.room.name"
        }))
        .unwrap();

        let stripped = stripped_invite_state_event(&pdu).unwrap();
        let value: Value = serde_json::from_str(stripped.as_str()).unwrap();

        assert_eq!(
            value,
            json!({
                "content": { "name": "Federated room" },
                "sender": "@alice:example.org",
                "state_key": "",
                "type": "m.room.name"
            })
        );
    }

    #[test]
    fn accepts_legacy_stripped_invite_state() {
        let event = to_raw_value(&json!({
            "content": { "join_rule": "invite" },
            "sender": "@alice:example.org",
            "state_key": "",
            "type": "m.room.join_rules"
        }))
        .unwrap();

        assert!(stripped_invite_state_event(&event).is_ok());
    }

    #[test]
    fn rejects_invite_state_without_required_common_fields() {
        let event = to_raw_value(&json!({
            "content": {},
            "type": "m.room.name"
        }))
        .unwrap();

        assert!(stripped_invite_state_event(&event).is_err());
    }

    #[test]
    fn strips_create_event_for_clients_in_domainless_rooms() {
        let event = to_raw_value(&json!({
            "auth_events": [],
            "content": { "room_version": "12" },
            "depth": 1,
            "origin_server_ts": 1,
            "sender": "@alice:example.org",
            "state_key": "",
            "type": "m.room.create"
        }))
        .unwrap();

        let stripped = stripped_invite_state_event(&event).unwrap();
        let value: Value = serde_json::from_str(stripped.as_str()).unwrap();

        assert!(value.get("origin_server_ts").is_none());
        assert!(value.get("depth").is_none());
        assert_eq!(value["type"], "m.room.create");
    }
}
