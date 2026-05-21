use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use crate::models::kiro::{KiroAccount, KiroAccountIndex, KiroOAuthCompletePayload};
use crate::modules::{account, kiro_oauth, logger};

const ACCOUNTS_INDEX_FILE: &str = "kiro_accounts.json";
const ACCOUNTS_DIR: &str = "kiro_accounts";
const LOCAL_AUTH_TOKEN_FILE_NAME: &str = "kiro-auth-token.json";
const LOCAL_USAGE_DB_KEY: &str = "kiro.kiroAgent";
const KIRO_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 10 * 60;

lazy_static::lazy_static! {
    static ref KIRO_ACCOUNT_INDEX_LOCK: Mutex<()> = Mutex::new(());
    static ref KIRO_QUOTA_ALERT_LAST_SENT: Mutex<HashMap<String, i64>> = Mutex::new(HashMap::new());
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn normalize_status_value(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_ascii_lowercase())
        }
    })
}

fn is_banned_status(value: Option<&str>) -> bool {
    matches!(
        normalize_status_value(value).as_deref(),
        Some("banned") | Some("ban") | Some("forbidden")
    )
}

fn is_banned_reason(value: Option<&str>) -> bool {
    let Some(reason) = normalize_status_value(value) else {
        return false;
    };
    reason.contains("banned")
        || reason.contains("forbidden")
        || reason.contains("suspended")
        || reason.contains("disabled")
        || reason.contains("封禁")
        || reason.contains("禁用")
}

pub(crate) fn is_banned_account(account: &KiroAccount) -> bool {
    is_banned_status(account.status.as_deref())
        || is_banned_reason(account.status_reason.as_deref())
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Kiro 账号目录失败: {}", e))?;
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

    // 防止目录穿越或创建子目录，账号文件必须落在固定目录下。
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

pub fn load_account(account_id: &str) -> Option<KiroAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_account_file(account: &KiroAccount) -> Result<(), String> {
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

fn load_account_index() -> KiroAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return KiroAccountIndex::new(),
    };

    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(KiroAccountIndex::new);
    }

    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空").unwrap_or_else(KiroAccountIndex::new)
        }
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            KiroAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(KiroAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Kiro Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(KiroAccountIndex::new)
            }
        },
        Err(_) => KiroAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<KiroAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(KiroAccountIndex::new());
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
        return Ok(KiroAccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<KiroAccountIndex>(
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

fn save_account_index(index: &KiroAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<KiroAccountIndex> {
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

    let mut index = KiroAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Kiro Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Kiro Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_warn(&format!(
        "[Kiro Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut KiroAccountIndex, account: &KiroAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: KiroAccount) -> Result<KiroAccount, String> {
    let _lock = KIRO_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Kiro 账号锁失败".to_string())?;
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

fn normalize_user_identity(value: Option<&str>) -> Option<String> {
    normalize_identity(value).and_then(|identity| {
        if identity.starts_with("arn:") {
            None
        } else {
            Some(identity)
        }
    })
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

fn normalize_token_identity(value: Option<&str>) -> Option<String> {
    normalize_non_empty(value)
}

fn account_matches_payload_identity(
    existing_user: Option<&String>,
    existing_email: Option<&String>,
    existing_refresh_token: Option<&String>,
    incoming_user_id: Option<&String>,
    incoming_email: Option<&String>,
    incoming_refresh_token: Option<&String>,
) -> bool {
    if let (Some(existing), Some(incoming)) = (existing_user, incoming_user_id) {
        if existing == incoming {
            return true;
        }
    }

    if let (Some(existing), Some(incoming)) = (existing_email, incoming_email) {
        if existing == incoming {
            // 双方都有 user_id 且不同时，不按邮箱合并（同邮箱不同账号，如 GitHub/Google 各自登录）
            if let (Some(eu), Some(iu)) = (existing_user, incoming_user_id) {
                if eu != iu {
                    return false;
                }
            }
            // 已有账号有 user_id 而 payload 无时，不按邮箱合并（按唯一标识区分）
            if existing_user.is_some() && incoming_user_id.is_none() {
                return false;
            }
            return true;
        }
    }

    if let (Some(existing), Some(incoming)) = (existing_refresh_token, incoming_refresh_token) {
        if existing == incoming {
            return true;
        }
    }

    false
}

fn accounts_are_duplicates(left: &KiroAccount, right: &KiroAccount) -> bool {
    let left_user = normalize_user_identity(left.user_id.as_deref());
    let right_user = normalize_user_identity(right.user_id.as_deref());
    let left_email = normalize_email_identity(Some(left.email.as_str()));
    let right_email = normalize_email_identity(Some(right.email.as_str()));
    let left_refresh = normalize_token_identity(left.refresh_token.as_deref());
    let right_refresh = normalize_token_identity(right.refresh_token.as_deref());

    let user_conflict = matches!(
        (left_user.as_ref(), right_user.as_ref()),
        (Some(left), Some(right)) if left != right
    );
    let email_conflict = matches!(
        (left_email.as_ref(), right_email.as_ref()),
        (Some(left), Some(right)) if left != right
    );
    if user_conflict || email_conflict {
        return false;
    }

    let user_match = matches!(
        (left_user.as_ref(), right_user.as_ref()),
        (Some(left), Some(right)) if left == right
    );
    let email_same = matches!(
        (left_email.as_ref(), right_email.as_ref()),
        (Some(left), Some(right)) if left == right
    );
    // 一方有 user_id 另一方无时，不单凭邮箱合并（按唯一标识区分）
    let email_match = email_same
        && !((left_user.is_some() && right_user.is_none())
            || (left_user.is_none() && right_user.is_some()));
    let refresh_match = matches!(
        (left_refresh.as_ref(), right_refresh.as_ref()),
        (Some(left), Some(right)) if left == right
    );

    user_match || email_match || refresh_match
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

fn fill_if_empty_string(target: &mut String, source: &str) {
    if target.trim().is_empty() {
        let incoming = source.trim();
        if !incoming.is_empty() {
            *target = incoming.to_string();
        }
    }
}

fn fill_if_none<T: Clone>(target: &mut Option<T>, source: &Option<T>) {
    if target.is_none() {
        *target = source.clone();
    }
}

fn merge_duplicate_account(primary: &mut KiroAccount, duplicate: &KiroAccount) {
    fill_if_empty_string(&mut primary.email, duplicate.email.as_str());
    fill_if_empty_string(&mut primary.access_token, duplicate.access_token.as_str());

    fill_if_none(&mut primary.user_id, &duplicate.user_id);
    fill_if_none(&mut primary.login_provider, &duplicate.login_provider);
    fill_if_none(&mut primary.refresh_token, &duplicate.refresh_token);
    fill_if_none(&mut primary.token_type, &duplicate.token_type);
    fill_if_none(&mut primary.expires_at, &duplicate.expires_at);
    fill_if_none(&mut primary.idc_region, &duplicate.idc_region);
    fill_if_none(&mut primary.issuer_url, &duplicate.issuer_url);
    fill_if_none(&mut primary.client_id, &duplicate.client_id);
    fill_if_none(&mut primary.scopes, &duplicate.scopes);
    fill_if_none(&mut primary.login_hint, &duplicate.login_hint);
    fill_if_none(&mut primary.plan_name, &duplicate.plan_name);
    fill_if_none(&mut primary.plan_tier, &duplicate.plan_tier);
    fill_if_none(&mut primary.credits_total, &duplicate.credits_total);
    fill_if_none(&mut primary.credits_used, &duplicate.credits_used);
    fill_if_none(&mut primary.bonus_total, &duplicate.bonus_total);
    fill_if_none(&mut primary.bonus_used, &duplicate.bonus_used);
    fill_if_none(&mut primary.usage_reset_at, &duplicate.usage_reset_at);
    fill_if_none(&mut primary.bonus_expire_days, &duplicate.bonus_expire_days);
    fill_if_none(
        &mut primary.kiro_auth_token_raw,
        &duplicate.kiro_auth_token_raw,
    );
    fill_if_none(&mut primary.kiro_profile_raw, &duplicate.kiro_profile_raw);
    fill_if_none(&mut primary.kiro_usage_raw, &duplicate.kiro_usage_raw);
    fill_if_none(&mut primary.status, &duplicate.status);
    fill_if_none(&mut primary.status_reason, &duplicate.status_reason);

    primary.tags = merge_string_list(primary.tags.clone(), duplicate.tags.clone());
    primary.created_at = primary.created_at.min(duplicate.created_at);
    primary.last_used = primary.last_used.max(duplicate.last_used);
}

fn choose_primary_account_index(
    group: &[usize],
    accounts: &[KiroAccount],
    preferred_bound_id: Option<&str>,
) -> usize {
    if let Some(bound_id) = preferred_bound_id.and_then(|value| normalize_non_empty(Some(value))) {
        if let Some(found) = group
            .iter()
            .copied()
            .find(|idx| accounts[*idx].id == bound_id)
        {
            return found;
        }
    }

    group
        .iter()
        .copied()
        .max_by(|left, right| {
            let left_account = &accounts[*left];
            let right_account = &accounts[*right];
            left_account
                .last_used
                .cmp(&right_account.last_used)
                .then_with(|| right_account.created_at.cmp(&left_account.created_at))
        })
        .unwrap_or(group[0])
}

fn normalize_account_index(index: &mut KiroAccountIndex) -> Vec<KiroAccount> {
    let mut loaded_accounts = Vec::new();
    let mut seen_summary_ids = HashSet::new();

    for summary in &index.accounts {
        if !seen_summary_ids.insert(summary.id.clone()) {
            continue;
        }
        if let Some(account) = load_account(&summary.id) {
            loaded_accounts.push(account);
        }
    }

    if loaded_accounts.len() <= 1 {
        index.accounts = loaded_accounts
            .iter()
            .map(|account| account.summary())
            .collect();
        return loaded_accounts;
    }

    let preferred_bound_id = crate::modules::kiro_instance::load_default_settings()
        .ok()
        .and_then(|settings| settings.bind_account_id);
    let mut parents: Vec<usize> = (0..loaded_accounts.len()).collect();

    fn find(parents: &mut [usize], idx: usize) -> usize {
        let parent = parents[idx];
        if parent == idx {
            return idx;
        }
        let root = find(parents, parent);
        parents[idx] = root;
        root
    }

    fn union(parents: &mut [usize], left: usize, right: usize) {
        let left_root = find(parents, left);
        let right_root = find(parents, right);
        if left_root != right_root {
            parents[right_root] = left_root;
        }
    }

    let total = loaded_accounts.len();
    for left in 0..total {
        for right in (left + 1)..total {
            if accounts_are_duplicates(&loaded_accounts[left], &loaded_accounts[right]) {
                union(&mut parents, left, right);
            }
        }
    }

    let mut grouped: HashMap<usize, Vec<usize>> = HashMap::new();
    for idx in 0..total {
        let root = find(&mut parents, idx);
        grouped.entry(root).or_default().push(idx);
    }

    let mut processed_roots = HashSet::new();
    let mut normalized_accounts = Vec::new();
    let mut removed_ids = Vec::new();
    for idx in 0..total {
        let root = find(&mut parents, idx);
        if !processed_roots.insert(root) {
            continue;
        }
        let Some(group) = grouped.get(&root) else {
            continue;
        };

        if group.len() == 1 {
            normalized_accounts.push(loaded_accounts[group[0]].clone());
            continue;
        }

        let primary_idx =
            choose_primary_account_index(group, &loaded_accounts, preferred_bound_id.as_deref());
        let mut primary = loaded_accounts[primary_idx].clone();
        for member in group {
            if *member == primary_idx {
                continue;
            }
            merge_duplicate_account(&mut primary, &loaded_accounts[*member]);
            removed_ids.push(loaded_accounts[*member].id.clone());
        }

        normalized_accounts.push(primary);
    }

    if !removed_ids.is_empty() {
        for account in &normalized_accounts {
            if let Err(err) = save_account_file(account) {
                logger::log_warn(&format!(
                    "[Kiro Account] 保存去重账号失败: id={}, error={}",
                    account.id, err
                ));
            }
        }
        for account_id in &removed_ids {
            if let Err(err) = delete_account_file(account_id) {
                logger::log_warn(&format!(
                    "[Kiro Account] 删除重复账号文件失败: id={}, error={}",
                    account_id, err
                ));
            }
        }
        logger::log_warn(&format!(
            "[Kiro Account] 检测到重复账号并已合并: removed_ids={}",
            removed_ids.join(",")
        ));
    }

    index.accounts = normalized_accounts
        .iter()
        .map(|account| account.summary())
        .collect();
    normalized_accounts
}

pub fn list_accounts() -> Vec<KiroAccount> {
    let mut index = load_account_index();
    let had_index_accounts = !index.accounts.is_empty();
    let accounts = normalize_account_index(&mut index);
    if had_index_accounts && accounts.is_empty() {
        logger::log_warn(
            "[Kiro Account] 账号索引中存在账号，但详情文件均无法读取，已跳过空索引写回",
        );
        return accounts;
    }
    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!("[Kiro Account] 保存账号索引失败: {}", err));
    }
    accounts
}

pub fn list_accounts_checked() -> Result<Vec<KiroAccount>, String> {
    let mut index = load_account_index_checked()?;
    let had_index_accounts = !index.accounts.is_empty();
    let accounts = normalize_account_index(&mut index);
    if had_index_accounts && accounts.is_empty() {
        return Err("Kiro 账号索引中存在账号，但详情文件均无法读取；已保留前端缓存，请从账号备份或本地账号文件恢复。".to_string());
    }
    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!("[Kiro Account] 保存账号索引失败: {}", err));
    }
    Ok(accounts)
}

fn apply_payload(account: &mut KiroAccount, payload: KiroOAuthCompletePayload) {
    let incoming_email = payload.email.trim().to_string();
    if !incoming_email.is_empty() {
        account.email = incoming_email;
    } else if !account.email.contains('@') {
        account.email.clear();
    }
    account.user_id = payload.user_id;
    account.login_provider = payload.login_provider;
    account.access_token = payload.access_token;
    account.refresh_token = payload.refresh_token;
    account.token_type = payload.token_type;
    account.expires_at = payload.expires_at;
    account.idc_region = payload.idc_region;
    account.issuer_url = payload.issuer_url;
    account.client_id = payload.client_id;
    account.scopes = payload.scopes;
    account.login_hint = payload.login_hint;
    account.plan_name = payload.plan_name;
    account.plan_tier = payload.plan_tier;
    account.credits_total = payload.credits_total;
    account.credits_used = payload.credits_used;
    account.bonus_total = payload.bonus_total;
    account.bonus_used = payload.bonus_used;
    account.usage_reset_at = payload.usage_reset_at;
    account.bonus_expire_days = payload.bonus_expire_days;
    account.kiro_auth_token_raw = payload.kiro_auth_token_raw;
    account.kiro_profile_raw = payload.kiro_profile_raw;
    account.kiro_usage_raw = payload.kiro_usage_raw;
    account.status = payload.status;
    account.status_reason = payload.status_reason;
    account.last_used = now_ts();
}

pub fn upsert_account(payload: KiroOAuthCompletePayload) -> Result<KiroAccount, String> {
    let _lock = KIRO_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Kiro 账号锁失败".to_string())?;
    let now = now_ts();
    let mut index = load_account_index();
    let incoming_user_id = normalize_user_identity(payload.user_id.as_deref());
    let incoming_email = normalize_email_identity(Some(payload.email.as_str()));
    let incoming_refresh_token = normalize_token_identity(payload.refresh_token.as_deref());
    let incoming_access_token = normalize_token_identity(Some(payload.access_token.as_str()));

    let identity_seed = incoming_user_id
        .clone()
        .or_else(|| incoming_email.clone())
        .or_else(|| incoming_refresh_token.clone())
        .or_else(|| incoming_access_token.clone())
        .unwrap_or_else(|| "kiro_user".to_string())
        .to_lowercase();
    let generated_id = format!("kiro_{:x}", md5::compute(identity_seed.as_bytes()));

    let account_id = index
        .accounts
        .iter()
        .filter_map(|item| load_account(&item.id))
        .find(|account| {
            let existing_user = normalize_user_identity(account.user_id.as_deref());
            let existing_email = normalize_email_identity(Some(account.email.as_str()));
            let existing_refresh_token = normalize_token_identity(account.refresh_token.as_deref());
            account_matches_payload_identity(
                existing_user.as_ref(),
                existing_email.as_ref(),
                existing_refresh_token.as_ref(),
                incoming_user_id.as_ref(),
                incoming_email.as_ref(),
                incoming_refresh_token.as_ref(),
            )
        })
        .map(|account| account.id)
        .unwrap_or(generated_id);

    let existing = load_account(&account_id);
    let tags = existing.as_ref().and_then(|acc| acc.tags.clone());
    let created_at = existing.as_ref().map(|acc| acc.created_at).unwrap_or(now);

    let mut account = existing.unwrap_or(KiroAccount {
        id: account_id.clone(),
        email: payload.email.clone(),
        user_id: payload.user_id.clone(),
        login_provider: payload.login_provider.clone(),
        tags,
        access_token: payload.access_token.clone(),
        refresh_token: payload.refresh_token.clone(),
        token_type: payload.token_type.clone(),
        expires_at: payload.expires_at,
        idc_region: payload.idc_region.clone(),
        issuer_url: payload.issuer_url.clone(),
        client_id: payload.client_id.clone(),
        scopes: payload.scopes.clone(),
        login_hint: payload.login_hint.clone(),
        plan_name: payload.plan_name.clone(),
        plan_tier: payload.plan_tier.clone(),
        credits_total: payload.credits_total,
        credits_used: payload.credits_used,
        bonus_total: payload.bonus_total,
        bonus_used: payload.bonus_used,
        usage_reset_at: payload.usage_reset_at,
        bonus_expire_days: payload.bonus_expire_days,
        kiro_auth_token_raw: payload.kiro_auth_token_raw.clone(),
        kiro_profile_raw: payload.kiro_profile_raw.clone(),
        kiro_usage_raw: payload.kiro_usage_raw.clone(),
        status: payload.status.clone(),
        status_reason: payload.status_reason.clone(),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
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
        "Kiro 账号已保存: id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

async fn refresh_account_token_once(account_id: &str) -> Result<KiroAccount, String> {
    let started_at = Instant::now();
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[Kiro Refresh] 开始刷新账号: id={}, email={}",
        account.id, account.email
    ));

    let payload = kiro_oauth::refresh_payload_for_account(&account).await?;
    let usage_refreshed = payload.kiro_usage_raw.is_some();
    let tags = account.tags.clone();
    let created_at = account.created_at;
    apply_payload(&mut account, payload);
    account.tags = tags;
    account.created_at = created_at;
    let refreshed_at = now_ts();
    if usage_refreshed {
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
        account.usage_updated_at = Some(refreshed_at);
    } else {
        account.quota_query_last_error = Some("未获取到有效配额数据".to_string());
        account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    }
    account.last_used = refreshed_at;

    let updated = account.clone();
    upsert_account_record(account)?;
    logger::log_info(&format!(
        "[Kiro Refresh] 刷新完成: id={}, email={}, elapsed={}ms",
        updated.id,
        updated.email,
        started_at.elapsed().as_millis()
    ));
    Ok(updated)
}

pub async fn refresh_account_token(account_id: &str) -> Result<KiroAccount, String> {
    let result = refresh_account_token_once(account_id).await;
    if let Err(err) = &result {
        persist_quota_query_error(account_id, err);
    }
    result
}

pub async fn refresh_all_tokens() -> Result<Vec<(String, Result<KiroAccount, String>)>, String> {
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    const MAX_CONCURRENT: usize = 5;
    let accounts = list_accounts();
    let total = accounts.len();
    let active_accounts: Vec<KiroAccount> = accounts
        .into_iter()
        .filter(|account| !is_banned_account(account))
        .collect();
    let skipped_banned = total.saturating_sub(active_accounts.len());
    if skipped_banned > 0 {
        logger::log_info(&format!(
            "[Kiro Refresh] 跳过封禁账号: skipped={}, total={}",
            skipped_banned, total
        ));
    }

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let tasks: Vec<_> = active_accounts
        .into_iter()
        .map(|account| {
            let id = account.id;
            let semaphore = semaphore.clone();
            async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|e| format!("获取 Kiro 刷新并发许可失败: {}", e))?;
                let result = refresh_account_token(&id).await;
                Ok::<(String, Result<KiroAccount, String>), String>((id, result))
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
    let _lock = KIRO_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Kiro 账号锁失败".to_string())?;
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

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<KiroAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    account.tags = Some(tags);
    account.last_used = now_ts();
    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

fn clone_object_value(value: Option<&Value>) -> Option<Value> {
    value.and_then(|raw| {
        if raw.is_object() {
            Some(raw.clone())
        } else {
            None
        }
    })
}

fn is_non_empty_json_value(value: &Value) -> bool {
    !value.is_null()
        && !value
            .as_str()
            .map(|text| text.trim().is_empty())
            .unwrap_or(false)
}

fn pick_import_usage(raw: &Value) -> Option<Value> {
    let obj = raw.as_object()?;
    for key in [
        "usageData",
        "usage_data",
        "usage",
        "usageState",
        "usage_state",
        "kiro_usage_raw",
    ] {
        if let Some(value) = obj.get(key).filter(|value| is_non_empty_json_value(value)) {
            return Some(value.clone());
        }
    }
    None
}

fn pick_import_profile(raw: &Value) -> Option<Value> {
    let obj = raw.as_object()?;
    for key in ["profile", "profileData", "profile_data", "kiro_profile_raw"] {
        if let Some(value) = clone_object_value(obj.get(key)) {
            return Some(value);
        }
    }

    let mut profile = serde_json::Map::new();
    for key in ["profileArn", "profile_arn", "arn"] {
        if let Some(value) = obj.get(key).filter(|value| is_non_empty_json_value(value)) {
            profile.insert("arn".to_string(), value.clone());
            break;
        }
    }
    for key in ["provider", "loginProvider"] {
        if let Some(value) = obj.get(key).filter(|value| is_non_empty_json_value(value)) {
            profile.insert("name".to_string(), value.clone());
            break;
        }
    }
    for key in ["email", "userEmail"] {
        if let Some(value) = obj.get(key).filter(|value| is_non_empty_json_value(value)) {
            profile.insert("email".to_string(), value.clone());
            break;
        }
    }

    if profile.is_empty() {
        None
    } else {
        Some(Value::Object(profile))
    }
}

fn build_import_auth_token(raw: &Value) -> Result<Value, String> {
    let raw_obj = raw
        .as_object()
        .ok_or_else(|| "Kiro 导入 JSON 必须是对象".to_string())?;

    let base = clone_object_value(raw_obj.get("kiro_auth_token_raw"))
        .or_else(|| clone_object_value(raw_obj.get("authToken")))
        .or_else(|| clone_object_value(raw_obj.get("token")))
        .or_else(|| clone_object_value(raw_obj.get("auth")))
        .unwrap_or_else(|| raw.clone());

    let mut auth_obj = match base {
        Value::Object(obj) => obj,
        _ => serde_json::Map::new(),
    };

    for key in [
        "accessToken",
        "access_token",
        "token",
        "idToken",
        "id_token",
        "refreshToken",
        "refresh_token",
        "expiresAt",
        "expires_at",
        "expiry",
        "expiration",
        "email",
        "userEmail",
        "userId",
        "user_id",
        "provider",
        "loginProvider",
        "authMethod",
        "login_option",
        "profileArn",
        "profile_arn",
        "arn",
        "login_hint",
        "loginHint",
        "idc_region",
        "idcRegion",
        "region",
        "issuer_url",
        "issuerUrl",
        "issuer",
        "client_id",
        "clientId",
        "client_secret",
        "clientSecret",
        "scope",
        "scopes",
        "startUrl",
        "start_url",
    ] {
        if auth_obj.contains_key(key) {
            continue;
        }
        if let Some(value) = raw_obj
            .get(key)
            .filter(|value| is_non_empty_json_value(value))
        {
            auth_obj.insert(key.to_string(), value.clone());
        }
    }

    Ok(Value::Object(auth_obj))
}

fn payload_from_import_value(raw: Value) -> Result<KiroOAuthCompletePayload, String> {
    if !raw.is_object() {
        return Err("Kiro 导入 JSON 必须是对象".to_string());
    }

    let usage = pick_import_usage(&raw);
    let profile = pick_import_profile(&raw);
    let auth_token = build_import_auth_token(&raw)?;
    kiro_oauth::build_payload_from_snapshot(auth_token, profile, usage)
}

fn payloads_from_import_json_value(value: Value) -> Result<Vec<KiroOAuthCompletePayload>, String> {
    match value {
        Value::Array(items) => {
            if items.is_empty() {
                return Err("导入数组为空".to_string());
            }

            let mut payloads = Vec::with_capacity(items.len());
            for (idx, item) in items.into_iter().enumerate() {
                let payload = payload_from_import_value(item)
                    .map_err(|e| format!("第 {} 条 Kiro 账号解析失败: {}", idx + 1, e))?;
                payloads.push(payload);
            }
            Ok(payloads)
        }
        Value::Object(mut obj) => {
            let object_value = Value::Object(obj.clone());
            if let Ok(payload) = payload_from_import_value(object_value.clone()) {
                return Ok(vec![payload]);
            }

            if let Some(accounts) = obj
                .remove("accounts")
                .or_else(|| obj.remove("items"))
                .and_then(|raw| raw.as_array().cloned())
            {
                if accounts.is_empty() {
                    return Err("导入数组为空".to_string());
                }
                let mut payloads = Vec::with_capacity(accounts.len());
                for (idx, item) in accounts.into_iter().enumerate() {
                    let payload = payload_from_import_value(item)
                        .map_err(|e| format!("第 {} 条 Kiro 账号解析失败: {}", idx + 1, e))?;
                    payloads.push(payload);
                }
                return Ok(payloads);
            }

            Err("无法解析 Kiro 导入对象".to_string())
        }
        _ => Err("Kiro 导入 JSON 必须是对象或数组".to_string()),
    }
}

pub fn import_from_json(json_content: &str) -> Result<Vec<KiroAccount>, String> {
    if let Ok(account) = serde_json::from_str::<KiroAccount>(json_content) {
        let saved = upsert_account_record(account)?;
        return Ok(vec![saved]);
    }

    if let Ok(accounts) = serde_json::from_str::<Vec<KiroAccount>>(json_content) {
        let mut result = Vec::new();
        for account in accounts {
            let saved = upsert_account_record(account)?;
            result.push(saved);
        }
        return Ok(result);
    }

    if let Ok(value) = serde_json::from_str::<Value>(json_content) {
        if let Ok(payloads) = payloads_from_import_json_value(value) {
            let mut result = Vec::with_capacity(payloads.len());
            for payload in payloads {
                let saved = upsert_account(payload)?;
                result.push(saved);
            }
            return Ok(result);
        }
    }

    Err("无法解析 JSON 内容".to_string())
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<KiroAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

fn normalize_quota_alert_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn clamp_percent(value: f64) -> i32 {
    value.round().clamp(0.0, 100.0) as i32
}

fn calc_remaining_percent(total: Option<f64>, used: Option<f64>) -> Option<i32> {
    let total = total?;
    if !total.is_finite() || total <= 0.0 {
        return None;
    }

    let used = used.unwrap_or(0.0);
    if !used.is_finite() {
        return None;
    }
    let remaining = (total - used).max(0.0);
    Some(clamp_percent((remaining / total) * 100.0))
}

pub(crate) fn extract_quota_metrics(account: &KiroAccount) -> Vec<(String, i32)> {
    let mut metrics = Vec::new();

    if let Some(pct) = calc_remaining_percent(account.credits_total, account.credits_used) {
        metrics.push(("Prompt Credits".to_string(), pct));
    }
    if let Some(pct) = calc_remaining_percent(account.bonus_total, account.bonus_used) {
        metrics.push(("Add-on Credits".to_string(), pct));
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

pub(crate) fn resolve_current_account_id(accounts: &[KiroAccount]) -> Option<String> {
    crate::modules::provider_current_state::resolve_existing_current_account_id(
        "kiro",
        accounts.iter().map(|account| account.id.as_str()),
    )
}

fn display_email(account: &KiroAccount) -> String {
    let trimmed = account.email.trim();
    if trimmed.is_empty() {
        account.id.clone()
    } else {
        trimmed.to_string()
    }
}

fn build_quota_alert_cooldown_key(account_id: &str, threshold: i32) -> String {
    format!("kiro:{}:{}", account_id, threshold)
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = KIRO_QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < KIRO_QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, threshold: i32) {
    if let Ok(mut state) = KIRO_QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(account_id, threshold));
    }
}

fn pick_quota_alert_recommendation(
    accounts: &[KiroAccount],
    current_id: &str,
) -> Option<KiroAccount> {
    let mut candidates: Vec<KiroAccount> = accounts
        .iter()
        .filter(|account| account.id != current_id)
        .filter(|account| !is_banned_account(account))
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
    if !cfg.kiro_quota_alert_enabled {
        return Ok(None);
    }

    let threshold = normalize_quota_alert_threshold(cfg.kiro_quota_alert_threshold);
    let accounts = list_accounts();
    let current_id = match resolve_current_account_id(&accounts) {
        Some(id) => id,
        None => return Ok(None),
    };

    let current = match accounts.iter().find(|account| account.id == current_id) {
        Some(account) => account,
        None => return Ok(None),
    };
    if is_banned_account(current) {
        return Ok(None);
    }

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
        platform: "kiro".to_string(),
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

pub fn get_default_kiro_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join("Library/Application Support/Kiro"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        return Ok(PathBuf::from(appdata).join("Kiro"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".config/Kiro"));
    }

    #[allow(unreachable_code)]
    Err("Kiro 账号导入仅支持 macOS、Windows 和 Linux".to_string())
}

pub fn get_default_kiro_profile_path() -> Result<PathBuf, String> {
    Ok(get_default_kiro_data_dir()?
        .join("User")
        .join("globalStorage")
        .join("kiro.kiroagent")
        .join("profile.json"))
}

pub fn get_default_kiro_state_db_path() -> Result<PathBuf, String> {
    Ok(get_default_kiro_data_dir()?
        .join("User")
        .join("globalStorage")
        .join("state.vscdb"))
}

pub fn get_default_kiro_auth_token_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    Ok(home
        .join(".aws")
        .join("sso")
        .join("cache")
        .join(LOCAL_AUTH_TOKEN_FILE_NAME))
}

pub fn read_local_auth_token_json() -> Result<Option<Value>, String> {
    let path = get_default_kiro_auth_token_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("读取 Kiro 本地授权文件失败({}): {}", path.display(), e))?;
    let parsed = serde_json::from_str::<Value>(&raw)
        .map_err(|e| format!("解析 Kiro 本地授权文件失败({}): {}", path.display(), e))?;
    Ok(Some(parsed))
}

pub fn read_local_profile_json() -> Result<Option<Value>, String> {
    let path = get_default_kiro_profile_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("读取 Kiro profile.json 失败({}): {}", path.display(), e))?;
    let parsed = serde_json::from_str::<Value>(&raw)
        .map_err(|e| format!("解析 Kiro profile.json 失败({}): {}", path.display(), e))?;
    Ok(Some(parsed))
}

pub fn read_local_usage_snapshot() -> Result<Option<Value>, String> {
    let path = get_default_kiro_state_db_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let conn = Connection::open(&path)
        .map_err(|e| format!("打开 Kiro 本地数据库失败({}): {}", path.display(), e))?;

    let raw = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [LOCAL_USAGE_DB_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("读取 Kiro usage 快照失败: {}", e))?;

    match raw {
        Some(value) => {
            let parsed = serde_json::from_str::<Value>(&value)
                .map_err(|e| format!("解析 Kiro usage 快照失败: {}", e))?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}
