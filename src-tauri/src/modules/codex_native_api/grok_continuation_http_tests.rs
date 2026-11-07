//! Strict loopback upstream: first turn, agent follow-up, compact, then continue.
use super::*;

struct FailureHost {
    base: String,
    failures: Arc<std::sync::atomic::AtomicUsize>,
}
impl Host for FailureHost {
    fn authorize<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            HostFixture {
                base: self.base.clone(),
            }
            .authorize(token)
            .await
        })
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            HostFixture {
                base: self.base.clone(),
            }
            .route(caller, request)
            .await
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: 1,
                ..Default::default()
            })
        })
    }
    fn route_failed<'a>(
        &'a self,
        _: &'a str,
        _: u16,
        _: Option<std::time::Duration>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {
            self.failures
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
    }
}

async fn ordinary(State(requests): State<Requests>, Json(body): Json<Value>) -> Response {
    let mut records = requests.lock().unwrap();
    records.push(body.clone());
    let index = records.len();
    drop(records);
    for item in body["input"].as_array().unwrap() {
        assert_ne!(item["type"], "agent_message", "unadapted agent message");
        assert_ne!(
            item["type"], "compaction_trigger",
            "compact control reached generation endpoint"
        );
    }
    if body["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["content"] == "keep going")
    {
        assert_eq!(
            body["input"].as_array().unwrap().len(),
            2,
            "compaction must replace the old prefix"
        );
        assert_eq!(body["input"][0]["type"], "compaction");
        assert!(body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "compaction"
                && item["encrypted_content"] == super::super::grok_wire::test_cipher()));
    }
    let response = json!({"id":format!("resp-{index}"),"status":"completed","object":"response","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"continued"}]}],"usage":{"input_tokens":5,"output_tokens":1}});
    if body["stream"] == true {
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!(
                "event: response.completed\ndata: {}\n\n",
                json!({"type":"response.completed","response":response})
            )))
            .unwrap()
    } else {
        Json(response).into_response()
    }
}

async fn compact(State(requests): State<Requests>, Json(body): Json<Value>) -> Response {
    requests.lock().unwrap().push(body.clone());
    for field in [
        "tools",
        "tool_choice",
        "stream",
        "max_output_tokens",
        "temperature",
        "top_p",
    ] {
        assert!(
            body.get(field).is_none(),
            "unsupported compact field {field}"
        );
    }
    assert!(!body["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "compaction_trigger"));
    Json(json!({"id":"cmp-strict","object":"response.compaction","output":[{"type":"compaction","id":"compact-item","encrypted_content":super::super::grok_wire::test_cipher()}],"usage":{"input_tokens":8,"output_tokens":2}})).into_response()
}

async fn websocket(State(requests): State<Requests>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        while let Some(Ok(Message::Text(raw))) = socket.next().await {
            let body: Value = serde_json::from_str(&raw).unwrap();
            assert!(!body["input"].as_array().unwrap().iter().any(|item| matches!(item["type"].as_str(), Some("agent_message" | "compaction_trigger"))));
            requests.lock().unwrap().push(body);
            let event = json!({"type":"response.completed","response":{"id":"ws-first","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"continued"}]}]}});
            if socket.send(Message::Text(event.to_string().into())).await.is_err() { return; }
        }
    })
}

fn input(trigger: bool) -> Value {
    let mut body = json!({"model":"grok-4.7","stream":true,"input":[{"type":"message","role":"user","content":"test an agent"},{"type":"agent_message","author":"/root/child","recipient":"/root","content":[{"type":"input_text","text":"child finished"}]}],"tools":[{"type":"function","name":"test","parameters":{"type":"object"}}],"tool_choice":"auto","max_output_tokens":30,"temperature":0.3});
    if trigger {
        body["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"compaction_trigger"}));
    }
    body
}

async fn receive(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(Ok(ClientMessage::Text(text))) = socket.next().await {
            let event: Value = serde_json::from_str(&text).unwrap();
            assert_ne!(event["type"], "error", "{event}");
            if event["type"] == "response.completed" {
                return event["response"].clone();
            }
        }
        panic!("missing terminal event")
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn grok_agent_and_compaction_followups_use_valid_http_sse_and_websocket_wire() {
    let requests: Requests = Default::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/responses", post(ordinary).get(websocket))
        .route("/responses/compact", post(compact))
        .with_state(requests.clone());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let mut service = start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(HostFixture { base }),
        root.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://{}/v1/responses", service.addr);
    for streaming in [false, true] {
        for trigger in [false, true] {
            let mut body = input(trigger);
            body["stream"] = json!(streaming);
            let response = client
                .post(&url)
                .bearer_auth("fixture")
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let raw = response.text().await.unwrap();
            let response: Value = if streaming {
                raw.lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .find(|event| event["type"] == "response.completed")
                    .unwrap()["response"]
                    .clone()
            } else {
                serde_json::from_str(&raw).unwrap()
            };
            assert_eq!(response["status"], "completed");
            if trigger {
                assert_eq!(
                    response["output"][0]["encrypted_content"],
                    super::super::grok_wire::test_cipher()
                );
            }
        }
    }
    let mut handshake = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    handshake
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer fixture"));
    let (mut socket, _) = tokio_tungstenite::connect_async(handshake).await.unwrap();
    let mut first = input(false);
    first["type"] = json!("response.create");
    socket
        .send(ClientMessage::Text(first.to_string().into()))
        .await
        .unwrap();
    let first_response = receive(&mut socket).await;
    assert_eq!(
        first_response["output"][0]["content"][0]["text"],
        "continued"
    );
    let mut compact_request = input(true);
    compact_request["type"] = json!("response.create");
    socket
        .send(ClientMessage::Text(compact_request.to_string().into()))
        .await
        .unwrap();
    let compact_response = receive(&mut socket).await;
    assert_eq!(compact_response["id"], "cmp-strict");
    let followup = json!({"type":"response.create","model":"grok-4.7","previous_response_id":"cmp-strict","input":[{"type":"message","role":"user","content":"keep going"}]});
    socket
        .send(ClientMessage::Text(followup.to_string().into()))
        .await
        .unwrap();
    assert_eq!(
        receive(&mut socket).await["output"][0]["content"][0]["text"],
        "continued"
    );
    assert_eq!(requests.lock().unwrap().len(), 7);
    socket.close(None).await.unwrap();
    service
        .shutdown(std::time::Duration::from_secs(5))
        .await
        .unwrap();
    task.abort();
}

#[tokio::test]
async fn grok_inline_compact_service_failures_do_not_cool_accounts() {
    for status in [500, 502, 503, 504, 429] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new().route(
            "/responses/compact",
            post(move || async move {
                (
                    axum::http::StatusCode::from_u16(status).unwrap(),
                    Json(json!({"error":{"message":"compact failure"}})),
                )
            }),
        );
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let root = tempfile::tempdir().unwrap();
        let mut service = start(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(FailureHost {
                base,
                failures: failures.clone(),
            }),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("fixture")
            .json(&input(true))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        response.bytes().await.unwrap();
        assert_eq!(
            failures.load(std::sync::atomic::Ordering::SeqCst) > 0,
            status == 429,
            "status {status}"
        );
        service
            .shutdown(std::time::Duration::from_secs(2))
            .await
            .unwrap();
        task.abort();
    }
}
