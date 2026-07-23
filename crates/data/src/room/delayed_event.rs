//! Persistence for MSC4140 delayed events.
//!
//! A delayed event is stored when scheduled and stays in the table after it is
//! finalized (sent, cancelled, or errored) so clients can look up the outcome.
//! The scheduler claims due rows by setting `finalized_at` first, then records
//! the send outcome; this keeps the claim atomic under concurrent workers and
//! management requests.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

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

/// Store a newly scheduled delayed event and return the stored row.
pub async fn create(new: NewDbDelayedEvent) -> DataResult<DbDelayedEvent> {
    diesel::insert_into(delayed_events::table)
        .values(&new)
        .get_result::<DbDelayedEvent>(&mut connect().await?)
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

/// List all delayed events that are due at `now`, in chronological order of
/// their scheduled send times (restart-recovery sends overdue events in this
/// order too).
pub async fn list_due(now: i64) -> DataResult<Vec<DbDelayedEvent>> {
    delayed_events::table
        .filter(delayed_events::finalized_at.is_null())
        .filter(delayed_events::send_at.le(now))
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

/// Atomically claim a scheduled delayed event for sending by finalizing it.
/// Returns the claimed row, or `None` if it was already finalized (sent,
/// cancelled, errored, or claimed by a concurrent worker).
///
/// After a successful claim the caller must record the outcome with
/// [`set_sent`] or [`set_error`], or roll the claim back with [`unclaim`].
pub async fn claim(row_id: i64, now: i64) -> DataResult<Option<DbDelayedEvent>> {
    diesel::update(
        delayed_events::table
            .filter(delayed_events::id.eq(row_id))
            .filter(delayed_events::finalized_at.is_null()),
    )
    .set(delayed_events::finalized_at.eq(now))
    .get_result::<DbDelayedEvent>(&mut connect().await?)
    .await
    .optional()
    .map_err(Into::into)
}

/// Record the event id of a claimed delayed event that was sent successfully.
pub async fn set_sent(row_id: i64, event_id: &EventId) -> DataResult<()> {
    diesel::update(delayed_events::table.filter(delayed_events::id.eq(row_id)))
        .set(delayed_events::event_id.eq(event_id))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Record the error of a claimed delayed event that failed to send.
pub async fn set_error(row_id: i64, error: &JsonValue) -> DataResult<()> {
    diesel::update(delayed_events::table.filter(delayed_events::id.eq(row_id)))
        .set(delayed_events::error.eq(error))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Roll back a claim so the delayed event stays scheduled (used when a manual
/// `send` action fails; the MSC requires the event to remain scheduled then).
pub async fn unclaim(row_id: i64) -> DataResult<()> {
    diesel::update(delayed_events::table.filter(delayed_events::id.eq(row_id)))
        .set(delayed_events::finalized_at.eq(None::<i64>))
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
