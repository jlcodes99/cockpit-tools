//! Exercise the production account selector through the HTTP/WS engine.
//! Both upstreams are loopback fixtures; no real credentials or service are used.
use super::*;
use axum::{
    body::Body,
    extract::Path as AxumPath,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

/// Real native routes persist installation identities even when accounts are
/// supplied in memory. Callers must hold test_support::env_lock until shutdown.
pub(super) struct NativeInstallationTestData {
    _directory: tempfile::TempDir,
    previous: [Option<std::ffi::OsString>; 2],
}

impl NativeInstallationTestData {
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let previous = [
            std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR"),
            std::env::var_os("COCKPIT_TOOLS_DATA_DIR"),
        ];
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", directory.path());
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", directory.path());
        Self { _directory: directory, previous }
    }
}

impl Drop for NativeInstallationTestData {
    fn drop(&mut self) {
        for (key, previous) in ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"]
            .into_iter().zip(&self.previous)
        {
            if let Some(previous) = previous {
                std::env::set_var(key, previous);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

struct IntegrationHost {
    inner: NativeHostBridge,
    reports: Mutex<Vec<native_contract::RequestReport>>,
}

impl native_contract::Host for IntegrationHost {
    fn authorize<'a>(
        &'a self,
        bearer: &'a str,
    ) -> BoxFuture<'a, Result<native_contract::Caller, native_contract::ServiceError>> {
        native_contract::Host::authorize(&self.inner, bearer)
    }
    fn route<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: native_contract::RouteRequest,
    ) -> BoxFuture<'a, Result<native_contract::Route, native_contract::ServiceError>> {
        native_contract::Host::route(&self.inner, caller, request)
    }
    fn capability_fallback_decision<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        route: &'a native_contract::Route,
        request: &'a native_contract::RouteRequest,
    ) -> BoxFuture<
        'a,
        Result<native_contract::CapabilityFallbackDecision, native_contract::ServiceError>,
    > {
        native_contract::Host::capability_fallback_decision(&self.inner, caller, route, request)
    }
    fn prepare_body<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        account: &'a native_contract::Route,
        headers: &'a axum::http::HeaderMap,
        body: Value,
        image: bool,
        compact: bool,
    ) -> BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        native_contract::Host::prepare_body(
            &self.inner,
            caller,
            account,
            headers,
            body,
            image,
            compact,
        )
    }
    fn models<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
    ) -> BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        native_contract::Host::models(&self.inner, caller)
    }
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, native_contract::ServiceError>> {
        native_contract::Host::websockets_enabled(&self.inner)
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        route: &'a native_contract::Route,
    ) -> BoxFuture<'a, Result<bool, native_contract::ServiceError>> {
        native_contract::Host::responses_websocket_passthrough(&self.inner, caller, route)
    }
    fn execution_policy(
        &self,
    ) -> BoxFuture<'_, Result<native_contract::ExecutionPolicy, native_contract::ServiceError>>
    {
        Box::pin(async {
            Ok(native_contract::ExecutionPolicy {
                total_timeout: Duration::from_secs(15),
                credential_retries: 1,
                ..Default::default()
            })
        })
    }
    fn record(&self, report: native_contract::RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.inner.record_failure_cooldown(&report);
            self.inner.record_soft_binding_result(&report).await;
            self.reports.lock().unwrap().push(report);
        })
    }
}

fn host(base: &str, enabled: bool, declared: bool, scoped: bool) -> IntegrationHost {
    let allowed = if scoped {
        vec!["primary"]
    } else {
        vec!["primary", "backup"]
    };
    let collection = serde_json::from_value(json!({
        "enabled":true,"port":1455,"apiKey":"fixture-client","accountIds":["primary","backup"],
        "createdAt":1,"updatedAt":1,"gatewayMode":"native","sessionAffinity":true,
        "responsesWebsocketsEnabled":true,
        "apiKeys":[{"id":"fixture-caller","label":"Fixture","key":"fixture-client",
            "inheritAccountPool":false,"accountIds":allowed,"priorityAccountIds":["primary"]}],
        "capabilityFallback":{"enabled":enabled,"accountIds":["backup"],"overrides":[
            {"accountId":"primary","model":"*","capabilities":{"mcp":"unsupported"}},
            {"accountId":"backup","model":"fixture-model","capabilities":if declared { json!({"mcp":"supported"}) } else { json!({}) }}
        ]}
    })).unwrap();
    let mut inner = NativeHostBridge::for_collection(collection);
    for id in ["primary", "backup"] {
        let mut account = CodexAccount::new_api_key(
            id.into(),
            "fixture@example.invalid".into(),
            "synthetic-upstream-key".into(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some(format!("{base}/{id}/v1")),
            None,
            None,
            vec!["fixture-model".into()],
        );
        account.api_wire_api = Some("responses".into());
        account.egress_proxy_disabled = true;
        inner.account_overrides.insert(id.into(), account);
    }
    IntegrationHost {
        inner,
        reports: Mutex::new(Vec::new()),
    }
}

fn body(stream: bool) -> Value {
    json!({"model":"fixture-model","stream":stream,"input":[{"role":"user","content":"Synthetic routing check"}],
        "tools":[{"type":"mcp","server_label":"fixture","server_url":"https://example.invalid/mcp","require_approval":"always"}]})
}

fn response(body: &Value, response_id: &str) -> Response {
    let item = json!({"id":"msg_fixture","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"OK","annotations":[]}]});
    let terminal = json!({"id":response_id,"object":"response","status":"completed","model":body["model"],"output":[item.clone()],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}});
    if body["stream"] == true {
        let events = [
            json!({"type":"response.created","response":{"id":response_id,"status":"in_progress","output":[]}}),
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            json!({"type":"response.completed","response":terminal}),
        ];
        let bytes = events
            .iter()
            .map(|event| {
                format!(
                    "event: {}\ndata: {event}\n\n",
                    event["type"].as_str().unwrap()
                )
            })
            .collect::<String>();
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(bytes))
            .unwrap()
    } else {
        Json(terminal).into_response()
    }
}

async fn run(enabled: bool, declared: bool, scoped: bool, socket: bool, continuation: bool) {
    let _env = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _data = NativeInstallationTestData::new();
    let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let capture = seen.clone();
    let router = Router::new().route(
        "/{account}/v1/responses",
        post(
            move |AxumPath(account): AxumPath<String>, Json(body): Json<Value>| {
                let capture = capture.clone();
                async move {
                    let response_id = format!("resp_{account}");
                    capture.lock().unwrap().push((account, body.clone()));
                    response(&body, &response_id)
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let host = Arc::new(host(&base, enabled, declared, scoped));
    let root = tempfile::tempdir().unwrap();
    let mut gateway = native_api::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.path().join("identity.sqlite"),
        native_api::Options::default(),
    )
    .await
    .unwrap();
    let succeeds = !scoped;
    if socket {
        let mut request = format!("ws://{}/v1/responses", gateway.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer fixture-client".parse().unwrap());
        let (mut connection, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let mut previous = None;
        for turn in 0..if continuation { 2 } else { 1 } {
            let mut message = body(true);
            if continuation && turn == 0 {
                message.as_object_mut().unwrap().remove("tools");
            } else if let Some(id) = &previous {
                message["previous_response_id"] = json!(id);
                message["input"] =
                    json!([{"role":"user","content":"Continue with the new tool capability"}]);
            }
            message["type"] = json!("response.create");
            connection
                .send(Message::Text(message.to_string().into()))
                .await
                .unwrap();
            let mut completed = false;
            while let Some(frame) = timeout(Duration::from_secs(20), connection.next())
                .await
                .unwrap()
            {
                if let Message::Text(text) = frame.unwrap() {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    assert_ne!(event["type"], "error", "{event}");
                    if event["type"] == "response.completed" {
                        previous = event["response"]["id"].as_str().map(str::to_owned);
                        completed = true;
                        break;
                    }
                }
            }
            assert!(completed);
        }
        connection.close(None).await.unwrap();
    } else {
        let result = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", gateway.addr))
            .bearer_auth("fixture-client")
            .json(&body(false))
            .send()
            .await
            .unwrap();
        let status = result.status();
        let value: Value = result.json().await.unwrap();
        assert_eq!(status.is_success(), succeeds, "{value}");
        if succeeds {
            assert_eq!(value["status"], "completed");
        } else {
            assert!(
                value["error"]["code"]
                    .as_str()
                    .unwrap()
                    .contains("capability_fallback"),
                "{value}"
            );
        }
    }
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
    let seen = seen.lock().unwrap();
    if !succeeds {
        assert!(
            seen.is_empty(),
            "incompatible primary and unauthorized backup must never receive the request"
        );
        return;
    }
    assert_eq!(seen.len(), if continuation { 2 } else { 1 });
    let last = seen.last().unwrap();
    assert_eq!(last.0, "backup");
    assert_eq!(last.1["model"], "fixture-model");
    if continuation {
        assert_eq!(seen[0].0, "primary");
        assert!(last.1.get("previous_response_id").is_none());
        let history = last.1["input"].to_string();
        assert!(history.contains("Synthetic routing check"));
        assert!(history.contains("Continue with the new tool capability"));
        assert!(last.1["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["role"] == "assistant"));
    }
    assert_eq!(
        last.1["tools"],
        body(false)["tools"],
        "fallback must not drop or rewrite hosted tool declarations"
    );
    let reports = host.reports.lock().unwrap();
    let report = reports
        .iter()
        .rev()
        .find(|report| report.success)
        .expect("successful report");
    let detail = native_diagnostic_detail(report.diagnostics.clone().expect("diagnostics"), false);
    assert!(detail.payloads.is_empty());
    assert_eq!(detail.attempts.len(), 1);
    let trace = &detail.attempts[0].capability_fallback;
    let trace = trace.as_ref().expect("persistable automatic capability fallback metadata");
    assert_eq!(trace.from_account_id, "primary");
    assert_eq!(trace.to_account_id, "backup");
    assert_eq!(trace.reason, "preflight");
    assert_eq!(trace.capabilities, vec!["mcp"]);
}

#[tokio::test]
async fn capability_fallback_real_host_http_ignores_legacy_opt_out_and_enforces_backup_scope() {
    for (enabled, declared, scoped) in [
        (true, true, false),
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        run(enabled, declared, scoped, false, false).await;
    }
}

#[tokio::test]
async fn capability_fallback_real_host_websocket_uses_the_same_eligible_backup_and_logs() {
    run(true, true, false, true, false).await;
}

#[tokio::test]
async fn capability_fallback_real_host_websocket_continuation_moves_complete_history() {
    run(true, true, false, true, true).await;
}

#[tokio::test]
async fn automatic_fallback_does_not_preempt_legacy_shell_or_unknown_extensions_on_the_primary() {
    let _env = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _data = NativeInstallationTestData::new();
    let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let capture = seen.clone();
    let router = Router::new().route("/{account}/v1/responses", post(
        move |AxumPath(account): AxumPath<String>, Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push((account, body.clone()));
                response(&body, "resp_primary_legacy")
            }
        }
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    let mut fixture = host(&base, false, false, true);
    fixture.inner.collection_override.as_mut().unwrap().capability_fallback = Default::default();
    let host = Arc::new(fixture);
    let root = tempfile::tempdir().unwrap();
    let mut gateway = native_api::start("127.0.0.1:0".parse().unwrap(), host.clone(),
        root.path().join("identity.sqlite"), native_api::Options::default()).await.unwrap();
    let body = json!({"model":"fixture-model","stream":false,"tools":[{"type":"future_extension"}],"input":[
        {"type":"local_shell_call","id":"legacy_shell","call_id":"call_legacy_shell","status":"completed","action":{"type":"exec","command":["echo","fixture"]}},
        {"type":"function_call_output","call_id":"call_legacy_shell","output":"fixture"},
        {"role":"user","content":"Continue the existing task"}
    ]});
    let result = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(15)).build().unwrap()
        .post(format!("http://{}/v1/responses", gateway.addr)).bearer_auth("fixture-client")
        .json(&body).send().await.unwrap();
    let status = result.status();
    let result: Value = result.json().await.unwrap();
    assert!(status.is_success(), "{result}");
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "primary");
    assert_eq!(seen[0].1["tools"], body["tools"]);
    assert!(host.reports.lock().unwrap().iter().all(|report|
        report.diagnostics.as_ref().is_none_or(|diagnostics| diagnostics.attempts.iter().all(|attempt| attempt.capability_fallback.is_none()))));
}
