use super::super::{contract::*, start, Options};
use super::*;
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
    routing::post,
    Router,
};
use futures_util::future::BoxFuture;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

fn overload() -> Value {
    json!({"type":"error","error":{"type":"service_unavailable_error","code":"server_is_overloaded","message":"Our servers are currently overloaded. Please try again later."}})
}
fn created() -> Value {
    json!({"type":"response.created","response":{"id":"temporary","status":"in_progress"}})
}
fn completed() -> Value {
    json!({"type":"response.completed","response":{"id":"done","status":"completed","output":[]}})
}

struct FixtureHost {
    base: String,
    policy: ExecutionPolicy,
    failing: usize,
    before_error: Vec<Value>,
    recover_on_same: bool,
    delayed_opening: u8,
    release_error: tokio::sync::Notify,
    calls: [AtomicUsize; 3],
    requests: Mutex<Vec<(usize, HeaderMap, Value)>>,
    failures: Mutex<Vec<(String, u16)>>,
    reports: Mutex<Vec<RequestReport>>,
}
impl Host for FixtureHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "caller".into(),
                label: "fixture".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let index = (0..3)
                .find(|index| {
                    let id = index.to_string();
                    request
                        .preferred_account_id
                        .as_ref()
                        .is_none_or(|preferred| preferred == &id)
                        && !request.excluded_account_ids.contains(&id)
                })
                .ok_or_else(|| ServiceError::new(503, "no_available_account", "Pool exhausted"))?;
            Ok(Route {
                continuity_account_id: None,
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                account_id: index.to_string(),
                owner_id: format!("owner-{index}"),
                base_url: format!("{}/{}", self.base, index),
                model: request.model,
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                headers: HeaderMap::new(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: false,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn route_failed<'a>(
        &'a self,
        account: &'a str,
        status: u16,
        _: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.failures.lock().unwrap().push((account.into(), status));
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}
async fn upstream(
    State(host): State<Arc<FixtureHost>>,
    Path(index): Path<usize>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let attempt = host.calls[index].fetch_add(1, Ordering::SeqCst);
    host.requests.lock().unwrap().push((index, headers, body));
    let events = if index < host.failing && !(host.recover_on_same && attempt > 0) {
        let mut events = host.before_error.clone();
        events.push(overload());
        events
    } else {
        vec![json!({"type":"response.created","response":{"id":format!("account-{index}-response"),"status":"in_progress"}}), completed()]
    };
    if index < host.failing && host.delayed_opening != 0 {
        let rejection = format!("data: {}\n\n", overload());
        let (first, last) = match host.delayed_opening {
            1 => (Bytes::from_static(b": connecting\n\n"), Bytes::from(rejection)),
            2 => (Bytes::copy_from_slice(&rejection.as_bytes()[..12]), Bytes::copy_from_slice(&rejection.as_bytes()[12..])),
            _ => (Bytes::from(format!("data: {}\n\n", created())), Bytes::from(rejection)),
        };
        let chunks = stream::once(async move { Ok::<_, std::io::Error>(first) })
            .chain(stream::once(async move {
                if host.delayed_opening == 3 {
                    host.release_error.notified().await;
                } else {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
                Ok::<_, std::io::Error>(last)
            }));
        return Response::builder().header("content-type", "text/event-stream")
            .body(Body::from_stream(chunks)).unwrap();
    }
    // Deliberately split frames across transport chunks, including JSON tokens.
    let raw = events
        .into_iter()
        .map(|v| format!("data: {v}\n\n"))
        .collect::<String>();
    let chunks = raw
        .as_bytes()
        .chunks(11)
        .map(Bytes::copy_from_slice)
        .map(Ok::<_, std::io::Error>)
        .collect::<Vec<_>>();
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream::iter(chunks)))
        .unwrap()
}
async fn run(
    limit: u16,
    before_error: Vec<Value>,
    failing: usize,
    same_retry: bool,
    stream: bool,
) -> (Arc<FixtureHost>, u16, String) {
    run_delayed(limit, before_error, failing, same_retry, stream, 0).await
}
async fn run_delayed(
    limit: u16,
    before_error: Vec<Value>,
    failing: usize,
    same_retry: bool,
    stream: bool,
    delayed_opening: u8,
) -> (Arc<FixtureHost>, u16, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(FixtureHost {
        base: format!("http://{}", listener.local_addr().unwrap()),
        policy: ExecutionPolicy {
            credential_retries: limit,
            status_retries: u8::from(same_retry),
            status_retry_base_delay: Duration::from_millis(1),
            total_timeout: Duration::from_secs(3),
            ..Default::default()
        },
        failing,
        before_error,
        recover_on_same: same_retry,
        delayed_opening,
        release_error: tokio::sync::Notify::new(),
        calls: Default::default(),
        requests: Mutex::new(vec![]),
        failures: Mutex::new(vec![]),
        reports: Mutex::new(vec![]),
    });
    let app = Router::new()
        .route("/{index}/responses", post(upstream))
        .with_state(host.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let mut handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        temp.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let mut response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .header("session-id", "client-session")
        .header("thread-id", "client-thread")
        .json(&json!({"model":"model","input":[],"stream":stream}))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let first = if delayed_opening == 3 {
        let first = tokio::time::timeout(Duration::from_secs(2), response.chunk())
            .await.expect("response.created must be delivered before the upstream can send its error")
            .unwrap().unwrap();
        assert!(String::from_utf8_lossy(&first).contains("response.created"));
        host.release_error.notify_one();
        String::from_utf8(first.to_vec()).unwrap()
    } else { String::new() };
    let text = first + &response.text().await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    (host, status, text)
}

#[tokio::test]
async fn zero_tries_all_distinct_accounts_positive_limits_include_first_for_stream_and_json() {
    for stream in [true, false] {
        for (limit, expected_calls, expected_status) in
            [(0, 3, 200), (1, 1, 503), (2, 2, 503), (3, 3, 200)]
        {
            let (host, status, text) = run(limit, vec![created()], 2, false, stream).await;
            assert_eq!(status, expected_status, "limit {limit}: {text}");
            assert_eq!(
                host.calls
                    .iter()
                    .map(|c| c.load(Ordering::SeqCst))
                    .sum::<usize>(),
                expected_calls
            );
            if status == 200 {
                assert!(!text.contains("server_is_overloaded"));
                let requests = host.requests.lock().unwrap();
                assert_ne!(requests[0].1["session-id"], requests[1].1["session-id"]);
                assert_ne!(requests[0].1["thread-id"], requests[1].1["thread-id"]);
            }
            assert!(host
                .failures
                .lock()
                .unwrap()
                .iter()
                .all(|(_, status)| *status == 503));
        }
    }
}

#[tokio::test]
async fn same_account_status_budget_recovers_without_rotating_ids() {
    let (host, status, _) = run(1, vec![created()], 1, true, true).await;
    assert_eq!(status, 200);
    assert_eq!(host.calls[0].load(Ordering::SeqCst), 2);
    assert_eq!(host.calls[1].load(Ordering::SeqCst), 0);
    let requests = host.requests.lock().unwrap();
    assert_eq!(requests[0].1["session-id"], requests[1].1["session-id"]);
    assert_eq!(requests[0].1["thread-id"], requests[1].1["thread-id"]);
}

#[tokio::test]
async fn unlimited_credential_count_stops_when_pool_is_exhausted() {
    let (host, status, _) = run(0, vec![created()], 3, false, true).await;
    assert_eq!(status, 503);
    assert!(host
        .calls
        .iter()
        .all(|calls| calls.load(Ordering::SeqCst) == 1));
}

#[tokio::test]
async fn no_replay_after_output_or_handshake_limit() {
    for before in [
        vec![
            created(),
            json!({"type":"response.output_text.delta","delta":"hello"}),
        ],
        vec![
            created(),
            json!({"type":"response.function_call_arguments.delta","delta":"{}"}),
        ],
        vec![created(); MAX_EVENTS + 1],
    ] {
        let (host, status, text) = run(0, before, 1, false, true).await;
        assert_eq!(status, 200);
        assert!(text.contains("server_is_overloaded"));
        assert_eq!(host.calls[0].load(Ordering::SeqCst), 1);
        assert_eq!(host.calls[1].load(Ordering::SeqCst), 0);
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports[0].status, 503);
        assert!(!reports[0].success);
    }
}

#[test]
fn input_faults_win_over_capacity_and_quota_hints() {
    assert_eq!(
        rejection(&json!({"type":"error","code":"server_is_overloaded","message":"overload"}))
            .unwrap()
            .0,
        503
    );
    assert!(rejection(&json!({"type":"error","error":{"type":"service_unavailable_error","message":"Context length exceeded"}})).is_none());
    for code in [
        "invalid_encrypted_content",
        "context_length_exceeded",
        "previous_response_not_found",
    ] {
        assert!(rejection(&json!({"type":"response.failed","response":{"error":{"code":code,"type":"service_unavailable_error"}}})).is_none());
    }
    for (code, status) in [
        ("account_disabled", 403),
        ("quota_exceeded", 429),
        ("server_is_overloaded", 503),
    ] {
        assert_eq!(
            rejection(&json!({"type":"response.failed","response":{"error":{"code":code}}}))
                .unwrap()
                .0,
            status
        );
    }
    assert!(rejection(
        &json!({"type":"response.output_text.delta","delta":"server_is_overloaded"})
    )
    .is_none());
}

#[test]
fn empty_incomplete_requires_explicit_zero_and_is_not_an_interrupt() {
    let mut event =
        json!({"type":"response.incomplete","response":{"output":[],"usage":{"output_tokens":0}}});
    assert_eq!(rejection(&event).unwrap().0, 502);
    event["response"]["incomplete_details"] = json!({"reason":"interrupted"});
    assert!(rejection(&event).is_none());
    event["response"]
        .as_object_mut()
        .unwrap()
        .remove("incomplete_details");
    for value in [Value::Null, json!("0"), json!(0.5), json!(1)] {
        event["response"]["usage"]["output_tokens"] = value;
        assert!(rejection(&event).is_none());
    }
}

#[tokio::test]
async fn comments_or_partial_json_do_not_spend_the_response_id_hold_budget() {
    for (opening, streaming) in [(1, true), (2, true), (1, false), (2, false)] {
        let (host, status, text) = run_delayed(0, vec![], 1, false, streaming, opening).await;
        assert_eq!(status, 200, "{text}");
        assert!(!text.contains("server_is_overloaded"), "{text}");
        if streaming { assert_eq!(text.matches("response.created").count(), 1); }
        assert_eq!(host.calls[0].load(Ordering::SeqCst), 1);
        assert_eq!(host.calls[1].load(Ordering::SeqCst), 1);
        let requests = host.requests.lock().unwrap();
        assert_ne!(requests[0].1["session-id"], requests[1].1["session-id"]);
        assert_ne!(requests[0].1["thread-id"], requests[1].1["thread-id"]);
    }
}

#[tokio::test]
async fn published_created_retains_pre_business_recovery_after_visibility_budget() {
    let (host, status, text) = run_delayed(0, vec![], 1, false, true, 3).await;
    assert_eq!(status, 200);
    assert!(!text.contains("server_is_overloaded"));
    assert_eq!(text.matches("response.created").count(), 2);
    assert_eq!(host.calls[0].load(Ordering::SeqCst), 1);
    assert_eq!(host.calls[1].load(Ordering::SeqCst), 1);
}

#[path = "nonofficial_pre_business_http_tests.rs"]
mod nonofficial_pre_business_http_tests;
