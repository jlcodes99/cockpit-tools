//! Loopback transport evidence must distinguish an unsent candidate from a real attempt.
use super::*;
use crate::modules::codex_native_api::{contract::*, start, Options};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn route(base_url: &str, account: &str) -> Route {
    Route {
        account_id: account.into(),
        continuity_account_id: None,
        owner_id: account.into(),
        installation_profile: None,
        transport_identity: None,
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        authorization_scope_fingerprint: None,
        official_fallback_failure: None,
        base_url: base_url.into(),
        model: "fixture-model".into(),
        headers: HeaderMap::new(),
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
        proxy_url: None,
        proxy_route: None,
        chatgpt: false,
        chatgpt_plan: false,
        wire_api: WireApi::Responses,
        compact_endpoint: None,
        websocket_endpoint: None,
        default_service_tier: None,
        permit: None,
    }
}

#[tokio::test]
async fn websocket_build_connect_and_timeout_have_distinct_dispatch_evidence() {
    let (_keepalive, stop) = tokio::sync::watch::channel(false);
    let count = AtomicUsize::new(0);
    let selected = route("http://127.0.0.1:0", "a");
    for (url, expected_count, expected_code) in [
        ("http://[", 0, "websocket_request_invalid"),
        (
            "http://127.0.0.1:0/responses",
            1,
            "websocket_connect_failed",
        ),
    ] {
        let error = connect_websocket_tracked(
            &selected,
            url,
            HeaderMap::new(),
            4096,
            Duration::from_secs(1),
            StopSignal(stop.clone()),
            |_| {
                count.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.code, expected_code);
        assert!(error.upstream_not_sent);
        assert_eq!(count.load(Ordering::SeqCst), expected_count);
    }
    let error = connect_websocket_tracked(
        &selected,
        "http://127.0.0.1:0/responses",
        HeaderMap::new(),
        4096,
        Duration::ZERO,
        StopSignal(stop.clone()),
        |_| {
            count.fetch_add(1, Ordering::SeqCst);
        },
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error.code, "websocket_connect_timeout");
    assert!(!error.upstream_not_sent);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    // A URL may parse before execute rejects its unsupported scheme. This is
    // still a local builder failure, not evidence of an upstream request.
    let error = connect_websocket_tracked(
        &selected,
        "ftp://127.0.0.1/responses",
        HeaderMap::new(),
        4096,
        Duration::from_secs(1),
        StopSignal(stop.clone()),
        |_| {},
    ).await.err().unwrap();
    assert!(error.upstream_not_sent);
}

#[tokio::test]
async fn websocket_header_timeout_and_bad_upgrade_keep_actual_attempt() {
    for reply in [None, Some("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: wrong\r\n\r\n")] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/responses", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(stream.read(&mut bytes).await.unwrap() > 0);
            if let Some(reply) = reply { stream.write_all(reply.as_bytes()).await.unwrap(); }
            std::future::pending::<()>().await;
        });
        let (_keepalive, stop) = tokio::sync::watch::channel(false);
        let count = AtomicUsize::new(0);
        let selected = route(&url, "a");
        let error = connect_websocket_tracked(&selected, &url, HeaderMap::new(), 4096,
            Duration::from_millis(100), StopSignal(stop), |_| { count.fetch_add(1, Ordering::SeqCst); })
            .await.err().unwrap();
        assert_eq!(error.code, if reply.is_some() { "invalid_websocket_handshake" } else { "websocket_connect_timeout" });
        assert!(!error.upstream_not_sent);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

struct DispatchHost {
    base: String,
    reject_then_unsent: bool,
    reports: Mutex<Vec<RequestReport>>,
    failures: Mutex<Vec<u16>>,
    selections: AtomicUsize,
}
impl Host for DispatchHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "dispatch-caller".into(),
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
            if self.reject_then_unsent {
                let account = ["live-ws", "unsent-ws"]
                    .into_iter()
                    .find(|id| !request.excluded_account_ids.iter().any(|old| old == id))
                    .ok_or_else(|| {
                        ServiceError::new(503, "no_available_account", "Fixture exhausted")
                    })?;
                return Ok(route(
                    if account == "live-ws" {
                        &self.base
                    } else {
                        "http://127.0.0.1:0/v1"
                    },
                    account,
                ));
            }
            Ok(route("http://127.0.0.1:0/v1", "unsent-aux"))
        })
    }
    fn image_route<'a>(
        &'a self,
        _: &'a Caller,
        request: ImageRouteRequest,
    ) -> BoxFuture<'a, Result<ImageRoute, ServiceError>> {
        Box::pin(async move {
            self.selections.fetch_add(1, Ordering::SeqCst);
            let candidates = if self.reject_then_unsent {
                ["live-image", "unsent-image-a", "unsent-image-b"]
            } else {
                ["unsent-image-a", "unsent-image-b", "live-image"]
            };
            let account = candidates
                .into_iter()
                .find(|id| !request.excluded_account_ids.iter().any(|old| old == id))
                .ok_or_else(|| {
                    ServiceError::new(503, "image_source_unavailable", "Fixture exhausted")
                })?;
            Ok(ImageRoute {
                route: route(
                    if account == "live-image" {
                        &self.base
                    } else {
                        "http://127.0.0.1:0/v1"
                    },
                    account,
                ),
                image_model: "fixture-image".into(),
                main_model: None,
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async move {
            Ok(ExecutionPolicy {
                credential_retries: if self.reject_then_unsent { 4 } else { 1 },
                send_retries: 0,
                ..Default::default()
            })
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
    fn route_failed<'a>(
        &'a self,
        _: &'a str,
        status: u16,
        _: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.failures.lock().unwrap().push(status);
        })
    }
}

#[tokio::test]
async fn image_unsent_candidates_preserve_one_attempt_budget_and_auxiliary_records_nothing() {
    let calls = Arc::new(AtomicUsize::new(0));
    let upstream_calls = calls.clone();
    let app = axum::Router::new().route(
        "/v1/images/generations",
        axum::routing::post(move || {
            let calls = upstream_calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"data":[{"b64_json":"AA=="}]}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async {
        axum::serve(listener, app).await.unwrap();
    });
    let host = Arc::new(DispatchHost {
        base,
        reject_then_unsent: false,
        reports: Mutex::new(vec![]),
        failures: Mutex::new(vec![]),
        selections: AtomicUsize::new(0),
    });
    let root = std::env::temp_dir().join(format!("native-unsent-{}", uuid::Uuid::now_v7()));
    let mut handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .post(format!("http://{}/v1/alpha/search", handle.addr))
        .bearer_auth("fixture")
        .json(&json!({"input":"fixture search"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    response.bytes().await.unwrap();
    let response = client
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("fixture")
        .json(&json!({"model":"fixture-image","prompt":"fixture image"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["data"][0]["b64_json"],
        "AA=="
    );
    handle.shutdown(Duration::from_secs(3)).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.selections.load(Ordering::SeqCst), 3);
    assert!(host.failures.lock().unwrap().is_empty());
    let reports = host.reports.lock().unwrap();
    assert_eq!(
        reports.len(),
        1,
        "Unsent auxiliary and image candidates must not persist failures"
    );
    assert_eq!(reports[0].account_id, "live-image");
    assert_eq!(reports[0].status, 200);
    let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].account_id, "live-image");
    drop(reports);
    upstream.abort();
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn images_and_websocket_keep_last_real_quota_when_remaining_candidates_never_send() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    let detail = json!({"code":"quota_exceeded","type":"rate_limit_error","message":"Synthetic quota is exhausted","param":"model","detail":{"limit":7}});
    let upstream_detail = detail.clone();
    let app = axum::Router::new().fallback(move || {
        let detail = upstream_detail.clone();
        async move {
            (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "17")],
                axum::Json(json!({"error":detail})),
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(DispatchHost {
        base: format!("http://{}/v1", listener.local_addr().unwrap()),
        reject_then_unsent: true,
        reports: Mutex::new(vec![]),
        failures: Mutex::new(vec![]),
        selections: AtomicUsize::new(0),
    });
    let upstream = tokio::spawn(async {
        axum::serve(listener, app).await.unwrap();
    });
    let root = tempfile::tempdir().unwrap();
    let mut handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("fixture")
        .json(&json!({"model":"fixture-image","prompt":"fixture image"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "17");
    assert_eq!(response.json::<Value>().await.unwrap()["error"], detail);
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer fixture".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(Message::Text(
            json!({"type":"response.create","model":"fixture-model","input":[{"role":"user","content":"fixture"}]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Ok(Message::Text(text))) = socket.next().await {
                break serde_json::from_str::<Value>(&text).unwrap();
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(event["status"], 429, "{event}");
    assert_eq!(event["retry_after"], 17.0);
    assert_eq!(event["error"], detail);
    let _ = socket.close(None).await;
    handle.shutdown(Duration::from_secs(3)).await.unwrap();
    assert_eq!(*host.failures.lock().unwrap(), vec![429, 429]);
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 2);
    for report in reports.iter() {
        assert_eq!(report.status, 429);
        assert_eq!(report.retry_after, Some(Duration::from_secs(17)));
        let attempts = &report.diagnostics.as_ref().unwrap().attempts;
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].status, Some(429));
    }
    upstream.abort();
}
