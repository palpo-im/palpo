//! Persistence for MSC4140 delayed events.
//!
//! A delayed event is stored when scheduled and stays in the table after it is
//! finalized (sent, cancelled, or errored) so clients can look up the outcome.
//! A sender holds a PostgreSQL row lock from selection through the room append
//! and outcome write. A worker crash rolls that transaction back, leaving the
//! event scheduled for another worker without a time-based lease race.

use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

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
pub async fn next_send_at() -> DataResult<Option<i64>> {
    delayed_events::table
        .filter(delayed_events::finalized_at.is_null())
        .select(diesel::dsl::min(delayed_events::send_at))
        .get_result::<Option<i64>>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Lock the next due event without waiting for another worker's row.
///
/// The caller owns the surrounding transaction and must keep it open through
/// the append and outcome write. `SKIP LOCKED` lets multiple server processes
/// work on different rows while guaranteeing that only one can append a given
/// delayed event.
pub async fn lock_next_due(
    conn: &mut AsyncPgConnection,
    now: i64,
) -> DataResult<Option<DbDelayedEvent>> {
    delayed_events::table
        .filter(delayed_events::finalized_at.is_null())
        .filter(delayed_events::send_at.le(now))
        .order((delayed_events::send_at.asc(), delayed_events::id.asc()))
        .for_update()
        .skip_locked()
        .first::<DbDelayedEvent>(conn)
        .await
        .optional()
        .map_err(Into::into)
}

/// Lock one user's delayed event for a manual send.
///
/// Unlike the scheduler this deliberately waits for an existing row holder:
/// once the lock is acquired the caller can return the definitive sent/error
/// result rather than guessing from an expiring lease.
pub async fn lock_for_send(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    delay_id: &str,
) -> DataResult<Option<DbDelayedEvent>> {
    delayed_events::table
        .filter(delayed_events::user_id.eq(user_id))
        .filter(delayed_events::delay_id.eq(delay_id))
        .for_update()
        .first::<DbDelayedEvent>(conn)
        .await
        .optional()
        .map_err(Into::into)
}

#[derive(QueryableByName)]
struct DelayedEventOutput {
    #[diesel(sql_type = diesel::sql_types::Text)]
    event_id: String,
}

/// Find the event atomically recorded when an outlier enters the timeline.
///
/// This is the durable recovery fence for a process that dies after appending
/// the room event but before finalizing the delayed-event row or recording its
/// regular transaction-id mapping.
pub async fn get_output(delay_id: &str) -> DataResult<Option<OwnedEventId>> {
    let row = diesel::sql_query(
        "SELECT output.event_id \
         FROM delayed_event_outputs AS output \
         INNER JOIN events ON events.id = output.event_id \
         WHERE output.delay_id = $1 AND events.is_outlier = FALSE",
    )
    .bind::<diesel::sql_types::Text, _>(delay_id)
    .get_result::<DelayedEventOutput>(&mut connect().await?)
    .await
    .optional()?;

    row.map(|row| OwnedEventId::try_from(row.event_id).map_err(Into::into))
        .transpose()
}

/// Restart a scheduled delayed event's timer. Returns the updated row, or
/// `None` if the event does not exist, is owned by another user, or was
/// already finalized.
pub async fn restart(
    user_id: &UserId,
    delay_id: &str,
    now: i64,
) -> DataResult<Option<DbDelayedEvent>> {
    // If a sender holds this row, PostgreSQL waits and then re-checks
    // `finalized_at` against the committed outcome.
    diesel::update(
        delayed_events::table
            .filter(delayed_events::user_id.eq(user_id))
            .filter(delayed_events::delay_id.eq(delay_id))
            .filter(delayed_events::finalized_at.is_null()),
    )
    .set((
        delayed_events::running_since.eq(now),
        delayed_events::send_at.eq(delayed_events::delay_ms + now),
    ))
    .get_result::<DbDelayedEvent>(&mut connect().await?)
    .await
    .optional()
    .map_err(Into::into)
}

/// Record the successful outcome while the caller still holds the row lock.
pub async fn set_sent_locked(
    conn: &mut AsyncPgConnection,
    row_id: i64,
    event_id: &EventId,
    now: i64,
) -> DataResult<()> {
    diesel::update(delayed_events::table.find(row_id))
        .set((
            delayed_events::event_id.eq(event_id),
            delayed_events::finalized_at.eq(now),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

/// Record a scheduled-send failure while the caller still holds the row lock.
pub async fn set_error_locked(
    conn: &mut AsyncPgConnection,
    row_id: i64,
    error: &JsonValue,
    now: i64,
) -> DataResult<()> {
    diesel::update(delayed_events::table.find(row_id))
        .set((
            delayed_events::error.eq(error),
            delayed_events::finalized_at.eq(now),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

/// Cancel a scheduled delayed event. Returns `true` if the event was
/// cancelled, `false` if it did not exist unfinalized (caller decides between
/// idempotent success and conflict from the row's current state).
pub async fn cancel(user_id: &UserId, delay_id: &str, now: i64) -> DataResult<bool> {
    // If a sender holds this row, PostgreSQL waits and then re-checks
    // `finalized_at` against the committed outcome.
    let count = diesel::update(
        delayed_events::table
            .filter(delayed_events::user_id.eq(user_id))
            .filter(delayed_events::delay_id.eq(delay_id))
            .filter(delayed_events::finalized_at.is_null()),
    )
    .set(delayed_events::finalized_at.eq(now))
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
