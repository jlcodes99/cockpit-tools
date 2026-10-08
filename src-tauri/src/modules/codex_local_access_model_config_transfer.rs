// Model-config transfer changes only these fields; credentials and service lifecycle stay local.
pub(crate) fn model_config_service_file_path() -> Result<PathBuf, String> {
    local_access_file_path()
}

pub(crate) fn model_config_service_sections(
    collection: &CodexLocalAccessCollection,
) -> crate::models::codex::CodexModelConfigApiService {
    crate::models::codex::CodexModelConfigApiService {
        routing_strategy: Some(collection.routing_strategy),
        model_pricings: collection.model_pricings.clone(),
        model_aliases: collection.model_aliases.clone(),
        account_model_rules: collection.account_model_rules.clone(),
        custom_routing_rules: collection.custom_routing_rules.clone(),
        excluded_models: collection.excluded_models.clone(),
    }
}

pub(crate) fn normalize_model_config_service_sections(
    config: crate::models::codex::CodexModelConfigApiService,
    collection: &CodexLocalAccessCollection,
) -> crate::models::codex::CodexModelConfigApiService {
    crate::models::codex::CodexModelConfigApiService {
        routing_strategy: config.routing_strategy,
        model_pricings: normalize_model_pricings(config.model_pricings),
        model_aliases: normalize_model_aliases(config.model_aliases),
        account_model_rules: normalize_account_model_rules(
            config.account_model_rules,
            &collection.account_ids,
        ),
        custom_routing_rules: normalize_custom_routing_rules(
            config.custom_routing_rules,
            &collection.account_ids,
        ),
        excluded_models: normalize_model_rule_list(config.excluded_models),
    }
}

pub(crate) fn apply_model_config_service_sections(
    collection: &mut CodexLocalAccessCollection,
    config: crate::models::codex::CodexModelConfigApiService,
) {
    if let Some(strategy) = config.routing_strategy {
        collection.routing_strategy = strategy;
    }
    if collection.model_pricings != config.model_pricings {
        collection.model_pricing_version = collection
            .model_pricing_version
            .max(DEFAULT_MODEL_PRICING_VERSION)
            .saturating_add(1);
    }
    collection.model_pricings = config.model_pricings;
    collection.model_aliases = config.model_aliases;
    collection.account_model_rules = config.account_model_rules;
    collection.custom_routing_rules = config.custom_routing_rules;
    collection.excluded_models = config.excluded_models;
    collection.updated_at = now_ms();
}

/// Publish only the imported fields, preserving concurrent changes to unrelated settings.
/// This does not start or take over profiles.
pub(crate) async fn publish_model_config_service_import(
    app: &AppHandle,
    before: Option<crate::models::codex::CodexModelConfigApiService>,
    after: Option<CodexLocalAccessCollection>,
) {
    let Some(after) = after else {
        return;
    };
    let mut runtime = gateway_runtime().lock().await;
    let Some(current) = runtime.collection.as_mut() else {
        return;
    };
    let current_sections = model_config_service_sections(current);
    let after_sections = model_config_service_sections(&after);
    let changed_prices = before
        .as_ref()
        .map(|before| changed_model_pricing_ids(&before.model_pricings, &after.model_pricings))
        .unwrap_or_default();
    if before
        .as_ref()
        .is_some_and(|before| *before == current_sections)
        || current_sections == after_sections
    {
        apply_model_config_service_sections(current, after_sections);
        current.model_pricing_version = after.model_pricing_version;
        // Avoid sync_runtime_collection: importing settings must not change lifecycle state.
        if !changed_prices.is_empty() && model_config_pricing_version_is_current(current, &after) {
            // queue_model_pricing_reprice only takes its short metadata lock; it performs no
            // file/DB/network I/O before returning. Keep publish + enqueue ordered so a
            // concurrent price save cannot queue its newer version before this older job.
            queue_model_pricing_reprice(app.clone(), after, changed_prices).await;
        }
    }
}

fn model_config_pricing_version_is_current(
    current: &CodexLocalAccessCollection,
    imported: &CodexLocalAccessCollection,
) -> bool {
    current.model_pricing_version == imported.model_pricing_version
        && current.model_pricings == imported.model_pricings
}

fn model_config_active_reload_allowed(
    running: bool,
    enabled: bool,
    stop_requests: usize,
    expected_generation: u64,
    current_generation: u64,
) -> bool {
    running && enabled && stop_requests == 0 && expected_generation == current_generation
}

/// Only a gateway that is still running in the same lifecycle generation may reload.
/// Stop requests invalidate this task before it can prepare or start any process.
pub(crate) fn reload_active_gateway_after_model_config_import(
    instance_id: String,
    revision: String,
) {
    let expected_generation = current_gateway_lifecycle_generation();
    tauri::async_runtime::spawn(async move {
        let result = {
            let _lifecycle_guard = gateway_lifecycle_lock().lock().await;
            let allowed = {
                let runtime = gateway_runtime().lock().await;
                model_config_active_reload_allowed(
                    runtime.running,
                    runtime
                        .collection
                        .as_ref()
                        .is_some_and(|collection| collection.enabled),
                    GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst),
                    expected_generation,
                    current_gateway_lifecycle_generation(),
                )
            };
            if !allowed {
                return;
            }
            ensure_gateway_matches_runtime_locked().await
        };
        if let Err(error) = result {
            let mut runtime = gateway_runtime().lock().await;
            runtime.last_error = Some(error.clone());
            logger::log_codex_api_warn(&format!(
                "[Codex模型导入] 运行中网关重载失败，配置已保存: {}",
                error
            ));
            drop(runtime);
            if let Some(app) = crate::get_app_handle() {
                let _ = app.emit(
                    "codex-model-config-apply-error",
                    json!({
                        "instanceId": instance_id, "revision": revision, "error": error,
                    }),
                );
            }
        }
        emit_local_access_state_updated();
    });
}

#[cfg(test)]
mod model_config_active_reload_tests {
    #[test]
    fn import_reprice_cannot_replace_a_newer_pricing_version() {
        let mut current: super::CodexLocalAccessCollection = serde_json::from_value(serde_json::json!({
            "enabled": false, "port": 19333, "apiKey": "test", "accountIds": [], "createdAt": 1, "updatedAt": 1,
        })).unwrap();
        let imported = current.clone();
        assert!(super::model_config_pricing_version_is_current(
            &current, &imported
        ));
        current.model_pricing_version += 1;
        assert!(!super::model_config_pricing_version_is_current(
            &current, &imported
        ));
    }
    #[test]
    fn imported_settings_cannot_start_a_stopped_disabled_or_replaced_gateway() {
        assert!(super::model_config_active_reload_allowed(
            true, true, 0, 4, 4
        ));
        assert!(!super::model_config_active_reload_allowed(
            false, true, 0, 4, 4
        ));
        assert!(!super::model_config_active_reload_allowed(
            true, false, 0, 4, 4
        ));
        assert!(!super::model_config_active_reload_allowed(
            true, true, 1, 4, 4
        ));
        assert!(!super::model_config_active_reload_allowed(
            true, true, 0, 4, 5
        ));
    }
}
