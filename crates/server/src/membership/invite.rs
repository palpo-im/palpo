use crate::core::events::TimelineEventType;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::federation::membership::InviteUserResBodyV2;
use crate::core::identifiers::*;
use crate::core::serde::to_raw_json_value;
use crate::event::{PduBuilder, gen_event_id_canonical_json, handler};
use crate::membership::federation::membership::{InviteUserReqArgs, InviteUserReqBodyV2};
use crate::room::{state, timeline};
use crate::{AppResult, GetUrlOrigin, IsRemoteOrLocal, MatrixError, data, room, sending};

pub(crate) async fn ensure_invite_allowed(invitee_id: &UserId) -> AppResult<()> {
    if invitee_id.is_local() && data::user::invite_blocked(invitee_id).await? {
        return Err(MatrixError::invite_blocked("This user has blocked room invites.").into());
    }
    Ok(())
}

/// Check local membership events before deduplication, signing or persistence.
pub(crate) async fn ensure_membership_invite_allowed(builder: &PduBuilder) -> AppResult<()> {
    if builder.event_type == TimelineEventType::RoomMember {
        let content: RoomMemberEventContent = serde_json::from_str(builder.content.get())?;
        if content.membership == MembershipState::Invite
            && let Some(state_key) = &builder.state_key
        {
            let invitee_id = UserId::parse(state_key)
                .map_err(|_| MatrixError::invalid_param("Invalid invite state_key."))?;
            ensure_invite_allowed(&invitee_id).await?;
        }
    }
    Ok(())
}

pub async fn invite_user(
    inviter_id: &UserId,
    invitee_id: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    is_direct: bool,
) -> AppResult<()> {
    if !room::user::is_joined(inviter_id, room_id).await? {
        return Err(MatrixError::forbidden(
            "you must be joined in the room you are trying to invite from",
            None,
        )
        .into());
    }
    if !room::user_can_invite(room_id, inviter_id, invitee_id).await {
        return Err(MatrixError::forbidden("you are not allowed to invite this user", None).into());
    }

    let conf = crate::config::get();
    if invitee_id.server_name().is_remote() {
        let (pdu, pdu_json, invite_room_state, federation_invite_room_state) = {
            let content = RoomMemberEventContent {
                avatar_url: None,
                display_name: None,
                is_direct: Some(is_direct),
                membership: MembershipState::Invite,
                third_party_invite: None,
                blurhash: None,
                reason,
                join_authorized_via_users_server: None,
                #[cfg(feature = "unstable-msc4293")]
                redact_events: false,
                extra_data: Default::default(),
            };

            let state_lock = crate::room::lock_state(room_id).await;
            let room_version = crate::room::get_version(room_id)
                .await
                .unwrap_or_else(|_| conf.default_room_version.clone());
            let (pdu, mut pdu_json) = PduBuilder::state(invitee_id.to_string(), &content)
                .hash_sign(inviter_id, room_id, &room_version)
                .await?;

            // This path builds and federates the invite itself rather than going through
            // `build_and_append_pdu`, so the Policy Server check has to happen here too --
            // otherwise a refused invite is still sent, and a policy-aware invitee rejects
            // an unsigned event we cannot take back.
            let version_rules = crate::room::get_version_rules(&room_version)?;
            crate::room::policy::check_event(room_id, &mut pdu_json, &version_rules).await?;
            let (pdu, pdu_json, _event_guard) =
                PduBuilder::save_as_outlier(pdu, pdu_json, inviter_id).await?;
            drop(state_lock);

            let invite_room_state = state::summary_stripped(&pdu).await?;
            let federation_invite_room_state = state::summary_pdus(&pdu).await?;

            (
                pdu,
                pdu_json,
                invite_room_state,
                federation_invite_room_state,
            )
        };
        let room_version_id = room::get_version(room_id).await?;

        crate::membership::update_membership(
            &pdu.event_id,
            pdu.event_sn,
            room_id,
            invitee_id,
            MembershipState::Invite,
            inviter_id,
            Some(invite_room_state.clone()),
        )
        .await?;

        let invite_request = crate::core::federation::membership::invite_user_request_v2(
            &invitee_id.server_name().origin().await,
            InviteUserReqArgs {
                room_id: room_id.to_owned(),
                event_id: (*pdu.event_id).to_owned(),
            },
            InviteUserReqBodyV2 {
                room_version: room_version_id.clone(),
                event: sending::convert_to_outgoing_federation_event(pdu_json.clone()).await,
                invite_room_state: federation_invite_room_state,
                via: state::servers_route_via(room_id).await.ok(),
            },
        )?
        .into_inner();
        let send_join_response =
            sending::send_federation_request(invitee_id.server_name(), invite_request, None)
                .await?
                .json::<InviteUserResBodyV2>()
                .await?;

        // We do not add the event_id field to the pdu here because of signature and hashes checks
        let (event_id, value) =
            gen_event_id_canonical_json(&send_join_response.event, &room_version_id).map_err(
                |e| {
                    tracing::error!("could not convert event to canonical json: {e}");
                    MatrixError::invalid_param("could not convert event to canonical json")
                },
            )?;

        if *pdu.event_id != *event_id {
            warn!(
                "server {} changed invite event, that's not allowed in the spec: ours: {:?}, theirs: {:?}",
                invitee_id.server_name(),
                pdu_json,
                value
            );
            return Err(MatrixError::bad_json(format!(
                "server `{}` sent event with wrong event id",
                invitee_id.server_name()
            ))
            .into());
        }

        let origin: OwnedServerName = serde_json::from_value(
            serde_json::to_value(
                value
                    .get("origin")
                    .ok_or(MatrixError::bad_json("event needs an origin field"))?,
            )
            .expect("CanonicalJson is valid json value"),
        )
        .map_err(|e| {
            MatrixError::bad_json(format!(
                "origin field in event is not a valid server name: {e}"
            ))
        })?;

        handler::process_incoming_pdu(
            &origin,
            &event_id,
            room_id,
            &room_version_id,
            value,
            true,
            false,
        )
        .await?;
        return sending::send_pdu_room(
            room_id,
            &event_id,
            &[invitee_id.server_name().to_owned()],
            &[],
        )
        .await;
    }

    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_json_value(&RoomMemberEventContent {
                membership: MembershipState::Invite,
                display_name: data::user::display_name(invitee_id).await?,
                avatar_url: data::user::avatar_url(invitee_id).await?,
                is_direct: Some(is_direct),
                third_party_invite: None,
                blurhash: data::user::blurhash(invitee_id).await?,
                reason,
                join_authorized_via_users_server: None,
                #[cfg(feature = "unstable-msc4293")]
                redact_events: false,
                extra_data: Default::default(),
            })
            .expect("event is valid, we just created it"),
            state_key: Some(invitee_id.to_string()),
            ..Default::default()
        },
        inviter_id,
        room_id,
        &crate::room::get_version(room_id).await?,
        &room::lock_state(room_id).await,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_invite_blocking_preserves_membership_and_suppresses_sync() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name": "dynamic.example", "db": {"url": "unused-test-config"}
            }))
            .unwrap()
        });
        let user_id =
            UserId::parse_with_server_name("blocked_invitee", &crate::config::get().server_name)
                .unwrap();
        let room_id: OwnedRoomId = "!blocked:example.org".try_into().unwrap();
        let inviter: OwnedUserId = "@inviter:example.org".try_into().unwrap();
        diesel::insert_into(data::schema::room_users::table)
            .values(data::room::NewDbRoomUser {
                event_id: "$invite:example.org".try_into().unwrap(),
                event_sn: 1,
                room_id: room_id.clone(),
                room_server_id: None,
                user_id: user_id.clone(),
                user_server_id: user_id.server_name().to_owned(),
                sender_id: inviter.clone(),
                membership: "invite".into(),
                forgotten: false,
                display_name: None,
                avatar_url: None,
                state_data: Some(json!([{"type": "m.room.member",
                "state_key": user_id.as_str(), "sender": inviter.as_str(),
                "content": {"membership": "invite"}}])),
                created_at: crate::core::UnixMillis::now(),
            })
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        let builder = PduBuilder::state(
            user_id.to_string(),
            &RoomMemberEventContent::new(MembershipState::Invite),
        );
        assert!(ensure_membership_invite_allowed(&builder).await.is_ok());
        assert_eq!(
            data::user::invited_rooms_for_sync(&user_id, 0)
                .await
                .unwrap()
                .len(),
            1
        );
        data::user::set_data(
            &user_id,
            None,
            "m.invite_permission_config",
            json!({"default_action": "block"}),
        )
        .await
        .unwrap();
        assert!(matches!(
            ensure_membership_invite_allowed(&builder).await,
            Err(crate::AppError::Matrix(MatrixError {
                kind: crate::core::error::ErrorKind::InviteBlocked,
                ..
            }))
        ));
        assert!(
            data::user::invited_rooms_for_sync(&user_id, 0)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            data::user::invited_rooms(&user_id, 0).await.unwrap().len(),
            1,
            "ordinary membership reads and appservice delivery must retain the invite"
        );
        let join = PduBuilder::state(
            user_id.to_string(),
            &RoomMemberEventContent::new(MembershipState::Join),
        );
        assert!(ensure_membership_invite_allowed(&join).await.is_ok());
        for content in [
            json!({}),
            json!({"default_action": "allow"}),
            json!({"default_action": "unknown"}),
            json!({"default_action": false}),
        ] {
            data::user::set_data(&user_id, None, "m.invite_permission_config", content)
                .await
                .unwrap();
            assert!(ensure_membership_invite_allowed(&builder).await.is_ok());
            assert_eq!(
                data::user::invited_rooms_for_sync(&user_id, 0)
                    .await
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}
