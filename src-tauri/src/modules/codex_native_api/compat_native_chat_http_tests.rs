use super::*;
use crate::modules::codex_native_api::{self as native, contract::*};
use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderValue},
    response::Response,
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, StreamExt};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct HostFixture {
    url: String,
    wire: WireApi,
    reports: Mutex<Vec<RequestReport>>,
    policy: ExecutionPolicy,
    route_delay: Duration,
}

#[tokio::test]
async fn parity22_http_responses_chat_restores_namespaced_custom_and_keeps_native_custom() {
    for streaming in [false, true] {
        let seen = Arc::new(Mutex::new(Value::Null));
        let capture = seen.clone();
        let app = Router::new().route("/v1/chat/completions", post(move |Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                *capture.lock().unwrap() = body.clone();
                let call = if body["tools"][0]["type"] == "custom" {
                    json!({"index":0,"id":"call-a","type":"custom","custom":{"name":"patch","input":"native patch"}})
                } else {
                    json!({"index":0,"id":"call-a","type":"function","function":{"name":"terminal__patch","arguments":"{\"input\":\"*** patch\\n猫\"}"}})
                };
                if body["stream"] == true {
                    let chunk=json!({"id":"r","choices":[{"index":0,"delta":{"tool_calls":[call]},"finish_reason":"tool_calls"}]});
                    Response::builder().header("content-type","text/event-stream").body(Body::from(format!("data: {chunk}\n\ndata: [DONE]\n\n"))).unwrap()
                } else {
                    Response::builder().header("content-type","application/json").body(Body::from(json!({"id":"r","choices":[{"index":0,"message":{"role":"assistant","tool_calls":[call]},"finish_reason":"tool_calls"}]}).to_string())).unwrap()
                }
            }
        }));
        let mut fixture = Fixture::new(app, WireApi::Chat).await;
        let request = json!({"model":"m","stream":streaming,"input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"terminal","tools":[{"type":"custom","name":"patch"}]}]}],"tool_choice":{"type":"custom","namespace":"terminal","name":"patch"}});
        let response = client()
            .post(fixture.url("/v1/responses"))
            .bearer_auth("key")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let canonical = if streaming {
            let text = response.text().await.unwrap();
            text.lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|event| event["type"] == "response.completed")
                .unwrap()["response"]
                .clone()
        } else {
            response.json::<Value>().await.unwrap()
        };
        assert_eq!(canonical["output"][0]["type"], "custom_tool_call");
        assert_eq!(canonical["output"][0]["namespace"], "terminal");
        assert_eq!(canonical["output"][0]["name"], "patch");
        assert_eq!(canonical["output"][0]["input"], "*** patch\n猫");
        assert!(canonical["output"][0].get("arguments").is_none());
        let wire = seen.lock().unwrap().clone();
        assert_eq!(wire["tools"][0]["type"], "function");
        assert_eq!(
            wire["tool_choice"],
            json!({"type":"function","function":{"name":"terminal__patch"}})
        );
        assert_eq!(wire["messages"], json!([]));
        fixture.shutdown().await;
    }
    let captured = Arc::new(Mutex::new(Value::Null));
    let capture = captured.clone();
    let app=Router::new().route("/v1/chat/completions",post(move |Json(body):Json<Value>| { let capture=capture.clone();async move { *capture.lock().unwrap()=body;Json(json!({"id":"native","choices":[{"index":0,"message":{"role":"assistant","tool_calls":[{"id":"a","type":"custom","custom":{"name":"patch","input":"native patch"}}]},"finish_reason":"tool_calls"}]})) } }));
    let mut fixture = Fixture::new(app, WireApi::Chat).await;
    let response=client().post(fixture.url("/v1/chat/completions")).bearer_auth("key").json(&json!({"model":"m","messages":[],"tools":[{"type":"custom","name":"patch","format":{"type":"text"}}]})).send().await.unwrap();
    assert!(response.status().is_success());
    let response: Value = response.json().await.unwrap();
    assert_eq!(
        response["choices"][0]["message"]["tool_calls"][0]["type"],
        "custom"
    );
    assert_eq!(captured.lock().unwrap()["tools"][0]["type"], "custom");
    fixture.shutdown().await;
}

#[tokio::test]
async fn parity22_http_compat_preserves_explicit_tier_and_summary_visibility() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = seen.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move |Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(body);
                Json(json!({"id":"r","status":"completed","output":[]}))
            }
        }),
    );
    let mut fixture = Fixture::new(app, WireApi::Responses).await;
    for (path, request) in [
        (
            "/v1/messages",
            json!({"model":"m","messages":[],"speed":"fast","thinking":{"type":"enabled","budget_tokens":1500,"display":"omitted"}}),
        ),
        (
            "/v1beta/models/m:generateContent",
            json!({"contents":[],"service_tier":"priority","generationConfig":{"thinkingConfig":{"thinkingBudget":-1,"includeThoughts":false}}}),
        ),
    ] {
        let response = client()
            .post(fixture.url(path))
            .bearer_auth("key")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{}",
            response.text().await.unwrap()
        );
    }
    let requests = seen.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["reasoning"]["effort"], "medium");
    assert_eq!(requests[1]["reasoning"]["effort"], "auto");
    for request in requests {
        assert_eq!(request["service_tier"], "priority");
        assert!(request["reasoning"].get("summary").is_none());
    }
    fixture.shutdown().await;
}
impl Host for HostFixture {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "native-compat".into(),
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
            tokio::time::sleep(self.route_delay).await;
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
                base_url: self.url.clone(),
                model: request.model,
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
        Box::pin(async { Ok(json!({"data":[{"id":"m"}]})) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.reports.lock().unwrap().push(report) })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
}
struct Fixture {
    handle: native::Handle,
    host: Arc<HostFixture>,
    task: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}
impl Fixture {
    async fn new(app: Router, wire: WireApi) -> Self {
        Self::configured(app, wire, ExecutionPolicy::default(), Duration::ZERO).await
    }
    async fn configured(
        app: Router,
        wire: WireApi,
        policy: ExecutionPolicy,
        route_delay: Duration,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = Arc::new(HostFixture {
            url: format!("http://{}/v1", listener.local_addr().unwrap()),
            wire,
            reports: Mutex::new(Vec::new()),
            policy,
            route_delay,
        });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let handle = native::start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            dir.path().join("identity.sqlite"),
            native::Options::default(),
        )
        .await
        .unwrap();
        Self {
            handle,
            host,
            task,
            _dir: dir,
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.handle.addr, path)
    }
    async fn shutdown(&mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.task.abort();
    }
}

#[tokio::test]
async fn immediate_compat_stream_preserves_keepalive_and_deferred_native_protocol_for_all_clients()
{
    for (path, body, ollama, expected_choices) in [
        (
            "/v1/messages",
            json!({"model":"m","stream":true,"messages":[{"role":"user","content":"hello"}]}),
            false,
            1,
        ),
        (
            "/v1beta/models/m:streamGenerateContent",
            json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}],"generationConfig":{"candidateCount":2}}),
            false,
            2,
        ),
        (
            "/api/chat",
            json!({"model":"m","stream":true,"messages":[{"role":"user","content":"hello"}]}),
            true,
            1,
        ),
    ] {
        let app=Router::new().route("/v1/chat/completions",post(|Json(body):Json<Value>|async move {
            let choices=body["n"].as_u64().unwrap_or(1);
            let text=json!({"id":"chat","choices":(0..choices).map(|index|json!({"index":index,"delta":{"content":format!("answer-{index}")},"finish_reason":null})).collect::<Vec<_>>()});
            let stop=json!({"id":"chat","choices":(0..choices).map(|index|json!({"index":index,"delta":{},"finish_reason":"stop"})).collect::<Vec<_>>()});
            let usage=json!({"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3}});
            Response::builder().header("content-type","text/event-stream").body(Body::from(format!("data: {text}\n\ndata: {stop}\n\ndata: {usage}\n\ndata: [DONE]\n\n"))).unwrap()
        }));
        let policy = ExecutionPolicy {
            immediate_sse: true,
            keepalive: Duration::from_millis(25),
            total_timeout: Duration::from_secs(2),
            ..Default::default()
        };
        let mut fixture =
            Fixture::configured(app, WireApi::Chat, policy, Duration::from_millis(300)).await;
        let started = tokio::time::Instant::now();
        let response = client()
            .post(fixture.url(path))
            .bearer_auth("key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        if !ollama {
            assert!(
                started.elapsed() < Duration::from_millis(250),
                "headers must precede slow route for {path}"
            );
        }
        let mut source = response.bytes_stream();
        let mut received = String::new();
        if !ollama {
            let first = tokio::time::timeout(Duration::from_millis(200), source.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(String::from_utf8_lossy(&first).contains(": keep-alive"));
            received.push_str(&String::from_utf8_lossy(&first));
        }
        while let Some(chunk) = source.next().await {
            received.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        }
        if ollama {
            let values = received
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>();
            assert!(values
                .iter()
                .any(|value| value["message"]["content"] == "answer-0"));
            assert_eq!(values.last().unwrap()["done"], true);
        } else {
            let mut decoder = native::events::SseDecoder::new(8192);
            let events = decoder.push(received.as_bytes()).unwrap();
            assert!(
                !events.iter().any(|event| event.get("error").is_some()),
                "{path}: {received}"
            );
            if expected_choices == 2 {
                assert!(events
                    .iter()
                    .any(
                        |event| event.pointer("/candidates/0/index") == Some(&json!(1))
                            && event.pointer("/candidates/0/content/parts/0/text")
                                == Some(&json!("answer-1"))
                    ));
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| event.pointer("/candidates/0/finishReason").is_some())
                        .count(),
                    2
                );
            } else {
                assert!(events
                    .iter()
                    .any(|event| event.pointer("/delta/text") == Some(&json!("answer-0"))));
                assert!(events.iter().any(|event| event["type"] == "message_stop"));
            }
        }
        fixture.shutdown().await;
        let reports = fixture.host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].success);
        assert_eq!(reports[0].usage.as_ref().unwrap()["input_tokens"], 2);
    }
}
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
fn native_reply(body: &Value) -> Value {
    let n = body["n"].as_u64().unwrap_or(1);
    json!({"id":"chat","model":"m","choices":(0..n).map(|index|json!({"index":index,"message":{"role":"assistant","content":format!("answer-{index}")},"finish_reason":"stop"})).collect::<Vec<_>>(),"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}})
}

#[tokio::test]
async fn provider_native_compat_preserves_generation_options_and_all_candidates() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = seen.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(body.clone());
                Json(native_reply(&body))
            }
        }),
    );
    let mut fixture = Fixture::new(app, WireApi::Chat).await;
    for (path, body) in [
        (
            "/v1/messages",
            json!({"model":"m","messages":[{"role":"user","content":"hello"}],"stop_sequences":["END"],"max_tokens":30}),
        ),
        (
            "/v1beta/models/m:generateContent",
            json!({"contents":[{"parts":[{"text":"hello"}]}],"generationConfig":{"candidateCount":2,"topK":4,"stopSequences":["END"]}}),
        ),
        (
            "/api/chat",
            json!({"model":"m","stream":false,"messages":[{"role":"user","content":"hello"}],"options":{"num_predict":20}}),
        ),
    ] {
        let response = client()
            .post(fixture.url(path))
            .bearer_auth("key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        let result: Value = response.json().await.unwrap();
        if path.contains("v1beta") {
            assert_eq!(result["candidates"].as_array().unwrap().len(), 2);
            assert_eq!(
                result["candidates"][1]["content"]["parts"][0]["text"],
                "answer-1"
            );
        }
    }
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0]["stop"], json!(["END"]));
        assert_eq!(seen[0]["max_tokens"], 30);
        assert_eq!(seen[1]["n"], 2);
        assert_eq!(seen[1]["top_k"], 4);
        assert_eq!(seen[2]["max_tokens"], 20);
    }
    fixture.shutdown().await;
    assert!(fixture
        .host
        .reports
        .lock()
        .unwrap()
        .iter()
        .all(|report| report.success && report.usage.as_ref().unwrap()["input_tokens"] == 2));
}

#[tokio::test]
async fn responses_route_accepts_go_ignored_options_and_omits_them_upstream() {
    let app = Router::new().route(
        "/v1/responses",
        post(|Json(body):Json<Value>| async move {
            assert!(body.get("stop_sequences").is_none());
            assert!(body.get("stop").is_none());
            assert_eq!(body["input"][0]["content"][0]["text"], "hello");
            Json(json!({"id":"accepted-options","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}]}))
        }),
    );
    let mut fixture = Fixture::new(app, WireApi::Responses).await;
    let response=client().post(fixture.url("/v1/messages")).bearer_auth("key").json(&json!({"model":"m","messages":[{"role":"user","content":"hello"}],"stop_sequences":["END"]})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    fixture.shutdown().await;
    assert!(fixture.host.reports.lock().unwrap()[0].success);
}

#[tokio::test]
async fn gemini_native_stream_preserves_interleaved_candidates_and_usage() {
    let payload=vec![
        json!({"id":"c","choices":[{"index":1,"delta":{"content":"two"},"finish_reason":null},{"index":0,"delta":{"content":"one"},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"},{"index":1,"delta":{},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3}}),
    ].into_iter().map(|event|format!("data: {event}\n\n")).collect::<String>()+"data: [DONE]\n\n";
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let payload = payload.clone();
            async move {
                assert_eq!(body["n"], 2);
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from(payload))
                    .unwrap()
            }
        }),
    );
    let mut fixture = Fixture::new(app, WireApi::Chat).await;
    let response = client()
        .post(fixture.url("/v1beta/models/m:streamGenerateContent"))
        .bearer_auth("key")
        .json(&json!({"contents":[],"generationConfig":{"candidateCount":2}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let received = response.text().await.unwrap();
    let mut decoder = native::events::SseDecoder::new(4096);
    let events = decoder.push(received.as_bytes()).unwrap();
    assert!(events.iter().any(
        |event| event.pointer("/candidates/0/index") == Some(&json!(1))
            && event.pointer("/candidates/0/content/parts/0/text") == Some(&json!("two"))
    ));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.pointer("/candidates/0/finishReason").is_some())
            .count(),
        2
    );
    fixture.shutdown().await;
    assert_eq!(
        fixture.host.reports.lock().unwrap()[0]
            .usage
            .as_ref()
            .unwrap()["output_tokens"],
        3
    );
}

#[tokio::test]
async fn native_compat_conversion_error_keeps_usage_and_failure_status() {
    let app=Router::new().route("/v1/chat/completions",post(||async{Json(json!({"id":"c","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call","type":"function","function":{"name":"f","arguments":"{"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3}}))}));
    let mut fixture = Fixture::new(app, WireApi::Chat).await;
    let response = client()
        .post(fixture.url("/v1/messages"))
        .bearer_auth("key")
        .json(&json!({"model":"m","messages":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    response.bytes().await.unwrap();
    fixture.shutdown().await;
    let reports = fixture.host.reports.lock().unwrap();
    assert_eq!(reports[0].status, 502);
    assert!(!reports[0].success);
    assert_eq!(reports[0].usage.as_ref().unwrap()["input_tokens"], 2);
}

#[tokio::test]
async fn cancelling_native_compat_stream_records_cancellation_and_received_usage() {
    let app=Router::new().route("/v1/chat/completions",post(||async{
        let first=format!("data: {}\n\n",json!({"id":"c","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}],"usage":{"prompt_tokens":2,"completion_tokens":3}}));
        let stream=futures_util::stream::once(async move{Ok::<_,std::io::Error>(Bytes::from(first))}).chain(futures_util::stream::pending());
        let mut response=Response::new(Body::from_stream(stream));response.headers_mut().insert("content-type",HeaderValue::from_static("text/event-stream"));response
    }));
    let mut fixture = Fixture::new(app, WireApi::Chat).await;
    let response = client()
        .post(fixture.url("/v1/messages"))
        .bearer_auth("key")
        .json(&json!({"model":"m","stream":true,"messages":[]}))
        .send()
        .await
        .unwrap();
    let mut bytes = response.bytes_stream();
    tokio::time::timeout(Duration::from_secs(2), bytes.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(bytes);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !fixture.host.reports.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    fixture.shutdown().await;
    let reports = fixture.host.reports.lock().unwrap();
    assert_eq!(reports[0].status, 499);
    assert_eq!(reports[0].usage.as_ref().unwrap()["output_tokens"], 3);
}
