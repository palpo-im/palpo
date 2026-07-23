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

use salvo::http::StatusCode;
use serde_json::value::to_raw_value;
use tokio::sync::Notify;

use crate::core::client::delayed_events::{
    DelayedEventData, DelayedEventError, SendDelayedEventReqArgs, SendDelayedEventReqBody,
    UpdateAction,
};
use crate::core::error::RetryAfter;
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

static WAKEUP: OnceLock<Notify> = OnceLock::new();

fn wakeup() -> &'static Notify {
    WAKEUP.get_or_init(Notify::new)
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
            let sleep = match delayed_event::next_send_at().await {
                Ok(Some(send_at)) => {
                    Duration::from_millis(send_at.saturating_sub(now) as u64).min(MAX_IDLE)
                }
                Ok(None) => MAX_IDLE,
                Err(error) => {
                    tracing::warn!(?error, "failed to load next delayed event send time");
                    MAX_IDLE
                }
            };
            tokio::select! {
                _ = wakeup().notified() => {},
                _ = tokio::time::sleep(sleep) => {},
                _ = prune.tick() => {
                    let conf = config::get();
                    let cutoff = UnixMillis::now().get() as i64
                        - conf.delayed_events.retention_ms as i64;
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
    let now = UnixMillis::now().get() as i64;
    for event in delayed_event::list_due(now).await? {
        let Some(claimed) = delayed_event::claim(event.id, now).await? else {
            // Finalized concurrently (cancelled or manually sent).
            continue;
        };
        match send_delayed_pdu(&claimed).await {
            Ok(event_id) => {
                delayed_event::set_sent(claimed.id, &event_id).await?;
            }
            Err(error) => {
                tracing::debug!(
                    delay_id = %claimed.delay_id,
                    room_id = %claimed.room_id,
                    ?error,
                    "delayed event failed to send at its scheduled time"
                );
                delayed_event::set_error(claimed.id, &error_body(error)).await?;
            }
        }
    }
    Ok(())
}

/// Build and append the PDU for a claimed delayed event through the normal
/// event authorization and federation paths.
async fn send_delayed_pdu(event: &DbDelayedEvent) -> AppResult<OwnedEventId> {
    let event_type: TimelineEventType = event.event_type.clone().into();
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

    let state_lock = room::lock_state(&event.room_id).await;
    let event_id = timeline::build_and_append_pdu(
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
    drop(state_lock);

    crate::transaction_id::add_txn_id(
        &event.txn_id,
        &event.user_id,
        event.device_id.as_deref(),
        Some(&event.room_id),
        Some(&event_id),
    )
    .await
    .ok();

    Ok((*event_id).to_owned())
}

/// Schedule a new delayed event, enforcing the configured limits. Returns the
/// `delay_id`, reusing the one from a previous identical transaction for
/// idempotency.
pub async fn schedule(
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    is_appservice: bool,
    args: &SendDelayedEventReqArgs,
    body: &SendDelayedEventReqBody,
) -> AppResult<String> {
    let conf = config::get();

    let delay_ms = body.delay.as_millis();
    if delay_ms == 0 {
        return Err(
            MatrixError::invalid_param("delay must be a positive number of milliseconds").into(),
        );
    }
    if delay_ms > conf.delayed_events.max_delay_ms as u128 {
        return Err(MatrixError::forbidden(
            format!(
                "the requested delay exceeds the maximum allowed delay of {} ms",
                conf.delayed_events.max_delay_ms
            ),
            None,
        )
        .into());
    }

    if !body.content.is_object() {
        return Err(MatrixError::bad_json("event content is not an object").into());
    }
    to_canonical_value(&body.content).map_err(|e| {
        MatrixError::bad_json(format!("event content is not valid canonical JSON: {e}"))
    })?;

    // Forbid m.room.encrypted if encryption is disabled, matching /send.
    if args.event_type == TimelineEventType::RoomEncrypted && !conf.allow_encryption {
        return Err(MatrixError::forbidden("Encryption has been disabled", None).into());
    }

    // The room must be known; auth rules themselves are evaluated at send time.
    crate::room::get_version(&args.room_id).await?;

    // Idempotency: same session + transaction id returns the same delay id.
    if let Some(existing) = delayed_event::get_by_txn_id(user_id, device_id, &args.txn_id).await? {
        return Ok(existing.delay_id);
    }

    let scheduled = delayed_event::count_scheduled(user_id).await?;
    if scheduled >= conf.delayed_events.max_scheduled as i64 {
        let retry_after = delayed_event::next_send_at_of_user(user_id)
            .await?
            .map(|send_at| {
                let now = UnixMillis::now().get() as i64;
                RetryAfter::Delay(Duration::from_millis(send_at.saturating_sub(now) as u64))
            });
        return Err(MatrixError::limit_exceeded(
            "The maximum number of delayed events has been reached.",
            retry_after,
        )
        .into());
    }

    let now = UnixMillis::now().get() as i64;
    let new = NewDbDelayedEvent {
        delay_id: utils::random_string(18),
        user_id: user_id.to_owned(),
        device_id: device_id.map(|d| d.to_owned()),
        room_id: args.room_id.clone(),
        event_type: args.event_type.to_string(),
        state_key: body.state_key.clone(),
        content: body.content.clone(),
        delay_ms: delay_ms as i64,
        txn_id: args.txn_id.clone(),
        origin_server_ts: if is_appservice {
            args.timestamp.map(|ts| ts.get() as i64)
        } else {
            None
        },
        running_since: now,
        send_at: now + delay_ms as i64,
        created_at: now,
    };
    let row = delayed_event::create(new).await?;
    wakeup().notify_one();
    Ok(row.delay_id)
}

/// Apply a management action (`restart`/`send`/`cancel`) to a delayed event.
pub async fn update(user_id: &UserId, delay_id: &str, action: &UpdateAction) -> AppResult<()> {
    let Some(event) = delayed_event::get_by_delay_id(user_id, delay_id).await? else {
        return Err(MatrixError::not_found("no delayed event with that delay_id was found").into());
    };
    let now = UnixMillis::now().get() as i64;

    match action {
        UpdateAction::Restart => {
            if delayed_event::restart(user_id, delay_id, now)
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
        UpdateAction::Send => {
            let Some(claimed) = delayed_event::claim(event.id, now).await? else {
                // Already finalized: sending is idempotent if it was sent,
                // conflicting otherwise.
                let refreshed = delayed_event::get_by_delay_id(user_id, delay_id)
                    .await?
                    .unwrap_or(event);
                if refreshed.event_id.is_some() {
                    return Ok(());
                }
                return Err(finalized_conflict(&refreshed, "send"));
            };
            match send_delayed_pdu(&claimed).await {
                Ok(event_id) => {
                    delayed_event::set_sent(claimed.id, &event_id).await?;
                    Ok(())
                }
                Err(error) => {
                    // The MSC requires the event to stay scheduled so the
                    // client can retry until the scheduled send time.
                    delayed_event::unclaim(claimed.id).await?;
                    Err(error)
                }
            }
        }
        UpdateAction::Cancel => {
            if delayed_event::cancel(user_id, delay_id, now).await? {
                Ok(())
            } else {
                let refreshed = delayed_event::get_by_delay_id(user_id, delay_id)
                    .await?
                    .unwrap_or(event);
                if refreshed.event_id.is_some() {
                    Err(finalized_conflict(&refreshed, "cancel"))
                } else {
                    // Already cancelled, either by user action or an error.
                    Ok(())
                }
            }
        }
        _ => Err(MatrixError::invalid_param("unknown delayed event action").into()),
    }
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
            let mut body = serde_json::to_value(&e).unwrap_or_default();
            if let Some(map) = body.as_object_mut() {
                map.insert("errcode".to_owned(), e.kind.code().to_string().into());
            }
            body
        }
        other => serde_json::json!({
            "errcode": "M_UNKNOWN",
            "error": other.to_string(),
        }),
    }
}
