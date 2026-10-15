use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::models::zcode::{ZcodeAccount, ZcodeAccountIndex, ZcodeAuthMode};
use crate::modules::{account, atomic_write};

const ACCOUNTS_DIR: &str = "zcode_accounts";
const ACCOUNTS_INDEX_FILE: &str = "zcode_accounts.json";
const CREDENTIALS_FILE: &str = "credentials.json";
const CONFIG_FILE: &str = "config.json";
const SETTINGS_FILE: &str = "setting.json";
const DEVICE_MID_FILE: &str = "zcode_device_mid";
const CREDENTIAL_PREFIX: &str = "enc:v1:";
// 与当前官方 ZCode 3.10.2 保持一致；安装路径可读取时优先使用实际版本。
const DEFAULT_APP_VERSION: &str = "3.10.2";
const BILLING_BALANCE_URL: &str = "https://zcode.z.ai/api/v1/zcode-plan/billing/balance";
const ACTIVE_PROVIDER_KEY: &str = "oauth:active_provider";
const ZCODE_JWT_KEY: &str = "zcodejwttoken";
const ZAI_API_KEY_PROVIDER_ID: &str = "builtin:zai";
const BIGMODEL_API_KEY_PROVIDER_ID: &str = "builtin:bigmodel";
const ZAI_API_BASE_URL: &str = "https://api.z.ai/api/anthropic";
const BIGMODEL_API_BASE_URL: &str = "https://open.bigmodel.cn/api/anthropic";

static ZCODE_ACCOUNT_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn normalize_string(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn normalize_provider(provider: &str) -> Result<String, String> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "zai" => Ok("zai".to_string()),
        "bigmodel" => Ok("bigmodel".to_string()),
        _ => Err("不支持的 ZCode OAuth provider".to_string()),
    }
}

fn normalize_tags(tags: Vec<String>) -> Option<Vec<String>> {
    let mut seen = HashSet::new();
    let values: Vec<String> = tags
        .into_iter()
        .filter_map(|tag| normalize_string(Some(&tag)))
        .filter(|tag| seen.insert(tag.to_ascii_lowercase()))
        .collect();
    (!values.is_empty()).then_some(values)
}

fn account_id(provider: &str, user_id: Option<&str>, email: Option<&str>) -> String {
    let identity = normalize_string(user_id)
        .or_else(|| normalize_string(email))
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    format!(
        "zcode_{:x}",
        md5::compute(format!("{}:{}", provider, identity))
    )
}

fn api_key_account_id(provider: &str, api_key: &str) -> String {
    format!(
        "zcode_apikey_{:x}",
        md5::compute(format!("{}:{}", provider, api_key))
    )
}

fn provider_display_name(provider: &str) -> &'static str {
    if provider == "bigmodel" {
        "BigModel"
    } else {
        "Z.ai"
    }
}

fn api_key_suffix(api_key: &str) -> String {
    let mut chars: Vec<char> = api_key.chars().rev().take(4).collect();
    chars.reverse();
    chars.into_iter().collect()
}

fn accounts_dir() -> Result<PathBuf, String> {
    let path = account::get_data_dir()?.join(ACCOUNTS_DIR);
    fs::create_dir_all(&path).map_err(|error| format!("创建 ZCode 账号目录失败: {}", error))?;
    Ok(path)
}

fn index_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(index_path()?.to_string_lossy().to_string())
}

fn safe_account_id(value: &str) -> Result<&str, String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err("ZCode 账号 ID 非法".to_string());
    }
    Ok(trimmed)
}

fn account_path(account_id: &str) -> Result<PathBuf, String> {
    Ok(accounts_dir()?.join(format!("{}.json", safe_account_id(account_id)?)))
}

fn load_index() -> Result<ZcodeAccountIndex, String> {
    let path = index_path()?;
    if !path.exists() {
        return Ok(ZcodeAccountIndex::default());
    }
    let content =
        fs::read_to_string(&path).map_err(|error| format!("读取 ZCode 账号索引失败: {}", error))?;
    if content.trim().is_empty() {
        return Ok(ZcodeAccountIndex::default());
    }
    atomic_write::parse_json_with_auto_restore(&path, &content)
        .map_err(|error| format!("解析 ZCode 账号索引失败: {}", error))
}

fn save_index(index: &ZcodeAccountIndex) -> Result<(), String> {
    let content = serde_json::to_string_pretty(index)
        .map_err(|error| format!("序列化 ZCode 账号索引失败: {}", error))?;
    atomic_write::write_string_atomic(&index_path()?, &content)
        .map_err(|error| format!("保存 ZCode 账号索引失败: {}", error))
}

pub fn load_account(account_id: &str) -> Option<ZcodeAccount> {
    let path = account_path(account_id).ok()?;
    let content = fs::read_to_string(&path).ok()?;
    match crate::modules::secure_account_storage::deserialize_account_file::<ZcodeAccount>(
        &path, &content,
    ) {
        Ok((account, needs_rotation)) => {
            if needs_rotation {
                let account_for_rewrite = account.clone();
                crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
                    "zcode",
                    account_for_rewrite.id.clone(),
                    path.clone(),
                    content.as_bytes(),
                    move || {
                        crate::modules::secure_account_storage::serialize_account_file(
                            "zcode",
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

fn save_account_file(account: &ZcodeAccount) -> Result<(), String> {
    let content = crate::modules::secure_account_storage::serialize_account_file("zcode", account)?;
    atomic_write::write_string_atomic(&account_path(&account.id)?, &content)
        .map_err(|error| format!("保存 ZCode 账号失败: {}", error))
}

pub fn upsert_account(mut value: ZcodeAccount) -> Result<ZcodeAccount, String> {
    let _guard = ZCODE_ACCOUNT_LOCK
        .lock()
        .map_err(|_| "获取 ZCode 账号锁失败".to_string())?;
    value.provider = normalize_provider(&value.provider)?;
    value.email =
        normalize_string(Some(&value.email)).unwrap_or_else(|| "unknown@zcode.local".to_string());
    value.user_id = normalize_string(value.user_id.as_deref());
    value.display_name = normalize_string(value.display_name.as_deref());
    value.avatar_url = normalize_string(value.avatar_url.as_deref());
    value.refresh_token = normalize_string(value.refresh_token.as_deref());
    value.api_key = normalize_string(value.api_key.as_deref());
    value.plan_type = normalize_string(value.plan_type.as_deref());
    value.tags = normalize_tags(value.tags.unwrap_or_default());
    if value.auth_mode == ZcodeAuthMode::ApiKey {
        let api_key = value
            .api_key
            .as_deref()
            .ok_or_else(|| "ZCode API Key 账号缺少 API Key".to_string())?;
        value.id = api_key_account_id(&value.provider, api_key);
        value.access_token.clear();
        value.refresh_token = None;
        value.zcode_jwt_token.clear();
        value.plan_type = Some("API Key".to_string());
    }
    if value.id.trim().is_empty() {
        value.id = account_id(
            &value.provider,
            value.user_id.as_deref(),
            Some(&value.email),
        );
    }
    if let Some(existing) = load_account(&value.id) {
        if value.tags.is_none() {
            value.tags = existing.tags;
        }
        if value.plan_type.is_none() {
            value.plan_type = existing.plan_type;
        }
        if value.quota_raw.is_none() {
            value.quota_total = existing.quota_total;
            value.quota_used = existing.quota_used;
            value.quota_remaining = existing.quota_remaining;
            value.quota_reset_at = existing.quota_reset_at;
            value.quota_query_last_error = existing.quota_query_last_error;
            value.quota_query_last_error_at = existing.quota_query_last_error_at;
            value.usage_updated_at = existing.usage_updated_at;
            value.subscription_raw = existing.subscription_raw;
            value.quota_raw = existing.quota_raw;
        }
        if existing.created_at > 0 {
            value.created_at = existing.created_at;
        }
    }
    if value.created_at <= 0 {
        value.created_at = now_ts();
    }
    if value.last_used <= 0 {
        value.last_used = now_ts();
    }

    let mut index = load_index()?;
    save_account_file(&value)?;
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == value.id) {
        *summary = value.summary();
    } else {
        index.accounts.push(value.summary());
    }
    index
        .accounts
        .sort_by(|left, right| right.last_used.cmp(&left.last_used));
    save_index(&index)?;
    Ok(value)
}

pub fn list_accounts_checked() -> Result<Vec<ZcodeAccount>, String> {
    let index = load_index()?;
    let mut values: Vec<ZcodeAccount> = index
        .accounts
        .iter()
        .filter_map(|summary| load_account(&summary.id))
        .collect();
    values.sort_by(|left, right| right.last_used.cmp(&left.last_used));
    Ok(values)
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    remove_accounts(&[account_id.to_string()])
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    let _guard = ZCODE_ACCOUNT_LOCK
        .lock()
        .map_err(|_| "获取 ZCode 账号锁失败".to_string())?;
    let ids: HashSet<&str> = account_ids.iter().map(String::as_str).collect();
    let mut index = load_index()?;
    index
        .accounts
        .retain(|item| !ids.contains(item.id.as_str()));
    if index
        .current_account_id
        .as_deref()
        .is_some_and(|id| ids.contains(id))
    {
        index.current_account_id = None;
    }
    save_index(&index)?;
    for id in account_ids {
        let path = account_path(id)?;
        if path.exists() {
            crate::modules::atomic_write::remove_file_locked(&path)
                .map_err(|error| format!("删除 ZCode 账号失败: {}", error))?;
        }
    }
    Ok(())
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<ZcodeAccount, String> {
    let mut value = load_account(account_id).ok_or_else(|| "ZCode 账号不存在".to_string())?;
    value.tags = normalize_tags(tags);
    upsert_account(value)
}

pub fn current_account_id() -> Result<Option<String>, String> {
    Ok(load_index()?.current_account_id)
}

fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

fn username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn fallback_credential_secret(platform: &str, home_dir: &Path, username: &str) -> String {
    format!(
        "zcode-credential-fallback:{}:{}:{}",
        platform,
        home_dir.to_string_lossy(),
        username
    )
}

pub(crate) fn credential_secret_for_home(home_dir: &Path) -> String {
    std::env::var("ZCODE_CREDENTIAL_SECRET")
        .ok()
        .filter(|secret| !secret.is_empty())
        .unwrap_or_else(|| fallback_credential_secret(platform_name(), home_dir, &username()))
}

fn credential_key(home_dir: &Path) -> [u8; 32] {
    Sha256::digest(credential_secret_for_home(home_dir).as_bytes()).into()
}

fn credential_key_from_fallback(platform: &str, home_dir: &Path, username: &str) -> [u8; 32] {
    Sha256::digest(fallback_credential_secret(platform, home_dir, username).as_bytes()).into()
}

fn decode_component(value: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|error| format!("解析 ZCode 凭据密文失败: {}", error))
}

pub fn decrypt_credential(value: &str, home_dir: &Path) -> Result<String, String> {
    decrypt_credential_with_key(value, &credential_key(home_dir))
}

fn decrypt_credential_with_key(value: &str, key: &[u8; 32]) -> Result<String, String> {
    if !value.starts_with(CREDENTIAL_PREFIX) {
        return Ok(value.to_string());
    }
    let parts: Vec<&str> = value[CREDENTIAL_PREFIX.len()..].split('.').collect();
    if parts.len() != 3 {
        return Err("ZCode 凭据密文格式无效".to_string());
    }
    let nonce = decode_component(parts[0])?;
    let tag = decode_component(parts[1])?;
    let mut encrypted = decode_component(parts[2])?;
    if nonce.len() != 12 || tag.len() != 16 {
        return Err("ZCode 凭据密文参数无效".to_string());
    }
    encrypted.extend_from_slice(&tag);
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| "初始化 ZCode 凭据解密器失败".to_string())?;
    let plain = cipher
        .decrypt(Nonce::from_slice(&nonce), encrypted.as_ref())
        .map_err(|_| "ZCode 凭据解密失败，当前用户或 HOME 与写入环境不一致".to_string())?;
    String::from_utf8(plain).map_err(|error| format!("ZCode 凭据不是有效 UTF-8: {}", error))
}

pub fn encrypt_credential(value: &str, home_dir: &Path) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(&credential_key(home_dir))
        .map_err(|_| "初始化 ZCode 凭据加密器失败".to_string())?;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let mut encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), value.as_bytes())
        .map_err(|_| "加密 ZCode 凭据失败".to_string())?;
    if encrypted.len() < 16 {
        return Err("ZCode 凭据加密结果无效".to_string());
    }
    let tag = encrypted.split_off(encrypted.len() - 16);
    Ok(format!(
        "{}{}.{}.{}",
        CREDENTIAL_PREFIX,
        URL_SAFE_NO_PAD.encode(nonce),
        URL_SAFE_NO_PAD.encode(tag),
        URL_SAFE_NO_PAD.encode(encrypted)
    ))
}

fn read_json_map(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("读取 ZCode 凭据失败: {}", error))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|error| format!("解析 ZCode 凭据失败: {}", error))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "ZCode 凭据文件必须是 JSON 对象".to_string())
}

fn resolve_default_v2_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    let default = home.join(".zcode/v2");
    let setting_path = default.join(SETTINGS_FILE);
    if let Ok(content) = fs::read_to_string(setting_path) {
        if let Ok(value) = serde_json::from_str::<Value>(&content) {
            if let Some(base) = value
                .get("dataBaseDir")
                .and_then(Value::as_str)
                .and_then(|v| normalize_string(Some(v)))
            {
                return Ok(PathBuf::from(base).join(".zcode/v2"));
            }
        }
    }
    Ok(default)
}

pub fn default_credentials_path() -> Result<PathBuf, String> {
    Ok(resolve_default_v2_dir()?.join(CREDENTIALS_FILE))
}

fn default_config_path() -> Result<PathBuf, String> {
    Ok(resolve_default_v2_dir()?.join(CONFIG_FILE))
}

fn default_settings_path() -> Result<PathBuf, String> {
    Ok(resolve_default_v2_dir()?.join(SETTINGS_FILE))
}

pub fn default_data_root_dir() -> Result<PathBuf, String> {
    resolve_default_v2_dir()?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "ZCode 数据目录无效".to_string())
}

pub fn credentials_path_for_instance_root(root: &Path) -> PathBuf {
    root.join("data/.zcode/v2").join(CREDENTIALS_FILE)
}

fn config_path_for_instance_root(root: &Path) -> PathBuf {
    root.join("data/.zcode/v2").join(CONFIG_FILE)
}

fn settings_path_for_instance_root(root: &Path) -> PathBuf {
    root.join("data/.zcode/v2").join(SETTINGS_FILE)
}

fn decrypted_value(
    values: &Map<String, Value>,
    name: &str,
    home: &Path,
) -> Result<Option<String>, String> {
    values
        .get(name)
        .and_then(Value::as_str)
        .map(|value| decrypt_credential(value, home))
        .transpose()
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .and_then(|value| normalize_string(Some(value)))
}

pub fn account_from_credentials_path(path: &Path) -> Result<ZcodeAccount, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    account_from_credentials_path_with_home(path, &home)
}

fn account_from_credentials_path_with_home(
    path: &Path,
    home: &Path,
) -> Result<ZcodeAccount, String> {
    let values = read_json_map(path)?;
    let provider = decrypted_value(&values, ACTIVE_PROVIDER_KEY, home)?
        .ok_or_else(|| "ZCode 本地凭据缺少 active provider".to_string())?;
    let provider = normalize_provider(&provider)?;
    let access_token = decrypted_value(&values, &format!("oauth:{}:access_token", provider), home)?
        .ok_or_else(|| "ZCode 本地凭据缺少 access token".to_string())?;
    let refresh_token =
        decrypted_value(&values, &format!("oauth:{}:refresh_token", provider), home)?;
    let zcode_jwt_token = decrypted_value(&values, ZCODE_JWT_KEY, home)?
        .ok_or_else(|| "ZCode 本地凭据缺少 zcode JWT".to_string())?;
    let user_info_text = decrypted_value(&values, &format!("oauth:{}:user_info", provider), home)?;
    let user_info = user_info_text
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .unwrap_or_else(|| json!({}));
    let user_id = value_string(&user_info, &["user_id", "id", "customerNumber", "sub"]);
    let email =
        value_string(&user_info, &["email"]).unwrap_or_else(|| "unknown@zcode.local".to_string());
    let display_name = value_string(
        &user_info,
        &[
            "name",
            "displayName",
            "username",
            "nickName",
            "customerName",
        ],
    );
    let avatar_url = value_string(&user_info, &["avatar", "avatarUrl", "picture"]);
    let now = now_ts();
    Ok(ZcodeAccount {
        id: account_id(&provider, user_id.as_deref(), Some(&email)),
        auth_mode: ZcodeAuthMode::Oauth,
        provider,
        email,
        user_id,
        display_name,
        avatar_url,
        access_token,
        refresh_token,
        zcode_jwt_token,
        api_key: None,
        expires_at: None,
        plan_type: None,
        quota_total: None,
        quota_used: None,
        quota_remaining: None,
        quota_reset_at: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        tags: None,
        user_info_raw: Some(user_info),
        subscription_raw: None,
        quota_raw: None,
        created_at: now,
        last_used: now,
    })
}

fn api_key_account(
    provider: &str,
    api_key: &str,
    account_name: Option<&str>,
) -> Result<ZcodeAccount, String> {
    let provider = normalize_provider(provider)?;
    let api_key =
        normalize_string(Some(api_key)).ok_or_else(|| "请输入 ZCode API Key".to_string())?;
    if api_key.chars().any(char::is_whitespace) {
        return Err("ZCode API Key 不能包含空白字符".to_string());
    }
    let display_name = normalize_string(account_name).unwrap_or_else(|| {
        format!(
            "{} API Key ...{}",
            provider_display_name(&provider),
            api_key_suffix(&api_key)
        )
    });
    let now = now_ts();
    Ok(ZcodeAccount {
        id: api_key_account_id(&provider, &api_key),
        auth_mode: ZcodeAuthMode::ApiKey,
        provider,
        email: "unknown@zcode.local".to_string(),
        user_id: None,
        display_name: Some(display_name),
        avatar_url: None,
        access_token: String::new(),
        refresh_token: None,
        zcode_jwt_token: String::new(),
        api_key: Some(api_key),
        expires_at: None,
        plan_type: Some("API Key".to_string()),
        quota_total: None,
        quota_used: None,
        quota_remaining: None,
        quota_reset_at: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        tags: None,
        user_info_raw: None,
        subscription_raw: None,
        quota_raw: None,
        created_at: now,
        last_used: now,
    })
}

pub fn import_api_key(
    api_key: &str,
    provider: &str,
    account_name: Option<&str>,
) -> Result<ZcodeAccount, String> {
    upsert_account(api_key_account(provider, api_key, account_name)?)
}

fn api_key_from_provider_config(root: &Map<String, Value>, provider: &str) -> Option<String> {
    let provider_id = if provider == "bigmodel" {
        BIGMODEL_API_KEY_PROVIDER_ID
    } else {
        ZAI_API_KEY_PROVIDER_ID
    };
    root.get("provider")?
        .get(provider_id)?
        .get("options")?
        .get("apiKey")?
        .as_str()
        .and_then(|value| normalize_string(Some(value)))
}

fn api_key_accounts_from_config(path: &Path) -> Result<Vec<ZcodeAccount>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let root = read_json_map(path)?;
    ["zai", "bigmodel"]
        .into_iter()
        .filter_map(|provider| {
            api_key_from_provider_config(&root, provider)
                .map(|api_key| api_key_account(provider, &api_key, None))
        })
        .collect()
}

pub async fn import_from_local() -> Result<Vec<ZcodeAccount>, String> {
    let mut imported = Vec::new();
    let credentials_path = default_credentials_path()?;
    if credentials_path.exists() {
        if let Ok(account) = account_from_credentials_path(&credentials_path) {
            let account = upsert_account(account)?;
            imported.push(refresh_account_quota(&account.id).await.unwrap_or(account));
        }
    }
    for account in api_key_accounts_from_config(&default_config_path()?)? {
        imported.push(upsert_account(account)?);
    }
    if imported.is_empty() {
        return Err("未找到可导入的 ZCode OAuth 凭据或 API Key".to_string());
    }
    Ok(imported)
}

fn official_user_info(account: &ZcodeAccount) -> Value {
    account.user_info_raw.clone().unwrap_or_else(|| {
        json!({
            "user_id": account.user_id,
            "email": account.email,
            "name": account.display_name,
            "avatar": account.avatar_url,
        })
    })
}

fn ensure_object<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let value = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("object inserted above")
}

fn write_json_map(path: &Path, values: Map<String, Value>, label: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("ZCode {} 目录无效", label))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 ZCode {} 目录失败: {}", label, error))?;
    let content = serde_json::to_string_pretty(&Value::Object(values))
        .map_err(|error| format!("序列化 ZCode {} 失败: {}", label, error))?;
    atomic_write::write_string_atomic(path, &content)
        .map_err(|error| format!("写入 ZCode {} 失败: {}", label, error))
}

fn api_key_provider_spec(provider: &str) -> (&'static str, &'static str, &'static str) {
    if provider == "bigmodel" {
        (
            BIGMODEL_API_KEY_PROVIDER_ID,
            "Bigmodel - API Key",
            BIGMODEL_API_BASE_URL,
        )
    } else {
        (ZAI_API_KEY_PROVIDER_ID, "Z.ai - API Key", ZAI_API_BASE_URL)
    }
}

fn default_api_key_models() -> Value {
    json!({
        "GLM-5.2": {
            "limit": { "context": 1_000_000 },
            "modalities": { "input": ["text"], "output": ["text"] }
        },
        "GLM-5-Turbo": {
            "name": "glm-5-turbo",
            "reasoning": {
                "enabled": true,
                "variants": ["enabled", "off"],
                "defaultVariant": "enabled"
            },
            "limit": { "context": 200_000, "output": 64_000 },
            "modalities": { "input": ["text"], "output": ["text"] }
        }
    })
}

fn write_api_key_to_config_path(account: &ZcodeAccount, path: &Path) -> Result<(), String> {
    let provider = normalize_provider(&account.provider)?;
    let api_key = account
        .api_key
        .as_deref()
        .and_then(|value| normalize_string(Some(value)))
        .ok_or_else(|| "ZCode API Key 账号缺少 API Key".to_string())?;
    let (provider_id, display_name, base_url) = api_key_provider_spec(&provider);
    let mut root = read_json_map(path)?;
    let providers = ensure_object(&mut root, "provider");
    let provider_value = providers
        .entry(provider_id.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !provider_value.is_object() {
        *provider_value = Value::Object(Map::new());
    }
    let provider_config = provider_value
        .as_object_mut()
        .expect("object inserted above");
    provider_config.insert("name".to_string(), Value::String(display_name.to_string()));
    provider_config.insert("kind".to_string(), Value::String("anthropic".to_string()));
    provider_config.insert("enabled".to_string(), Value::Bool(true));
    provider_config.insert("source".to_string(), Value::String("custom".to_string()));
    provider_config.insert("updatedAt".to_string(), Value::Number(now_ms().into()));
    provider_config.remove("systemDisabledReason");
    if !provider_config.get("models").is_some_and(Value::is_object) {
        provider_config.insert("models".to_string(), default_api_key_models());
    }
    let options = ensure_object(provider_config, "options");
    options.insert("apiKey".to_string(), Value::String(api_key));
    options.insert("baseURL".to_string(), Value::String(base_url.to_string()));
    write_json_map(path, root, "config.json")
}

fn write_auth_mode_to_settings_path(
    provider: &str,
    auth_mode: ZcodeAuthMode,
    path: &Path,
) -> Result<(), String> {
    let provider = normalize_provider(provider)?;
    let mut root = read_json_map(path)?;
    root.insert(
        "providerFamilyDomain".to_string(),
        Value::String(provider.clone()),
    );
    root.insert(
        "providerFamilyDomainUpdatedAt".to_string(),
        Value::Number(now_ms().into()),
    );
    root.insert(
        "providerFamilyDomainMigrated".to_string(),
        Value::Bool(true),
    );
    let family_modes = ensure_object(&mut root, "modelProviderFamilyModes");
    family_modes.insert(
        provider,
        Value::String(
            if auth_mode == ZcodeAuthMode::ApiKey {
                "apiKey"
            } else {
                "oauth"
            }
            .to_string(),
        ),
    );
    write_json_map(path, root, "setting.json")
}

pub fn write_account_to_credentials_path(
    account: &ZcodeAccount,
    path: &Path,
) -> Result<(), String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    write_account_to_credentials_path_with_home(account, path, &home)
}

fn write_account_to_credentials_path_with_home(
    account: &ZcodeAccount,
    path: &Path,
    home: &Path,
) -> Result<(), String> {
    if account.auth_mode != ZcodeAuthMode::Oauth {
        return Err(
            "API Key 账号应写入 ZCode config.json，而不是 OAuth credentials.json".to_string(),
        );
    }
    let mut values = read_json_map(path)?;
    for provider in ["zai", "bigmodel"] {
        for suffix in ["access_token", "refresh_token", "user_info"] {
            values.remove(&format!("oauth:{}:{}", provider, suffix));
        }
    }
    let provider = normalize_provider(&account.provider)?;
    let mut put = |key: String, value: &str| -> Result<(), String> {
        values.insert(key, Value::String(encrypt_credential(value, home)?));
        Ok(())
    };
    put(ACTIVE_PROVIDER_KEY.to_string(), &provider)?;
    put(
        format!("oauth:{}:access_token", provider),
        &account.access_token,
    )?;
    if let Some(refresh) = normalize_string(account.refresh_token.as_deref()) {
        put(format!("oauth:{}:refresh_token", provider), &refresh)?;
    }
    put(
        format!("oauth:{}:user_info", provider),
        &serde_json::to_string(&official_user_info(account))
            .map_err(|error| format!("序列化 ZCode 用户信息失败: {}", error))?,
    )?;
    put(ZCODE_JWT_KEY.to_string(), &account.zcode_jwt_token)?;
    let parent = path
        .parent()
        .ok_or_else(|| "ZCode 凭据目录无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建 ZCode 凭据目录失败: {}", error))?;
    let content = serde_json::to_string_pretty(&Value::Object(values))
        .map_err(|error| format!("序列化 ZCode 凭据失败: {}", error))?;
    atomic_write::write_string_atomic(path, &content)
        .map_err(|error| format!("写入 ZCode 凭据失败: {}", error))
}

pub fn inject_to_default(account_id: &str) -> Result<ZcodeAccount, String> {
    let mut value = load_account(account_id).ok_or_else(|| "ZCode 账号不存在".to_string())?;
    if value.auth_mode == ZcodeAuthMode::ApiKey {
        write_api_key_to_config_path(&value, &default_config_path()?)?;
    } else {
        write_account_to_credentials_path(&value, &default_credentials_path()?)?;
    }
    write_auth_mode_to_settings_path(&value.provider, value.auth_mode, &default_settings_path()?)?;
    value.last_used = now_ts();
    let value = upsert_account(value)?;
    let mut index = load_index()?;
    index.current_account_id = Some(value.id.clone());
    save_index(&index)?;
    Ok(value)
}

pub fn inject_to_instance_root(account_id: &str, root: &Path) -> Result<ZcodeAccount, String> {
    let value = load_account(account_id).ok_or_else(|| "ZCode 账号不存在".to_string())?;
    if value.auth_mode == ZcodeAuthMode::ApiKey {
        write_api_key_to_config_path(&value, &config_path_for_instance_root(root))?;
    } else {
        write_account_to_credentials_path(&value, &credentials_path_for_instance_root(root))?;
    }
    write_auth_mode_to_settings_path(
        &value.provider,
        value.auth_mode,
        &settings_path_for_instance_root(root),
    )?;
    Ok(value)
}

fn detect_app_version() -> String {
    crate::modules::client_version::detect_client_version("ZCode", None)
        .unwrap_or_else(|| DEFAULT_APP_VERSION.to_string())
}

fn normalize_header_value(value: impl Into<String>) -> Option<HeaderValue> {
    HeaderValue::from_str(value.into().trim()).ok()
}

fn zcode_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        "linux" => "linux",
        _ => std::env::consts::OS,
    }
}

fn zcode_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "ia32",
        "arm" => "arm",
        _ => std::env::consts::ARCH,
    }
}

fn zcode_os_category() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

fn zcode_os_version() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(output) = std::process::Command::new("sw_vers")
        .args(["-productVersion"])
        .output()
    {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if output.status.success() && !value.is_empty() {
            return value;
        }
    }
    sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn zcode_locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .map(|value| {
            value
                .split('.')
                .next()
                .unwrap_or(value.as_str())
                .replace('_', "-")
        })
        .unwrap_or_else(|| "en-US".to_string())
}

fn zcode_timezone() -> String {
    std::env::var("TZ")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "UTC".to_string())
}

fn zcode_device_mid() -> Result<String, String> {
    let data_dir = resolve_default_v2_dir()?;

    // 官方客户端会把稳定设备标识保存在 telemetry-state.json。优先复用该值，
    // 这样额度请求与 ZCode 自身的设备身份保持一致；旧版本或未安装官方客户端
    // 时再使用 Cockpit 自己维护的回退文件。
    let telemetry_path = data_dir.join("telemetry-state.json");
    if let Ok(content) = fs::read_to_string(&telemetry_path) {
        if let Ok(value) = serde_json::from_str::<Value>(&content) {
            if let Some(device_mid) = value
                .get("deviceMid")
                .and_then(Value::as_str)
                .and_then(|value| uuid::Uuid::parse_str(value.trim()).ok())
            {
                return Ok(device_mid.to_string());
            }
        }
    }

    let path = data_dir.join(DEVICE_MID_FILE);
    if let Ok(value) = fs::read_to_string(&path) {
        if let Ok(uuid) = uuid::Uuid::parse_str(value.trim()) {
            return Ok(uuid.to_string());
        }
    }
    let value = uuid::Uuid::new_v4().to_string();
    atomic_write::write_string_atomic(&path, &value)
        .map_err(|error| format!("保存 ZCode 设备标识失败: {}", error))?;
    Ok(value)
}

fn zcode_source_headers(app_version: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let values = [
        (USER_AGENT, format!("ZCode/{}", app_version)),
        (
            HeaderName::from_static("http-referer"),
            "https://zcode.z.ai".to_string(),
        ),
        (
            HeaderName::from_static("x-zcode-app-version"),
            app_version.to_string(),
        ),
        (
            HeaderName::from_static("x-title"),
            "Z Code@electron".to_string(),
        ),
        (
            HeaderName::from_static("x-platform"),
            format!("{}-{}", zcode_platform(), zcode_arch()),
        ),
        (
            HeaderName::from_static("x-release-channel"),
            if cfg!(debug_assertions) {
                "test".to_string()
            } else {
                "production".to_string()
            },
        ),
        (HeaderName::from_static("x-client-language"), zcode_locale()),
        (
            HeaderName::from_static("x-client-timezone"),
            zcode_timezone(),
        ),
        (
            HeaderName::from_static("x-os-category"),
            zcode_os_category().to_string(),
        ),
        (HeaderName::from_static("x-os-version"), zcode_os_version()),
    ];
    for (name, value) in values {
        if let Some(value) = normalize_header_value(value) {
            headers.insert(name, value);
        }
    }
    if let Ok(device_mid) = zcode_device_mid() {
        if let Some(value) = normalize_header_value(device_mid) {
            headers.insert(HeaderName::from_static("x-device-mid"), value);
        }
    }
    if let Some(value) = normalize_header_value(uuid::Uuid::new_v4().to_string()) {
        headers.insert(HeaderName::from_static("x-request-id"), value);
    }
    headers
}

fn number(value: Option<&Value>) -> f64 {
    value
        .and_then(|value| match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse::<f64>().ok(),
            _ => None,
        })
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
}

fn unix_seconds(value: Option<&Value>) -> Option<i64> {
    let number = value.and_then(|value| match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|v| v as i64)),
        Value::String(text) => text.trim().parse::<f64>().ok().map(|v| v as i64),
        _ => None,
    })?;
    (number > 0).then_some(number)
}

fn apply_quota_payload(value: &mut ZcodeAccount, payload: Value) -> Result<(), String> {
    if payload.get("code").and_then(Value::as_i64) != Some(0) {
        return Err(payload
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("ZCode 配额接口返回失败")
            .to_string());
    }

    let data = payload.get("data").cloned().unwrap_or_else(|| json!({}));
    let plans = data
        .get("plans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let balances = data
        .get("balances")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let plan_type = plans
        .iter()
        .find(|plan| plan.get("status").and_then(Value::as_str) == Some("active"))
        .or_else(|| plans.first())
        .and_then(|plan| value_string(plan, &["name", "plan_id"]));
    if plan_type.is_some() {
        value.plan_type = plan_type;
    }
    value.quota_total = Some(
        balances
            .iter()
            .map(|item| number(item.get("total_units")))
            .sum(),
    );
    value.quota_used = Some(
        balances
            .iter()
            .map(|item| number(item.get("used_units")))
            .sum(),
    );
    value.quota_remaining = Some(
        balances
            .iter()
            .map(|item| {
                number(
                    item.get("remaining_units")
                        .or_else(|| item.get("available_units")),
                )
            })
            .sum(),
    );
    value.quota_reset_at = balances
        .iter()
        .filter_map(|item| {
            item.get("period_end")
                .or_else(|| item.get("expires_at"))
                .and_then(|value| unix_seconds(Some(value)))
        })
        .min();
    value.subscription_raw = Some(Value::Array(plans));
    value.quota_raw = Some(Value::Array(balances));
    value.usage_updated_at = Some(now_ms());
    value.quota_query_last_error = None;
    value.quota_query_last_error_at = None;
    Ok(())
}

pub async fn refresh_account_quota(account_id: &str) -> Result<ZcodeAccount, String> {
    let mut value = load_account(account_id).ok_or_else(|| "ZCode 账号不存在".to_string())?;
    if value.auth_mode == ZcodeAuthMode::ApiKey {
        value.plan_type = Some("API Key".to_string());
        value.quota_query_last_error = None;
        value.quota_query_last_error_at = None;
        return upsert_account(value);
    }
    let (app_version, source_headers) = tokio::task::spawn_blocking(|| {
        let app_version = detect_app_version();
        let source_headers = zcode_source_headers(&app_version);
        (app_version, source_headers)
    })
    .await
    .unwrap_or_else(|_| {
        let app_version = DEFAULT_APP_VERSION.to_string();
        let source_headers = HeaderMap::new();
        (app_version, source_headers)
    });
    let url = format!("{}?app_version={}", BILLING_BALANCE_URL, app_version);
    let response = reqwest::Client::new()
        .get(&url)
        .headers(source_headers)
        .bearer_auth(&value.zcode_jwt_token)
        .send()
        .await
        .map_err(|error| format!("请求 ZCode 配额失败: {}", error));
    let payload = match response {
        Ok(response) if response.status().is_success() => response
            .json::<Value>()
            .await
            .map_err(|error| format!("解析 ZCode 配额失败: {}", error)),
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|payload| match payload.get("msg").and_then(Value::as_str) {
                    Some(message) => Some(message.to_string()),
                    None => payload
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
                .filter(|message| !message.trim().is_empty())
                .unwrap_or_else(|| "服务端未提供错误详情".to_string());
            Err(format!(
                "请求 ZCode 配额失败: HTTP {} ({})",
                status, message
            ))
        }
        Err(error) => Err(error),
    };

    match payload {
        Ok(payload) => match apply_quota_payload(&mut value, payload) {
            Ok(()) => upsert_account(value),
            Err(message) => {
                value.quota_query_last_error = Some(message.clone());
                value.quota_query_last_error_at = Some(now_ms());
                let _ = upsert_account(value);
                Err(message)
            }
        },
        Err(error) => {
            value.quota_query_last_error = Some(error.clone());
            value.quota_query_last_error_at = Some(now_ms());
            let _ = upsert_account(value);
            Err(error)
        }
    }
}

pub async fn refresh_all_accounts() -> Result<i32, String> {
    let accounts = list_accounts_checked()?;
    let mut success = 0;
    for value in accounts {
        if refresh_account_quota(&value.id).await.is_ok() {
            success += 1;
        }
    }
    Ok(success)
}

fn serialize_accounts_for_export(
    values: Vec<ZcodeAccount>,
    account_ids: &[String],
) -> Result<String, String> {
    let ids: HashSet<&str> = account_ids.iter().map(String::as_str).collect();
    let selected: Vec<ZcodeAccount> = values
        .into_iter()
        .filter(|value| ids.is_empty() || ids.contains(value.id.as_str()))
        .collect();
    serde_json::to_string_pretty(&selected)
        .map_err(|error| format!("序列化 ZCode 导出失败: {}", error))
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    serialize_accounts_for_export(list_accounts_checked()?, account_ids)
}

fn parse_import_accounts(content: &str) -> Result<Vec<ZcodeAccount>, String> {
    let root: Value =
        serde_json::from_str(content).map_err(|error| format!("JSON 解析失败: {}", error))?;
    let items: Vec<Value> = match root {
        Value::Array(items) => items,
        Value::Object(object) => object
            .get("accounts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![Value::Object(object)]),
        _ => return Err("ZCode 导入数据必须是对象或数组".to_string()),
    };
    let mut imported = Vec::new();
    for item in items {
        let mut account: ZcodeAccount = serde_json::from_value(item)
            .map_err(|error| format!("ZCode 导入账号格式无效: {}", error))?;
        if account.auth_mode == ZcodeAuthMode::ApiKey {
            if account
                .api_key
                .as_deref()
                .and_then(|value| normalize_string(Some(value)))
                .is_none()
            {
                return Err("ZCode 导入的 API Key 账号缺少 API Key".to_string());
            }
        } else if account.access_token.trim().is_empty()
            || account.zcode_jwt_token.trim().is_empty()
        {
            return Err("ZCode 导入的 OAuth 账号缺少必要 Token".to_string());
        }
        account.id.clear();
        imported.push(account);
    }
    Ok(imported)
}

pub fn import_from_json(content: &str) -> Result<Vec<ZcodeAccount>, String> {
    parse_import_accounts(content)?
        .into_iter()
        .map(upsert_account)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "{}-{}-{}",
            prefix,
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn sample_account() -> ZcodeAccount {
        ZcodeAccount {
            id: "zcode_fixture".to_string(),
            auth_mode: ZcodeAuthMode::Oauth,
            provider: "zai".to_string(),
            email: "fixture@example.com".to_string(),
            user_id: Some("fixture-user".to_string()),
            display_name: Some("Fixture User".to_string()),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            access_token: "access-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
            zcode_jwt_token: "zcode-jwt-token".to_string(),
            api_key: None,
            expires_at: Some(1_900_000_000),
            plan_type: None,
            quota_total: None,
            quota_used: None,
            quota_remaining: None,
            quota_reset_at: None,
            quota_query_last_error: Some("stale error".to_string()),
            quota_query_last_error_at: Some(1),
            usage_updated_at: None,
            tags: Some(vec!["work".to_string()]),
            user_info_raw: Some(json!({
                "user_id": "fixture-user",
                "email": "fixture@example.com",
                "name": "Fixture User",
                "avatar": "https://example.com/avatar.png"
            })),
            subscription_raw: None,
            quota_raw: None,
            created_at: 1_700_000_000,
            last_used: 1_700_000_001,
        }
    }

    #[test]
    fn credential_cipher_matches_official_shape_and_round_trips() {
        let home = Path::new("/tmp/zcode-cipher-test-home");
        let encrypted = encrypt_credential("secret-value", home).unwrap();
        assert!(encrypted.starts_with(CREDENTIAL_PREFIX));
        assert_eq!(encrypted[CREDENTIAL_PREFIX.len()..].split('.').count(), 3);
        assert_eq!(
            decrypt_credential(&encrypted, home).unwrap(),
            "secret-value"
        );
    }

    #[test]
    fn decrypts_fixed_official_enc_v1_fixture() {
        // Independently generated with Node.js AES-256-GCM using ZCode's fallback key material.
        let key =
            credential_key_from_fallback("darwin", Path::new("/Users/zcode-test"), "test-user");
        let encrypted =
            "enc:v1:AAECAwQFBgcICQoL.NTIF8rgqI66J7hvPIwTD8g.QTtgwDlfAEvz72ttQggYC2KZyVwLVA";
        assert_eq!(
            decrypt_credential_with_key(encrypted, &key).unwrap(),
            "official-fixture-token"
        );
    }

    #[test]
    fn fallback_credential_secret_matches_official_material() {
        assert_eq!(
            fallback_credential_secret("darwin", Path::new("/Users/zcode-test"), "test-user"),
            "zcode-credential-fallback:darwin:/Users/zcode-test:test-user"
        );
    }

    #[test]
    fn credentials_write_and_read_round_trip_in_instance_directory() {
        let root = make_temp_dir("zcode-credentials-round-trip");
        let path = credentials_path_for_instance_root(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"preserved":"value","oauth:bigmodel:access_token":"obsolete"}"#,
        )
        .unwrap();

        let account = sample_account();
        let credential_home = Path::new("/Users/zcode-round-trip-test");
        write_account_to_credentials_path_with_home(&account, &path, credential_home).unwrap();
        let written = read_json_map(&path).unwrap();
        assert_eq!(
            written.get("preserved"),
            Some(&Value::String("value".into()))
        );
        assert!(!written.contains_key("oauth:bigmodel:access_token"));
        assert!(written
            .get(ACTIVE_PROVIDER_KEY)
            .and_then(Value::as_str)
            .is_some_and(|value| value.starts_with(CREDENTIAL_PREFIX)));

        let restored = account_from_credentials_path_with_home(&path, credential_home).unwrap();
        assert_eq!(restored.provider, account.provider);
        assert_eq!(restored.email, account.email);
        assert_eq!(restored.user_id, account.user_id);
        assert_eq!(restored.access_token, account.access_token);
        assert_eq!(restored.refresh_token, account.refresh_token);
        assert_eq!(restored.zcode_jwt_token, account.zcode_jwt_token);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn api_key_writer_uses_official_provider_config_and_setting_mode() {
        let root = make_temp_dir("zcode-api-key-round-trip");
        let config_path = config_path_for_instance_root(&root);
        let settings_path = settings_path_for_instance_root(&root);
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(
            &config_path,
            r#"{"preserved":true,"provider":{"custom:test":{"name":"Keep me"}}}"#,
        )
        .unwrap();
        fs::write(
            &settings_path,
            r#"{"locale":"zh-CN","modelProviderFamilyModes":{"bigmodel":"oauth"}}"#,
        )
        .unwrap();

        let account = api_key_account("zai", "zai-test-key", Some("Work Key")).unwrap();
        write_api_key_to_config_path(&account, &config_path).unwrap();
        write_auth_mode_to_settings_path("zai", ZcodeAuthMode::ApiKey, &settings_path).unwrap();

        let config = read_json_map(&config_path).unwrap();
        assert_eq!(config.get("preserved"), Some(&Value::Bool(true)));
        assert_eq!(
            config["provider"]["custom:test"]["name"].as_str(),
            Some("Keep me")
        );
        assert_eq!(
            config["provider"][ZAI_API_KEY_PROVIDER_ID]["options"]["apiKey"].as_str(),
            Some("zai-test-key")
        );
        assert_eq!(
            config["provider"][ZAI_API_KEY_PROVIDER_ID]["options"]["baseURL"].as_str(),
            Some(ZAI_API_BASE_URL)
        );
        assert_eq!(
            config["provider"][ZAI_API_KEY_PROVIDER_ID]["enabled"].as_bool(),
            Some(true)
        );

        let settings = read_json_map(&settings_path).unwrap();
        assert_eq!(settings["locale"].as_str(), Some("zh-CN"));
        assert_eq!(settings["providerFamilyDomain"].as_str(), Some("zai"));
        assert_eq!(
            settings["modelProviderFamilyModes"]["zai"].as_str(),
            Some("apiKey")
        );
        assert_eq!(
            settings["modelProviderFamilyModes"]["bigmodel"].as_str(),
            Some("oauth")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn quota_payload_aggregates_models_and_prefers_active_plan() {
        let mut account = sample_account();
        apply_quota_payload(
            &mut account,
            json!({
                "code": 0,
                "data": {
                    "plans": [
                        {"name": "Expired Plan", "status": "expired"},
                        {
                            "plan_id": "zcode-v3-start-plan-0615",
                            "name": "ZCode Start Plan",
                            "status": "active"
                        }
                    ],
                    "balances": [
                        {
                            "show_name": "GLM-5.2",
                            "total_units": 3_000_000,
                            "used_units": 0,
                            "remaining_units": 3_000_000,
                            "available_units": 3_000_000,
                            "period_end": 1_783_785_599,
                            "expires_at": 1_783_785_599
                        },
                        {
                            "show_name": "GLM-5-Turbo",
                            "total_units": "2000000",
                            "used_units": "250000",
                            "remaining_units": "1750000",
                            "available_units": "1750000",
                            "period_end": "1783785599",
                            "expires_at": "1783785599"
                        }
                    ]
                }
            }),
        )
        .unwrap();

        assert_eq!(account.plan_type.as_deref(), Some("ZCode Start Plan"));
        assert_eq!(account.quota_total, Some(5_000_000.0));
        assert_eq!(account.quota_used, Some(250_000.0));
        assert_eq!(account.quota_remaining, Some(4_750_000.0));
        assert_eq!(account.quota_reset_at, Some(1_783_785_599));
        assert_eq!(
            account
                .quota_raw
                .as_ref()
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(account.usage_updated_at.is_some());
        assert!(account.quota_query_last_error.is_none());
        assert!(account.quota_query_last_error_at.is_none());
    }

    #[test]
    fn quota_payload_surfaces_api_error_without_overwriting_existing_values() {
        let mut account = sample_account();
        account.plan_type = Some("Existing Plan".to_string());
        let error = apply_quota_payload(
            &mut account,
            json!({"code": 3001, "msg": "app_version is required"}),
        )
        .unwrap_err();
        assert_eq!(error, "app_version is required");
        assert_eq!(account.plan_type.as_deref(), Some("Existing Plan"));
    }

    #[test]
    fn empty_success_keeps_plan_and_clears_previous_error() {
        let mut account = sample_account();
        account.plan_type = Some("ZCode Start Plan".to_string());
        account.quota_query_last_error = Some("HTTP 400".to_string());
        account.quota_query_last_error_at = Some(1);

        apply_quota_payload(
            &mut account,
            json!({
                "code": 0,
                "data": {"plans": [], "balances": []}
            }),
        )
        .unwrap();

        assert_eq!(account.plan_type.as_deref(), Some("ZCode Start Plan"));
        assert_eq!(account.quota_raw, Some(Value::Array(Vec::new())));
        assert_eq!(account.subscription_raw, Some(Value::Array(Vec::new())));
        assert_eq!(account.quota_total, Some(0.0));
        assert!(account.quota_query_last_error.is_none());
        assert!(account.quota_query_last_error_at.is_none());
        assert!(account.usage_updated_at.is_some());
    }

    #[test]
    fn account_id_is_stable() {
        assert_eq!(
            account_id("zai", Some("user-1"), Some("first@example.com")),
            account_id("zai", Some("user-1"), Some("second@example.com"))
        );
    }

    #[test]
    fn tags_are_trimmed_and_deduplicated_case_insensitively() {
        assert_eq!(
            normalize_tags(vec![
                " Work ".to_string(),
                "work".to_string(),
                "Team".to_string(),
                "".to_string(),
            ]),
            Some(vec!["Work".to_string(), "Team".to_string()])
        );
        assert_eq!(normalize_tags(Vec::new()), None);
    }

    #[test]
    fn import_parser_accepts_export_wrapper_and_requires_tokens() {
        let account = sample_account();
        let parsed = parse_import_accounts(
            &serde_json::to_string(&json!({ "accounts": [account.clone()] })).unwrap(),
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].id.is_empty());

        let mut invalid = serde_json::to_value(account).unwrap();
        invalid["zcode_jwt_token"] = Value::String(String::new());
        let error = parse_import_accounts(&invalid.to_string()).unwrap_err();
        assert!(error.contains("缺少必要 Token"));

        let api_key = api_key_account("bigmodel", "bigmodel-test-key", None).unwrap();
        let parsed = parse_import_accounts(&serde_json::to_string(&api_key).unwrap()).unwrap();
        assert_eq!(parsed[0].auth_mode, ZcodeAuthMode::ApiKey);
        assert_eq!(parsed[0].api_key.as_deref(), Some("bigmodel-test-key"));
    }

    #[test]
    fn export_serializer_respects_selected_account_ids() {
        let first = sample_account();
        let mut second = sample_account();
        second.id = "zcode_second".to_string();
        second.user_id = Some("fixture-user-2".to_string());
        second.email = "second@example.com".to_string();

        let selected = serialize_accounts_for_export(
            vec![first.clone(), second.clone()],
            std::slice::from_ref(&second.id),
        )
        .unwrap();
        let selected: Vec<ZcodeAccount> = serde_json::from_str(&selected).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, second.id);

        let all = serialize_accounts_for_export(vec![first, second], &[]).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<ZcodeAccount>>(&all)
                .unwrap()
                .len(),
            2
        );
    }
}
