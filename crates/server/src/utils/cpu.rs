//! Bound CPU work before placing it on Tokio's blocking executor.
use std::sync::{Arc, OnceLock};

use tokio::sync::Semaphore;

use crate::{AppError, AppResult};

pub async fn run<T, F>(task: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let slots = SLOTS.get_or_init(|| {
        let parallelism = std::thread::available_parallelism().map_or(1, usize::from);
        Arc::new(Semaphore::new(parallelism.clamp(1, 4)))
    });
    run_with_slots(slots.clone(), task).await
}

async fn run_with_slots<T, F>(slots: Arc<Semaphore>, task: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    // Apply async backpressure before enqueueing: at most this many blocking
    // jobs may run or wait in spawn_blocking's otherwise unbounded queue.
    let permit = slots
        .acquire_owned()
        .await
        .map_err(|_| AppError::internal("CPU worker pool closed"))?;
    tokio::task::spawn_blocking(move || {
        // Cancellation of the HTTP task must not release a slot while work still runs.
        let _permit = permit;
        task()
    })
    .await
    .map_err(|error| AppError::internal(format!("CPU worker failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn cpu_work_keeps_the_runtime_responsive_and_cancellation_keeps_its_slot() {
        let slots = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task_slots = slots.clone();
        let task = tokio::spawn(run_with_slots(task_slots, move || {
            let _ = started_tx.send(());
            release_rx.recv().unwrap();
            Ok(())
        }));
        tokio::time::timeout(std::time::Duration::from_secs(5), started_rx)
            .await
            .unwrap()
            .unwrap();
        // This async task is executing on the sole runtime thread while CPU work is blocked.
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(20),
                run_with_slots(slots.clone(), || Ok(())),
            )
            .await
            .is_err()
        );
        task.abort();
        let _ = task.await;
        assert_eq!(slots.available_permits(), 0);
        release_tx.send(()).unwrap();
        let _permit = tokio::time::timeout(std::time::Duration::from_secs(5), slots.acquire())
            .await
            .unwrap()
            .unwrap();
    }
}
