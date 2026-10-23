use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use axum::{
    body::{Body, Bytes},
    extract::{Multipart, State},
    http::{HeaderMap, HeaderValue},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, stream, StreamExt};
use serde_json::{json, Value};
use tokio::{net::TcpListener, task::JoinHandle};

use super::super::{contract::*, start, Handle, Options};

#[path = "images_batch_deadline_tests.rs"]
mod batch_deadline_tests;
#[path = "go_limits_images_tests.rs"]
mod go_limits_tests;

#[path = "images_credentials_retry_tests.rs"]
mod credentials_retry_tests;

#[tokio::test]
async fn oauth_image_request_fault_keeps_error_without_refresh_rotation_or_cooling() {
    for status in [401, 403, 503] {
        let raw = r#"{"error":{"code":"invalid_request_error","type":"invalid_request_error","message":"Image prompt rejected"}}"#;
        let app = Router::new().route(
            "/v1/responses",
            post(move || async move { (axum::http::StatusCode::from_u16(status).unwrap(), raw) }),
        );
        let (handle, host, upstream, root) = fixture(app, true, false).await;
        host.credential_retries.store(2, Ordering::SeqCst);
        host.alternate_sources.store(1, Ordering::SeqCst);
        host.policy.lock().unwrap().status_retries = 2;
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/images/generations", handle.addr))
            .bearer_auth("local-image-key")
            .json(&json!({"prompt":"draw a fixture","model":"image-model"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.text().await.unwrap(), raw);
        tokio::time::timeout(Duration::from_secs(2), async {
            while host.reports.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(host.selections.lock().unwrap().len(), 1);
        assert!(host.failures.lock().unwrap().is_empty());
        assert!(
            host.reports
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .diagnostics
                .as_ref()
                .unwrap()
                .upstream_request_scoped
        );
        cleanup(handle, upstream, root).await;
    }
}

struct ImageHost {
    base_url: String,
    oauth: bool,
    deny: bool,
    reports: Mutex<Vec<RequestReport>>,
    failures: Mutex<Vec<(String, u16)>>,
    selections: Mutex<Vec<ImageRouteRequest>>,
    capacity: Arc<tokio::sync::Semaphore>,
    retries: AtomicUsize,
    credential_retries: AtomicUsize,
    alternate_sources: AtomicUsize,
    policy: Mutex<ExecutionPolicy>,
}

impl Host for ImageHost {
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async move {
            let mut policy = self.policy.lock().unwrap().clone();
            policy.send_retries = self.retries.load(Ordering::SeqCst) as u8;
            policy.credential_retries = self.credential_retries.load(Ordering::SeqCst) as u16;
            Ok(policy)
        })
    }
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            if bearer != "local-image-key" {
                return Err(ServiceError::new(
                    401,
                    "invalid_api_key",
                    "invalid test key",
                ));
            }
            Ok(Caller {
                id: "images-caller".into(),
                label: "test".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _caller: &'a Caller,
        _request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async {
            Err(ServiceError::new(
                500,
                "unexpected_text_route",
                "Images must use image policy routing",
            ))
        })
    }
    fn image_route<'a>(
        &'a self,
        _caller: &'a Caller,
        request: ImageRouteRequest,
    ) -> BoxFuture<'a, Result<ImageRoute, ServiceError>> {
        Box::pin(async move {
            self.selections.lock().unwrap().push(request.clone());
            if self.deny {
                return Err(ServiceError::new(
                    404,
                    "image_generation_disabled",
                    "Images disabled",
                ));
            }
            let permit = self
                .capacity
                .clone()
                .try_acquire_owned()
                .map_err(|_| ServiceError::new(429, "account_busy", "busy"))?;
            let account_id = if request
                .excluded_account_ids
                .contains(&"image-account".to_owned())
            {
                if self.alternate_sources.load(Ordering::SeqCst) == 0
                    || request
                        .excluded_account_ids
                        .contains(&"image-account-2".to_owned())
                {
                    return Err(ServiceError::new(
                        503,
                        "image_source_unavailable",
                        "no alternate fixture source",
                    ));
                }
                "image-account-2"
            } else {
                "image-account"
            };
            let mut headers = HeaderMap::new();
            headers.insert(
                "authorization",
                HeaderValue::from_static("Bearer upstream-image-key"),
            );
            Ok(ImageRoute {
                route: Route {
                    continuity_account_id: None,
                    installation_profile: None,
                    official_authorization_revision: None,
                    official_registration_id: None,
                    official_fallback_registration_id: None,
                    official_fallback_authorization_revision: None,
                    authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                    transport_identity: None,
                    account_id: account_id.into(),
                    owner_id: format!("{account_id}-owner"),
                    base_url: self.base_url.clone(),
                    model: if self.oauth {
                        "text-main"
                    } else {
                        "image-model"
                    }
                    .into(),
                    headers,
                    client: reqwest::Client::builder().no_proxy().build().unwrap(),
                    proxy_url: None,
                    proxy_route: None,
                    chatgpt: self.oauth,
                    chatgpt_plan: false,
                    wire_api: WireApi::Responses,
                    compact_endpoint: None,
                    websocket_endpoint: None,
                    default_service_tier: None,
                    permit: Some(Arc::new(permit)),
                },
                image_model: "gpt-image-2.5-flare".into(),
                main_model: self.oauth.then(|| "text-main".into()),
            })
        })
    }
    fn models<'a>(&'a self, _caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
    fn route_failed<'a>(
        &'a self,
        account: &'a str,
        status: u16,
        _retry_after: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.failures.lock().unwrap().push((account.into(), status));
        })
    }
}

async fn fixture(
    app: Router,
    oauth: bool,
    deny: bool,
) -> (Handle, Arc<ImageHost>, JoinHandle<()>, PathBuf) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = fixture_host(format!("http://{address}/v1"), oauth, deny);
    let root = std::env::temp_dir().join(format!("cockpit-images-test-{}", uuid::Uuid::now_v7()));
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    (handle, host, upstream, root)
}

fn fixture_host(base_url: String, oauth: bool, deny: bool) -> Arc<ImageHost> {
    Arc::new(ImageHost {
        base_url,
        oauth,
        deny,
        reports: Mutex::new(Vec::new()),
        failures: Mutex::new(Vec::new()),
        selections: Mutex::new(Vec::new()),
        capacity: Arc::new(tokio::sync::Semaphore::new(1)),
        retries: AtomicUsize::new(0),
        credential_retries: AtomicUsize::new(1),
        alternate_sources: AtomicUsize::new(0),
        policy: Mutex::new(ExecutionPolicy::default()),
    })
}

async fn cleanup(mut handle: Handle, upstream: JoinHandle<()>, root: PathBuf) {
    handle.shutdown(Duration::from_secs(3)).await.unwrap();
    upstream.abort();
    let _ = upstream.await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn asynchronous_api_key_image_job_is_rejected_before_upstream_submission() {
    let called = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = called.clone();
    let app = Router::new().route(
        "/v1/images/generations",
        post(move || {
            let capture = capture.clone();
            async move {
                capture.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::ACCEPTED,
                    Json(json!({"id":"remote-job","status":"pending"})),
                )
            }
        }),
    );
    let (handle, _, upstream, root) = fixture(app, false, false).await;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw", "async":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "async_image_not_supported"
    );
    assert_eq!(called.load(Ordering::SeqCst), 0);
    cleanup(handle, upstream, root).await;
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

#[tokio::test]
async fn native_images_json_uses_native_endpoint_and_preserves_fields_and_usage() {
    let seen = Arc::new(Mutex::new(None));
    let app=Router::new().route("/v1/images/generations",post(|State(seen):State<Arc<Mutex<Option<Value>>>>,Json(body):Json<Value>|async move {
        *seen.lock().unwrap()=Some(body);Json(json!({"created":5,"data":[{"url":"https://images.example/result.png"}],"usage":{"total_tokens":12},"vendor_output":"kept"}))
    })).with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    let response=client().post(format!("http://{}/v1/images/generations",handle.addr)).bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw","model":"requested-image","response_format":"url","vendor_field":true})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let output: Value = response.json().await.unwrap();
    assert_eq!(output["vendor_output"], "kept");
    let body = seen.lock().unwrap().clone().unwrap();
    assert_eq!(body["model"], "gpt-image-2.5-flare");
    assert_eq!(body["vendor_field"], true);
    assert_eq!(
        host.reports.lock().unwrap()[0].usage.as_ref().unwrap()["total_tokens"],
        12
    );
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn native_images_multipart_preserves_binary_files_and_selected_model() {
    type Parts = Arc<Mutex<Vec<(String, Option<String>, Vec<u8>)>>>;
    let seen: Parts = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/v1/images/edits",
            post(
                |State(seen): State<Parts>, mut form: Multipart| async move {
                    let mut output = Vec::new();
                    while let Some(part) = form.next_field().await.unwrap() {
                        let name = part.name().unwrap().to_owned();
                        let file = part.file_name().map(str::to_owned);
                        output.push((name, file, part.bytes().await.unwrap().to_vec()));
                    }
                    *seen.lock().unwrap() = output;
                    Json(json!({"data":[{"b64_json":"AA=="}],"usage":{"total_tokens":3}}))
                },
            ),
        )
        .with_state(seen.clone());
    let (handle, _host, upstream, root) = fixture(app, false, false).await;
    let bytes = vec![0, 1, 2, 255];
    let form = reqwest::multipart::Form::new()
        .text("prompt", "edit")
        .text("model", "client-image")
        .part(
            "image[]",
            reqwest::multipart::Part::bytes(bytes.clone())
                .file_name("source.png")
                .mime_str("image/png")
                .unwrap(),
        );
    let response = client()
        .post(format!("http://{}/v1/images/edits", handle.addr))
        .bearer_auth("local-image-key")
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let parts = seen.lock().unwrap().clone();
    assert!(parts.iter().any(|(name, file, value)| name == "image[]"
        && file.as_deref() == Some("source.png")
        && value == &bytes));
    assert!(parts
        .iter()
        .any(|(name, _, value)| name == "model" && value == b"gpt-image-2.5-flare"));
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn oauth_images_runs_requested_count_once_each_and_sums_usage() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app=Router::new().route("/v1/responses",post(|State(seen):State<Arc<Mutex<Vec<Value>>>>,Json(body):Json<Value>|async move {
        seen.lock().unwrap().push(body);
        ([("content-type","text/event-stream")],format!("data: {}\n\n",json!({"type":"response.completed","response":{"id":format!("response-{}",uuid::Uuid::now_v7()),"status":"completed","output":[{"type":"image_generation_call","result":"AA=="}],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}})))
    })).with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw","n":2}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["data"].as_array().unwrap().len(), 2);
    assert_eq!(result["usage"]["total_tokens"], 10);
    let bodies = seen.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["model"], "text-main");
    assert_eq!(bodies[0]["tools"][0]["model"], "gpt-image-2.5-flare");
    assert_eq!(bodies[0]["tools"][0]["action"], "generate");
    assert_eq!(
        host.reports.lock().unwrap()[0].usage.as_ref().unwrap()["total_tokens"],
        10
    );
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn oauth_image_edits_stream_images_events_and_preserves_mask() {
    let seen = Arc::new(Mutex::new(None));
    let app=Router::new().route("/v1/responses",post(|State(seen):State<Arc<Mutex<Option<Value>>>>,Json(body):Json<Value>|async move {
        *seen.lock().unwrap()=Some(body);
        ([("content-type","text/event-stream")],format!("data: {}\n\ndata: {}\n\n",json!({"type":"response.image_generation_call.partial_image","partial_image_index":0,"partial_image_b64":"AA=="}),json!({"type":"response.completed","response":{"id":"image-edit-response","status":"completed","output":[{"type":"image_generation_call","result":"AQ=="}],"usage":{"total_tokens":7}}})))
    })).with_state(seen.clone());
    let (handle, _host, upstream, root) = fixture(app, true, false).await;
    let response=client().post(format!("http://{}/v1/images/edits",handle.addr)).bearer_auth("local-image-key")
        .json(&json!({"prompt":"edit","image":"data:image/png;base64,AA==","mask":{"image_url":"data:image/png;base64,AQ=="},"stream":true})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("event: image_edit.partial_image"));
    assert!(text.contains("event: image_edit.completed"));
    assert!(!text.contains("event: response.completed"));
    assert_eq!(
        seen.lock().unwrap().as_ref().unwrap()["tools"][0]["input_image_mask"]["image_url"],
        "data:image/png;base64,AQ=="
    );
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_source_policy_rejection_does_not_contact_upstream() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/images/generations",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Json(json!({"data":[]}))
            }),
        )
        .with_state(calls.clone());
    let (handle, _host, upstream, root) = fixture(app, false, true).await;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn disconnecting_image_stream_cancels_remaining_requested_images() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app=Router::new().route("/v1/responses",post(|State(calls):State<Arc<AtomicUsize>>|async move {
        calls.fetch_add(1,Ordering::SeqCst);
        let partial=Bytes::from(format!("data: {}\n\n",json!({"type":"response.image_generation_call.partial_image","partial_image_b64":"AA==","partial_image_index":0})));
        let stream=stream::once(async move {Ok::<_,std::io::Error>(partial)}).chain(stream::pending());
        ([("content-type","text/event-stream")],Body::from_stream(stream)).into_response()
    })).with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw","n":3,"stream":true}))
        .send()
        .await
        .unwrap();
    let mut response = response.bytes_stream();
    let _ = response.next().await.unwrap().unwrap();
    drop(response);
    tokio::time::timeout(Duration::from_secs(3), async {
        while handle.active_requests() != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn unauthorized_api_key_image_request_is_sent_once_and_releases_permit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/images/generations",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return (
                        axum::http::StatusCode::UNAUTHORIZED,
                        Json(json!({"error":{"message":"expired"}})),
                    )
                        .into_response();
                }
                Json(json!({"data":[{"b64_json":"AA=="}],"usage":{"total_tokens":5}}))
                    .into_response()
            }),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .timeout(Duration::from_secs(3))
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body, json!({"error":{"message":"expired"}}));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let selections = host.selections.lock().unwrap().clone();
    assert_eq!(selections.len(), 1);
    assert!(!selections[0].force_refresh);
    assert!(selections[0].preferred_account_id.is_none());
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn later_oauth_image_failure_keeps_prior_and_failed_operation_usage() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app=Router::new().route("/v1/responses",post(|State(calls):State<Arc<AtomicUsize>>|async move {
        let event=if calls.fetch_add(1,Ordering::SeqCst)==0 {
            json!({"type":"response.completed","response":{"status":"completed","id":"first-response","output":[{"type":"image_generation_call","result":"AA=="}],"usage":{"total_tokens":5}}})
        }else {json!({"type":"response.failed","response":{"error":{"message":"image failed"},"usage":{"total_tokens":3}}})};
        ([("content-type","text/event-stream")],format!("data: {event}\n\n"))
    })).with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw","n":3}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    let _: Value = response.json().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(!reports[0].success);
    assert_eq!(reports[0].usage.as_ref().unwrap()["total_tokens"], 8);
    drop(reports);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn forced_responses_image_generation_uses_the_native_image_source() {
    let seen = Arc::new(Mutex::new(None));
    let app=Router::new().route("/v1/images/generations",post(|State(seen):State<Arc<Mutex<Option<Value>>>>,Json(body):Json<Value>|async move {
        *seen.lock().unwrap()=Some(body);
        Json(json!({"data":[{"b64_json":"AA==","revised_prompt":"revised"}],"usage":{"total_tokens":9}}))
    })).with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    let response=client().post(format!("http://{}/v1/responses",handle.addr)).bearer_auth("local-image-key")
        .json(&json!({"model":"text-main","input":"Draw a square","tools":[{"type":"image_generation","quality":"high"}],"tool_choice":{"type":"image_generation"},"stream":true})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("response.created"));
    assert!(text.contains("response.output_item.done"));
    assert!(text.contains("response.completed"));
    assert!(text.contains("image_generation_call"));
    assert!(text.contains("revised"));
    assert_eq!(
        seen.lock().unwrap().as_ref().unwrap()["model"],
        "gpt-image-2.5-flare"
    );
    assert_eq!(seen.lock().unwrap().as_ref().unwrap()["quality"], "high");
    assert_eq!(
        host.selections.lock().unwrap()[0]
            .responses_model
            .as_deref(),
        Some("text-main")
    );
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn forced_oauth_responses_preserves_selected_image_owner_without_text_routing() {
    let seen = Arc::new(Mutex::new(None));
    let app=Router::new().route("/v1/responses",post(|State(seen):State<Arc<Mutex<Option<Value>>>>,Json(body):Json<Value>|async move {
        *seen.lock().unwrap()=Some(body);
        Json(json!({"id":"oauth-image-response","object":"response","status":"completed","output":[{"type":"image_generation_call","result":"AA=="}],"usage":{"total_tokens":4}}))
    })).with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    let response=client().post(format!("http://{}/v1/responses",handle.addr)).bearer_auth("local-image-key")
        .json(&json!({"model":"text-main","input":"Draw a square","tools":[{"type":"image_generation"}],"tool_choice":{"type":"image_generation"}})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let value: Value = response.json().await.unwrap();
    assert_eq!(value["id"], "oauth-image-response");
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    assert_eq!(seen.lock().unwrap().as_ref().unwrap()["model"], "text-main");
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_input_and_output_urls_cannot_contact_private_addresses() {
    let calls = Arc::new(AtomicUsize::new(0));
    let image_calls = Arc::new(AtomicUsize::new(0));
    let target = Router::new()
        .route(
            "/asset",
            axum::routing::get(|State(calls): State<Arc<AtomicUsize>>| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                "private"
            }),
        )
        .with_state(calls.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let asset_url = format!("http://{}/asset", listener.local_addr().unwrap());
    let asset_server = tokio::spawn(async move { axum::serve(listener, target).await.unwrap() });
    let returned = asset_url.clone();
    let app = Router::new().route(
        "/v1/images/generations",
        post(move || {
            let returned = returned.clone();
            let image_calls = image_calls.clone();
            async move {
                image_calls.fetch_add(1, Ordering::SeqCst);
                Json(json!({"data":[{"url":returned}]}))
            }
        }),
    );
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    let output = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(output.status(), 502);
    let input = client()
        .post(format!("http://{}/v1/images/edits", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"edit","image":asset_url}))
        .send()
        .await
        .unwrap();
    assert_eq!(input.status(), 400);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
    asset_server.abort();
    let _ = asset_server.await;
}

#[tokio::test]
async fn forced_oauth_unauthorized_retry_retains_image_scope_and_releases_permit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app=Router::new().route("/v1/responses",post(|State(calls):State<Arc<AtomicUsize>>|async move {
        if calls.fetch_add(1,Ordering::SeqCst)==0 {return (axum::http::StatusCode::UNAUTHORIZED,Json(json!({"error":{"message":"expired"}}))).into_response();}
        Json(json!({"id":"refreshed-image","object":"response","status":"completed","output":[{"type":"image_generation_call","result":"AA=="}]})).into_response()
    })).with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    let response=client().post(format!("http://{}/v1/responses",handle.addr)).bearer_auth("local-image-key").timeout(Duration::from_secs(3))
        .json(&json!({"model":"text-main","input":"Draw","tools":[{"type":"image_generation"}],"tool_choice":{"type":"image_generation"}})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let selections = host.selections.lock().unwrap().clone();
    assert_eq!(selections.len(), 2);
    assert!(selections[1].force_refresh);
    assert_eq!(
        selections[1].preferred_account_id.as_deref(),
        Some("image-account")
    );
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_transient_http_retry_uses_configured_policy_without_reacquiring_permit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/images/generations",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({"error":{"message":"temporary"}})),
                    )
                        .into_response();
                }
                Json(json!({"data":[{"b64_json":"AA=="}]})).into_response()
            }),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    host.policy.lock().unwrap().status_retries = 1;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .timeout(Duration::from_secs(3))
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn forced_responses_stream_delivers_partial_image_before_upstream_completion() {
    let complete = Arc::new(tokio::sync::Notify::new());
    let app=Router::new().route("/v1/images/generations",post(|State(complete):State<Arc<tokio::sync::Notify>>,Json(body):Json<Value>|async move {
        assert_eq!(body["stream"],true);
        let partial=Bytes::from(format!("data: {}\n\n",json!({"type":"image_generation.partial_image","b64_json":"AA==","partial_image_index":0})));
        let terminal=stream::once(async move {complete.notified().await;Ok::<_,std::io::Error>(Bytes::from(format!("data: {}\n\n",json!({"type":"image_generation.completed","b64_json":"AQ==","usage":{"total_tokens":6}}))))});
        let body=stream::once(async move {Ok::<_,std::io::Error>(partial)}).chain(terminal);
        ([("content-type","text/event-stream")],Body::from_stream(body))
    })).with_state(complete.clone());
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    let response=client().post(format!("http://{}/v1/responses",handle.addr)).bearer_auth("local-image-key")
        .json(&json!({"model":"text-main","input":"Draw","tools":[{"type":"image_generation","partial_images":1}],"tool_choice":{"type":"image_generation"},"stream":true})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let mut stream = response.bytes_stream();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !text.contains("response.image_generation_call.partial_image") {
            text.push_str(
                &String::from_utf8(stream.next().await.unwrap().unwrap().to_vec()).unwrap(),
            );
        }
    })
    .await
    .unwrap();
    assert_eq!(host.capacity.available_permits(), 0);
    complete.notify_one();
    while let Some(bytes) = stream.next().await {
        text.push_str(&String::from_utf8(bytes.unwrap().to_vec()).unwrap());
    }
    assert!(text.contains("response.completed"));
    assert!(text.contains("AQ=="));
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_credential_retry_excludes_failed_source_and_releases_its_permit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/images/generations",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return (
                        axum::http::StatusCode::FORBIDDEN,
                        Json(json!({"error":{"message":"source rejected"}})),
                    )
                        .into_response();
                }
                Json(json!({"data":[{"b64_json":"AA=="}]})).into_response()
            }),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    host.credential_retries.store(2, Ordering::SeqCst);
    host.alternate_sources.store(1, Ordering::SeqCst);
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .timeout(Duration::from_secs(3))
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let selections = host.selections.lock().unwrap().clone();
    assert_eq!(selections.len(), 2);
    assert_eq!(selections[1].excluded_account_ids, vec!["image-account"]);
    assert_eq!(
        host.reports.lock().unwrap().last().unwrap().account_id,
        "image-account-2"
    );
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn external_image_source_rejects_opaque_responses_history_without_sending_upstream() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/images/generations",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Json(json!({"data":[{"b64_json":"AA=="}]}))
            }),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    let response=client().post(format!("http://{}/v1/responses",handle.addr)).bearer_auth("local-image-key")
        .json(&json!({"model":"text-main","input":"Use the previous image","previous_response_id":"prior","tools":[{"type":"image_generation"}],"tool_choice":{"type":"image_generation"}})).send().await.unwrap();
    assert_eq!(response.status(), 409);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "continuation_not_supported");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn later_image_failure_never_switches_owner_after_an_image_completed() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app=Router::new().route("/v1/responses",post(|State(calls):State<Arc<AtomicUsize>>|async move {
        if calls.fetch_add(1,Ordering::SeqCst)>0{return (axum::http::StatusCode::FORBIDDEN,Json(json!({"error":{"message":"later source failure"}}))).into_response();}
        Json(json!({"object":"response","id":"first-image","status":"completed","output":[{"type":"image_generation_call","result":"AA=="}]})).into_response()
    })).with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    host.credential_retries.store(2, Ordering::SeqCst);
    host.alternate_sources.store(1, Ordering::SeqCst);
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw","n":2}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let _: Value = response.json().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

fn oauth_image_completion_event() -> Value {
    json!({"type":"response.completed","response":{"id":"timeout-image","status":"completed","output":[{"type":"image_generation_call","result":"AA=="}]}})
}
fn oauth_image_completion() -> axum::response::Response {
    ([("content-type", "text/event-stream")], format!("data: {}\n\n", oauth_image_completion_event())).into_response()
}

#[tokio::test]
async fn image_headers_use_image_open_timeout_instead_of_text_timeout() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/responses",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(90)).await;
                oauth_image_completion()
            }),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    {
        let mut policy = host.policy.lock().unwrap();
        policy.open_timeout = Duration::from_millis(20);
        policy.image_open_timeout = Duration::from_millis(300);
    }
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    {
        let mut policy = host.policy.lock().unwrap();
        policy.open_timeout = Duration::from_secs(1);
        policy.image_open_timeout = Duration::from_millis(25);
    }
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 504);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "upstream_connect_timeout");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_body_uses_image_idle_timeout_independently_of_text_idle() {
    let app = Router::new().route(
        "/v1/responses",
        post(|| async {
            let first =
                stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(b"data: {\"type\":\"response.created\",\"response\":{\"id\":\"timeout-image\",\"status\":\"in_progress\"}}\n\n")) });
            let tail = stream::once(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok::<_, std::io::Error>(Bytes::from(format!("data: {}\n\n", oauth_image_completion_event())))
            });
            (
                [("content-type", "text/event-stream")],
                Body::from_stream(first.chain(tail)),
            )
        }),
    );
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    {
        let mut policy = host.policy.lock().unwrap();
        policy.idle_timeout = Duration::from_millis(20);
        policy.image_idle_timeout = Duration::from_millis(300);
    }
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    {
        let mut policy = host.policy.lock().unwrap();
        policy.idle_timeout = Duration::from_secs(1);
        policy.image_idle_timeout = Duration::from_millis(25);
    }
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 504);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "upstream_idle_timeout");
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_stream_open_attempts_include_first_attempt_and_have_their_own_budget() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/responses",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(200)).await;
                oauth_image_completion()
            }),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    {
        let mut policy = host.policy.lock().unwrap();
        policy.image_open_timeout = Duration::from_millis(30);
        policy.stream_open_max_attempts = 3;
    }
    host.retries.store(5, Ordering::SeqCst);
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw","stream":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 504);
    let _: Value = response.json().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    host.retries.store(0, Ordering::SeqCst);
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 504);
    let _: Value = response.json().await.unwrap();
    // OAuth streams upstream even when the caller collects JSON.
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn transient_image_status_budget_is_independent_from_send_retries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/images/generations",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({"error":{"message":"temporary"}})),
                )
            }),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    host.retries.store(4, Ordering::SeqCst);
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    let _: Value = response.json().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    {
        let mut policy = host.policy.lock().unwrap();
        policy.status_retries = 2;
        policy.status_retry_base_delay = Duration::from_millis(1);
        policy.status_retry_max_delay = Duration::from_millis(1);
    }
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    let _: Value = response.json().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(host.selections.lock().unwrap().len(), 2);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_throttling_switches_source_without_same_source_status_retries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/images/generations",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        Json(json!({"error":{"message":"throttled"}})),
                    )
                        .into_response();
                }
                Json(json!({"data":[{"b64_json":"AA=="}]})).into_response()
            }),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    host.retries.store(3, Ordering::SeqCst);
    host.credential_retries.store(2, Ordering::SeqCst);
    host.alternate_sources.store(1, Ordering::SeqCst);
    host.policy.lock().unwrap().status_retries = 3;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        host.failures.lock().unwrap().as_slice(),
        &[("image-account".into(), 429)]
    );
    let selections = host.selections.lock().unwrap().clone();
    assert_eq!(selections.len(), 2);
    assert_eq!(selections[1].excluded_account_ids, vec!["image-account"]);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn exhausted_image_stream_open_budget_can_retry_an_alternate_before_output() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/responses",
            post(|State(calls): State<Arc<AtomicUsize>>| async move {
                if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                oauth_image_completion()
            }),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    {
        let mut policy = host.policy.lock().unwrap();
        policy.image_open_timeout = Duration::from_millis(30);
        policy.stream_open_max_attempts = 2;
    }
    host.retries.store(5, Ordering::SeqCst);
    host.credential_retries.store(2, Ordering::SeqCst);
    host.alternate_sources.store(1, Ordering::SeqCst);
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw","stream":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("image_generation.completed"));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(host.failures.lock().unwrap().is_empty());
    let selections = host.selections.lock().unwrap().clone();
    assert_eq!(selections.len(), 2);
    assert_eq!(selections[1].excluded_account_ids, vec!["image-account"]);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_status_and_network_retries_have_independent_attempt_counters() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let server_calls = calls.clone();
    let upstream = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            let expected = loop {
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break 0;
                }
                request.extend_from_slice(&chunk[..read]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    break end + 4 + length;
                }
            };
            while request.len() < expected {
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
            }
            let attempt = server_calls.fetch_add(1, Ordering::SeqCst);
            if attempt == 1 {
                drop(socket);
                continue;
            }
            let (status, body) = if attempt == 0 {
                (
                    "503 Service Unavailable",
                    json!({"error":{"message":"temporary"}}),
                )
            } else {
                ("200 OK", json!({"data":[{"b64_json":"AA=="}]}))
            };
            let body = body.to_string();
            let head=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    let host = fixture_host(format!("http://{address}/v1"), false, false);
    host.retries.store(1, Ordering::SeqCst);
    {
        let mut policy = host.policy.lock().unwrap();
        policy.status_retries = 1;
        policy.status_retry_base_delay = Duration::from_millis(1);
        policy.retry_base_delay = Duration::from_millis(1);
    }
    let root = std::env::temp_dir().join(format!("cockpit-images-test-{}", uuid::Uuid::now_v7()));
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .timeout(Duration::from_secs(3))
        .json(&json!({"prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_output_conversion_preserves_safe_headers_and_eof_completed_frames() {
    for (oauth, bridge, client_stream, source_json) in [
        (true, false, false, false),
        (true, false, true, false),
        (false, false, false, false),
        (false, false, true, true),
        (false, false, true, false),
        (false, true, false, false),
        (false, true, true, false),
    ] {
        let path = if oauth {
            "/v1/responses"
        } else {
            "/v1/images/generations"
        };
        let app = Router::new().route(path, post(move || async move {
            let (mime, body) = if oauth {
                ("text/event-stream", format!("data: {}", json!({"type":"response.completed","response":{
                    "id":"image-header-response","status":"completed","output":[{"type":"image_generation_call","result":"AA=="}],
                    "usage":{"total_tokens":999},"tool_usage":{"image_gen":{"total_tokens":14}}
                }})))
            } else if source_json {
                ("application/json", json!({"created":10,"data":[{"b64_json":"AA=="}],"usage":{"total_tokens":14}}).to_string())
            } else {
                ("text/event-stream", format!("data: {}",json!({"type":"image_generation.completed","b64_json":"AA==","usage":{"total_tokens":14}})))
            };
            ([("content-type", mime),("x-request-id","image-output-id"),("x-ratelimit-remaining-requests","42"),("set-cookie","private=1"),("x-litellm-fixture","blocked")], body)
        }));
        let (handle, _host, upstream, root) = fixture(app, oauth, false).await;
        let (path, body) = if bridge {
            (
                "responses",
                json!({"model":"text-main","input":"draw","tools":[{"type":"image_generation"}],"tool_choice":{"type":"image_generation"},"stream":client_stream}),
            )
        } else {
            (
                "images/generations",
                json!({"prompt":"draw","stream":client_stream}),
            )
        };
        let response = client()
            .post(format!("http://{}/v1/{path}", handle.addr))
            .bearer_auth("local-image-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            200,
            "oauth={oauth} bridge={bridge} stream={client_stream}"
        );
        assert_eq!(response.headers()["x-request-id"], "image-output-id");
        assert_eq!(response.headers()["x-ratelimit-remaining-requests"], "42");
        assert!(!response.headers().contains_key("set-cookie"));
        assert!(!response.headers().contains_key("x-litellm-fixture"));
        assert!(response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with(if client_stream {
                "text/event-stream"
            } else {
                "application/json"
            }));
        let wire = response.text().await.unwrap();
        assert!(!wire.contains("event: error"), "{wire}");
        assert!(!wire.contains("response.failed"), "{wire}");
        assert!(wire.contains("AA=="), "{wire}");
        assert!(wire.contains("14"), "{wire}");
        if !client_stream {
            assert_eq!(
                serde_json::from_str::<Value>(&wire).unwrap()["usage"]["total_tokens"],
                14
            );
        }
        cleanup(handle, upstream, root).await;
    }
}

#[tokio::test]
async fn oauth_multi_image_stream_keeps_distinct_output_indexes_across_operations() {
    let app = Router::new().route("/v1/responses",post(|| async {
        ([("content-type","text/event-stream")],format!("data: {}\n\n",json!({"type":"response.completed","response":{
            "id":"multi-image-response","status":"completed","output":[
                {"type":"image_generation_call","result":"AA=="},{"type":"image_generation_call","result":"AQ=="}
            ],"tool_usage":{"image_gen":{"total_tokens":7}}
        }})))
    }));
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw","n":2,"stream":true}))
        .send()
        .await
        .unwrap();
    let wire = response.text().await.unwrap();
    let events = wire
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str::<Value>(data).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .map(|event| event["output_index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert!(events
        .iter()
        .all(|event| event["usage"]["total_tokens"] == 7));
    assert_eq!(
        host.reports.lock().unwrap()[0].usage.as_ref().unwrap()["total_tokens"],
        14
    );
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn image_errors_preserve_details_status_and_sanitize_secret_text() {
    for (oauth, bridge, client_stream) in [
        (true, false, false),
        (true, false, true),
        (false, false, false),
        (false, false, true),
        (false, true, false),
        (false, true, true),
    ] {
        let path = if oauth {
            "/v1/responses"
        } else {
            "/v1/images/generations"
        };
        // Emit image output before the failure so bootstrap retry policy does not
        // replace the in-stream error under test with an opening HTTP rejection.
        let app=Router::new().route(path,post(move || async move {
            let partial=if oauth {json!({"type":"response.image_generation_call.partial_image","partial_image_b64":"AA==","partial_image_index":0})}
                else {json!({"type":"image_generation.partial_image","b64_json":"AA==","partial_image_index":0})};
            ([("content-type","text/event-stream")],format!("data: {partial}\n\nevent: error\ndata: {}\n\n",json!({
                "type":"error","status":429,"error":{"code":"rate_limit_exceeded","type":"rate_limit_error","message":"Retry token=image-secret","param":"prompt","retry_after":1}
            })))
        }));
        let (handle, _host, upstream, root) = fixture(app, oauth, false).await;
        let (path, body) = if bridge {
            (
                "responses",
                json!({"model":"text-main","input":"draw","tools":[{"type":"image_generation"}],"tool_choice":{"type":"image_generation"},"stream":client_stream}),
            )
        } else {
            (
                "images/generations",
                json!({"prompt":"draw","stream":client_stream}),
            )
        };
        let response = client()
            .post(format!("http://{}/v1/{path}", handle.addr))
            .bearer_auth("local-image-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            if client_stream { 200 } else { 429 },
            "oauth={oauth} bridge={bridge} stream={client_stream}"
        );
        let wire = response.text().await.unwrap();
        assert!(!wire.contains("image-secret"), "{wire}");
        assert!(wire.contains("[REDACTED]"), "{wire}");
        let event: Value = if client_stream {
            wire.lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .map(|data| serde_json::from_str::<Value>(data).unwrap())
                .find(|event| {
                    event.get("error").is_some() || event.pointer("/response/error").is_some()
                })
                .unwrap()
        } else {
            serde_json::from_str(&wire).unwrap()
        };
        let details = event
            .get("error")
            .or_else(|| event.pointer("/response/error"))
            .unwrap();
        assert_eq!(details["code"], "rate_limit_exceeded");
        assert_eq!(details["type"], "rate_limit_error");
        assert_eq!(details["param"], "prompt");
        cleanup(handle, upstream, root).await;
    }
}

#[tokio::test]
async fn image_partial_output_precedes_error_in_the_same_upstream_chunk() {
    for oauth in [false, true] {
        let path = if oauth {
            "/v1/responses"
        } else {
            "/v1/images/generations"
        };
        let app=Router::new().route(path,post(move || async move {
            let partial=if oauth {json!({"type":"response.image_generation_call.partial_image","partial_image_b64":"AA==","partial_image_index":0})}
                else {json!({"type":"image_generation.partial_image","b64_json":"AA==","partial_image_index":0})};
            ([("content-type","text/event-stream")],format!("data: {partial}\n\ndata: {}\n\n",json!({"type":"error","error":{"code":"rate_limit_exceeded","message":"image rejected"}})))
        }));
        let (handle, _host, upstream, root) = fixture(app, oauth, false).await;
        let response = client()
            .post(format!("http://{}/v1/images/generations", handle.addr))
            .bearer_auth("local-image-key")
            .json(&json!({"prompt":"draw","stream":true}))
            .send()
            .await
            .unwrap();
        let wire = response.text().await.unwrap();
        let partial = wire
            .find("event: image_generation.partial_image")
            .expect(&wire);
        let error = wire.find("event: error").expect(&wire);
        assert!(partial < error, "{wire}");
        assert_eq!(wire.matches("event: error").count(), 1);
        cleanup(handle, upstream, root).await;
    }
}

#[tokio::test]
async fn native_image_partial_tail_is_delivered_before_eof_failure() {
    let app=Router::new().route("/v1/images/generations",post(|| async {
        ([("content-type","text/event-stream")],format!("data: {}",json!({"type":"image_generation.partial_image","b64_json":"AA==","partial_image_index":0})))
    }));
    let (handle, _host, upstream, root) = fixture(app, false, false).await;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"prompt":"draw","stream":true}))
        .send()
        .await
        .unwrap();
    let wire = response.text().await.unwrap();
    assert!(
        wire.find("event: image_generation.partial_image")
            .expect(&wire)
            < wire.find("event: error").expect(&wire)
    );
    assert!(wire.contains("incomplete_upstream_stream"), "{wire}");
    cleanup(handle, upstream, root).await;
}
