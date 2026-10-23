//! Bounds work that continues after an async request is cancelled.
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::Semaphore;

use super::{
    contract::ServiceError,
    lifecycle::{stopping_error, StopSignal},
};

/// Unlike an async request guard, this guard remains in the real worker after
/// its waiter is cancelled. Handoff cannot export state before that work ends.
pub(super) async fn run_tracked<T, F>(
    state: &super::server::AppState,
    operation: F,
) -> Result<T, ServiceError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ServiceError> + Send + 'static,
{
    let work = state.blocking_work();
    run(
        state.stop.clone(),
        state.options.request_timeout,
        move || {
            let _work = work;
            operation()
        },
    )
    .await
}

pub(super) async fn run_prepared<T, F>(
    state: &super::server::AppState,
    operation: F,
) -> Result<T, ServiceError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ServiceError> + Send + 'static,
{
    let work = state.blocking_work();
    let budget =
        super::execution::remaining_prepare_budget().unwrap_or(state.options.request_timeout);
    run(state.stop.clone(), budget, move || {
        let _work = work;
        operation()
    })
    .await
}

pub(super) async fn run<T, F>(
    stop: StopSignal,
    budget: Duration,
    operation: F,
) -> Result<T, ServiceError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ServiceError> + Send + 'static,
{
    static POOL: OnceLock<Arc<Semaphore>> = OnceLock::new();
    run_with_pool(
        POOL.get_or_init(|| Arc::new(Semaphore::new(8))).clone(),
        stop,
        budget,
        operation,
    )
    .await
}

async fn run_with_pool<T, F>(
    pool: Arc<Semaphore>,
    mut stop: StopSignal,
    budget: Duration,
    operation: F,
) -> Result<T, ServiceError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ServiceError> + Send + 'static,
{
    let deadline = tokio::time::Instant::now() + budget;
    let permit = tokio::select! {
        _ = stop.wait() => return Err(stopping_error()),
        result = tokio::time::timeout_at(deadline, pool.acquire_owned()) => result
            .map_err(|_| ServiceError::new(503, "blocking_capacity_timeout", "Native API state workers are busy"))?
            .map_err(|_| stopping_error())?,
    };
    let task = tokio::task::spawn_blocking(move || {
        // This permit belongs to the real worker, not its cancellable async waiter.
        let _permit = permit;
        operation()
    });
    tokio::select! {
        _ = stop.wait() => Err(stopping_error()),
        result = tokio::time::timeout_at(deadline, task) => result
            .map_err(|_| ServiceError::new(503, "blocking_operation_timeout", "Native API state operation timed out"))?
            .map_err(|_| ServiceError::new(500, "blocking_operation_failed", "Native API state operation failed"))?,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_waiter_keeps_worker_capacity_reserved_until_real_completion() {
        let pool = Arc::new(Semaphore::new(1));
        let (_stop, receiver) = tokio::sync::watch::channel(false);
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let task = tokio::spawn(run_with_pool(
            pool.clone(),
            StopSignal(receiver),
            Duration::from_secs(5),
            move || {
                let _ = entered.send(());
                released.recv_timeout(Duration::from_secs(3)).unwrap();
                Ok(())
            },
        ));
        ready.await.unwrap();
        task.abort();
        let _ = task.await;
        assert_eq!(pool.available_permits(), 0);
        release.send(()).unwrap();
        let permit = tokio::time::timeout(Duration::from_secs(2), pool.acquire_owned())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
    }
}
