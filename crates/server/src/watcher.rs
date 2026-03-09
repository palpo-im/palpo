use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use diesel::prelude::*;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;

use crate::AppResult;
use crate::core::Seqnum;
use crate::core::identifiers::*;
use crate::data::schema::*;
use crate::data::{self, connect};

pub async fn watch(user_id: &UserId, device_id: &DeviceId) -> AppResult<()> {
    let mut conn = connect()?;

    let inbox_id = device_inboxes::table
        .filter(device_inboxes::user_id.eq(user_id))
        .filter(device_inboxes::device_id.eq(device_id))
        .order_by(device_inboxes::id.desc())
        .select(device_inboxes::id)
        .first::<i64>(&mut conn)
        .unwrap_or_default();
    let key_change_id = e2e_key_changes::table
        .filter(e2e_key_changes::user_id.eq(user_id))
        .order_by(e2e_key_changes::id.desc())
        .select(e2e_key_changes::id)
        .first::<i64>(&mut conn)
        .unwrap_or_default();
    let room_user_id = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .order_by(room_users::id.desc())
        .select(room_users::id)
        .first::<i64>(&mut conn)
        .unwrap_or_default();

    let room_ids = data::user::joined_rooms(user_id)?;
    let last_event_sn = event_points::table
        .filter(event_points::room_id.eq_any(&room_ids))
        .filter(event_points::frame_id.is_not_null())
        .order_by(event_points::event_sn.desc())
        .select(event_points::event_sn)
        .first::<Seqnum>(&mut conn)
        .unwrap_or_default();

    let push_rule_sn = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .order_by(user_datas::occur_sn.desc())
        .select(user_datas::occur_sn)
        .first::<i64>(&mut conn)
        .unwrap_or_default();

    // Get the current max typing occur_sn for this user's rooms
    let last_typing_sn = room_typings::table
        .filter(room_typings::room_id.eq_any(&room_ids))
        .select(diesel::dsl::max(room_typings::occur_sn))
        .first::<Option<i64>>(&mut conn)
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
            let current_room_ids = match data::user::joined_rooms(user_id) {
                Ok(ids) => ids,
                Err(e) => {
                    tracing::warn!("watcher: failed to fetch joined rooms: {e}");
                    room_ids.clone()
                }
            };

            let mut conn = connect()?;

            // Check typing changes (DB-backed, works across instances)
            let new_typing_sn = room_typings::table
                .filter(room_typings::room_id.eq_any(&current_room_ids))
                .select(diesel::dsl::max(room_typings::occur_sn))
                .first::<Option<i64>>(&mut conn)
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
                .unwrap_or_default();
            if inbox_id < new_inbox_id {
                return Ok(());
            }

            let new_key_change_id = e2e_key_changes::table
                .filter(e2e_key_changes::user_id.eq(user_id))
                .order_by(e2e_key_changes::id.desc())
                .select(e2e_key_changes::id)
                .first::<i64>(&mut conn)
                .unwrap_or_default();
            if key_change_id < new_key_change_id {
                return Ok(());
            }

            let new_room_user_id = room_users::table
                .filter(room_users::user_id.eq(user_id))
                .order_by(room_users::id.desc())
                .select(room_users::id)
                .first::<i64>(&mut conn)
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
                .unwrap_or_default();
            if last_event_sn < new_event_sn {
                return Ok(());
            }

            let new_push_rule_sn = user_datas::table
                .filter(user_datas::user_id.eq(user_id))
                .order_by(user_datas::occur_sn.desc())
                .select(user_datas::occur_sn)
                .first::<i64>(&mut conn)
                .unwrap_or_default();
            if push_rule_sn < new_push_rule_sn {
                return Ok(());
            }
        }
        Ok(())
    })));
    // Wait until one of them finds something
    futures.next().await;
    Ok(())
}
