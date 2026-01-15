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
    let inbox_id = device_inboxes::table
        .filter(device_inboxes::user_id.eq(user_id))
        .filter(device_inboxes::device_id.eq(device_id))
        .order_by(device_inboxes::id.desc())
        .select(device_inboxes::id)
        .first::<i64>(&mut connect()?)
        .unwrap_or_default();
    let key_change_id = e2e_key_changes::table
        .filter(e2e_key_changes::user_id.eq(user_id))
        .order_by(e2e_key_changes::id.desc())
        .select(e2e_key_changes::id)
        .first::<i64>(&mut connect()?)
        .unwrap_or_default();
    let room_user_id = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .order_by(room_users::id.desc())
        .select(room_users::id)
        .first::<i64>(&mut connect()?)
        .unwrap_or_default();

    let room_ids = data::user::joined_rooms(user_id)?;
    let last_event_sn = event_points::table
        .filter(event_points::room_id.eq_any(&room_ids))
        .filter(event_points::frame_id.is_not_null())
        .order_by(event_points::event_sn.desc())
        .select(event_points::event_sn)
        .first::<Seqnum>(&mut connect()?)
        .unwrap_or_default();

    let push_rule_sn = user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .order_by(user_datas::occur_sn.desc())
        .select(user_datas::occur_sn)
        .first::<i64>(&mut connect()?)
        .unwrap_or_default();

    let mut futures: FuturesUnordered<Pin<Box<dyn Future<Output = AppResult<()>> + Send>>> =
        FuturesUnordered::new();

    for room_id in room_ids.clone() {
        futures.push(Box::into_pin(Box::new(async move {
            crate::room::typing::wait_for_update(&room_id).await
        })));
    }
    futures.push(Box::into_pin(Box::new(async move {
        loop {
            if inbox_id
                < device_inboxes::table
                    .filter(device_inboxes::user_id.eq(user_id))
                    .filter(device_inboxes::device_id.eq(device_id))
                    .order_by(device_inboxes::id.desc())
                    .select(device_inboxes::id)
                    .first::<i64>(&mut connect()?)
                    .unwrap_or_default()
            {
                return Ok(());
            }
            if key_change_id
                < e2e_key_changes::table
                    .filter(e2e_key_changes::user_id.eq(user_id))
                    .order_by(e2e_key_changes::id.desc())
                    .select(e2e_key_changes::id)
                    .first::<i64>(&mut connect()?)
                    .unwrap_or_default()
            {
                return Ok(());
            }
            if room_user_id
                < room_users::table
                    .filter(room_users::user_id.eq(user_id))
                    .order_by(room_users::id.desc())
                    .select(room_users::id)
                    .first::<i64>(&mut connect()?)
                    .unwrap_or_default()
            {
                return Ok(());
            }
            if last_event_sn
                < event_points::table
                    .filter(event_points::room_id.eq_any(&room_ids))
                    .filter(event_points::frame_id.is_not_null())
                    .order_by(event_points::event_sn.desc())
                    .select(event_points::event_sn)
                    .first::<Seqnum>(&mut connect()?)
                    .unwrap_or_default()
            {
                return Ok(());
            }
            if push_rule_sn
                < user_datas::table
                    .filter(user_datas::user_id.eq(user_id))
                    .order_by(user_datas::occur_sn.desc())
                    .select(user_datas::occur_sn)
                    .first::<i64>(&mut connect()?)
                    .unwrap_or_default()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })));
    // Wait until one of them finds something
    futures.next().await;
    Ok(())
}
