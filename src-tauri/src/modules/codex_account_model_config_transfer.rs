// Versioned, credential-free model transfers. All I/O is called from blocking workers.
use crate::models::codex::{
    CodexModelConfigApiService, CodexModelConfigDocument, CodexModelConfigImportEntry,
    CodexModelConfigImportPreview,
};
use crate::models::codex_local_access::CodexLocalAccessCollection;

const MODEL_CONFIG_SCHEMA: &str = "cockpit-tools.codex-model-config";
const MODEL_CONFIG_MAX_BYTES: usize = 4 * 1024 * 1024;
const MODEL_CONFIG_MAX_ITEMS: usize = 1000;
const MODEL_CONFIG_JOURNAL: &str = ".cockpit-model-config-import.pending.json";
static MODEL_CONFIG_IMPORT_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelConfigFileChange {
    name: String,
    before: Option<String>,
    after: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelConfigImportJournal {
    version: u32,
    committed: bool,
    files: Vec<ModelConfigFileChange>,
}

struct ModelConfigSnapshot {
    profile_dir: Option<PathBuf>,
    files: Vec<ModelConfigFileChange>,
    models: Vec<CodexExperimentalModelDefinition>,
    default_model_id: Option<String>,
    service: Option<CodexLocalAccessCollection>,
    revision: String,
}

pub(crate) fn model_config_profile_dir(instance_id: &str) -> Result<PathBuf, String> {
    if instance_id == "__default__" {
        return crate::modules::codex_instance::get_default_codex_home();
    }
    let store = crate::modules::codex_instance::load_instance_store()?;
    store
        .instances
        .into_iter()
        .find(|instance| instance.id == instance_id)
        .map(|instance| PathBuf::from(instance.user_data_dir))
        .ok_or_else(|| "MODEL_CONFIG_INSTANCE_MISSING".to_string())
}

fn model_config_file_path(base_dir: &Path, name: &str) -> Result<PathBuf, String> {
    if name == "api-service" {
        return crate::modules::codex_local_access::model_config_service_file_path();
    }
    if [
        CODEX_CONFIG_FILE_NAME,
        CODEX_MANAGED_MODEL_CATALOG_FILE,
        CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE,
        CODEX_EXPERIMENTAL_MODEL_POLICY_FILE,
        CODEX_EXPERIMENTAL_MODEL_USER_CUSTOMIZED_FILE,
    ]
    .contains(&name)
    {
        return Ok(base_dir.join(name));
    }
    Err("MODEL_CONFIG_RECOVERY_INVALID".to_string())
}

fn model_config_read_file(path: &Path) -> Result<Option<String>, String> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > (MODEL_CONFIG_MAX_BYTES * 5) as u64 => {
            return Err("MODEL_CONFIG_TOO_LARGE".to_string())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("MODEL_CONFIG_READ_FAILED".to_string()),
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|_| "MODEL_CONFIG_READ_FAILED".to_string())
}

fn model_config_snapshot(base_dir: &Path) -> Result<ModelConfigSnapshot, String> {
    let mut files = Vec::new();
    for name in [
        CODEX_CONFIG_FILE_NAME,
        CODEX_MANAGED_MODEL_CATALOG_FILE,
        CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE,
        CODEX_EXPERIMENTAL_MODEL_POLICY_FILE,
        CODEX_EXPERIMENTAL_MODEL_USER_CUSTOMIZED_FILE,
        "api-service",
    ] {
        let before = model_config_read_file(&model_config_file_path(base_dir, name)?)?;
        files.push(ModelConfigFileChange {
            name: name.to_string(),
            after: before.clone(),
            before,
        });
    }
    let content = |name: &str| {
        files
            .iter()
            .find(|file| file.name == name)
            .and_then(|file| file.before.as_deref())
    };
    let saved = content(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)
        .map(|raw| {
            serde_json::from_str::<ExperimentalModelCatalogConfig>(raw)
                .map_err(|_| "MODEL_CONFIG_EXISTING_INVALID".to_string())
        })
        .transpose()?;
    let models = match saved.as_ref() {
        Some(saved) => normalize_experimental_model_definitions(saved.models.clone())?,
        None => default_experimental_model_definitions(base_dir),
    };
    let models = if content(CODEX_EXPERIMENTAL_MODEL_USER_CUSTOMIZED_FILE).is_some() {
        models
    } else {
        crate::modules::codex_local_access::overlay_rendered_pool_models_on_experimental_catalog(
            base_dir, models,
        )
    };
    let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
        content(CODEX_CONFIG_FILE_NAME).unwrap_or_default(),
    )
    .map_err(|_| "MODEL_CONFIG_EXISTING_INVALID".to_string())?;
    let default_model_id = saved
        .and_then(|saved| saved.default_model_id)
        .or_else(|| {
            doc.get("model")
                .and_then(|item| item.as_str())
                .map(str::to_string)
        })
        .filter(|id| {
            models
                .iter()
                .any(|model| model.model_id.eq_ignore_ascii_case(id))
        });
    let service = content("api-service")
        .map(|raw| {
            serde_json::from_str::<CodexLocalAccessCollection>(raw)
                .map_err(|_| "MODEL_CONFIG_EXISTING_INVALID".to_string())
        })
        .transpose()?;
    let revision = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&files).map_err(|_| "MODEL_CONFIG_SERIALIZE_FAILED".to_string())?
        )
    );
    Ok(ModelConfigSnapshot {
        profile_dir: Some(base_dir.to_path_buf()),
        files,
        models,
        default_model_id,
        service,
        revision,
    })
}

fn model_config_preview_entry(
    preview: &mut CodexModelConfigImportPreview,
    section: &str,
    id: &str,
    action: &str,
    error_code: Option<&str>,
) {
    let label = format!("{section}/{id}");
    match action {
        "added" => preview.added.push(label),
        "updated" => preview.updated.push(label),
        "conflict" => {
            preview.conflicts.push(label.clone());
            preview.skipped.push(label);
        }
        "error" => preview.errors.push(format!(
            "{label}:{}",
            error_code.unwrap_or("MODEL_CONFIG_INVALID")
        )),
        _ => preview.skipped.push(label),
    }
    preview.entries.push(CodexModelConfigImportEntry {
        section: section.to_string(),
        id: id.to_string(),
        action: action.to_string(),
        error_code: error_code.map(str::to_string),
    });
}

fn merge_model_config_entries<T: Clone + PartialEq>(
    existing: &[T],
    incoming: Vec<T>,
    section: &str,
    replace: bool,
    preview: &mut CodexModelConfigImportPreview,
    key: impl Fn(&T) -> String,
    validate: impl Fn(&T) -> Result<(), String>,
) -> Vec<T> {
    let mut result = existing.to_vec();
    let mut seen = HashSet::new();
    if incoming.len() > MODEL_CONFIG_MAX_ITEMS {
        model_config_preview_entry(
            preview,
            section,
            "*",
            "error",
            Some("MODEL_CONFIG_TOO_MANY_ITEMS"),
        );
        return result;
    }
    for item in incoming {
        let id = key(&item);
        if !seen.insert(id.trim().to_ascii_lowercase()) {
            model_config_preview_entry(
                preview,
                section,
                &id,
                "error",
                Some("MODEL_CONFIG_DUPLICATE"),
            );
            continue;
        }
        if let Err(code) = validate(&item) {
            model_config_preview_entry(preview, section, &id, "error", Some(&code));
            continue;
        }
        if let Some(index) = result
            .iter()
            .position(|current| key(current).eq_ignore_ascii_case(&id))
        {
            if result[index] == item {
                model_config_preview_entry(preview, section, &id, "skipped", None);
            } else if replace {
                result[index] = item;
                model_config_preview_entry(preview, section, &id, "updated", None);
            } else {
                model_config_preview_entry(preview, section, &id, "conflict", None);
            }
        } else {
            result.push(item);
            model_config_preview_entry(preview, section, &id, "added", None);
        }
    }
    result
}

fn model_config_valid_pattern(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-' | b'*')
        })
}

fn model_config_has_unknown_row_fields(raw: &serde_json::Value) -> bool {
    let sections: &[(&str, &[&str])] = &[
        (
            "modelPricings",
            &[
                "modelId",
                "standardLongPriceOverride",
                "longContextThresholdTokens",
                "inputUsdPerMillion",
                "outputUsdPerMillion",
                "cachedInputUsdPerMillion",
                "standardLongInputUsdPerMillion",
                "standardLongOutputUsdPerMillion",
                "standardLongCachedInputUsdPerMillion",
                "priorityInputUsdPerMillion",
                "priorityOutputUsdPerMillion",
                "priorityCachedInputUsdPerMillion",
                "priorityLongInputUsdPerMillion",
                "priorityLongOutputUsdPerMillion",
                "priorityLongCachedInputUsdPerMillion",
            ],
        ),
        ("modelAliases", &["sourceModel", "alias", "fork"]),
        ("accountModelRules", &["accountId", "excludedModels"]),
        (
            "customRoutingRules",
            &["accountId", "priority", "weight", "isBackup", "isPreferred"],
        ),
    ];
    sections.iter().any(|(name, keys)| {
        raw["apiService"][*name].as_array().is_some_and(|rows| {
            rows.iter().any(|row| {
                row.as_object()
                    .is_some_and(|object| object.keys().any(|key| !keys.contains(&key.as_str())))
            })
        })
    })
}

fn prepare_model_config_import(
    snapshot: &ModelConfigSnapshot,
    json: &str,
    strategy: &str,
) -> Result<(CodexModelConfigImportPreview, CodexModelConfigDocument), String> {
    if !["keep_existing", "replace"].contains(&strategy) {
        return Err("MODEL_CONFIG_STRATEGY_INVALID".to_string());
    }
    if json.len() > MODEL_CONFIG_MAX_BYTES {
        return Err("MODEL_CONFIG_TOO_LARGE".to_string());
    }
    let raw: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "MODEL_CONFIG_JSON_INVALID".to_string())?;
    let has_default = raw.get("defaultModelId").is_some();
    // Do not ignore credential/prompt/lifecycle fields in an allegedly safe transfer.
    if model_config_has_unknown_row_fields(&raw)
        || raw
            .get("models")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|models| {
                models.iter().any(|model| {
                    model.as_object().is_some_and(|object| {
                        object.keys().any(|key| {
                            ![
                                "model_id",
                                "display_name",
                                "reasoning_efforts",
                                "default_reasoning_effort",
                                "context_window",
                                "auto_compact_token_limit",
                            ]
                            .contains(&key.as_str())
                        })
                    })
                })
            })
    {
        return Err("MODEL_CONFIG_FIELDS_UNSUPPORTED".to_string());
    }
    let mut document: CodexModelConfigDocument =
        serde_json::from_value(raw).map_err(|_| "MODEL_CONFIG_FIELDS_UNSUPPORTED".to_string())?;
    if document.schema != MODEL_CONFIG_SCHEMA || document.version != 1 {
        return Err("MODEL_CONFIG_VERSION_UNSUPPORTED".to_string());
    }
    if document.models.is_empty() {
        return Err("EXPERIMENTAL_MODEL_CATALOG_MODELS_REQUIRED".to_string());
    }
    let replace = strategy == "replace";
    let mut preview = CodexModelConfigImportPreview {
        revision: snapshot.revision.clone(),
        ..Default::default()
    };
    // Normalize each row while keeping errors attached to that row.
    let mut incoming_models = Vec::new();
    for model in document.models.iter().take(MODEL_CONFIG_MAX_ITEMS + 1) {
        match normalize_experimental_model_definitions(vec![model.clone()]) {
            Ok(mut models) => incoming_models.push(models.remove(0)),
            Err(code) => model_config_preview_entry(
                &mut preview,
                "models",
                &model.model_id,
                "error",
                Some(&code),
            ),
        }
    }
    document.models = merge_model_config_entries(
        &snapshot.models,
        incoming_models,
        "models",
        replace,
        &mut preview,
        |model| model.model_id.clone(),
        |_| Ok(()),
    );
    if let Some(dir) = snapshot.profile_dir.as_ref() {
        let effective = model_reasoning_efforts_for_profile(dir, &document.models);
        for model in &document.models {
            if model
                .default_reasoning_effort
                .as_ref()
                .is_some_and(|default| {
                    !effective
                        .get(&model.model_id)
                        .is_some_and(|efforts| efforts.contains(default))
                })
            {
                model_config_preview_entry(
                    &mut preview,
                    "models",
                    &model.model_id,
                    "error",
                    Some("EXPERIMENTAL_MODEL_CATALOG_DEFAULT_REASONING_INVALID"),
                );
            }
        }
    }
    let model_ids: HashSet<String> = document
        .models
        .iter()
        .map(|model| model.model_id.to_ascii_lowercase())
        .collect();
    if has_default {
        if document
            .default_model_id
            .as_ref()
            .is_some_and(|id| !model_ids.contains(&id.to_ascii_lowercase()))
        {
            model_config_preview_entry(
                &mut preview,
                "defaultModel",
                "default",
                "error",
                Some("MODEL_CONFIG_MODEL_MISSING"),
            );
        } else if document.default_model_id == snapshot.default_model_id {
            model_config_preview_entry(&mut preview, "defaultModel", "default", "skipped", None);
        } else if snapshot.default_model_id.is_none() || replace {
            model_config_preview_entry(
                &mut preview,
                "defaultModel",
                "default",
                if snapshot.default_model_id.is_none() {
                    "added"
                } else {
                    "updated"
                },
                None,
            );
        } else {
            model_config_preview_entry(&mut preview, "defaultModel", "default", "conflict", None);
            document.default_model_id = snapshot.default_model_id.clone();
        }
    } else {
        document.default_model_id = snapshot.default_model_id.clone();
    }
    if let Some(incoming) = document.api_service.take() {
        if let Some(service) = snapshot.service.as_ref() {
            let existing =
                crate::modules::codex_local_access::model_config_service_sections(service);
            document.api_service = Some(prepare_model_config_api_import(
                service,
                existing,
                incoming,
                &model_ids,
                replace,
                &mut preview,
            ));
        } else if incoming != CodexModelConfigApiService::default() {
            model_config_preview_entry(
                &mut preview,
                "apiService",
                "*",
                "error",
                Some("MODEL_CONFIG_SERVICE_MISSING"),
            );
        }
    }
    Ok((preview, document))
}

fn prepare_model_config_api_import(
    service: &CodexLocalAccessCollection,
    existing: CodexModelConfigApiService,
    incoming: CodexModelConfigApiService,
    model_ids: &HashSet<String>,
    replace: bool,
    preview: &mut CodexModelConfigImportPreview,
) -> CodexModelConfigApiService {
    let mut known_models = model_ids.clone();
    let routing_strategy = match incoming.routing_strategy {
        Some(strategy) if Some(strategy) == existing.routing_strategy => {
            model_config_preview_entry(preview, "routing", "strategy", "skipped", None);
            Some(strategy)
        }
        Some(strategy) if replace => {
            model_config_preview_entry(preview, "routing", "strategy", "updated", None);
            Some(strategy)
        }
        Some(_) => {
            model_config_preview_entry(preview, "routing", "strategy", "conflict", None);
            existing.routing_strategy
        }
        None => existing.routing_strategy,
    };
    // Account IDs are references only. Read local data to validate actual targets; never export it.
    let accounts = service
        .account_ids
        .iter()
        .filter_map(|id| crate::modules::codex_account::load_account(id))
        .collect::<Vec<_>>();
    let valid_accounts: HashSet<_> = accounts.iter().map(|account| account.id.as_str()).collect();
    for account in &accounts {
        known_models.extend(
            account
                .api_model_catalog
                .iter()
                .map(|id| id.to_ascii_lowercase()),
        );
        known_models.extend(
            account
                .api_model_mappings
                .iter()
                .map(|mapping| mapping.client_model.to_ascii_lowercase()),
        );
    }
    known_models.extend(
        existing
            .model_aliases
            .iter()
            .map(|alias| alias.alias.to_ascii_lowercase()),
    );
    known_models.extend(
        existing
            .model_aliases
            .iter()
            .map(|alias| alias.source_model.to_ascii_lowercase()),
    );
    known_models.extend(
        existing
            .model_pricings
            .iter()
            .map(|price| price.model_id.to_ascii_lowercase()),
    );
    let model_exists = |id: &str| {
        is_valid_model_catalog_id(id.trim())
            && known_models.contains(&id.trim().to_ascii_lowercase())
    };
    let model_pricings = merge_model_config_entries(
        &existing.model_pricings,
        incoming.model_pricings,
        "prices",
        replace,
        preview,
        |price| price.model_id.clone(),
        |price| {
            if !model_exists(&price.model_id) {
                return Err("MODEL_CONFIG_MODEL_MISSING".to_string());
            }
            let rates = [
                Some(price.input_usd_per_million),
                Some(price.output_usd_per_million),
                price.cached_input_usd_per_million,
                price.standard_long_input_usd_per_million,
                price.standard_long_output_usd_per_million,
                price.standard_long_cached_input_usd_per_million,
                price.priority_input_usd_per_million,
                price.priority_output_usd_per_million,
                price.priority_cached_input_usd_per_million,
                price.priority_long_input_usd_per_million,
                price.priority_long_output_usd_per_million,
                price.priority_long_cached_input_usd_per_million,
            ];
            if rates
                .into_iter()
                .flatten()
                .any(|value| !value.is_finite() || value < 0.0)
                || price.long_context_threshold_tokens == Some(0)
            {
                return Err("MODEL_CONFIG_PRICE_INVALID".to_string());
            }
            Ok(())
        },
    );
    let model_aliases = merge_model_config_entries(
        &existing.model_aliases,
        incoming.model_aliases,
        "aliases",
        replace,
        preview,
        |alias| alias.alias.clone(),
        |alias| {
            if !model_exists(&alias.source_model) {
                return Err("MODEL_CONFIG_MODEL_MISSING".to_string());
            }
            if !is_valid_model_catalog_id(alias.alias.trim())
                || alias.alias.eq_ignore_ascii_case(&alias.source_model)
            {
                return Err("MODEL_CONFIG_ALIAS_INVALID".to_string());
            }
            Ok(())
        },
    );
    let account_model_rules = merge_model_config_entries(
        &existing.account_model_rules,
        incoming.account_model_rules,
        "accountRules",
        replace,
        preview,
        |rule| rule.account_id.clone(),
        |rule| {
            if !valid_accounts.contains(rule.account_id.as_str()) {
                return Err("MODEL_CONFIG_ACCOUNT_MISSING".to_string());
            }
            if rule.excluded_models.is_empty()
                || rule.excluded_models.len() > MODEL_CONFIG_MAX_ITEMS
                || rule
                    .excluded_models
                    .iter()
                    .any(|id| !model_config_valid_pattern(id))
            {
                return Err("MODEL_CONFIG_RULE_INVALID".to_string());
            }
            Ok(())
        },
    );
    let custom_routing_rules = merge_model_config_entries(
        &existing.custom_routing_rules,
        incoming.custom_routing_rules,
        "routing",
        replace,
        preview,
        |rule| rule.account_id.clone(),
        |rule| {
            if !valid_accounts.contains(rule.account_id.as_str()) {
                return Err("MODEL_CONFIG_ACCOUNT_MISSING".to_string());
            }
            if !(0..=100).contains(&rule.priority)
                || !(1..=100).contains(&rule.weight)
                || rule.is_backup && rule.is_preferred
            {
                return Err("MODEL_CONFIG_RULE_INVALID".to_string());
            }
            Ok(())
        },
    );
    let excluded_models = merge_model_config_entries(
        &existing.excluded_models,
        incoming.excluded_models,
        "exclusions",
        replace,
        preview,
        Clone::clone,
        |id| {
            if model_config_valid_pattern(id) {
                Ok(())
            } else {
                Err("MODEL_CONFIG_RULE_INVALID".to_string())
            }
        },
    );
    crate::modules::codex_local_access::normalize_model_config_service_sections(
        CodexModelConfigApiService {
            routing_strategy,
            model_pricings,
            model_aliases,
            account_model_rules,
            custom_routing_rules,
            excluded_models,
        },
        service,
    )
}

fn model_config_stage(
    snapshot: &ModelConfigSnapshot,
    document: &CodexModelConfigDocument,
) -> Result<
    (
        Vec<ModelConfigFileChange>,
        Option<CodexLocalAccessCollection>,
    ),
    String,
> {
    let mut files = snapshot.files.clone();
    let set_file = |files: &mut Vec<ModelConfigFileChange>, name: &str, content: String| {
        if let Some(file) = files.iter_mut().find(|file| file.name == name) {
            file.after = Some(content);
        }
    };
    let models_changed = document.models != snapshot.models;
    let default_model_changed = document.default_model_id != snapshot.default_model_id;
    if models_changed || default_model_changed {
        let mut config = files
            .iter()
            .find(|file| file.name == CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)
            .and_then(|file| file.before.as_deref())
            .map(|raw| {
                serde_json::from_str::<ExperimentalModelCatalogConfig>(raw)
                    .map_err(|_| "MODEL_CONFIG_EXISTING_INVALID".to_string())
            })
            .transpose()?
            .unwrap_or(ExperimentalModelCatalogConfig {
                version: EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION,
                models: Vec::new(),
                default_model_id: None,
                migrations: Vec::new(),
            });
        config.models = document.models.clone();
        config.default_model_id = document.default_model_id.clone();
        config.version = EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION;
        for migration in [
            GPT_6_ASTRA_MODEL_CATALOG_MIGRATION_ID,
            GPT_6_SOL_LUNA_MODEL_CATALOG_MIGRATION_ID,
            GPT_6_1_SOL_MODEL_CATALOG_MIGRATION_ID,
            GPT_6_ASTRA_DEFAULT_REPAIR_MIGRATION_ID,
            BUILTIN_MODEL_DISPLAY_NAME_MIGRATION_ID,
        ] {
            if !config.migrations.iter().any(|value| value == migration) {
                config.migrations.push(migration.to_string());
            }
        }
        set_file(
            &mut files,
            CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE,
            serde_json::to_string_pretty(&config)
                .map_err(|_| "MODEL_CONFIG_SERIALIZE_FAILED".to_string())?,
        );
        set_file(
            &mut files,
            CODEX_EXPERIMENTAL_MODEL_USER_CUSTOMIZED_FILE,
            "customized\n".to_string(),
        );
        let config_file = files
            .iter()
            .find(|file| file.name == CODEX_CONFIG_FILE_NAME)
            .unwrap();
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            config_file.before.as_deref().unwrap_or_default(),
        )
        .map_err(|_| "MODEL_CONFIG_EXISTING_INVALID".to_string())?;
        // Respect opt-out and routing-only use: do not add a catalog reference or enable switch.
        let catalog_active = files
            .iter()
            .any(|file| file.name == CODEX_EXPERIMENTAL_MODEL_POLICY_FILE && file.before.is_some());
        if catalog_active && default_model_changed {
            if let Some(id) = document.default_model_id.as_ref() {
                doc["model"] = value(id.as_str());
            } else {
                doc.remove("model");
            }
            set_file(
                &mut files,
                CODEX_CONFIG_FILE_NAME,
                crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc),
            );
        }
        if catalog_active && models_changed {
            let definitions = document
                .models
                .iter()
                .map(|model| {
                    (
                        model.model_id.clone(),
                        model.display_name.clone(),
                        model.reasoning_efforts.clone(),
                    )
                })
                .collect::<Vec<_>>();
            let mut catalog = crate::modules::codex_protocol::build_codex_client_models_response_with_model_definitions_and_reasoning(&definitions);
            crate::modules::codex_protocol::ensure_codex_reserve_fallback(&mut catalog);
            apply_model_context_config_to_catalog(&mut catalog, &document.models);
            apply_model_reasoning_config_to_catalog(&mut catalog, &document.models)?;
            set_file(
                &mut files,
                CODEX_MANAGED_MODEL_CATALOG_FILE,
                serde_json::to_string_pretty(&catalog)
                    .map_err(|_| "MODEL_CONFIG_SERIALIZE_FAILED".to_string())?,
            );
        }
    }
    let mut changed_service = None;
    if let (Some(service), Some(api_config)) =
        (snapshot.service.as_ref(), document.api_service.as_ref())
    {
        if crate::modules::codex_local_access::model_config_service_sections(service) != *api_config
        {
            let mut service = service.clone();
            crate::modules::codex_local_access::apply_model_config_service_sections(
                &mut service,
                api_config.clone(),
            );
            set_file(
                &mut files,
                "api-service",
                serde_json::to_string_pretty(&service)
                    .map_err(|_| "MODEL_CONFIG_SERIALIZE_FAILED".to_string())?,
            );
            changed_service = Some(service);
        }
    }
    files.retain(|file| file.before != file.after);
    Ok((files, changed_service))
}

fn rollback_model_config_files(
    base_dir: &Path,
    files: &[ModelConfigFileChange],
) -> Result<(), String> {
    let mut conflict = false;
    for file in files.iter().rev() {
        let path = model_config_file_path(base_dir, &file.name)?;
        let current = model_config_read_file(&path)?;
        if current == file.before {
            continue;
        }
        if current != file.after {
            conflict = true;
            continue;
        }
        if !crate::modules::atomic_write::replace_secret_string_if_matches(
            &path,
            file.after.as_deref(),
            file.before.as_deref(),
        )? {
            conflict = true;
        }
    }
    if conflict {
        Err("MODEL_CONFIG_RECOVERY_CONFLICT".to_string())
    } else {
        Ok(())
    }
}

/// Recovery is bounded to the profile's known files. It never trusts paths from the journal.
fn recover_model_config_import_if_pending(base_dir: &Path) -> Result<(), String> {
    if !base_dir.join(MODEL_CONFIG_JOURNAL).is_file() {
        return Ok(());
    }
    let _lease = try_acquire_profile_mutation_lease(base_dir, "model-config-recovery")?;
    recover_model_config_import(base_dir)
}

pub(crate) fn recover_model_config_import(base_dir: &Path) -> Result<(), String> {
    let path = base_dir.join(MODEL_CONFIG_JOURNAL);
    let Some(content) = model_config_read_file(&path)? else {
        return Ok(());
    };
    let journal: ModelConfigImportJournal =
        serde_json::from_str(&content).map_err(|_| "MODEL_CONFIG_RECOVERY_INVALID".to_string())?;
    if journal.version != 1 || journal.files.len() > 6 {
        return Err("MODEL_CONFIG_RECOVERY_INVALID".to_string());
    }
    for file in &journal.files {
        model_config_file_path(base_dir, &file.name)?;
    }
    if !journal.committed {
        if let Err(error) = rollback_model_config_files(base_dir, &journal.files) {
            if error == "MODEL_CONFIG_RECOVERY_CONFLICT" {
                // Preserve evidence and newer data without permanently blocking retries.
                fs::rename(
                    &path,
                    base_dir.join(format!(
                        ".cockpit-model-config-import.conflict-{}.json",
                        uuid::Uuid::new_v4()
                    )),
                )
                .map_err(|_| "MODEL_CONFIG_WRITE_FAILED".to_string())?;
            }
            return Err(error);
        }
    }
    if let Err(error) = crate::modules::atomic_write::remove_file_locked(&path) {
        logger::log_warn(&format!(
            "[Codex模型导入] 已提交，恢复记录将在下次访问时清理: {}",
            error
        ));
    }
    Ok(())
}

/// Startup maintenance may call this in spawn_blocking. It never blocks another mutation.
pub(crate) fn recover_pending_model_config_imports_for_known_profiles() {
    let Ok(_lock) = MODEL_CONFIG_IMPORT_LOCK.try_lock() else {
        return;
    };
    let mut dirs = Vec::new();
    if let Ok(dir) = crate::modules::codex_instance::get_default_codex_home() {
        dirs.push(dir);
    }
    if let Ok(store) = crate::modules::codex_instance::load_instance_store() {
        dirs.extend(
            store
                .instances
                .into_iter()
                .map(|instance| PathBuf::from(instance.user_data_dir)),
        );
    }
    dirs.sort();
    dirs.dedup();
    for dir in dirs {
        if !dir.join(MODEL_CONFIG_JOURNAL).is_file() {
            continue;
        }
        let Ok(_lease) = try_acquire_profile_mutation_lease(&dir, "model-config-recovery") else {
            continue;
        };
        if let Err(error) = recover_model_config_import(&dir) {
            logger::log_warn(&format!(
                "[Codex模型导入] 后台恢复失败，保留可用数据: profile={}, error={}",
                dir.display(),
                error
            ));
        }
        std::thread::yield_now();
    }
}

fn commit_model_config_files(
    base_dir: &Path,
    files: Vec<ModelConfigFileChange>,
) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let path = base_dir.join(MODEL_CONFIG_JOURNAL);
    let mut journal = ModelConfigImportJournal {
        version: 1,
        committed: false,
        files,
    };
    let serialize = |journal: &ModelConfigImportJournal| {
        serde_json::to_string(journal).map_err(|_| "MODEL_CONFIG_SERIALIZE_FAILED".to_string())
    };
    crate::modules::atomic_write::write_secret_string_atomic(&path, &serialize(&journal)?)?;
    let mut written = 0;
    let result = (|| {
        for file in &journal.files {
            if !crate::modules::atomic_write::replace_secret_string_if_matches(
                &model_config_file_path(base_dir, &file.name)?,
                file.before.as_deref(),
                file.after.as_deref(),
            )? {
                return Err("MODEL_CONFIG_STATE_CHANGED".to_string());
            }
            written += 1;
        }
        journal.committed = true;
        crate::modules::atomic_write::write_secret_string_atomic(&path, &serialize(&journal)?)
    })();
    if let Err(error) = result {
        rollback_model_config_files(base_dir, &journal.files[..written])?;
        crate::modules::atomic_write::remove_file_locked(&path)?;
        return Err(error);
    }
    if let Err(error) = crate::modules::atomic_write::remove_file_locked(&path) {
        logger::log_warn(&format!(
            "[Codex模型导入] 已提交，恢复记录将在下次访问时清理: {}",
            error
        ));
    }
    Ok(())
}

pub(crate) fn preview_model_config_import(
    base_dir: &Path,
    json: &str,
    strategy: &str,
) -> Result<CodexModelConfigImportPreview, String> {
    let _lock = MODEL_CONFIG_IMPORT_LOCK
        .try_lock()
        .map_err(|_| "MODEL_CONFIG_BUSY".to_string())?;
    recover_model_config_import_if_pending(base_dir)?;
    let snapshot = model_config_snapshot(base_dir)?;
    prepare_model_config_import(&snapshot, json, strategy).map(|(preview, _)| preview)
}

pub(crate) fn import_model_config(
    base_dir: &Path,
    json: &str,
    strategy: &str,
    expected_revision: &str,
) -> Result<
    (
        CodexModelConfigImportPreview,
        Option<CodexModelConfigApiService>,
        Option<CodexLocalAccessCollection>,
    ),
    String,
> {
    let _lock = MODEL_CONFIG_IMPORT_LOCK
        .try_lock()
        .map_err(|_| "MODEL_CONFIG_BUSY".to_string())?;
    recover_model_config_import_if_pending(base_dir)?;
    let snapshot = model_config_snapshot(base_dir)?;
    if snapshot.revision != expected_revision {
        return Err("MODEL_CONFIG_STATE_CHANGED".to_string());
    }
    let (mut preview, document) = prepare_model_config_import(&snapshot, json, strategy)?;
    if !preview.errors.is_empty() {
        return Err("MODEL_CONFIG_VALIDATION_FAILED".to_string());
    }
    let before_service = snapshot
        .service
        .as_ref()
        .map(crate::modules::codex_local_access::model_config_service_sections);
    if preview.added.is_empty() && preview.updated.is_empty() {
        return Ok((preview, before_service, None));
    }
    let (files, service) = model_config_stage(&snapshot, &document)?;
    let _lease = try_acquire_profile_mutation_lease(base_dir, "model-config-import")?;
    // Slow parsing, catalog rendering and account reads finish before the mutation lease.
    // Recheck the compact file snapshot immediately before the CAS transaction.
    for file in &snapshot.files {
        if model_config_read_file(&model_config_file_path(base_dir, &file.name)?)? != file.before {
            return Err("MODEL_CONFIG_STATE_CHANGED".to_string());
        }
    }
    commit_model_config_files(base_dir, files)?;
    preview.committed = preview.added.len() + preview.updated.len();
    preview.models = document.models;
    preview.default_model_id = document.default_model_id;
    Ok((preview, before_service, service))
}

pub(crate) fn export_model_config(base_dir: &Path) -> Result<String, String> {
    let _lock = MODEL_CONFIG_IMPORT_LOCK
        .try_lock()
        .map_err(|_| "MODEL_CONFIG_BUSY".to_string())?;
    recover_model_config_import_if_pending(base_dir)?;
    let snapshot = model_config_snapshot(base_dir)?;
    serde_json::to_string_pretty(&CodexModelConfigDocument {
        schema: MODEL_CONFIG_SCHEMA.to_string(),
        version: 1,
        models: snapshot.models,
        default_model_id: snapshot.default_model_id,
        api_service: snapshot
            .service
            .as_ref()
            .map(crate::modules::codex_local_access::model_config_service_sections),
    })
    .map_err(|_| "MODEL_CONFIG_SERIALIZE_FAILED".to_string())
}

#[cfg(test)]
#[path = "codex_account_model_config_transfer_tests.rs"]
mod model_config_transfer_tests;
