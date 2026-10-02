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

/// Mark a user as no longer typing: the timeout becomes `now_ms` (already
/// expired) and `occur_sn` is bumped, so every sync connection observes the
/// "stopped" transition while the row is kept for `TYPING_ROW_GRACE_MS`.
pub async fn stop_typing(
    room_id: &RoomId,
    user_id: &UserId,
    occur_sn: Seqnum,
    now_ms: i64,
) -> DataResult<()> {
    diesel::update(
        room_typings::table
            .filter(room_typings::room_id.eq(room_id))
            .filter(room_typings::user_id.eq(user_id)),
    )
    .set((
        room_typings::timeout_at.eq(now_ms),
        room_typings::occur_sn.eq(occur_sn),
    ))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// How long an expired or stopped typing row is kept. Deleting it at once
/// erased its `occur_sn`, so a sync connection that had not yet seen the change
/// (another client's sync ran the cleanup first) never learned the user stopped
/// and kept showing them as typing.
pub const TYPING_ROW_GRACE_MS: i64 = 5 * 60 * 1000;

/// Delete typing rows that expired more than `TYPING_ROW_GRACE_MS` ago.
pub async fn delete_expired_typings(room_id: &RoomId, now_ms: i64) -> DataResult<()> {
    diesel::delete(
        room_typings::table
            .filter(room_typings::room_id.eq(room_id))
            .filter(room_typings::timeout_at.lt(now_ms - TYPING_ROW_GRACE_MS)),
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

/// Users currently typing in a room: rows whose timeout is still ahead.
pub async fn typing_user_ids(room_id: &RoomId, now_ms: i64) -> DataResult<Vec<OwnedUserId>> {
    room_typings::table
        .filter(room_typings::room_id.eq(room_id))
        .filter(room_typings::timeout_at.ge(now_ms))
        .select(room_typings::user_id)
        .load::<OwnedUserId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}
