// Codex 账号模块：Auth projection, bundle writing and managed sidecar persistence。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
/// 获取当前激活的账号（基于 Tools 显式 current_account_id）
pub fn get_current_account() -> Option<CodexAccount> {
    let base_dir = get_codex_home();
    get_current_account_from_loaded(
        load_account_index(),
        |account_id| load_account(account_id),
        &base_dir,
    )
}

fn get_current_account_from_loaded(
    index: CodexAccountIndex,
    mut load: impl FnMut(&str) -> Option<CodexAccount>,
    base_dir: &Path,
) -> Option<CodexAccount> {
    let current_id = index.current_account_id?;
    let mut account = load(&current_id)?;

    if account.is_api_key_auth() {
        sync_api_key_account_from_local_state(&mut account, base_dir);
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

    if let Some(identity) = account.agent_identity.clone() {
        let mut value = serde_json::json!({
            "auth_mode": "agentIdentity",
            "agent_identity": normalize_agent_identity(identity)?,
        });
        mark_codex_auth_type(&mut value);
        return Ok(value);
    }

    if account.tokens.access_token.trim().is_empty() {
        return Err("OAuth 账号缺少 access_token，无法写入 auth.json".to_string());
    }

    // Access-token-only accounts: prefer official personal_access_token shape
    // (no empty id_token / fabricated refresh) when neither id nor refresh exist.
    if account.tokens.id_token.trim().is_empty()
        && normalize_optional_ref(account.tokens.refresh_token.as_deref()).is_none()
    {
        let mut value = serde_json::json!({
            "OPENAI_API_KEY": null,
            "personal_access_token": account.tokens.access_token,
        });
        mark_codex_auth_type(&mut value);
        return Ok(value);
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
            // Codex CLI's auth.json parser requires the refresh_token key to
            // exist even for access-token-only accounts. Use an empty string so
            // Cockpit can switch short-lived opaque `at-...` credentials without
            // inventing a refresh token that would be sent to OAuth refresh.
            refresh_token: Some(
                normalize_optional_ref(account.tokens.refresh_token.as_deref()).unwrap_or_default(),
            ),
            account_id: account.account_id.clone(),
        }),
        agent_identity: None,
        personal_access_token: None,
        last_refresh,
    })
    .map_err(|e| format!("auth.json 序列化失败: {}", e))?;
    mark_codex_auth_type(&mut value);
    Ok(value)
}

#[cfg(all(target_os = "macos", not(test)))]
fn build_codex_keychain_account(base_dir: &Path) -> String {
    let resolved_home = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(resolved_home.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let digest_hex = format!("{:x}", digest);
    format!("cli|{}", &digest_hex[..16])
}

#[cfg(all(target_os = "macos", not(test)))]
fn write_codex_keychain_value_to_dir(
    base_dir: &Path,
    payload: &serde_json::Value,
) -> Result<(), String> {
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

#[cfg(all(target_os = "macos", test))]
fn write_codex_keychain_value_to_dir(
    _base_dir: &Path,
    _payload: &serde_json::Value,
) -> Result<(), String> {
    Err("测试环境不写入 macOS keychain".to_string())
}

#[cfg(not(target_os = "macos"))]
fn write_codex_keychain_value_to_dir(
    _base_dir: &Path,
    _payload: &serde_json::Value,
) -> Result<(), String> {
    Err("当前平台尚未实现 Codex keyring 写入".to_string())
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
    if let Some(error) = crate::modules::windows_operation::format_permission_io_error(
        "write_file",
        action,
        path.to_string_lossy().as_ref(),
        error,
    ) {
        return error;
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
    crate::modules::atomic_write::write_string_atomic(path, content)
}

fn build_managed_projection_with_credential_owner(
    runtime_account: &CodexAccount,
    credential_account: &CodexAccount,
) -> CodexManagedAuthProjection {
    CodexManagedAuthProjection {
        version: CODEX_AUTH_PROJECTION_VERSION,
        writer: CODEX_AUTH_PROJECTION_WRITER.to_string(),
        account_id: runtime_account.id.clone(),
        email: runtime_account.email.clone(),
        token_generation: runtime_account.token_generation,
        credential_account_id: Some(credential_account.id.clone()),
        credential_email: Some(credential_account.email.clone()),
        credential_token_generation: Some(credential_account.token_generation),
        written_at: now_timestamp(),
    }
}

fn build_managed_projection(account: &CodexAccount) -> CodexManagedAuthProjection {
    build_managed_projection_with_credential_owner(account, account)
}

fn managed_projection_credential_account_id(projection: &CodexManagedAuthProjection) -> &str {
    projection
        .credential_account_id
        .as_deref()
        .unwrap_or(projection.account_id.as_str())
}

fn write_managed_projection_value_to_dir(
    base_dir: &Path,
    projection: &CodexManagedAuthProjection,
) -> Result<(), String> {
    let content = serde_json::to_string_pretty(projection)
        .map_err(|e| format!("受管投影序列化失败: {}", e))?;
    write_string_atomic(&projection_path_for_dir(base_dir), &content)
        .map_err(|e| format!("写入受管投影失败: {}", e))
}

fn projection_path_for_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_AUTH_PROJECTION_FILE_NAME)
}

fn write_managed_projection_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    let projection = build_managed_projection(account);
    write_managed_projection_value_to_dir(base_dir, &projection)
}

fn write_managed_projection_with_credential_owner_to_dir(
    base_dir: &Path,
    runtime_account: &CodexAccount,
    credential_account: &CodexAccount,
) -> Result<(), String> {
    let projection =
        build_managed_projection_with_credential_owner(runtime_account, credential_account);
    write_managed_projection_value_to_dir(base_dir, &projection)
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

fn persist_managed_projection_credential_owner(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<bool, String> {
    let Some(mut projection) = read_managed_projection_from_dir(base_dir) else {
        return Ok(false);
    };
    if projection.version >= CODEX_AUTH_PROJECTION_VERSION
        && projection.credential_account_id.as_deref() == Some(account.id.as_str())
        && projection.credential_email.as_deref() == Some(account.email.as_str())
        && projection.credential_token_generation == Some(account.token_generation)
    {
        return Ok(false);
    }

    projection.version = CODEX_AUTH_PROJECTION_VERSION;
    projection.credential_account_id = Some(account.id.clone());
    projection.credential_email = Some(account.email.clone());
    projection.credential_token_generation = Some(account.token_generation);
    projection.written_at = now_timestamp();
    write_managed_projection_value_to_dir(base_dir, &projection)?;
    Ok(true)
}

fn persist_managed_projection_credential_owner_best_effort(
    base_dir: &Path,
    account: &CodexAccount,
    context: &str,
) {
    match persist_managed_projection_credential_owner(base_dir, account) {
        Ok(true) => logger::log_info(&format!(
            "Codex 已记录受管投影凭据所有者: account_id={}, source_dir={}, context={}",
            account.id,
            base_dir.display(),
            context
        )),
        Ok(false) => {}
        Err(error) => logger::log_warn(&format!(
            "Codex 记录受管投影凭据所有者失败，继续使用已读取凭据: account_id={}, source_dir={}, context={}, error={}",
            account.id,
            base_dir.display(),
            context,
            error
        )),
    }
}

pub fn read_managed_projection_account_id_from_dir(base_dir: &Path) -> Option<String> {
    read_managed_projection_from_dir(base_dir).map(|projection| projection.account_id)
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

fn write_auth_json_value(auth_path: &Path, auth_value: &serde_json::Value) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(auth_value).map_err(|e| format!("序列化失败: {}", e))?;
    write_string_atomic(auth_path, &content).map_err(|e| {
        format!(
            "写入 auth.json 失败: path={}, error={}",
            auth_path.display(),
            e
        )
    })
}

fn remove_auth_json_after_keyring_write(auth_path: &Path) {
    match fs::remove_file(auth_path) {
        Ok(()) => logger::log_info(&format!(
            "[Codex切号] keyring 写入成功，已移除 auth.json fallback: {}",
            auth_path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => logger::log_warn(&format!(
            "[Codex切号] keyring 写入成功，但移除 auth.json fallback 失败: path={}, error={}",
            auth_path.display(),
            error
        )),
    }
}

fn write_auth_value_to_configured_store(
    base_dir: &Path,
    auth_path: &Path,
    auth_value: &serde_json::Value,
) -> Result<&'static str, String> {
    let mode = codex_auth_credentials_store_mode(base_dir);

    #[cfg(target_os = "macos")]
    match mode {
        CodexAuthCredentialsStoreMode::File => {
            write_auth_json_value(auth_path, auth_value)?;
            return Ok("file");
        }
        CodexAuthCredentialsStoreMode::Keyring => {
            write_codex_keychain_value_to_dir(base_dir, auth_value)?;
            remove_auth_json_after_keyring_write(auth_path);
            return Ok("keyring");
        }
        CodexAuthCredentialsStoreMode::Auto => {
            match write_codex_keychain_value_to_dir(base_dir, auth_value) {
                Ok(()) => {
                    remove_auth_json_after_keyring_write(auth_path);
                    return Ok("auto:keyring");
                }
                Err(error) => logger::log_warn(&format!(
                    "[Codex切号] auto 模式写入 keyring 失败，回退 auth.json: {}",
                    error
                )),
            }
            write_auth_json_value(auth_path, auth_value)?;
            return Ok("auto:file");
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        if mode != CodexAuthCredentialsStoreMode::File {
            logger::log_warn(
                "[Codex切号] 当前平台暂不支持直接写入 Codex keyring，保留 auth.json 兼容写入",
            );
        }
        write_auth_json_value(auth_path, auth_value)?;
        Ok("file")
    }
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

    crate::modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(base_dir)?;

    let auth_file = build_merged_auth_file_value(base_dir, account)?;
    let auth_store = write_auth_value_to_configured_store(base_dir, &auth_path, &auth_file)?;

    let provider_config = if account.is_api_key_auth() {
        let provider_config = infer_api_provider_config(
            account.api_base_url.as_deref(),
            Some(account.api_provider_mode.clone()),
            account.api_provider_id.as_deref(),
            account.api_provider_name.as_deref(),
        );
        write_api_key_runtime_provider_to_config_toml(
            base_dir,
            account,
            &provider_config,
            false,
            true,
        )?;
        provider_config
    } else {
        let provider_config = ApiProviderConfig {
            mode: CodexApiProviderMode::OpenaiBuiltin,
            base_url: None,
            provider_id: None,
            provider_name: None,
        };
        write_api_provider_to_config_toml_with_options(base_dir, &provider_config, false)?;
        provider_config
    };

    logger::log_info(&format!(
        "[Codex切号] 已写入登录信息: account_id={}, auth_store={}, target_file={}, has_base_url={}",
        account.id,
        auth_store,
        auth_path.display(),
        provider_config.base_url.is_some()
    ));

    Ok(())
}

fn resolve_account_for_bundle_write(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<CodexAccount, String> {
    let _ = base_dir;
    let mut resolved = account.clone();
    if resolved.is_api_key_auth()
        || resolved.agent_identity.is_some()
        || resolved.tokens.id_token.trim().is_empty()
    {
        return Ok(resolved);
    }

    let (_, _, _, _, id_token_account_id, _) = extract_user_info(&resolved.tokens.id_token)
        .map_err(|error| format!("Codex OAuth id_token 无法解析，已取消写入: {}", error))?;
    let access_token_account_id =
        extract_chatgpt_account_id_from_access_token(&resolved.tokens.access_token);
    if let (Some(id_account_id), Some(access_account_id)) = (
        id_token_account_id.as_deref(),
        access_token_account_id.as_deref(),
    ) {
        if id_account_id != access_account_id {
            return Err(format!(
                "Codex OAuth 授权账号不一致，已取消写入: id_token_account_id={}, access_token_account_id={}",
                id_account_id, access_account_id
            ));
        }
    }

    // Derive account/workspace metadata from the token pair immediately before serialization.
    // This prevents stale library metadata from producing a valid access token combined with an
    // old ChatGPT-Account-Id, which the desktop cloud-config request treats as a relogin error.
    sync_identity_from_tokens(&mut resolved);
    Ok(resolved)
}

pub(crate) fn write_prepared_account_bundle_to_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    let account = resolve_account_for_bundle_write(base_dir, account)?;
    write_auth_file_to_dir(base_dir, &account)?;
    write_managed_projection_to_dir(base_dir, &account)?;
    sync_or_cleanup_managed_model_catalog_for_dir(base_dir, &account)?;
    Ok(())
}

fn validate_api_key_bound_oauth_account(
    api_key_account: &CodexAccount,
    bound_oauth_account_id: &str,
) -> Result<CodexAccount, String> {
    if !api_key_account.is_api_key_auth() {
        return Err("仅 API Key 账号支持绑定 OAuth 账号".to_string());
    }

    let bound_id = normalize_optional_ref(Some(bound_oauth_account_id))
        .ok_or_else(|| "请选择要绑定的 OAuth 账号".to_string())?;
    if bound_id == api_key_account.id {
        return Err("API Key 账号不能绑定自身".to_string());
    }

    let oauth_account =
        load_account(&bound_id).ok_or_else(|| format!("绑定的 OAuth 账号不存在: {}", bound_id))?;
    if oauth_account.is_api_key_auth() {
        return Err("只能绑定 OAuth 账号，不能绑定 API Key 账号".to_string());
    }
    if oauth_account.is_agent_identity_auth() {
        return Err("Agent Identity 账号仅用于 API 服务，不能作为 OAuth 绑定账号".to_string());
    }
    if !account_has_refresh_token(&oauth_account) {
        return Err("只能绑定带 refresh_token 的 OAuth 账号".to_string());
    }

    Ok(oauth_account)
}

fn load_optional_bound_oauth_account_for_api_key(
    api_key_account: &CodexAccount,
) -> Result<Option<CodexAccount>, String> {
    let Some(bound_id) = normalize_optional_ref(api_key_account.bound_oauth_account_id.as_deref())
    else {
        return Ok(None);
    };
    validate_api_key_bound_oauth_account(api_key_account, &bound_id).map(Some)
}

fn write_api_key_provider_override_to_config_toml(
    base_dir: &Path,
    api_key_account: &CodexAccount,
) -> Result<ApiProviderConfig, String> {
    let provider_config = infer_api_provider_config(
        api_key_account.api_base_url.as_deref(),
        Some(api_key_account.api_provider_mode.clone()),
        api_key_account.api_provider_id.as_deref(),
        api_key_account.api_provider_name.as_deref(),
    );
    write_api_key_runtime_provider_to_config_toml(
        base_dir,
        api_key_account,
        &provider_config,
        true,
        true,
    )?;
    Ok(provider_config)
}

/// 按账号当前模型目录刷新 profile 上的 provider 生图 header（有则写、无则清）。
fn refresh_api_key_provider_projection_in_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    if !account.is_api_key_auth() {
        return Ok(());
    }
    if account_uses_deepseek_cdp_injection(account) {
        return Ok(());
    }
    if let Some(oauth) = load_optional_bound_oauth_account_for_api_key(account)? {
        if !oauth.tokens.id_token.trim().is_empty() {
            write_api_key_provider_override_to_config_toml(base_dir, account)?;
            return Ok(());
        }
    }
    let provider_config = infer_api_provider_config(
        account.api_base_url.as_deref(),
        Some(account.api_provider_mode.clone()),
        account.api_provider_id.as_deref(),
        account.api_provider_name.as_deref(),
    );
    write_api_key_runtime_provider_to_config_toml(
        base_dir,
        account,
        &provider_config,
        false,
        false,
    )?;
    Ok(())
}

fn write_api_key_account_bundle_with_oauth_to_dir(
    base_dir: &Path,
    api_key_account: &CodexAccount,
    oauth_account: &CodexAccount,
) -> Result<(), String> {
    if !api_key_account.is_api_key_auth() {
        return Err("仅 API Key 账号支持 OAuth 绑定写入".to_string());
    }
    if oauth_account.is_api_key_auth() {
        return Err("API Key 账号绑定目标必须是 OAuth 账号".to_string());
    }
    if api_key_account.bound_oauth_account_id.as_deref() != Some(oauth_account.id.as_str()) {
        return Err("API Key 账号绑定的 OAuth 账号不匹配".to_string());
    }

    if oauth_account.tokens.id_token.trim().is_empty() {
        write_prepared_account_bundle_to_dir(base_dir, api_key_account)?;
        logger::log_info(&format!(
            "[Codex切号] 已写入 API Key 账号配置，绑定 OAuth 缺少 id_token，跳过 OAuth 登录态投影: api_account_id={}, oauth_account_id={}, target_dir={}",
            api_key_account.id,
            oauth_account.id,
            base_dir.display()
        ));
        return Ok(());
    }

    write_prepared_account_bundle_to_dir(base_dir, oauth_account)?;
    let provider_config =
        write_api_key_provider_override_to_config_toml(base_dir, api_key_account)?;
    // config/Provider 归 API Key 账号所有，但 auth.json/keychain 中的一次性 RT 链
    // 归绑定的 OAuth 账号所有。必须同时持久化两种归属，否则官方客户端轮换 RT 后，
    // OAuth 账号单独启动时可能找不到最新凭据并再次消费旧 RT。
    write_managed_projection_with_credential_owner_to_dir(
        base_dir,
        api_key_account,
        oauth_account,
    )?;
    sync_or_cleanup_managed_model_catalog_for_dir(base_dir, api_key_account)?;
    logger::log_info(&format!(
        "[Codex切号] 已写入 API Key 账号绑定 OAuth 的组合配置: api_account_id={}, oauth_account_id={}, target_dir={}, has_base_url={}",
        api_key_account.id,
        oauth_account.id,
        base_dir.display(),
        provider_config.base_url.is_some()
    ));
    Ok(())
}

pub fn write_account_bundle_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    if account.is_api_key_auth() {
        if let Some(oauth_account) = load_optional_bound_oauth_account_for_api_key(account)? {
            return write_api_key_account_bundle_with_oauth_to_dir(
                base_dir,
                account,
                &oauth_account,
            );
        }
        return write_prepared_account_bundle_to_dir(base_dir, account);
    }

    let account = resolve_account_for_bundle_write(base_dir, account)?;
    write_prepared_account_bundle_to_dir(base_dir, &account)
}

/// File entry inside a remote Codex projection bundle (#1404 full SSH sync).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodexProjectionFile {
    pub relative_path: String,
    pub content: String,
    pub mode: u32,
    pub sha256: String,
}

/// Remote-safe Codex account projection (auth.json + config.toml + marker).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodexAccountProjectionBundle {
    pub account_id: String,
    pub account_email: String,
    pub token_generation: u64,
    pub files: Vec<CodexProjectionFile>,
    pub bundle_hash: String,
}

fn sha256_hex_bytes(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

fn build_bundle_hash(files: &[CodexProjectionFile]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.relative_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(file.sha256.as_bytes());
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}

/// Build a remote projection bundle without writing host keychain secrets.
pub(crate) fn build_projection_bundle_for_remote(
    account: &CodexAccount,
    existing_config_toml: Option<&str>,
) -> Result<CodexAccountProjectionBundle, String> {
    let temp_dir = std::env::temp_dir().join(format!(
        "cockpit-codex-remote-bundle-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("创建远程投影临时目录失败: {}", e))?;

    let build_result = (|| {
        if let Some(existing_config) = existing_config_toml {
            let config_path = temp_dir.join(CODEX_CONFIG_FILE_NAME);
            crate::modules::atomic_write::write_string_atomic(&config_path, existing_config)?;
        }

        write_account_bundle_to_dir(&temp_dir, account)?;

        let mut files = Vec::new();
        for (relative_path, mode) in [
            ("auth.json", 0o600_u32),
            (CODEX_CONFIG_FILE_NAME, 0o600),
            (CODEX_AUTH_PROJECTION_FILE_NAME, 0o600),
        ] {
            let path = temp_dir.join(relative_path);
            let content = if path.exists() {
                fs::read_to_string(&path)
                    .map_err(|e| format!("读取 Codex 投影文件失败: {}: {}", relative_path, e))?
            } else if relative_path == CODEX_CONFIG_FILE_NAME {
                String::new()
            } else {
                return Err(format!("Codex 投影缺少必要文件: {}", relative_path));
            };
            let sha256 = sha256_hex_bytes(content.as_bytes());
            files.push(CodexProjectionFile {
                relative_path: relative_path.to_string(),
                content,
                mode,
                sha256,
            });
        }

        let bundle_hash = build_bundle_hash(&files);
        Ok(CodexAccountProjectionBundle {
            account_id: account.id.clone(),
            account_email: account.email.clone(),
            token_generation: account.token_generation,
            files,
            bundle_hash,
        })
    })();

    if let Err(err) = fs::remove_dir_all(&temp_dir) {
        logger::log_warn(&format!(
            "[Codex SSH] 清理远程投影临时目录失败: path={}, error={}",
            temp_dir.display(),
            err
        ));
    }

    build_result
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

fn sync_default_codex_account_to_wsl<F>(account_id: &str, write_bundle: F)
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    let Some(wsl_dir) = configured_codex_wsl_config_dir() else {
        return;
    };

    match write_bundle(&wsl_dir) {
        Ok(()) => logger::log_info(&format!(
            "[Codex切号] 已同步默认账号到 WSL 配置目录: account_id={}, target_dir={}",
            account_id,
            wsl_dir.display()
        )),
        Err(err) => logger::log_warn(&format!(
            "[Codex切号] 同步默认账号到 WSL 配置目录失败，默认实例切号已完成: account_id={}, target_dir={}, error={}",
            account_id,
            wsl_dir.display(),
            err
        )),
    }
}

fn is_default_codex_projection_dir(dir: &Path) -> bool {
    if projection_dirs_equal(dir, &get_codex_home()) {
        return true;
    }

    configured_codex_wsl_config_dir()
        .as_deref()
        .map(|wsl_dir| projection_dirs_equal(dir, wsl_dir))
        .unwrap_or(false)
}

fn is_bound_api_key_account_id(
    bound_account_id: Option<&str>,
    oauth_account_id: &str,
    api_key_accounts: &[CodexAccount],
) -> bool {
    let Some(bound_account_id) = bound_account_id else {
        return false;
    };
    api_key_accounts.iter().any(|account| {
        account.id == bound_account_id
            && account.bound_oauth_account_id.as_deref() == Some(oauth_account_id)
    })
}

fn managed_projection_dirs_for_account(account_id: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let index = load_account_index();
    let bound_api_key_accounts: Vec<CodexAccount> = list_accounts()
        .into_iter()
        .filter(|account| {
            account.is_api_key_auth()
                && account.bound_oauth_account_id.as_deref() == Some(account_id)
        })
        .collect();
    if index.current_account_id.as_deref() == Some(account_id)
        || is_bound_api_key_account_id(
            index.current_account_id.as_deref(),
            account_id,
            &bound_api_key_accounts,
        )
    {
        dirs.push(get_codex_home());
        if let Some(wsl_dir) = configured_codex_wsl_config_dir() {
            dirs.push(wsl_dir);
        }
    }

    match crate::modules::codex_instance::load_instance_store() {
        Ok(store) => {
            if store.default_settings.bind_account_id.as_deref() == Some(account_id)
                || is_bound_api_key_account_id(
                    store.default_settings.bind_account_id.as_deref(),
                    account_id,
                    &bound_api_key_accounts,
                )
            {
                if let Ok(default_home) = crate::modules::codex_instance::get_default_codex_home() {
                    dirs.push(default_home);
                }
            }
            for instance in store.instances {
                if instance.bind_account_id.as_deref() == Some(account_id)
                    || is_bound_api_key_account_id(
                        instance.bind_account_id.as_deref(),
                        account_id,
                        &bound_api_key_accounts,
                    )
                {
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

/// 返回可能持有该 OAuth 账号最新轮换凭据的所有受管目录。
///
/// `managed_projection_dirs_for_account` 只描述当前绑定关系，适合 Token 写穿；这里还会
/// 读取投影中持久化的凭据所有者，使 API Key 解绑或实例改绑后，原组合实例产生的新 RT
/// 仍能在 OAuth 账号下次启动前被接回。v1 组合投影没有凭据所有者字段，只在 Token 身份
/// 确认匹配时兼容接回，避免把其它账号的投影误归属。
fn authority_projection_dirs_for_account(account: &CodexAccount) -> Vec<PathBuf> {
    let process_entries = crate::modules::process::collect_codex_process_entries();
    authority_projection_dirs_for_account_with_entries(account, &process_entries)
}

fn authority_projection_dirs_for_account_with_entries(
    account: &CodexAccount,
    process_entries: &[(u32, Option<String>)],
) -> Vec<PathBuf> {
    let mut dirs = managed_projection_dirs_for_account(&account.id);
    let mut candidates = vec![get_codex_home()];
    if let Some(wsl_dir) = configured_codex_wsl_config_dir() {
        candidates.push(wsl_dir);
    }
    if let Ok(store) = crate::modules::codex_instance::load_instance_store() {
        if let Ok(default_home) = crate::modules::codex_instance::get_default_codex_home() {
            candidates.push(default_home);
        }
        candidates.extend(
            store
                .instances
                .into_iter()
                .map(|instance| PathBuf::from(instance.user_data_dir)),
        );
    }
    candidates.extend(
        process_entries
            .iter()
            .filter_map(|(_, runtime_home)| runtime_home.as_deref().map(PathBuf::from)),
    );

    let mut seen = dirs
        .iter()
        .map(|dir| dir.to_string_lossy().to_string())
        .collect::<HashSet<_>>();
    for dir in candidates {
        let key = dir.to_string_lossy().to_string();
        if seen.contains(&key) {
            continue;
        }
        let Some(projection) = read_managed_projection_from_dir(&dir) else {
            continue;
        };
        let explicit_owner_matches =
            managed_projection_credential_account_id(&projection) == account.id;
        let legacy_combined_projection_matches = projection.credential_account_id.is_none()
            && projection.account_id != account.id
            && load_local_oauth_snapshot_from_official_store(&dir)
                .as_ref()
                .is_some_and(|snapshot| local_oauth_snapshot_matches_account(snapshot, account));
        if explicit_owner_matches || legacy_combined_projection_matches {
            seen.insert(key);
            dirs.push(dir);
        }
    }
    dirs
}

pub fn cleanup_managed_model_catalogs_on_startup() -> Result<usize, String> {
    let current_account_id = load_account_index().current_account_id;
    let account_requires_managed_catalog = |account_id: Option<&str>| {
        account_id
            .and_then(load_account)
            .map(|account| {
                crate::modules::codex_local_access::account_requires_provider_gateway(&account)
                    || account_syncs_model_catalog_to_codex(&account)
            })
            .unwrap_or(false)
    };
    let current_requires_managed_catalog =
        account_requires_managed_catalog(current_account_id.as_deref());
    let mut dirs: HashMap<String, (PathBuf, bool)> = HashMap::new();
    let mut add_dir = |dir: PathBuf, preserve_catalog: bool| {
        let key = dir.to_string_lossy().to_string();
        dirs.entry(key)
            .and_modify(|(_, preserve)| *preserve |= preserve_catalog)
            .or_insert((dir, preserve_catalog));
    };

    add_dir(get_codex_home(), current_requires_managed_catalog);
    if let Some(wsl_dir) = configured_codex_wsl_config_dir() {
        add_dir(wsl_dir, current_requires_managed_catalog);
    }
    if let Ok(store) = crate::modules::codex_instance::load_instance_store() {
        if let Ok(default_home) = crate::modules::codex_instance::get_default_codex_home() {
            add_dir(
                default_home,
                account_requires_managed_catalog(store.default_settings.bind_account_id.as_deref()),
            );
        }
        for instance in store.instances {
            add_dir(
                PathBuf::from(instance.user_data_dir),
                account_requires_managed_catalog(instance.bind_account_id.as_deref()),
            );
        }
    }

    let mut cleaned = 0;
    let mut failures = Vec::new();
    for (_, (dir, preserve_catalog)) in dirs {
        if preserve_catalog || experimental_model_policy_enabled(&dir) {
            continue;
        }
        match cleanup_managed_model_catalog_for_dir(&dir) {
            Ok(true) => cleaned += 1,
            Ok(false) => {}
            Err(error) => failures.push(format!("profile_dir={}, error={}", dir.display(), error)),
        }
    }

    if failures.is_empty() {
        Ok(cleaned)
    } else {
        Err(format!(
            "清理受管 Codex 模型目录部分失败: cleaned={}, failures={}",
            cleaned,
            failures.join("; ")
        ))
    }
}

fn projection_dirs_equal(left: &Path, right: &Path) -> bool {
    left.to_string_lossy() == right.to_string_lossy()
}

fn sync_managed_account_sidecar(account: &CodexAccount) {
    if let Err(err) = sync_managed_account_sidecar_checked(account) {
        logger::log_warn(&format!(
            "Codex Token 同步 API Service sidecar 未完成，后续会重试: account_id={}, error={}",
            account.id, err
        ));
    }
}

fn sync_managed_account_sidecar_checked(account: &CodexAccount) -> Result<(), String> {
    crate::modules::codex_local_access::sync_sidecar_auth_file_for_account(account).map_err(|err| {
        format!(
            "Codex Token 同步 API Service sidecar 认证失败: account_id={}, error={}",
            account.id, err
        )
    })
}

/// OAuth 重新授权后只更新 Cockpit 账号库关联的 API Service sidecar 认证。
/// 官方 profile 不做后台写穿；默认实例、多开实例和 API Key 绑定会在下次显式
/// 启动/切换时从账号库投影最新凭据，避免后台任务覆盖正在使用的 profile。
pub async fn sync_bound_oauth_consumers_after_reauth(account_id: &str) -> Result<(), String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err("OAuth 账号 ID 为空".to_string());
    }
    let account = load_account(account_id)
        .ok_or_else(|| format!("重新授权后找不到 OAuth 账号: {}", account_id))?;

    sync_managed_account_sidecar_checked(&account)
}

