// cockpit-core Codex 账号：Managed runtime validation, switching and token import。
// 通过 include! 保持原模块作用域和凭据调用路径。

async fn refresh_managed_account_locked(
    account_id: &str,
    force: bool,
    reason: &str,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        return Ok(account);
    }
    if let Err(err) = clear_stale_missing_refresh_token_reauth(&mut account) {
        logger::log_warn(&format!(
            "Codex 清理缺失 refresh_token 的过期重登标记失败，继续处理: account_id={}, error={}",
            account.id, err
        ));
    }
    if account.requires_reauth && codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Err(account
            .reauth_reason
            .clone()
            .unwrap_or_else(|| "账号需要重新登录".to_string()));
    }
    if !force && !managed_account_tokens_need_refresh(&account) {
        return Ok(account);
    }

    if !account_has_refresh_token(&account) {
        logger::log_warn(&format!(
            "Codex Token Authority 跳过刷新：账号缺少 refresh_token，按 access-token-only 模式继续使用当前 access_token: account_id={}, email={}, force={}, reason={}",
            account.id, account.email, force, reason
        ));
        return Ok(account);
    }
    let refresh_token = account.tokens.refresh_token.clone().unwrap_or_default();

    logger::log_info(&format!(
        "Codex Token Authority 开始刷新: account_id={}, email={}, force={}, reason={}",
        account.id, account.email, force, reason
    ));

    match codex_oauth::refresh_access_token_with_fallback(
        &refresh_token,
        Some(account.tokens.id_token.as_str()),
    )
    .await
    {
        Ok(new_tokens) => {
            account.tokens = new_tokens;
            sync_identity_from_tokens(&mut account);
            mark_token_chain_updated(&mut account);
            save_account(&account)?;
            write_managed_account_projections(&account);
            logger::log_info(&format!(
                "Codex Token Authority 刷新成功: account_id={}, generation={}",
                account.id, account.token_generation
            ));
            Ok(account)
        }
        Err(err) => {
            if is_reauth_required_refresh_error(&err) {
                let reason = format!("Codex refresh_token 已失效，请重新登录: {}", err);
                let _ = mark_account_requires_reauth(&mut account, &reason);
                return Err(reason);
            }
            Err(format!("Token 已过期且刷新失败: {}", err))
        }
    }
}

pub async fn ensure_managed_account_fresh(account_id: &str) -> Result<CodexAccount, String> {
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    refresh_managed_account_locked(account_id, false, "prepare").await
}

pub async fn force_refresh_managed_account(
    account_id: &str,
    reason: &str,
) -> Result<CodexAccount, String> {
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    refresh_managed_account_locked(account_id, true, reason).await
}

pub async fn execute_with_managed_account_projection<R, F>(
    account_id: &str,
    auth_dir: &Path,
    reason: &str,
    operation: F,
) -> Result<(CodexAccount, R, Option<String>), String>
where
    F: FnOnce(&CodexAccount) -> R,
{
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let account = refresh_managed_account_locked(account_id, false, reason).await?;
    write_account_bundle_to_dir(auth_dir, &account)?;

    let result = operation(&account);
    let sync_error = match sync_managed_projection_from_auth_dir(account_id, auth_dir) {
        Ok(_) => None,
        Err(err) => Some(err),
    };
    let latest_account = load_account(account_id).unwrap_or(account);

    Ok((latest_account, result, sync_error))
}

/// 准备账号注入：账号中心是唯一 Token 真源，必要时刷新并投影到目标目录。
pub async fn prepare_account_for_injection_from_auth_dir(
    account_id: &str,
    auth_dir: Option<&Path>,
) -> Result<CodexAccount, String> {
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let account = refresh_managed_account_locked(account_id, false, "prepare").await?;
    if let Some(dir) = auth_dir {
        write_account_bundle_to_dir(dir, &account)?;
    }
    Ok(account)
}

pub async fn prepare_account_for_injection(account_id: &str) -> Result<CodexAccount, String> {
    prepare_account_for_injection_from_store(account_id).await
}

/// 准备账号注入（存储真源模式）：
/// 仅使用账号中心存储作为 Token 真源，不从受管目录/本地 auth.json 回读，避免旧快照反向覆盖。
pub async fn prepare_account_for_injection_from_store(
    account_id: &str,
) -> Result<CodexAccount, String> {
    ensure_managed_account_fresh(account_id).await
}

fn switch_account_with_prepared(
    account_id: &str,
    account_for_write: CodexAccount,
) -> Result<CodexAccount, String> {
    let codex_home = get_codex_home();
    let auth_path = codex_home.join("auth.json");
    logger::log_info(&format!(
        "[Codex切号] 开始切换账号: account_id={}, email={}, target_dir={}",
        account_for_write.id,
        account_for_write.email,
        codex_home.display()
    ));
    write_account_bundle_to_dir(&codex_home, &account_for_write)?;
    logger::log_info(&format!(
        "[Codex切号] 已替换目录登录信息: target_dir={}, target_file={}",
        codex_home.display(),
        auth_path.display()
    ));
    sync_default_codex_account_to_wsl(&account_for_write);

    // 更新索引中的 current_account_id
    let mut index = load_account_index();
    index.current_account_id = Some(account_id.to_string());
    save_account_index(&index)?;

    // 更新账号的 last_used
    let mut updated_account = account_for_write.clone();
    updated_account.update_last_used();
    save_account(&updated_account)?;

    logger::log_info(&format!("已切换到 Codex 账号: {}", updated_account.email));

    Ok(updated_account)
}

pub async fn switch_account_managed(account_id: &str) -> Result<CodexAccount, String> {
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let account = refresh_managed_account_locked(account_id, false, "switch").await?;
    switch_account_with_prepared(account_id, account)
}

/// 从本地 auth.json 导入账号
pub fn import_from_local() -> Result<CodexAccount, String> {
    let auth_path = get_auth_json_path();
    if !auth_path.exists() {
        return Err("未找到 ~/.codex/auth.json 文件".to_string());
    }

    let content =
        fs::read_to_string(&auth_path).map_err(|e| format!("读取 auth.json 失败: {}", e))?;

    let auth_file: CodexAuthFile =
        serde_json::from_str(&content).map_err(|e| format!("解析 auth.json 失败: {}", e))?;
    let fallback_api_key = extract_api_key_from_auth_file(&auth_file);
    let config_provider = read_api_provider_from_config_toml(&get_codex_home());
    let fallback_provider = infer_api_provider_config(
        extract_api_base_url_from_auth_file(&auth_file)
            .or_else(|| config_provider.base_url.clone())
            .as_deref(),
        Some(config_provider.mode.clone()),
        config_provider.provider_id.as_deref(),
        config_provider.provider_name.as_deref(),
    );

    if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
        let api_key = fallback_api_key.ok_or("auth.json 缺少 OPENAI_API_KEY")?;
        return upsert_api_key_account(
            api_key,
            fallback_provider.base_url.clone(),
            Some(fallback_provider.mode),
            fallback_provider.provider_id.clone(),
            fallback_provider.provider_name.clone(),
        );
    }

    if let Some(tokens) = auth_file.tokens {
        return upsert_account_from_auth_tokens(tokens);
    }

    if let Some(api_key) = fallback_api_key {
        return upsert_api_key_account(
            api_key,
            fallback_provider.base_url.clone(),
            Some(fallback_provider.mode),
            fallback_provider.provider_id.clone(),
            fallback_provider.provider_name.clone(),
        );
    }

    Err("auth.json 缺少可导入的账号信息".to_string())
}

fn import_account_struct(account: CodexAccount) -> Result<CodexAccount, String> {
    if account.is_api_key_auth() || account.openai_api_key.is_some() {
        let api_key = normalize_optional_ref(account.openai_api_key.as_deref())
            .ok_or("API Key 账号缺少 OPENAI_API_KEY")?;
        let mut api_acc = upsert_api_key_account(
            api_key,
            account.api_base_url.clone(),
            Some(account.api_provider_mode),
            account.api_provider_id.clone(),
            account.api_provider_name.clone(),
        )?;
        let mut changed = false;
        if let Some(tags) = account.tags {
            api_acc.tags = Some(tags);
            changed = true;
        }
        if let Some(note) = account.account_note {
            api_acc.account_note = Some(note);
            changed = true;
        }
        if changed {
            save_account(&api_acc)?;
        }
        return Ok(api_acc);
    }

    let mut imported = upsert_account(account.tokens)?;
    let mut changed = false;
    if let Some(tags) = account.tags {
        imported.tags = Some(tags);
        changed = true;
    }
    if let Some(note) = account.account_note {
        imported.account_note = Some(note);
        changed = true;
    }

    if changed {
        save_account(&imported)?;
    }

    Ok(imported)
}

fn upsert_account_from_auth_tokens(tokens: CodexAuthTokens) -> Result<CodexAccount, String> {
    let account_id_hint = tokens.account_id.clone();
    let tokens = CodexTokens {
        id_token: tokens.id_token,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    };

    if normalize_optional_ref(Some(&tokens.id_token)).is_none()
        && decode_jwt_payload_value(&tokens.access_token).is_some()
    {
        return upsert_account_from_access_token(tokens.access_token, None);
    }

    upsert_account_with_hints(tokens, account_id_hint, None)
}

enum CodexJsonImportCandidate {
    FullToken {
        tokens: CodexTokens,
        account_id_hint: Option<String>,
        account_note: Option<String>,
    },
    AccessToken {
        access_token: String,
        account_note: Option<String>,
    },
    RefreshToken {
        refresh_token: String,
        account_note: Option<String>,
    },
}

fn extract_account_note_from_value(value: &serde_json::Value) -> Option<String> {
    let obj = value.as_object()?;
    [
        "account_note",
        "accountInfo",
        "account_info",
        "note",
        "notes",
        "remark",
    ]
    .iter()
    .find_map(|key| {
        obj.get(*key)
            .and_then(|value| value.as_str())
            .and_then(|value| normalize_optional_ref(Some(value)))
    })
}

fn extract_refresh_token_only_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(raw) => normalize_optional_ref(Some(raw))
            .filter(|token| decode_jwt_payload_value(token).is_none()),
        serde_json::Value::Object(_) => first_json_string(
            value,
            &[
                &["refresh_token"],
                &["refreshToken"],
                &["tokens", "refresh_token"],
                &["tokens", "refreshToken"],
            ],
        ),
        _ => None,
    }
}

fn extract_access_token_only_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(raw) => normalize_optional_ref(Some(raw))
            .filter(|token| decode_jwt_payload_value(token).is_some()),
        serde_json::Value::Object(_) => first_json_string(
            value,
            &[
                &["tokens", "access_token"],
                &["tokens", "accessToken"],
                &["credentials", "access_token"],
                &["credentials", "accessToken"],
                &["access_token"],
                &["accessToken"],
                &["token"],
            ],
        )
        .filter(|token| decode_jwt_payload_value(token).is_some()),
        _ => None,
    }
}

fn extract_codex_import_candidate_from_value(
    value: &serde_json::Value,
) -> Option<CodexJsonImportCandidate> {
    if let Some((tokens, account_id_hint)) = extract_codex_tokens_from_value(value) {
        return Some(CodexJsonImportCandidate::FullToken {
            tokens,
            account_id_hint,
            account_note: extract_account_note_from_value(value),
        });
    }

    if let Some(refresh_token) = extract_refresh_token_only_from_value(value) {
        return Some(CodexJsonImportCandidate::RefreshToken {
            refresh_token,
            account_note: extract_account_note_from_value(value),
        });
    }

    extract_access_token_only_from_value(value).map(|access_token| {
        CodexJsonImportCandidate::AccessToken {
            access_token,
            account_note: extract_account_note_from_value(value),
        }
    })
}

async fn upsert_account_from_refresh_token(
    refresh_token: String,
    account_note: Option<String>,
) -> Result<CodexAccount, String> {
    let tokens = codex_oauth::refresh_access_token(&refresh_token).await?;
    let mut account = upsert_account(tokens)?;
    if account_note.is_some() {
        account.account_note = account_note;
        save_account(&account)?;
    }
    Ok(account)
}

fn upsert_account_from_access_token(
    access_token: String,
    account_note: Option<String>,
) -> Result<CodexAccount, String> {
    let access_token =
        normalize_optional_value(Some(access_token)).ok_or("accessToken 不能为空")?;
    let (email, user_id, plan_type, account_id, organization_id) =
        extract_access_token_identity(&access_token);
    let email = email
        .or_else(|| account_id.as_ref().map(|value| format!("codex-{}", value)))
        .or_else(|| user_id.as_ref().map(|value| format!("codex-{}", value)))
        .unwrap_or_else(|| format!("codex-access-{}", access_token_fingerprint(&access_token)));
    let tokens = CodexTokens {
        id_token: String::new(),
        access_token,
        refresh_token: None,
    };

    let mut index = load_account_index();
    let generated_id =
        build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let existing_id = find_existing_account_id(
        &index,
        &email,
        account_id.as_deref(),
        organization_id.as_deref(),
    )
    .unwrap_or_else(|| generated_id.clone());
    let existing = index
        .accounts
        .iter()
        .position(|item| item.id == existing_id);

    let account = if let Some(pos) = existing {
        let existing_id = index.accounts[pos].id.clone();
        let mut acc = load_account(&existing_id)
            .unwrap_or_else(|| CodexAccount::new(existing_id, email.clone(), tokens.clone()));
        acc.tokens = tokens;
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        if account_note.is_some() {
            acc.account_note = account_note;
        }
        acc.update_last_used();
        acc
    } else {
        let mut acc = CodexAccount::new(existing_id.clone(), email.clone(), tokens);
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        acc.account_note = account_note;

        index.accounts.retain(|item| item.id != existing_id);
        index.accounts.push(CodexAccountSummary {
            id: existing_id.clone(),
            email: email.clone(),
            plan_type: plan_type.clone(),
            created_at: acc.created_at,
            last_used: acc.last_used,
        });
        acc
    };

    save_account(&account)?;

    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    save_account_index(&index)?;

    logger::log_info(&format!(
        "Codex accessToken 账号已保存: email={}, account_id={:?}, organization_id={:?}",
        email, account_id, organization_id
    ));

    Ok(account)
}

async fn import_codex_candidate(
    candidate: CodexJsonImportCandidate,
) -> Result<CodexAccount, String> {
    match candidate {
        CodexJsonImportCandidate::FullToken {
            tokens,
            account_id_hint,
            account_note,
        } => {
            let mut account = upsert_account_with_hints(tokens, account_id_hint, None)?;
            if account_note.is_some() {
                account.account_note = account_note;
                save_account(&account)?;
            }
            Ok(account)
        }
        CodexJsonImportCandidate::AccessToken {
            access_token,
            account_note,
        } => upsert_account_from_access_token(access_token, account_note),
        CodexJsonImportCandidate::RefreshToken {
            refresh_token,
            account_note,
        } => upsert_account_from_refresh_token(refresh_token, account_note).await,
    }
}

async fn import_accounts_from_token_lines(content: &str) -> Result<Vec<CodexAccount>, String> {
    let lines: Vec<String> = content
        .lines()
        .filter_map(|line| normalize_optional_ref(Some(line)))
        .collect();

    if lines.is_empty() {
        return Err("Token 不能为空".to_string());
    }

    let mut accounts = Vec::new();
    for line in lines {
        let values = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(serde_json::Value::Array(items)) => items,
            Ok(value) => vec![value],
            Err(_) => vec![serde_json::Value::String(line)],
        };

        for value in values {
            let candidate = extract_codex_import_candidate_from_value(&value).ok_or_else(|| {
                "未找到有效的 Codex Token（需要 accessToken/access_token、id_token + access_token，或 refresh_token）"
                    .to_string()
            })?;
            accounts.push(import_codex_candidate(candidate).await?);
        }
    }

    Ok(accounts)
}

fn is_sub2api_codex_oauth_account(value: &serde_json::Value) -> bool {
    let platform = first_json_string(value, &[&["platform"]])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let account_type = first_json_string(value, &[&["type"]])
        .unwrap_or_default()
        .to_ascii_lowercase();

    platform == "openai" && account_type == "oauth"
}

fn looks_like_sub2api_export(value: &serde_json::Value) -> bool {
    let Some(accounts) = value.get("accounts").and_then(|item| item.as_array()) else {
        return false;
    };

    value.get("exported_at").is_some()
        || value.get("proxies").is_some()
        || accounts
            .iter()
            .any(|item| item.get("credentials").is_some() && item.get("platform").is_some())
}

async fn import_sub2api_export_from_value(
    value: &serde_json::Value,
) -> Result<Option<Vec<CodexAccount>>, String> {
    if !looks_like_sub2api_export(value) {
        return Ok(None);
    }

    let accounts = value
        .get("accounts")
        .and_then(|item| item.as_array())
        .ok_or("Sub2API JSON 缺少 accounts 数组")?;
    let mut imported = Vec::new();

    for (index, item) in accounts.iter().enumerate() {
        if !is_sub2api_codex_oauth_account(item) {
            continue;
        }
        let candidate = extract_codex_import_candidate_from_value(item).ok_or_else(|| {
            format!(
                "Sub2API 第 {} 个 OpenAI OAuth 账号缺少有效 access_token",
                index + 1
            )
        })?;
        imported.push(import_codex_candidate(candidate).await?);
    }

    if imported.is_empty() {
        return Err("Sub2API JSON 中未找到可导入的 OpenAI OAuth access_token".to_string());
    }

    Ok(Some(imported))
}

/// 从 JSON 字符串导入账号
pub async fn import_from_json(json_content: &str) -> Result<Vec<CodexAccount>, String> {
    ensure_storage_writable_for_import()?;
    if !json_content.trim().is_empty()
        && !json_content.trim_start().starts_with('{')
        && !json_content.trim_start().starts_with('[')
    {
        return import_accounts_from_token_lines(json_content).await;
    }

    // 尝试解析为 auth.json 格式
    if let Ok(auth_file) = serde_json::from_str::<CodexAuthFile>(json_content) {
        let fallback_api_key = extract_api_key_from_auth_file(&auth_file);
        let fallback_provider = infer_api_provider_config(
            extract_api_base_url_from_auth_file(&auth_file).as_deref(),
            None,
            None,
            None,
        );
        if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
            let api_key = fallback_api_key.ok_or("auth.json 缺少 OPENAI_API_KEY")?;
            return Ok(vec![upsert_api_key_account(
                api_key,
                fallback_provider.base_url.clone(),
                Some(fallback_provider.mode),
                fallback_provider.provider_id.clone(),
                fallback_provider.provider_name.clone(),
            )?]);
        }

        if let Some(tokens) = auth_file.tokens {
            let account = upsert_account_from_auth_tokens(tokens)?;
            return Ok(vec![account]);
        }

        if let Some(api_key) = fallback_api_key {
            return Ok(vec![upsert_api_key_account(
                api_key,
                fallback_provider.base_url.clone(),
                Some(fallback_provider.mode),
                fallback_provider.provider_id.clone(),
                fallback_provider.provider_name.clone(),
            )?]);
        }
    }

    // 尝试解析为单账号（顶层 token）或通用数组（支持混合对象）
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_content) {
        if let Some(accounts) = import_sub2api_export_from_value(&parsed).await? {
            return Ok(accounts);
        }

        match parsed {
            serde_json::Value::Object(_) | serde_json::Value::String(_) => {
                if is_auth_mode_apikey(
                    parsed
                        .get("auth_mode")
                        .and_then(|value| value.as_str())
                        .or_else(|| parsed.get("authMode").and_then(|value| value.as_str())),
                ) {
                    if let Some(api_key) = parsed
                        .get("OPENAI_API_KEY")
                        .and_then(|value| value.as_str())
                        .and_then(normalize_api_key)
                    {
                        return Ok(vec![upsert_api_key_account(
                            api_key,
                            extract_api_base_url_from_json_value(&parsed),
                            None,
                            parsed
                                .get("api_provider_id")
                                .and_then(|value| value.as_str())
                                .map(|value| value.to_string()),
                            parsed
                                .get("api_provider_name")
                                .and_then(|value| value.as_str())
                                .map(|value| value.to_string()),
                        )?]);
                    }
                }

                if let Some(candidate) = extract_codex_import_candidate_from_value(&parsed) {
                    let account = import_codex_candidate(candidate).await?;
                    return Ok(vec![account]);
                }

                if let Ok(account) = serde_json::from_value::<CodexAccount>(parsed) {
                    let imported = import_account_struct(account)?;
                    return Ok(vec![imported]);
                }
            }
            serde_json::Value::Array(items) => {
                let mut result = Vec::new();

                for item in items {
                    if let Some(candidate) = extract_codex_import_candidate_from_value(&item) {
                        result.push(import_codex_candidate(candidate).await?);
                        continue;
                    }

                    if is_auth_mode_apikey(
                        item.get("auth_mode")
                            .and_then(|value| value.as_str())
                            .or_else(|| item.get("authMode").and_then(|value| value.as_str())),
                    ) {
                        if let Some(api_key) = item
                            .get("OPENAI_API_KEY")
                            .and_then(|value| value.as_str())
                            .and_then(normalize_api_key)
                        {
                            result.push(upsert_api_key_account(
                                api_key,
                                extract_api_base_url_from_json_value(&item),
                                None,
                                item.get("api_provider_id")
                                    .and_then(|value| value.as_str())
                                    .map(|value| value.to_string()),
                                item.get("api_provider_name")
                                    .and_then(|value| value.as_str())
                                    .map(|value| value.to_string()),
                            )?);
                            continue;
                        }
                    }

                    if let Ok(account) = serde_json::from_value::<CodexAccount>(item) {
                        result.push(import_account_struct(account)?);
                    }
                }

                if !result.is_empty() {
                    return Ok(result);
                }
            }
            _ => {}
        }
    }

    // 尝试解析为账号数组
    if let Ok(accounts) = serde_json::from_str::<Vec<CodexAccount>>(json_content) {
        let mut result = Vec::new();
        for acc in accounts {
            let imported = import_account_struct(acc)?;
            result.push(imported);
        }
        return Ok(result);
    }

    Err("无法解析 JSON 内容".to_string())
}

/// 导出账号为 JSON
pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<CodexAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();

    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

#[derive(serde::Serialize, Clone)]
pub struct CodexFileImportResult {
    pub imported: Vec<CodexAccount>,
    pub failed: Vec<CodexFileImportFailure>,
}

#[derive(serde::Serialize, Clone)]
pub struct CodexFileImportFailure {
    pub email: String,
    pub error: String,
}

/// 从单个 JSON 值中提取 CodexTokens
fn extract_codex_tokens_from_value(
    value: &serde_json::Value,
) -> Option<(CodexTokens, Option<String>)> {
    let obj = value.as_object()?;

    // 格式1: 顶层 access_token + id_token（用户导出格式）
    if let (Some(id_token), Some(access_token)) = (
        first_json_string(value, &[&["id_token"], &["idToken"]]),
        first_json_string(value, &[&["access_token"], &["accessToken"]]),
    ) {
        let refresh_token = first_json_string(
            value,
            &[
                &["refresh_token"],
                &["refreshToken"],
                &["session_token"],
                &["sessionToken"],
            ],
        );
        let account_id_hint = first_json_string(value, &[&["account_id"], &["accountId"]]);
        return Some((
            CodexTokens {
                id_token,
                access_token,
                refresh_token,
            },
            account_id_hint,
        ));
    }

    // 格式2: 嵌套 tokens 对象（CodexAuthFile 或 CodexAccount 格式）
    if obj.get("tokens").and_then(|v| v.as_object()).is_some() {
        if let (Some(id_token), Some(access_token)) = (
            first_json_string(value, &[&["tokens", "id_token"], &["tokens", "idToken"]]),
            first_json_string(
                value,
                &[&["tokens", "access_token"], &["tokens", "accessToken"]],
            ),
        ) {
            let refresh_token = first_json_string(
                value,
                &[
                    &["tokens", "refresh_token"],
                    &["tokens", "refreshToken"],
                    &["tokens", "session_token"],
                    &["tokens", "sessionToken"],
                    &["session_token"],
                    &["sessionToken"],
                ],
            );
            let account_id_hint = first_json_string(
                value,
                &[
                    &["tokens", "account_id"],
                    &["tokens", "accountId"],
                    &["account_id"],
                    &["accountId"],
                ],
            );
            return Some((
                CodexTokens {
                    id_token,
                    access_token,
                    refresh_token,
                },
                account_id_hint,
            ));
        }
    }

    None
}
