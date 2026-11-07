use super::*;
use crate::modules::codex_native_api::{self as native, contract::*};
use axum::{extract::State, routing::post, Json, Router};
use futures_util::{future::BoxFuture, SinkExt};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

struct PreparationHost {
    base: String,
    policy: ExecutionPolicy,
    delays: Vec<Duration>,
    body_delay: Duration,
    denied: bool,
    oauth: bool,
    selections: Mutex<Vec<RouteRequest>>,
    reports: Mutex<Vec<RequestReport>>,
}
impl Host for PreparationHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "prepare-caller".into(),
                label: "test".into(),
            })
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let delay = {
                let mut selections = self.selections.lock().unwrap();
                let delay = self
                    .delays
                    .get(selections.len())
                    .copied()
                    .unwrap_or(Duration::ZERO);
                selections.push(request.clone());
                delay
            };
            tokio::time::sleep(delay).await;
            if self.denied {
                return Err(ServiceError::new(
                    503,
                    "no_available_account",
                    "selected account is cooling down",
                )
                .with_retry_after(Some(Duration::from_secs(7))));
            }
            Ok(Route {
                continuity_account_id: None,
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                account_id: "a".into(),
                owner_id: "owner-a".into(),
                base_url: self.base.clone(),
                model: request.model,
                headers: HeaderMap::new(),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: self.oauth,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn prepare_body<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
        _: &'a HeaderMap,
        body: Value,
        _: bool,
        _: bool,
    ) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async move {
            tokio::time::sleep(self.body_delay).await;
            Ok(body)
        })
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(false) })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn image_route<'a>(
        &'a self,
        caller: &'a Caller,
        request: ImageRouteRequest,
    ) -> BoxFuture<'a, Result<ImageRoute, ServiceError>> {
        Box::pin(async move {
            let route = self
                .route(
                    caller,
                    RouteRequest {
                        model: request.model.unwrap_or_else(|| "image-model".into()),
                        session_id: None,
                        thread_id: None,
                        parent_thread_id: None,
                        preferred_account_id: request.preferred_account_id,
                        soft_preferred_account_id: None,
                        target_account_id: request.target_account_id,
                        force_refresh: request.force_refresh,
                        image_generation_requested: true,
                        image_input: false,
                        excluded_account_ids: request.excluded_account_ids,
                        requires_permit: true,
                        required_capabilities: Vec::new(),
                        capability_fallback_from: None,
                        auxiliary: Some("images.generations".into()),
                    },
                )
                .await?;
            Ok(ImageRoute {
                image_model: route.model.clone(),
                route,
                main_model: None,
            })
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}
struct PreparationSource {
    requests: AtomicUsize,
    first_delay: Duration,
    status: u16,
    error_body: Option<&'static str>,
    retry_after: Option<&'static str>,
}
async fn preparation_source(
    State(source): State<Arc<PreparationSource>>,
    uri: axum::http::Uri,
    Json(body): Json<Value>,
) -> Response {
    let index = source.requests.fetch_add(1, Ordering::SeqCst);
    if index == 0 {
        tokio::time::sleep(source.first_delay).await;
    }
    let status = if source.status == 401 && index > 0 {
        200
    } else {
        source.status
    };
    if status != 200 {
        let mut response = Response::builder().status(status);
        if let Some(retry_after) = source.retry_after {
            response = response.header("retry-after", retry_after);
        }
        return response
            .body(Body::from(source.error_body.unwrap_or("rejected")))
            .unwrap();
    }
    if uri.path().contains("/images/") {
        return Json(json!({"created":1,"data":[{"b64_json":"aW1hZ2U="}]})).into_response();
    }
    let value = json!({"id":"prepared-response","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":2}});
    if body["stream"] == true {
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!(
                "event: response.completed\ndata: {}\n\n",
                json!({"type":"response.completed","response":value})
            )))
            .unwrap()
    } else {
        Json(value).into_response()
    }
}
async fn preparation_fixture(
    policy: ExecutionPolicy,
    delays: Vec<Duration>,
    body_delay: Duration,
    denied: bool,
    source: PreparationSource,
) -> (
    native::Handle,
    Arc<PreparationHost>,
    Arc<PreparationSource>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    preparation_fixture_with_oauth(policy, delays, body_delay, denied, source, false).await
}
async fn preparation_fixture_with_oauth(
    policy: ExecutionPolicy,
    delays: Vec<Duration>,
    body_delay: Duration,
    denied: bool,
    source: PreparationSource,
    oauth: bool,
) -> (
    native::Handle,
    Arc<PreparationHost>,
    Arc<PreparationSource>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let source = Arc::new(source);
    let app = Router::new()
        .route("/v1/responses", post(preparation_source))
        .route("/v1/images/generations", post(preparation_source))
        .with_state(source.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = Arc::new(PreparationHost {
        base,
        policy,
        delays,
        body_delay,
        denied,
        oauth,
        selections: Mutex::new(vec![]),
        reports: Mutex::new(vec![]),
    });
    let dir = tempfile::tempdir().unwrap();
    let handle = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        dir.path().join("identity.sqlite"),
        native::Options {
            request_timeout: Duration::from_millis(100),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    (handle, host, source, task, dir)
}
fn preparation_policy() -> ExecutionPolicy {
    ExecutionPolicy {
        total_timeout: Duration::from_secs(2),
        open_timeout: Duration::from_secs(1),
        retry_max_delay: Duration::from_secs(1),
        ..Default::default()
    }
}
fn preparation_ok() -> PreparationSource {
    PreparationSource {
        requests: AtomicUsize::new(0),
        first_delay: Duration::ZERO,
        status: 200,
        error_body: None,
        retry_after: None,
    }
}
async fn preparation_request(handle: &native::Handle, stream: bool) -> reqwest::Response {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","input":[],"stream":stream}))
        .send()
        .await
        .unwrap()
}
async fn preparation_reports(host: &PreparationHost, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while host.reports.lock().unwrap().len() < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn account_and_body_preparation_can_outlast_body_read_cap_within_execution_budget() {
    let (mut handle, host, source, task, _dir) = preparation_fixture(
        preparation_policy(),
        vec![Duration::from_millis(250)],
        Duration::from_millis(150),
        false,
        preparation_ok(),
    )
    .await;
    let response = preparation_request(&handle, false).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["id"],
        "prepared-response"
    );
    preparation_reports(&host, 1).await;
    assert_eq!(source.requests.load(Ordering::SeqCst), 1);
    assert!(host.reports.lock().unwrap()[0].success);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn total_and_open_deadlines_cancel_initial_selection_before_any_upstream_attempt() {
    for total_limit in [true, false] {
        let mut policy = preparation_policy();
        if total_limit {
            policy.total_timeout = Duration::from_millis(200);
        } else {
            policy.open_timeout = Duration::from_millis(200);
        }
        let (mut handle, host, source, task, _dir) = preparation_fixture(
            policy,
            vec![Duration::from_millis(700)],
            Duration::ZERO,
            false,
            preparation_ok(),
        )
        .await;
        let started = tokio::time::Instant::now();
        let response = preparation_request(&handle, false).await;
        assert_eq!(response.status(), 504);
        let value = response.json::<Value>().await.unwrap();
        assert_eq!(
            value["error"]["code"],
            if total_limit {
                "upstream_total_timeout"
            } else {
                "upstream_connect_timeout"
            }
        );
        assert!(started.elapsed() < Duration::from_millis(600));
        assert_eq!(source.requests.load(Ordering::SeqCst), 0);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        assert!(host.reports.lock().unwrap().is_empty(),
            "selection/preparation failures must not create upstream request records");
        task.abort();
    }
}

#[tokio::test]
async fn selection_and_policy_preparation_share_one_total_deadline() {
    let policy = ExecutionPolicy {
        total_timeout: Duration::from_millis(300),
        ..preparation_policy()
    };
    let (mut handle, host, source, task, _dir) = preparation_fixture(
        policy,
        vec![Duration::from_millis(200)],
        Duration::from_millis(200),
        false,
        preparation_ok(),
    )
    .await;
    let response = preparation_request(&handle, false).await;
    assert_eq!(response.status(), 504);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "upstream_total_timeout"
    );
    assert_eq!(source.requests.load(Ordering::SeqCst), 0);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert!(host.reports.lock().unwrap().is_empty(),
        "selection/preparation failures must not create upstream request records");
    task.abort();
}

#[tokio::test]
async fn immediate_sse_headers_precede_selection_and_disabled_mode_keeps_real_status() {
    for immediate in [true, false] {
        let policy = ExecutionPolicy {
            immediate_sse: immediate,
            keepalive: Duration::from_millis(50),
            ..preparation_policy()
        };
        let (mut handle, host, source, task, _dir) = preparation_fixture(
            policy,
            vec![Duration::from_millis(400)],
            Duration::ZERO,
            true,
            preparation_ok(),
        )
        .await;
        let started = tokio::time::Instant::now();
        let response = preparation_request(&handle, true).await;
        if immediate {
            assert_eq!(response.status(), 200);
            assert!(started.elapsed() < Duration::from_millis(300));
            let mut stream = response.bytes_stream();
            let first = tokio::time::timeout(Duration::from_millis(150), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(String::from_utf8_lossy(&first).contains(": keep-alive"));
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                bytes.extend_from_slice(&chunk.unwrap());
            }
            let text = String::from_utf8(bytes).unwrap();
            assert!(text.contains("no_available_account"));
            assert!(text.contains("selected account is cooling down"));
            assert!(text.contains("\"retry_after\":7"));
        } else {
            assert_eq!(response.status(), 503);
            assert_eq!(response.headers()["retry-after"], "7");
            assert_eq!(
                response.json::<Value>().await.unwrap()["error"]["code"],
                "no_available_account"
            );
        }
        assert_eq!(source.requests.load(Ordering::SeqCst), 0);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        assert!(host.reports.lock().unwrap().is_empty(),
            "selection/preparation failures must not create upstream request records");
        task.abort();
    }
}

#[tokio::test]
async fn immediate_sse_preserves_actual_upstream_json_plaintext_and_retry_after_errors() {
    for (status,body,code,message,retry_after) in [
        (503,"{\"error\":{\"type\":\"server_error\",\"code\":\"source_unavailable\",\"message\":\"the selected source is unavailable\"}}","source_unavailable","the selected source is unavailable",None),
        (503,"proxy connection refused","upstream_http_error","proxy connection refused",None),
        (429,"{\"error\":{\"code\":\"rate_limit_exceeded\",\"message\":\"retry after the quota window\"}}","rate_limit_exceeded","retry after the quota window",Some("17")),
    ] {
        let policy=ExecutionPolicy{immediate_sse:true,..preparation_policy()};
        let source=PreparationSource{status,error_body:Some(body),retry_after,..preparation_ok()};
        let (mut handle,host,source,task,_dir)=preparation_fixture(policy,vec![],Duration::ZERO,false,source).await;
        let response=preparation_request(&handle,true).await;
        assert_eq!(response.status(),200);
        let text=response.text().await.unwrap();
        let mut decoder=SseDecoder::new(64*1024);
        let events=decoder.push(text.as_bytes()).unwrap();
        let event=events.last().unwrap();
        assert_eq!(event["status"],status);
        assert_eq!(event["error"]["code"],code);
        assert_eq!(event["error"]["message"],message);
        if retry_after.is_some() {assert_eq!(event["retry_after"],17.0);assert_eq!(event["headers"]["retry-after"],"17");}
        preparation_reports(&host,1).await;
        assert_eq!(host.reports.lock().unwrap()[0].status,status);
        assert_eq!(source.requests.load(Ordering::SeqCst),1);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();task.abort();
    }
}

#[tokio::test]
async fn credential_refresh_gets_a_new_open_budget_without_resetting_total() {
    let policy = ExecutionPolicy {
        open_timeout: Duration::from_millis(400),
        ..preparation_policy()
    };
    let source = PreparationSource {
        status: 401,
        first_delay: Duration::from_millis(300),
        ..preparation_ok()
    };
    let (mut handle, host, source, task, _dir) = preparation_fixture_with_oauth(
        policy,
        vec![Duration::from_millis(20), Duration::from_millis(180)],
        Duration::ZERO,
        false,
        source,
        true,
    )
    .await;
    let response = preparation_request(&handle, false).await;
    assert_eq!(response.status(), 200);
    assert_eq!(source.requests.load(Ordering::SeqCst), 2);
    preparation_reports(&host, 1).await;
    let selections = host.selections.lock().unwrap();
    assert_eq!(selections.len(), 2);
    assert!(selections[1].force_refresh);
    assert_eq!(selections[1].preferred_account_id.as_deref(), Some("a"));
    drop(selections);
    assert_eq!(host.reports.lock().unwrap().len(), 1);
    assert!(host.reports.lock().unwrap()[0].success);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn websocket_automatic_history_preparation_timeout_keeps_cipher_and_never_commits_or_retries()
{
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    let policy = ExecutionPolicy {
        total_timeout: Duration::from_millis(200),
        ..preparation_policy()
    };
    let (mut handle, host, source, task, dir) = preparation_fixture(
        policy,
        vec![Duration::from_millis(700)],
        Duration::ZERO,
        false,
        preparation_ok(),
    )
    .await;
    let path = dir.path().join("identity.sqlite");

    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer key".parse().unwrap());
    let mut socket = tokio_tungstenite::connect_async(request).await.unwrap().0;
    let original = json!({"type":"response.create","model":"m","client_metadata":{"session_id":"recover-thread","thread_id":"recover-thread"},"input":[{"type":"reasoning","encrypted_content":"original-cipher"}]});
    socket
        .send(Message::Text(original.to_string().into()))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let value: Value = serde_json::from_str(&event.into_text().unwrap()).unwrap();
    assert_eq!(value["status"], 504);
    assert_eq!(value["error"]["code"], "upstream_total_timeout");
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert!(host.reports.lock().unwrap().is_empty(),
        "selection/preparation failures must not create upstream request records");
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    assert_eq!(source.requests.load(Ordering::SeqCst), 0);
    let connection = rusqlite::Connection::open(&path).unwrap();
    for table in ["native_rebuild_active_v1", "native_rebuild_checkpoints_v1"] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert_eq!(original["input"][0]["encrypted_content"], "original-cipher");
    task.abort();
}

#[tokio::test]
async fn websocket_first_and_followup_selection_each_use_execution_instead_of_read_budget() {
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    let (mut handle, host, source, task, _dir) = preparation_fixture(
        preparation_policy(),
        vec![Duration::from_millis(250), Duration::from_millis(250)],
        Duration::ZERO,
        false,
        preparation_ok(),
    )
    .await;
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer key".parse().unwrap());
    let mut socket = tokio_tungstenite::connect_async(request).await.unwrap().0;
    for turn in 0..2 {
        socket.send(Message::Text(json!({"type":"response.create","model":"m","input":[{"role":"user","content":format!("turn {turn}")}]}).to_string().into())).await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(&event.into_text().unwrap()).unwrap();
        assert_eq!(value["type"], "response.completed");
    }
    preparation_reports(&host, 2).await;
    assert_eq!(source.requests.load(Ordering::SeqCst), 2);
    assert!(host
        .reports
        .lock()
        .unwrap()
        .iter()
        .all(|report| report.success));
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn images_initial_selection_uses_the_image_execution_budget() {
    let (mut handle, host, source, task, _dir) = preparation_fixture(
        preparation_policy().for_images(),
        vec![Duration::from_millis(250)],
        Duration::ZERO,
        false,
        preparation_ok(),
    )
    .await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"image-model","prompt":"draw a square"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["data"][0]["b64_json"],
        "aW1hZ2U="
    );
    preparation_reports(&host, 1).await;
    assert_eq!(source.requests.load(Ordering::SeqCst), 1);
    assert!(host.reports.lock().unwrap()[0].success);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
