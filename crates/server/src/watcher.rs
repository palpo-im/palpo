use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;

use crate::AppResult;
use crate::core::Seqnum;
use crate::core::identifiers::*;
use crate::data::schema::*;
use crate::data::{self, connect};

async fn latest_shared_profile_change_sn(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    room_ids: &[OwnedRoomId],
) -> Seqnum {
    let shared_users = room_users::table
        .filter(room_users::room_id.eq_any(room_ids))
        .filter(room_users::membership.eq("join"))
        .select(room_users::user_id);

    user_profile_changes::table
        .filter(
            user_profile_changes::user_id
                .eq(user_id)
                .or(user_profile_changes::user_id.eq_any(shared_users)),
        )
        .select(diesel::dsl::max(user_profile_changes::occur_sn))
        .first::<Option<Seqnum>>(conn)
        .await
        .unwrap_or(None)
        .unwrap_or_default()
}

fn profile_change_is_ready(
    profile_updates: bool,
    profile_after_sn: Option<Seqnum>,
    latest_change_sn: Seqnum,
) -> bool {
    profile_updates && profile_after_sn.is_some_and(|after_sn| latest_change_sn >= after_sn)
}

pub async fn watch(
    user_id: &UserId,
    device_id: &DeviceId,
    profile_updates: bool,
    profile_after_sn: Option<Seqnum>,
) -> AppResult<()> {
    // Resolve joined rooms *before* checking out a pooled connection. This call
    // acquires its own connection internally; doing it while `conn` below is
    // held would pin two connections per in-flight long-poll and can exhaust
    // the (small) pool under concurrent syncs.
    let room_ids = data::user::joined_rooms(user_id).await?;

    let mut conn = connect().await?;

    let inbox_id = device_inboxes::table
        .filter(device_inboxes::user_id.eq(user_id))
        .filter(device_inboxes::device_id.eq(device_id))
        .order_by(device_inboxes::id.desc())
        .select(device_inboxes::id)
        .first::<i64>(&mut conn)
        .await
        .unwrap_or_default();
    let key_change_id = e2e_key_changes::table
        .filter(e2e_key_changes::user_id.eq(user_id))
        .order_by(e2e_key_changes::id.desc())
        .select(e2e_key_changes::id)
        .first::<i64>(&mut conn)
        .await
        .unwrap_or_default();
    let room_user_id = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .order_by(room_users::id.desc())
        .select(room_users::id)
        .first::<i64>(&mut conn)
        .await
        .unwrap_or_default();

    let last_event_sn = event_points::table
        .filter(event_points::room_id.eq_any(&room_ids))
        .filter(event_points::frame_id.is_not_null())
        .order_by(event_points::event_sn.desc())
        .select(event_points::event_sn)
        .first::<Seqnum>(&mut conn)
        .await
        .unwrap_or_default();

    let push_rule_sn = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .order_by(user_datas::occur_sn.desc())
        .select(user_datas::occur_sn)
        .first::<i64>(&mut conn)
        .await
        .unwrap_or_default();

    let profile_change_sn = if profile_updates {
        latest_shared_profile_change_sn(&mut conn, user_id, &room_ids).await
    } else {
        0
    };

    // The first sync response was built before this watcher registered. If a profile
    // write committed in that window, its position is at or beyond the response's next
    // token and must wake the request immediately instead of becoming the watch baseline.
    if profile_change_is_ready(profile_updates, profile_after_sn, profile_change_sn) {
        return Ok(());
    }

    // Get the current max typing occur_sn for this user's rooms
    let last_typing_sn = room_typings::table
        .filter(room_typings::room_id.eq_any(&room_ids))
        .select(diesel::dsl::max(room_typings::occur_sn))
        .first::<Option<i64>>(&mut conn)
        .await
        .unwrap_or(None)
        .unwrap_or_default();

    drop(conn);

    let mut futures: FuturesUnordered<Pin<Box<dyn Future<Output = AppResult<()>> + Send>>> =
        FuturesUnordered::new();

    // Listen for ROTATE (shutdown/long-poll release) signals (same-instance only)
    futures.push(Box::into_pin(Box::new(async move {
        crate::ROTATE.watch().await;
        Ok(())
    })));

    // DB-polling loop that detects changes from ALL instances
    futures.push(Box::into_pin(Box::new(async move {
        const POLL_INTERVAL: Duration = Duration::from_secs(3);
        const MAX_POLLS: usize = 10;

        for _ in 0..MAX_POLLS {
            tokio::time::sleep(POLL_INTERVAL).await;

            // Re-fetch room_ids to handle joins/leaves during the wait
            let current_room_ids = match data::user::joined_rooms(user_id).await {
                Ok(ids) => ids,
                Err(e) => {
                    tracing::warn!("watcher: failed to fetch joined rooms: {e}");
                    room_ids.clone()
                }
            };

            let mut conn = connect().await?;

            // Check typing changes (DB-backed, works across instances)
            let new_typing_sn = room_typings::table
                .filter(room_typings::room_id.eq_any(&current_room_ids))
                .select(diesel::dsl::max(room_typings::occur_sn))
                .first::<Option<i64>>(&mut conn)
                .await
                .unwrap_or(None)
                .unwrap_or_default();
            if last_typing_sn < new_typing_sn {
                return Ok(());
            }

            let new_inbox_id = device_inboxes::table
                .filter(device_inboxes::user_id.eq(user_id))
                .filter(device_inboxes::device_id.eq(device_id))
                .order_by(device_inboxes::id.desc())
                .select(device_inboxes::id)
                .first::<i64>(&mut conn)
                .await
                .unwrap_or_default();
            if inbox_id < new_inbox_id {
                return Ok(());
            }

            let new_key_change_id = e2e_key_changes::table
                .filter(e2e_key_changes::user_id.eq(user_id))
                .order_by(e2e_key_changes::id.desc())
                .select(e2e_key_changes::id)
                .first::<i64>(&mut conn)
                .await
                .unwrap_or_default();
            if key_change_id < new_key_change_id {
                return Ok(());
            }

            let new_room_user_id = room_users::table
                .filter(room_users::user_id.eq(user_id))
                .order_by(room_users::id.desc())
                .select(room_users::id)
                .first::<i64>(&mut conn)
                .await
                .unwrap_or_default();
            if room_user_id < new_room_user_id {
                return Ok(());
            }

            let new_event_sn = event_points::table
                .filter(event_points::room_id.eq_any(&current_room_ids))
                .filter(event_points::frame_id.is_not_null())
                .order_by(event_points::event_sn.desc())
                .select(event_points::event_sn)
                .first::<Seqnum>(&mut conn)
                .await
                .unwrap_or_default();
            if last_event_sn < new_event_sn {
                return Ok(());
            }

            let new_push_rule_sn = user_datas::table
                .filter(user_datas::user_id.eq(user_id))
                .order_by(user_datas::occur_sn.desc())
                .select(user_datas::occur_sn)
                .first::<i64>(&mut conn)
                .await
                .unwrap_or_default();
            if push_rule_sn < new_push_rule_sn {
                return Ok(());
            }

            if profile_updates {
                let new_profile_change_sn =
                    latest_shared_profile_change_sn(&mut conn, user_id, &current_room_ids).await;
                if profile_change_sn < new_profile_change_sn {
                    return Ok(());
                }
            }
        }
        Ok(())
    })));
    // Wait until one of them finds something
    futures.next().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::profile_change_is_ready;

    #[test]
    fn profile_changes_at_or_after_the_response_token_are_already_ready() {
        assert!(profile_change_is_ready(true, Some(10), 10));
        assert!(profile_change_is_ready(true, Some(10), 11));
        assert!(!profile_change_is_ready(true, Some(10), 9));
        assert!(!profile_change_is_ready(false, Some(10), 11));
        assert!(!profile_change_is_ready(true, None, 11));
    }
}
