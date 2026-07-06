use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::qoderwork_cn::{AuthV2Data, AuthV2User, QoderworkCnAccount, QoderworkCnAccountIndex};
use crate::modules::{account, logger};

const ACCOUNTS_INDEX_FILE: &str = "qoderwork_cn_accounts.json";
const ACCOUNTS_DIR: &str = "qoderwork_cn_accounts";
const SESSIONS_DIR: &str = "qoderwork_cn_sessions";

static QODERWORK_CN_ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

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
        fs::create_dir_all(&dir).map_err(|e| format!("创建 QoderWork CN 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_sessions_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(SESSIONS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 QoderWork CN 会话目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_session_backup_dir(account_id: &str) -> Result<PathBuf, String> {
    let base = get_sessions_dir()?;
    let dir = base.join(account_id);
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("创建 QoderWork CN 会话备份目录失败: {}", e))?;
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

fn resolve_account_file_path(account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    Ok(get_accounts_dir()?.join(format!("{}.json", normalized)))
}

pub fn load_account(account_id: &str) -> Option<QoderworkCnAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_account_file(account: &QoderworkCnAccount) -> Result<(), String> {
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
    // Also clean up session backup directory
    let session_dir = get_sessions_dir()?.join(account_id);
    if session_dir.exists() {
        let _ = fs::remove_dir_all(&session_dir);
    }
    Ok(())
}

fn load_account_index() -> QoderworkCnAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return QoderworkCnAccountIndex::new(),
    };
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(QoderworkCnAccountIndex::new);
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空").unwrap_or_else(QoderworkCnAccountIndex::new)
        }
        Ok(content) => {
            match crate::modules::atomic_write::parse_json_with_auto_restore::<QoderworkCnAccountIndex>(
                &path, &content,
            ) {
                Ok(index) if !index.accounts.is_empty() => index,
                Ok(_) => repair_account_index_from_details("索引账号列表为空")
                    .unwrap_or_else(QoderworkCnAccountIndex::new),
                Err(err) => {
                    logger::log_warn(&format!(
                        "[QoderWorkCN Account] 账号索引解析失败: path={}, error={}",
                        path.display(),
                        err
                    ));
                    repair_account_index_from_details("索引文件损坏")
                        .unwrap_or_else(QoderworkCnAccountIndex::new)
                }
            }
        }
        Err(_) => QoderworkCnAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<QoderworkCnAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(QoderworkCnAccountIndex::new());
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
        return Ok(QoderworkCnAccountIndex::new());
    }
    match crate::modules::atomic_write::parse_json_with_auto_restore::<QoderworkCnAccountIndex>(
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

fn save_account_index(index: &QoderworkCnAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<QoderworkCnAccountIndex> {
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

    let mut index = QoderworkCnAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[QoderWorkCN Account] 自动修复前备份索引失败: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[QoderWorkCN Account] 自动修复索引保存失败: reason={}, error={}",
            reason, err
        ));
    }

    logger::log_warn(&format!(
        "[QoderWorkCN Account] 检测到账号索引异常，已自动重建: reason={}, recovered={}, backup={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut QoderworkCnAccountIndex, account: &QoderworkCnAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: QoderworkCnAccount) -> Result<QoderworkCnAccount, String> {
    let _lock = QODERWORK_CN_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 QoderWork CN 账号锁失败".to_string())?;
    let mut index = load_account_index();
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
}

pub fn upsert_account_from_payload(
    email: String,
    user_id: Option<String>,
    display_name: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_expires_at: Option<i64>,
    quota_raw: Option<Value>,
) -> Result<QoderworkCnAccount, String> {
    let now = now_ts();
    let account_id = if let Some(uid) = &user_id {
        format!("qoderwork_cn_uid_{}", sanitize_account_id_component(uid))
    } else {
        format!(
            "qoderwork_cn_email_{}",
            sanitize_account_id_component(&email.to_lowercase())
        )
    };

    let existing = load_account(&account_id);
    let account = QoderworkCnAccount {
        id: account_id,
        email: email.to_lowercase(),
        user_id,
        display_name,
        user_type: extract_user_type(&quota_raw),
        access_token,
        refresh_token,
        token_expires_at,
        credits_used: extract_quota_number(&quota_raw, &["user_quota", "used"]),
        credits_total: extract_quota_number(&quota_raw, &["user_quota", "total"]),
        credits_remaining: extract_quota_number(&quota_raw, &["user_quota", "remaining"]),
        credits_usage_percent: extract_quota_number(&quota_raw, &["total_usage_percentage"]),
        is_quota_exceeded: extract_quota_bool(&quota_raw, &["is_quota_exceeded"]),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: if quota_raw.is_some() {
            Some(now)
        } else {
            None
        },
        tags: existing.as_ref().and_then(|a| a.tags.clone()),
        quota_raw,
        session_backup_at: existing.as_ref().and_then(|a| a.session_backup_at),
        created_at: existing.as_ref().map(|a| a.created_at).unwrap_or(now),
        last_used: now,
    };

    upsert_account_record(account)
}

pub fn update_quota_query_error(
    account_id: &str,
    message: Option<String>,
) -> Result<Option<QoderworkCnAccount>, String> {
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

fn list_accounts_from_index(index: &QoderworkCnAccountIndex) -> Vec<QoderworkCnAccount> {
    let mut accounts = Vec::new();
    for summary in &index.accounts {
        if let Some(account) = load_account(&summary.id) {
            accounts.push(account);
        }
    }
    accounts.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    accounts
}

pub fn list_accounts() -> Vec<QoderworkCnAccount> {
    let index = load_account_index();
    list_accounts_from_index(&index)
}

pub fn list_accounts_checked() -> Result<Vec<QoderworkCnAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(list_accounts_from_index(&index))
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = QODERWORK_CN_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 QoderWork CN 账号锁失败".to_string())?;
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
    let _lock = QODERWORK_CN_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 QoderWork CN 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| !target.contains(&item.id));
    save_account_index(&index)?;
    for id in target {
        delete_account_file(&id)?;
    }
    Ok(())
}

pub fn update_account_tags(
    account_id: &str,
    tags: Vec<String>,
) -> Result<QoderworkCnAccount, String> {
    let mut account = load_account(account_id)
        .ok_or_else(|| format!("QoderWork CN 账号不存在: {}", account_id))?;
    account.tags = normalize_tags(tags);
    account.last_used = now_ts();
    upsert_account_record(account)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<QoderworkCnAccount>, String> {
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
        _ => return Err("仅支持对象或数组格式的 QoderWork CN JSON".to_string()),
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

fn parse_import_item(item: &Value) -> Result<QoderworkCnAccount, String> {
    // Try direct deserialization first
    if let Ok(account) = serde_json::from_value::<QoderworkCnAccount>(item.clone()) {
        return Ok(normalize_imported_account(account));
    }

    let Some(obj) = item.as_object() else {
        return Err("QoderWork CN 导入数据格式无效".to_string());
    };

    let email = obj
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let user_id = obj
        .get("user_id")
        .or_else(|| obj.get("userId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let access_token = obj
        .get("access_token")
        .or_else(|| obj.get("accessToken"))
        .or_else(|| obj.get("token"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let refresh_token = obj
        .get("refresh_token")
        .or_else(|| obj.get("refreshToken"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let now = now_ts();
    let account_id = if let Some(uid) = &user_id {
        format!("qoderwork_cn_uid_{}", sanitize_account_id_component(uid))
    } else {
        format!(
            "qoderwork_cn_email_{}",
            sanitize_account_id_component(&email.to_lowercase())
        )
    };

    Ok(QoderworkCnAccount {
        id: account_id,
        email: email.to_lowercase(),
        user_id,
        display_name: obj.get("display_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
        user_type: obj.get("user_type").and_then(|v| v.as_str()).map(|s| s.to_string()),
        access_token,
        refresh_token,
        token_expires_at: None,
        credits_used: None,
        credits_total: None,
        credits_remaining: None,
        credits_usage_percent: None,
        is_quota_exceeded: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        tags: None,
        quota_raw: obj.get("quota_raw").cloned(),
        session_backup_at: None,
        created_at: now,
        last_used: now,
    })
}

fn normalize_imported_account(mut account: QoderworkCnAccount) -> QoderworkCnAccount {
    let now = now_ts();
    account.id = sanitize_account_id_component(account.id.trim());
    if account.id.is_empty() {
        account.id = format!(
            "qoderwork_cn_email_{}",
            sanitize_account_id_component(&account.email.to_lowercase())
        );
    }
    if account.created_at == 0 {
        account.created_at = now;
    }
    if account.last_used == 0 {
        account.last_used = now;
    }
    account.email = account.email.to_lowercase();
    account
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts = list_accounts();
    let selected: Vec<QoderworkCnAccount> = if account_ids.is_empty() {
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

pub(crate) fn resolve_current_account_id(accounts: &[QoderworkCnAccount]) -> Option<String> {
    crate::modules::provider_current_state::resolve_existing_current_account_id(
        "qoderwork_cn",
        accounts.iter().map(|account| account.id.as_str()),
    )
}

// ==================== 本地导入 ====================

/// QoderWork CN 状态文件路径
fn get_qoderwork_cn_status_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let status_path = home.join(".qoderworkcn").join(".status.json");
    if status_path.exists() {
        Some(status_path)
    } else {
        None
    }
}

/// QoderWork CN 应用数据目录 (macOS: ~/Library/Application Support/QoderWork CN/)
fn get_qoderwork_cn_app_data_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "macos")]
    {
        let dir = home.join("Library/Application Support/QoderWork CN");
        if dir.exists() {
            return Some(dir);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let dir = PathBuf::from(appdata).join("QoderWork CN");
            if dir.exists() {
                return Some(dir);
            }
        }
    }
    None
}

/// QoderWork CN 配置目录 (~/.qoderworkcn/)
fn get_qoderwork_cn_config_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let dir = home.join(".qoderworkcn");
    if dir.exists() {
        Some(dir)
    } else {
        None
    }
}

#[derive(Debug, serde::Deserialize)]
struct QoderWorkCnStatus {
    #[serde(default)]
    logged_in: Option<bool>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
}

/// 从 avatar_url 中提取用户 UUID
fn extract_user_id_from_avatar_url(avatar_url: &str) -> Option<String> {
    let re = regex::Regex::new(r"users/([0-9a-fA-F\-]{36})").ok()?;
    re.captures(avatar_url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// 从本地 QoderWork CN 导入当前登录账号
pub fn import_from_local() -> Result<Option<QoderworkCnAccount>, String> {
    // 1. 读取 .status.json
    let Some(status_path) = get_qoderwork_cn_status_path() else {
        return Err("未找到 QoderWork CN 状态文件 (~/.qoderworkcn/.status.json)".to_string());
    };

    let status_content = fs::read_to_string(&status_path)
        .map_err(|e| format!("读取 QoderWork CN 状态文件失败: {}", e))?;
    let status: QoderWorkCnStatus = serde_json::from_str(&status_content)
        .map_err(|e| format!("解析 QoderWork CN 状态文件失败: {}", e))?;

    if status.logged_in != Some(true) {
        return Err("QoderWork CN 当前未登录".to_string());
    }

    // 2. 尝试从 auth-v2.dat 提取 access token 和用户信息
    let mut access_token: Option<String> = None;
    let mut refresh_token: Option<String> = None;
    let mut token_expires_at: Option<i64> = None;
    let mut user_id_from_auth: Option<String> = None;
    let mut email_from_auth: Option<String> = None;

    if let Some(app_dir) = get_qoderwork_cn_app_data_dir() {
        let auth_v2_path = app_dir.join("auth-v2.dat");
        if auth_v2_path.exists() {
            match super::vscode_inject::decrypt_qoderwork_cn_safe_storage_file(&auth_v2_path) {
                Ok(decrypted) => {
                    match serde_json::from_str::<AuthV2Data>(&decrypted) {
                        Ok(auth_data) => {
                            access_token = auth_data.token;
                            refresh_token = auth_data.refresh_token;
                            token_expires_at = auth_data.expires_at.as_deref()
                                .and_then(|s| s.parse::<i64>().ok());

                            // 从 auth-v2.dat 中提取用户 ID 和邮箱
                            if let Some(user) = &auth_data.user {
                                user_id_from_auth = user.id.clone();
                                email_from_auth = user.email.clone();
                            }

                            logger::log_info("[QoderWorkCN Account] 从 auth-v2.dat 成功提取 token 和用户信息");
                        }
                        Err(err) => {
                            logger::log_warn(&format!(
                                "[QoderWorkCN Account] 解析 auth-v2.dat JSON 失败: {}",
                                err
                            ));
                        }
                    }
                }
                Err(err) => {
                    logger::log_warn(&format!(
                        "[QoderWorkCN Account] 解密 auth-v2.dat 失败: {}",
                        err
                    ));
                }
            }
        }
    }

    // 3. 确定用户 ID：优先使用 auth-v2.dat 中的 user.id，其次从 .status.json 的 avatar_url 提取
    let user_id = user_id_from_auth.or_else(|| {
        status.avatar_url.as_ref().and_then(|url| extract_user_id_from_avatar_url(url))
    });

    // 4. 确定邮箱：优先使用 auth-v2.dat 中的 user.email，其次使用 .status.json 中的 username
    let email = email_from_auth.clone()
        .or(status.username.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // 5. 构建账号 ID
    let account_id = if let Some(uid) = &user_id {
        format!("qoderwork_cn_uid_{}", sanitize_account_id_component(uid))
    } else {
        format!(
            "qoderwork_cn_email_{}",
            sanitize_account_id_component(&email.to_lowercase())
        )
    };

    // 6. 备份当前会话文件
    let now = now_ts();
    if let Err(err) = backup_current_session_to(&account_id) {
        logger::log_warn(&format!(
            "[QoderWorkCN Account] 本地导入时备份会话失败: {}",
            err
        ));
    }

    // 7. 创建账号记录
    let account = QoderworkCnAccount {
        id: account_id,
        email: email.to_lowercase(),
        user_id,
        display_name: status.username.or(email_from_auth.clone()),
        user_type: None,
        access_token,
        refresh_token,
        token_expires_at,
        credits_used: None,
        credits_total: None,
        credits_remaining: None,
        credits_usage_percent: None,
        is_quota_exceeded: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        tags: None,
        quota_raw: None,
        session_backup_at: Some(now),
        created_at: now,
        last_used: now,
    };

    let saved = upsert_account_record(account)?;
    logger::log_info(&format!(
        "[QoderWorkCN Account] 从本地导入成功: id={}, email={}",
        saved.id, saved.email
    ));
    Ok(Some(saved))
}

// ==================== OAuth 会话文件写入 ====================

/// OAuth 登录后，主动写入 QoderWork CN 应用能识别的认证文件。
/// 这样备份时就能捕获到完整的会话数据，切换时能恢复登录状态。
pub fn write_oauth_session_files(
    access_token: &str,
    refresh_token: Option<&str>,
    token_expires_at: Option<i64>,
    user_id: Option<&str>,
    email: &str,
    display_name: Option<&str>,
    avatar_url: Option<&str>,
    userinfo: Option<&serde_json::Value>,
) -> Result<(), String> {
    logger::log_info("[QoderWorkCN Account] 开始写入 OAuth 会话文件");

    // 1. 写入 auth-v2.dat (加密的 JSON)
    if let Some(app_dir) = get_qoderwork_cn_app_data_dir() {
        // Convert token_expires_at (millis) to ISO 8601 string for QoderWork CN
        let expires_at_iso = token_expires_at.map(|ts_ms| {
            chrono::DateTime::from_timestamp_millis(ts_ms)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| ts_ms.to_string())
        });

        // Read machine-id for loginDeviceId
        let machine_id = get_qoderwork_cn_config_dir()
            .map(|d| d.join("machine-id"))
            .and_then(|p| fs::read_to_string(&p).ok())
            .map(|s| s.trim().to_string());

        let auth_v2 = AuthV2Data {
            token: Some(access_token.to_string()),
            refresh_token: refresh_token.map(|s| s.to_string()),
            expires_at: expires_at_iso,
            user: user_id.map(|uid| AuthV2User {
                id: Some(uid.to_string()),
                email: Some(email.to_string()),
                name: display_name.map(|s| s.to_string()),
            }),
            schema_version: Some(2),
            login_method: Some("browser".to_string()),
            refresh_strategy: Some("device_token".to_string()),
            refresh_token_expires_at: None,
            login_device_id: machine_id,
            login_timestamp: Some(chrono::Utc::now().timestamp_millis()),
        };
        let json_bytes = serde_json::to_vec(&auth_v2)
            .map_err(|e| format!("序列化 auth-v2 数据失败: {}", e))?;
        match super::vscode_inject::encrypt_qoderwork_cn_safe_storage_data(&json_bytes) {
            Ok(encrypted) => {
                let auth_v2_path = app_dir.join("auth-v2.dat");
                fs::write(&auth_v2_path, &encrypted).map_err(|e| {
                    format!("写入 auth-v2.dat 失败: {}", e)
                })?;
                logger::log_info(&format!(
                    "[QoderWorkCN Account] 写入 auth-v2.dat 完成: {} bytes, json_len={}",
                    encrypted.len(), json_bytes.len()
                ));
                // Log the JSON being written (redacted token for security)
                let json_str = String::from_utf8_lossy(&json_bytes);
                if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    if let Some(tok) = val.get("token").and_then(|v| v.as_str()) {
                        let redacted = format!("{}...({}chars)", &tok[..tok.len().min(8)], tok.len());
                        val["token"] = serde_json::json!(redacted);
                    }
                    if let Some(tok) = val.get("refreshToken").and_then(|v| v.as_str()) {
                        let redacted = format!("{}...({}chars)", &tok[..tok.len().min(8)], tok.len());
                        val["refreshToken"] = serde_json::json!(redacted);
                    }
                    logger::log_info(&format!(
                        "[QoderWorkCN Account] auth-v2.dat JSON: {}",
                        serde_json::to_string_pretty(&val).unwrap_or_default()
                    ));
                }
                // Verify: try to decrypt the written file
                match super::vscode_inject::decrypt_qoderwork_cn_safe_storage_file(&auth_v2_path) {
                    Ok(decrypted) => {
                        logger::log_info(&format!(
                            "[QoderWorkCN Account] auth-v2.dat 验证解密成功: {} bytes",
                            decrypted.len()
                        ));
                    }
                    Err(err) => {
                        logger::log_warn(&format!(
                            "[QoderWorkCN Account] auth-v2.dat 验证解密失败: {}",
                            err
                        ));
                    }
                }
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[QoderWorkCN Account] 加密 auth-v2.dat 失败: {} (跳过)",
                    err
                ));
            }
        }
    } else {
        logger::log_warn("[QoderWorkCN Account] 应用数据目录不存在，跳过 auth-v2.dat");
    }

    // 2. 写入 .auth-cn/id 和 .auth-cn/user
    if let Some(cfg_dir) = get_qoderwork_cn_config_dir() {
        let auth_cn_dir = cfg_dir.join(".auth-cn");
        let _ = fs::create_dir_all(&auth_cn_dir);

        if let Some(uid) = user_id {
            let id_path = auth_cn_dir.join("id");
            fs::write(&id_path, uid).map_err(|e| {
                format!("写入 .auth-cn/id 失败: {}", e)
            })?;
            logger::log_info(&format!("[QoderWorkCN Account] 写入 .auth-cn/id: {}", uid));
        }

        // 写入 user 文件 (JSON 格式)
        if let Some(info) = userinfo {
            let user_path = auth_cn_dir.join("user");
            let user_json = serde_json::to_string(info)
                .map_err(|e| format!("序列化 user 数据失败: {}", e))?;
            fs::write(&user_path, &user_json).map_err(|e| {
                format!("写入 .auth-cn/user 失败: {}", e)
            })?;
            logger::log_info(&format!(
                "[QoderWorkCN Account] 写入 .auth-cn/user: {} bytes",
                user_json.len()
            ));
        }
    } else {
        logger::log_warn("[QoderWorkCN Account] 配置目录不存在，跳过 .auth-cn");
    }

    // 3. 更新 .status.json，设置 logged_in: true
    if let Some(cfg_dir) = get_qoderwork_cn_config_dir() {
        let status_path = cfg_dir.join(".status.json");
        // 读取现有的 .status.json（如果存在），否则创建新的
        let mut status: serde_json::Value = if status_path.exists() {
            fs::read_to_string(&status_path)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_else(|| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        status["logged_in"] = serde_json::json!(true);
        if let Some(name) = display_name {
            status["username"] = serde_json::json!(name);
            status["name"] = serde_json::json!(name);
        }
        status["email"] = serde_json::json!(email);
        if let Some(avatar) = avatar_url {
            status["avatar_url"] = serde_json::json!(avatar);
        }

        let status_json = serde_json::to_string_pretty(&status)
            .map_err(|e| format!("序列化 .status.json 失败: {}", e))?;
        fs::write(&status_path, &status_json).map_err(|e| {
            format!("写入 .status.json 失败: {}", e)
        })?;
        logger::log_info(&format!(
            "[QoderWorkCN Account] 更新 .status.json: logged_in=true"
        ));
    }

    logger::log_info("[QoderWorkCN Account] OAuth 会话文件写入完成");
    Ok(())
}

// ==================== 会话切换 ====================

/// 需要备份/恢复的认证文件列表
const AUTH_FILES: &[&str] = &["auth.dat", "auth-v2.dat", "lockfile"];

/// 备份当前活跃会话到指定账号的备份目录
pub fn backup_current_session_to(account_id: &str) -> Result<(), String> {
    let backup_dir = get_session_backup_dir(account_id)?;
    let app_data_dir = get_qoderwork_cn_app_data_dir();
    let config_dir = get_qoderwork_cn_config_dir();

    logger::log_info(&format!(
        "[QoderWorkCN Account] 开始备份会话: account_id={}, backup_dir={}",
        account_id,
        backup_dir.display()
    ));

    // 列出源目录中的文件用于调试
    if let Some(app_dir) = &app_data_dir {
        if let Ok(entries) = fs::read_dir(app_dir) {
            let files: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            logger::log_info(&format!(
                "[QoderWorkCN Account] 应用数据目录内容: {:?}",
                files
            ));
        }
    }

    if let Some(cfg_dir) = &config_dir {
        if let Ok(entries) = fs::read_dir(cfg_dir) {
            let files: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            logger::log_info(&format!(
                "[QoderWorkCN Account] 配置目录内容: {:?}",
                files
            ));
        }
    }

    let mut backed_up_files = Vec::new();

    // 备份应用数据目录中的认证文件
    if let Some(app_dir) = &app_data_dir {
        for file_name in AUTH_FILES {
            let src = app_dir.join(file_name);
            if src.exists() {
                let dst = backup_dir.join(file_name);
                fs::copy(&src, &dst).map_err(|e| {
                    format!("备份 {} 失败: {}", file_name, e)
                })?;
                backed_up_files.push(format!("app_data/{}", file_name));
            } else {
                logger::log_warn(&format!(
                    "[QoderWorkCN Account] 源文件中不存在: app_data/{}",
                    file_name
                ));
            }
        }
    }

    // 备份 .auth-cn/ 目录中的文件
    if let Some(cfg_dir) = &config_dir {
        let auth_cn_dir = cfg_dir.join(".auth-cn");
        if auth_cn_dir.exists() {
            let dst_auth_cn = backup_dir.join("dot_auth-cn");
            let _ = fs::create_dir_all(&dst_auth_cn);
            for entry in ["id", "user"] {
                let src = auth_cn_dir.join(entry);
                if src.exists() {
                    let dst = dst_auth_cn.join(entry);
                    fs::copy(&src, &dst).map_err(|e| {
                        format!("备份 .auth-cn/{} 失败: {}", entry, e)
                    })?;
                    backed_up_files.push(format!(".auth-cn/{}", entry));
                } else {
                    logger::log_warn(&format!(
                        "[QoderWorkCN Account] 源文件中不存在: .auth-cn/{}",
                        entry
                    ));
                }
            }
        } else {
            logger::log_warn(&format!(
                "[QoderWorkCN Account] 源目录中不存在: .auth-cn"
            ));
        }

        // 备份 .status.json
        let status_src = cfg_dir.join(".status.json");
        if status_src.exists() {
            let status_dst = backup_dir.join("dot_status.json");
            fs::copy(&status_src, &status_dst).map_err(|e| {
                format!("备份 .status.json 失败: {}", e)
            })?;
            backed_up_files.push(".status.json".to_string());

            // 读取并记录 .status.json 的内容用于调试
            if let Ok(content) = fs::read_to_string(&status_src) {
                logger::log_info(&format!(
                    "[QoderWorkCN Account] 备份的 .status.json 内容: {}",
                    content
                ));
            }
        } else {
            logger::log_warn(&format!(
                "[QoderWorkCN Account] 源文件中不存在: .status.json"
            ));
        }
    }

    logger::log_info(&format!(
        "[QoderWorkCN Account] 会话备份完成: account_id={}, backup_dir={}, backed_up_files={:?}",
        account_id,
        backup_dir.display(),
        backed_up_files
    ));
    Ok(())
}

/// 检测当前活跃账号 ID
pub fn detect_current_active_account_id() -> Option<String> {
    let config_dir = get_qoderwork_cn_config_dir()?;
    let status_path = config_dir.join(".status.json");
    if !status_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&status_path).ok()?;
    let status: QoderWorkCnStatus = serde_json::from_str(&content).ok()?;
    if status.logged_in != Some(true) {
        return None;
    }
    let avatar_url = status.avatar_url?;
    let user_id = extract_user_id_from_avatar_url(&avatar_url)?;

    // Find matching account by user_id
    let accounts = list_accounts();
    accounts
        .iter()
        .find(|a| a.user_id.as_deref() == Some(&user_id))
        .map(|a| a.id.clone())
}

/// 杀死 QoderWork CN 相关进程
fn kill_qoderwork_cn_processes() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Use pkill to kill QoderWork CN processes
        let process_patterns = [
            "QoderWork CN",
            "qoderclicn",
            "qodercli",
        ];
        for pattern in &process_patterns {
            let _ = std::process::Command::new("pkill")
                .args(["-f", pattern])
                .output();
        }
        // Wait for processes to exit
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    #[cfg(target_os = "windows")]
    {
        let process_names = [
            "QoderWork CN.exe",
            "qoderclicn.exe",
            "qodercli.exe",
        ];
        for name in &process_names {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/IM", name])
                .output();
        }
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    #[cfg(target_os = "linux")]
    {
        let process_patterns = ["qoderwork", "qoderclicn", "qodercli"];
        for pattern in &process_patterns {
            let _ = std::process::Command::new("pkill")
                .args(["-f", pattern])
                .output();
        }
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    Ok(())
}

/// 清除当前活跃的会话文件
fn clear_active_session() -> Result<(), String> {
    if let Some(app_dir) = get_qoderwork_cn_app_data_dir() {
        for file_name in AUTH_FILES {
            let path = app_dir.join(file_name);
            if path.exists() {
                fs::remove_file(&path).map_err(|e| {
                    format!("清除 {} 失败: {}", file_name, e)
                })?;
            }
        }
    }

    if let Some(cfg_dir) = get_qoderwork_cn_config_dir() {
        // Clear .auth-cn/ files
        let auth_cn_dir = cfg_dir.join(".auth-cn");
        if auth_cn_dir.exists() {
            for entry in ["id", "user"] {
                let path = auth_cn_dir.join(entry);
                if path.exists() {
                    let _ = fs::remove_file(&path);
                }
            }
        }

        // Remove .status.json
        let status_path = cfg_dir.join(".status.json");
        if status_path.exists() {
            let _ = fs::remove_file(&status_path);
        }
    }

    Ok(())
}

/// 从备份目录恢复会话文件
fn restore_session(account_id: &str) -> Result<(), String> {
    let backup_dir = get_session_backup_dir(account_id)?;
    if !backup_dir.exists() {
        return Err(format!(
            "账号 {} 没有备份的会话数据",
            account_id
        ));
    }

    logger::log_info(&format!(
        "[QoderWorkCN Account] 开始恢复会话: account_id={}, backup_dir={}",
        account_id,
        backup_dir.display()
    ));

    // 列出备份目录中的所有文件
    if let Ok(entries) = fs::read_dir(&backup_dir) {
        let files: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        logger::log_info(&format!(
            "[QoderWorkCN Account] 备份目录内容: {:?}",
            files
        ));
    }

    // 恢复应用数据目录中的认证文件
    let mut restored_files = Vec::new();
    if let Some(app_dir) = get_qoderwork_cn_app_data_dir() {
        fs::create_dir_all(&app_dir)
            .map_err(|e| format!("创建应用数据目录失败: {}", e))?;
        for file_name in AUTH_FILES {
            let src = backup_dir.join(file_name);
            if src.exists() {
                let dst = app_dir.join(file_name);
                fs::copy(&src, &dst).map_err(|e| {
                    format!("恢复 {} 失败: {}", file_name, e)
                })?;
                restored_files.push(format!("app_data/{}", file_name));
            } else {
                logger::log_warn(&format!(
                    "[QoderWorkCN Account] 备份文件中不存在: {}",
                    file_name
                ));
            }
        }
    }

    // 恢复 .auth-cn/ 文件
    if let Some(cfg_dir) = get_qoderwork_cn_config_dir() {
        let auth_cn_dir = cfg_dir.join(".auth-cn");
        let _ = fs::create_dir_all(&auth_cn_dir);
        let src_auth_cn = backup_dir.join("dot_auth-cn");
        if src_auth_cn.exists() {
            for entry in ["id", "user"] {
                let src = src_auth_cn.join(entry);
                if src.exists() {
                    let dst = auth_cn_dir.join(entry);
                    fs::copy(&src, &dst).map_err(|e| {
                        format!("恢复 .auth-cn/{} 失败: {}", entry, e)
                    })?;
                    restored_files.push(format!(".auth-cn/{}", entry));
                } else {
                    logger::log_warn(&format!(
                        "[QoderWorkCN Account] 备份文件中不存在: .auth-cn/{}",
                        entry
                    ));
                }
            }
        } else {
            logger::log_warn(&format!(
                "[QoderWorkCN Account] 备份目录中不存在: dot_auth-cn"
            ));
        }

        // 恢复 .status.json
        let status_src = backup_dir.join("dot_status.json");
        if status_src.exists() {
            let status_dst = cfg_dir.join(".status.json");
            fs::copy(&status_src, &status_dst).map_err(|e| {
                format!("恢复 .status.json 失败: {}", e)
            })?;
            restored_files.push(".status.json".to_string());

            // 读取并记录 .status.json 的内容用于调试
            if let Ok(content) = fs::read_to_string(&status_dst) {
                logger::log_info(&format!(
                    "[QoderWorkCN Account] 恢复的 .status.json 内容: {}",
                    content
                ));
            }
        } else {
            logger::log_warn(&format!(
                "[QoderWorkCN Account] 备份文件中不存在: dot_status.json"
            ));
        }
    }

    logger::log_info(&format!(
        "[QoderWorkCN Account] 会话恢复完成: account_id={}, restored_files={:?}",
        account_id, restored_files
    ));
    Ok(())
}

/// 重启 QoderWork CN 应用
fn restart_qoderwork_cn_app() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("open")
            .args(["-a", "QoderWork CN"])
            .output()
            .map_err(|e| format!("启动 QoderWork CN 失败: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("启动 QoderWork CN 失败: {}", stderr));
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Try common installation paths
        let common_paths = [
            PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
                .join("QoderWork CN")
                .join("QoderWork CN.exe"),
            PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
                .join("Programs")
                .join("QoderWork CN")
                .join("QoderWork CN.exe"),
        ];
        let mut launched = false;
        for path in &common_paths {
            if path.exists() {
                std::process::Command::new(path)
                    .spawn()
                    .map_err(|e| format!("启动 QoderWork CN 失败: {}", e))?;
                launched = true;
                break;
            }
        }
        if !launched {
            return Err("未找到 QoderWork CN 安装路径".to_string());
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("qoderwork-cn")
            .spawn()
            .map_err(|e| format!("启动 QoderWork CN 失败: {}", e))?;
    }

    // Wait for app to start
    std::thread::sleep(std::time::Duration::from_secs(2));
    Ok(())
}

/// 切换账号：备份当前会话 → 杀进程 → 清除会话 → 恢复目标会话 → 重启应用
pub fn switch_account(account_id: &str) -> Result<String, String> {
    let account = load_account(account_id)
        .ok_or_else(|| format!("QoderWork CN 账号不存在: {}", account_id))?;

    // 1. 检测并备份当前活跃账号的会话
    if let Some(current_id) = detect_current_active_account_id() {
        if current_id != account_id {
            logger::log_info(&format!(
                "[QoderWorkCN Switch] 备份当前账号会话: {}",
                current_id
            ));
            if let Err(err) = backup_current_session_to(&current_id) {
                logger::log_warn(&format!(
                    "[QoderWorkCN Switch] 备份当前会话失败（继续切换）: {}",
                    err
                ));
            }
            // Update session_backup_at for the old account
            if let Some(mut old_account) = load_account(&current_id) {
                old_account.session_backup_at = Some(now_ts());
                let _ = upsert_account_record(old_account);
            }
        }
    }

    // 2. 杀死 QoderWork CN 进程
    logger::log_info("[QoderWorkCN Switch] 关闭 QoderWork CN 进程");
    kill_qoderwork_cn_processes()?;

    // 3. 清除当前会话
    logger::log_info("[QoderWorkCN Switch] 清除当前会话文件");
    clear_active_session()?;

    // 4. 恢复目标账号的会话
    logger::log_info(&format!(
        "[QoderWorkCN Switch] 恢复目标账号会话: {}",
        account_id
    ));
    restore_session(account_id)?;

    // 5. 更新当前账号记录
    crate::modules::provider_current_state::set_current_account_id(
        "qoderwork_cn",
        Some(account_id),
    )?;

    // 6. 重启应用
    logger::log_info("[QoderWorkCN Switch] 重启 QoderWork CN");
    if let Err(err) = restart_qoderwork_cn_app() {
        logger::log_warn(&format!("[QoderWorkCN Switch] 重启失败: {}", err));
        return Ok(format!("切换完成，但 QoderWork CN 启动失败: {}", err));
    }

    // Update account's last_used
    if let Some(mut updated) = load_account(account_id) {
        updated.last_used = now_ts();
        let _ = upsert_account_record(updated);
    }

    Ok(format!("切换完成: {}", account.email))
}

// ==================== 辅助函数 ====================

fn normalize_tags(tags: Vec<String>) -> Option<Vec<String>> {
    let cleaned: Vec<String> = tags
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn extract_user_type(quota_raw: &Option<Value>) -> Option<String> {
    quota_raw.as_ref().and_then(|raw| {
        raw.get("user_type")
            .or_else(|| raw.get("userType"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })
}

fn extract_quota_number(quota_raw: &Option<Value>, path: &[&str]) -> Option<f64> {
    let raw = quota_raw.as_ref()?;
    let mut current = raw;
    for (i, segment) in path.iter().enumerate() {
        if i == path.len() - 1 {
            return current.get(*segment).and_then(|v| v.as_f64());
        }
        current = current.get(*segment)?;
    }
    None
}

fn extract_quota_bool(quota_raw: &Option<Value>, path: &[&str]) -> Option<bool> {
    let raw = quota_raw.as_ref()?;
    let mut current = raw;
    for (i, segment) in path.iter().enumerate() {
        if i == path.len() - 1 {
            return current.get(*segment).and_then(|v| v.as_bool());
        }
        current = current.get(*segment)?;
    }
    None
}
