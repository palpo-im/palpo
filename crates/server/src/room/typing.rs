use std::sync::LazyLock;
use std::time::Duration;

use tokio::sync::broadcast;

use crate::core::UnixMillis;
use crate::core::events::SyncEphemeralRoomEvent;
use crate::core::events::typing::{TypingContent, TypingEventContent};
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::{AppResult, IsRemoteOrLocal, data, sending};

/// Local broadcast channel for same-instance fast notification.
/// Cross-instance coordination happens via DB polling in the watcher.
static TYPING_UPDATE_SENDER: LazyLock<broadcast::Sender<OwnedRoomId>> =
    LazyLock::new(|| broadcast::channel(100).0);

/// How often passed typing timeouts are published to sync.
const TYPING_EXPIRY_SWEEP: Duration = Duration::from_secs(1);

/// Sets a user as typing until the timeout timestamp is reached or remove_typing is called.
/// State is persisted to the database for cross-instance visibility.
pub async fn add_typing(
    user_id: &UserId,
    room_id: &RoomId,
    timeout: u64,
    broadcast: bool,
) -> AppResult<()> {
    let timeout = timeout.min(typing_deadline_cap(UnixMillis::now().get()));
    data::room::typing::upsert_typing(room_id, user_id, timeout as i64).await?;

    // Notify same-instance watchers immediately
    let _ = TYPING_UPDATE_SENDER.send(room_id.to_owned());

    if broadcast && user_id.is_local() {
        federation_send(room_id, user_id, true).await.ok();
    }
    Ok(())
}

/// When a remote user's typing notice ends. Federation EDUs carry no timeout,
/// so `federation_timeout_ms` (config `typing.federation_timeout`, already in
/// milliseconds) is added to `now_ms` as is.
pub fn federation_deadline(now_ms: u64, federation_timeout_ms: u64) -> u64 {
    // Stored as i64 downstream (add_typing): clamp so the cast never wraps.
    now_ms
        .saturating_add(federation_timeout_ms)
        .min(i64::MAX as u64)
}

/// Removes a user from typing before the timeout is reached.
/// Instead of deleting the row, it is marked stopped and occur_sn is bumped
/// so that sync picks up the "typing stopped" change before cleanup.
pub async fn remove_typing(user_id: &UserId, room_id: &RoomId, broadcast: bool) -> AppResult<()> {
    data::room::typing::stop_typing(room_id, user_id, UnixMillis::now().get() as i64).await?;

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

/// Latest deadline any typing notice may hold: `now_ms` plus the longer of
/// `typing.federation_timeout` and `typing.client_timeout_max`.
fn typing_deadline_cap(now_ms: u64) -> u64 {
    match crate::config::CONFIG.get() {
        Some(conf) => deadline_cap(now_ms, &conf.typing),
        // No config is loaded in database tests.
        None => deadline_cap(now_ms, &crate::config::TypingConfig::default()),
    }
}

fn deadline_cap(now_ms: u64, typing: &crate::config::TypingConfig) -> u64 {
    now_ms
        .saturating_add(typing.federation_timeout.max(typing.client_timeout_max))
        .min(i64::MAX as u64)
}

/// Publish passed typing timeouts so waiting syncs drop the user.
pub async fn publish_expired_typings() -> AppResult<()> {
    data::room::typing::publish_expired_typings(UnixMillis::now().get() as i64).await?;
    Ok(())
}

/// Clamp stored deadlines beyond [`typing_deadline_cap`]; repairs rows that
/// otherwise kept "X is typing" up for hours (#660).
pub async fn clamp_stale_typings() -> AppResult<()> {
    let cap = typing_deadline_cap(UnixMillis::now().get()) as i64;
    data::room::typing::clamp_typing_deadlines(cap).await?;
    Ok(())
}

/// Clamp stale deadlines once, then publish typing expiries for the life of
/// the process.
pub fn start_expiry_sweeper() {
    tokio::spawn(async move {
        if let Err(e) = clamp_stale_typings().await {
            // Non-fatal: stale rows still expire on their own, just late.
            tracing::warn!(error = ?e, "failed to clamp stale typing deadlines");
        }
        let mut interval = tokio::time::interval(TYPING_EXPIRY_SWEEP);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(e) = publish_expired_typings().await {
                // Retried on the next tick; a lost tick only delays the expiry.
                tracing::warn!(error = ?e, "failed to publish expired typing notifications");
            }
        }
    });
}

/// Removes expired typing entries from the database.
async fn maintain_typings(room_id: &RoomId) -> AppResult<()> {
    let current_timestamp = UnixMillis::now().get() as i64;
    data::room::typing::delete_expired_typings(room_id, current_timestamp).await?;
    Ok(())
}

/// Returns the sequence number of the last typing update in this room.
/// Reads from the database so it works across all instances.
/// We query max occur_sn BEFORE cleanup so that "stop typing" events
/// (rows kept, marked stopped) are still visible to sync.
pub async fn last_typing_update(room_id: &RoomId) -> AppResult<i64> {
    Ok(data::room::typing::last_typing_sn(room_id).await?)
}

/// Returns a new typing EDU with currently typing users from the database.
pub async fn all_typings(
    room_id: &RoomId,
) -> AppResult<SyncEphemeralRoomEvent<TypingEventContent>> {
    maintain_typings(room_id).await?;
    let user_ids =
        data::room::typing::typing_user_ids(room_id, UnixMillis::now().get() as i64).await?;

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
    sending::send_edu_room(room_id, &edu).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::deadline_cap;
    use crate::config::TypingConfig;

    #[test]
    fn deadline_cap_is_the_longer_of_federation_and_client_max() {
        let typing = TypingConfig::default(); // 30_000 federation, 45_000 client max
        assert_eq!(deadline_cap(1_000, &typing), 46_000);
        let typing = TypingConfig {
            federation_timeout: 90_000,
            ..TypingConfig::default()
        };
        assert_eq!(deadline_cap(1_000, &typing), 91_000);
        assert_eq!(deadline_cap(u64::MAX, &typing), i64::MAX as u64);
    }
}

#[cfg(test)]
mod database_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_typing_expiry_is_published_once() {
        crate::test_database::init();
        let room: OwnedRoomId = "!typing-expiry:example.org".try_into().unwrap();
        let user: OwnedUserId = "@typer:example.org".try_into().unwrap();
        let now = UnixMillis::now().get() as i64;
        let sn = data::room::typing::upsert_typing(&room, &user, now - 1)
            .await
            .unwrap();
        publish_expired_typings().await.unwrap();
        let published = last_typing_update(&room).await.unwrap();
        assert!(published > sn, "expiry did not move the typing stream");
        assert!(
            all_typings(&room)
                .await
                .unwrap()
                .content
                .user_ids
                .is_empty()
        );
        // Already announced: later sweeps leave the stream alone.
        publish_expired_typings().await.unwrap();
        assert_eq!(last_typing_update(&room).await.unwrap(), published);
        // An explicit stop is not republished as an expiry either.
        add_typing(&user, &room, (now + 10_000) as u64, false)
            .await
            .unwrap();
        remove_typing(&user, &room, false).await.unwrap();
        let stopped = last_typing_update(&room).await.unwrap();
        publish_expired_typings().await.unwrap();
        assert_eq!(last_typing_update(&room).await.unwrap(), stopped);
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_stale_far_future_typing_is_clamped_and_republished() {
        crate::test_database::init();
        let room: OwnedRoomId = "!typing-clamp:example.org".try_into().unwrap();
        let stale: OwnedUserId = "@stale:example.org".try_into().unwrap();
        let fresh: OwnedUserId = "@fresh:example.org".try_into().unwrap();
        let now = UnixMillis::now().get();
        let hours_ahead = (now + 8 * 3_600_000) as i64;
        data::room::typing::upsert_typing(&room, &stale, hours_ahead)
            .await
            .unwrap();
        let fresh_sn = data::room::typing::upsert_typing(&room, &fresh, (now + 10_000) as i64)
            .await
            .unwrap();

        clamp_stale_typings().await.unwrap();
        let clamped_sn = last_typing_update(&room).await.unwrap();
        assert!(
            clamped_sn > fresh_sn,
            "clamp did not bump the typing sequence"
        );
        let cap = typing_deadline_cap(UnixMillis::now().get()) as i64;
        let still_typing = data::room::typing::typing_user_ids(&room, cap + 1)
            .await
            .unwrap();
        assert!(
            !still_typing.contains(&stale),
            "stale typing still outlives the cap"
        );

        // Idempotent: nothing beyond the cap remains.
        clamp_stale_typings().await.unwrap();
        assert_eq!(last_typing_update(&room).await.unwrap(), clamped_sn);

        // add_typing never stores a deadline beyond the cap.
        add_typing(&stale, &room, hours_ahead as u64, false)
            .await
            .unwrap();
        let cap = typing_deadline_cap(UnixMillis::now().get()) as i64;
        let still_typing = data::room::typing::typing_user_ids(&room, cap + 1)
            .await
            .unwrap();
        assert!(
            !still_typing.contains(&stale),
            "add_typing stored a deadline beyond the cap"
        );
    }
}

#[cfg(test)]
#[path = "typing_publication_tests.rs"]
mod publication_tests;

#[cfg(test)]
mod federation_deadline_tests {
    use super::federation_deadline;

    #[test]
    fn remote_typing_ends_after_the_configured_milliseconds() {
        // Default typing.federation_timeout is 30_000 ms: the notice must end
        // 30 s later, not 30_000 s (the old code multiplied by 1000 again).
        assert_eq!(federation_deadline(1_000, 30_000), 31_000);
    }

    #[test]
    fn deadline_clamps_to_what_add_typing_can_store() {
        assert_eq!(federation_deadline(u64::MAX - 1, 30_000), i64::MAX as u64);
    }
}
