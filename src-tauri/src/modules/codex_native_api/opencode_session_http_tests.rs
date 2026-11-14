//! OpenCode ingress regressions using only native HTTP and a loopback upstream.
use super::*;

// v1.3.82 restricts native identity mapping to Codex OAuth routes. These
// regressions exercise that path explicitly, not a generic API-key route.
async fn identity_fixture(
    wire: WireApi,
    native_socket: bool,
    retry: bool,
) -> (
    Handle,
    Arc<IdentityHost>,
    Arc<IdentitySource>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    identity_fixture_with_options(wire, native_socket, retry, Options::default()).await
}

async fn identity_fixture_with_options(
    wire: WireApi,
    native_socket: bool,
    retry: bool,
    options: Options,
) -> (
    Handle,
    Arc<IdentityHost>,
    Arc<IdentitySource>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let fixture = super::identity_fixture_with_options(wire, native_socket, retry, options).await;
    fixture.1.oauth.store(true, Ordering::SeqCst);
    fixture
}

#[tokio::test]
async fn opencode_session_generic_routes_preserve_v1382_mapping_restriction() {
    let (mut handle, _host, source, task, _dir) =
        super::identity_fixture(WireApi::Responses, false, false).await;
    for enabled in [true, false] {
        handle.set_session_identity_mapping_enabled(enabled);
        for session in ["generic-a", "generic-b", "generic-a"] {
            session_success(
                session_request(&handle, "/v1/responses")
                    .header("x-opencode-session", session)
                    .json(&json!({"model":"m","input":[]})),
            )
            .await;
        }
    }
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 6);
    assert_session_pair(&seen[0], &seen[1], false);
    assert_session_pair(&seen[0], &seen[2], true);
    assert_session_pair(&seen[0], &seen[3], true);
    assert_session_pair(&seen[1], &seen[4], true);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

// The shared fixture intentionally authorizes a single fixed caller. Wrap that
// host only where authenticated caller isolation is part of the assertion.
struct SessionHost {
    inner: Arc<IdentityHost>,
}

impl Host for SessionHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            let id = match bearer {
                "key-a" | "key-a-rotated" => "caller-a",
                "key-b" => "caller-b",
                _ => return Err(ServiceError::new(401, "unauthorized", "Unknown test key")),
            };
            Ok(Caller {
                id: id.into(),
                label: "same-display-label".into(),
            })
        })
    }

    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        self.inner.route(caller, request)
    }

    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        self.inner.execution_policy()
    }

    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }

    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(report)
    }

    fn diagnostic(&self, context: DiagnosticContext) -> BoxFuture<'_, ()> {
        self.inner.diagnostic(context)
    }
}

async fn session_caller_fixture(
    retry: bool,
) -> (
    Handle,
    Arc<SessionHost>,
    Arc<IdentitySource>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let (mut original, inner, source, task, dir) =
        identity_fixture(WireApi::Responses, false, retry).await;
    original.shutdown(Duration::from_secs(2)).await.unwrap();
    let host = Arc::new(SessionHost { inner });
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        dir.path().join("opencode.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    (handle, host, source, task, dir)
}

fn session_request(handle: &Handle, path: &str) -> reqwest::RequestBuilder {
    session_request_as(handle, path, "key")
}

fn session_request_as(handle: &Handle, path: &str, key: &str) -> reqwest::RequestBuilder {
    http_client()
        .post(format!("http://{}{path}", handle.addr))
        .timeout(Duration::from_secs(5))
        .bearer_auth(key)
        .header("content-type", "application/json")
}

async fn session_success(request: reqwest::RequestBuilder) -> Value {
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}

fn assert_session_pair(left: &WireRecord, right: &WireRecord, equal: bool) {
    for header in ["session-id", "thread-id"] {
        let left = left.headers.get(header).expect("native identity header");
        let right = right.headers.get(header).expect("native identity header");
        assert_eq!(left == right, equal, "{header}");
    }
}

#[tokio::test]
async fn opencode_session_automatically_reuses_identity_on_conversation_endpoints() {
    for wire in [WireApi::Responses, WireApi::Chat] {
        let (mut handle, host, source, task, _dir) = identity_fixture(wire, false, false).await;
        // Public Responses/Chat/Completions ingress on third-party API routes.
        // Native Codex mapping is covered separately below.
        host.oauth.store(false, Ordering::SeqCst);
        let cases = [
            ("responses", json!({"model":"m","input":[]})),
            (
                "chat/completions",
                json!({"model":"m","messages":[{"role":"user","content":"hello"}]}),
            ),
            ("completions", json!({"model":"m","prompt":"hello"})),
        ];
        let mut count = 0;
        for (path, body) in cases {
            // The shared mock has no legacy /completions upstream handler.
            // Exercise that ingress through Codex Responses, not native Chat.
            if wire == WireApi::Chat && path == "completions" {
                continue;
            }
            // Both public path spellings must establish the same conversation.
            for prefix in ["/v1/", "/"] {
                session_success(
                    session_request(&handle, &format!("{prefix}{path}"))
                        .header("x-opencode-session", "ordinary-opencode-session")
                        .json(&body),
                )
                .await;
                count += 1;
            }
        }
        let contexts = captured(&host, count).await;
        let seen = source.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), count);
        for (context, record) in contexts.iter().zip(&seen) {
            assert_sent(context, record, None, None);
            assert_session_pair(&seen[0], record, true);
            assert_ne!(record.headers["session-id"], "ordinary-opencode-session");
        }
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn opencode_session_seed_is_stable_with_native_mapping_on_and_off() {
    let (mut handle, _host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    for enabled in [true, true, false, false, true] {
        handle.set_session_identity_mapping_enabled(enabled);
        session_success(
            session_request(&handle, "/v1/responses")
                .header("x-opencode-session", "mapping-session")
                .json(&json!({"model":"m","input":[]})),
        )
        .await;
    }
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 5);
    assert_session_pair(&seen[0], &seen[1], true);
    assert_session_pair(&seen[2], &seen[3], true);
    assert_session_pair(&seen[0], &seen[4], true);
    assert_session_pair(&seen[0], &seen[2], false);
    let seed = uuid::Uuid::parse_str(seen[2].headers["session-id"].to_str().unwrap()).unwrap();
    assert_eq!(seed.get_version_num(), 5);
    assert_eq!(seen[2].headers["session-id"], seen[2].headers["thread-id"]);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn opencode_session_compatibility_responses_alias_preserves_identity() {
    let (mut handle, host, source, task, _dir) =
        super::identity_fixture(WireApi::Responses, false, false).await;
    for path in [
        "/v1/chat/completions/v1/responses",
        "/v1/responses",
        "/v1/chat/completions/v1/responses",
    ] {
        session_success(
            session_request(&handle, path)
                .header("x-opencode-session", "compatibility-alias-session")
                .json(&json!({"model":"m","input":[]})),
        )
        .await;
    }
    let contexts = captured(&host, 3).await;
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    for (context, record) in contexts.iter().zip(&seen) {
        assert_sent(context, record, None, None);
        assert_session_pair(&seen[0], record, true);
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn opencode_session_switch_and_listener_restart_preserve_identity_in_both_mapping_modes() {
    for mapping in [true, false] {
        let options = Options {
            session_identity_mapping_enabled: mapping,
            ..Options::default()
        };
        let (mut handle, host, source, task, dir) =
            identity_fixture_with_options(WireApi::Responses, false, false, options.clone()).await;
        for session in ["restart-a", "restart-b", "restart-a"] {
            session_success(
                session_request(&handle, "/v1/responses")
                    .header("x-opencode-session", session)
                    .json(&json!({"model":"m","input":[]})),
            )
            .await;
        }
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        let mut restarted = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            dir.path().join("identity.sqlite"),
            options,
        )
        .await
        .unwrap();
        for session in ["restart-a", "restart-b"] {
            session_success(
                session_request(&restarted, "/v1/responses")
                    .header("x-opencode-session", session)
                    .json(&json!({"model":"m","input":[]})),
            )
            .await;
        }
        let seen = source.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 5);
        assert_session_pair(&seen[0], &seen[1], false);
        assert_session_pair(&seen[0], &seen[2], true);
        assert_session_pair(&seen[0], &seen[3], true);
        assert_session_pair(&seen[1], &seen[4], true);
        restarted.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn opencode_session_is_scoped_to_authenticated_caller_not_bearer_or_display_label() {
    for mapping in [true, false] {
        let (mut handle, _host, source, task, _dir) = session_caller_fixture(false).await;
        handle.set_session_identity_mapping_enabled(mapping);
        for (key, session) in [
            ("key-a", "shared-session"),
            ("key-a-rotated", "shared-session"),
            ("key-b", "shared-session"),
            ("key-b", "shared-session"),
            ("key-a", "another-session"),
            ("key-a", "another-session"),
        ] {
            session_success(
                session_request_as(&handle, "/v1/responses", key)
                    .header("x-opencode-session", session)
                    .json(&json!({"model":"m","input":[]})),
            )
            .await;
        }
        let response = session_request_as(&handle, "/v1/responses", "not-authorized")
            .header("x-opencode-session", "shared-session")
            .json(&json!({"model":"m","input":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401, "session labels are not credentials");
        let seen = source.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 6, "unauthorized requests must not dispatch");
        assert_session_pair(&seen[0], &seen[1], true);
        assert_session_pair(&seen[2], &seen[3], true);
        assert_session_pair(&seen[4], &seen[5], true);
        assert_session_pair(&seen[0], &seen[2], false);
        assert_session_pair(&seen[0], &seen[4], false);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn opencode_session_large_body_reaches_native_ingress_without_losing_continuity() {
    let (mut handle, host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    for size in [(1 << 20) - 1, (1 << 20) + 1, 2 << 20] {
        let body = json!({"model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"x".repeat(size)}]}]});
        session_success(
            session_request(&handle, "/v1/responses")
                .header("x-opencode-session", "large-session")
                .json(&body),
        )
        .await;
    }
    let contexts = captured(&host, 3).await;
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    for (context, record) in contexts.iter().zip(&seen) {
        assert_sent(context, record, None, None);
        assert_session_pair(&seen[0], record, true);
        assert_eq!(record.body["model"], "m");
    }
    for (record, size) in seen.iter().zip([(1 << 20) - 1, (1 << 20) + 1, 2 << 20]) {
        assert_eq!(
            record.body["input"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .len(),
            size
        );
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn opencode_session_conversation_id_header_presence_wins_in_both_mapping_modes() {
    for mapping in [true, false] {
        let (mut handle, host, source, task, _dir) =
            identity_fixture(WireApi::Responses, false, false).await;
        handle.set_session_identity_mapping_enabled(mapping);
        let body = json!({"model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"original payload"}]}]});
        for _ in 0..2 {
            session_success(
                session_request(&handle, "/v1/responses")
                    .header("x-opencode-session", "header-precedence")
                    .json(&body),
            )
            .await;
        }
        let mut count = 2;
        for name in ["conversation_id", "Conversation_ID", "CONVERSATION_ID"] {
            for value in ["explicit-conversation", ""] {
                // Compare native unlabeled ingress with repeated labeled requests.
                for label in [None, Some("header-precedence"), Some("header-precedence")] {
                    let mut request = session_request(&handle, "/v1/responses").header(name, value);
                    if let Some(label) = label {
                        request = request.header("x-opencode-session", label);
                    }
                    session_success(request.json(&body)).await;
                    count += 1;
                }
            }
        }
        let contexts = captured(&host, count).await;
        let seen = source.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), count);
        assert_session_pair(&seen[0], &seen[1], true);
        for (context, record) in contexts.iter().zip(&seen) {
            assert_sent(context, record, None, None);
            assert_eq!(record.body["input"], body["input"]);
        }
        for (records, contexts) in seen[2..].chunks_exact(3).zip(contexts[2..].chunks_exact(3)) {
            for index in [1, 2] {
                assert_eq!(
                    records[0].headers.get("conversation_id"),
                    records[index].headers.get("conversation_id")
                );
                let native = contexts[0].routing_identity.as_ref().unwrap();
                let labeled = contexts[index].routing_identity.as_ref().unwrap();
                assert_ne!(
                    native.session_id, labeled.session_id,
                    "explicit presence must not establish an OpenCode root"
                );
                assert_ne!(native.thread_id, labeled.thread_id);
            }
            if mapping {
                assert_session_pair(&records[1], &records[2], false);
            } else {
                for record in records {
                    assert!(!record.headers.contains_key("session-id"));
                    assert!(!record.headers.contains_key("thread-id"));
                }
            }
        }
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn opencode_session_conversation_id_invalid_header_keeps_native_errors_in_both_mapping_modes()
{
    for mapping in [true, false] {
        let (mut handle, _host, source, task, _dir) =
            identity_fixture(WireApi::Responses, false, false).await;
        handle.set_session_identity_mapping_enabled(mapping);
        for value in ["invalid\tidentity".to_owned(), "x".repeat(4097)] {
            let mut native_result = None;
            for label in [None, Some("must-not-repair-explicit-identity")] {
                let before = source.seen.lock().unwrap().len();
                let mut request = session_request(&handle, "/v1/responses")
                    .header("Conversation_ID", value.as_str());
                if let Some(label) = label {
                    request = request.header("x-opencode-session", label);
                }
                let response = request
                    .json(&json!({"model":"m","input":[]}))
                    .send()
                    .await
                    .unwrap();
                let status = response.status();
                let body: Value = response.json().await.unwrap();
                let result = (status, body.pointer("/error/code").cloned());
                if let Some(native) = &native_result {
                    assert_eq!(
                        &result, native,
                        "the label must not alter native validation"
                    );
                } else {
                    native_result = Some(result);
                }
                let seen = source.seen.lock().unwrap();
                if mapping {
                    assert_eq!(status, 400);
                    assert_eq!(body["error"]["code"], "invalid_protocol_identity");
                    assert_eq!(seen.len(), before, "invalid identity must not dispatch");
                } else if status.is_success() {
                    // OFF deliberately keeps native passthrough semantics;
                    // the fallback must not manufacture identity or new errors.
                    assert_eq!(seen.len(), before + 1);
                    assert!(!seen.last().unwrap().headers.contains_key("session-id"));
                    assert!(!seen.last().unwrap().headers.contains_key("thread-id"));
                    assert_eq!(seen.last().unwrap().headers["conversation_id"], value);
                } else {
                    assert_eq!(seen.len(), before);
                }
            }
        }
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn opencode_session_explicit_canonical_legacy_and_metadata_identity_take_precedence() {
    let (mut handle, host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    let canonical = json!({"session_id":"canonical-session","thread_id":"canonical-thread"});
    let cases = [
        (
            vec![("session-id", "header-session".to_owned())],
            json!({"model":"m","input":[]}),
            "header-session",
            "header-session",
        ),
        (
            vec![("session_id", "legacy-session".to_owned())],
            json!({"model":"m","input":[]}),
            "legacy-session",
            "legacy-session",
        ),
        (
            vec![("thread-id", "header-thread".to_owned())],
            json!({"model":"m","input":[]}),
            "header-thread",
            "header-thread",
        ),
        (
            vec![("session-id", "ignored-header".to_owned())],
            json!({"model":"m","input":[],"client_metadata":{
                "session_id":"flat-session","thread_id":"flat-thread"}}),
            "flat-session",
            "flat-thread",
        ),
        (
            vec![("x-codex-turn-metadata", canonical.to_string())],
            json!({"model":"m","input":[],"client_metadata":{"session_id":"ignored-flat"}}),
            "canonical-session",
            "canonical-thread",
        ),
        (
            vec![("session-id", "ignored-header".to_owned())],
            json!({"model":"m","input":[],"client_metadata":{
                "session_id":"ignored-flat","x-codex-turn-metadata":canonical.to_string()}}),
            "canonical-session",
            "canonical-thread",
        ),
    ];
    for (headers, body, _, _) in &cases {
        // Compare native behavior without the fallback label against a labeled
        // request: explicit identities must win without being replaced.
        for label in [None, Some("must-not-win")] {
            let mut request = session_request(&handle, "/v1/responses");
            if let Some(label) = label {
                request = request.header("x-opencode-session", label);
            }
            for (name, value) in headers {
                request = request.header(*name, value.as_str());
            }
            session_success(request.json(body)).await;
        }
    }
    let contexts = captured(&host, cases.len() * 2).await;
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), cases.len() * 2);
    for (index, (_, _, session, thread)) in cases.iter().enumerate() {
        assert_session_pair(&seen[index * 2], &seen[index * 2 + 1], true);
        for context in &contexts[index * 2..index * 2 + 2] {
            let identity = context.routing_identity.as_ref().unwrap();
            assert_eq!(identity.session_id.as_deref(), Some(*session));
            assert_eq!(identity.thread_id.as_deref(), Some(*thread));
        }
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn opencode_session_ambiguous_headers_and_duplicate_json_only_suppress_fallback() {
    let (mut handle, _host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    let bodies = [
        r#"{"model":"discarded","mo\u0064el":"m","input":[]}"#,
        r#"{"model":"m","input":[],"client_metadata":{"extra":1,"ex\u0074ra":2}}"#,
        r#"{"model":"m","input":[],"client_metadata":{"session_id":null},"client_metadata":{}}"#,
    ];
    for body in bodies {
        for _ in 0..2 {
            session_success(
                session_request(&handle, "/v1/responses")
                    .header("x-opencode-session", "ambiguous-session")
                    .body(body),
            )
            .await;
        }
    }
    for labels in [
        vec!["same", "same"],
        vec!["first", "second"],
        vec!["first, second"],
        vec![""],
    ] {
        for _ in 0..2 {
            let mut request = session_request(&handle, "/v1/responses");
            for label in &labels {
                // RequestBuilder::header appends, preserving duplicate fields.
                request = request.header("x-opencode-session", *label);
            }
            session_success(request.json(&json!({"model":"m","input":[]}))).await;
        }
    }
    // Duplicate keys deeper inside an unrelated extension are not duplicate
    // top-level or direct client_metadata keys and must not disable fallback.
    for _ in 0..2 {
        session_success(
            session_request(&handle, "/v1/responses")
                .header("x-opencode-session", "nested-extension-session")
                .body(r#"{"model":"m","input":[],"client_metadata":{"extension":{"nested":1,"nested":2}}}"#),
        )
        .await;
    }
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 16);
    for pair in seen[..14].chunks_exact(2) {
        // Baseline native stateless requests get fresh roots; ambiguity must
        // neither turn them into a conversation nor introduce a new error.
        assert_session_pair(&pair[0], &pair[1], false);
        assert_eq!(pair[0].body["model"], "m");
    }
    assert_eq!(seen[2].body["client_metadata"]["extra"], 2);
    assert_session_pair(&seen[14], &seen[15], true);
    assert_eq!(seen[14].body["client_metadata"]["extension"]["nested"], 2);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn opencode_session_explicit_presence_does_not_enable_fallback_or_change_native_parsing() {
    let (mut handle, _host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    let cases = [
        (None, json!({"client_metadata":null})),
        (None, json!({"previous_response_id":null})),
        (None, json!({"client_metadata":{"session_id":null}})),
        (None, json!({"client_metadata":{"thread_id":null}})),
        (None, json!({"client_metadata":{"turn_id":"turn"}})),
        (
            None,
            json!({"client_metadata":{"x-codex-parent-thread-id":"parent"}}),
        ),
        (None, json!({"client_metadata":{"parent_response_id":null}})),
        (None, json!({"client_metadata":{"x-codex-turn-state":null}})),
        (Some(("session-id", "")), json!({})),
        (Some(("session_id", "")), json!({})),
        (Some(("thread-id", "")), json!({})),
        // Native does not interpret this alias; fallback still must not guess
        // that the caller intended a new OpenCode conversation instead.
        (Some(("thread_id", "legacy-thread")), json!({})),
        (Some(("x-codex-parent-thread-id", "parent")), json!({})),
        (Some(("x-codex-turn-metadata", "{}")), json!({})),
        (Some(("x-codex-turn-state", "")), json!({})),
    ];
    for (header, extra) in &cases {
        let mut body = json!({"model":"m","input":[]});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        for _ in 0..2 {
            let mut request = session_request(&handle, "/v1/responses")
                .header("x-opencode-session", "not-a-native-identity");
            if let Some((name, value)) = header {
                request = request.header(*name, *value);
            }
            session_success(request.json(&body)).await;
        }
    }
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), cases.len() * 2);
    for pair in seen.chunks_exact(2) {
        assert_session_pair(&pair[0], &pair[1], false);
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn opencode_session_invalid_inputs_keep_native_errors_and_body_limit() {
    let (mut handle, _host, source, task, _dir) = identity_fixture_with_options(
        WireApi::Responses,
        false,
        false,
        Options {
            max_body_bytes: 256,
            ..Default::default()
        },
    )
    .await;
    for (body, header, status, code) in [
        ("{".to_owned(), None, 400, "invalid_json"),
        ("{} {}".to_owned(), None, 400, "invalid_json"),
        (
            r#"{"model":"m","input":[],"previous_response_id":123}"#.to_owned(),
            None,
            400,
            "invalid_protocol_identity",
        ),
        (
            r#"{"model":"m","input":[],"client_metadata":{"session_id":123}}"#.to_owned(),
            None,
            400,
            "invalid_protocol_identity",
        ),
        (
            r#"{"model":"m","input":[],"client_metadata":[]}"#.to_owned(),
            None,
            400,
            "invalid_protocol_identity",
        ),
        (
            r#"{"model":"m","input":[]}"#.to_owned(),
            Some(("x-codex-turn-metadata", "{")),
            400,
            "invalid_protocol_identity",
        ),
        (" ".repeat(257), None, 413, "request_too_large"),
    ] {
        // With and without the label, the native error remains authoritative.
        for label in [None, Some("cannot-repair-invalid-input")] {
            let mut request = session_request(&handle, "/v1/responses");
            if let Some(label) = label {
                request = request.header("x-opencode-session", label);
            }
            if let Some((name, value)) = header {
                request = request.header(name, value);
            }
            let response = request.body(body.clone()).send().await.unwrap();
            assert_eq!(response.status().as_u16(), status);
            let value: Value = response.json().await.unwrap();
            assert_eq!(value["error"]["code"], code);
        }
    }
    assert!(source.seen.lock().unwrap().is_empty());
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn opencode_session_non_opencode_and_unrelated_paths_keep_native_random_roots() {
    let (mut handle, _host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    for (path, label, body) in [
        ("/v1/responses", None, json!({"model":"m","input":[]})),
        (
            "/v1/responses/compact",
            Some("not-a-conversation-endpoint"),
            json!({"model":"m","input":[]}),
        ),
        (
            "/v1/alpha/search",
            Some("not-a-conversation-endpoint"),
            json!({"model":"m","input":"hello"}),
        ),
    ] {
        for _ in 0..2 {
            let mut request = session_request(&handle, path);
            if let Some(label) = label {
                request = request.header("x-opencode-session", label);
            }
            session_success(request.json(&body)).await;
        }
    }
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 6);
    for pair in seen.chunks_exact(2) {
        assert_session_pair(&pair[0], &pair[1], false);
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn opencode_session_continuation_retains_account_owner_and_rejects_other_callers() {
    for mapping in [true, false] {
        for oauth in [true, false] {
            session_continuation(mapping, oauth).await;
        }
    }
}

async fn session_continuation(mapping: bool, oauth: bool) {
    let (mut handle, host, source, task, _dir) = session_caller_fixture(true).await;
    host.inner.oauth.store(oauth, Ordering::SeqCst);
    handle.set_session_identity_mapping_enabled(mapping);
    let first = session_success(
        session_request_as(&handle, "/v1/responses", "key-a")
            .header("x-opencode-session", "continuation-session")
            .json(&json!({"model":"m","input":[]})),
    )
    .await;
    let continuation = json!({"model":"m","input":[],"previous_response_id":first["id"]});
    session_success(
        session_request_as(&handle, "/v1/responses", "key-a")
            // Explicit continuation, not this unrelated label, owns the route.
            .header("x-opencode-session", "must-not-replace-continuation")
            .json(&continuation),
    )
    .await;
    let response = session_request_as(&handle, "/v1/responses", "key-b")
        .header("x-opencode-session", "continuation-session")
        .json(&continuation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "upstream_state_caller_mismatch");
    let contexts = captured(&host.inner, 2).await;
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(
        seen.len(),
        3,
        "foreign continuation must not reach upstream"
    );
    assert_eq!(seen[0].headers["x-test-account"], "a");
    assert_eq!(seen[1].headers["x-test-account"], "b");
    assert_eq!(seen[2].headers["x-test-account"], "b");
    // Mapping OFF deliberately preserves protocol IDs across account attempts;
    // continuation ownership must still stay with the successful account.
    assert_session_pair(&seen[0], &seen[1], !(mapping && oauth));
    if mapping && oauth {
        assert_session_pair(&seen[1], &seen[2], true);
    } else {
        // With mapping OFF, native passthrough does not synthesize headers for
        // an explicit continuation. The restored logical identity still owns it.
        assert!(!seen[2].headers.contains_key("session-id"));
        assert!(!seen[2].headers.contains_key("thread-id"));
        let first = contexts[0].routing_identity.as_ref().unwrap();
        let continued = contexts[1].routing_identity.as_ref().unwrap();
        assert_eq!(first.session_id, continued.session_id);
        assert_eq!(first.thread_id, continued.thread_id);
    }
    if oauth {
        // Native ChatGPT HTTP cannot resolve previous_response_id: upstream
        // reconstructs the stored empty transcript while retaining ownership.
        assert!(seen[2].body.get("previous_response_id").is_none());
        assert_eq!(seen[2].body["input"], json!([]));
    } else {
        assert_eq!(seen[2].body["previous_response_id"], first["id"]);
    }
    assert_eq!(contexts[0].attempts.len(), 2);
    assert_eq!(contexts[1].attempts.len(), 1);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
