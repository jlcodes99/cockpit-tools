use super::{contract::*, start, Handle, Options};
#[path = "official_pre_business_gate_tests.rs"]
mod pre_business_gate_tests;
use axum::{body::Body, extract::OriginalUri, http::HeaderMap, response::Response, Json, Router};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const COMPLETED: &str = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"reply\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello\",\"annotations\":[]}]}],\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}\n\n";
type Captures = Arc<Mutex<Vec<(String, Value)>>>;

struct FallbackHost {
    base: String,
    fallback_calls: AtomicUsize,
    foreign_fallback: AtomicBool,
    fallback_disabled: AtomicBool,
    connect_failure: AtomicBool,
    local_connect_failure: AtomicBool,
    local_preflight_error: AtomicUsize,
    pool_disabled: AtomicBool,
    official_header_delay: Arc<AtomicBool>,
    direct_local: AtomicBool,
    local_oauth: AtomicBool,
    local_unauthorized_once: Arc<AtomicBool>,
    refresh_authorization_changed: AtomicBool,
    refresh_requests: Mutex<Vec<bool>>,
    official_failures: Mutex<Vec<OfficialFallbackFailure>>,
    credential_limit: AtomicUsize,
    reports: Mutex<Vec<RequestReport>>,
    requests: Mutex<Vec<RouteRequest>>,
}

impl FallbackHost {
    fn selected(&self, request: &RouteRequest, route: &str) -> Route {
        let official = route == "official";
        Route {
            account_id: if route == "backup" {
                "backup"
            } else {
                "source"
            }
            .into(),
            continuity_account_id: official.then(|| "chatgpt_plan_registration".into()),
            owner_id: format!("{route}-owner"),
            installation_profile: None,
            transport_identity: None,
            official_authorization_revision: official.then(|| "epoch".into()),
            official_registration_id: official.then(|| "registration".into()),
            official_fallback_registration_id: None,
            official_fallback_authorization_revision: None,
            authorization_scope_fingerprint: official.then(|| "acl-epoch".into()),
            official_fallback_failure: None,
            base_url: if (official && self.connect_failure.load(Ordering::SeqCst))
                || (route == "local" && self.local_connect_failure.load(Ordering::SeqCst))
            {
                // No TCP service can listen on port zero. No real API is used.
                "http://127.0.0.1:0/official".into()
            } else {
                format!("{}/{route}", self.base)
            },
            model: request.model.clone(),
            headers: HeaderMap::new(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            proxy_url: None,
            proxy_route: None,
            chatgpt: route == "local" && self.local_oauth.load(Ordering::SeqCst),
            chatgpt_plan: official,
            wire_api: WireApi::Responses,
            compact_endpoint: None,
            websocket_endpoint: None,
            default_service_tier: None,
            permit: None,
        }
    }
}
impl Host for FallbackHost {
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
            let backup = request.excluded_account_ids.iter().any(|id| id == "source");
            if backup && self.pool_disabled.load(Ordering::SeqCst) {
                return Err(ServiceError::new(
                    503,
                    "no_available_account",
                    "No authorized backup is available",
                ));
            }
            if request.excluded_account_ids.iter().any(|id| id == "backup") {
                return Err(ServiceError::new(
                    503,
                    "no_available_account",
                    "No fixture account remains",
                ));
            }
            let name = if backup {
                "backup"
            } else if self.direct_local.load(Ordering::SeqCst) {
                "local"
            } else {
                "official"
            };
            let mut route = self.selected(&request, name);
            if name == "local" {
                route.official_fallback_registration_id = Some("registration".into());
                route.official_fallback_authorization_revision = Some("epoch".into());
                route.official_fallback_failure = Some(OfficialFallbackFailure {
                    trigger_stage: "health_pause",
                    status: 0,
                    code: "official_unavailable".into(),
                    message: Some("Official health is cooling before sending".into()),
                    retry_after: None,
                });
            }
            Ok(route)
        })
    }
    fn fallback_to_local<'a>(
        &'a self,
        caller: &'a Caller,
        failed: &'a Route,
        request: RouteRequest,
        failure: OfficialFallbackFailure,
    ) -> BoxFuture<'a, Result<Option<Route>, ServiceError>> {
        Box::pin(async move {
            assert_eq!(caller.id, "caller");
            assert_eq!(failed.account_id, "source");
            assert!(failed.chatgpt_plan);
            assert!(failed.permit.is_none());
            if failure.trigger_stage == "upstream" && failure.code == "subscription_sharing_usage_limit_exceeded" {
                assert!(!self.official_failures.lock().unwrap().is_empty(), "quota health must be observed before selecting a fallback");
            }
            self.fallback_calls.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request.clone());
            if self.fallback_disabled.load(Ordering::SeqCst) {
                return Ok(None);
            }
            let local_status = self.local_preflight_error.load(Ordering::SeqCst);
            if local_status > 0 {
                return Err(ServiceError::new(
                    local_status as u16,
                    if local_status == 401 {
                        "credentials_unavailable"
                    } else {
                        "local_fallback_unavailable"
                    },
                    "Local source quota is exhausted",
                ));
            }
            let mut route = self.selected(&request, "local");
            route.official_fallback_registration_id = failed.official_registration_id.clone();
            route.official_fallback_authorization_revision =
                failed.official_authorization_revision.clone();
            route.official_fallback_failure = Some(failure);
            if self.foreign_fallback.load(Ordering::SeqCst) {
                route.account_id = "unauthorized-source".into();
            }
            Ok(Some(route))
        })
    }
    fn observe_official_failure<'a>(
        &'a self,
        route: &'a Route,
        _: &'a str,
        failure: &'a OfficialFallbackFailure,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            assert!(route.chatgpt_plan);
            assert_eq!(failure.trigger_stage, "upstream");
            self.official_failures.lock().unwrap().push(failure.clone());
        })
    }
    fn refresh_route<'a>(
        &'a self,
        caller: &'a Caller,
        previous: &'a Route,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            if previous.official_fallback_registration_id.is_none() {
                return self.route(caller, request).await;
            }
            self.refresh_requests
                .lock()
                .unwrap()
                .push(request.force_refresh);
            if self.refresh_authorization_changed.load(Ordering::SeqCst) {
                return Err(ServiceError::new(
                    409,
                    "official_fallback_authorization_changed",
                    "Synthetic authorization changed",
                ));
            }
            assert_eq!(request.preferred_account_id.as_deref(), Some("source"));
            let mut route = self.selected(&request, "local");
            route.official_fallback_registration_id =
                previous.official_fallback_registration_id.clone();
            route.official_fallback_authorization_revision =
                previous.official_fallback_authorization_revision.clone();
            route.authorization_scope_fingerprint =
                previous.authorization_scope_fingerprint.clone();
            route.official_fallback_failure = previous.official_fallback_failure.clone();
            Ok(route)
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[{"id":"fixture-model"}]})) })
    }
    fn capability_fallback_decision<'a>(
        &'a self,
        _: &'a Caller,
        route: &'a Route,
        request: &'a RouteRequest,
    ) -> BoxFuture<'a, Result<CapabilityFallbackDecision, ServiceError>> {
        Box::pin(async move {
            Ok(CapabilityFallbackDecision {
                enabled: true,
                unsupported: if route.chatgpt_plan {
                    request
                        .required_capabilities
                        .iter()
                        .filter(|cap| cap.as_str() == "code_interpreter")
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                },
            })
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async move {
            Ok(ExecutionPolicy {
                open_timeout: if self.official_header_delay.load(Ordering::SeqCst) {
                    Duration::from_millis(60)
                } else {
                    Duration::from_secs(30)
                },
                stream_open_max_attempts: 3,
                send_retries: 2,
                status_retries: 0,
                bootstrap_retries: 0,
                credential_retries: self.credential_limit.load(Ordering::SeqCst) as u16,
                ..Default::default()
            })
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}

struct Fixture {
    host: Arc<FallbackHost>,
    captures: Captures,
    handle: Handle,
    _dir: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new(
        status: u16,
        rejection: Value,
        sse: Option<&'static str>,
        local_failure: bool,
    ) -> Self {
        let captures: Captures = Arc::new(Mutex::new(Vec::new()));
        let seen = captures.clone();
        let official_header_delay = Arc::new(AtomicBool::new(false));
        let delayed = official_header_delay.clone();
        let local_unauthorized_once = Arc::new(AtomicBool::new(false));
        let local_rejection = local_unauthorized_once.clone();
        let router = Router::new().fallback(move |OriginalUri(uri): OriginalUri, Json(body): Json<Value>| {
            let seen = seen.clone(); let rejection = rejection.clone(); let delayed=delayed.clone();
            let local_rejection = local_rejection.clone();
            async move {
                let path = uri.path().to_owned();
                let request_index={ let mut captures=seen.lock().unwrap(); captures.push((path.clone(), body)); captures.len() };
                let response_id=format!("reply-{}-{request_index}",path.split('/').nth(1).unwrap_or("fixture"));
                if path.starts_with("/official/") {
                    if delayed.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(180)).await; }
                    if let Some(sse) = sse {
                        return Response::builder().header("content-type", "text/event-stream")
                            .header("x-request-id", "req-official").body(Body::from(sse.replace("\"id\":\"reply\"",&format!("\"id\":\"{response_id}\"")))).unwrap();
                    }
                    return Response::builder().status(status).header("content-type", "application/json")
                        .header("x-request-id", "req-official").body(Body::from(rejection.to_string())).unwrap();
                }
                if local_failure && path.starts_with("/local/") {
                    return Response::builder().status(503).header("content-type", "application/json")
                        .body(Body::from(r#"{"error":{"code":"service_unavailable","message":"local unavailable"}}"#)).unwrap();
                }
                if path.starts_with("/local/") && local_rejection.swap(false, Ordering::SeqCst) {
                    return Response::builder().status(401).header("content-type", "application/json")
                        .body(Body::from(r#"{"error":{"code":"invalid_token","message":"Synthetic local token expired"}}"#)).unwrap();
                }
                Response::builder().header("content-type", "text/event-stream")
                    .header("x-request-id", "req-local").body(Body::from(COMPLETED.replace("\"id\":\"reply\"",&format!("\"id\":\"{response_id}\"")))).unwrap()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let host = Arc::new(FallbackHost {
            base,
            fallback_calls: AtomicUsize::new(0),
            foreign_fallback: AtomicBool::new(false),
            fallback_disabled: AtomicBool::new(false),
            connect_failure: AtomicBool::new(false),
            local_connect_failure: AtomicBool::new(false),
            local_preflight_error: AtomicUsize::new(0),
            pool_disabled: AtomicBool::new(false),
            official_header_delay,
            direct_local: AtomicBool::new(false),
            local_oauth: AtomicBool::new(false),
            local_unauthorized_once,
            refresh_authorization_changed: AtomicBool::new(false),
            refresh_requests: Mutex::new(Vec::new()),
            official_failures: Mutex::new(Vec::new()),
            credential_limit: AtomicUsize::new(2),
            reports: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        });
        let dir = tempfile::tempdir().unwrap();
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            dir.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            host,
            captures,
            handle,
            _dir: dir,
            task,
        }
    }
    async fn send(&self, body: Value) -> reqwest::Response {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", self.handle.addr))
            .bearer_auth("mock-key")
            .json(&body)
            .send()
            .await
            .unwrap()
    }
    async fn report(&self) -> RequestReport {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(report) = self.host.reports.lock().unwrap().last().cloned() {
                    return report;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
    async fn close(mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.task.abort();
    }
}
fn request() -> Value {
    json!({"model":"fixture-model","input":"hello","stream":false,"temperature":0.5})
}

#[tokio::test]
async fn official_fallback_http_local_401_refresh_keeps_local_route_and_rejects_changed_authorization(
) {
    for changed in [false, true] {
        let fixture = Fixture::new(
            400,
            json!({"error":{"code":"subscription_sharing_unsupported_capability"}}),
            None,
            false,
        )
        .await;
        fixture.host.local_oauth.store(true, Ordering::SeqCst);
        fixture
            .host
            .local_unauthorized_once
            .store(true, Ordering::SeqCst);
        fixture
            .host
            .refresh_authorization_changed
            .store(changed, Ordering::SeqCst);
        // The official preflight rejects this tool, but leaves the grant healthy.
        let mut body = request();
        body["tools"] = json!([{"type":"code_interpreter","container":{"type":"auto"}}]);
        let response = fixture.send(body).await;
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status.as_u16(), if changed { 409 } else { 200 }, "{body}");
        if changed {
            assert_eq!(
                body["error"]["code"],
                "official_fallback_authorization_changed"
            );
        }
        let expected = if changed {
            vec!["/local/responses"]
        } else {
            vec!["/local/responses", "/local/responses"]
        };
        assert_eq!(
            fixture
                .captures
                .lock()
                .unwrap()
                .iter()
                .map(|capture| capture.0.clone())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(*fixture.host.refresh_requests.lock().unwrap(), [true]);
        assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn official_fallback_http_uses_same_source_and_original_body_before_other_accounts() {
    for (status, rejection, expected_code, reason) in [
        (
            403,
            json!({"error":{"code":"subscription_sharing_user_not_eligible","type":"invalid_request_error","message":"not eligible"}}),
            "subscription_sharing_user_not_eligible",
            "eligibility_rejected",
        ),
        (
            403,
            json!({"error":{"type":"subscription_sharing_user_not_eligible","message":"not eligible"}}),
            "subscription_sharing_user_not_eligible",
            "eligibility_rejected",
        ),
        (
            403,
            json!({"error":{"type":"invalid_request_error","message":"User is not eligible for subscription sharing."}}),
            "subscription_sharing_user_not_eligible",
            "eligibility_rejected",
        ),
        (
            401,
            json!({"error":{"code":"invalid_token","message":"expired"}}),
            "invalid_token",
            "authorization_failed",
        ),
        (
            400,
            json!({"error":{"code":"invalid_grant","type":"invalid_request_error","message":"grant expired"}}),
            "invalid_grant",
            "authorization_failed",
        ),
        (
            403,
            json!({"error":{"type":"invalid_request_error","message":"The ChatGPT user is not eligible for subscription sharing."}}),
            "subscription_sharing_user_not_eligible",
            "eligibility_rejected",
        ),
        (
            429,
            json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"limited"}}),
            "subscription_sharing_usage_limit_exceeded",
            "official_throttled",
        ),
        (
            503,
            json!({"error":{"code":"service_unavailable","message":"unavailable"}}),
            "service_unavailable",
            "official_unavailable",
        ),
        (
            404,
            json!({"error":{"code":"model_not_found","message":"Model unavailable"}}),
            "official_model_unavailable",
            "capability_unsupported",
        ),
    ] {
        let fixture = Fixture::new(status, rejection, None, false).await;
        let response = fixture.send(request()).await;
        assert_eq!(response.status(), 200, "{expected_code}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["status"], "completed");
        let captures = fixture.captures.lock().unwrap().clone();
        assert_eq!(
            captures.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(),
            ["/official/responses", "/local/responses"]
        );
        assert!(captures[0].1.get("temperature").is_none());
        assert_eq!(captures[1].1["temperature"], 0.5);
        assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 1);
        let report = fixture.report().await;
        assert!(report.success);
        assert_eq!(report.account_id, "source");
        assert!(report.official_registration_id.is_none());
        assert_eq!(
            report.official_fallback_registration_id.as_deref(),
            Some("registration")
        );
        let attempts = &report.diagnostics.as_ref().unwrap().attempts;
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].status, Some(status));
        assert_eq!(
            attempts[0].error_category.as_deref(),
            Some(if expected_code == "official_model_unavailable" {
                "model_not_found"
            } else {
                expected_code
            })
        );
        assert_eq!(
            attempts[0].upstream_request_id.as_deref(),
            Some("req-official")
        );
        let trace = attempts[1].authorization_fallback.as_ref().unwrap();
        assert_eq!(trace.trigger_stage, "upstream");
        assert_eq!(trace.reason, reason);
        assert_eq!(trace.from_account_id, "source");
        assert_eq!(trace.to_account_id, "source");
        assert_eq!(trace.error_code.as_deref(), Some(expected_code));
        assert_eq!(
            attempts[1].upstream_request_id.as_deref(),
            Some("req-local")
        );
        assert_eq!(fixture.host.reports.lock().unwrap().len(), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn official_fallback_http_preoutput_sse_rejection_uses_local() {
    for payload in [
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"official\",\"output\":[]}}\n\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_user_not_eligible\",\"type\":\"invalid_request_error\"}}}\n\n",
        "data: {\"type\":\"error\",\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"limited\"}}\n\n",
    ] {
        let fixture = Fixture::new(200,Value::Null,Some(payload),false).await;
        let response = fixture.send(request()).await; assert_eq!(response.status(),200);
        let _: Value = response.json().await.unwrap();
        assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst),1);
        assert_eq!(fixture.report().await.diagnostics.unwrap().attempts.len(),2);
        fixture.close().await;
    }
}

#[tokio::test]
async fn official_fallback_http_local_failure_then_crosses_accounts_once() {
    let fixture = Fixture::new(
        403,
        json!({"error":{"code":"subscription_sharing_user_not_eligible"}}),
        None,
        true,
    )
    .await;
    let response = fixture.send(request()).await;
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    assert_eq!(
        fixture
            .captures
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.0.clone())
            .collect::<Vec<_>>(),
        [
            "/official/responses",
            "/local/responses",
            "/backup/responses"
        ]
    );
    let report = fixture.report().await;
    assert_eq!(report.account_id, "backup");
    let attempts = report.diagnostics.unwrap().attempts;
    assert_eq!(attempts.len(), 3);
    assert!(attempts[1].authorization_fallback.is_some());
    assert!(attempts[2].authorization_fallback.is_none());
    assert_eq!(fixture.host.reports.lock().unwrap().len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_http_unsent_local_candidate_is_omitted_and_does_not_consume_pool_retry(
) {
    let fixture = Fixture::new(
        403,
        json!({"error":{"code":"subscription_sharing_user_not_eligible"}}),
        None,
        false,
    )
    .await;
    fixture
        .host
        .local_preflight_error
        .store(403, Ordering::SeqCst);
    let response = fixture.send(request()).await;
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    assert_eq!(
        fixture
            .captures
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.0.clone())
            .collect::<Vec<_>>(),
        ["/official/responses", "/backup/responses"]
    );
    let report = fixture.report().await;
    let attempts = report.diagnostics.unwrap().attempts;
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].status, Some(403));
    assert_eq!(attempts[1].account_id, "backup");
    assert_eq!(attempts[1].status, Some(200));
    assert!(attempts.iter().all(|attempt| attempt.transport != "preflight"));
    assert_eq!(fixture.host.reports.lock().unwrap().len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_websocket_client_keeps_http_bridge_after_same_source_local_fallback() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    let fixture=Fixture::new(403,json!({"error":{"code":"subscription_sharing_user_not_eligible","type":"invalid_request_error"}}),None,false).await;
    let mut upgrade = format!("ws://{}/v1/responses", fixture.handle.addr)
        .into_client_request()
        .unwrap();
    upgrade
        .headers_mut()
        .insert("authorization", "Bearer mock-key".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(upgrade).await.unwrap();
    let mut body = request();
    body["type"] = json!("response.create");
    body["generate"] = json!(true);
    body["service_tier"] = json!("priority");
    body["input"] =
        json!([{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]);
    body.as_object_mut().unwrap().remove("stream");
    socket
        .send(Message::Text(body.to_string().into()))
        .await
        .unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            if let Message::Text(text) = message {
                let event: Value = serde_json::from_str(&text).unwrap();
                assert_ne!(event["type"], "error", "{event}");
                if event["type"] == "response.completed" {
                    return event;
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(terminal["response"]["status"], "completed");
    assert_eq!(
        fixture
            .captures
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.0.clone())
            .collect::<Vec<_>>(),
        ["/official/responses", "/local/responses"]
    );
    {
        let captures = fixture.captures.lock().unwrap();
        assert!(captures[0].1.get("service_tier").is_none());
        assert_eq!(captures[1].1["service_tier"], "priority");
    }
    for (_, body) in fixture.captures.lock().unwrap().iter() {
        assert!(
            body.get("type").is_none(),
            "HTTP bridge must omit response.create envelope"
        );
        assert!(
            body.get("generate").is_none(),
            "HTTP bridge must omit socket generate flag"
        );
    }
    assert_eq!(
        fixture.host.requests.lock().unwrap()[0]
            .auxiliary
            .as_deref(),
        Some("responses.websocket")
    );
    let report = fixture.report().await;
    assert!(report.success);
    assert_eq!(report.requested_service_tier.as_deref(), Some("priority"));
    assert!(report.diagnostics.as_ref().unwrap().attempts[0]
        .service_tier_adaptation
        .is_some());
    assert!(report.diagnostics.as_ref().unwrap().attempts[1]
        .service_tier_adaptation
        .is_none());
    assert_eq!(
        report.diagnostics.unwrap().attempts[1]
            .authorization_fallback
            .as_ref()
            .unwrap()
            .to_account_id,
        "source"
    );
    socket.close(None).await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_http_fast_is_adapted_without_rejecting_official() {
    let fixture = Fixture::new(200, Value::Null, Some(COMPLETED), false).await;
    let mut body = request();
    body["service_tier"] = json!("priority");
    let response = fixture.send(body).await;
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let captures = fixture.captures.lock().unwrap().clone();
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].0, "/official/responses");
    assert!(captures[0].1.get("service_tier").is_none());
    assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 0);
    let report = fixture.report().await;
    assert!(report.requested_service_tier.is_none());
    assert!(report.response_service_tier.is_none());
    let attempts = report.diagnostics.unwrap().attempts;
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].authorization_fallback.is_none());
    let adaptation = attempts[0].service_tier_adaptation.as_ref().unwrap();
    assert_eq!(adaptation.requested_tier, "priority");
    assert_eq!(adaptation.sent_tier, "default");
    assert_eq!(adaptation.reason, "official_default_only");
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_http_preserves_fast_for_real_authorization_fallback() {
    let fixture = Fixture::new(
        403,
        json!({"error":{"code":"subscription_sharing_user_not_eligible"}}),
        None,
        false,
    )
    .await;
    let mut body = request();
    body["service_tier"] = json!("priority");
    let response = fixture.send(body).await;
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let captures = fixture.captures.lock().unwrap().clone();
    assert_eq!(captures.len(), 2);
    assert!(captures[0].1.get("service_tier").is_none());
    assert_eq!(captures[1].1["service_tier"], "priority");
    let report = fixture.report().await;
    assert_eq!(report.requested_service_tier.as_deref(), Some("priority"));
    let attempts = report.diagnostics.unwrap().attempts;
    assert!(attempts[0].service_tier_adaptation.is_some());
    assert!(attempts[1].service_tier_adaptation.is_none());
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_http_local_route_preserves_requested_fast() {
    let fixture = Fixture::new(500, Value::Null, None, false).await;
    fixture.host.direct_local.store(true, Ordering::SeqCst);
    let mut body = request();
    body["service_tier"] = json!("priority");
    let response = fixture.send(body).await;
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let captures = fixture.captures.lock().unwrap().clone();
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].0, "/local/responses");
    assert_eq!(captures[0].1["service_tier"], "priority");
    assert!(fixture.report().await.diagnostics.unwrap().attempts[0]
        .service_tier_adaptation
        .is_none());
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_http_preflight_local_unavailable_uses_scoped_capability_pool() {
    for local_status in [401, 429] {
        let fixture = Fixture::new(500, Value::Null, None, false).await;
        fixture.host.credential_limit.store(1, Ordering::SeqCst);
        fixture
            .host
            .local_preflight_error
            .store(local_status, Ordering::SeqCst);
        let mut body = request();
        body["tools"] = json!([{"type":"code_interpreter","container":{"type":"auto"}}]);
        body["service_tier"] = json!("priority");
        let response = fixture.send(body).await;
        let status = response.status();
        let response_body: Value = response.json().await.unwrap();
        assert_eq!(status, 200, "{response_body}");
        let captures = fixture.captures.lock().unwrap().clone();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].0, "/backup/responses");
        assert_eq!(captures[0].1["service_tier"], "priority");
        assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 1);
        let report = fixture.report().await;
        assert_eq!(report.account_id, "backup");
        let attempts = report.diagnostics.unwrap().attempts;
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].account_id, "backup");
        assert_eq!(attempts[0].transport, "http");
        assert_eq!(attempts[0].status, Some(200));
        assert!(attempts[0].service_tier_adaptation.is_none());
        assert!(fixture.host.official_failures.lock().unwrap().is_empty());
        fixture.close().await;
    }
}

#[tokio::test]
async fn official_fallback_http_preflight_owner_change_stops_without_pool() {
    let fixture = Fixture::new(500, Value::Null, None, false).await;
    fixture.host.foreign_fallback.store(true, Ordering::SeqCst);
    let mut body = request();
    body["tools"] = json!([{"type":"code_interpreter","container":{"type":"auto"}}]);
    let response = fixture.send(body).await;
    assert_eq!(response.status(), 409);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "official_fallback_owner_changed");
    assert!(fixture.captures.lock().unwrap().is_empty());
    let host = fixture.host.clone();
    fixture.close().await;
    assert!(host.reports.lock().unwrap().is_empty(), "unsent selection failures are not upstream request records");
    assert!(host.official_failures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn official_fallback_quota_cooling_is_observed_before_local_selection_and_preserves_reset() {
    let fixture = Fixture::new(429, json!({"error":{
        "code":"subscription_sharing_usage_limit_exceeded", "message":"Official sharing quota exhausted",
        "resets_in_seconds":7200
    }}), None, false).await;
    fixture.host.local_preflight_error.store(429, Ordering::SeqCst);
    fixture.host.pool_disabled.store(true, Ordering::SeqCst);
    let response = fixture.send(request()).await;
    assert_eq!(response.status(), 429);
    let body: Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("Official sharing quota exhausted"));
    let failures = fixture.host.official_failures.lock().unwrap().clone();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].code, "subscription_sharing_usage_limit_exceeded");
    assert_eq!(failures[0].retry_after, Some(Duration::from_secs(7200)));
    assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.captures.lock().unwrap().iter().map(|capture| capture.0.as_str()).collect::<Vec<_>>(), ["/official/responses"]);
    let report = fixture.report().await;
    assert_eq!(report.status, 429);
    assert_eq!(report.account_id, "source");
    let attempts = report.diagnostics.unwrap().attempts;
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, Some(429));
    assert_eq!(attempts[0].error_category.as_deref(), Some("subscription_sharing_usage_limit_exceeded"));
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_unsent_local_connect_failure_preserves_real_quota_rejection() {
    let fixture = Fixture::new(429, json!({"error":{
        "code":"subscription_sharing_usage_limit_exceeded", "message":"Official sharing quota exhausted"
    }}), None, false).await;
    fixture.host.local_connect_failure.store(true, Ordering::SeqCst);
    fixture.host.pool_disabled.store(true, Ordering::SeqCst);
    let response = fixture.send(request()).await;
    assert_eq!(response.status(), 429);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "subscription_sharing_usage_limit_exceeded");
    assert!(body["error"]["message"].as_str().unwrap().contains("Official sharing quota exhausted"));
    assert_eq!(fixture.captures.lock().unwrap().iter().map(|capture| capture.0.as_str()).collect::<Vec<_>>(), ["/official/responses"]);
    let report = fixture.report().await;
    assert_eq!(report.status, 429);
    assert_eq!(report.account_id, "source");
    let attempts = report.diagnostics.unwrap().attempts;
    assert_eq!(attempts.len(), 1, "a connection refused before send is not a second upstream attempt");
    assert_eq!(attempts[0].status, Some(429));
    assert_eq!(attempts[0].error_category.as_deref(), Some("subscription_sharing_usage_limit_exceeded"));
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_http_connect_failure_is_safe_but_foreign_source_is_rejected() {
    let fixture = Fixture::new(500, Value::Null, None, false).await;
    fixture.host.connect_failure.store(true, Ordering::SeqCst);
    let response = fixture.send(request()).await;
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    assert_eq!(fixture.captures.lock().unwrap().len(), 1);
    assert_eq!(
        fixture.report().await.diagnostics.unwrap().attempts[0]
            .authorization_fallback
            .as_ref()
            .unwrap()
            .error_code
            .as_deref(),
        Some("upstream_connect_failed")
    );
    fixture.close().await;
    let fixture = Fixture::new(
        403,
        json!({"error":{"code":"subscription_sharing_user_not_eligible"}}),
        None,
        false,
    )
    .await;
    fixture.host.foreign_fallback.store(true, Ordering::SeqCst);
    let response = fixture.send(request()).await;
    assert_eq!(response.status(), 409);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "official_fallback_owner_changed");
    assert_eq!(fixture.captures.lock().unwrap().len(), 1);
    let attempts = fixture.report().await.diagnostics.unwrap().attempts;
    assert_eq!(attempts[0].status, Some(403));
    assert_eq!(
        attempts[0].error_category.as_deref(),
        Some("subscription_sharing_user_not_eligible")
    );
    assert_eq!(attempts.len(), 1, "the rejected unsent owner must not create a second attempt");
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_http_never_replays_output_or_parameter_faults_or_opaque_history() {
    let payload = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"already executed\"}\n\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_user_not_eligible\"}}}\n\n";
    let fixture = Fixture::new(200, Value::Null, Some(payload), false).await;
    let response = fixture.send(request()).await;
    let _ = response.bytes().await;
    assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.captures.lock().unwrap().len(), 1);
    fixture.close().await;
    for status in [400, 403] {
        let fixture = Fixture::new(
            status,
            json!({"error":{"code":"invalid_request_error","message":"invalid parameter"}}),
            None,
            false,
        )
        .await;
        let response = fixture.send(request()).await;
        assert_eq!(response.status(), status);
        let _ = response.bytes().await;
        assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 0);
        fixture.close().await;
    }
    let fixture = Fixture::new(
        403,
        json!({"error":{"code":"subscription_sharing_user_not_eligible"}}),
        None,
        false,
    )
    .await;
    let mut body = request();
    body["previous_response_id"] = json!("unknown-owner-response");
    let response = fixture.send(body).await;
    assert!(!response.status().is_success());
    let _ = response.bytes().await;
    assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 0);
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_http_headers_timeout_never_replays_unknown_execution() {
    let fixture = Fixture::new(
        503,
        json!({"error":{"code":"service_unavailable"}}),
        None,
        false,
    )
    .await;
    fixture
        .host
        .official_header_delay
        .store(true, Ordering::SeqCst);
    let response = fixture.send(request()).await;
    assert_eq!(response.status(), 504);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "upstream_connect_timeout");
    assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .captures
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.0.clone())
            .collect::<Vec<_>>(),
        ["/official/responses"]
    );
    let report = fixture.report().await;
    assert_eq!(report.diagnostics.unwrap().attempts.len(), 1);
    assert_eq!(fixture.host.reports.lock().unwrap().len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_handshake_with_tool_output_never_replays_after_rejection() {
    let payload="data: {\"type\":\"response.created\",\"response\":{\"id\":\"executed\",\"output\":[{\"type\":\"function_call\",\"call_id\":\"call-executed\",\"name\":\"write_file\",\"arguments\":\"{}\"}]}}\n\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_user_not_eligible\"}}}\n\n";
    let fixture = Fixture::new(200, Value::Null, Some(payload), false).await;
    let response = fixture.send(request()).await;
    let _ = response.bytes().await;
    assert_eq!(fixture.host.fallback_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.captures.lock().unwrap().len(), 1);
    fixture.close().await;
    assert!(!super::bootstrap::handshake(
        &json!({"type":"response.in_progress","response":{"usage":{"output_tokens":1}}})
    ));
    assert!(super::bootstrap::handshake(
        &json!({"type":"response.created","response":{"output":[],"usage":{"output_tokens":0}}})
    ));
}

#[tokio::test]
async fn official_fallback_initial_route_health_pause_rebuilds_verified_history_and_rejects_opaque_shortcuts(
) {
    let fixture = Fixture::new(200, Value::Null, Some(COMPLETED), false).await;
    let mut first = request();
    first["service_tier"] = json!("priority");
    first["client_metadata"] = json!({"thread_id":"health-thread","session_id":"health-thread"});
    let response = fixture.send(first.clone()).await;
    assert_eq!(response.status(), 200);
    let first_response: Value = response.json().await.unwrap();
    let _ = fixture.report().await;
    fixture.host.direct_local.store(true, Ordering::SeqCst);
    let mut second = first;
    let mut history = vec![
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}),
    ];
    history.extend(first_response["output"].as_array().unwrap().iter().cloned());
    history.push(
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"next"}]}),
    );
    second["input"] = json!(history);
    let response = fixture.send(second).await;
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{body}");
    let captures = fixture.captures.lock().unwrap().clone();
    assert_eq!(
        captures.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(),
        ["/official/responses", "/local/responses"]
    );
    assert!(captures[0].1.get("service_tier").is_none());
    assert_eq!(captures[1].1["service_tier"], "priority");
    assert_eq!(captures[1].1["input"].as_array().unwrap().len(), 3);
    assert!(captures[1].1.get("previous_response_id").is_none());
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.host.reports.lock().unwrap().len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let report = fixture.host.reports.lock().unwrap()[1].clone();
    assert!(report.success);
    let attempts = report.diagnostics.unwrap().attempts;
    assert_eq!(attempts.len(), 1);
    let trace = attempts[0].authorization_fallback.as_ref().unwrap();
    assert_eq!(trace.trigger_stage, "health_pause");
    assert!(trace.status.is_none());
    for unsafe_body in [
        json!({"model":"fixture-model","previous_response_id":"unregistered-response","input":[]}),
        json!({"model":"fixture-model","input":[{"type":"reasoning","encrypted_content":"unregistered-ciphertext"}]}),
    ] {
        let response = fixture.send(unsafe_body).await;
        assert!(!response.status().is_success());
        let _ = response.bytes().await;
    }
    assert_eq!(fixture.captures.lock().unwrap().len(), 2);
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_unsent_local_candidate_preserves_only_the_original_upstream_failure() {
    let fixture=Fixture::new(403,json!({"error":{"code":"subscription_sharing_user_not_eligible","message":"Official user not eligible"}}),None,false).await;
    fixture
        .host
        .local_preflight_error
        .store(429, Ordering::SeqCst);
    fixture.host.pool_disabled.store(true, Ordering::SeqCst);
    let response = fixture.send(request()).await;
    assert_eq!(response.status(), 403);
    let body: Value = response.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Official user not eligible"));
    let report = fixture.report().await;
    let attempts = report.diagnostics.unwrap().attempts;
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, Some(403));
    assert_eq!(
        attempts[0].error_category.as_deref(),
        Some("subscription_sharing_user_not_eligible")
    );
    assert!(attempts[0]
        .error_message
        .as_deref()
        .unwrap()
        .contains("Official user not eligible"));
    assert_eq!(fixture.host.reports.lock().unwrap().len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn official_fallback_source_credentials_unavailable_uses_authorized_pool() {
    let fixture = Fixture::new(
        403,
        json!({"error":{"code":"subscription_sharing_user_not_eligible"}}),
        None,
        false,
    )
    .await;
    fixture
        .host
        .local_preflight_error
        .store(401, Ordering::SeqCst);
    let response = fixture.send(request()).await;
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{body}");
    let attempts = fixture.report().await.diagnostics.unwrap().attempts;
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].status, Some(403));
    assert_eq!(attempts[1].account_id, "backup");
    assert!(attempts.iter().all(|attempt| attempt.transport != "preflight"));
    assert_eq!(
        fixture
            .captures
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.0.clone())
            .collect::<Vec<_>>(),
        ["/official/responses", "/backup/responses"]
    );
    fixture.close().await;
}
