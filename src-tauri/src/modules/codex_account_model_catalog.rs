// Codex 账号模块：Model catalog, quick config and provider catalog persistence。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
/// 获取 Codex 数据目录
pub fn get_codex_home() -> PathBuf {
    if let Some(from_env) = resolve_codex_home_from_env() {
        return from_env;
    }
    dirs::home_dir().expect("无法获取用户主目录").join(".codex")
}

fn resolve_codex_home_from_env() -> Option<PathBuf> {
    let raw = std::env::var("CODEX_HOME").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 兼容用户使用 setx / shell 时可能包裹的引号
    let unquoted = trimmed.trim_matches('"').trim_matches('\'').trim();
    if unquoted.is_empty() {
        return None;
    }

    Some(PathBuf::from(unquoted))
}

/// 获取官方 auth.json 路径
pub fn get_auth_json_path() -> PathBuf {
    get_codex_home().join("auth.json")
}

fn get_config_toml_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_CONFIG_FILE_NAME)
}

fn read_top_level_int_from_doc(doc: &Document, key: &str) -> Option<i64> {
    doc.get(key).and_then(|item| item.as_integer())
}

#[derive(Debug, Default)]
struct ExperimentalModelCatalogState {
    enabled: bool,
    available: bool,
    unavailable_reason: Option<String>,
    conflict: Option<String>,
}

fn catalog_ref_targets_profile_file(value: &str, base_dir: &Path, file_name: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.eq_ignore_ascii_case(file_name) {
        return true;
    }

    let configured = expand_user_path(trimmed);
    if !catalog_file_name(&configured).eq_ignore_ascii_case(file_name) {
        return false;
    }
    let configured = if configured.is_absolute() {
        configured
    } else {
        base_dir.join(configured)
    };
    absolute_path_for_config(&configured)
        .eq_ignore_ascii_case(&absolute_path_for_config(&base_dir.join(file_name)))
}

fn catalog_ref_targets_cockpit_managed_file(value: &str, base_dir: &Path) -> bool {
    [
        CODEX_MANAGED_MODEL_CATALOG_FILE,
        CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE,
        CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE,
    ]
    .iter()
    .any(|file_name| catalog_ref_targets_profile_file(value, base_dir, file_name))
}

fn experimental_model_catalog_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_MANAGED_MODEL_CATALOG_FILE)
}

pub(crate) fn cleanup_legacy_managed_model_catalogs(base_dir: &Path) {
    for file_name in [
        CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE,
        CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE,
    ] {
        let path = base_dir.join(file_name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => logger::log_warn(&format!(
                "[Codex模型目录] 清理旧受管目录失败: path={}, error={}",
                path.display(),
                error
            )),
        }
    }
}

fn migrate_legacy_managed_catalog_reference(
    base_dir: &Path,
    doc: &mut Document,
) -> Result<bool, String> {
    let Some(reference) = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
    else {
        return Ok(false);
    };
    let legacy_file = if reference.eq_ignore_ascii_case(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE) {
        Some(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
    } else if reference.eq_ignore_ascii_case(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE) {
        Some(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
    } else {
        None
    };
    let Some(legacy_file) = legacy_file else {
        return Ok(false);
    };
    let managed_path = base_dir.join(CODEX_MANAGED_MODEL_CATALOG_FILE);
    if !managed_path.is_file() {
        let legacy_path = base_dir.join(legacy_file);
        if legacy_path.is_file() {
            let content = fs::read_to_string(&legacy_path).map_err(|error| {
                format!(
                    "读取旧 Codex 模型目录失败: path={}, error={}",
                    legacy_path.display(),
                    error
                )
            })?;
            write_string_atomic(&managed_path, &content).map_err(|error| {
                format!(
                    "迁移 Codex 模型目录失败: path={}, error={}",
                    managed_path.display(),
                    error
                )
            })?;
        } else {
            return Ok(false);
        }
    }
    doc[CODEX_CONFIG_MODEL_CATALOG_JSON_KEY] = value(CODEX_MANAGED_MODEL_CATALOG_FILE);
    Ok(true)
}

fn experimental_model_policy_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
}

fn experimental_model_config_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)
}

fn experimental_model_previous_catalog_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_EXPERIMENTAL_MODEL_PREVIOUS_CATALOG_FILE)
}

fn resolve_catalog_reference(value: &str, base_dir: &Path) -> PathBuf {
    let configured = expand_user_path(value.trim());
    if configured.is_absolute() {
        configured
    } else {
        base_dir.join(configured)
    }
}

fn read_previous_experimental_catalog_reference(base_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(experimental_model_previous_catalog_path(base_dir)).ok()?;
    serde_json::from_str::<serde_json::Value>(&content)
        .ok()?
        .get("model_catalog_json")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn read_previous_experimental_model(base_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(experimental_model_previous_catalog_path(base_dir)).ok()?;
    serde_json::from_str::<serde_json::Value>(&content)
        .ok()?
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn persist_previous_experimental_catalog_reference(
    base_dir: &Path,
    reference: Option<&str>,
    model: Option<&str>,
) -> Result<(), String> {
    let path = experimental_model_previous_catalog_path(base_dir);
    let reference = reference.map(str::trim).filter(|value| !value.is_empty());
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    if reference.is_none() && model.is_none() {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "清理原模型目录记录失败: path={}, error={}",
                path.display(),
                error
            )),
        };
    };
    let mut content = serde_json::json!({});
    if let Some(reference) = reference {
        content["model_catalog_json"] = serde_json::Value::String(reference.to_string());
    }
    if let Some(model) = model {
        content["model"] = serde_json::Value::String(model.to_string());
    }
    let mut content = serde_json::to_string_pretty(&content)
        .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_PREVIOUS_SERIALIZE_FAILED".to_string())?;
    content.push('\n');
    write_string_atomic(&path, &content)
        .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_PREVIOUS_WRITE_FAILED".to_string())
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ExperimentalModelCatalogConfig {
    #[serde(default)]
    version: u32,
    models: Vec<CodexExperimentalModelDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_model_id: Option<String>,
}

fn read_experimental_model_default_model_id(base_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(experimental_model_config_path(base_dir)).ok()?;
    let config = serde_json::from_str::<ExperimentalModelCatalogConfig>(&content).ok()?;
    if config.version < EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION {
        return None;
    }
    let default_model_id = config.default_model_id?.trim().to_string();
    if default_model_id.is_empty() {
        return None;
    }
    config
        .models
        .iter()
        .find(|model| {
            model
                .model_id
                .trim()
                .eq_ignore_ascii_case(&default_model_id)
        })
        .map(|model| model.model_id.trim().to_string())
}

fn experimental_model_config_requires_catalog_migration(base_dir: &Path) -> bool {
    let path = experimental_model_config_path(base_dir);
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<ExperimentalModelCatalogConfig>(&content)
        .map(|config| config.version < EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION)
        .unwrap_or(true)
}

fn default_experimental_model_definitions(
    _base_dir: &Path,
) -> Vec<CodexExperimentalModelDefinition> {
    let model_ids = SHIPPED_VISIBLE_CODEX_MODEL_IDS
        .iter()
        .map(|model_id| (*model_id).to_string())
        .collect::<Vec<_>>();
    let catalog = crate::modules::codex_protocol::build_codex_client_models_response(&model_ids);
    let models = catalog
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| {
                    if model
                        .get("visibility")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|visibility| visibility.eq_ignore_ascii_case("hide"))
                    {
                        return None;
                    }
                    let model_id = model
                        .get("slug")
                        .and_then(serde_json::Value::as_str)?
                        .trim();
                    if model_id.is_empty() {
                        return None;
                    }
                    let display_name = model
                        .get("display_name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(model_id);
                    Some(CodexExperimentalModelDefinition {
                        model_id: model_id.to_string(),
                        display_name: model_catalog_display_name(model_id, display_name),
                        reasoning_efforts: None,
                        context_window: None,
                        auto_compact_token_limit: None,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if models.is_empty() {
        model_ids
            .into_iter()
            .map(|model_id| CodexExperimentalModelDefinition {
                display_name: model_id.clone(),
                model_id,
                reasoning_efforts: None,
                context_window: None,
                auto_compact_token_limit: None,
            })
            .collect()
    } else {
        models
    }
}

fn model_catalog_display_name(model_id: &str, fallback: &str) -> String {
    match model_id.trim().to_ascii_lowercase().as_str() {
        "gpt-5.6-sol" => "5.6 Sol".to_string(),
        "gpt-5.6-terra" => "5.6 Terra".to_string(),
        "gpt-5.6-luna" => "5.6 Luna".to_string(),
        "gpt-5.3-codex" => "5.3 Codex".to_string(),
        "gpt-5.5" => "5.5".to_string(),
        "gpt-5.4" => "5.4".to_string(),
        "gpt-5.4-mini" => "5.4 Mini".to_string(),
        "gpt-5.3-codex-spark" => "5.3 Codex Spark".to_string(),
        "gpt-5.6-sol-wm" => "5.6 Sol WM".to_string(),
        _ => fallback.trim().to_string(),
    }
}

fn is_valid_model_catalog_id(model_id: &str) -> bool {
    !model_id.is_empty()
        && model_id.len() <= 128
        && model_id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn normalize_reasoning_efforts(
    efforts: Option<Vec<String>>,
) -> Result<Option<Vec<String>>, String> {
    let Some(efforts) = efforts else {
        return Ok(None);
    };
    let mut normalized = Vec::new();
    for effort in efforts {
        let effort = effort.trim().to_ascii_lowercase();
        if !CODEX_REASONING_EFFORTS.contains(&effort.as_str()) {
            return Err("EXPERIMENTAL_MODEL_CATALOG_REASONING_EFFORT_INVALID".to_string());
        }
        if !normalized.contains(&effort) {
            normalized.push(effort);
        }
    }
    if normalized.is_empty() {
        return Ok(None);
    }
    Ok(Some(normalized))
}

fn normalize_experimental_model_definitions(
    models: Vec<CodexExperimentalModelDefinition>,
) -> Result<Vec<CodexExperimentalModelDefinition>, String> {
    if models.is_empty() {
        return Err("EXPERIMENTAL_MODEL_CATALOG_MODELS_REQUIRED".to_string());
    }

    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(models.len());
    for model in models {
        let model_id = model.model_id.trim();
        let display_name = model.display_name.trim();
        if !is_valid_model_catalog_id(model_id) {
            return Err("EXPERIMENTAL_MODEL_CATALOG_MODEL_ID_INVALID".to_string());
        }
        if display_name.is_empty() || display_name.chars().count() > 100 {
            return Err("EXPERIMENTAL_MODEL_CATALOG_DISPLAY_NAME_INVALID".to_string());
        }
        let key = model_id.to_ascii_lowercase();
        if !seen.insert(key) {
            return Err("EXPERIMENTAL_MODEL_CATALOG_MODEL_ID_DUPLICATE".to_string());
        }
        let context_window = model.context_window.filter(|value| *value > 0);
        if model.context_window.is_some() && context_window.is_none() {
            return Err("EXPERIMENTAL_MODEL_CATALOG_CONTEXT_WINDOW_INVALID".to_string());
        }
        let auto_compact_token_limit = model.auto_compact_token_limit.filter(|value| *value > 0);
        if model.auto_compact_token_limit.is_some() && auto_compact_token_limit.is_none() {
            return Err("EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_INVALID".to_string());
        }
        if let (Some(context_window), Some(auto_compact_token_limit)) =
            (context_window, auto_compact_token_limit)
        {
            if auto_compact_token_limit >= context_window {
                return Err("EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_RANGE_INVALID".to_string());
            }
        }
        normalized.push(CodexExperimentalModelDefinition {
            model_id: model_id.to_string(),
            display_name: display_name.to_string(),
            reasoning_efforts: normalize_reasoning_efforts(model.reasoning_efforts.clone())?,
            context_window,
            auto_compact_token_limit,
        });
    }
    Ok(normalized)
}

fn apply_model_context_config_to_catalog(
    catalog: &mut serde_json::Value,
    models: &[CodexExperimentalModelDefinition],
) {
    let definitions = models
        .iter()
        .map(|model| {
            (
                model.model_id.clone(),
                model.context_window,
                model.auto_compact_token_limit,
            )
        })
        .collect::<Vec<_>>();
    crate::modules::codex_protocol::apply_model_context_overrides(catalog, &definitions);
}

pub(crate) fn decorate_managed_model_catalog_for_profile(
    base_dir: &Path,
    catalog_json: &str,
) -> Result<String, String> {
    if !experimental_model_policy_enabled(base_dir) {
        return Ok(catalog_json.to_string());
    }
    let mut catalog = serde_json::from_str::<serde_json::Value>(catalog_json)
        .map_err(|error| format!("解析 Codex 受管模型目录失败: {}", error))?;
    let models = read_experimental_model_definitions(base_dir);
    apply_model_context_config_to_catalog(&mut catalog, &models);
    serde_json::to_string_pretty(&catalog)
        .map_err(|error| format!("序列化 Codex 受管模型目录失败: {}", error))
}

pub(crate) fn read_experimental_model_definitions(
    base_dir: &Path,
) -> Vec<CodexExperimentalModelDefinition> {
    let path = experimental_model_config_path(base_dir);
    let Ok(content) = fs::read_to_string(&path) else {
        return default_experimental_model_definitions(base_dir);
    };
    match serde_json::from_str::<ExperimentalModelCatalogConfig>(&content)
        .map_err(|error| error.to_string())
        .and_then(|config| normalize_experimental_model_definitions(config.models))
    {
        Ok(_models) if experimental_model_config_requires_catalog_migration(base_dir) => {
            // A release migration intentionally resets all pre-release lists to the
            // shipped visible-model preset. Later user edits are preserved by version 4+.
            default_experimental_model_definitions(base_dir)
        }
        Ok(models) => models,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex实验模型] 模型配置无效，使用默认值: path={}, error={}",
                path.display(),
                error
            ));
            default_experimental_model_definitions(base_dir)
        }
    }
}

fn persist_experimental_model_definitions(
    base_dir: &Path,
    models: Vec<CodexExperimentalModelDefinition>,
    default_model_id: Option<&str>,
) -> Result<Vec<CodexExperimentalModelDefinition>, String> {
    let models = normalize_experimental_model_definitions(models)?;
    let default_model_id = default_model_id.and_then(|value| {
        models
            .iter()
            .find(|model| model.model_id.eq_ignore_ascii_case(value.trim()))
            .map(|model| model.model_id.clone())
    });
    let mut content = serde_json::to_string_pretty(&ExperimentalModelCatalogConfig {
        version: EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION,
        models: models.clone(),
        default_model_id,
    })
    .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_CONFIG_SERIALIZE_FAILED".to_string())?;
    content.push('\n');
    write_string_atomic(&experimental_model_config_path(base_dir), &content)
        .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_CONFIG_WRITE_FAILED".to_string())?;
    Ok(models)
}

fn experimental_model_policy_enabled(base_dir: &Path) -> bool {
    experimental_model_policy_path(base_dir).is_file()
}

fn persist_experimental_model_policy(base_dir: &Path, enabled: bool) -> Result<(), String> {
    let path = experimental_model_policy_path(base_dir);
    if enabled {
        return write_string_atomic(&path, "enabled\n").map_err(|error| {
            format!(
                "写入 Codex 实验模型策略失败: path={}, error={}",
                path.display(),
                error
            )
        });
    }
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "清理 Codex 实验模型策略失败: path={}, error={}",
            path.display(),
            error
        )),
    }
}

fn build_experimental_model_catalog(base_dir: &Path) -> Result<String, String> {
    let model_definitions = read_experimental_model_definitions(base_dir);
    let definitions = model_definitions
        .iter()
        .map(|model| {
            (
                model.model_id.clone(),
                model.display_name.clone(),
                model.reasoning_efforts.clone(),
            )
        })
        .collect::<Vec<_>>();
    let mut catalog =
        crate::modules::codex_protocol::build_codex_client_models_response_with_model_definitions_and_reasoning(&definitions);
    apply_model_context_config_to_catalog(&mut catalog, &model_definitions);
    serde_json::to_string_pretty(&catalog)
        .map(|mut content| {
            content.push('\n');
            content
        })
        .map_err(|error| format!("生成 Codex 模型目录失败: {}", error))
}

fn merge_existing_catalog_into_experimental_catalog(
    base_dir: &Path,
    configured_catalog: Option<&str>,
    generated_content: &str,
) -> Result<String, String> {
    let Some(configured_catalog) = configured_catalog
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(generated_content.to_string());
    };
    if catalog_ref_targets_profile_file(
        configured_catalog,
        base_dir,
        CODEX_MANAGED_MODEL_CATALOG_FILE,
    ) {
        return Ok(generated_content.to_string());
    }

    let source_path = resolve_catalog_reference(configured_catalog, base_dir);
    let Ok(source_content) = fs::read_to_string(&source_path) else {
        logger::log_warn(&format!(
            "[Codex实验模型] 原模型目录不存在，继续生成受管目录: path={}",
            source_path.display()
        ));
        return Ok(generated_content.to_string());
    };
    let Ok(source_catalog) = serde_json::from_str::<serde_json::Value>(&source_content) else {
        logger::log_warn(&format!(
            "[Codex实验模型] 原模型目录不是合法 JSON，继续生成受管目录: path={}",
            source_path.display()
        ));
        return Ok(generated_content.to_string());
    };
    let mut merged_catalog = serde_json::from_str::<serde_json::Value>(generated_content)
        .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_SERIALIZE_FAILED".to_string())?;
    let Some(source_models) = source_catalog
        .get("models")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(generated_content.to_string());
    };
    let Some(merged_models) = merged_catalog
        .get_mut("models")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(generated_content.to_string());
    };

    for source_model in source_models {
        let Some(source_slug) = source_model
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|slug| !slug.is_empty())
        else {
            continue;
        };
        if let Some(existing_model) = merged_models.iter_mut().find(|model| {
            model
                .get("slug")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|slug| slug.eq_ignore_ascii_case(source_slug))
        }) {
            *existing_model = source_model.clone();
        } else {
            merged_models.push(source_model.clone());
        }
    }

    serde_json::to_string_pretty(&merged_catalog)
        .map(|mut content| {
            content.push('\n');
            content
        })
        .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_SERIALIZE_FAILED".to_string())
}

fn read_catalog_model_definitions(
    base_dir: &Path,
    configured_catalog: Option<&str>,
) -> Vec<CodexExperimentalModelDefinition> {
    let Some(reference) = configured_catalog
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };
    let source_path = resolve_catalog_reference(reference, base_dir);
    let Ok(content) = fs::read_to_string(source_path) else {
        return Vec::new();
    };
    let Ok(catalog) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };
    catalog
        .get("models")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            if model
                .get("visibility")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|visibility| visibility.eq_ignore_ascii_case("hide"))
            {
                return None;
            }
            let model_id = model
                .get("slug")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| is_valid_model_catalog_id(value))?;
            let display_name = model
                .get("display_name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty() && value.chars().count() <= 100)
                .unwrap_or(model_id);
            let official_model =
                crate::modules::codex_protocol::build_codex_client_models_response(&[
                    model_id.to_string()
                ])
                .get("models")
                .and_then(serde_json::Value::as_array)
                .and_then(|models| models.first())
                .cloned();
            let official_context_window = official_model
                .as_ref()
                .and_then(|model| model.get("context_window"))
                .and_then(serde_json::Value::as_i64);
            let official_auto_compact_token_limit = official_model
                .as_ref()
                .and_then(|model| model.get("auto_compact_token_limit"))
                .and_then(serde_json::Value::as_i64);
            let context_window = model
                .get("context_window")
                .and_then(serde_json::Value::as_i64)
                .filter(|value| *value > 0 && Some(*value) != official_context_window);
            let auto_compact_token_limit = model
                .get("auto_compact_token_limit")
                .and_then(serde_json::Value::as_i64)
                .filter(|value| *value > 0 && Some(*value) != official_auto_compact_token_limit)
                .or_else(|| match context_window {
                    Some(1_000_000) => Some(900_000),
                    Some(516_000) => Some(460_000),
                    _ => None,
                });
            Some(CodexExperimentalModelDefinition {
                model_id: model_id.to_string(),
                display_name: model_catalog_display_name(model_id, display_name),
                reasoning_efforts: None,
                context_window,
                auto_compact_token_limit,
            })
        })
        .collect()
}

fn merge_model_definitions(
    mut definitions: Vec<CodexExperimentalModelDefinition>,
    extra: Vec<CodexExperimentalModelDefinition>,
) -> Vec<CodexExperimentalModelDefinition> {
    for model in extra {
        if let Some(existing) = definitions
            .iter_mut()
            .find(|existing| existing.model_id.eq_ignore_ascii_case(&model.model_id))
        {
            if model.context_window.is_some() {
                existing.context_window = model.context_window;
            }
            if model.auto_compact_token_limit.is_some() {
                existing.auto_compact_token_limit = model.auto_compact_token_limit;
            }
        } else {
            definitions.push(model);
        }
    }
    definitions
}

fn inspect_experimental_model_catalog(
    base_dir: &Path,
    doc: &Document,
) -> Result<ExperimentalModelCatalogState, String> {
    let configured_catalog = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let cockpit_managed_catalog_configured = configured_catalog
        .is_some_and(|value| catalog_ref_targets_cockpit_managed_file(value, base_dir));
    let policy_enabled = experimental_model_policy_enabled(base_dir);
    let enabled = policy_enabled;

    Ok(ExperimentalModelCatalogState {
        enabled,
        available: true,
        unavailable_reason: None,
        conflict: if policy_enabled || cockpit_managed_catalog_configured {
            None
        } else {
            configured_catalog.map(str::to_string)
        },
    })
}

fn apply_experimental_model_catalog_to_doc(
    base_dir: &Path,
    doc: &mut Document,
    enabled: Option<bool>,
) -> Result<bool, String> {
    let Some(enabled) = enabled else {
        return Ok(false);
    };
    let configured_catalog = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let managed_catalog_configured = configured_catalog
        .as_deref()
        .is_some_and(|value| catalog_ref_targets_cockpit_managed_file(value, base_dir));
    let policy_enabled = experimental_model_policy_enabled(base_dir);
    let currently_enabled = managed_catalog_configured && policy_enabled;

    if !enabled {
        if currently_enabled || policy_enabled {
            if managed_catalog_configured {
                if let Some(previous_catalog) =
                    read_previous_experimental_catalog_reference(base_dir)
                {
                    doc[CODEX_CONFIG_MODEL_CATALOG_JSON_KEY] = value(previous_catalog);
                } else {
                    let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
                }
            }
            if let Some(previous_model) = read_previous_experimental_model(base_dir) {
                doc["model"] = value(previous_model);
            }
        }
        return Ok(currently_enabled || policy_enabled);
    }

    let has_saved_model_definitions = experimental_model_config_path(base_dir).is_file();
    let migrate_saved_model_definitions = has_saved_model_definitions
        && experimental_model_config_requires_catalog_migration(base_dir);
    let mut experimental_models = read_experimental_model_definitions(base_dir);
    let user_catalog_reference = configured_catalog
        .as_deref()
        .filter(|catalog| !catalog_ref_targets_cockpit_managed_file(catalog, base_dir));
    let catalog_reference_for_models = configured_catalog.as_deref();
    if !has_saved_model_definitions || migrate_saved_model_definitions {
        if !migrate_saved_model_definitions {
            experimental_models = merge_model_definitions(
                experimental_models,
                read_catalog_model_definitions(base_dir, catalog_reference_for_models),
            );
        }
        experimental_models =
            persist_experimental_model_definitions(base_dir, experimental_models, None)?;
    }
    if experimental_models.is_empty() {
        return Err("EXPERIMENTAL_MODEL_CATALOG_MODELS_REQUIRED".to_string());
    }
    let generated_content = build_experimental_model_catalog(base_dir)
        .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_SERIALIZE_FAILED".to_string())?;
    if let Some(default_model_id) = read_experimental_model_default_model_id(base_dir) {
        if experimental_models
            .iter()
            .any(|model| model.model_id.eq_ignore_ascii_case(&default_model_id))
        {
            doc["model"] = value(default_model_id);
        }
    }
    if read_previous_experimental_catalog_reference(base_dir).is_none()
        && read_previous_experimental_model(base_dir).is_none()
    {
        let previous_model = doc.get("model").and_then(|item| item.as_str());
        persist_previous_experimental_catalog_reference(
            base_dir,
            user_catalog_reference,
            previous_model,
        )?;
    }
    let content = if has_saved_model_definitions && !migrate_saved_model_definitions {
        generated_content
    } else {
        merge_existing_catalog_into_experimental_catalog(
            base_dir,
            user_catalog_reference,
            &generated_content,
        )?
    };
    write_string_atomic(&experimental_model_catalog_path(base_dir), &content)
        .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_WRITE_FAILED".to_string())?;
    crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir)
        .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_CACHE_CLEAR_FAILED".to_string())?;
    cleanup_legacy_managed_model_catalogs(base_dir);
    doc[CODEX_CONFIG_MODEL_CATALOG_JSON_KEY] = value(CODEX_MANAGED_MODEL_CATALOG_FILE);
    Ok(false)
}

fn enforce_experimental_model_policy_for_dir(base_dir: &Path) -> Result<(), String> {
    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };
    let migrated_legacy_catalog = migrate_legacy_managed_catalog_reference(base_dir, &mut doc)?;
    apply_experimental_model_catalog_to_doc(base_dir, &mut doc, Some(true))?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;

    if migrated_legacy_catalog {
        cleanup_legacy_managed_model_catalogs(base_dir);
        let _ = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir);
    }
    persist_experimental_model_policy(base_dir, true)
}

pub(crate) fn reapply_experimental_model_policy_if_enabled(
    base_dir: &Path,
) -> Result<bool, String> {
    if !experimental_model_policy_enabled(base_dir) {
        return Ok(false);
    }
    enforce_experimental_model_policy_for_dir(base_dir)?;
    Ok(true)
}

pub fn read_quick_config_from_config_toml(base_dir: &Path) -> Result<CodexQuickConfig, String> {
    let config_path = get_config_toml_path(base_dir);
    let content = fs::read_to_string(config_path).unwrap_or_default();
    let doc = if content.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&content)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };
    let detected_model_context_window =
        read_top_level_int_from_doc(&doc, CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY);
    let detected_auto_compact_token_limit =
        read_top_level_int_from_doc(&doc, CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY)
            .filter(|value| *value > 0);
    let experimental = inspect_experimental_model_catalog(base_dir, &doc)?;
    let experimental_models = read_experimental_model_definitions(base_dir);
    let experimental_default_model_id =
        read_experimental_model_default_model_id(base_dir).or_else(|| {
            if !experimental.enabled {
                return None;
            }
            let configured_model = doc.get("model").and_then(|item| item.as_str())?.trim();
            experimental_models
                .iter()
                .find(|model| model.model_id.eq_ignore_ascii_case(configured_model))
                .map(|model| model.model_id.clone())
        });

    Ok(CodexQuickConfig {
        context_window_1m: detected_model_context_window == Some(CODEX_CONTEXT_WINDOW_1M_VALUE),
        auto_compact_token_limit: detected_auto_compact_token_limit
            .unwrap_or(CODEX_AUTO_COMPACT_DEFAULT_LIMIT),
        detected_model_context_window,
        detected_auto_compact_token_limit,
        experimental_model_catalog_enabled: experimental.enabled,
        experimental_model_catalog_available: experimental.available,
        experimental_model_catalog_unavailable_reason: experimental.unavailable_reason,
        experimental_model_catalog_conflict: experimental.conflict,
        experimental_model_catalog_models: experimental_models,
        experimental_model_catalog_default_model_id: experimental_default_model_id,
    })
}

pub fn load_current_quick_config() -> Result<CodexQuickConfig, String> {
    read_quick_config_from_config_toml(&get_codex_home())
}

fn write_quick_config_to_config_toml(
    base_dir: &Path,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    experimental_model_catalog_enabled: Option<bool>,
    experimental_model_catalog_models: Option<Vec<CodexExperimentalModelDefinition>>,
) -> Result<CodexQuickConfig, String> {
    write_quick_config_to_config_toml_with_default(
        base_dir,
        model_context_window,
        auto_compact_token_limit,
        experimental_model_catalog_enabled,
        experimental_model_catalog_models,
        None,
    )
}

fn write_quick_config_to_config_toml_with_default(
    base_dir: &Path,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    experimental_model_catalog_enabled: Option<bool>,
    experimental_model_catalog_models: Option<Vec<CodexExperimentalModelDefinition>>,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<CodexQuickConfig, String> {
    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();

    if existing.trim().is_empty()
        && model_context_window.is_none()
        && auto_compact_token_limit.is_none()
        && experimental_model_catalog_enabled.is_none()
        && experimental_model_catalog_models.is_none()
    {
        return read_quick_config_from_config_toml(base_dir);
    }

    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };
    let migrated_legacy_catalog = migrate_legacy_managed_catalog_reference(base_dir, &mut doc)?;

    if let Some(context_window) = model_context_window {
        if context_window <= 0 {
            return Err("上下文窗口必须大于 0".to_string());
        }
        doc[CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY] = value(context_window);
    } else {
        let _ = doc.remove(CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY);
    }

    if let Some(compact_limit) = auto_compact_token_limit {
        if compact_limit <= 0 {
            return Err("自动压缩阈值必须大于 0".to_string());
        }
        doc[CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY] = value(compact_limit);
    } else {
        let _ = doc.remove(CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY);
    }

    if let Some(models) = experimental_model_catalog_models {
        persist_experimental_model_definitions(
            base_dir,
            models,
            experimental_model_catalog_default_model_id.as_deref(),
        )?;
    }

    let effective_experimental_enabled = experimental_model_catalog_enabled
        .or_else(|| experimental_model_policy_enabled(base_dir).then_some(true));
    let remove_experimental_catalog_after_write = apply_experimental_model_catalog_to_doc(
        base_dir,
        &mut doc,
        effective_experimental_enabled,
    )?;

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;

    if migrated_legacy_catalog {
        cleanup_legacy_managed_model_catalogs(base_dir);
        let _ = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir);
    }

    if let Some(enabled) = experimental_model_catalog_enabled {
        persist_experimental_model_policy(base_dir, enabled)?;
    }

    if remove_experimental_catalog_after_write {
        if let Err(error) = crate::modules::atomic_write::remove_file_locked(
            &experimental_model_catalog_path(base_dir),
        ) {
            logger::log_warn(&format!(
                "[Codex实验模型] 配置已停用，但清理受管目录失败: profile={}, error={}",
                base_dir.display(),
                error
            ));
        }
        let _ = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir);
    }
    if experimental_model_catalog_enabled == Some(false) {
        cleanup_legacy_managed_model_catalogs(base_dir);
        persist_previous_experimental_catalog_reference(base_dir, None, None)?;
    }

    read_quick_config_from_config_toml(base_dir)
}

pub fn save_current_quick_config(
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    experimental_model_catalog_enabled: Option<bool>,
    experimental_model_catalog_models: Option<Vec<CodexExperimentalModelDefinition>>,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<CodexQuickConfig, String> {
    save_quick_config_for_base_dir_with_default(
        &get_codex_home(),
        model_context_window,
        auto_compact_token_limit,
        experimental_model_catalog_enabled,
        experimental_model_catalog_models,
        experimental_model_catalog_default_model_id,
    )
}

pub fn save_quick_config_for_base_dir(
    base_dir: &Path,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    experimental_model_catalog_enabled: Option<bool>,
    experimental_model_catalog_models: Option<Vec<CodexExperimentalModelDefinition>>,
) -> Result<CodexQuickConfig, String> {
    write_quick_config_to_config_toml(
        base_dir,
        model_context_window,
        auto_compact_token_limit,
        experimental_model_catalog_enabled,
        experimental_model_catalog_models,
    )
}

pub fn save_quick_config_for_base_dir_with_default(
    base_dir: &Path,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    experimental_model_catalog_enabled: Option<bool>,
    experimental_model_catalog_models: Option<Vec<CodexExperimentalModelDefinition>>,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<CodexQuickConfig, String> {
    write_quick_config_to_config_toml_with_default(
        base_dir,
        model_context_window,
        auto_compact_token_limit,
        experimental_model_catalog_enabled,
        experimental_model_catalog_models,
        experimental_model_catalog_default_model_id,
    )
}

fn read_api_provider_from_config_toml(base_dir: &Path) -> ApiProviderConfig {
    let config_path = get_config_toml_path(base_dir);
    let content = match fs::read_to_string(config_path) {
        Ok(content) if !content.trim().is_empty() => content,
        _ => {
            return ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: None,
                provider_id: None,
                provider_name: None,
            };
        }
    };

    let doc = match crate::modules::codex_config_format::read_codex_config_doc_from_str(&content) {
        Ok(doc) => doc,
        Err(_) => {
            return ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: None,
                provider_id: None,
                provider_name: None,
            };
        }
    };

    let openai_base_url = normalize_api_base_url(
        doc.get(CODEX_CONFIG_OPENAI_BASE_URL_KEY)
            .and_then(|item| item.as_str()),
    );
    let model_provider = normalize_optional_ref(
        doc.get(CODEX_CONFIG_MODEL_PROVIDER_KEY)
            .and_then(|item| item.as_str()),
    );

    if let Some(provider_id) = model_provider {
        if provider_id == CODEX_OPENAI_PROVIDER_ID {
            return infer_api_provider_config(
                openai_base_url.as_deref(),
                Some(CodexApiProviderMode::OpenaiBuiltin),
                None,
                None,
            );
        }
        let provider_base_url = doc
            .get(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
            .and_then(|item| item.get(provider_id.as_str()))
            .and_then(|item| item.get("base_url"))
            .and_then(|item| item.as_str())
            .and_then(|raw| normalize_api_base_url(Some(raw)));
        let provider_name = normalize_api_provider_name(
            doc.get(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
                .and_then(|item| item.get(provider_id.as_str()))
                .and_then(|item| item.get("name"))
                .and_then(|item| item.as_str()),
        );

        return infer_api_provider_config(
            provider_base_url.as_deref(),
            Some(CodexApiProviderMode::Custom),
            Some(provider_id.as_str()),
            provider_name.as_deref(),
        );
    }

    infer_api_provider_config(
        openai_base_url.as_deref(),
        Some(CodexApiProviderMode::OpenaiBuiltin),
        None,
        None,
    )
}

fn write_api_provider_to_config_toml(
    base_dir: &Path,
    provider_config: &ApiProviderConfig,
) -> Result<(), String> {
    write_api_provider_to_config_toml_with_options(base_dir, provider_config, true)
}

fn write_api_provider_to_config_toml_with_options(
    base_dir: &Path,
    provider_config: &ApiProviderConfig,
    cleanup_managed_model_catalog: bool,
) -> Result<(), String> {
    let config_path = get_config_toml_path(base_dir);
    let normalized = provider_config.base_url.clone();

    if !config_path.exists() && normalized.is_none() {
        return Ok(());
    }

    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };

    match provider_config.mode {
        CodexApiProviderMode::OpenaiBuiltin => {
            let preserved_user_model_provider = doc
                .get(CODEX_CONFIG_MODEL_PROVIDER_KEY)
                .and_then(|item| item.as_str())
                .map(str::trim)
                .filter(|provider_id| {
                    !provider_id.is_empty() && !is_managed_model_provider_id(provider_id)
                })
                .map(ToOwned::to_owned);
            if cleanup_managed_model_catalog {
                remove_managed_model_catalog_from_doc(&mut doc);
            }
            let _ = doc.remove(CODEX_CONFIG_MODEL_PROVIDER_KEY);
            remove_managed_api_key_model_providers_from_doc(&mut doc);
            #[cfg(target_os = "windows")]
            {
                write_windows_builtin_openai_provider_to_doc(&mut doc, normalized.as_deref())?;
                if let Some(provider_id) = preserved_user_model_provider.as_deref() {
                    doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(provider_id);
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                if let Some(provider_id) = preserved_user_model_provider.as_deref() {
                    doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(provider_id);
                }
                match normalized.as_deref() {
                    Some(base_url) => {
                        doc[CODEX_CONFIG_OPENAI_BASE_URL_KEY] = value(base_url);
                    }
                    None => {
                        let _ = doc.remove(CODEX_CONFIG_OPENAI_BASE_URL_KEY);
                    }
                }
            }
        }
        CodexApiProviderMode::Custom => {
            remove_managed_model_catalog_from_doc(&mut doc);
            let _ = doc.remove(CODEX_CONFIG_OPENAI_BASE_URL_KEY);
            let provider_id = provider_config
                .provider_id
                .as_deref()
                .ok_or("自定义供应商缺少 provider_id")?;
            let provider_name = provider_config
                .provider_name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(provider_id);
            let base_url = normalized.as_deref().ok_or("自定义供应商缺少 Base URL")?;

            doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(provider_id);
            if doc.get(CODEX_CONFIG_MODEL_PROVIDERS_KEY).is_none() {
                doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY] = toml_edit::table();
            }
            let model_providers = doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY]
                .as_table_mut()
                .ok_or("config.toml 中 model_providers 不是合法表结构")?;
            if !model_providers.contains_key(provider_id) {
                model_providers[provider_id] = toml_edit::table();
            }
            let provider_table = model_providers[provider_id]
                .as_table_mut()
                .ok_or("config.toml 中目标 provider 不是合法表结构")?;
            provider_table["name"] = value(provider_name);
            provider_table["base_url"] = value(base_url);
            provider_table["wire_api"] = value(CODEX_PROVIDER_WIRE_API);
            provider_table["requires_openai_auth"] = value(false);
            provider_table["supports_websockets"] = value(false);
        }
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))
}

fn remove_managed_model_catalog_from_doc(doc: &mut Document) -> bool {
    let managed_catalog = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim);
    let uses_managed_catalog = matches!(
        managed_catalog,
        Some(CODEX_MANAGED_MODEL_CATALOG_FILE)
            | Some(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
            | Some(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
    );
    if uses_managed_catalog {
        let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
        return true;
    }
    false
}

fn remove_provider_managed_model_catalog_from_doc(doc: &mut Document) -> bool {
    let managed_catalog = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim);
    if matches!(
        managed_catalog,
        Some(CODEX_MANAGED_MODEL_CATALOG_FILE)
            | Some(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
            | Some(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
    ) {
        let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
        return true;
    }
    false
}

fn cleanup_experimental_model_catalog_for_dir(base_dir: &Path) -> Result<(), String> {
    let config_path = get_config_toml_path(base_dir);
    let managed_catalog_path = experimental_model_catalog_path(base_dir);
    if config_path.exists() {
        let existing = fs::read_to_string(&config_path).unwrap_or_default();
        if !existing.trim().is_empty() {
            let mut doc =
                crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
                    .map_err(|e| format!("解析 config.toml 失败: {}", e))?;
            let uses_experimental_catalog = doc
                .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
                .and_then(|item| item.as_str())
                .is_some_and(|catalog| catalog_ref_targets_cockpit_managed_file(catalog, base_dir));
            if uses_experimental_catalog {
                apply_experimental_model_catalog_to_doc(base_dir, &mut doc, Some(false))?;
                if !experimental_model_policy_enabled(base_dir) {
                    let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
                }
            }
            if uses_experimental_catalog {
                let content =
                    crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
                crate::modules::codex_config_format::write_codex_config_toml_atomic(
                    &config_path,
                    &content,
                )
                .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
            }
        }
    }

    if managed_catalog_path.exists() {
        crate::modules::atomic_write::remove_file_locked(&managed_catalog_path).map_err(
            |error| {
                format!(
                    "清理 Codex 实验模型目录失败: path={}, error={}",
                    managed_catalog_path.display(),
                    error
                )
            },
        )?;
    }
    cleanup_legacy_managed_model_catalogs(base_dir);
    let _ = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir);
    persist_previous_experimental_catalog_reference(base_dir, None, None)?;
    Ok(())
}

fn account_syncs_model_catalog_to_codex(account: &CodexAccount) -> bool {
    account.is_api_key_auth()
        && account.api_sync_model_catalog_to_codex
        && account.api_provider_mode == CodexApiProviderMode::Custom
        && account
            .api_wire_api
            .as_deref()
            .map(str::trim)
            .unwrap_or(CODEX_PROVIDER_WIRE_API)
            .eq_ignore_ascii_case(CODEX_PROVIDER_WIRE_API)
        && !account.api_model_catalog.is_empty()
}

fn sync_api_key_model_catalog_to_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<bool, String> {
    if !account_syncs_model_catalog_to_codex(account) {
        return Ok(false);
    }
    if is_deepseek_responses_account(account) {
        return sync_deepseek_shell_remap_catalog_to_dir(base_dir, account);
    }

    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };
    if let Some(configured_catalog) = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if configured_catalog != CODEX_MANAGED_MODEL_CATALOG_FILE
            && configured_catalog != CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE
            && configured_catalog != CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE
        {
            return Ok(false);
        }
    }

    let upstream_models = normalize_api_model_catalog(account.api_model_catalog.clone());
    let slots = crate::modules::codex_local_access::allocate_provider_model_slots(&upstream_models);
    let client_models = slots
        .iter()
        .map(|slot| slot.client_model.clone())
        .collect::<Vec<_>>();
    let selected_model_is_available = doc
        .get("model")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(|selected_model| {
            client_models
                .iter()
                .any(|model| model.eq_ignore_ascii_case(selected_model))
        })
        .unwrap_or(false);
    if !selected_model_is_available {
        if let Some(default_model) = client_models.first() {
            doc["model"] = value(default_model.as_str());
        }
    }
    let content = crate::modules::codex_local_access::decorate_account_catalog_context_windows(
        &crate::modules::codex_local_access::build_provider_model_catalog_json(&slots)?,
        &slots,
        account,
        crate::modules::codex_local_access::read_toml_model_context_window(&doc),
    )?;
    let content = decorate_managed_model_catalog_for_profile(base_dir, &content)?;
    let catalog_path = base_dir.join(CODEX_MANAGED_MODEL_CATALOG_FILE);
    write_string_atomic(&catalog_path, &content).map_err(|e| {
        format!(
            "写入 Codex 模型目录失败: path={}, error={}",
            catalog_path.display(),
            e
        )
    })?;
    cleanup_legacy_managed_model_catalogs(base_dir);

    doc[CODEX_CONFIG_MODEL_CATALOG_JSON_KEY] = value(CODEX_MANAGED_MODEL_CATALOG_FILE);
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
    // 模型目录变更后同步生图 header：无 gpt-image-2 时清掉残留 actor，避免客户端误开生图卡住。
    if let Err(err) = refresh_api_key_provider_projection_in_dir(base_dir, account) {
        logger::log_warn(&format!(
            "[Codex切号] 同步模型目录后刷新 provider 生图配置失败: path={}, error={}",
            base_dir.display(),
            err
        ));
    }
    Ok(true)
}

fn sync_or_cleanup_account_model_catalog_for_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    if account.is_api_key_auth() {
        cleanup_experimental_model_catalog_for_dir(base_dir)?;
    }
    let _ = remove_leftover_deepseek_models_json(base_dir);
    if is_deepseek_responses_account(account) {
        if account_uses_deepseek_cdp_injection(account) {
            write_deepseek_cdp_responses_runtime_to_dir(base_dir, account)?;
            return Ok(());
        }
        if is_deepseek_official_runtime_access(account) {
            write_deepseek_official_responses_runtime_to_dir(base_dir, account)?;
            let _ = cleanup_managed_model_catalog_for_dir(base_dir)?;
            return Ok(());
        }
        let _ = sync_deepseek_shell_remap_catalog_to_dir(base_dir, account)?;
        return Ok(());
    }
    let _ = cleanup_deepseek_official_model_catalog_for_dir(base_dir)?;
    if account_syncs_model_catalog_to_codex(account) {
        let _ = sync_api_key_model_catalog_to_dir(base_dir, account)?;
    } else {
        let _ = cleanup_managed_model_catalog_for_dir(base_dir)?;
        // 未同步受管目录时仍按账号 catalog 收敛 header（无 image 则清）。
        if let Err(err) = refresh_api_key_provider_projection_in_dir(base_dir, account) {
            logger::log_warn(&format!(
                "[Codex切号] 清理模型目录后刷新 provider 生图配置失败: path={}, error={}",
                base_dir.display(),
                err
            ));
        }
    }
    Ok(())
}

fn sync_or_cleanup_managed_model_catalog_for_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    let preserve_experimental_policy =
        read_quick_config_from_config_toml(base_dir)?.experimental_model_catalog_enabled;
    sync_or_cleanup_account_model_catalog_for_dir(base_dir, account)?;
    if preserve_experimental_policy {
        enforce_experimental_model_policy_for_dir(base_dir)?;
    }
    Ok(())
}

fn cleanup_managed_model_catalog_for_dir(base_dir: &Path) -> Result<bool, String> {
    let mut changed = false;
    for file_name in [
        CODEX_MANAGED_MODEL_CATALOG_FILE,
        CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE,
        CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE,
    ] {
        let catalog_path = base_dir.join(file_name);
        if catalog_path.exists() {
            fs::remove_file(&catalog_path).map_err(|e| {
                format!(
                    "删除 Codex 模型目录失败: path={}, error={}",
                    catalog_path.display(),
                    e
                )
            })?;
            changed = true;
        }
    }

    let config_path = get_config_toml_path(base_dir);
    if !config_path.exists() {
        return Ok(changed);
    }
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    if existing.trim().is_empty() {
        return Ok(changed);
    }
    let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
        .map_err(|e| format!("解析 config.toml 失败: {}", e))?;
    if remove_provider_managed_model_catalog_from_doc(&mut doc) {
        let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
            .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
        changed = true;
    }
    Ok(changed)
}

fn collect_managed_api_key_provider_ids() -> HashSet<String> {
    HashSet::from([
        CODEX_RUNTIME_MODEL_PROVIDER_ID.to_string(),
        CODEX_COCKPIT_API_PROVIDER_ID.to_string(),
        CODEX_LEGACY_API_KEY_OPENAI_PROVIDER_ID.to_string(),
    ])
}

fn is_managed_model_provider_id(provider_id: &str) -> bool {
    matches!(
        provider_id,
        CODEX_OPENAI_PROVIDER_ID
            | CODEX_RUNTIME_MODEL_PROVIDER_ID
            | CODEX_COCKPIT_API_PROVIDER_ID
            | CODEX_LEGACY_API_KEY_OPENAI_PROVIDER_ID
    )
}

fn remove_managed_api_key_model_providers_from_doc(doc: &mut Document) {
    let managed_provider_ids = collect_managed_api_key_provider_ids();
    let should_remove_model_providers = doc
        .get_mut(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
        .and_then(|item| item.as_table_mut())
        .map(|model_providers| {
            for provider_id in &managed_provider_ids {
                let _ = model_providers.remove(provider_id.as_str());
            }
            model_providers.is_empty()
        })
        .unwrap_or(false);

    if should_remove_model_providers {
        let _ = doc.remove(CODEX_CONFIG_MODEL_PROVIDERS_KEY);
    }
}

#[cfg(target_os = "windows")]
fn write_windows_builtin_openai_provider_to_doc(
    doc: &mut Document,
    base_url: Option<&str>,
) -> Result<(), String> {
    doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(CODEX_OPENAI_PROVIDER_ID);
    match base_url {
        Some(base_url) if base_url != CODEX_DEFAULT_OPENAI_BASE_URL => {
            doc[CODEX_CONFIG_OPENAI_BASE_URL_KEY] = value(base_url);
        }
        _ => {
            let _ = doc.remove(CODEX_CONFIG_OPENAI_BASE_URL_KEY);
        }
    }
    let should_remove_model_providers = doc
        .get_mut(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
        .and_then(|item| item.as_table_mut())
        .map(|model_providers| {
            let _ = model_providers.remove(CODEX_OPENAI_PROVIDER_ID);
            model_providers.is_empty()
        })
        .unwrap_or(false);
    if should_remove_model_providers {
        let _ = doc.remove(CODEX_CONFIG_MODEL_PROVIDERS_KEY);
    }
    Ok(())
}

fn api_key_account_supports_image_generation(account: &CodexAccount) -> bool {
    account.is_api_key_auth()
        && account
            .api_model_catalog
            .iter()
            .any(|model| model.trim().eq_ignore_ascii_case(CODEX_IMAGE_MODEL_ID))
}

/// 是否应写入 Codex 生图兼容 header（actor 等）。
/// - 本地 API 服务 loopback：始终 true（网关自带 image 能力）
/// - 第三方：仅当账号模型目录显式包含 gpt-image-2（无则清 header，避免卡 Confirming）
fn api_key_provider_should_enable_imagegen(
    account: &CodexAccount,
    provider_config: &ApiProviderConfig,
) -> bool {
    let base_url = provider_config
        .base_url
        .as_deref()
        .unwrap_or(CODEX_DEFAULT_OPENAI_BASE_URL);
    let is_local_access_loopback = provider_config.provider_id.as_deref()
        == Some(CODEX_RUNTIME_MODEL_PROVIDER_ID)
        && is_loopback_http_base_url(Some(base_url));
    if is_local_access_loopback {
        return true;
    }
    api_key_account_supports_image_generation(account)
}

fn remove_provider_static_header(provider_table: &mut toml_edit::Table, header_name: &str) {
    let mut remove_http_headers = false;
    if let Some(headers) = provider_table.get_mut(CODEX_CONFIG_HTTP_HEADERS_KEY) {
        if let Some(inline) = headers.as_inline_table_mut() {
            let matching_keys: Vec<String> = inline
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(header_name))
                .map(|(key, _)| key.to_string())
                .collect();
            for key in matching_keys {
                let _ = inline.remove(&key);
            }
            remove_http_headers = inline.is_empty();
        } else if let Some(table) = headers.as_table_mut() {
            let matching_keys: Vec<String> = table
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(header_name))
                .map(|(key, _)| key.to_string())
                .collect();
            for key in matching_keys {
                let _ = table.remove(&key);
            }
            remove_http_headers = table.is_empty();
        }
    }
    if remove_http_headers {
        let _ = provider_table.remove(CODEX_CONFIG_HTTP_HEADERS_KEY);
    }
}

fn set_provider_static_header(
    provider_table: &mut toml_edit::Table,
    header_name: &str,
    header_value: &str,
) {
    remove_provider_static_header(provider_table, header_name);
    if provider_table.get(CODEX_CONFIG_HTTP_HEADERS_KEY).is_none() {
        provider_table[CODEX_CONFIG_HTTP_HEADERS_KEY] =
            toml_edit::Item::Value(toml_edit::Value::InlineTable(toml_edit::InlineTable::new()));
    }

    let headers = provider_table
        .get_mut(CODEX_CONFIG_HTTP_HEADERS_KEY)
        .expect("http_headers should exist after initialization");
    if let Some(inline) = headers.as_inline_table_mut() {
        inline.insert(header_name, toml_edit::Value::from(header_value));
    } else if let Some(table) = headers.as_table_mut() {
        table[header_name] = value(header_value);
    } else {
        let mut inline = toml_edit::InlineTable::new();
        inline.insert(header_name, toml_edit::Value::from(header_value));
        *headers = toml_edit::Item::Value(toml_edit::Value::InlineTable(inline));
    }
}

fn remove_imagegen_headers(provider_table: &mut toml_edit::Table) {
    remove_provider_static_header(provider_table, CODEX_IMAGEGEN_ACTOR_HEADER);
    remove_provider_static_header(provider_table, CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER);
}

fn set_imagegen_headers(provider_table: &mut toml_edit::Table, images_only_for_chat: bool) {
    remove_imagegen_headers(provider_table);
    set_provider_static_header(
        provider_table,
        CODEX_IMAGEGEN_ACTOR_HEADER,
        CODEX_IMAGEGEN_ACTOR_HEADER_VALUE,
    );
    if images_only_for_chat {
        set_provider_static_header(
            provider_table,
            CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
            CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE,
        );
    }
}

fn write_api_key_bearer_provider_override_to_config_toml(
    base_dir: &Path,
    provider_config: &ApiProviderConfig,
    bearer_token: &str,
    supports_websockets: bool,
    supports_image_generation: bool,
    // true → Codex 使用 auth.json/Keychain OAuth 登录态（绑定 OAuth）。
    // false → 纯 API Key，配合 actor 走 bearer 生图。
    require_openai_auth: bool,
    wire_api: &str,
) -> Result<(), String> {
    // This is the compatibility path for runtimes that need a bearer distinct from auth.json or
    // provider-only capabilities that Codex's built-in `openai` entry cannot be configured with.
    let config_path = get_config_toml_path(base_dir);
    let bearer_token = normalize_api_key(bearer_token)
        .ok_or_else(|| "API Key 账号缺少可写入 provider 的密钥".to_string())?;
    let base_url = provider_config
        .base_url
        .as_deref()
        .unwrap_or(CODEX_DEFAULT_OPENAI_BASE_URL);
    let provider_name = provider_config
        .provider_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(CODEX_DEFAULT_RUNTIME_PROVIDER_NAME);
    let wire_api = match wire_api.trim().to_ascii_lowercase().as_str() {
        "chat_completions" | "chat" => "chat_completions",
        _ => CODEX_PROVIDER_WIRE_API,
    };

    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };

    // A custom compatibility provider owns its endpoint. Drop any URL left by the previously
    // active built-in OpenAI relay so two accounts' routing state cannot coexist.
    let _ = doc.remove(CODEX_CONFIG_OPENAI_BASE_URL_KEY);
    doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(CODEX_RUNTIME_MODEL_PROVIDER_ID);
    if doc.get(CODEX_CONFIG_MODEL_PROVIDERS_KEY).is_none() {
        doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY] = toml_edit::table();
    }
    let model_providers = doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY]
        .as_table_mut()
        .ok_or("config.toml 中 model_providers 不是合法表结构")?;
    if !model_providers.contains_key(CODEX_RUNTIME_MODEL_PROVIDER_ID) {
        model_providers[CODEX_RUNTIME_MODEL_PROVIDER_ID] = toml_edit::table();
    }
    let provider_table = model_providers[CODEX_RUNTIME_MODEL_PROVIDER_ID]
        .as_table_mut()
        .ok_or("config.toml 中目标 provider 不是合法表结构")?;
    provider_table["name"] = value(provider_name);
    provider_table["base_url"] = value(base_url);
    provider_table["wire_api"] = value(wire_api);
    // require_openai_auth 与生图 headers 解耦：
    // - 纯 API Key 生图：require=false + actor
    // - 绑定 OAuth 的本地 API：require=true（显示账号）+ actor + chat disable（生图走本地）
    provider_table["requires_openai_auth"] = value(require_openai_auth);
    provider_table[CODEX_CONFIG_EXPERIMENTAL_BEARER_TOKEN_KEY] = value(bearer_token);
    provider_table["supports_websockets"] = value(supports_websockets);
    let is_local_access_loopback = provider_config.provider_id.as_deref()
        == Some(CODEX_RUNTIME_MODEL_PROVIDER_ID)
        && is_loopback_http_base_url(Some(base_url));
    if supports_image_generation {
        set_imagegen_headers(provider_table, is_local_access_loopback);
    } else {
        remove_imagegen_headers(provider_table);
    }
    // 本地 API 服务：写入实例 ID，供网关/请求日志区分多开来源。
    if is_local_access_loopback {
        let instance_id = client_instance_id_for_profile_dir(base_dir);
        set_provider_static_header(
            provider_table,
            CODEX_CLIENT_INSTANCE_ID_HEADER,
            &instance_id,
        );
    } else {
        remove_provider_static_header(provider_table, CODEX_CLIENT_INSTANCE_ID_HEADER);
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))
}

fn write_api_key_builtin_openai_to_config_toml(
    base_dir: &Path,
    provider_config: &ApiProviderConfig,
    cleanup_managed_model_catalog: bool,
) -> Result<(), String> {
    // Provider id/name remain user-facing Cockpit metadata. A Responses-compatible relay uses the
    // built-in Codex provider at runtime and changes only its account-specific `openai_base_url`.
    let builtin_openai_config = resolve_api_provider_config(
        provider_config.base_url.as_deref(),
        Some(CodexApiProviderMode::OpenaiBuiltin),
        None,
        None,
    )?;
    write_api_provider_to_config_toml_with_options(
        base_dir,
        &builtin_openai_config,
        cleanup_managed_model_catalog,
    )
}

fn api_key_account_requires_bearer_provider_override(
    account: &CodexAccount,
    provider_config: &ApiProviderConfig,
    oauth_bound: bool,
) -> bool {
    let base_url = provider_config
        .base_url
        .as_deref()
        .unwrap_or(CODEX_DEFAULT_OPENAI_BASE_URL);
    let uses_local_runtime = provider_config.provider_id.as_deref()
        == Some(CODEX_RUNTIME_MODEL_PROVIDER_ID)
        && is_loopback_http_base_url(Some(base_url));
    let requires_immediate_provider_override =
        crate::modules::codex_local_access::account_requires_provider_gateway(account)
            && !account_syncs_model_catalog_to_codex(account);
    let requires_http_only_responses_provider = account.api_provider_mode
        == CodexApiProviderMode::Custom
        && account.api_wire_api.as_deref() == Some(CODEX_PROVIDER_WIRE_API)
        && !account.api_supports_websockets
        && !account_syncs_model_catalog_to_codex(account);
    oauth_bound
        || uses_local_runtime
        || requires_immediate_provider_override
        || requires_http_only_responses_provider
        || api_key_provider_should_enable_imagegen(account, provider_config)
}

fn write_deepseek_official_responses_runtime_to_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    let api_key = normalize_api_key(account.openai_api_key.as_deref().unwrap_or_default())
        .ok_or_else(|| "DeepSeek 账号缺少 API Key".to_string())?;
    let selected_model = resolve_deepseek_startup_model(account);
    let _ = cleanup_managed_model_catalog_for_dir(base_dir)?;
    let _ = remove_leftover_deepseek_models_json(base_dir);
    if let Err(error) = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir) {
        logger::log_warn(&format!(
            "[Codex切号] 清理 Codex 模型缓存失败: path={}, error={}",
            base_dir.display(),
            error
        ));
    }

    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };

    doc["model"] = value(selected_model.as_str());
    let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
    doc["model_reasoning_effort"] = value("high");
    if doc
        .get("model_reasoning_summary")
        .and_then(|item| item.as_str())
        .is_some()
    {
        let _ = doc.remove("model_reasoning_summary");
    }
    doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(DEEPSEEK_PROVIDER_ID);
    doc["preferred_auth_method"] = value("apikey");
    let _ = doc.remove(CODEX_CONFIG_OPENAI_BASE_URL_KEY);

    if doc.get(CODEX_CONFIG_MODEL_PROVIDERS_KEY).is_none() {
        doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY] = toml_edit::table();
    }
    let model_providers = doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY]
        .as_table_mut()
        .ok_or("config.toml 中 model_providers 不是合法表结构")?;
    if !model_providers.contains_key(DEEPSEEK_PROVIDER_ID) {
        model_providers[DEEPSEEK_PROVIDER_ID] = toml_edit::table();
    }
    let provider_table = model_providers[DEEPSEEK_PROVIDER_ID]
        .as_table_mut()
        .ok_or("无法写入 DeepSeek provider")?;
    provider_table["name"] = value("DeepSeek");
    provider_table["base_url"] = value(DEEPSEEK_API_BASE_URL);
    provider_table["wire_api"] = value(CODEX_PROVIDER_WIRE_API);
    provider_table["requires_openai_auth"] = value(false);
    provider_table["supports_websockets"] = value(false);
    provider_table[CODEX_CONFIG_EXPERIMENTAL_BEARER_TOKEN_KEY] = value(api_key.as_str());
    remove_imagegen_headers(provider_table);

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
    logger::log_info(&format!(
        "[Codex切号] 已写入 DeepSeek 官方 Responses 直连配置: model={}, target_dir={}",
        selected_model,
        base_dir.display()
    ));
    Ok(())
}

fn write_deepseek_cdp_responses_runtime_to_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    write_deepseek_official_responses_runtime_to_dir(base_dir, account)?;
    // Keep the official DeepSeek slugs in config/catalog. The app-server talks to
    // api.deepseek.com directly, so shell IDs like gpt-5.4 cannot leave this machine.
    // CDP only injects those official slugs into the native picker.
    sync_deepseek_official_model_catalog_to_dir(base_dir, account)?;
    logger::log_info(&format!(
        "[Codex切号] 已写入 DeepSeek CDP 官方列表配置: startup_model={}, target_dir={}",
        resolve_deepseek_startup_model(account),
        base_dir.display()
    ));
    Ok(())
}

fn write_api_key_runtime_provider_to_config_toml(
    base_dir: &Path,
    account: &CodexAccount,
    provider_config: &ApiProviderConfig,
    oauth_bound: bool,
    cleanup_managed_model_catalog: bool,
) -> Result<(), String> {
    if account_uses_deepseek_cdp_injection(account) {
        return write_deepseek_official_responses_runtime_to_dir(base_dir, account);
    }
    if is_deepseek_official_runtime_access(account) {
        return write_deepseek_official_responses_runtime_to_dir(base_dir, account);
    }
    if !api_key_account_requires_bearer_provider_override(account, provider_config, oauth_bound) {
        return write_api_key_builtin_openai_to_config_toml(
            base_dir,
            provider_config,
            cleanup_managed_model_catalog,
        );
    }

    let api_key = normalize_api_key(account.openai_api_key.as_deref().unwrap_or_default())
        .ok_or_else(|| "API Key 账号缺少 OPENAI_API_KEY".to_string())?;
    let supports_image = api_key_provider_should_enable_imagegen(account, provider_config);
    let wire_api = account
        .api_wire_api
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(CODEX_PROVIDER_WIRE_API);
    write_api_key_bearer_provider_override_to_config_toml(
        base_dir,
        provider_config,
        &api_key,
        account.api_provider_mode == CodexApiProviderMode::Custom
            && account.api_supports_websockets,
        supports_image,
        oauth_bound || !supports_image,
        wire_api,
    )
}

