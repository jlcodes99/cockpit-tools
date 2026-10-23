//! Real loopback HTTP/SSE/WS: stale Grok OAuth is refreshed once, not retried
//! blindly or treated as a permanent permission failure. No external requests.
use super::{contract::*, start, Options};
use axum::{
    extract::{State, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

struct HostFixture {
    base: String,
    grok: bool,
    refreshes: AtomicUsize,
}
impl Host for HostFixture {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "fixture".into(),
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
            if request.force_refresh {
                self.refreshes.fetch_add(1, Ordering::SeqCst);
            }
            let mut headers = HeaderMap::new();
            if self.grok {
                headers.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
            }
            headers.insert(
                "authorization",
                HeaderValue::from_static(if self.refreshes.load(Ordering::SeqCst) > 0 {
                    "Bearer fresh"
                } else {
                    "Bearer stale"
                }),
            );
            Ok(Route {
                account_id: "fixture".into(),
                owner_id: "same-owner".into(),
                continuity_account_id: None,
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                base_url: self.base.clone(),
                model: request.model,
                headers,
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
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(true) })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: 1,
                total_timeout: std::time::Duration::from_secs(5),
                ..Default::default()
            })
        })
    }
}
fn done() -> Value {
    json!({"id":"resp-fixture","status":"completed","output":[]})
}
fn failed() -> Value {
    json!({"type":"error","status":403,"error":{"code":"bad-credentials","message":"Access token could not be validated"}})
}
fn fresh(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .is_some_and(|value| value == "Bearer fresh")
}
async fn http(State(stream_error): State<bool>, headers: HeaderMap) -> Response {
    if fresh(&headers) {
        return Json(done()).into_response();
    }
    if stream_error {
        (
            [("content-type", "text/event-stream")],
            format!("data: {}\n\n", failed()),
        )
            .into_response()
    } else {
        (StatusCode::FORBIDDEN, Json(failed())).into_response()
    }
}
async fn ws(
    State(stream_error): State<bool>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !stream_error && !fresh(&headers) {
        return (StatusCode::FORBIDDEN, Json(failed())).into_response();
    }
    upgrade.on_upgrade(move |mut socket| async move {
        let _ = socket.recv().await;
        let value = if fresh(&headers) {
            json!({"type":"response.completed","response":done()})
        } else {
            failed()
        };
        let _ = socket
            .send(axum::extract::ws::Message::Text(value.to_string().into()))
            .await;
    })
}

#[tokio::test]
async fn grok_invalid_credentials_refresh_over_http_sse_and_both_ws_error_forms() {
    for (websocket, stream_error) in [(false, false), (false, true), (true, false), (true, true)] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/responses", post(http).get(ws))
            .with_state(stream_error);
        let upstream = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let host = Arc::new(HostFixture {
            base,
            grok: true,
            refreshes: AtomicUsize::new(0),
        });
        let root = tempfile::tempdir().unwrap();
        let mut gateway = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        let request = json!({"model":"grok-4.7","input":[{"role":"user","content":"hello"}]});
        if websocket {
            let mut request_url = format!("ws://{}/v1/responses", gateway.addr)
                .into_client_request()
                .unwrap();
            request_url
                .headers_mut()
                .insert("authorization", HeaderValue::from_static("Bearer fixture"));
            let (mut socket, _) = tokio_tungstenite::connect_async(request_url).await.unwrap();
            let mut ws_body = request.clone();
            ws_body["type"] = json!("response.create");
            socket
                .send(Message::Text(ws_body.to_string().into()))
                .await
                .unwrap();
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if let Some(Ok(Message::Text(text))) = socket.next().await {
                        break serde_json::from_str::<Value>(&text).unwrap();
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(event["type"], "response.completed", "{event}");
            socket.close(None).await.unwrap();
        } else {
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(format!("http://{}/v1/responses", gateway.addr))
                .bearer_auth("fixture")
                .json(&request)
                .send()
                .await
                .unwrap();
            let status = response.status();
            let body = response.text().await.unwrap();
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(body.contains("resp-fixture"), "{body}");
        }
        assert_eq!(
            host.refreshes.load(Ordering::SeqCst),
            1,
            "ws={websocket}, stream={stream_error}"
        );
        gateway
            .shutdown(std::time::Duration::from_secs(1))
            .await
            .unwrap();
        upstream.abort();
    }
}

#[tokio::test]
async fn non_grok_forbidden_errors_do_not_trigger_grok_token_refresh() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/responses", post(http))
        .with_state(false);
    let upstream = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let host = Arc::new(HostFixture {
        base,
        grok: false,
        refreshes: AtomicUsize::new(0),
    });
    let root = tempfile::tempdir().unwrap();
    let mut gateway = start(
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
        .post(format!("http://{}/v1/responses", gateway.addr))
        .bearer_auth("fixture")
        .json(&json!({"model":"other-model","input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(host.refreshes.load(Ordering::SeqCst), 0);
    gateway
        .shutdown(std::time::Duration::from_secs(1))
        .await
        .unwrap();
    upstream.abort();
}
