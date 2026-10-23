//! Real Host preparation with synthetic credentials and an isolated data directory.
//! No upstream request, user profile mutation, or proxy connection is made.
use super::*;

struct Fixture {
    _directory: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
    registration: String,
    revision: String,
    path: PathBuf,
    source: CodexAccount,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", previous);
        } else {
            std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        }
    }
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", directory.path());
        let mut source = CodexAccount::new(
            "official-fallback-source".into(),
            "fallback@example.invalid".into(),
            crate::models::codex::CodexTokens {
                access_token: "at-synthetic-local-access".into(),
                id_token: "synthetic-local-id".into(),
                refresh_token: Some("synthetic-local-refresh".into()),
            },
        );
        source.user_id = Some("synthetic-user".into());
        source.account_id = Some("synthetic-workspace".into());
        source.plan_type = Some("plus".into());
        source.egress_proxy_disabled = true;
        codex_account::save_account(&source).unwrap();
        let registration = uuid::Uuid::new_v4().to_string();
        let revision = uuid::Uuid::new_v4().to_string();
        let encrypted = crate::modules::secure_account_storage::serialize_account_file(
            "chatgpt_plan",
            &json!({"client_id":"oaiapp_synthetic","subject":"synthetic-official-subject",
                "access_token":"synthetic-official-access","refresh_token":"synthetic-official-refresh",
                "id_token":"synthetic-official-id","expires_at":chrono::Utc::now().timestamp()+86400,
                "earliest_refresh_at":null,"scopes":["offline_access","resource.invoke","chatgpt.tokens.use.direct"]}),
        ).unwrap();
        let path = directory
            .path()
            .join("chatgpt-plan")
            .join(format!("{registration}.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, json!({
            "summary":{"id":registration,"label":"Synthetic grant","email":source.email,
                "subject":"synthetic-official-subject","client_id":"oaiapp_synthetic",
                "source_account_id":source.id,"connected":true,"needs_reauth":false,
                "verified_at":1,"verified_model":"gpt-6.1-sol","expires_at":chrono::Utc::now().timestamp()+86400,
                "created_at":1,"updated_at":1},
            "revision":"synthetic-token-revision","authorization_revision":revision,
            "encrypted_credentials":encrypted,"source_binding_required":true,
            "source_binding":{"source_account_id":source.id,
                "identity_fingerprint":crate::modules::chatgpt_plan::source_identity_fingerprint(&source).unwrap()},
            "model_catalog":["gpt-6.1-sol"]
        }).to_string()).unwrap();
        Self {
            _directory: directory,
            previous,
            registration,
            revision,
            path,
            source,
        }
    }

    fn host(&self, official: bool) -> NativeHostBridge {
        let mut collection: CodexLocalAccessCollection = serde_json::from_value(json!({
            "enabled":true,"port":1455,"apiKey":"synthetic-primary","accountIds":[self.source.id],
            "createdAt":1,"updatedAt":1,"sessionAffinity":false,
            "maxAccountConcurrency":1,"accountConcurrencyWaitMs":0,
            "apiKeys":[{"id":"synthetic-caller","label":"Synthetic key","key":"synthetic-secret"}]
        }))
        .unwrap();
        if official {
            collection
                .official_account_bindings
                .insert(self.source.id.clone(), self.registration.clone());
        }
        let mut host = NativeHostBridge::for_collection(collection);
        host.account_overrides
            .insert(self.source.id.clone(), self.source.clone());
        host
    }

    fn mutate_registration(&self, mutate: impl FnOnce(&mut Value)) {
        let mut value =
            serde_json::from_str(&std::fs::read_to_string(&self.path).unwrap()).unwrap();
        mutate(&mut value);
        std::fs::write(&self.path, value.to_string()).unwrap();
    }
}

fn caller() -> native_contract::Caller {
    native_contract::Caller {
        id: "synthetic-caller".into(),
        label: "Synthetic key".into(),
    }
}

fn request() -> native_contract::RouteRequest {
    native_contract::RouteRequest {
        model: "gpt-6.1-sol".into(),
        session_id: None,
        thread_id: None,
        parent_thread_id: None,
        preferred_account_id: None,
        soft_preferred_account_id: None,
        target_account_id: None,
        force_refresh: false,
        image_generation_requested: false,
        image_input: false,
        excluded_account_ids: vec![],
        requires_permit: true,
        auxiliary: None,
        required_capabilities: vec![],
        capability_fallback_from: None,
    }
}

fn rejected() -> native_contract::OfficialFallbackFailure {
    native_contract::OfficialFallbackFailure {
        status: 403,
        code: "subscription_sharing_user_not_eligible".into(),
        message: Some("Synthetic eligibility rejection".into()),
        retry_after: None,
        trigger_stage: "upstream",
    }
}

async fn official_route(host: &NativeHostBridge) -> native_contract::Route {
    let route = native_contract::Host::route(host, &caller(), request())
        .await
        .unwrap();
    assert!(route.chatgpt_plan);
    let mut failed = route.clone();
    failed.permit = None;
    drop(route); // The actual engine releases the official source permit before local preparation.
    failed
}

#[tokio::test]
async fn official_quota_observation_precedes_replay_and_preserves_local_health() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let host = fixture.host(true);
    let failed = official_route(&host).await;
    let health_path = fixture.path.with_extension("official-health");
    assert!(
        !health_path.exists(),
        "selecting an unattempted candidate is neutral"
    );
    let mut failure = native_contract::OfficialFallbackFailure {
        status: 429,
        code: "subscription_sharing_usage_limit_exceeded".into(),
        message: Some("Synthetic subscription sharing usage limit".into()),
        retry_after: Some(Duration::from_secs(24 * 60 * 60)),
        trigger_stage: "preflight",
    };
    native_contract::Host::observe_official_failure(&host, &failed, &failed.model, &failure).await;
    assert!(
        !health_path.exists(),
        "preflight is not an upstream rejection"
    );
    failure.trigger_stage = "upstream";
    let before = now_ms();
    // No fallback call is made: even an unsafe/unrebuildable history must have
    // its actual quota failure recorded before returning to the client.
    native_contract::Host::observe_official_failure(&host, &failed, &failed.model, &failure).await;
    let health: Value = serde_json::from_slice(&std::fs::read(&health_path).unwrap()).unwrap();
    assert_eq!(health["states"][""]["code"], failure.code);
    assert!(health["states"][""]["retry_at_ms"].as_i64().unwrap() >= before + 86_400_000);
    assert!(
        host.cooldowns.lock().unwrap().is_empty(),
        "official failure cannot cool local credentials"
    );
    let next = native_contract::Host::route(&host, &caller(), request())
        .await
        .unwrap();
    assert!(next.chatgpt && !next.chatgpt_plan);
    assert_eq!(
        next.official_fallback_failure
            .as_ref()
            .unwrap()
            .trigger_stage,
        "health_pause"
    );
    assert_eq!(
        host.collection().await.unwrap().official_account_bindings[&fixture.source.id],
        fixture.registration
    );
}

fn official_host_report(
    fixture: &Fixture,
    route: &native_contract::Route,
) -> native_contract::RequestReport {
    native_contract::RequestReport {
        official_access_fingerprint: Some(crate::modules::chatgpt_plan::token_fingerprint(
            "synthetic-official-access",
        )),
        official_authorization_revision: Some(fixture.revision.clone()),
        official_registration_id: Some(fixture.registration.clone()),
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        official_authorization_error: None,
        operation: "responses",
        diagnostics: Some(native_api::diagnostics::DiagnosticContext {
            failure_phase: Some("downstream".into()),
            attempts: vec![native_api::diagnostics::DiagnosticAttempt {
                authorization_route: Some("official".into()),
                account_id: route.account_id.clone(),
                model_id: route.model.clone(),
                transport: "http".into(),
                started_at_ms: now_ms() - 100,
                status: Some(200),
                success: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
        caller: caller(),
        account_id: route.account_id.clone(),
        requested_model: route.model.clone(),
        upstream_model: route.model.clone(),
        status: 502,
        success: false,
        elapsed: Duration::from_millis(100),
        usage: None,
        error: Some("adapter_invalid_response".into()),
        error_message: Some("Synthetic local adapter failure".into()),
        credential_scope: false,
        retry_after: None,
        requested_service_tier: None,
        response_service_tier: None,
        reasoning_effort: None,
    }
}

#[tokio::test]
async fn official_host_record_requires_actual_upstream_failure_not_local_adapter_status() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let host = fixture.host(true);
    let official = official_route(&host).await;
    let report = official_host_report(&fixture, &official);
    let path = fixture.path.with_extension("official-health");
    for kind in [
        "downstream",
        "upstream_200",
        "no_attempt",
        "preflight",
        "wrong_account",
    ] {
        let mut synthetic = report.clone();
        let diagnostics = synthetic.diagnostics.as_mut().unwrap();
        match kind {
            "upstream_200" => diagnostics.failure_phase = Some("upstream".into()),
            "no_attempt" => diagnostics.attempts.clear(),
            "preflight" => {
                diagnostics.failure_phase = Some("upstream".into());
                let attempt = &mut diagnostics.attempts[0];
                attempt.transport = "preflight".into();
                attempt.status = Some(429);
                attempt.success = false;
                attempt.failure_phase = Some("upstream".into());
            }
            "wrong_account" => diagnostics.attempts[0].account_id = "another-source".into(),
            _ => {}
        }
        native_contract::Host::record(&host, synthetic).await;
        assert!(
            !path.exists(),
            "{kind} cannot attribute a local failure to official credentials"
        );
    }
    let mut rejected = report.clone();
    rejected.status = 429;
    rejected.error = Some("subscription_sharing_usage_limit_exceeded".into());
    rejected.retry_after = Some(Duration::from_secs(3600));
    let diagnostics = rejected.diagnostics.as_mut().unwrap();
    diagnostics.failure_phase = Some("upstream".into());
    let attempt = &mut diagnostics.attempts[0];
    attempt.status = Some(429);
    attempt.success = false;
    attempt.failure_phase = Some("upstream".into());
    attempt.error_category = rejected.error.clone();
    native_contract::Host::record(&host, rejected).await;
    let health = std::fs::read(&path).unwrap();
    let state: Value = serde_json::from_slice(&health).unwrap();
    assert_eq!(
        state["states"][""]["code"],
        "subscription_sharing_usage_limit_exceeded"
    );
    native_contract::Host::record(&host, report).await;
    assert_eq!(
        std::fs::read(path).unwrap(),
        health,
        "an adapter failure cannot rewrite or extend the genuine quota window"
    );
    assert!(
        !crate::modules::chatgpt_plan::official_route_snapshot(
            &fixture.registration,
            &fixture.source.id,
            &official.model,
        )
        .await
        .unwrap()
        .available
    );
    assert!(host.cooldowns.lock().unwrap().is_empty());
}

#[tokio::test]
async fn official_failure_observation_rejects_replaced_authorization() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let host = fixture.host(true);
    let failed = official_route(&host).await;
    fixture.mutate_registration(|record| {
        record["authorization_revision"] = json!("replacement-epoch")
    });
    native_contract::Host::observe_official_failure(&host, &failed, &failed.model, &rejected())
        .await;
    assert!(!fixture.path.with_extension("official-health").exists());
    assert!(
        native_contract::Host::route(&host, &caller(), request())
            .await
            .unwrap()
            .chatgpt_plan
    );
}

#[tokio::test]
async fn official_failure_observation_cannot_cool_a_replaced_access_token() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let host = fixture.host(true);
    let failed = official_route(&host).await;
    let encrypted = crate::modules::secure_account_storage::serialize_account_file(
        "chatgpt_plan",
        &json!({"client_id":"oaiapp_synthetic","subject":"synthetic-official-subject",
            "access_token":"replacement-official-access","refresh_token":"synthetic-official-refresh",
            "id_token":"synthetic-official-id","expires_at":chrono::Utc::now().timestamp()+86400,
            "earliest_refresh_at":null,"scopes":["offline_access","resource.invoke","chatgpt.tokens.use.direct"]}),
    ).unwrap();
    fixture.mutate_registration(|record| record["encrypted_credentials"] = json!(encrypted));
    native_contract::Host::observe_official_failure(&host, &failed, &failed.model, &rejected())
        .await;
    assert!(!fixture.path.with_extension("official-health").exists());
    assert!(
        native_contract::Host::route(&host, &caller(), request())
            .await
            .unwrap()
            .chatgpt_plan
    );
}

#[tokio::test]
async fn official_quota_exhausted_local_source_requires_verified_unpin_and_scoped_backup() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let mut host = fixture.host(true);
    let failed = official_route(&host).await;
    let mut backup = fixture.source.clone();
    backup.id = "quota-backup-source".into();
    backup.email = "backup@example.invalid".into();
    backup.user_id = Some("synthetic-backup-user".into());
    backup.account_id = Some("synthetic-backup-workspace".into());
    backup.tokens.access_token = "at-synthetic-backup-access".into();
    codex_account::save_account(&backup).unwrap();
    let collection = host.collection_override.as_mut().unwrap();
    collection.account_ids.push(backup.id.clone());
    let mut restricted_key = collection.api_keys[0].clone();
    restricted_key.id = "restricted-caller".into();
    restricted_key.key = "restricted-secret".into();
    restricted_key.inherit_account_pool = Some(false);
    restricted_key.account_ids = vec![fixture.source.id.clone()];
    collection.api_keys.push(restricted_key);
    host.account_overrides
        .insert(backup.id.clone(), backup.clone());
    host.account_overrides
        .get_mut(&fixture.source.id)
        .unwrap()
        .quota =
        Some(serde_json::from_value(json!({"hourly_percentage":0,"weekly_percentage":0})).unwrap());
    let failure = native_contract::OfficialFallbackFailure {
        status: 429,
        code: "subscription_sharing_usage_limit_exceeded".into(),
        message: None,
        retry_after: Some(Duration::from_secs(3600)),
        trigger_stage: "upstream",
    };
    native_contract::Host::observe_official_failure(&host, &failed, &failed.model, &failure).await;
    let health_path = fixture.path.with_extension("official-health");
    let health = std::fs::read(&health_path).unwrap();
    let owner = format!(
        "{}{}",
        codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX,
        fixture.registration
    );
    let mut pinned = request();
    pinned.preferred_account_id = Some(owner.clone());
    let error = native_contract::Host::route(&host, &caller(), pinned)
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.code, "account_binding_unavailable",
        "host must not detach protected history merely because a backup exists"
    );
    // Mapping alone may clear this hard pin after it has verified and rebuilt
    // portable history. The host then skips the cooled official/unusable local
    // source and chooses the permitted backup without an upstream attempt.
    let mut rebuilt = request();
    rebuilt.excluded_account_ids.push(owner);
    let route = native_contract::Host::route(&host, &caller(), rebuilt.clone())
        .await
        .unwrap();
    assert_eq!(route.account_id, backup.id);
    assert!(!route.chatgpt_plan && route.official_fallback_registration_id.is_none());
    assert!(
        route.official_fallback_failure.is_none(),
        "an unattempted cooled candidate is not a fallback attempt"
    );
    drop(route);
    let restricted = native_contract::Caller {
        id: "restricted-caller".into(),
        label: "restricted".into(),
    };
    let error = native_contract::Host::route(&host, &restricted, rebuilt)
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.code, "no_available_account",
        "portable history never widens the caller's account scope"
    );
    assert_eq!(
        std::fs::read(health_path).unwrap(),
        health,
        "candidate selection cannot add health failures or extend the quota window"
    );
    assert!(host.cooldowns.lock().unwrap().is_empty());
}

#[tokio::test]
async fn official_fallback_host_pauses_grant_and_next_request_uses_local_without_mutating_binding()
{
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let host = fixture.host(true);
    let failed = official_route(&host).await;
    native_contract::Host::observe_official_failure(&host, &failed, &failed.model, &rejected())
        .await;
    let local =
        native_contract::Host::fallback_to_local(&host, &caller(), &failed, request(), rejected())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(local.account_id, fixture.source.id);
    assert!(!local.chatgpt_plan && local.chatgpt);
    assert!(local.official_registration_id.is_none());
    assert_eq!(
        local.official_fallback_registration_id.as_deref(),
        Some(fixture.registration.as_str())
    );
    assert_eq!(
        local.official_fallback_authorization_revision.as_deref(),
        Some(fixture.revision.as_str())
    );
    assert!(local.proxy_url.is_none());
    assert_eq!(
        local.headers.get(AUTHORIZATION).unwrap(),
        "Bearer at-synthetic-local-access"
    );
    assert_eq!(
        host.collection().await.unwrap().official_account_bindings[&fixture.source.id],
        fixture.registration
    );
    let blocked = native_contract::Host::route(&host, &caller(), request())
        .await
        .err()
        .unwrap();
    assert_eq!(blocked.code, "account_busy");
    drop(local);
    let next = native_contract::Host::route(&host, &caller(), request())
        .await
        .unwrap();
    assert!(!next.chatgpt_plan);
    let evidence = next.official_fallback_failure.as_ref().unwrap();
    assert_eq!(
        (evidence.status, evidence.trigger_stage),
        (0, "health_pause")
    );
    assert_eq!(evidence.code, "subscription_sharing_user_not_eligible");
    native_contract::Host::route_failed_attempt(&host, &next, &next.model, 503, None, false, None)
        .await;
    assert!(!host.cooldowns.lock().unwrap().is_empty());
    let before = std::fs::read(fixture.path.with_extension("official-health")).unwrap();
    native_contract::Host::route_failed_attempt(&host, &next, &next.model, 401, None, true, None)
        .await;
    assert_eq!(
        before,
        std::fs::read(fixture.path.with_extension("official-health")).unwrap()
    );
    drop(next);
    // An exhausted local source cannot cause another call to a paused official route.
    let mut no_quota = fixture.host(true);
    no_quota
        .account_overrides
        .get_mut(&fixture.source.id)
        .unwrap()
        .quota =
        Some(serde_json::from_value(json!({"hourly_percentage":0,"weekly_percentage":0})).unwrap());
    assert!(
        native_contract::Host::route(&no_quota, &caller(), request())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn official_fallback_host_refuses_scope_revision_and_binding_changes() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let mut host = fixture.host(true);
    let failed = official_route(&host).await;
    host.collection_override.as_mut().unwrap().api_keys[0].allowed_models =
        vec!["another-model".into()];
    let changed =
        native_contract::Host::fallback_to_local(&host, &caller(), &failed, request(), rejected())
            .await
            .err()
            .unwrap();
    assert_eq!(changed.code, "official_fallback_authorization_changed");
    let mut host = fixture.host(true);
    let local =
        native_contract::Host::fallback_to_local(&host, &caller(), &failed, request(), rejected())
            .await
            .unwrap()
            .unwrap();
    host.collection_override.as_mut().unwrap().api_keys[0].excluded_models = vec!["gpt-*".into()];
    let error = native_contract::Host::prepare_body(
        &host,
        &caller(),
        &local,
        &axum::http::HeaderMap::new(),
        json!({"input":"fixture"}),
        false,
        false,
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error.code, "official_fallback_authorization_changed");
    drop(local);
    fixture.mutate_registration(|record| {
        record["authorization_revision"] = json!(uuid::Uuid::new_v4().to_string())
    });
    let host = fixture.host(true);
    let changed =
        native_contract::Host::fallback_to_local(&host, &caller(), &failed, request(), rejected())
            .await
            .err()
            .unwrap();
    assert_eq!(changed.code, "official_fallback_authorization_changed");
    // A new grant does not inherit the old eligibility pause or local cooldown.
    let fresh = native_contract::Host::route(&host, &caller(), request())
        .await
        .unwrap();
    assert!(fresh.chatgpt_plan);
    drop(fresh);
    let mut host = fixture.host(true);
    host.collection_override
        .as_mut()
        .unwrap()
        .official_account_bindings
        .clear();
    assert_eq!(
        native_contract::Host::fallback_to_local(&host, &caller(), &failed, request(), rejected())
            .await
            .err()
            .unwrap()
            .code,
        "official_fallback_authorization_changed"
    );
}

#[tokio::test]
async fn official_fallback_host_preserves_local_only_and_reports_model_preflight() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let host = fixture.host(false);
    let local = native_contract::Host::route(&host, &caller(), request())
        .await
        .unwrap();
    assert!(!local.chatgpt_plan && local.official_fallback_registration_id.is_none());
    assert!(host
        .collection()
        .await
        .unwrap()
        .official_account_bindings
        .is_empty());
    drop(local);
    fixture.mutate_registration(|record| record["model_catalog"] = json!(["different-model"]));
    let host = fixture.host(true);
    let local = native_contract::Host::route(&host, &caller(), request())
        .await
        .unwrap();
    assert!(!local.chatgpt_plan);
    let evidence = local.official_fallback_failure.as_ref().unwrap();
    assert_eq!(
        (evidence.code.as_str(), evidence.trigger_stage),
        ("capability_unsupported", "preflight")
    );
    assert_eq!(
        host.collection().await.unwrap().official_account_bindings[&fixture.source.id],
        fixture.registration
    );
}

async fn capability_local_route(host: &NativeHostBridge) -> native_contract::Route {
    let failed = official_route(host).await;
    let mut failure = rejected();
    failure.status = 400;
    failure.code = "subscription_sharing_unsupported_capability".into();
    let mut local =
        native_contract::Host::fallback_to_local(host, &caller(), &failed, request(), failure)
            .await
            .unwrap()
            .unwrap();
    // The engine releases the previous permit before every credential or
    // transport refresh; retain only the credential namespace and scope.
    local.permit = None;
    local
}

#[tokio::test]
async fn official_fallback_host_refresh_preserves_local_credentials_when_official_is_healthy() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for force in [false, true] {
        let fixture = Fixture::new();
        let host = fixture.host(true);
        let local = capability_local_route(&host).await;
        let normal = native_contract::Host::route(&host, &caller(), request())
            .await
            .unwrap();
        assert!(
            normal.chatgpt_plan,
            "a capability rejection must not pause the official grant"
        );
        drop(normal);
        // Reuse a newer generation just as a concurrent 401 refresh does. This
        // exercises the real credential transaction without contacting OAuth.
        let mut latest = fixture.source.clone();
        latest.token_generation += 1;
        latest.tokens.access_token = "at-synthetic-refreshed-local-access".into();
        codex_account::save_account(&latest).unwrap();
        let mut refresh = request();
        refresh.force_refresh = force;
        refresh.preferred_account_id = Some(local.account_id.clone());
        let refreshed = native_contract::Host::refresh_route(&host, &caller(), &local, refresh)
            .await
            .unwrap();
        assert!(refreshed.chatgpt && !refreshed.chatgpt_plan);
        assert_eq!(refreshed.account_id, local.account_id);
        assert_eq!(refreshed.owner_id, local.owner_id);
        assert_eq!(
            refreshed.official_fallback_registration_id,
            local.official_fallback_registration_id
        );
        assert_eq!(
            refreshed.official_fallback_authorization_revision,
            local.official_fallback_authorization_revision
        );
        assert_eq!(
            refreshed.authorization_scope_fingerprint,
            local.authorization_scope_fingerprint
        );
        assert_eq!(
            refreshed.headers[AUTHORIZATION],
            if force {
                "Bearer at-synthetic-refreshed-local-access"
            } else {
                "Bearer at-synthetic-local-access"
            }
        );
        assert_eq!(
            host.collection().await.unwrap().official_account_bindings[&fixture.source.id],
            fixture.registration
        );
    }
}

#[tokio::test]
async fn official_fallback_host_refresh_rejects_scope_revision_binding_and_source_changes() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for force in [false, true] {
        for change in ["scope", "revision", "binding", "source"] {
            let fixture = Fixture::new();
            let mut host = fixture.host(true);
            let local = capability_local_route(&host).await;
            match change {
                "scope" => host.collection_override.as_mut().unwrap().api_keys[0]
                    .allowed_models
                    .push("new-scope".into()),
                "revision" => fixture.mutate_registration(|record| {
                    record["authorization_revision"] = json!("replacement-epoch")
                }),
                "binding" => host
                    .collection_override
                    .as_mut()
                    .unwrap()
                    .official_account_bindings
                    .clear(),
                "source" => {
                    host.account_overrides
                        .get_mut(&fixture.source.id)
                        .unwrap()
                        .account_id = Some("replacement-workspace".into())
                }
                _ => unreachable!(),
            }
            let mut refresh = request();
            refresh.force_refresh = force;
            refresh.preferred_account_id = Some(local.account_id.clone());
            let error = native_contract::Host::refresh_route(&host, &caller(), &local, refresh)
                .await
                .err()
                .unwrap();
            assert_eq!(
                (error.status, error.code),
                (409, "official_fallback_authorization_changed"),
                "{change}, force={force}"
            );
        }
    }
}

#[test]
fn official_fallback_scope_is_deterministic_ignores_usage_and_tracks_permissions() {
    let caller = caller();
    let mut a: CodexLocalAccessCollection = serde_json::from_value(json!({
        "enabled":true,"port":1455,"apiKey":"fixture","accountIds":["a","b"],"createdAt":1,"updatedAt":1,
        "officialAccountBindings":{"b":"second","a":"first"},
        "apiKeys":[{"id":caller.id,"label":"fixture","key":"fixture"}]
    }))
    .unwrap();
    let expected = native_official_scope_fingerprint(&a, &caller);
    let mut b = a.clone();
    b.official_account_bindings =
        HashMap::from([("a".into(), "first".into()), ("b".into(), "second".into())]);
    b.api_keys[0].last_used_at = Some(100);
    b.api_keys[0].token_used = 99;
    assert_eq!(expected, native_official_scope_fingerprint(&b, &caller));
    a.api_keys[0].allowed_models.push("allowed".into());
    assert_ne!(expected, native_official_scope_fingerprint(&a, &caller));
    assert!(!native_official_prepare_recoverable(&native_error(
        409,
        "account_binding_unavailable",
        "fixture"
    )));
}

#[tokio::test]
async fn official_fallback_host_keeps_local_quota_and_proxy_enforcement_separate() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let mut host = fixture.host(true);
    host.collection_override
        .as_mut()
        .unwrap()
        .bound_oauth_account_id = Some(fixture.source.id.clone());
    host.collection_override
        .as_mut()
        .unwrap()
        .bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
        hourly_percent: 20,
        weekly_percent: 20,
        strict: true,
        safety_margin_percent: 3,
    });
    host.account_overrides
        .get_mut(&fixture.source.id)
        .unwrap()
        .quota =
        Some(serde_json::from_value(json!({"hourly_percentage":0,"weekly_percentage":0})).unwrap());
    // The local reserve must not reject an independent official authorization.
    let failed = official_route(&host).await;
    native_contract::Host::observe_official_failure(&host, &failed, &failed.model, &rejected())
        .await;
    let unavailable =
        native_contract::Host::fallback_to_local(&host, &caller(), &failed, request(), rejected())
            .await
            .err()
            .unwrap();
    assert_eq!(
        (unavailable.status, unavailable.code),
        (429, "official_local_unavailable")
    );
    let mut host = fixture.host(true);
    let source = host.account_overrides.get_mut(&fixture.source.id).unwrap();
    source.egress_proxy_disabled = false;
    source.egress_proxy_url = Some("http://127.0.0.1:9".into());
    // The existing managed proxy router reads the durable account binding.
    codex_account::save_account(source).unwrap();
    let route = native_contract::Host::route(&host, &caller(), request())
        .await
        .unwrap();
    assert!(!route.chatgpt_plan);
    assert!(route.proxy_url.is_some());
    assert_eq!(route.proxy_route.as_ref().unwrap()["kind"], "proxy");
    assert_eq!(
        codex_account::load_account(&fixture.source.id)
            .unwrap()
            .egress_proxy_url
            .as_deref(),
        Some("http://127.0.0.1:9")
    );
}
