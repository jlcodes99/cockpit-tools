//! Loopback-only coverage: no real accounts, tokens, or provider requests.
use super::{contract::*, start, Options};
use axum::{
    body::Body,
    extract::{ws::Message, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};

struct HostFixture {
    base: String,
}
impl Host for HostFixture {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "fixture-caller".into(),
                label: "fixture".into(),
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
                account_id: "grok-fixture".into(),
                owner_id: "grok-owner".into(),
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
                headers: HeaderMap::from_iter([(
                    "x-xai-token-auth".parse().unwrap(),
                    HeaderValue::from_static("xai-grok-cli"),
                )]),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: false,
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
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(true) })
    }
}
type Requests = Arc<Mutex<Vec<Value>>>;
fn response_events(body: &Value) -> Vec<Value> {
    let tool = &body["tools"][0];
    let item = json!({"type":"function_call","id":"fc_search_fixture","call_id":"call_fixture","name":tool["name"],"arguments":"{\"goal\":\"find tools\"}","status":"completed"});
    vec![
        json!({"type":"response.created","response":{"id":"resp_fixture","status":"in_progress"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_search_fixture","call_id":"call_fixture","name":tool["name"],"arguments":""}}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc_search_fixture","delta":"{\"goal\":\"find tools\"}"}),
        json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"fc_search_fixture","arguments":"{\"goal\":\"find tools\"}"}),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
        json!({"type":"response.completed","response":{"id":"resp_fixture","status":"completed","output":[item]}}),
    ]
}
async fn http(State(requests): State<Requests>, Json(body): Json<Value>) -> Response {
    requests.lock().unwrap().push(body.clone());
    let events = response_events(&body);
    if body["stream"] == true {
        let text = events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(text))
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
            for event in response_events(&body) {
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
fn request() -> Value {
    json!({"model":"grok-4.7","input":[{"role":"user","content":"find tools"}],"tools":[{"type":"tool_search","execution":"client","parameters":{"type":"object","properties":{"goal":{"type":"string"}},"required":["goal"]}}]})
}
fn assert_item(item: &Value) {
    assert_eq!(item["type"], "tool_search_call");
    assert_eq!(item["execution"], "client");
    assert_eq!(item["call_id"], "call_fixture");
    assert_eq!(item["id"], "tsc_search_fixture");
    assert_eq!(item["arguments"], json!({"goal":"find tools"}));
}
#[tokio::test]
async fn grok_client_search_passes_http_json_sse_and_websocket() {
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
    assert_item(&response.json::<Value>().await.unwrap()["output"][0]);
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
    let text = response.text().await.unwrap();
    assert!(!text.contains("response.function_call_arguments"));
    let terminal = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["type"] == "response.completed")
        .unwrap();
    assert_item(&terminal["response"]["output"][0]);
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
    let receive = async {
        let mut completed = false;
        while let Some(Ok(ClientMessage::Text(text))) = socket.next().await {
            let event: Value = serde_json::from_str(&text).unwrap();
            assert_ne!(event["type"], "error", "{event}");
            assert_ne!(event["type"], "response.function_call_arguments.delta");
            if event["type"] == "response.completed" {
                assert_item(&event["response"]["output"][0]);
                completed = true;
                break;
            }
        }
        assert!(completed, "WebSocket closed without a completed response");
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), receive)
        .await
        .unwrap();
    let captured = requests.lock().unwrap().clone();
    assert_eq!(captured.len(), 3);
    for body in captured {
        assert_eq!(body["tools"][0]["type"], "function");
        assert!(body["tools"][0].get("execution").is_none());
        assert_eq!(
            body["tools"][0]["parameters"],
            request()["tools"][0]["parameters"]
        );
    }
    socket.close(None).await.unwrap();
    handle
        .shutdown(std::time::Duration::from_secs(5))
        .await
        .unwrap();
    task.abort();
}

#[path = "grok_wire_http_tests.rs"]
mod wire_compat_tests;

#[path = "grok_dispatcher_http_tests.rs"]
mod dispatcher_tests;

#[path = "grok_continuation_http_tests.rs"]
mod continuation_tests;
