//! Pre-Rust Go histories must reach the actual provider in its supported wire format.
use super::{contract::*, start, Handle, Options};
use axum::{
    body::Body,
    extract::{ws::Message as ServerMessage, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[derive(Clone, Copy, Debug)]
enum Delivery {
    Json,
    Sse,
    WebSocket,
}

#[derive(Clone, Copy)]
enum HistoryMode {
    DeclaredTools,
    NoCurrentTools,
    ResponsesLite,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap()
}

struct LegacyHost {
    base: String,
    native: bool,
    optimize: bool,
}
impl Host for LegacyHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "legacy-caller".into(),
                label: "fixture".into(),
            })
        })
    }
    fn multi_agent_policy<'a>(
        &'a self,
        _: &'a Caller,
        _: Option<&'a Route>,
        _: &'a str,
    ) -> BoxFuture<'a, Result<MultiAgentPolicy, ServiceError>> {
        Box::pin(async move {
            Ok(MultiAgentPolicy {
                options: MultiAgentOptions {
                    optimize: self.optimize,
                    ..Default::default()
                },
                models: vec![],
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
                account_id: "legacy-account".into(),
                owner_id: "legacy-owner".into(),
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
                chatgpt: self.native,
                chatgpt_plan: false,
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
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(true) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

struct StrictUpstream {
    native: bool,
    sent: Mutex<Vec<Value>>,
}
impl StrictUpstream {
    fn response(&self, body: &Value) -> Value {
        self.sent.lock().unwrap().push(body.clone());
        let invalid = body["input"].as_array().unwrap().iter().find(|item| {
            item["type"] == "agent_message"
                && (!self.native
                    || item["content"].as_array().is_some_and(|parts| {
                        parts.iter().any(|part| part["type"] == "encrypted_content")
                    }))
        });
        if let Some(item) = invalid {
            return json!({"type":"response.failed","response":{"status":"failed","error":{
                "code":"invalid_encrypted_content",
                "message":format!("The encrypted content for item {} could not be verified. Reason: encrypted content hydration failed: Encrypted content could not be decrypted or parsed.", item["id"].as_str().unwrap_or("unknown"))
            }}});
        }
        json!({"type":"response.completed","response":{"id":"resp-legacy-answer","status":"completed",
            "output":[{"type":"message","id":"msg_answer","role":"assistant","content":[{"type":"output_text","text":"continued"}]}]}})
    }
}

async fn upstream_http(
    State(state): State<Arc<StrictUpstream>>,
    Json(body): Json<Value>,
) -> Response {
    let event = state.response(&body);
    if event["type"] == "response.failed" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":event["response"]["error"]})),
        )
            .into_response();
    }
    if body["stream"] == true {
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!("data: {event}\n\n")))
            .unwrap()
    } else {
        Json(event["response"].clone()).into_response()
    }
}
async fn upstream_ws(
    State(state): State<Arc<StrictUpstream>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        while let Some(Ok(ServerMessage::Text(raw))) = socket.next().await {
            let body: Value = serde_json::from_str(&raw).unwrap();
            if socket
                .send(ServerMessage::Text(
                    state.response(&body).to_string().into(),
                ))
                .await
                .is_err()
            {
                break;
            }
        }
    })
}

fn legacy_request(delivery: Delivery) -> Value {
    json!({"model":"legacy-model","stream":!matches!(delivery, Delivery::Json),"store":false,
    "client_metadata":{"session_id":"legacy-root","thread_id":"legacy-root"},
    "tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent",
        "parameters":{"type":"object","properties":{"message":{"type":"string","encrypted":true}}}}]}],
    "input":[
        {"type":"message","role":"user","content":"original task"},
        {"type":"reasoning","id":"rs_old","summary":[],"encrypted_content":super::codex_cpa_wire::synthetic_gpt_signature(31)},
        {"type":"function_call","id":"fc_old","namespace":"collaboration","name":"spawn_agent","call_id":"legacy-call",
            "arguments":"{\"message\":\"原任务原文\\nkeep exact bytes\"}"},
        {"type":"function_call_output","call_id":"legacy-call","output":"delivered"},
        {"type":"agent_message","id":"amsg_legacy","author":"/root","recipient":"/root/worker",
            "internal_chat_message_metadata_passthrough":{"turn":"original"},"extension":"preserved",
            "content":[{"type":"input_text","text":"Payload:\n"},
                {"type":"encrypted_content","encrypted_content":"原任务原文\nkeep exact bytes","extension":7}]},
        {"type":"message","role":"user","content":"continue the original conversation"}
    ]})
}

async fn send(handle: &Handle, body: &Value, delivery: Delivery) -> Value {
    if matches!(delivery, Delivery::WebSocket) {
        let mut request = format!("ws://{}/v1/responses", handle.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer fixture".parse().unwrap());
        request.headers_mut().insert(
            "user-agent",
            "Codex Desktop/legacy-fixture".parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let mut wire = body.clone();
        wire["type"] = json!("response.create");
        socket
            .send(Message::Text(wire.to_string().into()))
            .await
            .unwrap();
        let terminal = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let message = socket.next().await.unwrap().unwrap();
                if let Message::Text(raw) = message {
                    let event: Value = serde_json::from_str(&raw).unwrap();
                    assert_ne!(event["type"], "error", "{event}");
                    assert_ne!(event["type"], "response.failed", "{event}");
                    if event["type"] == "response.completed" {
                        break event["response"].clone();
                    }
                }
            }
        })
        .await
        .unwrap();
        socket.close(None).await.unwrap();
        terminal
    } else {
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("fixture")
            .header("user-agent", "Codex Desktop/legacy-fixture")
            .json(body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let raw = response.text().await.unwrap();
        assert!(status.is_success(), "{status}: {raw}");
        if matches!(delivery, Delivery::Json) {
            serde_json::from_str(&raw).unwrap()
        } else {
            let events: Vec<Value> = raw
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            let event = events
                .iter()
                .find(|event| event["type"] == "response.completed");
            assert!(event.is_some(), "{raw}");
            event.unwrap()["response"].clone()
        }
    }
}

async fn legacy_roundtrip(native: bool, mode: HistoryMode) {
    for delivery in [Delivery::Json, Delivery::Sse, Delivery::WebSocket] {
        let source = Arc::new(StrictUpstream {
            native,
            sent: Mutex::new(vec![]),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/responses", post(upstream_http).get(upstream_ws))
            .with_state(source.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.sqlite");
        let host = Arc::new(LegacyHost {
            base,
            native,
            optimize: true,
        });
        let mut original = legacy_request(delivery);
        match mode {
            HistoryMode::DeclaredTools => {}
            HistoryMode::NoCurrentTools => {
                original.as_object_mut().unwrap().remove("tools");
            }
            HistoryMode::ResponsesLite => {
                original["client_metadata"]
                    ["ws_request_header_x_openai_internal_codex_responses_lite"] = json!(true);
            }
        }
        let saved = original.clone();
        for turn in 0..2 {
            let mut handle = start(
                "127.0.0.1:0".parse().unwrap(),
                host.clone(),
                path.clone(),
                Options::default(),
            )
            .await
            .unwrap();
            let mut body = original.clone();
            if turn == 1 {
                body["input"].as_array_mut().unwrap().push(
                    json!({"type":"message","role":"user","content":"continue after restart"}),
                );
            }
            let response = send(&handle, &body, delivery).await;
            assert_eq!(response["output"][0]["content"][0]["text"], "continued");
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
        }
        let sent = source.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        for body in sent.iter() {
            let input = body["input"].as_array().unwrap();
            let reasoning = input.iter().find(|item| item["id"] == "rs_old").unwrap();
            assert_eq!(
                reasoning["encrypted_content"],
                saved["input"][1]["encrypted_content"]
            );
            let call = input
                .iter()
                .find(|item| item["type"] == "function_call" && item["call_id"] == "legacy-call")
                .unwrap();
            assert_eq!(call["arguments"], saved["input"][2]["arguments"]);
            let result = input
                .iter()
                .find(|item| {
                    item["type"] == "function_call_output" && item["call_id"] == "legacy-call"
                })
                .unwrap();
            assert_eq!(result, &saved["input"][3]);
            if matches!(mode, HistoryMode::ResponsesLite) {
                assert_eq!(body["parallel_tool_calls"], false);
            }
            let item = input
                .iter()
                .find(|item| item["extension"] == "preserved")
                .unwrap();
            assert_eq!(
                item["type"],
                if native { "agent_message" } else { "message" }
            );
            assert_eq!(
                item["content"][1],
                json!({"type":"input_text","text":"原任务原文\nkeep exact bytes","extension":7})
            );
            assert_eq!(item["extension"], "preserved");
            if native {
                assert_eq!(item["id"], "amsg_legacy");
            } else {
                let metadata: Value = serde_json::from_str(
                    item["content"][2]["text"]
                        .as_str()
                        .unwrap()
                        .strip_prefix("Agent routing metadata: ")
                        .unwrap(),
                )
                .unwrap();
                for key in [
                    "id",
                    "author",
                    "recipient",
                    "internal_chat_message_metadata_passthrough",
                ] {
                    assert_eq!(metadata[key], saved["input"][4][key]);
                    assert!(item.get(key).is_none());
                }
            }
        }
        assert_eq!(original, saved);
        server.abort();
    }
}

#[tokio::test]
async fn legacy_agent_public_responses_history_replays_over_json_sse_and_websocket_after_restart() {
    legacy_roundtrip(false, HistoryMode::DeclaredTools).await;
}

#[tokio::test]
async fn legacy_agent_codex_oauth_history_preserves_native_envelope_over_all_transports() {
    legacy_roundtrip(true, HistoryMode::DeclaredTools).await;
}

#[tokio::test]
async fn legacy_agent_codex_oauth_replays_without_current_collaboration_declarations() {
    legacy_roundtrip(true, HistoryMode::NoCurrentTools).await;
}

#[tokio::test]
async fn legacy_agent_codex_oauth_responses_lite_replays_old_history_after_restart() {
    legacy_roundtrip(true, HistoryMode::ResponsesLite).await;
}

#[test]
fn legacy_agent_projection_respects_saved_opt_out_and_client_detection() {
    let original = legacy_request(Delivery::Json);
    for (agent, optimize) in [("Codex Desktop/fixture", false), ("curl/8", true)] {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", agent.parse().unwrap());
        let mut body = original.clone();
        super::multi_agent_compat::prepare_portable_gateway_input(
            &headers,
            &mut body,
            MultiAgentOptions {
                optimize,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(body, original);
    }
}
