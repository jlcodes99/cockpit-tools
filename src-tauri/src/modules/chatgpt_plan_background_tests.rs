//! Account-scoped wakeup/benchmark requests prefer a bound independent grant.
//! Missing bindings retain the original transport; invalid bindings never fall
//! back. Every upstream and client profile is an isolated, synthetic fixture.
use super::*;
use crate::models::codex::{CodexAccount, CodexTokens};
use crate::modules::{codex_account, codex_local_access as access};

fn background_case<F, Fut>(operation: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(access::with_chatgpt_plan_test_runtime(operation));
}

fn source_account(id: &str, expired: bool) -> CodexAccount {
    let expires_at = if expired {
        1
    } else {
        chrono::Utc::now().timestamp() + 3600
    };
    let token = format!(
        "e30.{}.synthetic-original-signature",
        URL_SAFE_NO_PAD.encode(json!({"exp": expires_at}).to_string())
    );
    let mut source = CodexAccount::new(
        id.into(),
        "test@example.invalid".into(),
        CodexTokens {
            access_token: token.clone(),
            id_token: token,
            refresh_token: None,
        },
    );
    source.user_id = Some("independent-codex-user".into());
    source.account_id = Some("independent-codex-workspace".into());
    // Preserve an existing refresh-authorization failure. A still-valid original
    // access token remains usable without a binding. A bound independent grant
    // must never repair or overwrite this original credential chain.
    source.requires_reauth = true;
    source.reauth_reason = Some("synthetic original authorization requires renewal".into());
    source.egress_proxy_disabled = true;
    codex_account::save_account(&source).unwrap();
    let mut index = codex_account::load_account_index();
    index
        .accounts
        .push(crate::models::codex::CodexAccountSummary {
            id: source.id.clone(),
            email: source.email.clone(),
            plan_type: source.plan_type.clone(),
            subscription_active_until: None,
            created_at: source.created_at,
            last_used: source.last_used,
        });
    codex_account::save_account_index(&index).unwrap();
    source
}

fn assert_original_unchanged(source: &CodexAccount) {
    let saved = codex_account::load_account(&source.id).unwrap();
    assert_eq!(
        serde_json::to_value(saved).unwrap(),
        serde_json::to_value(source).unwrap()
    );
}

async fn service_snapshot() -> Value {
    let mut state = access::get_local_access_state().await.unwrap();
    // Background statistics maintenance can advance this refresh timestamp
    // without a service request. Keep every config field, window and counter
    // in the equality assertion; ignore only the aggregate refresh time.
    state.stats.updated_at = 0;
    json!({
        "collection": state.collection,
        "running": state.running,
        "preparing": state.preparing,
        "member_count": state.member_count,
        "stats": state.stats,
    })
}

async fn assert_no_service_started() {
    let state = access::get_local_access_state().await.unwrap();
    assert!(!state.running);
    assert!(!state.preparing);
    assert_eq!(state.member_count, 0);
    assert!(state
        .collection
        .is_none_or(|collection| collection.account_ids.is_empty()));
}

struct UntouchedState {
    profile: std::path::PathBuf,
    auth: Vec<u8>,
    config: Vec<u8>,
    grant_path: std::path::PathBuf,
    grant: Vec<u8>,
    service: Value,
}

impl UntouchedState {
    async fn capture(fixture: &Fixture, registration: &str) -> Self {
        let profile = codex_account::get_codex_home();
        assert!(
            profile.starts_with(&fixture.directory),
            "client profile must be isolated"
        );
        std::fs::create_dir_all(&profile).unwrap();
        let auth = br#"{"tokens":{"access_token":"synthetic-existing-client"}}"#.to_vec();
        let config = b"model_provider = \"openai\"\nmodel = \"existing-client-model\"\n".to_vec();
        std::fs::write(profile.join("auth.json"), &auth).unwrap();
        std::fs::write(profile.join("config.toml"), &config).unwrap();
        let grant_path = fixture
            .directory
            .join("chatgpt-plan")
            .join(format!("{registration}.json"));
        let grant = std::fs::read(&grant_path).unwrap();
        Self {
            profile,
            auth,
            config,
            grant_path,
            grant,
            service: service_snapshot().await,
        }
    }

    async fn assert_original_and_service_unchanged(&self, source: &CodexAccount) {
        assert_original_unchanged(source);
        assert_eq!(
            std::fs::read(self.profile.join("auth.json")).unwrap(),
            self.auth
        );
        assert_eq!(
            std::fs::read(self.profile.join("config.toml")).unwrap(),
            self.config
        );
        assert_eq!(service_snapshot().await, self.service);
    }

    async fn assert_unchanged(&self, source: &CodexAccount) {
        self.assert_original_and_service_unchanged(source).await;
        assert_eq!(std::fs::read(&self.grant_path).unwrap(), self.grant);
    }
}

fn assert_grant_unused(fixture: &Fixture) {
    let state = fixture.state.lock().unwrap();
    assert_eq!(
        state.response_started, 0,
        "background operation must not use SIWC inference"
    );
    assert!(state.response_requests.is_empty());
    assert_eq!(
        state.refresh_started, 0,
        "background operation must not refresh SIWC"
    );
    assert_eq!(state.refresh_count, 0);
    assert_eq!(
        state.model_count, 0,
        "background operation must not load SIWC models"
    );
    assert_eq!(state.revoke_count, 0);
}

fn assert_original_requests(original: &OriginalLoopback, source: &CodexAccount, prompts: &[&str]) {
    let requests = original.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        prompts.len(),
        "no generation retry or extra request"
    );
    for ((headers, body), prompt) in requests.iter().zip(prompts) {
        assert_eq!(
            headers["authorization"].to_str().unwrap(),
            format!("Bearer {}", source.tokens.access_token)
        );
        assert!(headers.contains_key("x-codex-installation-id"));
        assert!(body["client_metadata"]["x-codex-installation-id"].is_string());
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["input"][0]["content"][0]["text"], *prompt);
        assert!(body.get("tools").is_none());
    }
}

struct OriginalLoopback {
    previous_server: Option<std::ffi::OsString>,
    server: tokio::task::JoinHandle<()>,
    requests: Arc<Mutex<Vec<(axum::http::HeaderMap, Value)>>>,
}

impl Drop for OriginalLoopback {
    fn drop(&mut self) {
        self.server.abort();
        if let Some(previous) = self.previous_server.take() {
            std::env::set_var("CODEX_BACKGROUND_TEST_SERVER", previous);
        } else {
            std::env::remove_var("CODEX_BACKGROUND_TEST_SERVER");
        }
    }
}

impl OriginalLoopback {
    async fn new(status: u16, delayed: bool) -> Self {
        use axum::response::IntoResponse;
        let previous_server = std::env::var_os("CODEX_BACKGROUND_TEST_SERVER");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        std::env::set_var(
            "CODEX_BACKGROUND_TEST_SERVER",
            format!("http://{}", listener.local_addr().unwrap()),
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        // Only the original /responses endpoint exists. A wrong path cannot
        // accidentally return a successful fixture response.
        let app = axum::Router::new().route(
            "/responses",
            axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                let observed = observed.clone();
                async move {
                    observed.lock().unwrap().push((headers, body));
                    if delayed {
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    }
                    if status != 200 {
                        return (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(json!({
                            "error": {"message": "synthetic original channel rejection", "code": "original_rejected"}
                        }))).into_response();
                    }
                    let completed = json!({"type":"response.completed","response":{
                        "id":"synthetic-original-response","model":"test-model","status":"completed",
                        "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ORIGINAL"}]}],
                        "usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}
                    }});
                    ([(axum::http::header::CONTENT_TYPE.as_str(), "text/event-stream"),
                      ("x-codex-primary-used-percent", "25"),
                      ("x-codex-primary-window-minutes", "300")],
                        format!("data: {{\"type\":\"response.output_text.delta\",\"delta\":\"ORIGINAL\"}}\n\ndata: {completed}\n\n")).into_response()
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            previous_server,
            server,
            requests,
        }
    }
}

async fn run_original_pair(source: &CodexAccount, original: &OriginalLoopback) {
    let wakeup = access::run_official_wakeup_chat(
        &source.id,
        Some("test-model"),
        Some("high"),
        "original wakeup",
    )
    .await
    .unwrap();
    assert_eq!(wakeup.account.id, source.id);
    assert_eq!(
        serde_json::to_value(wakeup.account.tokens).unwrap(),
        serde_json::to_value(&source.tokens).unwrap()
    );
    assert_eq!(wakeup.reply, "ORIGINAL");
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let deltas = Arc::new(Mutex::new(Vec::new()));
    let observed = deltas.clone();
    let pelican = access::run_pelican_chat(
        &source.id,
        "test-model",
        "high",
        "original benchmark",
        cancel_rx,
        move |delta| observed.lock().unwrap().push(delta),
    )
    .await
    .unwrap();
    assert_eq!(pelican.reply, "ORIGINAL");
    assert!(!pelican.uses_official_authorization);
    assert_eq!(*deltas.lock().unwrap(), vec!["ORIGINAL"]);
    assert_eq!(
        pelican.response_id.as_deref(),
        Some("synthetic-original-response")
    );
    assert_eq!(pelican.response_model.as_deref(), Some("test-model"));
    assert_eq!(pelican.usage.unwrap()["total_tokens"], 3);
    let quota = pelican
        .quota
        .expect("original transport retains its quota headers");
    assert_eq!(quota.used_percent, 25.0);
    assert_eq!(quota.remaining_percent, 75);
    assert_eq!(quota.window_minutes, Some(300));
    assert_original_requests(original, source, &["original wakeup", "original benchmark"]);
    let requests = original.requests.lock().unwrap();
    assert_eq!(requests[0].1["instructions"], "");
    assert_eq!(
        requests[1].1["instructions"],
        access::PELICAN_DELIVERY_INSTRUCTIONS
    );
}

fn assert_official_requests(fixture: &Fixture, token: &str, prompts: &[&str]) {
    let state = fixture.state.lock().unwrap();
    assert_eq!(
        state.response_started,
        prompts.len(),
        "each generation is sent once"
    );
    assert_eq!(state.response_requests.len(), prompts.len());
    for ((headers, body), prompt) in state.response_requests.iter().zip(prompts) {
        let headers = headers.to_ascii_lowercase();
        assert!(headers.starts_with("post /v1/responses http/1.1\r\n"));
        assert!(headers.contains(&format!("authorization: bearer {token}\r\n")));
        for private_header in [
            "chatgpt-account-id:",
            "x-codex-installation-id:",
            "x-codex-turn-metadata:",
            "session-id:",
            "thread-id:",
        ] {
            assert!(
                !headers.contains(private_header),
                "private header leaked: {private_header}"
            );
        }
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["input"][0]["content"][0]["text"], *prompt);
        assert!(body.get("client_metadata").is_none());
        assert!(body.get("tools").is_none());
    }
}

fn grant_activity(fixture: &Fixture) -> (usize, usize, usize, usize, usize, usize) {
    let state = fixture.state.lock().unwrap();
    (
        state.response_started,
        state.response_requests.len(),
        state.refresh_started,
        state.refresh_count,
        state.model_count,
        state.revoke_count,
    )
}

async fn run_official_pair(
    source: &CodexAccount,
    fixture: &Fixture,
    original: &OriginalLoopback,
    token: &str,
) {
    let wakeup = access::run_official_wakeup_chat(
        &source.id,
        Some("test-model"),
        Some("high"),
        "official wakeup",
    )
    .await
    .unwrap();
    assert_eq!(wakeup.account.id, source.id);
    assert_eq!(
        serde_json::to_value(wakeup.account.tokens).unwrap(),
        serde_json::to_value(&source.tokens).unwrap()
    );
    assert_eq!(wakeup.reply, "OK");
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let deltas = Arc::new(Mutex::new(Vec::new()));
    let observed = deltas.clone();
    let pelican = access::run_pelican_chat(
        &source.id,
        "test-model",
        "high",
        "official benchmark",
        cancel_rx,
        move |delta| observed.lock().unwrap().push(delta),
    )
    .await
    .unwrap();
    assert_eq!(pelican.reply, "OK");
    assert!(pelican.uses_official_authorization);
    assert_eq!(*deltas.lock().unwrap(), vec!["OK"]);
    assert_eq!(pelican.response_id.as_deref(), Some("synthetic-response"));
    assert_eq!(pelican.response_model.as_deref(), Some("test-model"));
    assert_eq!(pelican.usage.unwrap()["total_tokens"], 3);
    assert!(
        pelican.quota.is_none(),
        "never copy the original account quota into an official reply"
    );
    assert!(original.requests.lock().unwrap().is_empty());
    assert_official_requests(fixture, token, &["official wakeup", "official benchmark"]);
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.response_requests[0].1["instructions"], "");
    assert_eq!(
        state.response_requests[1].1["instructions"],
        access::PELICAN_DELIVERY_INSTRUCTIONS
    );
}

#[test]
fn bound_background_calls_prefer_official_grant_without_service() {
    background_case(|| async {
        let fixture = Fixture::new().await;
        let original = OriginalLoopback::new(200, false).await;
        let source = source_account("background-bound-source", false);
        let registration = fixture.login().await;
        assert!(registration.verified_at.is_none());
        bind_account(&registration.id, &source.id).await.unwrap();
        assert_original_unchanged(&source);
        let untouched = UntouchedState::capture(&fixture, &registration.id).await;
        assert_no_service_started().await;
        run_official_pair(&source, &fixture, &original, "synthetic-access-0").await;
        assert_eq!(grant_activity(&fixture), (2, 2, 0, 0, 0, 0));
        untouched.assert_unchanged(&source).await;
        assert_no_service_started().await;
    });
}

#[test]
fn expired_official_grant_refreshes_independently_of_expired_original_tokens() {
    background_case(|| async {
        let fixture = Fixture::new().await;
        let original = OriginalLoopback::new(200, false).await;
        let source = source_account("background-refresh-source", true);
        let registration = fixture.login().await;
        bind_account(&registration.id, &source.id).await.unwrap();
        expire(&registration.id).await;
        let before = storage::read(&registration.id).await.unwrap();
        let untouched = UntouchedState::capture(&fixture, &registration.id).await;
        run_official_pair(&source, &fixture, &original, "synthetic-access-1").await;
        assert_eq!(grant_activity(&fixture), (2, 2, 1, 1, 0, 0));
        let after = storage::read(&registration.id).await.unwrap();
        let credentials = after.credentials().unwrap();
        assert_eq!(credentials.access_token, "synthetic-access-1");
        assert_eq!(credentials.refresh_token, "synthetic-refresh-1");
        assert!(credentials.expires_at > chrono::Utc::now().timestamp());
        assert_eq!(credentials.id_token, before.credentials().unwrap().id_token);
        assert_eq!(after.authorization_revision, before.authorization_revision);
        assert_eq!(
            after.summary.source_account_id.as_deref(),
            Some(source.id.as_str())
        );
        assert_ne!(after.revision, before.revision);
        untouched
            .assert_original_and_service_unchanged(&source)
            .await;
        assert_no_service_started().await;
    });
}

#[test]
fn expired_original_credentials_do_not_block_a_valid_bound_grant() {
    background_case(|| async {
        let fixture = Fixture::new().await;
        let original = OriginalLoopback::new(401, false).await;
        let source = source_account("background-expired-source", true);
        let registration = fixture.login().await;
        bind_account(&registration.id, &source.id).await.unwrap();
        let untouched = UntouchedState::capture(&fixture, &registration.id).await;
        run_official_pair(&source, &fixture, &original, "synthetic-access-0").await;
        assert_eq!(grant_activity(&fixture), (2, 2, 0, 0, 0, 0));
        untouched.assert_unchanged(&source).await;
        assert_no_service_started().await;
    });
}

async fn assert_background_errors(source: &CodexAccount, expected: &str) {
    let error = access::run_official_wakeup_chat(
        &source.id,
        Some("test-model"),
        Some("high"),
        "failed wakeup",
    )
    .await
    .err()
    .expect("official failure must be reported");
    assert_eq!(error, expected);
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let error = access::run_pelican_chat(
        &source.id,
        "test-model",
        "high",
        "failed benchmark",
        cancel_rx,
        |_| panic!("failed response must not emit output"),
    )
    .await
    .err()
    .expect("official failure must be reported");
    assert_eq!(error, expected);
}

#[test]
fn official_background_http_failure_never_retries_or_falls_back_to_original() {
    background_case(|| async {
        let fixture = Fixture::new().await;
        let original = OriginalLoopback::new(200, false).await;
        let source = source_account("background-failure-source", false);
        let registration = fixture.login().await;
        bind_account(&registration.id, &source.id).await.unwrap();
        fixture.state.lock().unwrap().response_status = Some(503);
        let untouched = UntouchedState::capture(&fixture, &registration.id).await;
        assert_background_errors(&source, "CHATGPT_PLAN_NETWORK").await;
        assert_official_requests(
            &fixture,
            "synthetic-access-0",
            &["failed wakeup", "failed benchmark"],
        );
        assert_eq!(grant_activity(&fixture), (2, 2, 0, 0, 0, 0));
        assert!(original.requests.lock().unwrap().is_empty());
        untouched.assert_unchanged(&source).await;
        assert_no_service_started().await;
    });
}

#[test]
fn unusable_bound_grants_block_background_calls_before_either_upstream() {
    background_case(|| async {
        for (failure, expected) in [
            ("reauth", "CHATGPT_PLAN_REAUTH_REQUIRED"),
            ("disconnected", "CHATGPT_PLAN_DISCONNECTED"),
            ("source-changed", "CHATGPT_PLAN_SOURCE_CHANGED"),
        ] {
            let fixture = Fixture::new().await;
            let original = OriginalLoopback::new(200, false).await;
            let mut source = source_account(&format!("background-blocked-{failure}"), false);
            let registration = fixture.login().await;
            bind_account(&registration.id, &source.id).await.unwrap();
            if failure == "source-changed" {
                source.user_id = Some("replacement-codex-user".into());
                codex_account::save_account(&source).unwrap();
            } else {
                let _guard = storage::lock(&registration.id).await.unwrap();
                let mut record = storage::read(&registration.id).await.unwrap();
                if failure == "reauth" {
                    record.summary.needs_reauth = true;
                } else {
                    record.summary.connected = false;
                    record.encrypted_credentials = None;
                }
                storage::blocking(move || storage::save(&record))
                    .await
                    .unwrap();
            }
            let untouched = UntouchedState::capture(&fixture, &registration.id).await;
            assert_background_errors(&source, expected).await;
            assert!(original.requests.lock().unwrap().is_empty());
            assert_grant_unused(&fixture);
            untouched.assert_unchanged(&source).await;
            assert_no_service_started().await;
        }
    });
}

#[test]
fn original_account_without_binding_still_uses_original_loopback_transport() {
    background_case(|| async {
        let fixture = Fixture::new().await;
        let original = OriginalLoopback::new(200, false).await;
        let source = source_account("background-unbound-source", false);
        // An unbound grant must never be selected merely by matching email.
        let registration = fixture.login().await;
        let untouched = UntouchedState::capture(&fixture, &registration.id).await;
        run_original_pair(&source, &original).await;
        assert_grant_unused(&fixture);
        untouched.assert_unchanged(&source).await;
        assert_no_service_started().await;
    });
}

#[test]
fn api_service_connection_mode_does_not_override_bound_background_authorization() {
    for mode in ["codex", "official"] {
        background_case(|| async {
            let fixture = Fixture::new().await;
            let original = OriginalLoopback::new(200, false).await;
            let source = source_account(&format!("background-service-{mode}"), false);
            let registration = fixture.login().await;
            bind_account(&registration.id, &source.id).await.unwrap();
            let selected = crate::modules::chatgpt_plan_service::set_service_mode(
                &source.id,
                Some(&registration.id),
                mode,
                None,
            )
            .await
            .unwrap();
            assert!(!selected.running);
            let collection = selected.collection.unwrap();
            assert_eq!(collection.account_ids, vec![source.id.clone()]);
            assert_eq!(
                collection.official_account_bindings.get(&source.id),
                (mode == "official").then_some(&registration.id)
            );
            // The official service selection refreshes its catalog independently.
            // Await the final storage write before taking any no-change snapshots.
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while crate::modules::chatgpt_plan_service::catalog_refresh_pending_for_test(
                    &registration.id,
                ) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("service catalog refresh must finish");
            let models = usize::from(mode == "official");
            assert_eq!(grant_activity(&fixture), (0, 0, 0, 0, models, 0));
            let untouched = UntouchedState::capture(&fixture, &registration.id).await;
            run_official_pair(&source, &fixture, &original, "synthetic-access-0").await;
            assert_eq!(grant_activity(&fixture), (2, 2, 0, 0, models, 0));
            untouched.assert_unchanged(&source).await;
        });
    }
}

#[test]
fn bound_benchmark_pre_cancellation_sends_no_requests_or_refreshes() {
    background_case(|| async {
        let fixture = Fixture::new().await;
        let original = OriginalLoopback::new(200, false).await;
        let source = source_account("background-pre-cancel-source", true);
        let registration = fixture.login().await;
        bind_account(&registration.id, &source.id).await.unwrap();
        expire(&registration.id).await;
        let untouched = UntouchedState::capture(&fixture, &registration.id).await;
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(true);
        let error = access::run_pelican_chat(
            &source.id,
            "test-model",
            "high",
            "cancel before start",
            cancel_rx,
            |_| panic!("pre-cancelled benchmark must not emit output"),
        )
        .await
        .err()
        .expect("cancelled benchmark");
        assert_eq!(error, "PELICAN_CANCELLED");
        assert!(original.requests.lock().unwrap().is_empty());
        assert_grant_unused(&fixture);
        untouched.assert_unchanged(&source).await;
        assert_no_service_started().await;
    });
}

struct HeldOfficialStream {
    previous_server: Option<std::ffi::OsString>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for HeldOfficialStream {
    fn drop(&mut self) {
        self.server.abort();
        if let Some(previous) = self.previous_server.take() {
            std::env::set_var("CHATGPT_PLAN_TEST_SERVER", previous);
        } else {
            std::env::remove_var("CHATGPT_PLAN_TEST_SERVER");
        }
    }
}

impl HeldOfficialStream {
    async fn new(state: Arc<Mutex<MockState>>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let previous_server = std::env::var_os("CHATGPT_PLAN_TEST_SERVER");
        std::env::set_var(
            "CHATGPT_PLAN_TEST_SERVER",
            format!("http://{}", listener.local_addr().unwrap()),
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0; 2048];
                let n = stream.read(&mut chunk).await.unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
                assert!(bytes.len() < 65536);
            };
            let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
            assert!(headers.starts_with("POST /v1/responses HTTP/1.1\r\n"));
            let length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .unwrap()
                .1
                .trim()
                .parse::<usize>()
                .unwrap();
            assert!(length < 65536);
            while bytes.len() < header_end + length {
                let mut chunk = [0; 2048];
                let n = stream.read(&mut chunk).await.unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            let body = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            {
                let mut state = state.lock().unwrap();
                state.response_started += 1;
                state.response_requests.push((headers, body));
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            let event = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"PARTIAL\"}\n\n";
            stream
                .write_all(format!("{:x}\r\n{event}\r\n", event.len()).as_bytes())
                .await
                .unwrap();
            let mut byte = [0];
            let read =
                tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut byte))
                    .await
                    .expect("cancel must close the upstream stream")
                    .unwrap();
            assert_eq!(
                read, 0,
                "no retry or completion request on the original stream"
            );
            // The listener never accepts a second request, even after EOF.
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(180), listener.accept())
                    .await
                    .is_err(),
                "cancel must not reconnect or replay the generation"
            );
        });
        Self {
            previous_server,
            server,
        }
    }
}

#[test]
fn bound_benchmark_live_stream_cancellation_closes_official_connection_without_retry() {
    background_case(|| async {
        let fixture = Fixture::new().await;
        let original = OriginalLoopback::new(200, false).await;
        let source = source_account("background-cancel-source", false);
        let registration = fixture.login().await;
        bind_account(&registration.id, &source.id).await.unwrap();
        let untouched = UntouchedState::capture(&fixture, &registration.id).await;
        let mut upstream = HeldOfficialStream::new(fixture.state.clone()).await;
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let (delta_tx, mut delta_rx) = tokio::sync::mpsc::unbounded_channel();
        let source_id = source.id.clone();
        let pending = tokio::spawn(async move {
            access::run_pelican_chat(
                &source_id,
                "test-model",
                "high",
                "cancel benchmark",
                cancel_rx,
                move |delta| {
                    delta_tx.send(delta).unwrap();
                },
            )
            .await
        });
        let delta = tokio::time::timeout(std::time::Duration::from_secs(5), delta_rx.recv())
            .await
            .expect("official SSE must produce a partial delta")
            .unwrap();
        assert_eq!(delta, "PARTIAL");
        cancel_tx.send(true).unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), pending)
            .await
            .expect("cancellation must promptly finish")
            .unwrap()
            .err()
            .expect("cancelled benchmark");
        assert_eq!(error, "PELICAN_CANCELLED");
        (&mut upstream.server).await.unwrap();
        assert!(
            delta_rx.recv().await.is_none(),
            "no post-cancellation deltas"
        );
        assert_official_requests(&fixture, "synthetic-access-0", &["cancel benchmark"]);
        assert_eq!(grant_activity(&fixture), (1, 1, 0, 0, 0, 0));
        assert!(original.requests.lock().unwrap().is_empty());
        untouched.assert_unchanged(&source).await;
        assert_no_service_started().await;
    });
}

#[test]
fn source_identity_change_during_grant_refresh_prevents_generation() {
    background_case(|| async {
        for purpose in ["wakeup", "pelican"] {
            let fixture = Fixture::new().await;
            let original = OriginalLoopback::new(200, false).await;
            let mut source = source_account(&format!("background-race-{purpose}"), true);
            let registration = fixture.login().await;
            bind_account(&registration.id, &source.id).await.unwrap();
            expire(&registration.id).await;
            let release = Arc::new(tokio::sync::Notify::new());
            fixture.state.lock().unwrap().refresh_release = Some(release.clone());
            let untouched = UntouchedState::capture(&fixture, &registration.id).await;
            let source_id = source.id.clone();
            let pending = tokio::spawn(async move {
                if purpose == "wakeup" {
                    access::run_official_wakeup_chat(
                        &source_id,
                        Some("test-model"),
                        Some("high"),
                        "identity race",
                    )
                    .await
                    .map(|_| ())
                } else {
                    let (_tx, rx) = tokio::sync::watch::channel(false);
                    access::run_pelican_chat(
                        &source_id,
                        "test-model",
                        "high",
                        "identity race",
                        rx,
                        |_| panic!("replaced identity must not receive output"),
                    )
                    .await
                    .map(|_| ())
                }
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while fixture.state.lock().unwrap().refresh_started == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("grant refresh must reach the loopback barrier");
            source.user_id = Some("replacement-during-refresh".into());
            codex_account::save_account(&source).unwrap();
            release.notify_one();
            let error = tokio::time::timeout(std::time::Duration::from_secs(5), pending)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(error, "CHATGPT_PLAN_SOURCE_CHANGED");
            assert_eq!(grant_activity(&fixture), (0, 0, 1, 1, 0, 0));
            assert!(original.requests.lock().unwrap().is_empty());
            untouched
                .assert_original_and_service_unchanged(&source)
                .await;
            assert_no_service_started().await;
        }
    });
}
