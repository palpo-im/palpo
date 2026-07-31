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
) -> AppResult<Profiles> {
    let SyncInfo {
        sender_id,
        since_sn,
        req_body,
        ..
    } = sync_info;
    let config = &req_body.extensions.profiles;
    if !config.enabled.unwrap_or(false) {
        return Ok(Profiles::default());
    }

    let fields: FieldFilter = config.fields.as_ref().map(|fields| {
        fields
            .iter()
            .map(|field| field.as_str().to_owned())
            .collect()
    });

    let mut users: BTreeMap<OwnedUserId, Option<UserProfileUpdate>> = BTreeMap::new();

    // Rooms whose members need a full profile snapshot: those entering the response's room
    // subset for the first time in this connection. On an initial sync that is all of them.
    for room_id in snapshot_rooms(config_rooms(config, todo_rooms, listed_rooms), todo_rooms) {
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
            if let Some(profile) = snapshot(&user_id, &fields).await? {
                users.insert(user_id, Some(profile));
            }
        }
    }

    // Incremental changes go to every user the syncing user shares a room with, not just
    // those in the room subset: the subset can shrink and grow again, and the client must
    // not end up applying updates on top of a profile it never refreshed.
    if since_sn > 0 {
        let mut shared: BTreeSet<OwnedUserId> = BTreeSet::new();
        for room_id in all_joined_rooms {
            shared.extend(crate::room::joined_users(room_id, None).await?);
        }
        // The syncing user's own updates must always be delivered, so their other devices
        // see changes they made elsewhere.
        shared.insert(sender_id.to_owned());

        let shared: Vec<OwnedUserId> = shared.into_iter().collect();
        for change in data::user::profile_changes_since(Some(&shared), since_sn, until_sn).await? {
            if !wanted(&fields, &change.field) {
                continue;
            }
            let entry = users
                .entry(change.user_id)
                .or_insert_with(|| Some(UserProfileUpdate::new()))
                .get_or_insert_with(UserProfileUpdate::new);
            apply(entry, change.field, change.value);
        }
    }

    // A user who is no longer in any room with the syncing user gets a `null`, telling the
    // client it can stop tracking them.
    for user_id in departed(all_joined_rooms, since_sn, until_sn).await? {
        if user_id == sender_id {
            continue;
        }
        users.insert(user_id, None);
    }

    users.retain(|_, update| update.as_ref().is_none_or(|update| !update.is_empty()));

    Ok(Profiles { users })
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

/// The current profile of a user as a full update, or `None` if they have no profile.
async fn snapshot(user_id: &UserId, fields: &FieldFilter) -> AppResult<Option<UserProfileUpdate>> {
    let Some(profile) = data::user::get_profile(user_id, None).await? else {
        return Ok(None);
    };

    let mut update = UserProfileUpdate::new();
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
    if let Some(custom) = profile.fields.as_object() {
        for (field, value) in custom {
            if wanted(fields, field) {
                update.set(field.clone(), value.clone());
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
fn config_rooms(
    config: &crate::core::client::sync_events::v5::ProfilesConfig,
    todo_rooms: &TodoRooms,
    listed_rooms: &BTreeMap<String, BTreeSet<OwnedRoomId>>,
) -> BTreeSet<OwnedRoomId> {
    if config.lists.is_none() && config.rooms.is_none() {
        return todo_rooms.keys().cloned().collect();
    }

    let mut rooms = BTreeSet::new();
    for list_id in config.lists.iter().flatten() {
        if let Some(list_rooms) = listed_rooms.get(list_id) {
            rooms.extend(list_rooms.iter().cloned());
        }
    }
    for room in config.rooms.iter().flatten() {
        match room {
            ExtensionRoomConfig::AllSubscribed => rooms.extend(todo_rooms.keys().cloned()),
            ExtensionRoomConfig::Room(room_id) => {
                rooms.insert(room_id.clone());
            }
        }
    }
    rooms
}

/// Of the selected rooms, those entering the response's room subset for the first time.
///
/// `room_since_sn == 0` is how sliding sync marks a room the connection has not sent
/// anything for yet, which is exactly MSC4262's "enters this subset for the first time".
fn snapshot_rooms(
    selected: BTreeSet<OwnedRoomId>,
    todo_rooms: &TodoRooms,
) -> impl Iterator<Item = OwnedRoomId> + use<> {
    let initial: BTreeSet<OwnedRoomId> = todo_rooms
        .iter()
        .filter(|(_, todo)| todo.room_since_sn == 0)
        .map(|(room_id, _)| room_id.clone())
        .collect();

    selected
        .into_iter()
        .filter(move |room_id| initial.contains(room_id))
}

/// Users who have stopped sharing a room with the syncing user.
///
/// Derived from membership rows that now say `leave` or `ban`, rather than from a
/// reconstruction of who was joined at `since_sn`: membership updates replace the previous
/// row, so the old joined state is not recoverable from the current table.
///
/// A user is only reported once they are gone from *every* room the syncing user is in --
/// leaving one shared room while remaining in another is not a departure.
async fn departed(
    all_joined_rooms: &[&RoomId],
    since_sn: Seqnum,
    until_sn: Seqnum,
) -> AppResult<BTreeSet<OwnedUserId>> {
    if since_sn == 0 {
        return Ok(BTreeSet::new());
    }

    let mut left = BTreeSet::new();
    let mut still_joined = BTreeSet::new();
    for room_id in all_joined_rooms {
        left.extend(data::room::departed_users_since(room_id, since_sn, until_sn).await?);
        still_joined.extend(crate::room::joined_users(room_id, None).await?);
    }

    Ok(left.difference(&still_joined).cloned().collect())
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
}
