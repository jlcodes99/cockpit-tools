use super::{contract::*, start, Options};
use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::post,
    Json, Router,
};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct PlanHost {
    base: String,
    refreshes: AtomicUsize,
    reports: Mutex<Vec<RequestReport>>,
}
impl Host for PlanHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "plan-caller".into(),
                label: "test".into(),
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
            headers.insert(
                "authorization",
                if self.refreshes.load(Ordering::SeqCst) > 0 {
                    "Bearer renewed"
                } else {
                    "Bearer original"
                }
                .parse()
                .unwrap(),
            );
            Ok(Route {
                account_id: "chatgpt_plan_registration".into(),
                owner_id: "plan-owner".into(),
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
                chatgpt_plan: true,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[{"id":"plan-model"}]})) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}

const COMPLETED: &str = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"plan-response\",\"status\":\"completed\",\"output\":[{\"id\":\"msg-plan\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\",\"annotations\":[]}]}],\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}\n\n";

type Captures = Arc<Mutex<Vec<(HeaderMap, Value)>>>;

async fn fixture(
    payload: &'static str,
    reject_first: bool,
) -> (
    Arc<PlanHost>,
    Captures,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
    super::Handle,
) {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let seen = captures.clone();
    let upstream = Router::new().route(
        "/v1/responses",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let seen = seen.clone();
            async move {
                let first = {
                    let mut seen = seen.lock().unwrap();
                    seen.push((headers, body));
                    seen.len() == 1
                };
                if first && reject_first {
                    return Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .header("content-type", "application/json")
                        .body(Body::from(
                            r#"{"error":{"code":"invalid_token","message":"expired"}}"#,
                        ))
                        .unwrap();
                }
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from(payload))
                    .unwrap()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let host = Arc::new(PlanHost {
        base,
        refreshes: AtomicUsize::new(0),
        reports: Mutex::new(Vec::new()),
    });
    let root = tempfile::tempdir().unwrap();
    let service = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    (host, captures, task, root, service)
}

#[tokio::test]
async fn chatgpt_plan_http_renews_once_and_forwards_public_responses_for_buffered_callers() {
    let (host, captures, task, _root, mut service) = fixture(COMPLETED, true).await;
    let response = reqwest::Client::builder().no_proxy().build().unwrap().post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("local-key")
        .json(&json!({"model":"plan-model","input":"hello","stream":false,"store":true,"temperature":0.5,"multi_agent":true}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "completed");
    assert_eq!(host.refreshes.load(Ordering::SeqCst), 1);
    {
        let captures = captures.lock().unwrap();
        assert_eq!(captures.len(), 2);
        assert_eq!(captures[0].0["authorization"], "Bearer original");
        assert_eq!(captures[1].0["authorization"], "Bearer renewed");
        for (headers, body) in captures.iter() {
            assert_eq!(body["store"], false);
            assert_eq!(body["stream"], true);
            assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
            assert_eq!(body["input"][0]["content"][0]["text"], "hello");
            assert!(body.get("multi_agent").is_none());
            assert!(body.get("temperature").is_none());
            assert!(!headers.contains_key("chatgpt-account-id"));
        }
    }
    assert!(host.reports.lock().unwrap().last().unwrap().success);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn chatgpt_plan_http_rejects_unsupported_tools_and_compact_without_contacting_upstream() {
    let (_host, captures, task, _root, mut service) = fixture(COMPLETED, false).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (path, extra) in [
        ("responses", json!({"tools":[{"type":"image_generation"}]})),
        (
            "responses",
            json!({"tools":[{"type":"tool_search","execution":"server"}]}),
        ),
        ("responses/compact", json!({})),
    ] {
        let mut body = json!({"model":"plan-model","input":[{"role":"user","content":"hello"}]});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let response = client
            .post(format!("http://{}/v1/{path}", service.addr))
            .bearer_auth("local-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        let body: Value = response.json().await.unwrap();
        assert_eq!(
            body["error"]["code"],
            "subscription_sharing_unsupported_capability"
        );
    }
    assert!(captures.lock().unwrap().is_empty());
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn chatgpt_plan_http_chat_compatibility_keeps_function_tools_client_executable() {
    let (_host, captures, task, _root, mut service) = fixture(COMPLETED, false).await;
    let response = reqwest::Client::builder().no_proxy().build().unwrap().post(format!("http://{}/v1/chat/completions", service.addr))
        .bearer_auth("local-key").json(&json!({"model":"plan-model","messages":[{"role":"system","content":"Help"},{"role":"user","content":"hello"}],"tools":[{"type":"function","function":{"name":"read_file","parameters":{"type":"object","properties":{}}}}]}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "Hello");
    {
        let captures = captures.lock().unwrap();
        let body = &captures[0].1;
        assert_eq!(body["input"][0]["role"], "developer");
        let extra = body["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "additional_tools")
            .unwrap();
        assert_eq!(extra["role"], "developer");
        assert_eq!(extra["tools"][0]["name"], "read_file");
        assert_eq!(body["stream"], true);
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn chatgpt_plan_http_streamed_usage_failure_is_not_recorded_as_success() {
    let payload = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\ndata: {\"type\":\"response.failed\",\"response\":{\"id\":\"failed-plan\",\"status\":\"failed\",\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"Usage limit reached\"}}}\n\n";
    let (host, _captures, task, _root, mut service) = fixture(payload, false).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("local-key")
        .json(&json!({"model":"plan-model","input":"hello","stream":true}))
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    assert!(body.contains("subscription_sharing_usage_limit_exceeded"));
    assert!(!host.reports.lock().unwrap().last().unwrap().success);
    assert_eq!(host.reports.lock().unwrap().last().unwrap().status, 429);
    assert!(
        host.reports
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .credential_scope
    );
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn chatgpt_plan_http_requires_the_official_completed_event() {
    let payload = "data: {\"type\":\"response.done\",\"response\":{\"id\":\"legacy-done\",\"status\":\"completed\",\"output\":[]}}\n\n";
    let (host, _captures, task, _root, mut service) = fixture(payload, false).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("local-key")
        .json(&json!({"model":"plan-model","input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "incomplete_upstream_stream");
    assert!(!host.reports.lock().unwrap().last().unwrap().success);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[path = "chatgpt_plan_tools_http_tests.rs"]
mod tool_discovery;

#[path = "chatgpt_plan_request_http_tests.rs"]
mod request_configuration;
