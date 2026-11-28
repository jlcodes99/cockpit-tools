//! Explicit opt-in Windows CLI contracts. Everything except the selected client
//! executable is synthetic and local to the test run; no user profile is loaded.
#![cfg(target_os = "windows")]

use super::{contract::*, diagnostics::DiagnosticContext, start, Handle, Options};
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::post,
    Router,
};
use futures_util::{future::BoxFuture, stream, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOKEN: &str = "synthetic-stream-recovery-cli-only";
const ANSWER: &str = "STREAM_RECOVERY_CLI_OK";
const HOLD: Duration = Duration::from_millis(500);

#[path = "stream_recovery_cli_wire.rs"]
mod public_wire_decoder;

#[derive(Default)]
struct Source {
    bodies: Mutex<Vec<Vec<u8>>>,
    response_ids: Mutex<Vec<String>>,
}

fn sse(event: Value) -> String {
    format!(
        "event: {}\ndata: {event}\n\n",
        event["type"].as_str().unwrap()
    )
}

async fn upstream(State(source): State<Arc<Source>>, headers: HeaderMap, body: Bytes) -> Response {
    assert!(
        !headers.contains_key("authorization"),
        "the local client key must not reach the synthetic upstream"
    );
    let sequence = {
        let mut bodies = source.bodies.lock().unwrap();
        bodies.push(body.to_vec());
        bodies.len()
    };
    assert!(
        sequence <= 2,
        "a third upstream dispatch is never authorized by this fixture"
    );
    let id = format!("resp_cli_attempt_{sequence}");
    source.response_ids.lock().unwrap().push(id.clone());
    let created = sse(json!({"type":"response.created","sequence_number":0,
        "response":{"id":id,"object":"response","created_at":1700000000,"status":"in_progress","output":[]}}));
    if sequence == 1 {
        // The control frame is published before a real HTTP body transport error.
        // It cannot be confused with a scripted SSE error or a clean EOF.
        let chunks = stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(created)) })
            .chain(stream::once(async {
                tokio::time::sleep(HOLD).await;
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "synthetic CLI transport disconnect",
                ))
            }));
        return Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(chunks))
            .unwrap();
    }
    let message = json!({"id":"msg_cli_recovered","type":"message","role":"assistant","status":"completed",
        "content":[{"type":"output_text","text":ANSWER,"annotations":[]}]});
    let frames = [
        json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,
            "item":{"id":"msg_cli_recovered","type":"message","role":"assistant","status":"in_progress","content":[]}}),
        json!({"type":"response.output_text.delta","sequence_number":2,"item_id":"msg_cli_recovered","output_index":0,"content_index":0,"delta":ANSWER}),
        json!({"type":"response.output_item.done","sequence_number":3,"output_index":0,"item":message}),
        json!({"type":"response.completed","sequence_number":4,"response":{"id":id,"object":"response","created_at":1700000000,
            "status":"completed","output":[message],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
    ];
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from(
            created + &frames.into_iter().map(sse).collect::<String>(),
        ))
        .unwrap()
}

struct CliHost {
    base: String,
    authorizations: AtomicUsize,
    routes: AtomicUsize,
    diagnostics: Mutex<Vec<DiagnosticContext>>,
    reports: Mutex<Vec<RequestReport>>,
    policy: ExecutionPolicy,
    client: Mutex<(usize, reqwest::Client)>,
}

impl Host for CliHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            assert_eq!(
                key, TOKEN,
                "only the explicit synthetic client key is accepted"
            );
            self.authorizations.fetch_add(1, Ordering::SeqCst);
            Ok(Caller {
                id: "cli-fixture".into(),
                label: "synthetic".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            self.routes.fetch_add(1, Ordering::SeqCst);
            let (generation, client) = self.client.lock().unwrap().clone();
            Ok(Route {
                account_id: "synthetic-account".into(),
                continuity_account_id: None,
                owner_id: "synthetic-owner".into(),
                installation_profile: None,
                transport_identity: Some(format!("cli-pool-{generation}")),
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                base_url: self.base.clone(),
                model: request.model,
                headers: HeaderMap::new(),
                client,
                proxy_url: None,
                proxy_route: None,
                chatgpt: true,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn invalidate_transport(&self, identity: &str) -> bool {
        let mut state = self.client.lock().unwrap();
        if identity != format!("cli-pool-{}", state.0) {
            return false;
        }
        *state = (
            state.0 + 1,
            reqwest::Client::builder().no_proxy().build().unwrap(),
        );
        true
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
    fn diagnostic(&self, context: DiagnosticContext) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.diagnostics.lock().unwrap().push(context);
        })
    }
}

struct Fixture {
    gateway: Handle,
    host: Arc<CliHost>,
    source: Arc<Source>,
    upstream_task: tokio::task::JoinHandle<()>,
    deny_task: tokio::task::JoinHandle<()>,
    denied: Arc<AtomicUsize>,
    proxy_url: String,
    client_addr: std::net::SocketAddr,
    wire: Arc<Mutex<Vec<u8>>>,
    tap_task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new(directory: &Path, budget_exhausted: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = Arc::new(Source::default());
        let host = Arc::new(CliHost {
            base: format!("http://{}/v1", listener.local_addr().unwrap()),
            authorizations: AtomicUsize::new(0),
            routes: AtomicUsize::new(0),
            diagnostics: Mutex::new(vec![]),
            reports: Mutex::new(vec![]),
            policy: ExecutionPolicy {
                bootstrap_retries: 0,
                send_retries: 0,
                status_retries: 0,
                credential_retries: 1,
                open_timeout: Duration::from_secs(3),
                idle_timeout: Duration::from_secs(3),
                total_timeout: Duration::from_secs(8),
                network_recovery_timeout: Duration::from_secs(3),
                network_recovery_max_failures: if budget_exhausted { 1 } else { 4 },
                retry_base_delay: Duration::from_millis(1),
                ..Default::default()
            },
            client: Mutex::new((0, reqwest::Client::builder().no_proxy().build().unwrap())),
        });
        let app = Router::new()
            .route("/v1/responses", post(upstream))
            .with_state(source.clone());
        let upstream_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let gateway = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            directory.join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        // A byte-for-byte TCP tap observes the actual public stream without
        // parsing/rebuilding HTTP, changing headers, buffering frames or replaying.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client_addr = listener.local_addr().unwrap();
        let gateway_addr = gateway.addr;
        let wire = Arc::new(Mutex::new(Vec::new()));
        let tap_wire = wire.clone();
        let tap_task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((client, _)) = accepted else { break; };
                        let wire = tap_wire.clone();
                        connections.spawn(async move {
                            let gateway = tokio::net::TcpStream::connect(gateway_addr).await.unwrap();
                            let (mut client_read, mut client_write) = client.into_split();
                            let (mut gateway_read, mut gateway_write) = gateway.into_split();
                            let outbound = async {
                                let _ = tokio::io::copy(&mut client_read, &mut gateway_write).await;
                                let _ = gateway_write.shutdown().await;
                            };
                            let inbound = async {
                                let mut buffer = vec![0; 8192];
                                loop {
                                    let count = match gateway_read.read(&mut buffer).await { Ok(0) | Err(_) => break, Ok(count) => count };
                                    if client_write.write_all(&buffer[..count]).await.is_err() { break; }
                                    wire.lock().unwrap().extend_from_slice(&buffer[..count]);
                                }
                                let _ = client_write.shutdown().await;
                            };
                            tokio::join!(outbound, inbound);
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", listener.local_addr().unwrap());
        let denied = Arc::new(AtomicUsize::new(0));
        let counter = denied.clone();
        let deny = Router::new().fallback(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::SERVICE_UNAVAILABLE
            }
        });
        let deny_task = tokio::spawn(async move { axum::serve(listener, deny).await.unwrap() });
        Self {
            gateway,
            host,
            source,
            upstream_task,
            deny_task,
            denied,
            proxy_url,
            client_addr,
            wire,
            tap_task,
        }
    }
    async fn shutdown(&mut self) {
        let shutdown = self.gateway.shutdown(Duration::from_secs(3)).await;
        self.upstream_task.abort();
        self.deny_task.abort();
        self.tap_task.abort();
        shutdown.unwrap();
    }
}

fn explicit_client() -> (PathBuf, String, PathBuf) {
    assert_eq!(
        std::env::var("COCKPIT_STREAM_RECOVERY_CLI_OPT_IN").as_deref(),
        Ok("1"),
        "this ignored contract requires explicit opt-in"
    );
    let binary = PathBuf::from(
        std::env::var_os("COCKPIT_STREAM_RECOVERY_CODEX_BIN")
            .expect("select the exact Codex executable"),
    );
    assert!(
        binary.is_absolute() && binary.is_file(),
        "the selected Codex executable must exist at an absolute path"
    );
    let expected = std::env::var("COCKPIT_STREAM_RECOVERY_CODEX_SHA256")
        .expect("pin the selected executable SHA256");
    let actual = format!("{:x}", Sha256::digest(std::fs::read(&binary).unwrap()));
    assert!(
        actual.eq_ignore_ascii_case(&expected),
        "selected CLI hash differs from the explicit pin"
    );
    let evidence = PathBuf::from(
        std::env::var_os("COCKPIT_STREAM_RECOVERY_CLI_EVIDENCE_DIR")
            .expect("select the isolated evidence root"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    (binary, actual, evidence)
}

async fn run_case(budget_exhausted: bool) {
    use std::os::windows::process::CommandExt;
    let (binary, binary_hash, evidence) = explicit_client();
    let directory = evidence.join(if budget_exhausted {
        "budget-exhausted"
    } else {
        "metadata-recovered"
    });
    std::fs::create_dir(&directory).expect("each CLI scenario needs a new directory");
    let profile = directory.join("profile");
    let client_data = directory.join("client-data");
    let workspace = directory.join("workspace");
    let temp = directory.join("temp");
    for path in [
        &profile,
        &client_data,
        &workspace,
        &temp,
        &workspace.join(".git"),
        &profile.join("AppData/Roaming"),
        &profile.join("AppData/Local"),
    ] {
        std::fs::create_dir_all(path).unwrap();
    }
    let mut fixture = Fixture::new(&directory, budget_exhausted).await;
    let windows = std::env::var_os("SystemRoot").expect("Windows platform directory is required");
    let windows = PathBuf::from(windows);
    let configs = vec![
        "model_provider=\"local_fixture\"".to_string(),
        "model=\"gpt-5.4\"".into(),
        "model_reasoning_effort=\"low\"".into(),
        "model_providers.local_fixture.name=\"Loopback fixture\"".into(),
        format!(
            "model_providers.local_fixture.base_url=\"http://{}/v1\"",
            fixture.client_addr
        ),
        "model_providers.local_fixture.env_key=\"COCKPIT_CLI_SYNTHETIC_TOKEN\"".into(),
        "model_providers.local_fixture.wire_api=\"responses\"".into(),
        "model_providers.local_fixture.requires_openai_auth=false".into(),
        "model_providers.local_fixture.supports_websockets=false".into(),
        "model_providers.local_fixture.request_max_retries=0".into(),
        "model_providers.local_fixture.stream_max_retries=0".into(),
        "model_providers.local_fixture.stream_idle_timeout_ms=3000".into(),
        "cli_auth_credentials_store=\"file\"".into(),
        "mcp_oauth_credentials_store=\"file\"".into(),
        "apps._default.enabled=false".into(),
        "features.apps=false".into(),
        "features.plugins=false".into(),
        "orchestrator.mcp.enabled=false".into(),
        "project_doc_max_bytes=0".into(),
        "web_search=\"disabled\"".into(),
        "analytics.enabled=false".into(),
        "otel.exporter=\"none\"".into(),
        "otel.metrics_exporter=\"none\"".into(),
    ];
    let mut command = tokio::process::Command::new(&binary);
    command
        .env_clear()
        .current_dir(&workspace)
        .env("SystemRoot", &windows)
        .env("WINDIR", &windows)
        .env("COMSPEC", windows.join("System32/cmd.exe"))
        .env(
            "PATH",
            format!(
                "{};{}",
                binary.parent().unwrap().display(),
                windows.join("System32").display()
            ),
        )
        .env("USERPROFILE", &profile)
        .env("APPDATA", profile.join("AppData/Roaming"))
        .env("LOCALAPPDATA", profile.join("AppData/Local"))
        .env("TEMP", &temp)
        .env("TMP", &temp)
        // This is the application's child data-root setting, never a shell variable
        // or mutation of the invoking user's actual profile.
        .env("CODEX_HOME", &client_data)
        .env("COCKPIT_CLI_SYNTHETIC_TOKEN", TOKEN)
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("HTTP_PROXY", &fixture.proxy_url)
        .env("HTTPS_PROXY", &fixture.proxy_url)
        .env("ALL_PROXY", &fixture.proxy_url)
        .args([
            "--no-daemon",
            "exec",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--strict-config",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--color",
            "never",
            "--json",
            "-C",
        ])
        .arg(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.as_std_mut().creation_flags(0x08000000); // CREATE_NO_WINDOW for this child only.
    for config in &configs {
        command.args(["-c", config]);
    }
    command.arg("Reply OK. Do not use tools. This is an offline transport test.");
    let mut child = command
        .spawn()
        .expect("explicit Codex executable must start");
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut stdout_task = tokio::spawn(async move {
        let mut bytes = vec![];
        stdout.read_to_end(&mut bytes).await.unwrap();
        bytes
    });
    let mut stderr_task = tokio::spawn(async move {
        let mut bytes = vec![];
        stderr.read_to_end(&mut bytes).await.unwrap();
        bytes
    });
    let (status, timed_out) =
        match tokio::time::timeout(Duration::from_secs(25), child.wait()).await {
            Ok(result) => (result.unwrap(), false),
            Err(_) => {
                child.kill().await.unwrap();
                (child.wait().await.unwrap(), true)
            }
        };
    let stdout_result = tokio::time::timeout(Duration::from_secs(3), &mut stdout_task).await;
    let stderr_result = tokio::time::timeout(Duration::from_secs(3), &mut stderr_task).await;
    if stdout_result.is_err() {
        stdout_task.abort();
    }
    if stderr_result.is_err() {
        stderr_task.abort();
    }
    fixture.shutdown().await;
    let stdout = String::from_utf8(
        stdout_result
            .expect("CLI stdout pipe did not close")
            .unwrap(),
    )
    .unwrap();
    let stderr = String::from_utf8(
        stderr_result
            .expect("CLI stderr pipe did not close")
            .unwrap(),
    )
    .unwrap();
    // Persist the actual client outcome before observer decoding or assertions;
    // a failed observer must never erase useful client evidence.
    std::fs::write(directory.join("client.stdout.jsonl"), &stdout).unwrap();
    std::fs::write(directory.join("client.stderr.log"), &stderr).unwrap();
    std::fs::write(
        directory.join("client-exit.json"),
        serde_json::to_vec_pretty(&json!({
            "exitCode":status.code(),"timedOut":timed_out,"binarySha256":binary_hash
        }))
        .unwrap(),
    )
    .unwrap();
    let bodies = fixture.source.bodies.lock().unwrap();
    let ids = fixture.source.response_ids.lock().unwrap();
    let diagnostics = fixture.host.diagnostics.lock().unwrap();
    let reports = fixture.host.reports.lock().unwrap();
    let raw_wire = fixture.wire.lock().unwrap().clone();
    std::fs::write(directory.join("public-response.http"), &raw_wire).unwrap();
    let wire = public_wire_decoder::decode_public_wire(&raw_wire)
        .expect("observe framed/compressed public response");
    std::fs::write(directory.join("public-response.sse"), &wire).unwrap();
    let transmitted_created: Vec<Value> = wire
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["type"] == "response.created")
        .collect();
    let transmitted_ids: Vec<_> = transmitted_created
        .iter()
        .filter_map(|event| event["response"]["id"].as_str())
        .collect();
    let client_posts: Vec<_> = diagnostics
        .iter()
        .filter(|context| context.method == "POST" && context.endpoint == "/v1/responses")
        .collect();
    let events: Vec<Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let completed = events.iter().any(|event| event["type"] == "turn.completed");
    let terminal_failure = events.iter().any(|event| event["type"] == "turn.failed");
    let final_answer = events.iter().any(|event| {
        event["type"] == "item.completed"
            && event["item"]["type"] == "agent_message"
            && event["item"]["text"] == ANSWER
    });
    let result = json!({
        "binarySha256":binary_hash,"budgetExhausted":budget_exhausted,"configs":configs,"timedOut":timed_out,"exitCode":status.code(),
        "clientAuthorizations":fixture.host.authorizations.load(Ordering::SeqCst),"clientPostCount":client_posts.len(),
        "clientRequestIds":client_posts.iter().map(|context| &context.request_id).collect::<Vec<_>>(),
        "upstreamPostCount":bodies.len(),"upstreamResponseIds":*ids,"upstreamBodiesIdentical":bodies.len() == 2 && bodies[0] == bodies[1],
        "transmittedCreatedResponseIds":transmitted_ids,"transmittedCreatedCount":transmitted_created.len(),
        "blockedProxyRequests":fixture.denied.load(Ordering::SeqCst),"turnCompleted":completed,"turnFailed":terminal_failure,"finalAnswerMatched":final_answer,
        "reportErrors":reports.iter().map(|report| &report.error).collect::<Vec<_>>(),"stdout":stdout,"stderr":stderr,
        "limitation":"Configuration and directory isolation plus proxy observation; not an operating-system network sandbox. The installed CLI advertises its native tool namespace unchanged."
    });
    std::fs::write(
        directory.join("result.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    assert!(!timed_out, "isolated CLI timeout: {result}");
    assert_eq!(
        fixture.denied.load(Ordering::SeqCst),
        0,
        "unexpected proxy use: {result}"
    );
    // Reopening an upstream route reuses the existing caller. A second CLI POST
    // necessarily re-enters authorization and gets its own diagnostic request ID.
    assert_eq!(
        fixture.host.authorizations.load(Ordering::SeqCst),
        1,
        "client retried or made another authenticated request: {result}"
    );
    assert_eq!(
        client_posts.len(),
        1,
        "real gateway ingress must see exactly one POST: {result}"
    );
    assert_eq!(
        diagnostics.len(),
        1,
        "unexpected native API request: {result}"
    );
    if budget_exhausted {
        assert!(
            !status.success() && terminal_failure && !completed && !final_answer,
            "exhausted recovery must fail the one client turn: {result}"
        );
        assert_eq!(
            bodies.len(),
            1,
            "exhausted network budget cannot dispatch a replay: {result}"
        );
        assert!(
            reports
                .iter()
                .any(|report| report.error.as_deref() == Some("network_recovery_exhausted")),
            "terminal budget error must be preserved: {result}"
        );
        assert!(
            wire.contains("network_recovery_exhausted"),
            "budget terminal was not transmitted to the real client: {result}"
        );
    } else {
        assert!(
            status.success() && completed && final_answer,
            "actual CLI did not accept the recovered response: {result}"
        );
        assert_eq!(
            bodies.len(),
            2,
            "one bounded gateway replay is required: {result}"
        );
        assert_eq!(
            bodies[0], bodies[1],
            "upstream request bytes changed during recovery"
        );
        assert_eq!(
            &*ids,
            &[
                "resp_cli_attempt_1".to_string(),
                "resp_cli_attempt_2".to_string()
            ]
        );
        assert_ne!(ids[0], ids[1]);
        assert_eq!(
            transmitted_created.len(),
            2,
            "two distinct created frames must be transmitted to the actual CLI: {result}"
        );
        assert_eq!(transmitted_ids.len(), 2);
        assert_ne!(transmitted_ids[0], transmitted_ids[1]);
        assert!(
            reports.iter().any(|report| report.success),
            "gateway must record the successful final response"
        );
    }
}

#[tokio::test]
#[ignore = "requires explicit COCKPIT_STREAM_RECOVERY_CLI_OPT_IN, pinned Codex executable and isolated evidence directory"]
async fn actual_cli_zero_retries_accepts_two_response_ids_after_one_gateway_replay() {
    run_case(false).await;
}

#[tokio::test]
#[ignore = "requires explicit COCKPIT_STREAM_RECOVERY_CLI_OPT_IN, pinned Codex executable and isolated evidence directory"]
async fn actual_cli_zero_retries_observes_terminal_network_budget_without_reposting() {
    run_case(true).await;
}
