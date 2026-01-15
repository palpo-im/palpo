use std::collections::BTreeMap;
use std::sync::LazyLock;

use tokio::sync::{RwLock, broadcast};

use crate::core::UnixMillis;
use crate::core::events::SyncEphemeralRoomEvent;
use crate::core::events::typing::{TypingContent, TypingEventContent};
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::{AppResult, IsRemoteOrLocal, data, sending};

pub static TYPING: LazyLock<RwLock<BTreeMap<OwnedRoomId, BTreeMap<OwnedUserId, u64>>>> =
    LazyLock::new(Default::default); // u64 is unix timestamp of timeout
pub static LAST_TYPING_UPDATE: LazyLock<RwLock<BTreeMap<OwnedRoomId, i64>>> =
    LazyLock::new(Default::default); // timestamp of the last change to typing users
pub static TYPING_UPDATE_SENDER: LazyLock<broadcast::Sender<OwnedRoomId>> =
    LazyLock::new(|| broadcast::channel(100).0);

/// Sets a user as typing until the timeout timestamp is reached or roomremove_typing is
/// called.
pub async fn add_typing(
    user_id: &UserId,
    room_id: &RoomId,
    timeout: u64,
    broadcast: bool,
) -> AppResult<()> {
    TYPING
        .write()
        .await
        .entry(room_id.to_owned())
        .or_default()
        .insert(user_id.to_owned(), timeout);
    let event_sn = data::next_sn()?;
    LAST_TYPING_UPDATE
        .write()
        .await
        .insert(room_id.to_owned(), event_sn);

    // let current_frame_id = if let Some(s) = crate::room::get_frame_id(room_id, None)? {
    //     s
    // } else {
    //     error!("Room {} has no state", room_id);
    //     return Err(AppError::public("Room has no state"));
    // };
    // // Save the state after this sync so we can send the correct state diff next sync
    // let point_id = state::ensure_point(&room_id,
    // &OwnedEventId::from_str(&Ulid::new().to_string())?, event_sn as i64)?;
    // state::update_frame_id(point_id, current_frame_id)?;

    let _ = TYPING_UPDATE_SENDER.send(room_id.to_owned());

    if broadcast && user_id.is_local() {
        federation_send(room_id, user_id, true).await.ok();
    }
    Ok(())
}

/// Removes a user from typing before the timeout is reached.
pub async fn remove_typing(user_id: &UserId, room_id: &RoomId, broadcast: bool) -> AppResult<()> {
    TYPING
        .write()
        .await
        .entry(room_id.to_owned())
        .or_default()
        .remove(user_id);
    LAST_TYPING_UPDATE
        .write()
        .await
        .insert(room_id.to_owned(), data::next_sn()?);
    let _ = TYPING_UPDATE_SENDER.send(room_id.to_owned());

    if broadcast && user_id.is_local() {
        federation_send(room_id, user_id, false).await.ok();
    }
    Ok(())
}

pub async fn wait_for_update(room_id: &RoomId) -> AppResult<()> {
    let mut receiver = TYPING_UPDATE_SENDER.subscribe();
    while let Ok(next) = receiver.recv().await {
        if next == room_id {
            break;
        }
    }

    Ok(())
}

/// Makes sure that typing events with old timestamps get removed.
async fn maintain_typings(room_id: &RoomId) -> AppResult<()> {
    let current_timestamp = UnixMillis::now();
    let mut removable = Vec::new();
    {
        let typing = TYPING.read().await;
        let Some(room) = typing.get(room_id) else {
            return Ok(());
        };
        for (user_id, timeout) in room {
            if *timeout < current_timestamp.get() {
                removable.push(user_id.clone());
            }
        }
        drop(typing);
    }
    if !removable.is_empty() {
        let typing = &mut TYPING.write().await;
        let room = typing.entry(room_id.to_owned()).or_default();
        for user_id in &removable {
            room.remove(user_id);
        }
        LAST_TYPING_UPDATE
            .write()
            .await
            .insert(room_id.to_owned(), data::next_sn()?);
        let _ = TYPING_UPDATE_SENDER.send(room_id.to_owned());

        for user_id in &removable {
            if user_id.is_local() {
                federation_send(room_id, user_id, false).await.ok();
            }
        }
    }
    Ok(())
}

/// Returns the count of the last typing update in this room.
pub async fn last_typing_update(room_id: &RoomId) -> AppResult<i64> {
    maintain_typings(room_id).await?;
    Ok(LAST_TYPING_UPDATE
        .read()
        .await
        .get(room_id)
        .copied()
        .unwrap_or_default())
}

/// Returns a new typing EDU.
pub async fn all_typings(
    room_id: &RoomId,
) -> AppResult<SyncEphemeralRoomEvent<TypingEventContent>> {
    Ok(SyncEphemeralRoomEvent {
        content: TypingEventContent {
            user_ids: TYPING
                .read()
                .await
                .get(room_id)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default(),
        },
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
