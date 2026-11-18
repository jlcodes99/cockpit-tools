use super::*;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Fixture {
    fail_selection: bool,
    fail_cleanup: bool,
    stream_replies: bool,
    event_stream: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<String>>>>,
    events: tokio::sync::mpsc::UnboundedSender<String>,
    calls: Arc<Mutex<Vec<String>>>,
}

async fn server(
    fail_selection: bool,
    fail_cleanup: bool,
    stream_replies: bool,
) -> (String, Fixture, tokio::task::JoinHandle<()>) {
    let (events, event_stream) = tokio::sync::mpsc::unbounded_channel();
    let state = Fixture {
        fail_selection,
        fail_cleanup,
        stream_replies,
        event_stream: Arc::new(Mutex::new(Some(event_stream))),
        events,
        calls: Default::default(),
    };
    let app = Router::new()
        .route("/console/as/conversations/", post(|State(state): State<Fixture>, Json(body): Json<Value>| async move {
            state.calls.lock().unwrap().push("create".into());
            assert_eq!(body["model"], "chosen-model");
            assert_eq!(body["plugins"], json!([]));
            Json(json!({"code":0,"data":{"id":"test-conversation"}}))
        }))
        .route("/console/as/conversations/test-conversation/session", get(|headers: HeaderMap| async move {
            Json(json!({"data":{"link":format!("http://{}/acp",headers["host"].to_str().unwrap()),"token":"synthetic-session-token","sessionId":"session"}}))
        }))
        .route("/console/as/conversations/test-conversation/delete", post(|State(state): State<Fixture>, Json(body): Json<Value>| async move {
            assert_eq!(body["id"], "test-conversation");
            state.calls.lock().unwrap().push("delete".into());
            if state.fail_cleanup { (StatusCode::SERVICE_UNAVAILABLE, "failure") }
            else { (StatusCode::NO_CONTENT, "") }
        }))
        .route("/acp", get(|State(state): State<Fixture>| async move {
            let receiver = state.event_stream.lock().unwrap().take().unwrap();
            let events = futures_util::stream::unfold(receiver, |mut receiver| async {
                let event = receiver.recv().await?;
                Some((Ok::<_, std::io::Error>(event), receiver))
            });
            axum::response::Response::builder().header("content-type", "text/event-stream")
                .body(axum::body::Body::from_stream(events)).unwrap()
        }).post(|State(state): State<Fixture>, Json(body): Json<Value>| async move {
            let method = body["method"].as_str().unwrap();
            state.calls.lock().unwrap().push(method.into());
            let ack = if method == "session/set_model" && state.fail_selection {
                json!({"jsonrpc":"2.0","id":body["id"],"error":{"message":"selected model rejected"}})
            } else {
                json!({"jsonrpc":"2.0","id":body["id"],"result":{}})
            };
            if method == "session/set_model" {
                assert_eq!(body["params"]["modelId"], "chosen-model");
            }
            let event = |value: &Value| format!("data: {value}\n\n");
            if method == "session/prompt" {
                let update = json!({"params":{"sessionId":"session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"OK"}}}});
                if state.stream_replies {
                    state.events.send(event(&ack)).unwrap();
                    let events = state.events.clone();
                    let update = event(&update);
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                        let _ = events.send(update);
                    });
                    return StatusCode::OK.into_response();
                }
                // Direct POST replies must finish even when the GET channel is idle.
                return ([("content-type","text/event-stream")], event(&update)).into_response();
            }
            if state.stream_replies {
                state.events.send(event(&ack)).unwrap();
                return StatusCode::OK.into_response();
            }
            Json(ack).into_response()
        }))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, state, task)
}

fn account() -> crate::models::workbuddy::WorkbuddyAccount {
    serde_json::from_value(json!({"id":"fixture","email":"synthetic@example.invalid",
        "access_token":"synthetic-access-token","uid":"synthetic-user","created_at":0,"last_used":0})).unwrap()
}

#[tokio::test]
async fn direct_prompt_reply_requires_model_selection_and_reports_cleanup_failure() {
    let (base, fixture, task) = server(false, true, false).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let account = account();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        probe_with_client(&client, &base, &ProbeAccount::from_workbuddy(&account), "chosen-model", false),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.status, "available");
    assert_eq!(result.reply, "OK");
    assert!(!result.conversation_deleted);
    assert!(result.cleanup_error.is_some());
    assert_eq!(
        *fixture.calls.lock().unwrap(),
        [
            "create",
            "initialize",
            "session/load",
            "session/set_model",
            "session/prompt",
            "delete"
        ]
    );
    task.abort();
}

#[tokio::test]
async fn failed_model_selection_never_sends_prompt_and_still_deletes_the_conversation() {
    let (base, fixture, task) = server(true, false, false).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let account = account();
    let result = probe_with_client(&client, &base, &ProbeAccount::from_workbuddy(&account), "chosen-model", false)
        .await
        .unwrap();
    assert_eq!(result.status, "error");
    assert!(result.reply.is_empty());
    assert!(result.conversation_deleted);
    assert_eq!(
        *fixture.calls.lock().unwrap(),
        [
            "create",
            "initialize",
            "session/load",
            "session/set_model",
            "delete"
        ]
    );
    task.abort();
}

#[tokio::test]
async fn empty_handshake_allows_prompt_and_keeps_the_conversation_when_requested() {
    let (base, fixture, task) = server(false, false, true).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let account = account();
    let result = probe_with_client(&client, &base, &ProbeAccount::from_workbuddy(&account), "chosen-model", true)
        .await
        .unwrap();
    assert_eq!(result.status, "available");
    assert!(!result.conversation_deleted);
    assert!(!fixture
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|method| method == "delete"));
    task.abort();
}

#[tokio::test]
async fn empty_posts_use_stream_acknowledgments_and_return_the_model_reply() {
    let (base, fixture, task) = server(false, false, true).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let account = account();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        probe_with_client(&client, &base, &ProbeAccount::from_workbuddy(&account), "chosen-model", false),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.status, "available", "{}", result.message);
    assert_eq!(result.reply, "OK");
    assert!(result.conversation_deleted);
    assert_eq!(
        *fixture.calls.lock().unwrap(),
        ["create", "initialize", "session/load", "session/set_model", "session/prompt", "delete"]
    );
    task.abort();
}
