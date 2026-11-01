//! Bound CPU work before placing it on Tokio's blocking executor.
use std::future::Future;
use std::sync::{Arc, OnceLock};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{AppError, AppResult};

pub async fn run<T, F>(task: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    run_with_slots(slots(), task).await
}

fn slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| {
            let parallelism = std::thread::available_parallelism().map_or(1, usize::from);
            Arc::new(Semaphore::new(parallelism.clamp(1, 4)))
        })
        .clone()
}

fn thumbnail_pipeline_slots() -> Arc<Semaphore> {
    static PIPELINES: OnceLock<Arc<Semaphore>> = OnceLock::new();
    PIPELINES
        .get_or_init(|| {
            let parallelism = std::thread::available_parallelism().map_or(1, usize::from);
            Arc::new(Semaphore::new(parallelism.clamp(1, 4)))
        })
        .clone()
}

/// Admit the whole input-read/CPU pipeline before polling the read future. This
/// prevents queued thumbnail requests from retaining full originals outside the
/// separate thumbnail budget. Slow storage reads never reserve CPU slots needed
/// by password work. Cancellation during read releases the pipeline permit;
/// after dispatch the worker owns both permits until CPU work actually finishes.
pub async fn pipeline<R, T, RF, F>(read: RF, task: F) -> AppResult<T>
where
    R: Send + 'static,
    T: Send + 'static,
    RF: Future<Output = AppResult<R>> + Send,
    F: FnOnce(R) -> AppResult<T> + Send + 'static,
{
    pipeline_with_slots(thumbnail_pipeline_slots(), slots(), read, task).await
}

async fn pipeline_with_slots<R, T, RF, F>(
    pipeline_slots: Arc<Semaphore>,
    cpu_slots: Arc<Semaphore>,
    read: RF,
    task: F,
) -> AppResult<T>
where
    R: Send + 'static,
    T: Send + 'static,
    RF: Future<Output = AppResult<R>> + Send,
    F: FnOnce(R) -> AppResult<T> + Send + 'static,
{
    let pipeline_permit = pipeline_slots
        .acquire_owned()
        .await
        .map_err(|_| AppError::internal("Thumbnail pipeline closed"))?;
    let input = read.await?;
    run_with_slots(cpu_slots, move || {
        // While queued for CPU, both the full original and this permit stay
        // together. Abort drops them; once dispatched only the worker may free them.
        let _pipeline_permit = pipeline_permit;
        task(input)
    })
    .await
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
    run_with_permit(permit, task).await
}

async fn run_with_permit<T, F>(permit: OwnedSemaphorePermit, task: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
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

    #[tokio::test(flavor = "current_thread")]
    async fn queued_original_reads_do_not_allocate_outside_pipeline_budget() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let slots = Arc::new(Semaphore::new(1));
        let cpu_slots = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let first_slots = slots.clone();
        let first = tokio::spawn(pipeline_with_slots(
            first_slots,
            cpu_slots.clone(),
            async move {
                let _ = started_tx.send(());
                release_rx.await.unwrap();
                Ok(vec![1u8; 4096])
            },
            |input| Ok(input.len()),
        ));
        started_rx.await.unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let second_reads = reads.clone();
        let second = tokio::spawn(pipeline_with_slots(
            slots.clone(),
            cpu_slots.clone(),
            async move {
                second_reads.fetch_add(1, Ordering::SeqCst);
                Ok(vec![2u8; 4096])
            },
            |input| Ok(input.len()),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(
            reads.load(Ordering::SeqCst),
            0,
            "queued request polled its storage read before admission"
        );
        // A password-style CPU job must run even while every thumbnail read
        // slot is occupied by slow I/O and another read waits for admission.
        let cpu_result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_with_slots(cpu_slots.clone(), || Ok(42)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(cpu_result, 42);
        release_tx.send(()).unwrap();
        assert_eq!(first.await.unwrap().unwrap(), 4096);
        assert_eq!(second.await.unwrap().unwrap(), 4096);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(slots.available_permits(), 1);
        // Cancellation of an admitted storage read promptly restores the budget.
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let pending = tokio::spawn(pipeline_with_slots(
            slots.clone(),
            cpu_slots.clone(),
            async move {
                let _ = started_tx.send(());
                std::future::pending::<AppResult<Vec<u8>>>().await
            },
            |input| Ok(input.len()),
        ));
        started_rx.await.unwrap();
        assert_eq!(slots.available_permits(), 0);
        pending.abort();
        let _ = pending.await;
        assert_eq!(slots.available_permits(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn buffered_thumbnail_waiting_for_cpu_keeps_its_read_budget() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let pipeline_slots = Arc::new(Semaphore::new(1));
        let cpu_slots = Arc::new(Semaphore::new(1));
        let busy_cpu = cpu_slots.clone().acquire_owned().await.unwrap();
        let (read_tx, read_rx) = tokio::sync::oneshot::channel();
        let first = tokio::spawn(pipeline_with_slots(
            pipeline_slots.clone(),
            cpu_slots.clone(),
            async move {
                let _ = read_tx.send(());
                Ok(vec![1u8; 4096])
            },
            |bytes| Ok(bytes.len()),
        ));
        read_rx.await.unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let queued_reads = reads.clone();
        let second = tokio::spawn(pipeline_with_slots(
            pipeline_slots.clone(),
            cpu_slots.clone(),
            async move {
                queued_reads.fetch_add(1, Ordering::SeqCst);
                Ok(vec![2u8; 4096])
            },
            |bytes| Ok(bytes.len()),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(
            reads.load(Ordering::SeqCst),
            0,
            "buffered CPU waiter released its read budget too soon"
        );
        assert_eq!(pipeline_slots.available_permits(), 0);
        first.abort();
        let _ = first.await;
        // Cancelling the CPU waiter releases its input and admits the next read.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while reads.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(busy_cpu);
        assert_eq!(second.await.unwrap().unwrap(), 4096);
        assert_eq!(pipeline_slots.available_permits(), 1);
        assert_eq!(cpu_slots.available_permits(), 1);
    }
}
