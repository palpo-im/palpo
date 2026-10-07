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
    Ok(invite_relationship_changes(invitee, &[inviter.to_owned()])
        .await?
        .contains_key(inviter))
}

/// Resolve each distinct inviter once and each shared room's rule once per request.
#[cfg(feature = "unstable-msc4494")]
async fn invite_relationship_changes(
    invitee: &UserId,
    inviters: &[OwnedUserId],
) -> AppResult<std::collections::HashMap<OwnedUserId, i64>> {
    use std::collections::HashMap;

    use data::schema::room_users;
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;

    use crate::core::events::StateEventType;
    use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
    use crate::core::room::JoinRule;

    // Historical memberships cannot establish a relationship. Match joined_rooms'
    // latest-row semantics for inviters, limiting the batch to the recipient's rooms.
    let invitee_rooms = data::user::joined_rooms(invitee).await?;
    let mut changes: HashMap<OwnedUserId, i64> = HashMap::new();
    if invitee_rooms.is_empty() || inviters.is_empty() {
        return Ok(changes);
    }
    let memberships = room_users::table
        .filter(room_users::user_id.eq_any(inviters))
        .filter(room_users::room_id.eq_any(&invitee_rooms))
        .distinct_on((room_users::user_id, room_users::room_id))
        .select((
            room_users::user_id,
            room_users::room_id,
            room_users::membership,
        ))
        .order_by((
            room_users::user_id.desc(),
            room_users::room_id.desc(),
            room_users::id.desc(),
        ))
        .load::<(OwnedUserId, OwnedRoomId, String)>(&mut data::connect().await?)
        .await?;
    let mut qualifying_rooms: HashMap<OwnedRoomId, Option<i64>> = HashMap::new();
    for (inviter, room_id, membership) in memberships {
        if membership != "join" {
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(entry) =
            qualifying_rooms.entry(room_id.clone())
        {
            let rule_sn =
                match room::get_state(&room_id, &StateEventType::RoomJoinRules, "", None).await {
                    Ok(pdu) => {
                        // Bad state disqualifies this room, not the entire invitation or sync.
                        serde_json::from_str::<RoomJoinRulesEventContent>(pdu.content.get())
                            .ok()
                            .filter(|content| {
                                matches!(
                                    content.join_rule,
                                    JoinRule::Invite
                                        | JoinRule::Knock
                                        | JoinRule::Restricted(_)
                                        | JoinRule::KnockRestricted(_)
                                )
                            })
                            .map(|_| pdu.event_sn)
                    }
                    Err(e) if e.is_not_found() => None,
                    Err(e) => return Err(e),
                };
            let change = if let Some(rule_sn) = rule_sn {
                match room::user::join_sn(invitee, &room_id).await {
                    Ok(join_sn) => Some(rule_sn.max(join_sn)),
                    Err(e) if e.is_not_found() => None,
                    Err(e) => return Err(e),
                }
            } else {
                None
            };
            entry.insert(change);
        }
        if let Some(Some(room_change)) = qualifying_rooms.get(&room_id) {
            // The current joined period starts before join-to-join profile updates.
            let inviter_join_sn = match room::user::join_sn(&inviter, &room_id).await {
                Ok(join_sn) => join_sn,
                Err(e) if e.is_not_found() => continue,
                Err(e) => return Err(e),
            };
            let change = (*room_change).max(inviter_join_sn);
            changes
                .entry(inviter)
                .and_modify(|previous| *previous = (*previous).max(change))
                .or_insert(change);
        }
    }
    Ok(changes)
}

/// Apply membership-based filtering to retained invites in both sync versions.
pub(crate) async fn invited_rooms_for_sync(
    user_id: &UserId,
    since_sn: i64,
) -> AppResult<Vec<(OwnedRoomId, Vec<RawJson<AnyStrippedStateEvent>>)>> {
    let invites = data::user::invited_rooms_for_sync(user_id, since_sn).await?;
    #[cfg(feature = "unstable-msc4494")]
    if denies_public_invites(user_id).await? {
        use data::schema::{room_invite_admissions, room_users};
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        // The invitation can predate a newly qualifying membership or join rule.
        // Previously hidden invites are admitted once when they first qualify,
        // also preserving account-data replay already handled by the data layer.
        let incremental_rooms: std::collections::HashSet<_> =
            invites.iter().map(|(room_id, _)| room_id.clone()).collect();
        // Sliding sync already requested all invitations. Reuse that snapshot.
        let retained = if since_sn == 0 {
            invites
        } else {
            data::user::invited_rooms_for_sync(user_id, 0).await?
        };
        if retained.is_empty() {
            return Ok(Vec::new());
        }
        let room_ids: Vec<_> = retained.iter().map(|(room_id, _)| room_id).collect();
        let current_invites: std::collections::HashMap<OwnedRoomId, (i64, OwnedUserId)> =
            room_users::table
                .filter(room_users::user_id.eq(user_id))
                .filter(room_users::room_id.eq_any(&room_ids))
                .filter(room_users::membership.eq("invite"))
                .distinct_on(room_users::room_id)
                .order_by((room_users::room_id.desc(), room_users::id.desc()))
                .select((room_users::room_id, room_users::id, room_users::sender_id))
                .load::<(OwnedRoomId, i64, OwnedUserId)>(&mut data::connect().await?)
                .await?
                .into_iter()
                .map(|(room_id, id, sender)| (room_id, (id, sender)))
                .collect();
        let invite_ids: Vec<_> = current_invites.values().map(|(id, _)| *id).collect();
        let admitted: std::collections::HashMap<i64, i64> = room_invite_admissions::table
            .filter(room_invite_admissions::room_user_id.eq_any(&invite_ids))
            .select((
                room_invite_admissions::room_user_id,
                room_invite_admissions::admitted_sn,
            ))
            .load::<(i64, i64)>(&mut data::connect().await?)
            .await?
            .into_iter()
            .collect();
        // Qualification admits a pending invitation once. Losing a shared relationship
        // does not revoke an already permitted invite or hide it from a fresh client.
        let unique_inviters: std::collections::HashSet<_> = current_invites
            .values()
            .filter(|(id, _)| !admitted.contains_key(id))
            .map(|(_, sender)| sender.clone())
            .collect();
        let unique_inviters: Vec<_> = unique_inviters.into_iter().collect();
        let changes = if unique_inviters.is_empty() {
            std::collections::HashMap::new()
        } else {
            invite_relationship_changes(user_id, &unique_inviters).await?
        };
        let candidates: Vec<_> = current_invites
            .values()
            .filter(|(id, _)| !admitted.contains_key(id))
            .filter_map(|(id, sender)| changes.get(sender).map(|sn| (*id, *sn)))
            .collect();
        let admitted = if candidates.is_empty() {
            admitted
        } else {
            admit_pending_invites(candidates, &invite_ids).await?
        };
        let mut allowed = Vec::new();
        for (room_id, room_state) in retained {
            if let Some((id, _)) = current_invites.get(&room_id)
                && let Some(admitted_sn) = admitted.get(id)
                && (incremental_rooms.contains(&room_id) || *admitted_sn >= since_sn)
            {
                allowed.push((room_id, room_state));
            }
        }
        return Ok(allowed);
    }
    #[cfg(feature = "unstable-msc4494")]
    if !invites.is_empty() {
        use data::schema::{room_invite_admissions, room_users};
        use diesel::{ExpressionMethods, JoinOnDsl, QueryDsl};
        use diesel_async::RunQueryDsl;

        // Invites returned under an allowing preference must stay visible when
        // deny_public is enabled later, even without a qualifying relationship.
        let room_ids: Vec<_> = invites.iter().map(|(room_id, _)| room_id).collect();
        let candidates = room_users::table
            .left_join(
                room_invite_admissions::table
                    .on(room_invite_admissions::room_user_id.eq(room_users::id)),
            )
            .filter(room_users::user_id.eq(user_id))
            .filter(room_users::room_id.eq_any(&room_ids))
            .filter(room_users::membership.eq("invite"))
            .filter(room_invite_admissions::room_user_id.is_null())
            .select((room_users::id, room_users::event_sn))
            .load::<(i64, i64)>(&mut data::connect().await?)
            .await?;
        if !candidates.is_empty() {
            let invite_ids: Vec<_> = candidates.iter().map(|(id, _)| *id).collect();
            admit_pending_invites(candidates, &invite_ids).await?;
        }
    }
    Ok(invites)
}

#[cfg(feature = "unstable-msc4494")]
async fn admit_pending_invites(
    mut candidates: Vec<(i64, i64)>,
    invite_ids: &[i64],
) -> AppResult<std::collections::HashMap<i64, i64>> {
    use data::schema::{room_invite_admissions, room_users};
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::{AsyncConnection, RunQueryDsl};

    // Consistent insert order prevents concurrent multi-room syncs from deadlocking.
    candidates.sort_unstable_by_key(|(id, _)| *id);
    data::connect()
        .await?
        .transaction::<_, crate::AppError, _>(async |conn| {
            // A concurrent join/leave can replace the pending membership. Hold its
            // key while inserting admissions, instead of racing the foreign key.
            let candidate_ids: Vec<_> = candidates.iter().map(|(id, _)| *id).collect();
            let live: std::collections::HashSet<_> = room_users::table
                .filter(room_users::id.eq_any(&candidate_ids))
                .filter(room_users::membership.eq("invite"))
                .order_by(room_users::id.asc())
                .select(room_users::id)
                .for_key_share()
                .load::<i64>(conn)
                .await?
                .into_iter()
                .collect();
            let values: Vec<_> = candidates
                .iter()
                .filter(|(id, _)| live.contains(id))
                .map(|(id, sn)| {
                    (
                        room_invite_admissions::room_user_id.eq(*id),
                        room_invite_admissions::admitted_sn.eq(*sn),
                    )
                })
                .collect();
            if !values.is_empty() {
                diesel::insert_into(room_invite_admissions::table)
                    .values(values)
                    .on_conflict(room_invite_admissions::room_user_id)
                    .do_nothing()
                    .execute(conn)
                    .await?;
            }
            // Another device or instance may have admitted it first; use that decision.
            Ok(room_invite_admissions::table
                .filter(room_invite_admissions::room_user_id.eq_any(invite_ids))
                .select((
                    room_invite_admissions::room_user_id,
                    room_invite_admissions::admitted_sn,
                ))
                .load::<(i64, i64)>(conn)
                .await?
                .into_iter()
                .collect::<std::collections::HashMap<_, _>>())
        })
        .await
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
        #[cfg(feature = "unstable-msc4494")]
        {
            // A client received this invitation while allowing all senders.
            assert_eq!(invited_rooms_for_sync(&user_id, 0).await.unwrap().len(), 1);
            let restricted = data::user::set_data(
                &user_id,
                None,
                "m.invite_permission_config",
                json!({"default_action": "uk.timedout.msc4494.deny_public"}),
            )
            .await
            .unwrap();
            assert_eq!(
                invited_rooms_for_sync(&user_id, restricted.occur_sn)
                    .await
                    .unwrap()
                    .len(),
                1,
                "enabling deny_public preserves invites already returned while allowing"
            );
            assert_eq!(invited_rooms_for_sync(&user_id, 0).await.unwrap().len(), 1);
            assert!(
                invited_rooms_for_sync(&user_id, restricted.occur_sn + 1)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(matches!(
                ensure_membership_invite_allowed(&builder, &inviter).await,
                Err(crate::AppError::Matrix(MatrixError {
                    kind: crate::core::error::ErrorKind::InviteBlocked,
                    ..
                }))
            ));
            // A retained invite never returned to a client cannot inherit this admission.
            let hidden_room: OwnedRoomId =
                "!allow_transition_hidden:example.org".try_into().unwrap();
            diesel::insert_into(data::schema::room_users::table)
                .values(data::room::NewDbRoomUser {
                    event_id: "$allow_transition_hidden:example.org".try_into().unwrap(),
                    event_sn: data::next_sn().await.unwrap(),
                    room_id: hidden_room,
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
            assert_eq!(
                data::user::invited_rooms(&user_id, 0).await.unwrap().len(),
                2
            );
            assert_eq!(invited_rooms_for_sync(&user_id, 0).await.unwrap().len(), 1);
        }
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
            "malformed",
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
            let mut fresh_invite = room_users::table
                .filter(room_users::room_id.eq(&target))
                .filter(room_users::user_id.eq(&invitee))
                .first::<data::room::DbRoomUser>(&mut data::connect().await.unwrap())
                .await
                .unwrap();
            diesel::delete(room_users::table.find(fresh_invite.id))
                .execute(&mut data::connect().await.unwrap())
                .await
                .unwrap();
            fresh_invite.id = diesel::dsl::sql::<diesel::sql_types::BigInt>(
                "SELECT nextval(pg_get_serial_sequence('room_users', 'id'))",
            )
            .get_result(&mut data::connect().await.unwrap())
            .await
            .unwrap();
            diesel::insert_into(room_users::table)
                .values(&fresh_invite)
                .execute(&mut data::connect().await.unwrap())
                .await
                .unwrap();
            let raw = json!({
                "event_id": format!("$membership_rule_{i}:example.org"), "room_id": mutual,
                "type": "m.room.join_rules", "sender": inviter, "state_key": "",
                "content": if *rule == "malformed" { json!({}) } else { json!({"join_rule": rule, "allow": []}) }, "origin_server_ts": 1,
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
                    assert_eq!(
                        invited_rooms_for_sync(&invitee, 0).await.unwrap().len(),
                        1,
                        "an admitted invitation remains visible after losing its relationship"
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
                        0
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

    #[cfg(feature = "unstable-msc4494")]
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_invite_profile_updates_do_not_replay_retained_invites() {
        use std::sync::Arc;

        use diesel::{ExpressionMethods, QueryDsl};

        use crate::core::events::StateEventType;
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
        let invitee: OwnedUserId = "@invite_profile:dynamic.example".try_into().unwrap();
        let inviter: OwnedUserId = "@invite_profile_sender:example.org".try_into().unwrap();
        let mutual: OwnedRoomId = "!invite_profile_shared:example.org".try_into().unwrap();
        diesel::insert_into(rooms::table)
            .values(data::room::NewDbRoom {
                id: mutual.clone(),
                version: "11".into(),
                is_public: false,
                min_depth: 0,
                has_auth_chain_index: false,
                created_at: crate::core::UnixMillis::now(),
            })
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        data::user::set_data(
            &invitee,
            None,
            "m.invite_permission_config",
            json!({"default_action": "uk.timedout.msc4494.deny_public"}),
        )
        .await
        .unwrap();
        let mut compressed = CompressedState::new();
        for (i, member) in [&invitee, &inviter].into_iter().enumerate() {
            let raw = json!({
                "event_id": format!("$invite_profile_join_{i}:example.org"), "room_id": mutual,
                "type": "m.room.member", "sender": member, "state_key": member,
                "content": {"membership": "join"}, "origin_server_ts": 1,
                "depth": 1, "auth_events": [], "prev_events": [], "hashes": {"sha256": "test"}
            });
            let (pdu, _, guard) = PduBuilder::save_as_outlier(
                serde_json::from_value(raw.clone()).unwrap(),
                serde_json::from_value::<CanonicalJsonObject>(raw).unwrap(),
                member,
            )
            .await
            .unwrap();
            drop(guard);
            let field = state::ensure_field_id(&StateEventType::RoomMember, member.as_str())
                .await
                .unwrap();
            compressed.insert(CompressedEvent::new(field, pdu.event_sn));
            diesel::insert_into(room_users::table)
                .values(data::room::NewDbRoomUser {
                    event_id: pdu.event_id.clone(),
                    event_sn: pdu.event_sn,
                    room_id: mutual.clone(),
                    room_server_id: None,
                    user_id: member.clone(),
                    user_server_id: member.server_name().to_owned(),
                    sender_id: member.clone(),
                    membership: "join".into(),
                    forgotten: false,
                    display_name: None,
                    avatar_url: None,
                    state_data: None,
                    created_at: crate::core::UnixMillis::now(),
                })
                .execute(&mut data::connect().await.unwrap())
                .await
                .unwrap();
        }
        let raw = json!({
            "event_id": "$invite_profile_rule:example.org", "room_id": mutual,
            "type": "m.room.join_rules", "sender": inviter, "state_key": "",
            "content": {"join_rule": "invite"}, "origin_server_ts": 1,
            "depth": 1, "auth_events": [], "prev_events": [], "hashes": {"sha256": "test"}
        });
        let (pdu, _, guard) = PduBuilder::save_as_outlier(
            serde_json::from_value(raw.clone()).unwrap(),
            serde_json::from_value(raw).unwrap(),
            &inviter,
        )
        .await
        .unwrap();
        drop(guard);
        let field = state::ensure_field_id(&StateEventType::RoomJoinRules, "")
            .await
            .unwrap();
        compressed.insert(CompressedEvent::new(field, pdu.event_sn));
        let baseline = state::save_state(&mutual, Arc::new(compressed))
            .await
            .unwrap();
        state::set_room_state(&mutual, baseline.frame_id)
            .await
            .unwrap();
        // Malformed state in a second shared room must not break a valid relationship.
        let malformed: OwnedRoomId = "!invite_profile_malformed:example.org".try_into().unwrap();
        diesel::insert_into(rooms::table)
            .values(data::room::NewDbRoom {
                id: malformed.clone(),
                version: "11".into(),
                is_public: false,
                min_depth: 0,
                has_auth_chain_index: false,
                created_at: crate::core::UnixMillis::now(),
            })
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        for (i, member) in [&invitee, &inviter].into_iter().enumerate() {
            diesel::insert_into(room_users::table)
                .values(data::room::NewDbRoomUser {
                    event_id: format!("$invite_profile_malformed_join_{i}:example.org")
                        .try_into()
                        .unwrap(),
                    event_sn: 1,
                    room_id: malformed.clone(),
                    room_server_id: None,
                    user_id: member.clone(),
                    user_server_id: member.server_name().to_owned(),
                    sender_id: member.clone(),
                    membership: "join".into(),
                    forgotten: false,
                    display_name: None,
                    avatar_url: None,
                    state_data: None,
                    created_at: crate::core::UnixMillis::now(),
                })
                .execute(&mut data::connect().await.unwrap())
                .await
                .unwrap();
        }
        let raw = json!({
            "event_id": "$invite_profile_malformed_rule:example.org", "room_id": malformed,
            "type": "m.room.join_rules", "sender": inviter, "state_key": "", "content": {},
            "origin_server_ts": 1, "depth": 1, "auth_events": [], "prev_events": [], "hashes": {"sha256": "test"}
        });
        let (bad, _, guard) = PduBuilder::save_as_outlier(
            serde_json::from_value(raw.clone()).unwrap(),
            serde_json::from_value(raw).unwrap(),
            &inviter,
        )
        .await
        .unwrap();
        drop(guard);
        let bad_state: CompressedState = [CompressedEvent::new(field, bad.event_sn)]
            .into_iter()
            .collect();
        let bad_frame = state::save_state(&malformed, Arc::new(bad_state))
            .await
            .unwrap();
        state::set_room_state(&malformed, bad_frame.frame_id)
            .await
            .unwrap();
        ensure_invite_allowed(&invitee, &inviter).await.unwrap();
        // Many retained invitations from the same inviter share one relationship check.
        for i in 0..8 {
            let room_id: OwnedRoomId = format!("!invite_profile_target_{i}:example.org")
                .try_into()
                .unwrap();
            diesel::insert_into(room_users::table)
                .values(data::room::NewDbRoomUser {
                    event_id: format!("$invite_profile_target_{i}:example.org")
                        .try_into()
                        .unwrap(),
                    event_sn: 1,
                    room_id,
                    room_server_id: None,
                    user_id: invitee.clone(),
                    user_server_id: invitee.server_name().to_owned(),
                    sender_id: inviter.clone(),
                    membership: "invite".into(),
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
        let (first, concurrent) = tokio::join!(
            invited_rooms_for_sync(&invitee, 0),
            invited_rooms_for_sync(&invitee, 0)
        );
        assert_eq!(first.unwrap().len(), 8);
        assert_eq!(concurrent.unwrap().len(), 8);
        for (i, member) in [&invitee, &inviter].into_iter().enumerate() {
            let join_sn = room::user::join_sn(member, &mutual).await.unwrap();
            let raw = json!({
                "event_id": format!("$invite_profile_update_{i}:example.org"), "room_id": mutual,
                "type": "m.room.member", "sender": member, "state_key": member,
                "content": {"membership": "join", "displayname": "Changed", "avatar_url": "mxc://example.org/avatar"},
                "origin_server_ts": 1, "depth": 1, "auth_events": [], "prev_events": [], "hashes": {"sha256": "test"}
            });
            let (profile, _, guard) = PduBuilder::save_as_outlier(
                serde_json::from_value(raw.clone()).unwrap(),
                serde_json::from_value(raw).unwrap(),
                member,
            )
            .await
            .unwrap();
            drop(guard);
            crate::event::update_before_frame_id(&profile.event_id, baseline.frame_id)
                .await
                .unwrap();
            diesel::update(
                room_users::table
                    .filter(room_users::room_id.eq(&mutual))
                    .filter(room_users::user_id.eq(member)),
            )
            .set((
                room_users::event_id.eq(&profile.event_id),
                room_users::event_sn.eq(profile.event_sn),
            ))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
            assert_eq!(room::user::join_sn(member, &mutual).await.unwrap(), join_sn);
            assert!(
                invited_rooms_for_sync(&invitee, profile.event_sn)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(invited_rooms_for_sync(&invitee, 0).await.unwrap().len(), 8);
        }
        let raw = json!({
            "event_id": "$invite_profile_qualifying_update:example.org", "room_id": mutual,
            "type": "m.room.join_rules", "sender": inviter, "state_key": "", "content": {"join_rule": "knock"},
            "origin_server_ts": 1, "depth": 1, "auth_events": [], "prev_events": [], "hashes": {"sha256": "test"}
        });
        let (rule_update, _, guard) = PduBuilder::save_as_outlier(
            serde_json::from_value(raw.clone()).unwrap(),
            serde_json::from_value(raw).unwrap(),
            &inviter,
        )
        .await
        .unwrap();
        drop(guard);
        let updated: CompressedState = [CompressedEvent::new(field, rule_update.event_sn)]
            .into_iter()
            .collect();
        let frame = state::save_state(&mutual, Arc::new(updated)).await.unwrap();
        state::set_room_state(&mutual, frame.frame_id)
            .await
            .unwrap();
        assert!(
            invited_rooms_for_sync(&invitee, rule_update.event_sn)
                .await
                .unwrap()
                .is_empty(),
            "qualifying-to-qualifying rule changes must not replay old invitations"
        );
        // A second qualifying room must not re-admit already visible invitations.
        diesel::update(room_users::table.filter(room_users::room_id.eq(&malformed)))
            .set(room_users::membership.eq("leave"))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        let raw = json!({
            "event_id": "$invite_profile_second_qualifier:example.org", "room_id": malformed,
            "type": "m.room.join_rules", "sender": inviter, "state_key": "", "content": {"join_rule": "invite"},
            "origin_server_ts": 1, "depth": 1, "auth_events": [], "prev_events": [], "hashes": {"sha256": "test"}
        });
        let (second_rule, _, guard) = PduBuilder::save_as_outlier(
            serde_json::from_value(raw.clone()).unwrap(),
            serde_json::from_value(raw).unwrap(),
            &inviter,
        )
        .await
        .unwrap();
        drop(guard);
        let updated: CompressedState = [CompressedEvent::new(field, second_rule.event_sn)]
            .into_iter()
            .collect();
        let frame = state::save_state(&malformed, Arc::new(updated))
            .await
            .unwrap();
        state::set_room_state(&malformed, frame.frame_id)
            .await
            .unwrap();
        for member in [&invitee, &inviter] {
            let join_sn = data::next_sn().await.unwrap();
            diesel::update(
                room_users::table
                    .filter(room_users::room_id.eq(&malformed))
                    .filter(room_users::user_id.eq(member)),
            )
            .set((
                room_users::membership.eq("join"),
                room_users::event_sn.eq(join_sn),
            ))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
            assert!(
                invited_rooms_for_sync(&invitee, join_sn)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(invited_rooms_for_sync(&invitee, 0).await.unwrap().len(), 8);
        diesel::update(room_users::table.filter(room_users::room_id.eq(&malformed)))
            .set(room_users::membership.eq("leave"))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        // Losing every qualifying relationship blocks new invites while old ones stay visible.
        diesel::update(room_users::table.filter(room_users::room_id.eq(&mutual)))
            .set(room_users::membership.eq("leave"))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        assert!(matches!(
            ensure_invite_allowed(&invitee, &inviter).await,
            Err(crate::AppError::Matrix(MatrixError {
                kind: crate::core::error::ErrorKind::InviteBlocked,
                ..
            }))
        ));
        assert_eq!(invited_rooms_for_sync(&invitee, 0).await.unwrap().len(), 8);
        let after = data::curr_sn().await.unwrap() + 1;
        assert!(
            invited_rooms_for_sync(&invitee, after)
                .await
                .unwrap()
                .is_empty()
        );
        data::user::set_data(
            &invitee,
            None,
            "m.invite_permission_config",
            json!({"default_action": "block"}),
        )
        .await
        .unwrap();
        assert!(
            invited_rooms_for_sync(&invitee, 0)
                .await
                .unwrap()
                .is_empty()
        );
        data::user::set_data(
            &invitee,
            None,
            "m.invite_permission_config",
            json!({"default_action": "uk.timedout.msc4494.deny_public"}),
        )
        .await
        .unwrap();
        assert_eq!(invited_rooms_for_sync(&invitee, 0).await.unwrap().len(), 8);
        let target: OwnedRoomId = "!invite_profile_target_0:example.org".try_into().unwrap();
        let mut replacement = room_users::table
            .filter(room_users::room_id.eq(&target))
            .filter(room_users::user_id.eq(&invitee))
            .first::<data::room::DbRoomUser>(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        let old_id = replacement.id;
        diesel::delete(room_users::table.find(old_id))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        replacement.id = diesel::dsl::sql::<diesel::sql_types::BigInt>(
            "SELECT nextval(pg_get_serial_sequence('room_users', 'id'))",
        )
        .get_result(&mut data::connect().await.unwrap())
        .await
        .unwrap();
        replacement.event_id = "$invite_profile_replacement:example.org"
            .try_into()
            .unwrap();
        replacement.event_sn = data::next_sn().await.unwrap();
        diesel::insert_into(room_users::table)
            .values(&replacement)
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        use crate::data::schema::room_invite_admissions;
        assert!(
            room_invite_admissions::table
                .filter(room_invite_admissions::room_user_id.eq(old_id))
                .select(room_invite_admissions::room_user_id)
                .load::<i64>(&mut data::connect().await.unwrap())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            data::user::invited_rooms(&invitee, 0).await.unwrap().len(),
            8
        );
        assert_eq!(
            invited_rooms_for_sync(&invitee, 0).await.unwrap().len(),
            7,
            "a replacement invitation without current trust cannot inherit an old admission"
        );
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
