use super::*;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::sync::Mutex;
use tokio::sync::Semaphore;

#[derive(Default)]
struct Recorder {
    quotas: Mutex<Vec<(String, super::super::rate_limits::QuotaSnapshot)>>,
    reports: Mutex<Vec<RequestReport>>,
    contexts: Mutex<Vec<DiagnosticContext>>,
    invalidations: Mutex<Vec<String>>,
    changed: Notify,
    quota_checks: Mutex<Vec<String>>,
    quota_denied: Mutex<Vec<String>>,
}

impl Host for Recorder {
    fn check_quota_before_send(&self, account_id: &str) -> Result<(), ServiceError> {
        self.quota_checks.lock().unwrap().push(account_id.into());
        if self.quota_denied.lock().unwrap().iter().any(|id| id == account_id) {
            return Err(ServiceError::new(429, "quota_reserved", "Synthetic reserve"));
        }
        Ok(())
    }

    fn invalidate_transport(&self, identity: &str) -> bool {
        self.invalidations.lock().unwrap().push(identity.to_owned());
        true
    }
    fn observe_rate_limits(&self, account_id: &str, snapshot: super::super::rate_limits::QuotaSnapshot,
        _: std::time::Instant) {
        self.quotas.lock().unwrap().push((account_id.to_owned(), snapshot));
    }
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async { unreachable!() })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        _: super::super::contract::RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async { unreachable!() })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { unreachable!() })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
            self.changed.notify_one();
        })
    }
    fn diagnostic(&self, context: DiagnosticContext) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.contexts.lock().unwrap().push(context);
        })
    }
}

include!("lifecycle_dispatch_tests.rs");

fn usage() -> Value {
    json!({"input_tokens": 17, "output_tokens": 23, "total_tokens": 40})
}

async fn fixture() -> (
    RequestTracker,
    Arc<Recorder>,
    Arc<Activity>,
    watch::Sender<bool>,
    Arc<Semaphore>,
) {
    let activity = Arc::new(Activity::default());
    let host = Arc::new(Recorder::default());
    let (stop, stopped) = watch::channel(false);
    let permit = Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap();
    let mut tracker =
        RequestTracker::new(activity.clone(), host.clone(), StopSignal(stopped), permit);
    tracker.context("POST", "/v1/messages", &axum::http::HeaderMap::new(), false);
    tracker.authorized(
        &Caller {
            id: "caller".into(),
            label: "test".into(),
        },
        "model",
    );
    tracker.begin_attempt("responses");
    tracker.first_response();
    tracker.response_tier(Some("priority"));
    let route_gate = Arc::new(Semaphore::new(1));
    tracker.route_permit = Some(Arc::new(route_gate.clone().acquire_owned().await.unwrap()));
    (tracker, host, activity, stop, route_gate)
}

async fn recorded(host: &Recorder) -> RequestReport {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let changed = host.changed.notified();
            if let Some(report) = host.reports.lock().unwrap().pop() {
                return report;
            }
            changed.await;
        }
    })
    .await
    .expect("report persisted without waiting for a live request")
}

#[tokio::test]
async fn native_lifecycle_quota_is_observed_before_completion_and_ignores_outgoing_headers() {
    let (mut tracker, host, _activity, _stop, _gate) = fixture().await;
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("x-codex-primary-used-percent", "89.5".parse().unwrap());
    tracker.quota_headers(&headers);
    assert!(host.quotas.lock().unwrap().is_empty(), "unrelated providers cannot update Codex quota");
    tracker.quota_account = Some("oauth-quota-fixture".into());
    tracker.upstream_headers(&headers);
    assert!(host.quotas.lock().unwrap().is_empty(), "request headers are not quota evidence");
    tracker.quota_headers(&headers);
    tracker.quota_event(&json!({"type":"codex.rate_limits","rate_limits":{
        "secondary":{"used_percent":75.25,"window_minutes":10080,"reset_at":5000}}}));
    let quotas = host.quotas.lock().unwrap();
    assert_eq!(quotas.len(), 2);
    assert_eq!(quotas[0].0, "oauth-quota-fixture");
    assert_eq!(quotas[0].1.primary.as_ref().unwrap().remaining_percent, 10.5);
    assert_eq!(quotas[1].1.secondary.as_ref().unwrap().remaining_percent, 24.75);
    assert!(host.reports.lock().unwrap().is_empty(), "quota delivery does not wait for request persistence");
}

#[tokio::test]
async fn native_lifecycle_downstream_success_is_deferred_without_holding_inference_capacity() {
    let (mut tracker, host, activity, _stop, gate) = fixture().await;
    let mut downstream = tracker.defer_downstream();
    tokio::time::timeout(
        Duration::from_millis(100),
        tracker.complete(200, true, Some(usage()), None),
    )
    .await
    .unwrap();
    assert_eq!(activity.count.load(Ordering::SeqCst), 0);
    assert_eq!(gate.available_permits(), 1);
    assert!(host.reports.lock().unwrap().is_empty());
    drop(tracker);
    assert!(activity.tasks.load(Ordering::SeqCst) > 0);
    downstream.success();
    downstream.fail(&ServiceError::new(
        502,
        "late_failure",
        "ignored after success",
    ));
    downstream.success();
    drop(downstream);
    let report = recorded(&host).await;
    assert_eq!((report.status, report.success), (200, true));
    assert_eq!(report.usage, Some(usage()));
    assert_eq!(report.response_service_tier.as_deref(), Some("priority"));
    assert_eq!(report.error, None);
}

#[tokio::test]
async fn native_lifecycle_conversion_failure_keeps_usage_tier_and_successful_upstream_attempt() {
    let (mut tracker, host, _, _stop, _) = fixture().await;
    let mut downstream = tracker.defer_downstream();
    tracker.complete(200, true, Some(usage()), None).await;
    let error = ServiceError::new(502, "invalid_upstream_response", "Cannot convert response")
        .with_retry_after(Some(Duration::from_secs(2)));
    downstream.fail(&error);
    downstream.success();
    let report = recorded(&host).await;
    assert_eq!((report.status, report.success), (502, false));
    assert_eq!(report.error.as_deref(), Some("invalid_upstream_response"));
    assert_eq!(
        report.error_message.as_deref(),
        Some("Cannot convert response")
    );
    assert_eq!(report.usage, Some(usage()));
    assert_eq!(report.response_service_tier.as_deref(), Some("priority"));
    assert_eq!(report.retry_after, Some(Duration::from_secs(2)));
    let diagnostic = report.diagnostics.unwrap();
    assert_eq!(diagnostic.failure_phase.as_deref(), Some("downstream"));
    assert_eq!(diagnostic.attempts[0].status, Some(200));
    assert!(diagnostic.attempts[0].success);
    drop(tracker);
}

#[tokio::test]
async fn native_lifecycle_transport_failure_records_sanitized_source_and_phase_with_usage() {
    let (mut tracker, host, _, _stop, _) = fixture().await;
    tracker.capture_usage(&Some(usage()));
    let error = ServiceError::new(
        502,
        "upstream_stream_error",
        "connection reset https://user:secret@example.invalid/path authorization: Bearer raw-token",
    );
    tracker.fail(&error).await;
    let report = recorded(&host).await;
    assert_eq!(report.status, 502);
    assert_eq!(report.usage, Some(usage()));
    let message = report.error_message.unwrap();
    assert!(message.contains("connection reset"));
    assert!(!message.contains("raw-token"));
    assert!(!message.contains("example.invalid"));
    let diagnostic = report.diagnostics.unwrap();
    assert_eq!(diagnostic.failure_phase.as_deref(), Some("stream_read"));
    assert_eq!(
        diagnostic.attempts[0].error_message.as_deref(),
        Some(message.as_str())
    );
    assert_eq!(
        diagnostic.attempts[0].failure_phase.as_deref(),
        Some("stream_read")
    );
}

#[tokio::test]
async fn native_lifecycle_downstream_drop_cancels_only_successful_upstream_requests() {
    for upstream_success in [true, false] {
        let (mut tracker, host, _, _stop, _) = fixture().await;
        let downstream = tracker.defer_downstream();
        tracker
            .complete(
                if upstream_success { 200 } else { 429 },
                upstream_success,
                Some(usage()),
                (!upstream_success).then(|| "upstream_rate_limited".into()),
            )
            .await;
        drop(downstream);
        let report = recorded(&host).await;
        assert!(!report.success);
        assert_eq!(report.status, if upstream_success { 499 } else { 429 });
        assert_eq!(
            report.error.as_deref(),
            Some(if upstream_success {
                "client_cancelled"
            } else {
                "upstream_rate_limited"
            })
        );
        assert_eq!(report.usage, Some(usage()));
        drop(tracker);
    }
}

#[tokio::test]
async fn native_lifecycle_stop_settles_pending_adapter_without_waiting_for_guard_drop() {
    let (mut tracker, host, _, stop, _) = fixture().await;
    let mut downstream = tracker.defer_downstream();
    tracker.complete(200, true, Some(usage()), None).await;
    drop(tracker);
    stop.send(true).unwrap();
    let report = recorded(&host).await;
    assert_eq!((report.status, report.success), (503, false));
    assert_eq!(report.error.as_deref(), Some("service_stopping"));
    assert_eq!(report.usage, Some(usage()));
    downstream.success();
    tokio::task::yield_now().await;
    assert!(host.reports.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_lifecycle_unsettled_adapter_has_a_bounded_deadline() {
    let (mut tracker, _, _, _stop, _) = fixture().await;
    let downstream = tracker.defer_downstream();
    let receiver = tracker.downstream.take();
    let mut report = tracker.report.take();
    report.as_mut().unwrap().status = 200;
    report.as_mut().unwrap().success = true;
    report.as_mut().unwrap().usage = Some(usage());
    let mut diagnostic = tracker.diagnostics.take();
    settle_downstream(
        &mut report,
        &mut diagnostic,
        receiver,
        tracker.stop.clone(),
        tracker.started,
        Duration::from_millis(10),
    )
    .await;
    let report = report.unwrap();
    assert_eq!((report.status, report.success), (504, false));
    assert_eq!(report.error.as_deref(), Some("downstream_outcome_timeout"));
    assert_eq!(report.usage, Some(usage()));
    drop(downstream);
    drop(tracker);
}

#[tokio::test]
async fn native_lifecycle_rebuild_abort_follows_attempt_failure_drop_and_success() {
    for outcome in ["failure", "drop", "success"] {
        let (mut tracker, _, _, _stop, _) = fixture().await;
        let first = Arc::new(AtomicBool::new(false));
        let next = Arc::new(AtomicBool::new(false));
        tracker.track_rebuild(Some(first.clone()));
        tracker.track_rebuild(Some(first.clone()));
        assert!(!first.load(Ordering::Acquire));
        tracker.track_rebuild(Some(next.clone()));
        assert!(
            first.load(Ordering::Acquire),
            "Replaced worker cannot activate its old candidate"
        );
        assert!(!next.load(Ordering::Acquire));
        match outcome {
            "failure" => {
                tracker
                    .complete(503, false, None, Some("synthetic_failure".into()))
                    .await
            }
            "success" => tracker.complete(200, true, None, None).await,
            _ => (),
        }
        drop(tracker);
        assert_eq!(next.load(Ordering::Acquire), outcome != "success");
    }
}
