use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::iter::once;
use std::sync::Arc;
use std::time::Duration;

use diesel::prelude::*;
use salvo::http::StatusError;
use tokio::sync::RwLock;

use crate::core::client::membership::{JoinRoomResBody, ThirdPartySigned};
use crate::core::events::room::join_rules::{AllowRule, JoinRule, RoomJoinRulesEventContent};
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::federation::membership::{
    InviteUserResBodyV2, MakeJoinReqArgs, MakeLeaveResBody, SendJoinArgs, SendJoinResBodyV2, SendLeaveReqBody,
    make_leave_request,
};
use crate::core::identifiers::*;
use crate::core::serde::{
    CanonicalJsonObject, CanonicalJsonValue, RawJsonValue, to_canonical_value, to_raw_json_value,
};
use crate::core::{OwnedServerName, ServerName, UnixMillis, federation};

use crate::appservice::RegistrationInfo;
use crate::event::{DbEventData, NewDbEvent, PduBuilder, PduEvent, gen_event_id_canonical_json};
use crate::federation::maybe_strip_event_id;
use crate::membership::federation::membership::{
    InviteUserReqArgs, InviteUserReqBodyV2, MakeJoinResBody, RoomStateV1, RoomStateV2, SendJoinReqBody,
    SendLeaveReqArgsV2, send_leave_request_v2,
};
use crate::membership::state::DeltaInfo;
use crate::room::state::{self, CompressedEvent};
use crate::user::DbUser;
use crate::{
    AppError, AppResult, GetUrlOrigin, IsRemoteOrLocal, MatrixError, Seqnum, SigningKeys, db, diesel_exists, schema::*,
};

pub async fn send_join_v1(origin: &ServerName, room_id: &RoomId, pdu: &RawJsonValue) -> AppResult<RoomStateV1> {
    if !crate::room::room_exists(room_id)? {
        return Err(MatrixError::not_found("Room is unknown to this server.").into());
    }

    crate::event::handler::acl_check(origin, room_id)?;

    // We need to return the state prior to joining, let's keep a reference to that here
    let frame_id =
        crate::room::state::get_room_frame_id(room_id, None)?.ok_or(MatrixError::not_found("Pdu state not found."))?;

    // We do not add the event_id field to the pdu here because of signature and hashes checks
    let room_version_id = crate::room::state::get_room_version(room_id)?;

    let (event_id, mut value) = match gen_event_id_canonical_json(pdu, &room_version_id) {
        Ok(t) => t,
        Err(_) => {
            // Event could not be converted to canonical json
            return Err(MatrixError::invalid_param("Could not convert event to canonical json.").into());
        }
    };

    let event_room_id: OwnedRoomId = serde_json::from_value(
        value
            .get("room_id")
            .ok_or_else(|| MatrixError::bad_json("Event missing room_id property."))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json("room_id field is not a valid room ID: {e}"))?;

    if event_room_id != room_id {
        return Err(MatrixError::bad_json("Event room_id does not match request path room ID.").into());
    }

    let event_type: StateEventType = serde_json::from_value(
        value
            .get("type")
            .ok_or_else(|| MatrixError::bad_json("Event missing type property."))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json("Event has invalid state event type: {e}"))?;

    if event_type != StateEventType::RoomMember {
        return Err(MatrixError::bad_json("Not allowed to send non-membership state event to join endpoint.").into());
    }

    let content: RoomMemberEventContent = serde_json::from_value(
        value
            .get("content")
            .ok_or_else(|| MatrixError::bad_json("Event missing content property"))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json("Event content is empty or invalid: {e}"))?;

    if content.membership != MembershipState::Join {
        return Err(MatrixError::bad_json("Not allowed to send a non-join membership event to join endpoint.").into());
    }

    // ACL check sender user server name
    let sender: OwnedUserId = serde_json::from_value(
        value
            .get("sender")
            .ok_or_else(|| MatrixError::bad_json("Event missing sender property."))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json("sender property is not a valid user ID: {e}"))?;

    crate::event::handler::acl_check(sender.server_name(), room_id)?;

    // check if origin server is trying to send for another server
    if sender.server_name() != origin {
        return Err(MatrixError::forbidden("Not allowed to join on behalf of another server.").into());
    }

    let state_key: OwnedUserId = serde_json::from_value(
        value
            .get("state_key")
            .ok_or_else(|| MatrixError::bad_json("Event missing state_key property."))?
            .clone()
            .into(),
    )
    .map_err(|e| MatrixError::bad_json("State key is not a valid user ID: {e}"))?;
    if state_key != sender {
        return Err(MatrixError::bad_json("State key does not match sender user.").into());
    };

    if let Some(authorising_user) = content.join_authorized_via_users_server {
        use crate::core::RoomVersionId::*;

        if matches!(room_version_id, V1 | V2 | V3 | V4 | V5 | V6 | V7) {
            return Err(MatrixError::invalid_param(
                "Room version {room_version_id} does not support restricted rooms but \
				 join_authorised_via_users_server ({authorising_user}) was found in the event.",
            )
            .into());
        }

        if !crate::user::is_local(&authorising_user) {
            return Err(MatrixError::invalid_param(
                "Cannot authorise membership event through {authorising_user} as they do not \
				 belong to this homeserver",
            )
            .into());
        }

        if !crate::room::is_joined(&authorising_user, room_id)? {
            return Err(MatrixError::invalid_param(
                "Authorising user {authorising_user} is not in the room you are trying to join, \
				 they cannot authorise your join.",
            )
            .into());
        }

        if !crate::federation::user_can_perform_restricted_join(&state_key, room_id, &room_version_id).await? {
            return Err(
                MatrixError::unable_to_authorize_join("Joining user did not pass restricted room's rules.").into(),
            );
        }
    }

    crate::server_key::hash_and_sign_event(&mut value, &room_version_id)
        .map_err(|e| MatrixError::invalid_param("Failed to sign send_join event: {e}"))?;

    let origin: OwnedServerName = serde_json::from_value(
        serde_json::to_value(
            value
                .get("origin")
                .ok_or(MatrixError::invalid_param("Event needs an origin field."))?,
        )
        .expect("CanonicalJson is valid json value"),
    )
    .map_err(|_| MatrixError::invalid_param("Origin field is invalid."))?;

    // let mutex = Arc::clone(
    //     crate::ROOMID_MUTEX_FEDERATION
    //         .write()
    //         .await
    //         .entry(room_id.to_owned())
    //         .or_default(),
    // );
    // let mutex_lock = mutex.lock().await;
    crate::event::handler::handle_incoming_pdu(&origin, &event_id, room_id, value, true).await?;
    // drop(mutex_lock);

    let state_ids = crate::room::state::get_full_state_ids(frame_id)?;

    let mut state = state_ids
        .iter()
        .filter_map(|(_, id)| crate::room::timeline::get_pdu_json(id).ok().flatten())
        .map(PduEvent::convert_to_outgoing_federation_event)
        .collect();

    let auth_chain_ids = crate::room::auth_chain::get_auth_chain_ids(room_id, state_ids.values().map(|id| &**id))?;
    let auth_chain = auth_chain_ids
        .into_iter()
        .filter_map(|id| crate::room::timeline::get_pdu_json(&id).ok().flatten())
        .map(PduEvent::convert_to_outgoing_federation_event)
        .collect();

    //     let join_rules_event = crate::room::state::get_state(room_id, &StateEventType::RoomJoinRules, "", None)?;

    //     let join_rules_event_content: Option<RoomJoinRulesEventContent> = join_rules_event
    //         .as_ref()
    //         .map(|join_rules_event| {
    //             serde_json::from_str(join_rules_event.content.get()).map_err(|e| {
    //                 warn!("Invalid join rules event: {}", e);
    //                 AppError::public("Invalid join rules event in db.")
    //             })
    //         })
    //         .transpose()?;

    //     if let Some(join_rules_event_content) = join_rules_event_content {
    //         if matches!(
    //             join_rules_event_content.join_rule,
    //             JoinRule::Restricted { .. } | JoinRule::KnockRestricted { .. }
    //         ) {
    //             return Err(MatrixError::unable_to_authorize_join("Palpo does not support restricted rooms yet.").into());
    //         }
    //     }

    //     // let pub_key_map = RwLock::new(BTreeMap::new());
    //     // let mut auth_cache = EventMap::new();

    crate::sending::send_pdu_room(room_id, &event_id)?;
    Ok(RoomStateV1 {
        auth_chain,
        state,
        event: None, // TODO: handle restricted joins
    })
}
pub async fn send_join_v2(origin: &ServerName, room_id: &RoomId, pdu: &RawJsonValue) -> AppResult<RoomStateV2> {
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
pub async fn join_room(
    user: &DbUser,
    room_id: &RoomId,
    reason: Option<String>,
    servers: &[OwnedServerName],
    _third_party_signed: Option<&ThirdPartySigned>,
    appservice: Option<&RegistrationInfo>,
) -> AppResult<JoinRoomResBody> {
    // TODO: state lock
    if user.is_guest && appservice.is_none() && !crate::room::state::guest_can_join(room_id)? {
        return Err(MatrixError::forbidden("Guests are not allowed to join this room").into());
    }

    let user_id = &user.id;
    if crate::room::is_joined(user_id, room_id)? {
        return Ok(JoinRoomResBody {
            room_id: room_id.into(),
        });
    }

    let local_join = crate::room::is_server_in_room(crate::server_name(), room_id)?
        || servers.is_empty()
        || (servers.len() == 1 && servers[0] == crate::server_name());
    // Ask a remote server if we are not participating in this room
    if !local_join {
        info!("Joining {room_id} over federation.");

        let (make_join_response, remote_server) = make_join_request(user_id, room_id, servers).await?;

        info!("make_join finished");

        let room_version_id = match make_join_response.room_version {
            Some(room_version) if crate::supported_room_versions().contains(&room_version) => room_version,
            _ => return Err(AppError::public("Room version is not supported")),
        };

        let mut join_event_stub: CanonicalJsonObject = serde_json::from_str(make_join_response.event.get())
            .map_err(|_| AppError::public("Invalid make_join event json received from server."))?;

        let join_authorized_via_users_server = join_event_stub
            .get("content")
            .map(|s| s.as_object()?.get("join_authorised_via_users_server")?.as_str())
            .and_then(|s| OwnedUserId::try_from(s.unwrap_or_default()).ok());

        // TODO: Is origin needed?
        join_event_stub.insert(
            "origin".to_owned(),
            CanonicalJsonValue::String(crate::server_name().as_str().to_owned()),
        );
        join_event_stub.insert(
            "origin_server_ts".to_owned(),
            CanonicalJsonValue::Integer(UnixMillis::now().get() as i64),
        );
        join_event_stub.insert(
            "content".to_owned(),
            to_canonical_value(RoomMemberEventContent {
                membership: MembershipState::Join,
                display_name: crate::user::display_name(user_id)?,
                avatar_url: crate::user::avatar_url(user_id)?,
                is_direct: None,
                third_party_invite: None,
                blurhash: crate::user::blurhash(user_id)?,
                reason,
                join_authorized_via_users_server,
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

        let body = SendJoinReqBody(PduEvent::convert_to_outgoing_federation_event(join_event.clone()));
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

        let send_join_response = crate::sending::send_federation_request(&remote_server, send_join_request)
            .await?
            .json::<SendJoinResBodyV2>()
            .await?;
        // let send_join_response = sending::put(remote_server.build_url(&format!(
        //     "/federation/v2/send_join/{room_id}/{event_id}?omit_members=false"
        // ))?)
        // .stuff(SendJoinReqBodyV2 {
        //     pdu: PduEvent::convert_to_outgoing_federation_event(join_event.clone()),
        // })?
        // .send::<SendJoinResBody>()
        // .await?;

        info!("send_join finished");

        if let Some(signed_raw) = &send_join_response.0.event {
            info!(
                "There is a signed event. This room is probably using restricted joins. Adding signature to our event"
            );
            let (signed_event_id, signed_value) = match gen_event_id_canonical_json(signed_raw, &room_version_id) {
                Ok(t) => t,
                Err(_) => {
                    // Event could not be converted to canonical json
                    return Err(MatrixError::invalid_param("Could not convert event to canonical json.").into());
                }
            };

            if signed_event_id != event_id {
                return Err(MatrixError::invalid_param("Server sent event with wrong event id").into());
            }

            match signed_value["signatures"]
                .as_object()
                .ok_or(MatrixError::invalid_param("Server sent invalid signatures type"))
                .and_then(|e| {
                    e.get(remote_server.as_str())
                        .ok_or(MatrixError::invalid_param("Server did not send its signature"))
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

        crate::room::ensure_room(room_id, user_id)?;

        info!("Parsing join event");
        let parsed_join_pdu = PduEvent::from_canonical_object(&event_id, crate::next_sn()?, join_event.clone())
            .map_err(|e| {
                warn!("Invalid PDU in send_join response: {}", e);
                AppError::public("Invalid join event PDU.")
            })?;
        diesel::insert_into(events::table)
            .values(NewDbEvent::from_canonical_json(
                &event_id,
                Some(parsed_join_pdu.event_sn),
                &join_event,
            )?)
            .on_conflict_do_nothing()
            .execute(&mut *db::connect()?)?;

        let mut state = HashMap::new();
        let pub_key_map = RwLock::new(BTreeMap::new());

        info!("Acquiring server signing keys for response events");
        let resp_events = &send_join_response.0;
        let resp_state = &resp_events.state;
        let resp_auth = &resp_events.auth_chain;
        crate::server_key::acquire_events_pubkeys(resp_auth.iter().chain(resp_state.iter())).await;

        info!("Going through send_join response room_state");
        for result in send_join_response
            .0
            .state
            .iter()
            .map(|pdu| validate_and_add_event_id(pdu, &room_version_id, &pub_key_map))
        {
            let (event_id, value) = match result.await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let pdu = PduEvent::from_canonical_object(&event_id, crate::next_sn()?, value.clone()).map_err(|e| {
                warn!("Invalid PDU in send_join response: {} {:?}", e, value);
                AppError::public("Invalid PDU in send_join response.")
            })?;

            diesel::insert_into(events::table)
                .values(NewDbEvent::from_canonical_json(&event_id, Some(pdu.event_sn), &value)?)
                .on_conflict_do_nothing()
                .execute(&mut *db::connect()?)?;

            let event_data = DbEventData {
                event_id: event_id.to_owned(),
                event_sn: pdu.event_sn,
                room_id: pdu.room_id.clone(),
                internal_metadata: None,
                json_data: serde_json::to_value(&value)?,
                format_version: None,
            };
            diesel::insert_into(event_datas::table)
                .values(&event_data)
                .on_conflict((event_datas::event_id, event_datas::event_sn))
                .do_update()
                .set(&event_data)
                .execute(&mut db::connect()?)?;

            if let Some(state_key) = &pdu.state_key {
                let state_key_id = crate::room::state::ensure_field_id(&pdu.event_ty.to_string().into(), state_key)?;
                state.insert(state_key_id, pdu.event_id.clone());
            }
        }

        info!("Going through send_join response auth_chain");
        for result in send_join_response
            .0
            .auth_chain
            .iter()
            .map(|pdu| validate_and_add_event_id(pdu, &room_version_id, &pub_key_map))
        {
            let (event_id, value) = match result.await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let db_event = NewDbEvent::from_canonical_json(&event_id, None, &value)?;
            diesel::insert_into(events::table)
                .values(&db_event)
                .on_conflict_do_nothing()
                .execute(&mut *db::connect()?)?;
            let event_sn = events::table
                .find(&event_id)
                .select(events::sn)
                .first::<i64>(&mut *db::connect()?)?;
            let event_data = DbEventData {
                event_id: event_id.to_owned(),
                event_sn,
                room_id: db_event.room_id.clone(),
                internal_metadata: None,
                json_data: serde_json::to_value(&value)?,
                format_version: None,
            };
            diesel::insert_into(event_datas::table)
                .values(&event_data)
                .on_conflict((event_datas::event_id, event_datas::event_sn))
                .do_update()
                .set(&event_data)
                .execute(&mut db::connect()?)?;
        }

        info!("Running send_join auth check");
        // TODO: Authcheck
        // if !event_auth::auth_check(
        //     &RoomVersion::new(&room_version_id).expect("room version is supported"),
        //     &parsed_join_pdu,
        //     None::<PduEvent>, // TODO: third party invite
        //     |k, s| {
        //         crate::room::timeline::get_pdu(
        //             state.get(&crate::room::state::ensure_field_id(&k.to_string().into(), s).ok()?)?,
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
        } = crate::room::state::save_state(
            room_id,
            Arc::new(
                state
                    .into_iter()
                    .map(|(k, event_id)| {
                        let event_sn = crate::event::get_event_sn(&event_id)?;
                        let point_id = crate::room::state::ensure_point(room_id, &event_id, event_sn)?;
                        Ok(CompressedEvent::new(k, point_id))
                    })
                    .collect::<AppResult<_>>()?,
            ),
        )?;

        crate::room::state::force_state(room_id, frame_id, appended, disposed)?;

        info!("Updating joined counts for new room");
        crate::room::update_room_servers(room_id)?;
        crate::room::update_room_currents(room_id)?;

        // We append to state before appending the pdu, so we don't have a moment in time with the
        // pdu without it's state. This is okay because append_pdu can't fail.
        let state_hash_after_join = crate::room::state::append_to_state(&parsed_join_pdu)?;

        info!("Appending new room join event");
        crate::room::timeline::append_pdu(&parsed_join_pdu, join_event, once(parsed_join_pdu.event_id.borrow()))
            .unwrap();

        info!("Setting final room state for new room");
        // We set the room state after inserting the pdu, so that we never have a moment in time
        // where events in the current room state do not exist
        crate::room::state::set_room_state(room_id, state_hash_after_join)?;
    } else {
        info!("We can join locally");
        let join_rules_event = crate::room::state::get_state(room_id, &StateEventType::RoomJoinRules, "", None)?;
        // let power_levels_event = crate::room::state::get_state(room_id, &StateEventType::RoomPowerLevels, "", None)?;

        let join_rules_event_content: Option<RoomJoinRulesEventContent> = join_rules_event
            .as_ref()
            .map(|join_rules_event| {
                serde_json::from_str(join_rules_event.content.get()).map_err(|e| {
                    warn!("Invalid join rules event: {}", e);
                    AppError::public("Invalid join rules event in database.")
                })
            })
            .transpose()?;

        let restriction_rooms = match join_rules_event_content {
            Some(RoomJoinRulesEventContent {
                join_rule: JoinRule::Restricted(restricted),
            })
            | Some(RoomJoinRulesEventContent {
                join_rule: JoinRule::KnockRestricted(restricted),
            }) => restricted
                .allow
                .into_iter()
                .filter_map(|a| match a {
                    AllowRule::RoomMembership(r) => Some(r.room_id),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        let authorized_user = if restriction_rooms
            .iter()
            .any(|restriction_room_id| crate::room::is_joined(user_id, restriction_room_id).unwrap_or(false))
        {
            let mut auth_user = None;
            for joined_user in crate::room::get_joined_users(room_id, None)? {
                if joined_user.server_name() == crate::server_name()
                    && state::user_can_invite(room_id, &joined_user, user_id).unwrap_or(false)
                {
                    auth_user = Some(joined_user);
                    break;
                }
            }
            auth_user
        } else {
            None
        };

        let event = RoomMemberEventContent {
            membership: MembershipState::Join,
            display_name: crate::user::display_name(user_id)?,
            avatar_url: crate::user::avatar_url(user_id)?,
            is_direct: None,
            third_party_invite: None,
            blurhash: crate::user::blurhash(user_id)?,
            reason: reason.clone(),
            join_authorized_via_users_server: authorized_user,
        };

        // Try normal join first
        let error = match crate::room::timeline::build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomMember,
                content: to_raw_json_value(&event).expect("event is valid, we just created it"),
                state_key: Some(user_id.to_string()),
                ..Default::default()
            },
            user_id,
            room_id,
        ) {
            Ok(_event_id) => return Ok(JoinRoomResBody::new(room_id.to_owned())),
            Err(e) => e,
        };

        if !restriction_rooms.is_empty() && servers.iter().filter(|s| *s != crate::server_name()).count() > 0 {
            info!(
                "We couldn't do the join locally, maybe federation can help to satisfy the restricted join requirements"
            );
            let (make_join_response, remote_server) = make_join_request(user_id, room_id, servers).await?;

            let room_version_id = match make_join_response.room_version {
                Some(room_version_id) if crate::supported_room_versions().contains(&room_version_id) => room_version_id,
                _ => return Err(AppError::public("Room version is not supported")),
            };
            let mut join_event_stub: CanonicalJsonObject = serde_json::from_str(make_join_response.event.get())
                .map_err(|_| AppError::public("Invalid make_join event json received from server."))?;
            let join_authorized_via_users_server = join_event_stub
                .get("content")
                .map(|s| s.as_object()?.get("join_authorised_via_users_server")?.as_str())
                .and_then(|s| OwnedUserId::try_from(s.unwrap_or_default()).ok());
            // TODO: Is origin needed?
            join_event_stub.insert(
                "origin".to_owned(),
                CanonicalJsonValue::String(crate::server_name().as_str().to_owned()),
            );
            join_event_stub.insert(
                "origin_server_ts".to_owned(),
                CanonicalJsonValue::Integer(UnixMillis::now().get() as i64),
            );
            join_event_stub.insert(
                "content".to_owned(),
                to_canonical_value(RoomMemberEventContent {
                    membership: MembershipState::Join,
                    display_name: crate::user::display_name(user_id)?,
                    avatar_url: crate::user::avatar_url(user_id)?,
                    is_direct: None,
                    third_party_invite: None,
                    blurhash: crate::user::blurhash(user_id)?,
                    reason,
                    join_authorized_via_users_server,
                })
                .expect("event is valid, we just created it"),
            );

            // We don't leave the event id in the pdu because that's only allowed in v1 or v2 rooms
            join_event_stub.remove("event_id");

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
            let join_event = join_event_stub;

            let send_join_request = crate::core::federation::membership::send_join_request(
                &room_id.server_name().map_err(AppError::public)?.origin().await,
                SendJoinArgs {
                    room_id: room_id.to_owned(),
                    event_id: event_id.to_owned(),
                    omit_members: false,
                },
                SendJoinReqBody(PduEvent::convert_to_outgoing_federation_event(join_event.clone())),
            )?
            .into_inner();

            let send_join_response = crate::sending::send_federation_request(&remote_server, send_join_request)
                .await?
                .json::<SendJoinResBodyV2>()
                .await?;

            if let Some(signed_raw) = send_join_response.0.event {
                let (signed_event_id, signed_value) = match gen_event_id_canonical_json(&signed_raw, &room_version_id) {
                    Ok(t) => t,
                    Err(_) => {
                        // Event could not be converted to canonical json
                        return Err(MatrixError::invalid_param("Could not convert event to canonical json.").into());
                    }
                };

                if signed_event_id != event_id {
                    return Err(MatrixError::invalid_param("Server sent event with wrong event id").into());
                }

                // let pub_key_map = RwLock::new(BTreeMap::new());
                crate::event::handler::handle_incoming_pdu(
                    &remote_server,
                    &signed_event_id,
                    room_id,
                    signed_value,
                    true,
                    // &pub_key_map,
                )
                .await?;
            } else {
                return Err(error);
            }
        } else {
            return Err(error);
        }
    }

    Ok(JoinRoomResBody::new(room_id.to_owned()))
}

async fn make_join_request(
    user_id: &UserId,
    room_id: &RoomId,
    servers: &[OwnedServerName],
) -> AppResult<(MakeJoinResBody, OwnedServerName)> {
    let mut make_join_res_body_and_server = Err(StatusError::bad_request()
        .brief("No server available to assist in joining.")
        .into());

    for remote_server in servers {
        if remote_server == crate::server_name() {
            continue;
        }
        info!("Asking {remote_server} for make_join");

        let make_join_request = crate::core::federation::membership::make_join_request(
            &remote_server.origin().await,
            MakeJoinReqArgs {
                room_id: room_id.to_owned(),
                user_id: user_id.to_owned(),
                ver: crate::supported_room_versions(),
            },
        )?
        .into_inner();
        let make_join_response = crate::sending::send_federation_request(remote_server, make_join_request).await;
        match make_join_response {
            Ok(make_join_response) => {
                let res_body = make_join_response.json::<MakeJoinResBody>().await;
                make_join_res_body_and_server = res_body.map(|r| (r, remote_server.clone())).map_err(Into::into);
            }
            Err(e) => {
                tracing::error!("make_join_request failed: {e:?}");
            }
        }

        if make_join_res_body_and_server.is_ok() {
            break;
        }
    }

    make_join_res_body_and_server
}

async fn validate_and_add_event_id(
    pdu: &RawJsonValue,
    room_version: &RoomVersionId,
    pub_key_map: &RwLock<BTreeMap<String, SigningKeys>>,
) -> AppResult<(OwnedEventId, CanonicalJsonObject)> {
    let mut value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
        error!("Invalid PDU in server response: {:?}: {:?}", pdu, e);
        AppError::public("Invalid PDU in server response")
    })?;
    let event_id = EventId::parse(format!(
        "${}",
        crate::core::signatures::reference_hash(&value, room_version).expect("palpo can calculate reference hashes")
    ))
    .expect("palpo's reference hash~es are valid event ids");

    // TODO
    // let back_off = |id| match crate::BAD_EVENT_RATE_LIMITER.write().unwrap().entry(id) {
    //     Entry::Vacant(e) => {
    //         e.insert((Instant::now(), 1));
    //     }
    //     Entry::Occupied(mut e) => *e.get_mut() = (Instant::now(), e.get().1 + 1),
    // };

    if let Some((time, tries)) = crate::BAD_EVENT_RATE_LIMITER.read().unwrap().get(&event_id) {
        // Exponential backoff
        let mut min_elapsed_duration = Duration::from_secs(30) * (*tries) * (*tries);
        if min_elapsed_duration > Duration::from_secs(60 * 60 * 24) {
            min_elapsed_duration = Duration::from_secs(60 * 60 * 24);
        }

        if time.elapsed() < min_elapsed_duration {
            debug!("Backing off from {}", event_id);
            return Err(AppError::public("bad event, still backing off"));
        }
    }

    let origin_server_ts = value.get("origin_server_ts").ok_or_else(|| {
        error!("Invalid PDU, no origin_server_ts field");
        MatrixError::missing_param("Invalid PDU, no origin_server_ts field")
    })?;

    let origin_server_ts: UnixMillis = {
        let ts = origin_server_ts
            .as_integer()
            .ok_or_else(|| MatrixError::invalid_param("origin_server_ts must be an integer"))?;

        UnixMillis(
            ts.try_into()
                .map_err(|_| MatrixError::invalid_param("Time must be after the unix epoch"))?,
        )
    };

    let unfiltered_keys = (*pub_key_map.read().await).clone();

    let keys = crate::filter_keys_server_map(unfiltered_keys, origin_server_ts, room_version);

    // TODO
    // if let Err(e) = crate::core::signatures::verify_event(&keys, &value, room_version) {
    //     warn!("Event {} failed verification {:?} {}", event_id, pdu, e);
    //     back_off(event_id);
    //     return Err(AppError::public("Event failed verification."));
    // }

    value.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.as_str().to_owned()),
    );

    Ok((event_id, value))
}

pub(crate) async fn invite_user(
    inviter_id: &UserId,
    invitee_id: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    is_direct: bool,
) -> AppResult<()> {
    if invitee_id.server_name().is_remote() {
        let (pdu, pdu_json, invite_room_state) = {
            let content = RoomMemberEventContent {
                avatar_url: None,
                display_name: None,
                is_direct: Some(is_direct),
                membership: MembershipState::Invite,
                third_party_invite: None,
                blurhash: None,
                reason,
                join_authorized_via_users_server: None,
            };

            let (pdu, pdu_json) = crate::room::timeline::create_hash_and_sign_event(
                PduBuilder::state(invitee_id.to_string(), &content),
                inviter_id,
                room_id,
            )?;

            let invite_room_state = crate::room::state::calculate_invite_state(&pdu)?;

            (pdu, pdu_json, invite_room_state)
        };

        let room_version_id = crate::room::state::get_room_version(room_id)?;

        let invite_request = crate::core::federation::membership::invite_user_request_v2(
            &invitee_id.server_name().origin().await,
            InviteUserReqArgs {
                room_id: room_id.to_owned(),
                event_id: (&*pdu.event_id).to_owned(),
            },
            InviteUserReqBodyV2 {
                room_version: room_version_id.clone(),
                event: PduEvent::convert_to_outgoing_federation_event(pdu_json.clone()),
                invite_room_state,
                via: crate::room::state::servers_route_via(room_id).ok(),
            },
        )?
        .into_inner();
        let send_join_response = crate::sending::send_federation_request(invitee_id.server_name(), invite_request)
            .await?
            .json::<InviteUserResBodyV2>()
            .await?;

        // We do not add the event_id field to the pdu here because of signature and hashes checks
        let (event_id, value) =
            gen_event_id_canonical_json(&send_join_response.event, &room_version_id).map_err(|e| {
                tracing::error!("Could not convert event to canonical json: {e}");
                MatrixError::invalid_param("Could not convert event to canonical json.")
            })?;

        if *pdu.event_id != *event_id {
            warn!(
                "Server {} changed invite event, that's not allowed in the spec: ours: {:?}, theirs: {:?}",
                invitee_id.server_name(),
                pdu_json,
                value
            );
            return Err(MatrixError::bad_json(format!(
                "Server `{}` sent event with wrong event ID",
                invitee_id.server_name()
            ))
            .into());
        }

        let origin: OwnedServerName = serde_json::from_value(
            serde_json::to_value(
                value
                    .get("origin")
                    .ok_or(MatrixError::bad_json("Event needs an origin field."))?,
            )
            .expect("CanonicalJson is valid json value"),
        )
        .map_err(|e| MatrixError::bad_json(format!("Origin field in event is not a valid server name: {e}")))?;

        crate::event::handler::handle_incoming_pdu(&origin, &event_id, room_id, value, true).await?;
        return crate::sending::send_pdu_room(room_id, &event_id);
    }

    if !crate::room::is_joined(inviter_id, room_id)? {
        return Err(MatrixError::forbidden("You must be joined in the room you are trying to invite from.").into());
    }

    crate::room::timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_json_value(&RoomMemberEventContent {
                membership: MembershipState::Invite,
                display_name: crate::user::display_name(invitee_id)?,
                avatar_url: crate::user::avatar_url(invitee_id)?,
                is_direct: Some(is_direct),
                third_party_invite: None,
                blurhash: crate::user::blurhash(invitee_id)?,
                reason,
                join_authorized_via_users_server: None,
            })
            .expect("event is valid, we just created it"),
            state_key: Some(invitee_id.to_string()),
            ..Default::default()
        },
        inviter_id,
        room_id,
    )?;

    Ok(())
}

// Make a user leave all their joined rooms
pub async fn leave_all_rooms(user_id: &UserId) -> AppResult<()> {
    let all_room_ids = crate::user::joined_rooms(user_id, 0)?
        .into_iter()
        .chain(crate::user::invited_rooms(user_id, 0)?.into_iter().map(|t| t.0))
        .collect::<Vec<_>>();
    for room_id in all_room_ids {
        leave_room(user_id, &room_id, None).await.ok();
    }
    Ok(())
}

pub async fn leave_room(user_id: &UserId, room_id: &RoomId, reason: Option<String>) -> AppResult<()> {
    // Ask a remote server if we don't have this room
    if !crate::room::is_server_in_room(crate::server_name(), room_id)? {
        match leave_remote_room(user_id, room_id).await {
            Err(e) => {
                warn!("Failed to leave room {} remotely: {}", user_id, e);
            }
            Ok((event_id, event_sn)) => {
                let last_state = crate::room::state::get_user_state(user_id, room_id)?;

                // We always drop the invite, we can't rely on other servers
                crate::room::update_membership(
                    &event_id,
                    event_sn,
                    room_id,
                    user_id,
                    MembershipState::Leave,
                    user_id,
                    last_state,
                )?;
            }
        }
    } else {
        let member_event = crate::room::state::get_state(room_id, &StateEventType::RoomMember, user_id.as_str(), None)?;

        // Fix for broken rooms
        let Some(member_event) = member_event else {
            warn!("Trying to leave a room you are not a member of.");
            // crate::room::timeline::build_and_append_pdu(
            //     PduBuilder::state(
            //         user_id.to_string(),
            //         &RoomMemberEventContent::new(MembershipState::Leave),
            //     ),
            //     user_id,
            //     room_id,
            // )?;
            if let Some((event_id, event_sn)) = room_users::table
                .filter(room_users::room_id.eq(room_id))
                .filter(room_users::user_id.eq(user_id))
                .order_by(room_users::id.desc())
                .select((room_users::event_id, room_users::event_sn))
                .first::<(OwnedEventId, i64)>(&mut *db::connect()?)
                .optional()?
            {
                crate::room::update_membership(
                    &event_id,
                    event_sn,
                    room_id,
                    &user_id,
                    MembershipState::Leave,
                    user_id,
                    None,
                )?;
            }
            return Ok(());
        };

        let mut event: RoomMemberEventContent = serde_json::from_str(member_event.content.get())
            .map_err(|_| AppError::public("Invalid member event in database."))?;

        event.membership = MembershipState::Leave;
        event.reason = reason;

        crate::room::timeline::build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomMember,
                content: to_raw_json_value(&event).expect("event is valid, we just created it"),
                state_key: Some(user_id.to_string()),
                ..Default::default()
            },
            user_id,
            room_id,
        )?;
    }

    Ok(())
}

async fn leave_remote_room(user_id: &UserId, room_id: &RoomId) -> AppResult<(OwnedEventId, Seqnum)> {
    let mut make_leave_response_and_server = Err(AppError::public("No server available to assist in leaving."));
    let invite_state =
        crate::room::state::get_user_state(user_id, room_id)?.ok_or(MatrixError::bad_state("User is not invited."))?;

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
            &room_id.server_name().map_err(AppError::internal)?.origin().await,
            room_id,
            user_id,
        )?
        .into_inner();
        let make_leave_response =
            crate::sending::send_federation_request(&room_id.server_name().map_err(AppError::internal)?, request)
                .await?
                .json::<MakeLeaveResBody>()
                .await;

        make_leave_response_and_server = make_leave_response.map(|r| (r, remote_server)).map_err(Into::into);

        if make_leave_response_and_server.is_ok() {
            break;
        }
    }

    let (make_leave_response, remote_server) = make_leave_response_and_server?;

    let room_version_id = match make_leave_response.room_version {
        Some(version) if crate::supported_room_versions().contains(&version) => version,
        _ => return Err(AppError::public("Room version is not supported")),
    };

    let mut leave_event_stub = serde_json::from_str::<CanonicalJsonObject>(make_leave_response.event.get())
        .map_err(|_| AppError::public("Invalid make_leave event json received from server."))?;

    // TODO: Is origin needed?
    leave_event_stub.insert(
        "origin".to_owned(),
        CanonicalJsonValue::String(crate::server_name().as_str().to_owned()),
    );
    leave_event_stub.insert(
        "origin_server_ts".to_owned(),
        CanonicalJsonValue::Integer(UnixMillis::now().get() as i64),
    );
    // We don't leave the event id in the pdu because that's only allowed in v1 or v2 rooms
    leave_event_stub.remove("event_id");

    // In order to create a compatible ref hash (EventID) the `hashes` field needs to be present
    crate::server_key::hash_and_sign_event(&mut leave_event_stub, &room_version_id)
        .expect("event is valid, we just created it");

    // Generate event id
    let event_id = crate::event::gen_event_id(&leave_event_stub, &room_version_id)?;

    let new_db_event = NewDbEvent {
        id: event_id.to_owned(),
        sn: None,
        ty: MembershipState::Leave.to_string(),
        room_id: room_id.to_owned(),
        unrecognized_keys: None,
        depth: 0,
        origin_server_ts: Some(UnixMillis::now()),
        received_at: None,
        sender_id: Some(user_id.to_owned()),
        contains_url: false,
        worker_id: None,
        state_key: Some(user_id.to_string()),
        is_outlier: true,
        soft_failed: false,
        rejection_reason: None,
    };
    let event_sn = diesel::insert_into(events::table)
        .values(&new_db_event)
        .on_conflict_do_nothing()
        .returning(events::sn)
        .get_result::<Seqnum>(&mut *db::connect()?)?;
    // Add event_id back
    leave_event_stub.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.as_str().to_owned()),
    );

    let event_data = DbEventData {
        event_id: event_id.clone(),
        event_sn,
        room_id: room_id.to_owned(),
        internal_metadata: None,
        json_data: serde_json::to_value(&leave_event_stub)?,
        format_version: None,
    };
    diesel::insert_into(event_datas::table)
        .values(&event_data)
        .on_conflict_do_nothing()
        .execute(&mut db::connect()?)?;

    // It has enough fields to be called a proper event now
    let leave_event = leave_event_stub;

    let request = send_leave_request_v2(
        &remote_server.origin().await,
        SendLeaveReqArgsV2 {
            room_id: room_id.to_owned(),
            event_id: event_id.clone(),
        },
        SendLeaveReqBody(PduEvent::convert_to_outgoing_federation_event(leave_event.clone())),
    )?
    .into_inner();

    crate::sending::send_federation_request(&remote_server, request).await?;

    Ok((event_id, event_sn))
}

/// Makes a user forget a room.
#[tracing::instrument]
pub fn forget_room(user_id: &UserId, room_id: &RoomId) -> AppResult<()> {
    if diesel_exists!(
        room_users::table
            .filter(room_users::user_id.eq(user_id))
            .filter(room_users::room_id.eq(room_id))
            .filter(room_users::membership.eq("join")),
        &mut db::connect()?
    )? {
        return Err(MatrixError::unknown("The user has not left the room.").into());
    }
    diesel::update(
        room_users::table
            .filter(room_users::user_id.eq(user_id))
            .filter(room_users::room_id.eq(room_id)),
    )
    .set(room_users::forgotten.eq(true))
    .execute(&mut db::connect()?)?;
    Ok(())
}
