//! Real native Chat/Completions SSE bootstrap and per-attempt usage accounting.
use super::*;
use crate::modules::codex_native_api::{contract::*, start, Handle, Options};
use axum::{http::HeaderMap, routing::post, Json, Router};
use futures_util::future::BoxFuture;
use serde_json::json;
use std::{sync::Mutex, time::Duration};

struct HostFixture {
    base: String,
    limit: u16,
    selections: Mutex<Vec<RouteRequest>>,
    failures: Mutex<Vec<u16>>,
    reports: Mutex<Vec<RequestReport>>,
}

#[test]
fn native_chat_error_frames_match_actual_go_stream_executor() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/native_chat_bootstrap_go.json")).unwrap();
    for case in cases {
        let mut decoder = SseDecoder::new(4096);
        let frames = decoder
            .push_frames(case["wire"].as_str().unwrap().as_bytes())
            .unwrap();
        let status = frames
            .iter()
            .find_map(super::super::upstream_errors::chat_frame_failure)
            .unwrap()
            .status;
        assert_eq!(
            u64::from(status),
            case["expected_status"].as_u64().unwrap(),
            "{}",
            case["name"]
        );
        if let Some(expected) = case.get("expected_request_fault").and_then(Value::as_bool) {
            let frame = &frames[0];
            let error = super::super::upstream_errors::chat_frame_failure(frame).unwrap();
            let canonical = super::super::upstream_errors::canonical_chat_failure(frame, &error);
            assert_eq!(
                super::super::upstream_errors::request_fault(status, &canonical),
                expected,
                "{}",
                case["name"]
            );
            if status == 401 {
                assert!(
                    super::super::diagnostics::upstream_error_metadata_status(status, &canonical).1
                );
            }
            let raw = case["wire"].as_str().unwrap();
            let source: Value = serde_json::from_str(
                raw.lines()
                    .find_map(|line| line.strip_prefix("data: "))
                    .unwrap(),
            )
            .unwrap();
            if source.get("type").is_some() {
                assert_eq!(
                    frame.bytes.as_ref(),
                    raw.as_bytes(),
                    "declared failure bytes stay verbatim"
                );
            }
            if let Some(nested) = source.get("error") {
                assert_eq!(
                    frame.value.as_ref().unwrap().get("error"),
                    Some(nested),
                    "real nested failure stays untouched"
                );
            }
        }
    }
}
impl Host for HostFixture {
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
            self.selections.lock().unwrap().push(request.clone());
            let account = request.preferred_account_id.as_deref().unwrap_or(
                if request.excluded_account_ids.iter().any(|id| id == "a") {
                    "b"
                } else {
                    "a"
                },
            );
            if request.excluded_account_ids.iter().any(|id| id == account) {
                return Err(ServiceError::new(
                    503,
                    "no_available_account",
                    "no fixture route",
                ));
            }
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
                account_id: account.into(),
                owner_id: format!("owner-{account}"),
                base_url: format!("{}/{}", self.base, account),
                model: "upstream-model".into(),
                headers: HeaderMap::new(),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: false,
                chatgpt_plan: false,
                wire_api: WireApi::Chat,
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
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: self.limit,
                total_timeout: Duration::from_secs(3),
                ..Default::default()
            })
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
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}

struct Fixture {
    handle: Handle,
    host: Arc<HostFixture>,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
    upstream: tokio::task::JoinHandle<()>,
    root: tempfile::TempDir,
}
impl Fixture {
    async fn new(first: String, limit: u16) -> Self {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut app = Router::new();
        for account in ["a", "b"] {
            for endpoint in ["chat/completions", "completions"] {
                let seen = seen.clone();
                let first = first.clone();
                app=app.route(&format!("/{account}/{endpoint}"),post(move|Json(body):Json<Value>|{
                let seen=seen.clone();let first=first.clone();async move{
                    seen.lock().unwrap().push((account.into(),body.clone()));
                    let wire=if account=="a"{first}else{
                        let choices=(0..body["n"].as_u64().unwrap_or(1)).map(|index|if endpoint=="completions"{json!({"index":index,"text":format!("success-{index}"),"finish_reason":"stop"})}else{json!({"index":index,"delta":{"content":format!("success-{index}")},"finish_reason":"stop"})}).collect::<Vec<_>>();
                        format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",json!({"choices":choices}),json!({"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":2,"total_tokens":9},"extension":"kept"}))
                    };
                    let split=wire.len()/2;let chunks=vec![Ok::<_,std::io::Error>(Bytes::copy_from_slice(&wire.as_bytes()[..split])),Ok(Bytes::copy_from_slice(&wire.as_bytes()[split..]))];
                    axum::http::Response::builder().header("content-type","text/event-stream").body(Body::from_stream(stream::iter(chunks))).unwrap()
                }
            }));
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let host = Arc::new(HostFixture {
            base: format!("http://{addr}"),
            limit,
            selections: Mutex::new(vec![]),
            failures: Mutex::new(vec![]),
            reports: Mutex::new(vec![]),
        });
        let root = tempfile::tempdir().unwrap();
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            handle,
            host,
            seen,
            upstream,
            root,
        }
    }
    async fn send(&self, endpoint: &str, extra: Value) -> reqwest::Response {
        let mut body = if endpoint == "completions" {
            json!({"model":"client-model","prompt":"raw 250ms cipher","stream":true})
        } else {
            json!({"model":"client-model","messages":[{"role":"user","content":"raw 250ms cipher"}],"stream":true})
        };
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/{endpoint}", self.handle.addr))
            .bearer_auth("fixture")
            .json(&body)
            .send()
            .await
            .unwrap()
    }
    async fn finish(mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.upstream.abort();
    }
}

#[tokio::test]
async fn native_chat_bootstrap_errors_rotate_and_force_usage_per_attempt_without_losing_options() {
    for endpoint in ["chat/completions", "completions"] {
        for wire in [
            "data: {\"error\":{\"status_code\":429,\"message\":\"limited\"}}\n\n",
            "data: {\"response\":{\"error\":{\"status\":\"503\",\"message\":\"overloaded\"}}}\n\n",
            "event: error\ndata: {\"status_code\":503,\"message\":\"overloaded\"}\n\n",
            "data: {\"code\":\"busy\",\"message\":\"overloaded\",\"status\":503}\n\n",
            "event: error\n\n",
        ] {
            let mut fixture = Fixture::new(wire.into(), 0).await;
            let response=fixture.send(endpoint,json!({"n":2,"top_k":5,"seed":9,"tools":[{"type":"function","function":{"name":"tool","parameters":{"type":"object"}}}],"stream_options":{"include_usage":false,"extension":"kept"}})).await;
            assert_eq!(response.status(), 200);
            let text = response.text().await.unwrap();
            assert!(text.contains("success-0"));
            assert!(text.contains("success-1"));
            assert!(text.contains("\"extension\":\"kept\""));
            fixture
                .handle
                .shutdown(Duration::from_secs(2))
                .await
                .unwrap();
            let seen = fixture.seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            for (_, body) in seen.iter() {
                assert_eq!(
                    body["stream_options"]["include_usage"],
                    endpoint == "chat/completions"
                );
                assert_eq!(body["stream_options"]["extension"], "kept");
                assert_eq!(body["n"], 2);
                assert_eq!(body["top_k"], 5);
                assert_eq!(body["seed"], 9);
                assert_eq!(body["tools"][0]["function"]["name"], "tool");
            }
            drop(seen);
            assert_eq!(fixture.host.failures.lock().unwrap().len(), 1);
            let reports = fixture.host.reports.lock().unwrap();
            assert_eq!(reports.len(), 1);
            assert!(reports[0].success);
            assert_eq!(reports[0].usage.as_ref().unwrap()["input_tokens"], 7);
            assert_eq!(reports[0].usage.as_ref().unwrap()["output_tokens"], 2);
            drop(reports);
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn native_chat_bootstrap_request_fault_and_post_output_errors_never_replay() {
    for wire in [
        "data: {\"status\":403,\"error\":{\"code\":\"cyber_policy\",\"type\":\"invalid_request_error\",\"message\":\"bad prompt\"}}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"kept\"},\"finish_reason\":null}]}\n\ndata: {\"error\":{\"status\":503,\"message\":\"overloaded\"}}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"tool\",\"arguments\":\"raw 250ms cipher\"}}]},\"finish_reason\":null}]}\n\ndata: {\"error\":{\"status\":429,\"message\":\"limited\"}}\n\n",
    ] {
        let mut fixture=Fixture::new(wire.into(),0).await;
        let response=fixture.send("chat/completions",json!({"n":2})).await;
        let _=response.bytes().await.unwrap();fixture.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        assert_eq!(fixture.seen.lock().unwrap().len(),1);assert!(fixture.host.failures.lock().unwrap().is_empty());
        assert!(!fixture.host.reports.lock().unwrap()[0].success);fixture.finish().await;
    }
}

#[tokio::test]
async fn native_chat_declared_flat_error_types_keep_request_vs_authentication_classification() {
    for (wire,request_fault) in [
        ("event: error\ndata: {\"type\":\"invalid_request_error\",\"status\":503,\"message\":\"Malformed request\"}\n\n",true),
        ("event: error\ndata: {\"type\":\"authentication_error\",\"code\":\"invalid_request_error\",\"status\":401,\"message\":\"Expired key\"}\n\n",false),
        ("event: error\ndata: {\"type\":\"invalid_request_error\",\"status\":503,\"error\":{\"type\":\"overloaded_error\",\"code\":\"server_error\",\"message\":\"Malformed request\"}}\n\n",true),
        ("event: error\ndata: {\"type\":\"authentication_error\",\"status\":401,\"error\":{\"code\":\"invalid_request_error\",\"message\":\"Expired key\"}}\n\n",false),
    ] {
        let mut fixture=Fixture::new(wire.into(),0).await;
        let response=fixture.send("chat/completions",json!({})).await;
        assert_eq!(response.status().as_u16(),if request_fault {503}else{200});
        let _=response.bytes().await.unwrap();fixture.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        if request_fault {
            assert_eq!(fixture.seen.lock().unwrap().len(),1);
            assert!(fixture.host.failures.lock().unwrap().is_empty());
            let reports=fixture.host.reports.lock().unwrap();assert_eq!(reports.len(),1);
            assert!(reports[0].diagnostics.as_ref().unwrap().upstream_request_scoped);
            assert_eq!(reports[0].error_message.as_deref(),Some("Malformed request"));
        }else{
            assert_eq!(fixture.seen.lock().unwrap().len(),2);
            assert_eq!(*fixture.host.failures.lock().unwrap(),vec![401]);
            assert!(fixture.host.reports.lock().unwrap()[0].success);
        }
        fixture.finish().await;
    }
}

#[tokio::test]
async fn native_chat_usage_is_requested_when_omitted_or_false_but_legacy_options_are_preserved() {
    for (endpoint, extra) in [
        ("chat/completions", json!({})),
        (
            "chat/completions",
            json!({"stream_options":{"include_usage":false}}),
        ),
        (
            "chat/completions",
            json!({"stream_options":{"include_usage":false,"extension":"kept"}}),
        ),
        ("completions", json!({"echo":true,"best_of":4,"logprobs":2})),
    ] {
        let mut fixture = Fixture::new(
            "data: {\"error\":{\"status\":429,\"message\":\"limited\"}}\n\n".into(),
            0,
        )
        .await;
        let response = fixture.send(endpoint, extra.clone()).await;
        let _ = response.bytes().await.unwrap();
        fixture
            .handle
            .shutdown(Duration::from_secs(2))
            .await
            .unwrap();
        for (_, body) in fixture.seen.lock().unwrap().iter() {
            if endpoint == "chat/completions" {
                assert_eq!(body["stream_options"]["include_usage"], true);
            } else {
                assert!(body.get("stream_options").is_none());
                for key in ["echo", "best_of", "logprobs"] {
                    assert_eq!(body[key], extra[key]);
                }
            }
        }
        assert_eq!(
            fixture.host.reports.lock().unwrap()[0]
                .usage
                .as_ref()
                .unwrap()["total_tokens"],
            9
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn responses_bridge_to_native_chat_shares_bootstrap_retry_and_usage_request() {
    let mut fixture = Fixture::new(
        "data: {\"error\":{\"status\":429,\"message\":\"limited\"}}\n\n".into(),
        0,
    )
    .await;
    let response=reqwest::Client::builder().no_proxy().build().unwrap().post(format!("http://{}/v1/responses",fixture.handle.addr)).bearer_auth("fixture")
        .json(&json!({"model":"client-model","stream":true,"input":[{"role":"user","content":"raw 250ms cipher"}]})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("success-0"));
    assert!(text.contains("response.completed"));
    fixture
        .handle
        .shutdown(Duration::from_secs(2))
        .await
        .unwrap();
    let seen = fixture.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen
        .iter()
        .all(|(_, body)| body["stream_options"]["include_usage"] == true));
    drop(seen);
    assert!(fixture.host.reports.lock().unwrap()[0].success);
    fixture.finish().await;
}

#[tokio::test]
async fn native_chat_bootstrap_known_opaque_owner_never_moves_to_alternate() {
    for _ in [false] {
        let fixture = Fixture::new(
            "data: {\"error\":{\"status\":503,\"message\":\"overloaded\"}}\n\n".into(),
            0,
        )
        .await;
        let path = fixture.root.path().join("identity.sqlite");
        let store = super::super::identity::IdentityStore::open(path.clone()).unwrap();
        let mut body = json!({"model":"client-model","client_metadata":{"session_id":"root","thread_id":"root"}});
        let mut headers = HeaderMap::new();
        let context = store.inspect("caller", &headers, &body).unwrap();
        let binding = store
            .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
            .unwrap();
        store.observe_event(&binding,&json!({"type":"response.completed","response":{"id":"previous","status":"completed","output":[{"type":"reasoning","encrypted_content":"original-cipher"}]}})).unwrap();
        drop(store);
        let response=fixture.send("chat/completions",json!({"client_metadata":{"session_id":"root","thread_id":"root"},"previous_response_id":"previous","messages":[{"role":"assistant","content":"original history","reasoning_details":[{"type":"reasoning","encrypted_content":"original-cipher"}]}]})).await;
        let _ = response.bytes().await.unwrap();
        assert!(fixture
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|(account, _)| account == "a"));
        for (_, body) in fixture.seen.lock().unwrap().iter() {
            assert_eq!(body["messages"][0]["content"], "original history");
            assert_eq!(
                body["messages"][0]["reasoning_details"][0]["encrypted_content"],
                "original-cipher"
            );
        }
        assert!(fixture
            .host
            .selections
            .lock()
            .unwrap()
            .iter()
            .all(|selection| selection.excluded_account_ids.is_empty()));
        fixture.finish().await;
    }
}
