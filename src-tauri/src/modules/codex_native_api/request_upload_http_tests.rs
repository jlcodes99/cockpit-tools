//! Real HTTP/1 and HTTP/2 receiver backpressure, using synthetic bytes only.
//! The sender never sleeps or invents progress. The receiver controls its read
//! rate, so the tests exercise reqwest/hyper's actual body polling behavior.
use super::*;
use crate::modules::codex_native_api::{deadline::Deadline, request_upload::Opening};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicUsize;
use tokio::{sync::Notify, task::JoinSet, time::Instant};

const MIB: usize = 1024 * 1024;
const LARGE: usize = 24 * MIB;
const OPEN: Duration = Duration::from_millis(1500);
const WATCHDOG: Duration = Duration::from_secs(12);

#[derive(Clone, Default)]
struct Behavior {
    bytes_per_second: Option<usize>,
    header_delay: Duration,
    body_delay: Duration,
    never_read: bool,
    first_error: Option<(u16, &'static str, &'static str)>,
    redirect: bool,
    early_headers: bool,
}

#[derive(Clone)]
struct Capture {
    bytes: Vec<u8>,
    length: Option<String>,
    encoding: Option<String>,
    version: axum::http::Version,
}

struct Fixture {
    addr: SocketAddr,
    client: reqwest::Client,
    task: JoinHandle<()>,
    calls: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    arrived: Arc<Notify>,
    captures: Arc<Mutex<Vec<Capture>>>,
}

struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Dropping the owning JoinSet also cancels every accepted connection.
        self.task.abort();
    }
}

impl Fixture {
    async fn start(http2: bool, behavior: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let arrived = Arc::new(Notify::new());
        let captures = Arc::new(Mutex::new(Vec::new()));
        let (seen, running, notify, captured) = (
            calls.clone(),
            active.clone(),
            arrived.clone(),
            captures.clone(),
        );
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, peer) = accepted.unwrap();
                        assert!(peer.ip().is_loopback());
                        let (seen, running, notify, captured, behavior) = (seen.clone(), running.clone(), notify.clone(), captured.clone(), behavior.clone());
                        connections.spawn(async move {
                            let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                                let (seen, running, notify, captured, behavior) = (seen.clone(), running.clone(), notify.clone(), captured.clone(), behavior.clone());
                                async move {
                                    let index = seen.fetch_add(1, Ordering::SeqCst);
                                    running.fetch_add(1, Ordering::SeqCst);
                                    let _active = Active(running);
                                    notify.notify_one();
                                    let response = |status, content_type: &str, body: Body| {
                                        Response::builder().status(status).header("content-type", content_type).body(body).unwrap()
                                    };
                                    if index == 0 {
                                        if let Some((status, content_type, text)) = behavior.first_error {
                                            return Ok::<_, std::convert::Infallible>(response(status, content_type, Body::from(text)));
                                        }
                                    }
                                    let (parts, mut incoming) = request.into_parts();
                                    if behavior.early_headers {
                                        let output = Body::from_stream(futures_util::stream::once(async move {
                                            let started = Instant::now();
                                            let mut bytes = Vec::new();
                                            while let Some(frame) = incoming.frame().await {
                                                let frame = frame.map_err(std::io::Error::other)?;
                                                if let Ok(data) = frame.into_data() {
                                                    bytes.extend_from_slice(&data);
                                                    if let Some(rate) = behavior.bytes_per_second {
                                                        tokio::time::sleep_until(started + Duration::from_secs_f64(bytes.len() as f64 / rate as f64)).await;
                                                    }
                                                }
                                            }
                                            captured.lock().unwrap().push(Capture {
                                                bytes,
                                                length: parts.headers.get("content-length").map(|v| v.to_str().unwrap().to_owned()),
                                                encoding: None,
                                                version: parts.version,
                                            });
                                            Ok::<_,std::io::Error>(Bytes::from(format!("{}{}", created(), completed())))
                                        }));
                                        return Ok(response(200,"text/event-stream",output));
                                    }
                                    if behavior.never_read {
                                        std::future::pending::<()>().await;
                                    }
                                    let started = Instant::now();
                                    let mut bytes = Vec::new();
                                    while let Some(frame) = incoming.frame().await {
                                        let frame = match frame {
                                            Ok(frame) => frame,
                                            Err(_) => return Ok(response(400, "text/plain", Body::empty())),
                                        };
                                        if let Ok(data) = frame.into_data() {
                                            bytes.extend_from_slice(&data);
                                            if let Some(rate) = behavior.bytes_per_second {
                                                tokio::time::sleep_until(started + Duration::from_secs_f64(bytes.len() as f64 / rate as f64)).await;
                                            }
                                        }
                                    }
                                    captured.lock().unwrap().push(Capture {
                                        bytes,
                                        length: parts.headers.get("content-length").map(|v| v.to_str().unwrap().to_owned()),
                                        encoding: parts.headers.get("content-encoding").map(|v| v.to_str().unwrap().to_owned()),
                                        version: parts.version,
                                    });
                                    if behavior.redirect {
                                        // Complete the HTTP/1 upload before sending a final
                                        // response; closing an unread request may reset TCP.
                                        return Ok(Response::builder().status(307).header("location", "/never-follow").body(Body::empty()).unwrap());
                                    }
                                    tokio::time::sleep(behavior.header_delay).await;
                                    let delay = behavior.body_delay;
                                    let output = Body::from_stream(futures_util::stream::once(async move {
                                        tokio::time::sleep(delay).await;
                                        Ok::<_, std::io::Error>(Bytes::from(format!("{}{}", created(), completed())))
                                    }));
                                    Ok(response(200, "text/event-stream", output))
                                }
                            });
                            let io = hyper_util::rt::TokioIo::new(socket);
                            if http2 {
                                let mut builder = hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new());
                                builder.initial_stream_window_size(64 * 1024).initial_connection_window_size(64 * 1024);
                                let _ = builder.serve_connection(io, service).await;
                            } else {
                                let _ = hyper::server::conn::http1::Builder::new().serve_connection(io, service).await;
                            }
                        });
                    },
                    _ = connections.join_next(), if !connections.is_empty() => {},
                }
            }
        });
        let builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none());
        let client = if http2 {
            builder.http2_prior_knowledge()
        } else {
            builder.http1_only()
        }
        .build()
        .unwrap();
        Self {
            addr,
            client,
            task,
            calls,
            active,
            arrived,
            captures,
        }
    }

    fn request(&self, bytes: Vec<u8>) -> reqwest::Request {
        self.client
            .post(format!("http://{}/v1/responses", self.addr))
            .body(bytes)
            .build()
            .unwrap()
    }

    async fn execute(
        &self,
        size: usize,
        open: Duration,
        extra: Duration,
        total: Deadline,
    ) -> Result<reqwest::Response, ()> {
        let request = self.request(vec![b'x'; size]);
        let opening = Opening::new(Instant::now() + open, total, extra);
        let request = opening.track(request, true);
        tokio::time::timeout(WATCHDOG, opening.run(self.client.execute(request)))
            .await
            .unwrap()
            .map(|r| r.unwrap())
    }
}

#[tokio::test]
async fn upload_http_real_slow_receiver_red_green_http1_and_http2() {
    for http2 in [false, true] {
        let behavior = Behavior {
            bytes_per_second: Some(12 * MIB),
            header_delay: Duration::from_millis(100),
            ..Default::default()
        };
        let baseline = Fixture::start(http2, behavior.clone()).await;
        assert!(
            baseline
                .execute(LARGE, OPEN, Duration::ZERO, Deadline::NONE)
                .await
                .is_err(),
            "fixed opening must fail against real receiver backpressure"
        );
        assert_eq!(baseline.calls.load(Ordering::SeqCst), 1);
        assert!(
            baseline.captures.lock().unwrap().is_empty(),
            "baseline must stop before a complete body arrives"
        );
        drop(baseline);

        let fixed = Fixture::start(http2, behavior).await;
        let response = fixed
            .execute(LARGE, OPEN, OPEN, Deadline::NONE)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .text()
            .await
            .unwrap()
            .contains("response.completed"));
        assert_eq!(
            fixed.calls.load(Ordering::SeqCst),
            1,
            "extension must not add a send"
        );
        let captures = fixed.captures.lock().unwrap();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].bytes.len(), LARGE);
        assert_eq!(
            Sha256::digest(&captures[0].bytes),
            Sha256::digest(vec![b'x'; LARGE])
        );
        assert_eq!(
            captures[0].length.as_deref(),
            Some(LARGE.to_string().as_str())
        );
        assert_eq!(
            captures[0].version,
            if http2 {
                axum::http::Version::HTTP_2
            } else {
                axum::http::Version::HTTP_11
            }
        );
    }
}

#[tokio::test]
async fn upload_http_fast_body_does_not_buy_a_slow_response_header_budget() {
    let fixture = Fixture::start(
        true,
        Behavior {
            header_delay: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .await;
    let started = Instant::now();
    assert!(fixture
        .execute(
            8 * MIB,
            Duration::from_millis(200),
            Duration::from_millis(200),
            Deadline::NONE
        )
        .await
        .is_err());
    assert!(started.elapsed() < Duration::from_millis(650));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn upload_http_stalled_receiver_and_hard_cap_still_stop() {
    for never_read in [true, false] {
        let fixture = Fixture::start(
            true,
            Behavior {
                never_read,
                bytes_per_second: Some(MIB),
                ..Default::default()
            },
        )
        .await;
        let started = Instant::now();
        assert!(fixture
            .execute(
                8 * MIB,
                Duration::from_millis(200),
                Duration::from_millis(200),
                Deadline::NONE
            )
            .await
            .is_err());
        assert!(started.elapsed() < Duration::from_millis(750));
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        assert!(fixture.captures.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn upload_http_total_and_existing_network_recovery_override_progress() {
    let behavior = Behavior {
        bytes_per_second: Some(4 * MIB),
        ..Default::default()
    };
    let total = Fixture::start(true, behavior.clone()).await;
    let started = Instant::now();
    assert!(total
        .execute(
            8 * MIB,
            Duration::from_secs(1),
            Duration::from_secs(1),
            Deadline::from(started + Duration::from_millis(180))
        )
        .await
        .is_err());
    assert!(started.elapsed() < Duration::from_millis(600));
    assert_eq!(total.calls.load(Ordering::SeqCst), 1);

    let recovery = Fixture::start(true, behavior).await;
    let budget = crate::modules::codex_native_api::network_recovery::Budget::default();
    budget.observe(
        &ServiceError::new(
            504,
            "upstream_connect_timeout",
            "synthetic earlier transport failure",
        ),
        &ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(180),
            ..Default::default()
        },
    );
    let started = Instant::now();
    tokio::select! {
        _ = budget.wait() => {},
        _ = recovery.execute(8 * MIB, Duration::from_secs(1), Duration::from_secs(1), Deadline::NONE) => panic!("outer network fence must win"),
    }
    assert!(started.elapsed() < Duration::from_millis(600));
    assert!(budget.check().is_err());
    assert_eq!(recovery.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn upload_http_cancel_drops_the_inflight_body_without_retry() {
    let fixture = Fixture::start(
        true,
        Behavior {
            bytes_per_second: Some(4 * MIB),
            ..Default::default()
        },
    )
    .await;
    let request = fixture.request(vec![b'x'; LARGE]);
    let client = fixture.client.clone();
    let task = tokio::spawn(async move {
        let opening = Opening::new(Instant::now() + OPEN, Deadline::NONE, OPEN);
        opening
            .run(client.execute(opening.track(request, true)))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), fixture.arrived.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.active.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert!(fixture.captures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn upload_http_reqwest_timeout_and_redirect_policy_are_preserved() {
    let fixture = Fixture::start(
        true,
        Behavior {
            never_read: true,
            ..Default::default()
        },
    )
    .await;
    let mut request = fixture.request(vec![b'x'; 8 * MIB]);
    *request.timeout_mut() = Some(Duration::from_millis(90));
    let opening = Opening::new(
        Instant::now() + Duration::from_secs(1),
        Deadline::NONE,
        Duration::from_secs(1),
    );
    let result = opening
        .run(fixture.client.execute(opening.track(request, true)))
        .await
        .unwrap();
    assert!(result.unwrap_err().is_timeout());

    let redirect = Fixture::start(
        false,
        Behavior {
            redirect: true,
            ..Default::default()
        },
    )
    .await;
    let response = redirect
        .execute(8 * MIB, OPEN, OPEN, Deadline::NONE)
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 307);
    assert_eq!(redirect.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn upload_http_headers_end_opening_and_do_not_limit_successful_stream() {
    let fixture = Fixture::start(
        true,
        Behavior {
            body_delay: Duration::from_millis(650),
            ..Default::default()
        },
    )
    .await;
    let response = fixture
        .execute(
            8 * MIB,
            Duration::from_millis(300),
            Duration::from_millis(300),
            Deadline::NONE,
        )
        .await
        .unwrap();
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("response.completed"));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn upload_http_early_headers_do_not_park_the_remaining_request_body() {
    let fixture = Fixture::start(
        true,
        Behavior {
            early_headers: true,
            bytes_per_second: Some(4 * MIB),
            ..Default::default()
        },
    )
    .await;
    let response = fixture
        .execute(
            8 * MIB,
            Duration::from_millis(300),
            Duration::from_millis(300),
            Deadline::NONE,
        )
        .await
        .unwrap();
    let text = tokio::time::timeout(Duration::from_secs(5), response.text())
        .await
        .unwrap()
        .unwrap();
    assert!(text.contains("response.completed"));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.captures.lock().unwrap()[0].bytes.len(), 8 * MIB);
}

#[tokio::test]
async fn upload_http_only_validated_encoding_rejection_replays_plain_body() {
    for (status, content_type, error, safe) in [
        (
            415,
            "application/json",
            r#"{"error":{"code":"unsupported_content_encoding"}}"#,
            true,
        ),
        (
            400,
            "application/json",
            r#"{"error":{"code":"invalid_request","message":"generic rejection"}}"#,
            false,
        ),
        (
            400,
            "text/event-stream",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"started\"}\n\n",
            false,
        ),
    ] {
        let fixture = Fixture::start(
            true,
            Behavior {
                first_error: Some((status, content_type, error)),
                ..Default::default()
            },
        )
        .await;
        let plain = vec![b'x'; 8 * MIB];
        let compressed = zstd::stream::encode_all(plain.as_slice(), 1).unwrap();
        let mut compressed_request = fixture.request(compressed);
        compressed_request
            .headers_mut()
            .insert("content-encoding", HeaderValue::from_static("zstd"));
        let plain_request = fixture.request(plain.clone());
        assert!(plain_request.try_clone().is_some());
        let opening = Opening::new(Instant::now() + OPEN, Deadline::NONE, OPEN);
        let response = opening
            .run(async {
                let response = fixture
                    .client
                    .execute(opening.track(compressed_request, true))
                    .await
                    .unwrap();
                let (response, permitted) =
                    crate::modules::codex_native_api::http_compression::rejected_before_execution(
                        response,
                    )
                    .await
                    .unwrap();
                assert_eq!(permitted, safe);
                if permitted {
                    drop(response);
                    fixture
                        .client
                        .execute(opening.track(plain_request, true))
                        .await
                        .unwrap()
                } else {
                    response
                }
            })
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), if safe { 200 } else { status });
        assert_eq!(
            fixture.calls.load(Ordering::SeqCst),
            if safe { 2 } else { 1 }
        );
        if safe {
            let captures = fixture.captures.lock().unwrap();
            assert_eq!(captures.len(), 1);
            assert_eq!(Sha256::digest(&captures[0].bytes), Sha256::digest(&plain));
            assert!(captures[0].encoding.is_none());
        }
    }
}

struct UploadHost {
    inner: Arc<TestHost>,
    client: reqwest::Client,
}
impl Host for UploadHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(key)
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(report)
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                open_timeout: OPEN,
                credential_retries: 1,
                ..Default::default()
            })
        })
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let mut route = self.inner.route(caller, request).await?;
            route.client = self.client.clone();
            Ok(route)
        })
    }
}

#[tokio::test]
async fn upload_http_real_gateway_uses_progress_before_first_response() {
    let fixture = Fixture::start(
        true,
        Behavior {
            bytes_per_second: Some(12 * MIB),
            header_delay: Duration::from_millis(100),
            ..Default::default()
        },
    )
    .await;
    let inner = Arc::new(TestHost {
        base_url: format!("http://{}/v1", fixture.addr),
        chatgpt: true,
        proxy_url: Mutex::new(None),
        ws_enabled: AtomicBool::new(false),
        key_enabled: AtomicBool::new(true),
        change_owner_on_refresh: AtomicBool::new(false),
        route_by_model: AtomicBool::new(false),
        bridge_with_policy: AtomicBool::new(false),
        reports: Mutex::new(Vec::new()),
        reasoning_ceiling: Mutex::new(None),
        rejections: Mutex::new(Vec::new()),
        diagnostics: Mutex::new(Vec::new()),
        selections: Mutex::new(Vec::new()),
        capacity: Arc::new(tokio::sync::Semaphore::new(1)),
    });
    let host = Arc::new(UploadHost {
        inner: inner.clone(),
        client: fixture.client.clone(),
    });
    let temp = tempfile::tempdir().unwrap();
    let mut gateway = start(
        "127.0.0.1:0".parse().unwrap(),
        host,
        temp.path().join("identity.json"),
        Options::default(),
    )
    .await
    .unwrap();
    let input = "x".repeat(LARGE);
    let response = client()
        .post(format!("http://{}/v1/responses", gateway.addr))
        .bearer_auth("local-test-key")
        .json(&json!({"model":"test-model","stream":true,"input":input}))
        .timeout(WATCHDOG)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(text.contains("response.completed"));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    let capture = fixture.captures.lock().unwrap()[0].clone();
    let received: Value = serde_json::from_slice(&capture.bytes).unwrap();
    // The native wire canonicalizes string input to a user/input_text item.
    // Compare its text bytes, not the distinct source and canonical JSON shapes.
    let text = received["input"][0]["content"][0]["text"].as_str().unwrap();
    assert_eq!(received["input"].as_array().unwrap().len(), 1);
    assert_eq!(received["input"][0]["role"], "user");
    assert_eq!(
        Sha256::digest(text.as_bytes()),
        Sha256::digest(input.as_bytes())
    );
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    assert_eq!(inner.capacity.available_permits(), 1);
}
