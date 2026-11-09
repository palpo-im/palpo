//! Bounded readiness checks for standalone and embedded servers.
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use diesel_async::RunQueryDsl;
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::{data, sending};

const DATABASE_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_DATABASE_PROBES: usize = 4;
static DATABASE_PROBES: Semaphore = Semaphore::const_new(MAX_DATABASE_PROBES);
static STARTUP_COMPLETE: AtomicBool = AtomicBool::new(false);

pub(crate) fn mark_startup_complete() {
    STARTUP_COMPLETE.store(true, Ordering::Release);
}

/// A cheap connectivity result, without database URLs or internal error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseProbe {
    NotInitialized,
    Ready,
    Failed,
    TimedOut,
    Busy,
}

#[derive(Clone, Debug, Serialize)]
pub struct PoolStatus {
    pub max_size: usize,
    pub size: usize,
    pub available: usize,
    pub waiting: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct DatabasePools {
    pub query: PoolStatus,
    pub coordination: PoolStatus,
}

#[derive(Clone, Debug, Serialize)]
pub struct OutboundWorker {
    pub running: bool,
    pub ready: bool,
    pub restart_count: u64,
}

/// Operational status suitable for a host application's own readiness route.
/// This does not assert that every federation destination is reachable, or
/// that all optional background tasks and external services are healthy.
#[derive(Clone, Debug, Serialize)]
pub struct Readiness {
    pub ready: bool,
    pub startup_complete: bool,
    pub shutting_down: bool,
    pub database: DatabaseProbe,
    pub outbound_worker: OutboundWorker,
    pub database_pools: Option<DatabasePools>,
}

impl Readiness {
    fn evaluate(&mut self) {
        self.ready = self.startup_complete
            && !self.shutting_down
            && self.database == DatabaseProbe::Ready
            && self.outbound_worker.running
            && self.outbound_worker.ready;
    }
}

/// Check the ordinary query pool and `SELECT 1` within one second in total.
/// At most four probes run concurrently; excess probes fail immediately.
/// Pool telemetry and worker state are sampled after the database probe so an
/// observed worker exit during that await is reflected in the returned status.
pub async fn check() -> Readiness {
    // Installation publishes the two OnceLocks in sequence. is_initialized()
    // deliberately detects either one for initialization safety, but readiness
    // requires both. Keep the proven pair instead of re-reading partial globals.
    let pools = data::DIESEL_POOL.get().zip(data::COORDINATION_POOL.get());
    let database = if let Some((query, _)) = pools {
        probe(&DATABASE_PROBES, DATABASE_PROBE_TIMEOUT, async {
            let mut connection = query.get().await.map_err(|_| ())?;
            diesel::sql_query("SELECT 1")
                .execute(&mut connection)
                .await
                .map(|_| ())
                .map_err(|_| ())
        })
        .await
    } else {
        DatabaseProbe::NotInitialized
    };
    let database_pools = pools.map(|(query, coordination)| {
        let query = query.status();
        let coordination = coordination.status();
        DatabasePools {
            query: PoolStatus {
                max_size: query.max_size,
                size: query.size,
                available: query.available,
                waiting: query.waiting,
            },
            coordination: PoolStatus {
                max_size: coordination.max_size,
                size: coordination.size,
                available: coordination.available,
                waiting: coordination.waiting,
            },
        }
    });
    let mut status = Readiness {
        ready: false,
        startup_complete: AtomicBool::load(&STARTUP_COMPLETE, Ordering::Acquire),
        shutting_down: AtomicBool::load(&crate::SHUTDOWN, Ordering::Acquire),
        database,
        outbound_worker: OutboundWorker {
            running: sending::guard::is_running(),
            ready: sending::guard::is_ready(),
            restart_count: sending::guard::restart_count(),
        },
        database_pools,
    };
    status.evaluate();
    status
}

async fn probe(
    slots: &Semaphore,
    timeout: Duration,
    query: impl Future<Output = Result<(), ()>>,
) -> DatabaseProbe {
    let Ok(_permit) = slots.try_acquire() else {
        return DatabaseProbe::Busy;
    };
    match tokio::time::timeout(timeout, query).await {
        Ok(Ok(())) => DatabaseProbe::Ready,
        Ok(Err(())) => DatabaseProbe::Failed,
        Err(_) => DatabaseProbe::TimedOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_readiness_requires_startup_database_and_worker() {
        let mut status = Readiness {
            ready: false,
            startup_complete: true,
            shutting_down: false,
            database: DatabaseProbe::Ready,
            outbound_worker: OutboundWorker {
                running: true,
                ready: true,
                restart_count: 0,
            },
            database_pools: None,
        };
        status.evaluate();
        assert!(status.ready);
        for database in [
            DatabaseProbe::NotInitialized,
            DatabaseProbe::Failed,
            DatabaseProbe::TimedOut,
            DatabaseProbe::Busy,
        ] {
            let mut unhealthy = status.clone();
            unhealthy.database = database;
            unhealthy.evaluate();
            assert!(!unhealthy.ready);
        }
        for field in 0..4 {
            let mut unhealthy = status.clone();
            match field {
                0 => unhealthy.startup_complete = false,
                1 => unhealthy.shutting_down = true,
                2 => unhealthy.outbound_worker.running = false,
                _ => unhealthy.outbound_worker.ready = false,
            }
            unhealthy.evaluate();
            assert!(!unhealthy.ready);
        }
    }

    #[tokio::test]
    async fn review_readiness_probe_bounds_wait_and_concurrency_and_recovers() {
        let slots = Semaphore::new(1);
        let held = slots.acquire().await.unwrap();
        assert_eq!(
            probe(&slots, Duration::from_millis(10), async {
                panic!("busy probe must not poll its database future")
            })
            .await,
            DatabaseProbe::Busy
        );
        drop(held);
        assert_eq!(
            probe(&slots, Duration::from_millis(10), std::future::pending()).await,
            DatabaseProbe::TimedOut
        );
        assert_eq!(slots.available_permits(), 1);
        assert_eq!(
            probe(&slots, Duration::from_secs(1), async { Err(()) }).await,
            DatabaseProbe::Failed
        );
        assert_eq!(
            probe(&slots, Duration::from_secs(1), async { Ok(()) }).await,
            DatabaseProbe::Ready
        );
        assert_eq!(slots.available_permits(), 1);
    }
}
