// Codex Local Access：Provider gateway model slots, overrides and sidecar management。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn provider_gateway_api_key_id(account_id: &str) -> String {
    format!("provider_gateway_{}", account_id)
}

const MIXED_MODEL_ROUTING_RUNTIME_ID: &str = "mixed_model_routing";

fn mixed_model_routing_api_key_id() -> String {
    "mixed_model_routing".to_string()
}

fn provider_gateway_runtime_key(profile_dir: &Path, account_id: &str) -> String {
    format!(
        "{}\n{}",
        normalize_profile_dir_key(profile_dir),
        account_id.trim()
    )
}

fn provider_gateway_sidecar_dir(profile_dir: &Path, account_id: &str) -> Result<PathBuf, String> {
    let mut hasher = Sha256::new();
    hasher.update(normalize_profile_dir_key(profile_dir).as_bytes());
    hasher.update([0]);
    hasher.update(account_id.trim().as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    Ok(provider_gateway_sidecars_dir()?.join(digest))
}

fn provider_gateway_state_path(profile_dir: &Path, account_id: &str) -> Result<PathBuf, String> {
    Ok(provider_gateway_sidecar_dir(profile_dir, account_id)?
        .join(CODEX_PROVIDER_GATEWAY_STATE_FILE))
}

fn load_provider_gateway_profile_state(
    profile_dir: &Path,
    account_id: &str,
) -> Result<Option<ProviderGatewayProfileState>, String> {
    let path = provider_gateway_state_path(profile_dir, account_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取 Codex provider gateway 状态失败: {}", e))?;
    serde_json::from_str::<ProviderGatewayProfileState>(&content)
        .map(Some)
        .map_err(|e| format!("解析 Codex provider gateway 状态失败: {}", e))
}

fn save_provider_gateway_profile_state(
    profile_dir: &Path,
    account_id: &str,
    state: &ProviderGatewayProfileState,
) -> Result<(), String> {
    let path = provider_gateway_state_path(profile_dir, account_id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 Codex provider gateway 状态目录失败: {}", e))?;
    }
    let content = serde_json::to_string_pretty(state)
        .map_err(|e| format!("序列化 Codex provider gateway 状态失败: {}", e))?;
    write_string_atomic(&path, &content)
        .map_err(|e| format!("写入 Codex provider gateway 状态失败: {}", e))
}

fn provider_gateway_profile_api_key(
    profile_dir: &Path,
    account_id: &str,
) -> Result<String, String> {
    if let Some(state) = load_provider_gateway_profile_state(profile_dir, account_id)? {
        let api_key = state.api_key.trim();
        if !api_key.is_empty() {
            return Ok(api_key.to_string());
        }
    }

    let now = now_ms();
    let state = ProviderGatewayProfileState {
        api_key: generate_local_api_key(),
        port: None,
        created_at: now,
        updated_at: now,
    };
    save_provider_gateway_profile_state(profile_dir, account_id, &state)?;
    Ok(state.api_key)
}

fn provider_gateway_profile_port(
    profile_dir: &Path,
    account_id: &str,
) -> Result<u16, String> {
    if let Some(mut state) = load_provider_gateway_profile_state(profile_dir, account_id)? {
        if let Some(port) = state.port.filter(|port| *port > 0) {
            return Ok(port);
        }
        let port = allocate_random_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?;
        state.port = Some(port);
        state.updated_at = now_ms();
        save_provider_gateway_profile_state(profile_dir, account_id, &state)?;
        return Ok(port);
    }

    let now = now_ms();
    let state = ProviderGatewayProfileState {
        api_key: generate_local_api_key(),
        port: Some(allocate_random_local_port(
            CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST,
        )?),
        created_at: now,
        updated_at: now,
    };
    save_provider_gateway_profile_state(profile_dir, account_id, &state)?;
    Ok(state.port.expect("new provider gateway state must have a port"))
}

fn persisted_mixed_model_gateway_endpoint(
    profile_dir: &Path,
) -> Result<Option<(GatewayBindEndpoint, String)>, String> {
    let Some(state) =
        load_provider_gateway_profile_state(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID)?
    else {
        return Ok(None);
    };
    let Some(port) = state.port.filter(|port| *port > 0) else {
        return Ok(None);
    };
    let api_key = state.api_key.trim();
    if api_key.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        GatewayBindEndpoint {
            bind_host: CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST.to_string(),
            port,
        },
        api_key.to_string(),
    )))
}

async fn persisted_mixed_model_gateway_is_healthy(profile_dir: &Path) -> bool {
    let Ok(Some((endpoint, api_key))) = persisted_mixed_model_gateway_endpoint(profile_dir) else {
        return false;
    };
    probe_sidecar_ready_endpoint(endpoint.port, &api_key, Duration::from_millis(500))
        .await
        .is_ok()
}

#[derive(Debug)]
struct MixedModelProfileFileSnapshot {
    path: PathBuf,
    content: Option<Vec<u8>>,
}

#[derive(Debug)]
struct MixedModelProfileActivationSnapshot {
    files: Vec<MixedModelProfileFileSnapshot>,
    takeover_backup: Option<CodexLocalAccessProfileTakeoverBackup>,
    started_from_mixed_takeover: bool,
}

fn mixed_model_profile_activation_paths(profile_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for path in [
        profile_config_path(profile_dir),
        profile_auth_path(profile_dir),
        profile_dir.join(CODEX_LOCAL_ACCESS_AUTH_PROJECTION_FILE),
        profile_dir.join(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE),
        profile_dir.join(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE),
        profile_dir.join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE),
        profile_dir.join(CODEX_MODEL_CACHE_FILE),
        provider_model_backup_path(profile_dir),
    ] {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            paths.push(path);
        }
    }
    paths
}

fn capture_mixed_model_profile_activation_snapshot(
    profile_dir: &Path,
    api_key: &str,
) -> Result<MixedModelProfileActivationSnapshot, String> {
    let mut files = Vec::new();
    for path in mixed_model_profile_activation_paths(profile_dir) {
        let content = match fs::read(&path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "读取混合模型路由启动快照失败({}): {}",
                    path.display(),
                    error
                ));
            }
        };
        files.push(MixedModelProfileFileSnapshot { path, content });
    }

    let profile_key = normalize_profile_dir_key(profile_dir);
    let takeover_backup = load_takeover_backups()?
        .profiles
        .into_iter()
        .find(|backup| backup.profile_dir == profile_key);
    let current_config = read_optional_profile_file(&profile_config_path(profile_dir))?;
    let current_auth = read_optional_profile_file(&profile_auth_path(profile_dir))?;
    let started_from_mixed_takeover = current_config
        .as_deref()
        .is_some_and(|content| is_codex_local_access_config_for_api_key(content, api_key))
        || current_auth
            .as_deref()
            .is_some_and(|content| is_exact_codex_local_access_auth_text(content, api_key));

    Ok(MixedModelProfileActivationSnapshot {
        files,
        takeover_backup,
        started_from_mixed_takeover,
    })
}

fn restore_mixed_model_profile_activation_snapshot(
    profile_dir: &Path,
    snapshot: MixedModelProfileActivationSnapshot,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for file in snapshot.files {
        let result = match file.content {
            Some(content) => {
                let parent_result = file.path.parent().map_or(Ok(()), |parent| {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("创建启动回滚目录失败({}): {}", parent.display(), error)
                    })
                });
                parent_result.and_then(|()| {
                    crate::modules::atomic_write::write_bytes_atomic(&file.path, &content).map_err(
                        |error| format!("恢复启动快照失败({}): {}", file.path.display(), error),
                    )
                })
            }
            None => match fs::remove_file(&file.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!(
                    "清理启动失败残留失败({}): {}",
                    file.path.display(),
                    error
                )),
            },
        };
        if let Err(error) = result {
            errors.push(error);
        }
    }

    let profile_key = normalize_profile_dir_key(profile_dir);
    match load_takeover_backups().and_then(|mut backups| {
        backups
            .profiles
            .retain(|backup| backup.profile_dir != profile_key);
        if let Some(backup) = snapshot.takeover_backup {
            backups.profiles.push(backup);
        }
        backups.version = CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION;
        save_takeover_backups(&backups)
    }) {
        Ok(()) => {}
        Err(error) => errors.push(error),
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn rollback_mixed_model_profile_after_start_failure(
    profile_dir: &Path,
    snapshot: Option<MixedModelProfileActivationSnapshot>,
) -> Result<(), String> {
    if snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.started_from_mixed_takeover)
    {
        restore_mixed_model_gateway_profile(profile_dir)?;
        cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
        return Ok(());
    }
    if let Some(snapshot) = snapshot {
        return restore_mixed_model_profile_activation_snapshot(profile_dir, snapshot);
    }
    if restore_mixed_model_gateway_profile(profile_dir)? {
        cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
    }
    Ok(())
}

fn mixed_model_start_error_with_rollback(profile_dir: &Path, error: String) -> String {
    match rollback_mixed_model_profile_after_start_failure(profile_dir, None) {
        Ok(()) => error,
        Err(rollback_error) => format!("{}; 启动失败回滚也失败: {}", error, rollback_error),
    }
}

pub fn restore_mixed_model_gateway_profile(profile_dir: &Path) -> Result<bool, String> {
    let Some(state) =
        load_provider_gateway_profile_state(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID)?
    else {
        return Ok(false);
    };
    let api_key = state.api_key.trim();
    if api_key.is_empty() {
        return Ok(false);
    }

    let config_path = profile_config_path(profile_dir);
    let auth_path = profile_auth_path(profile_dir);
    let current_config = read_optional_profile_file(&config_path)?;
    let current_auth = read_optional_profile_file(&auth_path)?;
    let config_is_mixed = current_config
        .as_deref()
        .is_some_and(|content| is_codex_local_access_config_for_api_key(content, api_key));
    let auth_is_mixed = current_auth
        .as_deref()
        .is_some_and(|content| is_exact_codex_local_access_auth_text(content, api_key));
    if !config_is_mixed && !auth_is_mixed {
        return Ok(false);
    }

    let profile_key = normalize_profile_dir_key(profile_dir);
    let mut backups = load_takeover_backups()?;
    let backup_index = backups
        .profiles
        .iter()
        .position(|backup| backup.profile_dir == profile_key);
    if let Some(index) = backup_index {
        let backup = backups.profiles[index].clone();
        if config_is_mixed {
            let restored = restore_config_toml_from_takeover_backup(
                current_config.as_deref(),
                backup.config_toml.as_deref(),
            )?;
            write_optional_profile_file(&config_path, restored.as_deref())?;
        }
        if auth_is_mixed {
            write_optional_profile_file(&auth_path, backup.auth_json.as_deref())?;
        }
        let _ = cleanup_profile_takeover_artifacts(profile_dir)?;
        backups.profiles.remove(index);
        save_takeover_backups(&backups)?;
        return Ok(true);
    }

    cleanup_profile_takeover_without_backup(profile_dir, api_key, false)
}

pub fn profile_uses_mixed_model_gateway(profile_dir: &Path) -> Result<bool, String> {
    let Some(state) =
        load_provider_gateway_profile_state(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID)?
    else {
        return Ok(false);
    };
    let api_key = state.api_key.trim();
    if api_key.is_empty() {
        return Ok(false);
    }
    let config = read_optional_profile_file(&profile_config_path(profile_dir))?;
    let auth = read_optional_profile_file(&profile_auth_path(profile_dir))?;
    Ok(config
        .as_deref()
        .is_some_and(|content| is_codex_local_access_config_for_api_key(content, api_key))
        || auth
            .as_deref()
            .is_some_and(|content| is_exact_codex_local_access_auth_text(content, api_key)))
}

fn normalize_provider_gateway_models(models: Vec<&str>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty() || !seen.insert(model.to_ascii_lowercase()) {
            continue;
        }
        values.push(model.to_string());
    }
    values
}

fn mixed_route_upstream_models(
    catalog_models: &[String],
    selected_models: Option<&[String]>,
    extra_models: Option<&[String]>,
) -> Vec<String> {
    let selected = selected_models.map(|models| {
        models
            .iter()
            .map(|model| model.trim().to_ascii_lowercase())
            .collect::<HashSet<_>>()
    });
    let mut models = catalog_models
        .iter()
        .filter(|model| {
            selected.as_ref().is_none_or(|selected| {
                selected.contains(&model.trim().to_ascii_lowercase())
            })
        })
        .map(String::as_str)
        .collect::<Vec<_>>();
    models.extend(
        extra_models
            .unwrap_or_default()
            .iter()
            .filter(|model| {
                selected.as_ref().is_none_or(|selected| {
                    selected.contains(&model.trim().to_ascii_lowercase())
                })
            })
            .map(String::as_str),
    );
    normalize_provider_gateway_models(models)
}

fn provider_gateway_models_for_account(account: &CodexAccount) -> Vec<String> {
    let account_catalog = normalize_provider_gateway_models(
        account
            .api_model_catalog
            .iter()
            .map(String::as_str)
            .collect(),
    );
    if !account_catalog.is_empty() {
        return account_catalog;
    }
    let provider_id = account
        .api_provider_id
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let base_url = account
        .api_base_url
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if provider_id == "deepseek" || base_url.contains("api.deepseek.com") {
        return normalize_provider_gateway_models(vec!["deepseek-v4-flash", "deepseek-v4-pro"]);
    }
    if provider_id == "moonshot" || base_url.contains("api.moonshot.cn") {
        return normalize_provider_gateway_models(vec!["kimi-k2.6"]);
    }
    if provider_id == "zhipu_glm"
        || provider_id == "zhipu_glm_en"
        || base_url.contains("open.bigmodel.cn")
        || base_url.contains("api.z.ai")
    {
        return normalize_provider_gateway_models(vec!["glm-5.1"]);
    }
    Vec::new()
}

fn provider_gateway_default_model_for_account(account: &CodexAccount) -> String {
    provider_gateway_models_for_account(account)
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderGatewayModelSlot {
    pub(crate) client_model: String,
    pub(crate) upstream_model: String,
}

fn preferred_provider_gateway_slot<'a>(
    account: &CodexAccount,
    slots: &'a [ProviderGatewayModelSlot],
) -> Option<&'a ProviderGatewayModelSlot> {
    let startup = account
        .api_startup_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(startup) = startup {
        if let Some(slot) = slots.iter().find(|slot| {
            slot.upstream_model.eq_ignore_ascii_case(startup)
                || slot.client_model.eq_ignore_ascii_case(startup)
        }) {
            return Some(slot);
        }
    }
    slots.first()
}

fn is_provider_model_shell_slug(model: &str) -> bool {
    let model = model.trim();
    if model.is_empty() {
        return false;
    }
    CODEX_PROVIDER_MODEL_SHELL_POOL
        .iter()
        .any(|shell| shell.eq_ignore_ascii_case(model))
}

const DEEPSEEK_OFFICIAL_SHELL_SLOTS: &[(&str, &str)] = &[
    ("deepseek-v4-flash", "gpt-5.5"),
    ("deepseek-v4-pro", "gpt-5.4"),
];

fn allocate_official_deepseek_shell_slots(
    models: &[String],
) -> Option<Vec<ProviderGatewayModelSlot>> {
    let upstream_models =
        normalize_provider_gateway_models(models.iter().map(String::as_str).collect());
    if upstream_models.is_empty() {
        return None;
    }
    if !upstream_models.iter().all(|model| {
        DEEPSEEK_OFFICIAL_SHELL_SLOTS
            .iter()
            .any(|(upstream, _)| upstream.eq_ignore_ascii_case(model))
    }) {
        return None;
    }
    Some(
        DEEPSEEK_OFFICIAL_SHELL_SLOTS
            .iter()
            .filter(|(upstream, _)| {
                upstream_models
                    .iter()
                    .any(|model| model.eq_ignore_ascii_case(upstream))
            })
            .map(|(upstream, shell)| ProviderGatewayModelSlot {
                client_model: (*shell).to_string(),
                upstream_model: (*upstream).to_string(),
            })
            .collect(),
    )
}

/// Allocate client-visible model shells for upstream provider models.
///
/// 1. Official DeepSeek Responses models use a fixed 5.5 / 5.4 whitelist.
/// 2. Upstream IDs that already match an official shell keep identity.
/// 3. Remaining models claim free shells in pool order.
/// 4. If the shell pool is exhausted, keep the upstream ID so nothing is dropped.
pub(crate) fn allocate_provider_model_slots(models: &[String]) -> Vec<ProviderGatewayModelSlot> {
    if let Some(slots) = allocate_official_deepseek_shell_slots(models) {
        return slots;
    }
    let upstream_models =
        normalize_provider_gateway_models(models.iter().map(String::as_str).collect());
    let mut used_shells = HashSet::new();
    let mut slots = Vec::new();
    let mut deferred = Vec::new();

    for upstream_model in upstream_models {
        if is_provider_model_shell_slug(&upstream_model)
            && used_shells.insert(upstream_model.to_ascii_lowercase())
        {
            slots.push(ProviderGatewayModelSlot {
                client_model: upstream_model.clone(),
                upstream_model,
            });
        } else {
            deferred.push(upstream_model);
        }
    }

    let free_shells: Vec<&str> = CODEX_PROVIDER_MODEL_SHELL_POOL
        .iter()
        .copied()
        .filter(|shell| !used_shells.contains(&shell.to_ascii_lowercase()))
        .collect();
    let mut free_shells = free_shells.into_iter();

    for upstream_model in deferred {
        if let Some(shell) = free_shells.next() {
            used_shells.insert(shell.to_ascii_lowercase());
            slots.push(ProviderGatewayModelSlot {
                client_model: shell.to_string(),
                upstream_model,
            });
        } else {
            // Keep listing the model even without a free official shell.
            slots.push(ProviderGatewayModelSlot {
                client_model: upstream_model.clone(),
                upstream_model,
            });
        }
    }

    slots
}

fn provider_gateway_model_slots(models: &[String]) -> Vec<ProviderGatewayModelSlot> {
    allocate_provider_model_slots(models)
}

pub(crate) fn provider_model_slots_need_upstream_rewrite(
    slots: &[ProviderGatewayModelSlot],
) -> bool {
    slots
        .iter()
        .any(|slot| !slot.client_model.eq_ignore_ascii_case(&slot.upstream_model))
}

pub(crate) fn build_provider_model_catalog_json(
    slots: &[ProviderGatewayModelSlot],
) -> Result<String, String> {
    let mut model_ids = slots
        .iter()
        .map(|slot| slot.client_model.clone())
        .collect::<Vec<_>>();
    if !model_ids
        .iter()
        .any(|model| model.eq_ignore_ascii_case(CODEX_AUTO_REVIEW_MODEL_ID))
    {
        model_ids.push(CODEX_AUTO_REVIEW_MODEL_ID.to_string());
    }

    let mut client_models = codex_protocol::build_codex_client_models_response(&model_ids);
    if let Some(models) = client_models
        .get_mut("models")
        .and_then(Value::as_array_mut)
    {
        for model in models {
            let Some(slug) = model
                .get("slug")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            let Some(slot) = slots
                .iter()
                .find(|slot| slot.client_model.eq_ignore_ascii_case(&slug))
            else {
                continue;
            };
            let Some(object) = model.as_object_mut() else {
                continue;
            };
            object.insert(
                "display_name".to_string(),
                Value::String(slot.upstream_model.clone()),
            );
            object.insert(
                "description".to_string(),
                Value::String(slot.upstream_model.clone()),
            );
            // Ensure mapped provider models show up in the official picker.
            if !slug.eq_ignore_ascii_case(CODEX_AUTO_REVIEW_MODEL_ID) {
                object.insert("visibility".to_string(), Value::String("list".to_string()));
            }
        }
    }

    let catalog = json!({
        "models": client_models
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    });
    serde_json::to_string_pretty(&catalog).map_err(|e| format!("生成 Codex 模型目录失败: {}", e))
}

const FALLBACK_CATALOG_CONTEXT_WINDOW: i64 = 128_000;

fn lookup_explicit_catalog_context_window(
    slot: &ProviderGatewayModelSlot,
    explicit: &HashMap<String, i64>,
) -> Option<i64> {
    for key in [&slot.upstream_model, &slot.client_model] {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(window) = explicit.get(trimmed).copied().or_else(|| {
            explicit.iter().find_map(|(name, value)| {
                name.trim().eq_ignore_ascii_case(trimmed).then_some(*value)
            })
        }) {
            if window > 0 {
                return Some(window);
            }
        }
    }
    None
}

fn is_official_deepseek_catalog_model(model: &str) -> bool {
    DEEPSEEK_OFFICIAL_SHELL_SLOTS
        .iter()
        .any(|(upstream, _)| upstream.eq_ignore_ascii_case(model.trim()))
}

fn should_keep_official_catalog_window(slot: &ProviderGatewayModelSlot) -> bool {
    if is_official_deepseek_catalog_model(&slot.upstream_model)
        || is_official_deepseek_catalog_model(&slot.client_model)
    {
        return true;
    }
    slot.client_model
        .trim()
        .eq_ignore_ascii_case(slot.upstream_model.trim())
        && is_provider_model_shell_slug(&slot.client_model)
}

pub(crate) fn decorate_catalog_context_windows(
    catalog_json: &str,
    slots: &[ProviderGatewayModelSlot],
    explicit: &HashMap<String, i64>,
    default_window: Option<i64>,
) -> Result<String, String> {
    let mut catalog: Value =
        serde_json::from_str(catalog_json).map_err(|e| format!("解析模型目录失败: {}", e))?;
    let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) else {
        return Ok(catalog_json.to_string());
    };
    let fallback = default_window
        .filter(|value| *value > 0)
        .unwrap_or(FALLBACK_CATALOG_CONTEXT_WINDOW);
    for model in models.iter_mut() {
        let slug = model
            .get("slug")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if slug.is_empty() {
            continue;
        }
        let slot = slots
            .iter()
            .find(|slot| slot.client_model.eq_ignore_ascii_case(&slug));
        let window = if let Some(slot) = slot {
            lookup_explicit_catalog_context_window(slot, explicit).or_else(|| {
                if should_keep_official_catalog_window(slot) {
                    None
                } else {
                    Some(fallback)
                }
            })
        } else {
            explicit
                .get(&slug)
                .copied()
                .or_else(|| {
                    explicit.iter().find_map(|(name, value)| {
                        name.trim().eq_ignore_ascii_case(&slug).then_some(*value)
                    })
                })
                .filter(|value| *value > 0)
        };
        let Some(window) = window else {
            continue;
        };
        if let Some(object) = model.as_object_mut() {
            object.insert("context_window".to_string(), json!(window));
            object.insert("max_context_window".to_string(), json!(window));
        }
    }
    serde_json::to_string_pretty(&catalog).map_err(|e| format!("序列化模型目录失败: {}", e))
}

pub(crate) fn decorate_account_catalog_context_windows(
    catalog_json: &str,
    slots: &[ProviderGatewayModelSlot],
    account: &CodexAccount,
    default_window: Option<i64>,
) -> Result<String, String> {
    decorate_catalog_context_windows(
        catalog_json,
        slots,
        &account.api_model_context_windows,
        default_window,
    )
}

pub(crate) fn read_toml_model_context_window(doc: &Document) -> Option<i64> {
    doc.get("model_context_window")
        .and_then(|item| item.as_integer())
}

pub(crate) fn read_file_model_context_window(path: &std::path::Path) -> Option<i64> {
    let existing = std::fs::read_to_string(path).ok()?;
    if existing.trim().is_empty() {
        return None;
    }
    let doc =
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing).ok()?;
    read_toml_model_context_window(&doc)
}

/// Build a Codex client catalog that keeps official shell slugs for display, but copies
/// upstream official model metadata (tools / shell / apply_patch / instructions).
/// The gateway still rewrites the request `model` field back to `slot.upstream_model`.
pub(crate) fn build_official_template_mapped_catalog_json(
    slots: &[ProviderGatewayModelSlot],
    official_catalog_json: &str,
) -> Result<String, String> {
    if slots.is_empty() {
        return Err("模型目录为空".to_string());
    }
    let official: Value = serde_json::from_str(official_catalog_json)
        .map_err(|error| format!("解析官方模型目录失败: {}", error))?;
    let templates: HashMap<String, Value> = official
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "官方模型目录缺少 models 数组".to_string())?
        .iter()
        .filter_map(|model| {
            let slug = model.get("slug")?.as_str()?.trim();
            if slug.is_empty() {
                return None;
            }
            Some((slug.to_ascii_lowercase(), model.clone()))
        })
        .collect();

    let mut models = Vec::new();
    let mut missing = Vec::new();
    for slot in slots {
        let key = slot.upstream_model.trim().to_ascii_lowercase();
        if let Some(template) = templates.get(&key) {
            let mut entry = template.clone();
            if let Some(object) = entry.as_object_mut() {
                object.insert("slug".to_string(), Value::String(slot.client_model.clone()));
                object.insert("visibility".to_string(), Value::String("list".to_string()));
            }
            models.push(entry);
        } else {
            missing.push(slot.clone());
        }
    }
    if models.is_empty() {
        return Err("官方模型目录未匹配到任何上游模型".to_string());
    }
    if !missing.is_empty() {
        let fallback: Value = serde_json::from_str(&build_provider_model_catalog_json(&missing)?)
            .map_err(|error| format!("解析回退模型目录失败: {}", error))?;
        if let Some(fallback_models) = fallback.get("models").and_then(Value::as_array) {
            for model in fallback_models {
                let slug = model
                    .get("slug")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if slug.eq_ignore_ascii_case(CODEX_AUTO_REVIEW_MODEL_ID) {
                    continue;
                }
                models.push(model.clone());
            }
        }
    }
    if !models.iter().any(|model| {
        model
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| slug.eq_ignore_ascii_case(CODEX_AUTO_REVIEW_MODEL_ID))
    }) {
        let review_catalog: Value = serde_json::from_str(&build_provider_model_catalog_json(&[])?)
            .map_err(|error| format!("解析内置自动评审模型失败: {}", error))?;
        if let Some(review_models) = review_catalog.get("models").and_then(Value::as_array) {
            for model in review_models {
                if model
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|slug| slug.eq_ignore_ascii_case(CODEX_AUTO_REVIEW_MODEL_ID))
                {
                    models.push(model.clone());
                }
            }
        }
    }

    serde_json::to_string_pretty(&json!({ "models": models }))
        .map_err(|error| format!("生成 Codex 模型目录失败: {}", error))
}

fn apply_provider_gateway_model_slots(
    collection: &mut CodexLocalAccessCollection,
    models: &[String],
) {
    let slots = provider_gateway_model_slots(models);
    let client_models: HashSet<String> = slots
        .iter()
        .map(|slot| slot.client_model.to_ascii_lowercase())
        .collect();
    let upstream_models: HashSet<String> = slots
        .iter()
        .map(|slot| slot.upstream_model.to_ascii_lowercase())
        .collect();
    collection.model_aliases.retain(|alias| {
        !client_models.contains(&alias.alias.to_ascii_lowercase())
            && !upstream_models.contains(&alias.source_model.to_ascii_lowercase())
    });
    collection
        .model_aliases
        .extend(slots.into_iter().map(|slot| CodexLocalAccessModelAlias {
            source_model: slot.upstream_model,
            alias: slot.client_model,
            fork: false,
        }));
}

fn provider_gateway_wire_api_for_account(account: &CodexAccount) -> String {
    if account.auth_mode != CodexAuthMode::Apikey {
        return "responses".to_string();
    }
    if let Some(wire_api) = account
        .api_wire_api
        .as_deref()
        .map(str::trim)
        .filter(|value| *value == "responses" || *value == "chat_completions")
    {
        return wire_api.to_string();
    }
    let base_url = account
        .api_base_url
        .as_deref()
        .unwrap_or(DEFAULT_OPENAI_RESPONSES_BASE_URL)
        .trim()
        .to_ascii_lowercase();
    if base_url.contains("/chat/completions") {
        return "chat_completions".to_string();
    }
    let host = Url::parse(&base_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .unwrap_or_default();
    let chat_hosts = [
        "api.deepseek.com",
        "api.moonshot.cn",
        "api.siliconflow.cn",
        "api.siliconflow.com",
        "open.bigmodel.cn",
        "api.z.ai",
        "volces.com",
        "bytepluses.com",
        "qianfan.baidubce.com",
        "dashscope.aliyuncs.com",
        "api.stepfun.com",
        "api.stepfun.ai",
        "modelscope.cn",
        "api.longcat.chat",
        "api.minimax.io",
        "api.mini-max.chat",
        "api.minimaxi.com",
        "api.tbox.cn",
        "api.mimo.dev",
        "api.xiaomimimo.com",
        "token-plan-cn.xiaomimimo.com",
        "api.novita.ai",
        "integrate.api.nvidia.com",
        "runapi.co",
        "www.relaxycode.com",
        "cp.compshare.cn",
        "api.lemondata.cc",
        "e-flowcode.cc",
        "cc-api.pipellm.ai",
        "openrouter.ai",
        "api.therouter.ai",
    ];
    if chat_hosts.iter().any(|pattern| host.contains(pattern)) {
        "chat_completions".to_string()
    } else {
        "responses".to_string()
    }
}

fn is_official_deepseek_account(account: &CodexAccount) -> bool {
    account
        .api_provider_id
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("deepseek"))
        || account
            .api_base_url
            .as_deref()
            .and_then(|value| Url::parse(value.trim()).ok())
            .and_then(|url| url.host_str().map(str::to_string))
            .is_some_and(|host| host.eq_ignore_ascii_case("api.deepseek.com"))
}

fn account_uses_synced_model_shell_gateway(account: &CodexAccount) -> bool {
    if !account.is_api_key_auth() {
        return false;
    }
    if account.api_provider_mode != CodexApiProviderMode::Custom {
        return false;
    }
    if !account.api_sync_model_catalog_to_codex {
        return false;
    }
    if is_official_deepseek_account(account)
        && provider_gateway_wire_api_for_account(account) == "responses"
        && account
            .api_instance_access_mode
            .as_deref()
            .map(str::trim)
            .is_some_and(|mode| {
                mode.eq_ignore_ascii_case("direct") || mode.eq_ignore_ascii_case("cdp")
            })
    {
        return false;
    }
    // Responses path normally talks to upstream directly. When the synced catalog needs
    // official shells for UI display, route through provider gateway so requests can be
    // rewritten back to the real upstream model IDs. DeepSeek official Responses only
    // uses that rewrite; the sidecar keeps Responses passthrough.
    if provider_gateway_wire_api_for_account(account) != "responses" {
        return false;
    }
    let models = provider_gateway_models_for_account(account);
    if models.is_empty() {
        return false;
    }
    provider_model_slots_need_upstream_rewrite(&provider_gateway_model_slots(&models))
}

fn is_chat_completions_api_key_account(account: &CodexAccount) -> bool {
    account.is_api_key_auth()
        && provider_gateway_wire_api_for_account(account) == "chat_completions"
}

pub fn account_requires_provider_gateway(account: &CodexAccount) -> bool {
    if is_chat_completions_api_key_account(account) {
        return true;
    }
    account_uses_synced_model_shell_gateway(account)
}

/// 绑定 OAuth 的 API Key 不再走本地网关生图兼容（与「改前」一致）。
/// 纯 API Key 生图不依赖此路径。
pub fn account_requires_bound_oauth_local_gateway(account: &CodexAccount) -> bool {
    let _ = account;
    false
}

fn provider_gateway_image_generation_mode_for_account(
    _account: &CodexAccount,
    _inherited_mode: CodexLocalAccessImageGenerationMode,
) -> CodexLocalAccessImageGenerationMode {
    // 绑定 OAuth / 供应商网关：默认全开生图（改前逻辑）
    CodexLocalAccessImageGenerationMode::Enabled
}

pub fn is_local_access_runtime_account_id(account_id: &str) -> bool {
    account_id.trim() == CODEX_LOCAL_ACCESS_RUNTIME_ACCOUNT_ID
}

fn is_provider_gateway_eligible_account(account: &CodexAccount) -> bool {
    account_requires_provider_gateway(account)
}

fn collection_uses_provider_gateway_account(
    collection: &CodexLocalAccessCollection,
    account_id: &str,
) -> bool {
    collection.api_keys.iter().any(|item| {
        (item.provider_gateway.is_some()
            && item
                .account_ids
                .iter()
                .any(|candidate| candidate == account_id))
            || item.model_routing.as_ref().map_or(false, |routing| {
                routing
                    .routes
                    .iter()
                    .any(|r| r.provider_account_id == account_id)
            })
    })
}

fn provider_gateway_for_account(
    account: &CodexAccount,
) -> Result<CodexLocalAccessProviderGateway, String> {
    let api_key = account
        .openai_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "供应商账号缺少上游 API Key".to_string())?;
    let base_url = account
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_OPENAI_RESPONSES_BASE_URL);
    Ok(CodexLocalAccessProviderGateway {
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        upstream_model: provider_gateway_default_model_for_account(account),
        upstream_models: provider_gateway_models_for_account(account),
        wire_api: Some(provider_gateway_wire_api_for_account(account)),
        supports_vision: account.api_supports_vision,
        model_capabilities: account
            .api_model_vision_support
            .iter()
            .filter_map(|(model, supports_vision)| {
                let model = model.trim().to_lowercase();
                if model.is_empty() {
                    None
                } else {
                    Some((
                        model,
                        CodexLocalAccessProviderGatewayModelCapability {
                            supports_vision: *supports_vision,
                        },
                    ))
                }
            })
            .collect(),
        vision_routing_model: account
            .api_vision_routing_model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

fn apply_provider_gateway_template_settings(
    collection: &mut CodexLocalAccessCollection,
    template: &CodexLocalAccessCollection,
) {
    collection.client_base_url_host = template.client_base_url_host;
    collection.image_generation_mode = template.image_generation_mode;
    collection.upstream_proxy_url = template.upstream_proxy_url.clone();
    collection.routing_strategy = template.routing_strategy;
    collection.model_aliases = template.model_aliases.clone();
    collection.model_pricings = template.model_pricings.clone();
    collection.excluded_models = template.excluded_models.clone();
    collection.session_affinity = template.session_affinity;
    collection.session_affinity_ttl_ms = template.session_affinity_ttl_ms;
    collection.session_affinity_default_enabled_migrated =
        template.session_affinity_default_enabled_migrated;
    collection.max_retry_credentials = template.max_retry_credentials;
    collection.max_retry_interval_ms = template.max_retry_interval_ms;
    collection.timeouts = template.timeouts.clone();
    collection.active_timeout_preset_id = template.active_timeout_preset_id.clone();
    collection.timeout_presets = template.timeout_presets.clone();
    collection.disable_cooling = template.disable_cooling;
    collection.restrict_free_accounts = template.restrict_free_accounts;
    collection.debug_logs = template.debug_logs;
}

fn provider_gateway_bound_oauth_account_id_for_account(account: &CodexAccount) -> Option<String> {
    if !account.is_api_key_auth() {
        return None;
    }
    normalize_optional_account_ref(account.bound_oauth_account_id.as_deref())
}

fn normalize_mixed_model_namespace(namespace: &str) -> Result<String, String> {
    let namespace = namespace.trim().to_ascii_lowercase();
    if !(2..=32).contains(&namespace.len()) {
        return Err("模型路由命名空间长度必须为 2-32 个字符".to_string());
    }
    let mut chars = namespace.chars();
    let Some(first) = chars.next() else {
        return Err("模型路由命名空间不能为空".to_string());
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err("模型路由命名空间必须以小写字母或数字开头".to_string());
    }
    if chars.any(|ch| !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '_' && ch != '-') {
        return Err("模型路由命名空间只能包含小写字母、数字、下划线和连字符".to_string());
    }
    if ["official", "subscription", "openai", "codex", "oauth"]
        .iter()
        .any(|reserved| namespace == *reserved)
    {
        return Err(format!("模型路由命名空间 {} 为保留名称", namespace));
    }
    Ok(namespace)
}

pub fn validate_mixed_model_routing_config(
    bind_account_id: Option<&str>,
    routing: &CodexInstanceModelRouting,
) -> Result<CodexInstanceModelRouting, String> {
    if !routing.enabled {
        return Ok(routing.clone());
    }
    if routing.version != 1 {
        return Err(format!("不支持的混合模型路由版本: {}", routing.version));
    }
    let oauth_account_id = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("启用混合模型路由前必须绑定直接登录的 OAuth 订阅账号")?;
    let _ = validate_local_access_bound_oauth_account(oauth_account_id)?;

    let mut seen_namespaces = HashSet::new();
    let mut normalized_routes = Vec::with_capacity(routing.routes.len());
    let mut enabled_count = 0usize;
    for route in &routing.routes {
        let id = route.id.trim();
        if id.is_empty() {
            return Err("模型路由缺少 route id".to_string());
        }
        let namespace = normalize_mixed_model_namespace(&route.namespace)?;
        if !seen_namespaces.insert(namespace.clone()) {
            return Err(format!("模型路由命名空间重复: {}", namespace));
        }
        let provider_account_id = route.provider_account_id.trim();
        if provider_account_id.is_empty() {
            return Err(format!("模型路由 {} 尚未选择 API 账号", namespace));
        }
        if provider_account_id == oauth_account_id {
            return Err(format!("模型路由 {} 不能使用订阅 OAuth 账号", namespace));
        }
        let provider_account = codex_account::load_account(provider_account_id)
            .ok_or_else(|| format!("模型路由 {} 的 API 账号不存在", namespace))?;
        if !provider_account.is_api_key_auth() {
            return Err(format!("模型路由 {} 必须绑定 API Key 账号", namespace));
        }
        let _gateway = provider_gateway_for_account(&provider_account)?;
        let selected_models = route.selected_models.as_ref().map(|models| {
            normalize_provider_gateway_models(models.iter().map(String::as_str).collect())
        });
        if route.enabled && selected_models.as_ref().is_some_and(Vec::is_empty) {
            return Err(format!(
                "模型路由 {} 至少需要选择一个上游模型",
                namespace
            ));
        }
        let extra_models = route.extra_models.as_ref().map(|models| {
            normalize_provider_gateway_models(models.iter().map(String::as_str).collect())
        });
        if route.enabled {
            enabled_count += 1;
        }
        normalized_routes.push(CodexInstanceApiRoute {
            id: id.to_string(),
            namespace,
            provider_account_id: provider_account_id.to_string(),
            enabled: route.enabled,
            selected_models,
            extra_models,
        });
    }
    if enabled_count == 0 {
        return Err("混合模型路由至少需要一个已启用的 API 路由".to_string());
    }
    Ok(CodexInstanceModelRouting {
        enabled: true,
        version: 1,
        routes: normalized_routes,
    })
}

fn build_mixed_model_gateway_collection_for_profile(
    profile_dir: &Path,
    oauth_account: &CodexAccount,
    routing: &CodexInstanceModelRouting,
) -> Result<(CodexLocalAccessCollection, String), String> {
    let routing = validate_mixed_model_routing_config(Some(&oauth_account.id), routing)?;
    let mut collection = new_empty_local_access_collection()?;
    if let Some(template) = load_collection_from_disk()? {
        apply_provider_gateway_template_settings(&mut collection, &template);
    }

    collection.enabled = true;
    collection.port =
        provider_gateway_profile_port(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID)?;
    collection.access_scope = CodexLocalAccessScope::Localhost;
    collection.client_base_url_host = CodexLocalAccessClientBaseUrlHost::default();
    collection.gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
    collection.responses_websockets_enabled = false;
    collection.account_ids = vec![oauth_account.id.clone()];
    collection.custom_routing_rules.clear();
    collection.account_model_rules.clear();
    collection.api_keys.clear();
    collection.bound_oauth_account_id = Some(oauth_account.id.clone());

    let mut routes = Vec::new();
    for route in routing.routes.iter().filter(|route| route.enabled) {
        let provider_account = codex_account::load_account(&route.provider_account_id)
            .ok_or_else(|| format!("模型路由 {} 的 API 账号不存在", route.namespace))?;
        let mut provider_gateway = provider_gateway_for_account(&provider_account)?;
        provider_gateway.upstream_models = mixed_route_upstream_models(
            &provider_gateway.upstream_models,
            route.selected_models.as_deref(),
            route.extra_models.as_deref(),
        );
        routes.push(CodexLocalAccessModelRoute {
            id: route.id.clone(),
            namespace: route.namespace.clone(),
            provider_account_id: route.provider_account_id.clone(),
            provider_gateway,
        });
    }

    let key = provider_gateway_profile_api_key(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID)?;
    let now = now_ms();
    collection.api_key = key.clone();
    collection.api_keys.push(CodexLocalAccessApiKey {
        id: mixed_model_routing_api_key_id(),
        label: "Mixed Model Routing".to_string(),
        key: key.clone(),
        provider_gateway: None,
        model_routing: Some(CodexLocalAccessModelRouting {
            default_route: "oauth".to_string(),
            failure_policy: "strict".to_string(),
            routes,
        }),
        inherit_account_pool: Some(false),
        account_ids: vec![oauth_account.id.clone()],
        priority_account_ids: Vec::new(),
        preferred_account_id: Some(oauth_account.id.clone()),
        model_prefix: None,
        allowed_models: Vec::new(),
        excluded_models: Vec::new(),
        token_limit: None,
        token_used: 0,
        enabled: true,
        created_at: now,
        updated_at: now,
        last_used_at: None,
    });
    collection.updated_at = now;
    let (changed, _) = sanitize_collection(&mut collection)?;
    if changed {
        collection.updated_at = now_ms();
    }
    Ok((collection, key))
}

fn build_provider_gateway_collection_for_profile(
    profile_dir: &Path,
    account: &CodexAccount,
) -> Result<
    (
        CodexLocalAccessCollection,
        String,
        CodexLocalAccessProviderGateway,
    ),
    String,
> {
    let mut collection = new_empty_local_access_collection()?;
    if let Some(template) = load_collection_from_disk()? {
        apply_provider_gateway_template_settings(&mut collection, &template);
    }

    collection.enabled = true;
    collection.port = allocate_random_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?;
    collection.access_scope = CodexLocalAccessScope::Localhost;
    collection.client_base_url_host = CodexLocalAccessClientBaseUrlHost::default();
    collection.gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
    collection.image_generation_mode = provider_gateway_image_generation_mode_for_account(
        account,
        collection.image_generation_mode,
    );
    collection.account_ids.clear();
    collection.custom_routing_rules.clear();
    collection.account_model_rules.clear();
    collection.api_keys.clear();
    collection.bound_oauth_account_id =
        provider_gateway_bound_oauth_account_id_for_account(account);

    if !is_provider_gateway_eligible_account(account) {
        return Err("该供应商账号不符合本地网关使用条件".to_string());
    }

    let provider_gateway = provider_gateway_for_account(account)?;
    apply_provider_gateway_model_slots(&mut collection, &provider_gateway.upstream_models);
    let key = provider_gateway_profile_api_key(profile_dir, &account.id)?;
    let now = now_ms();
    collection.api_key = key.clone();
    collection.api_keys.push(CodexLocalAccessApiKey {
        id: provider_gateway_api_key_id(&account.id),
        label: format!("Provider Gateway: {}", account.email),
        key: key.clone(),
        provider_gateway: Some(provider_gateway.clone()),
        model_routing: None,
        inherit_account_pool: Some(false),
        account_ids: vec![account.id.clone()],
        priority_account_ids: Vec::new(),
        preferred_account_id: None,
        model_prefix: None,
        allowed_models: Vec::new(),
        excluded_models: Vec::new(),
        token_limit: None,
        token_used: 0,
        enabled: true,
        created_at: now,
        updated_at: now,
        last_used_at: None,
    });
    collection.updated_at = now;
    let (changed, _) = sanitize_collection(&mut collection)?;
    if changed {
        collection.updated_at = now_ms();
    }
    Ok((collection, key, provider_gateway))
}

fn build_bound_oauth_local_gateway_collection_for_profile(
    profile_dir: &Path,
    account: &CodexAccount,
) -> Result<(CodexLocalAccessCollection, String), String> {
    let mut collection = new_empty_local_access_collection()?;
    if let Some(template) = load_collection_from_disk()? {
        apply_provider_gateway_template_settings(&mut collection, &template);
    }

    collection.enabled = true;
    collection.port = allocate_random_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?;
    collection.access_scope = CodexLocalAccessScope::Localhost;
    collection.client_base_url_host = CodexLocalAccessClientBaseUrlHost::default();
    collection.gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
    collection.image_generation_mode = provider_gateway_image_generation_mode_for_account(
        account,
        collection.image_generation_mode,
    );
    collection.account_ids.clear();
    collection.custom_routing_rules.clear();
    collection.account_model_rules.clear();
    collection.api_keys.clear();
    collection.bound_oauth_account_id =
        provider_gateway_bound_oauth_account_id_for_account(account);

    if !account_requires_bound_oauth_local_gateway(account) {
        return Err("该 API Key 账号不需要绑定 OAuth 本地网关".to_string());
    }

    let key = provider_gateway_profile_api_key(profile_dir, &account.id)?;
    let now = now_ms();
    collection.api_key = key.clone();
    collection.api_keys.push(CodexLocalAccessApiKey {
        id: provider_gateway_api_key_id(&account.id),
        label: format!("Bound OAuth Local Gateway: {}", account.email),
        key: key.clone(),
        provider_gateway: None,
        model_routing: None,
        inherit_account_pool: Some(false),
        account_ids: vec![account.id.clone()],
        priority_account_ids: Vec::new(),
        preferred_account_id: None,
        model_prefix: None,
        allowed_models: Vec::new(),
        excluded_models: Vec::new(),
        token_limit: None,
        token_used: 0,
        enabled: true,
        created_at: now,
        updated_at: now,
        last_used_at: None,
    });
    collection.updated_at = now;
    let (changed, _) = sanitize_collection(&mut collection)?;
    if changed {
        collection.updated_at = now_ms();
    }
    Ok((collection, key))
}

#[derive(Debug, Clone)]
pub struct CodexModelProviderGatewayChatTestRequest {
    pub run_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub base_url: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub api_key: String,
    pub wire_api: String,
    pub model_catalog: Vec<String>,
    pub model_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone)]
pub struct CodexModelProviderGatewayChatTestResult {
    pub duration_ms: u64,
    pub reply: String,
}

fn push_unique_model_id(values: &mut Vec<String>, seen: &mut HashSet<String>, model: &str) {
    let model = model.trim();
    if model.is_empty() {
        return;
    }
    if seen.insert(model.to_ascii_lowercase()) {
        values.push(model.to_string());
    }
}

fn model_provider_gateway_test_models(model_id: &str, model_catalog: &[String]) -> Vec<String> {
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    push_unique_model_id(&mut values, &mut seen, model_id);
    for model in model_catalog {
        push_unique_model_id(&mut values, &mut seen, model);
    }
    values
}

fn model_provider_gateway_test_account_id(
    request: &CodexModelProviderGatewayChatTestRequest,
) -> String {
    format!(
        "model_provider_test_{}",
        stable_uuid_from_text(&format!(
            "{}\n{}\n{}",
            request.run_id.trim(),
            request.provider_id.trim(),
            request.api_key_id.as_deref().unwrap_or_default().trim()
        ))
    )
}

fn build_model_provider_gateway_test_account(
    request: &CodexModelProviderGatewayChatTestRequest,
    account_id: String,
    models: Vec<String>,
) -> CodexAccount {
    let provider_name = request.provider_name.trim();
    let mut account = CodexAccount::new_api_key(
        account_id,
        format!("{}@model-provider-test.local", request.provider_id.trim()),
        request.api_key.trim().to_string(),
        CodexApiProviderMode::Custom,
        Some(request.base_url.trim().to_string()),
        Some(request.provider_id.trim().to_string()),
        Some(provider_name.to_string()),
        models,
    );
    account.api_wire_api = Some(request.wire_api.trim().to_string());
    account.account_name = Some(format!(
        "Model Provider Test: {}",
        if provider_name.is_empty() {
            request.provider_id.trim()
        } else {
            provider_name
        }
    ));
    account
}

fn provider_gateway_test_api_key_id(request: &CodexModelProviderGatewayChatTestRequest) -> String {
    let source = format!(
        "{}\n{}\n{}",
        request.run_id.trim(),
        request.provider_id.trim(),
        request.model_id.trim()
    );
    format!("model_provider_test_{}", stable_uuid_from_text(&source))
}

fn provider_gateway_test_api_key_label(
    request: &CodexModelProviderGatewayChatTestRequest,
) -> String {
    let provider = request.provider_name.trim();
    let key_label = request
        .api_key_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| request.api_key_id.as_deref().map(str::trim))
        .filter(|value| !value.is_empty());
    match (provider.is_empty(), key_label) {
        (false, Some(key_label)) => format!("Model Provider Test: {} / {}", provider, key_label),
        (false, None) => format!("Model Provider Test: {}", provider),
        (true, Some(key_label)) => format!("Model Provider Test: {}", key_label),
        (true, None) => "Model Provider Test".to_string(),
    }
}

fn model_provider_test_uses_provider_gateway(
    request: &CodexModelProviderGatewayChatTestRequest,
) -> bool {
    request.wire_api.trim() == "chat_completions"
}

fn model_provider_direct_test_client_model() -> String {
    supported_codex_model_ids()
        .into_iter()
        .find(|model| model.eq_ignore_ascii_case("gpt-5.4"))
        .unwrap_or_else(|| "gpt-5.4".to_string())
}

fn build_model_provider_gateway_test_collection(
    request: &CodexModelProviderGatewayChatTestRequest,
    account: &CodexAccount,
    provider_gateway: Option<CodexLocalAccessProviderGateway>,
    client_model_id: &str,
) -> Result<CodexLocalAccessCollection, String> {
    let mut collection = new_empty_local_access_collection()?;
    if let Some(template) = load_collection_from_disk()? {
        apply_provider_gateway_template_settings(&mut collection, &template);
    }

    let now = now_ms();
    collection.enabled = true;
    collection.port = allocate_random_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?;
    collection.access_scope = CodexLocalAccessScope::Localhost;
    collection.client_base_url_host = CodexLocalAccessClientBaseUrlHost::default();
    collection.gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
    collection.image_generation_mode = CodexLocalAccessImageGenerationMode::Enabled;
    collection.account_ids.clear();
    collection.custom_routing_rules.clear();
    collection.account_model_rules.clear();
    collection.api_keys.clear();
    collection.model_aliases.clear();
    collection.excluded_models.clear();
    collection.bound_oauth_account_id = None;
    collection.api_key = generate_local_api_key();
    if provider_gateway.is_none() {
        collection.model_aliases.push(CodexLocalAccessModelAlias {
            source_model: request.model_id.trim().to_string(),
            alias: client_model_id.trim().to_string(),
            fork: false,
        });
    }
    collection.api_keys.push(CodexLocalAccessApiKey {
        id: provider_gateway_test_api_key_id(request),
        label: provider_gateway_test_api_key_label(request),
        key: collection.api_key.clone(),
        provider_gateway,
        model_routing: None,
        inherit_account_pool: Some(false),
        account_ids: vec![account.id.clone()],
        priority_account_ids: Vec::new(),
        preferred_account_id: None,
        model_prefix: None,
        allowed_models: vec![client_model_id.trim().to_string()],
        excluded_models: Vec::new(),
        token_limit: None,
        token_used: 0,
        enabled: true,
        created_at: now,
        updated_at: now,
        last_used_at: None,
    });
    collection.updated_at = now;
    Ok(collection)
}

fn extract_provider_gateway_test_output_text(body: &Value) -> Option<String> {
    if let Some(text) = body
        .get("output_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }
    let mut parts = Vec::new();
    if let Some(items) = body.get("output").and_then(Value::as_array) {
        for item in items {
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for part in content {
                    if let Some(text) = part
                        .get("text")
                        .or_else(|| part.get("content"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                    {
                        parts.push(text.to_string());
                    }
                }
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(""))
    }
}

fn extract_provider_gateway_test_error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| {
                    error
                        .get("message")
                        .or_else(|| error.get("detail"))
                        .or_else(|| error.get("code"))
                })
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| body.chars().take(1000).collect::<String>())
}

fn add_model_provider_test_header(
    builder: reqwest::RequestBuilder,
    name: &'static str,
    value: &str,
) -> reqwest::RequestBuilder {
    let value = value.trim();
    if value.is_empty() {
        return builder;
    }
    match HeaderValue::from_str(value) {
        Ok(value) => builder.header(HeaderName::from_static(name), value),
        Err(_) => builder,
    }
}

fn add_model_provider_test_header_b64(
    builder: reqwest::RequestBuilder,
    name: &'static str,
    value: &str,
) -> reqwest::RequestBuilder {
    let value = value.trim();
    if value.is_empty() {
        return builder;
    }
    let encoded = general_purpose::STANDARD.encode(value.as_bytes());
    add_model_provider_test_header(builder, name, &encoded)
}

async fn stop_temporary_provider_gateway_sidecar(
    mut child: Child,
    mut task: tokio::task::JoinHandle<()>,
    bind_host: String,
    port: u16,
    sidecar_dir: PathBuf,
) {
    match timeout(GATEWAY_SHUTDOWN_TIMEOUT, child.kill()).await {
        Ok(Ok(())) => {
            let _ = child.wait().await;
        }
        Ok(Err(error)) => {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][provider-gateway-test] 停止临时 sidecar 失败: {}",
                error
            ));
        }
        Err(_) => {
            logger::log_codex_api_warn(
                "[CodexLocalAccess][provider-gateway-test] 停止临时 sidecar 超时",
            );
        }
    }
    tokio::select! {
        result = &mut task => {
            let _ = result;
        }
        _ = tokio::time::sleep(GATEWAY_SHUTDOWN_TIMEOUT) => {
            logger::log_codex_api_warn("[CodexLocalAccess][provider-gateway-test] 停止临时 sidecar 监听任务超时，已中止");
            task.abort();
        }
    }
    if let Err(error) = wait_for_gateway_port_release(&bind_host, port).await {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][provider-gateway-test] 等待临时端口释放失败: bind={}:{} error={}",
            bind_host, port, error
        ));
    }
    if sidecar_dir.exists() {
        if let Err(error) = std::fs::remove_dir_all(&sidecar_dir) {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][provider-gateway-test] 清理临时 sidecar 目录失败: dir={} error={}",
                sidecar_dir.display(),
                error
            ));
        }
    }
}

pub async fn run_model_provider_gateway_chat_test(
    request: CodexModelProviderGatewayChatTestRequest,
) -> Result<CodexModelProviderGatewayChatTestResult, String> {
    let run_id = request.run_id.trim().to_string();
    if is_model_provider_chat_test_cancelled(&run_id) {
        return Err(MODEL_PROVIDER_CHAT_TEST_CANCELLED_ERROR.to_string());
    }
    let model_id = request.model_id.trim();
    if model_id.is_empty() {
        return Err("测试模型不能为空".to_string());
    }
    if request.api_key.trim().is_empty() {
        return Err("供应商缺少 API Key".to_string());
    }

    let models = model_provider_gateway_test_models(model_id, &request.model_catalog);
    let account_id = model_provider_gateway_test_account_id(&request);
    let account = build_model_provider_gateway_test_account(&request, account_id, models);
    let uses_provider_gateway = model_provider_test_uses_provider_gateway(&request);
    let client_model_id = if uses_provider_gateway {
        model_id.to_string()
    } else {
        model_provider_direct_test_client_model()
    };
    let provider_gateway = if uses_provider_gateway {
        Some(provider_gateway_for_account(&account)?)
    } else {
        None
    };
    let collection = build_model_provider_gateway_test_collection(
        &request,
        &account,
        provider_gateway,
        &client_model_id,
    )?;
    let sidecar_dir = provider_gateway_sidecars_dir()?
        .join(format!("model-provider-test-{}", uuid::Uuid::new_v4()));
    let mut account_overrides = HashMap::new();
    account_overrides.insert(account.id.clone(), account);
    let launch_config = match prepare_sidecar_launch_config_in_dir(
        &collection,
        sidecar_dir.clone(),
        HashMap::new(),
        None,
        account_overrides,
    )
    .await
    {
        Ok(launch_config) => launch_config,
        Err(error) => {
            if sidecar_dir.exists() {
                let _ = std::fs::remove_dir_all(&sidecar_dir);
            }
            return Err(error);
        }
    };

    if is_model_provider_chat_test_cancelled(&run_id) {
        if sidecar_dir.exists() {
            let _ = std::fs::remove_dir_all(&sidecar_dir);
        }
        return Err(MODEL_PROVIDER_CHAT_TEST_CANCELLED_ERROR.to_string());
    }

    let (child, task, bind_host) =
        match spawn_provider_gateway_sidecar(&collection, &launch_config, false).await {
            Ok(runtime) => runtime,
            Err(error) => {
                if sidecar_dir.exists() {
                    let _ = std::fs::remove_dir_all(&sidecar_dir);
                }
                return Err(error);
            }
        };
    if is_model_provider_chat_test_cancelled(&run_id) {
        stop_temporary_provider_gateway_sidecar(
            child,
            task,
            bind_host,
            collection.port,
            sidecar_dir,
        )
        .await;
        return Err(MODEL_PROVIDER_CHAT_TEST_CANCELLED_ERROR.to_string());
    }
    let client_request_id = format!(
        "{}:{}:{}:{}",
        request.run_id.trim(),
        request.provider_id.trim(),
        request.api_key_id.as_deref().unwrap_or_default().trim(),
        model_id
    );
    let (url, body) = if uses_provider_gateway {
        (
            format!("http://{}:{}/v1/responses", bind_host, collection.port),
            json!({
                "model": client_model_id.as_str(),
                "input": [
                    {
                        "type": "message",
                        "role": "user",
                        "content": [
                            {
                                "type": "input_text",
                                "text": request.prompt.as_str()
                            }
                        ]
                    }
                ],
                "instructions": "",
                "store": false,
                "stream": false,
                "max_output_tokens": 256,
                "metadata": {
                    "agtools_source": "codex_model_provider_batch_test",
                    "agtools_test_run_id": request.run_id.as_str(),
                    "agtools_provider_id": request.provider_id.as_str(),
                    "agtools_provider_name": request.provider_name.as_str(),
                    "agtools_provider_api_key_id": request.api_key_id.as_deref(),
                    "agtools_provider_api_key_name": request.api_key_name.as_deref(),
                    "agtools_wire_api": request.wire_api.as_str(),
                    "agtools_test_model": model_id,
                    "agtools_client_model": client_model_id.as_str(),
                    "agtools_client_request_id": client_request_id,
                }
            }),
        )
    } else {
        (
            format!(
                "http://{}:{}/v1/chat/completions",
                bind_host, collection.port
            ),
            json!({
                "model": client_model_id.as_str(),
                "stream": false,
                "messages": [
                    {
                        "role": "user",
                        "content": request.prompt.as_str()
                    }
                ],
                "max_tokens": 256
            }),
        )
    };

    let result = tokio::select! {
        result = async {
            let client =
                build_localhost_http_client(Duration::from_secs(90), "模型供应商临时网关测试")?;
            let started = Instant::now();
            let mut builder = client
                .post(&url)
                .bearer_auth(collection.api_key.trim())
                .header(ACCEPT, "application/json")
                .header(CONTENT_TYPE, "application/json");
            builder =
                add_model_provider_test_header(builder, "x-client-request-id", &client_request_id);
            builder = add_model_provider_test_header(builder, "x-agtools-test-run-id", &request.run_id);
            builder =
                add_model_provider_test_header(builder, "x-agtools-provider-id", &request.provider_id);
            builder = add_model_provider_test_header(builder, "x-agtools-wire-api", &request.wire_api);
            builder = add_model_provider_test_header_b64(
                builder,
                "x-agtools-provider-name-b64",
                &request.provider_name,
            );
            builder = add_model_provider_test_header_b64(
                builder,
                "x-agtools-provider-base-url-b64",
                &request.base_url,
            );
            if let Some(api_key_id) = request.api_key_id.as_deref() {
                builder = add_model_provider_test_header(
                    builder,
                    "x-agtools-provider-api-key-id",
                    api_key_id,
                );
            }
            if let Some(api_key_name) = request.api_key_name.as_deref() {
                builder = add_model_provider_test_header_b64(
                    builder,
                    "x-agtools-provider-api-key-name-b64",
                    api_key_name,
                );
            }
            builder = add_model_provider_test_header(builder, "x-agtools-test-model", model_id);
            builder =
                add_model_provider_test_header(builder, "x-agtools-client-model", &client_model_id);
            let response = builder
                .json(&body)
                .send()
                .await
                .map_err(|error| format!("本地网关对话请求失败: {}", error))?;
            let duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let status = response.status();
            let text = response
                .text()
                .await
                .map_err(|error| format!("读取本地网关对话响应失败: {}", error))?;
            if !status.is_success() {
                return Err(format!(
                    "本地网关对话失败({}): {}",
                    status.as_u16(),
                    extract_provider_gateway_test_error_message(&text)
                ));
            }
            let reply = if uses_provider_gateway {
                let parsed = serde_json::from_str::<Value>(&text)
                    .map_err(|error| format!("解析本地网关对话响应失败: {}", error))?;
                extract_provider_gateway_test_output_text(&parsed)
            } else {
                extract_chat_completion_output(&text).or_else(|| {
                    serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|parsed| extract_provider_gateway_test_output_text(&parsed))
                })
            }
            .ok_or_else(|| "本地网关对话未返回可读回复".to_string())?;
            Ok(CodexModelProviderGatewayChatTestResult { duration_ms, reply })
        } => result,
        _ = wait_for_model_provider_chat_test_cancellation(&run_id) => {
            Err(MODEL_PROVIDER_CHAT_TEST_CANCELLED_ERROR.to_string())
        }
    };

    stop_temporary_provider_gateway_sidecar(child, task, bind_host, collection.port, sidecar_dir)
        .await;
    result
}

fn write_local_access_profile_model_override(
    profile_dir: &Path,
    model: &str,
) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() {
        return Ok(());
    }
    let config_path = profile_config_path(profile_dir);
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 Codex config.toml 失败: {}", e))?
    };
    doc["model"] = value(model);
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
}

fn official_catalog_json_for_provider_gateway(account: &CodexAccount) -> Option<&'static str> {
    if is_official_deepseek_account(account)
        && provider_gateway_wire_api_for_account(account) == "responses"
    {
        Some(codex_account::deepseek_official_models_json())
    } else {
        None
    }
}

fn write_provider_gateway_model_catalog(
    profile_dir: &Path,
    slots: &[ProviderGatewayModelSlot],
) -> Result<(), String> {
    write_provider_gateway_model_catalog_with_templates(profile_dir, slots, None, None)
}

fn write_provider_gateway_model_catalog_with_templates(
    profile_dir: &Path,
    slots: &[ProviderGatewayModelSlot],
    official_catalog_json: Option<&str>,
    account: Option<&CodexAccount>,
) -> Result<(), String> {
    let raw = if let Some(official_catalog_json) = official_catalog_json {
        build_official_template_mapped_catalog_json(slots, official_catalog_json)?
    } else {
        build_provider_model_catalog_json(slots)?
    };
    let config_path = profile_config_path(profile_dir);
    let default_window = read_file_model_context_window(&config_path);
    let content = if let Some(account) = account {
        decorate_account_catalog_context_windows(&raw, slots, account, default_window)?
    } else {
        decorate_catalog_context_windows(&raw, slots, &HashMap::new(), default_window)?
    };
    let content = codex_account::decorate_managed_model_catalog_for_profile(profile_dir, &content)?;
    write_string_atomic(
        &profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE),
        &content,
    )
    .map_err(|e| format!("写入 Codex 模型目录失败: {}", e))?;
    codex_account::cleanup_legacy_managed_model_catalogs(profile_dir);
    invalidate_codex_model_cache(profile_dir)?;

    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 Codex config.toml 失败: {}", e))?
    };
    doc["model_catalog_json"] = value(CODEX_PROVIDER_MODEL_CATALOG_FILE);
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
}

fn provider_model_backup_path(profile_dir: &Path) -> PathBuf {
    crate::modules::backup_storage::behavior_backup_dir(
        "codex",
        &crate::modules::backup_storage::scope_for_path(profile_dir),
        "provider-model",
    )
    .map(|dir| dir.join(CODEX_PROVIDER_MODEL_BACKUP_FILE))
    .unwrap_or_else(|_| profile_dir.join(CODEX_PROVIDER_MODEL_BACKUP_FILE))
}

#[derive(Debug, Default)]
struct ProviderModelOverrideState {
    previous_model: Option<String>,
    managed_models: HashSet<String>,
}

fn read_provider_model_backup(profile_dir: &Path) -> Option<ProviderModelOverrideState> {
    let content = std::fs::read_to_string(provider_model_backup_path(profile_dir)).ok()?;
    let parsed = serde_json::from_str::<Value>(&content).ok()?;
    let previous_model = parsed
        .get("previous_model")
        .or_else(|| parsed.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let managed_models = parsed
        .get("managed_models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    Some(ProviderModelOverrideState {
        previous_model,
        managed_models,
    })
}

fn save_provider_model_backup(
    profile_dir: &Path,
    previous_model: Option<&str>,
    provider_models: &[String],
) -> Result<(), String> {
    let previous_model = previous_model
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let managed_models =
        normalize_provider_gateway_models(provider_models.iter().map(String::as_str).collect());
    if previous_model.is_none() && managed_models.is_empty() {
        return delete_provider_model_backup(profile_dir);
    }
    let content = serde_json::to_string_pretty(&json!({
        "previous_model": previous_model,
        "managed_models": managed_models,
    }))
    .map_err(|e| format!("生成 Codex provider 模型备份失败: {}", e))?;
    write_string_atomic(&provider_model_backup_path(profile_dir), &content)
        .map_err(|e| format!("写入 Codex provider 模型备份失败: {}", e))
}

fn delete_provider_model_backup(profile_dir: &Path) -> Result<(), String> {
    let path = provider_model_backup_path(profile_dir);
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("删除 Codex provider 模型备份失败: {}", e))?;
    }
    Ok(())
}

fn backup_current_profile_model_before_provider_gateway(
    profile_dir: &Path,
    provider_models: &[String],
) -> Result<(), String> {
    let config_path = profile_config_path(profile_dir);
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    if existing.trim().is_empty() {
        return save_provider_model_backup(profile_dir, None, provider_models);
    }
    let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
        .map_err(|e| format!("解析 Codex config.toml 失败: {}", e))?;
    let current_model = doc
        .get("model")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let previous_model = current_model.filter(|model| {
        !provider_models
            .iter()
            .any(|item| item.trim().eq_ignore_ascii_case(model))
    });
    save_provider_model_backup(profile_dir, previous_model, provider_models)
}

pub fn cleanup_provider_gateway_profile_model_overrides(profile_dir: &Path) -> Result<(), String> {
    let catalog_path = profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE);
    let override_state = read_provider_model_backup(profile_dir).unwrap_or_default();
    let previous_model = override_state.previous_model;
    let mut managed_models = override_state.managed_models;
    for file_name in [
        CODEX_MANAGED_MODEL_CATALOG_FILE,
        CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE,
        CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE,
    ] {
        if let Ok(content) = std::fs::read_to_string(profile_dir.join(file_name)) {
            if let Ok(parsed) = serde_json::from_str::<Value>(&content) {
                if let Some(models) = parsed.get("models").and_then(Value::as_array) {
                    for model in models {
                        if let Some(slug) = model.get("slug").and_then(Value::as_str) {
                            let slug = slug.trim();
                            if !slug.is_empty() {
                                managed_models.insert(slug.to_ascii_lowercase());
                            }
                        }
                    }
                }
            }
        }
    }

    let config_path = profile_config_path(profile_dir);
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    if !existing.trim().is_empty() {
        let mut doc =
            crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
                .map_err(|e| format!("解析 Codex config.toml 失败: {}", e))?;
        let mut changed = false;
        let uses_managed_catalog = doc
            .get("model_catalog_json")
            .and_then(|item| item.as_str())
            .is_some_and(is_cockpit_managed_model_catalog_name);
        if uses_managed_catalog {
            doc.remove("model_catalog_json");
            changed = true;
        }
        if let Some(previous_model) = previous_model.as_deref() {
            doc["model"] = value(previous_model);
            changed = true;
        } else {
            let current_model = doc
                .get("model")
                .and_then(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase);
            if let Some(model) = current_model {
                if managed_models.contains(&model) {
                    doc.remove("model");
                    changed = true;
                }
            }
        }
        if changed {
            let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
            crate::modules::codex_config_format::write_codex_config_toml_atomic(
                &config_path,
                &content,
            )
            .map_err(|e| format!("写入 Codex config.toml 失败: {}", e))?;
        }
    }

    if catalog_path.exists() {
        std::fs::remove_file(&catalog_path)
            .map_err(|e| format!("删除 Codex provider 模型目录失败: {}", e))?;
    }
    codex_account::cleanup_legacy_managed_model_catalogs(profile_dir);
    delete_provider_model_backup(profile_dir)?;
    Ok(())
}

pub async fn activate_provider_gateway_for_dir(
    profile_dir: &Path,
    account_id: &str,
) -> Result<CodexLocalAccessState, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err("供应商网关账号不能为空".to_string());
    }

    let account = codex_account::load_account(account_id)
        .ok_or_else(|| format!("供应商网关账号不存在: {}", account_id))?;
    let (collection, key, provider_gateway) =
        build_provider_gateway_collection_for_profile(profile_dir, &account)?;
    let model_slots = provider_gateway_model_slots(&provider_gateway.upstream_models);
    save_profile_takeover_backup(profile_dir, &key)?;
    write_local_access_profile_takeover(profile_dir, &collection, Some(&key)).await?;
    cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
    backup_current_profile_model_before_provider_gateway(
        profile_dir,
        &model_slots
            .iter()
            .map(|slot| slot.client_model.clone())
            .collect::<Vec<_>>(),
    )?;
    if let Some(default_slot) = preferred_provider_gateway_slot(&account, &model_slots) {
        write_local_access_profile_model_override(profile_dir, &default_slot.client_model)?;
    }
    if !model_slots.is_empty() {
        write_provider_gateway_model_catalog_with_templates(
            profile_dir,
            &model_slots,
            official_catalog_json_for_provider_gateway(&account),
            Some(&account),
        )?;
    }
    codex_account::reapply_experimental_model_policy_if_enabled(profile_dir)?;
    ensure_runtime_loaded_without_start().await?;
    let runtime = gateway_runtime().lock().await;
    Ok(build_state_snapshot(&runtime))
}

fn provider_gateway_sidecar_parent_pid(persistent_after_host_exit: bool) -> u32 {
    if persistent_after_host_exit {
        0
    } else {
        std::process::id()
    }
}

async fn spawn_provider_gateway_sidecar(
    collection: &CodexLocalAccessCollection,
    launch_config: &SidecarLaunchConfig,
    persistent_after_host_exit: bool,
) -> Result<(Child, tokio::task::JoinHandle<()>, String), String> {
    let bind_host = bind_host_for_collection(collection);
    let binary = sidecar_binary_path()?;
    let mut command = TokioCommand::new(&binary);
    sanitize_sidecar_command_env(&mut command);
    command
        .arg("--config")
        .arg(&launch_config.config_path)
        .arg("--manifest")
        .arg(&launch_config.manifest_path)
        .arg("--quota-reserve-state")
        .arg(&launch_config.quota_reserve_path)
        .arg("--quota-pool-state")
        .arg(&launch_config.quota_pool_path)
        .arg("--parent-pid")
        .arg(provider_gateway_sidecar_parent_pid(persistent_after_host_exit).to_string())
        .current_dir(
            launch_config
                .config_path
                .parent()
                .unwrap_or_else(|| Path::new(".")),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("启动 Codex provider gateway sidecar 失败: {}", e))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (ready_sender, mut ready_receiver) = oneshot::channel();
    let startup_diagnostics = Arc::new(Mutex::new(SidecarStartupDiagnostics::default()));
    let task_startup_diagnostics = Arc::clone(&startup_diagnostics);
    let task = tokio::spawn(async move {
        let stdout_diagnostics = Arc::clone(&task_startup_diagnostics);
        let stderr_diagnostics = Arc::clone(&task_startup_diagnostics);
        let stdout_task = stdout.map(|stdout| {
            tokio::spawn(drain_sidecar_stdout(
                stdout,
                ready_sender,
                stdout_diagnostics,
            ))
        });
        let stderr_task =
            stderr.map(|stderr| tokio::spawn(drain_sidecar_stderr(stderr, stderr_diagnostics)));
        if let Some(task) = stdout_task {
            let _ = task.await;
        }
        if let Some(task) = stderr_task {
            let _ = task.await;
        }
    });

    let ready_signal = match wait_for_sidecar_ready(&mut ready_receiver, &mut child, None).await {
        Ok(signal) => signal,
        Err(error) => {
            let diagnostics = sidecar_startup_diagnostics_text(&startup_diagnostics);
            let message = format!("{}; {}", error, diagnostics);
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][provider-gateway] sidecar ready 等待失败，将停止进程: {}",
                message
            ));
            let _ = child.kill().await;
            task.abort();
            let _ = task.await;
            return Err(message);
        }
    };

    if let Some(ready_port) = ready_signal.port {
        if ready_port != collection.port {
            let message = format!(
                "Codex provider gateway sidecar ready 端口不一致: expected={}, actual={}, host={}",
                collection.port, ready_port, ready_signal.host
            );
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][provider-gateway] sidecar ready 校验失败，将停止进程: {}",
                message
            ));
            let _ = child.kill().await;
            task.abort();
            let _ = task.await;
            return Err(message);
        }
    } else {
        let message = format!(
            "Codex provider gateway sidecar ready 事件缺少端口: host={}",
            ready_signal.host
        );
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][provider-gateway] sidecar ready 校验失败，将停止进程: {}",
            message
        ));
        let _ = child.kill().await;
        task.abort();
        let _ = task.await;
        return Err(message);
    }

    log_sidecar_proxy_signature(&launch_config.proxy_signature);
    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][provider-gateway] sidecar 已启动: bin={} bind={}:{} base={}",
        binary.display(),
        bind_host,
        collection.port,
        build_base_url(collection.port)
    ));

    Ok((child, task, bind_host.to_string()))
}

async fn stop_provider_gateway_runtime(runtime_key: &str) -> Option<GatewayBindEndpoint> {
    let (child, task, endpoint) = {
        let mut runtimes = provider_gateway_runtime_store().lock().await;
        let Some(mut runtime) = runtimes.remove(runtime_key) else {
            return None;
        };
        let endpoint = runtime
            .actual_port
            .zip(runtime.actual_bind_host.clone())
            .map(|(port, bind_host)| GatewayBindEndpoint { bind_host, port });
        (runtime.sidecar_child.take(), runtime.task.take(), endpoint)
    };

    if let Some(mut child) = child {
        match timeout(GATEWAY_SHUTDOWN_TIMEOUT, child.kill()).await {
            Ok(Ok(())) => {
                let _ = child.wait().await;
            }
            Ok(Err(error)) => {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess][provider-gateway] 停止 sidecar 失败: {}",
                    error
                ));
            }
            Err(_) => {
                logger::log_codex_api_warn(
                    "[CodexLocalAccess][provider-gateway] 停止 sidecar 超时",
                );
            }
        }
    }
    if let Some(mut task) = task {
        tokio::select! {
            result = &mut task => {
                let _ = result;
            }
            _ = tokio::time::sleep(GATEWAY_SHUTDOWN_TIMEOUT) => {
                logger::log_codex_api_warn("[CodexLocalAccess][provider-gateway] 停止监听任务超时，已中止");
                task.abort();
            }
        }
    }
    endpoint
}

async fn stop_spawned_provider_gateway_sidecar(
    mut child: Child,
    mut task: tokio::task::JoinHandle<()>,
    bind_host: &str,
    port: u16,
) {
    match timeout(GATEWAY_SHUTDOWN_TIMEOUT, child.kill()).await {
        Ok(Ok(())) => {
            let _ = child.wait().await;
        }
        Ok(Err(error)) => logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][mixed-model-routing] 启动回滚停止 sidecar 失败: {}",
            error
        )),
        Err(_) => logger::log_codex_api_warn(
            "[CodexLocalAccess][mixed-model-routing] 启动回滚停止 sidecar 超时",
        ),
    }
    tokio::select! {
        result = &mut task => {
            let _ = result;
        }
        _ = tokio::time::sleep(GATEWAY_SHUTDOWN_TIMEOUT) => {
            task.abort();
        }
    }
    if let Err(error) = wait_for_gateway_port_release(bind_host, port).await {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][mixed-model-routing] 启动回滚等待端口释放失败: bind={}:{} error={}",
            bind_host, port, error
        ));
    }
}

pub async fn stop_provider_gateways_for_profile(profile_dir: &Path) {
    let _guard = provider_gateway_lifecycle_lock().lock().await;
    stop_provider_gateways_for_profile_locked(profile_dir).await;
}

async fn stop_persisted_mixed_model_gateway(profile_dir: &Path) -> Option<GatewayBindEndpoint> {
    let Ok(Some((endpoint, api_key))) = persisted_mixed_model_gateway_endpoint(profile_dir) else {
        return None;
    };
    if probe_sidecar_ready_endpoint(endpoint.port, &api_key, Duration::from_millis(500))
        .await
        .is_err()
    {
        return None;
    }
    match process::kill_port_processes(endpoint.port) {
        Ok(killed) => {
            if killed > 0 {
                logger::log_codex_api_info(&format!(
                    "[CodexLocalAccess][mixed-model-routing] 已停止持久 sidecar: profile={} port={} killed={}",
                    profile_dir.display(),
                    endpoint.port,
                    killed
                ));
            }
            Some(endpoint)
        }
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][mixed-model-routing] 停止持久 sidecar 失败: profile={} port={} error={}",
                profile_dir.display(),
                endpoint.port,
                error
            ));
            None
        }
    }
}

async fn stop_provider_gateways_for_profile_locked(profile_dir: &Path) {
    let profile_prefix = format!("{}\n", normalize_profile_dir_key(profile_dir));
    let runtime_keys = {
        let runtimes = provider_gateway_runtime_store().lock().await;
        runtimes
            .keys()
            .filter(|key| key.starts_with(&profile_prefix))
            .cloned()
            .collect::<Vec<_>>()
    };
    for runtime_key in runtime_keys {
        if let Some(endpoint) = stop_provider_gateway_runtime(&runtime_key).await {
            if let Err(error) =
                wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess][provider-gateway] 等待端口释放失败: bind={}:{} error={}",
                    endpoint.bind_host, endpoint.port, error
                ));
            }
        }
    }
    if let Some(endpoint) = stop_persisted_mixed_model_gateway(profile_dir).await {
        if let Err(error) = wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][mixed-model-routing] 等待持久 sidecar 释放端口失败: bind={}:{} error={}",
                endpoint.bind_host, endpoint.port, error
            ));
        }
    }
}

#[cfg(target_os = "windows")]
pub fn has_running_persisted_mixed_model_gateway() -> bool {
    let default_running = crate::modules::codex_instance::load_default_settings()
        .ok()
        .zip(crate::modules::codex_instance::get_default_codex_home().ok())
        .is_some_and(|(settings, profile_dir)| {
            crate::modules::process::resolve_codex_pid(settings.last_pid, None).is_some()
                && persisted_mixed_model_gateway_endpoint(&profile_dir)
                    .ok()
                    .flatten()
                    .is_some()
        });
    if default_running {
        return true;
    }
    crate::modules::codex_instance::load_instance_store()
        .ok()
        .is_some_and(|store| {
            store.instances.into_iter().any(|instance| {
                let running = crate::modules::process::resolve_codex_pid(
                    instance.last_pid,
                    Some(&instance.user_data_dir),
                )
                .is_some();
                running
                    && persisted_mixed_model_gateway_endpoint(Path::new(&instance.user_data_dir))
                        .ok()
                        .flatten()
                        .is_some()
            })
        })
}

async fn stop_all_provider_gateways_for_app_shutdown() -> Vec<GatewayBindEndpoint> {
    let _guard = provider_gateway_lifecycle_lock().lock().await;
    let mut preserve_mixed_profiles = HashSet::new();
    let mut configured_profiles = HashMap::new();
    if let Ok(default_settings) = crate::modules::codex_instance::load_default_settings() {
        if let Ok(profile_dir) = crate::modules::codex_instance::get_default_codex_home() {
            configured_profiles.insert(normalize_profile_dir_key(&profile_dir), profile_dir.clone());
        }
        if crate::modules::process::resolve_codex_pid(default_settings.last_pid, None).is_some() {
            if let Ok(profile_dir) = crate::modules::codex_instance::get_default_codex_home() {
                preserve_mixed_profiles.insert(normalize_profile_dir_key(&profile_dir));
            }
        }
    }
    if let Ok(store) = crate::modules::codex_instance::load_instance_store() {
        for instance in store.instances {
            let profile_dir = PathBuf::from(&instance.user_data_dir);
            configured_profiles.insert(
                normalize_profile_dir_key(&profile_dir),
                profile_dir,
            );
            let running = crate::modules::process::resolve_codex_pid(
                instance.last_pid,
                Some(&instance.user_data_dir),
            )
            .is_some();
            if running
            {
                preserve_mixed_profiles
                    .insert(normalize_profile_dir_key(Path::new(&instance.user_data_dir)));
            }
        }
    }
    let runtime_keys = {
        let runtimes = provider_gateway_runtime_store().lock().await;
        runtimes
            .keys()
            .filter(|runtime_key| {
                let Some((profile_key, runtime_id)) = runtime_key.rsplit_once('\n') else {
                    return true;
                };
                let preserve = runtime_id == MIXED_MODEL_ROUTING_RUNTIME_ID
                    && preserve_mixed_profiles.contains(profile_key);
                if preserve {
                    logger::log_codex_api_info(&format!(
                        "[CodexLocalAccess][mixed-model-routing] Codex 仍在运行，应用退出后保留 sidecar: profile={}",
                        profile_key
                    ));
                }
                !preserve
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    if !runtime_keys.is_empty() {
        logger::log_codex_api_info(&format!(
            "[CodexLocalAccess][provider-gateway] 应用关闭前停止全部 sidecar: count={}",
            runtime_keys.len()
        ));
    }

    let mut endpoints = Vec::new();
    for runtime_key in runtime_keys {
        if let Some(endpoint) = stop_provider_gateway_runtime(&runtime_key).await {
            endpoints.push(endpoint);
        }
    }
    for (profile_key, profile_dir) in configured_profiles {
        if preserve_mixed_profiles.contains(&profile_key) {
            continue;
        }
        if let Some(endpoint) = stop_persisted_mixed_model_gateway(&profile_dir).await {
            endpoints.push(endpoint);
        }
    }
    endpoints
}

pub async fn mixed_model_gateway_runtime_is_healthy(profile_dir: &Path) -> bool {
    let runtime_key = provider_gateway_runtime_key(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID);
    let collection = {
        let mut runtimes = provider_gateway_runtime_store().lock().await;
        runtimes.get_mut(&runtime_key).and_then(|runtime| {
            let child_running = runtime
                .sidecar_child
                .as_mut()
                .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
            child_running.then(|| runtime.collection.clone()).flatten()
        })
    };
    if let Some(collection) = collection {
        if probe_sidecar_ready_once(&collection, Duration::from_millis(500))
            .await
            .is_ok()
        {
            return true;
        }
    }
    persisted_mixed_model_gateway_is_healthy(profile_dir).await
}

pub async fn mixed_model_gateway_runtime_is_managed(profile_dir: &Path) -> bool {
    let runtime_key = provider_gateway_runtime_key(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID);
    let mut runtimes = provider_gateway_runtime_store().lock().await;
    runtimes.get_mut(&runtime_key).is_some_and(|runtime| {
        runtime
            .sidecar_child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    })
}

pub async fn ensure_provider_gateway_for_dir(
    profile_dir: &Path,
    account_id: &str,
) -> Result<(), String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err("供应商网关账号不能为空".to_string());
    }

    let _guard = provider_gateway_lifecycle_lock().lock().await;
    let account = codex_account::load_account(account_id)
        .ok_or_else(|| format!("供应商网关账号不存在: {}", account_id))?;
    let (collection, key, provider_gateway) =
        build_provider_gateway_collection_for_profile(profile_dir, &account)?;
    let model_slots = provider_gateway_model_slots(&provider_gateway.upstream_models);
    save_profile_takeover_backup(profile_dir, &key)?;
    write_local_access_profile_takeover(profile_dir, &collection, Some(&key)).await?;
    cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
    backup_current_profile_model_before_provider_gateway(
        profile_dir,
        &model_slots
            .iter()
            .map(|slot| slot.client_model.clone())
            .collect::<Vec<_>>(),
    )?;
    if let Some(default_slot) = preferred_provider_gateway_slot(&account, &model_slots) {
        write_local_access_profile_model_override(profile_dir, &default_slot.client_model)?;
    }
    if !model_slots.is_empty() {
        write_provider_gateway_model_catalog_with_templates(
            profile_dir,
            &model_slots,
            official_catalog_json_for_provider_gateway(&account),
            Some(&account),
        )?;
    }
    codex_account::reapply_experimental_model_policy_if_enabled(profile_dir)?;

    let runtime_key = provider_gateway_runtime_key(profile_dir, account_id);
    if let Some(endpoint) = stop_provider_gateway_runtime(&runtime_key).await {
        wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
    }

    let sidecar_dir = provider_gateway_sidecar_dir(profile_dir, account_id)?;
    let runtime_sidecar_dir = sidecar_dir.clone();
    let default_service_tier =
        crate::modules::codex_speed::get_app_speed_config_for_dir(profile_dir)
            .map(|config| codex_app_speed_service_tier(&config.speed))?;
    let launch_config = prepare_sidecar_launch_config_in_dir(
        &collection,
        sidecar_dir,
        HashMap::new(),
        default_service_tier,
        HashMap::new(),
    )
    .await?;
    if probe_sidecar_ready_once(&collection, Duration::from_millis(250))
        .await
        .is_ok()
    {
        let killed = process::kill_port_processes(collection.port)?;
        if killed > 0 {
            logger::log_codex_api_info(&format!(
                "[CodexLocalAccess][provider-gateway] 已停止旧 sidecar: port={}, killed={}",
                collection.port, killed
            ));
        }
        wait_for_gateway_port_release(bind_host_for_collection(&collection), collection.port)
            .await?;
    }

    let (child, task, bind_host) =
        spawn_provider_gateway_sidecar(&collection, &launch_config, false).await?;
    let mut runtimes = provider_gateway_runtime_store().lock().await;
    runtimes.insert(
        runtime_key,
        ProviderGatewayRuntime {
            actual_port: Some(collection.port),
            actual_bind_host: Some(bind_host),
            task: Some(task),
            sidecar_child: Some(child),
            sidecar_dir: Some(runtime_sidecar_dir),
            collection: Some(collection),
            oauth_account_ids: Vec::new(),
        },
    );
    Ok(())
}

pub async fn ensure_mixed_model_gateway_for_dir(
    profile_dir: &Path,
    oauth_account_id: &str,
    routing: &CodexInstanceModelRouting,
) -> Result<(), String> {
    let oauth_account_id = oauth_account_id.trim();
    if oauth_account_id.is_empty() {
        return Err("混合模型路由缺少 OAuth 订阅账号".to_string());
    }

    let oauth_account = validate_local_access_bound_oauth_account(oauth_account_id)?;
    let routing = validate_mixed_model_routing_config(Some(oauth_account_id), routing)?;
    let _guard = provider_gateway_lifecycle_lock().lock().await;
    let (collection, key) =
        build_mixed_model_gateway_collection_for_profile(profile_dir, &oauth_account, &routing)?;
    stop_provider_gateways_for_profile_locked(profile_dir).await;
    let runtime_key = provider_gateway_runtime_key(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID);

    let sidecar_dir = provider_gateway_sidecar_dir(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID)
        .map_err(|error| mixed_model_start_error_with_rollback(profile_dir, error))?;
    let runtime_sidecar_dir = sidecar_dir.clone();
    let runtime_oauth_account_id = oauth_account.id.clone();
    let default_service_tier =
        crate::modules::codex_speed::get_app_speed_config_for_dir(profile_dir)
            .map(|config| codex_app_speed_service_tier(&config.speed))
            .map_err(|error| mixed_model_start_error_with_rollback(profile_dir, error))?;
    let launch_config = match prepare_sidecar_launch_config_in_dir(
        &collection,
        sidecar_dir,
        HashMap::new(),
        default_service_tier,
        HashMap::from([(oauth_account.id.clone(), oauth_account)]),
    )
    .await
    {
        Ok(config) => config,
        Err(error) => return Err(mixed_model_start_error_with_rollback(profile_dir, error)),
    };
    if probe_sidecar_ready_once(&collection, Duration::from_millis(250))
        .await
        .is_ok()
    {
        let killed = process::kill_port_processes(collection.port)
            .map_err(|error| mixed_model_start_error_with_rollback(profile_dir, error))?;
        if killed > 0 {
            logger::log_codex_api_info(&format!(
                "[CodexLocalAccess][mixed-model-routing] 已停止旧 sidecar: port={}, killed={}",
                collection.port, killed
            ));
        }
        wait_for_gateway_port_release(bind_host_for_collection(&collection), collection.port)
            .await
            .map_err(|error| mixed_model_start_error_with_rollback(profile_dir, error))?;
    }

    let (child, task, bind_host) =
        match spawn_provider_gateway_sidecar(&collection, &launch_config, true).await {
            Ok(runtime) => runtime,
            Err(error) => return Err(mixed_model_start_error_with_rollback(profile_dir, error)),
        };

    let activation_snapshot =
        match capture_mixed_model_profile_activation_snapshot(profile_dir, &key) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                stop_spawned_provider_gateway_sidecar(child, task, &bind_host, collection.port)
                    .await;
                return Err(mixed_model_start_error_with_rollback(profile_dir, error));
            }
        };

    let takeover_result = async {
        save_profile_takeover_backup(profile_dir, &key)?;
        cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
        write_local_access_profile_takeover(profile_dir, &collection, Some(&key)).await?;
        codex_account::reapply_experimental_model_policy_if_enabled(profile_dir)
    }
    .await;
    if let Err(error) = takeover_result {
        stop_spawned_provider_gateway_sidecar(child, task, &bind_host, collection.port).await;
        if let Err(rollback_error) =
            rollback_mixed_model_profile_after_start_failure(profile_dir, Some(activation_snapshot))
        {
            return Err(format!("{}; 启动失败回滚也失败: {}", error, rollback_error));
        }
        return Err(error);
    }

    let mut runtimes = provider_gateway_runtime_store().lock().await;
    runtimes.insert(
        runtime_key,
        ProviderGatewayRuntime {
            actual_port: Some(collection.port),
            actual_bind_host: Some(bind_host),
            task: Some(task),
            sidecar_child: Some(child),
            sidecar_dir: Some(runtime_sidecar_dir),
            collection: Some(collection),
            oauth_account_ids: vec![runtime_oauth_account_id],
        },
    );
    Ok(())
}

pub async fn ensure_bound_oauth_local_gateway_for_dir(
    profile_dir: &Path,
    account_id: &str,
) -> Result<(), String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err("绑定 OAuth 本地网关账号不能为空".to_string());
    }

    let _guard = provider_gateway_lifecycle_lock().lock().await;
    let account = codex_account::load_account(account_id)
        .ok_or_else(|| format!("绑定 OAuth 本地网关账号不存在: {}", account_id))?;
    let (collection, key) =
        build_bound_oauth_local_gateway_collection_for_profile(profile_dir, &account)?;
    save_profile_takeover_backup(profile_dir, &key)?;
    write_local_access_profile_takeover(profile_dir, &collection, Some(&key)).await?;
    cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
    codex_account::reapply_experimental_model_policy_if_enabled(profile_dir)?;

    let runtime_key = provider_gateway_runtime_key(profile_dir, account_id);
    if let Some(endpoint) = stop_provider_gateway_runtime(&runtime_key).await {
        wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
    }

    let sidecar_dir = provider_gateway_sidecar_dir(profile_dir, account_id)?;
    let runtime_sidecar_dir = sidecar_dir.clone();
    let runtime_oauth_account_id = account.id.clone();
    let default_service_tier =
        crate::modules::codex_speed::get_app_speed_config_for_dir(profile_dir)
            .map(|config| codex_app_speed_service_tier(&config.speed))?;
    let launch_config = prepare_sidecar_launch_config_in_dir(
        &collection,
        sidecar_dir,
        HashMap::new(),
        default_service_tier,
        HashMap::from([(account.id.clone(), account)]),
    )
    .await?;
    if probe_sidecar_ready_once(&collection, Duration::from_millis(250))
        .await
        .is_ok()
    {
        let killed = process::kill_port_processes(collection.port)?;
        if killed > 0 {
            logger::log_codex_api_info(&format!(
                "[CodexLocalAccess][bound-oauth-local-gateway] 已停止旧 sidecar: port={}, killed={}",
                collection.port, killed
            ));
        }
        wait_for_gateway_port_release(bind_host_for_collection(&collection), collection.port)
            .await?;
    }

    let (child, task, bind_host) =
        spawn_provider_gateway_sidecar(&collection, &launch_config, false).await?;
    let mut runtimes = provider_gateway_runtime_store().lock().await;
    runtimes.insert(
        runtime_key,
        ProviderGatewayRuntime {
            actual_port: Some(collection.port),
            actual_bind_host: Some(bind_host),
            task: Some(task),
            sidecar_child: Some(child),
            sidecar_dir: Some(runtime_sidecar_dir),
            collection: Some(collection),
            oauth_account_ids: vec![runtime_oauth_account_id],
        },
    );
    Ok(())
}

fn sync_provider_gateway_runtime_auth_file(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    sidecar_dir: &Path,
) -> Result<bool, String> {
    let auth_path = sidecar_auths_dir(sidecar_dir).join(sidecar_auth_file_name(&account.id));
    if !auth_path.exists() {
        return Ok(false);
    }
    let proxy_signature = sidecar_effective_proxy_signature(collection)?;
    let auth_json =
        sidecar_auth_json_for_account(account, collection, proxy_signature.proxy_url.as_deref());
    let auth_content = serde_json::to_string_pretty(&auth_json)
        .map_err(|error| format!("序列化实例 sidecar OAuth 认证失败: {}", error))?;
    let changed = write_string_atomic_if_changed(&auth_path, &auth_content)?;
    harden_sidecar_auth_file_permissions(&auth_path)?;
    Ok(changed)
}

pub fn sync_provider_gateway_auth_files_for_account_in_background(account: CodexAccount) {
    tauri::async_runtime::spawn(async move {
        let targets = {
            let runtimes = provider_gateway_runtime_store().lock().await;
            runtimes
                .values()
                .filter(|runtime| {
                    runtime
                        .oauth_account_ids
                        .iter()
                        .any(|account_id| account_id == &account.id)
                })
                .filter_map(|runtime| {
                    Some((runtime.collection.clone()?, runtime.sidecar_dir.clone()?))
                })
                .collect::<Vec<_>>()
        };

        for (collection, sidecar_dir) in targets {
            match sync_provider_gateway_runtime_auth_file(&account, &collection, &sidecar_dir) {
                Ok(true) => logger::log_codex_api_info(&format!(
                    "[CodexLocalAccess][provider-gateway] 已写穿实例 sidecar OAuth 凭证: account_id={}, sidecar_dir={}",
                    account.id,
                    sidecar_dir.display()
                )),
                Ok(false) => {}
                Err(error) => logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess][provider-gateway] 写穿实例 sidecar OAuth 凭证失败: account_id={}, sidecar_dir={}, error={}",
                    account.id,
                    sidecar_dir.display(),
                    error
                )),
            }
        }
    });
}

pub fn reload_provider_gateway_for_profile_in_background(
    profile_dir: PathBuf,
    account_id: String,
    reason: &'static str,
) {
    let account_id = account_id.trim().to_string();
    if account_id.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let runtime_key = provider_gateway_runtime_key(&profile_dir, &account_id);
        let is_running = {
            let runtimes = provider_gateway_runtime_store().lock().await;
            runtimes.contains_key(&runtime_key)
        };
        if !is_running {
            return;
        }
        let result = match codex_account::load_account(&account_id) {
            Some(account) if account_requires_provider_gateway(&account) => {
                ensure_provider_gateway_for_dir(&profile_dir, &account_id).await
            }
            Some(account) if account_requires_bound_oauth_local_gateway(&account) => {
                ensure_bound_oauth_local_gateway_for_dir(&profile_dir, &account_id).await
            }
            Some(_) => Ok(()),
            None => Err(format!("账号不存在: {}", account_id)),
        };
        match result {
            Ok(()) => logger::log_codex_api_info(&format!(
                "[CodexLocalAccess][provider-gateway] sidecar 重载完成: reason={}, profile={}, account_id={}",
                reason,
                profile_dir.display(),
                account_id
            )),
            Err(error) => logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][provider-gateway] sidecar 重载失败: reason={}, profile={}, account_id={}, error={}",
                reason,
                profile_dir.display(),
                account_id,
                error
            )),
        }
    });
}
