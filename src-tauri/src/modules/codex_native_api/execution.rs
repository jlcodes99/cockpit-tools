//! Per-request preparation budgets. Body reads and authentication keep their
//! own short limits; selecting and preparing an account use execution limits.
use crate::modules::codex_native_api::deadline::{self, Deadline};
use std::{future::Future, time::Duration};

use tokio::time::Instant;

use super::{
    contract::{Caller, ExecutionPolicy, Route, RouteRequest, ServiceError},
    lifecycle::{stopping_error, RequestTracker},
    server::AppState,
};

#[derive(Clone)]
pub(super) struct Execution {
    pub policy: ExecutionPolicy,
    pub total_deadline: Deadline,
    pub open_deadline: Instant,
}

tokio::task_local! {
    static CURRENT: Execution;
}

impl Execution {
    pub(super) fn new(tracker: &RequestTracker, policy: &ExecutionPolicy) -> Self {
        Self::from_deadline(policy.clone(), tracker.deadline(policy.total_timeout))
    }

    pub(super) fn from_deadline(policy: ExecutionPolicy, total_deadline: impl Into<Deadline>) -> Self {
        let total_deadline = total_deadline.into();
        let open_deadline = total_deadline.cap(Instant::now() + policy.open_timeout);
        Self {
            policy,
            total_deadline,
            open_deadline,
        }
    }

    pub(super) fn prepare_deadline(&self) -> Instant {
        self.total_deadline.cap(self.open_deadline)
    }

    /// A new opening attempt receives another open budget, never another total
    /// budget. Initial selection and the first send retain their shared budget.
    pub(super) fn renew_open(&self) -> Self {
        Self::from_deadline(self.policy.clone(), self.total_deadline)
    }

    pub(super) fn run<F: Future>(&self, future: F) -> impl Future<Output = F::Output> {
        // Construct the scope synchronously: an async fn would retain the large
        // input F in its initial state even if it boxed F on its first poll.
        // Boxing at this boundary keeps protocol branches off each parent
        // future's inline stack layout, without spawning or changing cancellation.
        CURRENT.scope(self.clone(), Box::pin(future))
    }

    fn timeout_error(&self) -> ServiceError {
        if Instant::now() >= self.total_deadline {
            super::retry::total_timeout()
        } else {
            ServiceError::new(
                504,
                "upstream_connect_timeout",
                "Upstream preparation exceeded the opening time limit",
            )
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use axum::extract::{Request, State};
    use std::sync::Arc;

    fn entry_size<F: Future>(_: impl FnOnce(State<Arc<AppState>>, Request) -> F) -> usize {
        std::mem::size_of::<F>()
    }

    #[test]
    fn protocol_entry_futures_fit_the_default_runtime_stack() {
        for (name, bytes) in [
            ("responses", entry_size(super::super::http::responses)),
            ("compact", entry_size(super::super::http::compact)),
            ("auxiliary_http", entry_size(super::super::auxiliary::http)),
            (
                "auxiliary_websocket",
                entry_size(super::super::auxiliary::websocket),
            ),
            (
                "images_generations",
                entry_size(super::super::images::generations),
            ),
            ("images_edits", entry_size(super::super::images::edits)),
        ] {
            eprintln!("native future layout {name}: {bytes} bytes");
            assert!(
                bytes < 256 * 1024,
                "{name} inline future is {bytes} bytes; protocol branches must be heap-pinned"
            );
        }
    }
}

pub(super) fn current() -> Option<Execution> {
    CURRENT.try_with(Clone::clone).ok()
}

pub(super) fn policy() -> Option<ExecutionPolicy> {
    current().map(|execution| execution.policy)
}

pub(super) async fn current_or_new(state: &AppState) -> Result<Execution, ServiceError> {
    if let Some(execution) = current() {
        return Ok(execution);
    }
    let policy = super::retry::execution_policy(state).await?;
    let deadline = Deadline::after(policy.total_timeout);
    Ok(Execution::from_deadline(policy, deadline))
}

pub(super) fn within<'a, T: 'a>(
    state: &'a AppState,
    future: impl Future<Output = Result<T, ServiceError>> + 'a,
) -> impl Future<Output = Result<T, ServiceError>> + 'a {
    // Box before constructing the wrapper. Nested retry preparation otherwise
    // retains and copies every large child future into each timeout layer.
    let future = Box::pin(future);
    async move {
        let execution = current_or_new(state).await?;
        let mut stop = state.stop.clone();
        tokio::select! {
            _ = stop.wait() => Err(stopping_error()),
            result = deadline::timeout_at(execution.prepare_deadline(), future) =>
                result.map_err(|_| execution.timeout_error())?,
        }
    }
}

pub(super) fn prepare<'a, T: 'a>(
    state: &'a AppState,
    future: impl Future<Output = Result<T, ServiceError>> + 'a,
) -> impl Future<Output = Result<T, ServiceError>> + 'a {
    within(state, future)
}

pub(super) async fn total<T>(
    state: &AppState,
    future: impl Future<Output = Result<T, ServiceError>>,
) -> Result<T, ServiceError> {
    let execution = current_or_new(state).await?;
    let mut stop = state.stop.clone();
    tokio::select! {
        _ = stop.wait() => Err(stopping_error()),
        result = deadline::timeout_at(execution.total_deadline, future) =>
            result.map_err(|_| super::retry::total_timeout())?,
    }
}

pub(super) async fn route(
    state: &AppState,
    caller: &Caller,
    request: RouteRequest,
) -> Result<Route, ServiceError> {
    within(state, state.host.route(caller, request)).await
}

/// Native host async callbacks run in the request's task. Blocking workers do
/// not inherit this scope and retain their existing tracked cancellation fence.
pub(crate) fn remaining_prepare_budget() -> Option<Duration> {
    current().map(|execution| {
        execution
            .prepare_deadline()
            .saturating_duration_since(Instant::now())
    })
}
