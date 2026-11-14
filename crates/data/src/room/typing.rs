use diesel::prelude::*;
use horme::RunQueryDsl;

use crate::DataResult;
use crate::core::Seqnum;
use crate::core::identifiers::*;
use crate::schema::*;

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
        .execute(&mut crate::db())
        .await?;
    Ok(())
}

/// A row whose stop has been published stores `timeout_at` as the negated
/// instant it stopped. Live rows hold a positive timeout, so the expiry sweep
/// can tell an unpublished expiry from one already announced without a schema
/// change, and readers comparing `timeout_at >= now` see neither.
fn stopped_marker(stopped_at_ms: i64) -> i64 {
    -stopped_at_ms
}

/// Mark a user as no longer typing: the row records the stop (see
/// [`stopped_marker`]) and `occur_sn` is bumped, so every sync connection
/// observes the "stopped" transition while the row is kept for
/// `TYPING_ROW_GRACE_MS`.
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
        room_typings::timeout_at.eq(stopped_marker(now_ms)),
        room_typings::occur_sn.eq(occur_sn),
    ))
    .execute(&mut crate::db())
    .await?;
    Ok(())
}

/// How long an expired or stopped typing row is kept. Deleting it at once
/// erased its `occur_sn`, so a sync connection that had not yet seen the change
/// (another client's sync ran the cleanup first) never learned the user stopped
/// and kept showing them as typing.
pub const TYPING_ROW_GRACE_MS: i64 = 5 * 60 * 1000;

/// Delete typing rows that stopped or expired more than `TYPING_ROW_GRACE_MS` ago.
pub async fn delete_expired_typings(room_id: &RoomId, now_ms: i64) -> DataResult<()> {
    let cutoff = now_ms - TYPING_ROW_GRACE_MS;
    diesel::delete(
        room_typings::table
            .filter(room_typings::room_id.eq(room_id))
            .filter(
                room_typings::timeout_at
                    .lt(cutoff)
                    .and(room_typings::timeout_at.ge(0))
                    .or(room_typings::timeout_at
                        .lt(0)
                        .and(room_typings::timeout_at.gt(stopped_marker(cutoff)))),
            ),
    )
    .execute(&mut crate::db())
    .await?;
    Ok(())
}

/// Publish every typing timeout that has passed: bump `occur_sn` once per
/// expiry, as [`stop_typing`] does for an explicit stop, so a waiting sync
/// sees the typing stream move and delivers the shrunken list. Without it a
/// timeout (the usual end of remote typing) never reached clients.
pub async fn publish_expired_typings(now_ms: i64) -> DataResult<usize> {
    diesel::update(
        room_typings::table
            // Zero is excluded: negating it would match again on every sweep.
            .filter(room_typings::timeout_at.gt(0))
            .filter(room_typings::timeout_at.lt(now_ms)),
    )
    .set((
        room_typings::timeout_at.eq(diesel::dsl::sql::<diesel::sql_types::BigInt>("-timeout_at")),
        room_typings::occur_sn.eq(diesel::dsl::sql::<diesel::sql_types::BigInt>(
            "nextval('occur_sn_seq')",
        )),
    ))
    .execute(&mut crate::db())
    .await
    .map_err(Into::into)
}

/// Pull every live typing deadline later than `max_timeout_at` back to it,
/// bumping `occur_sn` so sync republishes the list. Rows written while the
/// federated timeout was scaled twice (#660) sit hours in the future.
pub async fn clamp_typing_deadlines(max_timeout_at: i64) -> DataResult<usize> {
    diesel::update(room_typings::table.filter(room_typings::timeout_at.gt(max_timeout_at)))
        .set((
            room_typings::timeout_at.eq(max_timeout_at),
            room_typings::occur_sn.eq(diesel::dsl::sql::<diesel::sql_types::BigInt>(
                "nextval('occur_sn_seq')",
            )),
        ))
        .execute(&mut crate::db())
        .await
        .map_err(Into::into)
}

/// Sequence number of the most recent typing update in a room (0 if none).
pub async fn last_typing_sn(room_id: &RoomId) -> DataResult<Seqnum> {
    let sn = room_typings::table
        .filter(room_typings::room_id.eq(room_id))
        .select(diesel::dsl::max(room_typings::occur_sn))
        .first::<Option<Seqnum>>(&mut crate::db())
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
        .load::<OwnedUserId>(&mut crate::db())
        .await
        .map_err(Into::into)
}
