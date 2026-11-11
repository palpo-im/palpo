use std::collections::HashSet;

use diesel::prelude::*;
use horme::RunQueryDsl;

use crate::DataResult;
use crate::core::identifiers::*;
use crate::schema::*;

/// Whether `confirmed_user_id`'s membership was already lazily sent to this
/// `(user, device, room)`.
pub async fn was_sent_before(
    user_id: &UserId,
    device_id: &DeviceId,
    room_id: &RoomId,
    confirmed_user_id: &UserId,
) -> DataResult<bool> {
    let query = lazy_load_deliveries::table
        .filter(lazy_load_deliveries::user_id.eq(user_id))
        .filter(lazy_load_deliveries::device_id.eq(device_id))
        .filter(lazy_load_deliveries::room_id.eq(room_id))
        .filter(lazy_load_deliveries::confirmed_user_id.eq(confirmed_user_id));
    diesel_exists!(query, &mut crate::db()).map_err(Into::into)
}

/// Record that each `confirmed_user_id` was lazily delivered to this
/// `(user, device, room)`. Reuses one connection for the whole batch.
pub async fn mark_sent(
    user_id: &UserId,
    device_id: &DeviceId,
    room_id: &RoomId,
    confirmed_user_ids: HashSet<OwnedUserId>,
) -> DataResult<()> {
    let mut conn = crate::db();
    for confirmed_user_id in confirmed_user_ids {
        diesel::insert_into(lazy_load_deliveries::table)
            .values((
                lazy_load_deliveries::user_id.eq(user_id),
                lazy_load_deliveries::device_id.eq(device_id),
                lazy_load_deliveries::room_id.eq(room_id),
                lazy_load_deliveries::confirmed_user_id.eq(&confirmed_user_id),
            ))
            .on_conflict_do_nothing()
            .execute(&mut conn)
            .await?;
    }
    Ok(())
}

/// Forget all lazy-load deliveries for a `(user, device, room)`.
pub async fn reset(user_id: &UserId, device_id: &DeviceId, room_id: &RoomId) -> DataResult<()> {
    diesel::delete(
        lazy_load_deliveries::table
            .filter(lazy_load_deliveries::user_id.eq(user_id))
            .filter(lazy_load_deliveries::device_id.eq(device_id))
            .filter(lazy_load_deliveries::room_id.eq(room_id)),
    )
    .execute(&mut crate::db())
    .await?;
    Ok(())
}
