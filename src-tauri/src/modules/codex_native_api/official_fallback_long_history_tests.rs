//! Long conversations retain their owner or migrate complete verified history.
//! All storage and upstreams are isolated; no client profile or real token is used.
use crate::modules::codex_native_api::{
    contract::*, identity::IdentityStore, start, Handle, Options,
};
use axum::{body::Body, extract::OriginalUri, http::HeaderMap, response::Response, Json, Router};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::sync::Notify;

const THREAD: &str = "long-local-fallback-thread";
const LOCAL_OWNER: &str = "source-local-owner";
type Captures = Arc<Mutex<Vec<(String, Value)>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    HealthPause,
    OfficialThenUnavailableLocal,
    OfficialThenQuotaLocal,
}

struct LongHistoryHost {
    base: String,
    screen_candidates: bool,
    flow: Flow,
    official_cooling: AtomicBool,
    order: Arc<Mutex<Vec<String>>>,
    selections: Mutex<Vec<RouteRequest>>,
    reports: Mutex<Vec<RequestReport>>,
    recorded: Notify,
}

impl LongHistoryHost {
    fn selected(&self, request: &RouteRequest, account: &str, official: bool) -> Route {
        Route {
            account_id: account.into(),
            owner_id: if official {
                "official-owner".into()
            } else {
                format!("{account}-local-owner")
            },
            continuity_account_id: official.then(|| "chatgpt_plan_grant".into()),
            installation_profile: None,
            transport_identity: None,
            official_registration_id: official.then(|| "grant-source".into()),
            official_authorization_revision: official.then(|| "epoch".into()),
            official_fallback_registration_id: (!official).then(|| format!("grant-{account}")),
            official_fallback_authorization_revision: (!official).then(|| "epoch".into()),
            authorization_scope_fingerprint: Some("scope".into()),
            official_fallback_failure: (!official).then(|| OfficialFallbackFailure {
                trigger_stage: "health_pause",
                status: 0,
                code: "subscription_sharing_usage_limit_exceeded".into(),
                message: None,
                retry_after: None,
            }),
            base_url: format!(
                "{}/{}",
                self.base,
                if official { "official" } else { account }
            ),
            model: request.model.clone(),
            headers: HeaderMap::new(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            proxy_url: None,
            proxy_route: None,
            chatgpt: !official,
            chatgpt_plan: official,
            wire_api: WireApi::Responses,
            compact_endpoint: None,
            websocket_endpoint: None,
            default_service_tier: None,
            permit: None,
        }
    }
}

impl Host for LongHistoryHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            Ok(Caller {
                id: bearer.into(),
                label: "Synthetic history caller".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let candidates = if self.flow != Flow::HealthPause {
                vec!["source", "backup"]
            } else if self.screen_candidates {
                vec!["source", "screened", "backup"]
            } else {
                vec!["source"]
            };
            let account = candidates
                .into_iter()
                .find(|id| {
                    !request
                        .excluded_account_ids
                        .iter()
                        .any(|excluded| excluded == id)
                })
                .ok_or_else(|| {
                    ServiceError::new(
                        503,
                        "no_available_account",
                        "No synthetic candidate remains",
                    )
                })?;
            self.selections.lock().unwrap().push(request.clone());
            let official = self.flow != Flow::HealthPause
                && account == "source"
                && !self.official_cooling.load(Ordering::SeqCst);
            Ok(self.selected(&request, account, official))
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
            assert_eq!(failure.status, 429);
            assert_eq!(failure.code, "subscription_sharing_usage_limit_exceeded");
            self.order.lock().unwrap().push("official_cooling".into());
            self.official_cooling.store(true, Ordering::SeqCst);
        })
    }
    fn fallback_to_local<'a>(
        &'a self,
        _: &'a Caller,
        failed: &'a Route,
        request: RouteRequest,
        failure: OfficialFallbackFailure,
    ) -> BoxFuture<'a, Result<Option<Route>, ServiceError>> {
        Box::pin(async move {
            assert!(
                self.official_cooling.load(Ordering::SeqCst),
                "cooling must precede local replay selection"
            );
            assert_eq!(failed.account_id, "source");
            assert!(failed.permit.is_none());
            self.order.lock().unwrap().push("local_candidate".into());
            if self.flow == Flow::OfficialThenUnavailableLocal {
                return Err(ServiceError::new(
                    429,
                    "official_local_unavailable",
                    "Synthetic local quota is exhausted before sending",
                ));
            }
            let mut local = self.selected(&request, "source", false);
            local.official_fallback_failure = Some(failure);
            Ok(Some(local))
        })
    }
    fn capability_fallback_decision<'a>(
        &'a self,
        _: &'a Caller,
        route: &'a Route,
        request: &'a RouteRequest,
    ) -> BoxFuture<'a, Result<CapabilityFallbackDecision, ServiceError>> {
        Box::pin(async move {
            if self.flow != Flow::HealthPause && !route.chatgpt_plan {
                assert!(request.required_capabilities.iter().any(|id| id == "function"),
                    "finalize must recheck all restored history tool capabilities before local dispatch");
            }
            Ok(CapabilityFallbackDecision {
                enabled: true,
                unsupported: if self.screen_candidates && route.account_id != "backup" {
                    request.required_capabilities.clone()
                } else {
                    Vec::new()
                },
            })
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async move {
            Ok(ExecutionPolicy {
                credential_retries: if self.flow == Flow::HealthPause { 1 } else { 2 },
                ..Default::default()
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[{"id":"fixture"}]})) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
            self.recorded.notify_one();
        })
    }
}

fn message(role: &str, text: &str) -> Value {
    json!({"type":"message","role":role,"content":[{"type":if role == "assistant" { "output_text" } else { "input_text" },"text":text}]})
}

fn long_input() -> Vec<Value> {
    let mut input = Vec::new();
    for turn in 0..80 {
        input.push(message(
            "user",
            &format!("turn-{turn}:{}", "long-history-".repeat(80)),
        ));
        input.push(message("assistant", &format!("answer-{turn}")));
        if turn % 8 == 0 {
            input.push(json!({"type":"function_call","call_id":format!("call-{turn}"),"name":"read_file","arguments":"{\"path\":\"synthetic\"}"}));
            input.push(json!({"type":"function_call_output","call_id":format!("call-{turn}"),"output":format!("file-{turn}")}));
        }
    }
    input
}

fn request(input: Vec<Value>) -> Value {
    json!({"model":"fixture","stream":false,"instructions":"Preserve the complete synthetic history",
        "tools":[{"type":"function","name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}],"input":input})
}

struct Fixture {
    root: tempfile::TempDir,
    host: Arc<LongHistoryHost>,
    captures: Captures,
    server: tokio::task::JoinHandle<()>,
    handle: Handle,
    body: Value,
}

impl Fixture {
    async fn new(
        seed_account: &str,
        seed_owner: &str,
        encrypted: bool,
        screen_candidates: bool,
    ) -> Self {
        Self::with_flow(
            seed_account,
            seed_owner,
            encrypted,
            screen_candidates,
            Flow::HealthPause,
        )
        .await
    }
    async fn with_flow(
        seed_account: &str,
        seed_owner: &str,
        encrypted: bool,
        screen_candidates: bool,
        flow: Flow,
    ) -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("identity.sqlite");
        let store = IdentityStore::open(path.clone()).unwrap();
        let mut input = long_input();
        let mut initial = request(input.clone());
        let mut headers = HeaderMap::new();
        headers.insert("session-id", THREAD.parse().unwrap());
        headers.insert("thread-id", THREAD.parse().unwrap());
        let mut context = store
            .inspect_for_rebuild("caller", &headers, &initial)
            .unwrap();
        let plan = store
            .configure_rebuild(&mut context, &headers, &initial, true)
            .unwrap()
            .unwrap();
        context.start_rebuild_generation(&plan);
        let binding = store
            .rewrite(
                &context,
                seed_account,
                seed_owner,
                &mut headers,
                &mut initial,
            )
            .unwrap();
        let mut output = Vec::new();
        if encrypted {
            let cipher =
                crate::modules::codex_native_api::codex_cpa_wire::synthetic_gpt_signature(77);
            output.push(json!({"type":"reasoning","id":"synthetic-reasoning","summary":[],"encrypted_content":cipher}));
        }
        output.push(message("assistant", "seed completed"));
        store.observe_event(&binding, &json!({"type":"response.completed","response":{"id":"seed-response","status":"completed","output":output}})).unwrap();
        input.extend(output);
        input.push(message("user", "Continue the long conversation"));
        drop(store);
        let captures: Captures = Arc::new(Mutex::new(Vec::new()));
        let seen = captures.clone();
        let order = Arc::new(Mutex::new(Vec::new()));
        let upstream_order = order.clone();
        let router = Router::new().fallback(move |OriginalUri(uri): OriginalUri, Json(body): Json<Value>| {
            let seen = seen.clone();
            let upstream_order = upstream_order.clone();
            async move {
                let index = { let mut captures = seen.lock().unwrap(); captures.push((uri.path().into(), body)); captures.len() };
                upstream_order.lock().unwrap().push(uri.path().into());
                if uri.path().starts_with("/official/") {
                    return Response::builder().status(429).header("content-type", "application/json")
                        .header("retry-after", "120")
                        .body(Body::from(json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"Synthetic subscription sharing quota exhausted"}}).to_string())).unwrap();
                }
                if flow == Flow::OfficialThenQuotaLocal && uri.path().starts_with("/source/") {
                    return Response::builder().status(429).header("content-type", "application/json")
                        .header("retry-after", "120")
                        .body(Body::from(json!({"error":{"code":"usage_limit_reached","message":"Synthetic local OAuth quota exhausted"}}).to_string())).unwrap();
                }
                let event = json!({"type":"response.completed","response":{"id":format!("reply-{index}"),"status":"completed","output":[message("assistant","continued")],"usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}});
                Response::builder().header("content-type", "text/event-stream")
                    .body(Body::from(format!("data: {event}\n\n"))).unwrap()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = Arc::new(LongHistoryHost {
            base: format!("http://{}", listener.local_addr().unwrap()),
            screen_candidates,
            flow,
            official_cooling: AtomicBool::new(false),
            order,
            selections: Mutex::new(Vec::new()),
            reports: Mutex::new(Vec::new()),
            recorded: Notify::new(),
        });
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            path,
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            root,
            host,
            captures,
            server,
            handle,
            body: request(input),
        }
    }
    async fn send_as(&self, caller: &str, body: &Value) -> (u16, Value) {
        let response = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", self.handle.addr))
            .bearer_auth(caller)
            .header("session-id", THREAD)
            .header("thread-id", THREAD)
            .json(body)
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.json().await.unwrap())
    }
    async fn wait_reports(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let recorded = self.host.recorded.notified();
                if self.host.reports.lock().unwrap().len() >= count {
                    break;
                }
                recorded.await;
            }
        })
        .await
        .unwrap();
    }
    fn active_owner(&self) -> (String, String) {
        rusqlite::Connection::open(self.root.path().join("identity.sqlite")).unwrap()
            .query_row("SELECT account,owner FROM native_rebuild_active_v1 WHERE caller='caller' AND thread=?1", [THREAD], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
    }
    async fn close(mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.server.abort();
    }
}

#[tokio::test]
async fn long_history_same_local_owner_preserves_cipher_and_tools_across_followup_turns() {
    let fixture = Fixture::new("source", LOCAL_OWNER, true, false).await;
    let expected_cipher = fixture.body["input"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|item| item["encrypted_content"].as_str())
        .unwrap()
        .to_owned();
    assert!(
        crate::modules::codex_native_api::codex_cpa_wire::gpt_signature_valid(&expected_cipher)
    );
    let mut body = fixture.body.clone();
    for turn in 0..2 {
        let (status, response) = fixture.send_as("caller", &body).await;
        assert_eq!(status, 200, "{response}");
        fixture.wait_reports(turn + 1).await;
        let output = response["output"].as_array().unwrap().clone();
        let input = body["input"].as_array_mut().unwrap();
        input.extend(output);
        input.push(message("user", "Continue again"));
    }
    assert_eq!(
        fixture.active_owner(),
        ("source".into(), LOCAL_OWNER.into())
    );
    let captures = fixture.captures.lock().unwrap().clone();
    assert_eq!(captures.len(), 2);
    for (path, body) in captures {
        assert_eq!(path, "/source/responses");
        let input = body["input"].as_array().unwrap();
        assert!(input
            .iter()
            .any(|item| item["encrypted_content"].as_str() == Some(expected_cipher.as_str())));
        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == "function_call")
                .count(),
            10
        );
        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == "function_call_output")
                .count(),
            10
        );
        assert!(input
            .iter()
            .any(|item| item.to_string().contains("turn-79:")));
    }
    fixture.close().await;
}

#[tokio::test]
async fn long_history_cross_owner_cipher_requests_full_input_without_touching_upstream_or_owner() {
    for (account, owner) in [
        ("chatgpt_plan_grant", "official-owner"),
        ("source", "previous-local-owner"),
    ] {
        let fixture = Fixture::new(account, owner, true, false).await;
        let before = fixture.active_owner();
        let (status, response) = fixture.send_as("caller", &fixture.body).await;
        assert_eq!(status, 400, "{response}");
        assert_eq!(response["error"]["code"], "continuation_history_incomplete");
        assert!(fixture.captures.lock().unwrap().is_empty());
        assert_eq!(fixture.active_owner(), before);
        fixture.close().await;
    }
}

#[tokio::test]
async fn long_history_screened_candidates_use_no_upstream_budget_and_only_backup_is_recorded() {
    let fixture = Fixture::new("chatgpt_plan_grant", "official-owner", false, true).await;
    let (status, response) = fixture.send_as("caller", &fixture.body).await;
    assert_eq!(status, 200, "{response}");
    fixture.wait_reports(1).await;
    assert_eq!(fixture.host.selections.lock().unwrap().len(), 3);
    let captures = fixture.captures.lock().unwrap().clone();
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].0, "/backup/responses");
    assert!(
        captures[0].1["input"].as_array().unwrap().len()
            >= fixture.body["input"].as_array().unwrap().len()
    );
    let reports = fixture.host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(reports[0].success);
    let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].account_id, "backup");
    drop(reports);
    assert_eq!(
        fixture.active_owner(),
        ("backup".into(), "backup-local-owner".into())
    );
    fixture.close().await;
}

#[tokio::test]
async fn long_history_unpaired_tools_request_context_and_foreign_caller_still_fails_closed() {
    let fixture = Fixture::new("chatgpt_plan_grant", "official-owner", false, false).await;
    let mut body = fixture.body.clone();
    body["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"function_call_output","call_id":"missing-call","output":"unpaired"}));
    let (status, response) = fixture.send_as("caller", &body).await;
    assert_eq!(status, 400, "{response}");
    assert_eq!(response["error"]["code"], "continuation_history_incomplete");
    assert!(fixture.captures.lock().unwrap().is_empty());
    fixture.close().await;
    let fixture = Fixture::new("source", LOCAL_OWNER, true, false).await;
    let (status, response) = fixture.send_as("foreign-caller", &fixture.body).await;
    assert_eq!(status, 409, "{response}");
    assert_eq!(response["error"]["code"], "upstream_state_caller_mismatch");
    assert!(fixture.captures.lock().unwrap().is_empty());
    fixture.close().await;
}

fn semantic_history(body: &Value) -> Vec<Value> {
    body["input"].as_array().unwrap().iter().filter_map(|item| {
        match item["type"].as_str() {
            Some("message") if matches!(item["role"].as_str(), Some("user" | "assistant")) => {
                let parts = item["content"].as_array().unwrap().iter()
                    .map(|part| json!({"type":part["type"],"text":part["text"]})).collect::<Vec<_>>();
                Some(json!({"type":"message","role":item["role"],"content":parts}))
            }
            Some("function_call") => Some(json!({"type":"function_call","call_id":item["call_id"],"name":item["name"],"arguments":item["arguments"]})),
            Some("function_call_output") => Some(json!({"type":"function_call_output","call_id":item["call_id"],"output":item["output"]})),
            _ => None,
        }
    }).collect()
}

async fn assert_long_history_official_quota_failover(flow: Flow) {
    let fixture =
        Fixture::with_flow("chatgpt_plan_grant", "official-owner", false, false, flow).await;
    let original = semantic_history(&fixture.body);
    let (status, response) = fixture.send_as("caller", &fixture.body).await;
    assert_eq!(
        status, 200,
        "intermediate quota failures must remain internal: {response}"
    );
    assert!(response.get("error").is_none(), "{response}");
    fixture.wait_reports(1).await;
    assert!(fixture.host.official_cooling.load(Ordering::SeqCst));
    let local_sent = flow == Flow::OfficialThenQuotaLocal;
    let mut paths = vec!["/official/responses"];
    if local_sent {
        paths.push("/source/responses");
    }
    paths.push("/backup/responses");
    let captures = fixture.captures.lock().unwrap().clone();
    assert_eq!(
        captures
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        paths
    );
    for (_, body) in &captures {
        assert_eq!(
            semantic_history(body),
            original,
            "all 80 turns and all call/result pairs must survive every actual attempt"
        );
        assert_eq!(body["instructions"], fixture.body["instructions"]);
        let input = body["input"].as_array().unwrap();
        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == "function_call")
                .count(),
            10
        );
        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == "function_call_output")
                .count(),
            10
        );
        assert!(body.get("previous_response_id").is_none());
    }
    let mut order = vec!["/official/responses", "official_cooling", "local_candidate"];
    if local_sent {
        order.push("/source/responses");
    }
    order.push("/backup/responses");
    assert_eq!(
        fixture
            .host
            .order
            .lock()
            .unwrap()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        order
    );
    {
        let reports = fixture.host.reports.lock().unwrap();
        assert_eq!(
            reports.len(),
            1,
            "candidate screening must never create a separate failure record"
        );
        let report = &reports[0];
        assert!(report.success);
        let attempts = &report.diagnostics.as_ref().unwrap().attempts;
        assert_eq!(attempts.len(), if local_sent { 3 } else { 2 });
        assert_eq!(attempts[0].authorization_route.as_deref(), Some("official"));
        assert_eq!(attempts[0].status, Some(429));
        if local_sent {
            assert_eq!(attempts[1].account_id, "source");
            assert_eq!(
                attempts[1].authorization_route.as_deref(),
                Some("local_gateway")
            );
            assert_eq!(attempts[1].status, Some(429));
        }
        let final_attempt = attempts.last().unwrap();
        assert_eq!(final_attempt.account_id, "backup");
        assert_eq!(final_attempt.status, Some(200));
        assert!(final_attempt.success);
        assert!(attempts
            .iter()
            .all(|attempt| attempt.transport != "preflight"));
    }
    assert_eq!(
        fixture.active_owner(),
        ("backup".into(), "backup-local-owner".into())
    );
    fixture.close().await;
}

#[tokio::test]
async fn long_history_official_429_cools_then_skips_unsent_local_quota_candidate_and_uses_backup() {
    assert_long_history_official_quota_failover(Flow::OfficialThenUnavailableLocal).await;
}

#[tokio::test]
async fn long_history_official_429_then_real_local_429_records_three_actual_attempts_before_backup()
{
    assert_long_history_official_quota_failover(Flow::OfficialThenQuotaLocal).await;
}
