use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{AnyStrippedStateEvent, TimelineEventType};
use crate::core::federation::membership::InviteUserResBodyV2;
use crate::core::identifiers::*;
use crate::core::serde::{RawJson, RawJsonValue, to_raw_json_value};
use crate::event::{PduBuilder, PduEvent, gen_event_id_canonical_json, handler};
use crate::membership::federation::membership::{InviteUserReqArgs, InviteUserReqBodyV2};
use crate::room::{state, timeline};
use crate::{AppResult, GetUrlOrigin, IsRemoteOrLocal, MatrixError, data, room, sending};

pub(crate) async fn ensure_invite_allowed(
    invitee_id: &UserId,
    inviter_id: &UserId,
) -> AppResult<()> {
    if invitee_id.is_local() && data::user::invite_blocked(invitee_id).await? {
        return Err(MatrixError::invite_blocked("This user has blocked room invites.").into());
    }
    #[cfg(feature = "unstable-msc4494")]
    if invitee_id.is_local()
        && denies_public_invites(invitee_id).await?
        && !shares_non_public_room(invitee_id, inviter_id).await?
    {
        return Err(
            MatrixError::invite_blocked("No shared non-public room with the inviter.").into(),
        );
    }
    #[cfg(not(feature = "unstable-msc4494"))]
    let _ = inviter_id;
    Ok(())
}

#[cfg(feature = "unstable-msc4494")]
async fn denies_public_invites(user_id: &UserId) -> AppResult<bool> {
    let config =
        data::user::get_global_data::<serde_json::Value>(user_id, "m.invite_permission_config")
            .await?;
    Ok(config
        .as_ref()
        .and_then(|c| c.get("default_action"))
        .and_then(serde_json::Value::as_str)
        == Some("uk.timedout.msc4494.deny_public"))
}

#[cfg(feature = "unstable-msc4494")]
async fn shares_non_public_room(invitee: &UserId, inviter: &UserId) -> AppResult<bool> {
    Ok(shared_invite_room_change(invitee, inviter).await?.is_some())
}

#[cfg(feature = "unstable-msc4494")]
async fn current_join_positions(
    user_id: &UserId,
) -> AppResult<std::collections::HashMap<OwnedRoomId, i64>> {
    use data::schema::room_users;
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    // Match joined_rooms: historical memberships must not establish a relationship.
    let memberships = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .distinct_on(room_users::room_id)
        .select((
            room_users::room_id,
            room_users::membership,
            room_users::event_sn,
        ))
        .order_by((room_users::room_id.desc(), room_users::id.desc()))
        .load::<(OwnedRoomId, String, i64)>(&mut data::connect().await?)
        .await?;
    Ok(memberships
        .into_iter()
        .filter_map(|(room_id, membership, sn)| (membership == "join").then_some((room_id, sn)))
        .collect())
}

/// Latest stream position establishing a currently qualifying shared room.
#[cfg(feature = "unstable-msc4494")]
async fn shared_invite_room_change(invitee: &UserId, inviter: &UserId) -> AppResult<Option<i64>> {
    use crate::core::events::StateEventType;
    use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
    use crate::core::room::JoinRule;

    let invitee_rooms = current_join_positions(invitee).await?;
    let inviter_rooms = current_join_positions(inviter).await?;
    let mut latest = None;
    for (room_id, invitee_sn) in invitee_rooms {
        let Some(inviter_sn) = inviter_rooms.get(&room_id) else {
            continue;
        };
        match room::get_state(&room_id, &StateEventType::RoomJoinRules, "", None).await {
            Ok(pdu) => {
                let content: RoomJoinRulesEventContent = serde_json::from_str(pdu.content.get())?;
                // Only proposal-listed rules establish trust. Reserved and custom rules
                // have no supported admission policy and must fail closed.
                if matches!(
                    content.join_rule,
                    JoinRule::Invite
                        | JoinRule::Knock
                        | JoinRule::Restricted(_)
                        | JoinRule::KnockRestricted(_)
                ) {
                    let change_sn = invitee_sn.max(*inviter_sn).max(pdu.event_sn);
                    latest =
                        Some(latest.map_or(change_sn, |previous: i64| previous.max(change_sn)));
                }
            }
            // Without known room state there is no evidence of a qualifying room.
            Err(e) if e.is_not_found() => {}
            Err(e) => return Err(e),
        }
    }
    Ok(latest)
}

/// Apply membership-based filtering to retained invites in both sync versions.
pub(crate) async fn invited_rooms_for_sync(
    user_id: &UserId,
    since_sn: i64,
) -> AppResult<Vec<(OwnedRoomId, Vec<RawJson<AnyStrippedStateEvent>>)>> {
    let invites = data::user::invited_rooms_for_sync(user_id, since_sn).await?;
    #[cfg(feature = "unstable-msc4494")]
    if denies_public_invites(user_id).await? {
        use data::schema::room_users;
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;
        // The invitation can predate a newly qualifying membership or join rule.
        // Reconsider retained invites, but replay them only after relevant changes,
        // including the account-data replay already handled by the data layer.
        let incremental_rooms: std::collections::HashSet<_> =
            invites.into_iter().map(|(room_id, _)| room_id).collect();
        let retained = data::user::invited_rooms_for_sync(user_id, 0).await?;
        let mut allowed = Vec::new();
        for (room_id, state) in retained {
            let inviter = room_users::table
                .filter(room_users::user_id.eq(user_id))
                .filter(room_users::room_id.eq(&room_id))
                .filter(room_users::membership.eq("invite"))
                .order_by(room_users::id.desc())
                .select(room_users::sender_id)
                .first::<OwnedUserId>(&mut data::connect().await?)
                .await
                .optional()?;
            if let Some(inviter) = inviter
                && let Some(change_sn) = shared_invite_room_change(user_id, &inviter).await?
                && (incremental_rooms.contains(&room_id) || change_sn >= since_sn)
            {
                allowed.push((room_id, state));
            }
        }
        return Ok(allowed);
    }
    Ok(invites)
}

/// Check local membership events before deduplication, signing or persistence.
pub(crate) async fn ensure_membership_invite_allowed(
    builder: &PduBuilder,
    sender: &UserId,
) -> AppResult<()> {
    ensure_invite_event_allowed(
        &builder.event_type,
        &builder.content,
        builder.state_key.as_deref(),
        sender,
    )
    .await
}

/// Apply the same recipient preference to authenticated incoming membership PDUs.
pub(crate) async fn ensure_incoming_invite_allowed(pdu: &PduEvent) -> AppResult<()> {
    ensure_invite_event_allowed(
        &pdu.event_ty,
        &pdu.content,
        pdu.state_key.as_deref(),
        &pdu.sender,
    )
    .await
}

async fn ensure_invite_event_allowed(
    event_type: &TimelineEventType,
    content: &RawJsonValue,
    state_key: Option<&str>,
    sender: &UserId,
) -> AppResult<()> {
    if *event_type == TimelineEventType::RoomMember {
        let content: RoomMemberEventContent = serde_json::from_str(content.get())?;
        if content.membership == MembershipState::Invite
            && let Some(state_key) = state_key
        {
            let invitee_id = UserId::parse(state_key)
                .map_err(|_| MatrixError::invalid_param("Invalid invite state_key."))?;
            ensure_invite_allowed(&invitee_id, sender).await?;
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
        assert!(
            ensure_membership_invite_allowed(&builder, &inviter)
                .await
                .is_ok()
        );
        assert_eq!(
            data::user::invited_rooms_for_sync(&user_id, 0)
                .await
                .unwrap()
                .len(),
            1
        );
        let blocked = data::user::set_data(
            &user_id,
            None,
            "m.invite_permission_config",
            json!({"default_action": "block"}),
        )
        .await
        .unwrap();
        let blocked_token = blocked.occur_sn + 1;
        assert!(matches!(
            ensure_membership_invite_allowed(&builder, &inviter).await,
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
        assert!(
            ensure_membership_invite_allowed(&join, &inviter)
                .await
                .is_ok()
        );
        for content in [
            json!({}),
            json!({"default_action": "allow"}),
            json!({"default_action": "unknown"}),
            json!({"default_action": false}),
            #[cfg(not(feature = "unstable-msc4494"))]
            json!({"default_action": "uk.timedout.msc4494.deny_public"}),
        ] {
            let allowed =
                data::user::set_data(&user_id, None, "m.invite_permission_config", content)
                    .await
                    .unwrap();
            assert!(
                ensure_membership_invite_allowed(&builder, &inviter)
                    .await
                    .is_ok()
            );
            assert_eq!(
                data::user::invited_rooms_for_sync(&user_id, blocked_token)
                    .await
                    .unwrap()
                    .len(),
                1
            );
            assert!(
                data::user::invited_rooms_for_sync(&user_id, allowed.occur_sn + 1)
                    .await
                    .unwrap()
                    .is_empty(),
                "an unchanged preference must not replay old invites"
            );
        }
        let blocked = data::user::set_data(
            &user_id,
            None,
            "m.invite_permission_config",
            json!({"default_action": "block"}),
        )
        .await
        .unwrap();
        data::user::delete_global_data(&user_id, "m.invite_permission_config")
            .await
            .unwrap();
        assert_eq!(
            data::user::invited_rooms_for_sync(&user_id, blocked.occur_sn + 1)
                .await
                .unwrap()
                .len(),
            1,
            "deleting the preference also re-exposes retained invites"
        );
    }

    #[cfg(feature = "unstable-msc4494")]
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_membership_invite_filter_uses_current_joins_and_join_rules() {
        use std::sync::Arc;

        use diesel::{ExpressionMethods, QueryDsl};

        use crate::core::serde::CanonicalJsonObject;
        use crate::data::schema::{room_users, rooms};
        use crate::room::state::{CompressedEvent, CompressedState};

        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name": "dynamic.example", "db": {"url": "unused-test-config"}
            }))
            .unwrap()
        });
        let invitee =
            UserId::parse_with_server_name("membership_filter", crate::config::server_name())
                .unwrap();
        let inviter: OwnedUserId = "@membership_inviter:example.org".try_into().unwrap();
        let mutual: OwnedRoomId = "!membership_mutual:example.org".try_into().unwrap();
        let target: OwnedRoomId = "!membership_target:example.org".try_into().unwrap();
        data::user::set_data(
            &invitee,
            None,
            "m.invite_permission_config",
            json!({"default_action": "uk.timedout.msc4494.deny_public"}),
        )
        .await
        .unwrap();
        let builder = PduBuilder::state(
            invitee.to_string(),
            &RoomMemberEventContent::new(MembershipState::Invite),
        );
        let assert_blocked = |result| {
            assert!(matches!(
                result,
                Err(crate::AppError::Matrix(MatrixError {
                    kind: crate::core::error::ErrorKind::InviteBlocked,
                    ..
                }))
            ))
        };
        assert_blocked(ensure_membership_invite_allowed(&builder, &inviter).await);
        let incoming: PduEvent = serde_json::from_value(json!({
            "event_id": "$membership_invite:example.org", "room_id": target,
            "type": "m.room.member", "sender": inviter, "state_key": invitee,
            "content": {"membership": "invite"}, "origin_server_ts": 1,
            "depth": 1, "auth_events": [], "prev_events": [], "hashes": {"sha256": "test"}
        }))
        .unwrap();
        assert_blocked(ensure_incoming_invite_allowed(&incoming).await);

        for (i, (room, user, membership)) in [
            (&mutual, &inviter, "join"),
            (&mutual, &invitee, "join"),
            (&target, &invitee, "invite"),
        ]
        .into_iter()
        .enumerate()
        {
            diesel::insert_into(room_users::table)
                .values(data::room::NewDbRoomUser {
                    event_id: format!("$membership_fixture_{i}:example.org")
                        .try_into()
                        .unwrap(),
                    event_sn: 1,
                    room_id: room.clone(),
                    room_server_id: None,
                    user_id: user.clone(),
                    user_server_id: user.server_name().to_owned(),
                    sender_id: inviter.clone(),
                    membership: membership.into(),
                    forgotten: false,
                    display_name: None,
                    avatar_url: None,
                    state_data: Some(json!([])),
                    created_at: crate::core::UnixMillis::now(),
                })
                .execute(&mut data::connect().await.unwrap())
                .await
                .unwrap();
        }
        diesel::insert_into(rooms::table)
            .values(data::room::NewDbRoom {
                id: mutual.clone(),
                version: "11".into(),
                is_public: true,
                min_depth: 0,
                has_auth_chain_index: false,
                created_at: crate::core::UnixMillis::now(),
            })
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        // Membership alone does not suffice when the join rule is unavailable.
        assert_blocked(ensure_invite_allowed(&invitee, &inviter).await);

        for (i, rule) in [
            "public",
            "private",
            "example.custom",
            "invite",
            "knock",
            "restricted",
            "knock_restricted",
            "public",
        ]
        .iter()
        .enumerate()
        {
            let raw = json!({
                "event_id": format!("$membership_rule_{i}:example.org"), "room_id": mutual,
                "type": "m.room.join_rules", "sender": inviter, "state_key": "",
                "content": {"join_rule": rule, "allow": []}, "origin_server_ts": 1,
                "depth": 1, "auth_events": [], "prev_events": [], "hashes": {"sha256": "test"}
            });
            let pdu: PduEvent = serde_json::from_value(raw.clone()).unwrap();
            let canonical: CanonicalJsonObject = serde_json::from_value(raw).unwrap();
            let (stored, _, guard) = PduBuilder::save_as_outlier(pdu, canonical, &inviter)
                .await
                .unwrap();
            let field =
                state::ensure_field_id(&crate::core::events::StateEventType::RoomJoinRules, "")
                    .await
                    .unwrap();
            let compressed: CompressedState = [CompressedEvent::new(field, stored.event_sn)]
                .into_iter()
                .collect();
            let delta = state::save_state(&mutual, Arc::new(compressed))
                .await
                .unwrap();
            state::set_room_state(&mutual, delta.frame_id)
                .await
                .unwrap();
            drop(guard);
            let allowed = matches!(
                *rule,
                "invite" | "knock" | "restricted" | "knock_restricted"
            );
            assert_eq!(
                ensure_membership_invite_allowed(&builder, &inviter)
                    .await
                    .is_ok(),
                allowed,
                "{rule}"
            );
            assert_eq!(
                ensure_incoming_invite_allowed(&incoming).await.is_ok(),
                allowed,
                "{rule}"
            );
            assert_eq!(
                invited_rooms_for_sync(&invitee, 0).await.unwrap().len(),
                usize::from(allowed),
                "{rule}"
            );
            assert_eq!(
                data::user::invited_rooms(&invitee, 0).await.unwrap().len(),
                1
            );
            if allowed {
                // This invitation predates the setting and the join-rule update.
                // An incremental sync must replay it when the relationship becomes eligible.
                let rule_sn = stored.event_sn;
                assert!(
                    data::user::invited_rooms_for_sync(&invitee, rule_sn)
                        .await
                        .unwrap()
                        .is_empty()
                );
                assert_eq!(
                    invited_rooms_for_sync(&invitee, rule_sn)
                        .await
                        .unwrap()
                        .len(),
                    1
                );
                // It must not repeat on every subsequent empty incremental sync.
                assert!(
                    invited_rooms_for_sync(&invitee, rule_sn + 1)
                        .await
                        .unwrap()
                        .is_empty()
                );
                for member in [&invitee, &inviter] {
                    diesel::update(
                        room_users::table
                            .filter(room_users::room_id.eq(&mutual))
                            .filter(room_users::user_id.eq(member)),
                    )
                    .set(room_users::membership.eq("leave"))
                    .execute(&mut data::connect().await.unwrap())
                    .await
                    .unwrap();
                    assert_blocked(ensure_invite_allowed(&invitee, &inviter).await);
                    assert!(
                        invited_rooms_for_sync(&invitee, 0)
                            .await
                            .unwrap()
                            .is_empty()
                    );
                    let join_sn = data::next_sn().await.unwrap();
                    diesel::update(
                        room_users::table
                            .filter(room_users::room_id.eq(&mutual))
                            .filter(room_users::user_id.eq(member)),
                    )
                    .set((
                        room_users::membership.eq("join"),
                        room_users::event_sn.eq(join_sn),
                    ))
                    .execute(&mut data::connect().await.unwrap())
                    .await
                    .unwrap();
                    assert_eq!(
                        invited_rooms_for_sync(&invitee, join_sn)
                            .await
                            .unwrap()
                            .len(),
                        1
                    );
                    assert!(
                        invited_rooms_for_sync(&invitee, join_sn + 1)
                            .await
                            .unwrap()
                            .is_empty()
                    );
                }
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_federation_stream_blocks_invites_before_persistence() {
        use diesel::{ExpressionMethods, QueryDsl};

        use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
        use crate::core::serde::{CanonicalJsonObject, to_raw_json_value};
        use crate::core::signatures::{Ed25519KeyPair, hash_and_sign_event};
        use crate::core::{RoomVersionId, UnixMillis};
        use crate::data::schema::{events, room_joined_servers, room_users, rooms};

        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name": "dynamic.example", "db": {"url": "unused-test-config"}
            }))
            .unwrap()
        });
        let room_id: OwnedRoomId = "!blocked_stream:example.org".try_into().unwrap();
        let invitee =
            UserId::parse_with_server_name("blocked_stream", crate::config::server_name()).unwrap();
        let remote: OwnedServerName = "invite-stream.example.org".try_into().unwrap();
        let mut conn = data::connect().await.unwrap();
        diesel::insert_into(rooms::table)
            .values(data::room::NewDbRoom {
                id: room_id.clone(),
                version: "11".into(),
                is_public: false,
                min_depth: 0,
                has_auth_chain_index: false,
                created_at: UnixMillis::now(),
            })
            .execute(&mut conn)
            .await
            .unwrap();
        diesel::insert_into(room_joined_servers::table)
            .values((
                room_joined_servers::room_id.eq(&room_id),
                room_joined_servers::server_id.eq(crate::config::server_name()),
                room_joined_servers::occur_sn.eq(1i64),
            ))
            .execute(&mut conn)
            .await
            .unwrap();
        data::user::set_data(
            &invitee,
            None,
            "m.invite_permission_config",
            json!({"default_action": "block"}),
        )
        .await
        .unwrap();
        let key =
            Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), "invite_review".into())
                .unwrap();
        let mut keys =
            ServerSigningKeys::new(remote.clone(), UnixMillis(UnixMillis::now().0 + 86_400_000));
        keys.verify_keys.insert(
            "ed25519:invite_review".try_into().unwrap(),
            VerifyKey::from_bytes(key.public_key().to_vec()),
        );
        crate::server_key::add_signing_keys(keys).await.unwrap();
        let mut json: CanonicalJsonObject = serde_json::from_value(json!({
            "type": "m.room.member", "room_id": room_id,
            "sender": "@inviter:invite-stream.example.org", "state_key": invitee,
            "content": {"membership": "invite"}, "origin_server_ts": UnixMillis::now(),
            "depth": 1, "auth_events": [], "prev_events": [],
        }))
        .unwrap();
        let rules = crate::room::get_version_rules(&RoomVersionId::V11).unwrap();
        hash_and_sign_event(remote.as_str(), &key, &mut json, &rules.redaction).unwrap();
        let (event_id, json) =
            gen_event_id_canonical_json(&to_raw_json_value(&json).unwrap(), &RoomVersionId::V11)
                .unwrap();
        assert!(matches!(
            handler::process_incoming_pdu(
                &remote,
                &event_id,
                &room_id,
                &RoomVersionId::V11,
                json.clone(),
                true,
                false
            )
            .await,
            Err(crate::AppError::Matrix(MatrixError {
                kind: crate::core::error::ErrorKind::InviteBlocked,
                ..
            }))
        ));
        assert_eq!(
            events::table
                .filter(events::id.eq(&event_id))
                .count()
                .get_result::<i64>(&mut conn)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            room_users::table
                .filter(room_users::user_id.eq(&invitee))
                .count()
                .get_result::<i64>(&mut conn)
                .await
                .unwrap(),
            0
        );
        #[cfg(feature = "unstable-msc4494")]
        {
            data::user::set_data(
                &invitee,
                None,
                "m.invite_permission_config",
                json!({"default_action": "uk.timedout.msc4494.deny_public"}),
            )
            .await
            .unwrap();
            assert!(matches!(
                handler::process_incoming_pdu(
                    &remote,
                    &event_id,
                    &room_id,
                    &RoomVersionId::V11,
                    json.clone(),
                    true,
                    false,
                )
                .await,
                Err(crate::AppError::Matrix(MatrixError {
                    kind: crate::core::error::ErrorKind::InviteBlocked,
                    ..
                }))
            ));
            assert_eq!(
                events::table
                    .filter(events::id.eq(&event_id))
                    .count()
                    .get_result::<i64>(&mut conn)
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                room_users::table
                    .filter(room_users::user_id.eq(&invitee))
                    .count()
                    .get_result::<i64>(&mut conn)
                    .await
                    .unwrap(),
                0
            );
        }
        // Recovery must also recheck the preference before promoting an older outlier.
        let pdu =
            PduEvent::from_json_value(&room_id, &event_id, serde_json::to_value(&json).unwrap())
                .unwrap();
        let incoming = crate::event::SnPduEvent {
            pdu,
            event_sn: 1,
            is_outlier: true,
            soft_failed: false,
            is_backfill: false,
        };
        assert!(matches!(
            handler::process_to_timeline_pdu(incoming, json, Some(&remote)).await,
            Err(crate::AppError::Matrix(MatrixError {
                kind: crate::core::error::ErrorKind::InviteBlocked,
                ..
            }))
        ));
    }
}
