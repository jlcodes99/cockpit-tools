use super::*;

#[path = "codex_local_access_member_pause_tests.rs"]
mod paused;

fn source() -> CodexAccount {
    let mut account = CodexAccount::new(
        "codex-source".into(),
        "source@example.invalid".into(),
        crate::models::codex::CodexTokens {
            access_token: "synthetic-source-access".into(),
            id_token: "synthetic-source-id".into(),
            refresh_token: Some("synthetic-source-refresh".into()),
        },
    );
    account.plan_type = Some("plus".into());
    account.user_id = Some("fixture-user".into());
    account.account_id = Some("fixture-workspace".into());
    account
}

fn projection() -> CodexAccount {
    CodexAccount::new_api_key(
        "chatgpt_plan_12345678-1234-4234-8234-123456789abc".into(),
        "source@example.invalid".into(),
        "chatgpt-plan-managed".into(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some("https://api.openai.com/v1".into()),
        Some(codex_account::CHATGPT_PLAN_PROVIDER_ID.into()),
        Some("ChatGPT".into()),
        vec!["official-model".into()],
    )
}

fn collection() -> CodexLocalAccessCollection {
    serde_json::from_value(json!({
        "enabled":false,"port":19009,"apiKey":"synthetic-service-key",
        "accountIds":["codex-source","third"],"createdAt":1,"updatedAt":1,
        "boundOauthAccountId":"codex-source","sessionAffinity":true,
        "imageGenerationAccountPolicies":{"codex-source":"enabled"},
        "customRoutingRules":[{"accountId":"codex-source","priority":7,"weight":3,"isBackup":true}],
        "accountModelRules":[{"accountId":"codex-source","excludedModels":["blocked-model"]}],
        "apiKeys":[
            {"id":"source-key","label":"source","key":"synthetic-source-key","inheritAccountPool":false,
             "accountIds":["codex-source"],"priorityAccountIds":["codex-source"],"enabled":true,"createdAt":1,"updatedAt":1},
            {"id":"third-key","label":"third","key":"synthetic-third-key","inheritAccountPool":false,
             "accountIds":["third"],"enabled":true,"createdAt":1,"updatedAt":1}
        ]
    })).unwrap()
}

fn switch(current: &CodexLocalAccessCollection, official: bool) -> CodexLocalAccessCollection {
    let projection = projection();
    switched_chatgpt_plan_collection(
        current,
        &source(),
        Some(&projection),
        Some(&projection.id),
        official,
    )
    .unwrap()
}

#[test]
fn member_switch_preserves_scoped_keys_model_restrictions_and_original_codex_image_policy() {
    let original = collection();
    let official = switch(&original, true);
    let id = source().id;
    assert_eq!(official.official_account_bindings[&id], "12345678-1234-4234-8234-123456789abc");
    assert_eq!(official.account_ids, vec![id.clone(), "third".into()]);
    assert_eq!(official.api_keys[0].account_ids, vec![id.clone()]);
    assert_eq!(official.api_keys[0].priority_account_ids, vec![id.clone()]);
    assert_eq!(official.api_keys[0].inherit_account_pool, Some(false));
    assert_eq!(official.api_keys[1].account_ids, vec!["third"]);
    assert_eq!(official.custom_routing_rules[0].account_id, id);
    assert_eq!(official.custom_routing_rules[0].priority, 7);
    assert_eq!(official.custom_routing_rules[0].weight, 3);
    assert!(official.custom_routing_rules[0].is_backup);
    assert_eq!(
        official.account_model_rules[0].excluded_models,
        vec!["blocked-model"]
    );
    assert_eq!(
        official.bound_oauth_account_id,
        original.bound_oauth_account_id
    );
    assert_eq!(official.image_generation_account_policies, original.image_generation_account_policies);
    assert!(official.chatgpt_plan_codex_image_policies.is_empty());
    assert_eq!(official.chatgpt_plan_service_revision, 1);
    let decoded: CodexLocalAccessCollection =
        serde_json::from_str(&serde_json::to_string(&official).unwrap()).unwrap();
    let codex = switch(&decoded, false);
    assert_eq!(codex.account_ids, original.account_ids);
    assert_eq!(
        codex.api_keys[0].account_ids,
        original.api_keys[0].account_ids
    );
    assert_eq!(codex.custom_routing_rules, original.custom_routing_rules);
    assert_eq!(codex.account_model_rules, original.account_model_rules);
    assert_eq!(
        codex.image_generation_account_policies,
        original.image_generation_account_policies
    );
    assert!(codex.chatgpt_plan_codex_image_policies.is_empty());
    assert!(codex.official_account_bindings.is_empty());
    assert_eq!(
        codex.bound_oauth_account_id,
        original.bound_oauth_account_id
    );
    assert_eq!(codex.session_affinity, original.session_affinity);
    assert_eq!(codex.gateway_mode, original.gateway_mode);
    assert_eq!(codex.chatgpt_plan_service_revision, 2);
}

#[test]
fn member_switch_dual_legacy_members_keep_target_priority_and_merge_only_model_exclusions() {
    let mut current = collection();
    let id = projection().id;
    current.account_ids.push(id.clone());
    current
        .custom_routing_rules
        .push(CodexLocalAccessCustomRoutingRule {
            account_id: id.clone(),
            priority: 13,
            weight: 9,
            is_backup: false,
            is_preferred: true,
        });
    current
        .account_model_rules
        .push(CodexLocalAccessAccountModelRule {
            account_id: id.clone(),
            excluded_models: vec!["target-blocked".into()],
        });
    let mut official_only_key = current.api_keys[0].clone();
    official_only_key.id = "official-key".into();
    official_only_key.account_ids = vec![id.clone()];
    official_only_key.priority_account_ids.clear();
    current.api_keys.push(official_only_key);
    let official = switch(&current, true);
    let id = source().id;
    assert_eq!(official.account_ids, vec![id.clone(), "third".into()]);
    assert_eq!(official.custom_routing_rules.len(), 1);
    assert_eq!(official.custom_routing_rules[0].priority, 7);
    assert_eq!(official.custom_routing_rules[0].weight, 3);
    assert_eq!(official.account_model_rules.len(), 1);
    for model in ["blocked-model", "target-blocked"] {
        assert!(official.account_model_rules[0]
            .excluded_models
            .contains(&model.into()));
    }
    assert_eq!(official.api_keys[0].account_ids, vec![id.clone()]);
    assert_eq!(official.api_keys[2].account_ids, vec![id]);
    assert_eq!(official.api_keys[1].account_ids, vec!["third"]);
    let codex = switch(&official, false);
    assert_eq!(codex.api_keys[0].account_ids, vec!["codex-source"]);
    assert_eq!(codex.api_keys[2].account_ids, vec!["codex-source"]);
    assert_eq!(codex.api_keys[1].account_ids, vec!["third"]);
    assert_eq!(codex.account_model_rules[0].excluded_models.len(), 2);
}

#[test]
fn member_switch_does_not_rewrite_provider_routes_or_nonmember_scopes() {
    let mut current = collection();
    current.api_keys[0].model_routing = Some(serde_json::from_value(json!({
        "defaultRoute":"oauth","failurePolicy":"strict","routes":[{
            "id":"separate-route","namespace":"other","providerAccountId":"codex-source"
            ,"providerGateway":{"baseUrl":"https://provider.example.invalid/v1","apiKey":"synthetic-provider-key","upstreamModel":"third-model"}
        }]
    })).unwrap());
    let mut provider_key = current.api_keys[0].clone();
    provider_key.id = "provider-key".into();
    provider_key.provider_gateway = Some(serde_json::from_value(json!({
        "baseUrl":"https://provider.example.invalid/v1","apiKey":"synthetic-provider-key","upstreamModel":"third-model"
    })).unwrap());
    current.api_keys.push(provider_key);
    let provider_before = serde_json::to_value(&current.api_keys[2]).unwrap();
    let official = switch(&current, true);
    assert_eq!(
        serde_json::to_value(&official.api_keys[2]).unwrap(),
        provider_before
    );
    assert_eq!(
        serde_json::to_value(&official.api_keys[0].model_routing).unwrap(),
        serde_json::to_value(&current.api_keys[0].model_routing).unwrap()
    );
    let mut not_a_member = current;
    not_a_member.account_ids = vec!["third".into()];
    let added = switch(&not_a_member, true);
    assert_eq!(added.api_keys[0].account_ids, vec!["codex-source"]);
    assert_eq!(added.api_keys[1].account_ids, vec!["third"]);
    assert!(added.account_ids.contains(&source().id));
    assert_eq!(added.official_account_bindings[&source().id], "12345678-1234-4234-8234-123456789abc");
}

#[test]
fn member_switch_keeps_explicit_codex_image_source_independent_and_never_aliases_continuity_owner()
{
    let mut current = collection();
    current.image_generation_account_ids = vec![source().id.clone()];
    current
        .image_generation_account_models
        .insert(source().id.clone(), "gpt-image-2".into());
    let official = switch(&current, true);
    assert_eq!(
        official.image_generation_account_ids,
        current.image_generation_account_ids
    );
    assert_eq!(
        official.image_generation_account_models,
        current.image_generation_account_models
    );
    assert_eq!(
        official.image_generation_account_policies[&source().id],
        CodexLocalAccessImageGenerationPolicy::Enabled
    );
    assert!(!official.account_ids.contains(&projection().id));
    assert!(official.official_account_bindings.contains_key(&source().id));
    assert_ne!(
        continuity_account_owner(source()).unwrap(),
        continuity_account_owner(projection()).unwrap()
    );
    assert_eq!(source().id, "codex-source");
    assert!(projection()
        .id
        .starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX));
}

#[test]
fn member_switch_rejects_invalid_target_or_engine_without_mutating_original() {
    let mut original = collection();
    original.gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
    let before = serde_json::to_value(&original).unwrap();
    let projection = projection();
    assert_eq!(
        switched_chatgpt_plan_collection(
            &original,
            &source(),
            Some(&projection),
            Some(&projection.id),
            true
        )
        .unwrap_err(),
        "CHATGPT_PLAN_NATIVE_REQUIRED"
    );
    assert_eq!(serde_json::to_value(&original).unwrap(), before);
    original.gateway_mode = CodexLocalAccessGatewayMode::Native;
    let invalid_source = projection.clone();
    assert_eq!(
        switched_chatgpt_plan_collection(
            &original,
            &invalid_source,
            Some(&projection),
            Some(&projection.id),
            true
        )
        .unwrap_err(),
        "CHATGPT_PLAN_SOURCE_INVALID"
    );
    let mut free = source();
    free.plan_type = Some("free".into());
    assert_eq!(
        switched_chatgpt_plan_collection(
            &original,
            &free,
            Some(&projection),
            Some(&projection.id),
            false
        )
        .unwrap_err(),
        "CHATGPT_PLAN_SERVICE_ACCOUNT_INELIGIBLE"
    );
    assert_eq!(
        switched_chatgpt_plan_collection(&original, &source(), None, None, true)
            .unwrap_err(),
        "CHATGPT_PLAN_BINDING_REQUIRED"
    );
}

#[test]
fn member_switch_retains_access_token_only_codex_eligibility_and_separate_official_grant() {
    let current = collection();
    let mut source = source();
    source.tokens.refresh_token = None;
    assert!(is_local_access_eligible_account(
        &source,
        current.restrict_free_accounts
    ));
    let codex = switched_chatgpt_plan_collection(&current, &source, None, None, false).unwrap();
    assert!(codex.account_ids.contains(&source.id));
    let projection = projection();
    let official = switched_chatgpt_plan_collection(
        &current,
        &source,
        Some(&projection),
        Some(&projection.id),
        true,
    )
    .unwrap();
    assert!(!official.account_ids.contains(&projection.id));
    assert!(official.account_ids.contains(&source.id));
    assert!(official.official_account_bindings.contains_key(&source.id));
    assert!(source.tokens.refresh_token.is_none());
}

#[test]
fn member_switch_revision_blocks_stale_disk_and_runtime_publications_and_defaults_for_old_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collection.json");
    let original = collection();
    assert_eq!(original.chatgpt_plan_service_revision, 0);
    assert!(original.chatgpt_plan_codex_image_policies.is_empty());
    let official = switch(&original, true);
    save_collection_to_path(&path, &official).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(
        save_collection_to_path(&path, &original).unwrap_err(),
        "CHATGPT_PLAN_SERVICE_CHANGED"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let mut runtime = GatewayRuntime::default();
    runtime.collection = Some(official.clone());
    sync_runtime_collection(&mut runtime, original.clone());
    assert_eq!(
        collection_content_fingerprint(runtime.collection.as_ref().unwrap()).unwrap(),
        collection_content_fingerprint(&official).unwrap()
    );
    let decoded: CodexLocalAccessCollection = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        collection_content_fingerprint(&decoded).unwrap(),
        collection_content_fingerprint(&official).unwrap()
    );
    let disk_hash = Sha256::digest(&bytes).into();
    let codex = switch(&official, false);
    fs::write(&path, b"concurrent update").unwrap();
    assert!(
        !write_string_atomic_if_hash_matches(&path, disk_hash, || Ok(serde_json::to_string(
            &codex
        )
        .unwrap()))
        .unwrap()
    );
    assert_eq!(fs::read(&path).unwrap(), b"concurrent update");
}
