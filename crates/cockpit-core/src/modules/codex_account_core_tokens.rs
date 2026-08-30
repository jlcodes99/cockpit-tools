// cockpit-core Codex 账号：Token identity, refresh state, account index and lifecycle。
// 通过 include! 保持原模块作用域和凭据调用路径。
/// 获取我们的多账号存储路径（统一使用 ~/.antigravity_cockpit/）
fn get_accounts_storage_path() -> PathBuf {
    let data_dir = account::get_data_dir().unwrap_or_else(|_| {
        dirs::home_dir()
            .expect("无法获取用户目录")
            .join(".antigravity_cockpit")
    });
    fs::create_dir_all(&data_dir).ok();
    migrate_codex_data_if_needed(&data_dir);
    data_dir.join("codex_accounts.json")
}

/// 获取账号详情存储目录（统一使用 ~/.antigravity_cockpit/codex_accounts/）
fn get_accounts_dir() -> PathBuf {
    let data_dir = account::get_data_dir().unwrap_or_else(|_| {
        dirs::home_dir()
            .expect("无法获取用户目录")
            .join(".antigravity_cockpit")
    });
    let accounts_dir = data_dir.join("codex_accounts");
    fs::create_dir_all(&accounts_dir).ok();
    accounts_dir
}

/// 解析 JWT Token 的 payload
pub fn decode_jwt_payload(token: &str) -> Result<CodexJwtPayload, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return Err("无效的 JWT Token 格式".to_string());
    }

    let payload_b64 = parts[1];
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|e| format!("Base64 解码失败: {}", e))?;

    let payload: CodexJwtPayload =
        serde_json::from_slice(&payload_bytes).map_err(|e| format!("JSON 解析失败: {}", e))?;

    Ok(payload)
}

fn decode_jwt_payload_value(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let payload_str = String::from_utf8(payload_bytes).ok()?;
    serde_json::from_str(&payload_str).ok()
}

fn normalize_optional_value(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_optional_ref(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn first_json_string(value: &serde_json::Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        current
            .as_str()
            .and_then(|raw| normalize_optional_ref(Some(raw)))
    })
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn codex_token_lock_for(account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = CODEX_TOKEN_REFRESH_LOCKS
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    locks
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

fn mark_token_chain_updated(account: &mut CodexAccount) {
    account.token_generation = account.token_generation.saturating_add(1);
    account.token_updated_at = Some(now_timestamp());
    account.token_source_mode = CODEX_TOKEN_SOURCE_MANAGED.to_string();
    account.requires_reauth = false;
    account.reauth_reason = None;
}

fn sync_identity_from_tokens(account: &mut CodexAccount) {
    if let Ok((email, user_id, plan_type, id_token_account_id, id_token_org_id)) =
        extract_user_info(&account.tokens.id_token)
    {
        if !email.trim().is_empty() {
            account.email = email;
        }
        account.user_id = user_id;
        account.plan_type = plan_type;
        account.account_id = normalize_optional_value(
            extract_chatgpt_account_id_from_access_token(&account.tokens.access_token)
                .or(id_token_account_id)
                .or_else(|| account.account_id.clone()),
        );
        account.organization_id = normalize_optional_value(
            extract_chatgpt_organization_id_from_access_token(&account.tokens.access_token)
                .or(id_token_org_id)
                .or_else(|| account.organization_id.clone()),
        );
    }
}

fn is_reauth_required_refresh_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("refresh_token_reused")
        || lower.contains("token_invalidated")
        || lower.contains("invalid_grant")
        || lower.contains("invalid refresh token")
        || lower.contains("authentication token has been invalidated")
}

fn mark_account_requires_reauth(account: &mut CodexAccount, reason: &str) -> Result<(), String> {
    account.requires_reauth = true;
    account.reauth_reason = Some(reason.to_string());
    save_account(account)
}

fn is_missing_refresh_token_reason(reason: &str) -> bool {
    reason.contains("缺少 refresh_token") || reason.contains("无 refresh_token")
}

fn account_has_refresh_token(account: &CodexAccount) -> bool {
    account
        .tokens
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .is_some()
}

fn managed_account_tokens_need_refresh(account: &CodexAccount) -> bool {
    // app-server 使用 access_token 作为请求登录态；id_token 仅用于账号资料元数据。
    codex_oauth::is_token_expired(&account.tokens.access_token)
}

fn clear_stale_missing_refresh_token_reauth(account: &mut CodexAccount) -> Result<(), String> {
    let is_missing_refresh_token_reauth = account
        .reauth_reason
        .as_deref()
        .map(is_missing_refresh_token_reason)
        .unwrap_or(false);

    if !account.requires_reauth || !is_missing_refresh_token_reauth {
        return Ok(());
    }

    account.requires_reauth = false;
    account.reauth_reason = None;
    save_account(account)
}

pub fn extract_chatgpt_account_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload_value(access_token)?;
    let auth_data = payload.get("https://api.openai.com/auth")?;
    first_json_string(auth_data, &[&["chatgpt_account_id"], &["account_id"]])
}

pub fn extract_chatgpt_organization_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload_value(access_token)?;
    let auth_data = payload.get("https://api.openai.com/auth")?;
    const ORG_KEYS: [&str; 6] = [
        "organization_id",
        "chatgpt_organization_id",
        "chatgpt_org_id",
        "org_id",
        "poid",
        "POID",
    ];
    for key in ORG_KEYS {
        if let Some(value) = normalize_optional_ref(auth_data.get(key).and_then(|v| v.as_str())) {
            return Some(value);
        }
    }
    if let Some(orgs) = auth_data
        .get("organizations")
        .and_then(|value| value.as_array())
    {
        if let Some(default_org) = orgs.iter().find(|org| {
            org.get("is_default")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        }) {
            if let Some(value) = first_json_string(default_org, &[&["id"]]) {
                return Some(value);
            }
        }
        if let Some(first_org) = orgs.first() {
            if let Some(value) = first_json_string(first_org, &[&["id"]]) {
                return Some(value);
            }
        }
    }
    None
}

fn extract_access_token_identity(
    access_token: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let Some(payload) = decode_jwt_payload_value(access_token) else {
        return (None, None, None, None, None);
    };

    let auth_data = payload.get("https://api.openai.com/auth");
    let email = first_json_string(&payload, &[&["email"]])
        .or_else(|| first_json_string(&payload, &[&["https://api.openai.com/profile", "email"]]));
    let user_id = auth_data
        .and_then(|value| first_json_string(value, &[&["chatgpt_user_id"], &["user_id"]]))
        .or_else(|| first_json_string(&payload, &[&["sub"]]));
    let plan_type = auth_data.and_then(|value| first_json_string(value, &[&["chatgpt_plan_type"]]));
    let account_id = extract_chatgpt_account_id_from_access_token(access_token);
    let organization_id = extract_chatgpt_organization_id_from_access_token(access_token);

    (email, user_id, plan_type, account_id, organization_id)
}

fn access_token_fingerprint(access_token: &str) -> String {
    let digest = format!("{:x}", md5::compute(access_token.as_bytes()));
    digest.chars().take(12).collect()
}

fn build_account_storage_id(
    email: &str,
    account_id: Option<&str>,
    organization_id: Option<&str>,
) -> String {
    let mut seed = email.trim().to_string();
    if let Some(id) = normalize_optional_ref(account_id) {
        seed.push('|');
        seed.push_str(&id);
    }
    if let Some(org) = normalize_optional_ref(organization_id) {
        seed.push('|');
        seed.push_str(&org);
    }
    format!("codex_{:x}", md5::compute(seed.as_bytes()))
}

fn find_existing_account_id(
    index: &CodexAccountIndex,
    email: &str,
    account_id: Option<&str>,
    organization_id: Option<&str>,
) -> Option<String> {
    let expected_account_id = normalize_optional_ref(account_id);
    let expected_org_id = normalize_optional_ref(organization_id);
    let mut first_email_match: Option<String> = None;
    let mut email_match_count = 0usize;
    let mut account_id_match_without_org: Option<String> = None;
    let mut legacy_email_only_candidate: Option<String> = None;
    let mut legacy_email_only_count = 0usize;

    for summary in &index.accounts {
        if !summary.email.eq_ignore_ascii_case(email) {
            continue;
        }
        email_match_count += 1;
        if first_email_match.is_none() {
            first_email_match = Some(summary.id.clone());
        }

        let Some(account) = load_account(&summary.id) else {
            continue;
        };

        let current_account_id = normalize_optional_ref(account.account_id.as_deref());
        let current_org_id = normalize_optional_ref(account.organization_id.as_deref());

        let is_exact_match =
            current_account_id == expected_account_id && current_org_id == expected_org_id;
        if is_exact_match {
            return Some(summary.id.clone());
        }

        if expected_account_id.is_some()
            && current_account_id == expected_account_id
            && current_org_id.is_none()
            && account_id_match_without_org.is_none()
        {
            account_id_match_without_org = Some(summary.id.clone());
        }

        if (expected_account_id.is_some() || expected_org_id.is_some())
            && current_account_id.is_none()
            && current_org_id.is_none()
        {
            legacy_email_only_count += 1;
            if legacy_email_only_candidate.is_none() {
                legacy_email_only_candidate = Some(summary.id.clone());
            }
        }
    }

    if expected_account_id.is_some() || expected_org_id.is_some() {
        return account_id_match_without_org.or_else(|| {
            if legacy_email_only_count == 1 {
                legacy_email_only_candidate
            } else {
                None
            }
        });
    }

    if email_match_count == 1 {
        return first_email_match;
    }

    None
}

/// 从 id_token 提取用户信息
pub fn extract_user_info(
    id_token: &str,
) -> Result<
    (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    String,
> {
    let payload = decode_jwt_payload(id_token)?;

    let email = payload.email.ok_or("id_token 中缺少 email")?;
    let user_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.chatgpt_user_id.clone());
    let plan_type = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.chatgpt_plan_type.clone());
    let account_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.account_id.clone());
    let organization_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.organization_id.clone());

    Ok((email, user_id, plan_type, account_id, organization_id))
}

/// 读取账号索引
pub fn load_account_index() -> CodexAccountIndex {
    let path = get_accounts_storage_path();
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(CodexAccountIndex::new);
    }

    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空").unwrap_or_else(CodexAccountIndex::new)
        }
        Ok(content) => match serde_json::from_str::<CodexAccountIndex>(&content) {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(CodexAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Codex Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(CodexAccountIndex::new)
            }
        },
        Err(_) => CodexAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<CodexAccountIndex, String> {
    let path = get_accounts_storage_path();
    if !path.exists() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 检测到账号索引文件不存在，准备尝试自动修复: path={}",
            path.display()
        ));
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            logger::log_info(&format!(
                "[Codex Account][Repair] 索引文件不存在，已自动修复完成: recovered_accounts={}",
                index.accounts.len()
            ));
            return Ok(index);
        }
        logger::log_warn(
            "[Codex Account][Repair] 索引文件不存在，但未找到可恢复详情文件，返回空索引",
        );
        return Ok(CodexAccountIndex::new());
    }

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 读取账号索引失败，准备尝试自动修复: path={}, error={}",
                path.display(),
                err
            ));
            if let Some(index) = repair_account_index_from_details("索引文件读取失败") {
                logger::log_info(&format!(
                    "[Codex Account][Repair] 索引读取失败，已自动修复完成: recovered_accounts={}",
                    index.accounts.len()
                ));
                return Ok(index);
            }
            return Err(format!("读取账号索引失败: {}", err));
        }
    };

    if content.trim().is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 检测到账号索引文件为空，准备尝试自动修复: path={}",
            path.display()
        ));
        if let Some(index) = repair_account_index_from_details("索引文件为空") {
            logger::log_info(&format!(
                "[Codex Account][Repair] 空索引文件已自动修复完成: recovered_accounts={}",
                index.accounts.len()
            ));
            return Ok(index);
        }
        logger::log_warn(
            "[Codex Account][Repair] 索引文件为空，但未找到可恢复详情文件，返回空索引",
        );
        return Ok(CodexAccountIndex::new());
    }

    match serde_json::from_str::<CodexAccountIndex>(&content) {
        Ok(index) if !index.accounts.is_empty() => Ok(index),
        Ok(index) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 账号索引可解析但列表为空，准备尝试自动修复: path={}",
                path.display()
            ));
            if let Some(repaired) = repair_account_index_from_details("索引账号列表为空") {
                logger::log_info(&format!(
                    "[Codex Account][Repair] 空账号列表已自动修复完成: recovered_accounts={}",
                    repaired.accounts.len()
                ));
                return Ok(repaired);
            }
            Ok(index)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 账号索引解析失败，准备尝试自动修复: path={}, error={}",
                path.display(),
                err
            ));
            if let Some(index) = repair_account_index_from_details("索引文件损坏") {
                logger::log_info(&format!(
                    "[Codex Account][Repair] 损坏索引文件已自动修复完成: recovered_accounts={}",
                    index.accounts.len()
                ));
                return Ok(index);
            }
            Err(crate::error::file_corrupted_error(
                "codex_accounts.json",
                &path.to_string_lossy(),
                &err.to_string(),
            ))
        }
    }
}

/// 保存账号索引
pub fn save_account_index(index: &CodexAccountIndex) -> Result<(), String> {
    let path = get_accounts_storage_path();
    let content = serde_json::to_string_pretty(index).map_err(|e| format!("序列化失败: {}", e))?;
    write_string_atomic(&path, &content).map_err(|e| format!("写入账号索引失败: {}", e))?;
    Ok(())
}

fn repair_account_index_from_details(reason: &str) -> Option<CodexAccountIndex> {
    let index_path = get_accounts_storage_path();
    let accounts_dir = get_accounts_dir();
    logger::log_warn(&format!(
        "[Codex Account][Repair] 检测到索引异常，开始按详情文件重建: reason={}, index_path={}, accounts_dir={}",
        reason,
        index_path.display(),
        accounts_dir.display()
    ));

    let mut accounts = match crate::modules::account_index_repair::load_accounts_from_details(
        &accounts_dir,
        |account_id| load_account(account_id),
    ) {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 扫描账号详情文件失败，无法自动修复: reason={}, accounts_dir={}, error={}",
                reason,
                accounts_dir.display(),
                err
            ));
            return None;
        }
    };

    if accounts.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 账号详情目录中未发现可恢复账号，放弃自动修复: reason={}, accounts_dir={}",
            reason,
            accounts_dir.display()
        ));
        return None;
    }

    logger::log_info(&format!(
        "[Codex Account][Repair] 已扫描到 {} 个账号详情，准备重建索引",
        accounts.len()
    ));

    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut accounts,
        |account| account.last_used,
        |account| account.created_at,
        |account| account.id.as_str(),
    );

    let mut index = CodexAccountIndex::new();
    index.accounts = accounts
        .iter()
        .map(|account| CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        })
        .collect();
    index.current_account_id = accounts.first().map(|account| account.id.clone());

    logger::log_info(&format!(
        "[Codex Account][Repair] 索引重建完成，准备写回本地文件: recovered_accounts={}, current_account_id={}",
        index.accounts.len(),
        index.current_account_id.as_deref().unwrap_or("-")
    ));

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Codex Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Codex Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_info(&format!(
        "[Codex Account][Repair] 已根据详情文件自动重建账号索引: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

/// 读取单个账号详情
pub fn load_account(account_id: &str) -> Option<CodexAccount> {
    let path = get_accounts_dir().join(format!("{}.json", account_id));
    if !path.exists() {
        return None;
    }

    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).ok(),
        Err(_) => None,
    }
}

/// 保存单个账号详情
pub fn save_account(account: &CodexAccount) -> Result<(), String> {
    let path = get_accounts_dir().join(format!("{}.json", &account.id));
    let content =
        serde_json::to_string_pretty(account).map_err(|e| format!("序列化失败: {}", e))?;
    write_string_atomic(&path, &content).map_err(|e| format!("写入账号详情失败: {}", e))?;
    Ok(())
}

/// 删除单个账号
pub fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = get_accounts_dir().join(format!("{}.json", account_id));
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("删除文件失败: {}", e))?;
    }
    Ok(())
}

/// 列出所有账号
pub fn list_accounts() -> Vec<CodexAccount> {
    let index = load_account_index();
    index
        .accounts
        .iter()
        .filter_map(|summary| load_account(&summary.id))
        .collect()
}

pub fn list_accounts_checked() -> Result<Vec<CodexAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(index
        .accounts
        .iter()
        .filter_map(|summary| load_account(&summary.id))
        .collect())
}

/// 刷新账号资料（团队名/结构）
async fn refresh_account_profile_once(account_id: &str) -> Result<CodexAccount, String> {
    let mut account = prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() {
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

pub fn upsert_account_for_reauth(
    tokens: CodexTokens,
    target_account_id: &str,
) -> Result<CodexAccount, String> {
    upsert_account_with_hints_and_reauth_target(tokens, None, None, Some(target_account_id))
}

pub fn upsert_api_key_account(
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
) -> Result<CodexAccount, String> {
    let (api_key, api_base_url) = validate_api_key_credentials(&api_key, api_base_url.as_deref())?;
    let provider_config = resolve_api_provider_config(
        api_base_url.as_deref(),
        api_provider_mode,
        api_provider_id.as_deref(),
        api_provider_name.as_deref(),
    )?;
    let account_id = build_api_key_account_id(&api_key);
    let mut index = load_account_index();
    let existing = index.accounts.iter().position(|item| item.id == account_id);

    let mut account = if let Some(pos) = existing {
        let existing_id = index.accounts[pos].id.clone();
        let mut acc = load_account(&existing_id).unwrap_or_else(|| {
            CodexAccount::new_api_key(
                existing_id,
                build_api_key_email(&api_key),
                api_key.clone(),
                provider_config.mode.clone(),
                provider_config.base_url.clone(),
                provider_config.provider_id.clone(),
                provider_config.provider_name.clone(),
            )
        });
        apply_api_key_fields(&mut acc, &api_key, provider_config.clone());
        if acc.email.trim().is_empty() {
            acc.email = build_api_key_email(&api_key);
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
        );
        acc.plan_type = Some(API_KEY_LOGIN_PLAN_TYPE.to_string());
        index.accounts.push(CodexAccountSummary {
            id: account_id.clone(),
            email: acc.email.clone(),
            plan_type: acc.plan_type.clone(),
            created_at: acc.created_at,
            last_used: acc.last_used,
        });
        acc
    };

    account.auth_mode = CodexAuthMode::Apikey;
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
    upsert_account_with_hints_and_reauth_target(tokens, account_id_hint, organization_id_hint, None)
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
    tokens: CodexTokens,
    account_id_hint: Option<String>,
    organization_id_hint: Option<String>,
    reauth_target_account_id: Option<&str>,
) -> Result<CodexAccount, String> {
    let (email, user_id, plan_type, id_token_account_id, id_token_org_id) =
        extract_user_info(&tokens.id_token)?;
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
    let existing = index.accounts.iter().position(|a| a.id == existing_id);

    let account = if let Some(pos) = existing {
        // 更新现有账号
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
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        acc.update_last_used();
        acc
    } else {
        // 创建新账号
        let mut acc = CodexAccount::new(existing_id.clone(), email.clone(), tokens);
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();

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

    // 保存账号详情
    save_account(&account)?;

    // 更新索引中的摘要信息
    if let Some(summary) = index.accounts.iter_mut().find(|a| a.id == account.id) {
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
        "Codex 账号已保存: email={}, account_id={:?}, organization_id={:?}",
        email, account_id, organization_id
    ));

    Ok(account)
}

/// 更新索引中账号的 plan_type（供配额刷新时同步订阅标识）
pub fn update_account_plan_type_in_index(
    account_id: &str,
    plan_type: &Option<String>,
) -> Result<(), String> {
    let mut index = load_account_index();
    if let Some(summary) = index.accounts.iter_mut().find(|a| a.id == account_id) {
        summary.plan_type = plan_type.clone();
        save_account_index(&index)?;
    }
    Ok(())
}

/// 删除账号
pub fn remove_account(account_id: &str) -> Result<(), String> {
    let mut index = load_account_index();

    // 从索引中移除
    index.accounts.retain(|a| a.id != account_id);

    // 如果删除的是当前账号，清除 current_account_id
    if index.current_account_id.as_deref() == Some(account_id) {
        index.current_account_id = None;
    }

    save_account_index(&index)?;
    delete_account_file(account_id)?;

    Ok(())
}

/// 批量删除账号
pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        remove_account(id)?;
    }
    Ok(())
}

