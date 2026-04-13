use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::models::qoder::{QoderAccount, QoderAccountIndex};
use crate::modules::{account, logger};

const ACCOUNTS_INDEX_FILE: &str = "qoder_accounts.json";
const ACCOUNTS_DIR: &str = "qoder_accounts";

const QODER_SECRET_USER_INFO_KEY: &str = "secret://aicoding.auth.userInfo";
const QODER_SECRET_USER_PLAN_KEY: &str = "secret://aicoding.auth.userPlan";
const QODER_SECRET_CREDIT_USAGE_KEY: &str = "secret://aicoding.auth.creditUsage";

static QODER_ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Default)]
struct QoderSnapshot {
    user_info_raw: Option<Value>,
    user_plan_raw: Option<Value>,
    credit_usage_raw: Option<Value>,
}

#[derive(Debug, Clone)]
struct NumericCandidate {
    path: String,
    value: f64,
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
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
    normalize_non_empty(value).map(|v| v.to_lowercase())
}

fn sanitize_account_id_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn generate_account_id(
    snapshot: &QoderSnapshot,
    user_id: Option<&str>,
    email: Option<&str>,
) -> String {
    if let Some(uid) = normalize_non_empty(user_id) {
        let cleaned = sanitize_account_id_component(&uid);
        if !cleaned.is_empty() {
            return format!("qoder_uid_{}", cleaned);
        }
    }

    if let Some(addr) = normalize_email(email) {
        let cleaned = sanitize_account_id_component(&addr);
        if !cleaned.is_empty() {
            return format!("qoder_email_{}", cleaned);
        }
    }

    let basis = format!(
        "{}|{}|{}",
        snapshot
            .user_info_raw
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        snapshot
            .user_plan_raw
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        snapshot
            .credit_usage_raw
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    let digest = md5::compute(basis.as_bytes());
    format!("qoder_{:x}", digest)
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Qoder 账号目录失败: {}", e))?;
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

pub fn load_account(account_id: &str) -> Option<QoderAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_account_file(account: &QoderAccount) -> Result<(), String> {
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

fn load_account_index() -> QoderAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return QoderAccountIndex::new(),
    };
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(QoderAccountIndex::new);
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空").unwrap_or_else(QoderAccountIndex::new)
        }
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            QoderAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(QoderAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Qoder Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(QoderAccountIndex::new)
            }
        },
        Err(_) => QoderAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<QoderAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(QoderAccountIndex::new());
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
        return Ok(QoderAccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<QoderAccountIndex>(
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

fn save_account_index(index: &QoderAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<QoderAccountIndex> {
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

    let mut index = QoderAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Qoder Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Qoder Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_warn(&format!(
        "[Qoder Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut QoderAccountIndex, account: &QoderAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: QoderAccount) -> Result<QoderAccount, String> {
    let _lock = QODER_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Qoder 账号锁失败".to_string())?;
    let mut index = load_account_index();
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
}

pub fn update_quota_query_error(
    account_id: &str,
    message: Option<String>,
) -> Result<Option<QoderAccount>, String> {
    let Some(mut account) = load_account(account_id) else {
        return Ok(None);
    };
    account.quota_query_last_error = message;
    account.quota_query_last_error_at = account
        .quota_query_last_error
        .as_ref()
        .map(|_| chrono::Utc::now().timestamp_millis());
    let updated = upsert_account_record(account)?;
    Ok(Some(updated))
}

fn list_accounts_from_index(index: &QoderAccountIndex) -> Vec<QoderAccount> {
    let mut accounts = Vec::new();
    for summary in &index.accounts {
        if let Some(account) = load_account(&summary.id) {
            accounts.push(account);
        }
    }
    accounts.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    accounts
}

pub fn list_accounts() -> Vec<QoderAccount> {
    let index = load_account_index();
    list_accounts_from_index(&index)
}

pub fn list_accounts_checked() -> Result<Vec<QoderAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(list_accounts_from_index(&index))
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = QODER_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Qoder 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| item.id != account_id);
    save_account_index(&index)?;
    delete_account_file(account_id)?;
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    let target: HashSet<String> = account_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if target.is_empty() {
        return Ok(());
    }

    let _lock = QODER_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Qoder 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| !target.contains(&item.id));
    save_account_index(&index)?;
    for id in target {
        delete_account_file(&id)?;
    }
    Ok(())
}

fn parse_json_or_string(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn walk_value<'a>(value: &'a Value, path: &str, visit: &mut dyn FnMut(&str, &'a Value)) {
    visit(path, value);
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };
                walk_value(child, &child_path, visit);
            }
        }
        Value::Array(list) => {
            for (idx, child) in list.iter().enumerate() {
                let child_path = if path.is_empty() {
                    format!("[{}]", idx)
                } else {
                    format!("{}[{}]", path, idx)
                };
                walk_value(child, &child_path, visit);
            }
        }
        _ => {}
    }
}

fn path_last_segment(path: &str) -> &str {
    let mut last = path;
    if let Some(idx) = last.rfind('.') {
        last = &last[idx + 1..];
    }
    if let Some(idx) = last.rfind('[') {
        last = &last[..idx];
    }
    last
}

fn find_string_by_exact_keys(value: &Value, keys: &[&str]) -> Option<String> {
    let key_set: HashSet<String> = keys.iter().map(|k| k.to_ascii_lowercase()).collect();
    let mut found: Option<String> = None;
    walk_value(value, "", &mut |path, current| {
        if found.is_some() {
            return;
        }
        let Some(text) = current.as_str() else {
            return;
        };
        let Some(normalized) = normalize_non_empty(Some(text)) else {
            return;
        };
        let last = path_last_segment(path).to_ascii_lowercase();
        if key_set.contains(last.as_str()) {
            found = Some(normalized);
        }
    });
    found
}

fn find_string_by_path_keywords(value: &Value, includes: &[&str]) -> Option<String> {
    let mut found: Option<String> = None;
    walk_value(value, "", &mut |path, current| {
        if found.is_some() {
            return;
        }
        let Some(text) = current.as_str() else {
            return;
        };
        let Some(normalized) = normalize_non_empty(Some(text)) else {
            return;
        };
        let path_lower = path.to_ascii_lowercase();
        if includes
            .iter()
            .all(|keyword| path_lower.contains(&keyword.to_ascii_lowercase()))
        {
            found = Some(normalized);
        }
    });
    found
}

fn find_first_email(value: &Value) -> Option<String> {
    let mut found: Option<String> = None;
    walk_value(value, "", &mut |_path, current| {
        if found.is_some() {
            return;
        }
        let Some(text) = current.as_str() else {
            return;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if trimmed.contains('@') && trimmed.contains('.') {
            found = Some(trimmed.to_lowercase());
        }
    });
    found
}

fn collect_numeric_candidates(value: &Value, base_path: &str, output: &mut Vec<NumericCandidate>) {
    walk_value(value, base_path, &mut |path, current| {
        let num = match current {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    trimmed.parse::<f64>().ok()
                }
            }
            _ => None,
        };
        let Some(raw) = num else {
            return;
        };
        if !raw.is_finite() || raw.abs() > 1_000_000_000_000.0 {
            return;
        }
        output.push(NumericCandidate {
            path: path.to_ascii_lowercase(),
            value: raw,
        });
    });
}

fn pick_numeric_candidate(
    candidates: &[NumericCandidate],
    includes: &[&str],
    excludes: &[&str],
) -> Option<f64> {
    for candidate in candidates {
        if includes
            .iter()
            .all(|item| candidate.path.contains(&item.to_ascii_lowercase()))
            && excludes
                .iter()
                .all(|item| !candidate.path.contains(&item.to_ascii_lowercase()))
        {
            return Some(candidate.value);
        }
    }
    None
}

fn clamp_percent(value: f64) -> f64 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(0.0, 100.0)
}

fn extract_snapshot_email(snapshot: &QoderSnapshot) -> Option<String> {
    let candidates = [
        snapshot.user_info_raw.as_ref(),
        snapshot.user_plan_raw.as_ref(),
        snapshot.credit_usage_raw.as_ref(),
    ];
    for value in candidates.into_iter().flatten() {
        if let Some(email) = find_string_by_exact_keys(value, &["email", "mail"]) {
            return Some(email.to_lowercase());
        }
        if let Some(email) = find_first_email(value) {
            return Some(email);
        }
    }
    None
}

fn extract_snapshot_user_id(snapshot: &QoderSnapshot) -> Option<String> {
    let candidates = [
        snapshot.user_info_raw.as_ref(),
        snapshot.user_plan_raw.as_ref(),
        snapshot.credit_usage_raw.as_ref(),
    ];
    for value in candidates.into_iter().flatten() {
        if let Some(uid) = find_string_by_exact_keys(
            value,
            &[
                "uid",
                "user_id",
                "userid",
                "userId",
                "account_id",
                "accountId",
                "id",
            ],
        ) {
            return Some(uid);
        }
    }
    None
}

fn extract_snapshot_display_name(snapshot: &QoderSnapshot) -> Option<String> {
    let Some(value) = snapshot.user_info_raw.as_ref() else {
        return None;
    };
    find_string_by_exact_keys(
        value,
        &[
            "name",
            "nickname",
            "display_name",
            "displayName",
            "username",
        ],
    )
}

fn extract_snapshot_plan_type(snapshot: &QoderSnapshot) -> Option<String> {
    if let Some(value) = snapshot.user_plan_raw.as_ref() {
        if let Some(plan) = find_string_by_exact_keys(
            value,
            &[
                "plan",
                "plan_type",
                "planType",
                "plan_tier_name",
                "planTierName",
                "tier",
                "tier_name",
                "tierName",
                "package",
                "package_name",
                "packageName",
                "name",
            ],
        ) {
            return Some(plan);
        }
    }

    if let Some(value) = snapshot.user_info_raw.as_ref() {
        if let Some(plan) =
            find_string_by_exact_keys(value, &["userTag", "user_tag", "plan_tier_name"])
        {
            return Some(plan);
        }
    }

    if let Some(value) = snapshot.credit_usage_raw.as_ref() {
        if let Some(plan) = find_string_by_path_keywords(value, &["plan"]) {
            return Some(plan);
        }
    }

    None
}

fn extract_snapshot_credits(
    snapshot: &QoderSnapshot,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    let mut candidates = Vec::new();
    if let Some(value) = snapshot.credit_usage_raw.as_ref() {
        collect_numeric_candidates(value, "usage", &mut candidates);
    }
    if let Some(value) = snapshot.user_plan_raw.as_ref() {
        collect_numeric_candidates(value, "plan", &mut candidates);
    }

    let mut used = pick_numeric_candidate(
        &candidates,
        &["used"],
        &["percent", "rate", "ratio", "remaining", "remain", "left"],
    )
    .or_else(|| {
        pick_numeric_candidate(
            &candidates,
            &["consum"],
            &["percent", "rate", "ratio", "remaining", "remain", "left"],
        )
    });

    let mut remaining =
        pick_numeric_candidate(&candidates, &["remaining"], &["percent", "rate", "ratio"])
            .or_else(|| {
                pick_numeric_candidate(&candidates, &["remain"], &["percent", "rate", "ratio"])
            })
            .or_else(|| {
                pick_numeric_candidate(&candidates, &["left"], &["percent", "rate", "ratio"])
            })
            .or_else(|| {
                pick_numeric_candidate(&candidates, &["available"], &["percent", "rate", "ratio"])
            });

    let mut total = pick_numeric_candidate(
        &candidates,
        &["total"],
        &[
            "percent",
            "rate",
            "ratio",
            "remaining",
            "remain",
            "left",
            "used",
            "consum",
        ],
    )
    .or_else(|| {
        pick_numeric_candidate(
            &candidates,
            &["quota"],
            &[
                "percent",
                "rate",
                "ratio",
                "remaining",
                "remain",
                "left",
                "used",
                "consum",
            ],
        )
    })
    .or_else(|| {
        pick_numeric_candidate(
            &candidates,
            &["limit"],
            &[
                "percent",
                "rate",
                "ratio",
                "remaining",
                "remain",
                "left",
                "used",
                "consum",
            ],
        )
    });

    if total.is_none() {
        if let (Some(u), Some(r)) = (used, remaining) {
            total = Some(u + r);
        }
    }

    if remaining.is_none() {
        if let (Some(t), Some(u)) = (total, used) {
            remaining = Some((t - u).max(0.0));
        }
    }

    if used.is_none() {
        if let (Some(t), Some(r)) = (total, remaining) {
            used = Some((t - r).max(0.0));
        }
    }

    let mut usage_percent =
        pick_numeric_candidate(&candidates, &["percent"], &["remaining", "remain", "left"]);

    if usage_percent.is_none() {
        usage_percent = pick_numeric_candidate(&candidates, &["ratio"], &[]);
    }

    if let Some(pct) = usage_percent {
        let normalized = if pct <= 1.0 { pct * 100.0 } else { pct };
        usage_percent = Some(clamp_percent(normalized));
    } else if let (Some(u), Some(t)) = (used, total) {
        if t > 0.0 {
            usage_percent = Some(clamp_percent((u / t) * 100.0));
        }
    }

    (used, total, remaining, usage_percent)
}

fn snapshot_has_any_data(snapshot: &QoderSnapshot) -> bool {
    snapshot.user_info_raw.is_some()
        || snapshot.user_plan_raw.is_some()
        || snapshot.credit_usage_raw.is_some()
}

fn same_identity(
    account: &QoderAccount,
    user_id: Option<&str>,
    email: Option<&str>,
    generated_id: &str,
) -> bool {
    if let (Some(left), Some(right)) = (
        normalize_non_empty(account.user_id.as_deref()),
        normalize_non_empty(user_id),
    ) {
        if left.eq_ignore_ascii_case(&right) {
            return true;
        }
    }

    if let (Some(left), Some(right)) = (
        normalize_email(Some(account.email.as_str())),
        normalize_email(email),
    ) {
        if left == right {
            return true;
        }
    }

    account.id == generated_id
}

fn normalize_tags(tags: Vec<String>) -> Option<Vec<String>> {
    let mut set = HashSet::new();
    let mut result = Vec::new();
    for raw in tags {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_string();
        let lower = normalized.to_lowercase();
        if set.insert(lower) {
            result.push(normalized);
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn snapshot_to_account(snapshot: QoderSnapshot, existing: Option<&QoderAccount>) -> QoderAccount {
    let now = now_ts();
    let email = extract_snapshot_email(&snapshot)
        .or_else(|| existing.and_then(|item| normalize_email(Some(item.email.as_str()))))
        .unwrap_or_else(|| "unknown@qoder.local".to_string());
    let user_id = extract_snapshot_user_id(&snapshot)
        .or_else(|| existing.and_then(|item| item.user_id.clone()));
    let generated_id = generate_account_id(&snapshot, user_id.as_deref(), Some(email.as_str()));
    let display_name = extract_snapshot_display_name(&snapshot)
        .or_else(|| existing.and_then(|item| item.display_name.clone()));
    let plan_type = extract_snapshot_plan_type(&snapshot)
        .or_else(|| existing.and_then(|item| item.plan_type.clone()));
    let (credits_used, credits_total, credits_remaining, credits_usage_percent) =
        extract_snapshot_credits(&snapshot);

    QoderAccount {
        id: existing.map(|item| item.id.clone()).unwrap_or(generated_id),
        email,
        user_id,
        display_name,
        plan_type,
        credits_used: credits_used.or_else(|| existing.and_then(|item| item.credits_used)),
        credits_total: credits_total.or_else(|| existing.and_then(|item| item.credits_total)),
        credits_remaining: credits_remaining
            .or_else(|| existing.and_then(|item| item.credits_remaining)),
        credits_usage_percent: credits_usage_percent
            .or_else(|| existing.and_then(|item| item.credits_usage_percent)),
        quota_query_last_error: if snapshot.credit_usage_raw.is_some() {
            None
        } else {
            existing.and_then(|item| item.quota_query_last_error.clone())
        },
        quota_query_last_error_at: if snapshot.credit_usage_raw.is_some() {
            None
        } else {
            existing.and_then(|item| item.quota_query_last_error_at)
        },
        usage_updated_at: if snapshot.credit_usage_raw.is_some() {
            Some(now)
        } else {
            existing.and_then(|item| item.usage_updated_at)
        },
        tags: existing.and_then(|item| item.tags.clone()),
        auth_user_info_raw: snapshot
            .user_info_raw
            .or_else(|| existing.and_then(|item| item.auth_user_info_raw.clone())),
        auth_user_plan_raw: snapshot
            .user_plan_raw
            .or_else(|| existing.and_then(|item| item.auth_user_plan_raw.clone())),
        auth_credit_usage_raw: snapshot
            .credit_usage_raw
            .or_else(|| existing.and_then(|item| item.auth_credit_usage_raw.clone())),
        created_at: existing.map(|item| item.created_at).unwrap_or(now),
        last_used: now,
    }
}

fn find_existing_account_for_snapshot(
    snapshot: &QoderSnapshot,
    accounts: &[QoderAccount],
) -> Option<QoderAccount> {
    let user_id = extract_snapshot_user_id(snapshot);
    let email = extract_snapshot_email(snapshot);
    let generated_id = generate_account_id(snapshot, user_id.as_deref(), email.as_deref());
    accounts
        .iter()
        .find(|item| same_identity(item, user_id.as_deref(), email.as_deref(), &generated_id))
        .cloned()
}

fn read_qoder_secret_json(db_path: &Path, db_key: &str) -> Result<Option<Value>, String> {
    let raw =
        crate::modules::vscode_inject::read_qoder_secret_storage_value_by_db_path(db_path, db_key)?;
    Ok(raw.map(|text| parse_json_or_string(text.as_str())))
}

fn read_snapshot_from_state_db_path(db_path: &Path) -> Result<Option<QoderSnapshot>, String> {
    if !db_path.exists() {
        return Ok(None);
    }

    let snapshot = QoderSnapshot {
        user_info_raw: read_qoder_secret_json(db_path, QODER_SECRET_USER_INFO_KEY)?,
        user_plan_raw: read_qoder_secret_json(db_path, QODER_SECRET_USER_PLAN_KEY)?,
        credit_usage_raw: read_qoder_secret_json(db_path, QODER_SECRET_CREDIT_USAGE_KEY)?,
    };

    if snapshot_has_any_data(&snapshot) {
        Ok(Some(snapshot))
    } else {
        Ok(None)
    }
}

fn merge_snapshot(snapshot: QoderSnapshot) -> Result<QoderAccount, String> {
    let accounts = list_accounts();
    let existing = find_existing_account_for_snapshot(&snapshot, &accounts);
    let account = snapshot_to_account(snapshot, existing.as_ref());
    upsert_account_record(account)
}

pub fn upsert_account_from_snapshot(
    user_info_raw: Value,
    user_plan_raw: Option<Value>,
    credit_usage_raw: Option<Value>,
) -> Result<QoderAccount, String> {
    merge_snapshot(QoderSnapshot {
        user_info_raw: Some(user_info_raw),
        user_plan_raw,
        credit_usage_raw,
    })
}

pub fn get_default_qoder_state_db_path() -> Option<PathBuf> {
    let data_root = crate::modules::qoder_instance::get_default_qoder_user_data_dir().ok()?;
    Some(
        data_root
            .join("User")
            .join("globalStorage")
            .join("state.vscdb"),
    )
}

fn resolve_state_db_path_for_user_data_dir(user_data_dir: &str) -> PathBuf {
    PathBuf::from(user_data_dir)
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

pub fn ensure_state_db_path_for_user_data_dir(user_data_dir: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(user_data_dir);
    let candidates = vec![
        root.join("User").join("globalStorage").join("state.vscdb"),
        root.join("globalStorage").join("state.vscdb"),
        root.join("state.vscdb"),
    ];

    if let Some(existing) = candidates.iter().find(|path| path.exists()) {
        return Ok(existing.clone());
    }

    let preferred = resolve_state_db_path_for_user_data_dir(user_data_dir);
    if let Some(parent) = preferred.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建 Qoder globalStorage 目录失败: {}", e))?;
    }

    if let Some(default_db) = get_default_qoder_state_db_path() {
        if default_db.exists() && default_db != preferred {
            if let Err(err) = fs::copy(&default_db, &preferred) {
                logger::log_warn(&format!(
                    "[Qoder Inject] 复制默认 state.vscdb 失败，改为写入新库: from={}, to={}, error={}",
                    default_db.to_string_lossy(),
                    preferred.to_string_lossy(),
                    err
                ));
            }
        }
    }

    Ok(preferred)
}

fn ensure_default_state_db_path() -> Result<PathBuf, String> {
    let data_root = crate::modules::qoder_instance::get_default_qoder_user_data_dir()?;
    ensure_state_db_path_for_user_data_dir(&data_root.to_string_lossy())
}

pub fn import_from_local() -> Result<Option<QoderAccount>, String> {
    let db_path = ensure_default_state_db_path()?;
    let Some(snapshot) = read_snapshot_from_state_db_path(&db_path)? else {
        return Ok(None);
    };
    let account = merge_snapshot(snapshot)?;
    logger::log_info(&format!(
        "[Qoder Account] 从本地导入成功: id={}, email={}, db={}",
        account.id,
        account.email,
        db_path.to_string_lossy()
    ));
    Ok(Some(account))
}

pub(crate) fn resolve_current_account_id(accounts: &[QoderAccount]) -> Option<String> {
    let db_path = get_default_qoder_state_db_path()?;
    let snapshot = read_snapshot_from_state_db_path(db_path.as_path()).ok()??;
    let user_id = extract_snapshot_user_id(&snapshot);
    let email = extract_snapshot_email(&snapshot);
    let generated_id = generate_account_id(&snapshot, user_id.as_deref(), email.as_deref());

    accounts
        .iter()
        .find(|account| same_identity(account, user_id.as_deref(), email.as_deref(), &generated_id))
        .map(|account| account.id.clone())
}

fn serialize_raw_or_fallback(raw: &Option<Value>, fallback: Value) -> Result<String, String> {
    let value = raw.clone().unwrap_or(fallback);
    serde_json::to_string(&value).map_err(|e| format!("序列化 Qoder 注入数据失败: {}", e))
}

fn build_user_info_fallback(account: &QoderAccount) -> Value {
    serde_json::json!({
        "id": account.user_id.clone().unwrap_or_default(),
        "email": account.email,
        "name": account.display_name.clone().unwrap_or_default(),
    })
}

fn build_user_plan_fallback(account: &QoderAccount) -> Value {
    serde_json::json!({
        "plan": account.plan_type.clone().unwrap_or_default(),
        "tier": account.plan_type.clone().unwrap_or_default(),
    })
}

fn build_credit_usage_fallback(account: &QoderAccount) -> Value {
    serde_json::json!({
        "used": account.credits_used,
        "total": account.credits_total,
        "remaining": account.credits_remaining,
        "usagePercent": account.credits_usage_percent,
    })
}

fn verify_state_db_key_exists(db_path: &Path, db_key: &str) -> Result<(), String> {
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| format!("注入校验失败，无法打开 state.vscdb: {}", e))?;

    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [db_key],
            |row| row.get(0),
        )
        .ok();

    match value {
        Some(stored) if !stored.trim().is_empty() => Ok(()),
        _ => Err(format!(
            "注入校验失败，未在 state.vscdb 找到 key: db={}, key={}",
            db_path.to_string_lossy(),
            db_key
        )),
    }
}

fn verify_injected_account_matches(db_path: &Path, account: &QoderAccount) -> Result<(), String> {
    let snapshot = read_snapshot_from_state_db_path(db_path)?.ok_or_else(|| {
        format!(
            "注入校验失败，未读取到 state.vscdb 快照: {}",
            db_path.display()
        )
    })?;
    let effective_user_id = extract_snapshot_user_id(&snapshot);
    let effective_email = extract_snapshot_email(&snapshot);
    let generated_id = generate_account_id(
        &snapshot,
        effective_user_id.as_deref(),
        effective_email.as_deref(),
    );

    if same_identity(
        account,
        effective_user_id.as_deref(),
        effective_email.as_deref(),
        &generated_id,
    ) {
        return Ok(());
    }

    Err(format!(
        "注入校验失败，落盘账号与目标账号不一致: db={}, target_id={}, target_email={}, actual_user_id={:?}, actual_email={:?}",
        db_path.display(),
        account.id,
        account.email,
        effective_user_id,
        effective_email
    ))
}

pub fn inject_to_qoder(account_id: &str) -> Result<(), String> {
    let db_path = ensure_default_state_db_path()?;
    inject_to_qoder_at_path(&db_path, account_id)
}

pub fn inject_to_qoder_for_user_data_dir(
    user_data_dir: &str,
    account_id: &str,
) -> Result<(), String> {
    let db_path = ensure_state_db_path_for_user_data_dir(user_data_dir)?;
    inject_to_qoder_at_path(&db_path, account_id)
}

pub fn inject_to_qoder_at_path(db_path: &Path, account_id: &str) -> Result<(), String> {
    let account =
        load_account(account_id).ok_or_else(|| format!("Qoder 账号不存在: {}", account_id))?;
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建 Qoder state.vscdb 目录失败: {}", e))?;
    }

    let user_info_json = serialize_raw_or_fallback(
        &account.auth_user_info_raw,
        build_user_info_fallback(&account),
    )?;
    let user_plan_json = serialize_raw_or_fallback(
        &account.auth_user_plan_raw,
        build_user_plan_fallback(&account),
    )?;
    let credit_usage_json = serialize_raw_or_fallback(
        &account.auth_credit_usage_raw,
        build_credit_usage_fallback(&account),
    )?;

    crate::modules::vscode_inject::inject_secret_to_state_db_for_qoder(
        db_path,
        QODER_SECRET_USER_INFO_KEY,
        &user_info_json,
    )?;
    crate::modules::vscode_inject::inject_secret_to_state_db_for_qoder(
        db_path,
        QODER_SECRET_USER_PLAN_KEY,
        &user_plan_json,
    )?;
    crate::modules::vscode_inject::inject_secret_to_state_db_for_qoder(
        db_path,
        QODER_SECRET_CREDIT_USAGE_KEY,
        &credit_usage_json,
    )?;

    verify_state_db_key_exists(db_path, QODER_SECRET_USER_INFO_KEY)?;
    verify_state_db_key_exists(db_path, QODER_SECRET_USER_PLAN_KEY)?;
    verify_state_db_key_exists(db_path, QODER_SECRET_CREDIT_USAGE_KEY)?;
    verify_injected_account_matches(db_path, &account)?;

    let mut updated = account.clone();
    updated.last_used = now_ts();
    let _ = upsert_account_record(updated);

    logger::log_info(&format!(
        "[Qoder Inject] 注入成功: account_id={}, email={}, db={}",
        account.id,
        account.email,
        db_path.to_string_lossy()
    ));
    Ok(())
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<QoderAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("Qoder 账号不存在: {}", account_id))?;
    account.tags = normalize_tags(tags);
    account.last_used = now_ts();
    upsert_account_record(account)
}

fn normalize_imported_account(mut account: QoderAccount) -> QoderAccount {
    let now = now_ts();
    account.id = sanitize_account_id_component(account.id.trim());
    if account.id.is_empty() {
        let snapshot = QoderSnapshot {
            user_info_raw: account.auth_user_info_raw.clone(),
            user_plan_raw: account.auth_user_plan_raw.clone(),
            credit_usage_raw: account.auth_credit_usage_raw.clone(),
        };
        account.id = generate_account_id(
            &snapshot,
            account.user_id.as_deref(),
            Some(account.email.as_str()),
        );
    }
    account.email = normalize_email(Some(account.email.as_str()))
        .unwrap_or_else(|| "unknown@qoder.local".to_string());
    account.user_id = normalize_non_empty(account.user_id.as_deref());
    account.display_name = normalize_non_empty(account.display_name.as_deref());
    account.plan_type = normalize_non_empty(account.plan_type.as_deref());
    account.tags = normalize_tags(account.tags.unwrap_or_default());
    account.quota_query_last_error = normalize_non_empty(account.quota_query_last_error.as_deref());
    if account.created_at <= 0 {
        account.created_at = now;
    }
    if account.last_used <= 0 {
        account.last_used = now;
    }
    if account.credits_usage_percent.is_none() {
        if let (Some(used), Some(total)) = (account.credits_used, account.credits_total) {
            if total > 0.0 {
                account.credits_usage_percent = Some(clamp_percent((used / total) * 100.0));
            }
        }
    }
    account
}

fn parse_import_item(item: &Value) -> Result<QoderAccount, String> {
    if let Ok(account) = serde_json::from_value::<QoderAccount>(item.clone()) {
        return Ok(normalize_imported_account(account));
    }

    let Some(obj) = item.as_object() else {
        return Err("Qoder 导入数据格式无效".to_string());
    };

    let snapshot = QoderSnapshot {
        user_info_raw: obj
            .get("auth_user_info_raw")
            .or_else(|| obj.get("userInfo"))
            .cloned(),
        user_plan_raw: obj
            .get("auth_user_plan_raw")
            .or_else(|| obj.get("userPlan"))
            .cloned(),
        credit_usage_raw: obj
            .get("auth_credit_usage_raw")
            .or_else(|| obj.get("creditUsage"))
            .cloned(),
    };

    if !snapshot_has_any_data(&snapshot) {
        return Err("Qoder 导入项缺少账号字段".to_string());
    }

    Ok(snapshot_to_account(snapshot, None))
}

pub fn import_from_json(json_content: &str) -> Result<Vec<QoderAccount>, String> {
    let parsed: Value =
        serde_json::from_str(json_content).map_err(|e| format!("JSON 解析失败: {}", e))?;
    let items: Vec<Value> = match parsed {
        Value::Array(list) => list,
        Value::Object(map) => {
            if let Some(Value::Array(list)) = map.get("accounts") {
                list.clone()
            } else {
                vec![Value::Object(map)]
            }
        }
        _ => return Err("仅支持对象或数组格式的 Qoder JSON".to_string()),
    };

    if items.is_empty() {
        return Ok(Vec::new());
    }

    let mut imported = Vec::new();
    for item in items {
        let account = parse_import_item(&item)?;
        let saved = upsert_account_record(account)?;
        imported.push(saved);
    }

    Ok(imported)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts = list_accounts();
    let selected: Vec<QoderAccount> = if account_ids.is_empty() {
        accounts
    } else {
        let target: HashSet<String> = account_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect();
        accounts
            .into_iter()
            .filter(|item| target.contains(&item.id))
            .collect()
    };

    serde_json::to_string_pretty(&selected).map_err(|e| format!("序列化导出 JSON 失败: {}", e))
}
