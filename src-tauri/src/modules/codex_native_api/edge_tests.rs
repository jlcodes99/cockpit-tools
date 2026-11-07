use super::{contract::*, start, Handle, Options};
use axum::{
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct FixtureHost {
    base: String,
    wire: WireApi,
    reports: Mutex<Vec<RequestReport>>,
}
impl Host for FixtureHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            if key != "local-key" {
                return Err(ServiceError::new(401, "invalid_api_key", "Invalid key"));
            }
            Ok(Caller {
                id: "fixture".into(),
                label: "test".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _caller: &'a Caller,
        _request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
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
                account_id: "a".into(),
                owner_id: "owner".into(),
                base_url: self.base.clone(),
                model: "upstream-model".into(),
                headers: HeaderMap::new(),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: false,
                chatgpt_plan: false,
                wire_api: self.wire,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async {
            Ok(json!({"object":"list","data":[{"id":"public-model","object":"model"}]}))
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}
struct Fixture {
    handle: Handle,
    host: Arc<FixtureHost>,
    upstream: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}
impl Fixture {
    async fn new(wire: WireApi, router: Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let host = Arc::new(FixtureHost {
            base: format!("http://{addr}/v1"),
            wire,
            reports: Mutex::new(Vec::new()),
        });
        let dir = tempfile::tempdir().unwrap();
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            dir.path().join("identities.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            handle,
            host,
            upstream,
            _dir: dir,
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.handle.addr, path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.upstream.abort();
    }
}

#[tokio::test]
async fn native_edge_conversion_failure_keeps_usage_and_records_final_status() {
    let router = Router::new().route("/v1/responses", post(|| async {
        Json(json!({"id":"unrepresentable","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"unknown_content","data":"opaque"}]}],"usage":{"input_tokens":4,"output_tokens":2}}))
    }));
    let mut fixture = Fixture::new(WireApi::Responses, router).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (path, body) in [
        (
            "/v1/messages",
            json!({"model":"public-model","max_tokens":32,"messages":[{"role":"user","content":"test"}]}),
        ),
        (
            "/v1/chat/completions",
            json!({"model":"public-model","messages":[{"role":"user","content":"test"}]}),
        ),
    ] {
        let response = client
            .post(fixture.url(path))
            .bearer_auth("local-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 502, "{path}");
        let _: Value = response.json().await.unwrap();
    }
    fixture
        .handle
        .shutdown(Duration::from_secs(2))
        .await
        .unwrap();
    let reports = fixture.host.reports.lock().unwrap();
    assert_eq!(reports.len(), 2);
    for report in reports.iter() {
        assert_eq!(report.status, 502);
        assert!(!report.success);
        assert_eq!(report.usage.as_ref().unwrap()["input_tokens"], 4);
        assert_eq!(report.usage.as_ref().unwrap()["output_tokens"], 2);
        assert_eq!(
            report
                .diagnostics
                .as_ref()
                .unwrap()
                .failure_phase
                .as_deref(),
            Some("downstream")
        );
    }
}

#[tokio::test]
async fn native_edge_stream_conversion_error_preserves_completed_upstream_usage() {
    let router = Router::new().route("/v1/responses", post(|| async {
        let events = [
            json!({"type":"response.created","response":{"id":"bad-args"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"item","call_id":"call","name":"f","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"item","output_index":0,"delta":"{"}),
            json!({"type":"response.completed","response":{"id":"bad-args","status":"completed","output":[{"type":"function_call","id":"item","call_id":"call","name":"f","arguments":"{"}],"usage":{"input_tokens":4,"output_tokens":2}}}),
        ];
        ([("content-type", "text/event-stream")], events.into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>())
    }));
    let mut fixture = Fixture::new(WireApi::Responses, router).await;
    let response = reqwest::Client::builder().no_proxy().build().unwrap()
        .post(fixture.url("/v1/messages")).bearer_auth("local-key")
        .json(&json!({"model":"public-model","max_tokens":32,"stream":true,"messages":[{"role":"user","content":"test"}]}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("event: error"));
    fixture
        .handle
        .shutdown(Duration::from_secs(2))
        .await
        .unwrap();
    let reports = fixture.host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, 502);
    assert!(!reports[0].success);
    assert_eq!(reports[0].usage.as_ref().unwrap()["output_tokens"], 2);
}

#[tokio::test]
async fn native_edge_all_routes_accept_existing_service_key_forms_and_precedence() {
    let fixture = Fixture::new(WireApi::Responses, Router::new()).await;
    let client = reqwest::Client::new();
    for header in ["x-goog-api-key", "x-api-key"] {
        let response = client
            .get(fixture.url("/v1/models"))
            .header(header, "local-key")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "header {header}");
    }
    for query in ["key", "auth_token"] {
        let response = client
            .get(fixture.url(&format!("/v1/models?{query}=local-key")))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "query {query}");
    }
    let response = client
        .get(fixture.url("/v1/models?key=wrong"))
        .header("authorization", "")
        .header("x-api-key", "wrong")
        .header("x-goog-api-key", "local-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response = client
        .get(fixture.url("/v1/models"))
        .header("authorization", "local-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        200,
        "raw Authorization keys remain compatible"
    );
    let response = client
        .get(fixture.url("/v1/models?auth_token=local-key"))
        .bearer_auth("wrong")
        .header("x-goog-api-key", "local-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        401,
        "a present Bearer token remains authoritative"
    );
}

#[tokio::test]
async fn native_edge_upstream_errors_use_the_requested_protocol() {
    let fixture = Fixture::new(
        WireApi::Responses,
        Router::new().route(
            "/v1/responses",
            post(|| async {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", "3")],
                    Json(json!({"error":{"message":"busy"}})),
                )
            }),
        ),
    )
    .await;
    let response = reqwest::Client::new()
        .post(fixture.url("/v1/messages"))
        .header("x-api-key", "local-key")
        .json(&json!({"model":"public-model","messages":[{"role":"user","content":"hello"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "3");
    let value: Value = response.json().await.unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["type"], "rate_limit_error");
    assert_eq!(value["error"]["message"], "busy");
}

#[tokio::test]
async fn native_edge_malformed_core_parameter_returns_status_without_upstream_record() {
    let mut fixture = Fixture::new(WireApi::Responses, Router::new()).await;
    let response = reqwest::Client::new()
        .post(fixture.url("/v1/messages"))
        .bearer_auth("local-key")
        .json(&json!({"model":"public-model","messages":"invalid","stop_sequences":["END"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "invalid_compat_parameter");
    fixture.handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert!(fixture.host.reports.lock().unwrap().is_empty(),
        "local parameter rejection must not be logged as an upstream request");
}

#[tokio::test]
async fn native_edge_chat_stream_preserves_upstream_failure_as_error_event() {
    let fixture = Fixture::new(
        WireApi::Responses,
        Router::new().route("/v1/responses", post(|| async {
            ([("content-type", "text/event-stream")], concat!(
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r\",\"model\":\"upstream-model\",\"output\":[]}}\n\n",
                "data: {\"type\":\"response.failed\",\"response\":{\"id\":\"r\",\"status\":\"failed\",\"error\":{\"message\":\"fixture failed\"}}}\n\n"
            ))
        })),
    ).await;
    let response = reqwest::Client::new()
        .post(fixture.url("/v1/chat/completions"))
        .bearer_auth("local-key")
        .json(&json!({"model":"public-model","messages":[],"stream":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert!(body.contains("fixture failed"), "{body}");
    assert!(body.contains("\"error\""), "{body}");
    assert!(
        !body.contains("[DONE]"),
        "a failed stream must not report success"
    );
}

#[tokio::test]
async fn native_edge_raw_chat_preserves_provider_parameters_and_all_choices() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let capture = seen.clone();
    let router=Router::new().route("/v1/chat/completions",post(move|headers:HeaderMap,Json(body):Json<Value>|{let capture=capture.clone();async move{capture.lock().unwrap().push((headers,body));Json(json!({"id":"reply","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"first"},"finish_reason":"stop"},{"index":1,"message":{"role":"assistant","content":"second"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":7,"total_tokens":10}}))}}));
    let fixture = Fixture::new(WireApi::Chat, router).await;
    let response=reqwest::Client::new().post(fixture.url("/v1/chat/completions")).bearer_auth("local-key").header("session-id","local-session").json(&json!({"model":"public-model","messages":[{"role":"user","content":"hello"}],"n":2,"stop":["END"],"seed":7,"frequency_penalty":0.7})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let value: Value = response.json().await.unwrap();
    assert_eq!(value["choices"].as_array().unwrap().len(), 2);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].1["model"], "upstream-model");
    assert_eq!(seen[0].1["n"], 2);
    assert_eq!(seen[0].1["stop"], json!(["END"]));
    assert_ne!(seen[0].0["session-id"], "local-session");
    assert_eq!(
        fixture.host.reports.lock().unwrap()[0]
            .usage
            .as_ref()
            .unwrap()["input_tokens"],
        3
    );
}

#[tokio::test]
async fn native_edge_go_ignored_choice_count_generates_one_response() {
    let count = Arc::new(Mutex::new(0));
    let capture = count.clone();
    let fixture = Fixture::new(
        WireApi::Responses,
        Router::new().route(
            "/v1/responses",
            post(move |axum::Json(body):axum::Json<Value>| {
                *capture.lock().unwrap() += 1;
                assert!(body.get("n").is_none());
                async { Json(json!({"id":"single","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"one"}]}]})) }
            }),
        ),
    )
    .await;
    let response = reqwest::Client::new()
        .post(fixture.url("/v1/chat/completions"))
        .bearer_auth("local-key")
        .json(&json!({"model":"public-model","messages":[],"n":2}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["choices"][0]["message"]["content"],
        "one"
    );
    assert_eq!(*count.lock().unwrap(), 1);
}

#[tokio::test]
async fn native_edge_gemini_uses_same_native_responses_route() {
    let fixture=Fixture::new(WireApi::Responses,Router::new().route("/v1/responses",post(|Json(body):Json<Value>|async move{assert_eq!(body["model"],"upstream-model");assert_eq!(body["input"][0]["content"][0]["text"],"hello");Json(json!({"id":"r","object":"response","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"world"}]}],"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}))}))).await;
    let response = reqwest::Client::new()
        .post(fixture.url("/v1beta/models/public-model:generateContent?key=local-key"))
        .json(&json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let value: Value = response.json().await.unwrap();
    assert_eq!(
        value["candidates"][0]["content"]["parts"][0]["text"],
        "world"
    );
    assert_eq!(value["usageMetadata"]["totalTokenCount"], 3);
}

#[tokio::test]
async fn native_edge_count_and_preflight_do_not_issue_inference() {
    let fixture = Fixture::new(WireApi::Responses, Router::new()).await;
    let client = reqwest::Client::new();
    let response = client
        .post(fixture.url("/v1/messages/count_tokens"))
        .header("x-api-key", "local-key")
        .json(&json!({"model":"public-model","messages":[{"role":"user","content":"hello"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-cockpit-token-count"], "estimate");
    assert!(
        response.json::<Value>().await.unwrap()["input_tokens"]
            .as_u64()
            .unwrap()
            > 0
    );
    let response = client
        .request(
            reqwest::Method::OPTIONS,
            fixture.url("/v1/chat/completions"),
        )
        .header("origin", "https://app.example")
        .header(
            "access-control-request-headers",
            "authorization,content-type",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    assert_eq!(
        response.headers()["access-control-allow-headers"],
        "authorization,content-type"
    );
}

#[tokio::test]
async fn native_edge_raw_chat_stream_retains_late_usage() {
    let fixture=Fixture::new(WireApi::Chat,Router::new().route("/v1/chat/completions",post(||async{([("content-type","text/event-stream")],concat!("data: {\"id\":\"r\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\n","data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n","data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1,\"total_tokens\":5}}\n\n","data: [DONE]\n\n"))}))).await;
    let response=reqwest::Client::new().post(fixture.url("/v1/chat/completions")).bearer_auth("local-key").json(&json!({"model":"public-model","messages":[{"role":"user","content":"hello"}],"stream":true,"stream_options":{"include_usage":true}})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("[DONE]"));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        fixture.host.reports.lock().unwrap()[0]
            .usage
            .as_ref()
            .unwrap()["total_tokens"],
        5
    );
}

include!("native_chat_passthrough_tests.rs");

#[tokio::test]
async fn converted_clients_receive_one_terminal_and_no_duplicate_done() {
    let router = Router::new().route("/v1/responses", post(|| async {
        ([("content-type", "text/event-stream")], concat!(
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer\"}]}],\"usage\":{\"input_tokens\":2,\"output_tokens\":3}}}\n\n",
            "data: [DONE]\n\n"
        ))
    }));
    let mut fixture = Fixture::new(WireApi::Responses, router).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (path, body, terminal) in [
        (
            "/v1/chat/completions",
            json!({"model":"public-model","messages":[],"stream":true}),
            "data: [DONE]",
        ),
        (
            "/v1/completions",
            json!({"model":"public-model","prompt":"hi","stream":true}),
            "data: [DONE]",
        ),
        (
            "/v1/messages",
            json!({"model":"public-model","messages":[],"stream":true,"max_tokens":8}),
            "event: message_stop",
        ),
        (
            "/v1beta/models/public-model:streamGenerateContent",
            json!({"contents":[]}),
            "\"finishReason\":\"STOP\"",
        ),
        (
            "/api/chat",
            json!({"model":"public-model","messages":[],"stream":true}),
            "\"done\":true",
        ),
    ] {
        let response = client
            .post(fixture.url(path))
            .bearer_auth("local-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        let text = response.text().await.unwrap();
        assert_eq!(text.matches(terminal).count(), 1, "{path}: {text}");
        assert!(!text.contains("\"error\""), "{path}: {text}");
        if !path.contains("completions") {
            assert!(!text.contains("[DONE]"), "{path}: {text}");
        }
    }
    fixture
        .handle
        .shutdown(Duration::from_secs(2))
        .await
        .unwrap();
}

#[tokio::test]
async fn claude_token_count_normalizes_model_before_empty_body_fallback_estimate() {
    let mut fixture = Fixture::new(WireApi::Responses, Router::new()).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut results = Vec::new();
    for model in [
        "public-model",
        "                 public-model                 ",
    ] {
        let response = client
            .post(fixture.url("/v1/messages/count_tokens"))
            .bearer_auth("local-key")
            .json(&json!({"model":model,"messages":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        results.push(response.json::<Value>().await.unwrap());
    }
    assert_eq!(results[0], results[1]);
    fixture
        .handle
        .shutdown(Duration::from_secs(2))
        .await
        .unwrap();
}
