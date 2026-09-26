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

fn user_customized_model_catalog_marker_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_EXPERIMENTAL_MODEL_USER_CUSTOMIZED_FILE)
}

/// 用户是否主动接管了该 profile 的模型清单。
///
/// 只有 UI 保存清单时才会写标记；系统自动生成的清单（默认、迁移、重置）不带标记，
/// 因此仍会跟随账号池。旧版本遗留的清单没有标记，会被视为系统生成并重新跟随池。
pub(crate) fn has_saved_experimental_model_definitions(base_dir: &Path) -> bool {
    user_customized_model_catalog_marker_path(base_dir).is_file()
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

#[derive(Debug, Default)]
struct PreviousExperimentalCatalogState {
    had_catalog_reference: bool,
    catalog_reference: Option<String>,
    had_model: bool,
    model: Option<String>,
}

fn read_previous_experimental_catalog_state(
    base_dir: &Path,
) -> Option<PreviousExperimentalCatalogState> {
    let content = fs::read_to_string(experimental_model_previous_catalog_path(base_dir)).ok()?;
    let parsed = serde_json::from_str::<serde_json::Value>(&content).ok()?;
    let catalog_reference = parsed
        .get("model_catalog_json")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let model = parsed
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Some(PreviousExperimentalCatalogState {
        had_catalog_reference: parsed
            .get("had_model_catalog_json")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(catalog_reference.is_some()),
        catalog_reference,
        had_model: parsed
            .get("had_model")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(model.is_some()),
        model,
    })
}

fn persist_previous_experimental_catalog_reference(
    base_dir: &Path,
    reference: Option<&str>,
    model: Option<&str>,
) -> Result<(), String> {
    let path = experimental_model_previous_catalog_path(base_dir);
    let reference = reference.map(str::trim).filter(|value| !value.is_empty());
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    let mut content = serde_json::json!({
        "version": 2,
        "had_model_catalog_json": reference.is_some(),
        "had_model": model.is_some(),
    });
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

fn clear_previous_experimental_catalog_reference(base_dir: &Path) -> Result<(), String> {
    let path = experimental_model_previous_catalog_path(base_dir);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "清理原模型目录记录失败: path={}, error={}",
            path.display(),
            error
        )),
    }
}

fn clear_experimental_model_catalog_config(base_dir: &Path) -> Result<(), String> {
    let path = experimental_model_config_path(base_dir);
    crate::modules::atomic_write::remove_file_locked(&path)
        .map(|_| ())
        .map_err(|error| {
            format!(
                "清理 Codex 实验模型配置失败: path={}, error={}",
                path.display(),
                error
            )
        })
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ExperimentalModelCatalogConfig {
    #[serde(default)]
    version: u32,
    models: Vec<CodexExperimentalModelDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    migrations: Vec<String>,
}

const GPT_6_ASTRA_DEFAULT_REPAIR_MIGRATION_ID: &str = "repair-gpt-6-astra-default-model";
const BUILTIN_MODEL_DISPLAY_NAME_MIGRATION_ID: &str =
    "prefix-builtin-model-display-names";

/// 内建模型的早期短名 → 带官方 `GPT-` 前缀的展示名。
///
/// 只有「模型 ID 与短名同时匹配」的内建条目才会改名，用户自己改过的展示名不受影响。
const BUILTIN_MODEL_DISPLAY_NAME_MIGRATIONS: &[(&str, &str, &str)] = &[
    (GPT_6_ASTRA_MODEL_ID, "6 Astra", "GPT-6 Astra"),
    (GPT_6_SOL_MODEL_ID, "6 Sol", "GPT-6 Sol"),
    (GPT_6_LUNA_MODEL_ID, "6 Luna", "GPT-6 Luna"),
    ("gpt-5.6-sol", "5.6 Sol", "GPT-5.6 Sol"),
    ("gpt-5.6-terra", "5.6 Terra", "GPT-5.6 Terra"),
    ("gpt-5.6-luna", "5.6 Luna", "GPT-5.6 Luna"),
    ("gpt-5.3-codex", "5.3 Codex", "GPT-5.3 Codex"),
    ("gpt-5.5", "5.5", "GPT-5.5"),
    ("gpt-5.4", "5.4", "GPT-5.4"),
    ("gpt-5.4-mini", "5.4 Mini", "GPT-5.4 Mini"),
    ("gpt-5.3-codex-spark", "5.3 Codex Spark", "GPT-5.3 Codex Spark"),
    ("gpt-5.6-sol-wm", "5.6 Sol WM", "GPT-5.6 Sol WM"),
];

fn read_experimental_model_catalog_config(
    base_dir: &Path,
) -> Option<ExperimentalModelCatalogConfig> {
    let content = fs::read_to_string(experimental_model_config_path(base_dir)).ok()?;
    serde_json::from_str::<ExperimentalModelCatalogConfig>(&content).ok()
}

fn experimental_model_catalog_has_migration(base_dir: &Path, migration_id: &str) -> bool {
    read_experimental_model_catalog_config(base_dir)
        .is_some_and(|config| config.migrations.iter().any(|item| item == migration_id))
}

/// 该模型当前展示名是否是需要补前缀的内建短名；是则返回统一后的名字。
fn prefixed_builtin_display_name(
    model_id: &str,
    value: &serde_json::Value,
) -> Option<&'static str> {
    let name = value.as_str()?.trim();
    let model_id = model_id.trim();
    BUILTIN_MODEL_DISPLAY_NAME_MIGRATIONS
        .iter()
        .find(|(id, legacy, _)| {
            id.eq_ignore_ascii_case(model_id) && legacy.eq_ignore_ascii_case(name)
        })
        .map(|(_, _, canonical)| *canonical)
}

fn migrate_builtin_model_display_names(
    base_dir: &Path,
    doc: &Document,
) -> Result<bool, String> {
    let configured_catalog = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let managed_catalog_active = experimental_model_policy_enabled(base_dir)
        || configured_catalog.is_some_and(|value| {
            catalog_ref_targets_cockpit_managed_file(value, base_dir)
        });
    if !managed_catalog_active {
        return Ok(false);
    }

    let config_path = experimental_model_config_path(base_dir);
    let mut cached_config = None;
    let mut cached_config_needs_write = false;
    let mut migration_already_applied = false;
    if config_path.is_file() {
        let content = fs::read_to_string(&config_path).map_err(|error| {
            format!(
                "读取 Codex 模型配置缓存失败: path={}, error={}",
                config_path.display(),
                error
            )
        })?;
        let mut config = serde_json::from_str::<serde_json::Value>(&content).map_err(|error| {
            format!(
                "解析 Codex 模型配置缓存失败: path={}, error={}",
                config_path.display(),
                error
            )
        })?;
        migration_already_applied = config
            .get("migrations")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|migrations| {
                migrations.iter().any(|migration| {
                    migration.as_str() == Some(BUILTIN_MODEL_DISPLAY_NAME_MIGRATION_ID)
                })
            });
        if !migration_already_applied {
            if let Some(models) = config.get_mut("models").and_then(serde_json::Value::as_array_mut)
            {
                for model in models {
                    let model_id = model
                        .get("model_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let Some(display_name) = model
                        .get("display_name")
                        .and_then(|value| prefixed_builtin_display_name(&model_id, value))
                    else {
                        continue;
                    };
                    model["display_name"] = serde_json::Value::String(display_name.to_string());
                    cached_config_needs_write = true;
                }
            }
            if let Some(object) = config.as_object_mut() {
                let migrations = object
                    .entry("migrations")
                    .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                if let Some(migrations) = migrations.as_array_mut() {
                    migrations.push(serde_json::Value::String(
                        BUILTIN_MODEL_DISPLAY_NAME_MIGRATION_ID.to_string(),
                    ));
                    cached_config_needs_write = true;
                }
            }
        }
        cached_config = Some(config);
    }

    if migration_already_applied {
        return Ok(false);
    }

    let catalog_path = experimental_model_catalog_path(base_dir);
    let mut catalog_changed = false;
    let mut catalog = None;
    if catalog_path.is_file() {
        let content = fs::read_to_string(&catalog_path).map_err(|error| {
            format!(
                "读取 Codex 受管模型目录失败: path={}, error={}",
                catalog_path.display(),
                error
            )
        })?;
        let mut parsed = serde_json::from_str::<serde_json::Value>(&content).map_err(|error| {
            format!(
                "解析 Codex 受管模型目录失败: path={}, error={}",
                catalog_path.display(),
                error
            )
        })?;
        if let Some(models) = parsed.get_mut("models").and_then(serde_json::Value::as_array_mut) {
            for model in models {
                let model_id = model
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if let Some(display_name) = model
                    .get("display_name")
                    .and_then(|value| prefixed_builtin_display_name(&model_id, value))
                {
                    model["display_name"] = serde_json::Value::String(display_name.to_string());
                    catalog_changed = true;
                }
                if let Some(description) = model
                    .get("description")
                    .and_then(|value| prefixed_builtin_display_name(&model_id, value))
                {
                    model["description"] = serde_json::Value::String(description.to_string());
                    catalog_changed = true;
                }
            }
        }
        catalog = Some(parsed);
    }

    if catalog_changed {
        let mut content = serde_json::to_string_pretty(
            catalog
                .as_ref()
                .expect("catalog is present when marked changed"),
        )
        .map_err(|error| format!("序列化 Codex 受管模型目录失败: {}", error))?;
        content.push('\n');
        write_string_atomic(&catalog_path, &content).map_err(|error| {
            format!(
                "写入 Codex 受管模型目录迁移失败: path={}, error={}",
                catalog_path.display(),
                error
            )
        })?;
        if let Err(error) = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir)
        {
            logger::log_warn(&format!(
                "[Codex实验模型] 清理模型目录缓存失败，继续完成显示名称迁移: {}",
                error
            ));
        }
    }

    if cached_config_needs_write {
        let mut content = serde_json::to_string_pretty(
            cached_config
                .as_ref()
                .expect("cached config is present when marked changed"),
        )
        .map_err(|error| format!("序列化 Codex 模型配置缓存失败: {}", error))?;
        content.push('\n');
        write_string_atomic(&config_path, &content).map_err(|error| {
            format!(
                "写入 Codex 模型配置缓存迁移失败: path={}, error={}",
                config_path.display(),
                error
            )
        })?;
    }

    Ok(catalog_changed || cached_config_needs_write)
}

/// 把 GPT-6 家族按官方推荐顺序（astra → sol → luna）排到清单最前面。
///
/// 官方客户端的推荐集把这三个模型放在 5.6 系列之前；缺失的条目会被跳过，
/// 不会凭空插入用户清单里不存在的模型。
fn prioritize_gpt_6_model_definitions(
    mut models: Vec<CodexExperimentalModelDefinition>,
) -> Vec<CodexExperimentalModelDefinition> {
    for (target_index, model_id) in [
        GPT_6_ASTRA_MODEL_ID,
        GPT_6_SOL_MODEL_ID,
        GPT_6_LUNA_MODEL_ID,
    ]
    .into_iter()
    .enumerate()
    {
        let Some(index) = models
            .iter()
            .position(|model| model.model_id.eq_ignore_ascii_case(model_id))
        else {
            continue;
        };
        if index > target_index {
            let model = models.remove(index);
            models.insert(target_index, model);
        }
    }
    models
}

fn astra_default_repair_is_needed(
    base_dir: &Path,
    configured_model: Option<&str>,
    models: &[CodexExperimentalModelDefinition],
) -> bool {
    if experimental_model_catalog_has_migration(
        base_dir,
        GPT_6_ASTRA_DEFAULT_REPAIR_MIGRATION_ID,
    ) || !models
        .iter()
        .any(|model| model.model_id.eq_ignore_ascii_case(DEFAULT_CODEX_MODEL_ID))
    {
        return false;
    }

    let explicit_default = read_experimental_model_catalog_config(base_dir)
        .and_then(|config| config.default_model_id)
        .filter(|model_id| !model_id.trim().is_empty());
    explicit_default
        .as_deref()
        .is_some_and(|model_id| model_id.eq_ignore_ascii_case(GPT_6_ASTRA_MODEL_ID))
        || explicit_default.is_none()
            && configured_model
                .is_some_and(|model_id| model_id.eq_ignore_ascii_case(GPT_6_ASTRA_MODEL_ID))
}

fn read_experimental_model_default_model_id(base_dir: &Path) -> Option<String> {
    let config = read_experimental_model_catalog_config(base_dir)?;
    if config.version < EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION {
        return None;
    }
    let default_model_id = config.default_model_id?.trim().to_string();
    if default_model_id.is_empty() {
        return None;
    }
    let model_id = config
        .models
        .iter()
        .find(|model| {
            model
                .model_id
                .trim()
                .eq_ignore_ascii_case(&default_model_id)
        })
        .map(|model| model.model_id.trim().to_string())?;
    if model_id.eq_ignore_ascii_case(GPT_6_ASTRA_MODEL_ID)
        && astra_default_repair_is_needed(base_dir, None, &config.models)
    {
        return config
            .models
            .iter()
            .find(|model| model.model_id.eq_ignore_ascii_case(DEFAULT_CODEX_MODEL_ID))
            .map(|model| model.model_id.trim().to_string());
    }
    Some(model_id)
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
                    // 默认清单不再提供 5.5 之前的官方模型；用户手动添加不受此限制。
                    if crate::modules::codex_wakeup::is_codex_model_before_5_5(model_id) {
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

/// Add newly shipped models to an unchanged, previously generated default list.
/// Explicitly curated lists (including lists where the user removed a shipped model)
/// remain untouched so account switching does not override user visibility choices.
fn maybe_add_gpt_6_astra_to_previous_shipped_model_definitions(
    base_dir: &Path,
    mut models: Vec<CodexExperimentalModelDefinition>,
) -> Vec<CodexExperimentalModelDefinition> {
    if models
        .iter()
        .any(|model| model.model_id.eq_ignore_ascii_case(GPT_6_ASTRA_MODEL_ID))
    {
        return prioritize_gpt_6_model_definitions(models);
    }

    let existing_ids = models
        .iter()
        .map(|model| model.model_id.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if !PRE_ASTRA_SHIPPED_VISIBLE_CODEX_MODEL_IDS
        .iter()
        // 默认清单已不再包含 5.5 之前的模型，迁移判定也只要求当前仍在售的条目存在。
        .filter(|model_id| !crate::modules::codex_wakeup::is_codex_model_before_5_5(model_id))
        .all(|model_id| existing_ids.contains(&model_id.to_ascii_lowercase()))
    {
        return models;
    }

    if let Some(astra) = default_experimental_model_definitions(base_dir)
        .into_iter()
        .find(|model| model.model_id.eq_ignore_ascii_case(GPT_6_ASTRA_MODEL_ID))
    {
        models.insert(0, astra);
    }
    prioritize_gpt_6_model_definitions(models)
}

/// 把 `gpt-6-sol` / `gpt-6-luna` 补进上一版自动生成的清单。
///
/// 与 astra 的补齐规则一致：只有清单仍与上一版随包发布的自动清单一致时才补齐，
/// 用户自己增删过的精修清单保持原样（`gpt-6-sol` 与 `gpt-6-luna` 只存在其中一个时
/// 同样视为用户已接管，不再自动改动）。
fn maybe_add_gpt_6_sol_luna_to_previous_shipped_model_definitions(
    base_dir: &Path,
    mut models: Vec<CodexExperimentalModelDefinition>,
) -> Vec<CodexExperimentalModelDefinition> {
    let has_sol = models
        .iter()
        .any(|model| model.model_id.eq_ignore_ascii_case(GPT_6_SOL_MODEL_ID));
    let has_luna = models
        .iter()
        .any(|model| model.model_id.eq_ignore_ascii_case(GPT_6_LUNA_MODEL_ID));
    if has_sol || has_luna {
        return prioritize_gpt_6_model_definitions(models);
    }

    let existing_ids = models
        .iter()
        .map(|model| model.model_id.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if !PRE_GPT_6_SOL_LUNA_SHIPPED_VISIBLE_CODEX_MODEL_IDS
        .iter()
        .filter(|model_id| !crate::modules::codex_wakeup::is_codex_model_before_5_5(model_id))
        .all(|model_id| existing_ids.contains(&model_id.to_ascii_lowercase()))
    {
        return models;
    }

    let defaults = default_experimental_model_definitions(base_dir);
    let mut additions = [GPT_6_SOL_MODEL_ID, GPT_6_LUNA_MODEL_ID]
        .into_iter()
        .filter_map(|model_id| {
            defaults
                .iter()
                .find(|model| model.model_id.eq_ignore_ascii_case(model_id))
                .cloned()
        })
        .collect::<Vec<_>>();
    if additions.is_empty() {
        return models;
    }
    let insert_at = models
        .iter()
        .position(|model| model.model_id.eq_ignore_ascii_case(GPT_6_ASTRA_MODEL_ID))
        .map(|index| index + 1)
        .unwrap_or(0);
    for (offset, model) in additions.drain(..).enumerate() {
        models.insert(insert_at + offset, model);
    }
    prioritize_gpt_6_model_definitions(models)
}

fn model_catalog_display_name(model_id: &str, fallback: &str) -> String {
    match model_id.trim().to_ascii_lowercase().as_str() {
        "gpt-5.6-sol" => "GPT-5.6 Sol".to_string(),
        "gpt-5.6-terra" => "GPT-5.6 Terra".to_string(),
        "gpt-5.6-luna" => "GPT-5.6 Luna".to_string(),
        GPT_6_ASTRA_MODEL_ID => "GPT-6 Astra".to_string(),
        GPT_6_SOL_MODEL_ID => "GPT-6 Sol".to_string(),
        GPT_6_LUNA_MODEL_ID => "GPT-6 Luna".to_string(),
        "gpt-5.3-codex" => "GPT-5.3 Codex".to_string(),
        "gpt-5.5" => "GPT-5.5".to_string(),
        "gpt-5.4" => "GPT-5.4".to_string(),
        "gpt-5.4-mini" => "GPT-5.4 Mini".to_string(),
        "gpt-5.3-codex-spark" => "GPT-5.3 Codex Spark".to_string(),
        "gpt-5.6-sol-wm" => "GPT-5.6 Sol WM".to_string(),
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

pub(crate) fn normalize_experimental_model_definitions(
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
        if model.context_window.is_some_and(|value| value <= 0)
            || (model.context_window.is_none() && model.auto_compact_token_limit.is_some())
        {
            return Err("EXPERIMENTAL_MODEL_CATALOG_CONTEXT_WINDOW_INVALID".to_string());
        }
        if model.auto_compact_token_limit.is_some_and(|value| value <= 0) {
            return Err("EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_INVALID".to_string());
        }
        // 统一口径：给了上下文窗口就必须带压缩阈值；调用方只给窗口时按 90% 派生，
        // 但显式给出的阈值必须严格小于窗口。
        if let (Some(window), Some(compact)) =
            (model.context_window, model.auto_compact_token_limit)
        {
            if compact >= window {
                return Err("EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_RANGE_INVALID".to_string());
            }
        }
        normalized.push(CodexExperimentalModelDefinition {
            model_id: model_id.to_string(),
            display_name: display_name.to_string(),
            reasoning_efforts: normalize_reasoning_efforts(model.reasoning_efforts.clone())?,
            context_window: model.context_window,
            auto_compact_token_limit: model.context_window.map(|window| {
                model
                    .auto_compact_token_limit
                    .unwrap_or_else(|| {
                        crate::modules::codex_protocol::derived_auto_compact_token_limit(window)
                    })
            }),
        });
    }
    Ok(normalized)
}

pub(crate) fn read_experimental_model_definitions(
    base_dir: &Path,
) -> Vec<CodexExperimentalModelDefinition> {
    let path = experimental_model_config_path(base_dir);
    let Ok(content) = fs::read_to_string(&path) else {
        return default_experimental_model_definitions(base_dir);
    };
    let mut models = match serde_json::from_str::<ExperimentalModelCatalogConfig>(&content)
        .map_err(|error| error.to_string())
        .and_then(|config| {
            let requires_catalog_migration =
                config.version < EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION;
            let should_add_astra = config.version == EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION
                && !config
                    .migrations
                    .iter()
                    .any(|item| item == GPT_6_ASTRA_MODEL_CATALOG_MIGRATION_ID);
            let should_add_sol_luna = config.version == EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION
                && !config
                    .migrations
                    .iter()
                    .any(|item| item == GPT_6_SOL_LUNA_MODEL_CATALOG_MIGRATION_ID);
            normalize_experimental_model_definitions(config.models).map(|models| {
                (
                    models,
                    requires_catalog_migration,
                    should_add_astra,
                    should_add_sol_luna,
                )
            })
        })
    {
        Ok((_models, true, _, _)) => {
            // A release migration intentionally resets all pre-release lists to the
            // shipped visible-model preset. Later user edits are preserved by version 4+
            // and the additive Astra / GPT-6 Sol-Luna migration markers.
            default_experimental_model_definitions(base_dir)
        }
        Ok((models, false, should_add_astra, should_add_sol_luna)) => {
            let models = if should_add_astra {
                maybe_add_gpt_6_astra_to_previous_shipped_model_definitions(base_dir, models)
            } else {
                prioritize_gpt_6_model_definitions(models)
            };
            if should_add_sol_luna {
                maybe_add_gpt_6_sol_luna_to_previous_shipped_model_definitions(base_dir, models)
            } else {
                prioritize_gpt_6_model_definitions(models)
            }
        }
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex实验模型] 模型配置无效，使用默认值: path={}, error={}",
                path.display(),
                error
            ));
            default_experimental_model_definitions(base_dir)
        }
    };
    if !models.iter().any(|model| model.model_id.eq_ignore_ascii_case("gpt-reserve")) {
        models.push(CodexExperimentalModelDefinition {
            model_id: "gpt-reserve".to_string(),
            display_name: crate::modules::codex_protocol::CODEX_RESERVE_DISPLAY_NAME.to_string(),
            reasoning_efforts: None,
            context_window: None,
            auto_compact_token_limit: None,
        });
    }
    models
}

fn persist_experimental_model_definitions(
    base_dir: &Path,
    models: Vec<CodexExperimentalModelDefinition>,
    default_model_id: Option<&str>,
) -> Result<Vec<CodexExperimentalModelDefinition>, String> {
    let previous_config = read_experimental_model_catalog_config(base_dir);
    let previous_default_was_astra = previous_config
        .as_ref()
        .and_then(|config| config.default_model_id.as_deref())
        .is_some_and(|model_id| model_id.eq_ignore_ascii_case(GPT_6_ASTRA_MODEL_ID));
    let models = prioritize_gpt_6_model_definitions(normalize_experimental_model_definitions(
        models,
    )?);
    let mut default_model_id = default_model_id.and_then(|value| {
        models
            .iter()
            .find(|model| model.model_id.eq_ignore_ascii_case(value.trim()))
            .map(|model| model.model_id.clone())
    });
    let mut migrations = previous_config
        .map(|config| config.migrations)
        .unwrap_or_default();
    let requested_astra = default_model_id
        .as_deref()
        .is_some_and(|model_id| model_id.eq_ignore_ascii_case(GPT_6_ASTRA_MODEL_ID));
    let default_repair_pending = !migrations
        .iter()
        .any(|item| item == GPT_6_ASTRA_DEFAULT_REPAIR_MIGRATION_ID);
    if requested_astra
        && previous_default_was_astra
        && default_repair_pending
        && models
            .iter()
            .any(|model| model.model_id.eq_ignore_ascii_case(DEFAULT_CODEX_MODEL_ID))
    {
        default_model_id = Some(DEFAULT_CODEX_MODEL_ID.to_string());
    }
    if default_model_id.is_some() && default_repair_pending {
        migrations.push(GPT_6_ASTRA_DEFAULT_REPAIR_MIGRATION_ID.to_string());
    }
    if !migrations
        .iter()
        .any(|item| item == GPT_6_ASTRA_MODEL_CATALOG_MIGRATION_ID)
    {
        migrations.push(GPT_6_ASTRA_MODEL_CATALOG_MIGRATION_ID.to_string());
    }
    if !migrations
        .iter()
        .any(|item| item == GPT_6_SOL_LUNA_MODEL_CATALOG_MIGRATION_ID)
    {
        migrations.push(GPT_6_SOL_LUNA_MODEL_CATALOG_MIGRATION_ID.to_string());
    }
    let mut content = serde_json::to_string_pretty(&ExperimentalModelCatalogConfig {
        version: EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION,
        models: models.clone(),
        default_model_id,
        migrations,
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

fn apply_model_context_config_to_catalog(
    catalog: &mut serde_json::Value,
    models: &[CodexExperimentalModelDefinition],
) {
    let definitions = models
        .iter()
        .map(|model| (
            model.model_id.clone(),
            model.context_window,
            model.auto_compact_token_limit,
        ))
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
    // 统一口径收口：即使没有任何逐模型覆盖，也要保证受管目录里每个声明了上下文窗口的
    // 模型都带自动压缩阈值（缺失时按 90% 派生）。
    crate::modules::codex_protocol::ensure_client_model_auto_compact_limits(&mut catalog);
    serde_json::to_string_pretty(&catalog)
        .map_err(|error| format!("序列化 Codex 受管模型目录失败: {}", error))
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
    crate::modules::codex_protocol::ensure_codex_reserve_fallback(&mut catalog);
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
            let context = model.get("context_window").and_then(serde_json::Value::as_i64);
            let compact = model.get("auto_compact_token_limit").and_then(serde_json::Value::as_i64);
            let official = crate::modules::codex_protocol::build_codex_client_models_response(
                &[model_id.to_string()],
            );
            // 统一口径：外部目录只要声明了有效上下文窗口就采纳，压缩阈值缺失时按 90%
            // 派生，不再因为「没有阈值」把整对覆盖丢掉。
            let (context_window, auto_compact_token_limit) = match context.filter(|value| *value > 0)
            {
                Some(window) => {
                    let limit = compact
                        .filter(|value| *value > 0 && *value < window)
                        .unwrap_or_else(|| {
                            crate::modules::codex_protocol::derived_auto_compact_token_limit(window)
                        });
                    if official["models"][0]["context_window"].as_i64() == Some(window)
                        && official["models"][0]["auto_compact_token_limit"].as_i64() == Some(limit)
                    {
                        (None, None)
                    } else {
                        (Some(window), Some(limit))
                    }
                }
                None => (None, None),
            };
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
            let previous_state = read_previous_experimental_catalog_state(base_dir);
            if managed_catalog_configured {
                match previous_state.as_ref() {
                    Some(state) if state.had_catalog_reference => {
                        if let Some(previous_catalog) = state.catalog_reference.as_deref() {
                            doc[CODEX_CONFIG_MODEL_CATALOG_JSON_KEY] = value(previous_catalog);
                        }
                    }
                    _ => {
                        let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
                    }
                }
            }
            if let Some(state) = previous_state {
                if state.had_model {
                    if let Some(previous_model) = state.model.as_deref() {
                        doc["model"] = value(previous_model);
                    }
                } else {
                    let _ = doc.remove("model");
                }
            }
        }
        return Ok(currently_enabled || policy_enabled);
    }

    let has_saved_model_definitions = experimental_model_config_path(base_dir).is_file();
    let migrate_saved_model_definitions = has_saved_model_definitions
        && experimental_model_config_requires_catalog_migration(base_dir);
    let mut experimental_models =
        crate::modules::codex_local_access::overlay_rendered_pool_models_on_experimental_catalog(
            base_dir,
            read_experimental_model_definitions(base_dir),
        );
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
    let configured_model = doc.get("model").and_then(|item| item.as_str());
    if astra_default_repair_is_needed(base_dir, configured_model, &experimental_models) {
        experimental_models = persist_experimental_model_definitions(
            base_dir,
            experimental_models,
            Some(DEFAULT_CODEX_MODEL_ID),
        )?;
        doc["model"] = value(DEFAULT_CODEX_MODEL_ID);
    }
    let generated_content = build_experimental_model_catalog(base_dir)
        .map_err(|_| "EXPERIMENTAL_MODEL_CATALOG_SERIALIZE_FAILED".to_string())?;
    if read_previous_experimental_catalog_state(base_dir).is_none() {
        let previous_model = doc.get("model").and_then(|item| item.as_str());
        persist_previous_experimental_catalog_reference(
            base_dir,
            user_catalog_reference,
            previous_model,
        )?;
    }
    if let Some(default_model_id) = read_experimental_model_default_model_id(base_dir) {
        if experimental_models
            .iter()
            .any(|model| model.model_id.eq_ignore_ascii_case(&default_model_id))
        {
            doc["model"] = value(default_model_id);
        }
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

/// 一次性迁移标记：本版本把历史遗留的「模型管理」统一关闭，之后由用户自己决定。
const CODEX_MODEL_MANAGEMENT_DEFAULT_OFF_MARKER_FILE: &str =
    ".cockpit-model-management-default-off-v1";

/// 一次性关闭历史遗留的「模型管理」，恢复跟随官方模型目录。
///
/// 只在首次执行时生效（成功后写入标记文件），用户之后自己再开启模型管理不再被干预；
/// 已保存的模型清单文件会保留，用户重新开启时仍能看到自己的清单。
pub fn migrate_model_management_default_off_once(base_dir: &Path) -> Result<bool, String> {
    if base_dir.as_os_str().is_empty() {
        return Ok(false);
    }
    let marker_path = base_dir.join(CODEX_MODEL_MANAGEMENT_DEFAULT_OFF_MARKER_FILE);
    if marker_path.is_file() {
        return Ok(false);
    }

    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|error| format!("解析 config.toml 失败: {}", error))?
    };
    let policy_enabled = experimental_model_policy_enabled(base_dir);
    let managed_catalog_configured = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .is_some_and(|catalog| catalog_ref_targets_cockpit_managed_file(catalog, base_dir));

    if policy_enabled {
        apply_experimental_model_catalog_to_doc(base_dir, &mut doc, Some(false))?;
    } else if managed_catalog_configured {
        let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
    }
    if policy_enabled || managed_catalog_configured {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建 config.toml 目录失败: {}", error))?;
        }
        let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
            .map_err(|error| format!("写入 config.toml 失败: {}", error))?;
    }

    let managed_catalog_path = experimental_model_catalog_path(base_dir);
    if managed_catalog_path.exists() {
        crate::modules::atomic_write::remove_file_locked(&managed_catalog_path).map_err(|error| {
            format!(
                "清理 Codex 受管模型目录失败: path={}, error={}",
                managed_catalog_path.display(),
                error
            )
        })?;
    }
    cleanup_legacy_managed_model_catalogs(base_dir);
    let _ = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir);
    persist_experimental_model_policy(base_dir, false)?;
    clear_previous_experimental_catalog_reference(base_dir)?;

    write_string_atomic(&marker_path, "disabled\n")
        .map_err(|error| format!("写入模型管理默认关闭标记失败: {}", error))?;
    Ok(true)
}

/// 未运行 Codex 的 profile 目录（默认实例 + 托管实例）。
///
/// 正在运行的实例跳过迁移：配置在下次启动时才会生效，避免影响当前会话。
fn idle_codex_profile_dirs_for_model_management_migration() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Ok(settings) = crate::modules::codex_instance::load_default_settings() {
        if let Ok(default_dir) = crate::modules::codex_instance::get_default_codex_home() {
            let running =
                crate::modules::process::resolve_codex_pid(settings.last_pid, None).is_some();
            if !running && seen.insert(default_dir.to_string_lossy().to_string()) {
                dirs.push(default_dir);
            }
        }
    }
    if let Ok(store) = crate::modules::codex_instance::load_instance_store() {
        for instance in store.instances {
            let dir = PathBuf::from(&instance.user_data_dir);
            if dir.as_os_str().is_empty() {
                continue;
            }
            let running = crate::modules::process::resolve_codex_pid(
                instance.last_pid,
                Some(instance.user_data_dir.as_str()),
            )
            .is_some();
            if running || !seen.insert(dir.to_string_lossy().to_string()) {
                continue;
            }
            dirs.push(dir);
        }
    }
    dirs
}

/// 对默认实例和全部托管实例执行一次「模型管理默认关闭」迁移，返回实际迁移的 profile 数量。
pub fn migrate_model_management_default_off_for_all_profiles() -> usize {
    let mut migrated = 0usize;
    for profile_dir in idle_codex_profile_dirs_for_model_management_migration() {
        match migrate_model_management_default_off_once(&profile_dir) {
            Ok(true) => migrated += 1,
            Ok(false) => {}
            Err(error) => logger::log_warn(&format!(
                "[Codex模型目录] 关闭历史模型管理失败: profile={}, error={}",
                profile_dir.display(),
                error
            )),
        }
    }
    migrated
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
    if let Err(error) = migrate_builtin_model_display_names(base_dir, &doc) {
        logger::log_warn(&format!(
            "[Codex实验模型] 迁移 GPT-6 Astra 显示名称失败，继续读取现有配置: {}",
            error
        ));
    }
    let detected_model_context_window =
        read_top_level_int_from_doc(&doc, CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY);
    let detected_auto_compact_token_limit =
        read_top_level_int_from_doc(&doc, CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY)
            .filter(|value| *value > 0);
    let context_management_experimental_mode =
        read_context_management_experimental_mode_from_doc(&doc);
    let experimental = inspect_experimental_model_catalog(base_dir, &doc)?;
    let saved_model_definitions = read_experimental_model_definitions(base_dir);
    let experimental_models = if has_saved_experimental_model_definitions(base_dir) {
        // 用户已接管模型清单：只读取用户定义，不再叠加账号池模型或移除官方条目。
        saved_model_definitions
    } else {
        crate::modules::codex_local_access::overlay_rendered_pool_models_on_experimental_catalog(
            base_dir,
            saved_model_definitions,
        )
    };
    let experimental_default_model_id =
        read_experimental_model_default_model_id(base_dir).or_else(|| {
            if !experimental.enabled {
                return None;
            }
            let configured_model = doc.get("model").and_then(|item| item.as_str())?.trim();
            if astra_default_repair_is_needed(base_dir, Some(configured_model), &experimental_models)
            {
                return experimental_models
                    .iter()
                    .find(|model| model.model_id.eq_ignore_ascii_case(DEFAULT_CODEX_MODEL_ID))
                    .map(|model| model.model_id.clone());
            }
            experimental_models
                .iter()
                .find(|model| model.model_id.eq_ignore_ascii_case(configured_model))
                .map(|model| model.model_id.clone())
        });
    let experimental_default_model_id = experimental_default_model_id.and_then(|model_id| {
        experimental_models
            .iter()
            .find(|model| model.model_id.eq_ignore_ascii_case(&model_id))
            .map(|model| model.model_id.clone())
    }).or_else(|| {
        experimental.enabled.then(|| experimental_models.first().map(|model| model.model_id.clone())).flatten()
    });
    let experimental_reset_models =
        crate::modules::codex_local_access::overlay_rendered_pool_models_on_experimental_catalog(
            base_dir,
            default_experimental_model_definitions(base_dir),
        );
    let experimental_reset_default_model_id = experimental_reset_models
        .iter()
        .find(|model| model.model_id.eq_ignore_ascii_case(DEFAULT_CODEX_MODEL_ID))
        .or_else(|| experimental_reset_models.first())
        .map(|model| model.model_id.clone());

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
        experimental_model_catalog_reset_models: experimental_reset_models,
        experimental_model_catalog_reset_default_model_id: experimental_reset_default_model_id,
        context_management_experimental_mode,
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
    write_quick_config_to_config_toml_with_default_mode(
        base_dir,
        model_context_window,
        auto_compact_token_limit,
        experimental_model_catalog_enabled,
        experimental_model_catalog_models,
        experimental_model_catalog_default_model_id,
        false,
    )
}

fn write_model_catalog_to_config_toml_preserving_context(
    base_dir: &Path,
    experimental_model_catalog_enabled: bool,
    experimental_model_catalog_models: Vec<CodexExperimentalModelDefinition>,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<CodexQuickConfig, String> {
    write_quick_config_to_config_toml_with_default_mode(
        base_dir,
        None,
        None,
        Some(experimental_model_catalog_enabled),
        Some(experimental_model_catalog_models),
        experimental_model_catalog_default_model_id,
        true,
    )
}

fn write_quick_config_to_config_toml_with_default_mode(
    base_dir: &Path,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    experimental_model_catalog_enabled: Option<bool>,
    experimental_model_catalog_models: Option<Vec<CodexExperimentalModelDefinition>>,
    experimental_model_catalog_default_model_id: Option<String>,
    preserve_context: bool,
) -> Result<CodexQuickConfig, String> {
    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let disabling_experimental_catalog = experimental_model_catalog_enabled == Some(false);
    let had_experimental_model_control_state = experimental_model_policy_path(base_dir).is_file()
        || experimental_model_config_path(base_dir).is_file()
        || experimental_model_previous_catalog_path(base_dir).is_file();

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
    // Disabling the Cockpit catalog must not migrate or retain a legacy reference as part of
    // this save. The official client must use its own catalog and account-level visibility rules;
    // any user-owned catalog file is left untouched on disk, but is no longer activated here.
    let migrated_legacy_catalog = if disabling_experimental_catalog {
        false
    } else {
        migrate_legacy_managed_catalog_reference(base_dir, &mut doc)?
    };

    if !preserve_context {
        if let Some(context_window) = model_context_window {
            if context_window <= 0 {
                return Err("上下文窗口必须大于 0".to_string());
            }
            // 统一口径：写了上下文窗口就必须同时写自动压缩阈值；调用方没给阈值时按
            // 90% 派生，避免 config.toml 里出现「有窗口无阈值」的半边配置。
            let compact_limit = auto_compact_token_limit
                .filter(|value| *value > 0)
                .unwrap_or_else(|| {
                    crate::modules::codex_protocol::derived_auto_compact_token_limit(context_window)
                });
            if compact_limit >= context_window {
                return Err("自动压缩阈值必须小于上下文窗口".to_string());
            }
            doc[CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY] = value(context_window);
            doc[CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY] = value(compact_limit);
        } else {
            // 跟随官方：两个键必须一起移除，不允许留下孤立的压缩阈值。
            let _ = doc.remove(CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY);
            let _ = doc.remove(CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY);
        }
    }

    // A disabled catalog is an opt-out from Cockpit model control. In particular, do not
    // validate or persist a stale editor draft that the frontend sends together with the
    // unchecked switch; the state is removed below instead.
    if let Some(models) = experimental_model_catalog_models
        .filter(|_| !disabling_experimental_catalog)
    {
        persist_experimental_model_definitions(
            base_dir,
            crate::modules::codex_local_access::overlay_rendered_pool_models_on_experimental_catalog(
                base_dir,
                models,
            ),
            experimental_model_catalog_default_model_id.as_deref(),
        )?;
        // 用户通过 UI 保存了清单：从这里开始系统不再自动增删条目。
        write_string_atomic(&user_customized_model_catalog_marker_path(base_dir), "customized\n")
            .map_err(|error| format!("写入模型清单自定义标记失败: {}", error))?;
    }

    let effective_experimental_enabled = experimental_model_catalog_enabled
        .or_else(|| experimental_model_policy_enabled(base_dir).then_some(true));
    let remove_experimental_catalog_after_write = apply_experimental_model_catalog_to_doc(
        base_dir,
        &mut doc,
        effective_experimental_enabled,
    )?;
    if disabling_experimental_catalog {
        // Explicitly disabling this feature is the user's request to return to the official
        // catalog. Remove both Cockpit-managed and user-supplied model_catalog_json references;
        // the referenced user file is intentionally preserved for manual reuse later.
        let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
    }

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

    let remove_managed_catalog_after_write = remove_experimental_catalog_after_write
        || (disabling_experimental_catalog && had_experimental_model_control_state);
    if remove_managed_catalog_after_write {
        crate::modules::atomic_write::remove_file_locked(&experimental_model_catalog_path(base_dir))
            .map_err(|error| {
                format!(
                    "配置已停用，但清理 Codex 受管模型目录失败: profile={}, error={}",
                    base_dir.display(),
                    error
                )
            })?;
        let _ = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir);
    }
    if disabling_experimental_catalog {
        cleanup_legacy_managed_model_catalogs(base_dir);
        clear_previous_experimental_catalog_reference(base_dir)?;
        clear_experimental_model_catalog_config(base_dir)?;
        let _ = fs::remove_file(user_customized_model_catalog_marker_path(base_dir));
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

pub fn save_current_model_catalog_preserving_context(
    experimental_model_catalog_enabled: bool,
    experimental_model_catalog_models: Vec<CodexExperimentalModelDefinition>,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<CodexQuickConfig, String> {
    save_model_catalog_for_base_dir_preserving_context(
        &get_codex_home(),
        experimental_model_catalog_enabled,
        experimental_model_catalog_models,
        experimental_model_catalog_default_model_id,
    )
}

pub fn save_model_catalog_for_base_dir_preserving_context(
    base_dir: &Path,
    experimental_model_catalog_enabled: bool,
    experimental_model_catalog_models: Vec<CodexExperimentalModelDefinition>,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<CodexQuickConfig, String> {
    write_model_catalog_to_config_toml_preserving_context(
        base_dir,
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

/// 本次账号投影应写的官方登录方式。以写入 auth.json 的凭据归属为准：
/// API Key 账号写 `api`，OAuth（含 API Key 账号绑定 OAuth）写 `chatgpt`。
fn forced_login_method_for_account(account: &CodexAccount) -> &'static str {
    if account.is_api_key_auth() {
        CODEX_FORCED_LOGIN_METHOD_API
    } else {
        CODEX_FORCED_LOGIN_METHOD_CHATGPT
    }
}

/// 按本次切号写入的凭据类型对齐官方 `forced_login_method`。
///
/// 官方客户端在未设置该键时按 auth.json 自动识别登录方式；但该键一旦被用户手工设置，
/// 语义就是「限制登录方式」——键值与本次投影的凭据不一致时，客户端会把有效凭据判定为
/// 未登录，切号结果直接不可用。
///
/// 因此这里只修复「用户已显式设置、且与本次凭据冲突」的情况：未设置该键时官方本来就能
/// 自动选择登录方式，工具不为用户新增锁定，也不因此创建 config.toml；键值一致时不重复
/// 落盘。失败原样上报，由切号整体失败处理，避免留下半同步状态。
fn apply_forced_login_method_to_config_toml(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<bool, String> {
    let target = forced_login_method_for_account(account);
    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    if existing.trim().is_empty() {
        return Ok(false);
    }
    let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
        .map_err(|e| format!("解析 config.toml 失败: {}", e))?;

    let current = doc
        .get(CODEX_CONFIG_FORCED_LOGIN_METHOD_KEY)
        .and_then(|item| item.as_str())
        .map(|value| value.trim().to_ascii_lowercase());
    let Some(current) = current else {
        return Ok(false);
    };
    if current == target {
        return Ok(false);
    }

    doc[CODEX_CONFIG_FORCED_LOGIN_METHOD_KEY] = value(target);
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
    Ok(true)
}

/// profile 级「工具写入的 provider 引用」快照文件名。
///
/// 切到第三方/自建供应商账号时，工具会把 `model_provider` 与 `[model_providers.<id>]` 写进官方
/// profile 的 config.toml（含 DeepSeek、自建中转等）。切回官方内置 provider 时，官方账号投影只
/// 清理受管 provider（`openai` / `codex_local_access` / `cockpit_api` / `openai_api_key`），其余 id
/// 会被当作用户自定义 provider 保留——于是「账号已切回官方，客户端仍按上一个供应商发请求」
/// （例如切回普通账号后一直显示 DeepSeek）。
///
/// 因此写入前按 profile 记录原值，切回官方内置 provider 时按同一份记录还原；这是 profile 级
/// 状态，多开实例天然隔离，也不需要按供应商 id 写特判。
const CODEX_PROVIDER_OVERRIDE_FILE: &str = "cockpit-provider-override.json";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexProviderOverrideSnapshot {
    provider_id: String,
    /// 写入前 `model` 的原值；`None` 表示原本没有这个键。
    #[serde(default)]
    model: Option<String>,
    /// 写入前 `model_provider` 的原值；`None` 表示原本没有这个键。
    #[serde(default)]
    model_provider: Option<String>,
    /// 写入前 `[model_providers.<id>]` 的 TOML 片段；`None` 表示原本没有该表项。
    #[serde(default)]
    provider_table: Option<String>,
}

fn provider_override_snapshot_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_PROVIDER_OVERRIDE_FILE)
}

fn read_top_level_config_string(doc: &Document, key: &str) -> Option<String> {
    doc.get(key).and_then(|item| item.as_str()).map(str::to_string)
}

/// 写入工具 provider 之前记录原值。已有快照时不覆盖：一份快照对应一次「工具接管」，
/// 必须保留最早的用户原值，否则切走时只会还原成上一次的接管状态。
fn record_provider_override_snapshot(doc: &Document, base_dir: &Path, provider_id: &str) {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return;
    }
    let path = provider_override_snapshot_path(base_dir);
    if path.exists() {
        return;
    }
    let snapshot = CodexProviderOverrideSnapshot {
        provider_id: provider_id.to_string(),
        model: read_top_level_config_string(doc, CODEX_CONFIG_MODEL_KEY),
        model_provider: read_top_level_config_string(doc, CODEX_CONFIG_MODEL_PROVIDER_KEY),
        provider_table: doc
            .get(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
            .and_then(|item| item.get(provider_id))
            .map(ToString::to_string),
    };
    let Ok(content) = serde_json::to_string_pretty(&snapshot) else {
        return;
    };
    if let Err(error) = crate::modules::atomic_write::write_string_atomic(&path, &content) {
        logger::log_warn(&format!(
            "[Codex切号] 记录 provider 引用快照失败: path={}, error={}",
            path.display(),
            error
        ));
    }
}

/// 还原 `[model_providers.<id>]`：原本存在就写回原内容，原本不存在就移除工具写入的表项。
fn restore_provider_table_entry(doc: &mut Document, provider_id: &str, snapshot: Option<&str>) {
    let previous = snapshot.and_then(|raw| {
        let escaped = provider_id.replace('\\', "\\\\").replace('"', "\\\"");
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&format!(
            "[model_providers.\"{escaped}\"]\n{raw}"
        ))
        .ok()
        .and_then(|parsed| {
            parsed
                .get(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
                .and_then(|item| item.get(provider_id))
                .cloned()
        })
    });
    let providers_empty = doc
        .get_mut(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
        .and_then(|item| item.as_table_mut())
        .map(|providers| {
            match previous {
                Some(item) => {
                    providers.insert(provider_id, item);
                }
                None => {
                    let _ = providers.remove(provider_id);
                }
            }
            providers.is_empty()
        })
        .unwrap_or(false);
    if providers_empty {
        let _ = doc.remove(CODEX_CONFIG_MODEL_PROVIDERS_KEY);
    }
}

/// 切回官方内置 provider 时按快照还原工具写入的 provider 引用。
///
/// 必须在清理当前 provider 前调用，才能识别用户在工具写入之后做出的选择。
fn restore_provider_override_snapshot(base_dir: &Path, doc: &mut Document) -> bool {
    let path = provider_override_snapshot_path(base_dir);
    let Ok(content) = fs::read_to_string(&path) else {
        return false;
    };
    // 快照只服务一次还原：无论能否应用都丢弃，避免过期记录影响后续切号。
    let _ = fs::remove_file(&path);
    let Ok(snapshot) = serde_json::from_str::<CodexProviderOverrideSnapshot>(&content) else {
        return false;
    };
    let provider_id = snapshot.provider_id.trim().to_string();
    if provider_id.is_empty() {
        return false;
    }
    // 用户可能又手动切到了别的供应商：保留用户当前选择，只丢弃过期快照。
    let current_provider = doc
        .get(CODEX_CONFIG_MODEL_PROVIDER_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim);
    if current_provider != Some(provider_id.as_str()) {
        return false;
    }

    match snapshot.model_provider.as_deref() {
        Some(previous) => doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(previous),
        None => {
            let _ = doc.remove(CODEX_CONFIG_MODEL_PROVIDER_KEY);
        }
    }
    match snapshot.model.as_deref() {
        Some(previous) => doc[CODEX_CONFIG_MODEL_KEY] = value(previous),
        None => {
            let _ = doc.remove(CODEX_CONFIG_MODEL_KEY);
        }
    }
    restore_provider_table_entry(doc, &provider_id, snapshot.provider_table.as_deref());
    true
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
            // 先根据尚未清理的当前选择还原快照；用户已改选（含移除 provider）时保留
            // 当前 provider/model。之后再统一清理受管引用，旧快照也不能复活运行时 provider。
            restore_provider_override_snapshot(base_dir, &mut doc);
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

            let _ = record_provider_override_snapshot(&doc, base_dir, provider_id);
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
    clear_previous_experimental_catalog_reference(base_dir)?;
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
    // 官方 DeepSeek 使用官方 DeepSeek 目录/工具声明，避免客户端发出上游不认的
    // GPT 专属字段（如 web_search 的 search_content_types）。
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
    // 直连上游 / CDP 注入用上游真实模型 ID；其余按官方壳位分配。
    let slots =
        crate::modules::codex_local_access::provider_model_slots_for_account(account, &upstream_models);
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
    if let Err(err) =
        crate::modules::codex_managed_model_catalog_version::write_managed_catalog_meta(
            &catalog_path,
        )
    {
        logger::log_warn(&format!(
            "[Codex模型目录] 写入版本戳失败: path={}, error={}",
            catalog_path.display(),
            err
        ));
    }
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
            return Ok(());
        }
        let _ = sync_deepseek_shell_remap_catalog_to_dir(base_dir, account)?;
        return Ok(());
    }
    let _ = cleanup_deepseek_official_model_catalog_for_dir(base_dir)?;
    if account_syncs_model_catalog_to_codex(account) {
        // 第三方 CDP 注入：模型清单由注入脚本写入官方客户端，不再写壳位目录，
        // 否则客户端会同时看到注入列表与壳位列表。
        if crate::modules::codex_account::account_uses_cdp_model_injection(account) {
            let _ = cleanup_managed_model_catalog_for_dir(base_dir)?;
            return Ok(());
        }
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
    // API Key / 第三方供应商账号始终使用自己的模型目录（供应商网关目录、DeepSeek 官方目录等）。
    // 「模型管理」只服务于订阅账号：既不能覆盖这类账号写入的目录，也不能被它们清掉，
    // 否则会出现「切到第三方账号后看不到自己的模型」以及用户模型清单被丢弃。
    if account.is_api_key_auth() {
        return sync_or_cleanup_account_model_catalog_for_dir(base_dir, account);
    }
    let preserve_experimental_policy =
        read_quick_config_from_config_toml(base_dir)?.experimental_model_catalog_enabled;
    sync_or_cleanup_account_model_catalog_for_dir(base_dir, account)?;
    if preserve_experimental_policy {
        enforce_experimental_model_policy_for_dir(base_dir)?;
    } else {
        // A disabled catalog must not survive a later account/profile switch as latent
        // Cockpit state. Provider-owned files may still be recreated by the account-specific
        // sync path above, but the experimental definitions and restore marker are always gone.
        clear_experimental_model_catalog_config(base_dir)?;
        clear_previous_experimental_catalog_reference(base_dir)?;
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
            crate::modules::codex_managed_model_catalog_version::remove_managed_catalog_meta(
                &catalog_path,
            );
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

/// 启动时按版本戳重建落后的受管模型目录。
///
/// 目录内容用「旧目录里的模型清单 + 当前生成逻辑」重建，只刷新结构/能力字段，
/// 不改变用户实际可用的模型集合；展示名沿用旧目录，避免界面名称漂移。
pub(crate) fn rebuild_stale_managed_model_catalogs() -> usize {
    let mut rebuilt = 0usize;
    for base_dir in managed_catalog_profile_dirs() {
        let catalog_path = base_dir.join(CODEX_MANAGED_MODEL_CATALOG_FILE);
        if !crate::modules::codex_managed_model_catalog_version::managed_catalog_needs_rebuild(
            &catalog_path,
        ) {
            continue;
        }
        match rebuild_managed_catalog_from_existing(&catalog_path) {
            Ok(()) => {
                rebuilt += 1;
                logger::log_info(&format!(
                    "[Codex模型目录] 已按当前版本重建模型目录: path={}",
                    catalog_path.display()
                ));
            }
            Err(err) => logger::log_warn(&format!(
                "[Codex模型目录] 重建模型目录失败: path={}, error={}",
                catalog_path.display(),
                err
            )),
        }
    }
    rebuilt
}

fn managed_catalog_profile_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![get_codex_home()];
    if let Ok(data_dir) = crate::modules::account::get_data_dir() {
        let root = data_dir.join("instances").join("codex");
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                }
            }
        }
    }
    dirs
}

fn rebuild_managed_catalog_from_existing(catalog_path: &Path) -> Result<(), String> {
    use serde_json::Value as JsonValue;

    let existing =
        fs::read_to_string(catalog_path).map_err(|e| format!("读取现有模型目录失败: {}", e))?;
    let parsed: JsonValue =
        serde_json::from_str(&existing).map_err(|e| format!("解析现有模型目录失败: {}", e))?;
    let old_models = parsed
        .get("models")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let model_ids = old_models
        .iter()
        .filter_map(|model| model.get("slug").and_then(JsonValue::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    if model_ids.is_empty() {
        return Err("现有模型目录没有可重建的模型条目".to_string());
    }
    let mut rebuilt =
        crate::modules::codex_protocol::build_codex_client_models_response(&model_ids);
    if let Some(new_models) = rebuilt.get_mut("models").and_then(JsonValue::as_array_mut) {
        for model in new_models.iter_mut() {
            let Some(slug) = model.get("slug").and_then(JsonValue::as_str).map(str::to_string)
            else {
                continue;
            };
            let Some(old) = old_models
                .iter()
                .find(|item| item.get("slug").and_then(JsonValue::as_str) == Some(slug.as_str()))
            else {
                continue;
            };
            for field in ["display_name", "description"] {
                if let Some(value) = old.get(field).filter(|value| value.is_string()) {
                    model[field] = value.clone();
                }
            }
        }
    }
    let mut content = serde_json::to_string_pretty(&rebuilt)
        .map_err(|e| format!("序列化重建后的模型目录失败: {}", e))?;
    content.push('\n');
    crate::modules::atomic_write::write_string_atomic(catalog_path, &content)?;
    crate::modules::codex_managed_model_catalog_version::write_managed_catalog_meta(catalog_path)
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
    let wire_api = account
        .api_wire_api
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(CODEX_PROVIDER_WIRE_API);
    let requires_http_only_responses_provider = account.api_provider_mode
        == CodexApiProviderMode::Custom
        && wire_api.eq_ignore_ascii_case(CODEX_PROVIDER_WIRE_API)
        && !account.api_supports_websockets;
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
    let _ = remove_leftover_deepseek_models_json(base_dir);
    let _catalog_path = write_deepseek_official_model_catalog_file(base_dir, account)?;
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

    // 记录写入前的 model / model_provider / provider 表，切回官方账号时按同一份快照还原。
    record_provider_override_snapshot(&doc, base_dir, DEEPSEEK_PROVIDER_ID);
    doc["model"] = value(selected_model.as_str());
    let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
    crate::modules::codex_account::apply_deepseek_reasoning_effort(&mut doc);
    crate::modules::codex_account::apply_deepseek_config_overrides(&mut doc, base_dir);
    doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(DEEPSEEK_PROVIDER_ID);
    doc[CODEX_CONFIG_MODEL_CATALOG_JSON_KEY] = value(CODEX_MANAGED_MODEL_CATALOG_FILE);
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
