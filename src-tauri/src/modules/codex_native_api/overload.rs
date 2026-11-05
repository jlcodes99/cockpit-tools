//! Replay-safe capacity recovery shared by all transports in a turn.
use std::time::Duration;

use serde_json::Value;

use super::{
    contract::{ExecutionPolicy, ServiceError, WireApi},
    deadline::{self, Deadline},
    lifecycle::{stopping_error, RequestTracker},
    mapping::{self, Prepared},
    server::AppState,
};

pub(super) const MAX_RETRIES: u8 = 2;
static ACTIVE_RECOVERIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn active_recoveries() -> usize {
    ACTIVE_RECOVERIES.load(std::sync::atomic::Ordering::Relaxed)
}

struct ActiveRecovery;

impl Drop for ActiveRecovery {
    fn drop(&mut self) {
        ACTIVE_RECOVERIES.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}
pub(super) const RECOVERY_WINDOW: Duration = Duration::from_secs(15);

pub(super) fn capacity_code(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "server_is_overloaded"
            | "server_overloaded"
            | "service_unavailable_error"
            | "flex_unavailable"
            | "queue_full"
            | "model_at_capacity"
            | "model_is_at_capacity"
    )
}

pub(super) fn event(event: &Value) -> bool {
    let error = super::diagnostics::error_object(event);
    // Explicit quota/auth failures outrank a contradictory generic busy type.
    if [error.get("code"), error.get("type"), event.get("code")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|code| {
            matches!(
                code,
                "usage_limit_reached"
                    | "insufficient_quota"
                    | "quota_exceeded"
                    | "rate_limit_exceeded"
                    | "subscription_sharing_usage_limit_exceeded"
                    | "account_deactivated"
                    | "account_disabled"
                    | "account_suspended"
                    | "organization_deactivated"
                    | "invalid_api_key"
                    | "invalid_token"
            )
        })
    {
        return false;
    }
    let capacity = [error.get("code"), error.get("type"), event.get("code")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(capacity_code)
        || error
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| {
                message
                    .to_ascii_lowercase()
                    .contains("model is at capacity")
            });
    capacity && !super::upstream_errors::request_fault(503, event)
}

pub(super) fn failure(error: &ServiceError) -> bool {
    capacity_code(error.code) || error.response_error.as_ref().is_some_and(event)
}

#[derive(Default)]
pub(super) struct Budget {
    pub retries: u32,
    active: Option<ActiveRecovery>,
    pub started: Option<tokio::time::Instant>,
}

impl Budget {
    pub(super) fn recovered(&mut self) {
        self.active = None;
    }

    pub(super) fn continuous(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn deadline(&self, deadline: Deadline) -> Deadline {
        if self.active.is_some() {
            return deadline;
        }
        self.started
            .map_or(deadline, |start| deadline.min(start + RECOVERY_WINDOW))
    }
}

/// Caller must prove an explicit rejection before any model/tool execution.
/// Reselect the same owner through the host so revocations and concurrency still apply.
pub(super) fn retry_same<'a>(
    state: &'a AppState,
    prepared: &'a mut Prepared,
    tracker: &'a mut RequestTracker,
    wait: Option<Duration>,
    deadline: Deadline,
    policy: &'a ExecutionPolicy,
) -> futures_util::future::BoxFuture<'a, Result<bool, ServiceError>> {
    Box::pin(async move {
        let continuous = policy.continuous_overload_retry;
        let maximum_wait = policy.retry_max_delay;
        if prepared.compact
            || prepared.raw_chat.is_some()
            || prepared.image_route_request.is_some()
            || prepared.route.chatgpt_plan
            || prepared.route.wire_api != WireApi::Responses
            || prepared.request.capability_fallback_from.is_some()
            || (!continuous && tracker.overload.retries >= u32::from(MAX_RETRIES))
        {
            return Ok(false);
        }
        let start = *tracker
            .overload
            .started
            .get_or_insert_with(tokio::time::Instant::now);
        if continuous && !tracker.overload.continuous() {
            ACTIVE_RECOVERIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracker.overload.active = Some(ActiveRecovery);
        }
        let deadline = tracker.overload.deadline(deadline);
        let backoff = Duration::from_millis(300 * (1u64 << tracker.overload.retries.min(8)));
        let jitter = Duration::from_millis(u64::from(rand::random::<u8>()) % 151);
        if !continuous && (maximum_wait.is_zero() || wait.is_some_and(|hint| hint > maximum_wait)) {
            return Ok(false);
        }
        let maximum_wait = if continuous {
            maximum_wait.max(Duration::from_millis(300))
        } else {
            maximum_wait
        };
        let wait = wait
            .unwrap_or_default()
            .max((backoff + jitter).min(maximum_wait));
        if deadline
            .instant()
            .is_some_and(|end| wait >= end.saturating_duration_since(tokio::time::Instant::now()))
        {
            return Ok(false);
        }
        tracker.release_route();
        prepared.route.permit = None;
        let mut stop = state.stop.clone();
        let result = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            result = deadline::timeout_at(deadline, async {
                tokio::time::sleep(wait).await;
                deadline::timeout_at(deadline.cap(tokio::time::Instant::now() + state.options.request_timeout),
                    mapping::refresh_transport(state, prepared)).await
            }) => result,
        };
        match result {
            Ok(Ok(Ok(()))) => {
                if continuous {
                    // A delayed retry starts a fresh pool round. Only transient
                    // per-turn exclusions are cleared; the host rechecks eligibility.
                    prepared.request.excluded_account_ids.retain(|id| tracker.quota_exhausted_accounts.contains(id));
                }
                tracker.overload.retries = tracker.overload.retries.saturating_add(1);
                tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
                tracker.routing_identity(&prepared.request);
                tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                tracker.prepared_metadata(prepared);
                tracker.overload_retry(wait, start.elapsed());
                Ok(true)
            }
            // Keep the original upstream error when local preparation cannot recover.
            _ if state.stop.stopped() => Err(stopping_error()),
            Ok(Ok(Err(error))) if continuous => Err(error),
            _ => Ok(false),
        }
    })
}
