use std::collections::HashSet;

use palpo_core::Seqnum;

use crate::core::{DeviceId, OwnedUserId, RoomId, UserId};
use crate::{AppResult, data};

#[tracing::instrument]
pub async fn lazy_load_was_sent_before(
    user_id: &UserId,
    device_id: &DeviceId,
    room_id: &RoomId,
    confirmed_user_id: &UserId,
) -> AppResult<bool> {
    Ok(
        data::room::lazy_loading::was_sent_before(user_id, device_id, room_id, confirmed_user_id)
            .await?,
    )
}

/// Marks lazy load entries as sent by writing directly to the database.
/// In a clustered environment, this ensures all instances see the same state.
#[tracing::instrument]
pub async fn lazy_load_mark_sent(
    user_id: &UserId,
    device_id: &DeviceId,
    room_id: &RoomId,
    lazy_load: HashSet<OwnedUserId>,
    _until_sn: Seqnum,
) {
    if let Err(e) =
        data::room::lazy_loading::mark_sent(user_id, device_id, room_id, lazy_load).await
    {
        warn!("failed to mark lazy-load deliveries as sent: {e}");
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
pub async fn lazy_load_reset(
    user_id: &UserId,
    device_id: &DeviceId,
    room_id: &RoomId,
) -> AppResult<()> {
    data::room::lazy_loading::reset(user_id, device_id, room_id).await?;
    Ok(())
}
