#[cfg(feature = "unstable-msc4495")]
use std::collections::BTreeSet;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::pending;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use diesel_async::AsyncConnection;
#[cfg(not(feature = "unstable-msc4495"))]
use diesel_async::RunQueryDsl;
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::mpsc;

use super::{
    EduBuf, EduVec, MPSC_SENDER, OutgoingKind, SELECT_EDU_LIMIT, SELECT_PRESENCE_LIMIT,
    SELECT_RECEIPT_LIMIT, SendingEventType, TransactionStatus,
};
use crate::core::device::DeviceListUpdateContent;
use crate::core::events::receipt::{ReceiptContent, ReceiptData, ReceiptMap};
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::core::presence::{PresenceContent, PresenceUpdate};
use crate::core::{Seqnum, device_id};
use crate::exts::*;
use crate::room::state;
use crate::{AppResult, data};

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

#[cfg(test)]
#[path = "selection_tests.rs"]
mod selection_tests;

struct RunningWorker;

impl Drop for RunningWorker {
    fn drop(&mut self) {
        crate::observability::worker_stopped();
        WORKER_READY.store(false, Ordering::Release);
        WORKER_RUNNING.store(false, Ordering::Release);
    }
}

/// Whether the sender is processing work rather than recovering from failure.
pub fn is_running() -> bool {
    AtomicBool::load(&WORKER_RUNNING, Ordering::Acquire)
}

/// Whether durable startup recovery completed and the scheduler's latest
/// queue sweep succeeded. Destination-specific delivery failures do not make
/// the whole server unready: they retain durable retries instead.
pub fn is_ready() -> bool {
    is_running() && AtomicBool::load(&WORKER_READY, Ordering::Acquire)
}

pub fn restart_count() -> u64 {
    AtomicU64::load(&WORKER_RESTARTS, Ordering::Relaxed)
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
        crate::observability::worker_started();
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

/// Bound the complete destination lifecycle, including SQL preparation and ACK.
/// A zero sender_workers retains the configured outbound concurrency ceiling.
fn destination_budget() -> usize {
    let conf = crate::config::get();
    let ceiling = usize::from(conf.max_concurrent_requests).max(1);
    if conf.sender_workers == 0 {
        ceiling
    } else {
        conf.sender_workers.clamp(1, ceiling)
    }
}

fn enqueue_destination(
    kind: OutgoingKind,
    queue: &mut VecDeque<OutgoingKind>,
    queued: &mut HashSet<OutgoingKind>,
    statuses: &HashMap<OutgoingKind, TransactionStatus>,
    pending_wakeups: &mut HashSet<OutgoingKind>,
) {
    if let Some(status) = statuses.get(&kind) {
        // The in-flight query may already have observed an empty queue. Keep
        // its notification until completion so that new rows get another probe.
        if matches!(
            status,
            TransactionStatus::Running | TransactionStatus::Retrying(_)
        ) {
            pending_wakeups.insert(kind);
        }
    } else if queued.insert(kind.clone()) {
        queue.push_back(kind);
    }
}

async fn process(receiver: &mut mpsc::Receiver<super::WakeupMessage>) -> AppResult<()> {
    let budget = destination_budget();
    let mut futures =
        FuturesUnordered::<BoxFuture<'static, (OutgoingKind, u32, AppResult<bool>)>>::new();
    let mut statuses = HashMap::<OutgoingKind, TransactionStatus>::new();
    let mut queue = VecDeque::new();
    let mut queued = HashSet::new();
    let mut pending_wakeups = HashSet::new();
    // Only destination identities are recovered at startup; payload/EDU SQL
    // belongs to bounded destination futures, never the heartbeat loop.
    for kind in super::queued_kinds().await? {
        enqueue_destination(
            kind,
            &mut queue,
            &mut queued,
            &statuses,
            &mut pending_wakeups,
        );
    }
    #[cfg(feature = "unstable-msc4495")]
    crate::user::presence::recipients::reconcile_on_startup();

    let mut sweep = tokio::time::interval(SWEEP_INTERVAL);
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut scans = FuturesUnordered::new();
    WORKER_READY.store(true, Ordering::Release);
    crate::observability::worker_ready();
    loop {
        crate::observability::worker_heartbeat();
        // Failed destinations become eligible only after their backoff. Move
        // them to the tail, so an always-busy destination cannot monopolize a
        // released slot while other destinations are waiting.
        let due: Vec<_> = statuses
            .iter()
            .filter_map(|(kind, status)| match status {
                TransactionStatus::Failed(tries, at) if at.elapsed() >= retry_backoff(*tries) => {
                    Some((kind.clone(), *tries))
                }
                _ => None,
            })
            .collect();
        for (kind, tries) in due {
            statuses.insert(kind.clone(), TransactionStatus::Retrying(tries));
            if queued.insert(kind.clone()) {
                queue.push_back(kind);
            }
        }
        while futures.len() < budget {
            let Some(kind) = queue.pop_front() else {
                break;
            };
            queued.remove(&kind);
            let tries = match statuses.get(&kind) {
                Some(TransactionStatus::Retrying(tries)) => *tries,
                Some(_) => continue,
                None => 0,
            };
            statuses.insert(
                kind.clone(),
                if tries == 0 {
                    TransactionStatus::Running
                } else {
                    TransactionStatus::Retrying(tries)
                },
            );
            futures.push(
                async move {
                    let result = destination_transaction(&kind).await;
                    if let Err(error) = &result {
                        error!(
                            ?kind,
                            ?error,
                            "outbound destination failed; retaining durable retry"
                        );
                        #[cfg(feature = "unstable-msc4532")]
                        if let Err(error) =
                            crate::user::presence::revised::delivery(&kind, false).await
                        {
                            warn!(?error, "presence delivery failure recording failed");
                        }
                        if let Err(error) =
                            persist_retry_state(&kind, tries.saturating_add(1)).await
                        {
                            warn!(?kind, ?error, "failed to persist outbound retry state");
                        }
                    }
                    (kind, tries, result)
                }
                .boxed(),
            );
        }
        let retry_delay = next_retry_delay(&statuses);
        tokio::select! {
            Some((kind, tries, result)) = futures.next() => {
                match result {
                    Ok(had_work) => {
                        statuses.remove(&kind);
                        let notified = pending_wakeups.remove(&kind);
                        if had_work || notified {
                            enqueue_destination(kind, &mut queue, &mut queued, &statuses, &mut pending_wakeups);
                        }
                    }
                    Err(_) => {
                        pending_wakeups.remove(&kind);
                        statuses.insert(kind, TransactionStatus::Failed(tries.saturating_add(1), Instant::now()));
                    }
                }
            }
            Some(kind) = receiver.recv() => {
                enqueue_destination(kind, &mut queue, &mut queued, &statuses, &mut pending_wakeups);
            }
            _ = sweep.tick(), if scans.is_empty() => {
                // A slow sweep also runs outside the scheduling loop. There is
                // at most one scan in flight, with no spawn per notification.
                scans.push(super::queued_kinds());
            }
            Some(result) = scans.next() => {
                #[cfg(test)]
                selection_tests::INITIAL_SWEEP_FINISHED.notify_one();
                match result {
                    Ok(kinds) => {
                        WORKER_READY.store(true, Ordering::Release);
                        crate::observability::worker_ready();
                        for kind in kinds {
                            enqueue_destination(kind, &mut queue, &mut queued, &statuses, &mut pending_wakeups);
                        }
                    }
                    Err(error) => {
                        WORKER_READY.store(false, Ordering::Release);
                        crate::observability::worker_unready();
                        error!(?error, "outbound queue sweep failed; retrying durable recovery");
                    }
                }
            }
            _ = heartbeat.tick() => {}
            _ = async {
                if let Some(delay) = retry_delay { tokio::time::sleep(delay).await; }
                else { pending::<()>().await; }
            } => {}
        }
    }
}

/// One destination owns its preparation, network exchange and acknowledgement.
/// Dropping this future leaves all claimed rows and cursors recoverable.
async fn destination_transaction(kind: &OutgoingKind) -> AppResult<bool> {
    let active = super::active_requests_for(kind).await?;
    let mut events: Vec<_> = active.into_iter().map(|(_, event)| event).collect();
    if events.is_empty() {
        let queued = super::queued_requests(kind, super::QUEUED_REQUEST_LIMIT).await?;
        events = super::claim_queued_requests(&queued).await?;
    }
    let mut window = None;
    if let OutgoingKind::Normal(server) = kind {
        let durable_edus = events
            .iter()
            .filter(|event| matches!(event, SendingEventType::Edu(_)))
            .count();
        let selected =
            select_edu_window(server, super::EDU_LIMIT.saturating_sub(durable_edus)).await?;
        if !selected.events.is_empty() {
            if events.is_empty() {
                // Dynamic-only wakeups need a durable retry anchor too.
                let id = super::queue_request(kind, &SendingEventType::Flush).await?;
                events = super::claim_queued_requests(&[(id, SendingEventType::Flush)]).await?;
                if events.is_empty() {
                    return Ok(false);
                }
            }
            events.extend(selected.events.iter().cloned().map(SendingEventType::Edu));
        }
        if !events.is_empty() {
            window = Some(selected);
        }
    }
    if events.is_empty() {
        #[cfg(test)]
        selection_tests::pause_empty_probe(kind).await;
        return Ok(false);
    }
    super::send_events(kind.clone(), events)
        .await
        .map_err(|(_, error)| error)?;
    acknowledge_destination(kind, window.as_ref()).await?;
    #[cfg(feature = "unstable-msc4532")]
    if let Err(error) = crate::user::presence::revised::queue_completed(kind).await {
        warn!(?error, "presence delivery recovery failed");
    }
    Ok(true)
}

/// The pool normally rejects an open transaction at its *next checkout*.
/// A cancelled destination must not leave locks in an idle pooled object.
/// Until transaction().await returns completely, its backend state is treated
/// as uncertain independently of Diesel's transaction-depth bookkeeping.
pub(super) struct TransactionConnection {
    connection: Option<data::PgPooledConnection>,
    finished: bool,
}

impl TransactionConnection {
    pub(super) fn new(connection: data::PgPooledConnection) -> Self {
        Self {
            connection: Some(connection),
            finished: false,
        }
    }

    pub(super) fn connection(&mut self) -> &mut diesel_async::AsyncPgConnection {
        self.connection
            .as_mut()
            .expect("transaction connection is present")
    }

    pub(super) fn mark_finished(&mut self) {
        self.finished = true;
    }
}

impl Drop for TransactionConnection {
    fn drop(&mut self) {
        let Some(mut connection) = self.connection.take() else {
            return;
        };
        if !self.finished
            || diesel_async::pooled_connection::PoolableConnection::is_broken(&mut *connection)
        {
            // AsyncPgConnection::drop shuts down its driver; PostgreSQL rolls
            // back instead of keeping the cancelled transaction in the pool.
            drop(data::PgPooledConnection::take(connection));
        }
    }
}

async fn acknowledge_destination(kind: &OutgoingKind, window: Option<&EduWindow>) -> AppResult<()> {
    let mut connection = TransactionConnection::new(data::connect().await?);
    let result = connection.connection().transaction::<_, crate::AppError, _>(async |conn| {
        if let (OutgoingKind::Normal(server), Some(window)) = (kind, window) {
            #[cfg(feature = "unstable-msc4495")]
            crate::user::presence::recipients::confirm_sent_with_connection(
                conn, server, window.confirmed_through, window.presence_sn,
            ).await?;
            #[cfg(not(feature = "unstable-msc4495"))]
            diesel::sql_query(
                "INSERT INTO outgoing_edu_cursors (server_id,edu_sn,updated_at) VALUES ($1,$2,$3) \
                 ON CONFLICT (server_id) DO UPDATE SET edu_sn=GREATEST(outgoing_edu_cursors.edu_sn,excluded.edu_sn),updated_at=excluded.updated_at",
            ).bind::<diesel::sql_types::Text,_>(server.as_str())
             .bind::<diesel::sql_types::BigInt,_>(window.confirmed_through)
             .bind::<diesel::sql_types::BigInt,_>(crate::core::UnixMillis::now().get() as i64)
             .execute(conn).await?;
            data::sending::confirm_selector_position(conn, server, "device", &window.device).await?;
            data::sending::confirm_selector_position(conn, server, "receipt", &window.receipt).await?;
        }
        super::delete_active_requests_with_connection(conn, kind).await?;
        Ok(())
    }).await;
    connection.mark_finished();
    result
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

#[derive(Debug)]
struct EduWindow {
    events: EduVec,
    confirmed_through: Seqnum,
    // Presence pending rows are keyed by the actual presence selection bound,
    // which may differ from the common minimum across independent selectors.
    presence_sn: Option<Seqnum>,
    device: data::sending::SelectorPosition,
    receipt: data::sending::SelectorPosition,
}

async fn select_edus_device_changes(
    server: &ServerName,
    position: &data::sending::SelectorPosition,
    max: Seqnum,
    limit: usize,
) -> AppResult<(EduVec, data::sending::SelectorPosition)> {
    if limit == 0 {
        return Ok((EduVec::new(), position.clone()));
    }
    let rows = data::sending::outgoing_device_changes(
        server,
        &crate::config::get().server_name,
        position,
        max,
        limit + 1,
    )
    .await?;
    let truncated = rows.len() > limit;
    let mut next = data::sending::SelectorPosition::complete(max);
    let mut events = EduVec::new();
    for row in rows.into_iter().take(limit) {
        if truncated {
            next = data::sending::SelectorPosition {
                sn: row.sn,
                token: Some(row.user_id.to_string()),
            };
        }
        // Empty prev_id retains the existing force-resync placeholder policy.
        let edu = Edu::DeviceListUpdate(DeviceListUpdateContent {
            user_id: row.user_id,
            device_id: device_id!("placeholder").to_owned(),
            device_display_name: Some("Placeholder".to_owned()),
            stream_id: row.stream_id as u64,
            prev_id: Vec::new(),
            deleted: None,
            keys: None,
        });
        events.push(serde_json::to_vec(&edu)?);
    }
    Ok((events, next))
}

async fn select_edus_receipts(
    server: &ServerName,
    position: &data::sending::SelectorPosition,
    max: Seqnum,
) -> AppResult<(Option<EduBuf>, data::sending::SelectorPosition)> {
    let rows = data::sending::outgoing_receipts(
        server,
        &crate::config::get().server_name,
        position.sn,
        max,
        SELECT_RECEIPT_LIMIT + 1,
    )
    .await?;
    let mut receipts = BTreeMap::<OwnedRoomId, ReceiptMap>::new();
    let mut next = data::sending::SelectorPosition::complete(max);
    let mut last = position.sn;
    for (count, row) in rows.into_iter().enumerate() {
        let room = receipts.entry(row.room_id).or_insert_with(|| ReceiptMap {
            read: BTreeMap::new(),
        });
        // The wire map supports one scope per (room,user). Stop before a
        // second scope, leaving its unique SN for the next acknowledged page.
        if count == SELECT_RECEIPT_LIMIT || room.read.contains_key(&row.user_id) {
            next = data::sending::SelectorPosition::complete(last);
            break;
        }
        let receipt = serde_json::from_value(row.json_data)?;
        room.read.insert(
            row.user_id,
            ReceiptData {
                data: receipt,
                event_ids: vec![row.event_id],
            },
        );
        last = row.sn;
    }
    receipts.retain(|_, room| !room.read.is_empty());
    if receipts.is_empty() {
        return Ok((None, next));
    }
    Ok((
        Some(serde_json::to_vec(&Edu::Receipt(ReceiptContent::new(
            receipts,
        )))?),
        next,
    ))
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
    let window = select_edu_window(server_name, super::EDU_LIMIT).await?;
    Ok((
        window.events,
        window.confirmed_through,
        window.presence_sn.is_some(),
    ))
}

async fn select_edu_window(server: &ServerName, slots: usize) -> AppResult<EduWindow> {
    let max = data::user::curr_sn_after_presence_writes(None).await?;
    let cursor = data::sending::get_edu_cursor(server).await?;
    let since = edu_since_sn(cursor, max);
    // An old scalar cursor means *all* ties at N have been consumed. A new
    // destination preserves the old current-SN first-window behavior.
    let (device, receipt) =
        data::sending::selector_positions(server, since.saturating_sub(1)).await?;
    if slots == 0 {
        return Ok(EduWindow {
            events: EduVec::new(),
            confirmed_through: since.saturating_sub(1),
            presence_sn: None,
            device,
            receipt,
        });
    }
    let conf = crate::config::get();
    let reserved =
        usize::from(conf.read_receipt.allow_outgoing) + usize::from(conf.presence.allow_outgoing);
    let (mut events, device) = select_edus_device_changes(
        server,
        &device,
        max,
        slots.saturating_sub(reserved).min(SELECT_EDU_LIMIT),
    )
    .await?;
    let receipt = if conf.read_receipt.allow_outgoing && events.len() < slots {
        let (selected, next) = select_edus_receipts(server, &receipt, max).await?;
        if let Some(receipts) = selected {
            events.push(receipts);
        }
        next
    } else if conf.read_receipt.allow_outgoing {
        receipt
    } else {
        data::sending::SelectorPosition::complete(max)
    };
    let mut confirmed_through = device.confirmed_through().min(receipt.confirmed_through());
    let mut presence_sn = None;
    if conf.presence.allow_outgoing && events.len() < slots {
        let (selected, through) = select_edus_presence(server, since, max).await?;
        confirmed_through = confirmed_through.min(through);
        if let Some(presence) = selected {
            presence_sn = Some(through);
            events.push(presence);
        }
    }
    if conf.presence.allow_outgoing && presence_sn.is_none() && events.len() >= slots {
        confirmed_through = confirmed_through.min(since.saturating_sub(1));
    }
    Ok(EduWindow {
        events,
        confirmed_through,
        presence_sn,
        device,
        receipt,
    })
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
