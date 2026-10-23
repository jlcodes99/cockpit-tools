//! Host route regression fixtures: only synthetic account overrides, no IO to
//! account directories or upstream services.
use super::*;

fn caller(id: &str) -> native_contract::Caller {
    native_contract::Caller {
        id: id.into(),
        label: "fixture".into(),
    }
}
fn host() -> NativeHostBridge {
    let collection: CodexLocalAccessCollection = serde_json::from_value(json!({
        "enabled":true,"port":1455,"apiKey":"fixture","accountIds":["a","b"],
        "createdAt":1,"updatedAt":1,"sessionAffinity":true,
        "apiKeys":[{"id":"caller-a","label":"fixture","key":"a"},{"id":"caller-b","label":"fixture","key":"b"}]
    })).unwrap();
    let mut host = NativeHostBridge::for_collection(collection);
    for id in ["a", "b"] {
        let mut account = CodexAccount::new_api_key(
            id.into(),
            "fixture@example.invalid".into(),
            "fixture-key".into(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some("https://example.invalid/v1".into()),
            None,
            None,
            vec!["gpt-fixture".into(), "gpt-other".into()],
        );
        account.api_wire_api = Some("responses".into());
        account.egress_proxy_disabled = true;
        host.account_overrides.insert(id.into(), account);
    }
    host
}
fn request(session: &str, thread: &str, parent: Option<&str>) -> native_contract::RouteRequest {
    native_contract::RouteRequest {
        model: "gpt-fixture".into(),
        session_id: Some(session.into()),
        thread_id: Some(thread.into()),
        parent_thread_id: parent.map(str::to_owned),
        preferred_account_id: None,
        soft_preferred_account_id: None,
        target_account_id: None,
        force_refresh: false,
        image_generation_requested: false,
        image_input: false,
        excluded_account_ids: vec![],
        requires_permit: true,
        auxiliary: None,
        required_capabilities: Vec::new(),
        capability_fallback_from: None,
    }
}
async fn selected(
    host: &NativeHostBridge,
    caller_id: &str,
    request: native_contract::RouteRequest,
) -> String {
    native_contract::Host::route(host, &caller(caller_id), request)
        .await
        .unwrap()
        .account_id
}
fn cool(host: &NativeHostBridge, account: &str) {
    native_upsert_cooldown(
        &mut host.cooldowns.lock().unwrap(),
        account,
        "gpt-fixture",
        503,
        None,
        false,
        Instant::now(),
    );
}
async fn binding(
    host: &NativeHostBridge,
    caller_id: &str,
    model: &str,
    id: &str,
) -> Option<(String, i64)> {
    host.bindings
        .lock()
        .await
        .get(&native_soft_binding_key(caller_id, model, id))
        .cloned()
}
fn report(
    request: &native_contract::RouteRequest,
    account: &str,
    status: u16,
    error: Option<&str>,
    message: Option<&str>,
) -> native_contract::RequestReport {
    native_contract::RequestReport {
        official_access_fingerprint: None,
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        official_authorization_error: None,
        operation: "responses",
        caller: caller("caller-a"),
        account_id: account.into(),
        requested_model: request.model.clone(),
        upstream_model: request.model.clone(),
        status,
        success: status == 200,
        elapsed: Duration::ZERO,
        usage: None,
        error: error.map(str::to_owned),
        error_message: message.map(str::to_owned),
        credential_scope: false,
        retry_after: None,
        requested_service_tier: None,
        response_service_tier: None,
        reasoning_effort: None,
        diagnostics: Some(native_api::diagnostics::DiagnosticContext {
            endpoint: "/v1/responses".into(),
            client_session_id: request.session_id.clone(),
            client_thread_id: request.thread_id.clone(),
            routing_identity: Some(native_contract::RoutingIdentity {
                session_id: request.session_id.clone(),
                thread_id: request.thread_id.clone(),
                parent_thread_id: request.parent_thread_id.clone(),
            }),
            ..Default::default()
        }),
    }
}

#[tokio::test]
async fn child_failover_never_rebinds_parent_and_local_binding_precedes_parent() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let host = host();
    let parent = request("parent", "parent", None);
    assert_eq!(selected(&host, "caller-a", parent.clone()).await, "a");
    cool(&host, "a");
    let child = request("parent", "child", Some("parent"));
    assert_eq!(selected(&host, "caller-a", child.clone()).await, "b");
    host.cooldowns.lock().unwrap().clear();
    assert_eq!(
        binding(&host, "caller-a", "gpt-fixture", "parent")
            .await
            .unwrap()
            .0,
        "a"
    );
    assert_eq!(selected(&host, "caller-a", parent).await, "a");
    assert_eq!(selected(&host, "caller-a", child.clone()).await, "b");
    host.record_soft_binding_result(&report(&child, "b", 401, Some("invalid_api_key"), None))
        .await;
    assert!(binding(&host, "caller-a", "gpt-fixture", "child")
        .await
        .is_none());
    assert_eq!(
        binding(&host, "caller-a", "gpt-fixture", "parent")
            .await
            .unwrap()
            .0,
        "a"
    );
    assert_eq!(selected(&host, "caller-a", child).await, "a");
}

#[tokio::test]
async fn three_level_shared_session_preserves_root_and_immediate_parent() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let host = host();
    assert_eq!(
        selected(&host, "caller-a", request("shared-session", "root", None)).await,
        "a"
    );
    assert_eq!(
        selected(
            &host,
            "caller-a",
            request("shared-session", "parent", Some("root"))
        )
        .await,
        "a"
    );
    cool(&host, "a");
    let grandchild = request("shared-session", "grandchild", Some("parent"));
    assert_eq!(selected(&host, "caller-a", grandchild.clone()).await, "b");
    host.cooldowns.lock().unwrap().clear();
    for id in ["shared-session", "root", "parent"] {
        assert_eq!(
            binding(&host, "caller-a", "gpt-fixture", id)
                .await
                .unwrap()
                .0,
            "a"
        );
    }
    assert_eq!(selected(&host, "caller-a", grandchild.clone()).await, "b");
    host.record_soft_binding_result(&report(&grandchild, "b", 429, None, None))
        .await;
    for id in ["shared-session", "root", "parent"] {
        assert_eq!(
            binding(&host, "caller-a", "gpt-fixture", id)
                .await
                .unwrap()
                .0,
            "a"
        );
    }
    assert!(binding(&host, "caller-a", "gpt-fixture", "grandchild")
        .await
        .is_none());
}

#[tokio::test]
async fn ordinary_aliases_scope_and_disabled_cache_keep_their_route_semantics() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut host = host();
    cool(&host, "a");
    let alias = request("session-alias", "thread-alias", None);
    assert_eq!(selected(&host, "caller-a", alias.clone()).await, "b");
    host.cooldowns.lock().unwrap().clear();
    for id in ["session-alias", "thread-alias"] {
        assert_eq!(
            binding(&host, "caller-a", "gpt-fixture", id)
                .await
                .unwrap()
                .0,
            "b"
        );
    }
    assert_eq!(selected(&host, "caller-b", alias.clone()).await, "a");
    let mut model = alias.clone();
    model.model = "gpt-other".into();
    assert_eq!(selected(&host, "caller-a", model).await, "a");
    host.collection_override.as_mut().unwrap().session_affinity = false;
    let _ = selected(&host, "caller-a", alias.clone()).await;
    assert!(host.bindings.lock().await.is_empty());
    host.record_soft_binding_result(&report(&alias, "b", 200, None, None))
        .await;
    assert!(host.bindings.lock().await.is_empty());
    let _ = selected(&host, "caller-a", alias).await;
    assert!(host.bindings.lock().await.is_empty());
}

#[tokio::test]
async fn request_faults_and_cancellation_preserve_affinity_but_real_upstream_failures_unbind() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for (status, error, message, unbind) in [
        (400, "invalid_request_error", "invalid parameter", false),
        (409, "invalid_request_error", "conflicting parameter", false),
        (422, "invalid_prompt", "invalid prompt", false),
        (499, "client_cancelled", "client disconnected", false),
        (503, "connection_lifecycle", "websocket close 1000", false),
        (503, "upstream_unavailable", "connection refused", false),
        (504, "routing_timeout", "local prepare deadline", false),
        (
            404,
            "previous_response_not_found",
            "previous response missing",
            false,
        ),
        (401, "invalid_api_key", "expired credential", true),
        (402, "invalid_request_error", "payment required", true),
        (429, "invalid_request_error", "quota exceeded", true),
        (503, "overloaded_error", "server overloaded", true),
        (
            400,
            "invalid_request_error",
            "requested model is not supported",
            true,
        ),
        (404, "model_not_found", "model missing", true),
    ] {
        let host = host();
        let request = request("session", "session", None);
        assert_eq!(selected(&host, "caller-a", request.clone()).await, "a");
        host.record_soft_binding_result(&report(&request, "a", status, Some(error), Some(message)))
            .await;
        assert_eq!(
            binding(&host, "caller-a", "gpt-fixture", "session")
                .await
                .is_none(),
            unbind,
            "{status} {error}"
        );
        assert_eq!(
            selected(&host, "caller-a", request).await,
            if unbind { "b" } else { "a" },
            "{status} {error}"
        );
    }
}

#[tokio::test]
async fn report_uses_validated_body_identity_and_ignores_conflicting_raw_headers() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for raw_headers in [false, true] {
        let host = host();
        assert_eq!(
            selected(&host, "caller-a", request("shared-session", "root", None)).await,
            "a"
        );
        let child = request("shared-session", "child", Some("root"));
        assert_eq!(selected(&host, "caller-a", child.clone()).await, "a");
        let mut success = report(&child, "a", 200, None, None);
        let diagnostics = success.diagnostics.as_mut().unwrap();
        diagnostics.client_session_id = raw_headers.then(|| "raw-session".into());
        diagnostics.client_thread_id = raw_headers.then(|| "raw-thread".into());
        let mut bindings = host.bindings.lock().await;
        native_publish_soft_bindings(
            &host,
            &mut bindings,
            [native_soft_binding_key("caller-a", "gpt-fixture", "child")],
            "a",
            now_ms() - 30_000,
        );
        if raw_headers {
            native_publish_soft_bindings(
                &host,
                &mut bindings,
                [
                    native_soft_binding_key("caller-a", "gpt-fixture", "raw-session"),
                    native_soft_binding_key("caller-a", "gpt-fixture", "raw-thread"),
                ],
                "a",
                now_ms(),
            );
        }
        drop(bindings);
        let parent_before = binding(&host, "caller-a", "gpt-fixture", "root")
            .await
            .unwrap();
        host.record_soft_binding_result(&success).await;
        assert!(
            binding(&host, "caller-a", "gpt-fixture", "child")
                .await
                .unwrap()
                .1
                > now_ms() - 10_000
        );
        assert_eq!(
            binding(&host, "caller-a", "gpt-fixture", "root")
                .await
                .unwrap(),
            parent_before
        );
        success.success = false;
        success.status = 401;
        success.error = Some("invalid_api_key".into());
        host.record_soft_binding_result(&success).await;
        assert!(binding(&host, "caller-a", "gpt-fixture", "child")
            .await
            .is_none());
        assert_eq!(
            binding(&host, "caller-a", "gpt-fixture", "root")
                .await
                .unwrap(),
            parent_before
        );
        if raw_headers {
            assert!(binding(&host, "caller-a", "gpt-fixture", "raw-session")
                .await
                .is_some());
            assert!(binding(&host, "caller-a", "gpt-fixture", "raw-thread")
                .await
                .is_some());
        }
    }
}

#[tokio::test]
async fn hard_preferred_owner_overrides_soft_parent_and_never_broadens_when_busy_or_cooled() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut host = host();
    cool(&host, "a");
    let parent = request("parent", "parent", None);
    assert_eq!(selected(&host, "caller-a", parent).await, "b");
    host.cooldowns.lock().unwrap().clear();
    let mut pinned = request("parent", "child", Some("parent"));
    pinned.preferred_account_id = Some("a".into());
    assert_eq!(selected(&host, "caller-a", pinned.clone()).await, "a");
    cool(&host, "a");
    let error = native_contract::Host::route(&host, &caller("caller-a"), pinned.clone())
        .await
        .err()
        .unwrap();
    assert_eq!((error.status, error.code), (503, "account_cooling"));
    assert!(error.retry_after.is_some());
    host.cooldowns.lock().unwrap().clear();
    host.collection_override
        .as_mut()
        .unwrap()
        .max_account_concurrency = 1;
    host.collection_override
        .as_mut()
        .unwrap()
        .account_concurrency_wait_ms = 0;
    let busy = native_contract::Host::route(&host, &caller("caller-a"), pinned.clone())
        .await
        .unwrap();
    assert_eq!(
        native_contract::Host::route(&host, &caller("caller-a"), pinned)
            .await
            .err()
            .unwrap()
            .code,
        "account_busy"
    );
    drop(busy);
}

#[tokio::test]
async fn shared_go_failure_golden_keeps_classification_in_actual_http_report_wrappers() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cases: Value = serde_json::from_str(include_str!(
        "codex_native_api/fixtures/soft_affinity_failures_go.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let status = case["status"].as_u64().unwrap() as u16;
        let raw = case["body"].as_str().unwrap();
        let value =
            serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.into()));
        let skip = case["skip"].as_bool().unwrap();
        let host = host();
        let request = request("session", "session", None);
        assert_eq!(selected(&host, "caller-a", request.clone()).await, "a");
        let code = if case["kind"] == "context_cancelled" {
            "client_cancelled".into()
        } else if let Some(code) = case["code"].as_str() {
            code.into()
        } else {
            format!("upstream_http_{status}")
        };
        let (message, scope) =
            native_api::diagnostics::upstream_error_metadata_status(status, &value);
        let mut report = report(
            &request,
            "a",
            status,
            Some(&code),
            message.as_deref().or(Some(raw)),
        );
        report.credential_scope = scope;
        report.diagnostics.as_mut().unwrap().upstream_request_scoped =
            native_api::upstream_errors::request_fault(status, &value);
        assert_eq!(native_soft_failure_unbinds(&report), !skip, "{name}");
        host.record_failure_cooldown(&report);
        if skip {
            assert!(host.cooldowns.lock().unwrap().is_empty(), "{name}");
        } else if matches!(status, 401 | 402 | 403 | 429 | 500..=599) {
            assert!(!host.cooldowns.lock().unwrap().is_empty(), "{name}");
        }
        host.record_soft_binding_result(&report).await;
        assert_eq!(
            binding(&host, "caller-a", "gpt-fixture", "session")
                .await
                .is_some(),
            skip,
            "{name}"
        );
        if skip {
            assert_eq!(
                selected(&host, "caller-a", request.clone()).await,
                "a",
                "{name}"
            );
        }
    }
}
