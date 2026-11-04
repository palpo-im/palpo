//! Selective presence sharing ([MSC4495]).
//!
//! Presence is off by default under this proposal: a user's presence goes only to the
//! **recipient user set** their `m.presence.sharing` account data produces, and a user with
//! no configuration shares presence with nobody. That is deliberate -- falling back to the
//! legacy "everyone you share a room with" behaviour for unconfigured users would let a
//! client opt out of selective presence just by never writing the account data.
//!
//! [MSC4495]: https://github.com/matrix-org/matrix-spec-proposals/pull/4495

use std::collections::{BTreeMap, BTreeSet};

use crate::core::events::presence::sharing::{
    PresenceSharingEventContent, RoomPresenceSharingState, ServerPresenceSharingState,
    UserPresenceSharingState,
};
use crate::core::events::room::presence_sharing::{
    PresenceSharingHint, RoomPresenceSharingEventContent,
};
use crate::core::events::{GlobalAccountDataEventType, StateEventType, StaticEventContent};
use crate::core::identifiers::*;
use crate::{AppResult, config, data};

/// A room the sender is in, as far as the dispatch algorithm is concerned.
#[derive(Debug, Clone)]
pub struct SharedRoom {
    pub room_id: OwnedRoomId,
    /// The room's `m.room.presence_sharing` hint. A room without the state event is
    /// treated as `forbid`.
    pub hint: PresenceSharingHint,
    pub members: Vec<OwnedUserId>,
}

/// Applies the MSC4495 dispatch algorithm to produce a user's recipient user set.
///
/// The steps and their order are the proposal's, and the order matters: a server-wide
/// `deny` is applied before an explicit user `allow`, so naming a user always overrides a
/// blanket rule about their server, while a user `deny` is applied last and overrides
/// everything.
///
/// The sender is never their own recipient.
///
/// `local_server` is passed in rather than read from configuration so the algorithm stays
/// a pure function of its inputs.
pub fn effective_recipients(
    sender_id: &UserId,
    policy: &PresenceSharingEventContent,
    rooms: &[SharedRoom],
    local_server: &ServerName,
) -> BTreeSet<OwnedUserId> {
    let mut recipients = BTreeSet::new();

    // 1. Members of rooms the user allows, but only where the room's hint invites it. A room
    //    without a `suggest` hint contributes nobody, however the user configured it: the hint is
    //    how a room that has outgrown presence turns it off for everyone.
    for room in rooms {
        let allowed = matches!(
            policy.rooms.get(&room.room_id),
            Some(RoomPresenceSharingState::Allow)
        );
        if allowed && room.hint == PresenceSharingHint::Suggest {
            recipients.extend(room.members.iter().cloned());
        }
    }

    // 2. Drop everyone on a denied server.
    recipients.retain(|user_id| {
        !matches!(
            policy.servers.get(user_id.server_name()),
            Some(ServerPresenceSharingState::Deny)
        )
    });

    // Whoever the sender actually shares a room with. Steps 3 and 4 are limited to these:
    // the proposal only asks servers to include users the sender has some connection to,
    // and it keeps a stale entry in the configuration from generating federation traffic.
    let shares_room: BTreeSet<&UserId> = rooms
        .iter()
        .flat_map(|room| room.members.iter().map(AsRef::as_ref))
        .collect();

    // 3. Every local user the sender shares a room with, when sharing locally.
    if policy.share_locally {
        recipients.extend(
            shares_room
                .iter()
                .filter(|user_id| user_id.server_name() == local_server)
                .map(|user_id| (*user_id).to_owned()),
        );
    }

    // 4. Explicitly allowed users, overriding a denied server.
    for (user_id, state) in &policy.users {
        if *state == UserPresenceSharingState::Allow
            && shares_room.contains(AsRef::<UserId>::as_ref(user_id))
        {
            recipients.insert(user_id.clone());
        }
    }

    // 5. Explicitly denied users, overriding everything above.
    for (user_id, state) in &policy.users {
        if *state == UserPresenceSharingState::Deny {
            recipients.remove(user_id);
        }
    }

    recipients.remove(sender_id);
    recipients
}

/// Groups a recipient user set by the server that must be told about each recipient.
pub fn by_server(
    recipients: BTreeSet<OwnedUserId>,
) -> BTreeMap<OwnedServerName, BTreeSet<OwnedUserId>> {
    let mut by_server: BTreeMap<OwnedServerName, BTreeSet<OwnedUserId>> = BTreeMap::new();
    for user_id in recipients {
        by_server
            .entry(user_id.server_name().to_owned())
            .or_default()
            .insert(user_id);
    }
    by_server
}

/// The user's sharing configuration, or `None` when they have not set one.
///
/// `None` means "shares presence with nobody"; it is not the same as an empty
/// configuration only in that callers can tell the two apart for logging.
pub async fn sharing_policy(user_id: &UserId) -> AppResult<Option<PresenceSharingEventContent>> {
    match data::user::get_global_data::<PresenceSharingEventContent>(
        user_id,
        &GlobalAccountDataEventType::from(PresenceSharingEventContent::TYPE).to_string(),
    )
    .await
    {
        Ok(policy) => Ok(policy),
        Err(data::DataError::SerdeJson(e)) => {
            // An unreadable configuration must never widen sharing, so treat it as absent
            // rather than falling back to anything more permissive.
            warn!(%user_id, error = %e, "ignoring unparseable presence sharing configuration");
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

/// Collects the rooms the user is in, with the hint and membership each contributes.
pub async fn shared_rooms(user_id: &UserId) -> AppResult<Vec<SharedRoom>> {
    let mut rooms = Vec::new();
    for room_id in data::user::joined_rooms(user_id).await? {
        let hint = crate::room::get_state_content::<RoomPresenceSharingEventContent>(
            &room_id,
            &StateEventType::from(RoomPresenceSharingEventContent::TYPE),
            "",
            None,
        )
        .await
        .map(|content| content.presence_sharing)
        // No state event means the room forbids presence sharing, per the proposal.
        .unwrap_or(PresenceSharingHint::Forbid);

        let members = crate::room::joined_users(&room_id, None).await?;
        rooms.push(SharedRoom {
            room_id,
            hint,
            members,
        });
    }
    Ok(rooms)
}

/// The current recipient user set of a local user.
pub async fn recipients_of(user_id: &UserId) -> AppResult<BTreeSet<OwnedUserId>> {
    let Some(policy) = sharing_policy(user_id).await? else {
        return Ok(BTreeSet::new());
    };
    let rooms = shared_rooms(user_id).await?;
    Ok(effective_recipients(
        user_id,
        &policy,
        &rooms,
        config::server_name(),
    ))
}

/// Whether one viewer is included by a local user's sharing policy.
///
/// `shares_room` and `allowed_suggested_room` are calculated separately so callers checking a
/// single viewer do not need to construct the sender's complete recipient set.
fn policy_allows_viewer(
    policy: &PresenceSharingEventContent,
    viewer_id: &UserId,
    shares_room: bool,
    allowed_suggested_room: bool,
    local_server: &ServerName,
) -> bool {
    if !shares_room
        || matches!(
            policy.users.get(viewer_id),
            Some(UserPresenceSharingState::Deny)
        )
    {
        return false;
    }

    // These rules are applied after server denies and therefore override them.
    if (policy.share_locally && viewer_id.server_name() == local_server)
        || matches!(
            policy.users.get(viewer_id),
            Some(UserPresenceSharingState::Allow)
        )
    {
        return true;
    }

    !matches!(
        policy.servers.get(viewer_id.server_name()),
        Some(ServerPresenceSharingState::Deny)
    ) && allowed_suggested_room
}

/// Variant of [`may_see`] that reuses the viewer's joined-room set.
///
/// This is useful when authorizing many senders for one sync response. It performs a
/// viewer-specific policy check and never materializes the members of every room the sender has
/// joined.
pub async fn may_see_with_viewer_rooms(
    sender_id: &UserId,
    viewer_id: &UserId,
    viewer_rooms: &BTreeSet<OwnedRoomId>,
) -> AppResult<bool> {
    if sender_id == viewer_id {
        return Ok(true);
    }
    if sender_id.server_name() != config::server_name() {
        return match super::recipients::remote_set(sender_id).await? {
            Some((_, recipients)) => Ok(recipients.contains(viewer_id)),
            None => crate::room::state::user_can_see_user(viewer_id, sender_id).await,
        };
    }

    let Some(policy) = sharing_policy(sender_id).await? else {
        return Ok(false);
    };
    if matches!(
        policy.users.get(viewer_id),
        Some(UserPresenceSharingState::Deny)
    ) {
        return Ok(false);
    }

    let shared_rooms = data::user::joined_rooms(sender_id)
        .await?
        .into_iter()
        .filter(|room_id| viewer_rooms.contains(room_id))
        .collect::<Vec<_>>();
    if shared_rooms.is_empty() {
        return Ok(false);
    }

    // Local sharing and explicit user allows override server denies, and neither needs room
    // hints once a shared room is known.
    let direct_allow = (policy.share_locally && viewer_id.server_name() == config::server_name())
        || matches!(
            policy.users.get(viewer_id),
            Some(UserPresenceSharingState::Allow)
        );
    let server_denied = matches!(
        policy.servers.get(viewer_id.server_name()),
        Some(ServerPresenceSharingState::Deny)
    );
    if direct_allow || server_denied {
        return Ok(direct_allow);
    }

    let mut allowed_suggested_room = false;
    for room_id in &shared_rooms {
        if !matches!(
            policy.rooms.get(room_id),
            Some(RoomPresenceSharingState::Allow)
        ) {
            continue;
        }
        let hint = crate::room::get_state_content::<RoomPresenceSharingEventContent>(
            room_id,
            &StateEventType::from(RoomPresenceSharingEventContent::TYPE),
            "",
            None,
        )
        .await
        .map(|content| content.presence_sharing)
        .unwrap_or(PresenceSharingHint::Forbid);
        if hint == PresenceSharingHint::Suggest {
            allowed_suggested_room = true;
            break;
        }
    }

    Ok(policy_allows_viewer(
        &policy,
        viewer_id,
        true,
        allowed_suggested_room,
        config::server_name(),
    ))
}

/// Whether `viewer_id`, a local user, may be shown `sender_id`'s presence.
///
/// For a local sender this is their recipient set. For a remote sender it is the set their
/// server told us about; a remote server that has said nothing about recipient sets is
/// running the legacy protocol, and the proposal's compatibility rule is to fall back to
/// the pre-existing "shares a room" behaviour for those. The fallback is deliberately not
/// extended to local users: a local client could otherwise opt out of selective presence
/// just by never writing its configuration.
pub async fn may_see(sender_id: &UserId, viewer_id: &UserId) -> AppResult<bool> {
    if sender_id == viewer_id {
        return Ok(true);
    }
    if sender_id.server_name() == config::server_name() {
        let viewer_rooms = data::user::joined_rooms(viewer_id)
            .await?
            .into_iter()
            .collect();
        return may_see_with_viewer_rooms(sender_id, viewer_id, &viewer_rooms).await;
    }

    match super::recipients::remote_set(sender_id).await? {
        Some((_, recipients)) => Ok(recipients.contains(viewer_id)),
        None => crate::room::state::user_can_see_user(viewer_id, sender_id).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{SharedRoom, by_server, effective_recipients, policy_allows_viewer};
    use crate::core::events::presence::sharing::{
        PresenceSharingEventContent, RoomPresenceSharingState, ServerPresenceSharingState,
        UserPresenceSharingState,
    };
    use crate::core::events::room::presence_sharing::PresenceSharingHint;
    use crate::core::identifiers::*;
    use crate::core::{owned_room_id, owned_server_name, owned_user_id, server_name, user_id};

    fn room(hint: PresenceSharingHint, members: &[&str]) -> SharedRoom {
        SharedRoom {
            room_id: owned_room_id!("!room:example.org"),
            hint,
            members: members
                .iter()
                .map(|member| UserId::parse(*member).unwrap().to_owned())
                .collect(),
        }
    }

    fn policy() -> PresenceSharingEventContent {
        PresenceSharingEventContent::default()
    }

    #[test]
    fn a_user_with_no_rules_shares_with_nobody() {
        let rooms = [room(
            PresenceSharingHint::Suggest,
            &["@alice:example.org", "@bob:example.org"],
        )];

        assert!(
            effective_recipients(
                user_id!("@alice:example.org"),
                &policy(),
                &rooms,
                server_name!("example.org"),
            )
            .is_empty()
        );
    }

    #[test]
    fn a_room_contributes_members_only_when_allowed_and_hinted() {
        let mut policy = policy();
        policy.rooms.insert(
            owned_room_id!("!room:example.org"),
            RoomPresenceSharingState::Allow,
        );

        let suggested = [room(
            PresenceSharingHint::Suggest,
            &["@alice:example.org", "@bob:example.org"],
        )];
        assert_eq!(
            effective_recipients(
                user_id!("@alice:example.org"),
                &policy,
                &suggested,
                server_name!("example.org"),
            ),
            [owned_user_id!("@bob:example.org")].into()
        );

        // The hint is how a room turns presence off for everyone in it, so allowing the
        // room in the user's own configuration is not enough on its own.
        let forbidden = [room(
            PresenceSharingHint::Forbid,
            &["@alice:example.org", "@bob:example.org"],
        )];
        assert!(
            effective_recipients(
                user_id!("@alice:example.org"),
                &policy,
                &forbidden,
                server_name!("example.org"),
            )
            .is_empty()
        );
    }

    #[test]
    fn share_locally_covers_local_users_only() {
        let mut policy = policy();
        policy.share_locally = true;

        let rooms = [room(
            PresenceSharingHint::Forbid,
            &[
                "@alice:example.org",
                "@bob:example.org",
                "@carol:remote.example.org",
            ],
        )];

        // Local users are included even though the room forbids sharing -- step 3 does not
        // go through rooms -- but remote users are not.
        assert_eq!(
            effective_recipients(
                user_id!("@alice:example.org"),
                &policy,
                &rooms,
                server_name!("example.org"),
            ),
            [owned_user_id!("@bob:example.org")].into()
        );
    }

    #[test]
    fn naming_a_user_overrides_their_denied_server() {
        let mut policy = policy();
        policy.rooms.insert(
            owned_room_id!("!room:example.org"),
            RoomPresenceSharingState::Allow,
        );
        policy.servers.insert(
            owned_server_name!("remote.example.org"),
            ServerPresenceSharingState::Deny,
        );
        policy.users.insert(
            owned_user_id!("@carol:remote.example.org"),
            UserPresenceSharingState::Allow,
        );

        let rooms = [room(
            PresenceSharingHint::Suggest,
            &[
                "@alice:example.org",
                "@dave:remote.example.org",
                "@carol:remote.example.org",
            ],
        )];

        // Everyone else on the denied server is dropped; the named user survives.
        assert_eq!(
            effective_recipients(
                user_id!("@alice:example.org"),
                &policy,
                &rooms,
                server_name!("example.org"),
            ),
            [owned_user_id!("@carol:remote.example.org")].into()
        );
    }

    #[test]
    fn denying_a_user_beats_every_other_rule() {
        let mut policy = policy();
        policy.share_locally = true;
        policy.rooms.insert(
            owned_room_id!("!room:example.org"),
            RoomPresenceSharingState::Allow,
        );
        policy.users.insert(
            owned_user_id!("@bob:example.org"),
            UserPresenceSharingState::Deny,
        );

        let rooms = [room(
            PresenceSharingHint::Suggest,
            &["@alice:example.org", "@bob:example.org"],
        )];

        assert!(
            effective_recipients(
                user_id!("@alice:example.org"),
                &policy,
                &rooms,
                server_name!("example.org"),
            )
            .is_empty()
        );
    }

    #[test]
    fn allowing_a_stranger_does_not_add_them() {
        let mut policy = policy();
        policy.users.insert(
            owned_user_id!("@eve:remote.example.org"),
            UserPresenceSharingState::Allow,
        );

        let rooms = [room(
            PresenceSharingHint::Suggest,
            &["@alice:example.org", "@bob:example.org"],
        )];

        // A stale entry for someone the sender no longer shares a room with must not keep
        // generating federation traffic.
        assert!(
            effective_recipients(
                user_id!("@alice:example.org"),
                &policy,
                &rooms,
                server_name!("example.org"),
            )
            .is_empty()
        );
    }

    #[test]
    fn the_sender_is_never_their_own_recipient() {
        let mut policy = policy();
        policy.share_locally = true;
        policy.users.insert(
            owned_user_id!("@alice:example.org"),
            UserPresenceSharingState::Allow,
        );

        let rooms = [room(PresenceSharingHint::Suggest, &["@alice:example.org"])];

        assert!(
            effective_recipients(
                user_id!("@alice:example.org"),
                &policy,
                &rooms,
                server_name!("example.org"),
            )
            .is_empty()
        );
    }

    #[test]
    fn viewer_check_preserves_policy_precedence() {
        let viewer = user_id!("@bob:example.org");
        let local_server = server_name!("example.org");
        let mut policy = policy();
        policy.servers.insert(
            owned_server_name!("example.org"),
            ServerPresenceSharingState::Deny,
        );

        assert!(!policy_allows_viewer(
            &policy,
            viewer,
            true,
            true,
            local_server,
        ));

        policy.share_locally = true;
        assert!(policy_allows_viewer(
            &policy,
            viewer,
            true,
            false,
            local_server,
        ));

        policy
            .users
            .insert(viewer.to_owned(), UserPresenceSharingState::Deny);
        assert!(!policy_allows_viewer(
            &policy,
            viewer,
            true,
            true,
            local_server,
        ));
    }

    #[test]
    fn viewer_check_requires_a_current_shared_room() {
        let mut policy = policy();
        policy.share_locally = true;
        policy.users.insert(
            owned_user_id!("@bob:example.org"),
            UserPresenceSharingState::Allow,
        );

        assert!(!policy_allows_viewer(
            &policy,
            user_id!("@bob:example.org"),
            false,
            true,
            server_name!("example.org"),
        ));
    }

    #[test]
    fn recipients_are_grouped_by_destination_server() {
        let grouped = by_server(
            [
                owned_user_id!("@bob:example.org"),
                owned_user_id!("@carol:remote.example.org"),
                owned_user_id!("@dave:remote.example.org"),
            ]
            .into(),
        );

        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[&owned_server_name!("example.org")].len(), 1);
        assert_eq!(grouped[&owned_server_name!("remote.example.org")].len(), 2);
    }
}
