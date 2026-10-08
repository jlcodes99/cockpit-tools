use super::*;

fn definition(id: &str) -> CodexExperimentalModelDefinition {
    CodexExperimentalModelDefinition {
        model_id: id.to_string(),
        display_name: id.to_string(),
        reasoning_efforts: None,
        default_reasoning_effort: None,
        context_window: None,
        auto_compact_token_limit: None,
    }
}

fn snapshot(models: Vec<CodexExperimentalModelDefinition>) -> ModelConfigSnapshot {
    ModelConfigSnapshot {
        profile_dir: None,
        files: Vec::new(),
        models,
        default_model_id: None,
        service: None,
        revision: "revision".to_string(),
    }
}

fn json(models: Vec<CodexExperimentalModelDefinition>) -> String {
    serde_json::to_string(&CodexModelConfigDocument {
        schema: MODEL_CONFIG_SCHEMA.to_string(),
        version: 1,
        models,
        default_model_id: None,
        api_service: None,
    })
    .unwrap()
}

fn temp_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!("cockpit-model-config-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn import_preview_keeps_conflicts_and_replace_updates_only_named_models() {
    let original = definition("local-a");
    let mut changed = original.clone();
    changed.display_name = "Changed".to_string();
    let source = json(vec![changed.clone(), definition("local-b")]);
    let existing = snapshot(vec![original.clone(), definition("local-c")]);
    let (preview, prepared) =
        prepare_model_config_import(&existing, &source, "keep_existing").unwrap();
    assert_eq!(preview.added, vec!["models/local-b"]);
    assert_eq!(preview.conflicts, vec!["models/local-a"]);
    assert_eq!(prepared.models[0], original);
    assert_eq!(prepared.models[1].model_id, "local-c");
    let (preview, prepared) = prepare_model_config_import(&existing, &source, "replace").unwrap();
    assert_eq!(preview.updated, vec!["models/local-a"]);
    assert_eq!(prepared.models[0], changed);
    assert_eq!(prepared.models[1].model_id, "local-c");
}

#[test]
fn import_rejects_duplicates_credentials_unknown_version_and_invalid_defaults() {
    let existing = snapshot(vec![definition("local-a")]);
    let duplicate = json(vec![definition("new"), definition("NEW")]);
    let (preview, _) = prepare_model_config_import(&existing, &duplicate, "keep_existing").unwrap();
    assert!(preview
        .errors
        .iter()
        .any(|error| error.contains("MODEL_CONFIG_DUPLICATE")));
    let mut invalid = definition("gpt-5.6-sol");
    invalid.reasoning_efforts = Some(vec!["low".to_string()]);
    invalid.default_reasoning_effort = Some("high".to_string());
    let (preview, _) =
        prepare_model_config_import(&existing, &json(vec![invalid]), "replace").unwrap();
    assert!(preview
        .errors
        .iter()
        .any(|error| error.contains("DEFAULT_REASONING_INVALID")));
    let mut source: serde_json::Value =
        serde_json::from_str(&json(vec![definition("new")])).unwrap();
    source["apiKey"] = serde_json::json!("must-not-import");
    assert_eq!(
        prepare_model_config_import(&existing, &source.to_string(), "replace").unwrap_err(),
        "MODEL_CONFIG_FIELDS_UNSUPPORTED"
    );
    source.as_object_mut().unwrap().remove("apiKey");
    source["version"] = serde_json::json!(2);
    assert_eq!(
        prepare_model_config_import(&existing, &source.to_string(), "replace").unwrap_err(),
        "MODEL_CONFIG_VERSION_UNSUPPORTED"
    );
}

#[test]
fn default_reasoning_round_trips_and_decorates_provider_and_instance_catalogs() {
    let dir = temp_dir();
    let mut model = definition("gpt-5.6-sol");
    model.reasoning_efforts = Some(vec!["low".to_string(), "high".to_string()]);
    model.default_reasoning_effort = Some("high".to_string());
    persist_experimental_model_definitions(&dir, vec![model.clone()], None).unwrap();
    persist_experimental_model_policy(&dir, true).unwrap();
    let loaded = read_experimental_model_definitions(&dir);
    assert_eq!(loaded[0].default_reasoning_effort.as_deref(), Some("high"));
    let source = crate::modules::codex_protocol::build_codex_client_models_response(&[model
        .model_id
        .clone()]);
    let decorated: serde_json::Value = serde_json::from_str(
        &decorate_managed_model_catalog_for_profile(&dir, &source.to_string()).unwrap(),
    )
    .unwrap();
    assert_eq!(decorated["models"][0]["default_reasoning_level"], "high");
    assert_eq!(
        decorated["models"][0]["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    persist_experimental_model_policy(&dir, false).unwrap();
    assert_eq!(
        decorate_managed_model_catalog_for_profile(&dir, &source.to_string()).unwrap(),
        source.to_string()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn model_import_cas_failure_rolls_back_earlier_writes_and_keeps_newer_data() {
    let dir = temp_dir();
    fs::write(dir.join(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE), "old").unwrap();
    fs::write(dir.join(CODEX_CONFIG_FILE_NAME), "newer-user-edit").unwrap();
    let files = vec![
        ModelConfigFileChange {
            name: CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE.to_string(),
            before: Some("old".into()),
            after: Some("imported".into()),
        },
        ModelConfigFileChange {
            name: CODEX_CONFIG_FILE_NAME.to_string(),
            before: Some("stale".into()),
            after: Some("imported-config".into()),
        },
    ];
    assert_eq!(
        commit_model_config_files(&dir, files).unwrap_err(),
        "MODEL_CONFIG_STATE_CHANGED"
    );
    assert_eq!(
        fs::read_to_string(dir.join(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)).unwrap(),
        "old"
    );
    assert_eq!(
        fs::read_to_string(dir.join(CODEX_CONFIG_FILE_NAME)).unwrap(),
        "newer-user-edit"
    );
    assert!(!dir.join(MODEL_CONFIG_JOURNAL).exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn interrupted_import_recovers_idempotently_and_preserves_committed_state() {
    let dir = temp_dir();
    let change = ModelConfigFileChange {
        name: CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE.into(),
        before: Some("original".into()),
        after: Some("partial".into()),
    };
    fs::write(dir.join(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE), "partial").unwrap();
    fs::write(
        dir.join(MODEL_CONFIG_JOURNAL),
        serde_json::to_string(&ModelConfigImportJournal {
            version: 1,
            committed: false,
            files: vec![change.clone()],
        })
        .unwrap(),
    )
    .unwrap();
    recover_model_config_import(&dir).unwrap();
    recover_model_config_import(&dir).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)).unwrap(),
        "original"
    );
    fs::write(dir.join(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE), "partial").unwrap();
    fs::write(
        dir.join(MODEL_CONFIG_JOURNAL),
        serde_json::to_string(&ModelConfigImportJournal {
            version: 1,
            committed: true,
            files: vec![change],
        })
        .unwrap(),
    )
    .unwrap();
    recover_model_config_import(&dir).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)).unwrap(),
        "partial"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn price_alias_and_route_merges_report_each_section_and_validate_references() {
    use crate::models::codex_local_access::{
        CodexLocalAccessCustomRoutingRule, CodexLocalAccessModelAlias,
    };
    let mut preview = CodexModelConfigImportPreview::default();
    let existing = vec![CodexLocalAccessModelAlias {
        source_model: "gpt-5.6-sol".into(),
        alias: "local".into(),
        fork: false,
    }];
    let incoming = vec![CodexLocalAccessModelAlias {
        fork: true,
        ..existing[0].clone()
    }];
    let kept = merge_model_config_entries(
        &existing,
        incoming.clone(),
        "aliases",
        false,
        &mut preview,
        |alias| alias.alias.clone(),
        |_| Ok(()),
    );
    assert!(!kept[0].fork);
    assert_eq!(preview.conflicts, vec!["aliases/local"]);
    let replaced = merge_model_config_entries(
        &existing,
        incoming,
        "aliases",
        true,
        &mut preview,
        |alias| alias.alias.clone(),
        |_| Ok(()),
    );
    assert!(replaced[0].fork);
    assert_eq!(preview.updated, vec!["aliases/local"]);
    let route = CodexLocalAccessCustomRoutingRule {
        account_id: "missing-account".into(),
        priority: 0,
        weight: 1,
        is_backup: false,
        is_preferred: false,
    };
    let rules = merge_model_config_entries(
        &[],
        vec![route],
        "routing",
        true,
        &mut preview,
        |rule| rule.account_id.clone(),
        |_| Err("MODEL_CONFIG_ACCOUNT_MISSING".into()),
    );
    assert!(rules.is_empty());
    assert!(preview.errors[0].contains("MODEL_CONFIG_ACCOUNT_MISSING"));
}

struct ModelConfigTestEnvironment {
    dir: PathBuf,
    previous_data: Option<std::ffi::OsString>,
    previous_test_data: Option<std::ffi::OsString>,
}

impl ModelConfigTestEnvironment {
    fn new() -> Self {
        let dir = temp_dir();
        let data = dir.join("data");
        fs::create_dir_all(&data).unwrap();
        let previous_data = std::env::var_os("COCKPIT_TOOLS_DATA_DIR");
        let previous_test_data = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &data);
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data);
        Self {
            dir,
            previous_data,
            previous_test_data,
        }
    }
}

impl Drop for ModelConfigTestEnvironment {
    fn drop(&mut self) {
        for (name, previous) in [
            ("COCKPIT_TOOLS_DATA_DIR", &self.previous_data),
            ("COCKPIT_TOOLS_TEST_DATA_DIR", &self.previous_test_data),
        ] {
            if let Some(value) = previous {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn populated_profile(env: &ModelConfigTestEnvironment) -> PathBuf {
    let profile = env.dir.join("profile");
    fs::create_dir_all(&profile).unwrap();
    persist_experimental_model_definitions(&profile, vec![definition("gpt-5.6-sol")], None)
        .unwrap();
    fs::write(
        user_customized_model_catalog_marker_path(&profile),
        "customized\n",
    )
    .unwrap();
    let account = CodexAccount::new_api_key(
        "transfer-account".into(),
        "must-not-export@example.com".into(),
        "must-not-export-account-secret".into(),
        crate::models::codex::CodexApiProviderMode::OpenaiBuiltin,
        None,
        None,
        None,
        vec!["gpt-5.6-sol".into()],
    );
    save_account(&account).unwrap();
    let mut collection: CodexLocalAccessCollection = serde_json::from_value(serde_json::json!({
        "enabled": false, "port": 19333, "apiKey": "must-not-export-service-secret",
        "accountIds": ["transfer-account"], "createdAt": 1, "updatedAt": 1,
        "modelPricings": [{"modelId": "gpt-5.4", "inputUsdPerMillion": 1.25, "outputUsdPerMillion": 2.5},
            {"modelId": "gpt-image-2.5", "inputUsdPerMillion": 5.0, "outputUsdPerMillion": 7.0}],
        "modelAliases": [{"sourceModel": "gpt-5.6-sol", "alias": "local-sol", "fork": false}],
        "customRoutingRules": [{"accountId": "transfer-account", "priority": 1, "weight": 2}],
        "accountModelRules": [{"accountId": "transfer-account", "excludedModels": ["gpt-5.4*"]}]
    })).unwrap();
    let normalized = crate::modules::codex_local_access::normalize_model_config_service_sections(
        crate::modules::codex_local_access::model_config_service_sections(&collection),
        &collection,
    );
    crate::modules::codex_local_access::apply_model_config_service_sections(
        &mut collection,
        normalized,
    );
    let service_path =
        crate::modules::codex_local_access::model_config_service_file_path().unwrap();
    assert!(
        service_path.starts_with(env.dir.join("data")),
        "Tests must not write real user data"
    );
    fs::write(
        service_path,
        serde_json::to_string_pretty(&collection).unwrap(),
    )
    .unwrap();
    profile
}

#[test]
fn exported_config_round_trips_retired_image_prices_routes_and_excludes_credentials() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = ModelConfigTestEnvironment::new();
    let profile = populated_profile(&env);
    let exported = export_model_config(&profile).unwrap();
    assert!(!exported.contains("must-not-export"));
    assert!(!exported.contains("apiKey"));
    assert!(exported.contains("gpt-5.4"));
    assert!(exported.contains("gpt-image-2.5"));
    let preview = preview_model_config_import(&profile, &exported, "keep_existing").unwrap();
    assert!(preview.errors.is_empty(), "{:?}", preview.errors);
    assert!(preview.added.is_empty());
    assert!(preview.updated.is_empty());
    assert!(preview.conflicts.is_empty());
    let replaced = preview_model_config_import(&profile, &exported, "replace").unwrap();
    assert!(replaced.errors.is_empty());
    assert!(replaced.updated.is_empty());
    let (result, _, _) =
        import_model_config(&profile, &exported, "replace", &replaced.revision).unwrap();
    assert_eq!(result.committed, 0);
}

#[test]
fn real_import_previews_each_api_section_and_persists_without_enabling_service_or_catalog() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = ModelConfigTestEnvironment::new();
    let profile = populated_profile(&env);
    let mut source: serde_json::Value =
        serde_json::from_str(&export_model_config(&profile).unwrap()).unwrap();
    source["apiService"]["modelPricings"][0]["inputUsdPerMillion"] = serde_json::json!(9.0);
    source["apiService"]["modelAliases"][0]["fork"] = serde_json::json!(true);
    source["apiService"]["customRoutingRules"][0]["priority"] = serde_json::json!(8);
    source["apiService"]["accountModelRules"][0]["excludedModels"] =
        serde_json::json!(["gpt-5.6*"]);
    source["apiService"]["routingStrategy"] = serde_json::json!("custom");
    let kept = preview_model_config_import(&profile, &source.to_string(), "keep_existing").unwrap();
    assert!(kept.errors.is_empty());
    assert_eq!(kept.conflicts.len(), 5);
    let preview = preview_model_config_import(&profile, &source.to_string(), "replace").unwrap();
    assert!(preview.errors.is_empty());
    assert_eq!(preview.updated.len(), 5);
    let (saved, _, _) =
        import_model_config(&profile, &source.to_string(), "replace", &preview.revision).unwrap();
    assert_eq!(saved.committed, 5);
    let stored: CodexLocalAccessCollection = serde_json::from_str(
        &fs::read_to_string(
            crate::modules::codex_local_access::model_config_service_file_path().unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!stored.enabled);
    assert_eq!(stored.port, 19333);
    assert_eq!(stored.api_key, "must-not-export-service-secret");
    assert_eq!(stored.model_pricings[0].input_usd_per_million, 9.0);
    assert!(stored.model_aliases[0].fork);
    assert_eq!(stored.custom_routing_rules[0].priority, 8);
    assert_eq!(
        stored.routing_strategy,
        crate::models::codex_local_access::CodexLocalAccessRoutingStrategy::Custom
    );
    assert!(!experimental_model_policy_path(&profile).exists());
    assert!(!get_config_toml_path(&profile).exists());
}

#[test]
fn api_only_import_does_not_curate_models_or_change_unrelated_profile_defaults() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = ModelConfigTestEnvironment::new();
    let profile = populated_profile(&env);
    fs::remove_file(user_customized_model_catalog_marker_path(&profile)).unwrap();
    fs::write(experimental_model_policy_path(&profile), "enabled\n").unwrap();
    fs::write(
        get_config_toml_path(&profile),
        "model = \"external-user-default\"\nmodel_catalog_json = \"cockpit-model-catalog.json\"\n",
    )
    .unwrap();
    fs::write(experimental_model_catalog_path(&profile), "{\"models\":[]}").unwrap();
    let before_models = fs::read_to_string(experimental_model_config_path(&profile)).unwrap();
    let before_config = fs::read_to_string(get_config_toml_path(&profile)).unwrap();
    let before_catalog = fs::read_to_string(experimental_model_catalog_path(&profile)).unwrap();
    let snapshot = model_config_snapshot(&profile).unwrap();
    assert_eq!(snapshot.default_model_id, None);
    let mut document = CodexModelConfigDocument {
        schema: MODEL_CONFIG_SCHEMA.to_string(),
        version: 1,
        models: snapshot.models.clone(),
        default_model_id: snapshot.default_model_id.clone(),
        api_service: snapshot
            .service
            .as_ref()
            .map(crate::modules::codex_local_access::model_config_service_sections),
    };
    document.api_service.as_mut().unwrap().model_pricings[0].input_usd_per_million = 8.0;
    let (changes, _) = model_config_stage(&snapshot, &document).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].name, "api-service");
    commit_model_config_files(&profile, changes).unwrap();
    assert!(!user_customized_model_catalog_marker_path(&profile).exists());
    assert_eq!(
        fs::read_to_string(experimental_model_config_path(&profile)).unwrap(),
        before_models
    );
    assert_eq!(
        fs::read_to_string(get_config_toml_path(&profile)).unwrap(),
        before_config
    );
    assert_eq!(
        fs::read_to_string(experimental_model_catalog_path(&profile)).unwrap(),
        before_catalog
    );
}

#[test]
fn changing_model_metadata_preserves_default_toml_until_default_selection_changes() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = ModelConfigTestEnvironment::new();
    let profile = populated_profile(&env);
    fs::write(experimental_model_policy_path(&profile), "enabled\n").unwrap();
    fs::write(
        get_config_toml_path(&profile),
        "model = \"external-user-default\"\n",
    )
    .unwrap();
    let snapshot = model_config_snapshot(&profile).unwrap();
    let mut document = CodexModelConfigDocument {
        schema: MODEL_CONFIG_SCHEMA.to_string(),
        version: 1,
        models: snapshot.models.clone(),
        default_model_id: None,
        api_service: None,
    };
    document.models[0].display_name = "Changed name".into();
    let (changes, _) = model_config_stage(&snapshot, &document).unwrap();
    assert!(changes
        .iter()
        .any(|change| change.name == CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE));
    assert!(!changes
        .iter()
        .any(|change| change.name == CODEX_CONFIG_FILE_NAME));
    document.default_model_id = Some(document.models[0].model_id.clone());
    let (changes, _) = model_config_stage(&snapshot, &document).unwrap();
    assert!(changes
        .iter()
        .any(|change| change.name == CODEX_CONFIG_FILE_NAME));
}

#[test]
fn preview_rechecks_missing_account_and_revision_before_any_write() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = ModelConfigTestEnvironment::new();
    let profile = populated_profile(&env);
    let original = fs::read_to_string(experimental_model_config_path(&profile)).unwrap();
    let mut source: serde_json::Value =
        serde_json::from_str(&export_model_config(&profile).unwrap()).unwrap();
    source["models"][0]["display_name"] = serde_json::json!("New name");
    let preview = preview_model_config_import(&profile, &source.to_string(), "replace").unwrap();
    fs::write(
        get_config_toml_path(&profile),
        "model = \"external-new-model\"\n",
    )
    .unwrap();
    assert_eq!(
        import_model_config(&profile, &source.to_string(), "replace", &preview.revision)
            .unwrap_err(),
        "MODEL_CONFIG_STATE_CHANGED"
    );
    assert_eq!(
        fs::read_to_string(experimental_model_config_path(&profile)).unwrap(),
        original
    );
    source["apiService"]["customRoutingRules"][0]["accountId"] =
        serde_json::json!("missing-account");
    let preview = preview_model_config_import(&profile, &source.to_string(), "replace").unwrap();
    assert!(preview
        .errors
        .iter()
        .any(|error| error.contains("MODEL_CONFIG_ACCOUNT_MISSING")));
    assert_eq!(
        import_model_config(&profile, &source.to_string(), "replace", &preview.revision)
            .unwrap_err(),
        "MODEL_CONFIG_VALIDATION_FAILED"
    );
    assert_eq!(
        fs::read_to_string(experimental_model_config_path(&profile)).unwrap(),
        original
    );
}

#[test]
fn normal_preview_and_export_do_not_hold_or_require_a_profile_mutation_lease() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = ModelConfigTestEnvironment::new();
    let profile = populated_profile(&env);
    let _lease = try_acquire_profile_mutation_lease(&profile, "simulated-client-start").unwrap();
    let exported = export_model_config(&profile).unwrap();
    assert!(
        preview_model_config_import(&profile, &exported, "keep_existing")
            .unwrap()
            .errors
            .is_empty()
    );
}

#[test]
fn recovery_preserves_concurrent_edits_restores_other_files_and_allows_retry() {
    let dir = temp_dir();
    let files = vec![
        ModelConfigFileChange {
            name: CODEX_CONFIG_FILE_NAME.into(),
            before: Some("old-config".into()),
            after: Some("imported-config".into()),
        },
        ModelConfigFileChange {
            name: CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE.into(),
            before: Some("old-models".into()),
            after: Some("imported-models".into()),
        },
    ];
    fs::write(dir.join(CODEX_CONFIG_FILE_NAME), "newer-user-edit").unwrap();
    fs::write(
        dir.join(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
        "imported-models",
    )
    .unwrap();
    fs::write(
        dir.join(MODEL_CONFIG_JOURNAL),
        serde_json::to_string(&ModelConfigImportJournal {
            version: 1,
            committed: false,
            files,
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        recover_model_config_import(&dir).unwrap_err(),
        "MODEL_CONFIG_RECOVERY_CONFLICT"
    );
    assert_eq!(
        fs::read_to_string(dir.join(CODEX_CONFIG_FILE_NAME)).unwrap(),
        "newer-user-edit"
    );
    assert_eq!(
        fs::read_to_string(dir.join(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)).unwrap(),
        "old-models"
    );
    assert!(!dir.join(MODEL_CONFIG_JOURNAL).exists());
    recover_model_config_import(&dir).unwrap();
    assert!(fs::read_dir(&dir).unwrap().any(|file| file
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".cockpit-model-config-import.conflict-")));
    fs::remove_dir_all(dir).unwrap();
}
