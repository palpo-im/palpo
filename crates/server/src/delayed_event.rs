//! MSC4140 delayed events.
//!
//! Scheduling, management actions (`restart`/`send`/`cancel`), and the
//! background scheduler that sends events into their room once the delay
//! elapses. Scheduled events are persisted, so pending delayed events survive
//! restarts: on startup the scheduler picks up overdue events and sends them
//! in chronological order of their scheduled send times.
//!
//! Power levels and other auth rules are deliberately evaluated only at the
//! point of sending, as the MSC requires.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use diesel_async::{AsyncConnection, AsyncPgConnection};
use salvo::http::StatusCode;
use serde_json::value::to_raw_value;
use tokio::sync::{Notify, Semaphore};

use crate::core::client::delayed_events::{
    DelayedEventData, DelayedEventError, DelayedEventFinalization, UpdateAction,
};
use crate::core::error::{ErrorKind, RetryAfter};
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::identifiers::*;
use crate::core::serde::{JsonValue, to_canonical_value};
use crate::core::{MatrixError, UnixMillis};
use crate::data::room::delayed_event::{self, DbDelayedEvent, NewDbDelayedEvent};
use crate::room::timeline;
use crate::{AppError, AppResult, PduBuilder, config, room, utils};

/// Retention sweep cadence for finalized delayed events.
const PRUNE_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Upper bound on the scheduler's sleep so newly due work is never missed for
/// long even if a wakeup signal is lost.
const MAX_IDLE: Duration = Duration::from_secs(30);
/// Avoid a hot loop when another server process holds the earliest due row.
const LOCK_RETRY: Duration = Duration::from_secs(1);

static WAKEUP: OnceLock<Notify> = OnceLock::new();
static OPERATION_GATE: OnceLock<Option<Semaphore>> = OnceLock::new();

fn wakeup() -> &'static Notify {
    WAKEUP.get_or_init(Notify::new)
}

/// Bound delayed appends and row-locking management operations below the
/// coordination-pool capacity. One connection always remains available to an
/// ordinary room append which may already hold the process-local state mutex.
fn operation_gate() -> AppResult<&'static Semaphore> {
    OPERATION_GATE
        .get_or_init(|| {
            operation_permits(
                config::get().db.pool_size,
                config::get().db.coordination_pool_size,
            )
            .map(Semaphore::new)
        })
        .as_ref()
        .ok_or_else(|| {
            MatrixError::unknown(
                "delayed event operations require at least two coordination connections",
            )
            .into()
        })
}

fn operation_permits(total_pool_size: u32, configured: Option<u32>) -> Option<usize> {
    crate::data::coordination_pool_capacity(total_pool_size, configured)
        .checked_sub(1)
        .filter(|permits| *permits > 0)
}

fn scheduler_sleep(now: i64, next_wake: Option<i64>) -> Duration {
    next_wake
        .map(|at| {
            let delay_ms = at.saturating_sub(now);
            if delay_ms <= 0 {
                LOCK_RETRY
            } else {
                Duration::from_millis(delay_ms as u64).min(MAX_IDLE)
            }
        })
        .unwrap_or(MAX_IDLE)
}

fn may_be_authorized_without_joining(
    user_id: &UserId,
    event_type: &TimelineEventType,
    state_key: Option<&str>,
) -> bool {
    *event_type == TimelineEventType::RoomMember && state_key == Some(user_id.as_str())
}

/// Start the background scheduler that sends due delayed events.
pub fn start() {
    tokio::spawn(async move {
        let mut prune = tokio::time::interval(PRUNE_INTERVAL);
        loop {
            if let Err(error) = retry_pending_deliveries().await {
                tracing::warn!(?error, "failed to retry delayed-event delivery");
            }
            if let Err(error) = process_due_events().await {
                tracing::warn!(?error, "failed to process due delayed events");
            }

            let now = UnixMillis::now().get() as i64;
            let next_wake = match delayed_event::next_send_at().await {
                Ok(send_at) => send_at,
                Err(error) => {
                    tracing::warn!(?error, "failed to load next delayed event wake-up time");
                    None
                }
            };
            let sleep = scheduler_sleep(now, next_wake);
            tokio::select! {
                _ = wakeup().notified() => {},
                _ = tokio::time::sleep(sleep) => {},
                _ = prune.tick() => {
                    let conf = config::get();
                    let retention_ms = i64::try_from(conf.delayed_events.retention_ms)
                        .unwrap_or(i64::MAX);
                    let cutoff = (UnixMillis::now().get() as i64).saturating_sub(retention_ms);
                    if let Err(error) = delayed_event::prune_finalized(cutoff).await {
                        tracing::warn!(?error, "failed to prune finalized delayed events");
                    }
                },
            }
        }
    });
}

/// Send every delayed event that is due, in chronological order of scheduled
/// send times. Failures are recorded on the event instead of being retried,
/// per the MSC.
async fn process_due_events() -> AppResult<()> {
    while process_one_due_event().await? {}
    Ok(())
}

/// Lock, send, and finalize one due row in a single database transaction.
async fn process_one_due_event() -> AppResult<bool> {
    let _permit = operation_gate()?
        .acquire()
        .await
        .expect("the delayed-event operation semaphore is never closed");
    let mut conn = crate::data::coordination_connect().await?;
    let (processed, delivery) = conn
        .transaction::<_, AppError, _>(async |conn| {
            let now = UnixMillis::now().get() as i64;
            let Some(event) = delayed_event::lock_next_due(conn, now).await? else {
                return Ok((false, None));
            };
            let delivery = match send_delayed_pdu(conn, &event).await {
                Ok(event_id) => {
                    delayed_event::set_sent_locked(
                        conn,
                        event.id,
                        &event_id,
                        UnixMillis::now().get() as i64,
                    )
                    .await?;
                    Some((event.room_id.clone(), event_id))
                }
                Err(error) => {
                    // A previous worker may have stopped after the event entered the
                    // timeline but before it finalized this delayed row. The promotion
                    // trigger is the authoritative commit point, so do not misclassify
                    // that recovered case as a failed send.
                    if let Some(event_id) = delayed_event::get_output(&event.delay_id).await? {
                        tracing::warn!(
                            delay_id = %event.delay_id,
                            room_id = %event.room_id,
                            %event_id,
                            ?error,
                            "delayed event entered the timeline before a later send step failed"
                        );
                        delayed_event::set_sent_locked(
                            conn,
                            event.id,
                            &event_id,
                            UnixMillis::now().get() as i64,
                        )
                        .await?;
                        Some((event.room_id.clone(), event_id))
                    } else {
                        tracing::debug!(
                            delay_id = %event.delay_id,
                            room_id = %event.room_id,
                            ?error,
                            "delayed event failed to send at its scheduled time"
                        );
                        // The MSC says a scheduled send is not retried. The error
                        // is committed under the same row lock that fenced the
                        // append.
                        delayed_event::set_error_locked(
                            conn,
                            event.id,
                            &event.delay_id,
                            &error_body(error),
                            UnixMillis::now().get() as i64,
                        )
                        .await?;
                        None
                    }
                }
            };
            Ok((true, delivery))
        })
        .await?;

    drop(conn);
    drop(_permit);
    if let Some((room_id, event_id)) = delivery {
        deliver_after_commit(&room_id, &event_id).await;
    }
    Ok(processed)
}

async fn retry_pending_deliveries() -> AppResult<()> {
    let mut after: Option<OwnedEventId> = None;
    loop {
        let pending = delayed_event::pending_deliveries(after.as_deref()).await?;
        if pending.is_empty() {
            break;
        }
        after = pending.last().map(|(id, _)| id.clone());
        for (event_id, room_id) in pending {
            deliver_after_commit(&room_id, &event_id).await;
        }
    }
    Ok(())
}

async fn queue_committed_delivery(room_id: &RoomId, event_id: &EventId) -> AppResult<()> {
    let mut conn = crate::data::coordination_connect().await?;
    conn.transaction::<_, AppError, _>(async |conn| {
        if !delayed_event::lock_delivery(conn, event_id).await? {
            return Ok(());
        }
        // A crash after enqueueing but before deleting the marker can repeat an
        // event ID, which federation tolerates. Never delete before durable enqueue.
        timeline::deliver_local_pdu(room_id, event_id).await?;
        delayed_event::finish_delivery(conn, event_id).await?;
        Ok(())
    })
    .await
}

async fn deliver_after_commit(room_id: &RoomId, event_id: &EventId) {
    if let Err(error) = queue_committed_delivery(room_id, event_id).await {
        tracing::warn!(%room_id, %event_id, ?error,
            "delayed-event delivery remains queued for retry");
    }
}

/// Build and append the PDU for a locked delayed event through the normal
/// event authorization and timeline path. The caller queues federation delivery
/// only after the surrounding row-locking transaction commits.
///
/// A send interrupted after the append but before the outcome is recorded
/// leaves the delayed row scheduled. Database triggers track tentative
/// outliers and atomically confirm the output when it enters the timeline, so
/// recovery can replace an abandoned outlier but can never promote a second
/// event for the same delay id.
async fn send_delayed_pdu(
    conn: &mut AsyncPgConnection,
    event: &DbDelayedEvent,
) -> AppResult<OwnedEventId> {
    let event_type: TimelineEventType = event.event_type.clone().into();

    // This runs before either room lock or the state check below: recovering an
    // event that already reached the room must not wait behind unrelated room
    // work or re-run authorization. The event it just sent may itself have
    // changed the state that check reads, which would otherwise turn a
    // completed send into a permanent failure.
    // Use the delay-specific mapping: a reused transaction id can point at
    // an older ordinary send, while this marker identifies the exact delayed
    // event that actually entered the timeline.
    if let Some(event_id) = delayed_event::get_output(&event.delay_id).await? {
        // Repair the conventional transaction-id lookup when possible. The
        // trigger-backed output is already a sufficient idempotency fence, so
        // failure to write this secondary mapping must not turn a completed
        // room append into a failed delayed event.
        if let Err(error) = crate::transaction_id::add_txn_id(
            &event.txn_id,
            &event.user_id,
            event.device_id.as_deref(),
            Some(&event.room_id),
            Some(&event_id),
        )
        .await
        {
            tracing::warn!(
                delay_id = %event.delay_id,
                %event_id,
                ?error,
                "failed to repair delayed-event transaction-id mapping"
            );
        }
        return Ok(event_id);
    }

    // Delayed requests pass the HTTP limiter when they are scheduled, but the
    // MSC also requires protection at the point the event enters the DAG.
    crate::hoops::limit_delayed_message(&event.user_id)?;

    // All local append paths take the process-local state lock before the database
    // fence. Keeping one global order avoids deadlocking an ordinary local send with
    // a delayed worker in another process.
    let state_lock = room::lock_state(&event.room_id).await;
    crate::data::room::timeline::lock_event_append(conn, &event.room_id).await?;

    // A delayed send does not pass through the access-token hoop again. Apply
    // the same current account-usability policy explicitly so deactivated,
    // locked, or suspended users cannot emit previously queued events.
    let user = crate::data::user::get_user(&event.user_id).await?;
    crate::user::ensure_account_usable(&user)?;

    // Re-evaluate server-side send policy as well as room authorization. An
    // event scheduled while encryption was enabled must not bypass a later
    // administrator decision to disable encrypted messages. This belongs
    // after output recovery so an event that already entered the timeline is
    // still finalized correctly.
    if event_type == TimelineEventType::RoomEncrypted && !config::get().allow_encryption {
        return Err(MatrixError::forbidden("Encryption has been disabled", None).into());
    }
    if let Some(state_key) = &event.state_key {
        let state_event_type: StateEventType = event.event_type.clone().into();
        crate::state::allowed_to_send_state_event(
            &event.room_id,
            &state_event_type,
            state_key,
            &serde_json::from_value(event.content.clone())?,
        )
        .await?;
    }

    let mut unsigned = BTreeMap::new();
    unsigned.insert(
        "org.matrix.msc4140.delay_id".to_owned(),
        to_raw_value(&event.delay_id)?,
    );

    let event_id = timeline::build_and_append_pdu_force_locked(
        PduBuilder {
            event_type,
            content: to_raw_value(&event.content)?,
            unsigned,
            state_key: event.state_key.clone(),
            redacts: None,
            timestamp: event.origin_server_ts.map(|ts| UnixMillis(ts as u64)),
        },
        &event.user_id,
        &event.room_id,
        &crate::room::get_version(&event.room_id).await?,
        &state_lock,
    )
    .await?
    .pdu
    .event_id;

    // The database trigger has already recorded the authoritative output in
    // the same transaction that promoted the event into the timeline. Keep the
    // standard transaction-id mapping for normal idempotency lookups, but do
    // not misreport a completed room append if this secondary write fails.
    if let Err(error) = crate::transaction_id::add_txn_id(
        &event.txn_id,
        &event.user_id,
        event.device_id.as_deref(),
        Some(&event.room_id),
        Some(&event_id),
    )
    .await
    {
        tracing::warn!(
            delay_id = %event.delay_id,
            event_id = %event_id,
            ?error,
            "failed to record delayed-event transaction-id mapping"
        );
    }
    drop(state_lock);

    Ok((*event_id).to_owned())
}

/// Schedule a new delayed event, enforcing the configured limits. Returns the
/// `delay_id`, reusing the one from a previous identical transaction for
/// idempotency.
pub async fn schedule(
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    is_appservice: bool,
    room_id: &RoomId,
    event_type: &TimelineEventType,
    txn_id: &TransactionId,
    timestamp: Option<UnixMillis>,
    delay: Duration,
    state_key: Option<String>,
    content: JsonValue,
) -> AppResult<String> {
    let conf = config::get();
    let now = UnixMillis::now().get() as i64;

    // An already accepted transaction stays idempotent even if server limits,
    // room state, or feature-related configuration changed since the original
    // request. `create` repeats this lookup under its advisory lock to close
    // the concurrent-first-request race.
    if let Some(existing) = delayed_event::get_by_txn_id(user_id, device_id, txn_id).await? {
        return Ok(existing.delay_id);
    }

    if !conf.delayed_events.enable {
        return Err(MatrixError::forbidden("MSC4140 delayed events are disabled", None).into());
    }

    let requested_delay_ms = delay.as_millis();
    if requested_delay_ms == 0 {
        return Err(
            MatrixError::invalid_param("delay must be a positive number of milliseconds").into(),
        );
    }
    let max_delay_ms =
        u128::from(conf.delayed_events.max_delay_ms).min(i64::MAX.saturating_sub(now) as u128);
    if requested_delay_ms > max_delay_ms {
        return Err(MatrixError::delay_too_large(format!(
            "the requested delay exceeds the maximum allowed delay of {} ms",
            max_delay_ms
        ))
        .into());
    }
    let delay_ms = i64::try_from(requested_delay_ms)
        .map_err(|_| MatrixError::invalid_param("delay is too large"))?;

    if !content.is_object() {
        return Err(MatrixError::bad_json("event content is not an object").into());
    }
    to_canonical_value(&content).map_err(|e| {
        MatrixError::bad_json(format!("event content is not valid canonical JSON: {e}"))
    })?;

    // Forbid m.room.encrypted if encryption is disabled, matching /send.
    if event_type == &TimelineEventType::RoomEncrypted && !conf.allow_encryption {
        return Err(MatrixError::forbidden("Encryption has been disabled", None).into());
    }

    // Reject work that already has no chance of succeeding. Authorization is
    // still evaluated again against the then-current room state when the event
    // is actually sent.
    crate::room::get_version(room_id).await?;
    if !crate::room::user::is_joined(user_id, room_id).await?
        && !may_be_authorized_without_joining(user_id, event_type, state_key.as_deref())
    {
        return Err(MatrixError::forbidden(
            "you must be joined to the room to schedule a delayed event",
            None,
        )
        .into());
    }
    if let Some(state_key) = &state_key {
        let state_event_type: StateEventType = event_type.to_string().into();
        crate::state::allowed_to_send_state_event(
            room_id,
            &state_event_type,
            state_key,
            &serde_json::from_value(content.clone())?,
        )
        .await?;
    }

    let origin_server_ts = if is_appservice {
        timestamp
            .map(|ts| {
                i64::try_from(ts.get())
                    .map_err(|_| MatrixError::invalid_param("timestamp is too large"))
            })
            .transpose()?
    } else {
        None
    };
    let new = NewDbDelayedEvent {
        delay_id: utils::random_string(18),
        user_id: user_id.to_owned(),
        device_id: device_id.map(|d| d.to_owned()),
        room_id: room_id.to_owned(),
        event_type: event_type.to_string(),
        state_key,
        content,
        delay_ms,
        txn_id: txn_id.to_owned(),
        origin_server_ts,
        running_since: now,
        send_at: now + delay_ms,
        created_at: now,
    };
    // The limit is enforced inside the same transaction as the insert, so
    // concurrent requests cannot each observe a count below it and all succeed.
    let max_scheduled = i64::try_from(conf.delayed_events.max_scheduled).unwrap_or(i64::MAX);
    match delayed_event::create(new, max_scheduled).await? {
        delayed_event::Scheduled::Created(row) => {
            wakeup().notify_one();
            Ok(row.delay_id)
        }
        // A concurrent retry of this same transaction already scheduled it.
        delayed_event::Scheduled::AlreadyScheduled(row) => Ok(row.delay_id),
        delayed_event::Scheduled::LimitReached => {
            let retry_after = delayed_event::next_send_at_of_user(user_id)
                .await?
                .map(|send_at| RetryAfter::Delay(retry_after_delay(now, send_at)));
            Err(MatrixError::limit_exceeded(
                "The maximum number of delayed events has been reached.",
                retry_after,
            )
            .into())
        }
    }
}

fn retry_after_delay(now: i64, send_at: i64) -> Duration {
    Duration::from_millis(send_at.saturating_sub(now).max(0) as u64)
}

/// Apply a management action (`restart`/`send`/`cancel`) to a delayed event.
pub async fn update(user_id: &UserId, delay_id: &str, action: &UpdateAction) -> AppResult<()> {
    if delayed_event::get_by_delay_id(user_id, delay_id)
        .await?
        .is_none()
    {
        return Err(MatrixError::not_found("no delayed event with that delay_id was found").into());
    }
    match action {
        UpdateAction::Restart => {
            let _permit = operation_gate()?
                .acquire()
                .await
                .expect("the delayed-event operation semaphore is never closed");
            let mut conn = crate::data::coordination_connect().await?;
            if delayed_event::restart(&mut conn, user_id, delay_id)
                .await?
                .is_some()
            {
                wakeup().notify_one();
                Ok(())
            } else {
                let refreshed = delayed_event::get_by_delay_id(user_id, delay_id)
                    .await?
                    .ok_or_else(|| {
                        MatrixError::not_found("no delayed event with that delay_id was found")
                    })?;
                Err(finalized_conflict(&refreshed, "restart"))
            }
        }
        UpdateAction::Send => send_now(user_id, delay_id).await,
        UpdateAction::Cancel => {
            let _permit = operation_gate()?
                .acquire()
                .await
                .expect("the delayed-event operation semaphore is never closed");
            let mut conn = crate::data::coordination_connect().await?;
            if delayed_event::cancel(&mut conn, user_id, delay_id).await? {
                Ok(())
            } else {
                let refreshed = delayed_event::get_by_delay_id(user_id, delay_id)
                    .await?
                    .ok_or_else(|| {
                        MatrixError::not_found("no delayed event with that delay_id was found")
                    })?;
                if refreshed.event_id.is_some() {
                    Err(finalized_conflict(&refreshed, "cancel"))
                } else {
                    // The MSC treats cancelling an event that was already
                    // cancelled -- "either due to user action or an error" --
                    // as an idempotent success.
                    Ok(())
                }
            }
        }
        _ => Err(MatrixError::invalid_param("unknown delayed event action").into()),
    }
}

/// Manually send one row while holding its database lock through the append.
async fn send_now(user_id: &UserId, delay_id: &str) -> AppResult<()> {
    let _permit = operation_gate()?
        .acquire()
        .await
        .expect("the delayed-event operation semaphore is never closed");
    let mut conn = crate::data::coordination_connect().await?;
    let delivery = conn
        .transaction::<_, AppError, _>(async |conn| {
            let Some(event) = delayed_event::lock_for_send(conn, user_id, delay_id).await? else {
                return Err(
                    MatrixError::not_found("no delayed event with that delay_id was found").into(),
                );
            };

            if event.finalized_at.is_some() {
                return if event.event_id.is_some() {
                    Ok(None)
                } else {
                    Err(finalized_conflict(&event, "send"))
                };
            }
            match send_delayed_pdu(conn, &event).await {
                Ok(event_id) => {
                    delayed_event::set_sent_locked(
                        conn,
                        event.id,
                        &event_id,
                        UnixMillis::now().get() as i64,
                    )
                    .await?;
                    Ok(Some((event.room_id, event_id)))
                }
                Err(error) => {
                    // A previous manual send may have committed the room append before
                    // it finalized this delayed row. The trigger-backed output is
                    // authoritative, just as it is for scheduled sends.
                    if let Some(event_id) = delayed_event::get_output(&event.delay_id).await? {
                        tracing::warn!(
                            delay_id = %event.delay_id,
                            room_id = %event.room_id,
                            %event_id,
                            ?error,
                            "manually sent delayed event entered the timeline before a later step failed"
                        );
                        delayed_event::set_sent_locked(
                            conn,
                            event.id,
                            &event_id,
                            UnixMillis::now().get() as i64,
                        )
                        .await?;
                        Ok(Some((event.room_id, event_id)))
                    } else {
                        Err(error)
                    }
                }
            }
        })
        .await?;

    drop(conn);
    drop(_permit);
    if let Some((room_id, event_id)) = delivery {
        deliver_after_commit(&room_id, &event_id).await;
    }
    Ok(())
}

/// Fetch one delayed event owned by the user, whether scheduled or finalized.
pub async fn get(user_id: &UserId, delay_id: &str) -> AppResult<DelayedEventData> {
    delayed_event::get_by_delay_id(user_id, delay_id)
        .await?
        .map(to_event_data)
        .ok_or_else(|| {
            MatrixError::not_found("no delayed event with that delay_id was found").into()
        })
}

fn to_event_data(event: DbDelayedEvent) -> DelayedEventData {
    let event_id = event.event_id.clone();
    let error = if event_id.is_none() {
        event
            .error
            .clone()
            .and_then(|error| serde_json::from_value::<DelayedEventError>(error).ok())
    } else {
        None
    };
    let finalised = event
        .finalized_at
        .map(|finalized_at| DelayedEventFinalization {
            error,
            event_id,
            finalised_ts: unix_millis(finalized_at),
        });

    DelayedEventData {
        delay_id: event.delay_id,
        room_id: event.room_id,
        event_type: event.event_type.into(),
        state_key: event.state_key,
        content: event.content,
        delay: Duration::from_millis(u64::try_from(event.delay_ms).unwrap_or_default()),
        running_since: unix_millis(event.running_since),
        finalised,
    }
}

fn unix_millis(value: i64) -> UnixMillis {
    UnixMillis(u64::try_from(value).unwrap_or_default())
}

/// HTTP 409 for a management action that conflicts with the outcome the
/// delayed event was already finalized with.
fn finalized_conflict(event: &DbDelayedEvent, action: &str) -> AppError {
    let outcome = if event.event_id.is_some() {
        "already been sent"
    } else if event.error.is_some() {
        "already failed to send"
    } else {
        "already been cancelled"
    };
    let mut error = MatrixError::unknown(format!(
        "cannot {action} a delayed event that has {outcome}"
    ));
    error.status_code = Some(StatusCode::CONFLICT);
    error.into()
}

/// The standard error body stored for a delayed event that failed to send.
fn error_body(error: AppError) -> JsonValue {
    match error {
        AppError::Matrix(e) => {
            let retry_after = match &e.kind {
                ErrorKind::LimitExceeded {
                    retry_after: Some(RetryAfter::Delay(duration)),
                } => Some(*duration),
                _ => None,
            };
            let mut body = serde_json::to_value(&e).unwrap_or_default();
            if let Some(map) = body.as_object_mut() {
                map.insert("errcode".to_owned(), e.kind.code().to_string().into());
                if let Some(duration) = retry_after
                    && let Ok(ms) = u64::try_from(duration.as_millis())
                {
                    map.insert("retry_after_ms".to_owned(), ms.into());
                }
            }
            body
        }
        _ => serde_json::json!({
            "errcode": "M_UNKNOWN",
            "error": "internal server error",
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        LOCK_RETRY, MAX_IDLE, error_body, may_be_authorized_without_joining, operation_permits,
        retry_after_delay, scheduler_sleep,
    };
    use crate::AppError;
    use crate::core::MatrixError;
    use crate::core::error::RetryAfter;
    use crate::core::events::TimelineEventType;

    #[test]
    fn delayed_event_internal_errors_do_not_expose_details() {
        let body = error_body(AppError::internal("database secret path"));

        assert_eq!(body["errcode"], "M_UNKNOWN");
        assert_eq!(body["error"], "internal server error");
        assert!(!body.to_string().contains("database secret path"));
    }

    #[test]
    fn delayed_event_rate_limit_errors_keep_retry_delay() {
        let body = error_body(
            MatrixError::limit_exceeded(
                "slow down",
                Some(RetryAfter::Delay(Duration::from_millis(1500))),
            )
            .into(),
        );

        assert_eq!(body["errcode"], "M_LIMIT_EXCEEDED");
        assert_eq!(body["retry_after_ms"], 1500);
    }

    #[test]
    fn scheduler_retries_past_wakeups_without_unsigned_wraparound() {
        assert_eq!(scheduler_sleep(1_000, Some(999)), LOCK_RETRY);
        assert_eq!(scheduler_sleep(1_000, Some(1_000)), LOCK_RETRY);
        assert_eq!(
            scheduler_sleep(1_000, Some(1_250)),
            Duration::from_millis(250)
        );
        assert_eq!(scheduler_sleep(1_000, None), MAX_IDLE);
        assert_eq!(scheduler_sleep(1_000, Some(i64::MAX)), MAX_IDLE);
    }

    #[test]
    fn overdue_limit_retry_does_not_wrap_to_a_huge_duration() {
        assert_eq!(retry_after_delay(1_000, 999), Duration::ZERO);
        assert_eq!(retry_after_delay(1_000, 1_000), Duration::ZERO);
        assert_eq!(retry_after_delay(1_000, 1_250), Duration::from_millis(250));
        assert_eq!(retry_after_delay(i64::MAX, i64::MIN), Duration::ZERO);
    }

    #[test]
    fn delayed_operations_leave_one_coordination_connection_free() {
        assert_eq!(operation_permits(4, Some(2)), Some(1));
        assert_eq!(operation_permits(10, None), Some(1));
        assert_eq!(operation_permits(64, None), Some(15));
    }

    #[test]
    fn delayed_operations_reject_a_single_coordination_connection() {
        assert_eq!(operation_permits(3, None), None);
        assert_eq!(operation_permits(4, None), None);
    }

    #[test]
    fn only_self_membership_may_be_authorized_before_joining() {
        let user: crate::core::OwnedUserId = "@alice:example.org".try_into().unwrap();
        assert!(may_be_authorized_without_joining(
            &user,
            &TimelineEventType::RoomMember,
            Some(user.as_str())
        ));
        assert!(!may_be_authorized_without_joining(
            &user,
            &TimelineEventType::RoomMember,
            Some("@bob:example.org")
        ));
        assert!(!may_be_authorized_without_joining(
            &user,
            &TimelineEventType::RoomMessage,
            None
        ));
    }
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_delayed_delivery_survives_rollback_and_retention() {
        use super::*;
        crate::test_database::init();
        let new = NewDbDelayedEvent {
            delay_id: "delivery-regression".into(),
            user_id: "@delayed:example.org".try_into().unwrap(),
            device_id: Some("PHONE".into()),
            room_id: "!delayed:example.org".try_into().unwrap(),
            event_type: "m.room.message".into(),
            state_key: None,
            content: serde_json::json!({}),
            delay_ms: 1000,
            txn_id: "delivery-txn".into(),
            origin_server_ts: None,
            running_since: 1,
            send_at: 1001,
            created_at: 1,
        };
        let delayed_event::Scheduled::Created(row) = delayed_event::create(new, 10).await.unwrap()
        else {
            panic!("new row")
        };
        let event = EventId::parse("$delayed:example.org").unwrap();
        let mut conn = crate::data::connect().await.unwrap();
        let failed = conn
            .transaction::<(), AppError, _>(async |conn| {
                delayed_event::set_sent_locked(conn, row.id, &event, 2000).await?;
                Err(AppError::internal(
                    "simulate crash before completion commit",
                ))
            })
            .await;
        assert!(failed.is_err());
        assert!(
            delayed_event::get_by_delay_id(&row.user_id, &row.delay_id)
                .await
                .unwrap()
                .unwrap()
                .finalized_at
                .is_none()
        );
        assert!(
            delayed_event::pending_deliveries(None)
                .await
                .unwrap()
                .is_empty()
        );
        conn.transaction::<(), AppError, _>(async |conn| {
            delayed_event::set_sent_locked(conn, row.id, &event, 2000).await?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(delayed_event::prune_finalized(i64::MAX).await.unwrap(), 0);
        let failed = conn
            .transaction::<(), AppError, _>(async |conn| {
                assert!(delayed_event::lock_delivery(conn, &event).await?);
                delayed_event::finish_delivery(conn, &event).await?;
                Err(AppError::internal(
                    "simulate crash before delivery acknowledgement",
                ))
            })
            .await;
        assert!(failed.is_err());
        assert_eq!(
            delayed_event::pending_deliveries(None).await.unwrap().len(),
            1
        );
        conn.transaction::<(), AppError, _>(async |conn| {
            assert!(delayed_event::lock_delivery(conn, &event).await?);
            delayed_event::finish_delivery(conn, &event).await?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(delayed_event::prune_finalized(i64::MAX).await.unwrap(), 1);
    }
}
