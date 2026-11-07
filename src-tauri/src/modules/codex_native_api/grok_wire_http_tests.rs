//! Real loopback wire captures across JSON, SSE and WebSocket transports.
use super::*;

fn compatibility_request() -> Value {
    json!({"model":"grok-4.7","input":[
        {"type":"additional_tools","tools":[{"type":"namespace","name":"history","tools":[{"type":"function","name":"lookup","parameters":{"type":"object","properties":{"value":{"$ref":"#/$defs/Value"}},"$defs":{"Value":{"type":"number","minimum":1}}}}]}]},
        {"role":"user","content":"test compatibility"}
    ],"tools":[
        {"type":"function","name":"web_search"},
        {"type":"web_search_preview","external_web_access":true,"search_context_size":"high","filters":{"allowed_domains":["example.test"]}},
        {"type":"namespace","name":"codex_app","tools":[{"type":"function","name":"automation_update","strict":true,"parameters":{"oneOf":[{"type":"string"},{"type":"object"}]}}]}
    ],"tool_choice":"auto","stream_options":{"include_usage":true},"stop":["end"],"prompt_cache_retention":"24h","safety_identifier":"fixture"})
}

fn compatibility_events(body: &Value) -> Vec<Value> {
    let name = body["tools"][0]["name"].clone();
    let reasoning = json!({"type":"reasoning","id":"rs_compat","encrypted_content":"loopback-opaque","content":[{"type":"reasoning_text","text":"think"}]});
    let call = json!({"type":"function_call","id":"fc_compat","call_id":"call_compat","name":name,"arguments":"{}","status":"completed"});
    vec![
        json!({"type":"response.created","response":{"id":"resp_compat","status":"in_progress"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_compat","content":[]}}),
        json!({"type":"response.content_part.added","output_index":0,"item_id":"rs_compat","content_index":0,"part":{"type":"reasoning_text","text":""}}),
        json!({"type":"response.reasoning_text.delta","output_index":0,"item_id":"rs_compat","content_index":0,"delta":"think"}),
        json!({"type":"response.reasoning_text.done","output_index":0,"item_id":"rs_compat","content_index":0,"text":"think"}),
        json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
        json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_compat","call_id":"call_compat","name":name,"arguments":""}}),
        json!({"type":"response.output_item.done","output_index":1,"item":call}),
        json!({"type":"response.completed","response":{"id":"resp_compat","status":"completed","output":[reasoning,call]}}),
    ]
}

async fn compatibility_http(State(requests): State<Requests>, Json(body): Json<Value>) -> Response {
    requests.lock().unwrap().push(body.clone());
    let events = compatibility_events(&body);
    if body["stream"] == true {
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(
                events
                    .iter()
                    .map(|event| {
                        format!(
                            "event: {}\ndata: {event}\n\n",
                            event["type"].as_str().unwrap()
                        )
                    })
                    .collect::<String>(),
            ))
            .unwrap()
    } else {
        Json(events.last().unwrap()["response"].clone()).into_response()
    }
}

async fn compatibility_ws(State(requests): State<Requests>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        while let Some(Ok(Message::Text(raw))) = socket.next().await {
            let body: Value = serde_json::from_str(&raw).unwrap();
            requests.lock().unwrap().push(body.clone());
            for event in compatibility_events(&body) {
                if socket
                    .send(Message::Text(event.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    })
}

fn assert_response(response: &Value) {
    assert_eq!(
        response["output"][0]["summary"],
        json!([{"type":"summary_text","text":"think"}])
    );
    assert_eq!(
        response["output"][0]["encrypted_content"],
        "loopback-opaque"
    );
    assert!(response["output"][0].get("content").is_none());
    assert_eq!(response["output"][1]["name"], "web_search");
    assert_eq!(response["output"][1]["call_id"], "call_compat");
}

fn assert_events(events: &[Value]) {
    assert!(events.iter().any(
        |event| event["type"] == "response.reasoning_summary_text.delta"
            && event["summary_index"] == 0
    ));
    assert!(events.iter().any(
        |event| event["type"] == "response.reasoning_summary_text.done" && event["text"] == "think"
    ));
    assert!(events.iter().any(
        |event| event["type"] == "response.reasoning_summary_part.done"
            && event["part"]["type"] == "summary_text"
    ));
    assert!(!events.iter().any(|event| event["type"]
        .as_str()
        .unwrap_or("")
        .starts_with("response.reasoning_text")));
    assert_response(
        &events
            .iter()
            .find(|event| event["type"] == "response.completed")
            .unwrap()["response"],
    );
}

#[tokio::test]
async fn grok_compatibility_captures_http_sse_and_websocket_wire() {
    let requests: Requests = Default::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/responses", post(compatibility_http).get(compatibility_ws))
        .with_state(requests.clone());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let mut handle = start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(HostFixture { base }),
        root.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let url = format!("http://{}/v1/responses", handle.addr);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .post(&url)
        .bearer_auth("fixture")
        .json(&compatibility_request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_response(&response.json::<Value>().await.unwrap());
    let mut body = compatibility_request();
    body["stream"] = json!(true);
    let response = client
        .post(&url)
        .bearer_auth("fixture")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let sse = response.text().await.unwrap();
    let events: Vec<Value> = sse
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|raw| serde_json::from_str(raw).ok())
        .collect();
    assert_events(&events);
    assert!(!sse.contains("event: response.reasoning_text."));
    let mut connection = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    connection
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer fixture"));
    let (mut socket, _) = tokio_tungstenite::connect_async(connection).await.unwrap();
    body["type"] = json!("response.create");
    socket
        .send(ClientMessage::Text(body.to_string().into()))
        .await
        .unwrap();
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut events = vec![];
        while let Some(Ok(ClientMessage::Text(raw))) = socket.next().await {
            let event: Value = serde_json::from_str(&raw).unwrap();
            assert_ne!(event["type"], "error", "{event}");
            let completed = event["type"] == "response.completed";
            events.push(event);
            if completed {
                break;
            }
        }
        events
    })
    .await
    .unwrap();
    assert_events(&events);
    let captured = requests.lock().unwrap().clone();
    assert_eq!(captured.len(), 3);
    for body in captured {
        assert_eq!(
            body["input"],
            json!([{ "role":"user", "content":"test compatibility" }])
        );
        assert_eq!(body["tools"].as_array().unwrap().len(), 4);
        assert_ne!(body["tools"][0]["name"], "web_search");
        assert_eq!(
            body["tools"][0]["parameters"],
            json!({"type":"object","properties":{}})
        );
        assert_eq!(body["tools"][1]["type"], "web_search");
        assert!(body["tools"][1].get("external_web_access").is_none());
        assert!(body["tools"][1].get("search_context_size").is_none());
        assert_eq!(
            body["tools"][1]["filters"]["allowed_domains"],
            json!(["example.test"])
        );
        assert_eq!(
            body["tools"][2]["parameters"],
            json!({"type":"object","properties":{},"additionalProperties":true})
        );
        assert_eq!(body["tools"][2]["strict"], false);
        assert_eq!(
            body["tools"][3]["parameters"]["properties"]["value"],
            json!({"type":"number","minimum":1})
        );
        for field in [
            "stop",
            "stream_options",
            "safety_identifier",
            "prompt_cache_retention",
        ] {
            assert!(body.get(field).is_none());
        }
    }
    socket.close(None).await.unwrap();
    handle
        .shutdown(std::time::Duration::from_secs(5))
        .await
        .unwrap();
    task.abort();
}
