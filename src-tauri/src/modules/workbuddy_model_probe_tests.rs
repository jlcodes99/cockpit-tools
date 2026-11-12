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
    calls: Arc<Mutex<Vec<String>>>,
}

async fn server(
    fail_selection: bool,
    fail_cleanup: bool,
) -> (String, Fixture, tokio::task::JoinHandle<()>) {
    let state = Fixture {
        fail_selection,
        fail_cleanup,
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
        .route("/console/as/conversations/test-conversation", axum::routing::delete(|State(state): State<Fixture>| async move {
            state.calls.lock().unwrap().push("delete".into());
            if state.fail_cleanup { (StatusCode::SERVICE_UNAVAILABLE, "failure") }
            else { (StatusCode::NO_CONTENT, "") }
        }))
        .route("/acp", get(|| async {
            // The GET channel remains idle. A prompt POST with a direct reply must
            // complete without waiting for another event on that channel.
            axum::response::Response::builder().header("content-type", "text/event-stream")
                .body(axum::body::Body::from_stream(futures_util::stream::pending::<Result<axum::body::Bytes, std::io::Error>>())).unwrap()
        }).post(|State(state): State<Fixture>, Json(body): Json<Value>| async move {
            let method = body["method"].as_str().unwrap();
            state.calls.lock().unwrap().push(method.into());
            if method == "session/set_model" {
                assert_eq!(body["params"]["modelId"], "chosen-model");
                if state.fail_selection {
                    return Json(json!({"id":body["id"],"error":{"message":"selected model rejected"}})).into_response();
                }
            }
            if method == "session/prompt" {
                let update = json!({"params":{"sessionId":"session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"OK"}}}});
                return ([("content-type","text/event-stream")],format!("data: {update}\n\n").replace("\n","
")).into_response();
            }
            Json(json!({"jsonrpc":"2.0","id":body["id"],"result":{}})).into_response()
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
    let (base, fixture, task) = server(false, true).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        probe_with_client(&client, &base, &account(), "chosen-model", false),
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
    let (base, fixture, task) = server(true, false).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let result = probe_with_client(&client, &base, &account(), "chosen-model", false)
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
async fn explicitly_kept_conversation_is_not_deleted() {
    let (base, fixture, task) = server(false, false).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let result = probe_with_client(&client, &base, &account(), "chosen-model", true)
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
