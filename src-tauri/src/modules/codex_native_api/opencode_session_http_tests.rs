//! OpenCode ingress regressions using only native HTTP and a loopback upstream.
use super::*;

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
        let body = json!({"model":"m","input":"x".repeat(size)});
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
        assert_eq!(record.body["input"].as_str().unwrap().len(), size);
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
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
        session_continuation(mapping).await;
    }
}

async fn session_continuation(mapping: bool) {
    let (mut handle, host, source, task, _dir) = session_caller_fixture(true).await;
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
    assert_session_pair(&seen[0], &seen[1], !mapping);
    if mapping {
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
    assert_eq!(seen[2].body["previous_response_id"], first["id"]);
    assert_eq!(contexts[0].attempts.len(), 2);
    assert_eq!(contexts[1].attempts.len(), 1);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
