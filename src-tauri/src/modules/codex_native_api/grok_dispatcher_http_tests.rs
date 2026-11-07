//! Over-limit namespace transport tests use only the local fixture server.
use super::*;

fn request() -> Value {
    let mut tools: Vec<Value> = (0..198).map(|i| json!({"type":"function","name":format!("native_{i}"),"parameters":{"type":"object"}})).collect();
    tools.push(json!({"type":"namespace","name":"tools","tools":[{"type":"function","name":"lookup","parameters":{"type":"object","properties":{"query":{"type":"string"}}}},{"type":"custom","name":"patch","format":{"type":"text"}},{"type":"function","name":"status","parameters":{"type":"object"}}]}));
    tools.push(json!({"type":"tool_search","execution":"client"}));
    json!({"model":"grok-4.7","tools":tools,"input":[{"type":"function_call","name":"lookup","namespace":"tools","call_id":"old","arguments":"{\"query\":\"history\"}"},{"type":"function_call_output","call_id":"old","output":"found"},{"role":"user","content":"patch"}]})
}
fn events(body: &Value) -> Vec<Value> {
    let dispatcher = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| {
            tool["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("grok_namespace_"))
        })
        .unwrap();
    let name = dispatcher["name"].clone();
    let arguments = json!({"name":"patch","arguments":{"input":"PATCH\nexact"}}).to_string();
    let item = json!({"type":"function_call","id":"fc_dispatch","call_id":"call_dispatch","name":name,"arguments":arguments,"status":"completed"});
    vec![
        json!({"type":"response.created","response":{"id":"resp_dispatch","status":"in_progress"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_dispatch","call_id":"call_dispatch","name":name,"arguments":""}}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc_dispatch","delta":arguments}),
        json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"fc_dispatch","arguments":arguments}),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
        json!({"type":"response.completed","response":{"id":"resp_dispatch","status":"completed","output":[item]}}),
    ]
}
async fn http(State(requests): State<Requests>, Json(body): Json<Value>) -> Response {
    requests.lock().unwrap().push(body.clone());
    let events = events(&body);
    if body["stream"] == true {
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(
                events
                    .iter()
                    .map(|event| format!("data: {event}\n\n"))
                    .collect::<String>(),
            ))
            .unwrap()
    } else {
        Json(events.last().unwrap()["response"].clone()).into_response()
    }
}
async fn ws(State(requests): State<Requests>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let body: Value = serde_json::from_str(&text).unwrap();
            requests.lock().unwrap().push(body.clone());
            for event in events(&body) {
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
    assert_eq!(response["output"][0]["type"], "custom_tool_call");
    assert_eq!(response["output"][0]["name"], "patch");
    assert_eq!(response["output"][0]["namespace"], "tools");
    assert_eq!(response["output"][0]["call_id"], "call_dispatch");
    assert_eq!(response["output"][0]["input"], "PATCH\nexact");
}
fn assert_events(events: &[Value]) {
    let deltas: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "response.custom_tool_call_input.delta")
        .collect();
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0]["delta"], "PATCH\nexact");
    assert!(!events
        .iter()
        .any(|event| event["type"] == "response.function_call_arguments.delta"));
    assert_response(
        &events
            .iter()
            .find(|event| event["type"] == "response.completed")
            .unwrap()["response"],
    );
}

#[tokio::test]
async fn grok_namespace_dispatcher_roundtrips_http_sse_and_websocket() {
    let requests: Requests = Default::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/responses", post(http).get(ws))
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
        .json(&request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_response(&response.json::<Value>().await.unwrap());
    let mut body = request();
    body["stream"] = json!(true);
    let response = client
        .post(&url)
        .bearer_auth("fixture")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let raw = response.text().await.unwrap();
    let events: Vec<Value> = raw
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|raw| serde_json::from_str(raw).ok())
        .collect();
    assert_events(&events);
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
        assert_eq!(body["tools"].as_array().unwrap().len(), 200);
        let dispatcher = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| {
                tool["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("grok_namespace_"))
            })
            .unwrap();
        assert_eq!(body["input"][0]["name"], dispatcher["name"]);
        let history: Value =
            serde_json::from_str(body["input"][0]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(history["name"], "lookup");
        assert_eq!(history["arguments"], "{\"query\":\"history\"}");
        assert_eq!(
            dispatcher["parameters"]["properties"]["name"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
    socket.close(None).await.unwrap();
    handle
        .shutdown(std::time::Duration::from_secs(5))
        .await
        .unwrap();
    task.abort();
}
