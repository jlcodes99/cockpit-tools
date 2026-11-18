//! A silent zero-token `response.incomplete` terminal is an upstream glitch.
//! The gateway drops the frame and closes the stream abruptly so the client's
//! built-in stream reconnect can recover the turn; semantically explained or
//! metered incompletes are still forwarded untouched.
use super::super::{contract::*, start, Options};
use axum::{body::Body, response::Response, Router};
use futures_util::{future::BoxFuture, stream, StreamExt};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

struct TestHost {
    base: String,
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
                continuity_account_id: None,
                installation_profile: None,
                transport_identity: None,
                official_registration_id: None,
                official_authorization_revision: None,
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
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
            self.recorded.notify_one();
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(ExecutionPolicy::default()) })
    }
}

async fn exercise(terminal: Value, expect_abort: bool) {
    let sends = Arc::new(AtomicUsize::new(0));
    let upstream_sends = sends.clone();
    let app = Router::new().fallback(move || {
        upstream_sends.fetch_add(1, Ordering::SeqCst);
        let terminal = terminal.clone();
        async move {
            let frames = vec![
                json!({"type":"response.created","response":{"id":"incomplete-response","status":"in_progress"}}),
                json!({"type":"response.output_text.delta","delta":"already streamed"}),
                terminal,
            ];
            let chunks = frames.into_iter().map(|frame| Ok::<_, std::io::Error>(
                axum::body::Bytes::from(format!("data: {frame}\n\n"))));
            Response::builder().header("content-type", "text/event-stream")
                .body(Body::from_stream(stream::iter(chunks))).unwrap()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(TestHost {
        base: format!("http://{}", listener.local_addr().unwrap()),
        reports: Mutex::new(Vec::new()),
        recorded: tokio::sync::Notify::new(),
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let directory = tempfile::tempdir().unwrap();
    let mut handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        directory.path().join("identity.sqlite"),
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
        .json(&json!({"model":"fixture","stream":true,"input":"Synthetic request"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let mut stream = response.bytes_stream();
    let mut received = Vec::new();
    let mut saw_error = false;
    while let Some(item) = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("stream read timed out")
    {
        match item {
            Ok(bytes) => received.push(bytes),
            Err(_) => {
                saw_error = true;
                break;
            }
        }
    }
    let body = received.concat();
    let body = String::from_utf8_lossy(&body);
    if expect_abort {
        assert!(
            saw_error,
            "the dropped silent terminal must abort the body, got: {body}"
        );
        assert!(
            !body.contains("response.incomplete"),
            "the silent terminal must not reach the client, got: {body}"
        );
    } else {
        assert!(
            !saw_error,
            "an explained terminal must finish the stream cleanly, got: {body}"
        );
        assert!(
            body.contains("response.incomplete"),
            "an explained terminal must still be forwarded, got: {body}"
        );
    }
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
    assert!(
        !host.reports.lock().unwrap()[0].success,
        "either terminal shape records a failed request"
    );
    assert_eq!(
        sends.load(Ordering::SeqCst),
        1,
        "the gateway must not replay business output"
    );
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn silent_zero_token_incomplete_aborts_the_downstream_stream() {
    exercise(
        json!({"type":"response.incomplete","response":{"id":"incomplete-response","status":"incomplete","incomplete_details":null,"usage":null}}),
        true,
    )
    .await;
}

#[tokio::test]
async fn explained_incomplete_is_still_forwarded_and_finishes_cleanly() {
    exercise(
        json!({"type":"response.incomplete","response":{"id":"incomplete-response","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":3,"output_tokens":1}}}),
        false,
    )
    .await;
}

#[test]
fn silent_incomplete_termination_matches_unexplained_zero_usage_terminals() {
    let predicate = super::super::events::silent_incomplete_termination;
    assert!(predicate(&json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","incomplete_details":null,"usage":null}})));
    assert!(predicate(&json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","incomplete_details":{"reason":null},"usage":null}})));
    assert!(predicate(&json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","incomplete_details":{"reason":"unknown"},"usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0}}})));
    assert!(predicate(&json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete"}})));
}

#[test]
fn silent_incomplete_termination_keeps_explained_or_metered_terminals() {
    let predicate = super::super::events::silent_incomplete_termination;
    assert!(!predicate(&json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":3,"output_tokens":1}}})));
    assert!(!predicate(&json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","incomplete_details":{"reason":"content_filter"},"usage":null}})));
    assert!(!predicate(&json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","incomplete_details":null,"usage":{"input_tokens":12,"output_tokens":4}}})));
    assert!(!predicate(&json!({"type":"response.completed","response":{"id":"r","status":"completed"}})));
}
