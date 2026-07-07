use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::qoder_cn::{AuthV2Data, AuthV2User, QoderCnAccount, QoderCnAccountIndex};
use crate::modules::{account, logger};

const ACCOUNTS_INDEX_FILE: &str = "qoder_cn_accounts.json";
const ACCOUNTS_DIR: &str = "qoder_cn_accounts";
const SESSIONS_DIR: &str = "qoder_cn_sessions";

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
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Qoder CN 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_sessions_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(SESSIONS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Qoder CN 会话目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_session_backup_dir(account_id: &str) -> Result<PathBuf, String> {
    let base = get_sessions_dir()?;
    let dir = base.join(account_id);
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("创建 Qoder CN 会话备份目录失败: {}", e))?;
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

pub fn load_account(account_id: &str) -> Option<QoderCnAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_account_file(account: &QoderCnAccount) -> Result<(), String> {
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

fn load_account_index() -> QoderCnAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return QoderCnAccountIndex::new(),
    };
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(QoderCnAccountIndex::new);
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空").unwrap_or_else(QoderCnAccountIndex::new)
        }
        Ok(content) => {
            match crate::modules::atomic_write::parse_json_with_auto_restore::<QoderCnAccountIndex>(
                &path, &content,
            ) {
                Ok(index) if !index.accounts.is_empty() => index,
                Ok(_) => repair_account_index_from_details("索引账号列表为空")
                    .unwrap_or_else(QoderCnAccountIndex::new),
                Err(err) => {
                    logger::log_warn(&format!(
                        "[QoderCN Account] 账号索引解析失败: path={}, error={}",
                        path.display(),
                        err
                    ));
                    repair_account_index_from_details("索引文件损坏")
                        .unwrap_or_else(QoderCnAccountIndex::new)
                }
            }
        }
        Err(_) => QoderCnAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<QoderCnAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(QoderCnAccountIndex::new());
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
        return Ok(QoderCnAccountIndex::new());
    }
    match crate::modules::atomic_write::parse_json_with_auto_restore::<QoderCnAccountIndex>(
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

fn save_account_index(index: &QoderCnAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<QoderCnAccountIndex> {
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

    let mut index = QoderCnAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[QoderCN Account] 自动修复前备份索引失败: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[QoderCN Account] 自动修复索引保存失败: reason={}, error={}",
            reason, err
        ));
    }

    logger::log_warn(&format!(
        "[QoderCN Account] 检测到账号索引异常，已自动重建: reason={}, recovered={}, backup={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut QoderCnAccountIndex, account: &QoderCnAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: QoderCnAccount) -> Result<QoderCnAccount, String> {
    let _lock = QODERWORK_CN_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Qoder CN 账号锁失败".to_string())?;
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
) -> Result<QoderCnAccount, String> {
    let now = now_ts();
    let account_id = if let Some(uid) = &user_id {
        format!("qoder_cn_uid_{}", sanitize_account_id_component(uid))
    } else {
        format!(
            "qoder_cn_email_{}",
            sanitize_account_id_component(&email.to_lowercase())
        )
    };

    let existing = load_account(&account_id);
    let account = QoderCnAccount {
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
        auth_user_info_raw: existing.as_ref().and_then(|a| a.auth_user_info_raw.clone()),
        auth_user_plan_raw: existing.as_ref().and_then(|a| a.auth_user_plan_raw.clone()),
        session_backup_at: existing.as_ref().and_then(|a| a.session_backup_at),
        created_at: existing.as_ref().map(|a| a.created_at).unwrap_or(now),
        last_used: now,
    };

    upsert_account_record(account)
}

pub fn update_quota_query_error(
    account_id: &str,
    message: Option<String>,
) -> Result<Option<QoderCnAccount>, String> {
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

fn list_accounts_from_index(index: &QoderCnAccountIndex) -> Vec<QoderCnAccount> {
    let mut accounts = Vec::new();
    for summary in &index.accounts {
        if let Some(account) = load_account(&summary.id) {
            accounts.push(account);
        }
    }
    accounts.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    accounts
}

pub fn list_accounts() -> Vec<QoderCnAccount> {
    let index = load_account_index();
    list_accounts_from_index(&index)
}

pub fn list_accounts_checked() -> Result<Vec<QoderCnAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(list_accounts_from_index(&index))
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = QODERWORK_CN_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Qoder CN 账号锁失败".to_string())?;
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
        .map_err(|_| "获取 Qoder CN 账号锁失败".to_string())?;
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
) -> Result<QoderCnAccount, String> {
    let mut account = load_account(account_id)
        .ok_or_else(|| format!("Qoder CN 账号不存在: {}", account_id))?;
    account.tags = normalize_tags(tags);
    account.last_used = now_ts();
    upsert_account_record(account)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<QoderCnAccount>, String> {
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
        _ => return Err("仅支持对象或数组格式的 Qoder CN JSON".to_string()),
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

fn parse_import_item(item: &Value) -> Result<QoderCnAccount, String> {
    // Try direct deserialization first
    if let Ok(account) = serde_json::from_value::<QoderCnAccount>(item.clone()) {
        return Ok(normalize_imported_account(account));
    }

    let Some(obj) = item.as_object() else {
        return Err("Qoder CN 导入数据格式无效".to_string());
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
        format!("qoder_cn_uid_{}", sanitize_account_id_component(uid))
    } else {
        format!(
            "qoder_cn_email_{}",
            sanitize_account_id_component(&email.to_lowercase())
        )
    };

    Ok(QoderCnAccount {
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
        auth_user_info_raw: obj.get("auth_user_info_raw").cloned(),
        auth_user_plan_raw: obj.get("auth_user_plan_raw").cloned(),
        session_backup_at: None,
        created_at: now,
        last_used: now,
    })
}

fn normalize_imported_account(mut account: QoderCnAccount) -> QoderCnAccount {
    let now = now_ts();
    account.id = sanitize_account_id_component(account.id.trim());
    if account.id.is_empty() {
        account.id = format!(
            "qoder_cn_email_{}",
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
    let selected: Vec<QoderCnAccount> = if account_ids.is_empty() {
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

pub(crate) fn resolve_current_account_id(accounts: &[QoderCnAccount]) -> Option<String> {
    crate::modules::provider_current_state::resolve_existing_current_account_id(
        "qoder_cn",
        accounts.iter().map(|account| account.id.as_str()),
    )
}

// ==================== 本地导入 ====================

const QODER_CN_SECRET_USER_INFO_KEY: &str = "secret://aicoding.auth.userInfo";
const QODER_CN_SECRET_USER_PLAN_KEY: &str = "secret://aicoding.auth.userPlan";

/// Qoder CN 应用数据目录 (macOS: ~/Library/Application Support/QoderCN/)
fn get_qoder_cn_app_data_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "macos")]
    {
        let dir = home.join("Library/Application Support/QoderCN");
        if dir.exists() {
            return Some(dir);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let dir = PathBuf::from(appdata).join("QoderCN");
            if dir.exists() {
                return Some(dir);
            }
        }
    }
    None
}

/// Qoder CN 配置目录 (~/.qoder-cn/)
fn get_qoder_cn_config_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let dir = home.join(".qoder-cn");
    if dir.exists() {
        Some(dir)
    } else {
        None
    }
}

/// 获取 Qoder CN 的 machine-id（优先从配置目录读取，其次从应用数据目录读取）
fn get_qoder_cn_machine_id() -> Option<String> {
    // 1. 尝试从 ~/.qoder-cn/machine-id 读取
    if let Some(cfg_dir) = get_qoder_cn_config_dir() {
        let p = cfg_dir.join("machine-id");
        if let Ok(s) = fs::read_to_string(&p) {
            let trimmed = s.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    // 2. 尝试从 ~/Library/Application Support/QoderCN/machineid 读取
    if let Some(app_dir) = get_qoder_cn_app_data_dir() {
        let p = app_dir.join("machineid");
        if let Ok(s) = fs::read_to_string(&p) {
            let trimmed = s.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

/// 获取 Qoder CN 的 state.vscdb 路径
fn get_qoder_cn_state_db_path() -> Option<PathBuf> {
    let app_dir = get_qoder_cn_app_data_dir()?;
    let db_path = app_dir.join("User").join("globalStorage").join("state.vscdb");
    if db_path.exists() {
        Some(db_path)
    } else {
        None
    }
}

/// 从 state.vscdb 读取加密的 secret 值并解密
fn read_qoder_cn_secret_json(db_path: &std::path::Path, db_key: &str) -> Result<Option<serde_json::Value>, String> {
    let raw = super::vscode_inject::read_qoder_cn_secret_storage_value_by_db_path(db_path, db_key)?;
    Ok(raw.and_then(|text| serde_json::from_str(&text).ok()))
}

/// 从 userInfo JSON 中提取字段
fn extract_str(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// 从本地 Qoder CN 导入当前登录账号
///
/// Qoder CN 使用 VS Code 风格的 state.vscdb 存储认证数据：
/// - `secret://aicoding.auth.userInfo` — 用户信息 + token
/// - `secret://aicoding.auth.userPlan` — 计划/配额信息
pub fn import_from_local() -> Result<Option<QoderCnAccount>, String> {
    // 1. 找到 state.vscdb
    let Some(db_path) = get_qoder_cn_state_db_path() else {
        return Err("未找到 Qoder CN 数据库 (state.vscdb)，请确认已安装并登录过 Qoder CN".to_string());
    };

    // 2. 读取并解密 userInfo
    let user_info = read_qoder_cn_secret_json(&db_path, QODER_CN_SECRET_USER_INFO_KEY)?;
    let user_plan = read_qoder_cn_secret_json(&db_path, QODER_CN_SECRET_USER_PLAN_KEY)?;

    logger::log_info(&format!(
        "[QoderCN Import] 从 state.vscdb 读取: user_info={}, user_plan={}",
        if user_info.is_some() { "Some" } else { "None" },
        if user_plan.is_some() { "Some" } else { "None" }
    ));

    if user_info.is_none() && user_plan.is_none() {
        return Err("Qoder CN 数据库中没有找到认证数据，请先在 Qoder CN 中登录".to_string());
    }

    // 3. 从 userInfo 提取用户信息和 token
    let mut user_id: Option<String> = None;
    let mut email: Option<String> = None;
    let mut display_name: Option<String> = None;
    let mut access_token: Option<String> = None;
    let mut refresh_token: Option<String> = None;

    if let Some(ref info) = user_info {
        user_id = extract_str(info, "id").or_else(|| extract_str(info, "userId"));
        email = extract_str(info, "email");
        display_name = extract_str(info, "name").or_else(|| extract_str(info, "displayName"));
        access_token = extract_str(info, "accessToken").or_else(|| extract_str(info, "token"));
        refresh_token = extract_str(info, "refreshToken");
        logger::log_info(&format!(
            "[QoderCN Account] 从 state.vscdb userInfo 提取: user_id={:?}, email={:?}",
            user_id, email
        ));
    }

    // 4. 从 userPlan 提取配额信息
    let quota_raw = user_plan.clone();

    let email = email.unwrap_or_else(|| "unknown@qoder-cn.local".to_string());

    // 5. 构建账号 ID
    let account_id = if let Some(ref uid) = user_id {
        format!("qoder_cn_uid_{}", sanitize_account_id_component(uid))
    } else {
        format!(
            "qoder_cn_email_{}",
            sanitize_account_id_component(&email.to_lowercase())
        )
    };

    // 6. 备份当前会话文件
    let now = now_ts();
    if let Err(err) = backup_current_session_to(&account_id) {
        logger::log_warn(&format!(
            "[QoderCN Account] 本地导入时备份会话失败: {}",
            err
        ));
    }

    // 7. 创建账号记录
    let account = QoderCnAccount {
        id: account_id,
        email: email.to_lowercase(),
        user_id,
        display_name,
        user_type: None,
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
        quota_raw,
        auth_user_info_raw: user_info,
        auth_user_plan_raw: user_plan,
        session_backup_at: Some(now),
        created_at: now,
        last_used: now,
    };

    let saved = upsert_account_record(account)?;
    logger::log_info(&format!(
        "[QoderCN Account] 从本地导入成功: id={}, email={}, db={}",
        saved.id, saved.email, db_path.to_string_lossy()
    ));
    Ok(Some(saved))
}

// ==================== OAuth 会话文件写入 ====================

/// OAuth 登录后，主动写入 Qoder CN 应用能识别的认证文件。
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
    logger::log_info("[QoderCN Account] 开始写入 OAuth 会话文件");

    // 1. 写入 auth-v2.dat (加密的 JSON)
    if let Some(app_dir) = get_qoder_cn_app_data_dir() {
        // Convert token_expires_at (millis) to ISO 8601 string for Qoder CN
        let expires_at_iso = token_expires_at.map(|ts_ms| {
            chrono::DateTime::from_timestamp_millis(ts_ms)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| ts_ms.to_string())
        });

        // Read machine-id for loginDeviceId
        let machine_id = get_qoder_cn_machine_id();

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
        match super::vscode_inject::encrypt_qoder_cn_safe_storage_data(&json_bytes) {
            Ok(encrypted) => {
                let auth_v2_path = app_dir.join("auth-v2.dat");
                fs::write(&auth_v2_path, &encrypted).map_err(|e| {
                    format!("写入 auth-v2.dat 失败: {}", e)
                })?;
                logger::log_info(&format!(
                    "[QoderCN Account] 写入 auth-v2.dat 完成: {} bytes, json_len={}",
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
                        "[QoderCN Account] auth-v2.dat JSON: {}",
                        serde_json::to_string_pretty(&val).unwrap_or_default()
                    ));
                }
                // Verify: try to decrypt the written file
                match super::vscode_inject::decrypt_qoder_cn_safe_storage_file(&auth_v2_path) {
                    Ok(decrypted) => {
                        logger::log_info(&format!(
                            "[QoderCN Account] auth-v2.dat 验证解密成功: {} bytes",
                            decrypted.len()
                        ));
                    }
                    Err(err) => {
                        logger::log_warn(&format!(
                            "[QoderCN Account] auth-v2.dat 验证解密失败: {}",
                            err
                        ));
                    }
                }
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[QoderCN Account] 加密 auth-v2.dat 失败: {} (跳过)",
                    err
                ));
            }
        }
    } else {
        logger::log_warn("[QoderCN Account] 应用数据目录不存在，跳过 auth-v2.dat");
    }

    // 2. 写入 .auth-cn/id 和 .auth-cn/user
    if let Some(cfg_dir) = get_qoder_cn_config_dir() {
        let auth_cn_dir = cfg_dir.join(".auth-cn");
        let _ = fs::create_dir_all(&auth_cn_dir);

        if let Some(uid) = user_id {
            let id_path = auth_cn_dir.join("id");
            fs::write(&id_path, uid).map_err(|e| {
                format!("写入 .auth-cn/id 失败: {}", e)
            })?;
            logger::log_info(&format!("[QoderCN Account] 写入 .auth-cn/id: {}", uid));
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
                "[QoderCN Account] 写入 .auth-cn/user: {} bytes",
                user_json.len()
            ));
        }
    } else {
        logger::log_warn("[QoderCN Account] 配置目录不存在，跳过 .auth-cn");
    }

    // 3. 更新 .status.json，设置 logged_in: true
    if let Some(cfg_dir) = get_qoder_cn_config_dir() {
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
            "[QoderCN Account] 更新 .status.json: logged_in=true"
        ));
    }

    logger::log_info("[QoderCN Account] OAuth 会话文件写入完成");
    Ok(())
}

// ==================== 会话切换 ====================

/// 需要备份/恢复的认证文件列表
const AUTH_FILES: &[&str] = &["auth.dat", "auth-v2.dat", "lockfile"];

/// 备份当前活跃会话到指定账号的备份目录
pub fn backup_current_session_to(account_id: &str) -> Result<(), String> {
    let backup_dir = get_session_backup_dir(account_id)?;
    let app_data_dir = get_qoder_cn_app_data_dir();
    let config_dir = get_qoder_cn_config_dir();

    logger::log_info(&format!(
        "[QoderCN Account] 开始备份会话: account_id={}, backup_dir={}",
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
                "[QoderCN Account] 应用数据目录内容: {:?}",
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
                "[QoderCN Account] 配置目录内容: {:?}",
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
                    "[QoderCN Account] 源文件中不存在: app_data/{}",
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
                        "[QoderCN Account] 源文件中不存在: .auth-cn/{}",
                        entry
                    ));
                }
            }
        } else {
            logger::log_warn(&format!(
                "[QoderCN Account] 源目录中不存在: .auth-cn"
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
                    "[QoderCN Account] 备份的 .status.json 内容: {}",
                    content
                ));
            }
        } else {
            logger::log_warn(&format!(
                "[QoderCN Account] 源文件中不存在: .status.json"
            ));
        }
    }

    // 注意：不在此处从 state.vscdb 读取 auth_user_info_raw。
    // 因为 cockpit 认为的"当前账号"可能和 state.vscdb 中实际存储的账号不一致
    // （之前的切换可能失败，Qoder CN 仍使用旧账号），这会导致把 A 账号的数据
    // 存入 B 账号的 auth_user_info_raw，造成交叉污染。
    // state.vscdb 注入已改为始终使用 build_user_info_fallback 从账号记录构建。

    logger::log_info(&format!(
        "[QoderCN Account] 会话备份完成: account_id={}, backup_dir={}, backed_up_files={:?}",
        account_id,
        backup_dir.display(),
        backed_up_files
    ));
    Ok(())
}

/// 检测当前活跃账号 ID
pub fn detect_current_active_account_id() -> Option<String> {
    let db_path = get_qoder_cn_state_db_path()?;
    let user_info = read_qoder_cn_secret_json(&db_path, QODER_CN_SECRET_USER_INFO_KEY).ok()??;
    let user_id = extract_str(&user_info, "id").or_else(|| extract_str(&user_info, "userId"))?;

    // Find matching account by user_id
    let accounts = list_accounts();
    accounts
        .iter()
        .find(|a| a.user_id.as_deref() == Some(&user_id))
        .map(|a| a.id.clone())
}

/// 杀死 Qoder CN 相关进程
fn kill_qoder_cn_processes() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Use pkill to kill Qoder CN processes
        let process_patterns = [
            "Qoder CN",
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
            "Qoder CN.exe",
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
        let process_patterns = ["qodercn", "qoderclicn", "qodercli"];
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
    if let Some(app_dir) = get_qoder_cn_app_data_dir() {
        for file_name in AUTH_FILES {
            let path = app_dir.join(file_name);
            if path.exists() {
                fs::remove_file(&path).map_err(|e| {
                    format!("清除 {} 失败: {}", file_name, e)
                })?;
            }
        }
    }

    if let Some(cfg_dir) = get_qoder_cn_config_dir() {
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
        "[QoderCN Account] 开始恢复会话: account_id={}, backup_dir={}",
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
            "[QoderCN Account] 备份目录内容: {:?}",
            files
        ));
    }

    // 恢复应用数据目录中的认证文件
    let mut restored_files = Vec::new();
    if let Some(app_dir) = get_qoder_cn_app_data_dir() {
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
                    "[QoderCN Account] 备份文件中不存在: {}",
                    file_name
                ));
            }
        }
    }

    // 恢复 .auth-cn/ 文件
    if let Some(cfg_dir) = get_qoder_cn_config_dir() {
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
                        "[QoderCN Account] 备份文件中不存在: .auth-cn/{}",
                        entry
                    ));
                }
            }
        } else {
            logger::log_warn(&format!(
                "[QoderCN Account] 备份目录中不存在: dot_auth-cn"
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
                    "[QoderCN Account] 恢复的 .status.json 内容: {}",
                    content
                ));
            }
        } else {
            logger::log_warn(&format!(
                "[QoderCN Account] 备份文件中不存在: dot_status.json"
            ));
        }
    }

    logger::log_info(&format!(
        "[QoderCN Account] 会话恢复完成: account_id={}, restored_files={:?}",
        account_id, restored_files
    ));
    Ok(())
}

/// 重启 Qoder CN 应用
fn restart_qoder_cn_app() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("open")
            .args(["-a", "Qoder CN"])
            .output()
            .map_err(|e| format!("启动 Qoder CN 失败: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("启动 Qoder CN 失败: {}", stderr));
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Try common installation paths
        let common_paths = [
            PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
                .join("Qoder CN")
                .join("Qoder CN.exe"),
            PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
                .join("Programs")
                .join("Qoder CN")
                .join("Qoder CN.exe"),
        ];
        let mut launched = false;
        for path in &common_paths {
            if path.exists() {
                std::process::Command::new(path)
                    .spawn()
                    .map_err(|e| format!("启动 Qoder CN 失败: {}", e))?;
                launched = true;
                break;
            }
        }
        if !launched {
            return Err("未找到 Qoder CN 安装路径".to_string());
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("qoder-cn")
            .spawn()
            .map_err(|e| format!("启动 Qoder CN 失败: {}", e))?;
    }

    // Wait for app to start
    std::thread::sleep(std::time::Duration::from_secs(2));
    Ok(())
}

/// 切换账号：备份当前会话 → 杀进程 → 注入目标账号数据到 state.vscdb → 重启应用
pub fn switch_account(account_id: &str) -> Result<String, String> {
    let account = load_account(account_id)
        .ok_or_else(|| format!("Qoder CN 账号不存在: {}", account_id))?;

    // 1. 检测并备份当前活跃账号的会话
    if let Some(current_id) = detect_current_active_account_id() {
        if current_id != account_id {
            logger::log_info(&format!(
                "[QoderCN Switch] 备份当前账号会话: {}",
                current_id
            ));
            if let Err(err) = backup_current_session_to(&current_id) {
                logger::log_warn(&format!(
                    "[QoderCN Switch] 备份当前会话失败（继续切换）: {}",
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

    // 2. 杀死 Qoder CN 进程
    logger::log_info("[QoderCN Switch] 关闭 Qoder CN 进程");
    kill_qoder_cn_processes()?;

    // 2.5 检查账号是否有 access_token
    // 注意：不在这里主动刷新 token，因为 deviceToken/refresh API 返回的是
    // device token (dt-...)，而 Qoder CN 内部使用的是 session token。
    // 主动刷新会覆盖原始 access_token，导致 Qoder CN 启动时无法识别。
    // 如果有原始 access_token 就直接使用，让 Qoder CN 自己处理 token 刷新。
    let account = if account.access_token.is_none() || account.access_token.as_deref() == Some("") {
        match sync_refresh_qoder_cn_token(account_id) {
            Ok(updated) => {
                logger::log_info(&format!(
                    "[QoderCN Switch] token 刷新成功（account 无 access_token）: token={}...",
                    updated.access_token.as_deref().unwrap_or("?").chars().take(15).collect::<String>()
                ));
                updated
            }
            Err(err) => {
                return Err(format!("账号没有 access_token 且刷新失败: {}", err));
            }
        }
    } else {
        logger::log_info(&format!(
            "[QoderCN Switch] 使用账号现有 access_token: {}... (跳过刷新，让 Qoder CN 自行管理 token)",
            account.access_token.as_deref().unwrap_or("?").chars().take(15).collect::<String>()
        ));
        account
    };

    // 3. 获取 state.vscdb 路径
    let Some(db_path) = get_qoder_cn_state_db_path() else {
        return Err("未找到 Qoder CN 数据库 (state.vscdb)，请先在 Qoder CN 中登录一次".to_string());
    };

    // 4. 准备注入数据
    // 始终使用 build_user_info_fallback 从账号记录构建（user_id/token/refreshToken 均来自账号记录，数据正确）。
    // 不使用 auth_user_info_raw，因为它可能在备份时从 state.vscdb 采集到错误账号的数据（交叉污染）。
    const USER_INFO_KEY: &str = "secret://aicoding.auth.userInfo";
    const USER_PLAN_KEY: &str = "secret://aicoding.auth.userPlan";

    let user_info_json = serde_json::to_string(&build_user_info_fallback(&account))
        .map_err(|e| format!("序列化 Qoder CN userInfo 失败: {}", e))?;
    let user_plan_json = serde_json::to_string(&build_user_plan_fallback(&account))
        .map_err(|e| format!("序列化 Qoder CN userPlan 失败: {}", e))?;

    // 5. 注入目标账号数据到 state.vscdb
    logger::log_info(&format!(
        "[QoderCN Switch] 注入目标账号数据到 state.vscdb: {}",
        account_id
    ));
    super::vscode_inject::inject_secret_to_state_db_for_qoder_cn(
        &db_path,
        USER_INFO_KEY,
        &user_info_json,
    )?;
    super::vscode_inject::inject_secret_to_state_db_for_qoder_cn(
        &db_path,
        USER_PLAN_KEY,
        &user_plan_json,
    )?;

    // 6. 同步更新 .auth-cn/ 和 .status.json 文件
    // 防止 Qoder CN 启动时从这些文件读取并覆盖 state.vscdb
    logger::log_info(&format!(
        "[QoderCN Switch] 检查 auth_user_info_raw: has={}",
        account.auth_user_info_raw.is_some()
    ));
    if let Some(cfg_dir) = get_qoder_cn_config_dir() {
        let auth_cn_dir = cfg_dir.join(".auth-cn");
        let _ = fs::create_dir_all(&auth_cn_dir);

        // 写入 .auth-cn/id
        if let Some(ref uid) = account.user_id {
            let id_path = auth_cn_dir.join("id");
            fs::write(&id_path, uid).map_err(|e| {
                format!("写入 .auth-cn/id 失败: {}", e)
            })?;
            logger::log_info(&format!("[QoderCN Switch] 更新 .auth-cn/id: {}", uid));
        }

        // 写入 .auth-cn/user（Qoder CN 原生用户档案格式）
        // 注意：auth_user_info_raw 和 build_user_info_fallback 产生的是 state.vscdb 格式
        // （含 token/accountId/status/quota 等），而 .auth-cn/user 是用户档案格式
        // （含 avatar/username/created_at/source/third_party_identities 等），两者结构完全不同。
        // 因此这里必须用档案格式构建，不能用 state.vscdb 格式。
        // 数据来源：优先使用 auth_user_info_raw（本身就是档案格式）；否则用账号记录中
        // 权威且干净的字段重建。绝不从磁盘备份恢复——备份可能被交叉污染
        // （见 :1378 旧逻辑：019f2b5e 的备份里错误地存了 019f258c 的 id，导致切到
        // nick* 时仍显示 Vpen190），从备份恢复会重现该问题。
        let user_path = auth_cn_dir.join("user");
        let user_json: serde_json::Value = match &account.auth_user_info_raw {
            Some(raw) => raw.clone(),
            None => build_user_profile_fallback(&account),
        };
        let user_str = serde_json::to_string(&user_json)
            .map_err(|e| format!("序列化 .auth-cn/user 失败: {}", e))?;
        fs::write(&user_path, &user_str).map_err(|e| {
            format!("写入 .auth-cn/user 失败: {}", e)
        })?;
        logger::log_info(&format!(
            "[QoderCN Switch] 写入 .auth-cn/user: {} bytes (source={})",
            user_str.len(),
            if account.auth_user_info_raw.is_some() {
                "raw"
            } else {
                "fallback"
            }
        ));

        // 更新 .status.json
        let status_path = cfg_dir.join(".status.json");
        let mut status: serde_json::Value = if status_path.exists() {
            fs::read_to_string(&status_path)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_else(|| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        status["logged_in"] = serde_json::json!(true);
        if let Some(ref name) = account.display_name {
            status["username"] = serde_json::json!(name);
            status["name"] = serde_json::json!(name);
        }
        status["email"] = serde_json::json!(&account.email);
        // 始终从 user_id 构造头像 URL（auth_user_info_raw 可能被污染，不可信）
        if let Some(ref uid) = account.user_id {
            status["avatar_url"] = serde_json::json!(format!(
                "https://qoder.com.cn/users/{}/default/avatars",
                uid
            ));
        }

        let status_json = serde_json::to_string_pretty(&status)
            .map_err(|e| format!("序列化 .status.json 失败: {}", e))?;
        fs::write(&status_path, &status_json).map_err(|e| {
            format!("写入 .status.json 失败: {}", e)
        })?;
        logger::log_info(&format!(
            "[QoderCN Switch] 更新 .status.json: logged_in=true, email={}",
            account.email
        ));
    }

    // 6.5 删除旧的 auth.dat，避免 Qoder CN 从旧文件读取冲突数据
    if let Some(app_dir) = get_qoder_cn_app_data_dir() {
        let auth_dat_path = app_dir.join("auth.dat");
        if auth_dat_path.exists() {
            let _ = fs::remove_file(&auth_dat_path);
            logger::log_info("[QoderCN Switch] 删除旧的 auth.dat（避免与 auth-v2.dat 冲突）");
        }
    }

    // 7. 生成并写入 auth-v2.dat（从账号的 access_token/refresh_token 构建）
    // 这是关键：Qoder CN 启动时会从 auth-v2.dat 读取认证信息
    match write_auth_v2_dat(&account) {
        Ok(()) => {}
        Err(err) => {
            logger::log_warn(&format!(
                "[QoderCN Switch] 生成 auth-v2.dat 失败（尝试从备份恢复）: {}",
                err
            ));
            // Fallback: 从备份目录恢复
            let backup_dir = get_session_backup_dir(account_id)?;
            let auth_v2_src = backup_dir.join("auth-v2.dat");
            if auth_v2_src.exists() {
                if let Some(app_dir) = get_qoder_cn_app_data_dir() {
                    let auth_v2_dst = app_dir.join("auth-v2.dat");
                    fs::copy(&auth_v2_src, &auth_v2_dst).map_err(|e| {
                        format!("恢复 auth-v2.dat 失败: {}", e)
                    })?;
                }
            }
        }
    }

    // 8. 更新当前账号记录
    crate::modules::provider_current_state::set_current_account_id(
        "qoder_cn",
        Some(account_id),
    )?;

    // 9. 重启应用
    logger::log_info("[QoderCN Switch] 重启 Qoder CN");
    if let Err(err) = restart_qoder_cn_app() {
        logger::log_warn(&format!("[QoderCN Switch] 重启失败: {}", err));
        return Ok(format!("切换完成，但 Qoder CN 启动失败: {}", err));
    }

    // Wait for Qoder CN to fully initialize, then verify state.vscdb wasn't cleared
    std::thread::sleep(std::time::Duration::from_secs(4));

    if let Some(db_path) = get_qoder_cn_state_db_path() {
        match read_qoder_cn_secret_json(&db_path, USER_INFO_KEY) {
            Ok(Some(info)) => {
                let status = info.get("status").and_then(|v| v.as_i64()).unwrap_or(-1);
                let tok = info.get("token").or(info.get("accessToken"))
                    .and_then(|v| v.as_str())
                    .map(|s| format!("{}...({}chars)", &s[..s.len().min(12)], s.len()))
                    .unwrap_or_else(|| "None".to_string());
                let uid = info.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                logger::log_info(&format!(
                    "[QoderCN Switch] 启动后验证 state.vscdb.userInfo: status={}, id={}, token={}",
                    status, uid, tok
                ));
            }
            Ok(None) => {
                logger::log_warn("[QoderCN Switch] 启动后验证: state.vscdb 中 userInfo 已被清除!");
            }
            Err(e) => {
                logger::log_warn(&format!(
                    "[QoderCN Switch] 启动后验证: 读取 state.vscdb 失败: {}",
                    e
                ));
            }
        }
    }

    // Update account's last_used
    if let Some(mut updated) = load_account(account_id) {
        updated.last_used = now_ts();
        let _ = upsert_account_record(updated);
    }

    Ok(format!("切换完成: {}", account.email))
}

// ==================== 辅助函数 ====================

/// Synchronously refresh the Qoder CN account token using curl.
/// Returns the updated account with fresh access_token.
fn sync_refresh_qoder_cn_token(account_id: &str) -> Result<QoderCnAccount, String> {
    let account = load_account(account_id)
        .ok_or_else(|| format!("Qoder CN 账号不存在: {}", account_id))?;

    let refresh_token = account.refresh_token.as_ref()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "账号缺少 refresh_token".to_string())?;

    // Call refresh API using curl
    let output = std::process::Command::new("curl")
        .args([
            "-s", "-X", "POST",
            "-H", "Content-Type: application/json",
            "-d", &format!("{{\"refresh_token\":\"{}\"}}", refresh_token),
            "https://openapi.qoder.com.cn/api/v1/deviceToken/refresh",
        ])
        .output()
        .map_err(|e| format!("curl 执行失败: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "curl 返回错误: status={}",
            output.status
        ));
    }

    let body = String::from_utf8_lossy(&output.stdout).to_string();
    let resp: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("解析刷新响应失败: {}, body={}", e, &body[..body.len().min(200)]))?;

    // Log full response (with token redacted) for debugging
    let mut redacted_resp = resp.clone();
    for key in &["token", "device_token", "access_token", "accessToken", "refresh_token", "refreshToken"] {
        if let Some(val) = redacted_resp.get(*key).and_then(|v| v.as_str()) {
            let redacted = format!("{}...({}chars)", &val[..val.len().min(12)], val.len());
            redacted_resp[*key] = serde_json::json!(redacted);
        }
    }
    logger::log_info(&format!(
        "[QoderCN Switch] refresh API response: {}",
        serde_json::to_string(&redacted_resp).unwrap_or_default()
    ));

    // Extract new token from response:
    // Priority: access_token > accessToken > token > device_token
    // (access_token is the regular auth token that Qoder CN uses internally)
    let new_token = resp.get("access_token")
        .or(resp.get("accessToken"))
        .or(resp.get("token"))
        .or(resp.get("device_token"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Also extract new refresh_token if provided (some APIs rotate refresh tokens)
    let new_refresh_token = resp.get("refresh_token")
        .or(resp.get("refreshToken"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut updated = account.clone();
    if let Some(token) = new_token {
        logger::log_info(&format!(
            "[QoderCN Switch] 获得新 token: {}... (type={})",
            token.chars().take(15).collect::<String>(),
            if token.starts_with("dt-") { "device_token" } else { "access_token" }
        ));
        updated.access_token = Some(token);
        // Update expire time (typically 1 hour from now)
        updated.token_expires_at = Some(
            chrono::Utc::now().timestamp_millis() + 3600_000
        );
        // Also update refresh_token if the API returned a new one
        if let Some(ref new_rt) = new_refresh_token {
            logger::log_info(&format!(
                "[QoderCN Switch] 获得新 refresh_token: {}...",
                new_rt.chars().take(15).collect::<String>()
            ));
            updated.refresh_token = Some(new_rt.clone());
        }
        // Save updated account
        let _ = upsert_account_record(updated.clone());
    } else {
        return Err(format!(
            "刷新响应中没有 token: {}",
            &body[..body.len().min(200)]
        ));
    }

    Ok(updated)
}

fn build_user_info_fallback(account: &QoderCnAccount) -> serde_json::Value {
    let uid = account.user_id.clone().unwrap_or_default();
    let avatar_url = format!(
        "https://qoder.com.cn/users/{}/default/avatars",
        uid
    );
    let expire_time = account.token_expires_at.unwrap_or(0);
    let is_quota_exceeded = account.is_quota_exceeded.unwrap_or(false);
    let user_tag = match account.user_type.as_deref() {
        Some("personal_professional_trial") => "Pro Trial",
        Some("personal_free") => "Free",
        Some("personal_professional") => "Pro",
        _ => "",
    };

    serde_json::json!({
        "status": 2,
        "name": account.display_name.clone().unwrap_or_default(),
        "id": uid,
        "accountId": uid,
        "token": account.access_token.clone().unwrap_or_default(),
        "quota": 0,
        "userType": account.user_type.clone().unwrap_or_else(|| "unknown".to_string()),
        "whitelist": 3,
        "orgId": "",
        "orgName": "",
        "yxUid": "",
        "staffId": "",
        "avatarUrl": avatar_url,
        "messageId": "",
        "email": account.email,
        "refreshToken": account.refresh_token.clone().unwrap_or_default(),
        "expireTime": expire_time,
        "isSubAccount": false,
        "isQuotaExceeded": is_quota_exceeded,
        "privacyPolicyAgreed": true,
        "isPrivacyPolicyModifiable": false,
        "cloudType": "",
        "featureSwitches": {"allow_byok": 1},
        "userTag": user_tag,
    })
}

/// 构建 `.auth-cn/user` 所需的**用户档案**格式（注意：这是用户档案，不是 state.vscdb 格式）。
/// 当账号记录的 `auth_user_info_raw` 为空时使用本函数，从账号记录中权威且干净的字段重建，
/// 避免读取可能被交叉污染的磁盘备份。
fn build_user_profile_fallback(account: &QoderCnAccount) -> serde_json::Value {
    let uid = account.user_id.clone().unwrap_or_default();
    let avatar = format!("https://qoder.com.cn/users/{}/default/avatars", uid);
    let created_at = if account.created_at > 0 {
        chrono::DateTime::from_timestamp(account.created_at, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    } else {
        String::new()
    };
    serde_json::json!({
        "avatar": avatar,
        "created_at": created_at,
        "id": uid,
        "is_highest_tier": false,
        "is_privacy_policy_modifiable": false,
        "name": account.display_name.clone().unwrap_or_default(),
        "organization_id": "",
        "organization_name": "",
        "source": "sso.aliyun",
        "third_party_identities": [],
        "username": account.email
    })
}

fn build_user_plan_fallback(account: &QoderCnAccount) -> serde_json::Value {
    let user_type = account.user_type.clone().unwrap_or_else(|| "unknown".to_string());
    // Try to extract plan info from quota_raw
    let (plan_name, start_date, end_date) = if let Some(ref quota) = account.quota_raw {
        let expires = quota.get("expiresAt").and_then(|v| v.as_i64()).unwrap_or(0);
        (
            match user_type.as_str() {
                "personal_professional_trial" => "Pro Trial",
                "personal_free" => "Free",
                "personal_professional" => "Pro",
                _ => "Unknown",
            }.to_string(),
            0i64,
            expires,
        )
    } else {
        ("Unknown".to_string(), 0i64, 0i64)
    };

    serde_json::json!({
        "user_type": user_type,
        "plan_tier_name": plan_name,
        "is_personal_version": true,
        "is_highest_tier": false,
        "feature_allowed": {"quest": true, "wiki": true, "commit_indexing": true},
        "start_date": start_date,
        "end_date": end_date,
        "login_version": "2.0",
        "requestId": "",
        "errorCode": "",
        "errorMessage": "",
        "Result": null
    })
}

/// Generate and write auth-v2.dat from account tokens using AuthV2Data struct to ensure
/// exact field naming and format consistency with write_oauth_session_files.
fn write_auth_v2_dat(account: &QoderCnAccount) -> Result<(), String> {
    let Some(ref token) = account.access_token else {
        return Err("账号没有 access_token，无法生成 auth-v2.dat".to_string());
    };
    let Some(ref refresh_token) = account.refresh_token else {
        return Err("账号没有 refresh_token，无法生成 auth-v2.dat".to_string());
    };

    let uid = account.user_id.clone().unwrap_or_default();
    let expires_at = if let Some(exp_ms) = account.token_expires_at {
        chrono::DateTime::from_timestamp_millis(exp_ms)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "2099-01-01T00:00:00+00:00".to_string())
    } else {
        "2099-01-01T00:00:00+00:00".to_string()
    };

    let machine_id = get_qoder_cn_machine_id();

    let auth_v2 = AuthV2Data {
        token: Some(token.clone()),
        refresh_token: Some(refresh_token.clone()),
        expires_at: Some(expires_at),
        user: Some(AuthV2User {
            id: Some(uid),
            email: Some(account.email.clone()),
            name: account.display_name.clone(),
        }),
        schema_version: Some(2),
        login_method: Some("browser".to_string()),
        refresh_strategy: Some("device_token".to_string()),
        refresh_token_expires_at: None,
        login_device_id: machine_id,
        login_timestamp: Some(chrono::Utc::now().timestamp_millis()),
    };

    let plaintext = serde_json::to_string(&auth_v2)
        .map_err(|e| format!("序列化 auth-v2.dat JSON 失败: {}", e))?;

    let Some(app_dir) = get_qoder_cn_app_data_dir() else {
        return Err("未找到 Qoder CN 应用数据目录".to_string());
    };

    let encrypted = super::vscode_inject::encrypt_qoder_cn_safe_storage_data(plaintext.as_bytes())?;
    let auth_v2_path = app_dir.join("auth-v2.dat");
    fs::write(&auth_v2_path, &encrypted).map_err(|e| {
        format!("写入 auth-v2.dat 失败: {}", e)
    })?;

    logger::log_info(&format!(
        "[QoderCN Switch] 生成 auth-v2.dat: {} bytes (user={})",
        encrypted.len(),
        account.display_name.as_deref().unwrap_or("?")
    ));

    Ok(())
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
