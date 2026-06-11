use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::Seqnum;
use crate::core::identifiers::*;
use crate::schema::*;
use crate::{DataResult, connect};

/// Mark a user as typing until `timeout_at`, replacing any existing entry.
pub async fn upsert_typing(
    room_id: &RoomId,
    user_id: &UserId,
    timeout_at: i64,
    occur_sn: Seqnum,
) -> DataResult<()> {
    diesel::insert_into(room_typings::table)
        .values((
            room_typings::room_id.eq(room_id),
            room_typings::user_id.eq(user_id),
            room_typings::timeout_at.eq(timeout_at),
            room_typings::occur_sn.eq(occur_sn),
        ))
        .on_conflict((room_typings::room_id, room_typings::user_id))
        .do_update()
        .set((
            room_typings::timeout_at.eq(timeout_at),
            room_typings::occur_sn.eq(occur_sn),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Mark a user as no longer typing by zeroing the timeout and bumping `occur_sn`
/// so sync still observes the "stopped" transition before cleanup.
pub async fn stop_typing(room_id: &RoomId, user_id: &UserId, occur_sn: Seqnum) -> DataResult<()> {
    diesel::update(
        room_typings::table
            .filter(room_typings::room_id.eq(room_id))
            .filter(room_typings::user_id.eq(user_id)),
    )
    .set((
        room_typings::timeout_at.eq(0i64),
        room_typings::occur_sn.eq(occur_sn),
    ))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Delete typing rows whose timeout is strictly before `now_ms`.
pub async fn delete_expired_typings(room_id: &RoomId, now_ms: i64) -> DataResult<()> {
    diesel::delete(
        room_typings::table
            .filter(room_typings::room_id.eq(room_id))
            .filter(room_typings::timeout_at.lt(now_ms)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Sequence number of the most recent typing update in a room (0 if none).
pub async fn last_typing_sn(room_id: &RoomId) -> DataResult<Seqnum> {
    let sn = room_typings::table
        .filter(room_typings::room_id.eq(room_id))
        .select(diesel::dsl::max(room_typings::occur_sn))
        .first::<Option<Seqnum>>(&mut connect().await?)
        .await
        .unwrap_or(None)
        .unwrap_or_default();
    Ok(sn)
}

/// Users with a typing row in a room (callers should expire stale rows first).
pub async fn typing_user_ids(room_id: &RoomId) -> DataResult<Vec<OwnedUserId>> {
    room_typings::table
        .filter(room_typings::room_id.eq(room_id))
        .select(room_typings::user_id)
        .load::<OwnedUserId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}
