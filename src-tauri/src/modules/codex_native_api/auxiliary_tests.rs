//! Loopback-only protocol and admission regressions. No external API is contacted.
#[path = "auxiliary_execution_tests.rs"]
mod execution_tests;
#[path = "auxiliary_parity22_tests.rs"]
mod parity22_tests;
#[path = "auxiliary_request_fault_tests.rs"]
mod request_fault_tests;
#[path = "auxiliary_transport_tests.rs"]
mod transport_tests;
use super::super::{contract::*, start, Handle, Options};
use super::*;
use axum::{
    extract::Multipart,
    routing::{get, post},
    Router,
};
use futures_util::future::BoxFuture;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as UpstreamMessage};

struct TestHost {
    base: String,
    oauth: bool,
    owner_changed: AtomicBool,
    capacity: Arc<tokio::sync::Semaphore>,
    selections: Mutex<Vec<RouteRequest>>,
    reports: Mutex<Vec<RequestReport>>,
    execution_policy: Mutex<ExecutionPolicy>,
}
impl Host for TestHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            if !matches!(bearer, "local-a" | "local-b") {
                return Err(ServiceError::new(
                    401,
                    "invalid_api_key",
                    "Invalid fixture key",
                ));
            }
            Ok(Caller {
                id: bearer.into(),
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
            let permit: Option<Arc<dyn Send + Sync>> = if request.requires_permit {
                Some(Arc::new(
                    self.capacity
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| ServiceError::new(429, "account_busy", "Busy"))?,
                ))
            } else {
                None
            };
            let model = if request.model == "client-model" {
                "upstream-model".into()
            } else {
                request.model.clone()
            };
            self.selections.lock().unwrap().push(request);
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
                account_id: "account-a".into(),
                owner_id: if self.owner_changed.load(Ordering::SeqCst) {
                    "changed"
                } else {
                    "owner-a"
                }
                .into(),
                base_url: self.base.clone(),
                model,
                headers: HeaderMap::from_iter([(
                    header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer upstream-secret"),
                )]),
                client: client(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: self.oauth,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit,
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.execution_policy.lock().unwrap().clone()) })
    }
}
async fn fixture(
    app: Router,
    oauth: bool,
    options: Options,
) -> (Handle, Arc<TestHost>, JoinHandle<()>, PathBuf) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = Arc::new(TestHost {
        base: format!("http://{address}/v1"),
        oauth,
        owner_changed: AtomicBool::new(false),
        capacity: Arc::new(tokio::sync::Semaphore::new(1)),
        selections: Mutex::new(vec![]),
        reports: Mutex::new(vec![]),
        execution_policy: Mutex::new(ExecutionPolicy::default()),
    });
    let root = std::env::temp_dir().join(format!("native-aux-{}", uuid::Uuid::new_v4()));
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.join("identity.sqlite"),
        options,
    )
    .await
    .unwrap();
    (handle, host, upstream, root)
}
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
async fn cleanup(mut handle: Handle, upstream: JoinHandle<()>, root: PathBuf) {
    handle.shutdown(Duration::from_secs(3)).await.unwrap();
    upstream.abort();
    let _ = upstream.await;
    let _ = tokio::fs::remove_dir_all(root).await;
}
async fn wait_free(host: &TestHost) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while host.capacity.available_permits() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn search_accepts_missing_model_and_id_and_removes_local_cache_fields() {
    let seen = Arc::new(Mutex::new(None::<Value>));
    let app = Router::new().route("/v1/alpha/search", post(
        |State(seen): State<Arc<Mutex<Option<Value>>>>, Json(body): Json<Value>| async move {
            *seen.lock().unwrap() = Some(body);
            Json(json!({"results":[]}))
        }
    )).with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
    let response = client().post(format!("http://{}/v1/alpha/search", handle.addr))
        .bearer_auth("local-a").json(&json!({"input":"find this", "prompt_cache_key":"local-only", "prompt_cache_retention":"24h"}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(*seen.lock().unwrap(), Some(json!({"input":"find this"})));
    assert_eq!(host.selections.lock().unwrap()[0].model, "");
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_search_maps_caller_identity_and_model_without_changing_search_content() {
    let seen = Arc::new(Mutex::new(Vec::<(HeaderMap, Value)>::new()));
    let app = Router::new()
        .route(
            "/v1/alpha/search",
            post(
                |State(seen): State<Arc<Mutex<Vec<(HeaderMap, Value)>>>>,
                 headers: HeaderMap,
                 Json(body): Json<Value>| async move {
                    seen.lock().unwrap().push((headers, body));
                    Json(json!({"output":"result","results":[],"future":true}))
                },
            ),
        )
        .with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, false, Options::default()).await;
    for key in ["local-a", "local-a", "local-b"] {
        let response=client().post(format!("http://{}/v1/alpha/search",handle.addr)).bearer_auth(key).header("cookie","must-not-leak")
            .json(&json!({"id":"search-original","model":"client-model","input":"keep this content","commands":{"search_query":[{"q":"hello"}]},"vendor":true})).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.json::<Value>().await.unwrap()["future"], true);
    }
    wait_free(&host).await;
    let requests = seen.lock().unwrap();
    assert_eq!(requests[0].1["id"], requests[1].1["id"]);
    assert_ne!(requests[0].1["id"], requests[2].1["id"]);
    assert_ne!(requests[0].1["id"], "search-original");
    // Model canonicalization is for selection; the search payload retains its model.
    assert_eq!(requests[0].1["model"], "client-model");
    assert_eq!(requests[0].1["input"], "keep this content");
    assert_eq!(requests[0].1["vendor"], true);
    assert!(requests[0].1.get("client_metadata").is_none());
    assert!(!requests[0].0.contains_key("cookie"));
    assert_eq!(requests[0].0["authorization"], "Bearer upstream-secret");
    drop(requests);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_search_and_automatic_history_do_not_reserve_each_others_identity_mappings() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let app = Router::new()
        .route(
            "/v1/alpha/search",
            post(|State(calls): State<Arc<Mutex<Vec<String>>>>| async move {
                calls.lock().unwrap().push("search".into());
                Json(json!({"output":"result"}))
            }),
        )
        .route(
            "/v1/responses",
            post(
                |State(calls): State<Arc<Mutex<Vec<String>>>>,
                 headers: HeaderMap,
                 Json(body): Json<Value>| async move {
                    calls.lock().unwrap().push("responses".into());
                    assert_ne!(headers["thread-id"], "child");
                    assert_ne!(headers["session-id"], "root");
                    assert!(body.get("previous_response_id").is_none());
                    assert_eq!(body["input"][0]["encrypted_content"], "legacy-cipher");
                    Json(json!({"id":"automatic-response","status":"completed","output":[]}))
                },
            ),
        )
        .with_state(calls.clone());
    let (handle, _host, upstream, root) = fixture(app, false, Options::default()).await;
    let response = client()
        .post(format!("http://{}/v1/alpha/search", handle.addr))
        .bearer_auth("local-a")
        .json(&json!({"id":"root","model":"client-model","input":"query"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    let response=client().post(format!("http://{}/v1/responses",handle.addr)).bearer_auth("local-a").json(&json!({"model":"client-model","client_metadata":{"session_id":"root","thread_id":"child"},"input":[{"type":"reasoning","encrypted_content":"legacy-cipher"},{"role":"user","content":"continue"}]})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    assert_eq!(calls.lock().unwrap().as_slice(), &["search", "responses"]);
    assert!(!root
        .join("identity.sqlite")
        .with_extension("recovery")
        .exists());
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_audio_preserves_binary_file_and_maps_provider_model() {
    let seen = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let app =
        Router::new()
            .route(
                "/v1/audio/transcriptions",
                post(
                    |State(seen): State<Arc<Mutex<Vec<(String, Vec<u8>)>>>>,
                     mut form: Multipart| async move {
                        while let Some(field) = form.next_field().await.unwrap() {
                            let name = field.name().unwrap().to_string();
                            let bytes = field.bytes().await.unwrap().to_vec();
                            seen.lock().unwrap().push((name, bytes));
                        }
                        Json(json!({"text":"recognized"}))
                    },
                ),
            )
            .with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, false, Options::default()).await;
    let data = vec![0, 255, 13, 10, 128];
    let form = reqwest::multipart::Form::new()
        .text("model", "client-model")
        .part(
            "file",
            reqwest::multipart::Part::bytes(data.clone())
                .file_name("voice.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );
    let response = client()
        .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
        .bearer_auth("local-a")
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["text"],
        "recognized"
    );
    wait_free(&host).await;
    let fields = seen.lock().unwrap();
    assert!(fields.contains(&("file".into(), data)));
    assert!(fields.contains(&("model".into(), b"upstream-model".to_vec())));
    drop(fields);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_oauth_transcription_rejects_missing_or_malformed_text() {
    for reply in ["not-json", "{}", r#"{"text":null}"#, r#"{"text":12}"#] {
        let app = Router::new().route("/v1/transcribe", post(move || async move { reply }));
        let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
        let form = reqwest::multipart::Form::new()
            .text("model", "client-model")
            .part(
                "file",
                reqwest::multipart::Part::bytes(vec![1, 2, 3]).file_name("voice.wav"),
            );
        let response = client()
            .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
            .bearer_auth("local-a")
            .multipart(form)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 502, "reply: {reply}");
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"]["code"],
            "audio_invalid_upstream_response"
        );
        wait_free(&host).await;
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, 502);
        assert_eq!(
            reports[0].error.as_deref(),
            Some("audio_invalid_upstream_response")
        );
        drop(reports);
        cleanup(handle, upstream, root).await;
    }
}

#[tokio::test]
async fn auxiliary_routed_audio_network_failure_reports_actual_status() {
    let app = Router::new().route(
        "/v1/transcribe",
        post(|| async {
            Body::from_stream(stream::iter([
                Ok(axum::body::Bytes::from_static(b"{\"text\":")),
                Err(std::io::Error::other("fixture disconnected")),
            ]))
        }),
    );
    let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
    let response = client()
        .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
        .bearer_auth("local-a")
        .multipart(
            reqwest::multipart::Form::new()
                .text("model", "client-model")
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(vec![1, 2, 3]).file_name("voice.wav"),
                ),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    wait_free(&host).await;
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, 502);
    assert_eq!(reports[0].account_id, "account-a");
    assert_ne!(reports[0].error.as_deref(), Some("client_cancelled"));
    drop(reports);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_audio_uses_its_own_budget_for_slow_upstream_headers() {
    let app = Router::new().route(
        "/v1/transcribe",
        post(|| async {
            tokio::time::sleep(Duration::from_millis(450)).await;
            Json(json!({"text":"recognized"}))
        }),
    );
    let options = Options {
        request_timeout: Duration::from_millis(300),
        ..Options::default()
    };
    let (handle, host, upstream, root) = fixture(app, true, options).await;
    let response = client()
        .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
        .bearer_auth("local-a")
        .multipart(
            reqwest::multipart::Form::new()
                .text("model", "client-model")
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(vec![1, 2, 3]).file_name("voice.wav"),
                ),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["text"],
        "recognized"
    );
    wait_free(&host).await;
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_audio_total_deadline_is_not_reset_by_each_chunk() {
    let deadline = Instant::now() + Duration::from_millis(70);
    timed_read(Some(deadline), Duration::from_secs(1), async {
        tokio::time::sleep(Duration::from_millis(20)).await;
    })
    .await
    .unwrap();
    let error = timed_read(Some(deadline), Duration::from_secs(1), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
    })
    .await
    .unwrap_err();
    assert_eq!(error.status, 504);
    assert!(Instant::now() < deadline + Duration::from_millis(80));
}

#[tokio::test]
async fn auxiliary_stream_holds_account_slot_and_releases_on_disconnect_and_shutdown() {
    let app = Router::new().route(
        "/v1/audio/speech",
        post(|| async {
            let output =
                stream::once(async { Ok::<_, std::io::Error>("audio") }).chain(stream::pending());
            Body::from_stream(output)
        }),
    );
    let (mut handle, host, upstream, root) = fixture(app, false, Options::default()).await;
    for shutdown in [false, true] {
        let response = client()
            .post(format!("http://{}/v1/audio/speech", handle.addr))
            .bearer_auth("local-a")
            .json(&json!({"model":"client-model","input":"hello"}))
            .send()
            .await
            .unwrap();
        let mut body = response.bytes_stream();
        assert!(body.next().await.unwrap().is_ok());
        assert_eq!(host.capacity.available_permits(), 0);
        if shutdown {
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
        }
        drop(body);
        wait_free(&host).await;
    }
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_ephemeral_credentials_pin_owner_model_and_call_lifetime() {
    let seen = Arc::new(Mutex::new(None::<Value>));
    let app=Router::new().route("/v1/realtime/calls",post(|State(seen):State<Arc<Mutex<Option<Value>>>>,Json(body):Json<Value>|async move {
        *seen.lock().unwrap()=Some(body);([(header::LOCATION,"/v1/realtime/calls/rtc_fixture")],"v=answer\r\n")
    })).route("/v1/realtime/calls/rtc_fixture/hangup",post(||async {axum::http::StatusCode::OK})).with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
    let url = format!("http://{}", handle.addr);
    let secret = client()
        .post(format!("{url}/v1/realtime/client_secrets"))
        .bearer_auth("local-a")
        .json(&json!({"session":{"model":"gpt-realtime","id":"original"}}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["value"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(host.capacity.available_permits(), 1);
    for (path, body) in [
        ("/v1/realtime/client_secrets", json!({})),
        (
            "/v1/alpha/search",
            json!({"model":"gpt-realtime","id":"id"}),
        ),
    ] {
        assert_eq!(
            client()
                .post(format!("{url}{path}"))
                .bearer_auth(&secret)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let invalid = client()
        .post(format!("{url}/v1/realtime/calls"))
        .bearer_auth(&secret)
        .header("x-cockpit-target-account-id", "another")
        .body("v=0")
        .header("content-type", "application/sdp")
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 403);
    let call = client()
        .post(format!("{url}/v1/realtime/calls"))
        .bearer_auth(&secret)
        .body("v=0")
        .header("content-type", "application/sdp")
        .send()
        .await
        .unwrap();
    assert_eq!(call.status(), 200);
    assert_eq!(call.headers()["location"], "/v1/realtime/calls/rtc_fixture");
    let _ = call.bytes().await.unwrap();
    assert_eq!(host.capacity.available_permits(), 0);
    let body = seen.lock().unwrap().clone().unwrap();
    assert_eq!(body["session"]["model"], "gpt-live-1-codex");
    assert!(body["session"].get("id").is_none());
    assert_eq!(
        client()
            .post(format!("{url}/v1/realtime/calls/rtc_fixture/hangup"))
            .bearer_auth("local-b")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    host.owner_changed.store(true, Ordering::SeqCst);
    assert_eq!(
        client()
            .post(format!("{url}/v1/realtime/calls/rtc_fixture/hangup"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    host.owner_changed.store(false, Ordering::SeqCst);
    let response = client()
        .post(format!("{url}/v1/realtime/calls/rtc_fixture/hangup"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _ = response.bytes().await.unwrap();
    wait_free(&host).await;
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_provider_call_uses_official_multipart_and_shutdown_releases_orphan_call() {
    let seen = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let app =
        Router::new()
            .route(
                "/v1/realtime/calls",
                post(
                    |State(seen): State<Arc<Mutex<Vec<(String, Vec<u8>)>>>>,
                     mut form: Multipart| async move {
                        while let Some(field) = form.next_field().await.unwrap() {
                            let name = field.name().unwrap().to_string();
                            let bytes = field.bytes().await.unwrap().to_vec();
                            seen.lock().unwrap().push((name, bytes));
                        }
                        (
                            [(header::LOCATION, "/v1/realtime/calls/rtc_provider")],
                            "v=answer\r\n",
                        )
                    },
                ),
            )
            .with_state(seen.clone());
    let (mut handle, host, upstream, root) = fixture(app, false, Options::default()).await;
    let call = client()
        .post(format!("http://{}/v1/realtime/calls", handle.addr))
        .bearer_auth("local-a")
        .json(&json!({"sdp":"v=0\r\n","session":{"model":"client-model","id":"must-be-removed"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(call.status(), 200);
    let _ = call.bytes().await.unwrap();
    let fields = seen.lock().unwrap();
    assert!(fields.contains(&("sdp".into(), b"v=0\r\n".to_vec())));
    let session: Value =
        serde_json::from_slice(&fields.iter().find(|(name, _)| name == "session").unwrap().1)
            .unwrap();
    assert_eq!(session["model"], "upstream-model");
    assert!(session.get("id").is_none());
    drop(fields);
    assert_eq!(host.capacity.available_permits(), 0);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    wait_free(&host).await;
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_websocket_maps_session_identity_and_rejects_model_escalation() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route(
            "/v1/realtime",
            get(
                |ws: WebSocketUpgrade,
                 State(seen): State<Arc<Mutex<Vec<Value>>>>,
                 headers: HeaderMap| async move {
                    assert_eq!(headers["authorization"], "Bearer upstream-secret");
                    assert_ne!(headers["x-session-id"], "raw-header-session");
                    ws.on_upgrade(move |mut socket| async move {
                        while let Some(Ok(message)) = socket.recv().await {
                            if let Message::Text(text) = message {
                                let body: Value = serde_json::from_str(&text).unwrap();
                                seen.lock().unwrap().push(body);
                                socket
                                    .send(Message::Text("{\"type\":\"session.updated\"}".into()))
                                    .await
                                    .unwrap();
                            }
                        }
                    })
                },
            ),
        )
        .with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
    let mut request = format!("ws://{}/v1/realtime?model=gpt-realtime", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer local-a"));
    request.headers_mut().insert(
        "x-session-id",
        HeaderValue::from_static("raw-header-session"),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(host.capacity.available_permits(), 0);
    socket.send(UpstreamMessage::Text(json!({"type":"session.update","session":{"id":"raw-body-session","model":"gpt-realtime"}}).to_string().into())).await.unwrap();
    let _ = socket.next().await.unwrap().unwrap();
    let body = seen.lock().unwrap()[0].clone();
    assert_ne!(body["session"]["id"], "raw-body-session");
    assert_eq!(body["session"]["model"], "gpt-realtime");
    socket
        .send(UpstreamMessage::Text(
            json!({"type":"session.update","session":{"model":"unauthorized-model"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap();
    drop(socket);
    wait_free(&host).await;
    assert_eq!(seen.lock().unwrap().len(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn ephemeral_realtime_preset_is_sent_before_client_frames_and_strips_local_fields() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route(
            "/v1/realtime",
            get(
                |ws: WebSocketUpgrade, State(seen): State<Arc<Mutex<Vec<Value>>>>| async move {
                    ws.on_upgrade(move |mut socket| async move {
                        if let Some(Ok(Message::Text(text))) = socket.recv().await {
                            seen.lock()
                                .unwrap()
                                .push(serde_json::from_str(&text).unwrap());
                            socket
                                .send(Message::Text(
                                    json!({"type":"session.updated"}).to_string().into(),
                                ))
                                .await
                                .unwrap();
                            while socket.recv().await.is_some() {}
                        }
                    })
                },
            ),
        )
        .with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
    let secret = client().post(format!("http://{}/v1/realtime/client_secrets", handle.addr))
        .bearer_auth("local-a").json(&json!({"session":{"model":"gpt-realtime","id":"local-id","object":"private","expires_at":42,"client_secret":"must-not-leak","instructions":"speak briefly","audio":{"output":{"voice":"alloy"}}}}))
        .send().await.unwrap().json::<Value>().await.unwrap()["value"].as_str().unwrap().to_owned();
    wait_free(&host).await;
    let mut request = format!("ws://{}/v1/realtime", handle.addr)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {secret}")).unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(response.to_text().unwrap().contains("session.updated"));
    let preset = seen.lock().unwrap()[0].clone();
    assert_eq!(preset["type"], "session.update");
    assert_eq!(preset["session"]["instructions"], "speak briefly");
    assert_eq!(preset["session"]["audio"]["output"]["voice"], "alloy");
    for field in ["model", "id", "object", "expires_at", "client_secret"] {
        assert!(preset["session"].get(field).is_none(), "{field}");
    }
    socket.close(None).await.unwrap();
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn ephemeral_session_defaults_match_go_and_credentials_are_not_cacheable() {
    let (handle, host, upstream, root) = fixture(Router::new(), true, Options::default()).await;
    for body in [
        json!({"session":null}),
        json!({"session":{"type":" ","model":""}}),
        json!({"expires_after":{"anchor":"","seconds":60}}),
    ] {
        let response = client()
            .post(format!("http://{}/v1/realtime/client_secrets", handle.addr))
            .bearer_auth("local-a")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let secret = response.json::<Value>().await.unwrap();
        assert_eq!(secret["session"]["type"], "realtime");
        assert_eq!(secret["session"]["model"], "gpt-realtime");
        assert!(secret["value"].as_str().unwrap().starts_with("ek_native_"));
        assert_eq!(
            host.selections.lock().unwrap().last().unwrap().model,
            "gpt-realtime"
        );
        wait_free(&host).await;
    }
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_timeouts_release_account_admission_and_unsupported_capabilities_remain_501() {
    let app = Router::new().route(
        "/v1/alpha/search",
        post(|| async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            Json(json!({"output":"late"}))
        }),
    );
    let (handle, host, upstream, root) = fixture(
        app,
        false,
        Options {
            request_timeout: Duration::from_millis(75),
            ..Options::default()
        },
    )
    .await;
    // Body-read and inference opening are separate settings. The upstream below
    // stalls after the complete client body was accepted and an account admitted.
    *host.execution_policy.lock().unwrap() = ExecutionPolicy {
        open_timeout: Duration::from_millis(75),
        total_timeout: Duration::from_secs(2),
        ..ExecutionPolicy::default()
    };
    let response = client()
        .post(format!("http://{}/v1/alpha/search", handle.addr))
        .bearer_auth("local-a")
        .json(&json!({"id":"id","model":"client-model"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 504);
    wait_free(&host).await;
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    assert_eq!(host.reports.lock().unwrap()[0].account_id, "account-a");
    assert_eq!(
        host.reports.lock().unwrap()[0].error.as_deref(),
        Some("upstream_timeout")
    );
    for path in [
        "/v1/realtime/transcription_sessions",
        "/v1/realtime/translations/client_secrets",
        "/v1/realtime/calls/rtc_fixture/accept",
    ] {
        assert_eq!(
            client()
                .post(format!("http://{}{path}", handle.addr))
                .bearer_auth("local-a")
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            501
        );
    }
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_call_sideband_reuses_admission_and_hangup_closes_connection() {
    let app = Router::new()
        .route(
            "/v1/realtime/calls",
            post(|| async {
                (
                    [(header::LOCATION, "/v1/realtime/calls/rtc_sideband")],
                    "v=answer",
                )
            }),
        )
        .route(
            "/v1/realtime/calls/rtc_sideband",
            get(|ws: WebSocketUpgrade| async {
                ws.on_upgrade(
                    |mut socket| async move { while let Some(Ok(_)) = socket.recv().await {} },
                )
            }),
        )
        .route(
            "/v1/realtime/calls/rtc_sideband/hangup",
            post(|| async { axum::http::StatusCode::OK }),
        );
    let (handle, host, upstream, root) = fixture(app, false, Options::default()).await;
    let call = client()
        .post(format!("http://{}/v1/realtime/calls", handle.addr))
        .bearer_auth("local-a")
        .json(&json!({"sdp":"v=0","session":{"model":"client-model"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(call.status(), 200);
    let _ = call.bytes().await.unwrap();
    assert_eq!(host.capacity.available_permits(), 0);
    // A failed upgrade must leave the call available for a valid retry.
    let invalid = client()
        .get(format!(
            "http://{}/v1/realtime/calls/rtc_sideband",
            handle.addr
        ))
        .bearer_auth("local-a")
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 400);
    let mut request = format!("ws://{}/v1/realtime/calls/rtc_sideband", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer local-a"));
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(host.capacity.available_permits(), 0);
    let response = client()
        .post(format!(
            "http://{}/v1/realtime/calls/rtc_sideband/hangup",
            handle.addr
        ))
        .bearer_auth("local-a")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _ = response.bytes().await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap();
    drop(socket);
    wait_free(&host).await;
    assert!(host
        .selections
        .lock()
        .unwrap()
        .iter()
        .skip(1)
        .all(|request| !request.requires_permit
            && request.preferred_account_id.as_deref() == Some("account-a")));
    cleanup(handle, upstream, root).await;
}

#[test]
fn auxiliary_call_location_and_sideband_styles_match_official_protocols() {
    for (location, id) in [
        ("/v1/live/rtc_a", "rtc_a"),
        ("/v1/realtime?intent=quicksilver&call_id=rtc_b", "rtc_b"),
        ("https://example.invalid/backend-api/codex/rtc_c", "rtc_c"),
    ] {
        assert_eq!(call_id_from_location(location).as_deref(), Some(id));
    }
    for location in [
        "/v1/realtime/calls",
        "/v1/live",
        "/v1/realtime?call_id=..%2Fbad",
    ] {
        assert!(call_id_from_location(location).is_none());
    }
}

#[tokio::test]
async fn oauth_transcription_conversion_preserves_success_status_and_safe_headers() {
    for format in ["text", "json"] {
        let app = Router::new().route(
            "/v1/transcribe",
            post(|| async {
                (
                    axum::http::StatusCode::CREATED,
                    [
                        ("content-type", "application/json"),
                        ("x-request-id", "audio-output-id"),
                        ("set-cookie", "private=1"),
                    ],
                    Json(json!({"text":"transcript","upstream_only":"discard"})),
                )
            }),
        );
        let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
        let response = client()
            .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
            .bearer_auth("local-a")
            .multipart(
                reqwest::multipart::Form::new()
                    .text("response_format", format)
                    .part(
                        "file",
                        reqwest::multipart::Part::bytes(vec![1, 2, 3]).file_name("voice.wav"),
                    ),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(response.headers()["x-request-id"], "audio-output-id");
        assert!(!response.headers().contains_key("set-cookie"));
        let mime = response.headers()["content-type"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = response.text().await.unwrap();
        if format == "text" {
            assert_eq!(mime, "text/plain; charset=utf-8");
            assert_eq!(body, "transcript");
        } else {
            assert_eq!(mime, "application/json");
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap(),
                json!({"text":"transcript"})
            );
        }
        assert_eq!(host.reports.lock().unwrap()[0].status, 201);
        cleanup(handle, upstream, root).await;
    }
}

#[tokio::test]
async fn realtime_client_secret_capacity_error_has_retry_after() {
    let (handle, host, upstream, root) = fixture(Router::new(), true, Options::default()).await;
    for index in 0..65 {
        let response = client()
            .post(format!("http://{}/v1/realtime/client_secrets", handle.addr))
            .bearer_auth("local-a")
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        if index == 64 {
            assert_eq!(response.status(), 429);
            assert_eq!(response.headers()["retry-after"], "1");
            assert_eq!(
                response.json::<Value>().await.unwrap()["error"]["code"],
                "realtime_client_secret_capacity_exhausted"
            );
        } else {
            assert_eq!(response.status(), 200);
        }
        wait_free(&host).await;
    }
    cleanup(handle, upstream, root).await;
}
