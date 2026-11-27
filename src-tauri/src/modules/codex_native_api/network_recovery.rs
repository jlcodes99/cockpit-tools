//! A request-local fence for repeated transport failures before response headers.
//! Successful streams retain their own idle limits and have no new total timer.
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use tokio::{sync::watch, time::Instant};

use super::contract::{ExecutionPolicy, ServiceError};

pub(super) const CODE: &str = "network_recovery_exhausted";
pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
pub(super) const DEFAULT_MAX_FAILURES: u8 = 4;

#[derive(Clone, Copy, Default)]
struct State {
    deadline: Option<Instant>,
    failures: u32,
    exhausted: bool,
    last_code: Option<&'static str>,
}

#[derive(Clone)]
pub(super) struct Budget {
    state: watch::Sender<State>,
    request_bytes: Arc<AtomicU64>,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            state: watch::channel(State::default()).0,
            request_bytes: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Budget {
    pub(super) fn observe_request_bytes(&self, bytes: u64) {
        self.request_bytes.fetch_max(bytes, Ordering::Relaxed);
    }

    pub(super) fn request_bytes(&self) -> u64 {
        self.request_bytes.load(Ordering::Relaxed)
    }

    /// Actual HTTP headers end a continuous transport-failure episode. In
    /// particular, a later quota/auth/capacity reply must retain its own policy
    /// instead of inheriting an old network timer.
    pub(super) fn recovered(&self) {
        self.state.send_replace(State::default());
    }

    /// Count typed transport failures only. A quota/auth/server status does not
    /// arm the network timer or consume its failure count. Refreshing a route
    /// must never replace this request-local state.
    pub(super) fn observe(&self, error: &ServiceError, policy: &ExecutionPolicy) {
        if error.transport_failure_kind().is_none() {
            return;
        }
        self.state.send_modify(|state| {
            let timeout = if policy.network_recovery_timeout.is_zero() {
                DEFAULT_TIMEOUT
            } else {
                policy.network_recovery_timeout
            };
            state
                .deadline
                .get_or_insert_with(|| Instant::now() + timeout);
            state.failures = state.failures.saturating_add(1);
            state.last_code = Some(error.code);
            state.exhausted |=
                state.failures >= u32::from(policy.network_recovery_max_failures.max(1));
        });
    }

    pub(super) fn check(&self) -> Result<(), ServiceError> {
        let state = *self.state.borrow();
        if state.exhausted || state.deadline.is_some_and(|at| Instant::now() >= at) {
            Err(self.error())
        } else {
            Ok(())
        }
    }

    /// The watcher is outside send_inner's mutable tracker borrow. It also
    /// bounds a stalled route refresh/backoff after the first network failure.
    /// Dropping it after headers succeed never cancels an active response body.
    pub(super) async fn wait(&self) {
        let mut changes = self.state.subscribe();
        loop {
            let state = *changes.borrow_and_update();
            if state.exhausted {
                return;
            }
            match state.deadline {
                Some(at) => tokio::select! {
                    _ = tokio::time::sleep_until(at) => {
                        // Headers can reset the episode in the same tick that
                        // this old timer becomes ready. Recheck the live state.
                        if self.check().is_err() { return; }
                    },
                    result = changes.changed() => if result.is_err() { return; },
                },
                None => {
                    if changes.changed().await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    pub(super) fn error(&self) -> ServiceError {
        let state = *self.state.borrow();
        let request_bytes = self.request_bytes();
        let locale = crate::modules::config::get_user_config().language;
        let large_timeout = request_bytes >= super::connection_error::LARGE_REQUEST_BYTES
            && (matches!(
                state.last_code,
                Some("upstream_connect_timeout" | "upstream_timeout" | "upstream_idle_timeout")
            ) || state.deadline.is_some_and(|at| Instant::now() >= at));
        let message = if large_timeout {
            let size = super::connection_error::format_request_size_mb(request_bytes);
            crate::modules::i18n::translate(
                &locale,
                "common.networkErrors.recoveryExhaustedLargeRequest",
                &[("size", &size)],
            )
        } else {
            crate::modules::i18n::translate(
                &locale,
                "common.networkErrors.recoveryExhausted",
                &[],
            )
        };
        let mut details = serde_json::json!({
            "type": "server_error", "code": CODE, "retryable": false,
            "transport_failures": state.failures, "last_error_code": state.last_code,
        });
        if request_bytes >= super::connection_error::LARGE_REQUEST_BYTES {
            details["request_bytes"] = serde_json::json!(request_bytes);
        }
        ServiceError::new(504, CODE, message).with_error_details(details)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network() -> ServiceError {
        ServiceError::new(504, "upstream_connect_timeout", "synthetic timeout")
    }

    #[tokio::test]
    async fn network_recovery_is_unarmed_for_healthy_and_quota_requests() {
        let budget = Budget::default();
        budget.observe(
            &ServiceError::new(429, "usage_limit_reached", "quota"),
            &ExecutionPolicy::default(),
        );
        budget.observe(
            &ServiceError::new(401, "invalid_token", "auth"),
            &ExecutionPolicy::default(),
        );
        budget.observe(
            &ServiceError::new(503, "server_is_overloaded", "capacity"),
            &ExecutionPolicy::default(),
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), budget.wait())
                .await
                .is_err()
        );
        assert!(budget.check().is_ok());
    }

    #[tokio::test]
    async fn network_recovery_shared_deadline_does_not_renew() {
        let policy = ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(40),
            ..Default::default()
        };
        let budget = Budget::default();
        let watcher = budget.clone();
        budget.observe(&network(), &policy);
        let first = budget.state.borrow().deadline;
        budget.observe(&network(), &policy);
        assert_eq!(first, budget.state.borrow().deadline);
        tokio::time::timeout(Duration::from_secs(1), watcher.wait())
            .await
            .unwrap();
        assert_eq!(budget.check().unwrap_err().code, CODE);
        assert!(Budget::default().check().is_ok());
    }

    #[tokio::test]
    async fn network_recovery_failure_cap_wakes_existing_waiter() {
        let policy = ExecutionPolicy {
            network_recovery_max_failures: 2,
            ..Default::default()
        };
        let budget = Budget::default();
        budget.observe(&network(), &policy);
        assert!(budget.check().is_ok());
        budget.clone().observe(&network(), &policy);
        tokio::time::timeout(Duration::from_secs(1), budget.wait())
            .await
            .unwrap();
        assert_eq!(budget.check().unwrap_err().code, CODE);
    }

    #[tokio::test]
    async fn network_recovery_headers_clear_expired_state_before_next_episode() {
        let policy = ExecutionPolicy { network_recovery_max_failures: 1, ..Default::default() };
        let budget = Budget::default();
        budget.observe(&network(), &policy);
        assert!(budget.check().is_err());
        budget.recovered();
        assert!(budget.check().is_ok());
        assert!(tokio::time::timeout(Duration::from_millis(20), budget.wait()).await.is_err());
        budget.observe(&network(), &policy);
        assert_eq!(budget.error().response_error.unwrap()["transport_failures"], 1);
    }

    #[test]
    fn network_recovery_images_keep_their_longer_opening_budget() {
        let policy = ExecutionPolicy::default().for_images();
        assert!(policy.network_recovery_timeout >= policy.image_open_timeout);
        assert_eq!(policy.total_timeout, Duration::ZERO);
    }

    fn poll_wait(wait: std::pin::Pin<&mut impl std::future::Future<Output = ()>>) -> bool {
        let waker = futures_util::task::noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        wait.poll(&mut context).is_ready()
    }

    #[tokio::test]
    async fn network_recovery_headers_win_when_old_timer_and_reset_are_both_ready() {
        let policy = ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(20),
            network_recovery_max_failures: 2,
            ..Default::default()
        };
        let budget = Budget::default();
        budget.observe(&network(), &policy);
        let old_deadline = budget.state.borrow().deadline.unwrap();
        let mut wait = Box::pin(budget.wait());
        // Register the old timer, but keep control of when this future is
        // polled again. Timer wakeups cannot run it ahead of recovered().
        assert!(!poll_wait(wait.as_mut()));
        tokio::time::sleep_until(old_deadline + Duration::from_millis(1)).await;
        budget.recovered();
        assert!(budget.check().is_ok());
        assert!(!poll_wait(wait.as_mut()), "old timer must not end a recovered episode");

        // The same waiter must still observe a later independent episode.
        budget.observe(&network(), &policy);
        assert!(!poll_wait(wait.as_mut()));
        budget.observe(&network(), &policy);
        assert!(poll_wait(wait.as_mut()));
    }

    #[tokio::test]
    async fn network_recovery_stale_timer_rechecks_live_state_without_select_luck() {
        let policy = ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(20),
            network_recovery_max_failures: 2,
            ..Default::default()
        };
        let budget = Budget::default();
        budget.observe(&network(), &policy);
        let old_deadline = budget.state.borrow().deadline.unwrap();
        let mut wait = Box::pin(budget.wait());
        assert!(!poll_wait(wait.as_mut()));
        tokio::time::sleep_until(old_deadline + Duration::from_millis(1)).await;

        // Force the timer side of the simultaneous-readiness race. Change
        // only the private test state without waking the watch branch, so a
        // randomly preferred reset notification cannot hide a stale return.
        // The preceding test exercises the actual recovered() notification.
        assert!(!budget.state.send_if_modified(|state| {
            *state = State::default();
            false
        }));
        assert!(budget.check().is_ok());
        assert!(!poll_wait(wait.as_mut()), "an obsolete timer must recheck live state");
        budget.observe(&network(), &policy);
        budget.observe(&network(), &policy);
        assert!(poll_wait(wait.as_mut()));
    }

    #[tokio::test]
    async fn network_recovery_large_request_timeout_reports_size_and_survives_recovered() {
        let policy = ExecutionPolicy {
            network_recovery_max_failures: 1,
            ..Default::default()
        };
        let large_bytes = 110 * 1024 * 1024;
        let locale = crate::modules::config::get_user_config().language;

        let budget = Budget::default();
        budget.observe_request_bytes(large_bytes);
        budget.recovered();
        assert_eq!(budget.request_bytes(), large_bytes);

        budget.observe(&network(), &policy);
        let error = budget.check().unwrap_err();
        assert_eq!(error.code, CODE);
        assert_eq!(
            error.message,
            crate::modules::i18n::translate(
                &locale,
                "common.networkErrors.recoveryExhaustedLargeRequest",
                &[("size", "110.0 MB")],
            )
        );
        assert_eq!(error.response_error.as_ref().unwrap()["request_bytes"], large_bytes);

        let small = Budget::default();
        small.observe_request_bytes(1024);
        small.observe(&network(), &policy);
        let small_error = small.check().unwrap_err();
        assert_eq!(
            small_error.message,
            crate::modules::i18n::translate(&locale, "common.networkErrors.recoveryExhausted", &[])
        );
        assert!(small_error.response_error.as_ref().unwrap()["request_bytes"].is_null());

        let connection = Budget::default();
        connection.observe_request_bytes(large_bytes);
        connection.observe(
            &ServiceError::new(502, "upstream_unavailable", "connect failed").with_upstream_not_sent(true),
            &ExecutionPolicy {
                network_recovery_max_failures: 1,
                ..Default::default()
            },
        );
        let conn_error = connection.error();
        assert_eq!(
            conn_error.message,
            crate::modules::i18n::translate(&locale, "common.networkErrors.recoveryExhausted", &[])
        );
    }
}
