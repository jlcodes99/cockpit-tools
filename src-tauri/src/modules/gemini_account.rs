use crate::models::gemini::{GeminiAccount, GeminiAccountIndex, GeminiOAuthCompletePayload};
use crate::modules::{account, gemini_oauth, logger};
use base64::Engine;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const ACCOUNTS_INDEX_FILE: &str = "gemini_accounts.json";
const ACCOUNTS_DIR: &str = "gemini_accounts";
const GEMINI_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 10 * 60;

const GEMINI_OAUTH_FILE: &str = "oauth_creds.json";
const GEMINI_GOOGLE_ACCOUNTS_FILE: &str = "google_accounts.json";
const GEMINI_SETTINGS_FILE: &str = "settings.json";
const GEMINI_FILE_KEYCHAIN_FILE: &str = "gemini-credentials.json";
const GEMINI_HOME_DIR: &str = ".gemini";
const GEMINI_KEYCHAIN_SERVICE: &str = "gemini-cli-oauth";
const GEMINI_KEYCHAIN_ACCOUNT: &str = "main-account";

const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_ENDPOINT: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const CODE_ASSIST_LOAD_ENDPOINT: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const CODE_ASSIST_RETRIEVE_QUOTA_ENDPOINT: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota";
const CODE_ASSIST_REQUEST_RETRY_COUNT: usize = 3;
const CODE_ASSIST_REQUEST_RETRY_DELAY_MS: u64 = 100;

lazy_static::lazy_static! {
    static ref GEMINI_ACCOUNT_INDEX_LOCK: Mutex<()> = Mutex::new(());
    static ref GEMINI_QUOTA_ALERT_LAST_SENT: Mutex<HashMap<String, i64>> = Mutex::new(HashMap::new());
}

#[derive(Debug, Default, Clone, Deserialize)]
struct LocalOauthCreds {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    expiry_date: Option<Value>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalKeychainOauthCreds {
    token: Option<LocalKeychainToken>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalKeychainToken {
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    expires_at: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalGoogleAccounts {
    active: Option<String>,
    old: Vec<String>,
}

impl Default for LocalGoogleAccounts {
    fn default() -> Self {
        Self {
            active: None,
            old: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GoogleTokenRefreshResponse {
    access_token: Option<String>,
    expires_in: Option<i64>,
    id_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfoResponse {
    id: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

#[derive(Debug)]
struct LoadCodeAssistStatus {
    tier_id: Option<String>,
    tier_name: Option<String>,
    project_id: Option<String>,
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn now_ts_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
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

fn parse_i64_from_value(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn parse_jwt_claim_string(token: &str, key: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let payload_b64 = parts[1].replace('-', "+").replace('_', "/");
    let padded = match payload_b64.len() % 4 {
        2 => format!("{}==", payload_b64),
        3 => format!("{}=", payload_b64),
        _ => payload_b64,
    };

    let payload = base64::engine::general_purpose::STANDARD
        .decode(padded)
        .ok()?;
    let value: Value = serde_json::from_slice(&payload).ok()?;
    normalize_non_empty(value.get(key).and_then(|item| item.as_str()))
}

fn merge_local_oauth_creds(
    primary: LocalOauthCreds,
    fallback: Option<LocalOauthCreds>,
) -> LocalOauthCreds {
    let Some(fallback) = fallback else {
        return primary;
    };

    LocalOauthCreds {
        access_token: primary.access_token.or(fallback.access_token),
        refresh_token: primary.refresh_token.or(fallback.refresh_token),
        id_token: primary.id_token.or(fallback.id_token),
        token_type: primary.token_type.or(fallback.token_type),
        scope: primary.scope.or(fallback.scope),
        expiry_date: primary.expiry_date.or(fallback.expiry_date),
    }
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Gemini 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(get_accounts_index_path()?.to_string_lossy().to_string())
}

fn get_default_gemini_home() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    Ok(home.join(GEMINI_HOME_DIR))
}

fn resolve_gemini_home(cli_home_root: Option<&Path>) -> Result<PathBuf, String> {
    match cli_home_root {
        Some(root) => {
            let trimmed = root.to_string_lossy().trim().to_string();
            if trimmed.is_empty() {
                return Err("Gemini Cli HOME 目录不能为空".to_string());
            }
            Ok(PathBuf::from(trimmed).join(GEMINI_HOME_DIR))
        }
        None => get_default_gemini_home(),
    }
}

fn get_local_oauth_creds_path_for(cli_home_root: Option<&Path>) -> Result<PathBuf, String> {
    Ok(resolve_gemini_home(cli_home_root)?.join(GEMINI_OAUTH_FILE))
}

fn get_local_google_accounts_path_for(cli_home_root: Option<&Path>) -> Result<PathBuf, String> {
    Ok(resolve_gemini_home(cli_home_root)?.join(GEMINI_GOOGLE_ACCOUNTS_FILE))
}

fn get_local_settings_path_for(cli_home_root: Option<&Path>) -> Result<PathBuf, String> {
    Ok(resolve_gemini_home(cli_home_root)?.join(GEMINI_SETTINGS_FILE))
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

fn load_account_file(account_id: &str) -> Option<GeminiAccount> {
    let path = resolve_account_file_path(account_id).ok()?;
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&path, &content).ok()
}

pub fn load_account(account_id: &str) -> Option<GeminiAccount> {
    load_account_file(account_id)
}

fn save_account_file(account: &GeminiAccount) -> Result<(), String> {
    let path = resolve_account_file_path(&account.id)?;
    let content = serde_json::to_string_pretty(account)
        .map_err(|e| format!("序列化 Gemini 账号失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存 Gemini 账号失败: {}", e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("删除 Gemini 账号文件失败: {}", e))?;
    }
    Ok(())
}

fn load_account_index() -> GeminiAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(path) => path,
        Err(_) => return GeminiAccountIndex::new(),
    };

    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(GeminiAccountIndex::new);
    }

    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空")
                .unwrap_or_else(GeminiAccountIndex::new)
        }
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            GeminiAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(GeminiAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Gemini Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(GeminiAccountIndex::new)
            }
        },
        Err(_) => GeminiAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<GeminiAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(GeminiAccountIndex::new());
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
        return Ok(GeminiAccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<GeminiAccountIndex>(
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

fn save_account_index(index: &GeminiAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content = serde_json::to_string_pretty(index)
        .map_err(|e| format!("序列化 Gemini 账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入 Gemini 账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<GeminiAccountIndex> {
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

    let mut index = GeminiAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Gemini Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Gemini Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_warn(&format!(
        "[Gemini Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut GeminiAccountIndex, account: &GeminiAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: GeminiAccount) -> Result<GeminiAccount, String> {
    let _lock = GEMINI_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Gemini 账号锁失败".to_string())?;
    let mut index = load_account_index();
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
}

fn persist_quota_query_error(account_id: &str, message: &str) {
    let Some(mut account) = load_account_file(account_id) else {
        return;
    };
    account.quota_query_last_error = Some(message.to_string());
    account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    let _ = upsert_account_record(account);
}

fn build_account_id(email: &str, auth_id: Option<&str>) -> String {
    let mut seed = email.trim().to_lowercase();
    if let Some(auth_id) = normalize_non_empty(auth_id) {
        seed.push('|');
        seed.push_str(&auth_id);
    }
    format!("gemini_{:x}", md5::compute(seed.as_bytes()))
}

pub fn list_accounts() -> Vec<GeminiAccount> {
    let index = load_account_index();
    index
        .accounts
        .iter()
        .filter_map(|summary| load_account_file(&summary.id))
        .collect()
}

pub fn list_accounts_checked() -> Result<Vec<GeminiAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(index
        .accounts
        .iter()
        .filter_map(|summary| load_account_file(&summary.id))
        .collect())
}

pub fn upsert_account(payload: GeminiOAuthCompletePayload) -> Result<GeminiAccount, String> {
    let _lock = GEMINI_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Gemini 账号锁失败".to_string())?;

    let mut index = load_account_index();
    let now = now_ts();

    let generated_id = build_account_id(&payload.email, payload.auth_id.as_deref());
    let existing_id = index
        .accounts
        .iter()
        .filter_map(|item| load_account_file(&item.id).map(|account| (item.id.clone(), account)))
        .find(|(_, account)| {
            let same_auth = normalize_non_empty(account.auth_id.as_deref())
                == normalize_non_empty(payload.auth_id.as_deref());
            let same_email = account.email.eq_ignore_ascii_case(&payload.email);
            same_auth || same_email
        })
        .map(|(id, _)| id)
        .unwrap_or(generated_id);

    let existing = load_account_file(&existing_id);
    let created_at = existing.as_ref().map(|item| item.created_at).unwrap_or(now);
    let tags = existing.as_ref().and_then(|item| item.tags.clone());

    let account = GeminiAccount {
        id: existing_id,
        email: payload.email,
        auth_id: payload.auth_id,
        name: payload.name,
        tags,
        access_token: payload.access_token,
        refresh_token: payload.refresh_token,
        id_token: payload.id_token,
        token_type: payload.token_type,
        scope: payload.scope,
        expiry_date: payload.expiry_date,
        selected_auth_type: payload.selected_auth_type,
        project_id: payload.project_id,
        tier_id: payload.tier_id,
        plan_name: payload.plan_name,
        gemini_auth_raw: payload.gemini_auth_raw,
        gemini_usage_raw: payload.gemini_usage_raw,
        status: payload.status,
        status_reason: payload.status_reason,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: existing.as_ref().and_then(|item| item.usage_updated_at),
        created_at,
        last_used: now,
    };

    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;

    logger::log_info(&format!(
        "[Gemini Account] 账号已保存: id={}, email={}",
        account.id, account.email
    ));

    Ok(account)
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = GEMINI_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Gemini 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| item.id != account_id);
    save_account_index(&index)?;
    delete_account_file(account_id)?;
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for account_id in account_ids {
        remove_account(account_id)?;
    }
    Ok(())
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<GeminiAccount, String> {
    let mut account =
        load_account_file(account_id).ok_or_else(|| "Gemini 账号不存在".to_string())?;
    account.tags = Some(tags);
    account.last_used = now_ts();
    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

pub fn set_account_status(
    account_id: &str,
    status: Option<&str>,
    reason: Option<&str>,
) -> Result<(), String> {
    let mut account =
        load_account_file(account_id).ok_or_else(|| "Gemini 账号不存在".to_string())?;
    account.status = resolve_account_status(status, reason);
    account.status_reason = reason.and_then(|r| normalize_non_empty(Some(r)));
    upsert_account_record(account)?;
    Ok(())
}

fn parse_gemini_account_from_json_object(
    value: &Value,
) -> Result<GeminiOAuthCompletePayload, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "JSON 必须为对象".to_string())?;

    let access_token = obj
        .get("access_token")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("accessToken").and_then(|v| v.as_str()))
        .ok_or_else(|| "JSON 缺少 access_token".to_string())?
        .to_string();

    let refresh_token = normalize_non_empty(
        obj.get("refresh_token")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("refreshToken").and_then(|v| v.as_str())),
    );
    let id_token = normalize_non_empty(
        obj.get("id_token")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("idToken").and_then(|v| v.as_str())),
    );
    let id_token_email = id_token
        .as_deref()
        .and_then(|token| parse_jwt_claim_string(token, "email"));
    let id_token_sub = id_token
        .as_deref()
        .and_then(|token| parse_jwt_claim_string(token, "sub"));
    let email = normalize_non_empty(
        obj.get("email")
            .and_then(|v| v.as_str())
            .or(id_token_email.as_deref()),
    )
    .unwrap_or_else(|| "unknown@gmail.com".to_string());

    let auth_id = normalize_non_empty(
        obj.get("auth_id")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("sub").and_then(|v| v.as_str()))
            .or(id_token_sub.as_deref()),
    );

    let expiry_date =
        parse_i64_from_value(obj.get("expiry_date").or_else(|| obj.get("expiryDate")));

    Ok(GeminiOAuthCompletePayload {
        email,
        auth_id,
        name: normalize_non_empty(obj.get("name").and_then(|v| v.as_str())),
        access_token,
        refresh_token,
        id_token,
        token_type: normalize_non_empty(
            obj.get("token_type")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("tokenType").and_then(|v| v.as_str())),
        ),
        scope: normalize_non_empty(obj.get("scope").and_then(|v| v.as_str())),
        expiry_date,
        selected_auth_type: normalize_non_empty(
            obj.get("selected_auth_type")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("selectedAuthType").and_then(|v| v.as_str())),
        )
        .or_else(|| Some("oauth-personal".to_string())),
        project_id: normalize_non_empty(
            obj.get("project_id")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("projectId").and_then(|v| v.as_str())),
        ),
        tier_id: normalize_non_empty(
            obj.get("tier_id")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("tierId").and_then(|v| v.as_str())),
        ),
        plan_name: normalize_non_empty(
            obj.get("plan_name")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("planName").and_then(|v| v.as_str())),
        ),
        gemini_auth_raw: Some(value.clone()),
        gemini_usage_raw: obj.get("gemini_usage_raw").cloned(),
        status: normalize_non_empty(obj.get("status").and_then(|v| v.as_str())),
        status_reason: normalize_non_empty(obj.get("status_reason").and_then(|v| v.as_str())),
    })
}

pub fn import_from_json(json_content: &str) -> Result<Vec<GeminiAccount>, String> {
    if let Ok(account) = serde_json::from_str::<GeminiAccount>(json_content) {
        let saved = upsert_account_record(account)?;
        return Ok(vec![saved]);
    }

    if let Ok(accounts) = serde_json::from_str::<Vec<GeminiAccount>>(json_content) {
        let mut saved = Vec::with_capacity(accounts.len());
        for account in accounts {
            saved.push(upsert_account_record(account)?);
        }
        return Ok(saved);
    }

    let value: Value =
        serde_json::from_str(json_content).map_err(|e| format!("解析 JSON 失败: {}", e))?;

    if let Some(arr) = value.as_array() {
        let mut imported = Vec::with_capacity(arr.len());
        for item in arr {
            let payload = parse_gemini_account_from_json_object(item)?;
            imported.push(upsert_account(payload)?);
        }
        return Ok(imported);
    }

    let payload = parse_gemini_account_from_json_object(&value)?;
    let account = upsert_account(payload)?;
    Ok(vec![account])
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<GeminiAccount> = account_ids
        .iter()
        .filter_map(|id| load_account_file(id))
        .collect();

    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化导出 JSON 失败: {}", e))
}

fn read_local_oauth_creds_from_path(
    cli_home_root: Option<&Path>,
) -> Result<Option<LocalOauthCreds>, String> {
    let path = get_local_oauth_creds_path_for(cli_home_root)?;
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| format!("读取本地 Gemini oauth_creds.json 失败: {}", e))?;
    let creds = serde_json::from_str::<LocalOauthCreds>(&content)
        .map_err(|e| format!("解析本地 Gemini oauth_creds.json 失败: {}", e))?;

    Ok(Some(creds))
}

#[cfg(target_os = "macos")]
fn is_macos_default_keychain_available() -> bool {
    let output = match std::process::Command::new("security")
        .arg("default-keychain")
        .output()
    {
        Ok(output) => output,
        Err(_) => return false,
    };

    if !output.status.success() {
        return false;
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return false;
    }

    let normalized = raw.trim_matches('"');
    !normalized.trim().is_empty() && Path::new(normalized).exists()
}

#[cfg(not(target_os = "macos"))]
fn is_macos_default_keychain_available() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn read_local_oauth_creds_from_keychain() -> Result<Option<LocalOauthCreds>, String> {
    if !is_macos_default_keychain_available() {
        return Ok(None);
    }

    let output = std::process::Command::new("security")
        .arg("find-generic-password")
        .arg("-s")
        .arg(GEMINI_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(GEMINI_KEYCHAIN_ACCOUNT)
        .arg("-w")
        .output()
        .map_err(|e| format!("执行 security 读取 Gemini keychain 失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("could not be found") {
            return Ok(None);
        }
        return Err(format!(
            "读取 Gemini keychain 凭据失败: status={}, stderr={}",
            output.status,
            stderr.trim()
        ));
    }

    let secret = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if secret.is_empty() {
        return Ok(None);
    }

    let parsed = serde_json::from_str::<LocalKeychainOauthCreds>(&secret)
        .map_err(|e| format!("解析 Gemini keychain 凭据失败: {}", e))?;
    let token = parsed
        .token
        .ok_or_else(|| "Gemini keychain 凭据缺少 token 字段".to_string())?;

    Ok(Some(LocalOauthCreds {
        access_token: normalize_non_empty(token.access_token.as_deref()),
        refresh_token: normalize_non_empty(token.refresh_token.as_deref()),
        id_token: None,
        token_type: normalize_non_empty(token.token_type.as_deref()),
        scope: normalize_non_empty(token.scope.as_deref()),
        expiry_date: token.expires_at,
    }))
}

#[cfg(not(target_os = "macos"))]
fn read_local_oauth_creds_from_keychain() -> Result<Option<LocalOauthCreds>, String> {
    Ok(None)
}

fn read_local_oauth_creds() -> Result<Option<LocalOauthCreds>, String> {
    let keychain_creds = read_local_oauth_creds_from_keychain()?;
    let file_creds = read_local_oauth_creds_from_path(None)?;

    match (keychain_creds, file_creds) {
        (Some(primary), fallback) => Ok(Some(merge_local_oauth_creds(primary, fallback))),
        (None, fallback) => Ok(fallback),
    }
}

fn read_local_google_accounts_from_path(
    cli_home_root: Option<&Path>,
) -> Result<LocalGoogleAccounts, String> {
    let path = get_local_google_accounts_path_for(cli_home_root)?;
    if !path.exists() {
        return Ok(LocalGoogleAccounts::default());
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| format!("读取本地 Gemini google_accounts.json 失败: {}", e))?;
    let data = serde_json::from_str::<LocalGoogleAccounts>(&content)
        .unwrap_or_else(|_| LocalGoogleAccounts::default());
    Ok(data)
}

fn read_local_google_accounts() -> Result<LocalGoogleAccounts, String> {
    read_local_google_accounts_from_path(None)
}

fn write_local_google_accounts_to_path(
    accounts: &LocalGoogleAccounts,
    cli_home_root: Option<&Path>,
) -> Result<(), String> {
    let path = get_local_google_accounts_path_for(cli_home_root)?;
    let home = resolve_gemini_home(cli_home_root)?;
    if !home.exists() {
        fs::create_dir_all(&home).map_err(|e| format!("创建本地 Gemini 目录失败: {}", e))?;
    }

    let content = serde_json::to_string_pretty(accounts)
        .map_err(|e| format!("序列化本地 Gemini google_accounts.json 失败: {}", e))?;
    fs::write(path, content)
        .map_err(|e| format!("写入本地 Gemini google_accounts.json 失败: {}", e))
}

fn read_local_selected_auth_type_from_path(
    cli_home_root: Option<&Path>,
) -> Result<Option<String>, String> {
    let path = get_local_settings_path_for(cli_home_root)?;
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取本地 Gemini settings.json 失败: {}", e))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析本地 Gemini settings.json 失败: {}", e))?;

    Ok(normalize_non_empty(
        value
            .get("security")
            .and_then(|v| v.get("auth"))
            .and_then(|v| v.get("selectedType"))
            .and_then(|v| v.as_str()),
    ))
}

fn read_local_selected_auth_type() -> Result<Option<String>, String> {
    read_local_selected_auth_type_from_path(None)
}

fn write_local_selected_auth_type_to_path(
    selected_type: &str,
    cli_home_root: Option<&Path>,
) -> Result<(), String> {
    let path = get_local_settings_path_for(cli_home_root)?;
    let home = resolve_gemini_home(cli_home_root)?;
    if !home.exists() {
        fs::create_dir_all(&home).map_err(|e| format!("创建本地 Gemini 目录失败: {}", e))?;
    }

    let mut root_value = if path.exists() {
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("读取本地 Gemini settings.json 失败: {}", e))?;
        serde_json::from_str::<Value>(&content)
            .unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
    } else {
        Value::Object(serde_json::Map::new())
    };

    if !root_value.is_object() {
        root_value = Value::Object(serde_json::Map::new());
    }

    let root = root_value
        .as_object_mut()
        .ok_or_else(|| "本地 Gemini settings.json 根结构非法".to_string())?;

    let security = root
        .entry("security")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !security.is_object() {
        *security = Value::Object(serde_json::Map::new());
    }

    let security_obj = security
        .as_object_mut()
        .ok_or_else(|| "本地 Gemini settings.json.security 结构非法".to_string())?;

    let auth = security_obj
        .entry("auth")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !auth.is_object() {
        *auth = Value::Object(serde_json::Map::new());
    }

    let auth_obj = auth
        .as_object_mut()
        .ok_or_else(|| "本地 Gemini settings.json.security.auth 结构非法".to_string())?;

    auth_obj.insert(
        "selectedType".to_string(),
        Value::String(selected_type.to_string()),
    );

    let content = serde_json::to_string_pretty(&root_value)
        .map_err(|e| format!("序列化本地 Gemini settings.json 失败: {}", e))?;
    fs::write(path, content).map_err(|e| format!("写入本地 Gemini settings.json 失败: {}", e))
}

pub fn get_local_active_email() -> Option<String> {
    read_local_google_accounts()
        .ok()
        .and_then(|data| normalize_non_empty(data.active.as_deref()))
}

pub fn import_from_local() -> Result<Option<GeminiAccount>, String> {
    let local_creds = match read_local_oauth_creds()? {
        Some(creds) => creds,
        None => return Ok(None),
    };

    let access_token = normalize_non_empty(local_creds.access_token.as_deref())
        .ok_or_else(|| "本地 Gemini 凭据缺少 access_token".to_string())?;

    let active_email = read_local_google_accounts()?
        .active
        .and_then(|email| normalize_non_empty(Some(email.as_str())));

    let id_token_email = local_creds
        .id_token
        .as_deref()
        .and_then(|token| parse_jwt_claim_string(token, "email"));

    let email = active_email
        .or(id_token_email)
        .unwrap_or_else(|| "unknown@gmail.com".to_string());

    let auth_id = local_creds
        .id_token
        .as_deref()
        .and_then(|token| parse_jwt_claim_string(token, "sub"));

    let payload = GeminiOAuthCompletePayload {
        email,
        auth_id,
        name: local_creds
            .id_token
            .as_deref()
            .and_then(|token| parse_jwt_claim_string(token, "name")),
        access_token,
        refresh_token: normalize_non_empty(local_creds.refresh_token.as_deref()),
        id_token: normalize_non_empty(local_creds.id_token.as_deref()),
        token_type: normalize_non_empty(local_creds.token_type.as_deref()),
        scope: normalize_non_empty(local_creds.scope.as_deref()),
        expiry_date: parse_i64_from_value(local_creds.expiry_date.as_ref()),
        selected_auth_type: read_local_selected_auth_type()?
            .or_else(|| Some("oauth-personal".to_string())),
        project_id: None,
        tier_id: None,
        plan_name: None,
        gemini_auth_raw: None,
        gemini_usage_raw: None,
        status: None,
        status_reason: None,
    };

    let account = upsert_account(payload)?;

    logger::log_info(&format!(
        "[Gemini Account] 从本地导入成功: id={}, email={}",
        account.id, account.email
    ));

    Ok(Some(account))
}

fn write_local_oauth_creds_to_path(
    account: &GeminiAccount,
    cli_home_root: Option<&Path>,
) -> Result<(), String> {
    let path = get_local_oauth_creds_path_for(cli_home_root)?;
    let home = resolve_gemini_home(cli_home_root)?;
    if !home.exists() {
        fs::create_dir_all(&home).map_err(|e| format!("创建本地 Gemini 目录失败: {}", e))?;
    }

    let mut payload = serde_json::Map::new();
    payload.insert(
        "access_token".to_string(),
        Value::String(account.access_token.clone()),
    );

    if let Some(refresh_token) = account.refresh_token.as_ref() {
        payload.insert(
            "refresh_token".to_string(),
            Value::String(refresh_token.clone()),
        );
    }
    if let Some(id_token) = account.id_token.as_ref() {
        payload.insert("id_token".to_string(), Value::String(id_token.clone()));
    }
    if let Some(token_type) = account.token_type.as_ref() {
        payload.insert("token_type".to_string(), Value::String(token_type.clone()));
    }
    if let Some(scope) = account.scope.as_ref() {
        payload.insert("scope".to_string(), Value::String(scope.clone()));
    }
    if let Some(expiry_date) = account.expiry_date {
        payload.insert("expiry_date".to_string(), serde_json::json!(expiry_date));
    }

    let content = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("序列化本地 Gemini oauth_creds.json 失败: {}", e))?;
    fs::write(path, content).map_err(|e| format!("写入本地 Gemini oauth_creds.json 失败: {}", e))
}

#[cfg(target_os = "macos")]
fn write_local_oauth_creds_to_keychain(account: &GeminiAccount) -> Result<(), String> {
    if !is_macos_default_keychain_available() {
        logger::log_info("[Gemini Switch] 未检测到可用 macOS keychain，跳过 keychain 写入");
        return Ok(());
    }

    let access_token = normalize_non_empty(Some(account.access_token.as_str()))
        .ok_or_else(|| "Gemini keychain 写入失败: access_token 为空".to_string())?;
    let token_type =
        normalize_non_empty(account.token_type.as_deref()).unwrap_or_else(|| "Bearer".to_string());

    let mut token = serde_json::Map::new();
    token.insert("accessToken".to_string(), Value::String(access_token));
    token.insert("tokenType".to_string(), Value::String(token_type));
    if let Some(refresh_token) = normalize_non_empty(account.refresh_token.as_deref()) {
        token.insert("refreshToken".to_string(), Value::String(refresh_token));
    }
    if let Some(scope) = normalize_non_empty(account.scope.as_deref()) {
        token.insert("scope".to_string(), Value::String(scope));
    }
    if let Some(expires_at) = account.expiry_date {
        token.insert("expiresAt".to_string(), serde_json::json!(expires_at));
    }

    let payload = serde_json::json!({
        "serverName": GEMINI_KEYCHAIN_ACCOUNT,
        "token": token,
        "updatedAt": now_ts_ms(),
    });
    let secret = serde_json::to_string(&payload)
        .map_err(|e| format!("序列化 Gemini keychain 凭据失败: {}", e))?;

    let output = std::process::Command::new("security")
        .arg("add-generic-password")
        .arg("-U")
        .arg("-s")
        .arg(GEMINI_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(GEMINI_KEYCHAIN_ACCOUNT)
        .arg("-w")
        .arg(&secret)
        .output()
        .map_err(|e| format!("执行 security 写入 Gemini keychain 失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "写入 Gemini keychain 失败: status={}, stderr={}, stdout={}",
            output.status,
            if stderr.trim().is_empty() {
                "<empty>"
            } else {
                stderr.trim()
            },
            if stdout.trim().is_empty() {
                "<empty>"
            } else {
                stdout.trim()
            }
        ));
    }

    logger::log_info(&format!(
        "[Gemini Switch] 已更新 keychain 登录信息: service={}, account={}",
        GEMINI_KEYCHAIN_SERVICE, GEMINI_KEYCHAIN_ACCOUNT
    ));
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn write_local_oauth_creds_to_keychain(_account: &GeminiAccount) -> Result<(), String> {
    Ok(())
}

fn clear_local_file_keychain_to_path(cli_home_root: Option<&Path>) -> Result<(), String> {
    let path = resolve_gemini_home(cli_home_root)?.join(GEMINI_FILE_KEYCHAIN_FILE);
    if !path.exists() {
        return Ok(());
    }

    fs::remove_file(&path).map_err(|e| {
        format!(
            "清理 Gemini file keychain 失败: path={}, error={}",
            path.display(),
            e
        )
    })
}

fn update_local_active_account_with_path(
    email: &str,
    cli_home_root: Option<&Path>,
) -> Result<(), String> {
    let mut accounts = read_local_google_accounts_from_path(cli_home_root)?;

    if let Some(active) = normalize_non_empty(accounts.active.as_deref()) {
        if !active.eq_ignore_ascii_case(email)
            && !accounts
                .old
                .iter()
                .any(|item| item.eq_ignore_ascii_case(&active))
        {
            accounts.old.push(active);
        }
    }

    accounts.old = accounts
        .old
        .into_iter()
        .filter(|item| !item.eq_ignore_ascii_case(email))
        .collect();
    accounts.active = Some(email.to_string());

    write_local_google_accounts_to_path(&accounts, cli_home_root)
}

fn parse_tier_plan_name(tier_id: Option<&str>) -> Option<String> {
    match tier_id {
        Some(raw) if !raw.trim().is_empty() => Some(raw.trim().to_string()),
        _ => None,
    }
}

fn is_unauthorized_error(error: &str) -> bool {
    error.contains("UNAUTHORIZED") || error.contains("401")
}

fn is_forbidden_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("status=403")
        || lower.contains("403 forbidden")
        || lower.contains("\"code\":403")
        || lower.contains("\"code\": 403")
        || lower.contains("permission_denied")
        || lower.contains("caller does not have permission")
        || lower.contains("forbidden")
}

fn resolve_account_status(status: Option<&str>, reason: Option<&str>) -> Option<String> {
    if reason.map(is_forbidden_error).unwrap_or(false) {
        return Some("forbidden".to_string());
    }

    normalize_non_empty(status).map(|value| value.to_ascii_lowercase())
}

fn should_retry_code_assist_status(status: reqwest::StatusCode) -> bool {
    let code = status.as_u16();
    code == 429 || code == 499 || code >= 500
}

async fn post_code_assist_json_with_retry(
    access_token: &str,
    endpoint: &str,
    payload: &Value,
    action_name: &str,
) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut attempt = 0usize;
    loop {
        attempt += 1;
        let response = client
            .post(endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .header(CONTENT_TYPE, "application/json")
            .json(payload)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().as_u16() == 401 {
                    return Err("UNAUTHORIZED: Gemini access_token 已失效".to_string());
                }

                if resp.status().is_success() {
                    return resp
                        .json::<Value>()
                        .await
                        .map_err(|e| format!("解析 Gemini {} 响应失败: {}", action_name, e));
                }

                let status = resp.status();
                if should_retry_code_assist_status(status)
                    && attempt <= CODE_ASSIST_REQUEST_RETRY_COUNT
                {
                    logger::log_warn(&format!(
                        "[Gemini {}] 请求失败将重试: attempt={}/{}, status={}",
                        action_name,
                        attempt,
                        CODE_ASSIST_REQUEST_RETRY_COUNT + 1,
                        status
                    ));
                    tokio::time::sleep(std::time::Duration::from_millis(
                        CODE_ASSIST_REQUEST_RETRY_DELAY_MS,
                    ))
                    .await;
                    continue;
                }

                let body = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "<empty-body>".to_string());
                return Err(format!(
                    "请求 Gemini {} 失败: status={}, body_len={}",
                    action_name,
                    status,
                    body.len()
                ));
            }
            Err(err) => {
                if attempt <= CODE_ASSIST_REQUEST_RETRY_COUNT {
                    logger::log_warn(&format!(
                        "[Gemini {}] 请求异常将重试: attempt={}/{}, error={}",
                        action_name,
                        attempt,
                        CODE_ASSIST_REQUEST_RETRY_COUNT + 1,
                        err
                    ));
                    tokio::time::sleep(std::time::Duration::from_millis(
                        CODE_ASSIST_REQUEST_RETRY_DELAY_MS,
                    ))
                    .await;
                    continue;
                }
                return Err(format!("请求 Gemini {} 失败: {}", action_name, err));
            }
        }
    }
}

async fn refresh_access_token(refresh_token: &str) -> Result<GoogleTokenRefreshResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client
        .post(GOOGLE_TOKEN_ENDPOINT)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[
            ("client_id", gemini_oauth::gemini_oauth_client_id()),
            ("client_secret", gemini_oauth::gemini_oauth_client_secret()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| format!("刷新 Gemini access_token 请求失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<empty-body>".to_string());
        return Err(format!(
            "刷新 Gemini access_token 失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    let payload = response
        .json::<GoogleTokenRefreshResponse>()
        .await
        .map_err(|e| format!("解析 Gemini access_token 刷新响应失败: {}", e))?;

    if payload.access_token.is_none() {
        return Err(format!(
            "刷新 Gemini access_token 响应异常: error={:?}, desc={:?}",
            payload.error, payload.error_description
        ));
    }

    Ok(payload)
}

async fn fetch_google_userinfo(access_token: &str) -> Option<GoogleUserInfoResponse> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;
    let response = client
        .get(GOOGLE_USERINFO_ENDPOINT)
        .header(AUTHORIZATION, format!("Bearer {}", access_token))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    response.json::<GoogleUserInfoResponse>().await.ok()
}

async fn load_code_assist_status(access_token: &str) -> Result<LoadCodeAssistStatus, String> {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "ideType".to_string(),
        Value::String("IDE_UNSPECIFIED".to_string()),
    );
    metadata.insert(
        "platform".to_string(),
        Value::String("PLATFORM_UNSPECIFIED".to_string()),
    );
    metadata.insert(
        "pluginType".to_string(),
        Value::String("GEMINI".to_string()),
    );

    let mut payload = serde_json::Map::new();
    payload.insert("metadata".to_string(), Value::Object(metadata));

    let value = post_code_assist_json_with_retry(
        access_token,
        CODE_ASSIST_LOAD_ENDPOINT,
        &Value::Object(payload),
        "loadCodeAssist",
    )
    .await?;

    let current_tier_id = normalize_non_empty(
        value
            .get("currentTier")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str()),
    );
    let current_tier_name = normalize_non_empty(
        value
            .get("currentTier")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str()),
    );
    let current_tier_quota_tier = normalize_non_empty(
        value
            .get("currentTier")
            .and_then(|v| v.get("quotaTier"))
            .and_then(|v| v.as_str()),
    );
    let paid_tier_id = normalize_non_empty(
        value
            .get("paidTier")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str()),
    );
    let paid_tier_name = normalize_non_empty(
        value
            .get("paidTier")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str()),
    );
    let paid_tier_quota_tier = normalize_non_empty(
        value
            .get("paidTier")
            .and_then(|v| v.get("quotaTier"))
            .and_then(|v| v.as_str()),
    );
    let default_allowed_tier_id = value
        .get("allowedTiers")
        .and_then(|v| v.as_array())
        .and_then(|tiers| {
            tiers.iter().find(|tier| {
                tier.get("isDefault")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
        })
        .and_then(|tier| normalize_non_empty(tier.get("id").and_then(|v| v.as_str())));
    let allowed_tier_ids = value
        .get("allowedTiers")
        .and_then(|v| v.as_array())
        .map(|tiers| {
            tiers
                .iter()
                .filter_map(|tier| tier.get("id").and_then(|v| v.as_str()))
                .filter_map(|id| normalize_non_empty(Some(id)))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    let first_allowed_tier_id = allowed_tier_ids.first().cloned();
    let ineligible_tier_id = value
        .get("ineligibleTiers")
        .and_then(|v| v.as_array())
        .and_then(|tiers| tiers.first())
        .and_then(|tier| normalize_non_empty(tier.get("tierId").and_then(|v| v.as_str())));
    let selected_tier_id = paid_tier_id
        .clone()
        .or_else(|| current_tier_id.clone())
        .or_else(|| ineligible_tier_id.clone())
        .or_else(|| default_allowed_tier_id.clone())
        .or_else(|| first_allowed_tier_id.clone());
    let selected_tier_name = paid_tier_name.clone().or_else(|| current_tier_name.clone());

    let project_id = normalize_non_empty(
        value
            .get("cloudaicompanionProject")
            .and_then(|v| v.as_str())
            .or_else(|| {
                value
                    .get("cloudaicompanionProject")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str())
            })
            .or_else(|| {
                value
                    .get("cloudaicompanionProject")
                    .and_then(|v| v.get("projectId"))
                    .and_then(|v| v.as_str())
            }),
    );

    let top_level_keys = value
        .as_object()
        .map(|obj| {
            obj.keys()
                .map(|key| key.as_str())
                .collect::<Vec<&str>>()
                .join(",")
        })
        .unwrap_or_else(|| "<non-object>".to_string());

    logger::log_info(&format!(
        "[Gemini loadCodeAssist] keys=[{}], currentTier.id={:?}, currentTier.name={:?}, currentTier.quotaTier={:?}, paidTier.id={:?}, paidTier.name={:?}, paidTier.quotaTier={:?}, defaultAllowedTierId={:?}, firstAllowedTierId={:?}, ineligibleTierId={:?}, allowedTierIds=[{}], selectedTierId={:?}, projectId={:?}",
        top_level_keys,
        current_tier_id,
        current_tier_name,
        current_tier_quota_tier,
        paid_tier_id,
        paid_tier_name,
        paid_tier_quota_tier,
        default_allowed_tier_id,
        first_allowed_tier_id,
        ineligible_tier_id,
        allowed_tier_ids.join(","),
        selected_tier_id,
        project_id
    ));

    Ok(LoadCodeAssistStatus {
        tier_id: selected_tier_id,
        tier_name: selected_tier_name,
        project_id,
    })
}

async fn retrieve_user_quota(access_token: &str, project_id: &str) -> Result<Value, String> {
    let payload = serde_json::json!({
        "project": project_id
    });
    post_code_assist_json_with_retry(
        access_token,
        CODE_ASSIST_RETRIEVE_QUOTA_ENDPOINT,
        &payload,
        "retrieveUserQuota",
    )
    .await
}

async fn ensure_access_token_valid(account: &mut GeminiAccount) -> Result<(), String> {
    let should_refresh = account
        .expiry_date
        .map(|expiry| expiry <= now_ts_ms() + 60_000)
        .unwrap_or(false);

    if !should_refresh {
        return Ok(());
    }

    force_refresh_access_token(account).await
}

async fn force_refresh_access_token(account: &mut GeminiAccount) -> Result<(), String> {
    let refresh_token = account
        .refresh_token
        .clone()
        .ok_or_else(|| "Gemini refresh_token 不存在，无法刷新 access_token".to_string())?;

    let refreshed = refresh_access_token(&refresh_token).await?;
    account.access_token = refreshed
        .access_token
        .ok_or_else(|| "Gemini token 刷新后 access_token 为空".to_string())?;
    if let Some(id_token) = refreshed.id_token {
        account.id_token = Some(id_token);
    }
    if let Some(token_type) = refreshed.token_type {
        account.token_type = Some(token_type);
    }
    if let Some(scope) = refreshed.scope {
        account.scope = Some(scope);
    }
    if let Some(expires_in) = refreshed.expires_in {
        account.expiry_date = Some(now_ts_ms() + expires_in.saturating_mul(1000));
    }

    Ok(())
}

async fn refresh_account_token_once(account_id: &str) -> Result<GeminiAccount, String> {
    let mut account =
        load_account_file(account_id).ok_or_else(|| "Gemini 账号不存在".to_string())?;

    ensure_access_token_valid(&mut account).await?;

    let mut load_status = load_code_assist_status(&account.access_token).await;
    if let Err(err) = &load_status {
        if is_unauthorized_error(err) {
            force_refresh_access_token(&mut account).await?;
            load_status = load_code_assist_status(&account.access_token).await;
        }
    }
    let load_status = load_status?;

    let project_id = load_status.project_id.clone();

    if let Some(userinfo) = fetch_google_userinfo(&account.access_token).await {
        if let Some(email) = normalize_non_empty(userinfo.email.as_deref()) {
            account.email = email;
        }
        if account.auth_id.is_none() {
            account.auth_id = normalize_non_empty(userinfo.id.as_deref());
        }
        if account.name.is_none() {
            account.name = normalize_non_empty(userinfo.name.as_deref());
        }
    }

    if account.email.trim().is_empty() {
        if let Some(email) = account
            .id_token
            .as_deref()
            .and_then(|token| parse_jwt_claim_string(token, "email"))
        {
            account.email = email;
        }
    }

    if account.auth_id.is_none() {
        account.auth_id = account
            .id_token
            .as_deref()
            .and_then(|token| parse_jwt_claim_string(token, "sub"));
    }

    account.project_id = project_id;
    if let Some(tier_id) = load_status.tier_id {
        account.tier_id = Some(tier_id);
    }
    account.plan_name = load_status
        .tier_name
        .or_else(|| parse_tier_plan_name(account.tier_id.as_deref()));
    account.selected_auth_type = account
        .selected_auth_type
        .clone()
        .or_else(|| Some("oauth-personal".to_string()));
    let refreshed_at = now_ts();
    account.last_used = refreshed_at;

    let mut status: Option<String> = None;
    let mut status_reason: Option<String> = None;
    if let Some(project_id) = account.project_id.as_deref() {
        match retrieve_user_quota(&account.access_token, project_id).await {
            Ok(quota) => {
                account.gemini_usage_raw = Some(quota);
                account.quota_query_last_error = None;
                account.quota_query_last_error_at = None;
                account.usage_updated_at = Some(refreshed_at);
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[Gemini retrieveUserQuota] 刷新配额失败: account_id={}, project_id={}, error={}",
                    account.id, project_id, err
                ));
                account.quota_query_last_error = Some(err.clone());
                account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
                if is_forbidden_error(&err) {
                    account.gemini_usage_raw = None;
                    account.usage_updated_at = None;
                    status = Some("forbidden".to_string());
                    status_reason = Some(err);
                }
            }
        }
    } else {
        account.gemini_usage_raw = None;
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
        account.usage_updated_at = None;
    }

    account.status = status;
    account.status_reason = status_reason;

    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

pub async fn refresh_account_token(account_id: &str) -> Result<GeminiAccount, String> {
    let result = crate::modules::refresh_retry::retry_once_with_delay(
        "Gemini Refresh",
        account_id,
        || async { refresh_account_token_once(account_id).await },
    )
    .await;
    if let Err(err) = &result {
        persist_quota_query_error(account_id, err);
    }
    result
}

pub async fn refresh_all_tokens() -> Result<Vec<(String, Result<GeminiAccount, String>)>, String> {
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    const MAX_CONCURRENT: usize = 4;
    let accounts = list_accounts();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let tasks: Vec<_> = accounts
        .into_iter()
        .map(|account| {
            let account_id = account.id;
            let semaphore = semaphore.clone();
            async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|e| format!("获取 Gemini 刷新并发许可失败: {}", e))?;
                let result = refresh_account_token(&account_id).await;
                if let Err(ref error) = result {
                    let _ = set_account_status(&account_id, Some("error"), Some(error));
                }
                Ok::<(String, Result<GeminiAccount, String>), String>((account_id, result))
            }
        })
        .collect();

    let mut results = Vec::with_capacity(tasks.len());
    for task in join_all(tasks).await {
        match task {
            Ok(item) => results.push(item),
            Err(error) => return Err(error),
        }
    }

    Ok(results)
}

pub fn inject_to_gemini_home(account_id: &str, cli_home_root: Option<&Path>) -> Result<(), String> {
    let mut account =
        load_account_file(account_id).ok_or_else(|| "Gemini 账号不存在".to_string())?;

    write_local_oauth_creds_to_path(&account, cli_home_root)?;
    write_local_oauth_creds_to_keychain(&account)?;
    clear_local_file_keychain_to_path(cli_home_root)?;
    update_local_active_account_with_path(&account.email, cli_home_root)?;
    write_local_selected_auth_type_to_path("oauth-personal", cli_home_root)?;

    account.selected_auth_type = Some("oauth-personal".to_string());
    account.last_used = now_ts();
    upsert_account_record(account.clone())?;

    let target_home = resolve_gemini_home(cli_home_root)?
        .to_string_lossy()
        .to_string();
    logger::log_info(&format!(
        "[Gemini Switch] 凭证注入成功: account_id={}, email={}, gemini_home={}",
        account.id, account.email, target_home
    ));

    Ok(())
}

pub fn inject_to_gemini(account_id: &str) -> Result<(), String> {
    inject_to_gemini_home(account_id, None)
}

pub(crate) fn extract_account_model_remaining(account: &GeminiAccount) -> Vec<(String, i32)> {
    let mut model_remaining: HashMap<String, i32> = HashMap::new();

    let Some(raw_usage) = account.gemini_usage_raw.as_ref() else {
        return Vec::new();
    };
    let Some(buckets) = raw_usage.get("buckets").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    for bucket in buckets {
        let model_id = normalize_non_empty(bucket.get("modelId").and_then(|v| v.as_str()));
        let remaining_fraction = bucket
            .get("remainingFraction")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                bucket
                    .get("remainingFraction")
                    .and_then(|v| v.as_str()?.parse::<f64>().ok())
            });

        let (Some(model_id), Some(remaining_fraction)) = (model_id, remaining_fraction) else {
            continue;
        };

        let percent_left = (remaining_fraction * 100.0).round().clamp(0.0, 100.0) as i32;
        model_remaining
            .entry(model_id)
            .and_modify(|value| {
                if percent_left < *value {
                    *value = percent_left;
                }
            })
            .or_insert(percent_left);
    }

    let mut values: Vec<(String, i32)> = model_remaining.into_iter().collect();
    values.sort_by(|a, b| a.0.cmp(&b.0));
    values
}

pub(crate) fn resolve_current_account(accounts: &[GeminiAccount]) -> Option<GeminiAccount> {
    let active_email = get_local_active_email();

    if let Some(active_email) = active_email {
        if let Some(found) = accounts
            .iter()
            .find(|account| account.email.eq_ignore_ascii_case(&active_email))
        {
            return Some(found.clone());
        }
    }

    accounts
        .iter()
        .max_by_key(|account| account.last_used.max(account.created_at))
        .cloned()
}

fn normalize_quota_alert_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn build_quota_alert_cooldown_key(account_id: &str, threshold: i32) -> String {
    format!("{}:{}", account_id, threshold)
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = GEMINI_QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < GEMINI_QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, threshold: i32) {
    if let Ok(mut state) = GEMINI_QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(account_id, threshold));
    }
}

fn pick_quota_alert_recommendation(
    accounts: &[GeminiAccount],
    current_account_id: &str,
) -> Option<(String, String)> {
    let mut best: Option<(String, String, i32)> = None;

    for account in accounts {
        if account.id == current_account_id {
            continue;
        }

        let metrics = extract_account_model_remaining(account);
        if metrics.is_empty() {
            continue;
        }

        let lowest = metrics
            .iter()
            .map(|(_, percent)| *percent)
            .min()
            .unwrap_or(0);

        match &best {
            Some((_, _, best_lowest)) if *best_lowest >= lowest => {}
            _ => {
                best = Some((account.id.clone(), account.email.clone(), lowest));
            }
        }
    }

    best.map(|(id, email, _)| (id, email))
}

pub fn run_quota_alert_if_needed() -> Result<(), String> {
    let cfg = crate::modules::config::get_user_config();
    if !cfg.gemini_quota_alert_enabled {
        return Ok(());
    }

    let threshold = normalize_quota_alert_threshold(cfg.gemini_quota_alert_threshold);
    let accounts = list_accounts();
    if accounts.is_empty() {
        return Ok(());
    }

    let current_account = match resolve_current_account(&accounts) {
        Some(account) => account,
        None => return Ok(()),
    };

    let metrics = extract_account_model_remaining(&current_account);
    if metrics.is_empty() {
        clear_quota_alert_cooldown(&current_account.id, threshold);
        return Ok(());
    }

    let lowest_percentage = metrics
        .iter()
        .map(|(_, percent)| *percent)
        .min()
        .unwrap_or(100);

    let low_models: Vec<String> = metrics
        .iter()
        .filter(|(_, percent)| *percent <= threshold)
        .map(|(model, _)| model.clone())
        .collect();

    if low_models.is_empty() || lowest_percentage > threshold {
        clear_quota_alert_cooldown(&current_account.id, threshold);
        return Ok(());
    }

    let now = now_ts();
    let cooldown_key = build_quota_alert_cooldown_key(&current_account.id, threshold);
    if !should_emit_quota_alert(&cooldown_key, now) {
        return Ok(());
    }

    let recommendation = pick_quota_alert_recommendation(&accounts, &current_account.id);

    let payload = crate::modules::account::QuotaAlertPayload {
        platform: "gemini".to_string(),
        current_account_id: current_account.id,
        current_email: current_account.email,
        threshold,
        threshold_display: None,
        lowest_percentage,
        low_models,
        recommended_account_id: recommendation.as_ref().map(|item| item.0.clone()),
        recommended_email: recommendation.as_ref().map(|item| item.1.clone()),
        triggered_at: now,
    };

    crate::modules::account::dispatch_quota_alert(&payload);
    Ok(())
}
