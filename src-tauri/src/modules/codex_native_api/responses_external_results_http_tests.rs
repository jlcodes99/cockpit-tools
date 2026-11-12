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
                .or_else(|| input.iter().enumerate().any(|(index, item)| item["type"] == "function_call"
                    && !input.get(index + 1).is_some_and(|output| output["type"] == "function_call_output"
                        && output["call_id"] == item["call_id"])).then_some("Tool result must immediately follow its call"));
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
    input.swap(1, 2);
    input.insert(3, json!({
        "type":"function_call_output","call_id":"missing","output":"real result committed"
    }));
    let response = local.post(&gateway_url).bearer_auth("key")
        .json(&complete).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let mut interleaved = complete.clone();
    let hook = json!({"type":"message","role":"developer","content":"PostToolUse hook context"});
    interleaved["input"].as_array_mut().unwrap().insert(1, hook.clone());
    let response = host.client.post(&upstream_url).json(&interleaved).send().await.unwrap();
    assert_eq!(response.status(), 400);
    assert!(response.text().await.unwrap().contains("must immediately follow"));
    let response = local.post(&gateway_url).bearer_auth("key")
        .json(&interleaved).send().await.unwrap();
    assert_eq!(response.status(), 200);
    {
        let sent = captured.lock().unwrap();
        assert_eq!(sent.len(), 4);
        for wire in &sent[..2] {
            assert_eq!(wire["parallel_tool_calls"], false);
            assert_eq!(wire["input"][3]["call_id"], "missing");
            assert!(wire["input"][3]["output"].as_str().unwrap().starts_with("Tool result unavailable"));
            assert_eq!(wire["input"][1], original["input"][2]);
            assert_eq!(wire["input"][4]["type"], "message");
            assert!(wire["input"][4]["content"][0]["text"].as_str().unwrap().contains("cross-thread context"));
        }
        assert_eq!(sent[2], complete);
        let mut repaired = complete;
        repaired["input"].as_array_mut().unwrap().insert(2, hook);
        assert_eq!(sent[3], repaired);
    }
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();

    let (mut unrelated, _, upstream, _dir) =
        fixture(strict, WireApi::Responses, false, native::Options::default()).await;
    let response = local.post(format!("http://{}/v1/responses", unrelated.addr))
        .bearer_auth("key").json(&original).send().await.unwrap();
    assert_eq!(response.status(), 400, "other Responses upstreams retain their wire input");
    assert!(response.text().await.unwrap().contains("No tool output found"));
    unrelated.shutdown(Duration::from_secs(2)).await.unwrap();
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
