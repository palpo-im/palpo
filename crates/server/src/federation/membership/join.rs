use crate::core::events::StateEventType;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::federation::membership::{RoomStateV1, RoomStateV2};
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonValue, RawJsonValue, to_raw_json_value};
use crate::event::{gen_event_id_canonical_json, handler};
use crate::room::{state, timeline};
use crate::{AppResult, IsRemoteOrLocal, MatrixError, room, sending};

pub async fn send_join_v1(
    origin: &ServerName,
    room_id: &RoomId,
    pdu: &RawJsonValue,
) -> AppResult<RoomStateV1> {
    if !room::room_exists(room_id).await? {
        return Err(MatrixError::not_found("room is unknown to this server.").into());
    }

    handler::acl_check(origin, room_id).await?;

    // We need to return the state prior to joining, let's keep a reference to that here
    let frame_id = room::get_frame_id(room_id, None).await.unwrap_or_default();

    // We do not add the event_id field to the pdu here because of signature and hashes checks
    let room_version_id = room::get_version(room_id).await?;

    let (event_id, mut value) = gen_event_id_canonical_json(pdu, &room_version_id)
        .map_err(|_| MatrixError::invalid_param("could not convert event to canonical json"))?;

    let event_room_id: OwnedRoomId = serde_json::from_value(
        value
            .get("room_id")
            .ok_or_else(|| MatrixError::bad_json("event missing room_id property"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json(format!("room_id field is not a valid room id: {e}")))?;

    if event_room_id != room_id {
        return Err(
            MatrixError::bad_json("event room_id does not match request path room id").into(),
        );
    }

    let event_type: StateEventType = serde_json::from_value(
        value
            .get("type")
            .ok_or_else(|| MatrixError::bad_json("event missing type property"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json(format!("event has invalid state event type: {e}")))?;

    if event_type != StateEventType::RoomMember {
        return Err(MatrixError::bad_json(
            "Not allowed to send non-membership state event to join endpoint.",
        )
        .into());
    }

    let content: RoomMemberEventContent = serde_json::from_value(
        value
            .get("content")
            .ok_or_else(|| MatrixError::bad_json("event missing content property"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json(format!("event content is empty or invalid: {e}")))?;

    if content.membership != MembershipState::Join {
        return Err(MatrixError::bad_json(
            "not allowed to send a non-join membership event to join endpoint",
        )
        .into());
    }

    // ACL check sender user server name
    let sender: OwnedUserId = serde_json::from_value(
        value
            .get("sender")
            .ok_or_else(|| MatrixError::bad_json("event missing sender property"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json(format!("sender property is not a valid user id: {e}")))?;

    if room::user::is_banned(&sender, room_id).await? {
        return Err(MatrixError::forbidden("user is banned from the room", None).into());
    }

    handler::acl_check(sender.server_name(), room_id).await?;

    // check if origin server is trying to send for another server
    if sender.server_name() != origin {
        return Err(MatrixError::forbidden(
            "not allowed to join on behalf of another server",
            None,
        )
        .into());
    }

    let state_key: OwnedUserId = serde_json::from_value(
        value
            .get("state_key")
            .ok_or_else(|| MatrixError::bad_json("event missing state_key property"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json(format!("state key is not a valid user id: {e}")))?;
    if state_key != sender {
        return Err(MatrixError::bad_json("state key does not match sender user").into());
    };

    if let Some(authorising_user) = content.join_authorized_via_users_server {
        use crate::core::RoomVersionId::*;

        if matches!(room_version_id, V1 | V2 | V3 | V4 | V5 | V6 | V7) {
            return Err(MatrixError::invalid_param(
                "room version {room_version_id} does not support restricted rooms but \
				 join_authorised_via_users_server ({authorising_user}) was found in the event",
            )
            .into());
        }

        if !authorising_user.is_local() {
            return Err(MatrixError::invalid_param(
                "cannot authorise membership event through {authorising_user} as they do not \
				 belong to this homeserver",
            )
            .into());
        }

        if !room::user::is_joined(&authorising_user, room_id).await? {
            return Err(MatrixError::invalid_param(
                "authorising user {authorising_user} is not in the room you are trying to join, \
				 they cannot authorise your join",
            )
            .into());
        }

        if !crate::federation::user_can_perform_restricted_join(
            &state_key,
            room_id,
            &room_version_id,
            None,
        )
        .await?
        {
            return Err(MatrixError::unable_to_authorize_join(
                "joining user did not pass restricted room's rules",
            )
            .into());
        }
    }

    crate::server_key::hash_and_sign_event(&mut value, &room_version_id)
        .map_err(|e| MatrixError::invalid_param(format!("failed to sign send_join event: {e}")))?;

    let origin: OwnedServerName = serde_json::from_value(
        serde_json::to_value(
            value
                .get("origin")
                .ok_or(MatrixError::invalid_param("event needs an origin field"))?,
        )
        .expect("CanonicalJson is valid json value"),
    )
    .map_err(|_| MatrixError::invalid_param("origin field is invalid"))?;

    handler::process_incoming_pdu(
        &origin,
        &event_id,
        room_id,
        &room_version_id,
        value.clone(),
        true,
        false,
    )
    .await?;

    let state_ids = state::get_full_state_ids(frame_id).await?;
    let mut state = Vec::new();
    for (_, id) in state_ids.iter() {
        if let Some(pdu_json) = timeline::get_pdu_json(id).await.ok().flatten() {
            state.push(crate::sending::convert_to_outgoing_federation_event(pdu_json).await);
        }
    }
    let auth_chain_ids =
        room::auth_chain::get_auth_chain_ids(room_id, state_ids.values().map(|id| &**id)).await?;
    let mut auth_chain = Vec::new();
    for id in auth_chain_ids.into_iter() {
        if let Some(pdu_json) = timeline::get_pdu_json(&id).await.ok().flatten() {
            auth_chain.push(crate::sending::convert_to_outgoing_federation_event(pdu_json).await);
        }
    }

    if let Err(e) = sending::send_pdu_room(room_id, &event_id, &[], &[origin.to_owned()]).await {
        error!("failed to notify user joined to servers: {e}");
    }

    Ok(RoomStateV1 {
        auth_chain,
        state,
        event: to_raw_json_value(&CanonicalJsonValue::Object(value)).ok(),
        // event: None,
    })
}
pub async fn send_join_v2(
    origin: &ServerName,
    room_id: &RoomId,
    pdu: &RawJsonValue,
) -> AppResult<RoomStateV2> {
    // let sender_servername = body.sender_servername.as_ref().expect("server is authenticated");

    let RoomStateV1 {
        auth_chain,
        state,
        event,
    } = send_join_v1(origin, room_id, pdu).await?;
    let room_state = RoomStateV2 {
        members_omitted: false,
        auth_chain,
        state,
        event,
        servers_in_room: None,
    };

    Ok(room_state)
}
