use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use crate::models::windsurf::{
    WindsurfAccount, WindsurfAccountIndex, WindsurfOAuthCompletePayload,
};
use crate::modules::{account, logger, windsurf_oauth};

const ACCOUNTS_INDEX_FILE: &str = "windsurf_accounts.json";
const ACCOUNTS_DIR: &str = "windsurf_accounts";
static WINDSURF_ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));
static WINDSURF_QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
const WINDSURF_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 300;

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Windsurf 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(get_accounts_index_path()?.to_string_lossy().to_string())
}

pub fn load_account(account_id: &str) -> Option<WindsurfAccount> {
    let account_path = get_accounts_dir()
        .ok()
        .map(|dir| dir.join(format!("{}.json", account_id)))?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_account_file(account: &WindsurfAccount) -> Result<(), String> {
    let path = get_accounts_dir()?.join(format!("{}.json", account.id));
    let content =
        serde_json::to_string_pretty(account).map_err(|e| format!("序列化账号失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存账号失败: {}", e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = get_accounts_dir()?.join(format!("{}.json", account_id));
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("删除账号文件失败: {}", e))?;
    }
    Ok(())
}

fn load_account_index() -> WindsurfAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return WindsurfAccountIndex::new(),
    };

    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(WindsurfAccountIndex::new);
    }

    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空")
                .unwrap_or_else(WindsurfAccountIndex::new)
        }
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            WindsurfAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(WindsurfAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Windsurf Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(WindsurfAccountIndex::new)
            }
        },
        Err(_) => WindsurfAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<WindsurfAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(WindsurfAccountIndex::new());
    }

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            if let Some(index) = repair_account_index_from_details("索引文件读取失败") {
                return Ok(index);
            }
            return Err(format!("读取账号索引失败: {}", err));
        }
    };

    if content.trim().is_empty() {
        if let Some(index) = repair_account_index_from_details("索引文件为空") {
            return Ok(index);
        }
        return Ok(WindsurfAccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<WindsurfAccountIndex>(
        &path, &content,
    ) {
        Ok(index) if !index.accounts.is_empty() => Ok(index),
        Ok(index) => {
            if let Some(repaired) = repair_account_index_from_details("索引账号列表为空") {
                return Ok(repaired);
            }
            Ok(index)
        }
        Err(err) => {
            if let Some(index) = repair_account_index_from_details("索引文件损坏") {
                return Ok(index);
            }
            Err(crate::error::file_corrupted_error(
                ACCOUNTS_INDEX_FILE,
                &path.to_string_lossy(),
                &err.to_string(),
            ))
        }
    }
}

fn save_account_index(index: &WindsurfAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<WindsurfAccountIndex> {
    let index_path = get_accounts_index_path().ok()?;
    let accounts_dir = get_accounts_dir().ok()?;
    let mut accounts = crate::modules::account_index_repair::load_accounts_from_details(
        &accounts_dir,
        |account_id| load_account(account_id),
    )
    .ok()?;

    if accounts.is_empty() {
        return None;
    }

    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut accounts,
        |account| account.last_used,
        |account| account.created_at,
        |account| account.id.as_str(),
    );

    let mut index = WindsurfAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Windsurf Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Windsurf Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_warn(&format!(
        "[Windsurf Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut WindsurfAccountIndex, account: &WindsurfAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: WindsurfAccount) -> Result<WindsurfAccount, String> {
    let _lock = WINDSURF_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Windsurf 账号锁失败".to_string())?;
    let mut index = load_account_index();
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
}

fn persist_quota_query_error(account_id: &str, message: &str) {
    let Some(mut account) = load_account(account_id) else {
        return;
    };
    account.quota_query_last_error = Some(message.to_string());
    account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    let _ = upsert_account_record(account);
}

fn normalize_login(payload: &WindsurfOAuthCompletePayload) -> String {
    if !payload.github_login.trim().is_empty() {
        return payload.github_login.trim().to_string();
    }
    if let Some(email) = payload
        .github_email
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return email
            .split('@')
            .next()
            .unwrap_or("windsurf_user")
            .to_string();
    }
    if let Some(name) = payload
        .github_name
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return name.to_string();
    }
    "windsurf_user".to_string()
}

fn pick_string_from_object(obj: Option<&Value>, keys: &[&str]) -> Option<String> {
    let obj = obj.and_then(Value::as_object)?;
    for key in keys {
        if let Some(text) = obj.get(*key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_email(value: Option<&str>) -> Option<String> {
    normalize_non_empty(value).and_then(|email| {
        if email.contains('@') {
            Some(email.to_lowercase())
        } else {
            None
        }
    })
}

fn resolve_account_api_key(account: &WindsurfAccount) -> Option<String> {
    normalize_non_empty(account.windsurf_api_key.as_deref())
        .or_else(|| {
            pick_string_from_object(
                account.windsurf_auth_status_raw.as_ref(),
                &["apiKey", "api_key"],
            )
        })
        .or_else(|| {
            normalize_non_empty(Some(account.github_access_token.as_str()))
                .filter(|token| token.starts_with("sk-ws-"))
        })
}

fn resolve_account_email(account: &WindsurfAccount) -> Option<String> {
    normalize_email(account.github_email.as_deref())
        .or_else(|| pick_string_from_object(account.windsurf_auth_status_raw.as_ref(), &["email"]))
        .and_then(|email| normalize_email(Some(email.as_str())))
}

fn resolve_payload_api_key(payload: &WindsurfOAuthCompletePayload) -> Option<String> {
    normalize_non_empty(payload.windsurf_api_key.as_deref())
        .or_else(|| {
            pick_string_from_object(
                payload.windsurf_auth_status_raw.as_ref(),
                &["apiKey", "api_key"],
            )
        })
        .or_else(|| {
            normalize_non_empty(Some(payload.github_access_token.as_str()))
                .filter(|token| token.starts_with("sk-ws-"))
        })
}

fn account_apis_are_compatible(left: &WindsurfAccount, right: &WindsurfAccount) -> bool {
    match (
        resolve_account_api_key(left),
        resolve_account_api_key(right),
    ) {
        (Some(left_api), Some(right_api)) => left_api == right_api,
        _ => true,
    }
}

fn merge_tags(target: &mut Option<Vec<String>>, source: Option<&Vec<String>>) {
    let mut merged = target.clone().unwrap_or_default();
    let mut seen: HashSet<String> = merged
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();

    if let Some(source_tags) = source {
        for tag in source_tags {
            let normalized = tag.trim().to_lowercase();
            if normalized.is_empty() || !seen.insert(normalized.clone()) {
                continue;
            }
            merged.push(normalized);
        }
    }

    *target = if merged.is_empty() {
        None
    } else {
        Some(merged)
    };
}

fn merge_json_object_fields(target: &mut Option<Value>, source: Option<&Value>) {
    let Some(source_obj) = source.and_then(Value::as_object) else {
        return;
    };

    if !target.as_ref().map(Value::is_object).unwrap_or(false) {
        *target = Some(serde_json::json!({}));
    }

    if let Some(target_obj) = target.as_mut().and_then(Value::as_object_mut) {
        for (key, value) in source_obj {
            if value.is_null() {
                continue;
            }
            target_obj.insert(key.clone(), value.clone());
        }
    }
}

fn merge_account_group(group: Vec<WindsurfAccount>) -> WindsurfAccount {
    let keep_id = group
        .iter()
        .min_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)))
        .map(|account| account.id.clone())
        .unwrap_or_else(|| "windsurf_merged".to_string());

    let created_at = group
        .iter()
        .map(|account| account.created_at)
        .min()
        .unwrap_or_else(now_ts);
    let last_used = group
        .iter()
        .map(|account| account.last_used)
        .max()
        .unwrap_or(created_at);

    let mut merged = group
        .iter()
        .max_by_key(|account| account.last_used)
        .cloned()
        .unwrap_or_else(|| group[0].clone());

    merged.id = keep_id;
    merged.created_at = created_at;
    merged.last_used = last_used;

    for source in &group {
        merge_tags(&mut merged.tags, source.tags.as_ref());
        merge_json_object_fields(
            &mut merged.windsurf_auth_status_raw,
            source.windsurf_auth_status_raw.as_ref(),
        );

        if merged.github_email.is_none() {
            merged.github_email = source.github_email.clone();
        }
        if merged.github_name.is_none() {
            merged.github_name = source.github_name.clone();
        }
        if merged.windsurf_api_key.is_none() {
            merged.windsurf_api_key = source.windsurf_api_key.clone();
        }
        if merged.windsurf_api_server_url.is_none() {
            merged.windsurf_api_server_url = source.windsurf_api_server_url.clone();
        }
        if merged.windsurf_auth_token.is_none() {
            merged.windsurf_auth_token = source.windsurf_auth_token.clone();
        }
        if merged.windsurf_user_status.is_none() {
            merged.windsurf_user_status = source.windsurf_user_status.clone();
        }
        if merged.windsurf_plan_status.is_none() {
            merged.windsurf_plan_status = source.windsurf_plan_status.clone();
        }
    }

    if merged.windsurf_api_key.is_none() {
        merged.windsurf_api_key = resolve_account_api_key(&merged);
    }
    if merged.github_email.is_none() {
        merged.github_email = resolve_account_email(&merged);
    }

    merged
}

fn deduplicate_accounts_by_identity_with_index(index: WindsurfAccountIndex) -> Result<(), String> {
    if index.accounts.len() <= 1 {
        return Ok(());
    }

    let mut accounts: Vec<WindsurfAccount> = index
        .accounts
        .iter()
        .filter_map(|summary| load_account(&summary.id))
        .collect();
    if accounts.len() <= 1 {
        return Ok(());
    }

    for account in &mut accounts {
        merge_local_auth_status_into_account(account);
    }

    let mut groups: Vec<Vec<WindsurfAccount>> = Vec::new();
    for account in accounts {
        // 仅按 github_id 分组合并，同邮箱不同 github_id 不合并
        let mut matched_indices: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                group.iter().any(|existing| {
                    existing.github_id == account.github_id
                        && account_apis_are_compatible(&account, existing)
                })
            })
            .map(|(idx, _)| idx)
            .collect();

        if matched_indices.is_empty() {
            groups.push(vec![account]);
            continue;
        }

        matched_indices.sort_unstable();
        let primary_idx = matched_indices[0];
        groups[primary_idx].push(account);

        for idx in matched_indices.into_iter().skip(1).rev() {
            let mut moved = groups.remove(idx);
            groups[primary_idx].append(&mut moved);
        }
    }

    if groups.iter().all(|group| group.len() == 1) {
        return Ok(());
    }

    let old_ids: HashSet<String> = groups
        .iter()
        .flat_map(|group| group.iter().map(|account| account.id.clone()))
        .collect();

    let mut merged_accounts = Vec::new();
    let mut merged_duplicate_count = 0usize;
    for group in groups {
        if group.len() == 1 {
            merged_accounts.push(group.into_iter().next().unwrap());
            continue;
        }
        merged_duplicate_count += group.len() - 1;
        merged_accounts.push(merge_account_group(group));
    }

    merged_accounts.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

    for account in &merged_accounts {
        save_account_file(account)?;
    }

    let keep_ids: HashSet<String> = merged_accounts
        .iter()
        .map(|account| account.id.clone())
        .collect();
    for old_id in old_ids {
        if !keep_ids.contains(&old_id) {
            delete_account_file(&old_id)?;
        }
    }

    let mut next_index = WindsurfAccountIndex::new();
    next_index.accounts = merged_accounts
        .iter()
        .map(WindsurfAccount::summary)
        .collect();
    save_account_index(&next_index)?;

    logger::log_info(&format!(
        "Windsurf 账号去重完成：合并 {} 条重复记录",
        merged_duplicate_count
    ));
    Ok(())
}

fn deduplicate_accounts_by_identity() -> Result<(), String> {
    deduplicate_accounts_by_identity_with_index(load_account_index())
}

fn deduplicate_accounts_by_identity_checked() -> Result<(), String> {
    let index = load_account_index_checked()?;
    deduplicate_accounts_by_identity_with_index(index)
}

fn merge_local_auth_status_into_account(account: &mut WindsurfAccount) {
    let account_api_key = account
        .windsurf_api_key
        .clone()
        .or_else(|| {
            if account.github_access_token.starts_with("sk-ws-") {
                Some(account.github_access_token.clone())
            } else {
                None
            }
        })
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    let Some(account_api_key) = account_api_key else {
        return;
    };

    let local_auth_status = match read_local_auth_status() {
        Ok(Some(value)) => value,
        _ => return,
    };
    let local_api_key = pick_string_from_object(Some(&local_auth_status), &["apiKey", "api_key"]);
    if local_api_key.as_deref() != Some(account_api_key.as_str()) {
        return;
    }

    let mut merged = account
        .windsurf_auth_status_raw
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));
    if !merged.is_object() {
        merged = serde_json::json!({});
    }

    if let (Some(target), Some(source)) = (merged.as_object_mut(), local_auth_status.as_object()) {
        for (key, value) in source {
            if value.is_null() {
                continue;
            }
            target.insert(key.clone(), value.clone());
        }
    }

    account.windsurf_auth_status_raw = Some(merged.clone());
    if account.windsurf_api_server_url.is_none() {
        account.windsurf_api_server_url =
            pick_string_from_object(Some(&merged), &["apiServerUrl", "api_server_url"]);
    }
    if account.github_email.is_none() {
        account.github_email = pick_string_from_object(Some(&merged), &["email"]);
    }
    if account.github_name.is_none() {
        account.github_name = pick_string_from_object(Some(&merged), &["name"]);
    }
}

fn apply_payload(account: &mut WindsurfAccount, payload: WindsurfOAuthCompletePayload) {
    account.github_login = normalize_login(&payload);
    account.github_id = payload.github_id;
    account.github_name = payload.github_name;
    account.github_email = payload.github_email;
    account.github_access_token = payload.github_access_token;
    account.github_token_type = payload.github_token_type;
    account.github_scope = payload.github_scope;
    account.copilot_token = payload.copilot_token;
    account.copilot_plan = payload.copilot_plan;
    account.copilot_chat_enabled = payload.copilot_chat_enabled;
    account.copilot_expires_at = payload.copilot_expires_at;
    account.copilot_refresh_in = payload.copilot_refresh_in;
    account.copilot_quota_snapshots = payload.copilot_quota_snapshots;
    account.copilot_quota_reset_date = payload.copilot_quota_reset_date;
    account.copilot_limited_user_quotas = payload.copilot_limited_user_quotas;
    account.copilot_limited_user_reset_date = payload.copilot_limited_user_reset_date;
    account.windsurf_api_key = payload.windsurf_api_key;
    account.windsurf_api_server_url = payload.windsurf_api_server_url;
    account.windsurf_auth_token = payload.windsurf_auth_token;
    account.windsurf_user_status = payload.windsurf_user_status;
    account.windsurf_plan_status = payload.windsurf_plan_status;
    account.windsurf_auth_status_raw = payload.windsurf_auth_status_raw;
    account.last_used = now_ts();
}

fn value_has_number_like(value: &Value) -> bool {
    value.is_number()
        || value
            .as_str()
            .and_then(|text| text.trim().parse::<f64>().ok())
            .is_some()
}

fn status_has_credit_fields(status: Option<&Value>) -> bool {
    let keys = [
        "availablePromptCredits",
        "available_prompt_credits",
        "usedPromptCredits",
        "used_prompt_credits",
        "availableFlowCredits",
        "available_flow_credits",
        "usedFlowCredits",
        "used_flow_credits",
        "completions",
        "chat",
    ];

    let check_obj = |obj: Option<&serde_json::Map<String, Value>>| -> bool {
        let Some(obj) = obj else {
            return false;
        };
        keys.iter()
            .any(|key| obj.get(*key).map(value_has_number_like).unwrap_or(false))
    };

    let Some(value) = status else {
        return false;
    };
    if check_obj(value.as_object()) {
        return true;
    }

    check_obj(value.get("planStatus").and_then(Value::as_object))
}

fn status_has_plan_snapshot(status: Option<&Value>) -> bool {
    let Some(value) = status else {
        return false;
    };

    value.as_object().is_some()
        || value.get("planStatus").and_then(Value::as_object).is_some()
        || value
            .get("userStatus")
            .and_then(|user_status| user_status.get("planStatus"))
            .and_then(Value::as_object)
            .is_some()
}

fn has_quota_data(payload: &WindsurfOAuthCompletePayload) -> bool {
    status_has_credit_fields(payload.copilot_limited_user_quotas.as_ref())
        || status_has_plan_snapshot(payload.windsurf_plan_status.as_ref())
        || status_has_plan_snapshot(payload.windsurf_user_status.as_ref())
}

fn merge_refresh_payload_with_existing(
    account: &WindsurfAccount,
    payload: &mut WindsurfOAuthCompletePayload,
) -> bool {
    let mut preserved_quota = false;

    if !has_quota_data(payload) {
        preserved_quota = true;
        payload.copilot_token = account.copilot_token.clone();
        payload.copilot_quota_snapshots = account.copilot_quota_snapshots.clone();
        payload.copilot_quota_reset_date = account.copilot_quota_reset_date.clone();
        payload.copilot_limited_user_quotas = account.copilot_limited_user_quotas.clone();
        payload.copilot_limited_user_reset_date = account.copilot_limited_user_reset_date;
        if payload.copilot_plan.is_none() {
            payload.copilot_plan = account.copilot_plan.clone();
        }
        if payload.windsurf_plan_status.is_none() {
            payload.windsurf_plan_status = account.windsurf_plan_status.clone();
        }
        if payload.windsurf_user_status.is_none() {
            payload.windsurf_user_status = account.windsurf_user_status.clone();
        }
    }

    if payload.github_email.is_none() {
        payload.github_email = account.github_email.clone();
    }
    if payload.github_name.is_none() {
        payload.github_name = account.github_name.clone();
    }
    if payload.windsurf_api_key.is_none() {
        payload.windsurf_api_key = account.windsurf_api_key.clone();
    }
    if payload.windsurf_api_server_url.is_none() {
        payload.windsurf_api_server_url = account.windsurf_api_server_url.clone();
    }
    if payload.windsurf_auth_token.is_none() {
        payload.windsurf_auth_token = account.windsurf_auth_token.clone();
    }
    if payload.windsurf_auth_status_raw.is_none() {
        payload.windsurf_auth_status_raw = account.windsurf_auth_status_raw.clone();
    }

    preserved_quota
}

pub fn list_accounts() -> Vec<WindsurfAccount> {
    if let Err(err) = deduplicate_accounts_by_identity() {
        logger::log_warn(&format!("Windsurf 账号去重失败（已忽略）：{}", err));
    }

    let index = load_account_index();
    index
        .accounts
        .iter()
        .filter_map(|summary| load_account(&summary.id))
        .map(|mut account| {
            merge_local_auth_status_into_account(&mut account);
            account
        })
        .collect()
}

pub fn list_accounts_checked() -> Result<Vec<WindsurfAccount>, String> {
    deduplicate_accounts_by_identity_checked()?;
    let index = load_account_index_checked()?;
    Ok(index
        .accounts
        .iter()
        .filter_map(|summary| load_account(&summary.id))
        .map(|mut account| {
            merge_local_auth_status_into_account(&mut account);
            account
        })
        .collect())
}

pub fn upsert_account(payload: WindsurfOAuthCompletePayload) -> Result<WindsurfAccount, String> {
    let _lock = WINDSURF_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Windsurf 账号锁失败".to_string())?;
    if let Err(err) = deduplicate_accounts_by_identity() {
        logger::log_warn(&format!("Windsurf upsert 前去重失败（已忽略）：{}", err));
    }

    let now = now_ts();
    let mut index = load_account_index();
    let normalized_login = normalize_login(&payload);
    let payload_api_key = resolve_payload_api_key(&payload);
    // lgtm[rs/weak-cryptographic-algorithm] 仅用于生成本地稳定账号 ID（非密码学用途，不参与认证/签名/加密）
    let generated_id = format!(
        "windsurf_{:x}",
        md5::compute(format!("{}:{}", normalized_login, payload.github_id))
    );

    // 以 github_id 唯一标识区分账号，同邮箱不同 GitHub 账号不合并
    let account_id = index
        .accounts
        .iter()
        .filter_map(|item| load_account(&item.id))
        .find(|account| {
            if account.github_id != payload.github_id {
                return false;
            }
            if let (Some(incoming_api), Some(existing_api)) = (
                payload_api_key.as_ref(),
                resolve_account_api_key(account).as_ref(),
            ) {
                if incoming_api != existing_api {
                    return false;
                }
            }
            true
        })
        .map(|account| account.id)
        .unwrap_or(generated_id);

    let existing = load_account(&account_id);
    let tags = existing.as_ref().and_then(|acc| acc.tags.clone());
    let created_at = existing.as_ref().map(|acc| acc.created_at).unwrap_or(now);

    let mut account = existing.unwrap_or(WindsurfAccount {
        id: account_id.clone(),
        github_login: normalized_login.clone(),
        github_id: payload.github_id,
        github_name: payload.github_name.clone(),
        github_email: payload.github_email.clone(),
        tags,
        github_access_token: payload.github_access_token.clone(),
        github_token_type: payload.github_token_type.clone(),
        github_scope: payload.github_scope.clone(),
        copilot_token: payload.copilot_token.clone(),
        copilot_plan: payload.copilot_plan.clone(),
        copilot_chat_enabled: payload.copilot_chat_enabled,
        copilot_expires_at: payload.copilot_expires_at,
        copilot_refresh_in: payload.copilot_refresh_in,
        copilot_quota_snapshots: payload.copilot_quota_snapshots.clone(),
        copilot_quota_reset_date: payload.copilot_quota_reset_date.clone(),
        copilot_limited_user_quotas: payload.copilot_limited_user_quotas.clone(),
        copilot_limited_user_reset_date: payload.copilot_limited_user_reset_date,
        windsurf_api_key: payload.windsurf_api_key.clone(),
        windsurf_api_server_url: payload.windsurf_api_server_url.clone(),
        windsurf_auth_token: payload.windsurf_auth_token.clone(),
        windsurf_user_status: payload.windsurf_user_status.clone(),
        windsurf_plan_status: payload.windsurf_plan_status.clone(),
        windsurf_auth_status_raw: payload.windsurf_auth_status_raw.clone(),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        windsurf_token_type: payload.windsurf_token_type.clone(),
        devin_auth1_token: payload.devin_auth1_token.clone(),
        devin_account_id: payload.devin_account_id.clone(),
        devin_org_id: payload.devin_org_id.clone(),
        devin_session_token: payload.devin_session_token.clone(),
        devin_user_status_proto_b64: payload.devin_user_status_proto_b64.clone(),
        created_at,
        last_used: now,
    });

    apply_payload(&mut account, payload);
    account.id = account_id;
    account.created_at = created_at;
    account.quota_query_last_error = None;
    account.quota_query_last_error_at = None;
    account.last_used = now;

    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;

    logger::log_info(&format!(
        "Windsurf 账号已保存: id={}, login={}",
        account.id, account.github_login
    ));
    Ok(account)
}

async fn refresh_account_token_once(account_id: &str) -> Result<WindsurfAccount, String> {
    let started_at = Instant::now();
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[Windsurf Refresh] 开始刷新账号: id={}, login={}",
        account.id, account.github_login
    ));
    merge_local_auth_status_into_account(&mut account);
    let mut payload = match windsurf_oauth::refresh_payload_for_account(&account).await {
        Ok(value) => value,
        Err(err) => {
            logger::log_warn(&format!(
                "[Windsurf Refresh] 刷新失败: id={}, login={}, error={}",
                account.id, account.github_login, err
            ));
            return Err(err);
        }
    };
    let preserved_quota = merge_refresh_payload_with_existing(&account, &mut payload);
    if preserved_quota {
        logger::log_warn(&format!(
            "[Windsurf Refresh] 未获取到有效配额快照，保留旧配额: id={}, login={}",
            account.id, account.github_login
        ));
    }
    let tags = account.tags.clone();
    let created_at = account.created_at;
    apply_payload(&mut account, payload);
    account.tags = tags;
    account.created_at = created_at;
    let refreshed_at = now_ts();
    if !preserved_quota {
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
        account.usage_updated_at = Some(refreshed_at);
    } else {
        account.quota_query_last_error = Some("未获取到有效配额快照，已保留旧配额缓存".to_string());
        account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    }
    account.last_used = refreshed_at;

    let updated = account.clone();
    upsert_account_record(account)?;
    logger::log_info(&format!(
        "[Windsurf Refresh] 刷新完成: id={}, login={}, preserved_quota={}, elapsed={}ms",
        updated.id,
        updated.github_login,
        preserved_quota,
        started_at.elapsed().as_millis()
    ));
    Ok(updated)
}

pub async fn refresh_account_token(account_id: &str) -> Result<WindsurfAccount, String> {
    let result = refresh_account_token_once(account_id).await;
    if let Err(err) = &result {
        persist_quota_query_error(account_id, err);
    }
    result
}

pub async fn refresh_all_tokens() -> Result<Vec<(String, Result<WindsurfAccount, String>)>, String>
{
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    const MAX_CONCURRENT: usize = 5;
    let accounts = list_accounts();
    logger::log_info(&format!(
        "[Windsurf Refresh] 开始批量刷新: total={}",
        accounts.len()
    ));

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let tasks: Vec<_> = accounts
        .into_iter()
        .map(|account| {
            let id = account.id;
            let semaphore = semaphore.clone();
            async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|e| format!("获取 Windsurf 刷新并发许可失败: {}", e))?;
                let res = refresh_account_token(&id).await;
                Ok::<(String, Result<WindsurfAccount, String>), String>((id, res))
            }
        })
        .collect();

    let mut results = Vec::with_capacity(tasks.len());
    for task in join_all(tasks).await {
        match task {
            Ok(item) => {
                if let Err(err) = &item.1 {
                    logger::log_warn(&format!(
                        "[Windsurf Refresh] 账号刷新失败: id={}, error={}",
                        item.0, err
                    ));
                }
                results.push(item);
            }
            Err(err) => {
                logger::log_warn(&format!("[Windsurf Refresh] 执行任务失败: {}", err));
            }
        }
    }

    let success_count = results.iter().filter(|(_, item)| item.is_ok()).count();
    let failed_count = results.len().saturating_sub(success_count);
    logger::log_info(&format!(
        "[Windsurf Refresh] 批量刷新结束: success={}, failed={}",
        success_count, failed_count
    ));
    Ok(results)
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = WINDSURF_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Windsurf 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| item.id != account_id);
    save_account_index(&index)?;
    delete_account_file(account_id)?;
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        remove_account(id)?;
    }
    Ok(())
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<WindsurfAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    account.tags = Some(tags);
    account.last_used = now_ts();
    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<WindsurfAccount>, String> {
    if let Ok(account) = serde_json::from_str::<WindsurfAccount>(json_content) {
        let saved = upsert_account_record(account)?;
        return Ok(vec![saved]);
    }

    if let Ok(accounts) = serde_json::from_str::<Vec<WindsurfAccount>>(json_content) {
        let mut result = Vec::new();
        for account in accounts {
            let saved = upsert_account_record(account)?;
            result.push(saved);
        }
        return Ok(result);
    }

    Err("无法解析 JSON 内容".to_string())
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<WindsurfAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

pub fn get_default_state_db_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join("Library/Application Support/Windsurf/User/globalStorage/state.vscdb"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        return Ok(PathBuf::from(appdata).join("Windsurf\\User\\globalStorage\\state.vscdb"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".config/Windsurf/User/globalStorage/state.vscdb"));
    }

    #[allow(unreachable_code)]
    Err("Windsurf 账号导入仅支持 macOS、Windows 和 Linux".to_string())
}

pub fn read_local_auth_status() -> Result<Option<Value>, String> {
    let db_path = get_default_state_db_path()?;
    if !db_path.exists() {
        return Ok(None);
    }
    let conn =
        Connection::open(&db_path).map_err(|e| format!("打开 Windsurf 本地数据库失败: {}", e))?;
    let value = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            ["windsurfAuthStatus"],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("读取 windsurfAuthStatus 失败: {}", e))?;

    match value {
        Some(content) => {
            let parsed: Value = serde_json::from_str(&content)
                .map_err(|e| format!("解析 windsurfAuthStatus 失败: {}", e))?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

pub fn read_local_login_hint() -> Option<String> {
    let db_path = get_default_state_db_path().ok()?;
    if !db_path.exists() {
        return None;
    }
    let conn = Connection::open(&db_path).ok()?;
    let key = conn
        .query_row(
            "SELECT key FROM ItemTable WHERE key LIKE 'windsurf_auth-%' LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()?;
    key.strip_prefix("windsurf_auth-")
        .map(|value| value.to_string())
}

fn normalize_quota_alert_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn clamp_percent(value: f64) -> i32 {
    value.round().clamp(0.0, 100.0) as i32
}

fn parse_token_map(token: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let prefix = token.split(':').next().unwrap_or(token);
    for item in prefix.split(';') {
        let mut parts = item.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        if key.is_empty() {
            continue;
        }
        let value = parts.next().unwrap_or("").trim();
        map.insert(key.to_string(), value.to_string());
    }
    map
}

fn parse_token_number(map: &HashMap<String, String>, key: &str) -> Option<f64> {
    map.get(key)
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn get_json_number(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(num) => num.as_f64(),
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn calc_remaining_percent(remaining: f64, total: f64) -> Option<i32> {
    if total <= 0.0 {
        return None;
    }
    Some(clamp_percent((remaining.max(0.0) / total) * 100.0))
}

fn extract_limited_metrics(account: &WindsurfAccount) -> Vec<(String, i32)> {
    let Some(limited) = account
        .copilot_limited_user_quotas
        .as_ref()
        .and_then(|value| value.as_object())
    else {
        return Vec::new();
    };

    let token_map = parse_token_map(&account.copilot_token);
    let mut metrics = Vec::new();

    if let Some(remaining_completions) = limited.get("completions").and_then(get_json_number) {
        let total_completions =
            parse_token_number(&token_map, "cq").unwrap_or(remaining_completions);
        if let Some(percent) = calc_remaining_percent(remaining_completions, total_completions) {
            metrics.push(("Prompt Credits".to_string(), percent));
        }
    }

    if let Some(remaining_chat) = limited.get("chat").and_then(get_json_number) {
        let total_chat = parse_token_number(&token_map, "tq").unwrap_or(remaining_chat);
        if let Some(percent) = calc_remaining_percent(remaining_chat, total_chat) {
            metrics.push(("Flow Action Credits".to_string(), percent));
        }
    }

    metrics
}

fn extract_premium_metric(account: &WindsurfAccount) -> Option<(String, i32)> {
    let snapshots = account
        .copilot_quota_snapshots
        .as_ref()
        .and_then(|value| value.as_object())?;

    let premium = snapshots
        .get("premium_interactions")
        .or_else(|| snapshots.get("premium_models"))
        .and_then(|value| value.as_object())?;

    if premium.get("unlimited").and_then(|value| value.as_bool()) == Some(true) {
        return Some(("Premium Interactions".to_string(), 100));
    }

    let percent_remaining = premium
        .get("percent_remaining")
        .and_then(get_json_number)
        .map(clamp_percent)?;

    Some(("Premium Interactions".to_string(), percent_remaining))
}

pub(crate) fn extract_quota_metrics(account: &WindsurfAccount) -> Vec<(String, i32)> {
    let mut metrics = extract_limited_metrics(account);
    if let Some(premium) = extract_premium_metric(account) {
        metrics.push(premium);
    }
    metrics
}

fn average_quota_percentage(metrics: &[(String, i32)]) -> f64 {
    if metrics.is_empty() {
        return 0.0;
    }
    let sum: i32 = metrics.iter().map(|(_, pct)| *pct).sum();
    sum as f64 / metrics.len() as f64
}

pub(crate) fn resolve_current_account_id(accounts: &[WindsurfAccount]) -> Option<String> {
    if let Ok(Some(local_auth_status)) = read_local_auth_status() {
        let local_api_key =
            pick_string_from_object(Some(&local_auth_status), &["apiKey", "api_key"])
                .and_then(|value| normalize_non_empty(Some(value.as_str())));
        let local_email = pick_string_from_object(Some(&local_auth_status), &["email"])
            .and_then(|value| normalize_email(Some(value.as_str())));
        let local_login_hint = read_local_login_hint()
            .and_then(|value| normalize_non_empty(Some(value.as_str())))
            .map(|value| value.to_lowercase());

        if let Some(account_id) = accounts
            .iter()
            .find(|account| {
                if let (Some(existing), Some(incoming)) = (
                    resolve_account_api_key(account).as_ref(),
                    local_api_key.as_ref(),
                ) {
                    if existing == incoming {
                        return true;
                    }
                }

                if let (Some(existing), Some(incoming)) = (
                    resolve_account_email(account).as_ref(),
                    local_email.as_ref(),
                ) {
                    if existing == incoming {
                        return true;
                    }
                }

                if let Some(incoming) = local_login_hint.as_ref() {
                    return account.github_login.eq_ignore_ascii_case(incoming);
                }

                false
            })
            .map(|account| account.id.clone())
        {
            return Some(account_id);
        }
    }

    if let Ok(settings) = crate::modules::windsurf_instance::load_default_settings() {
        if let Some(bind_id) = settings.bind_account_id {
            let trimmed = bind_id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    accounts
        .iter()
        .max_by_key(|account| account.last_used)
        .map(|account| account.id.clone())
}

fn display_email(account: &WindsurfAccount) -> String {
    account
        .github_email
        .clone()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| account.github_login.clone())
}

fn build_quota_alert_cooldown_key(account_id: &str, threshold: i32) -> String {
    format!("windsurf:{}:{}", account_id, threshold)
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = WINDSURF_QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < WINDSURF_QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, threshold: i32) {
    if let Ok(mut state) = WINDSURF_QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(account_id, threshold));
    }
}

fn pick_quota_alert_recommendation(
    accounts: &[WindsurfAccount],
    current_id: &str,
) -> Option<WindsurfAccount> {
    let mut candidates: Vec<WindsurfAccount> = accounts
        .iter()
        .filter(|account| account.id != current_id)
        .filter(|account| !extract_quota_metrics(account).is_empty())
        .cloned()
        .collect();

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        let avg_a = average_quota_percentage(&extract_quota_metrics(a));
        let avg_b = average_quota_percentage(&extract_quota_metrics(b));
        avg_b
            .partial_cmp(&avg_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.last_used.cmp(&b.last_used))
    });

    candidates.into_iter().next()
}

pub fn run_quota_alert_if_needed(
) -> Result<Option<crate::modules::account::QuotaAlertPayload>, String> {
    let cfg = crate::modules::config::get_user_config();
    if !cfg.windsurf_quota_alert_enabled {
        return Ok(None);
    }

    let threshold = normalize_quota_alert_threshold(cfg.windsurf_quota_alert_threshold);
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
        .filter(|(_, pct)| *pct <= threshold)
        .collect();

    if low_models.is_empty() {
        clear_quota_alert_cooldown(&current_id, threshold);
        return Ok(None);
    }

    let now = chrono::Utc::now().timestamp();
    let cooldown_key = build_quota_alert_cooldown_key(&current_id, threshold);
    if !should_emit_quota_alert(&cooldown_key, now) {
        return Ok(None);
    }

    let recommendation = pick_quota_alert_recommendation(&accounts, &current_id);
    let lowest_percentage = low_models.iter().map(|(_, pct)| *pct).min().unwrap_or(0);
    let payload = crate::modules::account::QuotaAlertPayload {
        platform: "windsurf".to_string(),
        current_account_id: current_id,
        current_email: display_email(current),
        threshold,
        threshold_display: None,
        lowest_percentage,
        low_models: low_models.into_iter().map(|(name, _)| name).collect(),
        recommended_account_id: recommendation.as_ref().map(|account| account.id.clone()),
        recommended_email: recommendation.as_ref().map(display_email),
        triggered_at: now,
    };

    crate::modules::account::dispatch_quota_alert(&payload);
    Ok(Some(payload))
}
