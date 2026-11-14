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
use crate::modules::{account, logger, minimax_code_instance, minimax_code_oauth, process};

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
    Ok(crate::modules::provider_account_index::storage_path::<MinimaxCodeAccountIndex>(&get_accounts_index_path()?)?
        .to_string_lossy().into_owned())
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

pub fn touch_last_used(account_id: &str) -> Result<(), String> {
    let _guard = MINIMAX_CODE_ACCOUNT_INDEX_LOCK.lock().map_err(|error| error.to_string())?;
    let mut account = load_account(account_id).ok_or_else(|| format!("账号不存在: {account_id}"))?;
    account.last_switched_at = Some(now_ts());
    save_account_file(&account)
}

pub fn record_client_switch(account_id: &str, auth_file: &Path) -> Result<(), String> {
    let _guard = MINIMAX_CODE_ACCOUNT_INDEX_LOCK.lock().map_err(|error| error.to_string())?;
    let mut account = load_account(account_id).ok_or_else(|| format!("账号不存在: {account_id}"))?;
    account.last_switched_at = Some(now_ts());
    account.auth_file = Some(auth_file.to_string_lossy().into_owned());
    save_account_file(&account)
}

fn save_account_file(account: &MinimaxCodeAccount) -> Result<(), String> {
    let path = resolve_account_file_path(account.id.as_str())?;
    crate::modules::secure_account_storage::write_account_preserving_switch_time(&path, "minimaxcode", account)
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
    load_account_index_checked().unwrap_or_else(|error| {
        logger::log_warn(&format!("[MinimaxCode Account] {error}"));
        MinimaxCodeAccountIndex::new()
    })
}

fn load_account_index_checked() -> Result<MinimaxCodeAccountIndex, String> {
    crate::modules::provider_account_index::checked(
        &get_accounts_index_path()?,
        crate::modules::provider_index_maintenance::schedule,
        repair_account_index_from_details,
        |index| index.accounts.is_empty(),
    )
}

fn save_account_index(index: &MinimaxCodeAccountIndex) -> Result<(), String> {
    crate::modules::provider_account_index::save(&get_accounts_index_path()?, index)
}

fn repair_account_index_from_details(reason: &str) -> Option<MinimaxCodeAccountIndex> {
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

    if let Err(error) = save_account_index(&index) {
        logger::log_warn(&format!("[MinimaxCode Account] 保存恢复索引失败: reason={reason}, error={error}"));
        return None;
    }
    logger::log_info(&format!(
        "[MinimaxCode Account] 已恢复账号索引: reason={reason}, accounts={}",
        index.accounts.len()
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
    save_account_and_summary(account)
}

fn save_account_and_summary(account: MinimaxCodeAccount) -> Result<MinimaxCodeAccount, String> {
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
    primary.last_switched_at = primary.last_switched_at.max(dup.last_switched_at);
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

pub(crate) fn resolve_current_account_id(accounts: &[MinimaxCodeAccount]) -> Option<String> {
    crate::modules::provider_current_state::resolve_existing_current_account_id(
        "minimaxcode",
        accounts.iter().map(|account| account.id.as_str()),
    )
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
    if let Some(quota) = payload.quota_raw {
        account.plan_type = quota.get("membership").and_then(|value| value.get("plan_name"))
            .and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned);
        account.quota_raw = Some(quota);
        if payload.status.as_deref() != Some("quota_error") {
            account.usage_updated_at = Some(now_ts());
            account.quota_query_last_error = None;
            account.quota_query_last_error_at = None;
        }
    }
    if payload.status.as_deref() == Some("quota_error") {
        account.quota_query_last_error = payload.status_reason.clone();
        account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
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

    // Local import has no uid and its email is not a real address, so fall back to
    // the payload's explicit id (the client's per-data-dir record key). Without it,
    // every locally imported account would collapse onto the same id.
    let identity_seed = incoming_uid
        .clone()
        .or_else(|| incoming_email.clone())
        .or_else(|| payload.id.clone())
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
        last_switched_at: None,
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

/// Whether the credentials in the client's `auth.json` are fresher than ours.
///
/// MiniMax rotates refresh tokens on every use, so adopting the file's
/// credentials when ours are newer resurrects a refresh token we already spent.
/// The next refresh then fails with `invalid_grant` and we are left holding the
/// stale access token the file had, which the server has since invalidated — the
/// account "loses validity after one refresh".
fn file_credentials_are_fresh(file_expires_at: Option<i64>, account_expires_at: Option<i64>) -> bool {
    match (file_expires_at, account_expires_at) {
        (Some(file), Some(current)) => file > current,
        (Some(file), None) => file > 0,
        (None, _) => false,
    }
}

fn client_auth_files(account: &MinimaxCodeAccount, inspect_all: bool) -> (Vec<PathBuf>, bool) {
    let mut paths = account
        .auth_file
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let mut complete = true;
    let mut roots = Vec::new();
    let mut include_default = inspect_all;
    match minimax_code_instance::load_instance_store() {
        Ok(store) => {
            include_default |=
                store.default_settings.bind_account_id.as_deref() == Some(account.id.as_str());
            roots.extend(
                store
                    .instances
                    .into_iter()
                    .filter(|instance| {
                        inspect_all
                            || instance.bind_account_id.as_deref() == Some(account.id.as_str())
                    })
                    .map(|instance| PathBuf::from(instance.user_data_dir)),
            );
        }
        Err(error) => {
            complete = false;
            logger::log_warn(&format!("[MiniMax Code Refresh] 无法读取实例绑定: {error}"));
        }
    }
    if include_default {
        match minimax_code_instance::get_default_minimax_code_data_dir() {
            Ok(root) => roots.push(root),
            Err(_) => complete = false,
        }
    }
    if inspect_all && minimax_code_instance::explicit_minimax_code_data_dir().is_some() {
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".minimax"));
        }
    }
    for root in roots {
        let scope = minimax_code_instance::resolve_minimax_code_auth_scope(
            &root,
            account.region.as_deref(),
        );
        let path =
            minimax_code_instance::minimax_code_namespace_home(&root, &scope).join("auth.json");
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    (paths, complete)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClientCredentialOwner {
    Same,
    Other,
    Unknown,
}

impl ClientCredentialOwner {
    fn can_rotate(self, client_confirmed_stopped: impl FnOnce() -> bool) -> bool {
        match self {
            Self::Same => false,
            Self::Other => true,
            Self::Unknown => client_confirmed_stopped(),
        }
    }
}

fn client_tokens_overlap(account: &MinimaxCodeAccount, record: &Value) -> bool {
    let access = record.get("accessToken").and_then(Value::as_str);
    let refresh = record.get("refreshToken").and_then(Value::as_str);
    access.is_some_and(|value| !value.is_empty() && value == account.access_token)
        || refresh.is_some_and(|value| {
            !value.is_empty() && Some(value) == account.refresh_token.as_deref()
        })
}

fn client_identity_owner(
    account: &MinimaxCodeAccount,
    uid: Option<&str>,
    email: &str,
) -> ClientCredentialOwner {
    let ids = normalize_identity(account.uid.as_deref()).zip(normalize_identity(uid));
    let emails =
        normalize_email_identity(Some(&account.email)).zip(normalize_email_identity(Some(email)));
    match ids.or(emails) {
        Some((stored, incoming)) if stored == incoming => ClientCredentialOwner::Same,
        Some(_) => ClientCredentialOwner::Other,
        None => ClientCredentialOwner::Unknown,
    }
}

fn adopt_client_credentials(account: &mut MinimaxCodeAccount, record: &Value, auth_file: String) {
    let expires_at = record.get("expiresAtMs").and_then(Value::as_i64);
    if !file_credentials_are_fresh(expires_at, account.expires_at) {
        return;
    }
    let access = normalize_non_empty(record.get("accessToken").and_then(Value::as_str));
    let refresh = normalize_non_empty(record.get("refreshToken").and_then(Value::as_str));
    if let Some((access, refresh)) = access.zip(refresh) {
        account.access_token = access;
        account.refresh_token = Some(refresh);
        account.expires_at = expires_at;
        account.auth_file = Some(auth_file);
        if let Some(token_type) =
            normalize_non_empty(record.get("tokenType").and_then(Value::as_str))
        {
            account.token_type = Some(token_type);
        }
    }
}

fn read_client_auth_record_for_refresh(path: &Path) -> Option<(String, Value)> {
    let raw = fs::read_to_string(path).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    let records = parsed.get("records")?.as_object()?;
    let namespace = path.parent()?;
    let active = scope_from_namespace_home(namespace)
        .zip(namespace.ancestors().nth(3))
        .and_then(|(scope, auth_home)| {
            let key = minimax_code_record_key_for_auth_home(
                &auth_home.to_string_lossy(),
                &scope.service_prefix(),
            );
            records.get_key_value(&key)
        });
    // A legacy file with one record is unambiguous. If several records exist,
    // use the client's active key or leave ownership unknown.
    let (key, record) = active.or_else(|| {
        if records.len() == 1 {
            records.iter().next()
        } else {
            None
        }
    })?;
    let service = key.split_once('\0')?.0;
    let region = service.rsplit_once('.')?.1;
    if !minimax_code_instance::MinimaxCodeAuthScope::is_valid_region(region) {
        return None;
    }
    Some((region.to_string(), record.clone()))
}

async fn sync_client_credentials(
    account: &mut MinimaxCodeAccount,
    region: &str,
    inspect_all: bool,
    mut read_record: impl FnMut(&Path) -> Option<(String, Value)>,
) -> ClientCredentialOwner {
    let (paths, complete) = client_auth_files(account, inspect_all);
    let client = minimax_code_oauth::build_client().ok();
    let mut has_shared_credentials = false;
    let mut has_other_account = false;
    let mut has_unknown_account = !complete;

    for path in paths {
        let Some((file_region, record)) = read_record(&path) else {
            has_unknown_account |= !matches!(path.try_exists(), Ok(false));
            continue;
        };
        let file_is_fresh = file_credentials_are_fresh(
            record.get("expiresAtMs").and_then(Value::as_i64),
            account.expires_at,
        );
        let owner = if client_tokens_overlap(account, &record) {
            ClientCredentialOwner::Same
        } else if file_region != region {
            ClientCredentialOwner::Other
        } else if inspect_all || file_is_fresh {
            let access = record
                .get("accessToken")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty());
            if let Some((client, access)) = client.as_ref().zip(access) {
                match minimax_code_oauth::fetch_user_info(client, access, region).await {
                    Ok((uid, _, email)) => client_identity_owner(account, uid.as_deref(), &email),
                    Err(_) => ClientCredentialOwner::Unknown,
                }
            } else {
                ClientCredentialOwner::Unknown
            }
        } else {
            ClientCredentialOwner::Unknown
        };

        match owner {
            ClientCredentialOwner::Same => {
                has_shared_credentials = true;
                if file_region == region {
                    adopt_client_credentials(account, &record, path.to_string_lossy().into_owned());
                }
            }
            ClientCredentialOwner::Other => has_other_account = true,
            ClientCredentialOwner::Unknown => has_unknown_account = true,
        }
    }

    if has_shared_credentials {
        ClientCredentialOwner::Same
    } else if has_other_account && !has_unknown_account {
        ClientCredentialOwner::Other
    } else {
        ClientCredentialOwner::Unknown
    }
}

fn update_account_after_refresh(
    expected: &MinimaxCodeAccount,
    update: impl FnOnce(&mut MinimaxCodeAccount),
) -> Result<MinimaxCodeAccount, String> {
    let _lock = MINIMAX_CODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|error| error.to_string())?;
    let mut current = load_account(&expected.id).ok_or_else(|| "账号不存在".to_string())?;
    if current.access_token != expected.access_token
        || current.refresh_token != expected.refresh_token
    {
        return Err(credential_refresh_message("minimaxCode.credentialRefresh.credentialsChanged"));
    }
    update(&mut current);
    save_account_and_summary(current)
}

fn persist_changed_credentials(
    stored: MinimaxCodeAccount,
    account: &MinimaxCodeAccount,
) -> Result<MinimaxCodeAccount, String> {
    if account.access_token == stored.access_token
        && account.refresh_token == stored.refresh_token
        && account.expires_at == stored.expires_at
    {
        return Ok(stored);
    }
    update_account_after_refresh(&stored, |current| {
        current.access_token = account.access_token.clone();
        current.refresh_token = account.refresh_token.clone();
        current.token_type = account.token_type.clone();
        current.expires_at = account.expires_at;
        current.auth_file = account.auth_file.clone();
    })
}

pub(crate) struct PreparedClientSwitch {
    pub(crate) account: MinimaxCodeAccount,
    checked_records: HashMap<PathBuf, (String, Value)>,
    _credential_guard: tokio::sync::OwnedMutexGuard<()>,
}

fn changed_client_auth_record(
    path: &Path,
    checked_records: &HashMap<PathBuf, (String, Value)>,
) -> Option<(String, Value)> {
    let record = read_client_auth_record_for_refresh(path)?;
    (checked_records.get(path) != Some(&record)).then_some(record)
}

/// Wait for any background exchange and validate credentials while the client is open.
pub(crate) async fn prepare_client_switch(
    account_id: &str,
) -> Result<PreparedClientSwitch, String> {
    let credential_guard = lock_account_credentials(account_id).await?;
    let stored = load_account(account_id).ok_or_else(|| format!("绑定账号不存在: {account_id}"))?;
    let mut account = stored.clone();
    let region = account.region.clone().unwrap_or_else(|| "cn".to_string());
    let mut checked_records = HashMap::new();
    sync_client_credentials(&mut account, &region, false, |path| {
        let record = read_client_auth_record_for_refresh(path)?;
        checked_records.insert(path.to_path_buf(), record.clone());
        Some(record)
    })
    .await;
    Ok(PreparedClientSwitch {
        account: persist_changed_credentials(stored, &account)?,
        checked_records,
        _credential_guard: credential_guard,
    })
}

/// Check the last on-disk snapshot after exit. The client may have rotated its
/// token while shutting down. Only a changed record can need network validation.
pub(crate) async fn recheck_client_switch(
    prepared: &mut PreparedClientSwitch,
) -> Result<(), String> {
    let stored = load_account(&prepared.account.id).ok_or_else(|| "账号不存在".to_string())?;
    let mut account = stored.clone();
    let region = account.region.clone().unwrap_or_else(|| "cn".to_string());
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        sync_client_credentials(&mut account, &region, false, |path| {
            changed_client_auth_record(path, &prepared.checked_records)
        }),
    )
    .await
    .map_err(|_| credential_refresh_message("minimaxCode.credentialRefresh.credentialsChanged"))?;
    prepared.account = persist_changed_credentials(stored, &account)?;
    Ok(())
}

fn credential_refresh_message(key: &str) -> String {
    super::i18n::translate(&super::config::get_user_config().language, key, &[])
}

async fn refresh_account_token_once(
    account_id: &str,
    rejected_access: Option<&str>,
    exchange_attempted: &mut bool,
) -> Result<MinimaxCodeAccount, String> {
    let started_at = Instant::now();
    let stored = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    let mut account = stored.clone();
    logger::log_info(&format!(
        "[MiniMax Code Refresh] 开始刷新账号:id={}, email={}",
        account.id, account.email
    ));

    let region = account.region.clone().unwrap_or_else(|| "cn".to_string());

    let refresh_before = chrono::Utc::now().timestamp_millis() + 60_000;
    let needs_exchange = |account: &MinimaxCodeAccount| {
        rejected_access.map_or_else(
            || {
                account
                    .expires_at
                    .is_none_or(|expires| expires <= refresh_before)
            },
            |rejected| rejected == account.access_token,
        )
    };
    let inspect_all = !*exchange_attempted
        && (rejected_access.is_some()
            || (needs_exchange(&account) && account.refresh_token.is_some()));
    let owner = sync_client_credentials(
        &mut account,
        &region,
        inspect_all,
        read_client_auth_record_for_refresh,
    )
    .await;
    if owner == ClientCredentialOwner::Same
        && rejected_access.is_some_and(|rejected| rejected == account.access_token)
        && !process::minimax_code_client_confirmed_stopped()
    {
        // The official client can be between rejecting the old token and writing
        // its replacement. Wait once, then only retry with a changed access token.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        sync_client_credentials(
            &mut account,
            &region,
            true,
            read_client_auth_record_for_refresh,
        )
        .await;
    }
    let mut blocked_reason = None;

    // Both a pre-expiry refresh and a 401 retry can consume the client's refresh
    // token. Give the client ownership of any login it shares with Cockpit.
    if let Some(refresh_token) = account
        .refresh_token
        .as_deref()
        .filter(|token| !token.is_empty() && inspect_all && needs_exchange(&account))
    {
        if owner.can_rotate(process::minimax_code_client_confirmed_stopped) {
            *exchange_attempted = true;
            match minimax_code_oauth::refresh_token(refresh_token, &region).await {
                Ok(token_data) => {
                    let access =
                        normalize_non_empty(token_data.get("access_token").and_then(Value::as_str));
                    if let Some(access) = access {
                        account.access_token = access;
                        if let Some(refresh) = normalize_non_empty(
                            token_data.get("refresh_token").and_then(Value::as_str),
                        ) {
                            account.refresh_token = Some(refresh);
                        }
                        if let Some(token_type) = normalize_non_empty(
                            token_data.get("token_type").and_then(Value::as_str),
                        ) {
                            account.token_type = Some(token_type);
                        }
                        account.expires_at = token_data
                            .get("expires_in")
                            .and_then(Value::as_i64)
                            .map(minimax_code_oauth::expires_at_from_expires_in);
                    } else {
                        blocked_reason = Some(credential_refresh_message("minimaxCode.credentialRefresh.missingAccessToken"));
                    }
                }
                Err(err) => {
                    blocked_reason = Some(if err.contains("invalid_grant") {
                        credential_refresh_message("minimaxCode.credentialRefresh.authorizeAgain")
                    } else {
                        err
                    });
                }
            }
        } else {
            blocked_reason = Some(credential_refresh_message(
                "minimaxCode.credentialRefresh.signInClient",
            ));
        }
    }

    // A successful exchange invalidates the previous refresh token immediately.
    // Persist the pair before fetching profile and usage, which can fail independently.
    let stored = persist_changed_credentials(stored, &account)?;
    if rejected_access.is_some_and(|rejected| rejected == account.access_token) {
        if let Some(reason) = blocked_reason.as_ref() {
            update_account_after_refresh(&stored, |current| {
                current.quota_query_last_error = Some(reason.clone());
                current.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
                current.status = Some("quota_error".into());
                current.status_reason = Some(reason.clone());
            })?;
            return Err(reason.clone());
        }
        return Ok(stored);
    }

    // (c) Refresh quota and store the raw body on the account.
    let client = minimax_code_oauth::build_client()?;
    let mut quota_refresh_error: Option<String> = None;
    match minimax_code_oauth::fetch_user_info(&client, &account.access_token, &region).await {
        Ok((uid, nickname, email)) => {
            if account
                .uid
                .as_deref()
                .zip(uid.as_deref())
                .is_some_and(|(stored, incoming)| stored != incoming)
            {
                return Err("MiniMax 返回的账号身份与当前账号不一致，未覆盖账号数据".into());
            }
            account.email = minimax_code_oauth::resolve_display_name(
                &email,
                nickname.as_deref(),
                uid.as_deref(),
            );
            account.uid = uid.or(account.uid);
            account.nickname = nickname;
        }
        Err(error) => quota_refresh_error = Some(error),
    }
    let quota_raw =
        match minimax_code_oauth::fetch_usage_snapshot(&client, &account.access_token, &region)
            .await
        {
            Ok(body) => Some(body),
            Err(err) => {
                logger::log_warn(&format!(
                    "[MiniMax Code Refresh] 刷新 credit/details 失败:{}",
                    err
                ));
                if is_unauthorized(&err)
                    || !quota_refresh_error.as_deref().is_some_and(is_unauthorized)
                {
                    quota_refresh_error = Some(err);
                }
                None
            }
        };

    let unauthorized = quota_refresh_error.as_deref().is_some_and(is_unauthorized);
    if unauthorized && *exchange_attempted && blocked_reason.is_some() {
        quota_refresh_error = blocked_reason;
    }
    let payload = MinimaxCodeOAuthCompletePayload {
        email: account.email.clone(),
        id: Some(account.id.clone()),
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
        status: Some(
            if quota_refresh_error.is_some() {
                "quota_error"
            } else {
                "normal"
            }
            .into(),
        ),
        status_reason: quota_refresh_error,
    };
    let updated = update_account_after_refresh(&stored, |current| {
        apply_payload(current, payload);
    })?;
    logger::log_info(&format!(
        "[MiniMax Code Refresh] 刷新完成:id={}, email={}, elapsed={}ms",
        updated.id,
        updated.email,
        started_at.elapsed().as_millis()
    ));
    Ok(updated)
}

fn is_unauthorized(error: &str) -> bool {
    error.contains("HTTP 401") || error.contains("invalid access token")
}

async fn lock_account_credentials(
    account_id: &str,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    static LOCKS: std::sync::LazyLock<
        Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    > = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    let account_id = normalize_account_id(account_id)?;
    let lock = LOCKS
        .lock()
        .map_err(|error| error.to_string())?
        .entry(account_id)
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    Ok(lock.lock_owned().await)
}

pub async fn refresh_account_token(account_id: &str) -> Result<MinimaxCodeAccount, String> {
    let _guard = lock_account_credentials(account_id).await?;
    let mut exchange_attempted = false;
    let account = refresh_account_token_once(account_id, None, &mut exchange_attempted).await?;
    // A token may be rejected before expiry. Re-read the client's credentials
    // before allowing a single recovery exchange for an independent account.
    if !exchange_attempted
        && account
            .quota_query_last_error
            .as_deref()
            .is_some_and(is_unauthorized)
    {
        return refresh_account_token_once(
            account_id,
            Some(&account.access_token),
            &mut exchange_attempted,
        )
        .await;
    }
    Ok(account)
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
    let _lock = MINIMAX_CODE_ACCOUNT_INDEX_LOCK.lock().map_err(|error| error.to_string())?;
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    account.tags = Some(tags);
    account.last_used = now_ts();
    save_account_and_summary(account)
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
        last_switched_at: None,
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

    // Preserve an explicit id when the imported payload carries one (local export).
    let explicit_id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(MinimaxCodeOAuthCompletePayload {
        email,
        id: explicit_id,
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
    let _lock = MINIMAX_CODE_ACCOUNT_INDEX_LOCK.lock().map_err(|error| error.to_string())?;
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

    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from(
            "/Applications/MiniMax Code.app/Contents/MacOS/MiniMax Code",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/minimax-code"));
        candidates.push(PathBuf::from("/usr/local/bin/minimax-code"));
    }

    #[cfg(target_os = "windows")]
    {
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
    }

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
        #[cfg(target_os = "macos")]
        if path.is_dir() && path.extension().is_some_and(|ext| ext == "app") {
            let executable = path.join("Contents/MacOS/MiniMax Code");
            if executable.is_file() { return Ok(executable); }
        }
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

/// The MiniMax client's credential record key is
/// `{service}\0{base64url(sha256("<dataDir>/auth\0mcode-public"))}`.
///
/// The account component is **not** the Cockpit account id — it is a hash of the
/// namespace's auth home, so it differs per data root (i.e. per managed instance).
/// Verified against the installed client (`app.asar` `createAuthNamespace` +
/// `FileStore` `credentialKey`) and against the on-disk `auth.json` on this machine.
///
/// The hash input uses the OS path separator (`Path::join`), which on Windows yields
/// `C:\...\auth` — matching the client's `join(resolve(dataDir), 'auth')`.
#[cfg(test)]
pub fn minimax_code_record_key(data_dir: &str, service_prefix: &str) -> String {
    minimax_code_record_key_for_auth_home(
        Path::new(data_dir).join("auth").to_string_lossy().as_ref(),
        service_prefix,
    )
}

/// `auth_home` = `<data root>/auth`，与客户端 `join(resolve(dataDir), 'auth')` 一致。
pub fn minimax_code_record_key_for_auth_home(
    auth_home: &str,
    service_prefix: &str,
) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(
        format!(
            "{}\u{0}{}",
            auth_home,
            crate::modules::minimax_code_instance::MINIMAX_CODE_OAUTH_CLIENT_ID
        )
        .as_bytes(),
    );
    let account_component = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(hasher.finalize());
    format!("{}\u{0}{}", service_prefix, account_component)
}

// An unknown expiry is not a lease valid until 2100. The official parser accepts
// zero; getAccessToken then refreshes this credential before using it.
const MINIMAX_CODE_UNKNOWN_EXPIRES_AT_MS: i64 = 0;

/// Switch the local MiniMax client to `account` by rewriting its `auth.json`.
///
/// The record key is recomputed from the target data root (see `minimax_code_record_key`),
/// and a matching `auth-state.json` is written alongside so the client's
/// `MCodeOAuthCore.getStatus()` accepts the credential instead of reporting `error`.
/// The pre-switch login state is preserved at `<file>.json.cockpit.bak` (only the first
/// backup is kept, never overwritten). Returns the auth file path.
pub fn switch_minimax_code_account(account: &MinimaxCodeAccount) -> Result<PathBuf, String> {
    let root = crate::modules::minimax_code_instance::get_default_minimax_code_data_dir()?;
    switch_minimax_code_account_impl(account, Some(&root))
}

/// Switch a **managed instance** to `account` by rewriting the `auth.json` inside
/// `data_dir` (the instance's own `MINIMAX_DATA_DIR`).
///
/// The MiniMax client derives its credential namespace from the data root:
/// `<data root>/auth/<buildEnv>/<region>/<clientId>/auth.json`, and its record key
/// hashes the same auth home. Passing a per-instance data root therefore gives each
/// managed instance its own login state — the foundation of MiniMax Code multi-instance
/// support.
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
    use crate::modules::minimax_code_instance as instance;

    // `parseStoredCredential` rejects a record without a non-empty refreshToken.
    // Refuse loudly instead of writing a record the client would discard as corrupt.
    let refresh_token = account
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "该账号缺少 refreshToken，无法切换".to_string())?
        .to_string();

    let (auth_file, data_root, namespace_home) = match data_dir {
        // Managed instance: always target the instance's own namespace.
        Some(root) => {
            let scope = instance::resolve_minimax_code_auth_scope(root, account.region.as_deref());
            let namespace_home = instance::minimax_code_namespace_home(root, &scope);
            (namespace_home.join("auth.json"), root.to_path_buf(), namespace_home)
        }
        None => {
            let auth_file = account
                .auth_file
                .as_ref()
                .map(PathBuf::from)
                .filter(|p| p.is_file())
                .or_else(|| find_minimax_code_auth_files().into_iter().next());
            let Some(auth_file) = auth_file else {
                let root = instance::get_default_minimax_code_data_dir()?;
                return switch_minimax_code_account_impl(account, Some(&root));
            };
            // 全局实例：沿用现有命名空间。data root 从
            // `<data root>/auth/<buildEnv>/<region>/<clientId>/auth.json` 反推。
            let namespace_home = auth_file
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            let data_root = namespace_home
                .parent()
                .and_then(|region_dir| region_dir.parent())
                .and_then(|env_dir| env_dir.parent())
                .and_then(|auth| auth.parent())
                .map(PathBuf::from)
                .unwrap_or_else(|| namespace_home.clone());
            (auth_file, data_root, namespace_home)
        }
    };

    if let Some(parent) = auth_file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 MiniMax Code 登录态目录失败: {e}"))?;
    }

    let raw = match std::fs::read_to_string(&auth_file) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
            "{\"schemaVersion\":1,\"records\":{}}".to_string(),
        Err(error) => return Err(format!("读取 MiniMax Code 登录态失败: {error}")),
    };
    let mut parsed: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("MiniMax Code 登录态文件格式非法: {error}"))?;
    if !parsed.is_object() {
        return Err("MiniMax Code 登录态文件格式非法".to_string());
    }

    // Preserve the client's current login state before overwriting it. The backup
    // uses a distinct `.cockpit.bak` path and is written only once, so a later
    // switch can never clobber the first (oldest) login state. The paired
    // `auth-state.json` is backed up alongside so a restore can be consistent.
    let bak = auth_file.with_extension("json.cockpit.bak");
    if !bak.exists() {
        let _ = std::fs::copy(&auth_file, &bak);
        if let Some(state_path) = auth_state_path(&namespace_home) {
            if state_path.is_file() {
                let _ = std::fs::copy(&state_path, state_path.with_extension("json.cockpit.bak"));
            }
        }
    }

    // 客户端的 record key 由 data root 决定，必须重新计算，不能沿用账号 id。
    // `auth_home` 与客户端 `join(resolve(dataDir), 'auth')` 一致（OS 路径分隔符）。
    let auth_home = data_root.join("auth");
    // 优先按命名空间目录结构推断（与客户端实际使用的布局一致），
    // 再回退到 auth-state.json / auth.json 记录键前缀。
    let scope = scope_from_namespace_home(&namespace_home).unwrap_or_else(|| {
        instance::resolve_minimax_code_auth_scope(&data_root, account.region.as_deref())
    });
    let target_key = minimax_code_record_key_for_auth_home(
        &auth_home.to_string_lossy(),
        &scope.service_prefix(),
    );

    let mut record = serde_json::json!({
        "schemaVersion": 1,
        "tokenType": "Bearer",
        "clientId": instance::MINIMAX_CODE_OAUTH_CLIENT_ID,
        "scopes": instance::MINIMAX_CODE_OAUTH_SCOPES,
        "audience": instance::MINIMAX_CODE_OAUTH_AUDIENCE,
        "accessToken": account.access_token,
        "refreshToken": refresh_token,
        "expiresAtMs": account.expires_at.unwrap_or(MINIMAX_CODE_UNKNOWN_EXPIRES_AT_MS),
        "generation": 1,
    });
    // Identity belongs to the incoming credential. Never retain the previous
    // account's subject, accountId or authorization lease across a switch.
    if let Some(obj) = record.as_object_mut() {
        let generation = parsed.get("records").and_then(|records| records.get(&target_key))
            .and_then(|record| record.get("generation")).and_then(Value::as_i64).unwrap_or(1);
        obj.insert("generation".into(), serde_json::json!(generation + 1));
    }
    let generation = record
        .get("generation")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let expires_at_ms = record
        .get("expiresAtMs")
        .and_then(|v| v.as_i64())
        .unwrap_or(MINIMAX_CODE_UNKNOWN_EXPIRES_AT_MS);

    // Keep only the target record: the client's FileStore keys by the computed record key,
    // and `parsePayload` validates every record it finds.
    let mut new_records = serde_json::Map::new();
    new_records.insert(target_key, record);
    parsed["records"] = Value::Object(new_records);
    if parsed.get("schemaVersion").is_none() {
        parsed["schemaVersion"] = serde_json::json!(1);
    }

    // `write_bytes_atomic` replaces via temp-file + rename and creates no `.bak`,
    // so it never touches the first-state backup above.
    crate::modules::atomic_write::write_secret_string_atomic(
        &auth_file,
        &
        serde_json::to_string_pretty(&parsed)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写入 MiniMax Code 登录态失败: {e}"))?;

    write_minimax_code_auth_state(&namespace_home, &scope, generation, expires_at_ms)?;

    Ok(auth_file)
}

/// `<namespaceHome>/auth-state.json`。
fn auth_state_path(namespace_home: &Path) -> Option<PathBuf> {
    if namespace_home.as_os_str().is_empty() {
        return None;
    }
    Some(namespace_home.join("auth-state.json"))
}

/// 写 `auth-state.json`。
///
/// 客户端 `MCodeOAuthCore.getStatus()` 在 `status === "authenticated"` 时会取
/// `credentialStore.get(namespace.credentialKey)`，并要求
/// `credential.generation === state.generation`，否则直接返回 `error`。因此
/// `generation` 必须与 `auth.json` 里的记录一致。
///
/// 已存在 `auth-state.json` 时复用它的 `schemaVersion`/`buildEnv`/`region`（并沿用其
/// `expiresAtMs`），只把 `generation` 对齐到记录。
fn write_minimax_code_auth_state(
    namespace_home: &Path,
    scope: &crate::modules::minimax_code_instance::MinimaxCodeAuthScope,
    generation: i64,
    expires_at_ms: i64,
) -> Result<(), String> {
    use crate::modules::minimax_code_instance as instance;

    let state_path = auth_state_path(namespace_home)
        .ok_or_else(|| "无法确定 MiniMax Code auth-state.json 路径".to_string())?;
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 MiniMax Code 登录态目录失败: {e}"))?;
    }

    // 客户端 `parseState` 拒绝任何名字匹配 access/refresh token 等敏感模式的键。
    let state = serde_json::json!({
        "schemaVersion": 2,
        "status": "authenticated",
        "storeKind": "file",
        "clientId": instance::MINIMAX_CODE_OAUTH_CLIENT_ID,
        "scopes": instance::MINIMAX_CODE_OAUTH_SCOPES,
        "audience": instance::MINIMAX_CODE_OAUTH_AUDIENCE,
        "buildEnv": scope.build_env,
        "region": scope.region,
        "generation": generation,
        "expiresAtMs": expires_at_ms,
    });
    crate::modules::atomic_write::write_secret_string_atomic(
        &state_path,
        &
        serde_json::to_string_pretty(&state)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写入 MiniMax Code auth-state.json 失败: {e}"))?;
    Ok(())
}

/// 从命名空间目录 `<data root>/auth/<buildEnv>/<region>/<clientId>` 读取 buildEnv/region。
/// 与客户端 `createAuthNamespace` 的布局一一对应。
fn scope_from_namespace_home(
    namespace_home: &Path,
) -> Option<crate::modules::minimax_code_instance::MinimaxCodeAuthScope> {
    use crate::modules::minimax_code_instance as instance;
    let region_dir = namespace_home.parent()?;
    let env_dir = region_dir.parent()?;
    let build_env = env_dir.file_name().and_then(|name| name.to_str())?;
    let region = region_dir.file_name().and_then(|name| name.to_str())?;
    if !instance::MinimaxCodeAuthScope::is_valid_build_env(build_env)
        || !instance::MinimaxCodeAuthScope::is_valid_region(region)
    {
        return None;
    }
    Some(instance::MinimaxCodeAuthScope {
        build_env: build_env.to_string(),
        region: region.to_string(),
    })
}

/// Ask every running MiniMax Code client (matched by exact exe path) to exit.
///
/// This helper is only for an explicit application-wide close. Targeted
/// instance operations use process::close_minimax_code_instances instead.
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

#[cfg(test)]
mod minimax_code_refresh_freshness_tests {
    use super::*;
    use serde_json::json;

    fn account() -> MinimaxCodeAccount {
        serde_json::from_value(json!({
            "id": "test-account", "email": "test@example.com", "uid": "uid-1",
            "access_token": "stored-access", "refresh_token": "stored-refresh",
            "expires_at": 1_000, "created_at": 1, "last_used": 1,
        }))
        .unwrap()
    }

    #[test]
    fn adopts_client_credentials_only_when_they_are_fresher() {
        // The client renewed and wrote a later expiry -> adopt.
        assert!(file_credentials_are_fresh(Some(2_000), Some(1_000)));
        // An equal expiry cannot establish that a different token pair is newer.
        assert!(!file_credentials_are_fresh(Some(1_000), Some(1_000)));
        // Ours are newer: we already refreshed past what the file holds, so the
        // file's refresh token is spent and its access token is stale.
        // Regression: adopting here caused a 401 after one refresh.
        assert!(!file_credentials_are_fresh(Some(1_000), Some(2_000)));
    }

    #[test]
    fn does_not_replace_credentials_with_an_unknown_expiry() {
        assert!(file_credentials_are_fresh(Some(2_000), None));
        assert!(!file_credentials_are_fresh(Some(0), None));
        assert!(!file_credentials_are_fresh(None, Some(2_000)));
        assert!(!file_credentials_are_fresh(None, None));
    }

    #[test]
    fn shared_refresh_token_is_protected_even_when_the_access_token_differs() {
        let record = json!({"accessToken": "client-access", "refreshToken": "stored-refresh"});
        assert!(client_tokens_overlap(&account(), &record));
        assert!(!ClientCredentialOwner::Same.can_rotate(|| true));
        assert!(!ClientCredentialOwner::Same.can_rotate(|| false));
    }

    #[test]
    fn only_a_verified_other_account_or_confirmed_exit_allows_rotation() {
        let account = account();
        let other = client_identity_owner(&account, Some("uid-2"), "test@example.com");
        let unknown = client_identity_owner(&account, None, "");
        assert!(other.can_rotate(|| false));
        assert!(!unknown.can_rotate(|| false));
        assert!(unknown.can_rotate(|| true));
        assert!(!client_identity_owner(&account, Some("uid-1"), "").can_rotate(|| true));
    }

    #[test]
    fn adopts_the_new_client_pair_together_and_ignores_partial_records() {
        let mut account = account();
        adopt_client_credentials(
            &mut account,
            &json!({
                "accessToken": "partial-access", "expiresAtMs": 3_000,
            }),
            "auth.json".into(),
        );
        assert_eq!(account.access_token, "stored-access");
        assert_eq!(account.expires_at, Some(1_000));

        adopt_client_credentials(
            &mut account,
            &json!({
                "accessToken": "client-access", "refreshToken": "client-refresh", "expiresAtMs": 2_000,
            }),
            "auth.json".into(),
        );
        assert_eq!(account.access_token, "client-access");
        assert_eq!(account.refresh_token.as_deref(), Some("client-refresh"));
        assert_eq!(account.expires_at, Some(2_000));
    }

    #[test]
    fn refresh_selects_the_clients_active_record_in_a_multi_record_file() {
        let tmp = tempfile::tempdir().unwrap();
        let auth_home = tmp.path().join("auth");
        let namespace = auth_home.join("prod").join("cn").join("mcode-public");
        fs::create_dir_all(&namespace).unwrap();
        let path = namespace.join("auth.json");
        let key = minimax_code_record_key_for_auth_home(
            &auth_home.to_string_lossy(),
            "com.minimax.mcode.oauth.prod.cn",
        );
        fs::write(
            &path,
            json!({"records": {
                "com.minimax.mcode.oauth.prod.cn\u{0}inactive": {"accessToken": "inactive-access"},
                key: {"accessToken": "active-access"},
            }})
            .to_string(),
        )
        .unwrap();

        let (region, record) = read_client_auth_record_for_refresh(&path).unwrap();
        assert_eq!(region, "cn");
        assert_eq!(record["accessToken"], "active-access");
    }

    #[test]
    fn switch_rechecks_credentials_only_if_the_client_file_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("auth.json");
        let key = "com.minimax.mcode.oauth.prod.cn\0test";
        let write_record = |access: &str, refresh: &str| {
            fs::write(
                &path,
                json!({"records": {key: {
                    "accessToken": access, "refreshToken": refresh,
                }}})
                .to_string(),
            )
            .unwrap();
        };
        write_record("before-access", "before-refresh");
        let checked = HashMap::from([(
            path.clone(),
            read_client_auth_record_for_refresh(&path).unwrap(),
        )]);
        assert!(changed_client_auth_record(&path, &checked).is_none());

        write_record("after-access", "after-refresh");
        let (_, record) = changed_client_auth_record(&path, &checked).unwrap();
        assert_eq!(record["refreshToken"], "after-refresh");
    }
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

    #[test]
    fn local_import_ids_differ_per_data_dir() {
        // Two data dirs produce two different record-key account components, so the
        // two locally imported accounts must not collapse onto the same id.
        let write = |home: &Path, account: &str| -> PathBuf {
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
                    format!("com.minimax.mcode.oauth.prod.cn\u{0}{}", account): {
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
            fs::write(&file, serde_json::to_string_pretty(&body).unwrap()).unwrap();
            file
        };

        let home = TempHome::new();
        let a = write(&home.0, "hash_a");
        let b_dir = home.0.join("second");
        fs::create_dir_all(&b_dir).unwrap();
        let b = write(&b_dir, "hash_b");

        let pa = read_minimax_code_auth_file(&a).expect("read a");
        let pb = read_minimax_code_auth_file(&b).expect("read b");
        let ida = crate::modules::minimax_code_oauth::payload_from_local_record(
            &pa.0, &pa.1, &pa.2, &pa.3,
        );
        let idb = crate::modules::minimax_code_oauth::payload_from_local_record(
            &pb.0, &pb.1, &pb.2, &pb.3,
        );
        assert_eq!(ida.id.as_deref(), Some("hash_a"));
        assert_eq!(idb.id.as_deref(), Some("hash_b"));
        assert_ne!(ida.id, idb.id, "accounts from different data dirs must not collide");
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
            last_switched_at: None,
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

    /// 客户端真实的 record key：`{service}\0{base64url(sha256("<data root>/auth\0mcode-public"))}`。
    fn record_key_for(auth_file: &Path, service_prefix: &str) -> String {
        minimax_code_record_key(&data_root_of(auth_file).to_string_lossy(), service_prefix)
    }

    /// 从 `<data root>/auth/<buildEnv>/<region>/<clientId>/auth.json` 反推 data root。
    fn data_root_of(auth_file: &Path) -> PathBuf {
        auth_file
            .parent()
            .and_then(|client_dir| client_dir.parent())
            .and_then(|region_dir| region_dir.parent())
            .and_then(|env_dir| env_dir.parent())
            .and_then(|auth| auth.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn write_two_record_auth_file(dir: &Path) -> PathBuf {
        // 用真实的命名空间布局，record key 也由客户端算法生成，
        // 这样测试才与真实客户端一致（而不是用假的账号 id 当后缀）。
        let namespace = dir.join("auth").join("prod").join("cn").join("mcode-public");
        fs::create_dir_all(&namespace).unwrap();
        let file = namespace.join("auth.json");
        let service = "com.minimax.mcode.oauth.prod.cn";
        let body = json!({
            "schemaVersion": 1,
            "records": {
                record_key_for(&file, service): {
                    "accessToken": "mmoat_account_a",
                    "refreshToken": "mmort_account_a",
                    "tokenType": "Bearer",
                    "clientId": "mcode-public",
                    "scopes": ["agent.default"],
                    "audience": "agent-backend",
                    "expiresAtMs": 1792767948636_i64,
                    "generation": 7,
                    "subject": "previous-subject",
                    "accountId": "previous-account",
                    "loginEpoch": "previous-epoch"
                },
                "com.minimax.mcode.oauth.prod.cn\u{0}other_account": {
                    "accessToken": "mmoat_other",
                    "refreshToken": "mmort_other",
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
        let returned = switch_minimax_code_account_impl(&account_b, None).expect("switch should succeed");
        assert_eq!(returned, auth_file);

        let records = read_records(&auth_file);
        assert_eq!(records.len(), 1, "only the target record should remain");

        let (key, record) = records.iter().next().unwrap();
        assert_eq!(
            key,
            &record_key_for(&auth_file, "com.minimax.mcode.oauth.prod.cn"),
            "record key must be the client's sha256-derived key, not the Cockpit account id"
        );
        assert_eq!(
            record.get("accessToken").and_then(|v| v.as_str()),
            Some("mmoat_target_token")
        );
        assert_eq!(
            record.get("refreshToken").and_then(|v| v.as_str()),
            Some("mmort_target_refresh")
        );
        // A new login supersedes the existing generation and discards its identity.
        assert_eq!(record.get("generation").and_then(|v| v.as_i64()), Some(8));
        for field in ["subject", "accountId", "loginEpoch"] {
            assert!(record.get(field).is_none(), "old identity must not survive: {field}");
        }
        // 客户端 parseStoredCredential 要求有限的 expiresAtMs。
        let expires = record.get("expiresAtMs").and_then(|v| v.as_i64());
        assert_eq!(expires, Some(1792767948636_i64));

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
        switch_minimax_code_account_impl(&account_a, None).expect("second switch should succeed");

        let records = read_records(&auth_file);
        assert_eq!(records.len(), 1);
        let (key, record) = records.iter().next().unwrap();
        assert_eq!(
            key,
            &record_key_for(&auth_file, "com.minimax.mcode.oauth.prod.cn")
        );
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
        assert!(backup_records.contains_key(&record_key_for(
            &auth_file,
            "com.minimax.mcode.oauth.prod.cn"
        )));
        assert_eq!(
            backup_records
                .get(&record_key_for(
                    &auth_file,
                    "com.minimax.mcode.oauth.prod.cn"
                ))
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
        let account = fake_account(
            "account_b",
            "mmoat_target",
            Some("mmort_target"),
            None,
            &auth_file,
        );
        let err = switch_minimax_code_account_impl(&account, None).expect_err("array JSON must be rejected");
        assert_eq!(err, "MiniMax Code 登录态文件格式非法");
    }

    /// 账号缺少 refreshToken 时必须明确报错，而不是写一条客户端会判为 corrupt 的记录。
    #[test]
    fn switch_refuses_account_without_refresh_token() {
        let tmp = TempDir::new();
        let auth_file = tmp.0.join("auth.json");
        fs::write(&auth_file, "{\"schemaVersion\":1,\"records\":{}}").unwrap();
        let account = fake_account("account_b", "mmoat_target", None, None, &auth_file);
        let err = switch_minimax_code_account_impl(&account, None)
            .expect_err("missing refreshToken must be rejected");
        assert_eq!(err, "该账号缺少 refreshToken，无法切换");
    }

    #[test]
    fn switch_derives_prefix_from_another_record_when_target_absent() {
        let tmp = TempDir::new();
        // 真实命名空间布局：buildEnv/region 由目录决定（不再硬编码 cn）。
        let namespace = tmp.0.join("auth").join("prod").join("en").join("mcode-public");
        fs::create_dir_all(&namespace).unwrap();
        let auth_file = namespace.join("auth.json");
        // 只存在一个 `en` 记录，且它不是目标账号。
        let service = "com.minimax.mcode.oauth.prod.en";
        let body = json!({
            "schemaVersion": 1,
            "records": {
                record_key_for(&auth_file, service): {
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
        switch_minimax_code_account_impl(&account, None).expect("switch should succeed");

        let records = read_records(&auth_file);
        let (key, record) = records.iter().next().unwrap();
        assert_eq!(
            key,
            &record_key_for(&auth_file, "com.minimax.mcode.oauth.prod.en"),
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
        // 账号没有 expiresAtMs 时必须写一个有限的兜底值（客户端校验要求）。
        let expires = record.get("expiresAtMs").and_then(|v| v.as_i64());
        assert!(
            expires == Some(0),
            "unknown expiry must require client refresh instead of inventing a future lease"
        );
    }

    /// 账号没有 refreshToken 时明确报错（原「保留客户端已有 refreshToken」的行为已废弃：
    /// 客户端 `parseStoredCredential` 要求 refreshToken 非空）。
    #[test]
    fn switch_without_refresh_token_is_rejected_even_with_existing_record() {
        let tmp = TempDir::new();
        let auth_file = write_two_record_auth_file(&tmp.0);
        let account = fake_account("account_b", "mmoat_new_access", None, None, &auth_file);
        let err = switch_minimax_code_account_impl(&account, None)
            .expect_err("missing refreshToken must be rejected");
        assert_eq!(err, "该账号缺少 refreshToken，无法切换");
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

    // ---------------------------------------------------------------- --
    // 真实客户端契约：record key + auth-state.json
    // ---------------------------------------------------------------- //

    /// 独立复算客户端的 record key（与 `minimax_code_record_key` 的实现无关）。
    fn expected_record_key(data_dir: &str, service_prefix: &str) -> String {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let auth_home = Path::new(data_dir).join("auth");
        let mut hasher = Sha256::new();
        hasher.update(format!("{}\u{0}mcode-public", auth_home.to_string_lossy()).as_bytes());
        let account = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());
        format!("{}\u{0}{}", service_prefix, account)
    }

    #[test]
    fn record_key_matches_the_client_algorithm() {
        let tmp = TempDir::new();
        let service = "com.minimax.mcode.oauth.prod.cn";
        let got = minimax_code_record_key(&tmp.0.to_string_lossy(), service);
        let expected = expected_record_key(&tmp.0.to_string_lossy(), service);
        assert_eq!(got, expected, "record key must match the client's sha256 derivation");
        assert!(got.starts_with(&format!("{}\u{0}", service)));
        // 两个不同的 data root 必须得到不同的 key（多开隔离的根本）。
        let other = TempDir::new();
        let other_key = minimax_code_record_key(&other.0.to_string_lossy(), service);
        assert_ne!(got, other_key, "record keys must differ per data root");
    }

    #[test]
    fn switch_to_data_dir_writes_client_key_and_auth_state() {
        use crate::modules::minimax_code_instance as instance;

        let tmp = TempDir::new();
        let data_root = tmp.0.join("instance-data");
        let namespace = data_root
            .join("auth")
            .join("prod")
            .join("cn")
            .join(instance::MINIMAX_CODE_OAUTH_CLIENT_ID);
        fs::create_dir_all(&namespace).unwrap();
        let auth_file = namespace.join("auth.json");
        let service = "com.minimax.mcode.oauth.prod.cn";
        fs::write(namespace.join("auth-state.json"), json!({
            "schemaVersion": 1, "expiresAtMs": 1,
            "authorization": { "leaseId": "previous-login" }
        }).to_string()).unwrap();


        let account = MinimaxCodeAccount {
            id: "cockpit-account-id".to_string(),
            email: "real@example.com".to_string(),
            ..fake_account(
                "unused",
                "mmoat_real",
                Some("mmort_real"),
                Some(1792767948636_i64),
                &auth_file,
            )
        };

        let returned = switch_minimax_code_account_to_data_dir(&account, &data_root)
            .expect("switch should succeed");
        assert_eq!(returned, auth_file);

        // auth.json：恰好一条记录，key 是客户端算法算出来的那个。
        let records = read_records(&auth_file);
        assert_eq!(records.len(), 1, "exactly one record must remain");
        let expected_key = expected_record_key(&data_root.to_string_lossy(), service);
        let (key, record) = records.iter().next().unwrap();
        assert_eq!(
            key, &expected_key,
            "the written key must be the client's sha256-derived key"
        );
        assert_ne!(
            key,
            &format!("{}\u{0}{}", service, account.id),
            "the Cockpit account id must NOT be used as the key suffix"
        );
        assert_eq!(
            record.get("accessToken").and_then(|v| v.as_str()),
            Some("mmoat_real")
        );
        assert_eq!(
            record.get("refreshToken").and_then(|v| v.as_str()),
            Some("mmort_real")
        );
        assert_eq!(
            record.get("clientId").and_then(|v| v.as_str()),
            Some("mcode-public")
        );
        assert_eq!(
            record.get("audience").and_then(|v| v.as_str()),
            Some("agent-backend")
        );
        assert_eq!(
            record.get("tokenType").and_then(|v| v.as_str()),
            Some("Bearer")
        );
        let generation = record.get("generation").and_then(|v| v.as_i64()).unwrap();
        let expires_at_ms = record.get("expiresAtMs").and_then(|v| v.as_i64());
        assert!(expires_at_ms.is_some_and(|value| value > 0));

        // auth-state.json：客户端 getStatus() 要求 generation 与记录一致，否则返回 error。
        let state_path = namespace.join("auth-state.json");
        assert!(state_path.is_file(), "auth-state.json must be written");
        let state_raw = fs::read_to_string(&state_path).unwrap();
        let state: Value = serde_json::from_str(&state_raw).unwrap();
        assert_eq!(state.get("status").and_then(|v| v.as_str()), Some("authenticated"));
        assert!(state.get("authorization").is_none());
        assert_eq!(state["schemaVersion"], 2);
        assert_eq!(state.get("storeKind").and_then(|v| v.as_str()), Some("file"));
        assert_eq!(
            state.get("clientId").and_then(|v| v.as_str()),
            Some("mcode-public")
        );
        assert_eq!(
            state.get("buildEnv").and_then(|v| v.as_str()),
            Some("prod"),
            "buildEnv must be resolved, not hardcoded by accident"
        );
        assert_eq!(state.get("region").and_then(|v| v.as_str()), Some("cn"));
        assert_eq!(
            state.get("generation").and_then(|v| v.as_i64()),
            Some(generation),
            "auth-state.json generation must equal the record generation"
        );
        assert_eq!(
            state.get("expiresAtMs").and_then(|v| v.as_i64()),
            expires_at_ms
        );
        // 客户端 parseState 拒绝任何敏感字段名。
        assert!(
            !state_raw.to_lowercase().contains("accesstoken")
                && !state_raw.to_lowercase().contains("refreshtoken"),
            "auth-state.json must not contain token material"
        );
    }

    /// 空实例（没有任何 auth.json / auth-state.json）也必须写出完整可用的记录。
    #[test]
    fn switch_to_fresh_instance_writes_usable_credential() {
        let tmp = TempDir::new();
        let data_root = tmp.0.join("fresh-instance");

        let account = fake_account(
            "fresh-account",
            "mmoat_fresh",
            Some("mmort_fresh"),
            None,
            &tmp.0.join("auth.json"),
        );
        switch_minimax_code_account_to_data_dir(&account, &data_root)
            .expect("switch should succeed");

        let namespace = data_root
            .join("auth")
            .join("prod")
            .join("cn")
            .join(crate::modules::minimax_code_instance::MINIMAX_CODE_OAUTH_CLIENT_ID);
        let records = read_records(&namespace.join("auth.json"));
        assert_eq!(records.len(), 1);
        let (_, record) = records.iter().next().unwrap();
        // 客户端 parseStoredCredential 的全部必填项。
        assert!(record.get("accessToken").and_then(|v| v.as_str()).is_some());
        assert_eq!(
            record.get("refreshToken").and_then(|v| v.as_str()),
            Some("mmort_fresh")
        );
        assert_eq!(
            record.get("expiresAtMs").and_then(|v| v.as_i64()),
            Some(MINIMAX_CODE_UNKNOWN_EXPIRES_AT_MS),
            "unknown expiry must require client refresh"
        );
        let state: Value = serde_json::from_str(
            &fs::read_to_string(namespace.join("auth-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(state.get("status").and_then(|v| v.as_str()), Some("authenticated"));
        assert!(state.get("authorization").is_none());
        assert_eq!(state["schemaVersion"], 2);
        assert_eq!(
            state.get("generation").and_then(|v| v.as_i64()),
            record.get("generation").and_then(|v| v.as_i64())
        );
    }

    /// 解析出的命名空间路径必须与客户端布局一致。
    #[test]
    fn auth_scope_resolves_namespace_layout() {
        use crate::modules::minimax_code_instance as instance;
        let tmp = TempDir::new();
        let data_root = tmp.0.join("scope-instance");

        // 全新实例：默认 prod/cn。
        assert_eq!(
            instance::resolve_minimax_code_auth_file_path(&data_root),
            data_root
                .join("auth")
                .join("prod")
                .join("cn")
                .join(instance::MINIMAX_CODE_OAUTH_CLIENT_ID)
                .join("auth.json")
        );

        // 已有 auth.json 的 en 命名空间 → 沿用 en。
        let en_namespace = data_root.join("auth").join("prod").join("en").join("mcode-public");
        fs::create_dir_all(&en_namespace).unwrap();
        let en_auth = en_namespace.join("auth.json");
        let key = expected_record_key(&data_root.to_string_lossy(), "com.minimax.mcode.oauth.prod.en");
        fs::write(
            &en_auth,
            json!({
                "schemaVersion": 1,
                "records": { key: {
                    "accessToken": "mmoat_en", "refreshToken": "mmort_en",
                    "tokenType": "Bearer", "clientId": "mcode-public",
                    "scopes": ["agent.default"], "audience": "agent-backend",
                    "expiresAtMs": 1792767948636_i64, "generation": 1
                }}
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            instance::resolve_minimax_code_auth_file_path(&data_root),
            en_auth,
            "an existing namespace must be reused"
        );

        // 账号自带 region=en 时，全新实例应落在 en 目录。
        let other = TempDir::new();
        let fresh_root = other.0.join("fresh");
        let scope = instance::resolve_minimax_code_auth_scope(&fresh_root, Some("en"));
        assert_eq!(scope.region, "en");
        assert_eq!(scope.service_prefix(), "com.minimax.mcode.oauth.prod.en");
    }
}
