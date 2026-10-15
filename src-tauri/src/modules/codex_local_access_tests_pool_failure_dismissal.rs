#[test]
fn clearing_pool_failure_preserves_other_keys_and_scheduler_state() {
    let mut runtime = super::GatewayRuntime::default();
    for key in ["key-1", "key-2"] {
        runtime.account_pool_health.insert(
            key.to_string(),
            super::RuntimeAccountPoolHealth {
                last_failure_at: 100,
                ..Default::default()
            },
        );
    }
    runtime.account_health.insert(
        "account-1".to_string(),
        super::RuntimeAccountHealth::default(),
    );
    let cooldown_key = super::build_cooldown_key("account-1", "gpt-5.5").unwrap();
    runtime.model_cooldowns.insert(
        cooldown_key.clone(),
        super::AccountModelCooldown {
            model_key: "gpt-5.5".to_string(),
            next_retry_at_ms: 2_000_000,
            reason: "unauthorized".to_string(),
        },
    );
    assert!(super::clear_runtime_account_pool_failure(
        &mut runtime,
        " key-1 ",
        100
    ));
    assert!(!runtime.account_pool_health.contains_key("key-1"));
    assert!(runtime.account_pool_health.contains_key("key-2"));
    assert!(runtime.account_health.contains_key("account-1"));
    assert!(runtime.model_cooldowns.contains_key(&cooldown_key));
    assert!(super::clear_runtime_account_pool_failure(
        &mut runtime,
        "key-1",
        100
    ));
}

#[test]
fn clearing_pool_failure_rejects_a_newer_failure_and_allows_it_to_reappear() {
    let mut runtime = super::GatewayRuntime::default();
    let event = super::SidecarAuthResultEvent {
        api_key_id: "key-1".to_string(),
        error_code: Some("auth_unavailable".to_string()),
        candidate_auths: 1,
        ..Default::default()
    };
    super::apply_sidecar_account_pool_health(&mut runtime, &event, true, 100);
    // Even reports received in the same millisecond must have distinct IDs.
    super::apply_sidecar_account_pool_health(&mut runtime, &event, true, 100);
    assert_eq!(runtime.account_pool_health["key-1"].last_failure_at, 101);
    assert!(!super::clear_runtime_account_pool_failure(
        &mut runtime,
        "key-1",
        100
    ));
    assert!(runtime.account_pool_health.contains_key("key-1"));
    assert!(super::clear_runtime_account_pool_failure(
        &mut runtime,
        "key-1",
        101
    ));
    super::apply_sidecar_account_pool_health(&mut runtime, &event, true, 100);
    assert_eq!(runtime.account_pool_health["key-1"].last_failure_at, 102);
    super::apply_sidecar_account_pool_health(&mut runtime, &event, true, 200);
    assert_eq!(runtime.account_pool_health["key-1"].last_failure_at, 200);
}

#[test]
fn clearing_pool_failure_supports_unscoped_diagnostics() {
    let mut runtime = super::GatewayRuntime::default();
    let event = super::SidecarAuthResultEvent {
        error_code: Some("auth_unavailable".to_string()),
        ..Default::default()
    };
    super::apply_sidecar_account_pool_health(&mut runtime, &event, true, 100);
    assert!(runtime
        .account_pool_health
        .contains_key(super::UNSCOPED_ACCOUNT_POOL_HEALTH_KEY));
    assert!(super::clear_runtime_account_pool_failure(
        &mut runtime,
        " ",
        100
    ));
    assert!(runtime.account_pool_health.is_empty());
}

#[test]
fn pool_scope_diagnostics_round_trip_without_changing_account_health() {
    let event: super::SidecarAuthResultEvent = serde_json::from_value(serde_json::json!({
        "apiKeyId": "key-1", "model": "gpt-5.5", "errorCode": "auth_unavailable",
        "candidateAuths": 1, "scopedAuths": 0,
        "scopeDiagnostics": [
            {"accountId": " outside ", "accountEmail": " outside@example.com ",
             "reasonCode": " scope_mismatch "},
            {"reasonCode": "account_mapping_missing"},
            {"accountId": "bound", "reasonCode": "bound_account_not_candidate"},
            {"reasonCode": " "}
        ]
    }))
    .unwrap();
    let mut runtime = super::GatewayRuntime::default();
    super::apply_sidecar_account_pool_health(&mut runtime, &event, true, 100);
    let snapshot = super::build_account_pool_health_snapshot(&runtime);
    let value = serde_json::to_value(&snapshot[0]).unwrap();
    let diagnostics = value["scopeDiagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 3);
    assert_eq!(diagnostics[0]["accountId"], "outside");
    assert_eq!(diagnostics[0]["accountEmail"], "outside@example.com");
    assert_eq!(diagnostics[0]["reasonCode"], "scope_mismatch");
    assert_eq!(diagnostics[1]["accountId"], "");
    assert_eq!(diagnostics[1]["reasonCode"], "account_mapping_missing");
    assert!(runtime.account_health.is_empty());
    assert!(runtime.model_cooldowns.is_empty());
    assert!(snapshot[0].account_statuses.is_empty());
}

#[test]
fn pool_scope_diagnostics_accept_legacy_events_and_replace_old_details() {
    let scoped: super::SidecarAuthResultEvent = serde_json::from_value(serde_json::json!({
        "apiKeyId": "key-1", "scopeDiagnostics": [{"reasonCode": "account_mapping_missing"}]
    }))
    .unwrap();
    let legacy: super::SidecarAuthResultEvent = serde_json::from_value(serde_json::json!({
        "apiKeyId": "key-1", "candidateAuths": 0
    }))
    .unwrap();
    let mut runtime = super::GatewayRuntime::default();
    super::apply_sidecar_account_pool_health(&mut runtime, &scoped, true, 100);
    assert_eq!(
        runtime.account_pool_health["key-1"].scope_diagnostics.len(),
        1
    );
    super::apply_sidecar_account_pool_health(&mut runtime, &legacy, true, 200);
    assert!(runtime.account_pool_health["key-1"]
        .scope_diagnostics
        .is_empty());
}

fn pool_request_event(id: &str, model: &str, started: i64) -> super::SidecarAuthResultEvent {
    serde_json::from_value(serde_json::json!({
        "apiKeyId": "key-1", "apiKeyLabel": "Client A", "provider": "codex",
        "requestId": id, "startedAtMs": started, "requestKind": "text", "model": model,
        "errorCode": "auth_unavailable", "errorMessage": "no auth available",
        "candidateAuths": 1, "scopedAuths": 0,
        "scopeDiagnostics": [{"accountId":"outside", "reasonCode":"scope_mismatch"}]
    }))
    .unwrap()
}

#[test]
fn pool_request_identity_survives_snapshot_and_models_clear_independently() {
    let mut runtime = super::GatewayRuntime::default();
    let first = pool_request_event("request-a", "model-a", 10);
    let second = pool_request_event("request-b", "model-b", 20);
    super::apply_sidecar_account_pool_health(&mut runtime, &first, true, 100);
    super::apply_sidecar_account_pool_health(&mut runtime, &second, true, 200);
    let value = serde_json::to_value(super::build_account_pool_health_snapshot(&runtime)).unwrap();
    assert_eq!(value[0]["requestId"], "request-b");
    assert_eq!(value[1]["requestId"], "request-a");
    let success = super::SidecarAuthResultEvent {
        success: true,
        ..second
    };
    assert!(super::apply_sidecar_account_pool_health(
        &mut runtime,
        &success,
        false,
        300
    ));
    assert_eq!(runtime.account_pool_health.len(), 1);
    assert!(runtime
        .account_pool_health
        .contains_key(&super::account_pool_route_key(&first)));
    assert!(runtime.account_health.is_empty());
    assert!(runtime.model_cooldowns.is_empty());
}

#[test]
fn pool_late_results_and_same_millisecond_success_cannot_erase_new_failure() {
    let mut runtime = super::GatewayRuntime::default();
    let latest = pool_request_event("latest", "model-a", 20);
    super::apply_sidecar_account_pool_health(&mut runtime, &latest, true, 100);
    for started in [10, 20] {
        let success = super::SidecarAuthResultEvent {
            success: true,
            ..pool_request_event("older", "model-a", started)
        };
        assert!(!super::apply_sidecar_account_pool_health(
            &mut runtime,
            &success,
            false,
            200
        ));
        assert_eq!(
            runtime
                .account_pool_health
                .values()
                .next()
                .unwrap()
                .request_id,
            "latest"
        );
    }
    let success = super::SidecarAuthResultEvent {
        success: true,
        ..pool_request_event("new-success", "model-a", 30)
    };
    assert!(super::apply_sidecar_account_pool_health(
        &mut runtime,
        &success,
        false,
        300
    ));
    assert!(!super::apply_sidecar_account_pool_health(
        &mut runtime,
        &latest,
        true,
        400
    ));
    assert!(runtime.account_pool_health.is_empty());
}

#[test]
fn pool_new_request_cannot_inherit_old_details_but_same_request_can() {
    let mut runtime = super::GatewayRuntime::default();
    let detailed = pool_request_event("detailed", "model-a", 10);
    let key = super::account_pool_route_key(&detailed);
    super::apply_sidecar_account_pool_health(&mut runtime, &detailed, true, 100);
    let same_request = pool_request_event("detailed", "model-a", 10);
    super::apply_sidecar_account_pool_health(&mut runtime, &same_request, false, 200);
    assert_eq!(runtime.account_pool_health[&key].scope_diagnostics.len(), 1);
    let new_request = pool_request_event("generic-new", "model-a", 20);
    super::apply_sidecar_account_pool_health(&mut runtime, &new_request, false, 300);
    let health = &runtime.account_pool_health[&key];
    assert!(!health.diagnostic_available);
    assert_eq!(health.candidate_auths, 0);
    assert!(health.account_statuses.is_empty());
    assert!(health.scope_diagnostics.is_empty());
    assert_eq!(health.request_id, "generic-new");
}

#[test]
fn pool_route_clearing_and_configuration_pruning_keep_other_models() {
    let mut runtime = super::GatewayRuntime::default();
    let first = pool_request_event("a", "model-a", 10);
    let second = pool_request_event("b", "model-b", 20);
    super::apply_sidecar_account_pool_health(&mut runtime, &first, true, 100);
    super::apply_sidecar_account_pool_health(&mut runtime, &second, true, 200);
    assert!(super::clear_runtime_account_pool_failure(
        &mut runtime,
        "key-1",
        100
    ));
    assert_eq!(runtime.account_pool_health.len(), 1);
    assert_eq!(
        runtime
            .account_pool_health
            .values()
            .next()
            .unwrap()
            .request_id,
        "b"
    );
    let mut collection = test_local_access_collection(Vec::new());
    collection.api_keys = vec![serde_json::from_value(serde_json::json!({
        "id":"key-1", "label":"Client A", "key":"test-key"
    }))
    .unwrap()];
    runtime.collection = Some(collection);
    super::prune_runtime_account_state(&mut runtime);
    assert_eq!(runtime.account_pool_health.len(), 1);
    assert!(!runtime.account_pool_request_watermarks.is_empty());
    runtime.collection.as_mut().unwrap().api_keys.clear();
    super::prune_runtime_account_state(&mut runtime);
    assert!(runtime.account_pool_health.is_empty());
    assert!(runtime.account_pool_request_watermarks.is_empty());
}

#[test]
fn pool_diagnostics_and_ordering_watermarks_have_a_fixed_capacity() {
    let mut runtime = super::GatewayRuntime::default();
    for index in 1..=super::ACCOUNT_POOL_DIAGNOSTIC_LIMIT + 20 {
        let event = pool_request_event(
            &format!("request-{index}"),
            &format!("model-{index}"),
            index as i64,
        );
        super::apply_sidecar_account_pool_health(&mut runtime, &event, true, index as i64);
    }
    assert_eq!(
        runtime.account_pool_health.len(),
        super::ACCOUNT_POOL_DIAGNOSTIC_LIMIT
    );
    assert_eq!(
        runtime.account_pool_request_watermarks.len(),
        super::ACCOUNT_POOL_DIAGNOSTIC_LIMIT
    );
    let snapshot = super::build_account_pool_health_snapshot(&runtime);
    assert_eq!(snapshot.last().unwrap().request_id, "request-21");
    assert_eq!(
        snapshot[0].request_id,
        format!("request-{}", super::ACCOUNT_POOL_DIAGNOSTIC_LIMIT + 20)
    );
}

#[test]
fn pool_existing_failure_keeps_ordering_after_its_watermark_is_evicted() {
    let mut runtime = super::GatewayRuntime::default();
    let latest = pool_request_event("latest", "model-a", 200);
    let key = super::account_pool_route_key(&latest);
    super::apply_sidecar_account_pool_health(&mut runtime, &latest, true, 200);
    for index in 1..=super::ACCOUNT_POOL_DIAGNOSTIC_LIMIT + 1 {
        let success = super::SidecarAuthResultEvent {
            success: true,
            ..pool_request_event(
                &format!("success-{index}"),
                &format!("model-{index}"),
                200 + index as i64,
            )
        };
        super::apply_sidecar_account_pool_health(&mut runtime, &success, false, 500);
    }
    assert!(!runtime.account_pool_request_watermarks.contains_key(&key));
    assert_eq!(runtime.account_pool_health.len(), 1);
    let older = pool_request_event("older", "model-a", 100);
    assert!(!super::apply_sidecar_account_pool_health(
        &mut runtime,
        &older,
        true,
        600
    ));
    assert_eq!(runtime.account_pool_health[&key].request_id, "latest");
}
