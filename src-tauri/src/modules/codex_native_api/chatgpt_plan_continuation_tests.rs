//! Client transports and durable history for the stateless public plan route.
use super::{contract::*, Handle, Options};
use axum::{body::Body, http::HeaderMap, response::Response, routing::post, Json, Router};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

struct PlanHost {
    base: String,
}

impl Host for PlanHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            Ok(Caller {
                id: bearer.into(),
                label: "synthetic".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            Ok(Route {
                account_id: "chatgpt_plan_synthetic".into(),
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
                headers: HeaderMap::new(),
                client: client(),
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
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    // Deliberately retain Host's default passthrough policy. A plan route must
    // still force the HTTP bridge instead of attempting an upstream socket.
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
fn message(role: &str, text: &str) -> Value {
    json!({"type":"message","role":role,"content":[{"type":if role == "assistant" {"output_text"} else {"input_text"},"text":text}]})
}
fn full_request() -> Value {
    json!({"model":"plan-model","stream":false,"instructions":"Keep the synthetic instruction",
        "tools":[{"type":"function","name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}],
        "input":[message("user","Read the synthetic file")]})
}

struct Fixture {
    host: Arc<PlanHost>,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
    root: tempfile::TempDir,
    handle: Handle,
}
impl Fixture {
    async fn new(discovery: bool) -> Self {
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = requests.clone();
        let upstream = Router::new().route("/v1/responses", post(move |Json(body): Json<Value>| {
            let captured = captured.clone();
            async move {
                let count = { let mut requests = captured.lock().unwrap(); requests.push(body.clone()); requests.len() };
                let output = if count == 1 {
                    if discovery {
                        let name = body["input"].as_array().unwrap().iter()
                            .find(|item| item["type"] == "additional_tools").unwrap()["tools"][0]["name"].clone();
                        json!([{"type":"function_call","id":"search-item","call_id":"search-call","name":name,"arguments":"{\"query\":\"read file\"}"}])
                    } else {
                        json!([{"type":"function_call","id":"read-item","call_id":"read-call","name":"read_file","arguments":"{\"path\":\"fixture.txt\"}"}])
                    }
                } else if discovery && count == 2 {
                    json!([{"type":"function_call","id":"read-item","call_id":"read-call","name":"read_file","arguments":"{\"path\":\"fixture.txt\"}"}])
                } else { json!([message("assistant", "Synthetic file was read")]) };
                let event = json!({"type":"response.completed","response":{"id":format!("plan-response-{count}"),"status":"completed","output":output,
                    "usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}});
                Response::builder().header("content-type","text/event-stream")
                    .body(Body::from(format!("event: response.completed\ndata: {event}\n\n"))).unwrap()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = Arc::new(PlanHost {
            base: format!("http://{}/v1", listener.local_addr().unwrap()),
        });
        let task = tokio::spawn(async move {
            axum::serve(listener, upstream).await.unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let handle = super::start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            host,
            requests,
            task,
            root,
            handle,
        }
    }
    async fn request(&self, caller: &str, body: Value) -> reqwest::Response {
        client()
            .post(format!("http://{}/v1/responses", self.handle.addr))
            .bearer_auth(caller)
            .header("session-id", "plan-session")
            .header("thread-id", "plan-thread")
            .json(&body)
            .send()
            .await
            .unwrap()
    }
    async fn restart(&mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.handle = super::start(
            "127.0.0.1:0".parse().unwrap(),
            self.host.clone(),
            self.root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
    }
    async fn close(mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.task.abort();
    }
}

#[tokio::test]
async fn chatgpt_plan_http_checkpoint_restores_tools_after_restart_without_crossing_callers() {
    let mut fixture = Fixture::new(false).await;
    let first = fixture.request("owner", full_request()).await;
    assert_eq!(first.status(), 200);
    let first: Value = first.json().await.unwrap();
    assert_eq!(first["output"][0]["call_id"], "read-call");
    fixture.restart().await;
    let next = json!({"model":"plan-model","stream":false,"previous_response_id":first["id"],
        "input":[{"type":"function_call_output","call_id":"read-call","output":"synthetic contents"}]});
    let foreign = fixture.request("foreign", next.clone()).await;
    assert!(!foreign.status().is_success());
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    let second = fixture.request("owner", next).await;
    let status = second.status();
    let body: Value = second.json().await.unwrap();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "completed");
    {
        let requests = fixture.requests.lock().unwrap();
        let sent = &requests[1];
        assert!(sent.get("previous_response_id").is_none());
        assert_eq!(sent["store"], false);
        assert_eq!(sent["stream"], true);
        assert_eq!(sent["instructions"], "Keep the synthetic instruction");
        let input = sent["input"].as_array().unwrap();
        assert!(input
            .iter()
            .any(|item| item["type"] == "function_call" && item["call_id"] == "read-call"));
        assert!(input
            .iter()
            .any(|item| item["type"] == "function_call_output"
                && item["output"] == "synthetic contents"));
        assert!(input
            .iter()
            .any(|item| item["type"] == "additional_tools"
                && item["tools"][0]["name"] == "read_file"));
    }
    fixture.close().await;
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn terminal(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if matches!(
                        value["type"].as_str(),
                        Some("response.completed" | "response.failed" | "error")
                    ) {
                        return value;
                    }
                }
                Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                other => panic!("unexpected socket event: {other:?}"),
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn chatgpt_plan_websocket_bridges_discovery_and_tool_results_over_public_http() {
    let fixture = Fixture::new(true).await;
    let mut request = format!("ws://{}/v1/responses", fixture.handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer owner".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket.send(Message::Text(json!({"type":"response.create","model":"plan-model","input":[message("user","Find a file tool")],
        "tools":[{"type":"tool_search","execution":"client","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}]}).to_string().into())).await.unwrap();
    let first = terminal(&mut socket).await;
    assert_eq!(first["type"], "response.completed", "{first}");
    assert_eq!(first["response"]["output"][0]["type"], "tool_search_call");
    socket.send(Message::Text(json!({"type":"response.create","model":"plan-model","previous_response_id":first["response"]["id"],
        "input":[{"type":"tool_search_output","call_id":"search-call","execution":"client","tools":[
            {"type":"function","name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}
        ]}]}).to_string().into())).await.unwrap();
    let second = terminal(&mut socket).await;
    assert_eq!(second["type"], "response.completed", "{second}");
    assert_eq!(second["response"]["output"][0]["name"], "read_file");
    socket.send(Message::Text(json!({"type":"response.create","model":"plan-model","previous_response_id":second["response"]["id"],
        "input":[{"type":"function_call_output","call_id":"read-call","output":"synthetic contents"}]}).to_string().into())).await.unwrap();
    let third = terminal(&mut socket).await;
    assert_eq!(third["type"], "response.completed", "{third}");
    assert_eq!(
        third["response"]["output"][0]["content"][0]["text"],
        "Synthetic file was read"
    );
    {
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        for body in requests.iter() {
            assert!(body.get("previous_response_id").is_none());
            assert_eq!(body["store"], false);
            assert_eq!(body["stream"], true);
            assert!(!body.to_string().contains("\"type\":\"tool_search\""));
            assert!(!body.to_string().contains("\"type\":\"tool_search_call\""));
            assert!(!body.to_string().contains("\"type\":\"tool_search_output\""));
        }
    }
    socket.close(None).await.unwrap();
    fixture.close().await;
}

/// Run explicitly with COCKPIT_PLAN_CODEX_BIN. The real CLI only contacts this
/// loopback gateway; the scripted upstream never calls OpenAI or a real model.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires an explicitly selected local Codex CLI"]
async fn chatgpt_plan_real_codex_cli_reads_edits_and_checks_a_temporary_file() {
    use std::os::unix::process::CommandExt;
    fn exec_namespace(tools: &[Value], namespace: Option<&str>) -> Option<Option<String>> {
        for tool in tools {
            if tool["type"] == "namespace" {
                if let (Some(name), Some(children)) =
                    (tool["name"].as_str(), tool["tools"].as_array())
                {
                    if let Some(found) = exec_namespace(children, Some(name)) {
                        return Some(found);
                    }
                }
            } else if tool["type"] == "function" && tool["name"] == "exec_command" {
                return Some(tool["namespace"].as_str().or(namespace).map(str::to_owned));
            }
        }
        None
    }
    let binary = std::env::var("COCKPIT_PLAN_CODEX_BIN").expect("set COCKPIT_PLAN_CODEX_BIN");
    let workspace = tempfile::tempdir().unwrap();
    let workdir = workspace.path().to_string_lossy().into_owned();
    let file = workspace.path().join("fixture.txt");
    std::fs::write(&file, "before\n").unwrap();
    let captures = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = captures.clone();
    let upstream = Router::new().route("/v1/responses",post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let workdir = workdir.clone(); let captured = captured.clone();
        async move {
            // The local client key must never be forwarded to the model provider.
            assert!(!headers.contains_key("authorization"));
            let mut tools = body["tools"].as_array().cloned().unwrap_or_default();
            for item in body["input"].as_array().unwrap() {
                if item["type"] == "additional_tools" { tools.extend(item["tools"].as_array().unwrap().iter().cloned()); }
            }
            let namespace = exec_namespace(&tools, None).expect("official CLI must advertise exec_command");
            let count = {let mut requests=captured.lock().unwrap();requests.push(body);requests.len()};
            let output = if count<=3 {
                let command = match count {1=>"cat fixture.txt",2=>"printf 'after\\n' > fixture.txt",_=>"cat fixture.txt"};
                let mut call = json!({"id":format!("item-{count}"),"type":"function_call","call_id":format!("call-{count}"),"name":"exec_command",
                    "arguments":json!({"cmd":command,"workdir":workdir,"max_output_tokens":1000,"shell":"/bin/sh","login":false}).to_string(),"status":"completed"});
                if let Some(namespace) = namespace { call["namespace"] = json!(namespace); }
                json!([call])
            } else {json!([message("assistant","OK: temporary file read, edited, and checked")])};
            let response=json!({"id":format!("cli-response-{count}"),"object":"response","model":"gpt-6.1-sol","created_at":chrono::Utc::now().timestamp(),
                "status":"completed","output":output,"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}});
            let mut events=vec![json!({"type":"response.created","response":{"id":response["id"],"status":"in_progress","output":[]}})];
            for (index,item) in output.as_array().unwrap().iter().enumerate() {
                events.push(json!({"type":"response.output_item.added","output_index":index,"item":item}));
                events.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
            }
            events.push(json!({"type":"response.completed","response":response}));
            let wire=events.into_iter().map(|event|format!("event: {}\ndata: {event}\n\n",event["type"].as_str().unwrap())).collect::<String>();
            Response::builder().header("content-type","text/event-stream").body(Body::from(wire)).unwrap()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(PlanHost {
        base: format!("http://{}/v1", listener.local_addr().unwrap()),
    });
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let state = tempfile::tempdir().unwrap();
    let mut handle = super::start(
        "127.0.0.1:0".parse().unwrap(),
        host,
        state.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let codex_home = tempfile::tempdir().unwrap();
    let cli_tmp = codex_home.path().join("tmp");
    std::fs::create_dir(&cli_tmp).unwrap();
    #[cfg(target_os = "macos")]
    let mut command = {
        let workdir =
            serde_json::to_string(&std::fs::canonicalize(workspace.path()).unwrap()).unwrap();
        let home =
            serde_json::to_string(&std::fs::canonicalize(codex_home.path()).unwrap()).unwrap();
        // A Seatbelt process cannot install the CLI's second Seatbelt sandbox.
        // The outer profile enforces the network and temporary writable roots
        // for both the CLI and its shell children instead.
        let profile = format!("(version 1)(allow default)(deny network*)(allow network-outbound (remote ip \"localhost:*\"))(allow network-inbound (local ip \"localhost:*\"))(deny file-write*)(allow file-write* (subpath {workdir}) (subpath {home}) (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/ptmx\"))");
        let mut command = tokio::process::Command::new("/usr/bin/sandbox-exec");
        command.args(["-p", &profile]).arg(binary);
        command
    };
    #[cfg(not(target_os = "macos"))]
    let mut command = tokio::process::Command::new(binary);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", codex_home.path())
        .env("CODEX_HOME", codex_home.path())
        .env("TMPDIR", &cli_tmp)
        .args([
            "exec",
            "--ignore-user-config",
            "--ignore-rules",
            "--ephemeral",
            "--skip-git-repo-check",
            "--sandbox",
            if cfg!(target_os = "macos") {
                "danger-full-access"
            } else {
                "workspace-write"
            },
            "--json",
            "--model",
            "gpt-6.1-sol",
            "--cd",
        ])
        .arg(workspace.path())
        .env("COCKPIT_PLAN_CONTRACT_TOKEN", "synthetic-loopback-only")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    command.as_std_mut().process_group(0);
    for setting in [
        "model_provider=\"plan_contract\"".to_string(),
        "model_providers.plan_contract.name=\"Loopback contract\"".into(),
        format!(
            "model_providers.plan_contract.base_url=\"http://{}/v1\"",
            handle.addr
        ),
        "model_providers.plan_contract.env_key=\"COCKPIT_PLAN_CONTRACT_TOKEN\"".into(),
        "model_providers.plan_contract.wire_api=\"responses\"".into(),
        "model_providers.plan_contract.requires_openai_auth=false".into(),
        "model_providers.plan_contract.supports_websockets=false".into(),
        "mcp_servers={}".into(),
        "features.apps=false".into(),
        "features.plugins=false".into(),
        "features.hooks=false".into(),
        "features.shell_snapshot=false".into(),
        "features.enable_request_compression=false".into(),
        "web_search=\"disabled\"".into(),
        "history.persistence=\"none\"".into(),
        "approval_policy=\"never\"".into(),
    ] {
        command.args(["-c", &setting]);
    }
    command.arg("Read fixture.txt, replace its content with after, check the result, then report OK. Use only this temporary workspace.");
    let child = command.spawn().unwrap();
    let pid = child.id().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output()).await;
    if result.is_err() {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let result = result.expect("local Codex contract timed out").unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    assert!(
        result.status.success(),
        "local Codex failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let requests = captures.lock().unwrap();
    assert_eq!(requests.len(), 4);
    let tool_outputs: Vec<_> = requests[3]["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .collect();
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "after\n",
        "actual CLI tool outputs: {tool_outputs:?}"
    );
    for index in 1..=3 {
        let call_id = format!("call-{index}");
        let output = tool_outputs
            .iter()
            .find(|item| item["call_id"] == call_id)
            .expect("CLI must return every executed command");
        assert!(
            output["output"]
                .as_str()
                .unwrap()
                .contains("Process exited with code 0"),
            "command failed: {output}"
        );
    }
    assert!(requests[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "function_call_output"
            && item["call_id"] == "call-1"
            && item["output"].to_string().contains("before")));
    assert!(requests[3]["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "function_call_output"
            && item["call_id"] == "call-3"
            && item["output"].to_string().contains("after")));
    for body in requests.iter() {
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert!(body.get("previous_response_id").is_none());
    }
}

/// Explicit live acceptance: isolated local gateway, existing authorized grant,
/// two short synthetic discovery turns. Never runs during the normal test suite.
#[tokio::test]
#[ignore = "uses an explicitly selected ChatGPT plan registration and plan usage"]
async fn chatgpt_plan_live_public_discovery_and_tool_result_roundtrip() {
    struct LiveHost {
        registration: String,
        model: String,
    }
    impl Host for LiveHost {
        fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
            Box::pin(async {
                Ok(ExecutionPolicy {
                    credential_retries: 1,
                    total_timeout: Duration::from_secs(100),
                    ..ExecutionPolicy::default()
                })
            })
        }
        fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
            Box::pin(async move {
                if bearer != "synthetic-local-acceptance" {
                    return Err(ServiceError::new(401, "invalid_api_key", "invalid"));
                }
                Ok(Caller {
                    id: "live-plan-acceptance".into(),
                    label: "synthetic".into(),
                })
            })
        }
        fn route<'a>(
            &'a self,
            _: &'a Caller,
            request: RouteRequest,
        ) -> BoxFuture<'a, Result<Route, ServiceError>> {
            Box::pin(async move {
                if request.model != self.model {
                    return Err(ServiceError::new(
                        400,
                        "invalid_model",
                        "acceptance model mismatch",
                    ));
                }
                let token = crate::modules::chatgpt_plan::inference_token(
                    &self.registration,
                    request.force_refresh,
                )
                .await
                .map_err(|code| ServiceError::new(401, "plan_authorization_failed", code))?;
                let mut headers = HeaderMap::new();
                headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
                Ok(Route {
                    account_id: format!("chatgpt_plan_{}", self.registration),
                    owner_id: self.registration.clone(),
                    continuity_account_id: None,
                    installation_profile: None,
                    official_authorization_revision: None,
                    official_registration_id: None,
                    official_fallback_registration_id: None,
                    official_fallback_authorization_revision: None,
                    authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                    transport_identity: None,
                    base_url: "https://api.openai.com/v1".into(),
                    model: self.model.clone(),
                    headers,
                    client: reqwest::Client::builder()
                        .timeout(Duration::from_secs(90))
                        .build()
                        .unwrap(),
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
            Box::pin(async { Ok(json!({"data":[]})) })
        }
        fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
    }
    let registration = std::env::var("COCKPIT_PLAN_LIVE_REGISTRATION")
        .expect("select an authorized registration explicitly");
    let model =
        std::env::var("COCKPIT_PLAN_LIVE_MODEL").expect("select an available model explicitly");
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(LiveHost {
        registration,
        model: model.clone(),
    });
    let mut handle = super::start(
        "127.0.0.1:0".parse().unwrap(),
        host,
        root.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let client = client();
    let url = format!("http://{}/v1/responses", handle.addr);
    let mut input = vec![message(
        "user",
        "Find the echo tool. Use query echo. This is a synthetic connectivity test.",
    )];
    let search = json!({"type":"tool_search","execution":"client","description":"Find a synthetic echo tool.","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}});
    let mut final_text = String::new();
    for turn in 0..2 {
        let choice = if turn == 0 {
            json!({"type":"tool_search"})
        } else {
            json!("none")
        };
        let response=client.post(&url).bearer_auth("synthetic-local-acceptance")
            .header("session-id","live-acceptance").header("thread-id","live-acceptance")
            .json(&json!({"model":model,"stream":false,"include":["reasoning.encrypted_content"],"reasoning":{"effort":"low"},"instructions":"Use the selected synthetic tool. After its result, reply exactly OK.","tools":[search.clone()],"tool_choice":choice,"input":input}))
            .send().await.unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert!(
            status.is_success(),
            "live plan turn {turn}: {status}: {}",
            body.get("error").unwrap_or(&Value::Null)
        );
        assert_eq!(body["status"], "completed");
        let output = body["output"].as_array().expect("response output");
        input.extend(output.iter().cloned());
        if turn == 0 {
            let call = output
                .iter()
                .find(|item| item["type"] == "tool_search_call")
                .expect("client discovery call");
            input.push(json!({"type":"tool_search_output","execution":"client","call_id":call["call_id"],"status":"completed","tools":[
                {"type":"function","name":"echo","description":"Return the supplied message verbatim.","parameters":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false}}
            ]}));
            input.push(message(
                "user",
                "The discovery result is received. Reply exactly OK.",
            ));
        } else {
            for item in output {
                if let Some(parts) = item.get("content").and_then(Value::as_array) {
                    for part in parts {
                        if part["type"] == "output_text" {
                            final_text.push_str(part["text"].as_str().unwrap_or_default());
                        }
                    }
                }
            }
        }
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert_eq!(final_text.trim(), "OK");
}
