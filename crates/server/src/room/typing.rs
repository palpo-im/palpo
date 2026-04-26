use std::sync::LazyLock;

use diesel::prelude::*;
use tokio::sync::broadcast;

use crate::core::UnixMillis;
use crate::core::events::SyncEphemeralRoomEvent;
use crate::core::events::typing::{TypingContent, TypingEventContent};
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::data::connect;
use crate::data::schema::*;
use crate::{AppResult, IsRemoteOrLocal, data, sending};

/// Local broadcast channel for same-instance fast notification.
/// Cross-instance coordination happens via DB polling in the watcher.
static TYPING_UPDATE_SENDER: LazyLock<broadcast::Sender<OwnedRoomId>> =
    LazyLock::new(|| broadcast::channel(100).0);

/// Sets a user as typing until the timeout timestamp is reached or remove_typing is called.
/// State is persisted to the database for cross-instance visibility.
pub async fn add_typing(
    user_id: &UserId,
    room_id: &RoomId,
    timeout: u64,
    broadcast: bool,
) -> AppResult<()> {
    diesel::insert_into(room_typings::table)
        .values((
            room_typings::room_id.eq(room_id),
            room_typings::user_id.eq(user_id),
            room_typings::timeout_at.eq(timeout as i64),
            room_typings::occur_sn.eq(data::next_sn_sql()),
        ))
        .on_conflict((room_typings::room_id, room_typings::user_id))
        .do_update()
        .set((
            room_typings::timeout_at.eq(timeout as i64),
            room_typings::occur_sn.eq(data::next_sn_sql()),
        ))
        .execute(&mut connect()?)?;

    // Notify same-instance watchers immediately
    let _ = TYPING_UPDATE_SENDER.send(room_id.to_owned());

    if broadcast && user_id.is_local() {
        federation_send(room_id, user_id, true).await.ok();
    }
    Ok(())
}

/// Removes a user from typing before the timeout is reached.
/// Instead of deleting the row, we set timeout_at=0 and bump occur_sn
/// so that sync picks up the "typing stopped" change before cleanup.
pub async fn remove_typing(user_id: &UserId, room_id: &RoomId, broadcast: bool) -> AppResult<()> {
    diesel::update(
        room_typings::table
            .filter(room_typings::room_id.eq(room_id))
            .filter(room_typings::user_id.eq(user_id)),
    )
    .set((
        room_typings::timeout_at.eq(0i64),
        room_typings::occur_sn.eq(data::next_sn_sql()),
    ))
    .execute(&mut connect()?)?;

    // Notify same-instance watchers immediately
    let _ = TYPING_UPDATE_SENDER.send(room_id.to_owned());

    if broadcast && user_id.is_local() {
        federation_send(room_id, user_id, false).await.ok();
    }
    Ok(())
}

/// Wait for a typing update on this instance (same-instance optimization).
/// Cross-instance updates are detected via DB polling in the watcher.
pub async fn wait_for_update(room_id: &RoomId) -> AppResult<()> {
    let mut receiver = TYPING_UPDATE_SENDER.subscribe();
    while let Ok(next) = receiver.recv().await {
        if next == room_id {
            break;
        }
    }

    Ok(())
}

/// Removes expired typing entries from the database.
fn maintain_typings(room_id: &RoomId) -> AppResult<()> {
    let current_timestamp = UnixMillis::now().get() as i64;
    diesel::delete(
        room_typings::table
            .filter(room_typings::room_id.eq(room_id))
            .filter(room_typings::timeout_at.lt(current_timestamp)),
    )
    .execute(&mut connect()?)?;
    Ok(())
}

/// Returns the sequence number of the last typing update in this room.
/// Reads from the database so it works across all instances.
/// We query max occur_sn BEFORE cleanup so that "stop typing" events
/// (which set timeout_at=0) are still visible to sync.
pub async fn last_typing_update(room_id: &RoomId) -> AppResult<i64> {
    let sn = room_typings::table
        .filter(room_typings::room_id.eq(room_id))
        .select(diesel::dsl::max(room_typings::occur_sn))
        .first::<Option<i64>>(&mut connect()?)
        .unwrap_or(None)
        .unwrap_or_default();
    Ok(sn)
}

/// Returns a new typing EDU with currently typing users from the database.
pub async fn all_typings(
    room_id: &RoomId,
) -> AppResult<SyncEphemeralRoomEvent<TypingEventContent>> {
    maintain_typings(room_id)?;
    let user_ids = room_typings::table
        .filter(room_typings::room_id.eq(room_id))
        .select(room_typings::user_id)
        .load::<OwnedUserId>(&mut connect()?)?;

    Ok(SyncEphemeralRoomEvent {
        content: TypingEventContent { user_ids },
    })
}

async fn federation_send(room_id: &RoomId, user_id: &UserId, typing: bool) -> AppResult<()> {
    debug_assert!(
        user_id.is_local(),
        "tried to broadcast typing status of remote user",
    );

    if !crate::config::get().typing.allow_outgoing {
        return Ok(());
    }

    let content = TypingContent::new(room_id.to_owned(), user_id.to_owned(), typing);
    let edu = Edu::Typing(content);
    sending::send_edu_room(room_id, &edu)?;

    Ok(())
}
