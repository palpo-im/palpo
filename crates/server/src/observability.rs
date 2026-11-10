//! Process-local bounded diagnostics, separate from cheap readiness probes.
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use diesel::QueryableByName;
use diesel_async::{AsyncConnection, RunQueryDsl, SimpleAsyncConnection};
use serde::Serialize;
use tokio::sync::{AcquireError, Semaphore, SemaphorePermit};

use crate::data::telemetry::{WaitMetrics, WaitSnapshot, WaitTimer};
use crate::{AppError, data};

const QUEUE_CACHE_TTL: Duration = Duration::from_secs(30);
const QUEUE_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
static QUEUE_SAMPLE: QueueSampler = QueueSampler::new();
static PERMIT_WAIT: WaitMetrics = WaitMetrics::new();
static FEDERATION_DEADLINES: AtomicU64 = AtomicU64::new(0);

const QUEUE_SQL: &str = r#"
WITH observed AS (
    SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::BIGINT AS now_ms
)
SELECT max(now_ms) AS observed_at_db_ms,
       count(q.id)::BIGINT AS total_rows,
       count(q.id) FILTER (WHERE q.state = 'created')::BIGINT AS queued_rows,
       count(q.id) FILTER (WHERE q.state = 'pending')::BIGINT AS claimed_rows,
       count(q.id) FILTER (WHERE q.retry_count > 0)::BIGINT AS retry_marked_rows,
       count(q.id) FILTER (WHERE q.enqueued_at IS NULL)::BIGINT AS unknown_age_rows,
       count(q.id) FILTER (WHERE q.enqueued_at > now_ms)::BIGINT AS future_timestamp_rows,
       CASE WHEN count(q.enqueued_at) > 0 THEN
         GREATEST(0::numeric, LEAST(9223372036854775807::numeric,
             max(now_ms)::numeric - min(q.enqueued_at)::numeric))::BIGINT
       END AS oldest_known_age_ms
FROM observed LEFT JOIN outgoing_requests q ON true
"#;

fn increment(value: &AtomicU64) {
    let mut current = AtomicU64::load(value, Ordering::Relaxed);
    loop {
        match value.compare_exchange_weak(
            current,
            current.saturating_add(1),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

fn milliseconds(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[derive(Default)]
struct WorkerState {
    last_heartbeat: Option<Instant>,
    recovery_started: Option<Instant>,
    completed_recoveries: u64,
    last_recovery_ms: Option<u64>,
    longest_recovery_ms: u64,
}

static WORKER: Mutex<WorkerState> = Mutex::new(WorkerState {
    last_heartbeat: None,
    recovery_started: None,
    completed_recoveries: 0,
    last_recovery_ms: None,
    longest_recovery_ms: 0,
});

pub(crate) fn worker_started() {
    worker_unready();
    worker_heartbeat();
}

pub(crate) fn worker_stopped() {
    worker_unready();
}

pub(crate) fn worker_unready() {
    let mut state = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    state.recovery_started.get_or_insert_with(Instant::now);
}

pub(crate) fn worker_heartbeat() {
    WORKER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .last_heartbeat = Some(Instant::now());
}

pub(crate) fn worker_ready() {
    let mut state = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    state.last_heartbeat = Some(Instant::now());
    if let Some(started) = state.recovery_started.take() {
        let elapsed = milliseconds(started.elapsed());
        state.completed_recoveries = state.completed_recoveries.saturating_add(1);
        state.last_recovery_ms = Some(elapsed);
        state.longest_recovery_ms = state.longest_recovery_ms.max(elapsed);
    }
}

#[derive(Debug, Serialize)]
pub struct WorkerSnapshot {
    pub heartbeat_age_ms: Option<u64>,
    pub recovering_for_ms: Option<u64>,
    pub completed_recoveries: u64,
    pub last_recovery_ms: Option<u64>,
    pub longest_recovery_ms: u64,
}

fn worker_snapshot() -> WorkerSnapshot {
    let state = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    WorkerSnapshot {
        heartbeat_age_ms: state.last_heartbeat.map(|at| milliseconds(at.elapsed())),
        recovering_for_ms: state.recovery_started.map(|at| milliseconds(at.elapsed())),
        completed_recoveries: state.completed_recoveries,
        last_recovery_ms: state.last_recovery_ms,
        longest_recovery_ms: state.longest_recovery_ms,
    }
}

pub(crate) async fn acquire_outbound_permit(
    semaphore: &Semaphore,
) -> Result<SemaphorePermit<'_>, AcquireError> {
    let timer = PERMIT_WAIT.start();
    let result = semaphore.acquire().await;
    timer.finish(result.is_ok());
    result
}

pub(crate) fn federation_deadline() {
    increment(&FEDERATION_DEADLINES);
}

struct SendMetrics {
    latency: WaitMetrics,
    active: AtomicU64,
    outcomes: [AtomicU64; 8],
}

impl SendMetrics {
    const fn new() -> Self {
        Self {
            latency: WaitMetrics::new(),
            active: AtomicU64::new(0),
            outcomes: [const { AtomicU64::new(0) }; 8],
        }
    }
}

static SENDS: [SendMetrics; 3] = [const { SendMetrics::new() }; 3];

/// Fixed error families, not URL text or per-server labels.
pub const OUTCOME_NAMES: [&str; 8] = [
    "success",
    "storage",
    "transport_timeout",
    "transport",
    "decode",
    "matrix",
    "application",
    "cancelled",
];

pub(crate) struct SendAttempt {
    metrics: &'static SendMetrics,
    timer: Option<WaitTimer<'static>>,
    finished: bool,
}

impl SendAttempt {
    pub(crate) fn start(kind: &crate::sending::OutgoingKind) -> Self {
        let index = match kind {
            crate::sending::OutgoingKind::Normal(_) => 0,
            crate::sending::OutgoingKind::Appservice(_) => 1,
            crate::sending::OutgoingKind::Push(..) => 2,
        };
        let metrics = &SENDS[index];
        increment(&metrics.active);
        Self {
            metrics,
            timer: Some(metrics.latency.start()),
            finished: false,
        }
    }

    pub(crate) fn finish(mut self, error: Option<&AppError>) {
        let category = match error {
            None => 0,
            Some(AppError::Pool(_) | AppError::Data(_) | AppError::Diesel(_)) => 1,
            Some(AppError::Reqwest(error)) if error.is_timeout() => 2,
            Some(AppError::Reqwest(_) | AppError::ReqwestMiddleware(_) | AppError::Send(_)) => 3,
            Some(AppError::SerdeJson(_) | AppError::CanonicalJson(_) | AppError::Decoding(_)) => 4,
            Some(AppError::Matrix(_)) => 5,
            Some(_) => 6,
        };
        increment(&self.metrics.outcomes[category]);
        self.finished = true;
        self.timer.take().unwrap().finish(error.is_none());
    }
}

impl Drop for SendAttempt {
    fn drop(&mut self) {
        if !self.finished {
            increment(&self.metrics.outcomes[7]);
        }
        let active = &self.metrics.active;
        let mut current = AtomicU64::load(active, Ordering::Relaxed);
        loop {
            match active.compare_exchange_weak(
                current,
                current.saturating_sub(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SendSnapshot {
    pub kind: &'static str,
    pub active_attempts: u64,
    pub latency: WaitSnapshot,
    pub outcome_counts: [u64; 8],
}

#[derive(Clone, Debug, QueryableByName, Serialize)]
pub struct QueueSnapshot {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub observed_at_db_ms: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_rows: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub queued_rows: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub claimed_rows: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub retry_marked_rows: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub unknown_age_rows: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub future_timestamp_rows: i64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::BigInt>)]
    pub oldest_known_age_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueueSampleState {
    NotInitialized,
    Fresh,
    Cached,
    Stale,
    Failed,
    TimedOut,
    Busy,
}

#[derive(Debug, Serialize)]
pub struct QueueReport {
    pub state: QueueSampleState,
    pub snapshot_age_ms: Option<u64>,
    pub snapshot: Option<QueueSnapshot>,
}

struct CachedQueue {
    last_attempt: Option<Instant>,
    last_success: Option<Instant>,
    failed: bool,
    snapshot: Option<QueueSnapshot>,
}

struct QueueSampler {
    cache: Mutex<CachedQueue>,
    slot: Semaphore,
}

impl QueueSampler {
    const fn new() -> Self {
        Self {
            cache: Mutex::new(CachedQueue {
                last_attempt: None,
                last_success: None,
                failed: false,
                snapshot: None,
            }),
            slot: Semaphore::const_new(1),
        }
    }

    fn report(&self, state: QueueSampleState) -> QueueReport {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        QueueReport {
            state,
            snapshot_age_ms: cache.last_success.map(|at| milliseconds(at.elapsed())),
            snapshot: cache.snapshot.clone(),
        }
    }

    async fn sample(&self, ttl: Duration) -> QueueReport {
        if data::DIESEL_POOL.get().is_none() {
            return self.report(QueueSampleState::NotInitialized);
        }
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if cache.last_attempt.is_some_and(|at| at.elapsed() < ttl) {
                let state = if cache.failed {
                    if cache.snapshot.is_some() {
                        QueueSampleState::Stale
                    } else {
                        QueueSampleState::Failed
                    }
                } else {
                    QueueSampleState::Cached
                };
                drop(cache);
                return self.report(state);
            }
        }
        let Ok(_slot) = self.slot.try_acquire() else {
            return self.report(QueueSampleState::Busy);
        };
        // No await separates admission and recording the refresh attempt.
        {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            cache.last_attempt = Some(Instant::now());
            cache.failed = true;
        }
        let result = tokio::time::timeout(QUEUE_PROBE_TIMEOUT, async {
            data::connect()
                .await?
                .transaction::<_, AppError, _>(async |conn| {
                    conn.batch_execute("SET LOCAL statement_timeout = '500ms'")
                        .await?;
                    Ok(diesel::sql_query(QUEUE_SQL)
                        .get_result::<QueueSnapshot>(conn)
                        .await?)
                })
                .await
        })
        .await;
        match result {
            Ok(Ok(snapshot)) => {
                let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                cache.snapshot = Some(snapshot);
                cache.last_success = Some(Instant::now());
                cache.failed = false;
                drop(cache);
                self.report(QueueSampleState::Fresh)
            }
            failure => {
                let state = if failure.is_err() {
                    QueueSampleState::TimedOut
                } else {
                    QueueSampleState::Failed
                };
                self.cache.lock().unwrap_or_else(|e| e.into_inner()).failed = true;
                self.report(state)
            }
        }
    }
}

#[cfg(test)]
#[path = "observability_tests.rs"]
mod tests;

#[derive(Debug, Serialize)]
pub struct Diagnostics {
    pub readiness: crate::readiness::Readiness,
    pub worker: WorkerSnapshot,
    pub queue: QueueReport,
    pub query_pool_wait: WaitSnapshot,
    pub coordination_pool_wait: WaitSnapshot,
    pub outbound_permit_wait: WaitSnapshot,
    pub outbound: [SendSnapshot; 3],
    pub outcome_names: [&'static str; 8],
    pub latency_bucket_upper_us: [u64; 6],
    pub federation_deadline_exceeded: u64,
}

/// Structured process-local metrics. Queue sampling is cached and bounded;
/// the ordinary readiness endpoint does not call this queue aggregation.
pub async fn check() -> Diagnostics {
    let readiness = crate::readiness::check().await;
    let queue = QUEUE_SAMPLE.sample(QUEUE_CACHE_TTL).await;
    Diagnostics {
        readiness,
        worker: worker_snapshot(),
        queue,
        query_pool_wait: data::telemetry::QUERY_POOL_WAIT.snapshot(),
        coordination_pool_wait: data::telemetry::COORDINATION_POOL_WAIT.snapshot(),
        outbound_permit_wait: PERMIT_WAIT.snapshot(),
        outbound: std::array::from_fn(|index| SendSnapshot {
            kind: ["federation", "appservice", "push"][index],
            active_attempts: AtomicU64::load(&SENDS[index].active, Ordering::Relaxed),
            latency: SENDS[index].latency.snapshot(),
            outcome_counts: std::array::from_fn(|category| {
                AtomicU64::load(&SENDS[index].outcomes[category], Ordering::Relaxed)
            }),
        }),
        outcome_names: OUTCOME_NAMES,
        latency_bucket_upper_us: data::telemetry::LATENCY_BUCKET_UPPER_US,
        federation_deadline_exceeded: AtomicU64::load(&FEDERATION_DEADLINES, Ordering::Relaxed),
    }
}
