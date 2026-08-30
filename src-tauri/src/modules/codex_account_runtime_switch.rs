// Codex 账号模块：Managed runtime validation, injection preparation and account switching。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
pub fn is_managed_auth_refresh_due(account: &CodexAccount) -> bool {
    if account.is_api_key_auth() || account.requires_reauth || !account_has_refresh_token(account) {
        return false;
    }

    if managed_account_tokens_need_refresh(account) {
        return true;
    }

    account
        .token_updated_at
        .map(|updated_at| updated_at <= now_timestamp() - CODEX_PROACTIVE_REFRESH_INTERVAL_SECONDS)
        .unwrap_or(true)
}

async fn perform_managed_token_refresh(
    mut account: CodexAccount,
    reason: &str,
    force: bool,
) -> Result<CodexAccount, String> {
    crate::modules::codex_auth_diagnostic::log_event(
        "managed_token_refresh_start",
        serde_json::json!({
            "account_id": account.id,
            "email": account.email,
            "reason": reason,
            "force": force,
            "token_generation": account.token_generation,
            "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&account.tokens),
        }),
    );
    let refresh_token = match account
        .tokens
        .refresh_token
        .clone()
        .filter(|token| !token.trim().is_empty())
    {
        Some(token) => token,
        None => {
            logger::log_warn(&format!(
                "Codex Token Authority 跳过刷新：账号缺少 refresh_token，按 access-token-only 模式继续使用当前 access_token: account_id={}, email={}, reason={}",
                account.id, account.email, reason
            ));
            if force || codex_oauth::is_token_expired(&account.tokens.access_token) {
                mark_account_requires_reauth(
                    &mut account,
                    CODEX_MISSING_REFRESH_TOKEN_REAUTH_REASON,
                )?;
                return Err(CODEX_MISSING_REFRESH_TOKEN_REAUTH_REASON.to_string());
            }
            return Ok(account);
        }
    };

    logger::log_info(&format!(
        "Codex Token Authority 开始刷新: account_id={}, email={}, reason={}",
        account.id, account.email, reason
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
            sync_managed_account_sidecar(&account);
            crate::modules::codex_auth_diagnostic::log_event(
                "managed_token_refresh_saved",
                serde_json::json!({
                    "account_id": account.id,
                    "email": account.email,
                    "reason": reason,
                    "token_generation": account.token_generation,
                    "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&account.tokens),
                }),
            );
            logger::log_info(&format!(
                "Codex Token Authority 刷新成功: account_id={}, generation={}",
                account.id, account.token_generation
            ));
            Ok(account)
        }
        Err(err) => {
            let user_error = format_refresh_error_for_user(&err);
            crate::modules::codex_auth_diagnostic::log_event(
                "managed_token_refresh_error",
                serde_json::json!({
                    "account_id": account.id,
                    "email": account.email,
                    "reason": reason,
                    "force": force,
                    "error": user_error,
                    "refresh_error_kind": format!("{:?}", classify_refresh_error(&err)),
                }),
            );
            if !force && is_refresh_token_reused_error(&err) {
                // refresh_token_reused 只表示本次自动轮换没有成功，不再把账号写成
                // requires_reauth，也不阻断切号/启动；让官方客户端和真实 API 请求
                // 自己决定当前凭据是否仍可用。预览页 force=true 仍会把明确错误返回给用户。
                logger::log_warn(&format!(
                    "Codex 自动刷新遇到 refresh_token_reused，忽略账号状态与切号限制: account_id={}, reason={}",
                    account.id, reason
                ));
                return Ok(account);
            }
            if is_reauth_required_refresh_error(&err) {
                let _ = mark_account_requires_reauth(&mut account, &user_error);
                return Err(user_error);
            }
            Err(user_error)
        }
    }
}

async fn validate_managed_account_for_client_locked(
    mut account: CodexAccount,
    reason: &str,
) -> Result<CodexAccount, String> {
    if account.is_api_key_auth()
        || account.is_agent_identity_auth()
        || account.is_web_session_auth()
    {
        return Ok(account);
    }
    if let Err(error) = clear_stale_id_token_reauth(&mut account) {
        logger::log_warn(&format!(
            "清理旧版 id_token 重登标记失败，继续执行本地凭据校验: account_id={}, error={}",
            account.id, error
        ));
    }
    if account.requires_reauth {
        return Err(account
            .reauth_reason
            .clone()
            .unwrap_or_else(|| "账号需要重新授权".to_string()));
    }
    if codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Err("access_token 已过期，无法启动，请重新授权".to_string());
    }
    logger::log_info(&format!(
        "本地 Codex 启动凭据校验通过: account_id={}, email={}, reason={}",
        account.id, account.email, reason
    ));
    Ok(account)
}

async fn refresh_managed_account_locked(
    account_id: &str,
    force: bool,
    reason: &str,
    observed_generation: Option<u64>,
    validate_for_client: bool,
    retry_known_reauth: bool,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() || account.is_agent_identity_auth() {
        return finish_managed_runtime_account_refresh(account, validate_for_client);
    }
    let official_runtime_has_account = running_codex_oauth_account_ids()
        .map(|account_ids| account_ids.contains(&account.id))
        .unwrap_or(false);
    let sync_result = if official_runtime_has_account {
        sync_account_from_live_authority_sources(&mut account)
    } else {
        sync_account_from_authority_sources(&mut account)
    };
    if let Err(err) = sync_result {
        logger::log_warn(&format!(
            "Codex 账号刷新前同步官方凭证失败，继续使用账号库: account_id={}, error={}",
            account.id, err
        ));
    }
    clear_refresh_token_reused_state(&mut account)?;
    if let Err(err) = clear_stale_missing_refresh_token_reauth(&mut account) {
        logger::log_warn(&format!(
            "Codex 清理缺失 refresh_token 的过期重登标记失败，继续处理: account_id={}, error={}",
            account.id, err
        ));
    }
    if let Err(err) = clear_stale_id_token_reauth(&mut account) {
        logger::log_warn(&format!(
            "Codex 清理旧版 id_token 重登标记失败，继续处理: account_id={}, error={}",
            account.id, err
        ));
    }
    let token_refresh_due = if validate_for_client {
        managed_account_runtime_tokens_need_refresh(&account)
    } else {
        managed_account_tokens_need_refresh(&account)
    };
    let should_revalidate_known_reauth =
        retry_known_reauth && account.requires_reauth && token_refresh_due;
    if account.requires_reauth && token_refresh_due && !should_revalidate_known_reauth {
        return Err(account
            .reauth_reason
            .clone()
            .unwrap_or_else(|| "账号需要重新登录".to_string()));
    }
    if let Some(observed_generation) = observed_generation {
        if account.token_generation > observed_generation {
            let needs_refresh = if validate_for_client {
                managed_account_runtime_tokens_need_refresh(&account)
            } else {
                managed_account_tokens_need_refresh(&account)
            };
            if !needs_refresh && !should_revalidate_known_reauth {
                logger::log_info(&format!(
                    "Codex Token Authority 复用已完成的刷新结果: account_id={}, observed_generation={}, current_generation={}, reason={}",
                    account.id,
                    observed_generation,
                    account.token_generation,
                    reason
                ));
                return finish_managed_runtime_account_refresh(account, validate_for_client);
            }
            logger::log_warn(&format!(
                "Codex Token Authority 检测到刷新代际已推进但 OAuth token 仍过期，继续刷新: account_id={}, observed_generation={}, current_generation={}, reason={}",
                account.id,
                observed_generation,
                account.token_generation,
                reason
            ));
        }
    }
    let needs_refresh = managed_account_refresh_needed_for_request(
        &account,
        validate_for_client,
        should_revalidate_known_reauth,
    );
    if !force && !needs_refresh {
        return finish_managed_runtime_account_refresh(account, validate_for_client);
    }

    let account = perform_managed_token_refresh(account, reason, force).await?;
    finish_managed_runtime_account_refresh(account, validate_for_client)
}

async fn refresh_managed_account_with_authority(
    account_id: &str,
    force: bool,
    reason: &str,
    observed_generation: Option<u64>,
) -> Result<CodexAccount, String> {
    // A force refresh can be requested after a stale 401 response. Capture the
    // generation before waiting for the lock so a refresh completed by another
    // caller/process is reused instead of consuming the rotated refresh token
    // a second time.
    let observed_generation =
        observed_generation.or_else(|| loaded_account_token_generation(account_id));
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, reason).await?;
    refresh_managed_account_locked(account_id, force, reason, observed_generation, false, false)
        .await
}

async fn refresh_bound_oauth_account_for_api_key(
    api_key_account: &CodexAccount,
    reason: &str,
    validate_for_client: bool,
    retry_known_reauth: bool,
) -> Result<CodexAccount, String> {
    let bound_id = api_key_account
        .bound_oauth_account_id
        .as_deref()
        .ok_or_else(|| "API Key 账号需先绑定 OAuth 账号".to_string())?
        .to_string();
    let _ = validate_api_key_bound_oauth_account(api_key_account, &bound_id)?;
    let observed_generation = loaded_account_token_generation(&bound_id);
    let lock = codex_token_lock_for(&bound_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(&bound_id, reason).await?;
    let account = refresh_managed_account_locked(
        &bound_id,
        false,
        reason,
        observed_generation,
        validate_for_client,
        retry_known_reauth,
    )
    .await?;
    if validate_for_client {
        validate_managed_account_for_client_locked(account, reason).await
    } else {
        Ok(account)
    }
}

async fn refresh_bound_oauth_account_for_api_key_locked(
    api_key_account: &CodexAccount,
    reason: &str,
    validate_for_client: bool,
    retry_known_reauth: bool,
) -> Result<CodexAccount, String> {
    let bound_id = api_key_account
        .bound_oauth_account_id
        .as_deref()
        .ok_or_else(|| "API Key 账号需先绑定 OAuth 账号".to_string())?
        .to_string();
    let _ = validate_api_key_bound_oauth_account(api_key_account, &bound_id)?;
    let account = refresh_managed_account_locked(
        &bound_id,
        false,
        reason,
        None,
        validate_for_client,
        retry_known_reauth,
    )
    .await?;
    if validate_for_client {
        validate_managed_account_for_client_locked(account, reason).await
    } else {
        Ok(account)
    }
}

pub async fn ensure_managed_account_fresh(account_id: &str) -> Result<CodexAccount, String> {
    refresh_managed_account_with_authority(account_id, false, "prepare", None).await
}

pub async fn force_refresh_managed_account(
    account_id: &str,
    reason: &str,
) -> Result<CodexAccount, String> {
    refresh_managed_account_with_authority(account_id, true, reason, None).await
}

pub async fn force_refresh_managed_account_after_observed(
    account_id: &str,
    observed_generation: u64,
    reason: &str,
) -> Result<CodexAccount, String> {
    refresh_managed_account_with_authority(account_id, true, reason, Some(observed_generation))
        .await
}

pub async fn keepalive_managed_account(
    account_id: &str,
    reason: &str,
) -> Result<CodexAccount, String> {
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, reason).await?;
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() || account.is_agent_identity_auth() {
        return Ok(account);
    }
    let official_runtime_has_account = running_codex_oauth_account_ids()
        .map(|account_ids| account_ids.contains(&account.id))
        .unwrap_or(false);
    let sync_result = if official_runtime_has_account {
        sync_account_from_live_authority_sources(&mut account)
    } else {
        sync_account_from_authority_sources(&mut account)
    };
    if let Err(err) = sync_result {
        logger::log_warn(&format!(
            "Codex 保活同步官方凭证失败，继续使用账号库: account_id={}, error={}",
            account.id, err
        ));
    }
    clear_refresh_token_reused_state(&mut account)?;
    if let Err(err) = clear_stale_missing_refresh_token_reauth(&mut account) {
        logger::log_warn(&format!(
            "Codex 保活清理缺失 refresh_token 的过期重登标记失败，继续处理: account_id={}, error={}",
            account.id, err
        ));
    }
    if account.requires_reauth {
        return Err(account
            .reauth_reason
            .clone()
            .unwrap_or_else(|| "账号需要重新登录".to_string()));
    }
    if !is_managed_auth_refresh_due(&account) {
        return Ok(account);
    }

    perform_managed_token_refresh(account, reason, false).await
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
    let api_key_account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if api_key_account.is_api_key_auth() {
        let sync_error = if normalize_optional_ref(
            api_key_account.bound_oauth_account_id.as_deref(),
        )
        .is_some()
        {
            let oauth_account =
                refresh_bound_oauth_account_for_api_key(&api_key_account, reason, false, false)
                    .await?;
            write_api_key_account_bundle_with_oauth_to_dir(
                auth_dir,
                &api_key_account,
                &oauth_account,
            )?;

            let sync_result =
                match sync_managed_projection_from_auth_dir(&oauth_account.id, auth_dir) {
                    Ok(_) => {
                        let latest_oauth_account = load_account(&oauth_account.id)
                            .unwrap_or_else(|| oauth_account.clone());
                        match write_api_key_account_bundle_with_oauth_to_dir(
                            auth_dir,
                            &api_key_account,
                            &latest_oauth_account,
                        ) {
                            Ok(_) => None,
                            Err(err) => Some(err),
                        }
                    }
                    Err(err) => Some(err),
                };
            sync_result
        } else {
            write_prepared_account_bundle_to_dir(auth_dir, &api_key_account)?;
            None
        };
        let result = operation(&api_key_account);
        let latest_account = load_account(account_id).unwrap_or(api_key_account);

        return Ok((latest_account, result, sync_error));
    }

    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, reason).await?;
    let account =
        refresh_managed_account_locked(account_id, false, reason, None, false, false).await?;
    write_prepared_account_bundle_to_dir(auth_dir, &account)?;

    let result = operation(&account);
    let sync_error = match sync_managed_projection_from_auth_dir(account_id, auth_dir) {
        Ok(_) => None,
        Err(err) => Some(err),
    };
    let latest_account = load_account(account_id).unwrap_or(account);

    Ok((latest_account, result, sync_error))
}

/// 准备账号注入：刷新前会先采用更新的官方凭证，目标 profile 仅在本次显式注入时写入。
pub async fn prepare_account_for_injection_from_auth_dir(
    account_id: &str,
    auth_dir: Option<&Path>,
) -> Result<CodexAccount, String> {
    prepare_account_for_injection_from_auth_dir_impl(account_id, auth_dir, false)
    .await
}

/// 实例启动专用凭据准备。
///
/// 启动投影阶段只按本地凭据刷新与写入流程处理，不在这里重复网络检查。
pub async fn prepare_account_for_instance_launch_from_auth_dir(
    account_id: &str,
    auth_dir: Option<&Path>,
) -> Result<CodexAccount, String> {
    prepare_account_for_injection_from_auth_dir_impl(account_id, auth_dir, true).await
}

/// 实例关闭旧运行态前的凭据预检。仅在 access_token 过期时，
/// 先在 Token Authority 内完成 refresh_token 刷新。
/// 此阶段不写目标 profile，也不调用不存在的内部配置来源路径。
pub async fn prepare_account_for_instance_launch_preflight(
    account_id: &str,
) -> Result<CodexAccount, String> {
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_agent_identity_auth() {
        return Err("Agent Identity 账号仅支持 API 服务，无法用于客户端或 CLI 启动".to_string());
    }
    if account.is_web_session_auth() {
        return Err("Web Session 账号仅支持查看额度，无法用于客户端或 CLI 启动".to_string());
    }
    if account.is_api_key_auth() {
        if let Some(bound_id) = normalize_optional_ref(account.bound_oauth_account_id.as_deref()) {
            let _ = validate_api_key_bound_oauth_account(&account, &bound_id)?;
            let observed_generation = loaded_account_token_generation(&bound_id);
            let lock = codex_token_lock_for(&bound_id);
            let _guard = lock.lock().await;
            let _file_guard = acquire_codex_token_refresh_file_lock(&bound_id, "prepare").await?;
            refresh_managed_account_locked(
                &bound_id,
                false,
                "prepare",
                observed_generation,
                true,
                true,
            )
            .await?;
        }
        return Ok(account);
    }

    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, "prepare").await?;
    refresh_managed_account_locked(account_id, false, "prepare", None, true, true).await
}

/// 预检通过后，把账号库中的最新凭据投影到实例目录。该步骤不再发起网络请求，
/// 也不会再次轮换 refresh_token。
pub async fn project_preflighted_account_for_instance_launch(
    account_id: &str,
    auth_dir: &Path,
) -> Result<CodexAccount, String> {
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_agent_identity_auth() {
        return Err("Agent Identity 账号仅支持 API 服务，无法用于客户端或 CLI 启动".to_string());
    }
    if account.is_web_session_auth() {
        return Err("Web Session 账号仅支持查看额度，无法用于客户端或 CLI 启动".to_string());
    }

    let lock_account_id = if account.is_api_key_auth() {
        normalize_optional_ref(account.bound_oauth_account_id.as_deref())
            .unwrap_or_else(|| account.id.clone())
    } else {
        account.id.clone()
    };
    let lock = codex_token_lock_for(&lock_account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(&lock_account_id, "project").await?;
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        if let Some(bound_id) = normalize_optional_ref(account.bound_oauth_account_id.as_deref()) {
            let oauth_account = load_account(&bound_id)
                .ok_or_else(|| format!("绑定的 OAuth 账号不存在: {}", bound_id))?;
            write_api_key_account_bundle_with_oauth_to_dir(auth_dir, &account, &oauth_account)?;
        } else {
            write_prepared_account_bundle_to_dir(auth_dir, &account)?;
        }
    } else {
        write_prepared_account_bundle_to_dir(auth_dir, &account)?;
    }
    Ok(account)
}

async fn prepare_account_for_injection_from_auth_dir_impl(
    account_id: &str,
    auth_dir: Option<&Path>,
    retry_known_reauth: bool,
) -> Result<CodexAccount, String> {
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_agent_identity_auth() {
        return Err("Agent Identity 账号仅支持 API 服务，无法用于客户端或 CLI 启动".to_string());
    }
    if account.is_web_session_auth() {
        return Err("Web Session 账号仅支持查看额度，无法用于客户端或 CLI 启动".to_string());
    }
    if account.is_api_key_auth() {
        if let Some(dir) = auth_dir {
            if normalize_optional_ref(account.bound_oauth_account_id.as_deref()).is_some() {
                let oauth_account = refresh_bound_oauth_account_for_api_key(
                    &account,
                    "prepare",
                    false,
                    retry_known_reauth,
                )
                .await?;
                write_api_key_account_bundle_with_oauth_to_dir(dir, &account, &oauth_account)?;
            } else {
                write_prepared_account_bundle_to_dir(dir, &account)?;
            }
        }
        return Ok(account);
    }

    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, "prepare").await?;
    let account = refresh_managed_account_locked(account_id, false, "prepare", None, false, retry_known_reauth).await?;
    if let Some(dir) = auth_dir {
        write_prepared_account_bundle_to_dir(dir, &account)?;
    }
    Ok(account)
}

pub async fn prepare_account_for_injection(account_id: &str) -> Result<CodexAccount, String> {
    prepare_account_for_injection_from_store(account_id).await
}

/// 准备账号注入（账号中心模式）：
/// 只更新 Cockpit 账号库；刷新前采用官方运行态中最新的有效凭据。
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
    write_prepared_account_bundle_to_dir(&codex_home, &account_for_write)?;
    logger::log_info(&format!(
        "[Codex切号] 已替换目录登录信息: target_dir={}, target_file={}",
        codex_home.display(),
        auth_path.display()
    ));
    sync_default_codex_account_to_wsl(&account_for_write.id, |wsl_dir| {
        write_prepared_account_bundle_to_dir(wsl_dir, &account_for_write)
    });

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

async fn activate_provider_gateway_after_switch_if_needed(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    if crate::modules::codex_local_access::account_requires_provider_gateway(account) {
        logger::log_info(&format!(
            "[Codex切号] API Key 账号启用本地供应商网关: account_id={}, target_dir={}",
            account.id,
            base_dir.display()
        ));
        crate::modules::codex_local_access::ensure_provider_gateway_for_dir(base_dir, &account.id)
            .await?;
        return Ok(());
    }

    if crate::modules::codex_local_access::account_requires_bound_oauth_local_gateway(account) {
        logger::log_info(&format!(
            "[Codex切号] API Key 账号绑定 OAuth 且禁用 image_generation，启用 Responses 本地网关: account_id={}, target_dir={}",
            account.id,
            base_dir.display()
        ));
        crate::modules::codex_local_access::ensure_bound_oauth_local_gateway_for_dir(
            base_dir,
            &account.id,
        )
        .await?;
        return Ok(());
    }

    crate::modules::codex_local_access::stop_provider_gateways_for_profile(base_dir).await;
    Ok(())
}

/// 若导入结果包含当前激活账号，则重新切号落盘，避免库内 token 已更新但运行中仍用旧凭证。
/// 成功时返回已重新激活的账号，便于调用方补跑 Hermes/OpenCode/OpenClaw 等切号副作用。
/// 重新激活失败只记日志，不打断导入成功结果。
pub async fn reactivate_if_imported_matches_current(
    imported: &[CodexAccount],
) -> Option<CodexAccount> {
    let current_id = load_account_index().current_account_id?;
    if !imported
        .iter()
        .any(|account| account.id.as_str() == current_id.as_str())
    {
        return None;
    }

    match switch_account_managed(&current_id).await {
        Ok(account) => {
            logger::log_info(&format!(
                "[Codex导入] 当前账号已重新激活: id={}, email={}",
                account.id, account.email
            ));
            Some(account)
        }
        Err(error) => {
            logger::log_error(&format!(
                "[Codex导入] 当前账号重新激活失败（导入已成功）: id={}, error={}",
                current_id, error
            ));
            None
        }
    }
}

enum PreparedCodexAccountSwitch {
    Account(CodexAccount),
    ApiKeyWithOauth {
        api_key_account: CodexAccount,
        oauth_account: CodexAccount,
    },
}

async fn prepare_account_switch_locked(
    account_id: &str,
    retry_known_reauth: bool,
) -> Result<PreparedCodexAccountSwitch, String> {
    let account = load_account_after_index_repair(account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_agent_identity_auth() {
        return Err("Agent Identity 账号仅支持 API 服务，无法作为普通账号切换".to_string());
    }
    if account.is_web_session_auth() {
        return Err("Web Session 账号仅支持查看额度，无法作为普通账号切换或启动".to_string());
    }
    if account.is_api_key_auth() {
        if normalize_optional_ref(account.bound_oauth_account_id.as_deref()).is_none() {
            return Ok(PreparedCodexAccountSwitch::Account(account));
        }
        let oauth_account = refresh_bound_oauth_account_for_api_key_locked(
            &account,
            "switch",
            true,
            retry_known_reauth,
        )
        .await?;
        return Ok(PreparedCodexAccountSwitch::ApiKeyWithOauth {
            api_key_account: account,
            oauth_account,
        });
    }

    let account = refresh_managed_account_locked(
        account_id,
        false,
        "switch",
        None,
        true,
        retry_known_reauth,
    )
    .await?;
    let account = validate_managed_account_for_client_locked(account, "switch").await?;
    Ok(PreparedCodexAccountSwitch::Account(account))
}

fn prepare_freshly_reauthorized_account_switch_local_locked(
    account_id: &str,
    expected_token_generation: u64,
) -> Result<CodexAccount, String> {
    let account = load_account_after_index_repair(account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth()
        || account.is_agent_identity_auth()
        || account.is_web_session_auth()
    {
        return Err("重新授权后的快速切号仅支持普通 OAuth 账号".to_string());
    }
    if account.token_generation != expected_token_generation {
        return Err("重新授权后的账号凭据已发生变化，已停止自动切号，请重新点击切换。".to_string());
    }
    if codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Err("重新授权返回的 access_token 已过期或无效，已停止自动切号。".to_string());
    }
    Ok(account)
}

async fn prepare_freshly_reauthorized_account_switch_locked(
    account_id: &str,
    expected_token_generation: u64,
) -> Result<PreparedCodexAccountSwitch, String> {
    let account = prepare_freshly_reauthorized_account_switch_local_locked(
        account_id,
        expected_token_generation,
    )?;
    let account = validate_managed_account_for_client_locked(account, "reauth-switch").await?;
    Ok(PreparedCodexAccountSwitch::Account(account))
}

async fn commit_account_switch_locked(
    account_id: &str,
    prepared: PreparedCodexAccountSwitch,
) -> Result<CodexAccount, String> {
    match prepared {
        PreparedCodexAccountSwitch::Account(account) => {
            let updated_account = switch_account_with_prepared(account_id, account)?;
            let codex_home = get_codex_home();
            activate_provider_gateway_after_switch_if_needed(&codex_home, &updated_account).await?;
            Ok(updated_account)
        }
        PreparedCodexAccountSwitch::ApiKeyWithOauth {
            api_key_account: account,
            oauth_account,
        } => {
            let codex_home = get_codex_home();
            let auth_path = codex_home.join("auth.json");
            logger::log_info(&format!(
                "[Codex切号] 开始切换 API Key 账号绑定 OAuth: api_account_id={}, oauth_account_id={}, target_dir={}",
                account.id,
                oauth_account.id,
                codex_home.display()
            ));
            write_api_key_account_bundle_with_oauth_to_dir(&codex_home, &account, &oauth_account)?;
            logger::log_info(&format!(
                "[Codex切号] 已替换目录登录信息: target_dir={}, target_file={}",
                codex_home.display(),
                auth_path.display()
            ));
            sync_default_codex_account_to_wsl(&account.id, |wsl_dir| {
                write_api_key_account_bundle_with_oauth_to_dir(wsl_dir, &account, &oauth_account)
            });

            let mut index = load_account_index();
            index.current_account_id = Some(account_id.to_string());
            save_account_index(&index)?;

            let mut updated_account = account.clone();
            updated_account.update_last_used();
            save_account(&updated_account)?;

            logger::log_info(&format!(
                "已切换到 Codex API Key 账号: {}，登录态绑定 OAuth: {}",
                updated_account.email, oauth_account.email
            ));

            activate_provider_gateway_after_switch_if_needed(&codex_home, &updated_account).await?;

            Ok(updated_account)
        }
    }
}

pub async fn switch_account_managed(account_id: &str) -> Result<CodexAccount, String> {
    switch_account_managed_with_before_commit(account_id, || async { Ok(()) }).await
}

/// 切号事务：先同步当前官方凭证并准备目标凭证，准备成功后才停止旧 Codex
/// 运行态并提交。目标凭证准备失败时不会关闭当前客户端；`before_commit`
/// 失败时不会覆盖 auth.json / keyring，也不会更新当前账号索引。
pub async fn switch_account_managed_with_before_commit<F, Fut>(
    account_id: &str,
    before_commit: F,
) -> Result<CodexAccount, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    switch_account_managed_with_before_commit_options(account_id, false, before_commit)
    .await
}

/// 用户从账号页主动切号时使用：每次都重新读取当前凭据，并按统一 Token Authority
/// 规则刷新 access_token/id_token；不额外调用远端账号检查接口。
pub async fn switch_account_managed_with_before_commit_and_revalidation<F, Fut>(
    account_id: &str,
    before_commit: F,
) -> Result<CodexAccount, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    switch_account_managed_with_before_commit_and_revalidation_options(account_id, before_commit)
        .await
}

pub async fn switch_account_managed_with_before_commit_and_revalidation_options<F, Fut>(
    account_id: &str,
    before_commit: F,
) -> Result<CodexAccount, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    switch_account_managed_with_before_commit_options(account_id, true, before_commit)
    .await
}

/// OAuth 重新授权成功后的受控切号。
///
/// 本次授权已经返回并保存了新的 Token，因此不能再从仍在运行的旧客户端同步凭据，
/// 也不能立即再次轮换 refresh_token。先校验 OAuth 完成时观察到的 token
/// generation 和 access_token 有效期，通过本地凭据校验后再停止旧运行态；
/// id_token 只随刷新结果保存，不作为启动阻断条件。
pub async fn switch_account_managed_after_reauth_with_before_commit<F, Fut>(
    account_id: &str,
    expected_token_generation: u64,
    before_commit: F,
) -> Result<CodexAccount, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    switch_account_managed_after_reauth_with_before_commit_options(
        account_id,
        expected_token_generation,
        before_commit,
    )
    .await
}

pub async fn switch_account_managed_after_reauth_with_before_commit_options<F, Fut>(
    account_id: &str,
    expected_token_generation: u64,
    before_commit: F,
) -> Result<CodexAccount, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    crate::modules::codex_auth_diagnostic::log_event(
        "reauth_switch_start",
        serde_json::json!({
            "account_id": account_id,
            "expected_token_generation": expected_token_generation,
        }),
    );
    let _switch_guard = CODEX_ACCOUNT_SWITCH_LOCK.lock().await;
    let token_lock = codex_token_lock_for(account_id);
    let _token_guard = token_lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, "reauth-switch").await?;
    let prepared =
        prepare_freshly_reauthorized_account_switch_locked(account_id, expected_token_generation)
            .await?;
    before_commit().await?;
    let result = commit_account_switch_locked(account_id, prepared).await;
    crate::modules::codex_auth_diagnostic::log_event(
        "reauth_switch_finished",
        match &result {
            Ok(account) => serde_json::json!({
                "account_id": account.id,
                "success": true,
                "token_generation": account.token_generation,
                "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&account.tokens),
            }),
            Err(error) => serde_json::json!({
                "account_id": account_id,
                "success": false,
                "error": error,
            }),
        },
    );
    result
}

pub async fn switch_account_managed_with_before_commit_options<F, Fut>(
    account_id: &str,
    retry_known_reauth: bool,
    before_commit: F,
) -> Result<CodexAccount, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    switch_account_managed_with_before_commit_internal(account_id, retry_known_reauth, before_commit)
    .await
}
async fn switch_account_managed_with_before_commit_internal<F, Fut>(
    account_id: &str,
    retry_known_reauth: bool,
    before_commit: F,
) -> Result<CodexAccount, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let _switch_guard = CODEX_ACCOUNT_SWITCH_LOCK.lock().await;
    sync_active_official_account_before_switch().await?;

    let switch_lock_account_id = load_account_after_index_repair(account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))
        .map(|account| {
            if account.is_api_key_auth() {
                account
                    .bound_oauth_account_id
                    .clone()
                    .unwrap_or_else(|| account.id.clone())
            } else {
                account.id.clone()
            }
        })?;
    // Keep the token lock through preparation, stopping the old runtime, and
    // the final auth.json/keyring commit. Otherwise a background refresh can
    // update the account store between those phases and the prepared stale
    // snapshot would overwrite the fresh token during commit.
    let token_lock = codex_token_lock_for(&switch_lock_account_id);
    let _token_guard = token_lock.lock().await;
    let _file_guard =
        acquire_codex_token_refresh_file_lock(&switch_lock_account_id, "switch").await?;
    // 先完成目标凭据准备；账号级 Token 锁会串行化刷新与最终投影，
    // 但不会因为同一 OAuth 正被其它官方实例使用而阻断切换。
    let prepared = prepare_account_switch_locked(
        account_id,
        retry_known_reauth,
    )
    .await?;
    // 目标凭据已经通过检查并在账号库中落稳，才关闭旧运行态并提交到官方目录。
    before_commit().await?;
    commit_account_switch_locked(account_id, prepared).await
}
