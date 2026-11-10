#[cfg(feature = "unstable-msc4495")]
use std::collections::BTreeSet;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::pending;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::mpsc;

use super::{
    EduBuf, EduVec, MPSC_SENDER, OutgoingKind, SELECT_EDU_LIMIT, SELECT_PRESENCE_LIMIT,
    SELECT_RECEIPT_LIMIT, SendingEventType, TransactionStatus,
};
use crate::core::device::DeviceListUpdateContent;
use crate::core::events::receipt::{ReceiptContent, ReceiptData, ReceiptMap, ReceiptType};
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::core::presence::{PresenceContent, PresenceUpdate};
use crate::core::{Seqnum, device_id};
use crate::exts::*;
use crate::room::state;
use crate::{AppResult, data, room};

/// How often the guard scans the database for queued requests whose wakeup
/// was dropped (full wakeup queue) or that were left over by a previous
/// process.
const SWEEP_INTERVAL: Duration = Duration::from_secs(30);
const WORKER_RESTART_DELAY: Duration = Duration::from_secs(5);
static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
static WORKER_READY: AtomicBool = AtomicBool::new(false);
static WORKER_RESTARTS: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[path = "readiness_tests.rs"]
mod readiness_tests;

struct RunningWorker;

impl Drop for RunningWorker {
    fn drop(&mut self) {
        WORKER_READY.store(false, Ordering::Release);
        WORKER_RUNNING.store(false, Ordering::Release);
    }
}

/// Whether the sender is processing work rather than recovering from failure.
pub fn is_running() -> bool {
    WORKER_RUNNING.load(Ordering::Acquire)
}

/// Whether durable startup recovery completed and the scheduler's latest
/// queue sweep succeeded. Destination-specific delivery failures do not make
/// the whole server unready: they retain durable retries instead.
pub fn is_ready() -> bool {
    is_running() && WORKER_READY.load(Ordering::Acquire)
}

pub fn restart_count() -> u64 {
    WORKER_RESTARTS.load(Ordering::Relaxed)
}

/// A configured sending guard whose worker has not been started yet.
///
/// Keeping initialization separate from `start` lets startup admin commands
/// enqueue durable requests without dispatching network traffic in
/// `--server false` mode.
#[must_use = "call Guard::start when the server is enabled"]
pub struct Guard {
    receiver: mpsc::Receiver<super::WakeupMessage>,
}

pub fn init() -> Guard {
    let (sender, receiver) = mpsc::channel(super::WAKEUP_QUEUE_CAPACITY);
    assert!(
        MPSC_SENDER.set(sender).is_ok(),
        "sending guard is already initialized"
    );
    Guard { receiver }
}

impl Guard {
    pub fn start(self) {
        tokio::spawn(supervise(self.receiver, WORKER_RESTART_DELAY));
    }
}

async fn supervise(mut receiver: mpsc::Receiver<super::WakeupMessage>, delay: Duration) {
    loop {
        WORKER_READY.store(false, Ordering::Release);
        WORKER_RUNNING.store(true, Ordering::Release);
        let running = RunningWorker;
        let result = AssertUnwindSafe(process(&mut receiver))
            .catch_unwind()
            .await;
        drop(running);
        match result {
            Ok(Err(error)) => error!(
                ?error,
                "sending guard failed; recovering from durable queue"
            ),
            Err(_) => error!("sending guard panicked; recovering from durable queue"),
            Ok(Ok(())) => {
                error!("sending guard exited unexpectedly; recovering from durable queue")
            }
        }
        WORKER_RESTARTS.fetch_add(1, Ordering::Relaxed);
        // Keep the receiver open throughout recovery. Outstanding send futures
        // have been dropped; their unacknowledged rows remain pending and are
        // loaded again at startup. Never clear or acknowledge them on failure.
        tokio::time::sleep(delay).await;
    }
}

async fn process(receiver: &mut mpsc::Receiver<super::WakeupMessage>) -> AppResult<()> {
    let mut futures = FuturesUnordered::new();
    let mut current_transaction_status = HashMap::<OutgoingKind, TransactionStatus>::new();
    // EDU selection windows of in-flight transactions; persisted as the
    // destination's cursor once the transaction succeeds.
    let mut pending_edu_cursors = HashMap::<OutgoingKind, (Seqnum, bool)>::new();

    // Retry requests we could not finish yet
    let mut initial_transactions = HashMap::<OutgoingKind, Vec<SendingEventType>>::new();

    for (_id, outgoing_kind, event) in super::active_requests().await? {
        let entry = initial_transactions
            .entry(outgoing_kind.clone())
            .or_default();

        entry.push(event);
    }

    for (outgoing_kind, mut events) in initial_transactions {
        // Active rows survived a process restart, but their dynamic EDUs did not. Rebuild
        // the same unadvanced window before retrying so a pending recipient removal cannot
        // be confirmed by an unrelated PDU-only transaction.
        if let OutgoingKind::Normal(server_name) = &outgoing_kind {
            match select_edus(server_name).await {
                Ok((select_edus, last_sn, has_presence)) => {
                    if !select_edus.is_empty() {
                        events.extend(select_edus.into_iter().map(SendingEventType::Edu));
                        pending_edu_cursors.insert(outgoing_kind.clone(), (last_sn, has_presence));
                    }
                }
                Err(e) => {
                    error!(?server_name, error = ?e, "failed to rebuild startup EDU window");
                    current_transaction_status
                        .insert(outgoing_kind, TransactionStatus::Failed(1, Instant::now()));
                    continue;
                }
            }
        }
        current_transaction_status.insert(outgoing_kind.clone(), TransactionStatus::Running);
        futures.push(super::send_events(outgoing_kind.clone(), events));
    }

    // Recipient recalculation tasks are intentionally background work during normal room
    // processing. Re-scheduling every local presence row at startup closes the crash window
    // between committing an account-data/room change and that task creating its durable
    // Flush row.
    #[cfg(feature = "unstable-msc4495")]
    crate::user::presence::recipients::reconcile_on_startup();

    let mut sweep = tokio::time::interval(SWEEP_INTERVAL);
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    WORKER_READY.store(true, Ordering::Release);

    loop {
        let retry_delay = next_retry_delay(&current_transaction_status);

        tokio::select! {
            Some(response) = futures.next() => {
                match response {
                    Ok(outgoing_kind) => {
                        #[cfg(feature = "unstable-msc4532")]
                        if let Err(error) = crate::user::presence::revised::queue_completed(&outgoing_kind).await {
                            warn!(?error, "presence delivery recovery failed");
                        }
                        if let Some((edu_sn, has_presence)) = pending_edu_cursors.remove(&outgoing_kind)
                            && let OutgoingKind::Normal(server_name) = &outgoing_kind
                        {
                            #[cfg(feature = "unstable-msc4495")]
                            let recorded: AppResult<()> = crate::user::presence::recipients::confirm_sent(
                                server_name,
                                edu_sn,
                                has_presence.then_some(edu_sn),
                            )
                            .await;
                            #[cfg(not(feature = "unstable-msc4495"))]
                            let recorded: AppResult<()> = {
                                let _ = has_presence;
                                data::sending::advance_edu_cursor(server_name, edu_sn)
                                    .await
                                    .map_err(Into::into)
                            };
                            if let Err(e) = recorded {
                                // The peer received the transaction, but local durable state
                                // did not. Keep the active rows and retry the same idempotent
                                // window rather than acknowledging deltas we cannot recover.
                                error!(?server_name, error = ?e, "failed to record delivered EDU window");
                                current_transaction_status.insert(
                                    outgoing_kind,
                                    TransactionStatus::Failed(1, Instant::now()),
                                );
                                continue;
                            }
                        }

                        // Delete only after the cursor and recipient confirmations commit.
                        // A crash before this point leaves the active row as a durable retry
                        // anchor; a crash after it is safe because the delivery is recorded.
                        if let Err(error) = super::delete_all_active_requests_for(&outgoing_kind).await {
                            error!(?outgoing_kind, ?error, "failed to acknowledge delivered requests; retaining durable retry");
                            current_transaction_status.insert(
                                outgoing_kind,
                                TransactionStatus::Failed(1, Instant::now()),
                            );
                            continue;
                        }

                        // Find events that have been added since starting the last request
                        let new_events = match super::queued_requests(&outgoing_kind, super::QUEUED_REQUEST_LIMIT).await {
                            Ok(events) => events,
                            Err(error) => {
                                error!(?outgoing_kind, ?error, "failed to load follow-up requests; sweep will recover them");
                                current_transaction_status.remove(&outgoing_kind);
                                continue;
                            }
                        };

                        let mut events = if new_events.is_empty() {
                            Vec::new()
                        } else {
                            // Claim only rows that are still queued. Another
                            // process may have claimed or completed them since
                            // the select above.
                            match super::claim_queued_requests(&new_events).await {
                                Ok(events) => events,
                                Err(error) => {
                                    error!(?outgoing_kind, ?error, "failed to claim follow-up requests; sweep will recover them");
                                    current_transaction_status.remove(&outgoing_kind);
                                    continue;
                                }
                            }
                        };

                        if !events.is_empty() {
                            // Piggyback EDUs that accumulated while the previous
                            // transaction was in flight, so a busy destination
                            // does not starve presence/receipt/device-list updates.
                            if let OutgoingKind::Normal(server_name) = &outgoing_kind {
                                match select_edus(server_name).await {
                                    Ok((select_edus, last_sn, has_presence)) => {
                                        if !select_edus.is_empty() {
                                            events.extend(select_edus.into_iter().map(SendingEventType::Edu));
                                            pending_edu_cursors.insert(
                                                outgoing_kind.clone(),
                                                (last_sn, has_presence),
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        error!(?server_name, error = ?e, "failed to select follow-up EDU window");
                                        current_transaction_status.insert(
                                            outgoing_kind,
                                            TransactionStatus::Failed(1, Instant::now()),
                                        );
                                        continue;
                                    }
                                }
                            }

                            futures.push(super::send_events(outgoing_kind.clone(), events));
                        } else if let OutgoingKind::Normal(server_name) = &outgoing_kind {
                            match select_edus(server_name).await {
                                Ok((select_edus, last_sn, has_presence)) if !select_edus.is_empty() => {
                                    // No queued PDUs, but the EDU window is not empty
                                    // (e.g. a previous send failed and dropped it):
                                    // deliver the EDUs on their own instead of leaving
                                    // them until unrelated traffic shows up.
                                    let events = select_edus.into_iter().map(SendingEventType::Edu).collect::<Vec<_>>();
                                    pending_edu_cursors.insert(outgoing_kind.clone(), (last_sn, has_presence));
                                    futures.push(super::send_events(outgoing_kind.clone(), events));
                                }
                                Ok(_) => {
                                    current_transaction_status.remove(&outgoing_kind);
                                }
                                Err(e) => {
                                    // e.g. a concurrent recovery snapshot moved the recipient
                                    // stream mid-selection. Nothing queued would bring this
                                    // destination back, so let the retry timer re-select.
                                    error!(?server_name, error = ?e, "failed to select EDU-only follow-up window");
                                    current_transaction_status.insert(
                                        outgoing_kind,
                                        TransactionStatus::Failed(1, Instant::now()),
                                    );
                                }
                            }
                        } else {
                            current_transaction_status.remove(&outgoing_kind);
                        }
                    }
                    Err((outgoing_kind, event)) => {
                        #[cfg(feature = "unstable-msc4532")]
                        if let Err(error) = crate::user::presence::revised::delivery(&outgoing_kind, false).await {
                            warn!(?error, "presence delivery failure recording failed");
                        }
                        error!("failed to send event: {event:?}  outgoing_kind:{outgoing_kind:?}");
                        // Do not advance the cursor: the same EDU window is
                        // re-selected once the destination is reachable again.
                        pending_edu_cursors.remove(&outgoing_kind);
                        current_transaction_status.entry(outgoing_kind.clone()).and_modify(|e| *e = match e {
                            TransactionStatus::Running => {
                                TransactionStatus::Failed(1, Instant::now())
                            },
                            TransactionStatus::Retrying(n) => {
                                TransactionStatus::Failed(n.saturating_add(1), Instant::now())
                            },
                            TransactionStatus::Failed(_, _) => {
                                error!("Request that was not even running failed?!");
                                return
                            },
                        });
                        // Persist retry state to DB for cross-instance coordination
                        if let Some(TransactionStatus::Failed(tries, _)) = current_transaction_status.get(&outgoing_kind) {
                            let _ = persist_retry_state(&outgoing_kind, *tries).await;
                        }
                    }
                };
            },
            Some(outgoing_kind) = receiver.recv() => {
                if current_transaction_status.contains_key(&outgoing_kind) {
                    // A running or backing-off transaction will load queued
                    // rows when it settles or retries.
                    continue;
                }
                let new_events = match super::queued_requests(
                    &outgoing_kind,
                    super::QUEUED_REQUEST_LIMIT,
                ).await {
                    Ok(events) => events,
                    Err(e) => {
                        error!(?outgoing_kind, error = ?e, "failed to load queued requests for wakeup");
                        continue;
                    }
                };
                if new_events.is_empty() {
                    // A wakeup can also mean "there are EDUs to send" with nothing queued
                    // behind them. EDUs are built at selection time rather than stored as
                    // rows, so if this returned here they would wait for unrelated traffic
                    // -- which in a quiet room may never come.
                    if let OutgoingKind::Normal(server_name) = &outgoing_kind
                        && let Ok((select_edus, last_sn, has_presence)) = select_edus(server_name).await
                        && !select_edus.is_empty()
                    {
                        current_transaction_status
                            .insert(outgoing_kind.clone(), TransactionStatus::Running);
                        pending_edu_cursors.insert(outgoing_kind.clone(), (last_sn, has_presence));
                        let events = select_edus
                            .into_iter()
                            .map(SendingEventType::Edu)
                            .collect::<Vec<_>>();
                        futures.push(super::send_events(outgoing_kind, events));
                    }
                    continue;
                }
                match select_events(
                    &outgoing_kind,
                    new_events,
                    &mut current_transaction_status,
                    &mut pending_edu_cursors,
                ).await {
                    Ok(Some(events)) => {
                        futures.push(super::send_events(outgoing_kind, events));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        error!(?outgoing_kind, error = ?e, "failed to select queued requests for wakeup");
                    }
                }
            }
            _ = sweep.tick() => {
                // Recover destinations whose wakeup was dropped because the
                // wakeup queue was full, and requests left queued by a
                // previous process.
                let kinds = match super::queued_kinds().await {
                    Ok(kinds) => kinds,
                    Err(e) => {
                        WORKER_READY.store(false, Ordering::Release);
                        error!(error = ?e, "failed to load queued destinations for sweep");
                        continue;
                    }
                };
                WORKER_READY.store(true, Ordering::Release);
                for outgoing_kind in kinds {
                    if current_transaction_status.contains_key(&outgoing_kind) {
                        // Running, retrying, or backing off: the existing flow
                        // picks queued requests up when the transaction settles.
                        continue;
                    }
                    let new_events = match super::queued_requests(&outgoing_kind, super::QUEUED_REQUEST_LIMIT).await {
                        Ok(events) => events,
                        Err(e) => {
                            error!(?outgoing_kind, error = ?e, "failed to load queued requests for sweep");
                            continue;
                        }
                    };
                    if new_events.is_empty() {
                        continue;
                    }
                    match select_events(
                        &outgoing_kind,
                        new_events,
                        &mut current_transaction_status,
                        &mut pending_edu_cursors,
                    ).await {
                        Ok(Some(events)) => {
                            futures.push(super::send_events(outgoing_kind, events));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            error!(?outgoing_kind, error = ?e, "failed to select queued requests for sweep");
                        }
                    }
                }
            }
            _ = async {
                if let Some(delay) = retry_delay {
                    tokio::time::sleep(delay).await;
                } else {
                    pending::<()>().await;
                }
            } => {
                let retry_ready = current_transaction_status
                    .iter()
                    .filter_map(|(outgoing_kind, status)| match status {
                        TransactionStatus::Failed(tries, time)
                            if time.elapsed() >= retry_backoff(*tries) =>
                        {
                            Some((outgoing_kind.clone(), *tries))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();

                for (outgoing_kind, tries) in retry_ready {
                    let active_events = match super::active_requests_for(&outgoing_kind).await {
                        Ok(events) => events,
                        Err(e) => {
                            error!(
                                ?outgoing_kind,
                                error = ?e,
                                "failed to load active requests for retry"
                            );
                            current_transaction_status.insert(
                                outgoing_kind,
                                TransactionStatus::Failed(tries, Instant::now()),
                            );
                            continue;
                        }
                    };

                    let mut events = active_events.into_iter().map(|(_, event)| event).collect::<Vec<_>>();
                    // Also retry the EDU window whose delivery failed: it was
                    // dropped from pending_edu_cursors without advancing the
                    // cursor, so it is re-selected here instead of waiting for
                    // unrelated future traffic to this destination.
                    if let OutgoingKind::Normal(server_name) = &outgoing_kind {
                        match select_edus(server_name).await {
                            Ok((select_edus, last_sn, has_presence)) => {
                                if !select_edus.is_empty() {
                                    events.extend(select_edus.into_iter().map(SendingEventType::Edu));
                                    pending_edu_cursors.insert(
                                        outgoing_kind.clone(),
                                        (last_sn, has_presence),
                                    );
                                }
                            }
                            Err(e) => {
                                error!(?server_name, error = ?e, "failed to rebuild retry EDU window");
                                current_transaction_status.insert(
                                    outgoing_kind,
                                    TransactionStatus::Failed(tries, Instant::now()),
                                );
                                continue;
                            }
                        }
                    }

                    if events.is_empty() {
                        current_transaction_status.remove(&outgoing_kind);
                        continue;
                    }

                    current_transaction_status
                        .insert(outgoing_kind.clone(), TransactionStatus::Retrying(tries));
                    futures.push(super::send_events(outgoing_kind, events));
                }
            }
        }
    }
}

fn retry_backoff(tries: u32) -> Duration {
    Duration::from_secs(
        30_u64
            .saturating_mul(u64::from(tries))
            .saturating_mul(u64::from(tries))
            .min(60 * 60 * 24),
    )
}

fn next_retry_delay(
    current_transaction_status: &HashMap<OutgoingKind, TransactionStatus>,
) -> Option<Duration> {
    current_transaction_status
        .values()
        .filter_map(|status| match status {
            TransactionStatus::Failed(tries, time) => {
                Some(retry_backoff(*tries).saturating_sub(time.elapsed()))
            }
            _ => None,
        })
        .min()
}

#[tracing::instrument(skip_all)]
async fn select_events(
    outgoing_kind: &OutgoingKind,
    new_events: Vec<(i64, SendingEventType)>, // Events we want to send: event and full key
    current_transaction_status: &mut HashMap<OutgoingKind, TransactionStatus>,
    pending_edu_cursors: &mut HashMap<OutgoingKind, (Seqnum, bool)>,
) -> AppResult<Option<Vec<SendingEventType>>> {
    let mut retry = false;
    let mut allow = true;

    let entry = current_transaction_status.entry(outgoing_kind.clone());

    entry
        .and_modify(|e| match e {
            TransactionStatus::Running | TransactionStatus::Retrying(_) => {
                allow = false; // already running
            }
            TransactionStatus::Failed(tries, time) => {
                // Fail if a request has failed recently (exponential backoff)
                let min_elapsed_duration = retry_backoff(*tries);

                if time.elapsed() < min_elapsed_duration {
                    allow = false;
                } else {
                    retry = true;
                    *e = TransactionStatus::Retrying(*tries);
                }
            }
        })
        .or_insert(TransactionStatus::Running);

    if !allow {
        return Ok(None);
    }

    let mut events = Vec::new();

    if retry {
        // We retry the previous transaction
        for (_, e) in super::active_requests_for(outgoing_kind).await? {
            events.push(e);
        }
    } else {
        events = match super::claim_queued_requests(&new_events).await {
            Ok(events) => events,
            Err(error) => {
                current_transaction_status.remove(outgoing_kind);
                return Err(error);
            }
        };
        if events.is_empty() {
            // The wakeup was stale: its row was already claimed or deleted by
            // the periodic sweep (or another process). Undo the Running state
            // inserted above and do not start an empty transaction.
            current_transaction_status.remove(outgoing_kind);
            return Ok(None);
        }
    }

    // Piggyback pending EDUs on fresh and retry transactions alike. The
    // cursor only advances once a transaction succeeds, so a retry simply
    // re-selects the same window.
    if let OutgoingKind::Normal(server_name) = outgoing_kind {
        match select_edus(server_name).await {
            Ok((select_edus, last_sn, has_presence)) => {
                if !select_edus.is_empty() {
                    events.extend(select_edus.into_iter().map(SendingEventType::Edu));
                    pending_edu_cursors.insert(outgoing_kind.clone(), (last_sn, has_presence));
                }
            }
            Err(e) => {
                let tries = match current_transaction_status.get(outgoing_kind) {
                    Some(
                        TransactionStatus::Retrying(tries) | TransactionStatus::Failed(tries, _),
                    ) => *tries,
                    _ => 1,
                };
                current_transaction_status.insert(
                    outgoing_kind.clone(),
                    TransactionStatus::Failed(tries, Instant::now()),
                );
                return Err(e);
            }
        }
    }

    Ok(Some(events))
}

/// Persist retry state to the database so other instances can respect backoff.
async fn persist_retry_state(outgoing_kind: &OutgoingKind, tries: u32) -> AppResult<()> {
    let dest = match outgoing_kind {
        OutgoingKind::Normal(server_name) => {
            data::sending::OutgoingDestination::Normal(server_name)
        }
        OutgoingKind::Appservice(id) => data::sending::OutgoingDestination::Appservice(id),
        OutgoingKind::Push(user_id, pushkey) => {
            data::sending::OutgoingDestination::Push { user_id, pushkey }
        }
    };
    data::sending::persist_retry_state(dest, tries).await?;
    Ok(())
}

/// Look for device changes
#[tracing::instrument(level = "trace", skip(server_name))]
async fn select_edus_device_changes(
    server_name: &ServerName,
    since_sn: Seqnum,
    _max_edu_sn: &Seqnum,
    events_len: &AtomicUsize,
) -> AppResult<EduVec> {
    let mut events = EduVec::new();
    let server_rooms = state::server_joined_rooms(server_name).await?;

    let mut device_list_changes = HashSet::<OwnedUserId>::new();
    for room_id in server_rooms {
        let keys_changed = room::keys_changed_users(&room_id, since_sn, None)
            .await?
            .into_iter()
            .filter(|user_id| user_id.is_local());

        for user_id in keys_changed {
            // max_edu_sn.fetch_max(event_sn, Ordering::Relaxed);
            if !device_list_changes.insert(user_id.clone()) {
                continue;
            }

            // Empty prev id forces synapse to resync; because synapse resyncs,
            // we can just insert placeholder data
            let stream_id = data::user::key::device_list_stream_id(&user_id).await? as u64;
            let edu = Edu::DeviceListUpdate(DeviceListUpdateContent {
                user_id,
                device_id: device_id!("placeholder").to_owned(),
                device_display_name: Some("Placeholder".to_owned()),
                stream_id,
                prev_id: Vec::new(),
                deleted: None,
                keys: None,
            });

            let mut buf = EduBuf::new();
            if let Err(e) = serde_json::to_writer(&mut buf, &edu) {
                tracing::error!("failed to serialize device list update to JSON: {e}");
                continue;
            }

            events.push(buf);
            if events_len.fetch_add(1, Ordering::Relaxed) >= SELECT_EDU_LIMIT - 1 {
                return Ok(events);
            }
        }
    }

    Ok(events)
}

/// Look for read receipts in this room
#[tracing::instrument(level = "trace", skip(server_name, max_edu_sn))]
async fn select_edus_receipts(
    server_name: &ServerName,
    since_sn: Seqnum,
    max_edu_sn: &Seqnum,
) -> AppResult<Option<EduBuf>> {
    let mut num = 0;
    let mut receipts: BTreeMap<OwnedRoomId, ReceiptMap> = BTreeMap::new();
    for room_id in state::server_joined_rooms(server_name).await? {
        let Ok(receipt_map) =
            select_edus_receipts_room(&room_id, since_sn, max_edu_sn, &mut num).await
        else {
            continue;
        };

        if !receipt_map.read.is_empty() {
            receipts.insert(room_id, receipt_map);
        }
    }

    if receipts.is_empty() {
        return Ok(None);
    }

    let receipt_content = Edu::Receipt(ReceiptContent::new(receipts));

    let mut buf = EduBuf::new();
    if let Err(e) = serde_json::to_writer(&mut buf, &receipt_content) {
        tracing::error!("failed to serialize Receipt EDU to JSON: {e}");
        return Ok(None);
    }

    Ok(Some(buf))
}
/// Look for read receipts in this room
#[tracing::instrument(level = "trace", skip(since_sn))]
async fn select_edus_receipts_room(
    room_id: &RoomId,
    since_sn: Seqnum,
    _max_edu_sn: &Seqnum,
    num: &mut usize,
) -> AppResult<ReceiptMap> {
    let receipts = data::room::receipt::read_receipts(room_id, since_sn).await?;

    let mut read = BTreeMap::<OwnedUserId, ReceiptData>::new();
    for (user_id, read_receipt) in receipts {
        // if count > since_sn {
        //     break;
        // }

        // max_edu_sn.fetch_max(occur_sn, Ordering::Relaxed);
        if !user_id.is_local() {
            continue;
        }

        // let Ok(event) = serde_json::from_str(read_receipt.inner().get()) else {
        //     error!(
        //         ?user_id,
        //         ?read_receipt,
        //         "Invalid edu event in read_receipts."
        //     );
        //     continue;
        // };

        // let AnySyncEphemeralRoomEvent::Receipt(r) = event else {
        //     error!(?user_id, ?event, "Invalid event type in read_receipts");
        //     continue;
        // };

        let (event_id, mut receipt) = read_receipt
            .0
            .into_iter()
            .next()
            .expect("we only use one event per read receipt");

        let receipt = receipt
            .remove(&ReceiptType::Read)
            .expect("our read receipts always set this")
            .remove(&user_id)
            .expect("our read receipts always have the user here");

        let receipt_data = ReceiptData {
            data: receipt,
            event_ids: vec![event_id.clone()],
        };

        if read.insert(user_id.to_owned(), receipt_data).is_none() {
            *num = num.saturating_add(1);
            if *num >= SELECT_RECEIPT_LIMIT {
                break;
            }
        }
    }

    Ok(ReceiptMap { read })
}

/// What `server_name` must be told about `user_id`'s recipient set, or `None` when the
/// user shares presence with nobody there and no update should be sent at all.
///
/// A user who has not written an `m.presence.sharing` configuration shares presence with
/// nobody. MSC4495 is explicit that this must not fall back to legacy broadcasting:
/// otherwise a client could opt out of selective presence simply by never writing the
/// account data, which would defeat the point.
///
/// The delta is recorded as *pending*, not as sent. If the transaction carrying it fails
/// the destination's EDU cursor does not advance, the same window is reselected, and this
/// recomputes the identical delta from the same confirmed base -- which is what keeps a
/// removal from being dropped on the floor when a send fails.
#[cfg(feature = "unstable-msc4495")]
async fn selective_presence_delta(
    server_name: &ServerName,
    user_id: &UserId,
) -> AppResult<
    Option<(
        crate::user::presence::recipients::RecipientDelta,
        std::collections::BTreeSet<OwnedUserId>,
        bool,
    )>,
> {
    use std::collections::BTreeSet;

    use crate::user::presence::{recipients, sharing};

    let current: BTreeSet<_> = sharing::recipients_of(user_id)
        .await?
        .into_iter()
        .filter(|recipient| recipient.server_name() == server_name)
        .collect();

    let sent = recipients::sent_state(user_id, server_name).await?;
    let (prev_id, confirmed) = match sent.confirmed {
        Some((prev_id, set)) => (Some(prev_id), set),
        None => (None, BTreeSet::new()),
    };

    // Never told this destination anything and still have nothing to tell it.
    if current.is_empty() && confirmed.is_empty() && sent.pending.is_none() {
        return Ok(None);
    }

    let updates = recipients::diff(&confirmed, &current);
    if updates.is_empty() && sent.pending.is_none() {
        // Nothing changed for this destination, so keep naming the position it already
        // holds rather than inventing one it would have to resync against.
        return Ok(Some((
            recipients::RecipientDelta {
                stream_id: prev_id.unwrap_or(recipients::stream_id(user_id).await?),
                prev_id: None,
                updates,
            },
            current,
            false,
        )));
    }

    // Reuse an unacknowledged position only while it still describes the same set. If the
    // policy changed again during a failed send, use the user's newer global position;
    // a peer that received the older attempt will then resynchronise instead of treating
    // different sets as the same stream position.
    let stream_id = if let Some((stream_id, pending)) = sent.pending.as_ref()
        && pending == &current
    {
        *stream_id
    } else {
        // Bind a fresh identifier to the set we just computed. Reading the last global
        // position here is racy with a concurrent policy update: two different sets could
        // otherwise be emitted under the same stream_id, after which an unchanged update
        // could make a stale wider set look current to the peer. A failed retry reuses its
        // durable pending identifier above, while every genuinely different set gets a
        // sequence value that has never named another state.
        recipients::advance_stream(user_id).await?
    };
    Ok(Some((
        recipients::RecipientDelta {
            stream_id,
            prev_id,
            updates,
        },
        current,
        true,
    )))
}

/// Look for presence
#[tracing::instrument(level = "trace", skip(server_name))]
async fn select_edus_presence(
    server_name: &ServerName,
    since_sn: Seqnum,
    max_edu_sn: Seqnum,
) -> AppResult<(Option<EduBuf>, Seqnum)> {
    let presences = crate::data::user::presences_between(since_sn, max_edu_sn).await?;

    // Stable user ordering is part of federation retry idempotency: the transaction ID is
    // derived from the serialized EDU bytes.
    let mut presence_updates = BTreeMap::<OwnedUserId, PresenceUpdate>::new();
    #[cfg(feature = "unstable-msc4495")]
    let mut pending_updates = Vec::<(OwnedUserId, Seqnum, BTreeSet<OwnedUserId>)>::new();
    let mut confirmed_through = max_edu_sn;
    for (user_id, (presence_sn, presence_event)) in presences {
        if !user_id.is_local() {
            continue;
        }

        #[cfg(not(feature = "unstable-msc4495"))]
        if !state::server_can_see_user(server_name, &user_id).await? {
            continue;
        }

        // MSC4495: what this destination is allowed to see, and how its view of the
        // recipient set has to move to get there. `None` means the user shares presence
        // with nobody on this server, so nothing is sent at all.
        #[cfg(feature = "unstable-msc4495")]
        let Some((delta, recipients, needs_confirmation)) =
            selective_presence_delta(server_name, &user_id).await?
        else {
            continue;
        };

        let update = PresenceUpdate {
            #[cfg(feature = "unstable-msc4532")]
            status: {
                let mut wire = serde_json::to_value(&presence_event.content)?;
                crate::user::presence::revised::project(&user_id, &mut wire, true).await?;
                wire.get(crate::core::presence::revised::STATUS).cloned()
            },
            user_id: user_id.clone(),
            presence: presence_event.content.presence,
            currently_active: presence_event.content.currently_active.unwrap_or(false),
            status_msg: presence_event.content.status_msg,
            last_active_ago: presence_event.content.last_active_ago.unwrap_or(0),
            #[cfg(feature = "unstable-msc4495")]
            // A changed stream with no users relevant to this destination still carries
            // an explicit empty delta. Omitting `recipients` while retaining `prev_id`
            // produces a wire shape MSC4495 does not define.
            recipients: (delta.prev_id.is_some() || !delta.updates.is_empty())
                .then_some(delta.updates),
            #[cfg(feature = "unstable-msc4495")]
            stream_id: Some(delta.stream_id),
            #[cfg(feature = "unstable-msc4495")]
            prev_id: delta.prev_id,
        };

        #[cfg(feature = "unstable-msc4495")]
        if needs_confirmation {
            pending_updates.push((user_id.clone(), delta.stream_id, recipients));
        }
        presence_updates.insert(user_id, update);
        if presence_updates.len() >= SELECT_PRESENCE_LIMIT {
            // The rows are ordered. Confirm only through the last update actually placed
            // in this transaction; the next pass resumes at its successor.
            confirmed_through = presence_sn;
            break;
        }
    }

    if presence_updates.is_empty() {
        #[cfg(feature = "unstable-msc4495")]
        crate::user::presence::recipients::clear_pending(server_name).await?;
        return Ok((None, confirmed_through));
    }

    let presence_content = Edu::Presence(PresenceContent {
        push: presence_updates.into_values().collect(),
    });

    let mut buf = EduBuf::new();
    serde_json::to_writer(&mut buf, &presence_content)?;

    #[cfg(feature = "unstable-msc4495")]
    {
        crate::user::presence::recipients::clear_pending(server_name).await?;
        for (user_id, stream_id, recipients) in pending_updates {
            if !crate::user::presence::recipients::record_pending(
                &user_id,
                server_name,
                stream_id,
                &recipients,
                confirmed_through,
            )
            .await?
            {
                crate::user::presence::recipients::clear_pending(server_name).await?;
                return Err(crate::AppError::internal(
                    "presence recipient stream changed during EDU selection",
                ));
            }
        }
    }

    Ok((Some(buf), confirmed_through))
}

/// The lower bound (inclusive) of the next EDU selection window.
///
/// A destination with a persisted cursor resumes right after it; a
/// destination we have never completed an EDU transaction for starts at the
/// current sequence number so it is not flooded with historical updates.
fn edu_since_sn(cursor: Option<Seqnum>, max_edu_sn: Seqnum) -> Seqnum {
    cursor.map_or(max_edu_sn, |c| c + 1)
}

#[tracing::instrument(skip(server_name))]
pub async fn select_edus(server_name: &ServerName) -> AppResult<(EduVec, i64, bool)> {
    let max_edu_sn = data::user::curr_sn_after_presence_writes(None).await?;
    let conf = crate::config::get();

    let cursor = data::sending::get_edu_cursor(server_name).await?;
    let since_sn = edu_since_sn(cursor, max_edu_sn);

    let events_len = AtomicUsize::default();
    let device_changes =
        select_edus_device_changes(server_name, since_sn, &max_edu_sn, &events_len).await?;

    let mut events = device_changes;
    if conf.read_receipt.allow_outgoing
        && let Some(receipts) = select_edus_receipts(server_name, since_sn, &max_edu_sn).await?
    {
        events.push(receipts);
    }

    let mut confirmed_through = max_edu_sn;
    let mut has_presence = false;
    if conf.presence.allow_outgoing {
        let (presence, presence_confirmed_through) =
            select_edus_presence(server_name, since_sn, max_edu_sn).await?;
        confirmed_through = confirmed_through.min(presence_confirmed_through);
        has_presence = presence.is_some();
        if let Some(presence) = presence {
            events.push(presence);
        }
    }

    Ok((events, confirmed_through, has_presence))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edu_selection_resumes_after_cursor_and_starts_fresh_without_one() {
        // Selection queries are `occur_sn >= since_sn`, so resuming right
        // after the cursor neither re-sends nor skips updates.
        assert_eq!(edu_since_sn(Some(41), 100), 42);
        // No cursor yet: start at the current sequence number instead of
        // replaying history to a destination we have never sent EDUs to.
        assert_eq!(edu_since_sn(None, 100), 100);
    }

    #[test]
    fn retry_backoff_grows_quadratically_and_caps_at_one_day() {
        assert_eq!(retry_backoff(1), Duration::from_secs(30));
        assert_eq!(retry_backoff(2), Duration::from_secs(120));
        assert_eq!(retry_backoff(100), Duration::from_secs(60 * 60 * 24));
        assert_eq!(retry_backoff(u32::MAX), Duration::from_secs(60 * 60 * 24));
    }

    #[test]
    fn next_retry_delay_returns_zero_for_due_failed_transaction() {
        let mut statuses = HashMap::new();
        statuses.insert(
            OutgoingKind::Normal(OwnedServerName::try_from("remote.example").unwrap()),
            TransactionStatus::Failed(1, Instant::now() - Duration::from_secs(31)),
        );

        assert_eq!(next_retry_delay(&statuses), Some(Duration::ZERO));
    }

    #[test]
    fn next_retry_delay_ignores_non_failed_transactions() {
        let mut statuses = HashMap::new();
        statuses.insert(
            OutgoingKind::Normal(OwnedServerName::try_from("remote.example").unwrap()),
            TransactionStatus::Running,
        );

        assert_eq!(next_retry_delay(&statuses), None);
    }

    #[tokio::test]
    #[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn review_sender_survives_ack_failure_and_recovers_startup_failure() {
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        use crate::data::schema::outgoing_requests;
        crate::test_database::init();
        crate::config::CONFIG
            .set(
                serde_json::from_value(serde_json::json!({
                    "server_name":"sender-recovery.example", "db":{"url":"unused"},
                    "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
                }))
                .unwrap(),
            )
            .expect("run alone");

        let failed = OutgoingKind::Appservice("ack-fault".into());
        let healthy = OutgoingKind::Appservice("healthy".into());
        let claim_failed = OutgoingKind::Appservice("claim-fault".into());
        for kind in [&failed, &healthy, &claim_failed] {
            let id = super::super::queue_request(kind, &SendingEventType::Flush)
                .await
                .unwrap();
            super::super::claim_queued_requests(&[(id, SendingEventType::Flush)])
                .await
                .unwrap();
        }
        super::super::queue_request(&claim_failed, &SendingEventType::Flush)
            .await
            .unwrap();
        // nextval is deliberately non-transactional: it proves the failing
        // DELETE ran even though its transaction rolls back.
        let mut conn = data::connect().await.unwrap();
        for sql in [
            "CREATE SEQUENCE review_ack_attempts",
            "CREATE FUNCTION review_fail_ack() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.appservice_id = 'ack-fault' THEN PERFORM nextval('review_ack_attempts'); RAISE EXCEPTION 'injected transient acknowledgement failure'; END IF; RETURN OLD; END $$",
            "CREATE TRIGGER review_ack_failure BEFORE DELETE ON outgoing_requests FOR EACH ROW EXECUTE FUNCTION review_fail_ack()",
            "CREATE SEQUENCE review_claim_attempts",
            "CREATE FUNCTION review_fail_claim() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.appservice_id = 'claim-fault' AND NEW.state = 'pending' THEN PERFORM nextval('review_claim_attempts'); RAISE EXCEPTION 'injected transient claim failure'; END IF; RETURN NEW; END $$",
            "CREATE TRIGGER review_claim_failure BEFORE UPDATE ON outgoing_requests FOR EACH ROW EXECUTE FUNCTION review_fail_claim()",
        ] {
            diesel::sql_query(sql).execute(&mut conn).await.unwrap();
        }
        drop(conn);
        let (sender, receiver) = mpsc::channel(8);
        let restarts = restart_count();
        let worker = tokio::spawn(supervise(receiver, Duration::from_millis(20)));
        #[derive(QueryableByName)]
        struct Attempted {
            #[diesel(sql_type = diesel::sql_types::Bool)]
            is_called: bool,
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let attempted = diesel::sql_query("SELECT is_called FROM review_ack_attempts")
                    .get_result::<Attempted>(&mut data::connect().await.unwrap())
                    .await
                    .unwrap()
                    .is_called;
                let claim_attempted =
                    diesel::sql_query("SELECT is_called FROM review_claim_attempts")
                        .get_result::<Attempted>(&mut data::connect().await.unwrap())
                        .await
                        .unwrap()
                        .is_called;
                if attempted
                    && claim_attempted
                    && super::super::active_requests_for(&healthy)
                        .await
                        .unwrap()
                        .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("another destination must proceed despite acknowledgement failure");
        assert_eq!(
            restart_count(),
            restarts,
            "transient ack failure must stay inside the loop"
        );
        assert!(is_running());
        assert_eq!(
            super::super::queued_requests(&claim_failed, 30)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            super::super::active_requests_for(&failed)
                .await
                .unwrap()
                .len(),
            1
        );
        diesel::sql_query("DROP TRIGGER review_ack_failure ON outgoing_requests")
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        diesel::sql_query("DROP TRIGGER review_claim_failure ON outgoing_requests")
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(36), async {
            loop {
                let count: i64 = outgoing_requests::table
                    .count()
                    .get_result(&mut data::connect().await.unwrap())
                    .await
                    .unwrap();
                if count == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("ack failure must recover without process restart");
        assert!(
            !sender.is_closed(),
            "the receiver must remain open after recovery"
        );
        worker.abort();
        let _ = worker.await;
        assert!(
            !is_running(),
            "cancelling the supervisor must clear its status"
        );

        // Fail the startup SELECT itself, keep the receiver open, then restore
        // the database object. Pending work must be recovered automatically.
        let id = super::super::queue_request(&healthy, &SendingEventType::Flush)
            .await
            .unwrap();
        super::super::claim_queued_requests(&[(id, SendingEventType::Flush)])
            .await
            .unwrap();
        diesel::sql_query("ALTER TABLE outgoing_requests RENAME TO review_unavailable_requests")
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        let (_sender, receiver) = mpsc::channel(8);
        let worker = tokio::spawn(supervise(receiver, Duration::from_millis(20)));
        tokio::time::timeout(Duration::from_secs(3), async {
            while restart_count() == restarts {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        diesel::sql_query("ALTER TABLE review_unavailable_requests RENAME TO outgoing_requests")
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let count: i64 = outgoing_requests::table
                    .count()
                    .get_result(&mut data::connect().await.unwrap())
                    .await
                    .unwrap();
                if count == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("supervisor must resume durable work after database recovery");
        worker.abort();
        let _ = worker.await;
        assert!(!is_running());
    }
}
