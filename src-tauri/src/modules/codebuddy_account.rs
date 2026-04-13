use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;
use tauri::Emitter;

use crate::models::codebuddy::{
    CodebuddyAccount, CodebuddyAccountIndex, CodebuddyOAuthCompletePayload,
};
use crate::modules::{account, codebuddy_oauth, logger};

const ACCOUNTS_INDEX_FILE: &str = "codebuddy_accounts.json";
const ACCOUNTS_DIR: &str = "codebuddy_accounts";
const CODEBUDDY_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 10 * 60;
const CODEBUDDY_SECRET_EXTENSION_ID: &str = "tencent-cloud.coding-copilot";
const CODEBUDDY_SECRET_KEY: &str = "planning-genie.new.accessToken";

lazy_static::lazy_static! {
    static ref CODEBUDDY_ACCOUNT_INDEX_LOCK: Mutex<()> = Mutex::new(());
    static ref CODEBUDDY_QUOTA_ALERT_LAST_SENT: Mutex<HashMap<String, i64>> = Mutex::new(HashMap::new());
}

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
        fs::create_dir_all(&dir).map_err(|e| format!("创建 CodeBuddy 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(get_accounts_index_path()?.to_string_lossy().to_string())
}

fn normalize_account_id(account_id: &str) -> Result<String, String> {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("账号 ID 非法，包含路径字符".to_string());
    }
    let valid = trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.');
    if !valid {
        return Err("账号 ID 非法，仅允许字母/数字/._-".to_string());
    }
    Ok(trimmed.to_string())
}

fn resolve_account_file_path(account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    Ok(get_accounts_dir()?.join(format!("{}.json", normalized)))
}

pub fn load_account(account_id: &str) -> Option<CodebuddyAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_account_file(account: &CodebuddyAccount) -> Result<(), String> {
    let path = resolve_account_file_path(account.id.as_str())?;
    let content =
        serde_json::to_string_pretty(account).map_err(|e| format!("序列化账号失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存账号失败: {}", e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("删除账号文件失败: {}", e))?;
    }
    Ok(())
}

fn load_account_index() -> CodebuddyAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return CodebuddyAccountIndex::new(),
    };
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(CodebuddyAccountIndex::new);
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空")
                .unwrap_or_else(CodebuddyAccountIndex::new)
        }
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            CodebuddyAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(CodebuddyAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[CodeBuddy Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(CodebuddyAccountIndex::new)
            }
        },
        Err(_) => CodebuddyAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<CodebuddyAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(CodebuddyAccountIndex::new());
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
        return Ok(CodebuddyAccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<CodebuddyAccountIndex>(
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

fn save_account_index(index: &CodebuddyAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<CodebuddyAccountIndex> {
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

    let mut index = CodebuddyAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[CodeBuddy Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[CodeBuddy Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_warn(&format!(
        "[CodeBuddy Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut CodebuddyAccountIndex, account: &CodebuddyAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: CodebuddyAccount) -> Result<CodebuddyAccount, String> {
    let _lock = CODEBUDDY_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 CodeBuddy 账号锁失败".to_string())?;
    let mut index = load_account_index();
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
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

fn normalize_identity(value: Option<&str>) -> Option<String> {
    normalize_non_empty(value).map(|v| v.to_lowercase())
}

fn normalize_email_identity(value: Option<&str>) -> Option<String> {
    normalize_non_empty(value).and_then(|raw| {
        let lowered = raw.to_lowercase();
        if lowered.contains('@') {
            Some(lowered)
        } else {
            None
        }
    })
}

fn account_matches_payload_identity(
    existing_uid: Option<&String>,
    existing_email: Option<&String>,
    incoming_uid: Option<&String>,
    incoming_email: Option<&String>,
) -> bool {
    if let (Some(existing), Some(incoming)) = (existing_uid, incoming_uid) {
        if existing == incoming {
            return true;
        }
    }
    if let (Some(existing), Some(incoming)) = (existing_email, incoming_email) {
        if existing == incoming {
            if let (Some(eu), Some(iu)) = (existing_uid, incoming_uid) {
                if eu != iu {
                    return false;
                }
            }
            return true;
        }
    }
    false
}

fn accounts_are_duplicates(left: &CodebuddyAccount, right: &CodebuddyAccount) -> bool {
    let left_uid = normalize_identity(left.uid.as_deref());
    let right_uid = normalize_identity(right.uid.as_deref());
    let left_email = normalize_email_identity(Some(left.email.as_str()));
    let right_email = normalize_email_identity(Some(right.email.as_str()));

    let uid_conflict = matches!(
        (left_uid.as_ref(), right_uid.as_ref()),
        (Some(l), Some(r)) if l != r
    );
    let email_conflict = matches!(
        (left_email.as_ref(), right_email.as_ref()),
        (Some(l), Some(r)) if l != r
    );
    if uid_conflict || email_conflict {
        return false;
    }

    let uid_match = matches!(
        (left_uid.as_ref(), right_uid.as_ref()),
        (Some(l), Some(r)) if l == r
    );
    let email_match = matches!(
        (left_email.as_ref(), right_email.as_ref()),
        (Some(l), Some(r)) if l == r
    );

    uid_match || email_match
}

fn merge_string_list(
    primary: Option<Vec<String>>,
    secondary: Option<Vec<String>>,
) -> Option<Vec<String>> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    for source in [primary, secondary] {
        if let Some(values) = source {
            for value in values {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let key = trimmed.to_lowercase();
                if seen.insert(key) {
                    merged.push(trimmed.to_string());
                }
            }
        }
    }
    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

fn fill_if_none<T: Clone>(target: &mut Option<T>, source: &Option<T>) {
    if target.is_none() {
        *target = source.clone();
    }
}

fn merge_duplicate_account(primary: &mut CodebuddyAccount, dup: &CodebuddyAccount) {
    if primary.email.trim().is_empty() && !dup.email.trim().is_empty() {
        primary.email = dup.email.clone();
    }
    if primary.access_token.trim().is_empty() && !dup.access_token.trim().is_empty() {
        primary.access_token = dup.access_token.clone();
    }
    fill_if_none(&mut primary.uid, &dup.uid);
    fill_if_none(&mut primary.nickname, &dup.nickname);
    fill_if_none(&mut primary.enterprise_id, &dup.enterprise_id);
    fill_if_none(&mut primary.enterprise_name, &dup.enterprise_name);
    fill_if_none(&mut primary.refresh_token, &dup.refresh_token);
    fill_if_none(&mut primary.token_type, &dup.token_type);
    fill_if_none(&mut primary.expires_at, &dup.expires_at);
    fill_if_none(&mut primary.domain, &dup.domain);
    fill_if_none(&mut primary.plan_type, &dup.plan_type);
    fill_if_none(&mut primary.dosage_notify_code, &dup.dosage_notify_code);
    fill_if_none(&mut primary.payment_type, &dup.payment_type);
    fill_if_none(&mut primary.quota_raw, &dup.quota_raw);
    fill_if_none(&mut primary.auth_raw, &dup.auth_raw);
    fill_if_none(&mut primary.profile_raw, &dup.profile_raw);
    fill_if_none(&mut primary.usage_raw, &dup.usage_raw);
    fill_if_none(&mut primary.status, &dup.status);
    fill_if_none(
        &mut primary.quota_query_last_error,
        &dup.quota_query_last_error,
    );
    fill_if_none(
        &mut primary.quota_query_last_error_at,
        &dup.quota_query_last_error_at,
    );
    primary.tags = merge_string_list(primary.tags.clone(), dup.tags.clone());
    primary.created_at = primary.created_at.min(dup.created_at);
    primary.last_used = primary.last_used.max(dup.last_used);
}

fn choose_primary_account_index(group: &[usize], accounts: &[CodebuddyAccount]) -> usize {
    group
        .iter()
        .copied()
        .max_by(|l, r| {
            accounts[*l]
                .last_used
                .cmp(&accounts[*r].last_used)
                .then_with(|| accounts[*r].created_at.cmp(&accounts[*l].created_at))
        })
        .unwrap_or(group[0])
}

fn normalize_account_index(index: &mut CodebuddyAccountIndex) -> Vec<CodebuddyAccount> {
    let mut loaded = Vec::new();
    let mut seen = HashSet::new();
    for summary in &index.accounts {
        if !seen.insert(summary.id.clone()) {
            continue;
        }
        if let Some(account) = load_account(&summary.id) {
            loaded.push(account);
        }
    }
    if loaded.len() <= 1 {
        index.accounts = loaded.iter().map(|a| a.summary()).collect();
        return loaded;
    }

    let mut parents: Vec<usize> = (0..loaded.len()).collect();
    fn find(parents: &mut [usize], idx: usize) -> usize {
        let p = parents[idx];
        if p == idx {
            return idx;
        }
        let root = find(parents, p);
        parents[idx] = root;
        root
    }
    fn union(parents: &mut [usize], l: usize, r: usize) {
        let lr = find(parents, l);
        let rr = find(parents, r);
        if lr != rr {
            parents[rr] = lr;
        }
    }

    let total = loaded.len();
    for l in 0..total {
        for r in (l + 1)..total {
            if accounts_are_duplicates(&loaded[l], &loaded[r]) {
                union(&mut parents, l, r);
            }
        }
    }

    let mut grouped: HashMap<usize, Vec<usize>> = HashMap::new();
    for idx in 0..total {
        let root = find(&mut parents, idx);
        grouped.entry(root).or_default().push(idx);
    }

    let mut processed = HashSet::new();
    let mut normalized = Vec::new();
    let mut removed_ids = Vec::new();
    for idx in 0..total {
        let root = find(&mut parents, idx);
        if !processed.insert(root) {
            continue;
        }
        let Some(group) = grouped.get(&root) else {
            continue;
        };
        if group.len() == 1 {
            normalized.push(loaded[group[0]].clone());
            continue;
        }
        let primary_idx = choose_primary_account_index(group, &loaded);
        let mut primary = loaded[primary_idx].clone();
        for member in group {
            if *member == primary_idx {
                continue;
            }
            merge_duplicate_account(&mut primary, &loaded[*member]);
            removed_ids.push(loaded[*member].id.clone());
        }
        normalized.push(primary);
    }

    if !removed_ids.is_empty() {
        for acc in &normalized {
            let _ = save_account_file(acc);
        }
        for id in &removed_ids {
            let _ = delete_account_file(id);
        }
        logger::log_warn(&format!(
            "[CodeBuddy Account] 检测到重复账号并已合并: removed_ids={}",
            removed_ids.join(",")
        ));
    }

    index.accounts = normalized.iter().map(|a| a.summary()).collect();
    normalized
}

pub fn list_accounts() -> Vec<CodebuddyAccount> {
    let mut index = load_account_index();
    let accounts = normalize_account_index(&mut index);
    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!("[CodeBuddy Account] 保存账号索引失败: {}", err));
    }
    accounts
}

pub fn list_accounts_checked() -> Result<Vec<CodebuddyAccount>, String> {
    let mut index = load_account_index_checked()?;
    let accounts = normalize_account_index(&mut index);
    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!("[CodeBuddy Account] 保存账号索引失败: {}", err));
    }
    Ok(accounts)
}

fn apply_payload(account: &mut CodebuddyAccount, payload: CodebuddyOAuthCompletePayload) {
    let incoming_email = payload.email.trim().to_string();
    if !incoming_email.is_empty() {
        account.email = incoming_email;
    }
    account.uid = payload.uid;
    account.nickname = payload.nickname;
    account.enterprise_id = payload.enterprise_id;
    account.enterprise_name = payload.enterprise_name;
    account.access_token = payload.access_token;
    account.refresh_token = payload.refresh_token;
    account.token_type = payload.token_type;
    account.expires_at = payload.expires_at;
    account.domain = payload.domain;
    if payload.plan_type.is_some() {
        account.plan_type = payload.plan_type;
    }
    if payload.dosage_notify_code.is_some() {
        account.dosage_notify_code = payload.dosage_notify_code;
    }
    if payload.dosage_notify_zh.is_some() {
        account.dosage_notify_zh = payload.dosage_notify_zh;
    }
    if payload.dosage_notify_en.is_some() {
        account.dosage_notify_en = payload.dosage_notify_en;
    }
    if payload.payment_type.is_some() {
        account.payment_type = payload.payment_type;
    }
    if payload.quota_raw.is_some() {
        account.quota_raw = payload.quota_raw;
    }
    account.auth_raw = payload.auth_raw;
    if payload.profile_raw.is_some() {
        account.profile_raw = payload.profile_raw;
    }
    if payload.usage_raw.is_some() {
        account.usage_raw = payload.usage_raw;
    }
    account.status = payload.status;
    account.status_reason = payload.status_reason;
    account.last_used = now_ts();
}

pub fn upsert_account(payload: CodebuddyOAuthCompletePayload) -> Result<CodebuddyAccount, String> {
    let _lock = CODEBUDDY_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 CodeBuddy 账号锁失败".to_string())?;
    let now = now_ts();
    let mut index = load_account_index();

    let incoming_uid = normalize_identity(payload.uid.as_deref());
    let incoming_email = normalize_email_identity(Some(payload.email.as_str()));

    let identity_seed = incoming_uid
        .clone()
        .or_else(|| incoming_email.clone())
        .unwrap_or_else(|| "codebuddy_user".to_string())
        .to_lowercase();
    let generated_id = format!("codebuddy_{:x}", md5::compute(identity_seed.as_bytes()));

    let account_id = index
        .accounts
        .iter()
        .filter_map(|item| load_account(&item.id))
        .find(|account| {
            let existing_uid = normalize_identity(account.uid.as_deref());
            let existing_email = normalize_email_identity(Some(account.email.as_str()));
            account_matches_payload_identity(
                existing_uid.as_ref(),
                existing_email.as_ref(),
                incoming_uid.as_ref(),
                incoming_email.as_ref(),
            )
        })
        .map(|a| a.id)
        .unwrap_or(generated_id);

    let existing = load_account(&account_id);
    let tags = existing.as_ref().and_then(|a| a.tags.clone());
    let created_at = existing.as_ref().map(|a| a.created_at).unwrap_or(now);

    let mut account = existing.unwrap_or(CodebuddyAccount {
        id: account_id.clone(),
        email: payload.email.clone(),
        uid: payload.uid.clone(),
        nickname: payload.nickname.clone(),
        enterprise_id: payload.enterprise_id.clone(),
        enterprise_name: payload.enterprise_name.clone(),
        tags,
        access_token: payload.access_token.clone(),
        refresh_token: payload.refresh_token.clone(),
        token_type: payload.token_type.clone(),
        expires_at: payload.expires_at,
        domain: payload.domain.clone(),
        plan_type: payload.plan_type.clone(),
        dosage_notify_code: payload.dosage_notify_code.clone(),
        dosage_notify_zh: payload.dosage_notify_zh.clone(),
        dosage_notify_en: payload.dosage_notify_en.clone(),
        payment_type: payload.payment_type.clone(),
        quota_raw: payload.quota_raw.clone(),
        auth_raw: payload.auth_raw.clone(),
        profile_raw: payload.profile_raw.clone(),
        usage_raw: payload.usage_raw.clone(),
        status: payload.status.clone(),
        status_reason: payload.status_reason.clone(),
        last_checkin_time: None,
        checkin_streak: 0,
        checkin_rewards: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        created_at,
        last_used: now,
    });

    apply_payload(&mut account, payload);
    account.id = account_id;
    account.created_at = created_at;
    account.last_used = now;

    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;

    logger::log_info(&format!(
        "CodeBuddy 账号已保存: id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

async fn refresh_account_token_once(account_id: &str) -> Result<CodebuddyAccount, String> {
    let started_at = Instant::now();
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[CodeBuddy Refresh] 开始刷新账号: id={}, email={}",
        account.id, account.email
    ));

    let (payload, quota_refresh_error) =
        codebuddy_oauth::refresh_payload_for_account(&account).await?;
    let usage_refreshed = quota_refresh_error.is_none()
        && (payload.quota_raw.is_some() || payload.usage_raw.is_some());
    let tags = account.tags.clone();
    let created_at = account.created_at;
    apply_payload(&mut account, payload);
    if let Some(err) = quota_refresh_error {
        account.quota_query_last_error = Some(err);
        account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    } else {
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
    }
    account.tags = tags;
    account.created_at = created_at;
    let refreshed_at = now_ts();
    if usage_refreshed {
        account.usage_updated_at = Some(refreshed_at);
    }
    account.last_used = refreshed_at;

    let updated = account.clone();
    upsert_account_record(account)?;
    logger::log_info(&format!(
        "[CodeBuddy Refresh] 刷新完成: id={}, email={}, elapsed={}ms",
        updated.id,
        updated.email,
        started_at.elapsed().as_millis()
    ));
    Ok(updated)
}

pub async fn refresh_account_token(account_id: &str) -> Result<CodebuddyAccount, String> {
    crate::modules::refresh_retry::retry_once_with_delay(
        "CodeBuddy Refresh",
        account_id,
        || async { refresh_account_token_once(account_id).await },
    )
    .await
}

pub async fn refresh_all_tokens() -> Result<Vec<(String, Result<CodebuddyAccount, String>)>, String>
{
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    const MAX_CONCURRENT: usize = 5;
    let accounts = list_accounts();
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
                    .map_err(|e| format!("获取并发许可失败: {}", e))?;
                let result = refresh_account_token(&id).await;
                Ok::<(String, Result<CodebuddyAccount, String>), String>((id, result))
            }
        })
        .collect();

    let mut results = Vec::with_capacity(tasks.len());
    for task in join_all(tasks).await {
        match task {
            Ok(item) => results.push(item),
            Err(err) => return Err(err),
        }
    }
    Ok(results)
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = CODEBUDDY_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 CodeBuddy 账号锁失败".to_string())?;
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

pub fn update_account_tags(
    account_id: &str,
    tags: Vec<String>,
) -> Result<CodebuddyAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    account.tags = Some(tags);
    account.last_used = now_ts();
    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<CodebuddyAccount>, String> {
    if let Ok(account) = serde_json::from_str::<CodebuddyAccount>(json_content) {
        let saved = upsert_account_record(account)?;
        return Ok(vec![saved]);
    }

    if let Ok(accounts) = serde_json::from_str::<Vec<CodebuddyAccount>>(json_content) {
        let mut result = Vec::new();
        for account in accounts {
            let saved = upsert_account_record(account)?;
            result.push(saved);
        }
        return Ok(result);
    }

    if let Ok(value) = serde_json::from_str::<Value>(json_content) {
        return import_from_json_value(value);
    }

    Err("无法解析 CodeBuddy JSON 导入内容".to_string())
}

fn import_from_json_value(value: Value) -> Result<Vec<CodebuddyAccount>, String> {
    match value {
        Value::Array(items) => {
            if items.is_empty() {
                return Err("导入数组为空".to_string());
            }
            let mut results = Vec::new();
            for (idx, item) in items.into_iter().enumerate() {
                let payload = payload_from_import_value(item)
                    .map_err(|e| format!("第 {} 条记录解析失败: {}", idx + 1, e))?;
                let account = upsert_account_record_from_payload(payload)?;
                results.push(account);
            }
            Ok(results)
        }
        Value::Object(mut obj) => {
            let object_value = Value::Object(obj.clone());
            if let Ok(payload) = payload_from_import_value(object_value) {
                let account = upsert_account_record_from_payload(payload)?;
                return Ok(vec![account]);
            }

            if let Some(accounts) = obj
                .remove("accounts")
                .or_else(|| obj.remove("items"))
                .and_then(|raw| raw.as_array().cloned())
            {
                if accounts.is_empty() {
                    return Err("导入数组为空".to_string());
                }
                let mut results = Vec::new();
                for (idx, item) in accounts.into_iter().enumerate() {
                    let payload = payload_from_import_value(item)
                        .map_err(|e| format!("第 {} 条记录解析失败: {}", idx + 1, e))?;
                    let account = upsert_account_record_from_payload(payload)?;
                    results.push(account);
                }
                return Ok(results);
            }

            Err("无法解析 CodeBuddy 导入对象".to_string())
        }
        _ => Err("CodeBuddy 导入 JSON 必须是对象或数组".to_string()),
    }
}

fn upsert_account_record_from_payload(
    payload: CodebuddyOAuthCompletePayload,
) -> Result<CodebuddyAccount, String> {
    // Release lock pattern: upsert_account already takes lock internally
    drop(
        CODEBUDDY_ACCOUNT_INDEX_LOCK
            .lock()
            .map_err(|_| "获取锁失败".to_string())?,
    );
    let now = now_ts();
    let incoming_uid = normalize_identity(payload.uid.as_deref());
    let incoming_email = normalize_email_identity(Some(payload.email.as_str()));
    let identity_seed = incoming_uid
        .or_else(|| incoming_email)
        .unwrap_or_else(|| "codebuddy_user".to_string());
    let generated_id = format!("codebuddy_{:x}", md5::compute(identity_seed.as_bytes()));

    let account = CodebuddyAccount {
        id: generated_id,
        email: payload.email,
        uid: payload.uid,
        nickname: payload.nickname,
        enterprise_id: payload.enterprise_id,
        enterprise_name: payload.enterprise_name,
        tags: None,
        access_token: payload.access_token,
        refresh_token: payload.refresh_token,
        token_type: payload.token_type,
        expires_at: payload.expires_at,
        domain: payload.domain,
        plan_type: payload.plan_type,
        dosage_notify_code: payload.dosage_notify_code,
        dosage_notify_zh: payload.dosage_notify_zh,
        dosage_notify_en: payload.dosage_notify_en,
        payment_type: payload.payment_type,
        quota_raw: payload.quota_raw,
        auth_raw: payload.auth_raw,
        profile_raw: payload.profile_raw,
        usage_raw: payload.usage_raw,
        status: payload.status,
        status_reason: payload.status_reason,
        last_checkin_time: None,
        checkin_streak: 0,
        checkin_rewards: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        created_at: now,
        last_used: now,
    };
    upsert_account_record(account)
}

fn payload_from_import_value(raw: Value) -> Result<CodebuddyOAuthCompletePayload, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| "导入条目必须是对象".to_string())?;

    let access_token = obj
        .get("access_token")
        .or_else(|| obj.get("accessToken"))
        .or_else(|| obj.get("token"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if access_token.is_empty() {
        return Err("缺少 access_token".to_string());
    }

    let email = obj
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let uid = obj
        .get("uid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let nickname = obj
        .get("nickname")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let enterprise_id = obj
        .get("enterprise_id")
        .or_else(|| obj.get("enterpriseId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let enterprise_name = obj
        .get("enterprise_name")
        .or_else(|| obj.get("enterpriseName"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let refresh_token = obj
        .get("refresh_token")
        .or_else(|| obj.get("refreshToken"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let domain = obj
        .get("domain")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(CodebuddyOAuthCompletePayload {
        email,
        uid,
        nickname,
        enterprise_id,
        enterprise_name,
        access_token,
        refresh_token,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        domain,
        plan_type: None,
        dosage_notify_code: None,
        dosage_notify_zh: None,
        dosage_notify_en: None,
        payment_type: None,
        quota_raw: None,
        auth_raw: obj.get("auth_raw").cloned(),
        profile_raw: obj.get("profile_raw").cloned(),
        usage_raw: obj.get("usage_raw").cloned(),
        status: Some("normal".to_string()),
        status_reason: None,
        last_checkin_time: None,
        checkin_streak: 0,
        checkin_rewards: None,
    })
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<CodebuddyAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("导出失败: {}", e))
}

pub fn get_default_codebuddy_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        Some(home.join("Library/Application Support/CodeBuddy"))
    }

    #[cfg(target_os = "windows")]
    {
        dirs::data_dir().map(|d| d.join("CodeBuddy"))
    }

    #[cfg(target_os = "linux")]
    {
        dirs::config_dir().map(|d| d.join("CodeBuddy"))
    }
}

pub fn get_default_codebuddy_state_db_path() -> Option<PathBuf> {
    get_default_codebuddy_data_dir()
        .map(|d| d.join("User").join("globalStorage").join("state.vscdb"))
}

fn parse_local_access_token(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Array(arr) => arr.iter().find_map(parse_local_access_token),
        Value::Object(obj) => {
            let direct = obj
                .get("token")
                .or_else(|| obj.get("access_token"))
                .or_else(|| obj.get("accessToken"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if let Some(token) = direct {
                return Some(token);
            }

            let auth_token = obj
                .get("auth")
                .and_then(|v| v.as_object())
                .and_then(|auth| {
                    auth.get("accessToken")
                        .or_else(|| auth.get("access_token"))
                        .and_then(|v| v.as_str())
                })
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if let Some(token) = auth_token {
                return Some(token);
            }

            let encoded = obj
                .get("session")
                .or_else(|| obj.get("data"))
                .and_then(parse_local_access_token);
            if encoded.is_some() {
                return encoded;
            }

            None
        }
        _ => None,
    }
}

fn normalize_local_codebuddy_token(token: &str) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((_, suffix)) = trimmed.split_once('+') {
        let suffix = suffix.trim();
        if !suffix.is_empty() {
            return Some(suffix.to_string());
        }
    }
    Some(trimmed.to_string())
}

fn extract_local_codebuddy_token_parts(token: &str) -> Option<(Option<String>, String)> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((prefix, suffix)) = trimmed.split_once('+') {
        let uid = prefix.trim();
        let token_value = suffix.trim();
        if token_value.is_empty() {
            return None;
        }
        let uid_opt = if uid.is_empty() {
            None
        } else {
            Some(uid.to_string())
        };
        return Some((uid_opt, token_value.to_string()));
    }
    Some((None, trimmed.to_string()))
}

fn json_object_string_field(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        let value = obj
            .get(*key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        if let Some(found) = value {
            return Some(found.to_string());
        }
    }
    None
}

fn json_object_i64_field(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<i64> {
    for key in keys {
        let Some(raw) = obj.get(*key) else {
            continue;
        };
        if let Some(v) = raw.as_i64() {
            return Some(v);
        }
        if let Some(v) = raw.as_u64() {
            if let Ok(parsed) = i64::try_from(v) {
                return Some(parsed);
            }
        }
        if let Some(v) = raw.as_str() {
            if let Ok(parsed) = v.trim().parse::<i64>() {
                return Some(parsed);
            }
        }
    }
    None
}

fn build_local_import_payload(
    access_token: String,
    parsed_json: Option<Value>,
    uid_from_token: Option<String>,
) -> CodebuddyOAuthCompletePayload {
    let root_obj = parsed_json.as_ref().and_then(|v| v.as_object());
    let account_obj = root_obj.and_then(|obj| obj.get("account").and_then(|v| v.as_object()));
    let auth_obj = root_obj.and_then(|obj| obj.get("auth").and_then(|v| v.as_object()));

    let uid = root_obj
        .and_then(|obj| json_object_string_field(obj, &["uid"]))
        .or_else(|| account_obj.and_then(|obj| json_object_string_field(obj, &["uid", "id"])))
        .or(uid_from_token);

    let nickname = root_obj
        .and_then(|obj| json_object_string_field(obj, &["nickname", "name"]))
        .or_else(|| {
            account_obj.and_then(|obj| json_object_string_field(obj, &["nickname", "label"]))
        });

    let email = root_obj
        .and_then(|obj| json_object_string_field(obj, &["email"]))
        .or_else(|| account_obj.and_then(|obj| json_object_string_field(obj, &["email"])))
        .or_else(|| auth_obj.and_then(|obj| json_object_string_field(obj, &["email"])))
        .or_else(|| nickname.clone())
        .or_else(|| uid.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let enterprise_id = root_obj
        .and_then(|obj| json_object_string_field(obj, &["enterpriseId", "enterprise_id"]))
        .or_else(|| {
            account_obj
                .and_then(|obj| json_object_string_field(obj, &["enterpriseId", "enterprise_id"]))
        });
    let enterprise_name = root_obj
        .and_then(|obj| json_object_string_field(obj, &["enterpriseName", "enterprise_name"]))
        .or_else(|| {
            account_obj.and_then(|obj| {
                json_object_string_field(obj, &["enterpriseName", "enterprise_name"])
            })
        });

    let refresh_token = root_obj
        .and_then(|obj| json_object_string_field(obj, &["refreshToken", "refresh_token"]))
        .or_else(|| {
            auth_obj
                .and_then(|obj| json_object_string_field(obj, &["refreshToken", "refresh_token"]))
        });
    let token_type = root_obj
        .and_then(|obj| json_object_string_field(obj, &["tokenType", "token_type"]))
        .or_else(|| {
            auth_obj.and_then(|obj| json_object_string_field(obj, &["tokenType", "token_type"]))
        })
        .or_else(|| Some("Bearer".to_string()));
    let domain = root_obj
        .and_then(|obj| json_object_string_field(obj, &["domain"]))
        .or_else(|| auth_obj.and_then(|obj| json_object_string_field(obj, &["domain"])));
    let expires_at = root_obj
        .and_then(|obj| json_object_i64_field(obj, &["expiresAt", "expires_at"]))
        .or_else(|| {
            auth_obj.and_then(|obj| json_object_i64_field(obj, &["expiresAt", "expires_at"]))
        });

    CodebuddyOAuthCompletePayload {
        email,
        uid,
        nickname,
        enterprise_id,
        enterprise_name,
        access_token,
        refresh_token,
        token_type,
        expires_at,
        domain,
        plan_type: None,
        dosage_notify_code: None,
        dosage_notify_zh: None,
        dosage_notify_en: None,
        payment_type: None,
        quota_raw: None,
        auth_raw: parsed_json.clone(),
        profile_raw: account_obj.map(|obj| Value::Object(obj.clone())),
        usage_raw: None,
        status: Some("normal".to_string()),
        status_reason: None,
        last_checkin_time: None,
        checkin_streak: 0,
        checkin_rewards: None,
    }
}

pub fn import_payload_from_local() -> Result<Option<CodebuddyOAuthCompletePayload>, String> {
    let data_root = match get_default_codebuddy_data_dir() {
        Some(path) => path,
        None => return Ok(None),
    };

    let state_db = match get_default_codebuddy_state_db_path() {
        Some(path) => path,
        None => return Ok(None),
    };
    if !state_db.exists() {
        return Ok(None);
    }

    let raw_secret = crate::modules::vscode_inject::read_codebuddy_secret_storage_value(
        CODEBUDDY_SECRET_EXTENSION_ID,
        CODEBUDDY_SECRET_KEY,
        Some(data_root.to_string_lossy().as_ref()),
    )?;

    let Some(secret) = raw_secret else {
        return Ok(None);
    };

    let parsed_json = serde_json::from_str::<Value>(&secret).ok();
    let token_candidate = parsed_json
        .as_ref()
        .and_then(parse_local_access_token)
        .or_else(|| {
            let raw = secret.trim();
            if raw.is_empty() {
                None
            } else {
                Some(raw.to_string())
            }
        });

    let Some(raw_token) = token_candidate else {
        return Err("本地 CodeBuddy 登录信息解析失败：未找到 access token".to_string());
    };

    let Some((uid_from_token, normalized_token)) = extract_local_codebuddy_token_parts(&raw_token)
    else {
        return Err("本地 CodeBuddy 登录信息解析失败：access token 无效".to_string());
    };
    let Some(access_token) = normalize_local_codebuddy_token(&normalized_token) else {
        return Err("本地 CodeBuddy 登录信息解析失败：access token 为空".to_string());
    };

    let payload = build_local_import_payload(access_token, parsed_json, uid_from_token);
    Ok(Some(payload))
}

pub(crate) fn resolve_current_account_id(accounts: &[CodebuddyAccount]) -> Option<String> {
    let payload = import_payload_from_local().ok()??;
    let incoming_uid = normalize_identity(payload.uid.as_deref());
    let incoming_email = normalize_email_identity(Some(payload.email.as_str()));

    accounts
        .iter()
        .find(|account| {
            let existing_uid = normalize_identity(account.uid.as_deref());
            let existing_email = normalize_email_identity(Some(account.email.as_str()));
            account_matches_payload_identity(
                existing_uid.as_ref(),
                existing_email.as_ref(),
                incoming_uid.as_ref(),
                incoming_email.as_ref(),
            )
        })
        .map(|account| account.id.clone())
}

pub fn run_quota_alert_if_needed() -> Result<(), String> {
    let config = crate::modules::config::get_user_config();
    if !config.codebuddy_quota_alert_enabled {
        return Ok(());
    }
    let threshold = config.codebuddy_quota_alert_threshold;
    if threshold <= 0 {
        return Ok(());
    }

    let accounts = list_accounts();
    let now = now_ts();
    let mut last_sent = CODEBUDDY_QUOTA_ALERT_LAST_SENT
        .lock()
        .map_err(|_| "获取预警锁失败".to_string())?;

    for account in &accounts {
        let cooldown_key = account.id.clone();
        if let Some(last) = last_sent.get(&cooldown_key) {
            if now - last < CODEBUDDY_QUOTA_ALERT_COOLDOWN_SECONDS {
                continue;
            }
        }

        let should_alert = match account.dosage_notify_code.as_deref() {
            Some(code) if code != "USAGE_NORMAL" && !code.is_empty() => true,
            _ => false,
        };

        if should_alert {
            last_sent.insert(cooldown_key, now);
            if let Some(app) = crate::get_app_handle() {
                let msg = account
                    .dosage_notify_zh
                    .as_deref()
                    .or(account.dosage_notify_en.as_deref())
                    .unwrap_or("配额即将耗尽");

                let _ = app.emit(
                    "quota:alert",
                    serde_json::json!({
                        "platform": "codebuddy",
                        "accountId": account.id,
                        "email": account.email,
                        "message": msg,
                    }),
                );
            }
        }
    }

    Ok(())
}
