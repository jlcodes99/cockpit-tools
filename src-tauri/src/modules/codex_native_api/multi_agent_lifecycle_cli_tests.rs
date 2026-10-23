//! Real official CLI coordination against the real gateway and a scripted
//! loopback provider. This tests client interoperability, not OpenAI inference.
use super::*;
use std::os::unix::process::CommandExt;

const ROOT: &str = "ROOT_LIFECYCLE_FIXTURE";
const FIRST: &str = "FIRST_SUBTASK";
const SECOND: &str = "SECOND_MESSAGE";
const FOLLOWUP: &str = "FOLLOWUP_SUBTASK";
const FIRST_RESULT: &str = "CHILD_FIRST_RESULT";
const FINAL_RESULT: &str = "CHILD_FINAL_RESULT";

fn scripted_response(body: &Value, sequence: usize) -> Value {
    let raw = body.to_string();
    let parent = raw.contains(ROOT);
    let input = body["input"].as_array().unwrap();
    let has_result = |id: &str| {
        input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["call_id"] == id)
    };
    let next = if !parent {
        None
    } else if !has_result("fixture-spawn") {
        Some((
            "spawn_agent",
            "fixture-spawn".to_string(),
            json!({"task_name":"reviewer","message":FIRST,"fork_turns":"none"}),
        ))
    } else if !has_result("fixture-send") {
        Some((
            "send_message",
            "fixture-send".to_string(),
            json!({"target":"reviewer","message":SECOND}),
        ))
    } else if !raw.contains(FIRST_RESULT) {
        Some((
            "wait_agent",
            format!("fixture-wait-first-{sequence}"),
            json!({"timeout_ms":1000}),
        ))
    } else if !has_result("fixture-followup") {
        Some((
            "followup_task",
            "fixture-followup".to_string(),
            json!({"target":"reviewer","message":FOLLOWUP}),
        ))
    } else if !raw.contains(FINAL_RESULT) {
        Some((
            "wait_agent",
            format!("fixture-wait-final-{sequence}"),
            json!({"timeout_ms":1000}),
        ))
    } else {
        None
    };
    let output = if let Some((name, call_id, args)) = next {
        json!({"type":"function_call","id":format!("item-{call_id}"),"call_id":call_id,
            "namespace":declaration_namespace(body),"name":name,"arguments":args.to_string(),"status":"completed"})
    } else {
        let result = if parent {
            "PARENT_DONE"
        } else if raw.contains(FOLLOWUP) {
            FINAL_RESULT
        } else {
            FIRST_RESULT
        };
        json!({"type":"message","id":format!("msg-cli-{sequence}"),"role":"assistant","status":"completed",
            "content":[{"type":"output_text","text":result,"annotations":[]}]})
    };
    json!({"id":format!("cli-lifecycle-{sequence}"),"object":"response","model":"gpt-5.6-sol",
        "status":"completed","output":[output],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}})
}

#[tokio::test]
#[ignore = "requires COCKPIT_PLAN_CODEX_BIN; runs official CLI only against isolated loopback"]
async fn multi_agent_lifecycle_real_codex_cli_isolated_plan_gateway() {
    let binary = std::env::var("COCKPIT_PLAN_CODEX_BIN")
        .expect("select an official local Codex CLI explicitly");
    let workspace = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let codex_home = tempfile::tempdir().unwrap();
    let captures: Arc<Mutex<Vec<Value>>> = Default::default();
    let captured = captures.clone();
    let app = Router::new().route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let captured = captured.clone();
        async move {
            assert!(!headers.contains_key("authorization"), "local fixture key must not reach upstream");
            let sequence = { let mut captures = captured.lock().unwrap(); captures.push(body.clone()); captures.len() };
            let invalid = sequence > 24 || body["input"].as_array().unwrap().iter().any(|item| {
                item["type"] == "agent_message" || item.get("author").is_some() || item.get("recipient").is_some()
                    || item.get("internal_chat_message_metadata_passthrough").is_some()
            });
            if invalid {
                return (StatusCode::BAD_REQUEST, Json(json!({"error":{"code":"unsupported_value","param":"input","message":"Unsupported private agent input or excessive loopback turns"}}))).into_response();
            }
            let response = scripted_response(&body, sequence);
            let item = &response["output"][0];
            let events = [
                json!({"type":"response.created","response":{"id":response["id"],"status":"in_progress","output":[]}}),
                json!({"type":"response.output_item.added","output_index":0,"item":item}),
                json!({"type":"response.output_item.done","output_index":0,"item":item}),
                json!({"type":"response.completed","response":response}),
            ];
            Response::builder().header("content-type","text/event-stream")
                .body(Body::from(events.iter().map(|event|format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap())).collect::<String>())).unwrap()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(LifecycleHost {
        base: format!("http://{}/v1", listener.local_addr().unwrap()),
        plan: true,
        optimize: true,
    });
    let upstream = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut gateway = start(
        "127.0.0.1:0".parse().unwrap(),
        host,
        state.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    // The enclosing macOS sandbox denies every external network connection,
    // including incidental model discovery or telemetry outside the fixture.
    let mut command = tokio::process::Command::new("/usr/bin/sandbox-exec");
    command.args(["-p", "(version 1)(allow default)(deny network*)(allow network-outbound (remote ip \"localhost:*\"))(allow network-inbound (local ip \"localhost:*\"))"])
        .arg(binary).args(["exec","--ignore-user-config","--ignore-rules","--ephemeral","--skip-git-repo-check","--sandbox","read-only","--json","--model","gpt-5.6-sol","--cd"])
        .arg(workspace.path()).env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("CODEX_HOME", codex_home.path()).env("COCKPIT_PLAN_CONTRACT_TOKEN","synthetic-loopback-only")
        .env("HOME", codex_home.path())
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    command.as_std_mut().process_group(0);
    for setting in [
        "model_provider=\"plan_contract\"".to_string(),
        "model_providers.plan_contract.name=\"Loopback lifecycle\"".into(),
        format!(
            "model_providers.plan_contract.base_url=\"http://{}/v1\"",
            gateway.addr
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
        "features.multi_agent_v2.enabled=true".into(),
        "features.multi_agent_v2.tool_namespace=\"collaboration\"".into(),
        "features.multi_agent_v2.wait_agent_enabled=true".into(),
        "features.multi_agent_v2.min_wait_timeout_ms=1".into(),
        "features.multi_agent_v2.default_wait_timeout_ms=1000".into(),
        "features.multi_agent_v2.max_wait_timeout_ms=3000".into(),
        "web_search=\"disabled\"".into(),
        "history.persistence=\"none\"".into(),
        "approval_policy=\"never\"".into(),
    ] {
        command.args(["-c", &setting]);
    }
    command.arg(format!("{ROOT}: Synthetic integration fixture. Create a child, message it, wait, follow up, wait, then return its findings."));
    let child = command.spawn().unwrap();
    let pid = child.id().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(45), child.wait_with_output()).await;
    if result.is_err() {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    gateway.shutdown(Duration::from_secs(3)).await.unwrap();
    upstream.abort();
    let result = result
        .expect("isolated official CLI lifecycle timed out")
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        result.status.success(),
        "official CLI failed: {}\n{stdout}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        stdout.contains("PARENT_DONE"),
        "parent did not receive both actual child results: {stdout}"
    );
    let captures = captures.lock().unwrap();
    let root_requests: Vec<_> = captures
        .iter()
        .filter(|body| body.to_string().contains(ROOT))
        .collect();
    let child_requests: Vec<_> = captures
        .iter()
        .filter(|body| !body.to_string().contains(ROOT))
        .collect();
    assert!(
        root_requests.len() >= 4 && child_requests.len() >= 2,
        "CLI must create and resume a real child runtime"
    );
    let resumed_child = child_requests
        .iter()
        .find(|body| body.to_string().contains(FOLLOWUP))
        .expect("followup invoked the child again");
    for token in [FIRST, SECOND, FOLLOWUP, FIRST_RESULT] {
        assert!(
            resumed_child.to_string().contains(token),
            "child continuation lost {token}"
        );
    }
    let last_parent = root_requests.last().unwrap().to_string();
    for token in [
        FIRST_RESULT,
        FINAL_RESULT,
        "fixture-spawn",
        "fixture-send",
        "fixture-followup",
    ] {
        assert!(last_parent.contains(token), "parent history lost {token}");
    }
    for body in captures.iter() {
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert!(body.get("previous_response_id").is_none());
    }
}
