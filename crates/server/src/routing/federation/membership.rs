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
    CanonicalJsonObject, CanonicalJsonValue, JsonValue, RawJson, RawJsonValue, to_canonical_object,
};
use crate::core::signatures::Verified;
use crate::data::connect;
use crate::data::room::NewDbEvent;
use crate::data::schema::*;
use crate::event::{PduEvent, handler};
use crate::federation::maybe_strip_event_id;
use crate::room::{ensure_room, timeline};
use crate::{
    DepotExt, EmptyResult, IsRemoteOrLocal, JsonResult, MatrixError, PduBuilder, SnPduEvent,
    config, data, empty_ok, json_ok, membership, room,
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

/// #PUT /_matrix/federation/v2/invite/{room_id}/{event_id}
/// Invites a remote user to a room.
fn invite_state_is_full(invite_room_state: &[Box<RawJsonValue>]) -> bool {
    invite_room_state.iter().all(|raw| {
        serde_json::from_str::<JsonValue>(raw.get())
            .ok()
            .and_then(|event| {
                Some(
                    event.get("auth_events")?.is_array()
                        && event.get("depth")?.is_number()
                        && event.get("origin_server_ts")?.is_number()
                        && event.get("signatures")?.is_object(),
                )
            })
            .unwrap_or(false)
    })
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

async fn authenticate_invite_event(
    room_id: &RoomId,
    event_id: &EventId,
    event: &CanonicalJsonObject,
    room_version: &RoomVersionId,
    invite_room_state: &[Box<RawJsonValue>],
) -> Result<PduEvent, crate::AppError> {
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
    let rules = room::get_version_rules(room_version)?;
    let requires_full_invite_state = requires_full_invite_state(&rules);

    // When we participate in the room, authorise against our trusted event-time state.
    if room::is_server_joined(config::server_name(), room_id).await? {
        handler::auth_check(&incoming, &rules, None).await?;
        return Ok(incoming);
    }

    // Older servers can still send stripped invite state. It cannot be used as an auth
    // snapshot because it has neither event IDs nor signatures, but accepting it remains
    // necessary for federation compatibility through room version 11. Version 12 and
    // later require full room-version PDUs, including the create event (Matrix v1.16).
    // The invite event itself was signature-checked by the caller.
    if !invite_state_is_full(invite_room_state) {
        if requires_full_invite_state {
            return Err(MatrixError::invalid_param(
                "invite room state is not formatted for this room version",
            )
            .into());
        }
        return Ok(incoming);
    }

    // Full invite state can be checked for integrity and room binding, but it is not a
    // trusted current-state snapshot: federation invites do not carry the full auth chain.
    // Do not use these events to authorise each other.
    let mut has_create = false;
    for raw in invite_room_state {
        let original: CanonicalJsonObject = serde_json::from_str(raw.get())
            .map_err(|_| MatrixError::invalid_param("invite state event is invalid JSON"))?;
        if let Some(CanonicalJsonValue::String(event_room_id)) = original.get("room_id")
            && event_room_id != room_id.as_str()
        {
            return Err(MatrixError::invalid_param(
                "invite state event belongs to a different room",
            )
            .into());
        }
        let has_declared_room_id =
            original.get("room_id").and_then(CanonicalJsonValue::as_str) == Some(room_id.as_str());
        let state_event_id = crate::event::gen_event_id(&original, room_version)?;
        // The invite itself is commonly included for presentation to the invitee, but it
        // is not part of the state *before* itself and must never satisfy its own target
        // membership auth lookup.
        if state_event_id == event_id {
            continue;
        }
        match crate::server_key::verify_event(&original, room_version).await {
            Ok(Verified::All) => {}
            Ok(Verified::Signatures) => {
                return Err(MatrixError::invalid_param(
                    "invite state event has an invalid content hash",
                )
                .into());
            }
            Err(e) => {
                return Err(MatrixError::invalid_param(format!(
                    "invite state event signature verification failed: {e}"
                ))
                .into());
            }
        }
        let pdu = PduEvent::from_canonical_object(room_id, &state_event_id, original)
            .map_err(|_| MatrixError::invalid_param("invite state event is not a full PDU"))?;
        let Some(state_key) = pdu.state_key.as_deref() else {
            return Err(
                MatrixError::invalid_param("invite room state contains a non-state event").into(),
            );
        };
        if pdu.event_ty == TimelineEventType::RoomCreate && state_key.is_empty() {
            if has_create {
                return Err(MatrixError::invalid_param(
                    "invite room state contains multiple create events",
                )
                .into());
            }
            has_create = true;
            if rules.authorization.room_create_event_id_as_room_id {
                let derived_room_id = RoomId::new_v2(state_event_id.localpart())?;
                if derived_room_id != room_id {
                    return Err(MatrixError::invalid_param(
                        "invite create event does not match the room ID",
                    )
                    .into());
                }
            } else if !has_declared_room_id {
                return Err(MatrixError::invalid_param(
                    "invite create event has no matching room ID",
                )
                .into());
            }
        }
    }
    if !has_create && requires_full_invite_state {
        return Err(MatrixError::missing_param("invite room state has no create event").into());
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

    ensure_room(&args.room_id, &body.room_version).await?;
    if data::room::is_banned(&args.room_id).await? {
        return Err(MatrixError::forbidden("this room is banned on this homeserver", None).into());
    }

    if conf.block_non_admin_invites && !invitee.is_admin {
        return Err(MatrixError::forbidden("this server does not allow room invites", None).into());
    }

    authenticate_invite_event(
        &args.room_id,
        &event_id,
        &auth_event,
        &body.room_version,
        &body.invite_room_state,
    )
    .await?;

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

    let preserve_full_create = version_rules.authorization.room_create_event_id_as_room_id;
    let mut invite_state = body
        .invite_room_state
        .iter()
        .map(|event| stripped_invite_state_event(event, preserve_full_create))
        .collect::<Result<Vec<_>, _>>()?;

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

/// Convert federation invite state to the stripped form exposed to clients.
///
/// Current senders provide full PDUs, while pre-v1.16 senders may still send
/// stripped state. Both forms contain these four common fields.
fn stripped_invite_state_event(
    event: &RawJsonValue,
    preserve_full_create: bool,
) -> Result<RawJson<AnyStrippedStateEvent>, MatrixError> {
    let event_value: JsonValue = serde_json::from_str(event.get())
        .map_err(|_| MatrixError::invalid_param("invite state event is invalid JSON"))?;
    let event = event_value
        .as_object()
        .ok_or_else(|| MatrixError::invalid_param("invite state event is not an object"))?;

    if preserve_full_create
        && event.get("type").and_then(JsonValue::as_str) == Some("m.room.create")
    {
        return RawJson::from_value(&event_value)
            .map_err(|_| MatrixError::invalid_param("invite state event is invalid"));
    }

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
        event_id_for_pdu, invite_state_is_full, requires_full_invite_state,
        stripped_invite_state_event,
    };
    use crate::core::identifiers::RoomVersionId;
    use crate::core::room_version_rules::RoomVersionRules;
    use crate::core::serde::CanonicalJsonObject;

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

        let stripped = stripped_invite_state_event(&pdu, false).unwrap();
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
        assert!(invite_state_is_full(&[pdu]));
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

        assert!(stripped_invite_state_event(&event, false).is_ok());
        assert!(!invite_state_is_full(&[event]));
    }

    #[test]
    fn rejects_invite_state_without_required_common_fields() {
        let event = to_raw_value(&json!({
            "content": {},
            "type": "m.room.name"
        }))
        .unwrap();

        assert!(stripped_invite_state_event(&event, false).is_err());
    }

    #[test]
    fn preserves_full_create_event_for_domainless_rooms() {
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

        let preserved = stripped_invite_state_event(&event, true).unwrap();
        let value: Value = serde_json::from_str(preserved.as_str()).unwrap();

        assert_eq!(value["origin_server_ts"], 1);
        assert_eq!(value["depth"], 1);
    }
}
