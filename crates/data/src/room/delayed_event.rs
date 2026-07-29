//! Persistence for MSC4140 delayed events.
//!
//! A delayed event is stored when scheduled and stays in the table after it is
//! finalized (sent, cancelled, or errored) so clients can look up the outcome.
//! The scheduler leases a due row in `claimed_at` and only sets `finalized_at`
//! once it knows the outcome, so a worker that dies mid-send leaves a row that
//! is still scheduled and can be reclaimed after [`CLAIM_LEASE_MS`].

use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::core::{DeviceId, TransactionId, UserId};
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = delayed_events)]
pub struct DbDelayedEvent {
    pub id: i64,
    pub delay_id: String,
    pub user_id: OwnedUserId,
    pub device_id: Option<OwnedDeviceId>,
    pub room_id: OwnedRoomId,
    pub event_type: String,
    pub state_key: Option<String>,
    pub content: JsonValue,
    pub delay_ms: i64,
    pub txn_id: OwnedTransactionId,
    pub origin_server_ts: Option<i64>,
    pub running_since: i64,
    pub send_at: i64,
    pub event_id: Option<OwnedEventId>,
    pub error: Option<JsonValue>,
    pub claimed_at: Option<i64>,
    pub finalized_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = delayed_events)]
pub struct NewDbDelayedEvent {
    pub delay_id: String,
    pub user_id: OwnedUserId,
    pub device_id: Option<OwnedDeviceId>,
    pub room_id: OwnedRoomId,
    pub event_type: String,
    pub state_key: Option<String>,
    pub content: JsonValue,
    pub delay_ms: i64,
    pub txn_id: OwnedTransactionId,
    pub origin_server_ts: Option<i64>,
    pub running_since: i64,
    pub send_at: i64,
    pub created_at: i64,
}

/// Outcome of trying to schedule a delayed event.
pub enum Scheduled {
    /// The event was stored.
    Created(DbDelayedEvent),
    /// A concurrent retry of the same transaction won the race; this is its
    /// row.
    AlreadyScheduled(DbDelayedEvent),
    /// The user is already at `max_scheduled`.
    LimitReached,
}

/// Store a newly scheduled delayed event, enforcing the per-user limit.
///
/// The count and the insert run under a transaction-scoped advisory lock keyed
/// on the user. Checking the count in the caller and inserting here separately
/// let concurrent requests with distinct transaction ids all observe a count
/// below the limit and all succeed, so a limit of 100 could be pushed well
/// past it; the unique transaction index does not serialize distinct
/// transactions.
///
/// Two concurrent retries of the *same* transaction are a different race: both
/// pass the caller's [`get_by_txn_id`] lookup before either commits. The
/// transaction id is therefore re-resolved under the lock, so the loser is
/// answered with the winner's row and scheduling stays idempotent.
pub async fn create(new: NewDbDelayedEvent, max_scheduled: i64) -> DataResult<Scheduled> {
    let user_id = new.user_id.clone();
    let device_id = new.device_id.clone();
    let txn_id = new.txn_id.clone();

    let mut conn = connect().await?;
    conn.transaction::<_, diesel::result::Error, _>(async |conn| {
        // Serialize scheduling per user for the rest of this transaction;
        // released automatically on commit or rollback.
        diesel::sql_query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind::<diesel::sql_types::Text, _>(user_id.as_str())
            .execute(&mut *conn)
            .await?;

        // Re-resolve the transaction now that this request holds the lock. The
        // caller's lookup ran before it, so a concurrent retry of the same
        // request may have committed in between. Doing this first means the
        // insert can no longer hit the unique index -- which matters because a
        // unique violation aborts the surrounding transaction in Postgres, and
        // every statement after it would fail with 25P02 -- and it means an
        // idempotent retry is answered with its delay id rather than being
        // rejected by the limit check below when it took the last slot.
        let mut existing = delayed_events::table
            .filter(delayed_events::user_id.eq(&user_id))
            .filter(delayed_events::txn_id.eq(&txn_id))
            .into_boxed();
        existing = match device_id.as_deref() {
            Some(device_id) => existing.filter(delayed_events::device_id.eq(device_id)),
            None => existing.filter(delayed_events::device_id.is_null()),
        };
        if let Some(row) = existing
            .first::<DbDelayedEvent>(&mut *conn)
            .await
            .optional()?
        {
            return Ok(Scheduled::AlreadyScheduled(row));
        }

        let scheduled: i64 = delayed_events::table
            .filter(delayed_events::user_id.eq(&user_id))
            .filter(delayed_events::finalized_at.is_null())
            .count()
            .get_result(&mut *conn)
            .await?;
        if scheduled >= max_scheduled {
            return Ok(Scheduled::LimitReached);
        }

        diesel::insert_into(delayed_events::table)
            .values(&new)
            .get_result::<DbDelayedEvent>(&mut *conn)
            .await
            .map(Scheduled::Created)
    })
    .await
    .map_err(Into::into)
}

/// Look up a delayed event previously scheduled with the same transaction id
/// on the same session, for idempotent retries of the scheduling request.
pub async fn get_by_txn_id(
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    txn_id: &TransactionId,
) -> DataResult<Option<DbDelayedEvent>> {
    let mut query = delayed_events::table
        .filter(delayed_events::user_id.eq(user_id))
        .filter(delayed_events::txn_id.eq(txn_id))
        .into_boxed();
    if let Some(device_id) = device_id {
        query = query.filter(delayed_events::device_id.eq(device_id));
    } else {
        query = query.filter(delayed_events::device_id.is_null());
    }
    query
        .first::<DbDelayedEvent>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Fetch one delayed event owned by the user, whether scheduled or finalized.
pub async fn get_by_delay_id(
    user_id: &UserId,
    delay_id: &str,
) -> DataResult<Option<DbDelayedEvent>> {
    delayed_events::table
        .filter(delayed_events::user_id.eq(user_id))
        .filter(delayed_events::delay_id.eq(delay_id))
        .first::<DbDelayedEvent>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// List the user's scheduled (not yet finalized) delayed events in
/// chronological order of their intended send time.
pub async fn list_scheduled(user_id: &UserId) -> DataResult<Vec<DbDelayedEvent>> {
    delayed_events::table
        .filter(delayed_events::user_id.eq(user_id))
        .filter(delayed_events::finalized_at.is_null())
        .order(delayed_events::send_at.asc())
        .load::<DbDelayedEvent>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Count the user's scheduled (not yet finalized) delayed events.
pub async fn count_scheduled(user_id: &UserId) -> DataResult<i64> {
    delayed_events::table
        .filter(delayed_events::user_id.eq(user_id))
        .filter(delayed_events::finalized_at.is_null())
        .count()
        .get_result::<i64>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// The soonest scheduled send time among the user's delayed events, used for
/// the `Retry-After` header when the per-user limit is hit.
pub async fn next_send_at_of_user(user_id: &UserId) -> DataResult<Option<i64>> {
    delayed_events::table
        .filter(delayed_events::user_id.eq(user_id))
        .filter(delayed_events::finalized_at.is_null())
        .select(diesel::dsl::min(delayed_events::send_at))
        .get_result::<Option<i64>>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// The soonest scheduled send time across all users, used by the scheduler to
/// compute how long to sleep.
/// Rows under a live lease are skipped, since the scheduler cannot act on them
/// until the lease expires. Including their already-past `send_at` would make
/// it compute a zero sleep and spin on the database for the whole lease.
pub async fn next_send_at(now: i64) -> DataResult<Option<i64>> {
    delayed_events::table
        .filter(delayed_events::finalized_at.is_null())
        .filter(
            delayed_events::claimed_at
                .is_null()
                .or(delayed_events::claimed_at.lt(now - CLAIM_LEASE_MS)),
        )
        .select(diesel::dsl::min(delayed_events::send_at))
        .get_result::<Option<i64>>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// When the earliest live lease expires, so the scheduler can wake up to
/// reclaim it rather than sleeping through it.
pub async fn next_lease_expiry(now: i64) -> DataResult<Option<i64>> {
    delayed_events::table
        .filter(delayed_events::finalized_at.is_null())
        .filter(delayed_events::claimed_at.ge(now - CLAIM_LEASE_MS))
        .select(diesel::dsl::min(delayed_events::claimed_at))
        .get_result::<Option<i64>>(&mut connect().await?)
        .await
        .map(|claimed| claimed.map(|c| c + CLAIM_LEASE_MS))
        .map_err(Into::into)
}

/// List all delayed events that are due at `now`, in chronological order of
/// their scheduled send times (restart-recovery sends overdue events in this
/// order too).
/// Rows leased by a worker that has not reported an outcome within
/// [`CLAIM_LEASE_MS`] are included again, so a send interrupted by a crash is
/// retried after the server comes back up.
pub async fn list_due(now: i64) -> DataResult<Vec<DbDelayedEvent>> {
    delayed_events::table
        .filter(delayed_events::finalized_at.is_null())
        .filter(delayed_events::send_at.le(now))
        .filter(
            delayed_events::claimed_at
                .is_null()
                .or(delayed_events::claimed_at.lt(now - CLAIM_LEASE_MS)),
        )
        .order(delayed_events::send_at.asc())
        .load::<DbDelayedEvent>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Restart a scheduled delayed event's timer. Returns the updated row, or
/// `None` if the event does not exist, is owned by another user, or was
/// already finalized.
pub async fn restart(
    user_id: &UserId,
    delay_id: &str,
    now: i64,
) -> DataResult<Option<DbDelayedEvent>> {
    diesel::update(
        delayed_events::table
            .filter(delayed_events::user_id.eq(user_id))
            .filter(delayed_events::delay_id.eq(delay_id))
            .filter(delayed_events::finalized_at.is_null())
            // A worker already sending this event cannot be called back, so
            // the restart must fail rather than report a new send time the
            // event will not honour.
            .filter(
                delayed_events::claimed_at
                    .is_null()
                    .or(delayed_events::claimed_at.lt(now - CLAIM_LEASE_MS)),
            ),
    )
    .set((
        delayed_events::running_since.eq(now),
        delayed_events::send_at.eq(delayed_events::delay_ms + now),
        // Invalidate any expired lease, so a worker that outlived it cannot
        // still record an outcome against the row this restart just rescheduled.
        delayed_events::claimed_at.eq(None::<i64>),
    ))
    .get_result::<DbDelayedEvent>(&mut connect().await?)
    .await
    .optional()
    .map_err(Into::into)
}

/// How long a send lease is honoured before another worker may reclaim the
/// row. Only reached when the process died mid-send, so it just has to be
/// comfortably longer than a send takes.
pub const CLAIM_LEASE_MS: i64 = 5 * 60 * 1000;

/// Atomically lease a delayed event for sending.
///
/// Returns the claimed row, or `None` if it was finalized (sent, cancelled or
/// errored) or is already leased by a live worker.
///
/// The lease is recorded in `claimed_at`, leaving `finalized_at` null, so a
/// row whose worker died is still scheduled rather than looking finalized with
/// no outcome. After a successful claim the caller records the outcome with
/// [`set_sent`] or [`set_error`], or releases the lease with [`unclaim`].
///
/// This is the manual `send` action's claim, which deliberately ignores
/// `send_at` — sending ahead of the scheduled time is the point. The scheduler
/// uses [`claim_due`] instead.
pub async fn claim(row_id: i64, now: i64) -> DataResult<Option<DbDelayedEvent>> {
    diesel::update(
        delayed_events::table
            .filter(delayed_events::id.eq(row_id))
            .filter(delayed_events::finalized_at.is_null())
            .filter(
                delayed_events::claimed_at
                    .is_null()
                    .or(delayed_events::claimed_at.lt(now - CLAIM_LEASE_MS)),
            ),
    )
    .set(delayed_events::claimed_at.eq(now))
    .get_result::<DbDelayedEvent>(&mut connect().await?)
    .await
    .optional()
    .map_err(Into::into)
}

/// [`claim`] for the scheduler, additionally requiring the event to still be
/// due at `now`.
///
/// That predicate is what lets a `restart` arriving after `list_due` win the
/// race: it pushes `send_at` into the future, so the scheduler's now-stale
/// entry no longer claims and the event is not sent early.
pub async fn claim_due(row_id: i64, now: i64) -> DataResult<Option<DbDelayedEvent>> {
    diesel::update(
        delayed_events::table
            .filter(delayed_events::id.eq(row_id))
            .filter(delayed_events::finalized_at.is_null())
            .filter(delayed_events::send_at.le(now))
            .filter(
                delayed_events::claimed_at
                    .is_null()
                    .or(delayed_events::claimed_at.lt(now - CLAIM_LEASE_MS)),
            ),
    )
    .set(delayed_events::claimed_at.eq(now))
    .get_result::<DbDelayedEvent>(&mut connect().await?)
    .await
    .optional()
    .map_err(Into::into)
}

/// Record the event id of a claimed delayed event that was sent successfully.
///
/// Fenced on `lease`, the `claimed_at` value this worker set: a worker whose
/// lease was reclaimed while it was still running must not overwrite the
/// outcome the worker that superseded it recorded.
pub async fn set_sent(row_id: i64, lease: i64, event_id: &EventId, now: i64) -> DataResult<()> {
    diesel::update(
        delayed_events::table
            .filter(delayed_events::id.eq(row_id))
            .filter(delayed_events::claimed_at.eq(lease)),
    )
    .set((
        delayed_events::event_id.eq(event_id),
        delayed_events::finalized_at.eq(now),
    ))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Record the error of a claimed delayed event that failed to send. Fenced on
/// the lease, as [`set_sent`] is.
pub async fn set_error(row_id: i64, lease: i64, error: &JsonValue, now: i64) -> DataResult<()> {
    diesel::update(
        delayed_events::table
            .filter(delayed_events::id.eq(row_id))
            .filter(delayed_events::claimed_at.eq(lease)),
    )
    .set((
        delayed_events::error.eq(error),
        delayed_events::finalized_at.eq(now),
    ))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Release a lease so the delayed event stays scheduled (used when a manual
/// `send` action fails before the event could reach the timeline; the MSC
/// requires the event to remain scheduled then). Fenced on the lease so a
/// superseded worker cannot release the lease its successor now holds.
pub async fn unclaim(row_id: i64, lease: i64) -> DataResult<()> {
    diesel::update(
        delayed_events::table
            .filter(delayed_events::id.eq(row_id))
            .filter(delayed_events::claimed_at.eq(lease)),
    )
    .set(delayed_events::claimed_at.eq(None::<i64>))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Cancel a scheduled delayed event. Returns `true` if the event was
/// cancelled, `false` if it did not exist unfinalized (caller decides between
/// idempotent success and conflict from the row's current state).
pub async fn cancel(user_id: &UserId, delay_id: &str, now: i64) -> DataResult<bool> {
    let count = diesel::update(
        delayed_events::table
            .filter(delayed_events::user_id.eq(user_id))
            .filter(delayed_events::delay_id.eq(delay_id))
            .filter(delayed_events::finalized_at.is_null())
            // A row a worker is actively sending must not be cancelled out from
            // under it, or the caller is told the event was cancelled while it
            // is on its way into the room.
            .filter(
                delayed_events::claimed_at
                    .is_null()
                    .or(delayed_events::claimed_at.lt(now - CLAIM_LEASE_MS)),
            ),
    )
    .set((
        delayed_events::finalized_at.eq(now),
        // As in restart: drop the expired lease so its worker is fenced out.
        delayed_events::claimed_at.eq(None::<i64>),
    ))
    .execute(&mut connect().await?)
    .await?;
    Ok(count > 0)
}

/// Delete finalized delayed events whose retention period has passed.
pub async fn prune_finalized(finalized_before: i64) -> DataResult<usize> {
    diesel::delete(
        delayed_events::table
            .filter(delayed_events::finalized_at.is_not_null())
            .filter(delayed_events::finalized_at.le(finalized_before)),
    )
    .execute(&mut connect().await?)
    .await
    .map_err(Into::into)
}
