//! One logical request keeps the same owner, body and finite retry budget.
use super::super::super::{overload as capacity, server::Handle};
use super::*;
use futures_util::StreamExt;

struct SingleHost {
    inner: FixtureHost,
    revoke_on_refresh: bool,
    allow_alternates: bool,
    alternates_after_refresh: bool,
    refreshes: AtomicUsize,
    rejected: tokio::sync::Notify,
    overload_attempts: Mutex<Vec<(String, u32)>>,
    overload_outcomes: Mutex<Vec<String>>,
}
impl Host for SingleHost {
    fn authorize<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(token)
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        mut request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let alternatives_allowed = self.allow_alternates || self.alternates_after_refresh && self.refreshes.load(Ordering::SeqCst) > 0;
            if !alternatives_allowed && request.excluded_account_ids.iter().any(|id| id == "0") {
                return Err(ServiceError::new(
                    503,
                    "no_available_account",
                    "Only one permitted account",
                ));
            }
            if request.preferred_account_id.is_some() {
                self.refreshes.fetch_add(1, Ordering::SeqCst);
                if self.revoke_on_refresh {
                    return Err(ServiceError::new(
                        403,
                        "account_not_allowed",
                        "Access revoked",
                    ));
                }
            }
            if !alternatives_allowed { request.preferred_account_id = Some("0".into()); }
            self.inner.route(caller, request).await
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        self.inner.execution_policy()
    }
    fn route_failed<'a>(
        &'a self,
        id: &'a str,
        status: u16,
        retry: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        self.inner.route_failed(id, status, retry)
    }
    fn overload_attempt(&self, _request_id: &str, attempt: &super::super::super::diagnostics::DiagnosticAttempt) {
        self.overload_attempts.lock().unwrap().push((attempt.account_id.clone(), attempt.sequence));
    }
    fn overload_outcome(&self, _request_id: &str, outcome: &str, _account_id: Option<&str>) {
        self.overload_outcomes.lock().unwrap().push(outcome.to_owned());
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(report)
    }
}

struct Scenario {
    error: Value,
    prefix: Vec<Value>,
    retry_prefix: Option<Vec<Value>>,
    failures: usize,
    delayed: bool,
    revoke: bool,
    total_timeout: Duration,
    hang_retry: bool,
    http_status: u16,
    status_retries: u8,
    continuous: bool,
    allow_alternates: bool,
    alternates_after_refresh: bool,
}
impl Default for Scenario {
    fn default() -> Self {
        Self {
            error: overload(),
            prefix: vec![created()],
            retry_prefix: None,
            failures: 1,
            delayed: true,
            revoke: false,
            total_timeout: Duration::from_secs(5),
            hang_retry: false,
            http_status: 200,
            status_retries: 0,
            continuous: false,
            allow_alternates: false,
            alternates_after_refresh: false,
        }
    }
}
async fn fixture(
    case: Scenario,
) -> (
    Arc<SingleHost>,
    Handle,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(SingleHost {
        inner: FixtureHost {
            base: format!("http://{}", listener.local_addr().unwrap()),
            policy: ExecutionPolicy {
                status_retries: case.status_retries,
                continuous_overload_retry: case.continuous,
                credential_retries: 0,
                total_timeout: case.total_timeout,
                ..Default::default()
            },
            failing: 0,
            before_error: vec![],
            recover_on_same: false,
            delayed_opening: 0,
            release_error: tokio::sync::Notify::new(),
            calls: Default::default(),
            requests: Mutex::new(vec![]),
            failures: Mutex::new(vec![]),
            reports: Mutex::new(vec![]),
        },
        revoke_on_refresh: case.revoke,
        allow_alternates: case.allow_alternates,
        alternates_after_refresh: case.alternates_after_refresh,
        refreshes: AtomicUsize::new(0),
        rejected: tokio::sync::Notify::new(),
        overload_attempts: Mutex::new(Vec::new()),
        overload_outcomes: Mutex::new(Vec::new()),
    });
    let upstream_host = host.clone();
    let app = Router::new().fallback(
        move |uri: axum::extract::OriginalUri, headers: HeaderMap, axum::Json(body): axum::Json<Value>| {
            let h = upstream_host.clone();
            let failure = case.error.clone();
            let prefix = case
                .prefix
                .iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            let retry_prefix = case.retry_prefix.as_ref().map(|events| events.iter().map(|event| format!("data: {event}\n\n")).collect::<String>());
            async move {
                let index: usize = uri.0.path().split('/').nth(1).unwrap().parse().unwrap();
                let attempt = h.inner.calls[index].fetch_add(1, Ordering::SeqCst);
                let prefix = if attempt > 0 { retry_prefix.unwrap_or(prefix) } else { prefix };
                h.inner.requests.lock().unwrap().push((index, headers, body));
                if index == 0 && attempt < case.failures && case.http_status != 200 {
                    return Response::builder().status(case.http_status).header("content-type", "application/json")
                        .body(Body::from(failure.to_string())).unwrap();
                }
                let (prefix, tail) = if index == 0 && attempt < case.failures {
                    (prefix, format!("data: {failure}\n\n"))
                } else {
                    let mut started = created();
                    if index > 0 { started["response"]["id"] = json!(format!("account-{index}-response")); }
                    (
                        format!("data: {started}\n\n"),
                        format!("data: {}\n\n", completed()),
                    )
                };
                let chunks =
                    stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(prefix)) })
                        .chain(stream::once(async move {
                            if case.hang_retry && attempt > 0 {
                                std::future::pending::<()>().await;
                            }
                            if case.delayed {
                                tokio::time::sleep(Duration::from_millis(400)).await;
                            }
                            h.rejected.notify_one();
                            Ok::<_, std::io::Error>(Bytes::from(tail))
                        }));
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(chunks))
                    .unwrap()
            }
        },
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let temp = tempfile::tempdir().unwrap();
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        temp.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    (host, handle, server, temp)
}
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
fn input(streaming: bool) -> Value {
    json!({"model":"model","stream":streaming,
        "instructions":"private sizing instructions", "tools":[],
        "input":[{"role":"user","content":"private sizing input 中文"}]})
}

#[tokio::test]
async fn single_account_capacity_recovers_before_and_after_metadata_visibility() {
    for streaming in [false, true] {
        for delayed in [false, true] {
            let (host, mut handle, task, _temp) = fixture(Scenario {
                delayed,
                ..Default::default()
            })
            .await;
            let body = input(streaming);
            let request = client()
                .post(format!("http://{}/v1/responses", handle.addr))
                .bearer_auth("fixture");
            let request = if delayed {
                use std::io::Write;
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                encoder
                    .write_all(&serde_json::to_vec(&body).unwrap())
                    .unwrap();
                request
                    .header("content-type", "application/json")
                    .header("content-encoding", "gzip")
                    .body(encoder.finish().unwrap())
            } else {
                request.json(&body)
            };
            let response = request.send().await.unwrap();
            assert_eq!(response.status().as_u16(), 200);
            let text = response.text().await.unwrap();
            assert!(text.contains("completed"), "{text}");
            assert!(!text.contains("server_is_overloaded"), "{text}");
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
            task.abort();
            assert_eq!(host.inner.calls[0].load(Ordering::SeqCst), 2);
            assert_eq!(host.refreshes.load(Ordering::SeqCst), 1);
            let sent = host.inner.requests.lock().unwrap();
            assert_eq!(
                sent[0].2, sent[1].2,
                "same-owner request body must be retained verbatim"
            );
            assert_eq!(sent[0].1["session-id"], sent[1].1["session-id"]);
            let reports = host.inner.reports.lock().unwrap();
            assert_eq!(reports.len(), 1);
            assert!(reports[0].success);
            let context = reports[0].diagnostics.as_ref().unwrap();
            assert_eq!(context.attempts.len(), 2);
            assert_eq!(
                context.attempts[0].upstream_request_bytes,
                Some(serde_json::to_vec(&sent[0].2).unwrap().len() as u64)
            );
            assert_eq!(
                context.attempts[0].upstream_request_bytes,
                context.attempts[1].upstream_request_bytes
            );
            assert!(context.payloads.is_empty());
            assert_eq!(
                context.attempts[0]
                    .request_size
                    .as_ref()
                    .unwrap()
                    .total_bytes,
                serde_json::to_vec(&body).unwrap().len() as u64
            );
            assert_eq!(
                context.attempts[1].overload_retry.as_ref().unwrap().retry,
                1
            );
            assert!(!serde_json::to_string(context)
                .unwrap()
                .contains("private sizing"));
        }
    }
}

#[tokio::test]
async fn capacity_exhaustion_keeps_error_and_never_restarts_budget_after_metadata() {
    let (host, mut handle, task, _temp) = fixture(Scenario {
        failures: usize::MAX,
        ..Default::default()
    })
    .await;
    let text = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&input(true))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(text.contains("server_is_overloaded"), "{text}");
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    assert_eq!(
        host.inner.calls[0].load(Ordering::SeqCst),
        1 + usize::from(capacity::MAX_RETRIES)
    );
    assert_eq!(host.inner.reports.lock().unwrap().len(), 1);
    assert!(!host.inner.reports.lock().unwrap()[0].success);
}

#[tokio::test]
async fn capacity_does_not_replay_execution_input_faults_long_retry_after_or_revoked_access() {
    let mut cases = Vec::new();
    for kind in [
        "response.output_text.delta",
        "response.reasoning_text.delta",
        "response.function_call_arguments.delta",
    ] {
        cases.push(Scenario {
            prefix: vec![created(), json!({"type":kind,"delta":"executed"})],
            ..Default::default()
        });
    }
    cases.push(Scenario { error: json!({"type":"error","error":{"code":"context_length_exceeded","type":"service_unavailable_error"}}), ..Default::default() });
    cases.push(Scenario {
        error: json!({"type":"error","retry_after":30,"error":{"code":"queue_full"}}),
        ..Default::default()
    });
    cases.push(Scenario { error: json!({"type":"error","error":{"code":"quota_exceeded","type":"service_unavailable_error"}}), ..Default::default() });
    cases.push(Scenario {
        revoke: true,
        ..Default::default()
    });
    cases.push(Scenario {
        total_timeout: Duration::from_millis(600),
        ..Default::default()
    });
    for case in cases {
        let (host, mut handle, task, _temp) = fixture(case).await;
        let _ = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("fixture")
            .json(&input(true))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        assert_eq!(host.inner.calls[0].load(Ordering::SeqCst), 1);
        assert_eq!(host.inner.reports.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn disconnect_during_capacity_backoff_stops_upstream_retries() {
    let (host, mut handle, task, _temp) = fixture(Scenario::default()).await;
    let mut response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&input(true))
        .send()
        .await
        .unwrap();
    let first = response.chunk().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("response.created"));
    tokio::time::timeout(Duration::from_secs(1), host.rejected.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    drop(response);
    tokio::time::sleep(Duration::from_millis(700)).await;
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    assert_eq!(host.inner.calls[0].load(Ordering::SeqCst), 1);
}

#[test]
fn capacity_aliases_are_explicit_and_input_faults_take_precedence() {
    for code in [
        "server_is_overloaded",
        "server_overloaded",
        "model_at_capacity",
        "model_is_at_capacity",
        "flex_unavailable",
        "queue_full",
        "service_unavailable_error",
    ] {
        for wrapper in ["error", "response.failed", "response.error"] {
            let event = if wrapper == "response.failed" {
                json!({"type":wrapper,"response":{"error":{"code":code}}})
            } else {
                json!({"type":wrapper,"error":{"code":code}})
            };
            assert_eq!(rejection(&event).unwrap().0, if super::super::super::transient_rate_limit::applies(&event) { 429 } else { 503 }, "{event}");
        }
        let event = json!({"type":"error","error":{"code":code,"type":"invalid_request_error","message":"Context length exceeded"}});
        assert!(rejection(&event).is_none());
    }
    let mut budget = capacity::Budget::default();
    budget.retries = 2;
    budget.started = Some(tokio::time::Instant::now());
    assert!(budget
        .deadline(super::super::super::deadline::Deadline::NONE)
        .instant()
        .is_some());
}

#[tokio::test]
async fn capacity_recovery_window_bounds_a_stalled_retry_with_no_total_request_limit() {
    let (host, mut handle, task, _temp) = fixture(Scenario {
        total_timeout: Duration::ZERO,
        hang_retry: true,
        ..Default::default()
    })
    .await;
    let started = tokio::time::Instant::now();
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&input(true))
        .send()
        .await
        .unwrap();
    let text = tokio::time::timeout(
        capacity::RECOVERY_WINDOW + Duration::from_secs(3),
        response.text(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(text.contains("upstream_total_timeout"), "{text}");
    assert!(started.elapsed() < capacity::RECOVERY_WINDOW + Duration::from_secs(3));
    assert_eq!(host.inner.calls[0].load(Ordering::SeqCst), 2);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn shutdown_interrupts_capacity_backoff_without_another_send() {
    let (host, mut handle, task, _temp) = fixture(Scenario::default()).await;
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&input(true))
        .send()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), host.rejected.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    handle.shutdown(Duration::from_secs(1)).await.unwrap();
    drop(response);
    task.abort();
    assert_eq!(host.inner.calls[0].load(Ordering::SeqCst), 1);
}

#[path = "overload_safety_tests.rs"]
mod safety_tests;


#[tokio::test]
async fn continuous_capacity_hides_rejections_and_counts_every_upstream_attempt() {
    for (status, error) in [
        (200, overload()),
        (503, overload()),
        (429, json!({"error":{"code":"model_at_capacity","type":"rate_limit_error",
            "message":"Selected model is at capacity. Please try a different model."}})),
    ] {
        let (host, mut handle, task, _temp) = fixture(Scenario {
            continuous: true,
            failures: 4,
            http_status: status,
            error,
            delayed: false,
            total_timeout: Duration::from_secs(12),
            ..Default::default()
        }).await;
        let response = client().post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("fixture").json(&input(true)).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let text = response.text().await.unwrap();
        assert!(text.contains("completed"), "{text}");
        assert!(!text.contains("server_is_overloaded") && !text.contains("model_at_capacity"), "{text}");
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        assert_eq!(host.inner.calls[0].load(Ordering::SeqCst), 5);
        assert_eq!(*host.overload_attempts.lock().unwrap(),
            (1..=4).map(|number| ("0".to_owned(), number)).collect::<Vec<_>>());
        assert_eq!(*host.overload_outcomes.lock().unwrap(), ["recovered"]);
    }
}

#[tokio::test]
async fn continuous_capacity_cancels_backoff_when_client_disconnects() {
    let (host, mut handle, task, _temp) = fixture(Scenario {
        continuous: true,
        failures: usize::MAX,
        ..Default::default()
    }).await;
    let mut response = client().post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture").json(&input(true)).send().await.unwrap();
    assert!(response.chunk().await.unwrap().is_some());
    tokio::time::timeout(Duration::from_secs(1), host.rejected.notified()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    drop(response);
    tokio::time::sleep(Duration::from_millis(700)).await;
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    assert_eq!(host.inner.calls[0].load(Ordering::SeqCst), 1);
    assert_eq!(*host.overload_outcomes.lock().unwrap(), ["cancelled"]);
}
