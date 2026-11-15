use super::*;
use axum::extract::ws::{Message as UpMessage, WebSocketUpgrade};
use std::sync::atomic::AtomicUsize;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SeedFault {
    #[default]
    None,
    Reject,
    Output,
    OutputAfterCompletion,
    Silent,
}

#[derive(Default)]
struct SeedServer {
    inputs: Mutex<Vec<Value>>,
    message_sizes: Mutex<Vec<usize>>,
    generated: AtomicUsize,
    warms: AtomicUsize,
    http_calls: AtomicUsize,
    fault: SeedFault,
    echo_large_catalog: AtomicBool,
    connections: AtomicUsize,
    connection_inputs: Mutex<Vec<Vec<Value>>>,
}

async fn endpoint(State(state): State<Arc<SeedServer>>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(40 * 1024 * 1024).max_frame_size(40 * 1024 * 1024)
        .on_upgrade(move |mut socket| async move {
            let connection = state.connections.fetch_add(1, Ordering::SeqCst);
            state.connection_inputs.lock().unwrap().push(Vec::new());
            let mut previous = None::<String>;
            while let Some(Ok(UpMessage::Text(text))) = socket.recv().await {
                let body: Value = serde_json::from_str(&text).unwrap();
                state.message_sizes.lock().unwrap().push(text.len());
                assert_eq!(body.get("previous_response_id").and_then(Value::as_str), previous.as_deref());
                let warm = body["generate"] == false;
                let preload = warm && !body["input"].as_array().unwrap().is_empty();
                if preload && state.fault == SeedFault::Reject {
                    let event = json!({"type":"error","status":400,"error":{"code":"unsupported_parameter","param":"generate","message":"generate is unsupported"}});
                    let _ = socket.send(UpMessage::Text(event.to_string().into())).await;
                    continue;
                }
                if preload && state.fault == SeedFault::Output {
                    let event = json!({"type":"response.output_text.delta","delta":"unexpected generated output"});
                    let _ = socket.send(UpMessage::Text(event.to_string().into())).await;
                    continue;
                }
                if preload && state.fault == SeedFault::Silent { continue; }
                state.inputs.lock().unwrap().extend(body["input"].as_array().unwrap().iter().cloned());
                state.connection_inputs.lock().unwrap()[connection].extend(body["input"].as_array().unwrap().iter().cloned());
                let count = if warm { state.warms.fetch_add(1,Ordering::SeqCst) }
                    else { state.generated.fetch_add(1,Ordering::SeqCst) };
                let id = format!("{}-{count}",if warm {"warm"} else {"answer"});
                let output = if warm {json!([])} else {json!([{"type":"message","id":"answer-item","role":"assistant","content":[{"type":"output_text","text":"ok"}]}])};
                let mut created = json!({"id":id,"status":"in_progress","output":[]});
                let mut completed = json!({"id":id,"status":"completed","output":output});
                if state.echo_large_catalog.load(Ordering::SeqCst) {
                    for response in [&mut created,&mut completed] {
                        response["tools"] = json!([{"type":"function","name":"unused","description":"schema documentation ".repeat(4000),"parameters":{"type":"object","properties":{}}}]);
                    }
                }
                if warm {
                    for response in [&mut created,&mut completed] {
                        response["generate"] = json!(false);
                        response["image"] = json!({"context_compression":"enabled"});
                        response["reasoning"] = json!({"effort":"low","context":"automatic","mode":"adaptive","summary":null});
                        response["tool_usage"] = json!({"web_search":{"num_requests":0},"image_gen":{"input_tokens":0,"output_tokens":0}});
                    }
                }
                let events = [
                    json!({"type":"responsesapi.websocket_timing","timing":{"preprocess_ms":1}}),
                    json!({"type":"response.created","response":created}),
                    json!({"type":"response.completed","response":completed}),
                ];
                let mut documents = events.iter().map(Value::to_string).collect::<Vec<_>>();
                if preload && state.fault == SeedFault::OutputAfterCompletion {
                    documents.push(json!({"type":"response.output_text.delta","delta":"unexpected generated output"}).to_string());
                }
                let documents = documents.join("\n");
                if socket.send(UpMessage::Text(documents.into())).await.is_err() { return; }
                previous = Some(id);
            }
        })
}

async fn http(State(state): State<Arc<SeedServer>>, Json(body): Json<Value>) -> Json<Value> {
    state.http_calls.fetch_add(1, Ordering::SeqCst);
    assert!(body.get("previous_response_id").is_none());
    *state.inputs.lock().unwrap() = body["input"].as_array().unwrap().clone();
    Json(
        json!({"id":"http-answer","object":"response","status":"completed","output":[
            {"type":"message","id":"answer-item","role":"assistant","content":[{"type":"output_text","text":"ok"}]}
        ]}),
    )
}

async fn case(fault: SeedFault, preflight: bool, explicit_parent: bool) {
    let server = Arc::new(SeedServer {
        fault,
        // Match real Codex envelopes: the echoed catalog can exceed 64 KiB.
        echo_large_catalog: AtomicBool::new(true),
        ..Default::default()
    });
    let app = Router::new()
        .route("/v1/responses", get(endpoint).post(http))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(server.clone());
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, true, native::Options::default()).await;
    if fault == SeedFault::Silent {
        host.initial_timeout_ms.store(1_000, Ordering::SeqCst);
    }
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer key".parse().unwrap());
    request
        .headers_mut()
        .insert("user-agent", "codex_cli_rs".parse().unwrap());
    let mut client = tokio_tungstenite::connect_async(request).await.unwrap().0;
    let mut preflight_id = None;
    if preflight {
        send(
            &mut client,
            json!({"type":"response.create","model":"m","generate":false,"input":[]}),
        )
        .await;
        let event = terminal(&mut client).await;
        assert_eq!(event["type"], "response.completed");
        preflight_id = Some(event["response"]["id"].clone());
    }
    let input: Vec<Value> = (0..9)
        .map(|n| json!({"role":"user","content":format!("{n}{}","A".repeat(4*1024*1024))}))
        .collect();
    let mut body = json!({"type":"response.create","model":"m","input":input.clone()});
    if explicit_parent {
        body["previous_response_id"] = preflight_id.unwrap();
    }
    send(&mut client, body).await;
    let event = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let message = client.next().await.unwrap().unwrap();
            if let Message::Text(text) = message {
                let event: Value = serde_json::from_str(&text).unwrap();
                if matches!(
                    event["type"].as_str(),
                    Some("response.completed" | "response.failed" | "error")
                ) {
                    break event;
                }
                assert!(
                    !matches!(event["type"].as_str(), Some("response.created"))
                        || !event["response"]["id"]
                            .as_str()
                            .unwrap()
                            .starts_with("warm-")
                );
            }
        }
    })
    .await
    .unwrap();
    reports(&host, if preflight { 2 } else { 1 }).await;
    if matches!(
        fault,
        SeedFault::Output | SeedFault::OutputAfterCompletion | SeedFault::Silent
    ) {
        assert!(matches!(
            event["type"].as_str(),
            Some("response.failed" | "error")
        ));
        let expected_code = if fault == SeedFault::Silent {
            "websocket_context_seed_outcome_unknown"
        } else {
            "websocket_context_seed_output"
        };
        assert_eq!(
            event
                .pointer("/error/code")
                .or_else(|| event.pointer("/response/error/code"))
                .and_then(Value::as_str),
            Some(expected_code),
            "{event}"
        );
        if fault == SeedFault::Silent {
            assert_eq!(
                event.pointer("/error/preload_failure"),
                Some(&json!("response_timeout"))
            );
        }
        assert_eq!(server.http_calls.load(Ordering::SeqCst), 0);
        assert_eq!(server.generated.load(Ordering::SeqCst), 0);
    } else {
        assert_eq!(event["type"], "response.completed");
        assert_eq!(*server.inputs.lock().unwrap(), input);
        if fault == SeedFault::Reject {
            assert_eq!(server.http_calls.load(Ordering::SeqCst), 1);
            assert_eq!(server.generated.load(Ordering::SeqCst), 0);
        } else {
            assert_eq!(server.http_calls.load(Ordering::SeqCst), 0);
            assert!(server.warms.load(Ordering::SeqCst) > 0);
            assert_eq!(server.generated.load(Ordering::SeqCst), 1);
            assert!(server
                .message_sizes
                .lock()
                .unwrap()
                .iter()
                .all(|size| *size <= super::super::context_seed::CHUNK_TARGET_BYTES));
            if explicit_parent {
                assert_eq!(server.connections.load(Ordering::SeqCst), 1);
                send(
                    &mut client,
                    json!({"type":"response.create","model":"m",
                    "previous_response_id":event["response"]["id"],
                    "input":[{"role":"user","content":"small continuation"}]}),
                )
                .await;
                // Restoring a 38 MiB owner-checked checkpoint in an unoptimized
                // build exceeds the small fixtures' shared three-second bound.
                let next = tokio::time::timeout(Duration::from_secs(20), async {
                    loop {
                        match client.next().await.unwrap().unwrap() {
                            Message::Text(text) => {
                                let event: Value = serde_json::from_str(&text).unwrap();
                                if matches!(
                                    event["type"].as_str(),
                                    Some("response.completed" | "error" | "response.failed")
                                ) {
                                    break event;
                                }
                            }
                            Message::Ping(value) => {
                                client.send(Message::Pong(value)).await.unwrap()
                            }
                            _ => {}
                        }
                    }
                })
                .await
                .unwrap();
                assert_eq!(next["type"], "response.completed");
                assert_eq!(server.generated.load(Ordering::SeqCst), 2);
                assert_eq!(server.connections.load(Ordering::SeqCst), 1);
                assert!(*server.message_sizes.lock().unwrap().last().unwrap() < 4096);
            }
        }
    }
    client.close(None).await.ok();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn cold_seed_large_history_primes_same_socket_then_generates_once() {
    case(SeedFault::None, false, false).await;
}
#[tokio::test]
async fn cold_seed_rejected_preload_uses_one_complete_http_request() {
    case(SeedFault::Reject, false, false).await;
}
#[tokio::test]
async fn cold_seed_unexpected_output_never_replays_over_http() {
    case(SeedFault::Output, false, false).await;
}

#[tokio::test]
async fn cold_seed_empty_client_warmup_then_oversized_history_is_segmented() {
    case(SeedFault::None, true, false).await;
}

#[tokio::test]
async fn cold_seed_explicit_prewarm_parent_is_segmented_without_reconnecting() {
    case(SeedFault::None, true, true).await;
}

#[tokio::test]
async fn cold_seed_explicit_parent_rejected_preload_replays_complete_history_once() {
    case(SeedFault::Reject, true, true).await;
}

#[tokio::test]
async fn cold_seed_explicit_parent_unexpected_output_never_replays() {
    case(SeedFault::Output, true, true).await;
}

#[tokio::test]
async fn cold_seed_silent_preload_never_replays_over_http() {
    case(SeedFault::Silent, false, false).await;
}

#[tokio::test]
async fn cold_seed_output_after_completion_in_same_frame_never_generates() {
    case(SeedFault::OutputAfterCompletion, false, false).await;
}

#[tokio::test]
async fn cold_seed_reused_socket_checks_output_after_completion_in_same_frame() {
    case(SeedFault::OutputAfterCompletion, true, true).await;
}

async fn reserve_during_preload(recovers: bool, no_backup: bool, preflight: bool) {
    let server = Arc::new(SeedServer::default());
    let app = Router::new()
        .route("/v1/responses", get(endpoint).post(http))
        .with_state(server.clone());
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, true, native::Options::default()).await;
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer key".parse().unwrap());
    request
        .headers_mut()
        .insert("user-agent", "codex_cli_rs".parse().unwrap());
    let mut client = tokio_tungstenite::connect_async(request).await.unwrap().0;
    let mut parent = None;
    host.quota_gate.lock().unwrap().allow_checks = Some(usize::MAX);
    if preflight {
        send(
            &mut client,
            json!({"type":"response.create","model":"m","generate":false,"input":[]}),
        )
        .await;
        parent = Some(terminal(&mut client).await["response"]["id"].clone());
    }
    *host.quota_gate.lock().unwrap() = SeedQuotaGate {
        // A cold open checks before/after connect, then before its first
        // preload. A reused socket checks once before the preload itself.
        allow_checks: Some(if preflight { 2 } else { 3 }),
        refresh_recovers: recovers,
        no_backup,
        ..Default::default()
    };
    let input: Vec<Value> = (0..9)
        .map(|n| json!({"role":"user","content":format!("marker-{n}{}","A".repeat(4*1024*1024))}))
        .collect();
    let mut body = json!({"type":"response.create","model":"m","input":input});
    if let Some(parent) = parent {
        body["previous_response_id"] = parent;
    }
    send(&mut client, body).await;
    let event = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match client.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if matches!(
                        event["type"].as_str(),
                        Some("response.completed" | "error" | "response.failed")
                    ) {
                        break event;
                    }
                }
                Message::Ping(v) => client.send(Message::Pong(v)).await.unwrap(),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(server.http_calls.load(Ordering::SeqCst), 0);
    if no_backup {
        assert_eq!(event["error"]["code"], "quota_reserved", "{event}");
        assert_eq!(server.generated.load(Ordering::SeqCst), 0);
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    } else {
        assert_eq!(event["type"], "response.completed", "{event}");
        assert_eq!(server.generated.load(Ordering::SeqCst), 1);
        assert_eq!(
            server.connections.load(Ordering::SeqCst),
            if recovers { 1 } else { 2 }
        );
        assert_eq!(
            *server.connection_inputs.lock().unwrap().last().unwrap(),
            input
        );
    }
    assert!(server
        .message_sizes
        .lock()
        .unwrap()
        .iter()
        .all(|n| *n < 32 * 1024 * 1024));
    client.close(None).await.ok();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn cold_seed_stale_quota_confirmation_recovers_without_reconnecting() {
    reserve_during_preload(true, false, false).await;
}
#[tokio::test]
async fn cold_seed_reserve_mid_preload_switches_account_before_generating() {
    reserve_during_preload(false, false, false).await;
}
#[tokio::test]
async fn cold_seed_reserve_mid_append_rebuilds_validated_parent_on_backup() {
    reserve_during_preload(false, false, true).await;
}
#[tokio::test]
async fn cold_seed_reserve_without_backup_preserves_error_and_never_generates() {
    reserve_during_preload(false, true, false).await;
}
