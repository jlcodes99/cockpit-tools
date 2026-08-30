// Codex 账号模块：Account index, summaries, persistence and quota policy listing。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
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
        Option<String>,
    ),
    String,
> {
    let payload = decode_jwt_payload(id_token)?;

    let email = payload
        .email
        .or_else(|| {
            payload
                .profile_data
                .as_ref()
                .and_then(|data| data.email.clone())
        })
        .ok_or("id_token 中缺少 email")?;
    let user_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.chatgpt_user_id.clone());
    let plan_type = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.chatgpt_plan_type.clone());
    let subscription_active_until = payload
        .auth_data
        .as_ref()
        .and_then(|d| normalize_optional_json_scalar(d.chatgpt_subscription_active_until.as_ref()));
    let account_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.account_id.clone());
    let organization_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.organization_id.clone());

    Ok((
        email,
        user_id,
        plan_type,
        subscription_active_until,
        account_id,
        organization_id,
    ))
}

fn account_summary_from_account(account: &CodexAccount) -> CodexAccountSummary {
    CodexAccountSummary {
        id: account.id.clone(),
        email: account.email.clone(),
        plan_type: account.plan_type.clone(),
        subscription_active_until: account.subscription_active_until.clone(),
        created_at: account.created_at,
        last_used: account.last_used,
    }
}

fn account_summary_matches_account(summary: &CodexAccountSummary, account: &CodexAccount) -> bool {
    summary.email == account.email
        && summary.plan_type == account.plan_type
        && summary.subscription_active_until == account.subscription_active_until
        && summary.created_at == account.created_at
        && summary.last_used == account.last_used
}

fn sync_loaded_accounts_to_index_cache(
    index: &mut CodexAccountIndex,
    accounts: &[CodexAccount],
) -> bool {
    let mut changed = false;
    if index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION {
        index.detail_schema_version = CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION;
        changed = true;
    }

    for account in accounts {
        let next_summary = account_summary_from_account(account);
        if let Some(summary) = index
            .accounts
            .iter_mut()
            .find(|summary| summary.id == account.id)
        {
            if !account_summary_matches_account(summary, account) {
                *summary = next_summary;
                changed = true;
            }
        } else {
            index.accounts.push(next_summary);
            changed = true;
        }
    }

    changed
}

fn apply_index_summary_to_account_detail(
    account: &mut CodexAccount,
    summary: &CodexAccountSummary,
) -> bool {
    let mut changed = false;

    if account.email.trim().is_empty() && !summary.email.trim().is_empty() {
        account.email = summary.email.clone();
        changed = true;
    }

    if account.plan_type.is_none() && summary.plan_type.is_some() {
        account.plan_type = summary.plan_type.clone();
        changed = true;
    }

    if account.subscription_active_until.is_none() && summary.subscription_active_until.is_some() {
        account.subscription_active_until = summary.subscription_active_until.clone();
        changed = true;
    }

    if account.created_at <= 0 && summary.created_at > 0 {
        account.created_at = summary.created_at;
        changed = true;
    }

    if summary.last_used > account.last_used {
        account.last_used = summary.last_used;
        changed = true;
    } else if account.last_used <= 0 {
        account.last_used = account.created_at.max(summary.last_used);
        changed = true;
    }

    changed
}

fn collect_account_detail_file_ids() -> Result<HashSet<String>, String> {
    let accounts_dir = get_accounts_dir();
    if !accounts_dir.exists() {
        return Ok(HashSet::new());
    }

    let entries = fs::read_dir(&accounts_dir).map_err(|error| {
        format!(
            "读取 Codex 账号详情目录失败: path={}, error={}",
            accounts_dir.display(),
            error
        )
    })?;

    let mut ids = HashSet::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("遍历 Codex 账号详情目录失败: {}", error))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_json = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("json"))
            .unwrap_or(false);
        if !is_json {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|name| name.to_str()) {
            if !account_is_tombstoned(stem) {
                ids.insert(stem.to_string());
            }
        }
    }

    Ok(ids)
}

fn build_account_index_from_summaries(
    mut summaries: Vec<CodexAccountSummary>,
    previous_current_account_id: Option<String>,
) -> CodexAccountIndex {
    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut summaries,
        |summary| summary.last_used,
        |summary| summary.created_at,
        |summary| summary.id.as_str(),
    );

    let mut index = CodexAccountIndex::new();
    index.detail_schema_version = CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION;
    index.accounts = summaries;
    index.current_account_id = previous_current_account_id.filter(|current_id| {
        index
            .accounts
            .iter()
            .any(|summary| summary.id.as_str() == current_id.as_str())
    });
    index
}

fn empty_reconciled_account_index() -> CodexAccountIndex {
    let mut index = CodexAccountIndex::new();
    index.detail_schema_version = CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION;
    index
}

fn should_reconcile_account_index_with_details(
    index: &CodexAccountIndex,
    detail_ids: &HashSet<String>,
) -> bool {
    if index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION {
        return true;
    }

    if index.accounts.len() != detail_ids.len() {
        return true;
    }

    let index_ids: HashSet<String> = index
        .accounts
        .iter()
        .map(|account| account.id.clone())
        .collect();
    if &index_ids != detail_ids {
        return true;
    }

    if let Some(current_id) = index.current_account_id.as_deref() {
        return !detail_ids.contains(current_id);
    }

    false
}

fn reconcile_account_index_with_details_if_needed(
    index: CodexAccountIndex,
    reason: &str,
) -> CodexAccountIndex {
    let detail_ids = match collect_account_detail_file_ids() {
        Ok(ids) => ids,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 检查账号详情目录失败，保留当前索引: reason={}, error={}",
                reason, error
            ));
            return index;
        }
    };

    if detail_ids.is_empty() {
        if !index.accounts.is_empty()
            || index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION
            || index.current_account_id.is_some()
        {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 账号详情目录为空，已清空索引缓存: reason={}, indexed_accounts={}",
                reason,
                index.accounts.len()
            ));
            let empty = empty_reconciled_account_index();
            if let Err(error) = save_account_index(&empty) {
                logger::log_warn(&format!(
                    "[Codex Account][Repair] 清空 Codex 索引缓存失败: reason={}, error={}",
                    reason, error
                ));
            }
            return empty;
        }
        return index;
    }

    if !should_reconcile_account_index_with_details(&index, &detail_ids) {
        return index;
    }

    logger::log_warn(&format!(
        "[Codex Account][Repair] 检测到索引缓存与详情文件不一致，准备按详情重建: reason={}, indexed_accounts={}, detail_files={}, detail_schema_version={}",
        reason,
        index.accounts.len(),
        detail_ids.len(),
        index.detail_schema_version
    ));

    repair_account_index_from_details_with_previous(reason, Some(&index)).unwrap_or(index)
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
            Ok(index) if index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION => {
                reconcile_account_index_with_details_if_needed(index, "初始化账号详情数据")
            }
            Ok(index) => index,
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
        Ok(index) => Ok(reconcile_account_index_with_details_if_needed(
            index,
            "读取账号索引",
        )),
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
    let mut index = index.clone();
    if index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION {
        index.detail_schema_version = CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION;
    }
    let content = serde_json::to_string_pretty(&index).map_err(|e| format!("序列化失败: {}", e))?;
    write_string_atomic(&path, &content).map_err(|e| format!("写入账号索引失败: {}", e))?;
    Ok(())
}

fn repair_account_index_from_details(reason: &str) -> Option<CodexAccountIndex> {
    let index_path = get_accounts_storage_path();
    let previous_index = fs::read_to_string(&index_path)
        .ok()
        .and_then(|content| serde_json::from_str::<CodexAccountIndex>(&content).ok());
    repair_account_index_from_details_with_previous(reason, previous_index.as_ref())
}

fn repair_account_index_from_details_with_previous(
    reason: &str,
    previous_index: Option<&CodexAccountIndex>,
) -> Option<CodexAccountIndex> {
    let index_path = get_accounts_storage_path();
    let accounts_dir = get_accounts_dir();
    let previous_current_account_id =
        previous_index.and_then(|index| index.current_account_id.clone());
    let summary_by_id: HashMap<String, CodexAccountSummary> = previous_index
        .map(|index| {
            index
                .accounts
                .iter()
                .map(|summary| (summary.id.clone(), summary.clone()))
                .collect()
        })
        .unwrap_or_default();
    logger::log_warn(&format!(
        "[Codex Account][Repair] 检测到索引异常，开始按详情文件重建: reason={}, index_path={}, accounts_dir={}",
        reason,
        index_path.display(),
        accounts_dir.display()
    ));

    let detail_ids = match collect_account_detail_file_ids() {
        Ok(ids) => ids,
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

    if detail_ids.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 账号详情目录中未发现可恢复账号，放弃自动修复: reason={}, accounts_dir={}",
            reason,
            accounts_dir.display()
        ));
        return None;
    }

    let mut account_ids: Vec<String> = detail_ids.into_iter().collect();
    account_ids.sort();
    let mut summaries = Vec::with_capacity(account_ids.len());
    let mut failed = Vec::new();
    for account_id in account_ids {
        match load_account_with_summary(&account_id, summary_by_id.get(&account_id)) {
            Ok(Some(account)) => summaries.push(account_summary_from_account(&account)),
            Ok(None) => failed.push(format!("{}: 详情文件不存在", account_id)),
            Err(error) => failed.push(format!("{}: {}", account_id, error)),
        }
    }

    if !failed.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 部分详情文件无法恢复，已跳过: reason={}, failed={}",
            reason,
            failed.join("; ")
        ));
    }

    if summaries.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 账号详情目录中未发现可恢复账号，放弃自动修复: reason={}, accounts_dir={}",
            reason,
            accounts_dir.display()
        ));
        return None;
    }

    logger::log_info(&format!(
        "[Codex Account][Repair] 已扫描到 {} 个账号详情，准备重建索引",
        summaries.len()
    ));

    let index = build_account_index_from_summaries(summaries, previous_current_account_id);

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

fn read_json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let raw = keys
        .iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_str()))?;
    normalize_optional_ref(Some(raw))
}

fn read_codex_fingerprint_mode(value: &serde_json::Value) -> Option<String> {
    read_json_string(value, &["codex_fingerprint_mode", "codexFingerprintMode"])
        .or_else(|| {
            value.get("extra").and_then(|extra| {
                read_json_string(extra, &["codex_fingerprint_mode", "codexFingerprintMode"])
            })
        })
        .map(|mode| mode.trim().to_ascii_lowercase())
        .filter(|mode| matches!(mode.as_str(), "off" | "device" | "session" | "full"))
}

fn read_codex_client_policy_bool(value: &serde_json::Value, key: &str) -> Option<bool> {
    read_json_bool(value, &[key]).or_else(|| {
        value
            .get("extra")
            .and_then(|extra| read_json_bool(extra, &[key]))
    })
}

pub(crate) fn resolved_codex_fingerprint_mode(account: &CodexAccount) -> &'static str {
    resolved_codex_fingerprint_mode_value(account.codex_fingerprint_mode.as_deref())
}

fn resolved_codex_fingerprint_mode_value(raw: Option<&str>) -> &'static str {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("device") => "device",
        Some("off") => "off",
        Some("full") => "full",
        _ => "session",
    }
}

fn read_json_i64(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let item = value.get(*key)?;
        if item.is_string() {
            return parse_auth_file_last_refresh(Some(item));
        }
        item.as_i64()
            .or_else(|| item.as_u64().and_then(|raw| i64::try_from(raw).ok()))
    })
}

fn read_json_bool(value: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_bool()))
}

fn read_json_string_array(value: &serde_json::Value, keys: &[&str]) -> Option<Vec<String>> {
    let items = keys
        .iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_array()))?;
    let normalized = items
        .iter()
        .filter_map(|item| item.as_str())
        .filter_map(|item| normalize_optional_ref(Some(item)))
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn read_account_two_factor_secret(value: &serde_json::Value) -> Option<String> {
    read_json_string(
        value,
        &[
            "two_factor_secret",
            "twoFactorSecret",
            "account_two_factor_secret",
            "accountTwoFactorSecret",
        ],
    )
}

fn read_account_password(value: &serde_json::Value) -> Option<String> {
    read_json_string(value, &["account_password", "accountPassword", "password"])
}

fn read_account_phone_number(value: &serde_json::Value) -> Option<String> {
    read_json_string(
        value,
        &[
            "phone_number",
            "phoneNumber",
            "account_phone_number",
            "accountPhoneNumber",
        ],
    )
}

fn read_account_mail_url(value: &serde_json::Value) -> Option<String> {
    read_json_string(
        value,
        &[
            "mail_url",
            "mailUrl",
            "mail_address",
            "mailAddress",
            "mail_query_url",
            "mailQueryUrl",
        ],
    )
}

fn apply_account_sensitive_note_metadata(account: &mut CodexAccount, value: &serde_json::Value) {
    if let Some(secret) = read_account_two_factor_secret(value) {
        account.two_factor_secret = Some(secret);
    }
    if let Some(password) = read_account_password(value) {
        account.account_password = Some(password);
    }
    if let Some(phone_number) = read_account_phone_number(value) {
        account.phone_number = Some(phone_number);
    }
    if let Some(mail_url) = read_account_mail_url(value) {
        account.mail_url = Some(mail_url);
    }
}

fn read_codex_api_provider_mode(value: &serde_json::Value) -> Option<CodexApiProviderMode> {
    value
        .get("api_provider_mode")
        .or_else(|| value.get("apiProviderMode"))
        .and_then(|item| serde_json::from_value::<CodexApiProviderMode>(item.clone()).ok())
}

fn apply_compat_account_metadata(
    account: &mut CodexAccount,
    value: &serde_json::Value,
    summary: Option<&CodexAccountSummary>,
) {
    let now = now_timestamp();
    if account.email.trim().is_empty() {
        account.email = read_json_string(value, &["email", "account_email"])
            .or_else(|| summary.map(|item| item.email.clone()))
            .unwrap_or_else(|| account.id.clone());
    }
    account.account_name = read_json_string(value, &["account_name", "accountName"])
        .or_else(|| account.account_name.clone());
    account.account_structure = read_json_string(value, &["account_structure", "accountStructure"])
        .or_else(|| account.account_structure.clone());
    account.account_note = read_json_string(value, &["account_note", "accountNote"])
        .or_else(|| account.account_note.clone());
    account.codex_fingerprint_mode =
        read_codex_fingerprint_mode(value).or_else(|| account.codex_fingerprint_mode.clone());
    if let Some(enabled) = read_codex_client_policy_bool(value, "codex_cli_only") {
        account.codex_cli_only = enabled;
    }
    if let Some(enabled) = read_codex_client_policy_bool(value, "codex_cli_only_allow_app_server") {
        account.codex_cli_only_allow_app_server = enabled;
    }
    apply_account_sensitive_note_metadata(account, value);
    account.auth_file_plan_type =
        read_json_string(value, &["auth_file_plan_type", "authFilePlanType"])
            .or_else(|| account.auth_file_plan_type.clone());
    account.plan_type = read_json_string(value, &["plan_type", "planType"])
        .or_else(|| account.plan_type.clone())
        .or_else(|| summary.and_then(|item| item.plan_type.clone()));
    account.subscription_active_until = read_json_string(
        value,
        &["subscription_active_until", "subscriptionActiveUntil"],
    )
    .or_else(|| account.subscription_active_until.clone())
    .or_else(|| summary.and_then(|item| item.subscription_active_until.clone()));
    account.created_at = read_json_i64(value, &["created_at", "createdAt"])
        .or_else(|| summary.map(|item| item.created_at))
        .unwrap_or(now);
    account.last_used = read_json_i64(value, &["last_used", "lastUsed"])
        .or_else(|| summary.map(|item| item.last_used))
        .unwrap_or(account.created_at);
    account.token_updated_at = read_json_i64(value, &["token_updated_at", "tokenUpdatedAt"])
        .or_else(|| parse_auth_file_last_refresh(value.get("last_refresh")))
        .or(account.token_updated_at);
    account.authorization_status =
        read_json_string(value, &["authorization_status", "authorizationStatus"])
            .or_else(|| account.authorization_status.clone());
    account.tags = read_json_string_array(value, &["tags"]).or_else(|| account.tags.clone());
}

fn apply_api_key_import_metadata(account: &mut CodexAccount, value: &serde_json::Value) {
    if let Some(account_name) = read_json_string(value, &["account_name", "accountName"]) {
        account.account_name = Some(account_name);
    }
    if let Some(account_note) = read_json_string(value, &["account_note", "accountNote"]) {
        account.account_note = Some(account_note);
    }
    apply_account_sensitive_note_metadata(account, value);
    if let Some(plan_type) = read_json_string(value, &["plan_type", "planType"]) {
        account.plan_type = Some(plan_type);
    }
    if let Some(subscription_active_until) = read_json_string(
        value,
        &["subscription_active_until", "subscriptionActiveUntil"],
    ) {
        account.subscription_active_until = Some(subscription_active_until);
    }
    if let Some(auth_file_plan_type) =
        read_json_string(value, &["auth_file_plan_type", "authFilePlanType"])
    {
        account.auth_file_plan_type = Some(auth_file_plan_type);
    }
    if let Some(tags) = read_json_string_array(value, &["tags"]) {
        account.tags = Some(tags);
    }
    if let Some(api_wire_api) = read_json_string(value, &["api_wire_api", "apiWireApi"]) {
        account.api_wire_api = normalize_api_wire_api(Some(api_wire_api));
    }
    if let Some(sync_model_catalog) = read_json_bool(
        value,
        &[
            "api_sync_model_catalog_to_codex",
            "apiSyncModelCatalogToCodex",
        ],
    ) {
        account.api_sync_model_catalog_to_codex = sync_model_catalog;
    }
    if let Some(supports_websockets) =
        read_json_bool(value, &["api_supports_websockets", "apiSupportsWebsockets"])
    {
        account.api_supports_websockets = supports_websockets;
        let _ = normalize_api_key_websocket_capability(account);
    }
    if let Some(windows_value) = value
        .get("api_model_context_windows")
        .or_else(|| value.get("apiModelContextWindows"))
    {
        if let Ok(parsed) = serde_json::from_value::<HashMap<String, i64>>(windows_value.clone()) {
            account.api_model_context_windows = normalize_api_model_context_windows(
                parsed,
                &account.api_model_catalog,
                &account.api_model_mappings,
            );
        }
    }
}

fn parse_codex_account_compat(
    value: serde_json::Value,
    fallback_id: &str,
    summary: Option<&CodexAccountSummary>,
) -> Result<Option<CodexAccount>, String> {
    if let Ok(mut account) = serde_json::from_value::<CodexAccount>(value.clone()) {
        if account.id.trim().is_empty() {
            account.id = fallback_id.to_string();
        }
        apply_compat_account_metadata(&mut account, &value, summary);
        normalize_api_key_websocket_capability(&mut account);
        return Ok(Some(account));
    }

    if is_auth_mode_apikey(
        value
            .get("auth_mode")
            .and_then(|item| item.as_str())
            .or_else(|| value.get("authMode").and_then(|item| item.as_str())),
    ) {
        let Some(api_key) = value
            .get("OPENAI_API_KEY")
            .and_then(|item| item.as_str())
            .and_then(normalize_api_key)
        else {
            return Ok(None);
        };
        let api_base_url_hint = extract_api_base_url_from_json_value(&value);
        let (api_key, api_base_url) =
            validate_api_key_credentials(&api_key, api_base_url_hint.as_deref())?;
        let provider_config = resolve_api_provider_config(
            api_base_url.as_deref(),
            read_codex_api_provider_mode(&value),
            value.get("api_provider_id").and_then(|item| item.as_str()),
            value
                .get("api_provider_name")
                .and_then(|item| item.as_str()),
        )?;
        let mut account = CodexAccount::new_api_key(
            fallback_id.to_string(),
            read_json_string(&value, &["email", "account_email"])
                .or_else(|| summary.map(|item| item.email.clone()))
                .unwrap_or_else(|| build_api_key_email(&api_key)),
            api_key,
            provider_config.mode,
            provider_config.base_url,
            provider_config.provider_id,
            provider_config.provider_name,
            Vec::new(),
        );
        apply_compat_account_metadata(&mut account, &value, summary);
        apply_api_key_import_metadata(&mut account, &value);
        account.plan_type = Some(API_KEY_LOGIN_PLAN_TYPE.to_string());
        return Ok(Some(account));
    }

    let Some((tokens, account_id_hint)) = extract_codex_tokens_from_value(&value) else {
        return Ok(None);
    };
    let mut account = CodexAccount::new(
        fallback_id.to_string(),
        read_json_string(&value, &["email", "account_email"])
            .or_else(|| summary.map(|item| item.email.clone()))
            .unwrap_or_else(|| fallback_id.to_string()),
        tokens,
    );
    account.account_id = normalize_optional_value(
        extract_chatgpt_account_id_from_access_token(&account.tokens.access_token)
            .or(account_id_hint)
            .or_else(|| read_json_string(&value, &["account_id", "accountId"])),
    );
    account.organization_id = normalize_optional_value(read_json_string(
        &value,
        &["organization_id", "organizationId"],
    ));
    sync_identity_from_tokens(&mut account);
    apply_compat_account_metadata(&mut account, &value, summary);
    Ok(Some(account))
}

/// 读取单个账号详情
pub fn load_account(account_id: &str) -> Option<CodexAccount> {
    load_account_with_summary(account_id, None).ok().flatten()
}

/// 绑定 OAuth 的 API Key：不走本地网关生图兼容（保持绑定显示/客户端能力）。
/// 纯 API Key 生图走 provider 的 gpt-image-2 + actor header，与本开关无关。
fn clear_bound_oauth_local_gateway_flag(account: &mut CodexAccount) -> bool {
    if !account.bound_oauth_use_local_gateway {
        return false;
    }
    account.bound_oauth_use_local_gateway = false;
    true
}

fn load_account_after_index_repair(account_id: &str) -> Option<CodexAccount> {
    if let Some(account) = load_account(account_id) {
        return Some(account);
    }

    logger::log_warn(&format!(
        "[Codex Account][Repair] 切号目标账号详情缺失，尝试按详情文件重建索引后重试: account_id={}",
        account_id
    ));
    let repaired = repair_account_index_from_details("切号目标账号不存在")?;
    if !repaired
        .accounts
        .iter()
        .any(|summary| summary.id == account_id)
    {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 重建索引后仍未找到切号目标账号: account_id={}",
            account_id
        ));
        return None;
    }

    load_account(account_id)
}

fn load_account_with_summary(
    account_id: &str,
    summary: Option<&CodexAccountSummary>,
) -> Result<Option<CodexAccount>, String> {
    if account_is_tombstoned(account_id) {
        return Ok(None);
    }
    let path = get_accounts_dir().join(format!("{}.json", account_id));
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取账号详情失败 ({}): {}", path.display(), error))?;

    // AES-GCM envelope first (#1104), then plaintext + compat paths.
    if let Ok((mut account, needs_rotation)) =
        crate::modules::secure_account_storage::deserialize_account_file::<CodexAccount>(
            &path, &content,
        )
    {
        let migrated_index_summary = summary
            .map(|summary| apply_index_summary_to_account_detail(&mut account, summary))
            .unwrap_or(false);
        // 绑定 OAuth 时强制关闭本地网关标志，避免误走旧「禁生图 + 本地网关」路径。
        let cleared_bound_oauth_gateway = clear_bound_oauth_local_gateway_flag(&mut account);
        let migrated_wire_api = migrate_apikey_fun_wire_api(&mut account);
        let migrated_deepseek = enforce_deepseek_responses_account(&mut account);
        let migrated_websocket = normalize_api_key_websocket_capability(&mut account);
        let cleared_retired_app_server_preflight =
            clear_retired_app_server_preflight_reauth(&mut account);
        if !validate_loaded_account_tombstone(&account)? {
            return Ok(None);
        }
        if needs_rotation
            || migrated_wire_api
            || migrated_deepseek
            || migrated_websocket
            || cleared_retired_app_server_preflight
            || cleared_bound_oauth_gateway
            || migrated_index_summary
        {
            let account_for_rewrite = account.clone();
            crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
                "codex",
                account_for_rewrite.id.clone(),
                path.clone(),
                content.as_bytes(),
                move || {
                    crate::modules::secure_account_storage::serialize_account_file(
                        "codex",
                        &account_for_rewrite,
                    )
                },
            );
        }
        return Ok(Some(account));
    }

    let value = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|error| format!("账号详情不是有效 JSON ({}): {}", path.display(), error))?;
    let mut account = parse_codex_account_compat(value.clone(), account_id, summary)?
        .ok_or_else(|| format!("账号详情缺少可识别凭据 ({})", path.display()))?;
    let _ = migrate_apikey_fun_wire_api(&mut account);
    let _ = enforce_deepseek_responses_account(&mut account);
    let _ = clear_bound_oauth_local_gateway_flag(&mut account);
    let _ = clear_retired_app_server_preflight_reauth(&mut account);
    if !validate_loaded_account_tombstone(&account)? {
        return Ok(None);
    }

    let account_for_rewrite = account.clone();
    crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
        "codex",
        account_for_rewrite.id.clone(),
        path.clone(),
        content.as_bytes(),
        move || {
            crate::modules::secure_account_storage::serialize_account_file(
                "codex",
                &account_for_rewrite,
            )
        },
    );

    Ok(Some(account))
}

/// 保存单个账号详情
pub fn save_account(account: &CodexAccount) -> Result<(), String> {
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex 账号写入锁已损坏".to_string())?;
    save_account_with_tombstone_guard(account)
}

fn save_account_with_tombstone_guard(account: &CodexAccount) -> Result<(), String> {
    let mut next_tombstone = None;
    if let Some(tombstone) = read_account_tombstone(&account.id) {
        let credential_hash = account_credential_hash(account);
        if tombstone.deleted
            || account.token_generation < tombstone.generation
            || (account.token_generation == tombstone.generation
                && credential_hash != tombstone.credential_hash)
        {
            return Err(format!(
                "账号已删除或凭据快照已过期，拒绝后台写回: account_id={}",
                account.id
            ));
        }
        if account.token_generation > tombstone.generation {
            next_tombstone = Some(credential_hash);
        }
    }
    save_account_unchecked(account)?;
    if let Some(credential_hash) = next_tombstone {
        write_account_tombstone(
            &account.id,
            false,
            account.token_generation,
            credential_hash,
        )?;
    }
    Ok(())
}

fn save_account_unchecked(account: &CodexAccount) -> Result<(), String> {
    let path = get_accounts_dir().join(format!("{}.json", &account.id));
    let content = crate::modules::secure_account_storage::serialize_account_file("codex", account)?;
    write_string_atomic(&path, &content).map_err(|e| format!("写入账号详情失败: {}", e))?;
    Ok(())
}

fn save_account_from_user_action(account: &mut CodexAccount) -> Result<(), String> {
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex 账号写入锁已损坏".to_string())?;
    let tombstone = read_account_tombstone(&account.id);
    if let Some(tombstone) = tombstone.as_ref() {
        account.token_generation = account
            .token_generation
            .max(tombstone.generation.saturating_add(1));
    }
    save_account_unchecked(account)?;
    if tombstone.is_some() {
        write_account_tombstone(
            &account.id,
            false,
            account.token_generation,
            account_credential_hash(account),
        )?;
    }
    Ok(())
}

/// 删除单个账号
pub fn delete_account_file(account_id: &str) -> Result<(), String> {
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex 账号写入锁已损坏".to_string())?;
    delete_account_file_unlocked(account_id)
}

fn delete_account_file_unlocked(account_id: &str) -> Result<(), String> {
    let path = get_accounts_dir().join(format!("{}.json", account_id));
    if path.exists() {
        crate::modules::atomic_write::remove_file_locked(&path)
            .map_err(|e| format!("删除文件失败: {}", e))?;
    }
    Ok(())
}

// ─── Codex 分组额度刷新策略（最高优先级）────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexAccountGroupRecord {
    #[serde(default)]
    account_ids: Vec<String>,
    /// null/缺省 = 继承平台；-1 = 不刷新；>0 = 自定义分钟
    #[serde(default)]
    quota_auto_refresh_minutes: Option<i32>,
    /// 旧字段兼容：false → 不刷新
    #[serde(default)]
    quota_refresh_enabled: Option<bool>,
}

/// 分组额度策略：继承 / 关闭 / 自定义分钟
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexGroupQuotaRefreshPolicy {
    Inherit,
    Disabled,
    Minutes(u32),
}

impl CodexAccountGroupRecord {
    fn policy(&self) -> CodexGroupQuotaRefreshPolicy {
        if let Some(minutes) = self.quota_auto_refresh_minutes {
            if minutes <= -1 {
                return CodexGroupQuotaRefreshPolicy::Disabled;
            }
            if minutes > 0 {
                let clamped = minutes.clamp(1, 999) as u32;
                return CodexGroupQuotaRefreshPolicy::Minutes(clamped);
            }
            // 0 视为关闭
            return CodexGroupQuotaRefreshPolicy::Disabled;
        }
        if self.quota_refresh_enabled == Some(false) {
            return CodexGroupQuotaRefreshPolicy::Disabled;
        }
        CodexGroupQuotaRefreshPolicy::Inherit
    }
}

fn codex_account_groups_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_ACCOUNT_GROUPS_FILE))
}

fn load_codex_account_group_records() -> Vec<CodexAccountGroupRecord> {
    let path = match codex_account_groups_path() {
        Ok(path) => path,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex Groups] 解析数据目录失败，跳过分组额度策略: {}",
                error
            ));
            return Vec::new();
        }
    };

    if !path.exists() {
        return Vec::new();
    }

    let raw = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex Groups] 读取分组文件失败，跳过分组额度策略: path={}, error={}",
                path.display(),
                error
            ));
            return Vec::new();
        }
    };

    match serde_json::from_str::<Vec<CodexAccountGroupRecord>>(&raw) {
        Ok(groups) => groups,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex Groups] 解析分组文件失败，跳过分组额度策略: path={}, error={}",
                path.display(),
                error
            ));
            Vec::new()
        }
    }
}

/// 读取分组配置中「关闭额度刷新」的账号 ID 集合（策略 = Disabled / -1）。
pub fn load_quota_refresh_disabled_account_ids() -> HashSet<String> {
    let mut disabled = HashSet::new();
    for group in load_codex_account_group_records() {
        if group.policy() != CodexGroupQuotaRefreshPolicy::Disabled {
            continue;
        }
        for account_id in group.account_ids {
            let trimmed = account_id.trim();
            if !trimmed.is_empty() {
                disabled.insert(trimmed.to_string());
            }
        }
    }
    disabled
}

/// 账号是否允许参与「受策略约束」的额度刷新（自动/全量/默认批量）。
pub fn is_quota_refresh_enabled_for_account(account_id: &str) -> bool {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return true;
    }
    !load_quota_refresh_disabled_account_ids().contains(trimmed)
}

/// 按分组策略过滤账号 ID（剔除 Disabled），保持顺序。
pub fn filter_account_ids_by_quota_refresh_policy(account_ids: &[String]) -> Vec<String> {
    let disabled = load_quota_refresh_disabled_account_ids();
    if disabled.is_empty() {
        return account_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect();
    }
    account_ids
        .iter()
        .filter_map(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() || disabled.contains(trimmed) {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

/// 列出所有账号
pub fn list_accounts() -> Vec<CodexAccount> {
    let mut index = load_account_index();
    let accounts: Vec<CodexAccount> = index
        .accounts
        .iter()
        .filter_map(
            |summary| match load_account_with_summary(&summary.id, Some(summary)) {
                Ok(account) => account,
                Err(error) => {
                    logger::log_warn(&format!(
                        "[Codex Account] 跳过无法读取的账号详情: account_id={}, error={}",
                        summary.id, error
                    ));
                    None
                }
            },
        )
        .collect();
    if sync_loaded_accounts_to_index_cache(&mut index, &accounts) {
        if let Err(error) = save_account_index(&index) {
            logger::log_warn(&format!(
                "[Codex Account] 同步账号详情摘要到索引缓存失败: error={}",
                error
            ));
        }
    }
    spawn_fingerprint_default_session_resync();
    accounts
}

pub fn list_accounts_checked() -> Result<Vec<CodexAccount>, String> {
    let mut index = load_account_index_checked()?;
    let mut accounts = Vec::new();
    let mut failed = Vec::new();
    let mut missing_detail_ids = Vec::new();
    let mut has_non_missing_failure = false;

    for summary in &index.accounts {
        match load_account_with_summary(&summary.id, Some(summary)) {
            Ok(Some(account)) => accounts.push(account),
            Ok(None) => {
                missing_detail_ids.push(summary.id.clone());
                failed.push(format!("{}: 详情文件不存在", summary.id));
            }
            Err(error) => {
                has_non_missing_failure = true;
                failed.push(format!("{}: {}", summary.id, error));
            }
        }
    }

    if !index.accounts.is_empty() && accounts.is_empty() {
        if !has_non_missing_failure && missing_detail_ids.len() == index.accounts.len() {
            logger::log_warn(&format!(
                "[Codex Account] 账号索引仅剩缺失详情文件的孤儿记录，已清空索引: {}",
                missing_detail_ids.join(", ")
            ));
            index.accounts.clear();
            index.current_account_id = None;
            save_account_index(&index)?;
            return Ok(Vec::new());
        }
        return Err(format!(
            "Codex 账号索引中有 {} 个账号，但详情文件均无法读取；已保留前端缓存，请从账号备份或本地账号文件恢复。{}",
            index.accounts.len(),
            failed.join("; ")
        ));
    }

    if !failed.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account] 部分账号详情无法读取，已保留可读取账号: loaded={}, failed={}",
            accounts.len(),
            failed.join("; ")
        ));
    }

    if sync_loaded_accounts_to_index_cache(&mut index, &accounts) {
        save_account_index(&index)?;
    }

    spawn_fingerprint_default_session_resync();
    Ok(accounts)
}

