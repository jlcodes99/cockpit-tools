// cockpit-core Codex 账号：Official authority synchronization and projection persistence。
// 通过 include! 保持原模块作用域和凭据调用路径。
#[derive(Debug, Clone)]
struct LocalCodexOAuthSnapshot {
    tokens: CodexTokens,
    email: String,
    account_id: Option<String>,
    organization_id: Option<String>,
}

fn build_local_oauth_snapshot(tokens: CodexAuthTokens) -> Option<LocalCodexOAuthSnapshot> {
    let (email, _, _, id_token_account_id, id_token_org_id) =
        extract_user_info(&tokens.id_token).ok()?;
    let account_id = normalize_optional_value(
        tokens
            .account_id
            .clone()
            .or_else(|| extract_chatgpt_account_id_from_access_token(&tokens.access_token))
            .or(id_token_account_id),
    );
    let organization_id = normalize_optional_value(
        extract_chatgpt_organization_id_from_access_token(&tokens.access_token).or(id_token_org_id),
    );

    Some(LocalCodexOAuthSnapshot {
        tokens: CodexTokens {
            id_token: tokens.id_token,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
        },
        email,
        account_id,
        organization_id,
    })
}

fn load_local_oauth_snapshot_from_dir(base_dir: &Path) -> Option<LocalCodexOAuthSnapshot> {
    let auth_path = base_dir.join("auth.json");
    if !auth_path.exists() {
        return None;
    }

    let content = fs::read_to_string(&auth_path).ok()?;
    let auth_file: CodexAuthFile = serde_json::from_str(&content).ok()?;
    if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
        return None;
    }

    build_local_oauth_snapshot(auth_file.tokens?)
}

fn local_oauth_snapshot_matches_account(
    snapshot: &LocalCodexOAuthSnapshot,
    account: &CodexAccount,
) -> bool {
    if !account.email.eq_ignore_ascii_case(&snapshot.email) {
        return false;
    }

    let expected_id = build_account_storage_id(
        &snapshot.email,
        snapshot.account_id.as_deref(),
        snapshot.organization_id.as_deref(),
    );
    if account.id == expected_id {
        return true;
    }

    if let Some(account_id) = snapshot.account_id.as_deref() {
        if normalize_optional_ref(account.account_id.as_deref()).as_deref() != Some(account_id) {
            return false;
        }
    }

    if let Some(organization_id) = snapshot.organization_id.as_deref() {
        if normalize_optional_ref(account.organization_id.as_deref()).as_deref()
            != Some(organization_id)
        {
            return false;
        }
    }

    true
}

fn apply_local_oauth_snapshot(
    account: &mut CodexAccount,
    snapshot: &LocalCodexOAuthSnapshot,
) -> bool {
    let mut changed = false;
    let mut token_changed = false;

    if account.tokens.id_token != snapshot.tokens.id_token {
        account.tokens.id_token = snapshot.tokens.id_token.clone();
        changed = true;
        token_changed = true;
    }

    if account.tokens.access_token != snapshot.tokens.access_token {
        account.tokens.access_token = snapshot.tokens.access_token.clone();
        changed = true;
        token_changed = true;
    }

    if let Some(refresh_token) = normalize_optional_ref(snapshot.tokens.refresh_token.as_deref()) {
        if account.tokens.refresh_token.as_deref() != Some(refresh_token.as_str()) {
            account.tokens.refresh_token = Some(refresh_token);
            changed = true;
            token_changed = true;
        }
    }

    if normalize_optional_ref(account.account_id.as_deref()) != snapshot.account_id {
        account.account_id = snapshot.account_id.clone();
        changed = true;
    }

    if normalize_optional_ref(account.organization_id.as_deref()) != snapshot.organization_id {
        account.organization_id = snapshot.organization_id.clone();
        changed = true;
    }

    if token_changed {
        mark_token_chain_updated(account);
    }

    changed
}

fn sync_account_from_auth_dir_if_current(
    account: &mut CodexAccount,
    base_dir: &Path,
) -> Result<bool, String> {
    let Some(snapshot) = load_local_oauth_snapshot_from_dir(base_dir) else {
        return Ok(false);
    };

    if !local_oauth_snapshot_matches_account(&snapshot, account) {
        return Ok(false);
    }

    if apply_local_oauth_snapshot(account, &snapshot) {
        save_account(account)?;
        logger::log_info(&format!(
            "Codex 账号已从本地 auth.json 同步最新 Token: account_id={}, source_dir={}",
            account.id,
            base_dir.display()
        ));
    }

    Ok(true)
}

/// 显式导入/同步入口：只在用户主动选择从指定目录回读时使用，业务主路径禁止自动调用。
pub fn sync_account_from_auth_dir(
    account_id: &str,
    base_dir: &Path,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        return Ok(account);
    }

    let _ = sync_account_from_auth_dir_if_current(&mut account, base_dir)?;
    Ok(account)
}

pub fn sync_managed_projection_from_auth_dir(
    account_id: &str,
    base_dir: &Path,
) -> Result<CodexAccount, String> {
    let projection = read_managed_projection_from_dir(base_dir)
        .ok_or_else(|| "目标目录不是 Cockpit 受管 Codex 投影，已拒绝反向同步".to_string())?;
    if projection.account_id != account_id {
        return Err(format!(
            "受管投影账号不匹配: expected={}, actual={}",
            account_id, projection.account_id
        ));
    }

    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        return Ok(account);
    }
    if account.token_generation != projection.token_generation {
        return Err(format!(
            "受管投影版本已过期，跳过反向同步: account_id={}, store_generation={}, projection_generation={}",
            account_id, account.token_generation, projection.token_generation
        ));
    }

    let snapshot = load_local_oauth_snapshot_from_dir(base_dir)
        .ok_or_else(|| "受管投影缺少可同步的 OAuth Token".to_string())?;
    if !local_oauth_snapshot_matches_account(&snapshot, &account) {
        return Err("受管投影 Token 与账号不匹配，已拒绝反向同步".to_string());
    }

    if apply_local_oauth_snapshot(&mut account, &snapshot) {
        save_account(&account)?;
        write_account_bundle_to_dir(base_dir, &account)?;
        write_managed_account_projections(&account);
        logger::log_info(&format!(
            "Codex 受管投影已同步回账号库: account_id={}, generation={}, source_dir={}",
            account.id,
            account.token_generation,
            base_dir.display()
        ));
    }

    Ok(account)
}

fn sync_api_key_account_from_local_state(account: &mut CodexAccount, base_dir: &Path) {
    let auth_path = base_dir.join("auth.json");
    if !auth_path.exists() || !account.is_api_key_auth() {
        return;
    }

    let Ok(content) = fs::read_to_string(&auth_path) else {
        return;
    };
    let Ok(auth_file) = serde_json::from_str::<CodexAuthFile>(&content) else {
        return;
    };
    let is_apikey_mode = is_auth_mode_apikey(auth_file.auth_mode.as_deref());
    let local_api_key = extract_api_key_from_auth_file(&auth_file);
    if !(is_apikey_mode || (auth_file.tokens.is_none() && local_api_key.is_some())) {
        return;
    }

    let Some(local_api_key) = normalize_optional_ref(local_api_key.as_deref()) else {
        return;
    };
    let Some(account_api_key) = normalize_optional_ref(account.openai_api_key.as_deref()) else {
        return;
    };
    if local_api_key != account_api_key {
        return;
    }

    let config_provider = read_api_provider_from_config_toml(base_dir);
    let local_base_url = extract_api_base_url_from_auth_file(&auth_file)
        .or_else(|| config_provider.base_url.clone());
    let account_provider = infer_api_provider_config(
        account.api_base_url.as_deref(),
        Some(account.api_provider_mode.clone()),
        account.api_provider_id.as_deref(),
        account.api_provider_name.as_deref(),
    );
    let preserve_account_provider_identity = should_preserve_account_provider_identity(
        &account_provider,
        &config_provider,
        local_base_url.as_deref(),
    );
    let provider_mode = if preserve_account_provider_identity {
        account.api_provider_mode.clone()
    } else {
        config_provider.mode.clone()
    };
    let provider_id = if preserve_account_provider_identity {
        account.api_provider_id.as_deref()
    } else {
        config_provider.provider_id.as_deref()
    };
    let provider_name = if preserve_account_provider_identity {
        account.api_provider_name.as_deref()
    } else {
        config_provider.provider_name.as_deref()
    };
    let current_provider = infer_api_provider_config(
        local_base_url.as_deref(),
        Some(provider_mode),
        provider_id,
        provider_name,
    );

    if account_provider == current_provider {
        return;
    }

    account.api_base_url = current_provider.base_url.clone();
    account.api_provider_mode = current_provider.mode.clone();
    account.api_provider_id = current_provider.provider_id.clone();
    account.api_provider_name = current_provider.provider_name.clone();
    let _ = save_account(account);
}

/// 获取当前激活的账号（基于 Tools 显式 current_account_id）
pub fn get_current_account() -> Option<CodexAccount> {
    let current_id = load_account_index().current_account_id?;
    let mut account = load_account(&current_id)?;
    let base_dir = get_codex_home();

    if account.is_api_key_auth() {
        sync_api_key_account_from_local_state(&mut account, &base_dir);
    }
    Some(account)
}

fn mark_codex_auth_type(value: &mut serde_json::Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "type".to_string(),
            serde_json::Value::String(CODEX_AUTH_TYPE.to_string()),
        );
    }
}

fn is_codex_auth_token_payload_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "access_token"
            | "refresh_token"
            | "id_token"
            | "session_id"
            | "expired"
            | "last_refresh"
            | "expires_in"
            | "timestamp"
            | "token_type"
            | "user_code"
            | "verification_uri"
            | "verification_uri_complete"
            | "openai_api_key"
            | "personal_access_token"
            | "tokens"
            | "agent_identity"
            | "agentidentity"
            | "auth_mode"
            | "authmode"
            | "base_url"
            | "api_base_url"
            | "apibaseurl"
    )
}

fn is_codex_auth_account_identity_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "email"
            | "account_email"
            | "accountemail"
            | "account_name"
            | "accountname"
            | "account_id"
            | "accountid"
            | "chatgpt_account_id"
            | "chatgptaccountid"
            | "chatgpt_user_id"
            | "chatgptuserid"
            | "user_id"
            | "userid"
            | "type"
    )
}

fn should_drop_existing_auth_metadata_key(key: &str) -> bool {
    is_codex_auth_token_payload_key(key) || is_codex_auth_account_identity_key(key)
}

fn read_existing_auth_file_object(
    base_dir: &Path,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let content = fs::read_to_string(base_dir.join("auth.json")).ok()?;
    match serde_json::from_str(&content).ok()? {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    }
}

fn merge_existing_auth_file_value(
    existing: Option<serde_json::Map<String, serde_json::Value>>,
    next: serde_json::Value,
) -> serde_json::Value {
    let mut merged = existing.unwrap_or_default();
    let stale_keys: Vec<String> = merged
        .keys()
        .filter(|key| should_drop_existing_auth_metadata_key(key))
        .cloned()
        .collect();
    for key in stale_keys {
        merged.remove(&key);
    }
    if let serde_json::Value::Object(next_map) = next {
        for (key, value) in next_map {
            merged.insert(key, value);
        }
    }
    serde_json::Value::Object(merged)
}

fn build_merged_auth_file_value(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<serde_json::Value, String> {
    let next = build_auth_file_value(account)?;
    Ok(merge_existing_auth_file_value(
        read_existing_auth_file_object(base_dir),
        next,
    ))
}

fn build_auth_file_value(account: &CodexAccount) -> Result<serde_json::Value, String> {
    if account.is_api_key_auth() {
        let api_key = normalize_optional_ref(account.openai_api_key.as_deref())
            .ok_or("API Key 账号缺少 OPENAI_API_KEY")?;
        return Ok(serde_json::json!({
            "auth_mode": API_KEY_AUTH_MODE,
            "OPENAI_API_KEY": api_key,
        }));
    }

    if account.tokens.access_token.trim().is_empty() {
        return Err("OAuth 账号缺少 access_token，无法写入 auth.json".to_string());
    }

    let last_refresh = account
        .token_updated_at
        .and_then(|timestamp| chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0))
        .map(|value| serde_json::Value::String(value.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()));
    let mut value = serde_json::to_value(CodexAuthFile {
        auth_mode: None,
        openai_api_key: Some(serde_json::Value::Null),
        base_url: None,
        tokens: Some(CodexAuthTokens {
            id_token: account.tokens.id_token.clone(),
            access_token: account.tokens.access_token.clone(),
            refresh_token: Some(account.tokens.refresh_token.clone().unwrap_or_default()),
            account_id: account.account_id.clone(),
        }),
        agent_identity: None,
        last_refresh,
    })
    .map_err(|e| format!("auth.json 序列化失败: {}", e))?;
    mark_codex_auth_type(&mut value);
    Ok(value)
}

#[cfg(target_os = "macos")]
fn build_codex_keychain_account(base_dir: &Path) -> String {
    let resolved_home = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(resolved_home.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let digest_hex = format!("{:x}", digest);
    format!("cli|{}", &digest_hex[..16])
}

#[cfg(target_os = "macos")]
fn write_codex_keychain_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    if account.is_api_key_auth() {
        return Ok(());
    }

    let payload = read_existing_auth_file_object(base_dir)
        .map(serde_json::Value::Object)
        .unwrap_or(build_merged_auth_file_value(base_dir, account)?);
    let secret = serde_json::to_string(&payload)
        .map_err(|e| format!("序列化 Codex keychain 数据失败: {}", e))?;
    let keychain_account = build_codex_keychain_account(base_dir);

    let output = std::process::Command::new("security")
        .arg("add-generic-password")
        .arg("-U")
        .arg("-s")
        .arg(CODEX_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(&keychain_account)
        .arg("-w")
        .arg(&secret)
        .output()
        .map_err(|e| format!("执行 security 命令失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "写入 Codex keychain 失败: status={}, stderr={}, stdout={}",
            output.status,
            if stderr.trim().is_empty() {
                "<empty>"
            } else {
                stderr.trim()
            },
            if stdout.trim().is_empty() {
                "<empty>"
            } else {
                stdout.trim()
            }
        ));
    }

    logger::log_info(&format!(
        "[Codex切号] 已更新 keychain 登录信息: service={}, account={}",
        CODEX_KEYCHAIN_SERVICE, keychain_account
    ));
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn write_codex_keychain_to_dir(_base_dir: &Path, _account: &CodexAccount) -> Result<(), String> {
    Ok(())
}

fn is_disk_full_io_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(28) | Some(112))
}

fn is_disk_full_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("disk_full:")
        || lower.contains("os error 28")
        || lower.contains("os error 112")
        || lower.contains("no space left on device")
        || lower.contains("not enough space on the disk")
        || lower.contains("磁盘空间不足")
}

fn format_io_error(action: &str, path: &Path, error: &std::io::Error) -> String {
    if is_disk_full_io_error(error) {
        return format!(
            "{}:{}失败: path={}, 磁盘空间不足，请清理磁盘后重试",
            DISK_FULL_ERROR_CODE,
            action,
            path.display()
        );
    }
    format!("{}失败: path={}, error={}", action, path.display(), error)
}

fn build_temp_file_path(parent: &Path, target: &Path, suffix: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(
        ".{}.tmp.{}.{}.{}",
        target
            .file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("file"),
        std::process::id(),
        unique,
        suffix
    ))
}

fn write_string_atomic(path: &Path, content: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("无法定位目标目录")?;
    fs::create_dir_all(parent).map_err(|e| format_io_error("创建目录", parent, &e))?;
    let temp_path = build_temp_file_path(parent, path, "atomic");
    fs::write(&temp_path, content).map_err(|e| format_io_error("写入临时文件", &temp_path, &e))?;
    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format_io_error("替换文件", path, &err));
    }

    Ok(())
}

fn build_managed_projection(account: &CodexAccount) -> CodexManagedAuthProjection {
    CodexManagedAuthProjection {
        version: 1,
        writer: CODEX_AUTH_PROJECTION_WRITER.to_string(),
        account_id: account.id.clone(),
        email: account.email.clone(),
        token_generation: account.token_generation,
        written_at: now_timestamp(),
    }
}

fn projection_path_for_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_AUTH_PROJECTION_FILE_NAME)
}

fn write_managed_projection_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    let projection = build_managed_projection(account);
    let content = serde_json::to_string_pretty(&projection)
        .map_err(|e| format!("受管投影序列化失败: {}", e))?;
    write_string_atomic(&projection_path_for_dir(base_dir), &content)
        .map_err(|e| format!("写入受管投影失败: {}", e))
}

fn read_managed_projection_from_dir(base_dir: &Path) -> Option<CodexManagedAuthProjection> {
    let path = projection_path_for_dir(base_dir);
    let content = fs::read_to_string(path).ok()?;
    let projection: CodexManagedAuthProjection = serde_json::from_str(&content).ok()?;
    if projection.writer == CODEX_AUTH_PROJECTION_WRITER {
        Some(projection)
    } else {
        None
    }
}

fn ensure_directory_writable_for_import(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format_io_error("创建导入目录", path, &e))?;
    let probe_path = build_temp_file_path(path, path, "import-probe");
    fs::write(&probe_path, b"probe")
        .map_err(|e| format_io_error("导入前磁盘写入预检", &probe_path, &e))?;
    fs::remove_file(&probe_path).map_err(|e| {
        format!(
            "导入预检清理失败: path={}, error={}",
            probe_path.display(),
            e
        )
    })?;
    Ok(())
}

fn ensure_storage_writable_for_import() -> Result<(), String> {
    let accounts_dir = get_accounts_dir();
    ensure_directory_writable_for_import(&accounts_dir)?;

    let index_path = get_accounts_storage_path();
    let index_dir = index_path
        .parent()
        .ok_or_else(|| format!("无法定位索引目录: {}", index_path.display()))?;
    ensure_directory_writable_for_import(index_dir)?;
    Ok(())
}

pub fn write_auth_file_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    let auth_path = base_dir.join("auth.json");
    logger::log_info(&format!(
        "[Codex切号] 准备写入登录信息: account_id={}, email={}, target_dir={}, target_file={}",
        account.id,
        account.email,
        base_dir.display(),
        auth_path.display()
    ));

    let auth_file = build_merged_auth_file_value(base_dir, account)?;
    let content =
        serde_json::to_string_pretty(&auth_file).map_err(|e| format!("序列化失败: {}", e))?;
    write_string_atomic(&auth_path, &content).map_err(|e| {
        format!(
            "写入 auth.json 失败: path={}, error={}",
            auth_path.display(),
            e
        )
    })?;

    let provider_config = if account.is_api_key_auth() {
        let provider_config = infer_api_provider_config(
            account.api_base_url.as_deref(),
            Some(account.api_provider_mode.clone()),
            account.api_provider_id.as_deref(),
            account.api_provider_name.as_deref(),
        );
        write_api_key_provider_to_config_toml(base_dir, &provider_config)?;
        provider_config
    } else {
        let provider_config = ApiProviderConfig {
            mode: CodexApiProviderMode::OpenaiBuiltin,
            base_url: None,
            provider_id: None,
            provider_name: None,
        };
        write_api_provider_to_config_toml(base_dir, &provider_config)?;
        provider_config
    };

    logger::log_info(&format!(
        "[Codex切号] 已写入登录信息: account_id={}, target_file={}, has_base_url={}",
        account.id,
        auth_path.display(),
        provider_config.base_url.is_some()
    ));

    Ok(())
}

pub fn write_account_bundle_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    write_auth_file_to_dir(base_dir, account)?;
    if let Err(err) = write_codex_keychain_to_dir(base_dir, account) {
        logger::log_warn(&format!(
            "[Codex切号] 写入 keychain 失败，目标目录可能缺少完整登录快照: {}",
            err
        ));
    }
    write_managed_projection_to_dir(base_dir, account)?;
    Ok(())
}

fn configured_codex_wsl_config_dir() -> Option<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    {
        None
    }

    #[cfg(target_os = "windows")]
    {
        let cfg = crate::modules::config::get_user_config();
        if !cfg.codex_sync_wsl {
            return None;
        }
        let trimmed = cfg.codex_wsl_config_dir.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(PathBuf::from(trimmed))
    }
}

fn sync_default_codex_account_to_wsl(account: &CodexAccount) {
    let Some(wsl_dir) = configured_codex_wsl_config_dir() else {
        return;
    };

    match write_account_bundle_to_dir(&wsl_dir, account) {
        Ok(()) => logger::log_info(&format!(
            "[Codex切号] 已同步默认账号到 WSL 配置目录: account_id={}, target_dir={}",
            account.id,
            wsl_dir.display()
        )),
        Err(err) => logger::log_warn(&format!(
            "[Codex切号] 同步默认账号到 WSL 配置目录失败，默认实例切号已完成: account_id={}, target_dir={}, error={}",
            account.id,
            wsl_dir.display(),
            err
        )),
    }
}

fn managed_projection_dirs_for_account(account_id: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let index = load_account_index();
    if index.current_account_id.as_deref() == Some(account_id) {
        dirs.push(get_codex_home());
        if let Some(wsl_dir) = configured_codex_wsl_config_dir() {
            dirs.push(wsl_dir);
        }
    }

    match crate::modules::codex_instance::load_instance_store() {
        Ok(store) => {
            if store.default_settings.bind_account_id.as_deref() == Some(account_id) {
                if let Ok(default_home) = crate::modules::codex_instance::get_default_codex_home() {
                    dirs.push(default_home);
                }
            }
            for instance in store.instances {
                if instance.bind_account_id.as_deref() == Some(account_id) {
                    dirs.push(PathBuf::from(instance.user_data_dir));
                }
            }
        }
        Err(err) => {
            logger::log_warn(&format!(
                "读取 Codex 实例绑定失败，跳过投影写穿: account_id={}, error={}",
                account_id, err
            ));
        }
    }

    let mut seen = HashSet::new();
    dirs.retain(|dir| seen.insert(dir.to_string_lossy().to_string()));
    dirs
}

fn write_managed_account_projections(account: &CodexAccount) {
    for dir in managed_projection_dirs_for_account(&account.id) {
        if let Err(err) = write_account_bundle_to_dir(&dir, account) {
            logger::log_warn(&format!(
                "Codex Token 写穿受管投影失败: account_id={}, target_dir={}, error={}",
                account.id,
                dir.display(),
                err
            ));
        }
    }
}
