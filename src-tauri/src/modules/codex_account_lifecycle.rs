// Codex 账号模块：Profile refresh, account upsert and account removal lifecycle。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
/// 刷新账号资料（团队名/结构）
async fn refresh_account_profile_once(account_id: &str) -> Result<CodexAccount, String> {
    let mut account = prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() || account.is_agent_identity_auth() {
        return Ok(account);
    }

    let (account_name, account_structure, account_id_from_remote) =
        fetch_remote_account_profile(&account).await?;

    let mut changed = false;

    if let Some(remote_account_id) = normalize_optional_value(account_id_from_remote) {
        if normalize_optional_ref(account.account_id.as_deref()) != Some(remote_account_id.clone())
        {
            account.account_id = Some(remote_account_id);
            changed = true;
        }
    }

    if let Some(name) = normalize_optional_value(account_name) {
        if normalize_optional_ref(account.account_name.as_deref()) != Some(name.clone()) {
            account.account_name = Some(name);
            changed = true;
        }
    }

    if let Some(structure) = normalize_optional_value(account_structure) {
        if normalize_optional_ref(account.account_structure.as_deref()) != Some(structure.clone()) {
            account.account_structure = Some(structure);
            changed = true;
        }
    }

    if changed {
        save_account(&account)?;
    }

    Ok(account)
}

pub async fn refresh_account_profile(account_id: &str) -> Result<CodexAccount, String> {
    refresh_account_profile_once(account_id).await
}

/// 添加或更新账号
pub fn upsert_account(tokens: CodexTokens) -> Result<CodexAccount, String> {
    upsert_account_with_hints(tokens, None, None)
}

fn build_agent_identity_account_draft(
    identity: CodexAgentIdentity,
) -> Result<CodexAccount, String> {
    let identity = normalize_agent_identity(identity)?;
    let email = identity
        .email
        .clone()
        .unwrap_or_else(|| identity.chatgpt_user_id.clone());
    let account_storage_id =
        build_agent_identity_account_id(&identity.account_id, &identity.chatgpt_user_id);
    let mut account = CodexAccount::new(
        account_storage_id,
        email,
        CodexTokens {
            id_token: String::new(),
            access_token: String::new(),
            refresh_token: None,
        },
    );
    account.agent_identity = Some(identity.clone());
    account.user_id = Some(identity.chatgpt_user_id.clone());
    account.account_id = Some(identity.account_id.clone());
    account.plan_type = identity.plan_type.clone();
    Ok(account)
}

pub fn upsert_agent_identity_account(identity: CodexAgentIdentity) -> Result<CodexAccount, String> {
    let draft = build_agent_identity_account_draft(identity)?;
    let identity = draft
        .agent_identity
        .clone()
        .ok_or("Agent Identity 凭据为空")?;
    let account_storage_id = draft.id.clone();
    let mut index = load_account_index();
    let legacy_account_storage_id = build_legacy_agent_identity_account_id(&identity.account_id);
    let legacy_account = load_account(&legacy_account_storage_id).filter(|account| {
        account.agent_identity.as_ref().is_some_and(|stored| {
            stored.account_id.trim() == identity.account_id
                && stored.chatgpt_user_id.trim() == identity.chatgpt_user_id
        })
    });
    let mut account = load_account(&account_storage_id)
        .or(legacy_account)
        .unwrap_or(draft);
    account.email = identity
        .email
        .clone()
        .unwrap_or_else(|| identity.chatgpt_user_id.clone());
    account.auth_mode = CodexAuthMode::OAuth;
    account.openai_api_key = None;
    account.api_base_url = None;
    account.agent_identity = Some(identity.clone());
    account.user_id = Some(identity.chatgpt_user_id.clone());
    account.account_id = Some(identity.account_id.clone());
    account.plan_type = identity.plan_type.clone();
    account.tokens = CodexTokens {
        id_token: String::new(),
        access_token: String::new(),
        refresh_token: None,
    };
    account.requires_reauth = false;
    account.reauth_reason = None;
    account.authorization_status = None;
    account.client_auth_status = None;
    account.last_client_auth_observed_at = None;
    account.last_client_login_redirect_at = None;
    account.last_client_launch_at = None;
    account.last_client_auth_instance_id = None;
    account.update_last_used();
    save_account_from_user_action(&mut account)?;

    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }
    save_account_index(&index)?;
    Ok(account)
}

pub fn upsert_account_for_reauth(
    tokens: CodexTokens,
    target_account_id: &str,
) -> Result<CodexAccount, String> {
    upsert_account_with_hints_and_reauth_target(tokens, None, None, None, Some(target_account_id))
}

pub fn upsert_api_key_account(
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
    let (api_key, api_base_url) = validate_api_key_credentials(&api_key, api_base_url.as_deref())?;
    let provider_config = resolve_api_provider_config(
        api_base_url.as_deref(),
        api_provider_mode,
        api_provider_id.as_deref(),
        api_provider_name.as_deref(),
    )?;
    let account_id = build_api_key_account_id(&api_key);
    let account_name = normalize_optional_value(account_name);
    let mut index = load_account_index();

    let mut account = if let Some(mut acc) = load_account(&account_id) {
        let sync_model_catalog_to_codex =
            api_sync_model_catalog_to_codex.unwrap_or(acc.api_sync_model_catalog_to_codex);
        apply_api_key_fields(
            &mut acc,
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
        if acc.email.trim().is_empty() {
            acc.email = build_api_key_email(&api_key);
        }
        if let Some(name) = account_name.clone() {
            if normalize_optional_ref(acc.account_name.as_deref()).is_none() {
                acc.account_name = Some(name);
            }
        }
        acc.update_last_used();
        acc
    } else {
        let mut acc = CodexAccount::new_api_key(
            account_id.clone(),
            build_api_key_email(&api_key),
            api_key,
            provider_config.mode.clone(),
            provider_config.base_url.clone(),
            provider_config.provider_id.clone(),
            provider_config.provider_name.clone(),
            normalize_api_model_catalog(api_model_catalog.clone()),
        );
        acc.plan_type = Some(API_KEY_LOGIN_PLAN_TYPE.to_string());
        acc.account_name = account_name;
        acc.api_sync_model_catalog_to_codex = api_sync_model_catalog_to_codex.unwrap_or(false);
        acc.api_wire_api = normalize_api_wire_api(api_wire_api.clone());
        acc.api_supports_websockets = api_supports_websockets;
        let _ = normalize_api_key_websocket_capability(&mut acc);
        acc.api_supports_vision = api_supports_vision;
        acc.api_model_vision_support = normalize_api_model_vision_support(api_model_vision_support);
        acc.api_vision_routing_model = normalize_optional_value(api_vision_routing_model);
        acc
    };

    account.auth_mode = CodexAuthMode::Apikey;
    let _ = enforce_deepseek_responses_account(&mut account);
    if api_model_context_windows.is_some() || !account.api_model_context_windows.is_empty() {
        account.api_model_context_windows = normalize_api_model_context_windows(
            api_model_context_windows.unwrap_or_else(|| account.api_model_context_windows.clone()),
            &account.api_model_catalog,
            &account.api_model_mappings,
        );
    }
    save_account_from_user_action(&mut account)?;

    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.subscription_active_until = account.subscription_active_until.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    save_account_index(&index)?;

    logger::log_info(&format!(
        "Codex API Key 账号已保存: account_id={}, email={}, has_base_url={}",
        account.id,
        account.email,
        normalize_optional_ref(account.api_base_url.as_deref()).is_some()
    ));
    Ok(account)
}

fn upsert_account_with_hints(
    tokens: CodexTokens,
    account_id_hint: Option<String>,
    organization_id_hint: Option<String>,
) -> Result<CodexAccount, String> {
    upsert_account_with_hints_and_reauth_target(
        tokens,
        account_id_hint,
        organization_id_hint,
        None,
        None,
    )
}

fn upsert_account_with_import_hints(
    tokens: CodexTokens,
    account_id_hint: Option<String>,
    organization_id_hint: Option<String>,
    subscription_active_until_hint: Option<String>,
) -> Result<CodexAccount, String> {
    upsert_account_with_hints_and_reauth_target(
        tokens,
        account_id_hint,
        organization_id_hint,
        subscription_active_until_hint,
        None,
    )
}

fn resolve_reauth_target_account_id(
    target_account_id: Option<&str>,
    email: &str,
) -> Result<Option<String>, String> {
    let Some(target_id) = normalize_optional_ref(target_account_id) else {
        return Ok(None);
    };
    let target =
        load_account(&target_id).ok_or_else(|| format!("重新授权目标账号不存在: {}", target_id))?;
    if target.is_api_key_auth() {
        return Err("API Key 账号不能通过 OAuth 重新授权".to_string());
    }
    if !target.email.trim().is_empty() && !target.email.eq_ignore_ascii_case(email) {
        return Err(format!(
            "重新授权账号邮箱不匹配: 目标账号为 {}，本次授权为 {}",
            target.email, email
        ));
    }
    Ok(Some(if target.id.trim().is_empty() {
        target_id
    } else {
        target.id
    }))
}

fn upsert_account_with_hints_and_reauth_target(
    mut tokens: CodexTokens,
    account_id_hint: Option<String>,
    organization_id_hint: Option<String>,
    subscription_active_until_hint: Option<String>,
    reauth_target_account_id: Option<&str>,
) -> Result<CodexAccount, String> {
    crate::modules::codex_auth_diagnostic::log_event(
        if reauth_target_account_id.is_some() {
            "reauth_upsert_start"
        } else {
            "oauth_account_upsert_start"
        },
        serde_json::json!({
            "reauth_target_account_id": reauth_target_account_id,
            "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&tokens),
        }),
    );
    let (
        email,
        user_id,
        plan_type,
        token_subscription_active_until,
        id_token_account_id,
        id_token_org_id,
    ) = extract_user_info(&tokens.id_token)?;
    let subscription_active_until = normalize_optional_value(
        subscription_active_until_hint.or(token_subscription_active_until),
    );
    let account_id = normalize_optional_value(
        extract_chatgpt_account_id_from_access_token(&tokens.access_token)
            .or(id_token_account_id)
            .or(account_id_hint),
    );
    let organization_id = normalize_optional_value(
        extract_chatgpt_organization_id_from_access_token(&tokens.access_token)
            .or(id_token_org_id)
            .or(organization_id_hint),
    );

    let mut index = load_account_index();
    let generated_id =
        build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let has_reauth_target = normalize_optional_ref(reauth_target_account_id).is_some();

    // 明确的重新授权来自某个旧账号卡片，必须优先覆盖该旧账号。
    let existing_id = resolve_reauth_target_account_id(reauth_target_account_id, &email)?
        .or_else(|| {
            find_existing_account_id(
                &index,
                &email,
                account_id.as_deref(),
                organization_id.as_deref(),
            )
        })
        .unwrap_or_else(|| generated_id.clone());

    let mut account = if let Some(mut acc) = load_account(&existing_id) {
        // 更新现有账号
        tokens = retain_existing_refresh_token_if_missing(tokens, Some(&acc));
        acc.tokens = tokens;
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.agent_identity = None;
        acc.authorization_status = None;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.bound_oauth_account_id = None;
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.subscription_active_until = subscription_active_until.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        acc.update_last_used();
        acc
    } else {
        // 创建新账号
        tokens = retain_existing_refresh_token_if_missing(tokens, None);
        let mut acc = CodexAccount::new(existing_id.clone(), email.clone(), tokens);
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.agent_identity = None;
        acc.authorization_status = None;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.bound_oauth_account_id = None;
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.subscription_active_until = subscription_active_until.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();

        index.accounts.retain(|item| item.id != existing_id);
        index.accounts.push(CodexAccountSummary {
            id: existing_id.clone(),
            email: email.clone(),
            plan_type: plan_type.clone(),
            subscription_active_until: subscription_active_until.clone(),
            created_at: acc.created_at,
            last_used: acc.last_used,
        });
        acc
    };

    // OAuth 成功已经替换了当前账号的凭据链；此前由旧客户端页面留下的
    // login_required 只代表历史观测结果，不能继续污染这次授权后的状态。
    // 这里不能只依赖 reauth_target：用户从 OAuth 入口重新授权时可能没有
    // 携带旧卡片 ID，但仍然会复用同一个账号记录。
    account.client_auth_status = None;
    account.last_client_auth_observed_at = None;
    account.last_client_login_redirect_at = None;
    account.last_client_launch_at = None;
    account.last_client_auth_instance_id = None;

    // 远端 API 鉴权拒绝描述的是旧 access_token。OAuth 已换入新凭据后继续
    // 保留它，会让 sidecar 账号池错误地把新凭据当成仍被远端拒绝。
    // 普通额度、限流和网络错误仍由额度状态独立保留和刷新。
    if account_has_remote_api_auth_rejection(&account) {
        account.quota_error = None;
    }

    if has_reauth_target && generated_id != account.id {
        let removed_duplicate = index.accounts.iter().any(|item| item.id == generated_id);
        if removed_duplicate {
            index.accounts.retain(|item| item.id != generated_id);
            if index.current_account_id.as_deref() == Some(generated_id.as_str()) {
                index.current_account_id = Some(account.id.clone());
            }
            if let Err(err) = delete_account_file(&generated_id) {
                logger::log_warn(&format!(
                    "清理 Codex 重新授权重复账号详情失败: duplicate_id={}, target_id={}, error={}",
                    generated_id, account.id, err
                ));
            } else {
                logger::log_info(&format!(
                    "已清理 Codex 重新授权重复账号: duplicate_id={}, target_id={}",
                    generated_id, account.id
                ));
            }
        }
    }

    // 显式导入/授权可以重新创建用户刚刚删除过的同一账号。
    save_account_from_user_action(&mut account)?;

    // 更新索引中的摘要信息
    if let Some(summary) = index.accounts.iter_mut().find(|a| a.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.subscription_active_until = account.subscription_active_until.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    save_account_index(&index)?;

    logger::log_info(&format!(
        "Codex 账号已保存: email={}, account_id={:?}, organization_id={:?}",
        email, account_id, organization_id
    ));

    crate::modules::codex_auth_diagnostic::log_event(
        if has_reauth_target {
            "reauth_upsert_saved"
        } else {
            "oauth_account_upsert_saved"
        },
        serde_json::json!({
            "account_id": account.id,
            "email": account.email,
            "account_id_claim": account.account_id,
            "organization_id": account.organization_id,
            "token_generation": account.token_generation,
            "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&account.tokens),
            "requires_reauth": account.requires_reauth,
        }),
    );

    Ok(account)
}

/// 更新索引中账号的 plan_type（供配额刷新时同步订阅标识）
pub fn update_account_plan_type_in_index(
    account_id: &str,
    plan_type: &Option<String>,
    subscription_active_until: &Option<String>,
) -> Result<(), String> {
    let mut index = load_account_index();
    if let Some(summary) = index.accounts.iter_mut().find(|a| a.id == account_id) {
        summary.plan_type = plan_type.clone();
        summary.subscription_active_until = subscription_active_until.clone();
        save_account_index(&index)?;
    }
    Ok(())
}

/// 删除账号
pub fn remove_account(account_id: &str) -> Result<(), String> {
    remove_accounts(&[account_id.to_string()])
}

/// 批量删除账号
pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    let remove_ids: HashSet<String> = account_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    if remove_ids.is_empty() {
        return Ok(());
    }

    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex 账号写入锁已损坏".to_string())?;

    let mut index = load_account_index();
    let accounts_dir = get_accounts_dir();
    for account_id in &remove_ids {
        let account_generation = load_account(account_id)
            .map(|account| account.token_generation)
            .unwrap_or(0);
        let previous_generation = read_account_tombstone(account_id)
            .map(|tombstone| tombstone.generation)
            .unwrap_or(0);
        write_account_tombstone(
            account_id,
            true,
            account_generation.max(previous_generation),
            String::new(),
        )?;
    }
    let mut missing_detail_ids = HashSet::new();
    index.accounts.retain(|account| {
        if remove_ids.contains(&account.id) {
            return false;
        }
        if !accounts_dir.join(format!("{}.json", account.id)).exists() {
            missing_detail_ids.insert(account.id.clone());
            return false;
        }
        true
    });
    if !missing_detail_ids.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account] 删除账号时清理缺失详情文件的孤儿索引: {}",
            missing_detail_ids
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if index
        .current_account_id
        .as_ref()
        .map(|current_id| {
            remove_ids.contains(current_id) || missing_detail_ids.contains(current_id)
        })
        .unwrap_or(false)
    {
        index.current_account_id = None;
    }
    save_account_index(&index)?;

    for account_id in remove_ids {
        delete_account_file_unlocked(&account_id)?;
    }
    Ok(())
}
