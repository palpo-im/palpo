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

use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use salvo::http::StatusCode;
use serde_json::value::to_raw_value;
use tokio::sync::{Notify, Semaphore};

use crate::core::client::delayed_events::{DelayedEventData, DelayedEventError, UpdateAction};
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
const MAX_IDLE: Duration = Duration::from_secs(60);
/// Avoid a hot loop when another server process holds the earliest due row.
const LOCK_RETRY: Duration = Duration::from_secs(1);

static WAKEUP: OnceLock<Notify> = OnceLock::new();
static OPERATION_GATE: OnceLock<Semaphore> = OnceLock::new();

fn wakeup() -> &'static Notify {
    WAKEUP.get_or_init(Notify::new)
}

/// Bound both delayed appends and row-locking management operations. Appends
/// perform nested queries through the shared pool, while the same limit keeps
/// management requests from opening an unbounded number of dedicated database
/// connections when they wait for an in-flight sender's row lock.
fn operation_gate() -> &'static Semaphore {
    OPERATION_GATE.get_or_init(|| {
        let permits = config::get().db.pool_size.saturating_sub(1).max(1) as usize;
        Semaphore::new(permits)
    })
}

/// Open a connection outside the shared pool for a transaction that holds a
/// delayed-event row lock while the timeline append uses pooled connections.
/// Management actions use the same path, so waiting on that row can never
/// consume the pool capacity needed by the sender that owns it.
async fn dedicated_delayed_event_connection() -> AppResult<AsyncPgConnection> {
    let db_config = config::get().db.clone().into_data_db_config();
    let url = crate::data::connection_url(&db_config, &db_config.url);
    let mut conn = AsyncPgConnection::establish(&url)
        .await
        .map_err(|error| AppError::internal(format!("failed to connect to database: {error}")))?;
    let statement_timeout = db_config.statement_timeout.min(3_600_000);
    diesel::sql_query(format!("SET statement_timeout = {statement_timeout}"))
        .execute(&mut conn)
        .await?;
    Ok(conn)
}

/// Start the background scheduler that sends due delayed events.
pub fn start() {
    tokio::spawn(async move {
        let mut prune = tokio::time::interval(PRUNE_INTERVAL);
        loop {
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
            let sleep = next_wake
                .map(|at| {
                    let until = Duration::from_millis(at.saturating_sub(now) as u64);
                    if until.is_zero() {
                        LOCK_RETRY
                    } else {
                        until.min(MAX_IDLE)
                    }
                })
                .unwrap_or(MAX_IDLE);
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
    let _permit = operation_gate()
        .acquire()
        .await
        .expect("the delayed-event operation semaphore is never closed");
    let mut conn = dedicated_delayed_event_connection().await?;
    conn.transaction::<_, AppError, _>(async |conn| {
        let now = UnixMillis::now().get() as i64;
        let Some(event) = delayed_event::lock_next_due(conn, now).await? else {
            return Ok(false);
        };

        match send_delayed_pdu(&event).await {
            Ok(event_id) => {
                delayed_event::set_sent_locked(
                    conn,
                    event.id,
                    &event_id,
                    UnixMillis::now().get() as i64,
                )
                .await?;
            }
            Err(error) => {
                // `build_and_append_pdu` can report a later delivery or
                // bookkeeping failure after the event has already entered the
                // timeline. The promotion trigger is the authoritative commit
                // point, so do not misclassify that case as a failed send.
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
                        &error_body(error),
                        UnixMillis::now().get() as i64,
                    )
                    .await?;
                }
            }
        }
        Ok(true)
    })
    .await
}

/// Build and append the PDU for a locked delayed event through the normal
/// event authorization and federation paths.
///
/// A send interrupted after the append but before the outcome is recorded
/// leaves the delayed row scheduled. Database triggers track tentative
/// outliers and atomically confirm the output when it enters the timeline, so
/// recovery can replace an abandoned outlier but can never promote a second
/// event for the same delay id.
async fn send_delayed_pdu(event: &DbDelayedEvent) -> AppResult<OwnedEventId> {
    let event_type: TimelineEventType = event.event_type.clone().into();
    let state_lock = room::lock_state(&event.room_id).await;

    // This runs before the state check below: recovering a state event that
    // already reached the room must not re-run authorization, because the
    // event it just sent may itself have changed the state that check reads,
    // which would turn a completed send into a permanent failure.
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
    unsigned.insert("transaction_id".to_owned(), to_raw_value(&event.txn_id)?);

    let event_id = timeline::build_and_append_pdu_force(
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

    // The room must be known; auth rules themselves are evaluated at send time.
    crate::room::get_version(room_id).await?;

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
                .map(|send_at| {
                    RetryAfter::Delay(Duration::from_millis(send_at.saturating_sub(now) as u64))
                });
            Err(MatrixError::limit_exceeded(
                "The maximum number of delayed events has been reached.",
                retry_after,
            )
            .into())
        }
    }
}

/// Apply a management action (`restart`/`send`/`cancel`) to a delayed event.
pub async fn update(user_id: &UserId, delay_id: &str, action: &UpdateAction) -> AppResult<()> {
    let Some(event) = delayed_event::get_by_delay_id(user_id, delay_id).await? else {
        return Err(MatrixError::not_found("no delayed event with that delay_id was found").into());
    };
    match action {
        UpdateAction::Restart => {
            let _permit = operation_gate()
                .acquire()
                .await
                .expect("the delayed-event operation semaphore is never closed");
            let mut conn = dedicated_delayed_event_connection().await?;
            if delayed_event::restart(&mut conn, user_id, delay_id)
                .await?
                .is_some()
            {
                wakeup().notify_one();
                Ok(())
            } else {
                let refreshed = delayed_event::get_by_delay_id(user_id, delay_id)
                    .await?
                    .unwrap_or(event);
                Err(finalized_conflict(&refreshed, "restart"))
            }
        }
        UpdateAction::Send => send_now(user_id, delay_id).await,
        UpdateAction::Cancel => {
            let _permit = operation_gate()
                .acquire()
                .await
                .expect("the delayed-event operation semaphore is never closed");
            let mut conn = dedicated_delayed_event_connection().await?;
            if delayed_event::cancel(&mut conn, user_id, delay_id).await? {
                Ok(())
            } else {
                let refreshed = delayed_event::get_by_delay_id(user_id, delay_id)
                    .await?
                    .unwrap_or(event);
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
    let _permit = operation_gate()
        .acquire()
        .await
        .expect("the delayed-event operation semaphore is never closed");
    let mut conn = dedicated_delayed_event_connection().await?;
    conn.transaction::<_, AppError, _>(async |conn| {
        let Some(event) = delayed_event::lock_for_send(conn, user_id, delay_id).await? else {
            return Err(
                MatrixError::not_found("no delayed event with that delay_id was found").into(),
            );
        };

        if event.finalized_at.is_some() {
            return if event.event_id.is_some() {
                Ok(())
            } else {
                Err(finalized_conflict(&event, "send"))
            };
        }

        match send_delayed_pdu(&event).await {
            Ok(event_id) => {
                delayed_event::set_sent_locked(
                    conn,
                    event.id,
                    &event_id,
                    UnixMillis::now().get() as i64,
                )
                .await?;
                Ok(())
            }
            Err(error) => {
                // The room append may already have committed before a later
                // delivery or bookkeeping step failed. The trigger-backed
                // output is authoritative, just as it is for scheduled sends.
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
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
    })
    .await
}

/// List the user's scheduled delayed events in chronological send order.
pub async fn list(user_id: &UserId) -> AppResult<Vec<DelayedEventData>> {
    Ok(delayed_event::list_scheduled(user_id)
        .await?
        .into_iter()
        .map(to_event_data)
        .collect())
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
    DelayedEventData {
        delay_id: event.delay_id,
        room_id: event.room_id,
        event_type: event.event_type.into(),
        state_key: event.state_key,
        content: event.content,
        delay: Duration::from_millis(event.delay_ms as u64),
        running_since: UnixMillis(event.running_since as u64),
        error: event
            .error
            .and_then(|error| serde_json::from_value::<DelayedEventError>(error).ok()),
        event_id: event.event_id,
        finalized_ts: event.finalized_at.map(|ts| UnixMillis(ts as u64)),
    }
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

    use super::error_body;
    use crate::AppError;
    use crate::core::MatrixError;
    use crate::core::error::RetryAfter;

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
}
