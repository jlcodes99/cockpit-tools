//! Business output prevents replay, but does not hide a real official quota rejection.
use super::super::{contract::*, start, Options};
use axum::{body::Body, response::Response, Router};
use futures_util::{future::BoxFuture, stream};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

struct TestHost {
    base: String,
    official: bool,
    observations: Mutex<Vec<OfficialFallbackFailure>>,
    fallback_calls: AtomicUsize,
    reports: Mutex<Vec<RequestReport>>,
    recorded: tokio::sync::Notify,
}

impl Host for TestHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "caller".into(),
                label: "fixture".into(),
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            Ok(Route {
                account_id: "source".into(),
                owner_id: "owner".into(),
                continuity_account_id: self.official.then(|| "chatgpt_plan_grant".into()),
                installation_profile: None,
                transport_identity: None,
                official_registration_id: self.official.then(|| "grant".into()),
                official_authorization_revision: self.official.then(|| "epoch".into()),
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                base_url: self.base.clone(),
                model: request.model,
                headers: axum::http::HeaderMap::new(),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: false,
                chatgpt_plan: self.official,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn observe_official_failure<'a>(
        &'a self,
        _: &'a Route,
        _: &'a str,
        failure: &'a OfficialFallbackFailure,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.observations.lock().unwrap().push(failure.clone());
        })
    }
    fn fallback_to_local<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
        _: RouteRequest,
        _: OfficialFallbackFailure,
    ) -> BoxFuture<'a, Result<Option<Route>, ServiceError>> {
        Box::pin(async move {
            self.fallback_calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: 4,
                ..Default::default()
            })
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
            self.recorded.notify_one();
        })
    }
}

async fn exercise(official: bool, code: &'static str, streaming: bool) {
    let sends = Arc::new(AtomicUsize::new(0));
    let upstream_sends = sends.clone();
    let app = Router::new().fallback(move || {
        upstream_sends.fetch_add(1, Ordering::SeqCst);
        async move {
            let frames = vec![
                json!({"type":"response.created","response":{"id":"late-response","status":"in_progress"}}),
                json!({"type":"response.output_text.delta","delta":"already generated business output"}),
                json!({"type":"response.failed","response":{"id":"late-response","status":"failed",
                    "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"already generated business output"}]}],
                    "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2},
                    "error":{"code":code,"message":"Synthetic late refusal","resets_in_seconds":240}}}),
            ];
            let chunks = frames.into_iter().map(|frame| Ok::<_, std::io::Error>(
                axum::body::Bytes::from(format!("data: {frame}\n\n"))));
            Response::builder().header("content-type", "text/event-stream")
                .header("retry-after", "120")
                .body(Body::from_stream(stream::iter(chunks))).unwrap()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(TestHost {
        base: format!("http://{}", listener.local_addr().unwrap()),
        official,
        observations: Mutex::new(Vec::new()),
        fallback_calls: AtomicUsize::new(0),
        reports: Mutex::new(Vec::new()),
        recorded: tokio::sync::Notify::new(),
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let directory = tempfile::tempdir().unwrap();
    let mut handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        directory.path().join("history.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&json!({"model":"fixture","stream":streaming,"input":"Synthetic request"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        if streaming { 200 } else { 429 }
    );
    let body = response.text().await.unwrap();
    assert!(body.contains(code), "{body}");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let notified = host.recorded.notified();
            if !host.reports.lock().unwrap().is_empty() {
                break;
            }
            notified.await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        sends.load(Ordering::SeqCst),
        1,
        "business output must never be replayed"
    );
    assert_eq!(host.fallback_calls.load(Ordering::SeqCst), 0);
    let observations = host.observations.lock().unwrap();
    if official && code == "subscription_sharing_usage_limit_exceeded" {
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].code, code);
        assert_eq!(observations[0].status, 429);
        assert_eq!(observations[0].trigger_stage, "upstream");
        assert_eq!(observations[0].retry_after, Some(Duration::from_secs(240)));
    } else {
        assert!(
            observations.is_empty(),
            "generic limits and non-official routes cannot create official quota health"
        );
    }
    drop(observations);
    assert_eq!(
        host.reports.lock().unwrap()[0]
            .diagnostics
            .as_ref()
            .unwrap()
            .attempts
            .len(),
        1
    );
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn official_late_quota_after_business_output_is_observed_without_replay() {
    for streaming in [false, true] {
        exercise(true, "subscription_sharing_usage_limit_exceeded", streaming).await;
    }
}

#[tokio::test]
async fn late_generic_or_nonofficial_limits_do_not_create_official_quota_health() {
    for streaming in [false, true] {
        exercise(true, "rate_limit_exceeded", streaming).await;
        exercise(
            false,
            "subscription_sharing_usage_limit_exceeded",
            streaming,
        )
        .await;
    }
}
