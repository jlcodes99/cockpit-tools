//! A synthetic Codex client actually opens child inference requests, returns
//! their results to the parent, and resumes both histories. Every network hop
//! is loopback; no installed client, account, credential, or real tool is used.
use super::{contract::*, start, Handle, Options};
use axum::{
    body::Body,
    extract::{ws::Message as ServerMessage, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use base64::Engine as _;
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivery {
    Json,
    Sse,
    WebSocket,
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap()
}

struct LifecycleHost {
    base: String,
    plan: bool,
    optimize: bool,
}
impl Host for LifecycleHost {
    fn multi_agent_policy<'a>(&'a self, _: &'a Caller, _: Option<&'a Route>, _: &'a str)
        -> BoxFuture<'a, Result<MultiAgentPolicy, ServiceError>> {
        Box::pin(async move { Ok(MultiAgentPolicy {
            options: MultiAgentOptions { optimize: self.optimize, ..Default::default() },
            models: Vec::new(),
        }) })
    }
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
                account_id: "lifecycle-account".into(),
                owner_id: "lifecycle-owner".into(),
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
                chatgpt: !self.plan,
                chatgpt_plan: self.plan,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[{"id":"lifecycle-model"}]})) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

fn text(role: &str, value: &str) -> Value {
    json!({"type":"message","role":role,"content":[{"type":if role == "assistant" {"output_text"} else {"input_text"},"text":value}]})
}
fn tools() -> Value {
    let tools: Vec<_> = ["spawn_agent", "send_message", "followup_task", "wait_agent"].into_iter().map(|name| {
        json!({"type":"function","name":name,"description":"Synthetic client coordination","parameters":{"type":"object","properties":{"message":{"type":"string"},"target":{"type":"string"},"task_name":{"type":"string"}}}})
    }).collect();
    json!([{"type":"namespace","name":"collaboration","description":"Synthetic child agents","tools":tools}])
}
fn agent_message(payload: &str) -> Value {
    json!({"type":"agent_message","author":"/root","recipient":"/root/reviewer",
        "internal_chat_message_metadata_passthrough":{"synthetic":true},
        "content":[{"type":"input_text","text":"Message Type: NEW_TASK\nPayload:\n"},
                   {"type":"encrypted_content","encrypted_content":payload}]})
}
fn tool_result(call: &Value, output: Value) -> Value {
    json!({"type":"function_call_output","call_id":call["call_id"],"output":output.to_string()})
}
fn declaration_namespace(body: &Value) -> String {
    let mut declarations = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in body["input"].as_array().unwrap() {
        if item["type"] == "additional_tools" {
            declarations.extend(item["tools"].as_array().unwrap().iter().cloned());
        }
    }
    declarations
        .iter()
        .find(|tool| {
            tool["type"] == "namespace"
                && tool["tools"]
                    .as_array()
                    .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "spawn_agent"))
        })
        .and_then(|tool| tool["name"].as_str())
        .unwrap_or("collaboration")
        .into()
}
fn call(body: &Value, name: &str, args: Value) -> Value {
    json!({"type":"function_call","id":format!("item-{name}"),"call_id":format!("call-{name}"),
        "namespace":declaration_namespace(body),"name":name,"arguments":args.to_string(),"status":"completed"})
}
fn signed_reasoning() -> Value {
    // Valid GPT envelope shape; synthetic bytes carry no real encrypted data.
    let mut bytes = vec![7u8; 73];
    bytes[0] = 0x80;
    bytes[1..9].fill(0);
    let encrypted = base64::engine::general_purpose::URL_SAFE.encode(bytes);
    json!({"type":"reasoning","id":"rs_synthetic-signed","encrypted_content":encrypted,
        "signature":"synthetic-signature","summary":[{"type":"summary_text","text":"Completed child review"}]})
}

struct Upstream {
    plan: bool,
    requests: Mutex<Vec<(bool, HeaderMap, Value)>>,
}
impl Upstream {
    fn response(
        &self,
        websocket: bool,
        headers: HeaderMap,
        body: Value,
    ) -> Result<Vec<Value>, String> {
        let round = {
            let mut requests = self.requests.lock().unwrap();
            requests.push((websocket, headers, body.clone()));
            requests.len()
        };
        if self.plan {
            if websocket {
                return Err("Plan credentials must use the HTTP bridge".into());
            }
            if body.get("previous_response_id").is_some() {
                return Err("Plan inference requires complete history".into());
            }
            for item in body["input"].as_array().ok_or("input must be an array")? {
                if item["type"] == "agent_message" {
                    return Err(
                        "User subscription sharing does not support agent_message input".into(),
                    );
                }
                if item["type"] == "message"
                    && [
                        "author",
                        "recipient",
                        "internal_chat_message_metadata_passthrough",
                    ]
                    .iter()
                    .any(|field| item.get(*field).is_some())
                {
                    return Err("Portable messages cannot retain internal agent metadata".into());
                }
                if item
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|parts| {
                        parts.iter().any(|part| part["type"] == "encrypted_content")
                    })
                {
                    return Err("Portable message content must use input_text".into());
                }
            }
        }
        let output = match round {
            1 => vec![call(
                &body,
                "spawn_agent",
                json!({"task_name":"reviewer","message":"Review the synthetic changes"}),
            )],
            2 => vec![text("assistant", "Initial child findings")],
            3 => vec![call(
                &body,
                "send_message",
                json!({"target":"/root/reviewer","message":"Also inspect the synthetic gateway"}),
            )],
            4 => vec![text("assistant", "Gateway inspected")],
            5 => vec![call(
                &body,
                "followup_task",
                json!({"target":"/root/reviewer","message":"Return the final review result"}),
            )],
            6 => vec![text(
                "assistant",
                "Child review complete: no synthetic defects",
            )],
            7 => vec![call(
                &body,
                "wait_agent",
                json!({"target":"/root/reviewer"}),
            )],
            8 => vec![
                signed_reasoning(),
                text("assistant", "Parent received the child review result"),
            ],
            9 => vec![text(
                "assistant",
                "Parent resumed with the complete child history",
            )],
            _ => return Err(format!("Unexpected lifecycle round {round}")),
        };
        let mut events = vec![
            json!({"type":"response.created","response":{"id":format!("lifecycle-{round}"),"status":"in_progress","output":[]}}),
        ];
        for (index, item) in output.iter().enumerate() {
            let mut added = item.clone();
            added["status"] = json!("in_progress");
            if item["type"] == "function_call" {
                added["arguments"] = json!("");
            }
            events.push(
                json!({"type":"response.output_item.added","output_index":index,"item":added}),
            );
            if let Some(arguments) = item["arguments"].as_str() {
                let split = arguments.len() / 2;
                for delta in [&arguments[..split], &arguments[split..]] {
                    events.push(json!({"type":"response.function_call_arguments.delta","output_index":index,"item_id":item["id"],"delta":delta}));
                }
                events.push(json!({"type":"response.function_call_arguments.done","output_index":index,"item_id":item["id"],"arguments":arguments}));
            }
            events
                .push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
        }
        events.push(json!({"type":"response.completed","response":{"id":format!("lifecycle-{round}"),"status":"completed","output":output,"usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}}));
        Ok(events)
    }
}
async fn http_upstream(
    State(state): State<Arc<Upstream>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let streaming = body["stream"] == true;
    match state.response(false, headers, body) {
        Ok(events) if streaming => Response::builder()
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
            .unwrap(),
        Ok(events) => Json(events.last().unwrap()["response"].clone()).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"code":"unsupported_value","param":"input","message":error}})),
        )
            .into_response(),
    }
}
async fn ws_upstream(
    State(state): State<Arc<Upstream>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        while let Some(Ok(message)) = socket.recv().await {
            match message {
                ServerMessage::Text(text) => {
                    let body: Value = serde_json::from_str(&text).unwrap();
                    let events = state.response(true, headers.clone(), body).unwrap();
                    for event in events {
                        if socket
                            .send(ServerMessage::Text(event.to_string().into()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                ServerMessage::Ping(data) => {
                    if socket.send(ServerMessage::Pong(data)).await.is_err() {
                        return;
                    }
                }
                ServerMessage::Close(_) => break,
                _ => {}
            }
        }
    })
}

struct Fixture {
    host: Arc<LifecycleHost>,
    upstream: Arc<Upstream>,
    task: tokio::task::JoinHandle<()>,
    root: tempfile::TempDir,
    handle: Handle,
}
impl Fixture {
    async fn new(plan: bool) -> Self {
        let upstream = Arc::new(Upstream {
            plan,
            requests: Mutex::new(Vec::new()),
        });
        let app = Router::new()
            .route("/v1/responses", post(http_upstream).get(ws_upstream))
            .with_state(upstream.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = Arc::new(LifecycleHost {
            base: format!("http://{}/v1", listener.local_addr().unwrap()),
            plan,
            optimize: false,
        });
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            host,
            upstream,
            task,
            root,
            handle,
        }
    }
    async fn restart(&mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.handle = start(
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

struct ClientThread {
    delivery: Delivery,
    thread: &'static str,
    socket: Option<Socket>,
    previous: Option<String>,
    input: Vec<Value>,
}
impl ClientThread {
    fn new(delivery: Delivery, thread: &'static str) -> Self {
        Self {
            delivery,
            thread,
            socket: None,
            previous: None,
            input: Vec::new(),
        }
    }
    async fn disconnect(&mut self) {
        if let Some(mut socket) = self.socket.take() {
            socket.close(None).await.unwrap();
        }
    }
    async fn send(&mut self, fixture: &Fixture, new_items: Vec<Value>) -> Value {
        self.input.extend(new_items.clone());
        let mut body = json!({"model":"lifecycle-model","stream":self.delivery != Delivery::Json,
            "instructions":"Preserve all synthetic child findings", "tools":tools(),
            "input":if fixture.host.plan && self.previous.is_some() {new_items} else {self.input.clone()}});
        if let Some(previous) = &self.previous {
            body["previous_response_id"] = json!(previous);
        }
        let response = match self.delivery {
            Delivery::Json | Delivery::Sse => {
                let response = client()
                    .post(format!("http://{}/v1/responses", fixture.handle.addr))
                    .bearer_auth("lifecycle-caller")
                    .header("user-agent", "Codex Desktop/synthetic")
                    .header("session-id", "lifecycle-session")
                    .header("thread-id", self.thread)
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                let status = response.status();
                let raw = response.text().await.unwrap();
                assert_eq!(status, 200, "{:?}: {raw}", self.delivery);
                if self.delivery == Delivery::Json {
                    serde_json::from_str(&raw).unwrap()
                } else {
                    let events: Vec<Value> = raw
                        .lines()
                        .filter_map(|line| line.strip_prefix("data: "))
                        .filter_map(|line| serde_json::from_str(line).ok())
                        .collect();
                    assert_client_events(&events);
                    events
                        .iter()
                        .find(|event| event["type"] == "response.completed")
                        .expect("terminal SSE response")["response"]
                        .clone()
                }
            }
            Delivery::WebSocket => {
                if self.socket.is_none() {
                    let mut request = format!("ws://{}/v1/responses", fixture.handle.addr)
                        .into_client_request()
                        .unwrap();
                    for (name, value) in [
                        ("authorization", "Bearer lifecycle-caller"),
                        ("user-agent", "Codex Desktop/synthetic"),
                        ("session-id", "lifecycle-session"),
                        ("thread-id", self.thread),
                    ] {
                        request.headers_mut().insert(name, value.parse().unwrap());
                    }
                    self.socket = Some(tokio_tungstenite::connect_async(request).await.unwrap().0);
                }
                body["type"] = json!("response.create");
                let socket = self.socket.as_mut().unwrap();
                socket
                    .send(Message::Text(body.to_string().into()))
                    .await
                    .unwrap();
                let events = tokio::time::timeout(Duration::from_secs(8), async {
                    let mut events = Vec::new();
                    loop {
                        match socket.next().await.expect("socket stays open").unwrap() {
                            Message::Text(text) => {
                                let event: Value = serde_json::from_str(&text).unwrap();
                                let done = matches!(
                                    event["type"].as_str(),
                                    Some("response.completed" | "response.failed" | "error")
                                );
                                events.push(event);
                                if done {
                                    return events;
                                }
                            }
                            Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                            other => panic!("unexpected websocket frame: {other:?}"),
                        }
                    }
                })
                .await
                .unwrap();
                assert_eq!(
                    events.last().unwrap()["type"],
                    "response.completed",
                    "{events:?}"
                );
                assert_client_events(&events);
                events.last().unwrap()["response"].clone()
            }
        };
        assert_eq!(response["status"], "completed", "{response}");
        self.previous = response["id"].as_str().map(str::to_owned);
        self.input
            .extend(response["output"].as_array().unwrap().iter().cloned());
        response
    }
}
fn assert_client_events(events: &[Value]) {
    for event in events {
        if event["item"]["type"] == "function_call" {
            assert_eq!(
                event["item"]["namespace"], "collaboration",
                "wire namespace must never escape to the client"
            );
        }
    }
}
fn returned_call(response: &Value, name: &str, plan: bool) -> Value {
    let call = response["output"][0].clone();
    assert_eq!(call["type"], "function_call");
    assert_eq!(call["namespace"], "collaboration");
    assert_eq!(call["name"], name);
    if plan && name != "wait_agent" {
        assert_eq!(call["encrypted_function_args"], json!([]));
    }
    if !plan {
        assert!(call.get("encrypted_function_args").is_none());
    }
    call
}
fn message_argument(call: &Value) -> String {
    serde_json::from_str::<Value>(call["arguments"].as_str().unwrap()).unwrap()["message"]
        .as_str()
        .unwrap()
        .into()
}

async fn lifecycle(plan: bool, delivery: Delivery) {
    let mut fixture = Fixture::new(plan).await;
    let mut parent = ClientThread::new(delivery, "lifecycle-parent");
    let mut child = ClientThread::new(delivery, "lifecycle-child");
    let spawn = returned_call(
        &parent
            .send(
                &fixture,
                vec![text("user", "Review my synthetic changes using a child")],
            )
            .await,
        "spawn_agent",
        plan,
    );
    let first_message = agent_message(&message_argument(&spawn));
    let first = child
        .send(
            &fixture,
            vec![
                text("user", "You are the synthetic reviewer"),
                first_message.clone(),
            ],
        )
        .await;
    assert_eq!(
        first["output"][0]["content"][0]["text"],
        "Initial child findings"
    );
    let send = returned_call(
        &parent
            .send(
                &fixture,
                vec![tool_result(
                    &spawn,
                    json!({"agent_id":"reviewer","status":"running"}),
                )],
            )
            .await,
        "send_message",
        plan,
    );
    let second_message = agent_message(&message_argument(&send));
    let second = child.send(&fixture, vec![second_message.clone()]).await;
    assert_eq!(
        second["output"][0]["content"][0]["text"],
        "Gateway inspected"
    );
    let followup = returned_call(
        &parent
            .send(
                &fixture,
                vec![tool_result(&send, json!({"status":"delivered"}))],
            )
            .await,
        "followup_task",
        plan,
    );
    let last_message = agent_message(&message_argument(&followup));
    let child_result = child.send(&fixture, vec![last_message.clone()]).await;
    let result_text = child_result["output"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(result_text, "Child review complete: no synthetic defects");
    let wait = returned_call(
        &parent
            .send(
                &fixture,
                vec![tool_result(&followup, json!({"status":"running"}))],
            )
            .await,
        "wait_agent",
        plan,
    );
    let result = parent
        .send(
            &fixture,
            vec![tool_result(
                &wait,
                json!({"agent_id":"reviewer","status":"completed","result":result_text}),
            )],
        )
        .await;
    assert_eq!(
        result["output"][0],
        signed_reasoning(),
        "encrypted reasoning and signature survive the result path"
    );
    assert_eq!(
        result["output"][1]["content"][0]["text"],
        "Parent received the child review result"
    );
    parent.disconnect().await;
    child.disconnect().await;
    fixture.restart().await;
    let resumed = parent
        .send(
            &fixture,
            vec![text("user", "Continue the same review after restart")],
        )
        .await;
    assert_eq!(
        resumed["output"][0]["content"][0]["text"],
        "Parent resumed with the complete child history"
    );
    {
        let requests = fixture.upstream.requests.lock().unwrap();
        assert_eq!(requests.len(), 9, "no silent retry or dropped child turn");
        for (index, (ws, _, body)) in requests.iter().enumerate() {
            assert_eq!(
                *ws,
                !plan && delivery == Delivery::WebSocket,
                "transport in round {}",
                index + 1
            );
            if plan {
                assert_eq!(body["stream"], true);
                assert_eq!(body["store"], false);
            }
        }
        for (request_index, originals) in [
            (1, vec![&first_message]),
            (3, vec![&first_message, &second_message]),
            (5, vec![&first_message, &second_message, &last_message]),
        ] {
            let input = requests[request_index].2["input"].as_array().unwrap();
            for original in originals {
                if plan {
                    let payload = &original["content"][1]["encrypted_content"];
                    let message = input
                        .iter()
                        .find(|item| {
                            item["type"] == "message"
                                && item["content"].as_array().is_some_and(|parts| {
                                    parts.iter().any(|part| &part["text"] == payload)
                                })
                        })
                        .expect("child content is kept as portable message");
                    assert_eq!(message["role"], "user");
                    assert_eq!(message["content"][0], original["content"][0]);
                    assert!(message.get("author").is_none());
                    assert!(message.get("recipient").is_none());
                } else {
                    assert!(
                        input.contains(original),
                        "native agent_message bytes must survive unchanged"
                    );
                }
            }
        }
        let input = requests[8].2["input"].as_array().unwrap();
        for name in ["spawn_agent", "send_message", "followup_task", "wait_agent"] {
            let id = format!("call-{name}");
            assert_eq!(
                input
                    .iter()
                    .filter(|item| item["type"] == "function_call" && item["call_id"] == id)
                    .count(),
                1,
                "exactly one historical call {name}"
            );
            assert_eq!(
                input
                    .iter()
                    .filter(|item| item["type"] == "function_call_output" && item["call_id"] == id)
                    .count(),
                1,
                "exactly one historical result {name}"
            );
        }
        assert!(input.contains(&signed_reasoning()));
        assert!(input.iter().any(|item| item["output"]
            .as_str()
            .is_some_and(|value| value.contains(result_text))));
        assert!(
            !input
                .iter()
                .any(|item| item["content"].as_array().is_some_and(|parts| parts
                    .iter()
                    .any(|part| part["text"] == "You are the synthetic reviewer"))),
            "parent history must not acquire child-local instructions"
        );
    }
    parent.disconnect().await;
    fixture.close().await;
}

#[tokio::test]
async fn multi_agent_lifecycle_official_plan_json_sse_and_websocket_bridge() {
    for delivery in [Delivery::Json, Delivery::Sse, Delivery::WebSocket] {
        lifecycle(true, delivery).await;
    }
}

#[tokio::test]
async fn multi_agent_lifecycle_native_json_sse_and_websocket_keep_opaque_agent_input() {
    for delivery in [Delivery::Json, Delivery::Sse, Delivery::WebSocket] {
        lifecycle(false, delivery).await;
    }
}

#[cfg(target_os = "macos")]
#[path = "multi_agent_lifecycle_cli_tests.rs"]
mod real_cli;
