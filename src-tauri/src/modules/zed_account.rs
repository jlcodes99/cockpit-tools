use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Mutex;

use crate::models::zed::{ZedAccount, ZedAccountIndex, ZedStoredAccount};
use crate::modules::{account, logger};

const ACCOUNTS_INDEX_FILE: &str = "zed_accounts.json";
const ACCOUNTS_DIR: &str = "zed_accounts";
const ZED_SERVER_URL: &str = "https://zed.dev";
const ZED_CLOUD_BASE_URL: &str = "https://cloud.zed.dev";
const ZED_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 10 * 60;

static ZED_ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));
static ZED_QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
struct ZedFetchBundle {
    user_raw: Value,
    subscription_raw: Value,
    usage_raw: Value,
    usage_tokens_raw: Value,
    preferences_raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZedExportPayload {
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_account_id: Option<String>,
    accounts: Vec<ZedStoredAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZedKeychainCredentials {
    pub user_id: String,
    pub access_token: String,
}

fn now_ts() -> i64 {
    Utc::now().timestamp()
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

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Zed 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

fn resolve_account_file_path(account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    Ok(get_accounts_dir()?.join(format!("{}.json", normalized)))
}

pub fn load_stored_account(account_id: &str) -> Option<ZedStoredAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_stored_account_file(account: &ZedStoredAccount) -> Result<(), String> {
    let path = resolve_account_file_path(account.public_account.id.as_str())?;
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

fn load_account_index() -> ZedAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return ZedAccountIndex::new(),
    };
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(ZedAccountIndex::new);
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空").unwrap_or_else(ZedAccountIndex::new)
        }
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            ZedAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(ZedAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Zed Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(ZedAccountIndex::new)
            }
        },
        Err(_) => ZedAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<ZedAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(ZedAccountIndex::new());
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
        return Ok(ZedAccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<ZedAccountIndex>(
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

fn save_account_index(index: &ZedAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<ZedAccountIndex> {
    let index_path = get_accounts_index_path().ok()?;
    let accounts_dir = get_accounts_dir().ok()?;
    let mut accounts = crate::modules::account_index_repair::load_accounts_from_details(
        &accounts_dir,
        |account_id| load_stored_account(account_id),
    )
    .ok()?;

    if accounts.is_empty() {
        return None;
    }

    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut accounts,
        |account| account.public_account.last_used,
        |account| account.public_account.created_at,
        |account| account.public_account.id.as_str(),
    );

    let mut index = ZedAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();
    index.current_account_id = accounts
        .first()
        .map(|account| account.public_account.id.clone());

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Zed Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Zed Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_warn(&format!(
        "[Zed Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut ZedAccountIndex, account: &ZedStoredAccount) {
    if let Some(summary) = index
        .accounts
        .iter_mut()
        .find(|item| item.id == account.public_account.id)
    {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn normalize_tags(tags: Vec<String>) -> Option<Vec<String>> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            normalized.push(trimmed.to_string());
        }
    }
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn list_stored_accounts_from_index(index: &ZedAccountIndex) -> Vec<ZedStoredAccount> {
    let mut accounts = Vec::new();
    for summary in &index.accounts {
        if let Some(account) = load_stored_account(&summary.id) {
            accounts.push(account);
        }
    }
    accounts.sort_by(|a, b| {
        b.public_account
            .last_used
            .cmp(&a.public_account.last_used)
            .then_with(|| a.public_account.id.cmp(&b.public_account.id))
    });
    accounts
}

pub fn list_accounts() -> Vec<ZedAccount> {
    let index = load_account_index();
    list_stored_accounts_from_index(&index)
        .into_iter()
        .map(|account| account.to_public())
        .collect()
}

pub fn list_accounts_checked() -> Result<Vec<ZedAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(list_stored_accounts_from_index(&index)
        .into_iter()
        .map(|account| account.to_public())
        .collect())
}

fn load_all_stored_accounts() -> Vec<ZedStoredAccount> {
    let index = load_account_index();
    list_stored_accounts_from_index(&index)
}

pub fn resolve_current_account_id() -> Option<String> {
    let index = load_account_index();
    let accounts = list_stored_accounts_from_index(&index);
    if accounts.is_empty() {
        return None;
    }

    if let Ok(Some(credentials)) = read_credentials_from_keychain() {
        if let Some(account) = accounts
            .iter()
            .find(|item| item.public_account.user_id == credentials.user_id)
        {
            return Some(account.public_account.id.clone());
        }
    }

    if let Some(current_id) = index.current_account_id {
        if accounts
            .iter()
            .any(|account| account.public_account.id == current_id)
        {
            return Some(current_id);
        }
    }

    accounts
        .first()
        .map(|account| account.public_account.id.clone())
}

pub fn set_current_account_id(account_id: Option<&str>) -> Result<(), String> {
    let _lock = ZED_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Zed 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.current_account_id = account_id.map(|value| value.to_string());
    save_account_index(&index)
}

fn upsert_account_record(
    mut account: ZedStoredAccount,
    set_current_if_missing: bool,
    force_current: bool,
) -> Result<ZedAccount, String> {
    let _lock = ZED_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Zed 账号锁失败".to_string())?;
    let mut index = load_account_index();

    if let Some(existing) = load_stored_account(&account.public_account.id) {
        account.public_account.created_at = existing.public_account.created_at;
        if account.public_account.tags.is_none() {
            account.public_account.tags = existing.public_account.tags;
        }
    }

    save_stored_account_file(&account)?;
    refresh_summary(&mut index, &account);

    if force_current {
        index.current_account_id = Some(account.public_account.id.clone());
    } else if set_current_if_missing && index.current_account_id.is_none() {
        index.current_account_id = Some(account.public_account.id.clone());
    }

    save_account_index(&index)?;
    Ok(account.to_public())
}

fn update_quota_query_error(
    account_id: &str,
    message: Option<String>,
) -> Result<Option<ZedAccount>, String> {
    let Some(mut stored) = load_stored_account(account_id) else {
        return Ok(None);
    };
    stored.public_account.quota_query_last_error = message;
    stored.public_account.quota_query_last_error_at = stored
        .public_account
        .quota_query_last_error
        .as_ref()
        .map(|_| chrono::Utc::now().timestamp_millis());
    let updated = upsert_account_record(stored, false, false)?;
    Ok(Some(updated))
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = ZED_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Zed 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| item.id != account_id);
    if index.current_account_id.as_deref() == Some(account_id) {
        index.current_account_id = index.accounts.first().map(|item| item.id.clone());
    }
    save_account_index(&index)?;
    delete_account_file(account_id)?;
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    let targets: HashSet<String> = account_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if targets.is_empty() {
        return Ok(());
    }

    let _lock = ZED_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Zed 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| !targets.contains(&item.id));
    if let Some(current_id) = index.current_account_id.clone() {
        if targets.contains(&current_id) {
            index.current_account_id = index.accounts.first().map(|item| item.id.clone());
        }
    }
    save_account_index(&index)?;

    for account_id in targets {
        delete_account_file(&account_id)?;
    }

    Ok(())
}

fn json_nested<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => normalize_non_empty(Some(text)),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Object(map) => map.get("limited").and_then(value_to_string),
        _ => None,
    }
}

fn json_nested_str(value: &Value, path: &[&str]) -> Option<String> {
    json_nested(value, path).and_then(value_to_string)
}

fn json_nested_i64(value: &Value, path: &[&str]) -> Option<i64> {
    json_nested(value, path).and_then(|raw| match raw {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|v| v.round() as i64)),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    })
}

fn json_nested_bool(value: &Value, path: &[&str]) -> Option<bool> {
    json_nested(value, path).and_then(|raw| match raw {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        Value::Number(number) => number.as_i64().map(|value| value != 0),
        _ => None,
    })
}

fn parse_timestamp_from_string(raw: &str) -> Option<i64> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return Some(parsed.timestamp());
    }
    if let Ok(parsed) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Utc
            .from_local_datetime(&parsed.and_hms_opt(0, 0, 0)?)
            .single()
            .map(|dt| dt.timestamp());
    }
    None
}

fn json_nested_timestamp(value: &Value, path: &[&str]) -> Option<i64> {
    json_nested(value, path).and_then(|raw| match raw {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => parse_timestamp_from_string(text.trim()),
        _ => None,
    })
}

fn pick_first_string(candidates: &[Option<String>]) -> Option<String> {
    for candidate in candidates {
        if let Some(value) = candidate.clone() {
            return Some(value);
        }
    }
    None
}

fn pick_first_i64(candidates: &[Option<i64>]) -> Option<i64> {
    for candidate in candidates {
        if candidate.is_some() {
            return *candidate;
        }
    }
    None
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("cockpit-tools/zed")
        .build()
        .map_err(|e| format!("创建 Zed HTTP 客户端失败: {}", e))
}

fn build_authorization_header(user_id: &str, access_token: &str) -> String {
    format!("{} {}", user_id.trim(), access_token.trim())
}

async fn fetch_json(
    client: &reqwest::Client,
    authorization_header: &str,
    path: &str,
) -> Result<Value, String> {
    let url = format!("{}{}", ZED_CLOUD_BASE_URL, path);
    let response = client
        .get(&url)
        .header("Authorization", authorization_header)
        .send()
        .await
        .map_err(|e| format!("请求 Zed 接口失败 ({}): {}", path, e))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "请求 Zed 接口失败 ({}): status={}, body_len={}",
            path,
            status,
            body.len()
        ));
    }

    response
        .json::<Value>()
        .await
        .map_err(|e| format!("解析 Zed 接口响应失败 ({}): {}", path, e))
}

async fn fetch_remote_bundle(user_id: &str, access_token: &str) -> Result<ZedFetchBundle, String> {
    let client = build_client()?;
    let authorization_header = build_authorization_header(user_id, access_token);

    // The official desktop client derives plan and usage state from /client/users/me.
    // The /frontend/billing/* endpoints are not part of the native client flow and
    // can return 401 for valid desktop credentials.
    let user_raw = fetch_json(&client, &authorization_header, "/client/users/me").await?;

    Ok(ZedFetchBundle {
        user_raw,
        subscription_raw: json!({}),
        usage_raw: json!({}),
        usage_tokens_raw: json!({}),
        preferences_raw: json!({}),
    })
}

fn build_account_id(user_id: &str) -> String {
    format!("zed_{}", sanitize_account_id_component(user_id))
}

fn build_stored_account_from_bundle(
    requested_user_id: &str,
    access_token: &str,
    bundle: ZedFetchBundle,
    existing: Option<&ZedStoredAccount>,
) -> Result<ZedStoredAccount, String> {
    let resolved_user_id = pick_first_string(&[
        json_nested_str(&bundle.user_raw, &["user", "id"]),
        json_nested_str(&bundle.user_raw, &["id"]),
        normalize_non_empty(Some(requested_user_id)),
    ])
    .ok_or_else(|| "Zed 用户信息缺少 user_id".to_string())?;

    let github_login = pick_first_string(&[
        json_nested_str(&bundle.user_raw, &["user", "github_login"]),
        json_nested_str(&bundle.user_raw, &["user", "githubLogin"]),
        json_nested_str(&bundle.user_raw, &["github_login"]),
        json_nested_str(&bundle.user_raw, &["githubLogin"]),
        existing.map(|item| item.public_account.github_login.clone()),
        normalize_non_empty(Some(&resolved_user_id)),
    ])
    .ok_or_else(|| "Zed 用户信息缺少 github_login".to_string())?;

    let plan_raw = pick_first_string(&[
        json_nested_str(&bundle.subscription_raw, &["subscription", "name"]),
        json_nested_str(&bundle.subscription_raw, &["name"]),
        json_nested_str(&bundle.user_raw, &["plan", "plan_v3"]),
        json_nested_str(&bundle.user_raw, &["plan", "plan"]),
        json_nested_str(&bundle.usage_raw, &["plan"]),
        json_nested_str(&bundle.user_raw, &["plan", "name"]),
    ]);

    let token_spend_used_cents = pick_first_i64(&[
        json_nested_i64(&bundle.usage_raw, &["current_usage", "token_spend", "used"]),
        json_nested_i64(&bundle.usage_raw, &["token_spend", "used"]),
    ]);
    let token_spend_limit_cents = pick_first_i64(&[
        json_nested_i64(
            &bundle.usage_raw,
            &["current_usage", "token_spend", "limit"],
        ),
        json_nested_i64(&bundle.usage_raw, &["token_spend", "limit"]),
    ]);
    let token_spend_remaining_cents = pick_first_i64(&[
        json_nested_i64(
            &bundle.usage_raw,
            &["current_usage", "token_spend", "remaining"],
        ),
        json_nested_i64(&bundle.usage_raw, &["token_spend", "remaining"]),
    ]);

    let edit_predictions_limit_raw = pick_first_string(&[
        json_nested(
            &bundle.usage_raw,
            &["current_usage", "edit_predictions", "limit"],
        )
        .and_then(value_to_string),
        json_nested(&bundle.usage_raw, &["edit_predictions", "limit"]).and_then(value_to_string),
    ]);
    let edit_predictions_remaining_raw = pick_first_string(&[
        json_nested(
            &bundle.usage_raw,
            &["current_usage", "edit_predictions", "remaining"],
        )
        .and_then(value_to_string),
        json_nested(&bundle.usage_raw, &["edit_predictions", "remaining"])
            .and_then(value_to_string),
    ]);

    let created_at = existing
        .map(|account| account.public_account.created_at)
        .unwrap_or_else(now_ts);

    Ok(ZedStoredAccount {
        public_account: ZedAccount {
            id: build_account_id(&resolved_user_id),
            user_id: resolved_user_id,
            github_login,
            display_name: pick_first_string(&[
                json_nested_str(&bundle.user_raw, &["user", "name"]),
                json_nested_str(&bundle.user_raw, &["name"]),
            ]),
            avatar_url: pick_first_string(&[
                json_nested_str(&bundle.user_raw, &["user", "avatar_url"]),
                json_nested_str(&bundle.user_raw, &["user", "avatarUrl"]),
                json_nested_str(&bundle.user_raw, &["avatar_url"]),
                json_nested_str(&bundle.user_raw, &["avatarUrl"]),
            ]),
            plan_raw,
            subscription_status: pick_first_string(&[
                json_nested_str(&bundle.subscription_raw, &["subscription", "status"]),
                json_nested_str(&bundle.subscription_raw, &["status"]),
            ]),
            has_overdue_invoices: json_nested_bool(
                &bundle.user_raw,
                &["plan", "has_overdue_invoices"],
            ),
            billing_period_start_at: pick_first_i64(&[
                json_nested_timestamp(
                    &bundle.subscription_raw,
                    &["subscription", "period", "start_at"],
                ),
                json_nested_timestamp(&bundle.subscription_raw, &["period", "start_at"]),
                json_nested_timestamp(
                    &bundle.user_raw,
                    &["plan", "subscription_period", "started_at"],
                ),
            ]),
            billing_period_end_at: pick_first_i64(&[
                json_nested_timestamp(
                    &bundle.subscription_raw,
                    &["subscription", "period", "end_at"],
                ),
                json_nested_timestamp(&bundle.subscription_raw, &["period", "end_at"]),
                json_nested_timestamp(
                    &bundle.user_raw,
                    &["plan", "subscription_period", "ended_at"],
                ),
            ]),
            trial_started_at: pick_first_i64(&[
                json_nested_timestamp(&bundle.preferences_raw, &["trial_started_at"]),
                json_nested_timestamp(
                    &bundle.subscription_raw,
                    &["subscription", "trial_started_at"],
                ),
                json_nested_timestamp(&bundle.subscription_raw, &["trial_started_at"]),
                json_nested_timestamp(&bundle.user_raw, &["plan", "trial_started_at"]),
            ]),
            trial_end_at: pick_first_i64(&[
                json_nested_timestamp(&bundle.subscription_raw, &["subscription", "trial_end_at"]),
                json_nested_timestamp(&bundle.subscription_raw, &["trial_end_at"]),
            ]),
            token_spend_used_cents,
            token_spend_limit_cents,
            token_spend_remaining_cents,
            edit_predictions_used: pick_first_i64(&[
                json_nested_i64(
                    &bundle.usage_raw,
                    &["current_usage", "edit_predictions", "used"],
                ),
                json_nested_i64(&bundle.usage_raw, &["edit_predictions", "used"]),
                json_nested_i64(
                    &bundle.user_raw,
                    &["plan", "usage", "edit_predictions", "used"],
                ),
            ]),
            edit_predictions_limit_raw: edit_predictions_limit_raw.or_else(|| {
                json_nested(
                    &bundle.user_raw,
                    &["plan", "usage", "edit_predictions", "limit"],
                )
                .and_then(value_to_string)
            }),
            edit_predictions_remaining_raw,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: pick_first_i64(&[
                json_nested_timestamp(&bundle.usage_tokens_raw, &["usage_cache_updated_at"]),
                json_nested_timestamp(&bundle.usage_raw, &["usage_cache_updated_at"]),
            ]),
            spending_limit_cents: pick_first_i64(&[
                json_nested_i64(
                    &bundle.preferences_raw,
                    &["max_monthly_llm_usage_spending_in_cents"],
                ),
                json_nested_i64(&bundle.preferences_raw, &["spend_limit_in_cents"]),
            ]),
            billing_portal_url: pick_first_string(&[
                json_nested_str(&bundle.usage_raw, &["portal_url"]),
                json_nested_str(&bundle.subscription_raw, &["portal_url"]),
                json_nested_str(&bundle.preferences_raw, &["portal_url"]),
            ]),
            tags: existing.and_then(|account| account.public_account.tags.clone()),
            user_raw: Some(bundle.user_raw),
            subscription_raw: Some(bundle.subscription_raw),
            usage_raw: Some(bundle.usage_raw),
            usage_tokens_raw: Some(bundle.usage_tokens_raw),
            preferences_raw: Some(bundle.preferences_raw),
            created_at,
            last_used: now_ts(),
        },
        access_token: access_token.trim().to_string(),
    })
}

pub async fn upsert_account_from_credentials(
    user_id: &str,
    access_token: &str,
) -> Result<ZedAccount, String> {
    let existing = load_all_stored_accounts()
        .into_iter()
        .find(|account| account.public_account.user_id == user_id.trim());
    let bundle = fetch_remote_bundle(user_id, access_token).await?;
    let stored_account =
        build_stored_account_from_bundle(user_id, access_token, bundle, existing.as_ref())?;
    upsert_account_record(stored_account, true, false)
}

pub async fn refresh_account(account_id: &str) -> Result<ZedAccount, String> {
    let stored =
        load_stored_account(account_id).ok_or_else(|| format!("Zed 账号不存在: {}", account_id))?;
    let bundle =
        match fetch_remote_bundle(&stored.public_account.user_id, &stored.access_token).await {
            Ok(bundle) => bundle,
            Err(err) => {
                let _ = update_quota_query_error(account_id, Some(err.clone()));
                return Err(err);
            }
        };
    let refreshed = build_stored_account_from_bundle(
        &stored.public_account.user_id,
        &stored.access_token,
        bundle,
        Some(&stored),
    )?;
    let updated = upsert_account_record(refreshed, false, false)?;
    let _ = update_quota_query_error(account_id, None)?;
    Ok(updated)
}

pub async fn refresh_all_accounts() -> Result<Vec<ZedAccount>, String> {
    let mut refreshed = Vec::new();
    for account in load_all_stored_accounts() {
        match refresh_account(&account.public_account.id).await {
            Ok(updated) => refreshed.push(updated),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Zed] 刷新账号失败: account_id={}, err={}",
                    account.public_account.id, err
                ));
            }
        }
    }
    Ok(refreshed)
}

pub async fn import_from_local() -> Result<ZedAccount, String> {
    let credentials = read_credentials_from_keychain()?
        .ok_or_else(|| "未在本机 Zed 客户端登录态中找到可导入的账号信息".to_string())?;
    upsert_account_from_credentials(&credentials.user_id, &credentials.access_token).await
}

pub fn import_from_json(json_content: &str) -> Result<Vec<ZedAccount>, String> {
    let trimmed = json_content.trim();
    if trimmed.is_empty() {
        return Err("导入内容不能为空".to_string());
    }

    let mut payload_current_id = None;
    let accounts: Vec<ZedStoredAccount> =
        if let Ok(payload) = serde_json::from_str::<ZedExportPayload>(trimmed) {
            payload_current_id = payload.current_account_id.clone();
            payload.accounts
        } else if let Ok(list) = serde_json::from_str::<Vec<ZedStoredAccount>>(trimmed) {
            list
        } else if let Ok(single) = serde_json::from_str::<ZedStoredAccount>(trimmed) {
            vec![single]
        } else {
            return Err("导入内容格式无效，需为 Zed 账号导出 JSON".to_string());
        };

    if accounts.is_empty() {
        return Ok(Vec::new());
    }

    let mut imported = Vec::new();
    for stored in accounts {
        let public = upsert_account_record(stored, true, false)?;
        imported.push(public);
    }

    if let Some(current_id) = payload_current_id {
        let _ = set_current_account_id(Some(&current_id));
    }

    Ok(imported)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let targets: Option<HashSet<String>> = if account_ids.is_empty() {
        None
    } else {
        Some(
            account_ids
                .iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect(),
        )
    };

    let mut accounts = Vec::new();
    for account in load_all_stored_accounts() {
        let include = targets
            .as_ref()
            .map(|target| target.contains(&account.public_account.id))
            .unwrap_or(true);
        if include {
            accounts.push(account);
        }
    }

    let payload = ZedExportPayload {
        version: "1.0".to_string(),
        current_account_id: resolve_current_account_id(),
        accounts,
    };

    serde_json::to_string_pretty(&payload).map_err(|e| format!("导出账号失败: {}", e))
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<ZedAccount, String> {
    let mut stored =
        load_stored_account(account_id).ok_or_else(|| format!("Zed 账号不存在: {}", account_id))?;
    stored.public_account.tags = normalize_tags(tags);
    upsert_account_record(stored, false, false)
}

pub fn inject_account(account_id: &str) -> Result<ZedAccount, String> {
    let stored =
        load_stored_account(account_id).ok_or_else(|| format!("Zed 账号不存在: {}", account_id))?;
    write_credentials_to_keychain(&stored.public_account.user_id, &stored.access_token)?;
    let public = upsert_account_record(stored, false, true)?;
    Ok(public)
}

pub fn clear_current_runtime_account() -> Result<(), String> {
    clear_credentials_from_keychain()?;
    set_current_account_id(None)
}

#[cfg(target_os = "macos")]
fn security_command_output(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("security")
        .args(args)
        .output()
        .map_err(|e| format!("执行 security 命令失败: {}", e))
}

#[cfg(target_os = "macos")]
fn parse_account_from_security_output(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.split("\"acct\"<blob>=\"").nth(1) {
            if let Some(value) = rest.split('"').next() {
                if let Some(normalized) = normalize_non_empty(Some(value)) {
                    return Some(normalized);
                }
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
pub fn read_credentials_from_keychain() -> Result<Option<ZedKeychainCredentials>, String> {
    let meta_output = security_command_output(&["find-internet-password", "-s", ZED_SERVER_URL])?;
    if !meta_output.status.success() {
        let stderr = String::from_utf8_lossy(&meta_output.stderr);
        if stderr.contains("could not be found") {
            return Ok(None);
        }
        return Err(format!(
            "读取 Zed Keychain 元数据失败: status={}, stderr={}",
            meta_output.status,
            stderr.trim()
        ));
    }

    let password_output =
        security_command_output(&["find-internet-password", "-s", ZED_SERVER_URL, "-w"])?;
    if !password_output.status.success() {
        let stderr = String::from_utf8_lossy(&password_output.stderr);
        return Err(format!(
            "读取 Zed Keychain 密码失败: status={}, stderr={}",
            password_output.status,
            stderr.trim()
        ));
    }

    let meta_text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&meta_output.stdout),
        String::from_utf8_lossy(&meta_output.stderr)
    );
    let user_id = parse_account_from_security_output(&meta_text)
        .ok_or_else(|| "解析 Zed Keychain 账号失败".to_string())?;
    let access_token = String::from_utf8_lossy(&password_output.stdout)
        .trim()
        .to_string();
    if access_token.is_empty() {
        return Err("Zed Keychain access_token 为空".to_string());
    }

    Ok(Some(ZedKeychainCredentials {
        user_id,
        access_token,
    }))
}

#[cfg(not(target_os = "macos"))]
pub fn read_credentials_from_keychain() -> Result<Option<ZedKeychainCredentials>, String> {
    Ok(None)
}

#[cfg(target_os = "macos")]
pub fn clear_credentials_from_keychain() -> Result<(), String> {
    loop {
        let output = security_command_output(&["delete-internet-password", "-s", ZED_SERVER_URL])?;
        if output.status.success() {
            continue;
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("could not be found") {
            return Ok(());
        }
        return Err(format!(
            "删除 Zed Keychain 凭据失败: status={}, stderr={}",
            output.status,
            stderr.trim()
        ));
    }
}

#[cfg(not(target_os = "macos"))]
pub fn clear_credentials_from_keychain() -> Result<(), String> {
    Err("Zed 切号当前仅支持 macOS".to_string())
}

#[cfg(target_os = "macos")]
pub fn write_credentials_to_keychain(user_id: &str, access_token: &str) -> Result<(), String> {
    let normalized_user_id =
        normalize_non_empty(Some(user_id)).ok_or_else(|| "Zed user_id 不能为空".to_string())?;
    let normalized_token = normalize_non_empty(Some(access_token))
        .ok_or_else(|| "Zed access_token 不能为空".to_string())?;

    clear_credentials_from_keychain()?;

    let output = security_command_output(&[
        "add-internet-password",
        "-U",
        "-a",
        &normalized_user_id,
        "-s",
        ZED_SERVER_URL,
        "-w",
        &normalized_token,
    ])?;
    if !output.status.success() {
        return Err(format!(
            "写入 Zed Keychain 凭据失败: status={}, stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    logger::log_info(&format!(
        "[Zed] 已覆盖 Keychain 登录信息: service={}, user_id={}",
        ZED_SERVER_URL, normalized_user_id
    ));
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn write_credentials_to_keychain(_user_id: &str, _access_token: &str) -> Result<(), String> {
    Err("Zed 切号当前仅支持 macOS".to_string())
}

fn display_account_label(account: &ZedAccount) -> String {
    normalize_non_empty(account.display_name.as_deref())
        .or_else(|| normalize_non_empty(Some(account.github_login.as_str())))
        .or_else(|| normalize_non_empty(Some(account.user_id.as_str())))
        .unwrap_or_else(|| account.id.clone())
}

fn parse_numeric_text(raw: Option<&str>) -> Option<f64> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }
    value
        .parse::<f64>()
        .ok()
        .filter(|parsed| parsed.is_finite())
}

fn compute_remaining_percent_i64(
    used: Option<i64>,
    limit: Option<i64>,
    remaining: Option<i64>,
) -> Option<i32> {
    let limit = limit?;
    if limit <= 0 {
        return None;
    }
    let remaining_value = remaining.unwrap_or_else(|| limit.saturating_sub(used.unwrap_or(0)));
    let percent = ((remaining_value.max(0) as f64 / limit as f64) * 100.0).round() as i32;
    Some(percent.clamp(0, 100))
}

fn compute_remaining_percent_f64(
    used: Option<f64>,
    limit: Option<f64>,
    remaining: Option<f64>,
) -> Option<i32> {
    let limit = limit?;
    if !limit.is_finite() || limit <= 0.0 {
        return None;
    }
    let remaining_value = remaining.unwrap_or_else(|| (limit - used.unwrap_or(0.0)).max(0.0));
    if !remaining_value.is_finite() {
        return None;
    }
    let percent = ((remaining_value.max(0.0) / limit) * 100.0).round() as i32;
    Some(percent.clamp(0, 100))
}

pub(crate) fn extract_quota_metrics(account: &ZedAccount) -> Vec<(String, i32)> {
    let mut metrics = Vec::new();

    if let Some(percent) = compute_remaining_percent_i64(
        account.token_spend_used_cents,
        account.token_spend_limit_cents,
        account.token_spend_remaining_cents,
    ) {
        metrics.push(("Token Spend".to_string(), percent));
    }

    let edit_limit = parse_numeric_text(account.edit_predictions_limit_raw.as_deref());
    let edit_remaining = parse_numeric_text(account.edit_predictions_remaining_raw.as_deref());
    let edit_used = account.edit_predictions_used.map(|value| value as f64);
    if let Some(percent) = compute_remaining_percent_f64(edit_used, edit_limit, edit_remaining) {
        metrics.push(("Edit Predictions".to_string(), percent));
    }

    metrics
}

fn average_quota_percentage(metrics: &[(String, i32)]) -> f64 {
    if metrics.is_empty() {
        return 0.0;
    }
    let total: i32 = metrics.iter().map(|(_, pct)| *pct).sum();
    total as f64 / metrics.len() as f64
}

fn build_quota_alert_cooldown_key(account_id: &str, threshold: i32) -> String {
    format!("zed:{}:{}", account_id, threshold)
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = ZED_QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < ZED_QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, threshold: i32) {
    if let Ok(mut state) = ZED_QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(account_id, threshold));
    }
}

fn pick_quota_alert_recommendation(
    accounts: &[ZedAccount],
    current_id: &str,
) -> Option<ZedAccount> {
    let mut candidates: Vec<ZedAccount> = accounts
        .iter()
        .filter(|account| account.id != current_id)
        .filter(|account| !extract_quota_metrics(account).is_empty())
        .cloned()
        .collect();

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|left, right| {
        let avg_left = average_quota_percentage(&extract_quota_metrics(left));
        let avg_right = average_quota_percentage(&extract_quota_metrics(right));
        avg_right
            .partial_cmp(&avg_left)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.last_used.cmp(&left.last_used))
    });

    candidates.into_iter().next()
}

pub fn run_quota_alert_if_needed(
) -> Result<Option<crate::modules::account::QuotaAlertPayload>, String> {
    let config = crate::modules::config::get_user_config();
    if !config.zed_quota_alert_enabled {
        return Ok(None);
    }

    let threshold = config.zed_quota_alert_threshold.clamp(0, 100);
    let accounts = list_accounts();
    let current_id = match resolve_current_account_id() {
        Some(id) => id,
        None => return Ok(None),
    };
    let current_account = match accounts.iter().find(|account| account.id == current_id) {
        Some(account) => account,
        None => return Ok(None),
    };

    let metrics = extract_quota_metrics(current_account);
    let low_models: Vec<(String, i32)> = metrics
        .into_iter()
        .filter(|(_, pct)| *pct <= threshold)
        .collect();

    if low_models.is_empty() {
        clear_quota_alert_cooldown(&current_id, threshold);
        return Ok(None);
    }

    let now = Utc::now().timestamp();
    let cooldown_key = build_quota_alert_cooldown_key(&current_id, threshold);
    if !should_emit_quota_alert(&cooldown_key, now) {
        return Ok(None);
    }

    let recommendation = pick_quota_alert_recommendation(&accounts, &current_id);
    let lowest_percentage = low_models.iter().map(|(_, pct)| *pct).min().unwrap_or(0);
    let payload = crate::modules::account::QuotaAlertPayload {
        platform: "zed".to_string(),
        current_account_id: current_id,
        current_email: display_account_label(current_account),
        threshold,
        threshold_display: None,
        lowest_percentage,
        low_models: low_models.into_iter().map(|(name, _)| name).collect(),
        recommended_account_id: recommendation.as_ref().map(|account| account.id.clone()),
        recommended_email: recommendation.as_ref().map(display_account_label),
        triggered_at: now,
    };

    crate::modules::account::dispatch_quota_alert(&payload);
    Ok(Some(payload))
}
