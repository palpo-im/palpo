//! Revised presence in simplified sliding sync ([MSC4559]).
//!
//! [MSC4559]: https://github.com/matrix-org/matrix-spec-proposals/pull/4559

use std::collections::{BTreeMap, BTreeSet};

use crate::core::Seqnum;
use crate::core::client::sync_events::v5::{
    ExtensionRoomConfig, Presence, PresenceConfig, PresenceUpdate, RoomSubscription, SyncInfo,
};
use crate::core::identifiers::*;
use crate::core::presence::revised::{self, Status};
use crate::{AppResult, data};

/// Limit one response so a large room cannot make a sync response unbounded.
const PRESENCE_BATCH_SIZE: usize = 500;

pub(super) async fn collect(
    sync_info: SyncInfo<'_>,
    listed_rooms: &BTreeMap<String, BTreeSet<OwnedRoomId>>,
    until_sn: Seqnum,
) -> AppResult<(
    Presence,
    BTreeMap<OwnedUserId, Seqnum>,
    BTreeSet<OwnedUserId>,
    BTreeSet<OwnedUserId>,
)> {
    let SyncInfo {
        sender_id,
        device_id,
        since_sn,
        req_body,
    } = sync_info;
    let config = &req_body.extensions.presence;
    if !config.enabled.unwrap_or(false) || !crate::config::get().presence.allow_local {
        return Ok((
            Presence::default(),
            BTreeMap::new(),
            BTreeSet::new(),
            BTreeSet::new(),
        ));
    }

    let mut room_subset: BTreeSet<OwnedRoomId> = listed_rooms
        .values()
        .flat_map(|rooms| rooms.iter().cloned())
        .collect();
    room_subset.extend(req_body.room_subscriptions.keys().cloned());
    let rooms = config_rooms(
        config,
        &room_subset,
        listed_rooms,
        &req_body.room_subscriptions,
    );

    let mut visible = BTreeSet::new();
    for room_id in rooms {
        // An explicit subscription is accepted on room existence alone. Never use it to expose
        // the member list or presence of a room the requester has not joined.
        if crate::room::user::is_joined(sender_id, &room_id).await? {
            visible.extend(crate::room::joined_users(&room_id, None).await?);
        }
    }

    // Honour selective-presence policy as well as MSC4559's room visibility. Revalidate on
    // every response so a queued update cannot escape after authorization changes.
    let mut authorized = BTreeSet::new();
    for user_id in visible {
        if user_id == sender_id
            || crate::user::presence::sharing::may_see(&user_id, sender_id).await?
        {
            authorized.insert(user_id);
        }
    }

    let tracked =
        crate::sync_v5::tracked_presence_users(sender_id, device_id, &req_body.conn_id).await;
    let initial: BTreeSet<OwnedUserId> = authorized.difference(&tracked).cloned().collect();

    // Changed users take priority over the initial backlog. Both paths load the latest complete
    // state at response time, so a user is present at most once and never receives a stale state.
    let changed: BTreeMap<OwnedUserId, Seqnum> = data::user::presences_between(since_sn, until_sn)
        .await?
        .into_iter()
        .filter(|(user_id, _)| authorized.contains(user_id))
        .map(|(user_id, (sn, _))| (user_id, sn))
        .collect();
    let queued = crate::sync_v5::queue_presence_updates(
        sender_id,
        device_id,
        &req_body.conn_id,
        &authorized,
        changed,
    )
    .await;
    let queued_users = queued.keys().cloned().collect::<BTreeSet<_>>();
    let mut candidates = queued_users.iter().cloned().collect::<Vec<_>>();
    candidates.extend(initial.difference(&queued_users).cloned());
    candidates.truncate(PRESENCE_BATCH_SIZE);

    let mut user_ids = BTreeMap::new();
    let mut sent = BTreeMap::new();
    let mut initial_sent = BTreeSet::new();
    for user_id in candidates {
        if initial.contains(&user_id) {
            initial_sent.insert(user_id.clone());
        }
        sent.insert(
            user_id.clone(),
            queued.get(&user_id).copied().unwrap_or_default(),
        );
        user_ids.insert(user_id.clone(), current_update(&user_id).await?);
    }

    Ok((Presence { user_ids }, sent, initial_sent, authorized))
}

async fn current_update(user_id: &UserId) -> AppResult<PresenceUpdate> {
    let Some(event) = data::user::maybe_last_presence(user_id).await? else {
        return Ok(PresenceUpdate {
            presence: crate::core::presence::PresenceState::Offline,
            status: Status::default(),
        });
    };
    let mut content = serde_json::to_value(event.content)?;
    crate::user::presence::revised::project(user_id, &mut content, true).await?;
    let presence = serde_json::from_value(
        content
            .get("presence")
            .cloned()
            .unwrap_or_else(|| "offline".into()),
    )?;
    let status = content
        .get(revised::STATUS)
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    Ok(PresenceUpdate { presence, status })
}

/// Resolve common per-room extension selectors to their current room set.
pub(super) fn config_rooms(
    config: &PresenceConfig,
    room_subset: &BTreeSet<OwnedRoomId>,
    listed_rooms: &BTreeMap<String, BTreeSet<OwnedRoomId>>,
    subscriptions: &BTreeMap<OwnedRoomId, RoomSubscription>,
) -> BTreeSet<OwnedRoomId> {
    if config.lists.is_none() && config.rooms.is_none() {
        return room_subset.clone();
    }

    let mut rooms = BTreeSet::new();
    match &config.lists {
        Some(list_ids) => {
            for list_id in list_ids {
                if let Some(list_rooms) = listed_rooms.get(list_id) {
                    rooms.extend(list_rooms.iter().cloned());
                }
            }
        }
        None => rooms.extend(listed_rooms.values().flatten().cloned()),
    }
    match &config.rooms {
        Some(room_selectors) => {
            for room in room_selectors {
                match room {
                    ExtensionRoomConfig::AllSubscribed => {
                        rooms.extend(subscriptions.keys().cloned())
                    }
                    ExtensionRoomConfig::Room(room_id) => {
                        rooms.insert(room_id.clone());
                    }
                }
            }
        }
        None => rooms.extend(subscriptions.keys().cloned()),
    }
    rooms
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::config_rooms;
    use crate::core::client::sync_events::v5::{
        ExtensionRoomConfig, PresenceConfig, RoomSubscription,
    };
    use crate::core::owned_room_id;

    #[test]
    fn selectors_do_not_pull_unselected_lists_through_subscription_wildcard() {
        let listed = BTreeMap::from([
            (
                "selected".to_owned(),
                BTreeSet::from([owned_room_id!("!selected:example.org")]),
            ),
            (
                "other".to_owned(),
                BTreeSet::from([owned_room_id!("!other:example.org")]),
            ),
        ]);
        let subscribed = BTreeMap::from([(
            owned_room_id!("!subscribed:example.org"),
            RoomSubscription::default(),
        )]);
        let subset = listed.values().flatten().cloned().collect();
        let config = PresenceConfig {
            enabled: Some(true),
            lists: Some(vec!["selected".to_owned()]),
            rooms: Some(vec![ExtensionRoomConfig::AllSubscribed]),
        };

        assert_eq!(
            config_rooms(&config, &subset, &listed, &subscribed),
            BTreeSet::from([
                owned_room_id!("!selected:example.org"),
                owned_room_id!("!subscribed:example.org")
            ])
        );
    }

    #[test]
    fn absent_selectors_cover_the_whole_response_subset() {
        let room = owned_room_id!("!room:example.org");
        assert_eq!(
            config_rooms(
                &PresenceConfig {
                    enabled: Some(true),
                    ..Default::default()
                },
                &BTreeSet::from([room.clone()]),
                &BTreeMap::new(),
                &BTreeMap::new()
            ),
            BTreeSet::from([room])
        );
    }

    #[test]
    fn omitted_room_selector_still_covers_all_subscriptions() {
        let listed_room = owned_room_id!("!listed:example.org");
        let subscribed_room = owned_room_id!("!subscribed:example.org");
        let listed =
            BTreeMap::from([("selected".to_owned(), BTreeSet::from([listed_room.clone()]))]);
        let subscriptions =
            BTreeMap::from([(subscribed_room.clone(), RoomSubscription::default())]);

        assert_eq!(
            config_rooms(
                &PresenceConfig {
                    enabled: Some(true),
                    lists: Some(vec!["selected".to_owned()]),
                    rooms: None,
                },
                &BTreeSet::from([listed_room.clone(), subscribed_room.clone()]),
                &listed,
                &subscriptions,
            ),
            BTreeSet::from([listed_room, subscribed_room])
        );
    }
}
