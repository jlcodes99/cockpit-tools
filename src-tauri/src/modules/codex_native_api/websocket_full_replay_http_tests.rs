use super::*;
use axum::extract::ws::{Message as UpMessage, WebSocketUpgrade};
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct ReplayServer {
    requests: Mutex<Vec<Value>>,
    connections: AtomicUsize,
    reject_delta: AtomicBool,
}

fn call() -> Value {
    json!({"type":"function_call","id":"tool-item","call_id":"tool-call",
        "namespace":"collaboration","name":"spawn_agent","arguments":"{\"message\":\"synthetic\"}",
        "encrypted_function_args":[]})
}

async fn endpoint(State(state): State<Arc<ReplayServer>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |mut socket| async move {
        state.connections.fetch_add(1, Ordering::SeqCst);
        while let Some(Ok(UpMessage::Text(text))) = socket.recv().await {
            let body: Value = serde_json::from_str(&text).unwrap();
            let sequence = {
                let mut requests = state.requests.lock().unwrap();
                requests.push(body.clone());
                requests.len()
            };
            if body.get("previous_response_id").is_some()
                && state.reject_delta.swap(false, Ordering::SeqCst)
            {
                let event = json!({"type":"error","status":400,"error":{"code":"previous_response_not_found","message":"Invalid previous_response_id"}});
                let _ = socket.send(UpMessage::Text(event.to_string().into())).await;
                continue;
            }
            let id = if sequence == 1 { "anchor" } else { "next" };
            let output = if sequence == 1 { vec![call()] } else {
                vec![json!({"type":"message","id":"answer","role":"assistant","content":[{"type":"output_text","text":"ok"}]})]
            };
            let events = [
                json!({"type":"response.created","response":{"id":id,"status":"in_progress","output":[]}}),
                json!({"type":"response.output_item.done","output_index":0,"item":output[0]}),
                json!({"type":"response.completed","response":{"id":id,"status":"completed","output":output}}),
            ];
            for event in events {
                if socket.send(UpMessage::Text(event.to_string().into())).await.is_err() { return; }
            }
        }
    })
}

async fn replay_case(reject_delta: bool, codex_client: bool) {
    let state = Arc::new(ReplayServer::default());
    state.reject_delta.store(reject_delta, Ordering::SeqCst);
    let app = Router::new()
        .route("/v1/responses", get(endpoint))
        .with_state(state.clone());
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, true, native::Options::default()).await;
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer key".parse().unwrap());
    if codex_client {
        request
            .headers_mut()
            .insert("user-agent", "codex_cli_rs".parse().unwrap());
    }
    let mut client = tokio_tungstenite::connect_async(request).await.unwrap().0;
    let original = json!({"role":"user","content":[
        {"type":"input_text","text":"context"},
        {"type":"input_image","image_url":format!("data:image/png;base64,{}", "A".repeat(64 * 1024))}
    ]});
    send(
        &mut client,
        json!({"type":"response.create","model":"model","input":[original.clone()]}),
    )
    .await;
    let first = terminal(&mut client).await;
    assert_eq!(
        first["response"]["output"][0]["encrypted_function_args"],
        json!([])
    );
    let mut replay_call = call();
    replay_call
        .as_object_mut()
        .unwrap()
        .remove("encrypted_function_args");
    let output = json!({"type":"function_call_output","call_id":"tool-call","output":"new synthetic result"});
    let full = json!([original, replay_call, output.clone()]);
    send(
        &mut client,
        json!({"type":"response.create","model":"model","input":full.clone()}),
    )
    .await;
    assert_eq!(terminal(&mut client).await["type"], "response.completed");
    reports(&host, 2).await;
    let requests = state.requests.lock().unwrap().clone();
    if codex_client {
        assert_eq!(requests[1]["previous_response_id"], "anchor");
        assert_eq!(requests[1]["input"], json!([output]));
        assert!(requests[1].to_string().len() < 1024);
        if reject_delta {
            assert_eq!(requests.len(), 3);
            assert!(requests[2].get("previous_response_id").is_none());
            assert_eq!(requests[2]["input"], full);
            assert_eq!(state.connections.load(Ordering::SeqCst), 2);
        } else {
            assert_eq!(requests.len(), 2);
            assert_eq!(state.connections.load(Ordering::SeqCst), 1);
        }
    } else {
        assert_eq!(requests[1]["input"], full);
        assert!(requests[1].get("previous_response_id").is_none());
    }
    client.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn full_replay_codex_marker_mismatch_reuses_socket_without_resending_images() {
    replay_case(false, true).await;
}

#[tokio::test]
async fn full_replay_expired_anchor_retries_original_full_input_once() {
    replay_case(true, true).await;
}

#[tokio::test]
async fn full_replay_non_codex_client_keeps_explicit_full_request() {
    replay_case(false, false).await;
}
