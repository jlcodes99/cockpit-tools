use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use tauri::Emitter;

use crate::models::minimax_code::{
    MinimaxCodeAccount, MinimaxCodeAccountIndex, MinimaxCodeOAuthCompletePayload,
};
use crate::modules::{account, logger, minimax_code_oauth};

const ACCOUNTS_INDEX_FILE: &str = "minimax_code_accounts.json";
const ACCOUNTS_DIR: &str = "minimax_code_accounts";
const MINIMAX_CODE_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 10 * 60;

lazy_static::lazy_static! {
    static ref MINIMAX_CODE_ACCOUNT_INDEX_LOCK: Mutex<()> = Mutex::new(());
    static ref MINIMAX_CODE_QUOTA_ALERT_LAST_SENT: Mutex<HashMap<String, i64>> = Mutex::new(HashMap::new());
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
        fs::create_dir_all(&dir).map_err(|e| format!("创建 MiniMax Code 账号目录失败:{}", e))?;
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

pub fn load_account(account_id: &str) -> Option<MinimaxCodeAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    match crate::modules::secure_account_storage::deserialize_account_file::<MinimaxCodeAccount>(
        &account_path,
        &content,
    ) {
        Ok((account, needs_rotation)) => {
            if needs_rotation {
                let account_for_rewrite = account.clone();
                crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
                    "minimaxcode",
                    account_for_rewrite.id.clone(),
                    account_path.clone(),
                    content.as_bytes(),
                    move || {
                        crate::modules::secure_account_storage::serialize_account_file(
                            "minimaxcode",
                            &account_for_rewrite,
                        )
                    },
                );
            }
            Some(account)
        }
        Err(_) => None,
    }
}

fn save_account_file(account: &MinimaxCodeAccount) -> Result<(), String> {
    let path = resolve_account_file_path(account.id.as_str())?;
    let content =
        crate::modules::secure_account_storage::serialize_account_file("minimaxcode", account)?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存账号失败:{}", e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    if path.exists() {
        crate::modules::atomic_write::remove_file_locked(&path)
            .map_err(|e| format!("删除账号文件失败:{}", e))?;
    }
    Ok(())
}

fn load_account_index() -> MinimaxCodeAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return MinimaxCodeAccountIndex::new(),
    };
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(MinimaxCodeAccountIndex::new);
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空")
                .unwrap_or_else(MinimaxCodeAccountIndex::new)
        }
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            MinimaxCodeAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(MinimaxCodeAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[MiniMax Code Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(MinimaxCodeAccountIndex::new)
            }
        },
        Err(_) => MinimaxCodeAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<MinimaxCodeAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(MinimaxCodeAccountIndex::new());
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
        return Ok(MinimaxCodeAccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<MinimaxCodeAccountIndex>(
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

fn save_account_index(index: &MinimaxCodeAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败:{}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败:{}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<MinimaxCodeAccountIndex> {
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

    let mut index = MinimaxCodeAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[MiniMax Code Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[MiniMax Code Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_warn(&format!(
        "[MiniMax Code Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut MinimaxCodeAccountIndex, account: &MinimaxCodeAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: MinimaxCodeAccount) -> Result<MinimaxCodeAccount, String> {
    let _lock = MINIMAX_CODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 MiniMax Code 账号锁失败".to_string())?;
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

fn accounts_are_duplicates(left: &MinimaxCodeAccount, right: &MinimaxCodeAccount) -> bool {
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

fn merge_duplicate_account(primary: &mut MinimaxCodeAccount, dup: &MinimaxCodeAccount) {
    if primary.email.trim().is_empty() && !dup.email.trim().is_empty() {
        primary.email = dup.email.clone();
    }
    if primary.access_token.trim().is_empty() && !dup.access_token.trim().is_empty() {
        primary.access_token = dup.access_token.clone();
    }
    fill_if_none(&mut primary.uid, &dup.uid);
    fill_if_none(&mut primary.nickname, &dup.nickname);
    fill_if_none(&mut primary.refresh_token, &dup.refresh_token);
    fill_if_none(&mut primary.token_type, &dup.token_type);
    fill_if_none(&mut primary.expires_at, &dup.expires_at);
    fill_if_none(&mut primary.region, &dup.region);
    fill_if_none(&mut primary.auth_file, &dup.auth_file);
    fill_if_none(&mut primary.plan_type, &dup.plan_type);
    fill_if_none(&mut primary.quota_raw, &dup.quota_raw);
    fill_if_none(&mut primary.auth_raw, &dup.auth_raw);
    fill_if_none(&mut primary.profile_raw, &dup.profile_raw);
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

fn choose_primary_account_index(group: &[usize], accounts: &[MinimaxCodeAccount]) -> usize {
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

fn normalize_account_index(index: &mut MinimaxCodeAccountIndex) -> Vec<MinimaxCodeAccount> {
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
            "[MiniMax Code Account] 检测到重复账号并已合并:removed_ids={}",
            removed_ids.join(",")
        ));
    }

    index.accounts = normalized.iter().map(|a| a.summary()).collect();
    normalized
}

pub fn list_accounts() -> Vec<MinimaxCodeAccount> {
    let mut index = load_account_index();
    let had_index_accounts = !index.accounts.is_empty();
    let index_before_normalize = serde_json::to_vec(&index).ok();
    let accounts = normalize_account_index(&mut index);
    if had_index_accounts && accounts.is_empty() {
        logger::log_warn(
            "[MiniMax Code Account] 账号索引中存在账号，但详情文件均无法读取，已跳过空索引写回",
        );
        return accounts;
    }
    let index_changed = index_before_normalize
        .as_ref()
        .map(|before| Some(before.as_slice()) != serde_json::to_vec(&index).ok().as_deref())
        .unwrap_or(true);
    if index_changed {
        if let Err(err) = save_account_index(&index) {
            logger::log_warn(&format!("[MiniMax Code Account] 保存账号索引失败:{}", err));
        }
    }
    accounts
}

pub fn list_accounts_checked() -> Result<Vec<MinimaxCodeAccount>, String> {
    let mut index = load_account_index_checked()?;
    let had_index_accounts = !index.accounts.is_empty();
    let index_before_normalize = serde_json::to_vec(&index).ok();
    let accounts = normalize_account_index(&mut index);
    if had_index_accounts && accounts.is_empty() {
        return Err("MiniMax Code 账号索引中存在账号，但详情文件均无法读取；已保留前端缓存，请从账号备份或本地账号文件恢复。".to_string());
    }
    let index_changed = index_before_normalize
        .as_ref()
        .map(|before| Some(before.as_slice()) != serde_json::to_vec(&index).ok().as_deref())
        .unwrap_or(true);
    if index_changed {
        if let Err(err) = save_account_index(&index) {
            logger::log_warn(&format!("[MiniMax Code Account] 保存账号索引失败:{}", err));
        }
    }
    Ok(accounts)
}

fn apply_payload(account: &mut MinimaxCodeAccount, payload: MinimaxCodeOAuthCompletePayload) {
    let incoming_email = payload.email.trim().to_string();
    if !incoming_email.is_empty() {
        account.email = incoming_email;
    }
    account.uid = payload.uid;
    account.nickname = payload.nickname;
    account.access_token = payload.access_token;
    account.refresh_token = payload.refresh_token;
    account.token_type = payload.token_type;
    account.expires_at = payload.expires_at;
    if payload.region.is_some() {
        account.region = payload.region;
    }
    if payload.auth_file.is_some() {
        account.auth_file = payload.auth_file;
    }
    if payload.quota_raw.is_some() {
        account.quota_raw = payload.quota_raw;
    }
    account.auth_raw = payload.auth_raw;
    if payload.profile_raw.is_some() {
        account.profile_raw = payload.profile_raw;
    }
    account.status = payload.status;
    account.status_reason = payload.status_reason;
    account.last_used = now_ts();
}

pub fn upsert_account(payload: MinimaxCodeOAuthCompletePayload) -> Result<MinimaxCodeAccount, String> {
    let _lock = MINIMAX_CODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 MiniMax Code 账号锁失败".to_string())?;
    let now = now_ts();
    let mut index = load_account_index();

    let incoming_uid = normalize_identity(payload.uid.as_deref());
    let incoming_email = normalize_email_identity(Some(payload.email.as_str()));

    let identity_seed = incoming_uid
        .clone()
        .or_else(|| incoming_email.clone())
        .unwrap_or_else(|| "minimax_code_user".to_string())
        .to_lowercase();
    let generated_id = format!("minimax_code_{:x}", md5::compute(identity_seed.as_bytes()));

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

    let mut account = existing.unwrap_or(MinimaxCodeAccount {
        id: account_id.clone(),
        email: payload.email.clone(),
        uid: payload.uid.clone(),
        nickname: payload.nickname.clone(),
        tags,
        access_token: payload.access_token.clone(),
        refresh_token: payload.refresh_token.clone(),
        token_type: payload.token_type.clone(),
        expires_at: payload.expires_at,
        region: payload.region.clone(),
        auth_file: payload.auth_file.clone(),
        plan_type: None,
        quota_raw: payload.quota_raw.clone(),
        auth_raw: payload.auth_raw.clone(),
        profile_raw: payload.profile_raw.clone(),
        status: payload.status.clone(),
        status_reason: payload.status_reason.clone(),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        last_checkin_time: None,
        checkin_streak: None,
        checkin_rewards: None,
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
        "MiniMax Code 账号已保存:id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

async fn refresh_account_token_once(account_id: &str) -> Result<MinimaxCodeAccount, String> {
    let started_at = Instant::now();
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[MiniMax Code Refresh] 开始刷新账号:id={}, email={}",
        account.id, account.email
    ));

    let region = account
        .region
        .clone()
        .unwrap_or_else(|| "cn".to_string());

    // (a) MiniMax access tokens are short-lived (~1h) and the official client
    // renews them in the local auth.json. Re-read that file first so we always
    // use the freshest access token before attempting a refresh.
    if let Some(auth_file) = account.auth_file.clone() {
        if let Some((_id, _region, rec, _path)) = read_minimax_code_auth_file(Path::new(&auth_file)) {
            if let Some(access) = rec
                .get("accessToken")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                account.access_token = access.to_string();
            }
            if let Some(refresh) = rec
                .get("refreshToken")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                account.refresh_token = Some(refresh.to_string());
            }
            if let Some(token_type) = rec
                .get("tokenType")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                account.token_type = Some(token_type.to_string());
            }
            if let Some(expires_at) = rec.get("expiresAtMs").and_then(|v| v.as_i64()) {
                account.expires_at = Some(expires_at);
            }
        }
    }

    // (b) Refresh the access token when a refresh token is available.
    if let Some(refresh_token) = account.refresh_token.as_deref() {
        match minimax_code_oauth::refresh_token(refresh_token, &region).await {
            Ok(token_data) => {
                if let Some(access) = token_data
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    account.access_token = access.to_string();
                }
                if let Some(refresh) = token_data
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    account.refresh_token = Some(refresh.to_string());
                }
                if let Some(token_type) = token_data
                    .get("token_type")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    account.token_type = Some(token_type.to_string());
                }
                if let Some(expires_in) = token_data.get("expires_in").and_then(|v| v.as_i64()) {
                    // Keep the model in epoch milliseconds, matching `expiresAtMs`.
                    account.expires_at =
                        Some(minimax_code_oauth::expires_at_from_expires_in(expires_in));
                }
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[MiniMax Code Refresh] Token 刷新失败，将使用现有 token 查询配额:{}",
                    err
                ));
            }
        }
    }

    // (c) Refresh quota and store the raw body on the account.
    let client = minimax_code_oauth::build_client()?;
    let mut quota_refresh_error: Option<String> = None;
    let quota_raw =
        match minimax_code_oauth::fetch_credit_details(&client, &account.access_token, &region).await
        {
            Ok(body) => Some(body),
            Err(err) => {
                logger::log_warn(&format!(
                    "[MiniMax Code Refresh] 刷新 credit/details 失败:{}",
                    err
                ));
                quota_refresh_error = Some(err);
                None
            }
        };

    let tags = account.tags.clone();
    let created_at = account.created_at;
    let payload = MinimaxCodeOAuthCompletePayload {
        email: account.email.clone(),
        uid: account.uid.clone(),
        nickname: account.nickname.clone(),
        access_token: account.access_token.clone(),
        refresh_token: account.refresh_token.clone(),
        token_type: account.token_type.clone(),
        expires_at: account.expires_at,
        region: Some(region.clone()),
        auth_file: account.auth_file.clone(),
        quota_raw,
        auth_raw: account.auth_raw.clone(),
        profile_raw: account.profile_raw.clone(),
        status: account.status.clone().or_else(|| Some("normal".to_string())),
        status_reason: account.status_reason.clone(),
    };
    let usage_refreshed = quota_refresh_error.is_none();
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
        "[MiniMax Code Refresh] 刷新完成:id={}, email={}, elapsed={}ms",
        updated.id,
        updated.email,
        started_at.elapsed().as_millis()
    ));
    Ok(updated)
}

pub async fn refresh_account_token(account_id: &str) -> Result<MinimaxCodeAccount, String> {
    refresh_account_token_once(account_id).await
}

pub async fn refresh_all_tokens() -> Result<Vec<(String, Result<MinimaxCodeAccount, String>)>, String>
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
                    .map_err(|e| format!("获取并发许可失败:{}", e))?;
                let result = refresh_account_token(&id).await;
                Ok::<(String, Result<MinimaxCodeAccount, String>), String>((id, result))
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
    let _lock = MINIMAX_CODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 MiniMax Code 账号锁失败".to_string())?;
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
) -> Result<MinimaxCodeAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    account.tags = Some(tags);
    account.last_used = now_ts();
    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<MinimaxCodeAccount>, String> {
    if let Ok(account) = serde_json::from_str::<MinimaxCodeAccount>(json_content) {
        let saved = upsert_account_record(account)?;
        return Ok(vec![saved]);
    }

    if let Ok(accounts) = serde_json::from_str::<Vec<MinimaxCodeAccount>>(json_content) {
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

    Err("无法解析 MiniMax Code JSON 导入内容".to_string())
}

fn import_from_json_value(value: Value) -> Result<Vec<MinimaxCodeAccount>, String> {
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

            Err("无法解析 MiniMax Code 导入对象".to_string())
        }
        _ => Err("MiniMax Code 导入 JSON 必须是对象或数组".to_string()),
    }
}

fn upsert_account_record_from_payload(
    payload: MinimaxCodeOAuthCompletePayload,
) -> Result<MinimaxCodeAccount, String> {
    let now = now_ts();
    let incoming_uid = normalize_identity(payload.uid.as_deref());
    let incoming_email = normalize_email_identity(Some(payload.email.as_str()));
    let identity_seed = incoming_uid
        .or_else(|| incoming_email)
        .unwrap_or_else(|| "minimax_code_user".to_string());
    let generated_id = format!("minimax_code_{:x}", md5::compute(identity_seed.as_bytes()));

    let account = MinimaxCodeAccount {
        id: generated_id,
        email: payload.email,
        uid: payload.uid,
        nickname: payload.nickname,
        tags: None,
        access_token: payload.access_token,
        refresh_token: payload.refresh_token,
        token_type: payload.token_type,
        expires_at: payload.expires_at,
        region: payload.region,
        auth_file: payload.auth_file,
        plan_type: None,
        quota_raw: payload.quota_raw,
        auth_raw: payload.auth_raw,
        profile_raw: payload.profile_raw,
        status: payload.status,
        status_reason: payload.status_reason,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        last_checkin_time: None,
        checkin_streak: None,
        checkin_rewards: None,
        created_at: now,
        last_used: now,
    };
    upsert_account_record(account)
}

fn payload_from_import_value(raw: Value) -> Result<MinimaxCodeOAuthCompletePayload, String> {
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

    let refresh_token = obj
        .get("refresh_token")
        .or_else(|| obj.get("refreshToken"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let token_type = obj
        .get("token_type")
        .or_else(|| obj.get("tokenType"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let region = obj
        .get("region")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let auth_file = obj
        .get("auth_file")
        .or_else(|| obj.get("authFile"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let expires_at = json_object_i64_field(obj, &["expires_at", "expiresAt", "expiresAtMs"]);

    Ok(MinimaxCodeOAuthCompletePayload {
        email,
        uid,
        nickname,
        access_token,
        refresh_token,
        token_type: token_type.or_else(|| Some("Bearer".to_string())),
        expires_at,
        region,
        auth_file,
        quota_raw: None,
        auth_raw: obj.get("auth_raw").cloned(),
        profile_raw: obj.get("profile_raw").cloned(),
        status: Some("normal".to_string()),
        status_reason: None,
    })
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

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<MinimaxCodeAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("导出失败:{}", e))
}

fn json_amount(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Lowest remaining-percentage across all credit detail groups, if computable.
fn minimax_code_quota_remaining_percent(quota_raw: &Value) -> Option<f64> {
    let details = quota_raw
        .get("data")
        .and_then(|data| data.get("details"))
        .and_then(Value::as_array)?;
    let mut lowest: Option<f64> = None;
    for detail in details {
        let granted = json_amount(
            detail
                .get("granted_amount")
                .or_else(|| detail.get("grantedAmount")),
        );
        let remaining = json_amount(
            detail
                .get("remaining_amount")
                .or_else(|| detail.get("remainingAmount")),
        );
        if granted > 0.0 {
            let percent = (remaining / granted) * 100.0;
            lowest = Some(lowest.map_or(percent, |prev| prev.min(percent)));
        }
    }
    lowest
}

pub fn run_quota_alert_if_needed() -> Result<(), String> {
    let config = crate::modules::config::get_user_config();
    if !config.minimax_code_quota_alert_enabled {
        return Ok(());
    }
    let threshold = config.minimax_code_quota_alert_threshold;
    if threshold <= 0 {
        return Ok(());
    }

    let accounts = list_accounts();
    let now = now_ts();
    let mut last_sent = MINIMAX_CODE_QUOTA_ALERT_LAST_SENT
        .lock()
        .map_err(|_| "获取预警锁失败".to_string())?;

    for account in &accounts {
        let cooldown_key = account.id.clone();
        if let Some(last) = last_sent.get(&cooldown_key) {
            if now - last < MINIMAX_CODE_QUOTA_ALERT_COOLDOWN_SECONDS {
                continue;
            }
        }

        let remaining = account
            .quota_raw
            .as_ref()
            .and_then(minimax_code_quota_remaining_percent);

        if let Some(percent) = remaining {
            if percent <= threshold as f64 {
                last_sent.insert(cooldown_key, now);
                if let Some(app) = crate::get_app_handle() {
                    let msg = format!("MiniMax Code 配额即将耗尽（剩余 {:.1}%）", percent);
                    let _ = app.emit(
                        "quota:alert",
                        serde_json::json!({
                            "platform": "minimaxcode",
                            "accountId": account.id,
                            "email": account.email,
                            "message": msg,
                        }),
                    );
                }
            }
        }
    }

    Ok(())
}

pub fn update_checkin_info(
    account_id: &str,
    last_checkin_time: Option<i64>,
    streak: i32,
    rewards: Option<serde_json::Value>,
) -> Result<MinimaxCodeAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;

    if let Some(time) = last_checkin_time {
        account.last_checkin_time = Some(time);
    }
    account.checkin_streak = Some(streak);
    account.checkin_rewards = rewards;

    account.last_used = now_ts();
    let updated = account.clone();
    save_account_file(&account)?;

    logger::log_info(&format!(
        "[MiniMax Code Checkin] 签到信息已更新: account_id={}, streak={}",
        updated.id, streak
    ));

    Ok(updated)
}

/// Recursively find `~/.minimax/auth/**/auth.json` (depth ≤ 4; layout varies by version).
pub fn find_minimax_code_auth_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(home) = dirs::home_dir() else { return out; };
    collect_auth_files(&home.join(".minimax").join("auth"), &mut out, 0);
    out
}

fn collect_auth_files(dir: &PathBuf, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 4 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_auth_files(&path, out, depth + 1);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("auth.json") {
            out.push(path);
        }
    }
}

/// Read the first non-empty account from a MiniMax `auth.json`.
/// Returns `(account_id, region, record, auth_file_path)`.
pub fn read_minimax_code_auth_file(path: &std::path::Path) -> Option<(String, String, Value, String)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    let records = parsed.get("records")?.as_object()?;
    for (key, rec) in records {
        let access = rec.get("accessToken").and_then(|v| v.as_str()).unwrap_or("");
        if access.is_empty() { continue; }
        let id = key.rsplit('\u{0}').next().unwrap_or(key).to_string();
        let prefix = key.split('\u{0}').next().unwrap_or(key);
        let region = prefix.rsplit('.').next().unwrap_or("cn").to_string();
        return Some((id, region, rec.clone(), path.to_string_lossy().to_string()));
    }
    None
}

/// Import every account found in the local MiniMax client.
pub fn import_payload_from_local() -> Result<Vec<MinimaxCodeOAuthCompletePayload>, String> {
    let mut out = Vec::new();
    for file in find_minimax_code_auth_files() {
        if let Some((id, region, rec, path)) = read_minimax_code_auth_file(&file) {
            out.push(crate::modules::minimax_code_oauth::payload_from_local_record(&id, &region, &rec, &path));
        }
    }
    Ok(out)
}

/// Resolve the MiniMax client exe (config override, then candidates).
pub fn resolve_minimax_code_launch_path() -> Result<PathBuf, String> {
    let configured = crate::modules::config::get_user_config().minimax_code_app_path;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        candidates.push(
            local
                .join("Programs")
                .join("MiniMax Code")
                .join("MiniMax Code.exe"),
        );
    }
    candidates.push(PathBuf::from(
        r"D:\Programs\MiniMax Code\MiniMax Code.exe",
    ));
    candidates.push(PathBuf::from(
        r"C:\Program Files\MiniMax Code\MiniMax Code.exe",
    ));
    resolve_minimax_code_launch_path_from(&configured, &candidates)
}

/// Resolver core, split out so tests can supply a deterministic candidate list.
fn resolve_minimax_code_launch_path_from(
    configured: &str,
    candidates: &[PathBuf],
) -> Result<PathBuf, String> {
    let trimmed = configured.trim();
    if !trimmed.is_empty() {
        let path = PathBuf::from(trimmed);
        if path.is_file() {
            return Ok(path);
        }
    }
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    Err("未找到 MiniMax Code 客户端可执行文件".to_string())
}

/// Switch the local MiniMax client to `account` by rewriting its `auth.json`.
/// The client keeps the FIRST non-empty `accessToken` record, so only the target
/// account's record is written back. The pre-switch login state is preserved at
/// `<file>.json.cockpit.bak` (only the first backup is kept, never overwritten).
/// Returns the auth file path.
pub fn switch_minimax_code_account(account: &MinimaxCodeAccount) -> Result<PathBuf, String> {
    switch_minimax_code_account_impl(account, None)
}

/// Switch a **managed instance** to `account` by rewriting the `auth.json` inside
/// `data_dir` (the instance's own `MINIMAX_DATA_DIR`).
///
/// The MiniMax client derives its credential namespace from the data root:
/// `<data root>/auth/<buildEnv>/<region>/<clientId>/auth.json`
/// (see `createAuthNamespace` in the client bundle). Passing a per-instance data root
/// therefore gives each managed instance its own login state — the foundation of
/// MiniMax Code multi-instance support.
///
/// When the instance has no `auth.json` yet, the record key prefix is inherited from
/// the account's stored `auth_file` (or any global `auth.json`) so the namespace layout
/// matches what the client would have created itself.
pub fn switch_minimax_code_account_to_data_dir(
    account: &MinimaxCodeAccount,
    data_dir: &Path,
) -> Result<PathBuf, String> {
    switch_minimax_code_account_impl(account, Some(data_dir))
}

fn switch_minimax_code_account_impl(
    account: &MinimaxCodeAccount,
    data_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    let auth_file = match data_dir {
        // Managed instance: always target the instance's own namespace.
        Some(root) => crate::modules::minimax_code_instance::resolve_minimax_code_auth_file_path(root),
        None => account
            .auth_file
            .as_ref()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| find_minimax_code_auth_files().into_iter().next())
            .ok_or_else(|| "未找到 MiniMax Code 登录态文件".to_string())?,
    };
    if let Some(parent) = auth_file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 MiniMax Code 登录态目录失败: {e}"))?;
    }
    // For a fresh instance namespace, borrow the record key prefix from an existing
    // auth file so the client recognises the layout verbatim.
    let prefix_hint = if data_dir.is_some() && !auth_file.is_file() {
        account
            .auth_file
            .as_ref()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| find_minimax_code_auth_files().into_iter().next())
            .and_then(|source| read_auth_file_prefix(&source))
    } else {
        None
    };

    let raw = std::fs::read_to_string(&auth_file)
        .unwrap_or_else(|_| "{\"schemaVersion\":1,\"records\":{}}".to_string());
    let mut parsed: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|_| serde_json::json!({"schemaVersion": 1, "records": {}}));
    if !parsed.is_object() {
        return Err("MiniMax Code 登录态文件格式非法".to_string());
    }

    // Preserve the client's current login state before overwriting it. The backup
    // uses a distinct `.cockpit.bak` path and is written only once, so a later
    // switch can never clobber the first (oldest) login state.
    let bak = auth_file.with_extension("json.cockpit.bak");
    if !bak.exists() {
        let _ = std::fs::copy(&auth_file, &bak);
    }

    let records = parsed
        .get("records")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    // Reuse the target account's own key prefix when it already exists so the
    // env/region/clientId layout is preserved verbatim; otherwise fall back to any
    // existing key, then to the default prod/cn prefix.
    let target_suffix = format!("\u{0}{}", account.id);
    let prefix = records
        .keys()
        .find(|key| key.ends_with(&target_suffix))
        .or_else(|| records.keys().next())
        .and_then(|key| key.split('\u{0}').next().map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
        .or(prefix_hint)
        .unwrap_or_else(|| "com.minimax.mcode.oauth.prod.cn".to_string());
    let target_key = format!("{}\u{0}{}", prefix, account.id);

    let mut record = records
        .get(&target_key)
        .cloned()
        .or_else(|| records.values().next().cloned())
        .unwrap_or_else(|| {
            serde_json::json!({
                "schemaVersion": 1, "tokenType": "Bearer", "clientId": "mcode-public",
                "scopes": ["agent.default"], "audience": "agent-backend", "generation": 1
            })
        });
    if let Some(obj) = record.as_object_mut() {
        obj.insert("accessToken".into(), Value::String(account.access_token.clone()));
        // Only overwrite when we actually hold a refresh token: a `None` must not
        // blank a refresh token the client has already renewed locally.
        if let Some(refresh) = account
            .refresh_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            obj.insert("refreshToken".into(), Value::String(refresh.to_string()));
        }
        match account.expires_at {
            Some(ms) => {
                obj.insert("expiresAtMs".into(), serde_json::json!(ms));
            }
            None => {
                // Drop a stale expiry copied from another account's record.
                obj.remove("expiresAtMs");
            }
        }
        let generation = obj.get("generation").and_then(|v| v.as_i64()).unwrap_or(0);
        obj.insert("generation".into(), serde_json::json!(generation + 1));
    }

    // Keep only the target record: the client takes the first non-empty token.
    let mut new_records = serde_json::Map::new();
    new_records.insert(target_key, record);
    parsed["records"] = Value::Object(new_records);
    if parsed.get("schemaVersion").is_none() {
        parsed["schemaVersion"] = serde_json::json!(1);
    }

    // `write_bytes_atomic` replaces via temp-file + rename and creates no `.bak`,
    // so it never touches the first-state backup above.
    crate::modules::atomic_write::write_bytes_atomic(
        &auth_file,
        serde_json::to_string_pretty(&parsed)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
    .map_err(|e| format!("写入 MiniMax Code 登录态失败: {e}"))?;
    Ok(auth_file)
}

/// Read only the record key prefix (`com.minimax.mcode.oauth.<buildEnv>.<region>`) from
/// an `auth.json`. Never returns token material.
fn read_auth_file_prefix(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    parsed
        .get("records")?
        .as_object()?
        .keys()
        .next()
        .and_then(|key| key.split('\u{0}').next().map(|s| s.to_string()))
        .filter(|value| !value.is_empty())
}

/// Ask every running MiniMax Code client (matched by exact exe path) to exit.
///
/// **Multi-instance limitation.** The MiniMax client does not expose its
/// `MINIMAX_DATA_DIR` on the process command line (the Electron main process runs as
/// `"…\MiniMax Code.exe"` with no data-dir argument), so a running process cannot be
/// attributed to a specific managed instance data root. PID matching is therefore
/// exe-path based, and closing before a switch closes *all* MiniMax Code windows.
/// Each instance still owns an isolated `auth.json` under its own data root, so account
/// bindings are never mixed — but only one MiniMax Code instance can be logged in /
/// in the foreground at a time.
pub fn close_minimax_code_instances(timeout_secs: u64) -> Result<(), String> {
    let exe = resolve_minimax_code_launch_path()?;
    close_running_minimax_code_client(&exe, timeout_secs)
}

/// Ask a running MiniMax Code client (matched by exact exe path) to exit so the
/// injected `auth.json` is actually picked up. Returns a clear error when the
/// client cannot be closed, instead of injecting into a live process that would
/// silently revert the change on exit.
fn close_running_minimax_code_client(exe: &Path, timeout_secs: u64) -> Result<(), String> {
    crate::modules::process::close_processes_by_exact_exe_paths(
        &[exe.to_path_buf()],
        timeout_secs,
    )
    .map_err(|_| "MiniMax Code 正在运行，请稍后重试".to_string())?;
    // Give the OS a moment to release the file handle before rewriting auth.json.
    std::thread::sleep(std::time::Duration::from_millis(200));
    Ok(())
}

/// Switch + launch the client. Returns the launched exe path.
pub fn switch_and_launch(account: &MinimaxCodeAccount) -> Result<PathBuf, String> {
    // A running client keeps the previous login in memory and rewrites auth.json on
    // exit, which would silently revert the injection. So: resolve the exe, close
    // any running instance, inject, and only then relaunch.
    let exe = resolve_minimax_code_launch_path()?;
    close_running_minimax_code_client(&exe, 10)?;
    switch_minimax_code_account(account)?;
    std::process::Command::new(&exe)
        .spawn()
        .map_err(|e| format!("启动 MiniMax Code 失败: {e}"))?;
    Ok(exe)
}

#[cfg(test)]
mod minimax_code_local_import_tests {
    use super::*;
    use serde_json::json;

    struct TempHome(PathBuf);

    impl TempHome {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "cockpit-minimax-code-home-{}-{}",
                std::process::id(),
                unique
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_fake_auth_file(home: &Path) -> PathBuf {
        let dir = home
            .join(".minimax")
            .join("auth")
            .join("prod")
            .join("cn")
            .join("mcode-public");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("auth.json");
        let body = json!({
            "schemaVersion": 1,
            "records": {
                "com.minimax.mcode.oauth.prod.cn\u{0}abc123": {
                    "accessToken": "mmoat_test_token",
                    "refreshToken": "mmort_test_refresh",
                    "tokenType": "Bearer",
                    "clientId": "mcode-public",
                    "scopes": ["agent.default"],
                    "audience": "agent-backend",
                    "expiresAtMs": 1792767948636_i64,
                    "generation": 31
                }
            }
        });
        fs::write(&file, body.to_string()).unwrap();
        file
    }

    #[test]
    fn read_auth_file_returns_first_non_empty_record() {
        let home = TempHome::new();
        let file = write_fake_auth_file(&home.0);
        let (id, region, rec, path) = read_minimax_code_auth_file(&file).expect("auth file should parse");
        assert_eq!(id, "abc123");
        assert_eq!(region, "cn");
        assert_eq!(
            rec.get("accessToken").and_then(|v| v.as_str()),
            Some("mmoat_test_token")
        );
        assert!(path.ends_with("auth.json"));
    }

    #[test]
    fn read_auth_file_parses_region_from_record_key() {
        let home = TempHome::new();
        let dir = home
            .0
            .join(".minimax")
            .join("auth")
            .join("prod")
            .join("en")
            .join("mcode-public");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("auth.json");
        let body = json!({
            "schemaVersion": 1,
            "records": {
                "com.minimax.mcode.oauth.prod.en\u{0}abc": {
                    "accessToken": "mmoat_en_token",
                    "refreshToken": "mmort_en_refresh",
                    "tokenType": "Bearer",
                    "clientId": "mcode-public",
                    "expiresAtMs": 1792767948636_i64
                }
            }
        });
        fs::write(&file, body.to_string()).unwrap();

        let (id, region, rec, _path) =
            read_minimax_code_auth_file(&file).expect("auth file should parse");
        assert_eq!(id, "abc");
        assert_eq!(region, "en");
        assert_eq!(
            rec.get("accessToken").and_then(|v| v.as_str()),
            Some("mmoat_en_token")
        );
    }

    #[test]
    fn collect_auth_files_finds_nested_auth_json() {
        let home = TempHome::new();
        let file = write_fake_auth_file(&home.0);
        let mut out = Vec::new();
        collect_auth_files(&home.0.join(".minimax").join("auth"), &mut out, 0);
        assert!(
            out.iter().any(|found| found == &file),
            "expected to find {:?}, got {:?}",
            file,
            out
        );
    }
}

#[cfg(test)]
mod minimax_code_switch_tests {
    use super::*;
    use serde_json::json;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "cockpit-minimax-code-switch-{}-{}",
                std::process::id(),
                unique
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fake_account(
        id: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<i64>,
        auth_file: &Path,
    ) -> MinimaxCodeAccount {
        MinimaxCodeAccount {
            id: id.to_string(),
            email: format!("{id}@example.com"),
            uid: None,
            nickname: None,
            tags: None,
            access_token: access_token.to_string(),
            refresh_token: refresh_token.map(|value| value.to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at,
            region: Some("cn".to_string()),
            auth_file: Some(auth_file.to_string_lossy().to_string()),
            plan_type: None,
            quota_raw: None,
            auth_raw: None,
            profile_raw: None,
            status: Some("normal".to_string()),
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            last_checkin_time: None,
            checkin_streak: None,
            checkin_rewards: None,
            created_at: 1,
            last_used: 1,
        }
    }

    fn write_two_record_auth_file(dir: &Path) -> PathBuf {
        let file = dir.join("auth.json");
        let body = json!({
            "schemaVersion": 1,
            "records": {
                "com.minimax.mcode.oauth.prod.cn\u{0}account_a": {
                    "accessToken": "mmoat_account_a",
                    "refreshToken": "mmort_account_a",
                    "tokenType": "Bearer",
                    "clientId": "mcode-public",
                    "scopes": ["agent.default"],
                    "audience": "agent-backend",
                    "expiresAtMs": 1792767948636_i64,
                    "generation": 7
                },
                "com.minimax.mcode.oauth.prod.cn\u{0}account_b": {
                    "accessToken": "mmoat_account_b",
                    "refreshToken": "mmort_account_b",
                    "tokenType": "Bearer",
                    "clientId": "mcode-public",
                    "scopes": ["agent.default"],
                    "audience": "agent-backend",
                    "expiresAtMs": 1792767948636_i64,
                    "generation": 9
                }
            }
        });
        fs::write(&file, body.to_string()).unwrap();
        file
    }

    fn read_records(file: &Path) -> serde_json::Map<String, Value> {
        let raw = fs::read_to_string(file).unwrap();
        let parsed: Value = serde_json::from_str(&raw).unwrap();
        parsed
            .get("records")
            .and_then(|value| value.as_object())
            .cloned()
            .unwrap()
    }

    #[test]
    fn switch_minimax_code_account_rewrites_single_target_record() {
        let tmp = TempDir::new();
        let auth_file = write_two_record_auth_file(&tmp.0);
        let cockpit_bak = auth_file.with_extension("json.cockpit.bak");

        // First switch: account_b.
        let account_b = fake_account(
            "account_b",
            "mmoat_target_token",
            Some("mmort_target_refresh"),
            Some(1792767948636_i64),
            &auth_file,
        );
        let returned = switch_minimax_code_account(&account_b).expect("switch should succeed");
        assert_eq!(returned, auth_file);

        let records = read_records(&auth_file);
        assert_eq!(records.len(), 1, "only the target record should remain");

        let (key, record) = records.iter().next().unwrap();
        assert_eq!(
            key,
            "com.minimax.mcode.oauth.prod.cn\u{0}account_b",
            "record key must reuse the existing prefix and target account id"
        );
        assert_eq!(
            record.get("accessToken").and_then(|v| v.as_str()),
            Some("mmoat_target_token")
        );
        assert_eq!(
            record.get("refreshToken").and_then(|v| v.as_str()),
            Some("mmort_target_refresh")
        );
        // generation bumped from the reused record (9 -> 10).
        assert_eq!(record.get("generation").and_then(|v| v.as_i64()), Some(10));

        assert!(
            cockpit_bak.is_file(),
            "first-state backup should exist at {cockpit_bak:?}"
        );
        assert_eq!(
            read_records(&cockpit_bak).len(),
            2,
            "backup must preserve the original two records"
        );
        // The atomic writer must not create its own `.bak` and clobber the first-state backup.
        assert!(
            !auth_file.with_extension("json.bak").exists(),
            "writer-owned .bak must not be created"
        );

        // Second switch: account_a. The first-state backup must survive untouched.
        let account_a = fake_account(
            "account_a",
            "mmoat_switch_two",
            Some("mmort_switch_two"),
            None,
            &auth_file,
        );
        switch_minimax_code_account(&account_a).expect("second switch should succeed");

        let records = read_records(&auth_file);
        assert_eq!(records.len(), 1);
        let (key, record) = records.iter().next().unwrap();
        assert_eq!(key, "com.minimax.mcode.oauth.prod.cn\u{0}account_a");
        assert_eq!(
            record.get("accessToken").and_then(|v| v.as_str()),
            Some("mmoat_switch_two")
        );

        let backup_records = read_records(&cockpit_bak);
        assert_eq!(
            backup_records.len(),
            2,
            "a later switch must not overwrite the first-state backup"
        );
        assert!(backup_records.contains_key("com.minimax.mcode.oauth.prod.cn\u{0}account_b"));
        assert_eq!(
            backup_records
                .get("com.minimax.mcode.oauth.prod.cn\u{0}account_a")
                .and_then(|item| item.get("accessToken"))
                .and_then(|value| value.as_str()),
            Some("mmoat_account_a"),
            "backup still holds the original account_a token"
        );
    }

    #[test]
    fn switch_rejects_non_object_auth_file_without_panicking() {
        let tmp = TempDir::new();
        let auth_file = tmp.0.join("auth.json");
        fs::write(&auth_file, "[1, 2, 3]").unwrap();
        let account = fake_account("account_b", "mmoat_target", None, None, &auth_file);
        let err = switch_minimax_code_account(&account).expect_err("array JSON must be rejected");
        assert_eq!(err, "MiniMax Code 登录态文件格式非法");
    }

    #[test]
    fn switch_derives_prefix_from_another_record_when_target_absent() {
        let tmp = TempDir::new();
        let auth_file = tmp.0.join("auth.json");
        // Only an `en` record exists, and it is not the target account.
        let body = json!({
            "schemaVersion": 1,
            "records": {
                "com.minimax.mcode.oauth.prod.en\u{0}account_x": {
                    "accessToken": "mmoat_x",
                    "refreshToken": "mmort_x",
                    "tokenType": "Bearer",
                    "clientId": "mcode-public",
                    "expiresAtMs": 1792767948636_i64,
                    "generation": 3
                }
            }
        });
        fs::write(&auth_file, body.to_string()).unwrap();

        let account =
            fake_account("account_missing", "mmoat_new", Some("mmort_new"), None, &auth_file);
        switch_minimax_code_account(&account).expect("switch should succeed");

        let records = read_records(&auth_file);
        let (key, record) = records.iter().next().unwrap();
        assert_eq!(
            key,
            "com.minimax.mcode.oauth.prod.en\u{0}account_missing",
            "prefix must be derived from the other existing key, not hardcoded cn"
        );
        assert_eq!(
            record.get("accessToken").and_then(|v| v.as_str()),
            Some("mmoat_new")
        );
        assert_eq!(
            record.get("refreshToken").and_then(|v| v.as_str()),
            Some("mmort_new")
        );
        assert_eq!(
            record.get("clientId").and_then(|v| v.as_str()),
            Some("mcode-public"),
            "reused record shape should be preserved"
        );
        assert_eq!(record.get("generation").and_then(|v| v.as_i64()), Some(4));
        assert!(
            record.get("expiresAtMs").is_none(),
            "stale expiresAtMs should be removed when the account has none"
        );
    }

    #[test]
    fn switch_does_not_blank_existing_refresh_token_when_account_has_none() {
        let tmp = TempDir::new();
        let auth_file = write_two_record_auth_file(&tmp.0);
        let account = fake_account("account_b", "mmoat_new_access", None, None, &auth_file);
        switch_minimax_code_account(&account).expect("switch should succeed");

        let records = read_records(&auth_file);
        let record = records.values().next().unwrap();
        assert_eq!(
            record.get("accessToken").and_then(|v| v.as_str()),
            Some("mmoat_new_access")
        );
        assert_eq!(
            record.get("refreshToken").and_then(|v| v.as_str()),
            Some("mmort_account_b"),
            "a None refresh token must not blank the client's existing one"
        );
        assert!(
            record.get("expiresAtMs").is_none(),
            "stale expiresAtMs should be removed when the account has none"
        );
    }

    #[test]
    fn resolve_launch_path_prefers_existing_config_override() {
        let tmp = TempDir::new();
        let fake_exe = tmp.0.join("MiniMax Code.exe");
        fs::write(&fake_exe, b"fake").unwrap();
        let resolved = resolve_minimax_code_launch_path_from(&fake_exe.to_string_lossy(), &[])
            .expect("config override should win over candidates");
        assert_eq!(resolved, fake_exe);
    }

    #[test]
    fn resolve_launch_path_errors_when_nothing_found() {
        let err = resolve_minimax_code_launch_path_from("", &[])
            .expect_err("no configured path and no candidates must error");
        assert_eq!(err, "未找到 MiniMax Code 客户端可执行文件");
    }

    #[test]
    fn resolve_launch_path_falls_back_to_candidates() {
        let tmp = TempDir::new();
        let candidate = tmp.0.join("candidate.exe");
        fs::write(&candidate, b"fake").unwrap();
        let resolved = resolve_minimax_code_launch_path_from(
            r"C:\definitely\not\installed\MiniMax Code.exe",
            std::slice::from_ref(&candidate),
        )
        .expect("a missing config override must fall back to candidates");
        assert_eq!(resolved, candidate);
    }
}
