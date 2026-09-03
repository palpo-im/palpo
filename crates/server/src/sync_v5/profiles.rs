//! Profile updates in simplified sliding sync ([MSC4262]).
//!
//! A client that renders member lists needs to learn about profile changes without polling
//! `/profile` per user. This extension delivers them: the first sync of a connection --
//! and the first sync in which a room enters the response's room subset -- carries a
//! snapshot of the profiles in those rooms, and every later sync carries only the fields
//! that changed since the client's position.
//!
//! Change detection reads the `user_profile_changes` stream rather than diffing profiles,
//! which is what makes the extension restart-safe and correct under concurrent writes: the
//! client's sync position is a position on that stream, so nothing depends on server
//! process state.
//!
//! [MSC4262]: https://github.com/matrix-org/matrix-spec-proposals/pull/4262

use std::collections::{BTreeMap, BTreeSet};

use crate::core::Seqnum;
use crate::core::client::sync_events::v5::{ExtensionRoomConfig, Profiles, SyncInfo, TodoRooms};
use crate::core::identifiers::*;
use crate::core::profile::UserProfileUpdate;
use crate::core::serde::JsonValue;
use crate::exts::IsRemoteOrLocal;
use crate::{AppResult, data};

/// Profile fields the client asked for, or `None` when it wants all of them.
type FieldFilter = Option<BTreeSet<String>>;

/// Builds the `org.matrix.msc4262.profiles` extension for one sync response.
pub(super) async fn collect(
    sync_info: SyncInfo<'_>,
    all_joined_rooms: &[&RoomId],
    todo_rooms: &TodoRooms,
    listed_rooms: &BTreeMap<String, BTreeSet<OwnedRoomId>>,
    until_sn: Seqnum,
) -> AppResult<(Profiles, BTreeSet<OwnedRoomId>)> {
    let SyncInfo {
        sender_id,
        device_id,
        since_sn,
        req_body,
    } = sync_info;
    let config = &req_body.extensions.profiles;
    if !config.enabled.unwrap_or(false) {
        return Ok((Profiles::default(), BTreeSet::new()));
    }

    let fields: FieldFilter = config.fields.as_ref().map(|fields| {
        fields
            .iter()
            .map(|field| field.as_str().to_owned())
            .collect()
    });

    let mut users: BTreeMap<OwnedUserId, Option<UserProfileUpdate>> = BTreeMap::new();

    // Rooms whose members need a full profile snapshot: those this connection has not been
    // sent profiles for yet. Tracked per extension rather than reusing whether the room
    // itself was delivered, because a client can enable the extension -- or widen a
    // selector -- long after the room first appeared, and would otherwise get incremental
    // changes with no base to apply them to.
    let room_subset = todo_rooms.keys().cloned().collect();
    let selected = config_rooms(
        config,
        &room_subset,
        listed_rooms,
        &req_body.room_subscriptions,
    );
    let fresh = crate::sync_v5::unsnapshotted_profile_rooms(
        sender_id,
        device_id,
        &req_body.conn_id,
        selected,
    )
    .await;
    let mut snapshotted_rooms = BTreeSet::new();
    for room_id in fresh {
        // A room subscription is accepted on existence alone, so membership has to be
        // checked here: without it, anyone who knows a private room's ID could subscribe
        // to it and read back its member roster and profiles.
        if !crate::room::user::is_joined(sender_id, &room_id).await? {
            continue;
        }
        for user_id in crate::room::joined_users(&room_id, None).await? {
            if users.contains_key(&user_id) {
                continue;
            }
            if let Some(profile) = snapshot(&user_id, &room_id, &fields).await? {
                users.insert(user_id, Some(profile));
            }
        }
        snapshotted_rooms.insert(room_id);
    }

    // A previously departed user may be shared again without changing their global
    // profile. Send a new base for joins in the window; when the syncing user joined the
    // room, every current member is newly shared from this client's point of view.
    if since_sn > 0 {
        for room_id in all_joined_rooms {
            let joined = data::room::joined_users_since(room_id, since_sn, until_sn).await?;
            let candidates = if joined.iter().any(|user_id| user_id == sender_id) {
                crate::room::joined_users(room_id, None).await?
            } else {
                joined
            };
            for user_id in candidates {
                if users.contains_key(&user_id) {
                    continue;
                }
                if let Some(profile) = snapshot(&user_id, room_id, &fields).await? {
                    users.insert(user_id, Some(profile));
                }
            }
        }
    }
    let mut shared = BTreeSet::new();
    if since_sn > 0 {
        for room_id in all_joined_rooms {
            shared.extend(crate::room::joined_users(room_id, None).await?);
        }
    }

    // Incremental changes go to every user the syncing user shares a room with, not just
    // those in the room subset: the subset can shrink and grow again, and the client must
    // not end up applying updates on top of a profile it never refreshed.
    if since_sn > 0 {
        // The syncing user's own updates must always be delivered, so their other devices
        // see changes they made elsewhere.
        let mut users_to_update: Vec<OwnedUserId> = shared
            .iter()
            .filter(|user| user.is_local())
            .cloned()
            .collect();
        if !shared.contains(sender_id) {
            users_to_update.push(sender_id.to_owned());
        }
        for change in
            data::user::profile_changes_since(Some(&users_to_update), since_sn, until_sn).await?
        {
            // Match snapshot(): remote global profiles remain unknown until MSC4259.
            if !change.user_id.is_local() || !wanted(&fields, &change.field) {
                continue;
            }
            let entry = users
                .entry(change.user_id)
                .or_insert_with(|| Some(UserProfileUpdate::new()))
                .get_or_insert_with(UserProfileUpdate::new);
            apply(entry, change.field, change.value);
        }
    }

    // A user who is no longer shared gets a `null`, but only if this connection has
    // acknowledged receiving that user's profile. Inferring historical sharing from the
    // current membership table is incomplete (rows are overwritten) and can leak users
    // who joined a private room only after the syncing user left it.
    if since_sn > 0 {
        let tracked =
            crate::sync_v5::tracked_profile_users(sender_id, device_id, &req_body.conn_id).await;
        for user_id in tracked.difference(&shared) {
            if user_id != sender_id {
                users.insert(user_id.clone(), None);
            }
        }
    }

    users.retain(|_, update| update.as_ref().is_none_or(|update| !update.is_empty()));

    Ok((Profiles { users }, snapshotted_rooms))
}

/// Whether the client asked for this field.
fn wanted(fields: &FieldFilter, field: &str) -> bool {
    fields.as_ref().is_none_or(|fields| fields.contains(field))
}

/// Folds one change into a user's pending update.
///
/// A later change wins outright: setting a field after clearing it must not leave the field
/// in `removed`, and clearing it after setting it must not leave a stale value in `updated`.
fn apply(update: &mut UserProfileUpdate, field: String, value: Option<JsonValue>) {
    match value {
        Some(value) => {
            update.removed.retain(|removed| *removed != field);
            update.set(field, value);
        }
        None => {
            update.updated.remove(&field);
            if !update.removed.contains(&field) {
                update.remove(field);
            }
        }
    }
}

/// The current global profile of a user as a full update, or `None` if nothing is known.
///
/// In particular, an `m.room.member` profile is deliberately not used as a fallback for a
/// remote user. Membership profiles are scoped to one room and may contain a room-specific
/// pseudonym or avatar; treating either as the user's global profile would disclose it to
/// clients that only share a different room. Remote profiles can be included once they are
/// learned through the global-profile federation mechanism (MSC4259).
async fn snapshot(
    user_id: &UserId,
    _room_id: &RoomId,
    fields: &FieldFilter,
) -> AppResult<Option<UserProfileUpdate>> {
    // `create_user` installs a localpart display-name placeholder for remote members so
    // membership bookkeeping has a user row. It is not a verified global profile and
    // must not be exposed through MSC4262. Remote profiles remain omitted until Palpo
    // learns them through the global-profile federation mechanism (MSC4259).
    if user_id.is_remote() {
        return Ok(None);
    }

    let mut update = UserProfileUpdate::new();

    if let Some(profile) = data::user::get_profile(user_id, None).await? {
        if let Some(display_name) = profile.display_name
            && wanted(fields, "displayname")
        {
            update.set("displayname".to_owned(), display_name.into());
        }
        if let Some(avatar_url) = profile.avatar_url
            && wanted(fields, "avatar_url")
        {
            update.set("avatar_url".to_owned(), avatar_url.as_str().into());
        }
        if let Some(blurhash) = profile.blurhash
            && wanted(fields, "xyz.amorgan.blurhash")
        {
            update.set("xyz.amorgan.blurhash".to_owned(), blurhash.into());
        }
        if let Some(custom) = profile.fields.as_object() {
            for (field, value) in custom {
                if wanted(fields, field) {
                    update.set(field.clone(), value.clone());
                }
            }
        }
    }

    Ok((!update.is_empty()).then_some(update))
}

/// The rooms the extension applies to, honouring the `lists` and `rooms` selectors.
///
/// With no selector the extension covers the whole room subset of the response, which is
/// what a client that just sets `enabled` expects. List selectors resolve against what the
/// lists produced in *this* response, not the cache from the previous one -- the cache is
/// empty on an initial sync and stale for exactly the rooms that have just entered a list,
/// which are the ones that need a snapshot.
pub(super) fn config_rooms(
    config: &crate::core::client::sync_events::v5::ProfilesConfig,
    room_subset: &BTreeSet<OwnedRoomId>,
    listed_rooms: &BTreeMap<String, BTreeSet<OwnedRoomId>>,
    subscriptions: &BTreeMap<OwnedRoomId, crate::core::client::sync_events::v5::RoomSubscription>,
) -> BTreeSet<OwnedRoomId> {
    if config.lists.is_none() && config.rooms.is_none() {
        return room_subset.clone();
    }

    let mut rooms = BTreeSet::new();
    for list_id in config.lists.iter().flatten() {
        if let Some(list_rooms) = listed_rooms.get(list_id) {
            rooms.extend(list_rooms.iter().cloned());
        }
    }
    for room in config.rooms.iter().flatten() {
        match room {
            // `*` is defined as the global room subscriptions, not the whole room subset
            // of the response: a list the client did not select for this extension must
            // not be pulled in by the wildcard.
            ExtensionRoomConfig::AllSubscribed => rooms.extend(subscriptions.keys().cloned()),
            ExtensionRoomConfig::Room(room_id) => {
                rooms.insert(room_id.clone());
            }
        }
    }
    rooms
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FieldFilter, apply, wanted};
    use crate::core::profile::UserProfileUpdate;

    fn filter(fields: &[&str]) -> FieldFilter {
        Some(fields.iter().map(|f| (*f).to_owned()).collect())
    }

    #[test]
    fn an_absent_field_filter_means_every_field() {
        assert!(wanted(&None, "displayname"));
        assert!(wanted(&None, "org.example.anything"));

        let only_avatar = filter(&["avatar_url"]);
        assert!(wanted(&only_avatar, "avatar_url"));
        assert!(!wanted(&only_avatar, "displayname"));
    }

    #[test]
    fn the_last_change_to_a_field_wins() {
        let mut update = UserProfileUpdate::new();

        // Set, then clear: the field is removed and carries no stale value.
        apply(&mut update, "displayname".to_owned(), Some(json!("Alice")));
        apply(&mut update, "displayname".to_owned(), None);
        assert_eq!(update.removed, vec!["displayname".to_owned()]);
        assert!(update.updated.is_empty());

        // Clear, then set again: the field is updated and no longer listed as removed.
        apply(&mut update, "displayname".to_owned(), Some(json!("Alicia")));
        assert!(update.removed.is_empty());
        assert_eq!(update.get("displayname"), Some(&json!("Alicia")));
    }

    #[test]
    fn a_stored_null_is_a_value_not_a_removal() {
        let mut update = UserProfileUpdate::new();

        apply(
            &mut update,
            "org.example.field".to_owned(),
            Some(json!(null)),
        );

        assert!(update.removed.is_empty());
        assert_eq!(update.get("org.example.field"), Some(&json!(null)));
    }

    #[test]
    fn repeated_removals_are_listed_once() {
        let mut update = UserProfileUpdate::new();

        apply(&mut update, "avatar_url".to_owned(), None);
        apply(&mut update, "avatar_url".to_owned(), None);

        assert_eq!(update.removed, vec!["avatar_url".to_owned()]);
    }
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_concurrent_profile_creation_preserves_the_winner() {
        use crate::data;
        crate::test_database::init();
        let profile = data::user::NewDbProfile {
            user_id: "@profile:example.org".try_into().unwrap(),
            room_id: None,
            display_name: Some("first".into()),
            avatar_url: None,
            blurhash: None,
        };
        let (first, second) = tokio::join!(
            data::user::create_profile(&profile),
            data::user::create_profile(&profile)
        );
        first.unwrap();
        second.unwrap();
        data::user::set_global_display_name(&profile.user_id, Some("edited"))
            .await
            .unwrap();
        data::user::create_profile(&profile).await.unwrap();
        let stored = data::user::get_profile(&profile.user_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.display_name.as_deref(), Some("edited"));
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        use crate::data::schema::user_profiles;
        assert_eq!(
            user_profiles::table
                .filter(user_profiles::user_id.eq(&profile.user_id))
                .count()
                .get_result::<i64>(&mut data::connect().await.unwrap())
                .await
                .unwrap(),
            1
        );
    }
}
