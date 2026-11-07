use super::*;
use crate::modules::codex_native_api::anthropic::{messages_to_response, responses_to_messages};

fn code_mode_request() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/anthropic-code-mode.json"
    )))
    .unwrap()
}

// Opt-in live probe: credentials stay in memory and generated tools are only
// inspected, never executed. Normal regression runs remain fully offline.
#[tokio::test]
#[ignore = "Requires COCKPIT_CLAUDE_PROBE_ACCOUNT and an explicitly selected gateway"]
async fn anthropic_code_mode_live_gateway_probe() {
    let account_id = std::env::var("COCKPIT_CLAUDE_PROBE_ACCOUNT").unwrap();
    let root = std::path::PathBuf::from(std::env::var("COCKPIT_CLAUDE_PROBE_DATA_DIR").unwrap());
    let path = root
        .join("codex_accounts")
        .join(format!("{account_id}.json"));
    let account = crate::modules::secure_account_storage::read_account_file_readonly::<
        crate::models::codex::CodexAccount,
    >(&path, &root.join("secure-account-storage.key"))
    .unwrap();
    let request = if let Ok(path) = std::env::var("COCKPIT_CLAUDE_PROBE_REQUEST") {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    } else {
        code_mode_request()
    };
    let wire = responses_to_messages(&request).unwrap();
    let url = crate::modules::codex_native_api::anthropic::endpoint(
        account.api_base_url.as_deref().unwrap(),
        "messages",
    )
    .unwrap();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap();
    let mut call = client.post(url).header("anthropic-version", "2023-06-01");
    let key = account.openai_api_key.as_deref().unwrap();
    call = if account.api_auth_scheme.as_deref() == Some("bearer") {
        call.bearer_auth(key)
    } else {
        call.header("x-api-key", key)
    };
    let reply = call.json(&wire).send().await.unwrap();
    assert!(
        reply.status().is_success(),
        "gateway HTTP status: {}",
        reply.status()
    );
    let reply: Value = reply.json().await.unwrap();
    let converted =
        messages_to_response(&reply, request["model"].as_str().unwrap(), &request).unwrap();
    let calls: Vec<_> = converted["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| {
            matches!(
                item["type"].as_str(),
                Some("function_call" | "custom_tool_call")
            )
        })
        .collect();
    assert!(!calls.is_empty(), "gateway returned no tool call");
    for call in calls {
        assert_eq!(call["type"], "custom_tool_call");
        assert_eq!(call["namespace"], "functions");
        assert_eq!(call["name"], "exec");
    }
    if let Ok(path) = std::env::var("COCKPIT_CLAUDE_PROBE_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&converted).unwrap()).unwrap();
    }
    println!(
        "Live gateway returned a declared code-mode tool; usage={}",
        converted["usage"]
    );
}

#[test]
fn anthropic_code_mode_keeps_nested_tools_inside_exec_for_both_declaration_shapes() {
    for additional in [false, true] {
        let mut request = code_mode_request();
        if !additional {
            request["tools"] = request["input"][0]["tools"].take();
            request["input"].as_array_mut().unwrap().remove(0);
        }
        let wire = responses_to_messages(&request).unwrap();
        assert_eq!(wire["tools"].as_array().unwrap().len(), 2);
        assert_eq!(wire["tools"][0]["name"], "functions__exec");
        assert_eq!(wire["tools"][1]["name"], "functions__wait");
        let guide = wire["system"].as_array().unwrap().last().unwrap()["text"]
            .as_str()
            .unwrap();
        assert!(guide.contains("functions__exec is the JavaScript dispatcher"));
        assert!(guide.contains("not additional stand-alone Claude tools"));
        assert!(guide.contains("Do not call tools.apply_patch({input:"));
        assert!(guide.contains("an Add File block has no +++ file header"));
        let script = "text(await tools.apply_patch(\"*** Begin Patch\\n*** Add File: outputs/probe.txt\\n+hello\\n*** End Patch\"));";
        let response = json!({"id":"msg_exec","content":[{"type":"tool_use","id":"toolu_exec","name":"functions__exec","input":{"input":script}}],"stop_reason":"tool_use","usage":{}});
        let converted = messages_to_response(&response, "claude-opus-5-5", &request).unwrap();
        let call = &converted["output"][0];
        assert_eq!(call["type"], "custom_tool_call");
        assert_eq!(call["name"], "exec");
        assert_eq!(call["namespace"], "functions");
        assert_eq!(call["input"], script);
        request["input"].as_array_mut().unwrap().extend([call.clone(),json!({"type":"custom_tool_call_output","call_id":"toolu_exec","output":"Success. Updated outputs/probe.txt"})]);
        let replay = responses_to_messages(&request).unwrap();
        assert!(replay["messages"]
            .to_string()
            .contains("Success. Updated outputs/probe.txt"));
    }
    let direct = responses_to_messages(&json!({"model":"claude-opus-5-5","input":"edit","tools":[{"type":"custom","name":"apply_patch"}]})).unwrap();
    assert!(direct.get("system").is_none());
}

#[tokio::test]
async fn anthropic_http_code_mode_stream_preserves_javascript_and_tool_results() {
    let app = axum::Router::new().route("/v1/messages", axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
        assert!(body["system"].to_string().contains("Codex code-mode tool contract"));
        let followup = body["messages"].to_string().contains("probe-created");
        let content = if followup {
            json!([{"type":"text","text":"done"}])
        } else {
            json!([{"type":"tool_use","id":"toolu_code","name":"functions__exec","input":{"input":"text(await tools.apply_patch(\"*** Begin Patch\\n*** Add File: outputs/probe.txt\\n+hello\\n*** End Patch\"));"}}])
        };
        sse(json!({"id":if followup {"msg_done"} else {"msg_code"},"content":content,"stop_reason":if followup {"end_turn"} else {"tool_use"},"usage":{}}))
    }));
    let (mut handle, _host, task, _dir) =
        fixture(app, WireApi::Anthropic, false, native::Options::default()).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut request = code_mode_request();
    request["stream"] = json!(true);
    for followup in [false, true] {
        let reply = client
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), 200);
        let text = reply.text().await.unwrap();
        let events: Vec<Value> = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let response = &events
            .iter()
            .find(|event| event["type"] == "response.completed")
            .unwrap()["response"];
        if followup {
            assert!(response.to_string().contains("done"));
            break;
        }
        let call = &response["output"][0];
        assert_eq!(call["name"], "exec");
        assert_eq!(call["namespace"], "functions");
        let streamed: String = events
            .iter()
            .filter(|event| event["type"] == "response.custom_tool_call_input.delta")
            .filter_map(|event| event["delta"].as_str())
            .collect();
        assert_eq!(streamed, call["input"].as_str().unwrap());
        request["input"].as_array_mut().unwrap().extend([call.clone(),json!({"type":"custom_tool_call_output","call_id":"toolu_code","output":"probe-created"})]);
    }
    handle
        .shutdown(std::time::Duration::from_secs(2))
        .await
        .unwrap();
    task.abort();
}
