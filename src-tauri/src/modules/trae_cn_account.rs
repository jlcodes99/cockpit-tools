use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::trae::{TraeAccount, TraeImportPayload};
use crate::models::trae_cn::TraeCnAccountIndex;
use crate::modules::{account, logger};

const ACCOUNTS_INDEX_FILE: &str = "trae_cn_accounts.json";
const ACCOUNTS_DIR: &str = "trae_cn_accounts";

lazy_static::lazy_static! {
    static ref TRAE_CN_ACCOUNT_INDEX_LOCK: Mutex<()> = Mutex::new(());
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
    normalize_non_empty(value).and_then(|raw| {
        if raw.contains('@') {
            Some(raw.to_lowercase())
        } else {
            None
        }
    })
}

fn normalize_timestamp(raw: Option<i64>) -> Option<i64> {
    let value = raw?;
    if value <= 0 {
        return None;
    }
    if value > 10_000_000_000 {
        return Some(value / 1000);
    }
    Some(value)
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let dir = get_data_dir()?.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Trae CN 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn get_default_trae_cn_data_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    Ok(home
        .join("Library")
        .join("Application Support")
        .join("Trae CN"))
}

pub fn get_default_trae_cn_storage_path() -> Result<PathBuf, String> {
    Ok(get_default_trae_cn_data_dir()?
        .join("User")
        .join("globalStorage")
        .join("storage.json"))
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
    Ok(get_accounts_dir()?.join(format!("{}.json", normalize_account_id(account_id)?)))
}

pub fn load_account(account_id: &str) -> Option<TraeAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_account_file(account: &TraeAccount) -> Result<(), String> {
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

fn load_account_index() -> TraeCnAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(path) => path,
        Err(_) => return TraeCnAccountIndex::new(),
    };
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(TraeCnAccountIndex::new);
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => repair_account_index_from_details("索引文件为空")
            .unwrap_or_else(TraeCnAccountIndex::new),
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            TraeCnAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(TraeCnAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Trae CN Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(TraeCnAccountIndex::new)
            }
        },
        Err(_) => TraeCnAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<TraeCnAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(TraeCnAccountIndex::new());
    }
    let content = fs::read_to_string(&path).map_err(|err| format!("读取账号索引失败: {}", err))?;
    if content.trim().is_empty() {
        if let Some(index) = repair_account_index_from_details("索引文件为空") {
            return Ok(index);
        }
        return Ok(TraeCnAccountIndex::new());
    }
    crate::modules::atomic_write::parse_json_with_auto_restore::<TraeCnAccountIndex>(
        &path, &content,
    )
    .map_err(|err| {
        crate::error::file_corrupted_error(
            ACCOUNTS_INDEX_FILE,
            &path.to_string_lossy(),
            &err.to_string(),
        )
    })
}

fn save_account_index(index: &TraeCnAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<TraeCnAccountIndex> {
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
    let mut index = TraeCnAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();
    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Trae CN Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });
    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Trae CN Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }
    logger::log_warn(&format!(
        "[Trae CN Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));
    Some(index)
}

fn refresh_summary(index: &mut TraeCnAccountIndex, account: &TraeAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(mut account: TraeAccount) -> Result<TraeAccount, String> {
    let _lock = TRAE_CN_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Trae CN 账号锁失败".to_string())?;
    let now = now_ts();
    if account.created_at <= 0 {
        account.created_at = now;
    }
    account.last_used = now;
    let mut index = load_account_index();
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
}

fn extract_json_value(root: Option<&Value>, path: &[&str]) -> Option<Value> {
    let mut current = root?;
    for key in path {
        current = current.as_object()?.get(*key)?;
    }
    Some(current.clone())
}

fn pick_string(root: Option<&Value>, paths: &[&[&str]]) -> Option<String> {
    for path in paths {
        if let Some(value) = extract_json_value(root, path) {
            if let Some(text) = value.as_str() {
                if let Some(normalized) = normalize_non_empty(Some(text)) {
                    return Some(normalized);
                }
            }
            if let Some(num) = value.as_i64() {
                return Some(num.to_string());
            }
            if let Some(num) = value.as_u64() {
                return Some(num.to_string());
            }
        }
    }
    None
}

fn pick_i64(root: Option<&Value>, paths: &[&[&str]]) -> Option<i64> {
    for path in paths {
        if let Some(value) = extract_json_value(root, path) {
            if let Some(num) = value.as_i64() {
                return Some(num);
            }
            if let Some(num) = value.as_u64() {
                if num <= i64::MAX as u64 {
                    return Some(num as i64);
                }
            }
            if let Some(text) = value.as_str() {
                if let Ok(num) = text.trim().parse::<i64>() {
                    return Some(num);
                }
            }
        }
    }
    None
}

fn account_from_import_value(raw: Value) -> Result<TraeAccount, String> {
    if let Ok(account) = serde_json::from_value::<TraeAccount>(raw.clone()) {
        return Ok(account);
    }

    let now = now_ts();
    let access_token = pick_string(
        Some(&raw),
        &[&["access_token"], &["accessToken"], &["token"], &["trae_access_token"]],
    )
    .ok_or_else(|| "缺少 access_token 字段".to_string())?;
    let user_id = pick_string(Some(&raw), &[&["user_id"], &["userId"], &["uid"], &["id"]]);
    let email = normalize_email(
        pick_string(Some(&raw), &[&["email"], &["trae_email"], &["user", "email"]]).as_deref(),
    )
    .unwrap_or_else(|| "unknown".to_string());
    let identity = user_id
        .clone()
        .or_else(|| normalize_email(Some(email.as_str())))
        .unwrap_or_else(|| access_token.clone());
    let id = pick_string(Some(&raw), &[&["id"]])
        .filter(|value| normalize_account_id(value).is_ok())
        .unwrap_or_else(|| format!("trae_cn_{:x}", md5::compute(identity.as_bytes())));

    Ok(TraeAccount {
        id,
        email,
        user_id,
        nickname: pick_string(
            Some(&raw),
            &[&["nickname"], &["name"], &["displayName"], &["user", "name"]],
        ),
        tags: None,
        access_token,
        refresh_token: pick_string(Some(&raw), &[&["refresh_token"], &["refreshToken"]]),
        token_type: pick_string(Some(&raw), &[&["token_type"], &["tokenType"]]),
        expires_at: normalize_timestamp(pick_i64(
            Some(&raw),
            &[&["expires_at"], &["expiresAt"], &["expiredAt"]],
        )),
        plan_type: pick_string(
            Some(&raw),
            &[&["plan_type"], &["planType"], &["identityStr"], &["identity_str"]],
        ),
        plan_reset_at: normalize_timestamp(pick_i64(
            Some(&raw),
            &[&["plan_reset_at"], &["planResetAt"]],
        )),
        trae_auth_raw: raw
            .get("trae_auth_raw")
            .cloned()
            .or_else(|| raw.get("auth_raw").cloned())
            .or_else(|| raw.get("auth").cloned()),
        trae_profile_raw: raw
            .get("trae_profile_raw")
            .cloned()
            .or_else(|| raw.get("profile_raw").cloned())
            .or_else(|| raw.get("profile").cloned()),
        trae_entitlement_raw: raw
            .get("trae_entitlement_raw")
            .cloned()
            .or_else(|| raw.get("entitlement_raw").cloned())
            .or_else(|| raw.get("quota_raw").cloned()),
        trae_usage_raw: raw
            .get("trae_usage_raw")
            .cloned()
            .or_else(|| raw.get("usage_raw").cloned()),
        trae_server_raw: raw
            .get("trae_server_raw")
            .cloned()
            .or_else(|| raw.get("server_raw").cloned())
            .or_else(|| raw.get("server").cloned()),
        trae_usertag_raw: raw
            .get("trae_usertag_raw")
            .and_then(|value| value.as_str())
            .and_then(|value| normalize_non_empty(Some(value))),
        status: pick_string(Some(&raw), &[&["status"]]),
        status_reason: pick_string(Some(&raw), &[&["status_reason"], &["statusReason"]]),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        created_at: now,
        last_used: now,
    })
}

fn merge_auth_raw_for_cn(
    auth_raw: Option<Value>,
    payload: &TraeImportPayload,
) -> Option<Value> {
    let mut map = auth_raw
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();

    map.insert(
        "token".to_string(),
        Value::String(payload.access_token.clone()),
    );
    if let Some(refresh_token) = normalize_non_empty(payload.refresh_token.as_deref()) {
        map.insert("refreshToken".to_string(), Value::String(refresh_token));
    }
    if let Some(user_id) = normalize_non_empty(payload.user_id.as_deref()) {
        map.insert("userId".to_string(), Value::String(user_id));
    }
    if let Some(expires_at) = normalize_timestamp(payload.expires_at) {
        map.insert(
            "expiredAt".to_string(),
            Value::Number(serde_json::Number::from(expires_at * 1000)),
        );
    }
    map.insert(
        "host".to_string(),
        Value::String("https://api.trae.cn".to_string()),
    );
    map.insert(
        "userRegion".to_string(),
        serde_json::json!({
            "region": "CN",
            "_aiRegion": "CN"
        }),
    );

    Some(Value::Object(map))
}

fn account_from_import_payload(payload: TraeImportPayload) -> Result<TraeAccount, String> {
    let now = now_ts();
    let normalized_user_id = normalize_non_empty(payload.user_id.as_deref());
    let normalized_email = normalize_email(Some(payload.email.as_str()))
        .unwrap_or_else(|| payload.email.trim().to_string());
    let identity = normalized_user_id
        .clone()
        .or_else(|| normalize_email(Some(normalized_email.as_str())))
        .unwrap_or_else(|| payload.access_token.clone());

    Ok(TraeAccount {
        id: format!("trae_cn_{:x}", md5::compute(identity.as_bytes())),
        email: normalized_email,
        user_id: normalized_user_id,
        nickname: normalize_non_empty(payload.nickname.as_deref()),
        tags: None,
        access_token: payload.access_token.clone(),
        refresh_token: normalize_non_empty(payload.refresh_token.as_deref()),
        token_type: normalize_non_empty(payload.token_type.as_deref()),
        expires_at: normalize_timestamp(payload.expires_at),
        plan_type: normalize_non_empty(payload.plan_type.as_deref()),
        plan_reset_at: normalize_timestamp(payload.plan_reset_at),
        trae_auth_raw: merge_auth_raw_for_cn(payload.trae_auth_raw.clone(), &payload),
        trae_profile_raw: payload.trae_profile_raw.clone(),
        trae_entitlement_raw: payload.trae_entitlement_raw.clone(),
        trae_usage_raw: payload.trae_usage_raw.clone(),
        trae_server_raw: payload.trae_server_raw.clone(),
        trae_usertag_raw: normalize_non_empty(payload.trae_usertag_raw.as_deref()),
        status: normalize_non_empty(payload.status.as_deref()),
        status_reason: normalize_non_empty(payload.status_reason.as_deref()),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        created_at: now,
        last_used: now,
    })
}

fn accounts_from_json_value(raw: Value) -> Result<Vec<TraeAccount>, String> {
    match raw {
        Value::Array(items) => {
            if items.is_empty() {
                return Err("导入数组为空".to_string());
            }
            items
                .into_iter()
                .enumerate()
                .map(|(idx, item)| {
                    account_from_import_value(item)
                        .map_err(|e| format!("第 {} 条 Trae CN 账号解析失败: {}", idx + 1, e))
                })
                .collect()
        }
        Value::Object(obj) => {
            if let Some(accounts_raw) = obj.get("accounts") {
                if let Some(accounts) = accounts_raw.as_array() {
                    if accounts.is_empty() {
                        return Err("导入数组为空".to_string());
                    }
                    return accounts
                        .iter()
                        .enumerate()
                        .map(|(idx, item)| {
                            account_from_import_value(item.clone()).map_err(|e| {
                                format!("第 {} 条 Trae CN 账号解析失败: {}", idx + 1, e)
                            })
                        })
                        .collect();
                }
            }
            Ok(vec![account_from_import_value(Value::Object(obj))?])
        }
        _ => Err("Trae CN 导入 JSON 必须是对象或数组".to_string()),
    }
}

pub fn list_accounts_checked() -> Result<Vec<TraeAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(index
        .accounts
        .iter()
        .filter_map(|item| load_account(item.id.as_str()))
        .collect())
}

pub fn list_accounts() -> Vec<TraeAccount> {
    let index = load_account_index();
    index
        .accounts
        .iter()
        .filter_map(|item| load_account(item.id.as_str()))
        .collect()
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = TRAE_CN_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Trae CN 账号锁失败".to_string())?;
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

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<TraeAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    account.tags = Some(tags);
    upsert_account_record(account)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<TraeAccount>, String> {
    let value =
        serde_json::from_str::<Value>(json_content).map_err(|e| format!("解析 JSON 失败: {}", e))?;
    let accounts = accounts_from_json_value(value)?;
    let mut result = Vec::with_capacity(accounts.len());
    for account in accounts {
        result.push(upsert_account_record(account)?);
    }
    Ok(result)
}

pub fn import_from_local() -> Result<Option<TraeAccount>, String> {
    let storage_path = get_default_trae_cn_storage_path()?;
    let payload = match crate::modules::trae_account::read_local_trae_auth_from_storage_path(
        &storage_path,
    )? {
        Some(payload) => payload,
        None => return Ok(None),
    };
    let account = account_from_import_payload(payload)?;
    let account = upsert_account_record(account)?;
    logger::log_info(&format!(
        "[Trae CN Account] 本地登录态导入成功: id={}, email={}",
        account.id, account.email
    ));
    Ok(Some(account))
}

pub fn add_account_with_token(access_token: &str) -> Result<TraeAccount, String> {
    let access_token = normalize_non_empty(Some(access_token))
        .ok_or_else(|| "Trae CN access token 不能为空".to_string())?;
    let payload = TraeImportPayload {
        email: "unknown".to_string(),
        user_id: None,
        nickname: Some("Trae CN Token".to_string()),
        access_token,
        refresh_token: None,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        plan_type: None,
        plan_reset_at: None,
        trae_auth_raw: None,
        trae_profile_raw: None,
        trae_entitlement_raw: None,
        trae_usage_raw: None,
        trae_server_raw: None,
        trae_usertag_raw: None,
        status: Some("token_only".to_string()),
        status_reason: Some(
            "仅包含 access token；不能刷新 token 或查询完整套餐信息".to_string(),
        ),
    };
    let account = account_from_import_payload(payload)?;
    let account = upsert_account_record(account)?;
    logger::log_info(&format!(
        "[Trae CN Account] Token 导入成功: id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

pub fn upsert_import_payload(payload: TraeImportPayload) -> Result<TraeAccount, String> {
    let account = account_from_import_payload(payload)?;
    upsert_account_record(account)
}

pub fn inject_to_trae_cn(account_id: &str) -> Result<TraeAccount, String> {
    let account =
        load_account(account_id).ok_or_else(|| format!("Trae CN 账号不存在: {}", account_id))?;
    let storage_path = get_default_trae_cn_storage_path()?;
    crate::modules::trae_account::inject_account_to_trae_at_path(storage_path.as_path(), &account)
        .map_err(|err| format!("写入 Trae CN 登录态失败: {}", err))?;
    logger::log_info(&format!(
        "[Trae CN Account] 注入成功: id={}, email={}, path={}",
        account.id,
        account.email,
        storage_path.display()
    ));
    Ok(account)
}

pub(crate) fn resolve_current_account_id(accounts: &[TraeAccount]) -> Option<String> {
    crate::modules::provider_current_state::resolve_existing_current_account_id(
        "trae_cn",
        accounts.iter().map(|account| account.id.as_str()),
    )
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<TraeAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}
