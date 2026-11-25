//! Real native HTTP gateway and loopback TCP failures, with synthetic accounts only.
//! Every case owns its identity database, listener, policy, and admission slots.
use super::*;
use crate::modules::codex_native_api::Handle;
use std::sync::atomic::AtomicBool;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

const CANDIDATES: usize = 45;
const WATCHDOG: Duration = Duration::from_secs(5);

struct RoutePermit(Arc<AtomicUsize>);
impl Drop for RoutePermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct NetworkHost {
    inner: FixtureHost,
    oauth: bool,
    selections: Mutex<Vec<(usize, bool)>>,
    permits: Arc<AtomicUsize>,
    stall_replacement: AtomicBool,
}

impl Host for NetworkHost {
    fn authorize<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(token)
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        self.inner.execution_policy()
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            if !request.excluded_account_ids.is_empty()
                && self.stall_replacement.load(Ordering::SeqCst)
            {
                std::future::pending::<()>().await;
            }
            let index = (0..CANDIDATES)
                .find(|index| {
                    let id = index.to_string();
                    request
                        .preferred_account_id
                        .as_ref()
                        .is_none_or(|p| p == &id)
                        && !request.excluded_account_ids.contains(&id)
                })
                .ok_or_else(|| {
                    ServiceError::new(503, "no_available_account", "Fixture pool exhausted")
                })?;
            self.selections
                .lock()
                .unwrap()
                .push((index, request.force_refresh));
            // Reuse the established fixture's Route contract, extending only
            // its candidate identities and attaching an observable permit.
            let mut local = request;
            local.preferred_account_id = Some("0".into());
            local.excluded_account_ids.clear();
            let mut route = self.inner.route(caller, local).await?;
            route.account_id = index.to_string();
            route.owner_id = format!("owner-{index}");
            route.base_url = format!("{}/{index}", self.inner.base);
            route.chatgpt = self.oauth;
            self.permits.fetch_add(1, Ordering::SeqCst);
            route.permit = Some(Arc::new(RoutePermit(self.permits.clone())));
            Ok(route)
        })
    }
    fn route_failed<'a>(
        &'a self,
        account: &'a str,
        status: u16,
        wait: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        self.inner.route_failed(account, status, wait)
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(report)
    }
}

#[derive(Clone)]
enum Reply {
    Disconnect,
    NoHeaders,
    Json {
        status: u16,
        body: Value,
        delay: Duration,
    },
    Stream {
        header_delay: Duration,
        gap: Duration,
        chunks: usize,
    },
}

fn success() -> Reply {
    Reply::Stream {
        header_delay: Duration::ZERO,
        gap: Duration::ZERO,
        chunks: 1,
    }
}

fn rejection(status: u16, code: &str) -> Reply {
    Reply::Json {
        status,
        body: json!({"error":{"code":code,"message":"Synthetic upstream rejection"}}),
        delay: Duration::ZERO,
    }
}

// Read the complete request before simulating a network loss. This exercises
// the gateway's dispatched-request transport errors, not a fake status code.
async fn read_request(socket: &mut TcpStream) -> std::io::Result<usize> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break at + 4;
        }
        let mut block = [0u8; 4096];
        let count = socket.read(&mut block).await?;
        if count == 0 || bytes.len() > 128 * 1024 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&block[..count]);
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let index = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .split('/')
        .nth(1)
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
        .unwrap_or(0);
    while bytes.len() - header_end < length {
        let mut block = [0u8; 4096];
        let count = socket.read(&mut block).await?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&block[..count]);
    }
    Ok(index)
}

async fn serve_reply(mut socket: TcpStream, reply: Reply) -> std::io::Result<()> {
    match reply {
        Reply::Disconnect => {}
        Reply::NoHeaders => {
            // End when the gateway drops its cancelled/timed-out connection.
            let mut byte = [0];
            let _ = socket.read(&mut byte).await?;
        }
        Reply::Json {
            status,
            body,
            delay,
        } => {
            tokio::time::sleep(delay).await;
            let body = body.to_string();
            let headers = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            socket.write_all(headers.as_bytes()).await?;
            socket.write_all(body.as_bytes()).await?;
        }
        Reply::Stream {
            header_delay,
            gap,
            chunks,
        } => {
            tokio::time::sleep(header_delay).await;
            let mut frames = vec![format!("data: {}\n\n", created())];
            for index in 0..chunks {
                frames.push(format!(
                    "data: {}\n\n",
                    json!({
                        "type":"response.output_text.delta", "output_index":0,
                        "content_index":0, "delta":format!("piece-{index};")
                    })
                ));
            }
            frames.push(format!("data: {}\n\n", completed()));
            let length: usize = frames.iter().map(String::len).sum();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").as_bytes()).await?;
            for (index, frame) in frames.into_iter().enumerate() {
                if index > 1 {
                    tokio::time::sleep(gap).await;
                }
                socket.write_all(frame.as_bytes()).await?;
                socket.flush().await?;
            }
        }
    }
    Ok(())
}

struct Fixture {
    handle: Handle,
    host: Arc<NetworkHost>,
    seen: Arc<Mutex<Vec<usize>>>,
    upstream: JoinHandle<()>,
    _temp: tempfile::TempDir,
}

impl Fixture {
    async fn start(
        policy: ExecutionPolicy,
        oauth: bool,
        reply: impl Fn(usize, usize) -> Reply + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let received = seen.clone();
        let reply = Arc::new(reply);
        let upstream = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, peer) = accepted.unwrap();
                        assert!(peer.ip().is_loopback());
                        let received = received.clone();
                        let reply = reply.clone();
                        connections.spawn(async move {
                            let Ok(account) = read_request(&mut socket).await else { return; };
                            let sequence = {
                                let mut requests = received.lock().unwrap();
                                let sequence = requests.len();
                                requests.push(account);
                                sequence
                            };
                            let _ = serve_reply(socket, reply(sequence, account)).await;
                        });
                    },
                    result = connections.join_next(), if !connections.is_empty() => {
                        result.unwrap().unwrap();
                    },
                }
            }
        });
        let host = Arc::new(NetworkHost {
            inner: FixtureHost {
                base,
                policy,
                failing: 0,
                before_error: vec![],
                recover_on_same: false,
                delayed_opening: 0,
                release_error: tokio::sync::Notify::new(),
                calls: Default::default(),
                requests: Mutex::new(vec![]),
                failures: Mutex::new(vec![]),
                reports: Mutex::new(vec![]),
            },
            oauth,
            selections: Mutex::new(vec![]),
            permits: Arc::new(AtomicUsize::new(0)),
            stall_replacement: AtomicBool::new(false),
        });
        let temp = tempfile::tempdir().unwrap();
        let options = Options {
            max_in_flight: 1,
            request_timeout: Duration::from_secs(3),
            idle_timeout: Duration::from_secs(3),
            ..Default::default()
        };
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            temp.path().join("identity.sqlite"),
            options,
        )
        .await
        .unwrap();
        Self {
            handle,
            host,
            seen,
            upstream,
            _temp: temp,
        }
    }

    async fn request(&self, streaming: bool) -> (u16, String) {
        tokio::time::timeout(WATCHDOG, async {
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(format!("http://{}/v1/responses", self.handle.addr))
                .bearer_auth("synthetic-fixture")
                .json(&json!({"model":"model","input":[],"stream":streaming}))
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            (status, response.text().await.unwrap())
        })
        .await
        .expect("loopback request exceeded its independent watchdog")
    }

    async fn wait_idle(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.handle.active_requests() != 0
                || self.host.permits.load(Ordering::SeqCst) != 0
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("request and account slots must be released");
    }

    async fn finish(mut self) {
        self.wait_idle().await;
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.upstream.abort();
        let _ = self.upstream.await;
    }

    async fn wait_reports(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.host.inner.reports.lock().unwrap().len() < count {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("final report persistence must settle");
    }
}

fn policy() -> ExecutionPolicy {
    ExecutionPolicy {
        total_timeout: Duration::ZERO,
        open_timeout: Duration::from_secs(2),
        idle_timeout: Duration::from_secs(2),
        credential_retries: 0,
        send_retries: 0,
        status_retries: 0,
        retry_base_delay: Duration::from_millis(1),
        network_recovery_timeout: Duration::from_secs(3),
        network_recovery_max_failures: 4,
        ..Default::default()
    }
}

fn exhausted(status: u16, body: &str, failures: u64) {
    assert_eq!(status, 504, "{body}");
    let value: Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        value["error"]["code"], "network_recovery_exhausted",
        "{body}"
    );
    assert_eq!(value["error"]["transport_failures"], failures, "{body}");
    assert_eq!(value["error"]["retryable"], false, "{body}");
}

#[tokio::test]
async fn network_recovery_stops_45_candidates_at_shared_failure_cap() {
    // Each account has its own send retry. Rotating to another account must
    // not replenish the enclosing request's transport failure allowance.
    let fixture = Fixture::start(
        ExecutionPolicy {
            send_retries: 1,
            ..policy()
        },
        false,
        |_, _| Reply::Disconnect,
    )
    .await;
    let started = tokio::time::Instant::now();
    let (status, body) = fixture.request(false).await;
    exhausted(status, &body, 4);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(*fixture.seen.lock().unwrap(), vec![0, 0, 1, 1]);
    assert!(
        fixture.host.inner.failures.lock().unwrap().is_empty(),
        "transport loss must not cool credentials"
    );
    fixture.wait_idle().await;
    fixture.wait_reports(1).await;
    let reports = fixture.host.inner.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, 504);
    assert_eq!(reports[0].error.as_deref(), Some("network_recovery_exhausted"));
    assert!(!reports[0].credential_scope);
    assert!(!reports[0].success);
    assert_eq!(reports[0].diagnostics.as_ref().unwrap().attempts.len(), 4);
    for attempt in &reports[0].diagnostics.as_ref().unwrap().attempts {
        assert_eq!(attempt.status, Some(502));
        assert_eq!(attempt.transport_error.as_deref(), Some("network"));
        assert_ne!(attempt.error_category.as_deref(), Some("network_recovery_exhausted"));
    }
    assert!(reports[0].diagnostics.as_ref().unwrap().payloads.is_empty());
    drop(reports);
    fixture.finish().await;
}

#[tokio::test]
async fn network_recovery_deadline_interrupts_retry_waiting_for_headers() {
    let fixture = Fixture::start(
        ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(150),
            network_recovery_max_failures: 45,
            ..policy()
        },
        false,
        |sequence, _| {
            if sequence == 0 {
                Reply::Disconnect
            } else {
                Reply::NoHeaders
            }
        },
    )
    .await;
    let started = tokio::time::Instant::now();
    let (status, body) = fixture.request(false).await;
    exhausted(status, &body, 1);
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "per-attempt 2s opening limit must be capped by the armed recovery window"
    );
    assert_eq!(*fixture.seen.lock().unwrap(), vec![0, 1]);
    fixture.finish().await;
}

#[tokio::test]
async fn network_recovery_deadline_also_bounds_stalled_replacement_routing() {
    let fixture = Fixture::start(
        ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(150),
            ..policy()
        },
        false,
        |_, _| Reply::Disconnect,
    )
    .await;
    fixture.host.stall_replacement.store(true, Ordering::SeqCst);
    let started = tokio::time::Instant::now();
    let (status, body) = fixture.request(false).await;
    exhausted(status, &body, 1);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(*fixture.seen.lock().unwrap(), vec![0]);
    fixture.finish().await;
}

#[tokio::test]
async fn network_recovery_can_succeed_before_exhaustion() {
    let fixture = Fixture::start(
        ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(300),
            ..policy()
        },
        false,
        |sequence, _| {
            if sequence < 2 {
                Reply::Disconnect
            } else {
                success()
            }
        },
    )
    .await;
    let (status, body) = fixture.request(true).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("response.completed"), "{body}");
    assert!(!body.contains("network_recovery_exhausted"), "{body}");
    assert_eq!(*fixture.seen.lock().unwrap(), vec![0, 1, 2]);
    fixture.finish().await;
}

#[tokio::test]
async fn network_recovery_window_is_not_a_healthy_first_attempt_deadline() {
    let fixture = Fixture::start(
        ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(80),
            ..policy()
        },
        false,
        |_, _| Reply::Stream {
            header_delay: Duration::from_millis(300),
            gap: Duration::ZERO,
            chunks: 1,
        },
    )
    .await;
    let started = tokio::time::Instant::now();
    let (status, body) = fixture.request(true).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("response.completed"), "{body}");
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert_eq!(*fixture.seen.lock().unwrap(), vec![0]);
    fixture.finish().await;
}

#[tokio::test]
async fn network_recovery_does_not_truncate_successful_long_stream() {
    let fixture = Fixture::start(
        ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(120),
            ..policy()
        },
        false,
        |sequence, _| {
            if sequence == 0 {
                Reply::Disconnect
            } else {
                Reply::Stream {
                    header_delay: Duration::ZERO,
                    gap: Duration::from_millis(80),
                    chunks: 6,
                }
            }
        },
    )
    .await;
    let started = tokio::time::Instant::now();
    let (status, body) = fixture.request(true).await;
    assert_eq!(status, 200, "{body}");
    assert!(started.elapsed() >= Duration::from_millis(480));
    for index in 0..6 {
        assert!(body.contains(&format!("piece-{index};")), "{body}");
    }
    assert!(body.contains("response.completed"), "{body}");
    assert!(!body.contains("network_recovery_exhausted"), "{body}");
    assert_eq!(*fixture.seen.lock().unwrap(), vec![0, 1]);
    fixture.finish().await;
}

#[tokio::test]
async fn network_recovery_cancel_releases_global_and_account_slots() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let recovered = cancelled.clone();
    let fixture = Fixture::start(
        ExecutionPolicy {
            immediate_sse: true,
            keepalive: Duration::from_millis(10),
            network_recovery_timeout: Duration::from_secs(3),
            ..policy()
        },
        false,
        move |sequence, _| {
            if recovered.load(Ordering::SeqCst) {
                success()
            } else if sequence == 0 {
                Reply::Disconnect
            } else {
                Reply::NoHeaders
            }
        },
    )
    .await;
    let mut socket = TcpStream::connect(fixture.handle.addr).await.unwrap();
    let body = json!({"model":"model","input":[],"stream":true}).to_string();
    socket.write_all(format!("POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer synthetic-fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.seen.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("retry must be in flight before downstream cancellation");
    assert_eq!(fixture.handle.active_requests(), 1);
    assert!(fixture.host.permits.load(Ordering::SeqCst) > 0);
    drop(socket);
    tokio::time::timeout(Duration::from_secs(1), fixture.wait_idle())
        .await
        .expect("downstream cancellation must release slots before the 3s recovery deadline");
    cancelled.store(true, Ordering::SeqCst);
    let (status, body) = fixture.request(false).await;
    assert_eq!(
        status, 200,
        "released capacity must admit another request: {body}"
    );
    assert!(body.contains("completed"), "{body}");
    fixture.wait_idle().await;
    fixture.wait_reports(2).await;
    let reports = fixture.host.inner.reports.lock().unwrap();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].status, 499);
    assert!(reports[1].success);
    drop(reports);
    fixture.finish().await;
}

#[tokio::test]
async fn network_recovery_new_request_gets_a_fresh_budget() {
    let fixture = Fixture::start(policy(), false, |sequence, _| {
        if sequence < 7 {
            Reply::Disconnect
        } else {
            success()
        }
    })
    .await;
    let (status, body) = fixture.request(false).await;
    exhausted(status, &body, 4);
    fixture.wait_idle().await;
    let (status, body) = fixture.request(true).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("response.completed"), "{body}");
    assert_eq!(*fixture.seen.lock().unwrap(), vec![0, 1, 2, 3, 0, 1, 2, 3]);
    fixture.finish().await;
}

#[tokio::test]
async fn network_recovery_auth_and_quota_do_not_arm_transport_budget() {
    for (status, code) in [(401, "invalid_token"), (429, "usage_limit_reached")] {
        let fixture = Fixture::start(
            ExecutionPolicy {
                network_recovery_max_failures: 1,
                network_recovery_timeout: Duration::from_millis(50),
                ..policy()
            },
            true,
            move |sequence, _| {
                if sequence == 0 {
                    rejection(status, code)
                } else {
                    Reply::Stream {
                        header_delay: Duration::from_millis(180),
                        gap: Duration::ZERO,
                        chunks: 1,
                    }
                }
            },
        )
        .await;
        let (actual, body) = fixture.request(true).await;
        assert_eq!(actual, 200, "after {status}: {body}");
        assert!(
            body.contains("response.completed"),
            "after {status}: {body}"
        );
        assert!(!body.contains("network_recovery_exhausted"), "{body}");
        let expected = if status == 401 {
            vec![0, 0]
        } else {
            vec![0, 1]
        };
        assert_eq!(*fixture.seen.lock().unwrap(), expected);
        assert_eq!(
            fixture
                .host
                .selections
                .lock()
                .unwrap()
                .iter()
                .any(|(_, refresh)| *refresh),
            status == 401
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn network_recovery_valid_auth_response_ends_previous_network_episode() {
    let fixture = Fixture::start(
        ExecutionPolicy {
            network_recovery_max_failures: 2,
            ..policy()
        },
        true,
        |sequence, _| match sequence {
            0 | 2 | 3 => Reply::Disconnect,
            1 => rejection(401, "invalid_token"),
            _ => success(),
        },
    )
    .await;
    let (status, body) = fixture.request(false).await;
    exhausted(status, &body, 2);
    assert_eq!(*fixture.seen.lock().unwrap(), vec![0, 1, 1, 2]);
    assert!(fixture
        .host
        .selections
        .lock()
        .unwrap()
        .iter()
        .any(|(_, refresh)| *refresh));
    fixture.finish().await;
}

#[tokio::test]
async fn network_recovery_quota_response_clears_timer_before_slow_healthy_route() {
    let fixture = Fixture::start(
        ExecutionPolicy {
            network_recovery_timeout: Duration::from_millis(120),
            network_recovery_max_failures: 2,
            ..policy()
        },
        true,
        |sequence, _| match sequence {
            0 => Reply::Disconnect,
            1 => rejection(429, "usage_limit_reached"),
            _ => Reply::Stream {
                header_delay: Duration::from_millis(400),
                gap: Duration::ZERO,
                chunks: 1,
            },
        },
    )
    .await;
    let started = tokio::time::Instant::now();
    let (status, body) = fixture.request(true).await;
    assert_eq!(status, 200, "{body}");
    assert!(started.elapsed() >= Duration::from_millis(400));
    assert!(body.contains("response.completed"), "{body}");
    assert!(!body.contains("network_recovery_exhausted"), "{body}");
    assert_eq!(*fixture.seen.lock().unwrap(), vec![0, 1, 2]);
    fixture.wait_reports(1).await;
    let reports = fixture.host.inner.reports.lock().unwrap();
    assert_eq!(reports[0].status, 200);
    assert!(reports[0].success);
    drop(reports);
    fixture.finish().await;
}
