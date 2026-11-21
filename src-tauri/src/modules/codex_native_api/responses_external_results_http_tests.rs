//! Strict synthetic upstreams exercise receipts through the actual HTTP send path.
use super::*;
use axum::response::IntoResponse;

fn receipt(output: Value) -> Value {
    json!({"type":"function_call_output","namespace":"agent",
        "name":"codex_apps_open_page","output":output})
}

fn request() -> Value {
    json!({"model":"m","input":[
        {"type":"message","role":"user","content":"Continue this existing conversation"},
        {"type":"function_call","name":"read","call_id":"first","arguments":"{}"},
        {"type":"function_call","name":"read","call_id":"second","arguments":"{}"},
        receipt(json!("old page context\n  preserved  ")),
        {"type":"function_call_output","call_id":"first","output":"first result"},
        receipt(json!([
            {"type":"input_text","text":"selected page"},
            {"type":"input_image","image_url":"data:image/png;base64,AAAA","detail":"low"}
        ])),
        {"type":"function_call_output","call_id":"second","output":"second result"}
    ],"tools":[{"type":"function","name":"read","parameters":{"type":"object","properties":{}}}]})
}

fn strict_error(wire: WireApi, body: &Value) -> Option<&'static str> {
    let nonempty = |value: &Value| value.as_str().is_some_and(|id| !id.trim().is_empty());
    match wire {
        WireApi::Responses => {
            for item in body["input"].as_array().unwrap() {
                if item["type"] == "function_call_output" && !nonempty(&item["call_id"]) {
                    return Some("function_call_output requires call_id on HTTP requests; continuation via previous_response_id is only supported on Responses WebSocket v2");
                }
            }
        }
        WireApi::Chat => {
            let mut pending = std::collections::HashSet::new();
            for message in body["messages"].as_array().unwrap() {
                if message["role"] == "tool" {
                    if !nonempty(&message["tool_call_id"])
                        || !pending.remove(message["tool_call_id"].as_str().unwrap())
                    {
                        return Some("tool result does not match a pending call");
                    }
                } else {
                    if !pending.is_empty() {
                        return Some("message interrupts parallel tool results");
                    }
                    for call in message["tool_calls"].as_array().into_iter().flatten() {
                        pending.insert(call["id"].as_str().unwrap().to_owned());
                    }
                }
            }
        }
        WireApi::Anthropic => {
            for part in body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|message| message["content"].as_array().unwrap())
            {
                if part["type"] == "tool_result" && !nonempty(&part["tool_use_id"]) {
                    return Some("tool_result requires tool_use_id");
                }
            }
        }
    }
    None
}

fn success(wire: WireApi, body: &Value, id: &str) -> Response {
    let response = match wire {
        WireApi::Responses => json!({"id":id,"object":"response","status":"completed","output":[]}),
        WireApi::Chat => json!({"id":id,"object":"chat.completion","model":"m",
            "choices":[{"index":0,"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}),
        WireApi::Anthropic => json!({"id":id,"type":"message","role":"assistant","model":"m",
            "content":[{"type":"text","text":"done"}],"stop_reason":"end_turn",
            "usage":{"input_tokens":1,"output_tokens":1}}),
    };
    if body["stream"] == true && wire == WireApi::Responses {
        return (
            [("content-type", "text/event-stream")],
            format!(
                "data: {}\n\n",
                json!({"type":"response.completed","response":response})
            ),
        )
            .into_response();
    }
    if body["stream"] == true && wire == WireApi::Anthropic {
        return super::anthropic_tests::sse(response);
    }
    if body["stream"] == true && wire == WireApi::Chat {
        let chunk = json!({"id":id,"object":"chat.completion.chunk","model":"m",
            "choices":[{"index":0,"delta":{"role":"assistant","content":"done"},"finish_reason":"stop"}]});
        return (
            [("content-type", "text/event-stream")],
            format!("data: {chunk}\n\ndata: [DONE]\n\n"),
        )
            .into_response();
    }
    Json(response).into_response()
}

#[tokio::test]
async fn deepseek_responses_can_continue_after_an_incomplete_tool_batch() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let observed = captured.clone();
    let strict = Router::new().route("/v1/responses", post(move |Json(body): Json<Value>| {
        let observed = observed.clone();
        async move {
            let input = body["input"].as_array().unwrap();
            let calls = input.iter().filter(|item| item["type"] == "function_call")
                .filter_map(|item| item["call_id"].as_str())
                .collect::<std::collections::HashSet<_>>();
            let outputs = input.iter().filter(|item| item["type"] == "function_call_output")
                .filter_map(|item| item["call_id"].as_str())
                .collect::<std::collections::HashSet<_>>();
            let failure = strict_error(WireApi::Responses, &body)
                .or_else(|| (!calls.is_subset(&outputs)).then_some("No tool output found"))
                .or_else(|| (!outputs.is_subset(&calls)).then_some("No tool call found"))
                .or_else(|| {
                    let mut pending = std::collections::HashSet::new();
                    let mut receiving_outputs = false;
                    let thinking = input.iter().any(|item| item["type"] == "reasoning");
                    let mut has_reasoning = false;
                    for item in input {
                        match item["type"].as_str() {
                            Some("reasoning") if pending.is_empty() => {
                                has_reasoning = item["content"].as_array().is_some_and(|parts| {
                                    parts.iter().any(|part| part["type"] == "reasoning_text")
                                });
                            }
                            Some("function_call") if !receiving_outputs => {
                                if pending.is_empty() && thinking && !std::mem::take(&mut has_reasoning) {
                                    return Some("The reasoning_text in the thinking mode must be passed back to the API");
                                }
                                pending.insert(item["call_id"].as_str().unwrap());
                            }
                            Some("function_call_output") => {
                                if !pending.remove(item["call_id"].as_str().unwrap()) {
                                    return Some("Tool results must immediately follow their batch");
                                }
                                receiving_outputs = !pending.is_empty();
                            }
                            _ if !pending.is_empty() => {
                                return Some("Tool results must immediately follow their batch");
                            }
                            _ => {}
                        }
                    }
                    None
                });
            if let Some(message) = failure {
                return (StatusCode::BAD_REQUEST, Json(json!({"error":{
                    "type":"invalid_request_error","message":message
                }}))).into_response();
            }
            let mut observed = observed.lock().unwrap();
            let response = success(WireApi::Responses, &body, &format!("response-{}", observed.len()));
            observed.push(body);
            response
        }
    }));
    let (mut gateway, host, upstream, _dir) = fixture_with_upstream_host(
        strict.clone(), WireApi::Responses, false, native::Options::default(), Some("api.deepseek.com")
    ).await;
    let upstream_url = format!("{}/responses", host.base);
    let gateway_url = format!("http://{}/v1/responses", gateway.addr);
    let local = reqwest::Client::builder().no_proxy().build().unwrap();
    let original = json!({"model":"deepseek-v4-flash","parallel_tool_calls":false,"input":[
        {"type":"function_call","name":"read","call_id":"done","arguments":"{}"},
        {"type":"function_call","name":"read","call_id":"missing","arguments":"{}"},
        {"type":"function_call_output","call_id":"done","output":"original output"},
        {"type":"message","role":"user","content":"continue"}
    ]});
    let standalone = json!({"type":"function_call_output","namespace":"codex_app",
        "name":"send_message_to_thread","output":"cross-thread context"});
    for (body, error) in [
        (original.clone(), "No tool output found"),
        (json!({"model":"deepseek-v4-flash","input":[standalone.clone()]}), "requires call_id"),
    ] {
        let response = host.client.post(&upstream_url).json(&body).send().await.unwrap();
        assert_eq!(response.status(), 400);
        assert!(response.text().await.unwrap().contains(error));
    }
    let mut combined = original.clone();
    combined["input"].as_array_mut().unwrap().insert(3, standalone);
    for streaming in [false, true] {
        combined["stream"] = json!(streaming);
        let response = local.post(&gateway_url).bearer_auth("key")
            .json(&combined).send().await.unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    }
    let mut complete = original.clone();
    let input = complete["input"].as_array_mut().unwrap();
    input.insert(3, json!({
        "type":"function_call_output","call_id":"missing","output":"real result committed"
    }));
    input.insert(0, json!({"type":"reasoning","summary":[],
        "content":[{"type":"reasoning_text","text":"Read both files before answering."}]}));
    let response = local.post(&gateway_url).bearer_auth("key")
        .json(&complete).send().await.unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    let mut interleaved = complete.clone();
    interleaved["reasoning"] = json!({"effort":"ultra"});
    let hook = json!({"type":"message","role":"developer","content":"PostToolUse hook context"});
    interleaved["input"].as_array_mut().unwrap().insert(4, hook.clone());
    let response = host.client.post(&upstream_url).json(&interleaved).send().await.unwrap();
    assert_eq!(response.status(), 400);
    assert!(response.text().await.unwrap().contains("must immediately follow"));
    let response = local.post(&gateway_url).bearer_auth("key")
        .json(&interleaved).send().await.unwrap();
    assert_eq!(response.status(), 200);

    let mut summary_replay = complete.clone();
    let stub = "00000000-0000-4000-8000-000000000001-0";
    let summary = json!({
        "type": "reasoning",
        "summary": [{"type": "summary_text", "text": "Read both files before answering."}],
        "encrypted_content": stub,
    });
    summary_replay["input"][0] = summary.clone();
    for streaming in [false, true] {
        summary_replay["stream"] = json!(streaming);
        let response = local
            .post(&gateway_url)
            .header("thread-id", "fixture-child-thread")
            .bearer_auth("key")
            .json(&summary_replay)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    }
    let mut opaque_replay = summary_replay.clone();
    opaque_replay["stream"] = json!(false);
    opaque_replay["input"][0]["summary"] = json!([]);
    let response = local.post(&gateway_url).bearer_auth("key")
        .json(&opaque_replay).send().await.unwrap();
    assert_eq!(response.status(), 400);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "deepseek_reasoning_replay_required");

    {
        let sent = captured.lock().unwrap();
        assert_eq!(sent.len(), 6);
        for wire in &sent[..2] {
            assert_eq!(wire["parallel_tool_calls"], false);
            assert_eq!(wire["input"][3]["call_id"], "missing");
            assert!(wire["input"][3]["output"].as_str().unwrap().starts_with("Tool result unavailable"));
            assert_eq!(wire["input"][1], original["input"][1]);
            assert_eq!(wire["input"][2], original["input"][2]);
            assert_eq!(wire["input"][4]["type"], "message");
            assert!(wire["input"][4]["content"][0]["text"].as_str().unwrap().contains("cross-thread context"));
        }
        assert_eq!(sent[2], complete);
        let mut repaired = complete;
        repaired["reasoning"] = json!({"effort":"max"});
        repaired["input"].as_array_mut().unwrap().insert(5, hook);
        assert_eq!(sent[3], repaired);
        let mut restored = summary;
        restored["content"] = json!([{
            "type": "reasoning_text",
            "text": "Read both files before answering."
        }]);
        for wire in &sent[4..] {
            assert_eq!(wire["input"][0], restored);
            assert_eq!(wire["input"][1], original["input"][0]);
        }
    }
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();

    let (mut unrelated, _, upstream, _dir) =
        fixture(strict, WireApi::Responses, false, native::Options::default()).await;
    let response = local.post(format!("http://{}/v1/responses", unrelated.addr))
        .bearer_auth("key").json(&original).send().await.unwrap();
    assert_eq!(response.status(), 400, "other Responses upstreams retain their wire input");
    assert!(response.text().await.unwrap().contains("No tool output found"));
    let response = local.post(format!("http://{}/v1/responses", unrelated.addr))
        .bearer_auth("key").json(&summary_replay).send().await.unwrap();
    assert_eq!(response.status(), 400);
    assert!(response.text().await.unwrap().contains("must be passed back"));
    unrelated.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[tokio::test]
async fn deepseek_child_history_replays_plaintext_without_forwarding_the_parent_reasoning_stub() {
    let stub = "0c2ef6b0-b5f1-44ed-9b6f-8a9f2f96ed85-0";
    let reasoning = json!({"type":"reasoning", "id":"provider-item",
        "summary":[], "encrypted_content":stub,
        "content":[{"type":"reasoning_text","text":"Synthetic parent reasoning."}]});
    let parent_reasoning = reasoning.clone();
    let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
    let upstream_bodies = observed.clone();
    let upstream_app = Router::new().route("/v1/responses", post(move |Json(body): Json<Value>| {
        let parent_reasoning = parent_reasoning.clone();
        let upstream_bodies = upstream_bodies.clone();
        async move {
            let previous = body["input"].as_array().unwrap().iter().find(|item| item["type"] == "reasoning");
            upstream_bodies.lock().unwrap().push(body.clone());
            let Some(previous) = previous else {
                return Json(json!({"id":"parent-response","object":"response","status":"completed",
                    "output":[parent_reasoning]})).into_response();
            };
            if !previous["encrypted_content"].is_null() || previous["content"][0]["type"] != "reasoning_text" {
                return (StatusCode::BAD_REQUEST, Json(json!({"error":{
                    "type":"invalid_request_error",
                    "message":"The reasoning_text in the thinking mode must be passed back to the API."
                }}))).into_response();
            }
            success(WireApi::Responses, &body, "child-response")
        }
    }));
    let (mut gateway, _, upstream, _dir) = fixture_with_upstream_host(
        upstream_app.clone(), WireApi::Responses, false, native::Options::default(), Some("api.deepseek.com"),
    ).await;
    let url = format!("http://{}/v1/responses", gateway.addr);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let user = json!({"type":"message","role":"user","content":"fixture parent request"});
    let parent = client.post(&url).bearer_auth("key")
        .header("x-codex-turn-metadata", json!({"session_id":"parent","thread_id":"parent"}).to_string())
        .json(&json!({"model":"deepseek-v4-flash","input":[user.clone()]})).send().await.unwrap();
    assert_eq!(parent.status(), 200);
    assert_eq!(parent.json::<Value>().await.unwrap()["output"][0], reasoning);
    let mut child = json!({"model":"deepseek-v4-flash","input":[user, reasoning.clone(),
        {"type":"function_call","name":"lookup","call_id":"call-1","arguments":"{}"},
        {"type":"function_call_output","call_id":"call-1","output":"fixture tool result"},
        {"type":"message","role":"user","content":"continue child"}
    ]});
    let metadata = json!({"session_id":"child","thread_id":"child","parent_thread_id":"parent",
        "parent_response_id":"parent-response"}).to_string();
    let response = client.post(&url).bearer_auth("key").header("x-codex-turn-metadata", &metadata)
        .json(&child).send().await.unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    {
        let sent = observed.lock().unwrap();
        assert_eq!(sent[1]["input"][1]["content"], reasoning["content"]);
        assert!(sent[1]["input"][1].get("encrypted_content").is_none());
        assert_eq!(sent[1]["input"][2], child["input"][2]);
        assert_eq!(sent[1]["input"][3], child["input"][3]);
    }
    child["stream"] = json!(true);
    let response = client.post(&url).bearer_auth("key").header("x-codex-turn-metadata", &metadata)
        .json(&child).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("child-response"));
    assert!(observed.lock().unwrap()[2]["input"][1].get("encrypted_content").is_none());

    child["stream"] = json!(false);
    child["input"][1].as_object_mut().unwrap().remove("content");
    let response = client.post(&url).bearer_auth("key").header("x-codex-turn-metadata", &metadata)
        .json(&child).send().await.unwrap();
    assert_eq!(response.status(), 400);
    let error = response.json::<Value>().await.unwrap();
    assert_eq!(error["error"]["code"], "deepseek_reasoning_history_missing");
    assert_eq!(observed.lock().unwrap().len(), 4);
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();

    let (mut unrelated, _, upstream, _dir) =
        fixture(upstream_app, WireApi::Responses, false, native::Options::default()).await;
    child["input"][1] = reasoning;
    let response = client.post(format!("http://{}/v1/responses", unrelated.addr)).bearer_auth("key")
        .json(&child).send().await.unwrap();
    assert_eq!(response.status(), 400);
    assert!(response.text().await.unwrap().contains("thinking mode must be passed back"));
    assert_eq!(observed.lock().unwrap().last().unwrap()["input"][1]["encrypted_content"], stub);
    unrelated.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[tokio::test]
async fn deepseek_expands_previous_response_before_sending_the_tool_result() {
    let reasoning = json!({"type":"reasoning","id":"rs_parent",
        "summary":[],"encrypted_content":"0c2ef6b0-b5f1-44ed-9b6f-8a9f2f96ed85-0",
        "content":[{"type":"reasoning_text","text":"Synthetic tool reasoning."}]});
    let call = json!({"type":"function_call","id":"fc_parent","call_id":"call_parent",
        "name":"lookup","arguments":"{}","status":"completed"});
    let root_output = json!([reasoning, call]);
    let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = observed.clone();
    let upstream_app = Router::new().route("/v1/responses", post(move |Json(body): Json<Value>| {
        let root_output = root_output.clone();
        let captured = captured.clone();
        async move {
            captured.lock().unwrap().push(body.clone());
            let input = body["input"].as_array().unwrap();
            if input.len() == 1 && input[0]["role"] == "user" {
                return Json(json!({"id":"resp_parent","object":"response","status":"completed",
                    "output":root_output})).into_response();
            }
            // The stateless upstream ignores a response ID and validates its inline history.
            if input.len() != 4 || input[1]["content"][0]["type"] != "reasoning_text"
                || !input[1]["encrypted_content"].is_null() || input[2]["call_id"] != input[3]["call_id"]
            {
                return (StatusCode::BAD_REQUEST, Json(json!({"error":{
                    "type":"invalid_request_error",
                    "message":"The `reasoning_text` in the thinking mode must be passed back to the API."
                }}))).into_response();
            }
            success(WireApi::Responses, &body, "resp_next")
        }
    }));
    let (mut gateway, _, upstream, _dir) = fixture_with_upstream_host(
        upstream_app, WireApi::Responses, false, native::Options::default(), Some("api.deepseek.com"),
    ).await;
    let url = format!("http://{}/v1/responses", gateway.addr);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let metadata = json!({"session_id":"session","thread_id":"thread"}).to_string();
    let user = json!({"type":"message","role":"user","content":"Read a fixture"});
    let parent = client.post(&url).bearer_auth("key").header("x-codex-turn-metadata", &metadata)
        .json(&json!({"model":"deepseek-flash","input":[user],"tools":[
            {"type":"function","name":"lookup","parameters":{"type":"object","properties":{}}}
        ]})).send().await.unwrap();
    assert_eq!(parent.status(), 200);
    assert_eq!(parent.json::<Value>().await.unwrap()["output"][0], reasoning);
    let result = json!({"type":"function_call_output","call_id":"call_parent","output":"fixture result"});
    let next = client.post(&url).bearer_auth("key").header("x-codex-turn-metadata", &metadata)
        .json(&json!({"model":"deepseek-flash","previous_response_id":"resp_parent","input":[result]}))
        .send().await.unwrap();
    assert_eq!(next.status(), 200, "{}", next.text().await.unwrap());
    assert_eq!(next.json::<Value>().await.unwrap()["id"], "resp_next");
    let foreign = client.post(&url).bearer_auth("key").header("x-codex-turn-metadata",
        json!({"session_id":"unrelated","thread_id":"unrelated"}).to_string())
        .json(&json!({"model":"deepseek-flash","previous_response_id":"resp_parent","input":[result]}))
        .send().await.unwrap();
    assert!(!foreign.status().is_success());
    {
        let sent = observed.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent[1].get("previous_response_id").is_none());
        assert_eq!(sent[1]["input"][1]["content"], reasoning["content"]);
        assert!(sent[1]["input"][1].get("encrypted_content").is_none());
        assert_eq!(sent[1]["input"][2]["call_id"], "call_parent");
        assert_eq!(sent[1]["input"][3], result);
    }
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[tokio::test]
async fn external_results_survive_strict_http_and_websocket_http_bridges() {
    for wire in [WireApi::Responses, WireApi::Chat, WireApi::Anthropic] {
        for delivery in ["json", "sse", "websocket"] {
            let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
            let capture = captured.clone();
            let app = Router::new().fallback(move |Json(body): Json<Value>| {
                let capture = capture.clone();
                async move {
                    if let Some(error) = strict_error(wire, &body) {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error":{
                            "type":"invalid_request_error","message":error}})),
                        )
                            .into_response();
                    }
                    let n = {
                        let mut sent = capture.lock().unwrap();
                        sent.push(body.clone());
                        sent.len()
                    };
                    success(wire, &body, &format!("response-{n}"))
                }
            });
            let (mut handle, host, task, _dir) =
                fixture(app, wire, false, native::Options::default()).await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let mut body = request();
            body["stream"] = json!(delivery != "json");
            if wire == WireApi::Responses {
                // Demonstrate that this fixture rejects the original faulty wire shape.
                let raw = client
                    .post(format!("{}/responses", host.base))
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(raw.status(), 400);
            }
            if delivery == "websocket" {
                let mut ws = socket(&handle).await;
                body["type"] = json!("response.create");
                send(&mut ws, body.clone()).await;
                let first = terminal(&mut ws).await;
                assert_eq!(first["type"], "response.completed", "{wire:?}: {first}");
                send(
                    &mut ws,
                    json!({"type":"response.create","model":"m",
                    "previous_response_id":first["response"]["id"],
                    "input":[receipt(json!("latest page context")),
                        {"role":"user","content":"next turn"}]}),
                )
                .await;
                let next = terminal(&mut ws).await;
                assert_eq!(next["type"], "response.completed", "{wire:?}: {next}");
                ws.close(None).await.unwrap();
            } else {
                let result = client
                    .post(format!("http://{}/v1/responses", handle.addr))
                    .bearer_auth("key")
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                let status = result.status();
                let text = result.text().await.unwrap();
                assert_eq!(status, 200, "{wire:?}/{delivery}: {text}");
                if delivery == "sse" {
                    assert!(text.contains("response.completed"), "{text}");
                }
                if wire == WireApi::Responses {
                    // A real ID may refer to a stored upstream call absent from this delta.
                    let delta = json!({"model":"m","previous_response_id":"response-1","input":[
                        receipt(json!("latest page context")),
                        {"type":"function_call_output","call_id":"stored-call","output":"stored result"}
                    ]});
                    let response = client
                        .post(format!("http://{}/v1/responses", handle.addr))
                        .bearer_auth("key")
                        .json(&delta)
                        .send()
                        .await
                        .unwrap();
                    let status = response.status();
                    let text = response.text().await.unwrap();
                    assert_eq!(status, 200, "{text}");
                    let sent = captured.lock().unwrap();
                    assert_eq!(sent[1]["previous_response_id"], "response-1");
                    assert_eq!(sent[1]["input"][1], delta["input"][1]);
                }
            }
            let sent = captured.lock().unwrap();
            let text = sent[0].to_string();
            for value in [
                "old page context",
                "selected page",
                "first result",
                "second result",
            ] {
                assert!(text.contains(value), "{wire:?}/{delivery}: missing {value}");
            }
            if wire == WireApi::Anthropic {
                assert!(text.contains("\"data\":\"AAAA\""));
                assert!(text.contains("\"media_type\":\"image/png\""));
            } else {
                assert!(text.contains("data:image/png;base64,AAAA"));
            }
            if wire == WireApi::Responses {
                assert_eq!(sent[0]["input"][4], body["input"][4]);
                assert_eq!(sent[0]["input"][6], body["input"][6]);
            }
            if delivery == "websocket" {
                let replay = sent[1].to_string();
                assert!(replay.contains("old page context"));
                assert!(replay.contains("latest page context"));
                assert!(sent[1].get("previous_response_id").is_none());
            }
            drop(sent);
            if wire == WireApi::Responses && delivery == "json" {
                let malformed = json!({"model":"m","input":[
                    {"type":"function_call_output","output":null}
                ]});
                let response = client
                    .post(format!("http://{}/v1/responses", handle.addr))
                    .bearer_auth("key")
                    .json(&malformed)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), 400);
                assert!(response.text().await.unwrap().contains("requires call_id"));
            }
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
            task.abort();
        }
    }
}

#[tokio::test]
async fn external_results_keep_native_websocket_frames_unchanged() {
    let original = request();
    let expected = original["input"].clone();
    let app = Router::new().route(
        "/v1/responses",
        get(move |ws: axum::extract::WebSocketUpgrade| {
            let expected = expected.clone();
            async move {
                ws.on_upgrade(move |mut upstream| async move {
                    let message = upstream.recv().await.unwrap().unwrap();
                    let body: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                    assert_eq!(body["input"], expected);
                    upstream
                        .send(axum::extract::ws::Message::Text(
                            json!({
                                "type":"response.completed","response":{"id":"native-ws",
                                "status":"completed","output":[]}
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                })
            }
        }),
    );
    let (mut handle, _host, task, _dir) =
        fixture(app, WireApi::Responses, true, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    let mut body = original;
    body["type"] = json!("response.create");
    send(&mut ws, body).await;
    let event = terminal(&mut ws).await;
    assert_eq!(event["response"]["id"], "native-ws", "{event}");
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
