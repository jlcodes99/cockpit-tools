use crate::modules::codex_native_api::{self as native, contract::*, Options};
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::HeaderMap,
    response::Response,
    routing::post,
    Router,
};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct FormatHost {
    url: String,
    reports: Mutex<Vec<RequestReport>>,
    selections: Mutex<Vec<RouteRequest>>,
}

impl Host for FormatHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "format-caller".into(),
                label: "test".into(),
            })
        })
    }

    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let account = request.target_account_id.as_deref().unwrap_or("account-a");
            let route = Route {
                continuity_account_id: None,
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                account_id: account.into(),
                owner_id: format!("owner-{account}"),
                base_url: self.url.clone(),
                model: request.model.clone(),
                headers: HeaderMap::new(),
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
            };
            self.selections.lock().unwrap().push(request);
            Ok(route)
        })
    }

    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({})) })
    }

    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}

struct Source {
    content_type: Option<&'static str>,
    payload: String,
    requests: Mutex<Vec<(HeaderMap, Value)>>,
}

async fn source_response(
    State(source): State<Arc<Source>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    source
        .requests
        .lock()
        .unwrap()
        .push((headers, serde_json::from_slice(&body).unwrap()));
    // Deliberately split the SSE field prefix across source chunks.
    let split = source.payload.len().min(2);
    let parts = vec![
        Bytes::copy_from_slice(&source.payload.as_bytes()[..split]),
        Bytes::copy_from_slice(&source.payload.as_bytes()[split..]),
    ];
    let stream = futures_util::stream::iter(parts.into_iter().map(Ok::<_, std::io::Error>));
    let mut response = Response::builder();
    if let Some(content_type) = source.content_type {
        response = response.header("content-type", content_type);
    }
    response.body(Body::from_stream(stream)).unwrap()
}

async fn fixture(
    content_type: Option<&'static str>,
    payload: String,
) -> (Arc<FormatHost>, Arc<Source>, tokio::task::JoinHandle<()>) {
    let source = Arc::new(Source {
        content_type,
        payload,
        requests: Mutex::new(Vec::new()),
    });
    let app = Router::new()
        .route("/v1/responses", post(source_response))
        .route("/v1/responses/compact", post(source_response))
        .with_state(source.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let host = Arc::new(FormatHost {
        url,
        reports: Mutex::new(Vec::new()),
        selections: Mutex::new(Vec::new()),
    });
    (host, source, task)
}

fn response_events() -> String {
    let events = [
        json!({"type":"response.created","response":{"id":"format-response","status":"in_progress"}}),
        json!({"type":"response.output_item.done","item":{"type":"reasoning","encrypted_content":"format-cipher"}}),
        json!({"type":"response.completed","response":{"id":"format-response","status":"completed","output":[{"type":"reasoning","encrypted_content":"format-cipher"}],"usage":{"input_tokens":2,"output_tokens":3,"input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}}}}),
    ];
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}

fn request() -> Value {
    json!({
        "model":"format-model", "stream":true,
        "client_metadata":{"session_id":"format-thread","thread_id":"format-thread"},
        "input":[{"role":"user","content":"hello"}]
    })
}

#[tokio::test]
async fn mislabeled_sse_records_cipher_and_continues_after_restart_without_crossing_owners() {
    for content_type in [
        None,
        Some("application/json"),
        Some("application/octet-stream"),
        Some("text/plain"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.sqlite");
        let (host, source, task) = fixture(content_type, response_events()).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut service = native::start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            path.clone(),
            Options::default(),
        )
        .await
        .unwrap();
        let response = client
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("key")
            .json(&request())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        assert_eq!(response.text().await.unwrap(), response_events());
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            for kind in ["response", "parent_response", "encrypted"] {
                let count: i64 = connection
                    .query_row(
                        "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind=?1",
                        [kind],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(count, 1, "{content_type:?}: {kind}");
            }
            let reports = host.reports.lock().unwrap();
            assert!(reports[0].success);
            assert_eq!(reports[0].usage.as_ref().unwrap()["input_tokens"], 2);
        }
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        drop(service);
        let mut service = native::start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            path,
            Options::default(),
        )
        .await
        .unwrap();
        let mut continuation = request();
        continuation["previous_response_id"] = json!("format-response");
        continuation["input"] =
            json!([{ "type":"reasoning", "encrypted_content":"format-cipher" }]);
        let response = client
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("key")
            .json(&continuation)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
        {
            let selections = host.selections.lock().unwrap();
            assert_eq!(
                selections[1].preferred_account_id.as_deref(),
                Some("account-a")
            );
            let requests = source.requests.lock().unwrap();
            assert_eq!(requests[0].0["accept"], "text/event-stream");
            assert_eq!(requests[0].0["session-id"], requests[1].0["session-id"]);
            assert_eq!(requests[0].0["thread-id"], requests[1].0["thread-id"]);
            assert_eq!(requests[1].1["input"], continuation["input"]);
        }
        let response = client
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("key")
            .header("x-cockpit-target-account-id", "account-b")
            .json(&continuation)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
        assert_eq!(source.requests.lock().unwrap().len(), 2);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn json_response_keeps_json_and_invalid_success_body_is_not_counted_as_success() {
    for (payload, expected) in [
        (json!({"id":"json-response","status":"completed","output":[],"usage":{"input_tokens":1}}).to_string(), 200),
        ("<html>upstream error</html>".to_string(), 502),
        ("{}".to_string(), 502),
        (json!({"output":[{"type":"reasoning","encrypted_content":"unowned"}]}).to_string(), 502),
        (json!({"id":"","output":[]}).to_string(), 502),
        (json!({"response":{"id":"wrapped-response","status":"completed","output":[{"type":"reasoning","encrypted_content":"unowned"}]}}).to_string(), 502),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (host, _, task) = fixture(Some("application/json"), payload.clone()).await;
        let mut service = native::start(
            "127.0.0.1:0".parse().unwrap(), host.clone(), directory.path().join("identity.sqlite"), Options::default(),
        ).await.unwrap();
        let response = reqwest::Client::builder().no_proxy().build().unwrap()
            .post(format!("http://{}/v1/responses", service.addr)).bearer_auth("key")
            .json(&request()).send().await.unwrap();
        assert_eq!(response.status(), expected);
        let text = response.text().await.unwrap();
        if expected == 200 {
            let mut expected_body:Value = serde_json::from_str(&payload).unwrap();
            expected_body["usage"]["input_tokens_details"] = json!({"cached_tokens":0});
            expected_body["usage"]["output_tokens_details"] = json!({"reasoning_tokens":0});
            assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected_body);
        } else {
            assert!(text.contains("invalid_upstream_response"));
        }
        assert_eq!(host.reports.lock().unwrap()[0].success, expected == 200);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn unknown_response_prefix_is_bounded_before_relay() {
    let directory = tempfile::tempdir().unwrap();
    let (host, _, task) = fixture(None, format!("{}{}", " ".repeat(4096), response_events())).await;
    let mut service = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        directory.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("key")
        .json(&request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("unknown_upstream_format"));
    assert!(!host.reports.lock().unwrap()[0].success);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn nonstream_mislabeled_sse_collects_json_and_records_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.sqlite");
    let (host, source, task) = fixture(Some("application/json"), response_events()).await;
    let mut service = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        path.clone(),
        Options::default(),
    )
    .await
    .unwrap();
    let mut body = request();
    body["stream"] = json!(false);
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("key")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/json");
    let value: Value = response.json().await.unwrap();
    assert_eq!(value["id"], "format-response");
    assert_eq!(value["status"], "completed");
    assert_eq!(value["usage"]["input_tokens"], 2);
    {
        let requests = source.requests.lock().unwrap();
        assert_eq!(requests[0].0["accept"], "application/json");
        assert_eq!(requests[0].1["stream"], false);
        let connection = rusqlite::Connection::open(&path).unwrap();
        for kind in ["response", "parent_response", "encrypted"] {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind=?1",
                    [kind],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{kind}");
        }
        let reports = host.reports.lock().unwrap();
        assert!(reports[0].success);
        assert_eq!(reports[0].usage.as_ref().unwrap()["output_tokens"], 3);
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn compact_json_preserves_cipher_and_accepts_no_status_field() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.sqlite");
    let compact = json!({
        "id":"compact-format-response", "object":"response.compaction",
        "output":[{"type":"compaction","encrypted_content":"compact-format-cipher"}],
        "usage":{"input_tokens":5,"output_tokens":1}
    });
    let (host, source, task) = fixture(Some("application/json"), compact.to_string()).await;
    let mut service = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        path.clone(),
        Options::default(),
    )
    .await
    .unwrap();
    let mut body = request();
    body.as_object_mut().unwrap().remove("stream");
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses/compact", service.addr))
        .bearer_auth("key")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.json::<Value>().await.unwrap(), compact);
    {
        let requests = source.requests.lock().unwrap();
        assert_eq!(requests[0].0["accept"], "application/json");
        assert!(requests[0].1.get("stream").is_none());
        let reports = host.reports.lock().unwrap();
        assert!(reports[0].success);
        assert_eq!(reports[0].usage.as_ref().unwrap()["input_tokens"], 5);
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    drop(service);
    let store = native::identity::IdentityStore::open(path).unwrap();
    body["input"] = compact["output"].clone();
    let identity = store
        .inspect("format-caller", &HeaderMap::new(), &body)
        .unwrap();
    assert_eq!(identity.state_account_id.as_deref(), Some("account-a"));
    task.abort();
}

include!("sse_http_tests.rs");
