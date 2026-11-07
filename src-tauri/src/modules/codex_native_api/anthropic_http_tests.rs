//! Real HTTP/SSE/WebSocket calls against a local Anthropic fixture.
use super::*;

#[path = "anthropic_code_mode_tests.rs"]
mod code_mode_tests;

fn sse(message: Value) -> Response {
    let mut frames = vec![
        json!({"type":"message_start","message":{"id":message["id"],"usage":message["usage"]}}),
    ];
    for (index, block) in message["content"].as_array().unwrap().iter().enumerate() {
        let mut start = block.clone();
        let delta = match block["type"].as_str().unwrap() {
            "text" => {
                start["text"] = json!("");
                json!({"type":"text_delta","text":block["text"]})
            }
            "thinking" => {
                start["thinking"] = json!("");
                start.as_object_mut().unwrap().remove("signature");
                json!({"type":"thinking_delta","thinking":block["thinking"]})
            }
            "tool_use" | "server_tool_use" => {
                start["input"] = json!({});
                json!({"type":"input_json_delta","partial_json":block["input"].to_string()})
            }
            "web_search_tool_result" => Value::Null,
            _ => unreachable!(),
        };
        frames.push(json!({"type":"content_block_start","index":index,"content_block":start}));
        if !delta.is_null() {
            frames.push(json!({"type":"content_block_delta","index":index,"delta":delta}));
        }
        if block["type"] == "thinking" {
            frames.push(json!({"type":"content_block_delta","index":index,"delta":{"type":"signature_delta","signature":block["signature"]}}));
        }
        frames.push(json!({"type":"content_block_stop","index":index}));
    }
    frames.push(json!({"type":"message_delta","delta":{"stop_reason":message["stop_reason"]},"usage":{"output_tokens":4}}));
    frames.push(json!({"type":"message_stop"}));
    let body: String = frames
        .into_iter()
        .map(|v| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap()))
        .collect();
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn anthropic_http_sse_and_websocket_preserve_custom_tools_and_followup_history() {
    for streaming in [false, true] {
        let captures = Arc::new(Mutex::new(Vec::<Value>::new()));
        let copy = captures.clone();
        let app=Router::new().route("/v1/messages",post(move |headers:HeaderMap,Json(body):Json<Value>| {
            let captures=copy.clone();async move {
                assert_eq!(headers["anthropic-version"],"2023-06-01");
                assert!(body.get("input").is_none());
                assert_eq!(body["model"],"claude-sonnet-4-6");
                let name=body["tools"][0]["name"].clone();
                captures.lock().unwrap().push(body.clone());
                let result=body["messages"].as_array().unwrap().iter().any(|m|m["content"].as_array().unwrap().iter().any(|b|b["type"]=="tool_result"));
                let message=json!({"id":if result {"msg_followup"}else{"msg_first"},"type":"message","role":"assistant","model":"claude-sonnet-4-6","stop_reason":if result {"end_turn"}else{"tool_use"},"content":if result {json!([{"type":"text","text":"done"}])} else {json!([{"type":"thinking","thinking":"plan","signature":"fixture-signature"},{"type":"tool_use","id":"toolu_1","name":name,"input":{"input":"patch"}}])},"usage":{"input_tokens":10,"output_tokens":4}});
                if body["stream"]==true {sse(message)}else{Json(message).into_response()}
            }
        }));
        let (mut handle, _host, task, _dir) =
            fixture(app, WireApi::Anthropic, false, native::Options::default()).await;
        let tools = json!([{"type":"namespace","name":"functions","tools":[{"type":"custom","name":"apply_patch"}]}]);
        let mut request = json!({"model":"claude-sonnet-4-6","input":[{"role":"user","content":"apply patch"}],"tools":tools,"stream":streaming});
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
        let text = response.text().await.unwrap();
        let first: Value = if streaming {
            text.lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|v| v["type"] == "response.completed")
                .unwrap_or_else(|| panic!("{text}"))["response"]
                .clone()
        } else {
            serde_json::from_str(&text).unwrap()
        };
        assert_eq!(first["output"][1]["type"], "custom_tool_call");
        assert_eq!(first["output"][1]["input"], "patch");
        let input = request["input"].as_array_mut().unwrap();
        input.extend(first["output"].as_array().unwrap().iter().cloned());
        input
            .push(json!({"type":"custom_tool_call_output","call_id":"toolu_1","output":"applied"}));
        let response = client
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .json(&request)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status, 200, "{text}");
        assert!(text.contains("done"), "{text}");
        let sent = captures.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|m| m["content"].as_array().unwrap())
            .any(|b| b["signature"] == "fixture-signature"));
        drop(sent);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn anthropic_websocket_uses_local_http_bridge() {
    let app=Router::new().route("/v1/messages",post(||async {sse(json!({"id":"msg_ws","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":2}}))}));
    let (mut handle, _host, task, _dir) =
        fixture(app, WireApi::Anthropic, false, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    send(&mut ws,json!({"type":"response.create","model":"claude-sonnet-4-6","input":[{"role":"user","content":"hello"}]})).await;
    let event = terminal(&mut ws).await;
    assert_eq!(event["type"], "response.completed", "{event}");
    assert_eq!(
        event["response"]["output"][0]["content"][0]["text"],
        "hello"
    );
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

use axum::response::IntoResponse;

#[tokio::test]
async fn anthropic_http_previous_response_id_uses_the_authorized_checkpoint() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let copy = count.clone();
    let app=Router::new().route("/v1/messages",post(move |Json(body):Json<Value>|{let copy=copy.clone();async move{
        let turn=copy.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
        if turn==1{assert!(body["messages"].to_string().contains("first answer"),"history lost: {body}");assert!(body["messages"].to_string().contains("original question"),"history lost: {body}");}
        Json(json!({"id":format!("msg_{turn}"),"type":"message","role":"assistant","model":"claude-sonnet-4-6","stop_reason":"end_turn","content":[{"type":"text","text":if turn==0{"first answer"}else{"continued"}}],"usage":{"input_tokens":4,"output_tokens":3}}))
    }}));
    let (mut handle, _host, task, _dir) =
        fixture(app, WireApi::Anthropic, false, native::Options::default()).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://{}/v1/responses", handle.addr);
    let first=client.post(&url).bearer_auth("key").header("session-id","http-claude-checkpoint").json(&json!({"model":"claude-sonnet-4-6","input":[{"role":"user","content":"original question"}]})).send().await.unwrap();
    let status = first.status();
    let first: Value = first.json().await.unwrap();
    assert_eq!(status, 200, "{first}");
    let second=client.post(&url).bearer_auth("key").header("session-id","http-claude-checkpoint").json(&json!({"model":"claude-sonnet-4-6","previous_response_id":first["id"],"input":[{"role":"user","content":"continue"}]})).send().await.unwrap();
    let status = second.status();
    let text = second.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    assert!(text.contains("continued"), "{text}");
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn anthropic_compaction_endpoint_and_inline_control_replay_the_summary() {
    for inline in [false, true] {
        let captures = Arc::new(Mutex::new(Vec::<Value>::new()));
        let copy = captures.clone();
        let app=Router::new().route("/v1/messages",post(move |Json(body):Json<Value>|{let copy=copy.clone();async move{
            assert_eq!(body["stream"],false);
            let summary=body["messages"].to_string().contains("concise and comprehensive summary");
            if !summary{assert!(body["system"].to_string().contains("preserve the plan"),"{body}");}
            copy.lock().unwrap().push(body);
            Json(json!({"id":format!("msg_{}",uuid::Uuid::now_v7()),"type":"message","role":"assistant","model":"claude-sonnet-4-6","stop_reason":"end_turn","content":[{"type":"text","text":if summary{"preserve the plan"}else{"continued"}}],"usage":{"input_tokens":10,"output_tokens":4}}))
        }}));
        let (mut handle, _host, task, _dir) =
            fixture(app, WireApi::Anthropic, false, native::Options::default()).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut request = json!({"model":"claude-sonnet-4-6","input":[{"role":"user","content":"remember the task"}]});
        let path = if inline {
            request["input"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":"compaction_trigger"}));
            request["stream"] = json!(true);
            "responses"
        } else {
            "responses/compact"
        };
        let response = client
            .post(format!("http://{}/v1/{path}", handle.addr))
            .bearer_auth("key")
            .header("session_id", "claude-compact-fixture")
            .json(&request)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status, 200, "{text}");
        let compact: Value = if inline {
            text.lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|event| event["type"] == "response.completed")
                .unwrap_or_else(|| panic!("{text}"))["response"]
                .clone()
        } else {
            serde_json::from_str(&text).unwrap()
        };
        assert_eq!(compact["output"][0]["type"], "compaction", "{compact}");
        assert!(compact["output"][0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with("cpa-ag-compact-v1:"));
        let mut history = compact["output"].as_array().unwrap().clone();
        history.push(json!({"role":"user","content":"continue"}));
        let response = client
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .header("session_id", "claude-compact-fixture")
            .json(&json!({"model":"claude-sonnet-4-6","input":history}))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status, 200, "{text}");
        assert!(text.contains("continued"), "{text}");
        assert_eq!(captures.lock().unwrap().len(), 2);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn anthropic_search_cache_restores_results_dropped_by_codex_history() {
    for streaming in [false, true] {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let copy = captured.clone();
        let app=Router::new().route("/v1/messages",post(move |Json(body):Json<Value>|{let copy=copy.clone();async move{
            let turn={let mut sent=copy.lock().unwrap();sent.push(body.clone());sent.len()};
            let content=if turn==1{json!([{"type":"server_tool_use","id":"srvtoolu_fixture","name":"web_search","input":{"query":"docs"}},{"type":"web_search_tool_result","tool_use_id":"srvtoolu_fixture","content":[{"type":"web_search_result","url":"https://example.test/docs","title":"Docs","encrypted_content":"opaque-search-result"}]},{"type":"text","text":"found docs"}])}else{
                assert!(body["messages"].as_array().unwrap().iter().flat_map(|message|message["content"].as_array().unwrap()).any(|block|block["type"]=="web_search_tool_result"&&block["content"][0]["encrypted_content"]=="opaque-search-result"),"search results lost: {body}");
                json!([{"type":"text","text":"continued with sources"}])
            };
            let message=json!({"id":format!("msg_{turn}"),"type":"message","model":"claude-sonnet-4-6","role":"assistant","content":content,"stop_reason":"end_turn","usage":{"input_tokens":5,"output_tokens":4}});
            if body["stream"]==true{sse(message)}else{Json(message).into_response()}
        }}));
        let (mut handle, _host, task, _dir) =
            fixture(app, WireApi::Anthropic, false, native::Options::default()).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut request = json!({"model":"claude-sonnet-4-6","stream":streaming,"input":[{"role":"user","content":"search docs"}],"tools":[{"type":"web_search"}]});
        for turn in 1..=2 {
            let response = client
                .post(format!("http://{}/v1/responses", handle.addr))
                .bearer_auth("key")
                .header("session_id", "claude-search-fixture")
                .json(&request)
                .send()
                .await
                .unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status, 200, "{text}");
            let response: Value = if streaming {
                text.lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .find(|event| event["type"] == "response.completed")
                    .unwrap_or_else(|| panic!("{text}"))["response"]
                    .clone()
            } else {
                serde_json::from_str(&text).unwrap()
            };
            if turn == 1 {
                assert_eq!(
                    response["output"][0]["results"][0]["encrypted_content"],
                    "opaque-search-result"
                );
                let history = request["input"].as_array_mut().unwrap();
                for item in response["output"].as_array().unwrap() {
                    let mut item = item.clone();
                    item.as_object_mut().unwrap().remove("results");
                    history.push(item);
                }
                history.push(json!({"role":"user","content":"continue from those sources"}));
            } else {
                assert!(text.contains("continued with sources"), "{text}");
            }
        }
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn anthropic_client_discovery_loads_mcp_tools_and_replays_results() {
    for streaming in [false, true] {
        let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = calls.clone();
        let app = Router::new().route("/v1/messages", post(move |Json(body): Json<Value>| {
            let captured = captured.clone();
            async move {
                let turn = {
                    let mut calls = captured.lock().unwrap();
                    calls.push(body.clone());
                    calls.len()
                };
                let content = if turn == 1 {
                    let name = body["tools"].as_array().unwrap().iter()
                        .find(|tool| tool["name"].as_str().unwrap().starts_with("cockpit_client_tool_search")).unwrap()["name"].clone();
                    json!([{"type":"tool_use","id":"search_1","name":name,"input":{"query":"read workspace"}}])
                } else if turn == 2 {
                    let name = body["tools"].as_array().unwrap().iter()
                        .find(|tool| tool["name"] == "workspace__read_file").expect("discovered tool must be callable")["name"].clone();
                    assert!(body["messages"].as_array().unwrap().iter().flat_map(|m| m["content"].as_array().unwrap()).any(|b| b["type"] == "tool_result" && b["tool_use_id"] == "search_1"));
                    json!([{"type":"tool_use","id":"read_1","name":name,"input":{"path":"README.md"}}])
                } else {
                    assert!(body["messages"].as_array().unwrap().iter().flat_map(|m| m["content"].as_array().unwrap()).any(|b| b["type"] == "tool_result" && b["tool_use_id"] == "read_1" && b["content"] == "file contents"));
                    json!([{"type":"text","text":"read complete"}])
                };
                let message = json!({"id":format!("msg_{turn}"),"type":"message","role":"assistant","content":content,"stop_reason":if turn<3 {"tool_use"} else {"end_turn"},"usage":{"input_tokens":3,"output_tokens":4}});
                if body["stream"] == true { sse(message) } else { Json(message).into_response() }
            }
        }));
        let (mut handle, _host, task, _dir) =
            fixture(app, WireApi::Anthropic, false, native::Options::default()).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut request = json!({"model":"claude-sonnet-4-6","stream":streaming,"input":[{"role":"user","content":"read file"}],"tools":[{"type":"tool_search","execution":"client","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}]});
        for turn in 1..=3 {
            let response = client
                .post(format!("http://{}/v1/responses", handle.addr))
                .bearer_auth("key")
                .json(&request)
                .send()
                .await
                .unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status, 200, "{text}");
            let response: Value = if streaming {
                assert!(
                    !text.contains("cockpit_client_tool_search"),
                    "private search identity leaked: {text}"
                );
                text.lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .find(|event| event["type"] == "response.completed")
                    .unwrap_or_else(|| panic!("{text}"))["response"]
                    .clone()
            } else {
                serde_json::from_str(&text).unwrap()
            };
            let output = &response["output"][0];
            let input = request["input"].as_array_mut().unwrap();
            input.extend(response["output"].as_array().unwrap().iter().cloned());
            if turn == 1 {
                assert_eq!(output["type"], "tool_search_call");
                assert_eq!(output["execution"], "client");
                assert_eq!(output["arguments"]["query"], "read workspace");
                input.push(json!({"type":"tool_search_output","call_id":"search_1","execution":"client","status":"completed","tools":[{"type":"function","namespace":"workspace","name":"read_file","defer_loading":true,"parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]}));
            } else if turn == 2 {
                assert_eq!(output["type"], "function_call");
                assert_eq!(output["namespace"], "workspace");
                assert_eq!(output["name"], "read_file");
                input.push(json!({"type":"function_call_output","call_id":"read_1","output":"file contents"}));
            } else {
                assert_eq!(output["content"][0]["text"], "read complete");
            }
        }
        assert_eq!(calls.lock().unwrap().len(), 3);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn anthropic_messages_clients_can_replay_service_issued_thinking_signatures() {
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let copy = captured.clone();
    let app=Router::new().route("/v1/messages",post(move |Json(body):Json<Value>| {
        let copy=copy.clone();async move {
            copy.lock().unwrap().push(body);
            Json(json!({"id":format!("msg_{}",uuid::Uuid::now_v7()),"type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"thinking","thinking":"plan","signature":"fixture-signature"},{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}))
        }
    }));
    let (mut handle, _host, task, _dir) =
        fixture(app, WireApi::Anthropic, false, native::Options::default()).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://{}/v1/messages", handle.addr);
    let first=client.post(&url).bearer_auth("key").json(&json!({"model":"claude-sonnet-4-6","max_tokens":256,"messages":[{"role":"user","content":"hello"}]})).send().await.unwrap();
    let status = first.status();
    let first: Value = first.json().await.unwrap();
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["content"][0]["signature"], "claude#fixture-signature");
    let second=client.post(&url).bearer_auth("key").json(&json!({"model":"claude-sonnet-4-6","max_tokens":256,"messages":[{"role":"user","content":"hello"},{"role":"assistant","content":first["content"]},{"role":"user","content":"continue"}]})).send().await.unwrap();
    let status = second.status();
    let second: Value = second.json().await.unwrap();
    assert_eq!(status, 200, "{second}");
    assert_eq!(
        captured.lock().unwrap()[1]["messages"][1]["content"][0]["signature"],
        "fixture-signature"
    );
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
