use std::collections::HashSet;

use diesel::prelude::*;
use palpo_core::Seqnum;

use crate::AppResult;
use crate::core::{DeviceId, OwnedUserId, RoomId, UserId};
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};

#[tracing::instrument]
pub fn lazy_load_was_sent_before(
    user_id: &UserId,
    device_id: &DeviceId,
    room_id: &RoomId,
    confirmed_user_id: &UserId,
) -> AppResult<bool> {
    let query = lazy_load_deliveries::table
        .filter(lazy_load_deliveries::user_id.eq(user_id))
        .filter(lazy_load_deliveries::device_id.eq(device_id))
        .filter(lazy_load_deliveries::room_id.eq(room_id))
        .filter(lazy_load_deliveries::confirmed_user_id.eq(confirmed_user_id));
    diesel_exists!(query, &mut connect()?).map_err(Into::into)
}

/// Marks lazy load entries as sent by writing directly to the database.
/// In a clustered environment, this ensures all instances see the same state.
#[tracing::instrument]
pub fn lazy_load_mark_sent(
    user_id: &UserId,
    device_id: &DeviceId,
    room_id: &RoomId,
    lazy_load: HashSet<OwnedUserId>,
    _until_sn: Seqnum,
) {
    for confirmed_user_id in lazy_load {
        if let Ok(mut conn) = connect() {
            let _ = diesel::insert_into(lazy_load_deliveries::table)
                .values((
                    lazy_load_deliveries::user_id.eq(user_id),
                    lazy_load_deliveries::device_id.eq(device_id),
                    lazy_load_deliveries::room_id.eq(room_id),
                    lazy_load_deliveries::confirmed_user_id.eq(confirmed_user_id),
                ))
                .on_conflict_do_nothing()
                .execute(&mut conn);
        }
    }
}

/// No-op in the DB-backed implementation since `lazy_load_mark_sent` writes directly.
#[tracing::instrument]
pub fn lazy_load_confirm_delivery(
    _user_id: &UserId,
    _device_id: &DeviceId,
    _room_id: &RoomId,
    _occur_sn: Seqnum,
) -> AppResult<()> {
    // In the DB-backed implementation, lazy_load_mark_sent writes directly to
    // lazy_load_deliveries, so there's nothing to confirm.
    Ok(())
}

#[tracing::instrument]
pub fn lazy_load_reset(user_id: &UserId, device_id: &DeviceId, room_id: &RoomId) -> AppResult<()> {
    diesel::delete(
        lazy_load_deliveries::table
            .filter(lazy_load_deliveries::user_id.eq(user_id))
            .filter(lazy_load_deliveries::device_id.eq(device_id))
            .filter(lazy_load_deliveries::room_id.eq(room_id)),
    )
    .execute(&mut connect()?)?;
    Ok(())
}
