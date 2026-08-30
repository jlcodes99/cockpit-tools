// Codex 账号模块：Account metadata mutations, OAuth binding and quota auto-switch alerts。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
/// 从本地文件导入 Codex 账号（支持多种 JSON 格式）
pub async fn import_from_files(file_paths: Vec<String>) -> Result<CodexFileImportResult, String> {
    use std::path::Path;

    if file_paths.is_empty() {
        return Err("未选择任何文件".to_string());
    }
    ensure_storage_writable_for_import()?;

    logger::log_info(&format!(
        "Codex: 开始从 {} 个文件导入账号...",
        file_paths.len()
    ));

    // 原有文件导入候选: (CodexTokens, account_id_hint, label, auth_file_plan_type)
    let mut candidates: Vec<(CodexTokens, Option<String>, String, Option<String>)> = Vec::new();
    // 旧规则未识别到账号时，才用 Token/JSON 粘贴框的解析逻辑处理整个文件内容。
    let mut fallback_files: Vec<(String, String, Option<String>)> = Vec::new();

    for file_path in &file_paths {
        let path = Path::new(file_path);
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                logger::log_error(&format!("读取文件失败 {:?}: {}", file_path, e));
                continue;
            }
        };

        // 从文件名推断 email 作为 label
        let filename_label = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let auth_file_plan_type = detect_auth_file_plan_type_from_path(path);

        let parsed: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                logger::log_warn(&format!(
                    "Codex 文件旧规则 JSON 解析失败，将尝试 Token/JSON 导入逻辑 {:?}: {}",
                    file_path, e
                ));
                fallback_files.push((content, filename_label, auth_file_plan_type));
                continue;
            }
        };

        let before_count = candidates.len();
        match &parsed {
            serde_json::Value::Object(_) => {
                if let Some((tokens, hint)) = extract_codex_tokens_from_value(&parsed) {
                    candidates.push((
                        tokens,
                        hint,
                        filename_label.clone(),
                        auth_file_plan_type.clone(),
                    ));
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    if let Some((tokens, hint)) = extract_codex_tokens_from_value(item) {
                        let label = item
                            .get("email")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&filename_label)
                            .to_string();
                        candidates.push((tokens, hint, label, auth_file_plan_type.clone()));
                    }
                }
            }
            _ => {}
        }

        if candidates.len() == before_count {
            logger::log_info(&format!(
                "Codex 文件旧规则未找到账号，将尝试 Token/JSON 导入逻辑 {:?}",
                file_path
            ));
            fallback_files.push((content, filename_label, auth_file_plan_type));
        }
    }

    if candidates.is_empty() && fallback_files.is_empty() {
        return Err(
            "未找到有效的 Codex Token（需要 accessToken/access_token、id_token + access_token，或 refresh_token）"
                .to_string(),
        );
    }

    logger::log_info(&format!(
        "Codex: 发现 {} 个旧格式候选账号，{} 个文件待尝试 Token/JSON 导入逻辑...",
        candidates.len(),
        fallback_files.len()
    ));

    let mut imported = Vec::new();
    let mut failed: Vec<CodexFileImportFailure> = Vec::new();
    let total = candidates.len() + fallback_files.len();
    let mut progress_index = 0usize;

    for (tokens, account_id_hint, label, auth_file_plan_type) in candidates {
        progress_index += 1;
        if let Some(app_handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = app_handle.emit(
                "codex:file-import-progress",
                serde_json::json!({
                    "current": progress_index,
                    "total": total,
                    "email": &label,
                }),
            );
        }

        match upsert_account_with_hints(tokens, account_id_hint, None) {
            Ok(mut account) => {
                if apply_auth_file_plan_type(&mut account, auth_file_plan_type) {
                    save_account(&account)?;
                }
                logger::log_info(&format!("Codex 导入成功: {}", account.email));
                imported.push(account);
            }
            Err(e) => {
                if is_disk_full_error_message(&e) {
                    logger::log_error(&format!(
                        "Codex 导入因磁盘空间不足终止: label={}, imported={}, error={}",
                        label,
                        imported.len(),
                        e
                    ));
                    return Err(format!(
                        "磁盘空间不足，已终止导入（已成功 {} 个）。{}",
                        imported.len(),
                        e
                    ));
                }
                logger::log_error(&format!("Codex 导入失败 {}: {}", label, e));
                failed.push(CodexFileImportFailure {
                    email: label,
                    error: e,
                });
            }
        }
    }

    for (content, label, auth_file_plan_type) in fallback_files {
        progress_index += 1;
        if let Some(app_handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = app_handle.emit(
                "codex:file-import-progress",
                serde_json::json!({
                    "current": progress_index,
                    "total": total,
                    "email": &label,
                }),
            );
        }

        match import_from_json(&content).await {
            Ok(accounts) => {
                for mut account in accounts {
                    if apply_auth_file_plan_type(&mut account, auth_file_plan_type.clone()) {
                        save_account(&account)?;
                    }
                    logger::log_info(&format!("Codex 导入成功: {}", account.email));
                    imported.push(account);
                }
            }
            Err(e) => {
                if is_disk_full_error_message(&e) {
                    logger::log_error(&format!(
                        "Codex 导入因磁盘空间不足终止: label={}, imported={}, error={}",
                        label,
                        imported.len(),
                        e
                    ));
                    return Err(format!(
                        "磁盘空间不足，已终止导入（已成功 {} 个）。{}",
                        imported.len(),
                        e
                    ));
                }
                logger::log_error(&format!("Codex 导入失败 {}: {}", label, e));
                failed.push(CodexFileImportFailure {
                    email: label,
                    error: e,
                });
            }
        }
    }

    logger::log_info(&format!(
        "Codex 文件导入完成，成功 {} 个，失败 {} 个",
        imported.len(),
        failed.len()
    ));

    Ok(CodexFileImportResult { imported, failed })
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    account.tags = Some(tags);
    save_account(&account)?;

    Ok(account)
}

fn spawn_fingerprint_default_session_resync() {
    if std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR").is_ok() {
        return;
    }
    if CODEX_FINGERPRINT_DEFAULT_SESSION_RESYNC_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        if let Err(error) = resync_sidecar_fingerprint_after_default_session() {
            logger::log_warn(&format!(
                "[Codex Fingerprint] 默认会话回写 sidecar 失败: {}",
                error
            ));
        }
    });
}

fn resync_sidecar_fingerprint_after_default_session() -> Result<(), String> {
    let marker = account::get_data_dir()?.join(CODEX_FINGERPRINT_DEFAULT_SESSION_MARKER);
    if marker.exists() {
        return Ok(());
    }
    for account in list_accounts() {
        if !is_standard_oauth_account(&account) {
            continue;
        }
        if let Err(error) =
            crate::modules::codex_local_access::sync_sidecar_auth_file_for_account(&account)
        {
            logger::log_warn(&format!(
                "[Codex Fingerprint] 同步会话默认到 API Service 失败: account_id={}, error={}",
                account.id, error
            ));
        }
    }
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建数据目录失败: {error}"))?;
    }
    fs::write(&marker, "1").map_err(|error| format!("写入指纹回写标记失败: {error}"))?;
    Ok(())
}

pub fn update_accounts_fingerprint_mode(
    account_ids: &[String],
    mode: String,
) -> Result<Vec<CodexAccount>, String> {
    let normalized = mode.trim().to_ascii_lowercase();
    if !matches!(normalized.as_str(), "off" | "device" | "session" | "full") {
        return Err("设备指纹模式无效".to_string());
    }
    let mut accounts = Vec::with_capacity(account_ids.len());
    for account_id in account_ids {
        let account =
            load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
        if !is_standard_oauth_account(&account) {
            return Err(format!("账号不支持设备指纹设置: {}", account_id));
        }
        accounts.push(account);
    }

    let mut updated = Vec::with_capacity(accounts.len());
    for mut account in accounts {
        account.codex_fingerprint_mode = if normalized == "session" {
            None
        } else {
            Some(normalized.clone())
        };
        save_account(&account)?;
        if let Err(error) =
            crate::modules::codex_local_access::sync_sidecar_auth_file_for_account(&account)
        {
            logger::log_warn(&format!(
                "同步设备指纹模式到 API Service sidecar 失败: account_id={}, error={}",
                account.id, error
            ));
        }
        updated.push(account);
    }
    Ok(updated)
}

pub fn update_account_client_policy(
    account_id: &str,
    codex_cli_only: bool,
    allow_app_server: bool,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if !is_standard_oauth_account(&account) {
        return Err(format!("账号不支持 Codex 客户端策略设置: {}", account_id));
    }
    account.codex_cli_only = codex_cli_only;
    account.codex_cli_only_allow_app_server = codex_cli_only && allow_app_server;
    save_account(&account)?;
    if let Err(error) =
        crate::modules::codex_local_access::sync_sidecar_auth_file_for_account(&account)
    {
        logger::log_warn(&format!(
            "同步 Codex 客户端策略到 API Service sidecar 失败: account_id={}, error={}",
            account.id, error
        ));
    }
    Ok(account)
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CodexAccountNoteUpdate {
    pub note: Option<String>,
    pub two_factor_secret: Option<String>,
    pub account_password: Option<String>,
    pub phone_number: Option<String>,
    pub mail_url: Option<String>,
}

fn apply_account_note_update(account: &mut CodexAccount, update: CodexAccountNoteUpdate) {
    if let Some(note) = update.note {
        account.account_note = normalize_optional_value(Some(note));
    }
    if let Some(secret) = update.two_factor_secret {
        account.two_factor_secret = normalize_optional_value(Some(secret));
    }
    if let Some(password) = update.account_password {
        account.account_password = normalize_optional_value(Some(password));
    }
    if let Some(phone_number) = update.phone_number {
        account.phone_number = normalize_optional_value(Some(phone_number));
    }
    if let Some(mail_url) = update.mail_url {
        account.mail_url = normalize_optional_value(Some(mail_url));
    }
}

pub fn update_account_note(
    account_id: &str,
    update: CodexAccountNoteUpdate,
    chatgpt_account_id: Option<String>,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    apply_account_note_update(&mut account, update);
    let previous_chatgpt_account_id = account.account_id.clone();
    if let Some(chatgpt_account_id) = chatgpt_account_id {
        if !is_opaque_access_token(&account.tokens.access_token)
            || normalize_optional_ref(account.tokens.refresh_token.as_deref()).is_some()
        {
            return Err("仅 at-* 个人访问令牌账号支持手动设置 ChatGPT Workspace ID".to_string());
        }
        let normalized_chatgpt_account_id = normalize_optional_value(Some(chatgpt_account_id));
        if normalized_chatgpt_account_id
            .as_deref()
            .is_some_and(|value| {
                value.len() > 256 || value.chars().any(|character| character.is_control())
            })
        {
            return Err("ChatGPT Workspace ID 格式无效".to_string());
        }
        account.account_id = normalized_chatgpt_account_id;
    }
    save_account(&account)?;

    if account.account_id != previous_chatgpt_account_id {
        if let Err(error) =
            crate::modules::codex_local_access::sync_sidecar_auth_file_for_account(&account)
        {
            logger::log_warn(&format!(
                "同步 ChatGPT Workspace ID 到 API Service sidecar 失败: account_id={}, error={}",
                account.id, error
            ));
        }
    }

    Ok(account)
}

pub fn create_pending_oauth_account(
    email: String,
    update: CodexAccountNoteUpdate,
) -> Result<CodexAccount, String> {
    let email =
        normalize_optional_value(Some(email)).ok_or_else(|| "账号邮箱不能为空".to_string())?;
    let mut index = load_account_index();

    if let Some(summary) = index
        .accounts
        .iter()
        .find(|item| item.email.eq_ignore_ascii_case(&email))
        .cloned()
    {
        if let Some(mut account) = load_account(&summary.id) {
            if !is_pending_oauth_account(&account) {
                return Err(format!("Codex 账号已存在: {}", email));
            }
            apply_account_note_update(&mut account, update);
            account.email = email.clone();
            account.last_used = chrono::Utc::now().timestamp();
            save_account_from_user_action(&mut account)?;
            if let Some(item) = index.accounts.iter_mut().find(|item| item.id == account.id) {
                item.email = account.email.clone();
                item.plan_type = account.plan_type.clone();
                item.subscription_active_until = account.subscription_active_until.clone();
                item.last_used = account.last_used;
            }
            save_account_index(&index)?;
            return Ok(account);
        }
    }

    let account_id = build_account_storage_id(&email, Some("pending_oauth"), None);
    let now = chrono::Utc::now().timestamp();
    let mut account = if let Some(mut account) = load_account(&account_id) {
        if !is_pending_oauth_account(&account) {
            return Err(format!("Codex 账号已存在: {}", email));
        }
        account.email = email.clone();
        account.last_used = now;
        account
    } else {
        let mut account = CodexAccount::new(
            account_id.clone(),
            email.clone(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.auth_mode = CodexAuthMode::OAuth;
        account.authorization_status = Some(CODEX_AUTHORIZATION_STATUS_PENDING.to_string());
        account.token_updated_at = None;
        account.token_generation = 0;
        account.requires_reauth = false;
        account.reauth_reason = None;
        account.quota = None;
        account.quota_error = None;
        account.created_at = now;
        account.last_used = now;
        account
    };
    apply_account_note_update(&mut account, update);

    index.accounts.retain(|item| item.id != account_id);
    index.accounts.push(account_summary_from_account(&account));

    save_account_from_user_action(&mut account)?;
    save_account_index(&index)?;
    logger::log_info(&format!(
        "Codex 待授权 OAuth 账号已保存: account_id={}, email={}",
        account.id, account.email
    ));

    Ok(account)
}

pub fn update_account_app_speed(
    account_id: &str,
    speed: CodexAppSpeed,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    account.app_speed = speed;
    save_account(&account)?;

    Ok(account)
}

pub async fn update_api_key_bound_oauth_account(
    account_id: &str,
    bound_oauth_account_id: Option<String>,
) -> Result<CodexAccount, String> {
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    if !account.is_api_key_auth() {
        return Err("仅 API Key 账号支持绑定 OAuth 账号".to_string());
    }

    let bound_id = normalize_optional_ref(bound_oauth_account_id.as_deref());
    let is_current = load_account_index()
        .current_account_id
        .as_deref()
        .map(|current_id| current_id == account.id)
        .unwrap_or(false);
    let _profile_lease = if is_current {
        Some(try_acquire_profile_mutation_lease(
            &get_codex_home(),
            "api-key-oauth-bind",
        )?)
    } else {
        None
    };
    if let Some(bound_id) = bound_id {
        // 绑定时必须把 OAuth 的 Token lock 持有到组合凭据写入完成，避免
        // refresh 在 freshness 检查之后、auth.json 写入之前推进 generation，
        // 导致旧快照短暂覆盖到官方目录。
        return update_api_key_bound_oauth_account_with_bound(account, bound_id).await;
    }

    let mut account = account;
    account.bound_oauth_account_id = bound_id.clone();
    // 绑定 OAuth：不走本地网关生图兼容（与改前一致，保证绑定可展示、客户端能力正常）。
    // 纯 API Key 生图仍走 gpt-image-2 + actor header，不依赖此标志。
    account.bound_oauth_use_local_gateway = false;
    save_account(&account)?;

    if is_current {
        let codex_home = get_codex_home();
        crate::modules::codex_local_access::stop_provider_gateways_for_profile(&codex_home).await;
        write_prepared_account_bundle_to_dir(&codex_home, &account)?;
    }

    Ok(account)
}

async fn update_api_key_bound_oauth_account_with_bound(
    mut account: CodexAccount,
    bound_id: String,
) -> Result<CodexAccount, String> {
    let bound_account = validate_api_key_bound_oauth_account(&account, &bound_id)?;
    let is_current = load_account_index()
        .current_account_id
        .as_deref()
        .map(|current_id| current_id == account.id)
        .unwrap_or(false);
    let token_lock = codex_token_lock_for(&bound_account.id);
    let _token_guard = token_lock.lock().await;
    let _file_guard =
        acquire_codex_token_refresh_file_lock(&bound_account.id, "api-key-bind").await?;

    // 与普通请求共用同一套 authority 同步和 refresh 逻辑，但不在这里再次
    // 获取 Token lock；锁会一直覆盖到下面的账号关系及官方投影写入完成。
    account.bound_oauth_account_id = Some(bound_id.clone());
    // 绑定动作也执行客户端级 Token 预检，确保 access_token 不可用时先刷新，
    // 并沿用统一的重新授权错误协议。
    let bound_oauth_account = match refresh_bound_oauth_account_for_api_key_locked(
        &account,
        "api-key-bind",
        true,
        false,
    )
    .await
    {
        Ok(account) => account,
        Err(error) => {
            return Err(crate::modules::codex_account::format_account_switch_error(
                &bound_id, error,
            ));
        }
    };
    account.bound_oauth_use_local_gateway = false;
    save_account(&account)?;

    if is_current {
        let codex_home = get_codex_home();
        write_api_key_account_bundle_with_oauth_to_dir(
            &codex_home,
            &account,
            &bound_oauth_account,
        )?;
        activate_provider_gateway_after_switch_if_needed(&codex_home, &account).await?;
    }

    Ok(account)
}

pub fn update_api_key_credentials(
    account_id: &str,
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Vec<String>,
    api_sync_model_catalog_to_codex: Option<bool>,
    api_wire_api: Option<String>,
    api_supports_websockets: bool,
    api_supports_vision: bool,
    api_model_vision_support: std::collections::HashMap<String, bool>,
    api_vision_routing_model: Option<String>,
    account_name: Option<String>,
    api_model_context_windows: Option<HashMap<String, i64>>,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    if !account.is_api_key_auth() {
        return Err("仅 API Key 账号支持编辑凭据".to_string());
    }

    let (normalized_key, normalized_base_url) =
        validate_api_key_credentials(&api_key, api_base_url.as_deref())?;
    let provider_config = resolve_api_provider_config(
        normalized_base_url.as_deref(),
        api_provider_mode,
        api_provider_id.as_deref(),
        api_provider_name.as_deref(),
    )?;
    let old_id = account.id.clone();
    let new_id = build_api_key_account_id(&normalized_key);
    let mut index = load_account_index();
    let was_current = get_current_account()
        .map(|current| current.id == old_id)
        .unwrap_or(false);

    if new_id != old_id && index.accounts.iter().any(|item| item.id == new_id) {
        return Err("该 API Key 已存在，请直接使用已有账号".to_string());
    }

    if new_id != old_id {
        account.id = new_id.clone();
    }

    let sync_model_catalog_to_codex =
        api_sync_model_catalog_to_codex.unwrap_or(account.api_sync_model_catalog_to_codex);
    apply_api_key_fields(
        &mut account,
        &normalized_key,
        provider_config,
        api_model_catalog,
        sync_model_catalog_to_codex,
        api_wire_api,
        api_supports_websockets,
        api_supports_vision,
        api_model_vision_support,
        api_vision_routing_model,
        api_model_context_windows,
    );
    if let Some(account_name) = normalize_optional_value(account_name) {
        account.account_name = Some(account_name);
    }
    account.update_last_used();
    save_account(&account)?;

    if old_id != account.id {
        delete_account_file(&old_id)?;
    }

    let mut summary_found = false;
    for summary in &mut index.accounts {
        if summary.id == old_id {
            summary.id = account.id.clone();
            summary.email = account.email.clone();
            summary.plan_type = account.plan_type.clone();
            summary.subscription_active_until = account.subscription_active_until.clone();
            summary.last_used = account.last_used;
            summary_found = true;
            break;
        }
    }

    if !summary_found {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    if index.current_account_id.as_deref() == Some(old_id.as_str()) {
        index.current_account_id = Some(account.id.clone());
    }
    save_account_index(&index)?;

    if old_id != account.id {
        if let Err(err) =
            crate::modules::codex_instance::replace_bind_account_references(&old_id, &account.id)
        {
            logger::log_warn(&format!(
                "Codex API Key 账号编辑后同步实例绑定失败: old_id={}, new_id={}, error={}",
                old_id, account.id, err
            ));
        }
    }

    if was_current {
        let codex_home = get_codex_home();
        write_account_bundle_to_dir(&codex_home, &account)?;
    }

    logger::log_info(&format!(
        "Codex API Key 账号凭据已更新: old_id={}, new_id={}, has_base_url={}",
        old_id,
        account.id,
        normalize_optional_ref(account.api_base_url.as_deref()).is_some()
    ));

    Ok(account)
}

pub fn sync_api_key_provider_accounts(
    account_ids: Vec<String>,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Vec<String>,
    api_wire_api: Option<String>,
    api_supports_websockets: bool,
    api_supports_vision: bool,
    api_model_vision_support: std::collections::HashMap<String, bool>,
    api_vision_routing_model: Option<String>,
    api_model_context_windows: Option<std::collections::HashMap<String, i64>>,
) -> Result<usize, String> {
    let provider_config = resolve_api_provider_config(
        api_base_url.as_deref(),
        api_provider_mode,
        api_provider_id.as_deref(),
        api_provider_name.as_deref(),
    )?;
    let current_account_id = load_account_index().current_account_id;
    let mut seen = HashSet::new();
    let mut updated_accounts = Vec::new();

    for account_id in account_ids {
        if !seen.insert(account_id.clone()) {
            continue;
        }
        let Some(mut account) = load_account(&account_id) else {
            continue;
        };
        if !account.is_api_key_auth() {
            continue;
        }
        let api_key = normalize_api_key(account.openai_api_key.as_deref().unwrap_or_default())
            .ok_or_else(|| format!("API Key 账号缺少密钥: {}", account.id))?;
        let sync_model_catalog_to_codex = account.api_sync_model_catalog_to_codex;
        apply_api_key_fields(
            &mut account,
            &api_key,
            provider_config.clone(),
            api_model_catalog.clone(),
            sync_model_catalog_to_codex,
            api_wire_api.clone(),
            api_supports_websockets,
            api_supports_vision,
            api_model_vision_support.clone(),
            api_vision_routing_model.clone(),
            api_model_context_windows.clone(),
        );
        save_account(&account)?;
        updated_accounts.push(account);
    }

    if let Some(current_account) = updated_accounts
        .iter()
        .find(|account| current_account_id.as_deref() == Some(account.id.as_str()))
    {
        write_account_bundle_to_dir(&get_codex_home(), current_account)?;
    }

    Ok(updated_accounts.len())
}

pub fn update_account_name(account_id: &str, name: String) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    if !account.is_api_key_auth() {
        return Err("仅 API Key 账号支持重命名".to_string());
    }

    account.account_name = normalize_optional_value(Some(name));
    save_account(&account)?;

    Ok(account)
}

fn normalize_quota_alert_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn normalize_auto_switch_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn normalize_auto_switch_account_scope_mode(raw: &str) -> String {
    let normalized = raw.trim().to_lowercase();
    if normalized == CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED {
        CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED.to_string()
    } else {
        CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_ALL.to_string()
    }
}

fn normalize_auto_switch_selected_account_ids(raw: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let normalized = item.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        result.push(normalized);
    }
    result
}

fn resolve_monitored_auto_switch_account_ids(
    scope_mode: &str,
    selected_account_ids: &[String],
    accounts: &[CodexAccount],
) -> HashSet<String> {
    if scope_mode != CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED {
        return accounts.iter().map(|account| account.id.clone()).collect();
    }

    let selected = normalize_auto_switch_selected_account_ids(selected_account_ids);
    if selected.is_empty() {
        return HashSet::new();
    }

    let existing: HashSet<&str> = accounts.iter().map(|account| account.id.as_str()).collect();
    selected
        .into_iter()
        .filter(|account_id| existing.contains(account_id.as_str()))
        .collect()
}

fn format_codex_quota_metric_label(window_minutes: Option<i64>, fallback: &str) -> String {
    const HOUR_MINUTES: i64 = 60;
    const DAY_MINUTES: i64 = 24 * HOUR_MINUTES;
    const WEEK_MINUTES: i64 = 7 * DAY_MINUTES;

    let Some(minutes) = window_minutes.filter(|value| *value > 0) else {
        return fallback.to_string();
    };

    if minutes >= WEEK_MINUTES - 1 {
        let weeks = (minutes + WEEK_MINUTES - 1) / WEEK_MINUTES;
        return if weeks <= 1 {
            "Weekly".to_string()
        } else {
            format!("{} Week", weeks)
        };
    }

    if minutes >= DAY_MINUTES - 1 {
        let days = (minutes + DAY_MINUTES - 1) / DAY_MINUTES;
        return format!("{}d", days);
    }

    if minutes >= HOUR_MINUTES {
        let hours = (minutes + HOUR_MINUTES - 1) / HOUR_MINUTES;
        return format!("{}h", hours);
    }

    format!("{}m", minutes)
}

#[derive(Debug, Clone)]
struct CodexQuotaMetric {
    key: &'static str,
    label: String,
    percentage: i32,
}

fn extract_quota_metrics(account: &CodexAccount) -> Vec<CodexQuotaMetric> {
    let Some(quota) = account.quota.as_ref() else {
        return Vec::new();
    };

    let has_presence =
        quota.hourly_window_present.is_some() || quota.weekly_window_present.is_some();
    let mut metrics = Vec::new();

    if !has_presence || quota.hourly_window_present.unwrap_or(false) {
        metrics.push(CodexQuotaMetric {
            key: "primary_window",
            label: format_codex_quota_metric_label(quota.hourly_window_minutes, "5h"),
            percentage: quota.hourly_percentage.clamp(0, 100),
        });
    }

    if !has_presence || quota.weekly_window_present.unwrap_or(false) {
        metrics.push(CodexQuotaMetric {
            key: "secondary_window",
            label: format_codex_quota_metric_label(quota.weekly_window_minutes, "Weekly"),
            percentage: quota.weekly_percentage.clamp(0, 100),
        });
    }

    if metrics.is_empty() {
        metrics.push(CodexQuotaMetric {
            key: "primary_window",
            label: format_codex_quota_metric_label(quota.hourly_window_minutes, "5h"),
            percentage: quota.hourly_percentage.clamp(0, 100),
        });
    }

    metrics
}

fn average_quota_percentage(metrics: &[CodexQuotaMetric]) -> f64 {
    if metrics.is_empty() {
        return 0.0;
    }
    let sum: i32 = metrics.iter().map(|metric| metric.percentage).sum();
    sum as f64 / metrics.len() as f64
}

fn metric_crossed_threshold(
    metric: &CodexQuotaMetric,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> bool {
    match metric.key {
        "primary_window" => metric.percentage <= primary_threshold,
        "secondary_window" => metric.percentage <= secondary_threshold,
        _ => false,
    }
}

fn metric_above_threshold(
    metric: &CodexQuotaMetric,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> bool {
    match metric.key {
        "primary_window" => metric.percentage > primary_threshold,
        "secondary_window" => metric.percentage > secondary_threshold,
        _ => true,
    }
}

fn metric_margin_over_threshold(
    metric: &CodexQuotaMetric,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> Option<i32> {
    match metric.key {
        "primary_window" => Some(metric.percentage - primary_threshold),
        "secondary_window" => Some(metric.percentage - secondary_threshold),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct CodexSwitchCandidate {
    account: CodexAccount,
    min_margin: i32,
    min_percentage: i32,
    average_percentage: f64,
}

fn build_switch_candidate(
    account: &CodexAccount,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> Option<CodexSwitchCandidate> {
    let metrics = extract_quota_metrics(account);
    if metrics.is_empty() {
        return None;
    }
    if !metrics
        .iter()
        .all(|metric| metric_above_threshold(metric, primary_threshold, secondary_threshold))
    {
        return None;
    }

    let min_margin = metrics
        .iter()
        .filter_map(|metric| {
            metric_margin_over_threshold(metric, primary_threshold, secondary_threshold)
        })
        .min()?;
    let min_percentage = metrics.iter().map(|metric| metric.percentage).min()?;
    let average_percentage = average_quota_percentage(&metrics);

    Some(CodexSwitchCandidate {
        account: account.clone(),
        min_margin,
        min_percentage,
        average_percentage,
    })
}

fn pick_best_candidate(mut candidates: Vec<CodexSwitchCandidate>) -> Option<CodexAccount> {
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        b.min_margin
            .cmp(&a.min_margin)
            .then_with(|| b.min_percentage.cmp(&a.min_percentage))
            .then_with(|| {
                b.average_percentage
                    .partial_cmp(&a.average_percentage)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.account.last_used.cmp(&b.account.last_used))
    });

    candidates
        .into_iter()
        .next()
        .map(|candidate| candidate.account)
}

fn build_quota_alert_cooldown_key(
    account_id: &str,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> String {
    format!(
        "codex:{}:{}:{}",
        account_id, primary_threshold, secondary_threshold
    )
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = CODEX_QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < CODEX_QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, primary_threshold: i32, secondary_threshold: i32) {
    if let Ok(mut state) = CODEX_QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(
            account_id,
            primary_threshold,
            secondary_threshold,
        ));
    }
}

pub(crate) fn resolve_current_account_id(accounts: &[CodexAccount]) -> Option<String> {
    let current_id = get_current_account()?.id;
    accounts
        .iter()
        .any(|account| account.id == current_id)
        .then_some(current_id)
}

fn pick_quota_alert_recommendation(
    accounts: &[CodexAccount],
    current_id: &str,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> Option<CodexAccount> {
    let candidates: Vec<CodexSwitchCandidate> = accounts
        .iter()
        .filter(|account| account.id != current_id)
        .filter_map(|account| {
            build_switch_candidate(account, primary_threshold, secondary_threshold)
        })
        .collect();

    pick_best_candidate(candidates)
}

pub fn pick_auto_switch_target_if_needed() -> Result<Option<CodexAccount>, String> {
    if CODEX_AUTO_SWITCH_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        logger::log_info("[AutoSwitch][Codex] 自动切号进行中，跳过本次检查");
        return Ok(None);
    }

    let result = (|| {
        let cfg = crate::modules::config::get_user_config();
        if !cfg.codex_auto_switch_enabled {
            return Ok(None);
        }

        let primary_threshold =
            normalize_auto_switch_threshold(cfg.codex_auto_switch_primary_threshold);
        let secondary_threshold =
            normalize_auto_switch_threshold(cfg.codex_auto_switch_secondary_threshold);
        let account_scope_mode =
            normalize_auto_switch_account_scope_mode(&cfg.codex_auto_switch_account_scope_mode);

        let accounts = list_accounts();
        let monitored_account_ids = resolve_monitored_auto_switch_account_ids(
            &account_scope_mode,
            &cfg.codex_auto_switch_selected_account_ids,
            &accounts,
        );
        if monitored_account_ids.is_empty() {
            logger::log_warn(&format!(
                "[AutoSwitch][Codex] 可监控账号范围为空(scope={})，跳过自动切号",
                account_scope_mode
            ));
            return Ok(None);
        }
        let current_id = match resolve_current_account_id(&accounts) {
            Some(id) => id,
            None => return Ok(None),
        };
        if !monitored_account_ids.contains(&current_id) {
            logger::log_info(&format!(
                "[AutoSwitch][Codex] 当前账号不在监控范围内(current_id={}, scope={})，跳过自动切号",
                current_id, account_scope_mode
            ));
            return Ok(None);
        }

        let current = match accounts.iter().find(|account| account.id == current_id) {
            Some(account) => account,
            None => return Ok(None),
        };

        let current_metrics = extract_quota_metrics(current);
        if current_metrics.is_empty() {
            return Ok(None);
        }

        let should_switch = current_metrics
            .iter()
            .any(|metric| metric_crossed_threshold(metric, primary_threshold, secondary_threshold));
        if !should_switch {
            return Ok(None);
        }

        let candidates: Vec<CodexSwitchCandidate> = accounts
            .iter()
            .filter(|account| monitored_account_ids.contains(&account.id))
            .filter(|account| account.id != current_id)
            .filter_map(|account| {
                build_switch_candidate(account, primary_threshold, secondary_threshold)
            })
            .collect();

        if candidates.is_empty() {
            logger::log_warn(&format!(
                "[AutoSwitch][Codex] 当前账号命中阈值 (primary<={}%, secondary<={}%)，但没有可切换候选账号",
                primary_threshold, secondary_threshold
            ));
            return Ok(None);
        }

        Ok(pick_best_candidate(candidates))
    })();

    CODEX_AUTO_SWITCH_IN_PROGRESS.store(false, Ordering::SeqCst);
    result
}

pub fn run_quota_alert_if_needed(
) -> Result<Option<crate::modules::account::QuotaAlertPayload>, String> {
    let cfg = crate::modules::config::get_user_config();
    if !cfg.codex_quota_alert_enabled {
        return Ok(None);
    }

    let primary_threshold =
        normalize_quota_alert_threshold(cfg.codex_quota_alert_primary_threshold);
    let secondary_threshold =
        normalize_quota_alert_threshold(cfg.codex_quota_alert_secondary_threshold);
    let accounts = list_accounts();
    let current_id = match resolve_current_account_id(&accounts) {
        Some(id) => id,
        None => return Ok(None),
    };

    let current = match accounts.iter().find(|account| account.id == current_id) {
        Some(account) => account,
        None => return Ok(None),
    };

    let metrics = extract_quota_metrics(current);
    let low_models: Vec<(String, i32)> = metrics
        .into_iter()
        .filter(|metric| metric_crossed_threshold(metric, primary_threshold, secondary_threshold))
        .map(|metric| (metric.label, metric.percentage))
        .collect();

    if low_models.is_empty() {
        clear_quota_alert_cooldown(&current_id, primary_threshold, secondary_threshold);
        return Ok(None);
    }

    let now = chrono::Utc::now().timestamp();
    let cooldown_key =
        build_quota_alert_cooldown_key(&current_id, primary_threshold, secondary_threshold);
    if !should_emit_quota_alert(&cooldown_key, now) {
        return Ok(None);
    }

    let recommendation = pick_quota_alert_recommendation(
        &accounts,
        &current_id,
        primary_threshold,
        secondary_threshold,
    );
    let lowest_percentage = low_models.iter().map(|(_, pct)| *pct).min().unwrap_or(0);
    let payload = crate::modules::account::QuotaAlertPayload {
        platform: "codex".to_string(),
        current_account_id: current_id,
        current_email: current.email.clone(),
        threshold: primary_threshold,
        threshold_display: Some(format!(
            "primary_window<={}%, secondary_window<={}%",
            primary_threshold, secondary_threshold
        )),
        lowest_percentage,
        low_models: low_models.into_iter().map(|(name, _)| name).collect(),
        recommended_account_id: recommendation.as_ref().map(|account| account.id.clone()),
        recommended_email: recommendation.as_ref().map(|account| account.email.clone()),
        triggered_at: now,
    };

    crate::modules::account::dispatch_quota_alert(&payload);
    Ok(Some(payload))
}
