use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{AnyStrippedStateEvent, TimelineEventType};
use crate::core::federation::membership::InviteUserResBodyV2;
use crate::core::identifiers::*;
use crate::core::serde::{RawJson, RawJsonValue, to_raw_json_value};
use crate::event::{PduBuilder, PduEvent, gen_event_id_canonical_json, handler};
use crate::membership::federation::membership::{InviteUserReqArgs, InviteUserReqBodyV2};
use crate::room::{state, timeline};
use crate::{AppResult, GetUrlOrigin, IsRemoteOrLocal, MatrixError, data, room, sending};

#[cfg(feature = "unstable-msc4494")]
const MEMBERSHIP_INVITE_ACTION: &str = "uk.timedout.msc4494.deny_public";

pub(crate) async fn ensure_invite_allowed(
    invitee_id: &UserId,
    inviter_id: &UserId,
) -> AppResult<()> {
    if !invitee_id.is_local() {
        return Ok(());
    }
    #[cfg(feature = "unstable-msc4494")]
    let membership_action = Some(MEMBERSHIP_INVITE_ACTION);
    #[cfg(not(feature = "unstable-msc4494"))]
    let membership_action = None;
    let permission = data::user::invite_permission_snapshot(
        invitee_id,
        &[inviter_id.to_owned()],
        membership_action,
    )
    .await?;
    if permission.default_action.as_deref() == Some("block") {
        return Err(MatrixError::invite_blocked("This user has blocked room invites.").into());
    }
    #[cfg(feature = "unstable-msc4494")]
    if permission.default_action.as_deref() == membership_action
        && !eligible_inviters(&permission.shared_rooms)
            .await?
            .contains(inviter_id)
    {
        return Err(
            MatrixError::invite_blocked("No shared non-public room with the inviter.").into(),
        );
    }
    Ok(())
}

/// Resolve each distinct inviter once and each shared room's rule once per request.
#[cfg(feature = "unstable-msc4494")]
async fn eligible_inviters(
    shared_rooms: &[data::user::SharedInviteRoom],
) -> AppResult<std::collections::HashSet<OwnedUserId>> {
    use std::collections::{HashMap, HashSet};

    use crate::core::events::StateEventType;
    use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
    use crate::core::room::JoinRule;
    let mut eligible = HashSet::new();
    let mut qualifying_frames: HashMap<i64, bool> = HashMap::new();
    for shared in shared_rooms {
        if let std::collections::hash_map::Entry::Vacant(entry) =
            qualifying_frames.entry(shared.frame_id)
        {
            let qualifies =
                match state::get_state(shared.frame_id, &StateEventType::RoomJoinRules, "").await {
                    Ok(pdu) => serde_json::from_str::<RoomJoinRulesEventContent>(pdu.content.get())
                        .ok()
                        .is_some_and(|content| {
                            matches!(
                                content.join_rule,
                                JoinRule::Invite
                                    | JoinRule::Knock
                                    | JoinRule::Restricted(_)
                                    | JoinRule::KnockRestricted(_)
                            )
                        }),
                    Err(e) if e.is_not_found() => false,
                    Err(e) => return Err(e),
                };
            entry.insert(qualifies);
        }
        if qualifying_frames.get(&shared.frame_id) == Some(&true) {
            eligible.insert(shared.inviter.clone());
        }
    }
    Ok(eligible)
}

/// Read-only invitation inventory, with decisions tied to the exact membership rows.
/// Persist only the subset included in a successfully constructed sync response.
pub(crate) struct InviteSyncSnapshot {
    pub(crate) rooms: std::collections::BTreeMap<OwnedRoomId, Vec<RawJson<AnyStrippedStateEvent>>>,
    pub(crate) until_sn: i64,
    #[cfg(feature = "unstable-msc4494")]
    pending_admissions: std::collections::HashMap<OwnedRoomId, (i64, OwnedEventId)>,
    #[cfg(feature = "unstable-msc4494")]
    device_id: OwnedDeviceId,
    #[cfg(feature = "unstable-msc4494")]
    user_id: OwnedUserId,
}

impl InviteSyncSnapshot {
    /// First delivery is owed independently of whether the event cursor advanced.
    pub(crate) fn has_pending_delivery(&self, room_id: &RoomId) -> bool {
        #[cfg(feature = "unstable-msc4494")]
        {
            self.pending_admissions.contains_key(room_id)
        }
        #[cfg(not(feature = "unstable-msc4494"))]
        {
            let _ = room_id;
            false
        }
    }

    pub(crate) async fn record_returned(&self, room_ids: &[&RoomId]) -> AppResult<()> {
        #[cfg(feature = "unstable-msc4494")]
        {
            let candidates: Vec<_> = room_ids
                .iter()
                .filter_map(|room_id| self.pending_admissions.get(*room_id).cloned())
                .collect();
            if !candidates.is_empty() {
                admit_pending_invites(candidates, self.until_sn, &self.device_id, &self.user_id)
                    .await?;
            }
        }
        #[cfg(not(feature = "unstable-msc4494"))]
        let _ = room_ids;
        Ok(())
    }
}

/// Read a consistent current invitation decision without changing the caller's
/// response window. The cursor's stream fences do not cover every sequence writer,
/// so invitation handling must never reacquire or advance the global boundary.
/// Shared membership/rule eligibility is current state, independently of that
/// event window; only invitation events are bounded by `until_sn`.
pub(crate) async fn invited_rooms_for_sync(
    user_id: &UserId,
    since_sn: i64,
    device_id: &DeviceId,
    until_sn: i64,
) -> AppResult<InviteSyncSnapshot> {
    #[cfg(feature = "unstable-msc4494")]
    let retained_action = Some(MEMBERSHIP_INVITE_ACTION);
    #[cfg(not(feature = "unstable-msc4494"))]
    let retained_action = None;
    #[cfg(feature = "unstable-msc4494")]
    let inventory = loop {
        let inventory = data::connect()
            .await?
            .build_transaction()
            .read_only()
            .read_committed()
            .run::<_, crate::AppError, _>(async |conn| {
                // A writer that commits while a lock is awaited must be visible.
                if !data::user::lock_invite_sync_state(conn, user_id, MEMBERSHIP_INVITE_ACTION)
                    .await?
                {
                    return Ok(None);
                }
                let inventory = data::user::invite_sync_inventory_with_conn(
                    conn,
                    user_id,
                    since_sn,
                    until_sn,
                    retained_action,
                    Some(device_id),
                )
                .await?;
                Ok(Some(inventory))
            })
            .await?;
        if let Some(inventory) = inventory {
            break inventory;
        }
        // Release every scope before rediscovery. The response boundary stays
        // fixed across retries, and no partial decision or admission is published.
        tokio::task::yield_now().await;
    };
    #[cfg(not(feature = "unstable-msc4494"))]
    let inventory = data::user::invite_sync_inventory(
        user_id,
        since_sn,
        until_sn,
        retained_action,
        Some(device_id),
    )
    .await?;
    let mut snapshot = InviteSyncSnapshot {
        rooms: std::collections::BTreeMap::new(),
        until_sn,
        #[cfg(feature = "unstable-msc4494")]
        pending_admissions: std::collections::HashMap::new(),
        #[cfg(feature = "unstable-msc4494")]
        device_id: device_id.to_owned(),
        #[cfg(feature = "unstable-msc4494")]
        user_id: user_id.to_owned(),
    };
    #[cfg(feature = "unstable-msc4494")]
    let deny_public = inventory.default_action.as_deref() == retained_action;
    #[cfg(feature = "unstable-msc4494")]
    let eligible = eligible_inviters(&inventory.shared_rooms).await?;
    for invite in inventory.invites {
        #[cfg(feature = "unstable-msc4494")]
        if deny_public {
            if invite.admitted_sn.is_none() && !eligible.contains(&invite.sender_id) {
                continue;
            }
            // Never-delivered, eligible invitations are sent once, even if their event
            // was committed late or the client already synced past the qualification.
            if let Some(delivered_sn) = invite.delivered_sn
                && invite.event_sn < inventory.replay_since_sn
                && delivered_sn < since_sn
            {
                continue;
            }
        }
        #[cfg(feature = "unstable-msc4494")]
        if invite.delivered_sn.is_none() {
            snapshot.pending_admissions.insert(
                invite.room_id.clone(),
                (invite.membership_id, invite.event_id),
            );
        }
        snapshot.rooms.insert(invite.room_id, invite.state);
    }
    Ok(snapshot)
}

#[cfg(feature = "unstable-msc4494")]
async fn admit_pending_invites(
    mut candidates: Vec<(i64, OwnedEventId)>,
    delivery_sn: i64,
    device_id: &DeviceId,
    user_id: &UserId,
) -> AppResult<()> {
    use data::schema::{room_invite_admissions, room_users};
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::{AsyncConnection, RunQueryDsl};

    // Consistent insert order prevents concurrent multi-room syncs from deadlocking.
    candidates.sort_unstable_by_key(|(id, _)| *id);
    data::connect()
        .await?
        .transaction::<_, crate::AppError, _>(async |conn| {
            data::user::lock_invite_user_write(conn, user_id).await?;
            // A concurrent join/leave can replace the pending membership. Hold its
            // key while inserting admissions, instead of racing the foreign key.
            let candidate_ids: Vec<_> = candidates.iter().map(|(id, _)| *id).collect();
            let live: std::collections::HashMap<_, _> = room_users::table
                .filter(room_users::id.eq_any(&candidate_ids))
                .filter(room_users::user_id.eq(user_id))
                .filter(room_users::membership.eq("invite"))
                .order_by(room_users::id.asc())
                .select((room_users::id, room_users::event_id))
                .for_key_share()
                .load::<(i64, OwnedEventId)>(conn)
                .await?
                .into_iter()
                .collect();
            let values: Vec<_> = candidates
                .iter()
                .filter(|(id, event_id)| live.get(id) == Some(event_id))
                .map(|(id, _)| {
                    (
                        room_invite_admissions::room_user_id.eq(*id),
                        room_invite_admissions::admitted_sn.eq(delivery_sn),
                        room_invite_admissions::delivered_devices
                            .eq(serde_json::json!({device_id.as_str(): delivery_sn})),
                    )
                })
                .collect();
            if !values.is_empty() {
                diesel::insert_into(room_invite_admissions::table)
                    .values(values)
                    .on_conflict(room_invite_admissions::room_user_id)
                    .do_update()
                    .set(room_invite_admissions::delivered_devices.eq(
                        diesel::dsl::sql::<diesel::sql_types::Jsonb>(
                            "EXCLUDED.delivered_devices || room_invite_admissions.delivered_devices",
                        ),
                    ))
                    .execute(conn)
                    .await?;
            }
            // Existing keys win, preserving the first delivery for each device.
            // The global admission position also remains unchanged.
            Ok(())
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

    #[derive(Clone, Copy)]
    enum InviteSyncStream {
        Ordinary,
        Sliding,
    }

    impl InviteSyncStream {
        async fn capture_cursor(self, user: &UserId, device: &DeviceId) -> AppResult<i64> {
            match self {
                Self::Ordinary => {
                    crate::event::sticky::curr_sn_after_sync_writes(user, device).await
                }
                Self::Sliding => {
                    #[cfg(feature = "unstable-msc4262")]
                    let sn =
                        data::user::curr_sn_after_presence_profile_and_inbox_writes(user, device)
                            .await?;
                    #[cfg(not(feature = "unstable-msc4262"))]
                    let sn =
                        data::user::curr_sn_after_presence_writes(Some((user, device))).await?;
                    Ok(sn)
                }
            }
        }
    }

    async fn snapshot_for_sync(
        user: &UserId,
        since_sn: i64,
        device: &DeviceId,
        stream: InviteSyncStream,
    ) -> AppResult<InviteSyncSnapshot> {
        let until_sn = stream.capture_cursor(user, device).await?;
        super::invited_rooms_for_sync(user, since_sn, device, until_sn).await
    }

    // These membership regressions model a sync returning every candidate invite.
    async fn invited_rooms_for_sync(
        user_id: &UserId,
        since_sn: i64,
    ) -> AppResult<Vec<(OwnedRoomId, Vec<RawJson<AnyStrippedStateEvent>>)>> {
        let snapshot =
            snapshot_for_sync(user_id, since_sn, "TEST".into(), InviteSyncStream::Ordinary).await?;
        snapshot
            .record_returned(&snapshot.rooms.keys().map(AsRef::as_ref).collect::<Vec<_>>())
            .await?;
        Ok(snapshot.rooms.into_iter().collect())
    }

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

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_invite_snapshot_waits_for_uncommitted_inbox_writes() {
        use std::time::Duration;

        use diesel::{ExpressionMethods, QueryDsl};

        use crate::data::schema::device_inboxes;

        async fn capture_cursor(user: &UserId, device: &DeviceId, sliding: bool) -> AppResult<i64> {
            if !sliding {
                return crate::event::sticky::curr_sn_after_sync_writes(user, device).await;
            }
            #[cfg(feature = "unstable-msc4262")]
            let sn =
                data::user::curr_sn_after_presence_profile_and_inbox_writes(user, device).await?;
            #[cfg(not(feature = "unstable-msc4262"))]
            let sn = data::user::curr_sn_after_presence_writes(Some((user, device))).await?;
            Ok(sn)
        }

        crate::test_database::init();
        let user: OwnedUserId = "@snapshot_cursor:example.org".try_into().unwrap();
        let device: OwnedDeviceId = "CURSOR".into();
        data::next_sn().await.unwrap();
        for sliding in [false, true] {
            let initial = capture_cursor(&user, &device, sliding).await.unwrap();
            // A separate connection models another server instance. Its sequence
            // allocation is immediately visible, while the inbox row is not.
            let mut writer = data::connect().await.unwrap();
            diesel::sql_query("BEGIN")
                .execute(&mut writer)
                .await
                .unwrap();
            data::user::device::lock_inbox_stream(&mut writer, &user, &device)
                .await
                .unwrap();
            let pending_sn = diesel::insert_into(device_inboxes::table)
                .values(data::user::device::NewDbDeviceInbox {
                    user_id: user.clone(),
                    device_id: device.clone(),
                    json_data: json!({"type": "m.test", "sender": user, "content": {}}),
                    created_at: 1,
                })
                .returning(device_inboxes::occur_sn)
                .get_result::<i64>(&mut writer)
                .await
                .unwrap();
            assert!(pending_sn > initial);
            let snapshot = snapshot_for_sync(
                &user,
                0,
                &device,
                if sliding {
                    InviteSyncStream::Sliding
                } else {
                    InviteSyncStream::Ordinary
                },
            );
            tokio::pin!(snapshot);
            assert!(
                tokio::time::timeout(Duration::from_millis(200), &mut snapshot)
                    .await
                    .is_err(),
                "sync must wait for the writer before publishing its new cursor (sliding={sliding})"
            );
            diesel::sql_query("COMMIT")
                .execute(&mut writer)
                .await
                .unwrap();
            let snapshot = snapshot.await.unwrap();
            assert!(snapshot.until_sn >= pending_sn);
            let delivered = device_inboxes::table
                .filter(device_inboxes::user_id.eq(&user))
                .filter(device_inboxes::device_id.eq(&device))
                .filter(device_inboxes::occur_sn.ge(initial + 1))
                .filter(device_inboxes::occur_sn.lt(snapshot.until_sn + 1))
                .select(device_inboxes::occur_sn)
                .load::<i64>(&mut data::connect().await.unwrap())
                .await
                .unwrap();
            assert_eq!(delivered, vec![pending_sn]);
        }
    }

    #[cfg(feature = "unstable-msc4494")]
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_invite_snapshot_checks_current_eligibility_without_advancing_cursor() {
        use std::sync::Arc;

        use diesel::{ExpressionMethods, JoinOnDsl, QueryDsl};

        use crate::core::serde::CanonicalJsonObject;
        use crate::data::schema::{room_invite_admissions, room_users, rooms};
        use crate::room::state::{CompressedEvent, CompressedState};

        async fn set_rule(room: &RoomId, sender: &UserId, case: &str, rule: &str) -> i64 {
            let raw = json!({
                "event_id": format!("$boundary_{case}_{rule}:example.org"), "room_id": room,
                "type": "m.room.join_rules", "sender": sender, "state_key": "",
                "content": {"join_rule": rule}, "origin_server_ts": 1,
                "depth": 1, "auth_events": [], "prev_events": [], "hashes": {"sha256": "test"}
            });
            let pdu: PduEvent = serde_json::from_value(raw.clone()).unwrap();
            let canonical: CanonicalJsonObject = serde_json::from_value(raw).unwrap();
            let (stored, _, guard) = PduBuilder::save_as_outlier(pdu, canonical, sender)
                .await
                .unwrap();
            let field =
                state::ensure_field_id(&crate::core::events::StateEventType::RoomJoinRules, "")
                    .await
                    .unwrap();
            let compressed: CompressedState = [CompressedEvent::new(field, stored.event_sn)]
                .into_iter()
                .collect();
            let delta = state::save_state(room, Arc::new(compressed)).await.unwrap();
            state::set_room_state(room, delta.frame_id).await.unwrap();
            let sn = stored.event_sn;
            drop(guard);
            sn
        }

        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name": "dynamic.example", "db": {"url": "unused-test-config"}
            }))
            .unwrap()
        });
        for (case, stream) in [
            "recipient_leave",
            "inviter_leave",
            "recipient_profile",
            "inviter_profile",
            "rule_public",
            "rule_knock",
            "rule_gain",
            "rule_first",
            "permission_deny_public",
            "permission_block",
            "ignore",
        ]
        .into_iter()
        .flat_map(|case| {
            [InviteSyncStream::Ordinary, InviteSyncStream::Sliding].map(|stream| (case, stream))
        }) {
            let label = format!(
                "{case}_{}",
                match stream {
                    InviteSyncStream::Ordinary => "ordinary",
                    InviteSyncStream::Sliding => "sliding",
                }
            );
            let recipient: OwnedUserId = format!("@boundary_{label}:dynamic.example")
                .try_into()
                .unwrap();
            let inviter: OwnedUserId = format!("@boundary_{label}:example.org").try_into().unwrap();
            // Membership updates must not create a missing remote user's profile
            // while this fixture deliberately holds a sync stream lock.
            data::user::create_user(&data::user::NewDbUser {
                id: inviter.clone(),
                ty: None,
                is_admin: false,
                is_guest: false,
                is_local: false,
                localpart: inviter.localpart().to_owned(),
                server_name: inviter.server_name().to_owned(),
                appservice_id: None,
                created_at: crate::core::UnixMillis::now(),
            })
            .await
            .unwrap();
            let shared: OwnedRoomId = format!("!boundary_shared_{label}:example.org")
                .try_into()
                .unwrap();
            let target: OwnedRoomId = format!("!boundary_invite_{label}:example.org")
                .try_into()
                .unwrap();
            for room in [&shared, &target] {
                diesel::insert_into(rooms::table)
                    .values(data::room::NewDbRoom {
                        id: room.clone(),
                        version: "11".into(),
                        is_public: false,
                        min_depth: 0,
                        has_auth_chain_index: false,
                        created_at: crate::core::UnixMillis::now(),
                    })
                    .execute(&mut data::connect().await.unwrap())
                    .await
                    .unwrap();
            }
            data::user::set_data(
                &recipient,
                None,
                "m.invite_permission_config",
                json!({"default_action": if case == "permission_deny_public" { "allow" } else { "uk.timedout.msc4494.deny_public" }}),
            )
            .await
            .unwrap();
            for (i, (room, user, membership)) in [
                (&shared, &recipient, "join"),
                (&shared, &inviter, "join"),
                (&target, &recipient, "invite"),
            ]
            .into_iter()
            .enumerate()
            {
                diesel::insert_into(room_users::table)
                    .values(data::room::NewDbRoomUser {
                        event_id: format!("$boundary_{label}_{i}:example.org")
                            .try_into()
                            .unwrap(),
                        event_sn: data::next_sn().await.unwrap(),
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
            if case != "rule_first" {
                set_rule(
                    &shared,
                    &inviter,
                    &label,
                    if matches!(case, "permission_deny_public" | "rule_gain") {
                        "public"
                    } else {
                        "invite"
                    },
                )
                .await;
            }
            let captured = stream
                .capture_cursor(&recipient, "TEST".into())
                .await
                .unwrap();
            let before = super::invited_rooms_for_sync(&recipient, 0, "TEST".into(), captured)
                .await
                .unwrap();
            assert_eq!(
                before.rooms.len(),
                usize::from(!matches!(case, "rule_gain" | "rule_first")),
                "{case}: initial eligibility"
            );

            // Eligibility is current state. A later membership, rule or preference
            // change must affect the decision without enlarging the event window.
            let changed = if case.starts_with("permission_") {
                data::user::set_data(&recipient, None, "m.invite_permission_config",
                    json!({"default_action": if case == "permission_block" { "block" } else { "uk.timedout.msc4494.deny_public" }}),
                ).await.unwrap().occur_sn
            } else if case == "ignore" {
                data::user::set_ignored_users(&recipient, std::slice::from_ref(&inviter))
                    .await
                    .unwrap();
                data::next_sn().await.unwrap()
            } else if case.starts_with("rule_") {
                set_rule(
                    &shared,
                    &inviter,
                    &label,
                    if matches!(case, "rule_gain" | "rule_first") {
                        "invite"
                    } else {
                        case.strip_prefix("rule_").unwrap()
                    },
                )
                .await
            } else {
                let member = if case.starts_with("recipient_") {
                    &recipient
                } else {
                    &inviter
                };
                let event_sn = data::next_sn().await.unwrap();
                crate::membership::update_membership(
                    &EventId::parse(format!("$boundary_{label}_replacement:example.org")).unwrap(),
                    event_sn,
                    &shared,
                    member,
                    if case.ends_with("leave") {
                        MembershipState::Leave
                    } else {
                        MembershipState::Join
                    },
                    &inviter,
                    None,
                )
                .await
                .unwrap();
                event_sn
            };
            assert!(changed > captured);
            let after = super::invited_rooms_for_sync(&recipient, 0, "TEST".into(), captured)
                .await
                .unwrap();
            assert_eq!(
                after.until_sn, captured,
                "{case}: current eligibility must preserve the original event window"
            );
            let qualifies = case.ends_with("profile")
                || matches!(case, "rule_knock" | "rule_gain" | "rule_first");
            assert_eq!(after.rooms.len(), usize::from(qualifies), "{case}");
            assert!(
                room_invite_admissions::table
                    .select(room_invite_admissions::room_user_id)
                    .filter(
                        room_invite_admissions::room_user_id.eq_any(
                            room_users::table
                                .filter(room_users::user_id.eq(&recipient))
                                .select(room_users::id)
                        )
                    )
                    .load::<i64>(&mut data::connect().await.unwrap())
                    .await
                    .unwrap()
                    .is_empty()
            );
            after.record_returned(&[target.as_ref()]).await.unwrap();
            let admissions = room_invite_admissions::table
                .inner_join(
                    room_users::table.on(room_users::id.eq(room_invite_admissions::room_user_id)),
                )
                .filter(room_users::user_id.eq(&recipient))
                .select(room_invite_admissions::admitted_sn)
                .load::<i64>(&mut data::connect().await.unwrap())
                .await
                .unwrap();
            assert_eq!(
                admissions,
                if qualifies {
                    vec![after.until_sn]
                } else {
                    vec![]
                }
            );
            if case.ends_with("leave") {
                // One device prepares an eligible response. Another observes the
                // leave and advances its cursor before the first records delivery.
                before.record_returned(&[target.as_ref()]).await.unwrap();
                data::next_sn().await.unwrap();
                let other: OwnedDeviceId = "OTHER".into();
                let recovered = snapshot_for_sync(
                    &recipient,
                    after.until_sn + 1,
                    &other,
                    InviteSyncStream::Ordinary,
                )
                .await
                .unwrap();
                assert_eq!(
                    recovered.rooms.len(),
                    1,
                    "{case}: a delayed delivery on another device must not be lost behind its cursor"
                );
                let returned = [target.as_ref()];
                let (repeat, first) = tokio::join!(
                    before.record_returned(&returned),
                    recovered.record_returned(&returned),
                );
                repeat.unwrap();
                first.unwrap();
                let (admitted_sn, devices) = room_invite_admissions::table
                    .inner_join(
                        room_users::table
                            .on(room_users::id.eq(room_invite_admissions::room_user_id)),
                    )
                    .filter(room_users::room_id.eq(&target))
                    .filter(room_users::user_id.eq(&recipient))
                    .select((
                        room_invite_admissions::admitted_sn,
                        room_invite_admissions::delivered_devices,
                    ))
                    .first::<(i64, serde_json::Value)>(&mut data::connect().await.unwrap())
                    .await
                    .unwrap();
                assert_eq!(admitted_sn, before.until_sn);
                assert_eq!(
                    devices,
                    json!({"TEST": before.until_sn, "OTHER": recovered.until_sn})
                );
                data::next_sn().await.unwrap();
                let next = snapshot_for_sync(
                    &recipient,
                    recovered.until_sn + 1,
                    &other,
                    InviteSyncStream::Ordinary,
                )
                .await
                .unwrap();
                assert!(
                    next.rooms.is_empty(),
                    "{case}: this device must not receive a duplicate"
                );
            }
        }
    }

    #[cfg(feature = "unstable-msc4494")]
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_invite_snapshot_does_not_cross_unpublished_ordinary_events() {
        use diesel_async::AsyncConnection;

        use crate::core::serde::CanonicalJsonObject;
        use crate::data::schema::rooms;

        crate::test_database::init();
        for stream in [InviteSyncStream::Ordinary, InviteSyncStream::Sliding] {
            let label = match stream {
                InviteSyncStream::Ordinary => "ordinary",
                InviteSyncStream::Sliding => "sliding",
            };
            let user: OwnedUserId = format!("@publication_{label}:example.org")
                .try_into()
                .unwrap();
            let room: OwnedRoomId = format!("!publication_{label}:example.org")
                .try_into()
                .unwrap();
            let event: OwnedEventId = format!("$publication_{label}:example.org")
                .try_into()
                .unwrap();
            let device: OwnedDeviceId = "PUBLICATION".into();
            diesel::insert_into(rooms::table)
                .values(data::room::NewDbRoom {
                    id: room.clone(),
                    version: "11".into(),
                    is_public: false,
                    min_depth: 0,
                    has_auth_chain_index: false,
                    created_at: crate::core::UnixMillis::now(),
                })
                .execute(&mut data::connect().await.unwrap())
                .await
                .unwrap();
            let captured = stream.capture_cursor(&user, &device).await.unwrap();
            let raw = json!({
                "event_id": event, "room_id": room, "sender": user,
                "type": "m.room.message", "content": {"msgtype": "m.text", "body": "pending publication"},
                "origin_server_ts": 1, "depth": 1, "auth_events": [], "prev_events": [],
                "hashes": {"sha256": "test"}
            });
            let pdu: PduEvent = serde_json::from_value(raw.clone()).unwrap();
            let canonical: CanonicalJsonObject = serde_json::from_value(raw).unwrap();
            let (stored, _, guard) = PduBuilder::save_as_outlier(pdu, canonical, &user)
                .await
                .unwrap();
            // Another node's queue guard cannot be observed by this sync process.
            drop(guard);
            assert!(stored.event_sn > captured);
            assert!(stored.sticky_duration_ms().is_none());
            let snapshot = super::invited_rooms_for_sync(&user, 0, &device, captured)
                .await
                .unwrap();
            assert_eq!(
                snapshot.until_sn, captured,
                "{label}: invitation reads must preserve the original response boundary"
            );
            assert!(
                data::room::timeline::get_pdus_by_room(&room, Some(captured), 10, false)
                    .await
                    .unwrap()
                    .is_empty()
            );
            data::connect()
                .await
                .unwrap()
                .transaction::<_, crate::AppError, _>(async |conn| {
                    crate::event::sticky::promote_to_timeline_with_conn(conn, &stored).await
                })
                .await
                .unwrap();
            let next =
                data::room::timeline::get_pdus_by_room(&room, Some(snapshot.until_sn), 10, false)
                    .await
                    .unwrap();
            assert_eq!(
                next.iter().map(|pdu| &pdu.id).collect::<Vec<_>>(),
                vec![&event]
            );
        }
    }

    #[cfg(feature = "unstable-msc4494")]
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_invite_snapshot_recovers_late_admission_at_an_idle_cursor() {
        use crate::core::client::sync_events::v3::SyncEventsReqArgs;
        use crate::data::schema::{room_users, rooms};

        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name": "dynamic.example", "db": {"url": "unused-test-config"}
            }))
            .unwrap()
        });
        let user: OwnedUserId = "@idle_admission:dynamic.example".try_into().unwrap();
        let sender: OwnedUserId = "@idle_sender:example.org".try_into().unwrap();
        let room: OwnedRoomId = "!idle_admission:example.org".try_into().unwrap();
        let device: OwnedDeviceId = "IDLE".into();
        diesel::insert_into(rooms::table)
            .values(data::room::NewDbRoom {
                id: room.clone(),
                version: "11".into(),
                is_public: false,
                min_depth: 0,
                has_auth_chain_index: false,
                created_at: crate::core::UnixMillis::now(),
            })
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        diesel::insert_into(room_users::table)
            .values(data::room::NewDbRoomUser {
                room_id: room.clone(),
                room_server_id: None,
                user_id: user.clone(),
                user_server_id: user.server_name().to_owned(),
                sender_id: sender,
                event_id: "$idle_admission:example.org".try_into().unwrap(),
                event_sn: data::next_sn().await.unwrap(),
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
        data::user::set_data(
            &user,
            None,
            "m.invite_permission_config",
            json!({"default_action": "allow"}),
        )
        .await
        .unwrap();
        crate::user::get_push_rules(&user).await.unwrap();
        let early = snapshot_for_sync(&user, 0, "EARLY".into(), InviteSyncStream::Ordinary)
            .await
            .unwrap();
        assert_eq!(early.rooms.len(), 1);
        data::user::set_data(
            &user,
            None,
            "m.invite_permission_config",
            json!({"default_action": MEMBERSHIP_INVITE_ACTION}),
        )
        .await
        .unwrap();
        let initial: SyncEventsReqArgs = serde_json::from_value(json!({})).unwrap();
        let hidden = crate::sync_v3::sync_events(&user, &device, &initial)
            .await
            .unwrap();
        assert!(hidden.rooms.invite.is_empty());
        let idle_sn = data::curr_sn().await.unwrap();
        assert_eq!(
            hidden.next_batch,
            crate::event::BatchToken::new_live(idle_sn + 1).to_string()
        );
        // Another device completes its earlier response behind this device's cursor.
        // Admission does not allocate a new event position to wake the early-return path.
        early.record_returned(&[room.as_ref()]).await.unwrap();
        assert_eq!(data::curr_sn().await.unwrap(), idle_sn);
        let args: SyncEventsReqArgs =
            serde_json::from_value(json!({"since": hidden.next_batch})).unwrap();
        let recovered = crate::sync_v3::sync_events(&user, &device, &args)
            .await
            .unwrap();
        assert!(
            recovered.rooms.invite.contains_key(&room),
            "an idle sync must check delayed first deliveries before returning"
        );
        assert_eq!(recovered.next_batch, hidden.next_batch);
        assert_eq!(data::curr_sn().await.unwrap(), idle_sn);
        let repeated = crate::sync_v3::sync_events(&user, &device, &args)
            .await
            .unwrap();
        assert!(repeated.rooms.invite.is_empty());
        assert_eq!(repeated.next_batch, hidden.next_batch);
        let future: SyncEventsReqArgs = serde_json::from_value(
            json!({"since": crate::event::BatchToken::new_live(idle_sn + 2).to_string()}),
        )
        .unwrap();
        let rejected = crate::sync_v3::sync_events(&user, &device, &future)
            .await
            .unwrap();
        assert!(rejected.rooms.invite.is_empty());
        assert_eq!(rejected.next_batch, future.since.unwrap());
    }

    #[cfg(feature = "unstable-msc4494")]
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_invite_snapshot_retries_new_senders_while_waiting_for_locks() {
        use std::time::Duration;

        use diesel_async::AsyncConnection;

        use crate::data::schema::{room_users, rooms};

        crate::test_database::init();
        let a: OwnedUserId = "@scope_retry_a:dynamic.example".try_into().unwrap();
        let z: OwnedUserId = "@scope_retry_z:dynamic.example".try_into().unwrap();
        async fn key(user: &UserId) -> i64 {
            diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT hashtextextended(")
                .bind::<diesel::sql_types::Text, _>(user.as_str())
                .sql(", ")
                .bind::<diesel::sql_types::BigInt, _>(0x494e5655_i64)
                .sql(")")
                .get_result(&mut data::connect().await.unwrap())
                .await
                .unwrap()
        }
        let (sender, recipient) = if key(&a).await < key(&z).await {
            (a, z)
        } else {
            (z, a)
        };
        let newcomer: OwnedUserId = "@scope_retry_new:dynamic.example".try_into().unwrap();
        let first: OwnedRoomId = "!scope_retry_first:example.org".try_into().unwrap();
        let second: OwnedRoomId = "!scope_retry_second:example.org".try_into().unwrap();
        for room in [&first, &second] {
            diesel::insert_into(rooms::table)
                .values(data::room::NewDbRoom {
                    id: room.clone(),
                    version: "11".into(),
                    is_public: false,
                    min_depth: 0,
                    has_auth_chain_index: false,
                    created_at: crate::core::UnixMillis::now(),
                })
                .execute(&mut data::connect().await.unwrap())
                .await
                .unwrap();
        }
        data::user::set_data(
            &recipient,
            None,
            "m.invite_permission_config",
            json!({"default_action": "uk.timedout.msc4494.deny_public"}),
        )
        .await
        .unwrap();
        async fn store_invite(recipient: &UserId, sender: &UserId, room: &RoomId) -> AppResult<()> {
            data::connect()
                .await?
                .transaction::<_, crate::AppError, _>(async |conn| {
                    data::user::lock_invite_user_write(conn, recipient).await?;
                    diesel::insert_into(room_users::table)
                        .values(data::room::NewDbRoomUser {
                            event_id: EventId::parse(format!(
                                "${}:example.org",
                                sender.localpart()
                            ))
                            .unwrap(),
                            event_sn: diesel::dsl::sql::<diesel::sql_types::BigInt>(
                                "SELECT nextval('occur_sn_seq')",
                            )
                            .get_result(conn)
                            .await?,
                            room_id: room.to_owned(),
                            room_server_id: None,
                            user_id: recipient.to_owned(),
                            user_server_id: recipient.server_name().to_owned(),
                            sender_id: sender.to_owned(),
                            membership: "invite".into(),
                            forgotten: false,
                            display_name: None,
                            avatar_url: None,
                            state_data: Some(json!([])),
                            created_at: crate::core::UnixMillis::now(),
                        })
                        .execute(conn)
                        .await?;
                    Ok(())
                })
                .await
        }
        store_invite(&recipient, &sender, &first).await.unwrap();
        let mut blocker = data::connect().await.unwrap();
        diesel::sql_query("BEGIN")
            .execute(&mut blocker)
            .await
            .unwrap();
        data::user::lock_invite_user_write(&mut blocker, &sender)
            .await
            .unwrap();
        let snapshot = snapshot_for_sync(&recipient, 0, "RETRY".into(), InviteSyncStream::Ordinary);
        tokio::pin!(snapshot);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut snapshot)
                .await
                .is_err()
        );
        // The sorted first key is blocked, so the recipient is still writable.
        tokio::time::timeout(
            Duration::from_secs(2),
            store_invite(&recipient, &newcomer, &second),
        )
        .await
        .unwrap()
        .unwrap();
        let mut new_blocker = data::connect().await.unwrap();
        diesel::sql_query("BEGIN")
            .execute(&mut new_blocker)
            .await
            .unwrap();
        data::user::lock_invite_user_write(&mut new_blocker, &newcomer)
            .await
            .unwrap();
        diesel::sql_query("COMMIT")
            .execute(&mut blocker)
            .await
            .unwrap();
        drop(blocker);
        // The first attempt did not know this scope. It must release its locks,
        // retry discovery, and now wait for the newcomer before publishing.
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut snapshot)
                .await
                .is_err()
        );
        diesel::sql_query("COMMIT")
            .execute(&mut new_blocker)
            .await
            .unwrap();
        drop(new_blocker);
        assert!(
            tokio::time::timeout(Duration::from_secs(3), snapshot)
                .await
                .unwrap()
                .unwrap()
                .rooms
                .is_empty()
        );
    }

    #[cfg(feature = "unstable-msc4494")]
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_invite_writers_wait_for_the_sync_read_phase() {
        use std::sync::Arc;
        use std::time::Duration;

        use crate::data::schema::{room_users, rooms};

        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name": "dynamic.example", "db": {"url": "unused-test-config"}
            }))
            .unwrap()
        });
        let user: OwnedUserId = "@phase_reader:dynamic.example".try_into().unwrap();
        let inviter: OwnedUserId = "@phase_inviter:dynamic.example".try_into().unwrap();
        let room: OwnedRoomId = "!phase_read:example.org".try_into().unwrap();
        let shared: OwnedRoomId = "!phase_shared:example.org".try_into().unwrap();
        diesel::insert_into(rooms::table)
            .values(data::room::NewDbRoom {
                id: room.clone(),
                version: "11".into(),
                is_public: false,
                min_depth: 0,
                has_auth_chain_index: false,
                created_at: crate::core::UnixMillis::now(),
            })
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        diesel::insert_into(rooms::table)
            .values(data::room::NewDbRoom {
                id: shared.clone(),
                version: "11".into(),
                is_public: false,
                min_depth: 0,
                has_auth_chain_index: false,
                created_at: crate::core::UnixMillis::now(),
            })
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        for (i, member) in [&user, &inviter].into_iter().enumerate() {
            diesel::insert_into(room_users::table)
                .values(data::room::NewDbRoomUser {
                    event_id: EventId::parse(format!("$phase_join_{i}:example.org")).unwrap(),
                    event_sn: data::next_sn().await.unwrap(),
                    room_id: shared.clone(),
                    room_server_id: None,
                    user_id: member.clone(),
                    user_server_id: member.server_name().to_owned(),
                    sender_id: inviter.clone(),
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
        let frame = state::save_state(&shared, Arc::new(Default::default()))
            .await
            .unwrap()
            .frame_id;
        let event: OwnedEventId = "$phase_invite:example.org".try_into().unwrap();
        let id = diesel::insert_into(room_users::table)
            .values(data::room::NewDbRoomUser {
                event_id: event.clone(),
                event_sn: data::next_sn().await.unwrap(),
                room_id: room.clone(),
                room_server_id: None,
                user_id: user.clone(),
                user_server_id: user.server_name().to_owned(),
                sender_id: inviter.clone(),
                membership: "invite".into(),
                forgotten: false,
                display_name: None,
                avatar_url: None,
                state_data: Some(json!([])),
                created_at: crate::core::UnixMillis::now(),
            })
            .returning(room_users::id)
            .get_result::<i64>(&mut data::connect().await.unwrap())
            .await
            .unwrap();

        for operation in [
            "permission",
            "delete_permission",
            "ignore",
            "frame",
            "inviter_leave",
            "admission",
            "leave",
        ] {
            data::user::set_data(
                &user,
                None,
                "m.invite_permission_config",
                json!({"default_action": "uk.timedout.msc4494.deny_public"}),
            )
            .await
            .unwrap();
            data::user::set_ignored_users(&user, &[]).await.unwrap();
            let mut reader = data::connect().await.unwrap();
            diesel::sql_query("BEGIN ISOLATION LEVEL READ COMMITTED")
                .execute(&mut reader)
                .await
                .unwrap();
            data::user::lock_invite_sync_state(
                &mut reader,
                &user,
                "uk.timedout.msc4494.deny_public",
            )
            .await
            .unwrap();
            let inventory = data::user::invite_sync_inventory_with_conn(
                &mut reader,
                &user,
                0,
                i64::MAX,
                Some("uk.timedout.msc4494.deny_public"),
                Some("PHASE".into()),
            )
            .await
            .unwrap();

            // Another device can hold the shared decision lock concurrently.
            let mut other = data::connect().await.unwrap();
            diesel::sql_query("BEGIN")
                .execute(&mut other)
                .await
                .unwrap();
            tokio::time::timeout(
                Duration::from_millis(200),
                data::user::lock_invite_sync_state(
                    &mut other,
                    &user,
                    "uk.timedout.msc4494.deny_public",
                ),
            )
            .await
            .unwrap()
            .unwrap();
            diesel::sql_query("COMMIT")
                .execute(&mut other)
                .await
                .unwrap();
            drop(other);

            assert_eq!(inventory.invites.len(), 1);
            if operation == "permission" {
                // An unrelated user's write must not wait for these scopes.
                let unrelated: OwnedUserId = "@phase_unrelated:dynamic.example".try_into().unwrap();
                tokio::time::timeout(
                    Duration::from_millis(200),
                    data::user::set_data(
                        &unrelated,
                        None,
                        "m.invite_permission_config",
                        json!({"default_action": "block"}),
                    ),
                )
                .await
                .unwrap()
                .unwrap();
            }

            let writer = async {
                match operation {
                    "permission" => {
                        data::user::set_data(
                            &user,
                            None,
                            "m.invite_permission_config",
                            json!({"default_action": "block"}),
                        )
                        .await?;
                    }
                    "delete_permission" => {
                        data::user::delete_global_data(&user, "m.invite_permission_config").await?;
                    }
                    "ignore" => {
                        data::user::set_ignored_users(&user, std::slice::from_ref(&inviter))
                            .await?;
                    }
                    "frame" => {
                        state::set_room_state(&shared, frame).await?;
                    }
                    "inviter_leave" => {
                        crate::membership::update_membership(
                            &EventId::parse("$phase_inviter_leave:example.org").unwrap(),
                            data::next_sn().await?,
                            &shared,
                            &inviter,
                            MembershipState::Leave,
                            &inviter,
                            None,
                        )
                        .await?;
                    }
                    "admission" => {
                        admit_pending_invites(
                            vec![(id, event.clone())],
                            data::curr_sn().await?,
                            "PHASE".into(),
                            &user,
                        )
                        .await?;
                    }
                    "leave" => {
                        crate::membership::update_membership(
                            &EventId::parse("$phase_leave:example.org").unwrap(),
                            data::next_sn().await?,
                            &room,
                            &user,
                            MembershipState::Leave,
                            &inviter,
                            None,
                        )
                        .await?;
                    }
                    _ => unreachable!(),
                }
                AppResult::Ok(())
            };
            tokio::pin!(writer);
            assert!(
                tokio::time::timeout(Duration::from_millis(200), &mut writer)
                    .await
                    .is_err(),
                "{operation} committed inside the protected read phase"
            );
            diesel::sql_query("COMMIT")
                .execute(&mut reader)
                .await
                .unwrap();
            drop(reader);
            tokio::time::timeout(Duration::from_secs(3), writer)
                .await
                .unwrap()
                .unwrap();
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

        let mut qualifying_frame = None;
        let mut public_frame = None;
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
            if allowed {
                qualifying_frame = Some(delta.frame_id);
            }
            if *rule == "public" {
                public_frame = Some(delta.frame_id);
            }
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
            let inventory =
                snapshot_for_sync(&invitee, 0, "TEST".into(), InviteSyncStream::Ordinary)
                    .await
                    .unwrap();
            assert_eq!(inventory.rooms.len(), usize::from(allowed));
            use crate::data::schema::room_invite_admissions;
            assert!(
                room_invite_admissions::table
                    .filter(room_invite_admissions::room_user_id.eq(fresh_invite.id))
                    .select(room_invite_admissions::room_user_id)
                    .load::<i64>(&mut data::connect().await.unwrap())
                    .await
                    .unwrap()
                    .is_empty(),
                "even qualifying inventory must not admit an invitation before delivery"
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
        // Current qualification and the rule used to prove it share a pinned frame.
        state::set_room_state(&mutual, qualifying_frame.unwrap())
            .await
            .unwrap();
        let permission = data::user::invite_permission_snapshot(
            &invitee,
            std::slice::from_ref(&inviter),
            Some("uk.timedout.msc4494.deny_public"),
        )
        .await
        .unwrap();
        assert!(
            eligible_inviters(&permission.shared_rooms)
                .await
                .unwrap()
                .contains(&inviter),
            "current qualification is independent of the invitation event window"
        );
        assert!(
            super::invited_rooms_for_sync(&invitee, 0, "TEST".into(), 0)
                .await
                .unwrap()
                .rooms
                .is_empty(),
            "invitations beyond the original event window must wait for a later sync"
        );
        state::set_room_state(&mutual, public_frame.unwrap())
            .await
            .unwrap();
        assert!(
            eligible_inviters(&permission.shared_rooms)
                .await
                .unwrap()
                .contains(&inviter),
            "rule lookup must use the captured frame instead of a newer rule"
        );
        assert_blocked(ensure_invite_allowed(&invitee, &inviter).await);
        state::set_room_state(&mutual, qualifying_frame.unwrap())
            .await
            .unwrap();
        // Reserve a stream position, then commit the new invite after a client passed it.
        let mut delayed = room_users::table
            .filter(room_users::user_id.eq(&invitee))
            .filter(room_users::room_id.eq(&target))
            .first::<data::room::DbRoomUser>(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        diesel::delete(room_users::table.find(delayed.id))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        delayed.id = diesel::dsl::sql::<diesel::sql_types::BigInt>(
            "SELECT nextval(pg_get_serial_sequence('room_users', 'id'))",
        )
        .get_result(&mut data::connect().await.unwrap())
        .await
        .unwrap();
        delayed.event_id = "$membership_delayed_invite:example.org".try_into().unwrap();
        delayed.event_sn = data::next_sn().await.unwrap();
        let since = delayed.event_sn + 1;
        for _ in 0..3 {
            data::next_sn().await.unwrap();
        }
        let until = data::curr_sn().await.unwrap();
        let delayed_id = delayed.id;
        diesel::insert_into(room_users::table)
            .values(delayed)
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        let snapshot =
            snapshot_for_sync(&invitee, since, "TEST".into(), InviteSyncStream::Ordinary)
                .await
                .unwrap();
        assert_eq!(
            snapshot.rooms.len(),
            1,
            "an eligible, never-delivered invite must survive a passed event cursor"
        );
        snapshot.record_returned(&[target.as_ref()]).await.unwrap();
        use crate::data::schema::room_invite_admissions;
        let first_delivery = room_invite_admissions::table
            .find(delayed_id)
            .select(room_invite_admissions::admitted_sn)
            .first::<i64>(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            first_delivery, until,
            "delivery has its own position, separate from qualification and event positions"
        );
        data::next_sn().await.unwrap();
        assert!(
            snapshot_for_sync(
                &invitee,
                until + 1,
                "TEST".into(),
                InviteSyncStream::Ordinary
            )
            .await
            .unwrap()
            .rooms
            .is_empty(),
            "the delivering client must not receive a duplicate"
        );
        assert_eq!(
            snapshot_for_sync(&invitee, since, "TEST".into(), InviteSyncStream::Ordinary)
                .await
                .unwrap()
                .rooms
                .len(),
            1,
            "another device with an older cursor must receive the admission"
        );
        let captured = data::curr_sn().await.unwrap();
        data::user::set_data(
            &invitee,
            None,
            "m.invite_permission_config",
            json!({"default_action": "allow"}),
        )
        .await
        .unwrap();
        let permission_update =
            snapshot_for_sync(&invitee, 0, "TEST".into(), InviteSyncStream::Ordinary)
                .await
                .unwrap();
        assert!(permission_update.until_sn > captured);
        assert_eq!(
            permission_update.rooms.len(),
            1,
            "the response boundary must include the observed permission update"
        );
        assert_eq!(
            snapshot_for_sync(&invitee, 0, "TEST".into(), InviteSyncStream::Ordinary)
                .await
                .unwrap()
                .rooms
                .len(),
            1
        );
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
