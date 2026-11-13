use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::cursor::{CursorAccount, CursorAccountIndex, CursorImportPayload, CursorSandUsage};
use crate::modules::{account, logger};

const ACCOUNTS_INDEX_FILE: &str = "cursor_accounts.json";
const ACCOUNTS_DIR: &str = "cursor_accounts";
const CURSOR_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 10 * 60;
const CURSOR_ACCESS_TOKEN_REFRESH_THRESHOLD_SECONDS: i64 = 5 * 60;

lazy_static::lazy_static! {
    static ref CURSOR_ACCOUNT_INDEX_LOCK: Mutex<()> = Mutex::new(());
    static ref CURSOR_QUOTA_ALERT_LAST_SENT: Mutex<HashMap<String, i64>> = Mutex::new(HashMap::new());
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

pub(crate) fn is_banned_account(account: &CursorAccount) -> bool {
    is_banned_status(account.status.as_deref())
        || is_banned_reason(account.status_reason.as_deref())
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Cursor 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
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

// ---------------------------------------------------------------------------
// Account file operations
// ---------------------------------------------------------------------------

pub fn load_account(account_id: &str) -> Option<CursorAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    match crate::modules::secure_account_storage::deserialize_account_file::<CursorAccount>(
        &account_path,
        &content,
    ) {
        Ok((account, needs_rotation)) => {
            if needs_rotation {
                let account_for_rewrite = account.clone();
                crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
                    "cursor",
                    account_for_rewrite.id.clone(),
                    account_path.clone(),
                    content.as_bytes(),
                    move || {
                        crate::modules::secure_account_storage::serialize_account_file(
                            "cursor",
                            &account_for_rewrite,
                        )
                    },
                );
            }
            Some(account)
        }
        Err(_) => {
            let backup = cursor_account_recovery::detail_backup(&account_path)?;
            let content = fs::read_to_string(backup).ok()?;
            let restored = decode_recovery_detail(&content).ok()?;
            (restored.id == account_id).then_some(restored)
        }
    }
}

fn save_account_file(account: &CursorAccount) -> Result<(), String> {
    let path = resolve_account_file_path(account.id.as_str())?;
    crate::modules::secure_account_storage::write_account_preserving_switch_time(&path, "cursor", account)
        .map_err(|e| format!("保存账号失败: {}", e))
}

// Record credential-switch success separately from refresh activity.
pub fn touch_last_used(account_id: &str) -> Result<(), String> {
    let mut account = load_account(account_id)
        .ok_or_else(|| format!("Cursor account not found: {}", account_id))?;
    account.last_switched_at = Some(now_ts());
    save_account_file(&account)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Index operations
// ---------------------------------------------------------------------------

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    let _lock = crate::modules::account_file_crypto::lock_account_file_for_write(&path)?;
    crate::modules::atomic_write::remove_file_locked(&path).map(|_| ())
}

include!("../../../crates/cockpit-core/src/modules/cursor_account_index_flow.rs");

fn refresh_summary(index: &mut CursorAccountIndex, account: &CursorAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: CursorAccount) -> Result<CursorAccount, String> {
    let _lock = CURSOR_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Cursor 账号锁失败".to_string())?;
    let mut index = load_account_index_checked()?;
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
}

fn persist_quota_query_error(account_id: &str, message: &str) {
    let Ok(_lock) = CURSOR_ACCOUNT_INDEX_LOCK.lock() else {
        return;
    };
    let Some(mut account) = load_account(account_id) else {
        return;
    };
    account.quota_query_last_error = Some(message.to_string());
    account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    let _ = save_account_file(&account);
}

// ---------------------------------------------------------------------------
// Identity helpers
// ---------------------------------------------------------------------------

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

fn normalize_auth_identity(value: Option<&str>) -> Option<String> {
    normalize_non_empty(value)
}

fn decode_access_token_payload(access_token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = access_token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let payload_b64 = parts[1].replace('-', "+").replace('_', "/");
    let padded = match payload_b64.len() % 4 {
        2 => format!("{}==", payload_b64),
        3 => format!("{}=", payload_b64),
        _ => payload_b64,
    };

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(padded)
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn extract_auth_id_from_access_token(access_token: &str) -> Option<String> {
    let value = decode_access_token_payload(access_token)?;
    normalize_non_empty(value.get("sub").and_then(|raw| raw.as_str()))
}

fn extract_access_token_exp(access_token: &str) -> Option<i64> {
    let value = decode_access_token_payload(access_token)?;
    value.get("exp").and_then(|raw| raw.as_i64())
}

fn access_token_needs_refresh(access_token: &str) -> bool {
    let Some(exp) = extract_access_token_exp(access_token) else {
        return true;
    };
    exp <= now_ts() + CURSOR_ACCESS_TOKEN_REFRESH_THRESHOLD_SECONDS
}

fn extract_auth_id_from_raw_value(raw: Option<&Value>) -> Option<String> {
    let obj = raw.and_then(|value| value.as_object())?;

    normalize_auth_identity(
        obj.get("authId")
            .and_then(|value| value.as_str())
            .or_else(|| obj.get("auth_id").and_then(|value| value.as_str()))
            .or_else(|| obj.get("workosId").and_then(|value| value.as_str()))
            .or_else(|| obj.get("workos_id").and_then(|value| value.as_str())),
    )
}

fn resolve_payload_auth_id(payload: &CursorImportPayload) -> Option<String> {
    normalize_auth_identity(payload.auth_id.as_deref())
        .or_else(|| extract_auth_id_from_raw_value(payload.cursor_auth_raw.as_ref()))
        .or_else(|| extract_auth_id_from_access_token(payload.access_token.as_str()))
}

fn resolve_account_auth_id(account: &CursorAccount) -> Option<String> {
    normalize_auth_identity(account.auth_id.as_deref())
        .or_else(|| extract_auth_id_from_raw_value(account.cursor_auth_raw.as_ref()))
        .or_else(|| extract_auth_id_from_access_token(account.access_token.as_str()))
}

fn cursor_auth_raw_object_mut(account: &mut CursorAccount) -> &mut serde_json::Map<String, Value> {
    if !matches!(account.cursor_auth_raw, Some(Value::Object(_))) {
        account.cursor_auth_raw = Some(Value::Object(serde_json::Map::new()));
    }

    match account.cursor_auth_raw.as_mut() {
        Some(Value::Object(obj)) => obj,
        _ => unreachable!("cursor_auth_raw 应始终为对象"),
    }
}

fn upsert_cursor_auth_raw_string(account: &mut CursorAccount, key: &str, value: Option<String>) {
    let Some(text) = normalize_non_empty(value.as_deref()) else {
        return;
    };
    cursor_auth_raw_object_mut(account).insert(key.to_string(), Value::String(text));
}

fn upsert_cursor_auth_raw_bool(account: &mut CursorAccount, key: &str, value: Option<bool>) {
    let Some(flag) = value else {
        return;
    };
    cursor_auth_raw_object_mut(account).insert(key.to_string(), Value::Bool(flag));
}

fn normalize_cursor_sign_up_type(value: Option<&str>) -> Option<String> {
    let raw = normalize_non_empty(value)?;
    match raw.as_str() {
        "SIGN_UP_TYPE_AUTH_0" => Some("Auth_0".to_string()),
        "SIGN_UP_TYPE_GOOGLE" => Some("Google".to_string()),
        "SIGN_UP_TYPE_GITHUB" => Some("Github".to_string()),
        "SIGN_UP_TYPE_WORKOS" => Some("WorkOS".to_string()),
        _ => Some(raw),
    }
}

// ---------------------------------------------------------------------------
// Merge helpers
// ---------------------------------------------------------------------------

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

fn merge_duplicate_account(primary: &mut CursorAccount, duplicate: &CursorAccount) {
    fill_if_empty_string(&mut primary.email, duplicate.email.as_str());
    fill_if_empty_string(&mut primary.access_token, duplicate.access_token.as_str());

    fill_if_none(&mut primary.auth_id, &duplicate.auth_id);
    fill_if_none(&mut primary.name, &duplicate.name);
    fill_if_none(&mut primary.refresh_token, &duplicate.refresh_token);
    fill_if_none(&mut primary.membership_type, &duplicate.membership_type);
    fill_if_none(
        &mut primary.subscription_status,
        &duplicate.subscription_status,
    );
    fill_if_none(&mut primary.sign_up_type, &duplicate.sign_up_type);
    fill_if_none(&mut primary.cursor_auth_raw, &duplicate.cursor_auth_raw);
    fill_if_none(&mut primary.cursor_usage_raw, &duplicate.cursor_usage_raw);
    fill_if_none(&mut primary.status, &duplicate.status);
    fill_if_none(&mut primary.status_reason, &duplicate.status_reason);

    primary.tags = merge_string_list(primary.tags.clone(), duplicate.tags.clone());
    primary.created_at = primary.created_at.min(duplicate.created_at);
    primary.last_used = primary.last_used.max(duplicate.last_used);
    primary.last_switched_at = primary.last_switched_at.max(duplicate.last_switched_at);
}

fn choose_primary_account_index(group: &[usize], accounts: &[CursorAccount]) -> usize {
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

fn schedule_cursor_recovery(store: IndexStore<CursorAccountIndex>) -> Result<(), String> {
    let directory = get_data_dir()?.join(ACCOUNTS_DIR);
    let mut job = cursor_account_recovery::CursorRecovery::new(
        store, directory, decode_recovery_detail, normalize_account_id, &CURSOR_ACCOUNT_INDEX_LOCK,
    );
    crate::modules::background_maintenance::schedule(
        job.migration_key(),
        move || {
            let result = job.run_batch();
            if matches!(&result, Ok(true) | Err(_)) && job.take_refresh_needed() {
                if let Some(app) = crate::get_app_handle() {
                    use tauri::Emitter;
                    let _ = app.emit("cursor:index-recovered", ());
                }
            }
            result
        },
    );
    Ok(())
}

fn decode_recovery_detail(content: &str) -> Result<CursorAccount, String> {
    serde_json::from_str(content).or_else(|_| {
        crate::modules::secure_account_storage::deserialize_encrypted_snapshot(content)
    })
}

fn cursor_recovery_busy() -> String {
    crate::modules::background_maintenance::busy_message()
}

pub(crate) fn schedule_index_migration() -> Result<(), String> {
    schedule_cursor_recovery(index_store()?)
}

fn apply_payload(
    account: &mut CursorAccount,
    payload: CursorImportPayload,
    resolved_auth_id: Option<String>,
) {
    let incoming_email = payload.email.trim().to_string();
    if !incoming_email.is_empty() {
        account.email = incoming_email;
    } else if !account.email.contains('@') {
        account.email.clear();
    }
    account.name = payload.name;
    account.access_token = payload.access_token;
    account.refresh_token = payload.refresh_token;
    account.membership_type = payload.membership_type;
    account.subscription_status = payload.subscription_status;
    account.sign_up_type = payload.sign_up_type;
    account.cursor_auth_raw = payload.cursor_auth_raw;
    account.cursor_usage_raw = payload.cursor_usage_raw;
    if let Some(auth_id) = resolved_auth_id {
        account.auth_id = Some(auth_id.clone());
        upsert_cursor_auth_raw_string(account, "authId", Some(auth_id));
    }
    account.status = payload.status;
    account.status_reason = payload.status_reason;
    account.last_used = now_ts();
}

pub fn upsert_account(payload: CursorImportPayload) -> Result<CursorAccount, String> {
    let _lock = CURSOR_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Cursor 账号锁失败".to_string())?;

    let now = now_ts();
    let mut index = load_account_index_checked()?;
    let incoming_auth_id = resolve_payload_auth_id(&payload);
    let incoming_email = normalize_email_identity(Some(payload.email.as_str()));
    let incoming_token = normalize_token_identity(Some(payload.access_token.as_str()));

    let identity_seed = incoming_auth_id
        .clone()
        .or_else(|| incoming_email.clone())
        .or_else(|| incoming_token.clone())
        .unwrap_or_else(|| "cursor_user".to_string())
        .to_lowercase();
    let generated_id = format!("cursor_{:x}", md5::compute(identity_seed.as_bytes()));

    let account_id = index
        .accounts
        .iter()
        .filter_map(|item| load_account(&item.id))
        .find(|account| {
            let existing_auth_id = resolve_account_auth_id(account);
            if let (Some(existing), Some(incoming)) =
                (existing_auth_id.as_ref(), incoming_auth_id.as_ref())
            {
                return existing == incoming;
            }
            if existing_auth_id.is_some() || incoming_auth_id.is_some() {
                return false;
            }

            let existing_email = normalize_email_identity(Some(account.email.as_str()));
            let existing_token = normalize_token_identity(Some(account.access_token.as_str()));
            if let (Some(ex), Some(inc)) = (existing_email.as_ref(), incoming_email.as_ref()) {
                if ex == inc {
                    return true;
                }
            }
            if let (Some(ex), Some(inc)) = (existing_token.as_ref(), incoming_token.as_ref()) {
                if ex == inc {
                    return true;
                }
            }
            false
        })
        .map(|account| account.id)
        .unwrap_or(generated_id);

    let existing = load_account(&account_id);
    let tags = existing.as_ref().and_then(|acc| acc.tags.clone());
    let created_at = existing.as_ref().map(|acc| acc.created_at).unwrap_or(now);

    let mut account = existing.unwrap_or(CursorAccount {
        last_switched_at: None,
        id: account_id.clone(),
        email: payload.email.clone(),
        auth_id: incoming_auth_id.clone(),
        name: payload.name.clone(),
        tags,
        access_token: payload.access_token.clone(),
        refresh_token: payload.refresh_token.clone(),
        membership_type: payload.membership_type.clone(),
        subscription_status: payload.subscription_status.clone(),
        sign_up_type: payload.sign_up_type.clone(),
        cursor_auth_raw: payload.cursor_auth_raw.clone(),
        cursor_usage_raw: payload.cursor_usage_raw.clone(),
        sand_usage: None,
        status: payload.status.clone(),
        status_reason: payload.status_reason.clone(),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        created_at,
        last_used: now,
    });

    apply_payload(&mut account, payload, incoming_auth_id);
    account.id = account_id;
    account.created_at = created_at;
    account.quota_query_last_error = None;
    account.quota_query_last_error_at = None;
    account.last_used = now;

    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;

    logger::log_info(&format!(
        "Cursor 账号已保存: id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = CURSOR_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Cursor 账号锁失败".to_string())?;
    let path = resolve_account_file_path(account_id)?;
    let _file_lock = crate::modules::account_file_crypto::lock_account_file_for_write(&path)?;
    let mut index = load_account_index_checked()?;
    index.accounts.retain(|item| item.id != account_id);
    index_store()?.replace_with_removals(&index, &[account_id.to_string()])?;
    crate::modules::atomic_write::remove_file_locked(&path)
        .map_err(|error| format!("删除账号文件失败: {error}"))?;
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        remove_account(id)?;
    }
    Ok(())
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<CursorAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    account.tags = Some(tags);
    account.last_used = now_ts();
    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

// ---------------------------------------------------------------------------
// Import / Export
// ---------------------------------------------------------------------------

fn clone_object_value(value: Option<&Value>) -> Option<Value> {
    value.and_then(|raw| {
        if raw.is_object() {
            Some(raw.clone())
        } else {
            None
        }
    })
}

fn extract_string(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = obj.get(*key) {
            if let Some(text) = value.as_str().map(str::trim).filter(|v| !v.is_empty()) {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn payload_from_import_value(raw: Value) -> Result<CursorImportPayload, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| "Cursor 导入 JSON 必须是对象".to_string())?;

    let email = extract_string(obj, &["email", "cachedEmail", "cursor_email"])
        .ok_or_else(|| "缺少 email 字段".to_string())?;
    let access_token = extract_string(
        obj,
        &[
            "access_token",
            "accessToken",
            "token",
            "cursor_access_token",
        ],
    )
    .ok_or_else(|| "缺少 access_token 字段".to_string())?;

    let name = extract_string(obj, &["name", "displayName"]);
    let refresh_token = extract_string(
        obj,
        &["refresh_token", "refreshToken", "cursor_refresh_token"],
    );
    let membership_type = extract_string(
        obj,
        &[
            "membership_type",
            "membershipType",
            "stripeMembershipType",
            "plan",
        ],
    );
    let subscription_status = extract_string(
        obj,
        &[
            "subscription_status",
            "subscriptionStatus",
            "stripeSubscriptionStatus",
        ],
    );
    let sign_up_type = extract_string(obj, &["sign_up_type", "signUpType", "cachedSignUpType"]);
    let status = extract_string(obj, &["status"]);
    let status_reason = extract_string(obj, &["status_reason", "statusReason"]);

    let cursor_auth_raw = clone_object_value(obj.get("cursor_auth_raw"))
        .or_else(|| clone_object_value(obj.get("cursorAuthRaw")));
    let cursor_usage_raw = clone_object_value(obj.get("cursor_usage_raw"))
        .or_else(|| clone_object_value(obj.get("cursorUsageRaw")));
    let auth_id = extract_string(obj, &["auth_id", "authId", "workos_id", "workosId"])
        .or_else(|| extract_auth_id_from_raw_value(cursor_auth_raw.as_ref()))
        .or_else(|| extract_auth_id_from_access_token(access_token.as_str()));

    Ok(CursorImportPayload {
        email,
        auth_id,
        name,
        access_token,
        refresh_token,
        membership_type,
        subscription_status,
        sign_up_type,
        cursor_auth_raw,
        cursor_usage_raw,
        status,
        status_reason,
    })
}

fn payloads_from_import_json_value(value: Value) -> Result<Vec<CursorImportPayload>, String> {
    match value {
        Value::Array(items) => {
            if items.is_empty() {
                return Err("导入数组为空".to_string());
            }
            let mut payloads = Vec::with_capacity(items.len());
            for (idx, item) in items.into_iter().enumerate() {
                let payload = payload_from_import_value(item)
                    .map_err(|e| format!("第 {} 条 Cursor 账号解析失败: {}", idx + 1, e))?;
                payloads.push(payload);
            }
            Ok(payloads)
        }
        Value::Object(mut obj) => {
            let object_value = Value::Object(obj.clone());
            if let Ok(payload) = payload_from_import_value(object_value) {
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
                        .map_err(|e| format!("第 {} 条 Cursor 账号解析失败: {}", idx + 1, e))?;
                    payloads.push(payload);
                }
                return Ok(payloads);
            }

            Err("无法解析 Cursor 导入对象".to_string())
        }
        _ => Err("Cursor 导入 JSON 必须是对象或数组".to_string()),
    }
}

pub fn import_from_json(json_content: &str) -> Result<Vec<CursorAccount>, String> {
    if let Ok(account) = serde_json::from_str::<CursorAccount>(json_content) {
        let saved = upsert_account_record(account)?;
        return Ok(vec![saved]);
    }

    if let Ok(accounts) = serde_json::from_str::<Vec<CursorAccount>>(json_content) {
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
    let accounts: Vec<CursorAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

// ---------------------------------------------------------------------------
// Local import (read from Cursor's state.vscdb)
// ---------------------------------------------------------------------------

pub fn get_default_cursor_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join("Library/Application Support/Cursor"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        return Ok(PathBuf::from(appdata).join("Cursor"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".config/Cursor"));
    }

    #[allow(unreachable_code)]
    Err("Cursor 账号导入仅支持 macOS、Windows 和 Linux".to_string())
}

pub fn get_default_cursor_state_db_path() -> Result<PathBuf, String> {
    Ok(get_default_cursor_data_dir()?
        .join("User")
        .join("globalStorage")
        .join("state.vscdb"))
}

fn read_vscdb_item(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .optional()
    .ok()
    .flatten()
    .and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

pub fn read_local_cursor_auth() -> Result<Option<CursorImportPayload>, String> {
    let db_path = get_default_cursor_state_db_path()?;
    if !db_path.exists() {
        return Ok(None);
    }

    let conn = Connection::open(&db_path)
        .map_err(|e| format!("打开 Cursor 本地数据库失败({}): {}", db_path.display(), e))?;

    let access_token = match read_vscdb_item(&conn, "cursorAuth/accessToken") {
        Some(t) => t,
        None => return Ok(None),
    };

    let email = read_vscdb_item(&conn, "cursorAuth/cachedEmail").unwrap_or_default();
    if email.is_empty() {
        return Ok(None);
    }

    let refresh_token = read_vscdb_item(&conn, "cursorAuth/refreshToken");
    let auth_id = read_vscdb_item(&conn, "cursorAuth/authId")
        .or_else(|| extract_auth_id_from_access_token(access_token.as_str()));
    let membership_type = read_vscdb_item(&conn, "cursorAuth/stripeMembershipType");
    let subscription_status = read_vscdb_item(&conn, "cursorAuth/stripeSubscriptionStatus");
    let sign_up_type = read_vscdb_item(&conn, "cursorAuth/cachedSignUpType");

    let mut auth_raw = serde_json::Map::new();
    auth_raw.insert(
        "accessToken".to_string(),
        Value::String(access_token.clone()),
    );
    if let Some(ref rt) = refresh_token {
        auth_raw.insert("refreshToken".to_string(), Value::String(rt.clone()));
    }
    if let Some(ref auth_id_value) = auth_id {
        auth_raw.insert("authId".to_string(), Value::String(auth_id_value.clone()));
    }
    auth_raw.insert("cachedEmail".to_string(), Value::String(email.clone()));
    if let Some(ref mt) = membership_type {
        auth_raw.insert(
            "stripeMembershipType".to_string(),
            Value::String(mt.clone()),
        );
    }
    if let Some(ref ss) = subscription_status {
        auth_raw.insert(
            "stripeSubscriptionStatus".to_string(),
            Value::String(ss.clone()),
        );
    }
    if let Some(ref st) = sign_up_type {
        auth_raw.insert("cachedSignUpType".to_string(), Value::String(st.clone()));
    }

    Ok(Some(CursorImportPayload {
        email,
        auth_id,
        name: None,
        access_token,
        refresh_token,
        membership_type,
        subscription_status,
        sign_up_type,
        cursor_auth_raw: Some(Value::Object(auth_raw)),
        cursor_usage_raw: None,
        status: None,
        status_reason: None,
    }))
}

pub fn import_from_local() -> Result<Option<CursorAccount>, String> {
    let payload = match read_local_cursor_auth()? {
        Some(p) => p,
        None => return Ok(None),
    };
    let account = upsert_account(payload)?;
    logger::log_info(&format!(
        "[Cursor Account] 从本地导入成功: id={}, email={}",
        account.id, account.email
    ));
    Ok(Some(account))
}

// ---------------------------------------------------------------------------
// Inject (write auth fields back to Cursor's state.vscdb)
// ---------------------------------------------------------------------------

fn upsert_vscdb_item(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
        (key, value),
    )
    .map_err(|e| format!("写入 {} 失败: {}", key, e))?;
    Ok(())
}

// Cursor 3.27 owns membership caches by the JWT subject. Both entry points
// replace the complete session in one transaction, including absent fields.
fn write_cursor_auth(conn: &mut Connection, account: &CursorAccount) -> Result<(), String> {
    conn.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for key in [
        "cursorAuth/refreshToken",
        "cursorAuth/stripeMembershipType",
        "cursorAuth/stripeMembershipAuthId",
        "cursorAuth/stripeSubscriptionStatus",
        "cursorAuth/stripeCustomerId",
        "cursorAuth/cachedSignUpType",
        "cursorAuth/cachedScopedProfile",
        "cursorAuth/cachedTeam",
    ] {
        tx.execute("DELETE FROM ItemTable WHERE key = ?1", [key])
            .map_err(|e| e.to_string())?;
    }
    for (key, value) in [
        ("cursorAuth/accessToken", account.access_token.as_str()),
        ("cursorAuth/cachedEmail", account.email.as_str()),
        ("cursor.accessToken", account.access_token.as_str()),
        ("cursor.email", account.email.as_str()),
    ] {
        upsert_vscdb_item(&tx, key, value)?;
    }
    for (key, value) in [
        ("cursorAuth/refreshToken", account.refresh_token.as_deref()),
        (
            "cursorAuth/stripeMembershipType",
            account.membership_type.as_deref(),
        ),
        (
            "cursorAuth/stripeSubscriptionStatus",
            account.subscription_status.as_deref(),
        ),
        (
            "cursorAuth/cachedSignUpType",
            account.sign_up_type.as_deref(),
        ),
    ] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            upsert_vscdb_item(&tx, key, value)?;
        }
    }
    if account.membership_type.is_some() {
        if let Some(auth_id) = extract_auth_id_from_access_token(&account.access_token) {
            upsert_vscdb_item(&tx, "cursorAuth/stripeMembershipAuthId", &auth_id)?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

pub fn inject_to_cursor(account_id: &str) -> Result<(), String> {
    let account =
        load_account(account_id).ok_or_else(|| format!("Cursor 账号不存在: {}", account_id))?;
    let db_path = get_default_cursor_state_db_path()?;
    if !db_path.exists() {
        return Err(format!("Cursor state.vscdb 不存在: {}", db_path.display()));
    }

    let mut conn =
        Connection::open(&db_path).map_err(|e| format!("打开 Cursor 本地数据库失败: {}", e))?;

    write_cursor_auth(&mut conn, &account)?;
    touch_last_used(account_id)?;

    logger::log_info(&format!(
        "[Cursor Account] 注入成功: id={}, email={}",
        account.id, account.email
    ));
    Ok(())
}

pub fn inject_to_cursor_at_path(db_path: &std::path::Path, account_id: &str) -> Result<(), String> {
    let account =
        load_account(account_id).ok_or_else(|| format!("Cursor 账号不存在: {}", account_id))?;
    if !db_path.exists() {
        return Err(format!("Cursor state.vscdb 不存在: {}", db_path.display()));
    }

    let mut conn =
        Connection::open(db_path).map_err(|e| format!("打开 Cursor 本地数据库失败: {}", e))?;

    write_cursor_auth(&mut conn, &account)?;
    touch_last_used(account_id)?;

    logger::log_info(&format!(
        "[Cursor Account] 注入成功(自定义路径): id={}, email={}, path={}",
        account.id,
        account.email,
        db_path.display()
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Cursor usage API
// ---------------------------------------------------------------------------

const CURSOR_USAGE_SUMMARY_URL: &str = "https://cursor.com/api/usage-summary";
const CURSOR_SAND_USAGE_URL: &str = "https://cursor.com/api/dashboard/get-sand-usage-status";
const CURSOR_GET_USER_META_URL: &str = "https://api2.cursor.sh/aiserver.v1.AuthService/GetUserMeta";
const CURSOR_FULL_STRIPE_PROFILE_URL: &str = "https://api2.cursor.sh/auth/full_stripe_profile";
const CURSOR_STRIPE_PROFILE_URL: &str = "https://api2.cursor.sh/auth/stripe_profile";
// 与官方 Cursor 客户端保持一致：使用 api2.cursor.sh/oauth/token 和内置 client_id 交换新 token。
const CURSOR_OAUTH_TOKEN_URL: &str = "https://api2.cursor.sh/oauth/token";
const CURSOR_AUTH_CLIENT_ID: &str = "KbZUR41cY7W6zRSdpSUJ7I7mLYBKOCmB";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorUserMetaResponse {
    email: Option<String>,
    sign_up_type: Option<String>,
    workos_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorStripeProfileResponse {
    membership_type: Option<String>,
    individual_membership_type: Option<String>,
    subscription_status: Option<String>,
    team_membership_type: Option<String>,
    is_team_member: Option<bool>,
    is_enterprise: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct CursorRefreshTokenResponse {
    #[serde(alias = "accessToken")]
    access_token: Option<String>,
    #[serde(alias = "refreshToken")]
    refresh_token: Option<String>,
    #[serde(default, alias = "shouldLogout")]
    should_logout: bool,
}

fn format_error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(err) = source {
        let detail = err.to_string();
        if !detail.trim().is_empty() && parts.last().map(|item| item != &detail).unwrap_or(true) {
            parts.push(detail);
        }
        source = err.source();
    }
    parts.join(" | caused by: ")
}

pub(super) fn format_reqwest_error(error: &reqwest::Error) -> String {
    let mut tags = Vec::new();
    if error.is_timeout() {
        tags.push("timeout");
    }
    if error.is_connect() {
        tags.push("connect");
    }
    if error.is_request() {
        tags.push("request");
    }

    let detail = format_error_chain(error);
    if tags.is_empty() {
        detail
    } else {
        format!("{} [{}]", detail, tags.join(","))
    }
}

fn build_cursor_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", format_reqwest_error(&e)))
}

fn extract_workos_user_id(jwt: &str) -> Option<String> {
    let value = decode_access_token_payload(jwt)?;
    let sub = value.get("sub")?.as_str()?;
    let user_id = sub.rsplit('|').next().unwrap_or(sub);
    if user_id.starts_with("user_") {
        Some(user_id.to_string())
    } else {
        None
    }
}

pub(crate) fn normalize_pasted_access_token(input: &str) -> Result<String, String> {
    const INVALID_BROWSER_TOKEN: &str = "CURSOR_INVALID_BROWSER_TOKEN";
    let token = input.trim();
    let cookie_value = token.strip_prefix("WorkosCursorSessionToken=");
    let has_session_prefix =
        token.starts_with("user_") && (token.contains("::") || token.contains('%'));
    if cookie_value.is_none() && !has_session_prefix {
        return Ok(token.to_owned());
    }

    let decoded = urlencoding::decode(cookie_value.unwrap_or(token))
        .map_err(|_| INVALID_BROWSER_TOKEN.to_owned())?;
    let (user_id, jwt) = decoded
        .split_once("::")
        .ok_or_else(|| INVALID_BROWSER_TOKEN.to_owned())?;
    if extract_workos_user_id(jwt).as_deref() != Some(user_id)
        || jwt.split('.').count() != 3
        || jwt.split('.').any(str::is_empty)
        || jwt
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(INVALID_BROWSER_TOKEN.to_owned());
    }
    Ok(jwt.to_owned())
}

fn build_session_cookie(access_token: &str) -> Option<String> {
    let user_id = extract_workos_user_id(access_token)?;
    Some(format!(
        "WorkosCursorSessionToken={}%3A%3A{}",
        user_id, access_token
    ))
}

fn resolve_membership_from_stripe_profile(profile: &CursorStripeProfileResponse) -> Option<String> {
    let membership = normalize_non_empty(profile.membership_type.as_deref());
    let individual = normalize_non_empty(profile.individual_membership_type.as_deref());

    if let Some(individual_value) = individual.as_ref() {
        if !individual_value.eq_ignore_ascii_case("free")
            && !matches!(
                membership.as_deref(),
                Some(value) if value.eq_ignore_ascii_case("enterprise")
            )
        {
            return Some(individual_value.clone());
        }
    }

    membership.or(individual)
}

async fn exchange_refresh_token_with_client(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<CursorRefreshTokenResponse, String> {
    let response = client
        .post(CURSOR_OAUTH_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": CURSOR_AUTH_CLIENT_ID,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .map_err(|e| {
            format!(
                "请求 Cursor token 刷新接口失败: {}",
                format_reqwest_error(&e)
            )
        })?;

    let status = response.status().as_u16();
    let body = response.text().await.map_err(|e| {
        format!(
            "读取 Cursor token 刷新响应失败: {}",
            format_reqwest_error(&e)
        )
    })?;

    if status == 401 || status == 403 {
        return Err("Cursor refresh token 已过期或无效，请重新导入账号".to_string());
    }
    if status != 200 {
        let detail = body.trim();
        return Err(if detail.is_empty() {
            format!("Cursor token 刷新接口返回异常状态码: {}", status)
        } else {
            format!(
                "Cursor token 刷新接口返回异常状态码: {}, body_len={}",
                status,
                body.len()
            )
        });
    }

    serde_json::from_str::<CursorRefreshTokenResponse>(&body)
        .map_err(|e| format!("解析 Cursor token 刷新响应失败: {}", e))
}

async fn refresh_account_access_token_with_client(
    client: &reqwest::Client,
    account: &mut CursorAccount,
) -> Result<bool, String> {
    let Some(refresh_token) = normalize_non_empty(account.refresh_token.as_deref()) else {
        return Ok(false);
    };

    let response = exchange_refresh_token_with_client(client, refresh_token.as_str()).await?;
    if response.should_logout {
        return Err("Cursor refresh token 已失效，请重新导入账号".to_string());
    }

    let new_access_token = normalize_non_empty(response.access_token.as_deref())
        .ok_or_else(|| "Cursor token 刷新响应缺少 access_token".to_string())?;
    let new_refresh_token =
        normalize_non_empty(response.refresh_token.as_deref()).or(Some(refresh_token));

    account.access_token = new_access_token.clone();
    account.refresh_token = new_refresh_token.clone();
    upsert_cursor_auth_raw_string(account, "accessToken", Some(new_access_token));
    upsert_cursor_auth_raw_string(account, "refreshToken", new_refresh_token);
    Ok(true)
}

async fn fetch_user_meta_with_client(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<CursorUserMetaResponse, String> {
    let response = client
        .post(CURSOR_GET_USER_META_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| format!("请求 Cursor user meta 失败: {}", format_reqwest_error(&e)))?;

    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        return Err("Cursor 会话已过期或未认证，请重新导入账号".to_string());
    }
    if status != 200 {
        return Err(format!("Cursor user meta API 返回异常状态码: {}", status));
    }

    let body = response.text().await.map_err(|e| {
        format!(
            "读取 Cursor user meta 响应失败: {}",
            format_reqwest_error(&e)
        )
    })?;

    serde_json::from_str::<CursorUserMetaResponse>(&body)
        .map_err(|e| format!("解析 Cursor user meta JSON 失败: {}", e))
}

async fn fetch_stripe_profile_with_client(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<Option<CursorStripeProfileResponse>, String> {
    let full_response = client
        .get(CURSOR_FULL_STRIPE_PROFILE_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| {
            format!(
                "请求 Cursor full stripe profile 失败: {}",
                format_reqwest_error(&e)
            )
        })?;

    let full_status = full_response.status().as_u16();
    if full_status == 401 || full_status == 403 {
        return Err("Cursor 会话已过期或未认证，请重新导入账号".to_string());
    }
    if full_status == 200 {
        let body = full_response.text().await.map_err(|e| {
            format!(
                "读取 Cursor full stripe profile 响应失败: {}",
                format_reqwest_error(&e)
            )
        })?;
        let profile = serde_json::from_str::<CursorStripeProfileResponse>(&body)
            .map_err(|e| format!("解析 Cursor full stripe profile JSON 失败: {}", e))?;
        return Ok(Some(profile));
    }

    let fallback_response = client
        .get(CURSOR_STRIPE_PROFILE_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| {
            format!(
                "请求 Cursor stripe profile 失败: {}",
                format_reqwest_error(&e)
            )
        })?;

    let fallback_status = fallback_response.status().as_u16();
    if fallback_status == 401 || fallback_status == 403 {
        return Err("Cursor 会话已过期或未认证，请重新导入账号".to_string());
    }
    if fallback_status != 200 {
        return Ok(None);
    }

    let body = fallback_response.text().await.map_err(|e| {
        format!(
            "读取 Cursor stripe profile 响应失败: {}",
            format_reqwest_error(&e)
        )
    })?;

    let parsed = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| format!("解析 Cursor stripe profile JSON 失败: {}", e))?;

    match parsed {
        Value::Object(_) => serde_json::from_value::<CursorStripeProfileResponse>(parsed)
            .map(Some)
            .map_err(|e| format!("解析 Cursor stripe profile 对象失败: {}", e)),
        Value::String(text) => {
            if text.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(CursorStripeProfileResponse {
                    membership_type: Some("pro".to_string()),
                    individual_membership_type: None,
                    subscription_status: None,
                    team_membership_type: None,
                    is_team_member: None,
                    is_enterprise: None,
                }))
            }
        }
        _ => Ok(None),
    }
}

async fn fetch_usage_summary_with_client(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<serde_json::Value, String> {
    let cookie = build_session_cookie(access_token)
        .ok_or_else(|| "无法从 accessToken 解析 WorkOS 用户 ID".to_string())?;

    let response = client
        .get(CURSOR_USAGE_SUMMARY_URL)
        .header("Accept", "application/json")
        .header("Cookie", &cookie)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        )
        .send()
        .await
        .map_err(|e| format!("请求 Cursor usage API 失败: {}", format_reqwest_error(&e)))?;

    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        return Err("Cursor 会话已过期或未认证，请重新导入账号".to_string());
    }
    if status != 200 {
        return Err(format!("Cursor usage API 返回异常状态码: {}", status));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("读取 Cursor usage 响应失败: {}", format_reqwest_error(&e)))?;

    serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| format!("解析 Cursor usage JSON 失败: {}", e))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorSandUsageResponse {
    usage_percent: Option<f64>,
    included_limit_zero: Option<bool>,
    has_non_zero_included_limit: Option<bool>,
    next_reset_timestamp_utc: Option<String>,
    sand_trial_expires_at: Option<String>,
    sand_trial: Option<Value>,
}

impl CursorSandUsageResponse {
    fn usage(self, now: i64) -> Result<Option<CursorSandUsage>, String> {
        if self.included_limit_zero.is_none()
            && self.has_non_zero_included_limit.is_none()
            && self.sand_trial_expires_at.is_none()
            && self.sand_trial.is_none()
        {
            return Err("Cursor Sand 用量响应缺少额度状态".into());
        }
        let timestamp = |value: Option<&str>| {
            chrono::DateTime::parse_from_rfc3339(value?)
                .ok()
                .map(|date| date.timestamp())
        };
        let next_reset = timestamp(self.next_reset_timestamp_utc.as_deref());
        let included = self
            .included_limit_zero
            .map(|zero| !zero)
            .or(self.has_non_zero_included_limit)
            .unwrap_or(false);
        let trial_expiry = timestamp(self.sand_trial_expires_at.as_deref())
            .or_else(|| self.sand_trial.filter(Value::is_object).and(next_reset));
        let reset_at = if included {
            next_reset
        } else if let Some(expiry) = trial_expiry.filter(|expiry| *expiry > now) {
            Some(expiry)
        } else {
            return Ok(None);
        };
        let percent_used = self
            .usage_percent
            .filter(|value| value.is_finite() && *value >= 0.0)
            .ok_or_else(|| "Cursor Sand 用量响应缺少有效比例".to_string())?;
        Ok(Some(CursorSandUsage {
            percent_used,
            reset_at,
            stale: false,
        }))
    }
}

async fn fetch_sand_usage_with_client(
    client: &reqwest::Client,
    endpoint: &str,
    access_token: &str,
) -> Result<Option<CursorSandUsage>, String> {
    let cookie = build_session_cookie(access_token)
        .ok_or_else(|| "无法从 accessToken 解析 WorkOS 用户 ID".to_string())?;
    let response = client
        .post(endpoint)
        .header("Accept", "application/json")
        .header("Origin", "https://cursor.com")
        .header("Cookie", cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| format!("请求 Cursor Sand 用量失败: {}", format_reqwest_error(&err)))?;
    let status: CursorSandUsageResponse = response
        .json()
        .await
        .map_err(|err| format!("解析 Cursor Sand 用量失败: {}", format_reqwest_error(&err)))?;
    status.usage(now_ts())
}

// ---------------------------------------------------------------------------
// Refresh (updates our own account storage + fetches usage from official APIs)
// ---------------------------------------------------------------------------

async fn refresh_account_async_once(account_id: &str) -> Result<CursorAccount, String> {
    let existing = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[Cursor Refresh] 开始刷新账号: id={}, email={}",
        existing.id, existing.email
    ));

    let client = build_cursor_http_client()?;
    let mut account = existing.clone();

    if access_token_needs_refresh(&account.access_token) {
        match refresh_account_access_token_with_client(&client, &mut account).await {
            Ok(true) => {
                logger::log_info(&format!(
                    "[Cursor Refresh] access token 刷新成功: id={}",
                    account.id
                ));
            }
            Ok(false) => {}
            Err(err) => {
                logger::log_warn(&format!(
                    "[Cursor Refresh] access token 刷新失败，继续使用现有 token: id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    match fetch_user_meta_with_client(&client, &account.access_token).await {
        Ok(meta) => {
            if let Some(email) = normalize_email_identity(meta.email.as_deref()) {
                account.email = email.clone();
                upsert_cursor_auth_raw_string(&mut account, "cachedEmail", Some(email));
            }

            if let Some(sign_up_type) = normalize_cursor_sign_up_type(meta.sign_up_type.as_deref())
            {
                account.sign_up_type = Some(sign_up_type.clone());
                upsert_cursor_auth_raw_string(&mut account, "cachedSignUpType", Some(sign_up_type));
            }

            upsert_cursor_auth_raw_string(&mut account, "workosId", meta.workos_id.clone());
            if account.auth_id.is_none() {
                account.auth_id = normalize_non_empty(meta.workos_id.as_deref());
            }

            logger::log_info(&format!(
                "[Cursor Refresh] 用户信息拉取成功: id={}, email={}",
                account.id, account.email
            ));
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Cursor Refresh] 用户信息拉取失败: id={}, error={}",
                account.id, err
            ));
        }
    }

    match fetch_stripe_profile_with_client(&client, &account.access_token).await {
        Ok(Some(profile)) => {
            if let Some(membership_type) = resolve_membership_from_stripe_profile(&profile) {
                account.membership_type = Some(membership_type.clone());
                upsert_cursor_auth_raw_string(
                    &mut account,
                    "stripeMembershipType",
                    Some(membership_type),
                );
            }

            let subscription_status = normalize_non_empty(profile.subscription_status.as_deref());
            if let Some(status) = subscription_status.clone() {
                account.subscription_status = Some(status);
            }
            upsert_cursor_auth_raw_string(
                &mut account,
                "stripeSubscriptionStatus",
                subscription_status,
            );
            upsert_cursor_auth_raw_string(
                &mut account,
                "teamMembershipType",
                normalize_non_empty(profile.team_membership_type.as_deref()),
            );
            upsert_cursor_auth_raw_bool(&mut account, "isTeamMember", profile.is_team_member);
            upsert_cursor_auth_raw_bool(&mut account, "isEnterprise", profile.is_enterprise);

            logger::log_info(&format!(
                "[Cursor Refresh] 订阅信息拉取成功: id={}",
                account.id
            ));
        }
        Ok(None) => {
            logger::log_warn(&format!(
                "[Cursor Refresh] 未获取到订阅信息: id={}",
                account.id
            ));
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Cursor Refresh] 订阅信息拉取失败: id={}, error={}",
                account.id, err
            ));
        }
    }

    let usage_result = refresh_usage_snapshot(&client, &mut account).await;
    let usage_refreshed = usage_result.is_ok();
    match usage_result {
        Ok(()) => {
            account.quota_query_last_error = None;
            account.quota_query_last_error_at = None;
        }
        Err(err) => {
            logger::log_warn(&format!("[Cursor Refresh] 配额拉取失败: id={}, error={err}", account.id));
            account.quota_query_last_error = Some(err);
            account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
        }
    }

    let refreshed_at = now_ts();
    if usage_refreshed {
        account.usage_updated_at = Some(refreshed_at);
    }
    account.last_used = refreshed_at;
    let updated = tauri::async_runtime::spawn_blocking(move || commit_cursor_refresh(existing, account))
        .await
        .map_err(|error| format!("保存 Cursor 刷新结果失败: {error}"))??;
    logger::log_info(&format!(
        "[Cursor Refresh] 刷新完成: id={}, email={}",
        updated.id, updated.email
    ));
    Ok(updated)
}

async fn refresh_usage_snapshot(
    client: &reqwest::Client,
    account: &mut CursorAccount,
) -> Result<(), String> {
    use super::cursor_usage::{dashboard_read, read_credits};
    use serde_json::json;

    let token = &account.access_token;
    let (plan_result, period_usage) = tokio::join!(
        dashboard_read(client, token, "GetPlanInfo", json!({})),
        dashboard_read(client, token, "GetCurrentPeriodUsage", json!({})),
    );
    let previous = account.cursor_usage_raw.as_ref().unwrap_or(&Value::Null);
    let plan = match &plan_result {
        Ok(plan) => plan,
        Err(err) => {
            logger::log_warn(&format!("[Cursor Refresh] {err}"));
            previous
        }
    };
    let enterprise = account
        .cursor_auth_raw
        .as_ref()
        .and_then(|raw| raw["isEnterprise"].as_bool())
        == Some(true);
    let unified = is_unified_usage(plan, account.membership_type.as_deref());
    let (legacy_summary, sand, credits) = tokio::join!(
        async {
            if unified {
                Ok(None)
            } else {
                fetch_usage_summary_with_client(client, token)
                    .await
                    .map(Some)
            }
        },
        async {
            if unified {
                Ok(None)
            } else {
                fetch_sand_usage_with_client(client, CURSOR_SAND_USAGE_URL, token).await
            }
        },
        async {
            if unified && !enterprise {
                Some(read_credits(client, token).await)
            } else {
                None
            }
        },
    );
    let legacy_summary = match legacy_summary {
        Ok(summary) => summary,
        Err(err) => {
            logger::log_warn(&format!("[Cursor Refresh] 旧版用量详情拉取失败: {err}"));
            None
        }
    };
    let legacy_summary = legacy_summary.filter(Value::is_object);
    let mut usage = match (period_usage, legacy_summary.as_ref()) {
        (Ok(usage), _) => usage,
        (Err(err), Some(summary)) => {
            logger::log_warn(&format!("[Cursor Refresh] 使用旧版用量数据: {err}"));
            summary.clone()
        }
        (Err(err), None) => return Err(err),
    };
    if let Some(summary) = &legacy_summary {
        if let Some(mt) = summary["membershipType"]
            .as_str()
            .filter(|value| !value.is_empty())
        {
            account.membership_type = Some(mt.to_string());
        }
        // Preserve the legacy personal/team spending settings alongside the
        // desktop's included usage. The client response has no enabled flag.
        for key in [
            "individualUsage",
            "individual_usage",
            "teamUsage",
            "team_usage",
            "limitType",
            "limit_type",
            "isUnlimited",
            "is_unlimited",
            "unifiedIncludedUsage",
            "unified_included_usage",
            "includedUsagePeriod",
            "included_usage_period",
            "includedUsageResetsAt",
            "included_usage_resets_at",
        ] {
            if let Some(value) = summary.get(key) {
                usage[key] = value.clone();
            }
        }
    }
    if let Some(info) = plan.get("planInfo").filter(|info| info.is_object()) {
        usage["planInfo"] = info.clone();
    }
    if let Some(merged) = plan["isMerged"].as_bool() {
        usage["isMerged"] = json!(merged && !enterprise);
    }
    if let Some(credits) = credits {
        usage["credits"] = credits;
    }
    match sand {
        Ok(usage) => account.sand_usage = usage,
        Err(err) => {
            if let Some(usage) = &mut account.sand_usage {
                usage.stale = true;
            }
            logger::log_warn(&format!("[Cursor Refresh] Sand 用量拉取失败: {err}"));
        }
    }
    account.cursor_usage_raw = Some(usage);
    Ok(())
}

fn is_unified_usage(raw: &Value, membership: Option<&str>) -> bool {
    let info = raw.get("planInfo").or_else(|| raw.get("plan_info"));
    let period = info
        .and_then(|info| {
            info.get("includedUsagePeriod")
                .or_else(|| info.get("included_usage_period"))
        })
        .or_else(|| {
            raw.get("includedUsagePeriod")
                .or_else(|| raw.get("included_usage_period"))
        });
    if raw["isMerged"].as_bool() == Some(true)
        || matches!(
            period.and_then(Value::as_str),
            Some("INCLUDED_USAGE_PERIOD_WEEKLY" | "weekly" | "2")
        )
        || period.and_then(Value::as_i64) == Some(2)
    {
        return true;
    }
    if matches!(
        period.and_then(Value::as_str),
        Some("INCLUDED_USAGE_PERIOD_MONTHLY" | "monthly" | "1")
    ) || period.and_then(Value::as_i64) == Some(1)
    {
        return false;
    }
    membership.is_some_and(|tier| tier.trim().to_ascii_lowercase().starts_with("unified_"))
        || ["unifiedIncludedUsage", "unified_included_usage"]
            .iter()
            .any(|key| raw[*key].as_bool() == Some(true))
        || ["includedUsageResetsAt", "included_usage_resets_at"]
            .iter()
            .any(|key| {
                info.and_then(|info| info.get(*key))
                    .or_else(|| raw.get(*key))
                    .is_some_and(|value| !value.is_null())
            })
}

fn commit_cursor_refresh(
    previous: CursorAccount,
    mut refreshed: CursorAccount,
) -> Result<CursorAccount, String> {
    let _lock = CURSOR_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Cursor 账号锁失败".to_string())?;
    let current = load_account(&previous.id).ok_or_else(|| "账号不存在".to_string())?;
    // A quota request can finish after deletion, another refresh, or re-login.
    // Keep the account currently in storage if the request used old credentials.
    if current.access_token != previous.access_token
        || current.refresh_token != previous.refresh_token
    {
        return Ok(current);
    }
    refreshed.tags = current.tags;
    refreshed.last_switched_at = current.last_switched_at;
    refreshed.last_used = refreshed.last_used.max(current.last_used);
    let mut index = load_account_index_checked()?;
    save_account_file(&refreshed)?;
    refresh_summary(&mut index, &refreshed);
    save_account_index(&index)?;
    Ok(refreshed)
}

pub async fn refresh_account_async(account_id: &str) -> Result<CursorAccount, String> {
    let result = refresh_account_async_once(account_id).await;
    if let Err(err) = &result {
        persist_quota_query_error(account_id, err);
    }
    result
}

pub async fn refresh_all_tokens() -> Result<Vec<(String, Result<CursorAccount, String>)>, String> {
    let ids = list_accounts()
        .into_iter()
        .filter(|account| !is_banned_account(account))
        .map(|account| account.id)
        .collect();
    Ok(refresh_tokens_for_ids(ids).await)
}

pub async fn refresh_tokens_for_ids(ids: Vec<String>) -> Vec<(String, Result<CursorAccount, String>)> {
    let mut results = Vec::with_capacity(ids.len());
    for id in ids {
        let result = refresh_account_async(&id).await;
        results.push((id, result));
    }
    results
}

// ---------------------------------------------------------------------------
// Quota alert
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct CursorUsagePercent {
    total_used: Option<i32>,
    auto_used: Option<i32>,
    api_used: Option<i32>,
}

fn clamp_percent(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    if value <= 0.0 {
        return 0;
    }
    if value >= 100.0 {
        return 100;
    }
    value.round() as i32
}

fn pick_number(value: Option<&Value>, keys: &[&str]) -> Option<f64> {
    let obj = value?.as_object()?;
    for key in keys {
        let Some(raw) = obj.get(*key) else {
            continue;
        };
        if let Some(n) = raw.as_f64() {
            if n.is_finite() {
                return Some(n);
            }
            continue;
        }
        if let Some(text) = raw.as_str() {
            if let Ok(parsed) = text.trim().parse::<f64>() {
                if parsed.is_finite() {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

fn read_usage_percent(account: &CursorAccount) -> CursorUsagePercent {
    let Some(raw) = account.cursor_usage_raw.as_ref() else {
        return CursorUsagePercent::default();
    };

    let raw_obj = match raw.as_object() {
        Some(value) => value,
        None => return CursorUsagePercent::default(),
    };

    let desktop_plan = raw_obj.get("planUsage").or_else(|| raw_obj.get("plan_usage"));
    if is_unified_usage(raw, account.membership_type.as_deref()) {
        return CursorUsagePercent {
            total_used: pick_number(desktop_plan, &["totalPercentUsed", "total_percent_used"])
                .map(clamp_percent),
            ..CursorUsagePercent::default()
        };
    }
    let plan_value = desktop_plan.or_else(|| {
        raw_obj.get("individualUsage")
            .or_else(|| raw_obj.get("individual_usage"))
            .and_then(|value| value.get("plan"))
    });

    let total_direct = pick_number(plan_value, &["totalPercentUsed", "total_percent_used"]);
    let auto_direct = pick_number(plan_value, &["autoPercentUsed", "auto_percent_used"]);
    let api_direct = pick_number(plan_value, &["apiPercentUsed", "api_percent_used"]);

    let used = pick_number(plan_value, &["includedSpend", "included_spend", "used", "totalSpend", "total_spend"]);
    let limit = pick_number(plan_value, &["limit"]);
    let total_ratio = match (used, limit) {
        (Some(used_val), Some(limit_val)) if limit_val > 0.0 => {
            Some((used_val / limit_val) * 100.0)
        }
        _ => None,
    };

    CursorUsagePercent {
        total_used: total_direct.or(total_ratio).map(clamp_percent),
        auto_used: auto_direct.map(clamp_percent),
        api_used: api_direct.map(clamp_percent),
    }
}

pub(crate) fn extract_quota_metrics(account: &CursorAccount) -> Vec<(String, i32)> {
    let usage = read_usage_percent(account);
    let mut metrics = Vec::new();

    if let Some(used) = usage.total_used {
        metrics.push(("Total Usage".to_string(), 100 - used.clamp(0, 100)));
    }
    if let Some(used) = usage.auto_used {
        metrics.push(("Auto + Composer".to_string(), 100 - used.clamp(0, 100)));
    }
    if let Some(used) = usage.api_used {
        metrics.push(("API Usage".to_string(), 100 - used.clamp(0, 100)));
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

fn normalize_quota_alert_threshold(value: i32) -> i32 {
    value.clamp(0, 100)
}

pub(crate) fn resolve_current_account_id(accounts: &[CursorAccount]) -> Option<String> {
    if let Ok(Some(local_payload)) = read_local_cursor_auth() {
        let incoming_auth_id = resolve_payload_auth_id(&local_payload);
        let incoming_email = normalize_email_identity(Some(local_payload.email.as_str()));
        let incoming_token = normalize_token_identity(Some(local_payload.access_token.as_str()));

        if let Some(account_id) = accounts
            .iter()
            .find(|account| {
                let existing_auth_id = resolve_account_auth_id(account);
                if let (Some(existing), Some(incoming)) =
                    (existing_auth_id.as_ref(), incoming_auth_id.as_ref())
                {
                    return existing == incoming;
                }
                if existing_auth_id.is_some() || incoming_auth_id.is_some() {
                    return false;
                }

                let existing_email = normalize_email_identity(Some(account.email.as_str()));
                let existing_token = normalize_token_identity(Some(account.access_token.as_str()));
                if let (Some(existing), Some(incoming)) =
                    (existing_email.as_ref(), incoming_email.as_ref())
                {
                    if existing == incoming {
                        return true;
                    }
                }
                if let (Some(existing), Some(incoming)) =
                    (existing_token.as_ref(), incoming_token.as_ref())
                {
                    if existing == incoming {
                        return true;
                    }
                }
                false
            })
            .map(|account| account.id.clone())
        {
            return Some(account_id);
        }
    }

    if let Ok(settings) = crate::modules::cursor_instance::load_default_settings() {
        if let Some(bind_id) = settings.bind_account_id {
            let trimmed = bind_id.trim();
            if !trimmed.is_empty() && accounts.iter().any(|account| account.id == trimmed) {
                return Some(trimmed.to_string());
            }
        }
    }

    crate::modules::provider_current_state::resolve_existing_current_account_id(
        "cursor",
        accounts.iter().map(|account| account.id.as_str()),
    )
    .or_else(|| {
        accounts
            .iter()
            .max_by_key(|account| account.last_used)
            .map(|account| account.id.clone())
    })
}

fn pick_quota_alert_recommendation(
    accounts: &[CursorAccount],
    current_id: &str,
) -> Option<CursorAccount> {
    let mut candidates: Vec<CursorAccount> = accounts
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

fn display_email(account: &CursorAccount) -> String {
    let trimmed = account.email.trim();
    if trimmed.is_empty() {
        account.id.clone()
    } else {
        trimmed.to_string()
    }
}

fn build_quota_alert_cooldown_key(account_id: &str, threshold: i32) -> String {
    format!("cursor:{}:{}", account_id, threshold)
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = CURSOR_QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < CURSOR_QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, threshold: i32) {
    if let Ok(mut state) = CURSOR_QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(account_id, threshold));
    }
}

pub fn run_quota_alert_if_needed(
) -> Result<Option<crate::modules::account::QuotaAlertPayload>, String> {
    let cfg = crate::modules::config::get_user_config();
    if !cfg.cursor_quota_alert_enabled {
        return Ok(None);
    }

    let threshold = normalize_quota_alert_threshold(cfg.cursor_quota_alert_threshold);
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
    if metrics.is_empty() {
        clear_quota_alert_cooldown(&current_id, threshold);
        return Ok(None);
    }

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
        platform: "cursor".to_string(),
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

#[cfg(test)]
mod sand_usage_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pasted_browser_session_token_keeps_the_jwt_and_checks_account_identity() {
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"sub":"auth0|user_fixture"}"#);
        let jwt = format!("e30.{claims}.fixture");
        for input in [
            jwt.clone(),
            format!("user_fixture::{jwt}"),
            format!("user_fixture%3a%3A{jwt}"),
            format!("WorkosCursorSessionToken=user_fixture%3A%3A{jwt}"),
        ] {
            let normalized = normalize_pasted_access_token(&input).unwrap();
            assert_eq!(normalized, jwt);
            assert_eq!(
                build_session_cookie(&normalized).unwrap(),
                format!("WorkosCursorSessionToken=user_fixture%3A%3A{jwt}")
            );
        }
        for input in [
            format!("user_other%3A%3A{jwt}"),
            format!("user_fixture%3A%3A{jwt};other=value"),
            "user_fixture%3A%3A".to_owned(),
        ] {
            assert_eq!(
                normalize_pasted_access_token(&input).unwrap_err(),
                "CURSOR_INVALID_BROWSER_TOKEN"
            );
        }
    }

    #[test]
    fn sand_usage_refresh_cannot_overwrite_reauthorized_or_deleted_account() {
        let _environment = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        struct RestoreDataDir(Option<std::ffi::OsString>);
        impl Drop for RestoreDataDir {
            fn drop(&mut self) {
                if let Some(value) = self.0.take() {
                    std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value);
                } else {
                    std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
                }
            }
        }
        let _restore = RestoreDataDir(std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR"));
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", dir.path());

        let previous: CursorAccount = serde_json::from_value(json!({
            "id": "fixture", "email": "cursor@example.test", "access_token": "old",
            "created_at": 1, "last_used": 1
        }))
        .unwrap();
        let mut refreshed = previous.clone();
        refreshed.sand_usage = Some(CursorSandUsage {
            percent_used: 12.0,
            reset_at: None,
            stale: false,
        });
        upsert_account_record(previous.clone()).unwrap();
        commit_cursor_refresh(previous.clone(), refreshed.clone()).unwrap();
        assert_eq!(
            load_account(&previous.id).unwrap().sand_usage.unwrap().percent_used,
            12.0
        );
        let mut current = previous.clone();
        current.access_token = "reauthorized".into();
        upsert_account_record(current).unwrap();

        let preserved = commit_cursor_refresh(previous.clone(), refreshed.clone()).unwrap();
        assert_eq!(preserved.access_token, "reauthorized");
        assert!(load_account(&previous.id).unwrap().sand_usage.is_none());
        remove_account(&previous.id).unwrap();
        assert!(commit_cursor_refresh(previous.clone(), refreshed).is_err());
        assert!(load_account(&previous.id).is_none());
    }

    #[test]
    fn sand_usage_distinguishes_paid_exhausted_trial_and_missing_quota() {
        const RESET: &str = "2026-11-20T12:00:00Z";
        const TRIAL_END: &str = "2026-11-18T12:00:00Z";
        let reset = chrono::DateTime::parse_from_rfc3339(RESET).unwrap().timestamp();
        let trial_end = chrono::DateTime::parse_from_rfc3339(TRIAL_END).unwrap().timestamp();
        let parse = |payload, now| {
            serde_json::from_value::<CursorSandUsageResponse>(payload)
                .unwrap()
                .usage(now)
        };
        for paid in [
            json!({"includedLimitZero": false, "usagePercent": 0, "nextResetTimestampUtc": RESET,
                "sandTrialExpiresAt": TRIAL_END}),
            json!({"hasNonZeroIncludedLimit": true, "usagePercent": 0, "nextResetTimestampUtc": RESET}),
        ] {
            let usage = parse(paid, trial_end).unwrap().unwrap();
            assert_eq!(usage.percent_used, 0.0);
            assert_eq!(usage.reset_at, Some(reset));
        }

        let trial = json!({"includedLimitZero": true, "usagePercent": 100,
            "hasAvailableUsage": false, "sandTrialExpiresAt": TRIAL_END, "nextResetTimestampUtc": RESET});
        let usage = parse(trial.clone(), trial_end - 1).unwrap().unwrap();
        assert_eq!(usage.percent_used, 100.0);
        assert_eq!(usage.reset_at, Some(trial_end));
        assert!(parse(trial, trial_end).unwrap().is_none());
        assert!(parse(json!({"includedLimitZero":true,"usagePercent":0}), trial_end).unwrap().is_none());
        assert!(parse(json!({"includedLimitZero":false}), trial_end).is_err());
        assert!(parse(json!({"error":"missing authentication"}), trial_end).is_err());
    }

    #[tokio::test]
    async fn sand_usage_posts_empty_json_using_cursor_session_cookie() {
        use axum::{
            http::{HeaderMap, StatusCode},
            routing::post,
            Json, Router,
        };
        let jwt = format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(br#"{"sub":"workos|user_fixture"}"#)
        );
        let expected_cookie = format!("WorkosCursorSessionToken=user_fixture%3A%3A{jwt}");
        let app = Router::new()
            .route(
                "/usage",
                post(move |headers: HeaderMap, Json(body): Json<Value>| async move {
                    assert_eq!(headers["cookie"], expected_cookie);
                    assert_eq!(headers["origin"], "https://cursor.com");
                    assert_eq!(body, json!({}));
                    Json(json!({"includedLimitZero":false,"usagePercent":13.55}))
                }),
            )
            .route("/failure", post(|| async { StatusCode::UNAUTHORIZED }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let usage = fetch_sand_usage_with_client(&client, &format!("http://{addr}/usage"), &jwt)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(usage.percent_used, 13.55);
        assert!(!usage.stale);
        assert!(
            fetch_sand_usage_with_client(&client, &format!("http://{addr}/failure"), &jwt)
                .await
                .unwrap_err()
                .contains("401")
        );
        server.abort();
    }
}

#[cfg(test)]
mod session_write_tests {
    use super::*;
    fn account() -> CursorAccount {
        serde_json::from_value(serde_json::json!({
            "id":"new", "email":"new@example.test", "access_token":"header.eyJzdWIiOiJuZXctdXNlciJ9.signature",
            "membership_type":"pro", "created_at":0, "last_used":0
        })).unwrap()
    }
    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE ItemTable(key TEXT PRIMARY KEY, value TEXT);")
            .unwrap();
        for key in [
            "cursorAuth/refreshToken",
            "cursorAuth/cachedTeam",
            "cursorAuth/stripeMembershipAuthId",
            "unrelated",
        ] {
            upsert_vscdb_item(&conn, key, "old").unwrap();
        }
        conn
    }
    #[test]
    fn cursor_switch_clears_old_session_and_owns_new_membership() {
        let mut conn = database();
        write_cursor_auth(&mut conn, &account()).unwrap();
        let count: i64 = conn.query_row("SELECT count(*) FROM ItemTable WHERE key IN ('cursorAuth/refreshToken','cursorAuth/cachedTeam')", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
        let owner: String = conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'cursorAuth/stripeMembershipAuthId'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owner, "new-user");
        assert_eq!(
            conn.query_row(
                "SELECT value FROM ItemTable WHERE key = 'unrelated'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "old"
        );
    }
    #[test]
    fn cursor_switch_rolls_back_when_any_field_fails() {
        let mut conn = database();
        conn.execute_batch("CREATE TRIGGER reject_auth BEFORE INSERT ON ItemTable WHEN NEW.key = 'cursorAuth/accessToken' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(write_cursor_auth(&mut conn, &account()).is_err());
        assert_eq!(
            conn.query_row(
                "SELECT value FROM ItemTable WHERE key = 'cursorAuth/refreshToken'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "old"
        );
    }
}
