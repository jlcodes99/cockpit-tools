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
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::models::zcode::{ZcodeAccount, ZcodeAccountIndex, ZcodeAuthMode, ZcodeTeamPlan};
use crate::modules::{account, atomic_write};

const ACCOUNTS_DIR: &str = "zcode_accounts";
const ACCOUNTS_INDEX_FILE: &str = "zcode_accounts.json";
const CREDENTIALS_FILE: &str = "credentials.json";
const CONFIG_FILE: &str = "config.json";
const SETTINGS_FILE: &str = "setting.json";
const DEVICE_MID_FILE: &str = "zcode_device_mid";
const CREDENTIAL_PREFIX: &str = "enc:v1:";
const CREDENTIAL_V2_PREFIX: &str = "enc:v2:";
const CREDENTIAL_KEYRING_SERVICE: &str = "ZCode Credential Encryption";
const CREDENTIAL_SECRET_FILE: &str = "credential-secret";
// 安装路径可读取时优先使用 ZCode 的实际版本。
const DEFAULT_APP_VERSION: &str = "3.18.2";
const BILLING_BALANCE_URL: &str = "https://zcode.z.ai/api/v1/zcode-plan/billing/balance";
const MCP_USAGE_URL: &str = "https://zcode.z.ai/api/v1/mcp/usage";
const ACTIVE_PROVIDER_KEY: &str = "oauth:active_provider";
const ZCODE_JWT_KEY: &str = "zcodejwttoken";
pub(crate) const ZAI_API_KEY_PROVIDER_ID: &str = "builtin:zai";
pub(crate) const BIGMODEL_API_KEY_PROVIDER_ID: &str = "builtin:bigmodel";
pub(crate) const ZAI_API_BASE_URL: &str = "https://api.z.ai/api/anthropic";
pub(crate) const BIGMODEL_API_BASE_URL: &str = "https://open.bigmodel.cn/api/anthropic";
pub(crate) const ZCODE_PLAN_API_BASE_URL: &str = "https://zcode.z.ai/api/v1/zcode-plan/anthropic";

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

/// Only an active personal Start Plan on Z.ai uses the session JWT for inference.
/// The dashboard's display label can fall back to an expired plan.
pub(crate) fn oauth_inference_session(account: &ZcodeAccount) -> Result<&str, &'static str> {
    if account.provider.eq_ignore_ascii_case("bigmodel") {
        return Err("ZCODE_BIGMODEL_PROJECT_API_KEY_REQUIRED");
    }
    if account.auth_mode != ZcodeAuthMode::Oauth
        || !account.provider.eq_ignore_ascii_case("zai")
        || account.team_plan.is_some()
    {
        return Err("ZCODE_START_PLAN_REQUIRED");
    }
    let plans = account.subscription_raw.as_ref().and_then(Value::as_array)
        .ok_or("ZCODE_START_PLAN_REQUIRED")?;
    let mut active = plans.iter().filter(|plan| plan["status"].as_str()
        .is_some_and(|status| status.trim().eq_ignore_ascii_case("active"))).peekable();
    if active.peek().is_none() || !active.all(|plan| {
        ["plan_id", "name"].iter().filter_map(|key| plan[*key].as_str()).any(|value| {
            let id = value.to_ascii_lowercase();
            id.contains("start-plan") || id.contains("start plan")
        })
    }) {
        return Err("ZCODE_START_PLAN_REQUIRED");
    }
    let jwt = account.zcode_jwt_token.trim();
    if jwt.is_empty() {
        return Err("ZCODE_OAUTH_REAUTH_REQUIRED");
    }
    Ok(jwt)
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
    Ok(crate::modules::provider_account_index::storage_path::<ZcodeAccountIndex>(&index_path()?)?
        .to_string_lossy().into_owned())
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
    crate::modules::provider_account_index::checked(
        &index_path()?,
        crate::modules::provider_index_maintenance::schedule,
        |_| None,
        |_| false,
    )
}

fn save_index(index: &ZcodeAccountIndex) -> Result<(), String> {
    crate::modules::provider_account_index::save(&index_path()?, index)
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

pub fn upsert_account(value: ZcodeAccount) -> Result<ZcodeAccount, String> {
    let _guard = ZCODE_ACCOUNT_LOCK
        .lock()
        .map_err(|_| "获取 ZCode 账号锁失败".to_string())?;
    upsert_account_locked(value)
}

fn upsert_account_locked(mut value: ZcodeAccount) -> Result<ZcodeAccount, String> {
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
        value.team_plan = None;
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
        let same_plan_scope = value.team_plan == existing.team_plan;
        if same_plan_scope && value.plan_type.is_none() {
            value.plan_type = existing.plan_type;
        }
        if same_plan_scope && value.quota_raw.is_none() {
            value.quota_total = existing.quota_total;
            value.quota_used = existing.quota_used;
            value.quota_remaining = existing.quota_remaining;
            value.quota_reset_at = existing.quota_reset_at;
            if value.quota_query_last_error.is_none() {
                value.quota_query_last_error = existing.quota_query_last_error;
                value.quota_query_last_error_at = existing.quota_query_last_error_at;
            }
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

fn decrypt_credential(value: &str, home_dir: &Path) -> Result<String, String> {
    let Some(payload) = value.strip_prefix(CREDENTIAL_PREFIX) else {
        return if value.starts_with("enc:") {
            Err("不支持的 ZCode 凭据密文版本".to_string())
        } else {
            Ok(value.to_string())
        };
    };
    decrypt_payload(payload, &credential_key(home_dir)).or_else(|error| {
        if std::env::var_os("ZCODE_CREDENTIAL_SECRET").is_none() {
            return Err(error);
        }
        let fallback = credential_key_from_fallback(platform_name(), home_dir, &username());
        decrypt_payload(payload, &fallback)
    })
}

#[cfg(test)]
fn decrypt_credential_with_key(value: &str, key: &[u8; 32]) -> Result<String, String> {
    decrypt_payload(value.strip_prefix(CREDENTIAL_PREFIX).unwrap(), key)
}

fn decrypt_payload(payload: &str, key: &[u8; 32]) -> Result<String, String> {
    let [nonce, tag, encrypted] = payload.split('.').collect::<Vec<_>>()[..] else {
        return Err("ZCode 凭据密文格式无效".to_string());
    };
    let nonce = decode_component(nonce)?;
    let tag = decode_component(tag)?;
    let mut encrypted = decode_component(encrypted)?;
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

fn zcode_keyring_account(credentials_path: &Path) -> Result<String, String> {
    // Use the same lexical absolute path as Node path.resolve. Resolving symlinks
    // could change the name of the keyring entry created by ZCode.
    let absolute = std::path::absolute(credentials_path)
        .map_err(|_| "无法解析 ZCode 凭据文件路径".to_string())?;
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::CurDir => {}
            _ => resolved.push(component),
        }
    }
    let resolved = resolved.to_string_lossy();
    #[cfg(target_os = "windows")]
    let resolved = resolved.replace('/', "\\");
    Ok(format!("{:x}", Sha256::digest(resolved.as_bytes())))
}

fn zcode_v2_system_key(credentials_path: &Path) -> Result<[u8; 32], String> {
    let account = zcode_keyring_account(credentials_path)?;
    let entry = keyring::Entry::new(CREDENTIAL_KEYRING_SERVICE, &account)
        .map_err(|_| "无法打开 ZCode 的系统凭据库条目".to_string())?;
    let value = entry.get_password().map_err(|error| match error {
        keyring::Error::NoEntry => "系统凭据库中缺少该 ZCode 文件对应的密钥".to_string(),
        _ => "无法读取 ZCode 系统凭据密钥，请检查凭据库是否解锁".to_string(),
    })?;
    decode_component(value.trim())?
        .try_into()
        .map_err(|_| "ZCode 系统凭据密钥长度无效".to_string())
}

fn zcode_v2_managed_key(credentials_path: &Path) -> Result<[u8; 32], String> {
    let secret = std::env::var("ZCODE_CREDENTIAL_SECRET")
        .ok()
        .filter(|value| !value.is_empty());
    let explicit_file = std::env::var_os("ZCODE_CREDENTIAL_SECRET_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    if let Some(secret) = secret {
        if explicit_file.is_some() {
            return Err("ZCode 的环境密钥和密钥文件不能同时配置".to_string());
        }
        return Ok(Sha256::digest(secret.as_bytes()).into());
    }
    let path = explicit_file
        .clone()
        .unwrap_or_else(|| credentials_path.with_file_name(CREDENTIAL_SECRET_FILE));
    let metadata = fs::metadata(&path)
        .map_err(|_| "找不到可读的 ZCode 凭据密钥文件".to_string())?;
    if !metadata.is_file() || !(32..=4096).contains(&metadata.len()) {
        return Err("ZCode 凭据密钥文件必须为 32–4096 字节".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if explicit_file.is_none() && metadata.permissions().mode() & 0o077 != 0 {
            return Err("ZCode 的默认凭据密钥文件权限必须仅限当前用户".to_string());
        }
    }
    let mut content = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(4097).read_to_end(&mut content))
        .map_err(|_| "无法读取 ZCode 凭据密钥文件".to_string())?;
    if !(32..=4096).contains(&content.len()) {
        return Err("ZCode 凭据密钥文件必须为 32–4096 字节".to_string());
    }
    Ok(Sha256::digest(String::from_utf8_lossy(&content).as_bytes()).into())
}

fn decrypt_credential_v2_payload(
    value: &str,
    path: &Path,
    cached_key: &mut Option<(&'static str, [u8; 32])>,
) -> Result<String, String> {
    let [source, fingerprint, payload] = value.split(':').collect::<Vec<_>>()[..] else {
        return Err("ZCode v2 凭据密文格式无效".to_string());
    };
    if fingerprint.len() != 24
        || !fingerprint.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("ZCode v2 凭据的密钥指纹无效".to_string());
    }
    let (key_source, key) = match *cached_key {
        Some((cached_source, key)) if cached_source == source => (cached_source, key),
        _ => match source {
            "os" => ("os", zcode_v2_system_key(path)?),
            "env" => ("env", zcode_v2_managed_key(path)?),
            _ => return Err("不支持的 ZCode v2 密钥来源".to_string()),
        },
    };
    if &format!("{:x}", Sha256::digest(key))[..24] != fingerprint {
        return Err("ZCode v2 凭据的密钥指纹不匹配".to_string());
    }
    *cached_key = Some((key_source, key));
    decrypt_payload(payload, &key)
}

pub fn encrypt_credential(value: &str, home_dir: &Path) -> Result<String, String> {
    encrypt_payload(value, &credential_key(home_dir))
        .map(|payload| format!("{CREDENTIAL_PREFIX}{payload}"))
}

fn encrypt_payload(value: &str, key: &[u8; 32]) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(key)
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
        "{}.{}.{}",
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
    Ok(resolve_default_v2_dir_for_home(&home))
}

fn resolve_default_v2_dir_for_home(home: &Path) -> PathBuf {
    let default = home.join(".zcode/v2");
    let setting_path = settings_path_for_home(home);
    if let Ok(content) = fs::read_to_string(setting_path) {
        if let Ok(value) = serde_json::from_str::<Value>(&content) {
            if let Some(base) = value
                .get("dataBaseDir")
                .and_then(Value::as_str)
                .and_then(|v| normalize_string(Some(v)))
            {
                return PathBuf::from(base).join(".zcode/v2");
            }
        }
    }
    default
}

pub fn default_credentials_path() -> Result<PathBuf, String> {
    Ok(resolve_default_v2_dir()?.join(CREDENTIALS_FILE))
}

fn default_config_path() -> Result<PathBuf, String> {
    Ok(resolve_default_v2_dir()?.join(CONFIG_FILE))
}

fn settings_path_for_home(home: &Path) -> PathBuf {
    home.join(".zcode/v2").join(SETTINGS_FILE)
}

pub(super) fn default_settings_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    Ok(settings_path_for_home(&home))
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

pub(super) fn settings_path_for_instance_root(root: &Path) -> PathBuf {
    settings_path_for_home(&root.join("data"))
}

#[cfg(test)]
fn decrypt_credential_v2(value: &str, path: &Path) -> Result<String, String> {
    let payload = value.strip_prefix(CREDENTIAL_V2_PREFIX)
        .ok_or_else(|| "ZCode v2 凭据密文格式无效".to_string())?;
    decrypt_credential_v2_payload(payload, path, &mut None)
}

#[cfg(test)]
fn decrypted_value(
    values: &Map<String, Value>,
    name: &str,
    credentials_path: &Path,
    home: &Path,
) -> Result<Option<String>, String> {
    values
        .get(name)
        .and_then(Value::as_str)
        .map(|value| {
            if value.starts_with(CREDENTIAL_V2_PREFIX) {
                decrypt_credential_v2(value, credentials_path)
            } else {
                decrypt_credential(value, home)
            }
        })
        .transpose()
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .and_then(|value| normalize_string(Some(value)))
}

fn bigmodel_team_plan_from_settings(path: &Path) -> Option<ZcodeTeamPlan> {
    let settings = read_json_map(path).ok()?;
    if settings.get("providerFamilyDomain")?.as_str()? != "bigmodel" {
        return None;
    }
    let selection = settings.get("providerFamilyConnectionSelections")?.get("bigmodel")?;
    if selection.get("kind")?.as_str()? != "team-coding-plan" {
        return None;
    }
    Some(ZcodeTeamPlan {
        organization_id: value_string(selection, &["organizationId"])?,
        project_id: value_string(selection, &["projectId"])?,
    })
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
    account_from_credential_map(&values, path, home)
}

fn account_from_credential_map(
    values: &Map<String, Value>,
    path: &Path,
    home: &Path,
) -> Result<ZcodeAccount, String> {
    let mut cached_key = None;
    let mut decrypt = |name: &str| {
        values.get(name).and_then(Value::as_str).map(|value| {
            if let Some(payload) = value.strip_prefix(CREDENTIAL_V2_PREFIX) {
                decrypt_credential_v2_payload(payload, path, &mut cached_key)
            } else {
                decrypt_credential(value, home)
            }
        }).transpose()
    };
    let provider = decrypt(ACTIVE_PROVIDER_KEY)?
        .ok_or_else(|| "ZCode 本地凭据缺少 active provider".to_string())?;
    let provider = normalize_provider(&provider)?;
    let access_token = decrypt(&format!("oauth:{}:access_token", provider))?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "ZCode 本地凭据缺少 access token".to_string())?;
    let refresh_token = decrypt(&format!("oauth:{}:refresh_token", provider))?;
    let zcode_jwt_token = decrypt(ZCODE_JWT_KEY)?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "ZCode 本地凭据缺少 zcode JWT".to_string())?;
    let user_info_text = decrypt(&format!("oauth:{}:user_info", provider))?;
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
    let team_plan = if provider == "bigmodel" {
        let settings_path = if path == resolve_default_v2_dir_for_home(home).join(CREDENTIALS_FILE) {
            settings_path_for_home(home)
        } else {
            path.with_file_name(SETTINGS_FILE)
        };
        bigmodel_team_plan_from_settings(&settings_path)
    } else {
        None
    };
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
        team_plan,
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
        team_plan: None,
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

fn api_key_accounts_from_values(root: &Map<String, Value>) -> Result<Vec<ZcodeAccount>, String> {
    ["zai", "bigmodel"]
        .into_iter()
        .filter_map(|provider| {
            api_key_from_provider_config(root, provider)
                .map(|api_key| api_key_account(provider, &api_key, None))
        })
        .collect()
}

fn api_key_accounts_from_config(path: &Path) -> Result<Vec<ZcodeAccount>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let root = read_json_map(path)?;
    api_key_accounts_from_values(&root)
}

pub async fn import_from_local() -> Result<Vec<ZcodeAccount>, String> {
    let mut imported = Vec::new();
    let oauth = tokio::task::spawn_blocking(|| {
        let path = default_credentials_path()?;
        path.exists().then(|| account_from_credentials_path(&path)).transpose()
    }).await.map_err(|_| "读取 ZCode 本地凭据失败".to_string())?;
    let oauth_error = match oauth {
        Ok(Some(account)) => {
            let account = upsert_account(account)?;
            imported.push(refresh_account_quota(&account.id).await.unwrap_or(account));
            None
        }
        Ok(None) => None,
        Err(error) => Some(error),
    };
    for account in api_key_accounts_from_config(&default_config_path()?)? {
        imported.push(upsert_account(account)?);
    }
    if imported.is_empty() {
        return Err(oauth_error.unwrap_or_else(|| "未找到可导入的 ZCode OAuth 凭据或 API Key".to_string()));
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

fn bigmodel_user_profile(account: &ZcodeAccount) -> Result<Value, String> {
    let raw = account
        .user_info_raw
        .as_ref()
        .filter(|value| value.is_object());
    if let Some(profile) = raw.filter(|profile| {
        ["id", "username", "displayName"]
            .iter()
            .all(|key| value_string(profile, &[*key]).is_some())
    }) {
        return Ok(profile.clone());
    }

    let jwt_subject = account
        .zcode_jwt_token
        .split('.')
        .nth(1)
        .and_then(|payload| URL_SAFE_NO_PAD.decode(payload).ok())
        .and_then(|payload| serde_json::from_slice::<Value>(&payload).ok())
        .and_then(|claims| value_string(&claims, &["sub"]));
    let id = jwt_subject
        .or_else(|| normalize_string(account.user_id.as_deref()))
        .ok_or_else(|| "ZCode BigModel 登录资料缺少用户 ID，请重新授权后切换".to_string())?;
    let display_name = raw
        .and_then(|profile| {
            ["customerName", "nickName", "name"]
                .iter()
                .find_map(|key| value_string(profile, &[*key]))
        })
        .or_else(|| normalize_string(account.display_name.as_deref()))
        .unwrap_or_else(|| id.clone());
    let mut profile = json!({
        "id": id,
        "username": display_name,
        "displayName": display_name,
        "email": account.email,
    });
    if let Some(avatar) = normalize_string(account.avatar_url.as_deref()) {
        profile["avatarUrl"] = Value::String(avatar);
    }
    if let Some(raw) = raw {
        profile["rawProfile"] = raw.clone();
    }
    Ok(profile)
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
    preserve_family_domain: bool,
) -> Result<(), String> {
    let provider = normalize_provider(provider)?;
    let mut root = read_json_map(path)?;
    if !preserve_family_domain {
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
    }
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
    let provider = normalize_provider(&account.provider)?;
    let profile = if provider == "bigmodel" {
        bigmodel_user_profile(account)?
    } else {
        official_user_info(account)
    };
    let mut values = read_json_map(path)?;
    let providers: &[&str] = if provider == "bigmodel" {
        &["zai", "bigmodel", "zai_cn"]
    } else {
        &["zai", "bigmodel"]
    };
    for provider in providers {
        for suffix in ["access_token", "refresh_token", "user_info"] {
            values.remove(&format!("oauth:{}:{}", provider, suffix));
        }
    }
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
        &serde_json::to_string(&profile)
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
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    write_default_account_for_home(&value, &home, preserve_bigmodel_oauth_domain(&value))?;
    value.last_used = now_ts();
    let value = upsert_account(value)?;
    let mut index = load_index()?;
    index.current_account_id = Some(value.id.clone());
    save_index(&index)?;
    Ok(value)
}

fn write_default_account_for_home(
    account: &ZcodeAccount,
    home: &Path,
    preserve_family_domain: bool,
) -> Result<(), String> {
    let data_dir = resolve_default_v2_dir_for_home(home);
    if account.auth_mode == ZcodeAuthMode::ApiKey {
        write_api_key_to_config_path(account, &data_dir.join(CONFIG_FILE))?;
    } else {
        write_account_to_credentials_path_with_home(
            account,
            &data_dir.join(CREDENTIALS_FILE),
            home,
        )?;
    }
    write_auth_mode_to_settings_path(
        &account.provider,
        account.auth_mode,
        &settings_path_for_home(home),
        preserve_family_domain,
    )
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
        preserve_bigmodel_oauth_domain(&value),
    )?;
    Ok(value)
}

fn preserve_bigmodel_oauth_domain(account: &ZcodeAccount) -> bool {
    if account.auth_mode != ZcodeAuthMode::Oauth || account.provider != "bigmodel" {
        return false;
    }
    let configured_path = crate::modules::config::get_user_config().zcode_app_path;
    let version =
        crate::modules::client_version::detect_client_version("ZCode", Some(&configured_path));
    let legacy_version = version.as_deref().and_then(|version| {
        let mut parts = version.split('.');
        Some((
            parts.next()?.parse::<u32>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
        ))
    });
    // 3.18 起，活跃 OAuth 账号和 provider family 由 ZCode 分开管理。
    !legacy_version.is_some_and(|version| version < (3, 18))
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

/// ZCode 官方客户端会随请求附带的稳定客户端身份头。
///
/// 供本地 Provider Gateway 的 OAuth 路径复用，避免用 Cockpit 内部账号 ID
/// 捏造 `x-device-mid` / `x-session-id` 等设备或会话身份。
pub(crate) fn zcode_client_headers() -> HeaderMap {
    zcode_source_headers(&detect_app_version())
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

    if value.team_plan.is_some() {
        apply_team_quota(value, &payload["data"])?;
    } else {
        apply_personal_quota(value, payload);
    }
    value.usage_updated_at = Some(now_ms());
    value.quota_query_last_error = None;
    value.quota_query_last_error_at = None;
    Ok(())
}

fn apply_team_quota(value: &mut ZcodeAccount, data: &Value) -> Result<(), String> {
    let usage = &data["total_usage"];
    let (Some(total), Some(used), Some(remaining)) = (
        usage["limit"].as_f64(),
        usage["used"].as_f64(),
        usage["remaining"].as_f64(),
    ) else {
        return Err("ZCode 团队额度接口未返回完整的额度数据".to_string());
    };
    let reset_at = unix_seconds(data.get("next_refresh_at"));
    value.plan_type = value_string(data, &["level"]);
    value.quota_total = Some(total);
    value.quota_used = Some(used);
    value.quota_remaining = Some(remaining);
    value.quota_reset_at = reset_at;
    value.subscription_raw = None;
    value.quota_raw = Some(json!([{
        "entitlement_id": "zcode-team",
        "show_name": "ZCode Team",
        "total_units": total,
        "used_units": used,
        "remaining_units": remaining,
        "next_refresh_at": reset_at,
    }]));
    Ok(())
}

fn apply_personal_quota(value: &mut ZcodeAccount, payload: Value) {
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
}

fn quota_request(
    client: &reqwest::Client,
    value: &ZcodeAccount,
    app_version: &str,
    source_headers: HeaderMap,
) -> Result<reqwest::RequestBuilder, String> {
    let request = if let Some(team) = &value.team_plan {
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", value.access_token))
            .map_err(|_| "ZCode 团队额度授权格式无效".to_string())?;
        authorization.set_sensitive(true);
        client
            .get(MCP_USAGE_URL)
            .header("Bigmodel-Organization", &team.organization_id)
            .header("Bigmodel-Project", &team.project_id)
            .header("Bigmodel-Target-Type", "TEAM")
            .header("X-Bigmodel-Authorization", authorization)
    } else {
        client
            .get(BILLING_BALANCE_URL)
            .query(&[("app_version", app_version)])
            .headers(source_headers)
    };
    Ok(request.bearer_auth(&value.zcode_jwt_token))
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
    let response = match quota_request(&reqwest::Client::new(), &value, &app_version, source_headers)
    {
        Ok(request) => request
            .send()
            .await
            .map_err(|error| format!("请求 ZCode 配额失败: {}", error)),
        Err(error) => Err(error),
    };
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

    let _guard = ZCODE_ACCOUNT_LOCK
        .lock()
        .map_err(|_| "获取 ZCode 账号锁失败".to_string())?;
    let mut current = load_account(account_id).ok_or_else(|| "ZCode 账号不存在".to_string())?;
    if current.auth_mode != value.auth_mode
        || current.provider != value.provider
        || current.team_plan != value.team_plan
        || current.zcode_jwt_token != value.zcode_jwt_token
        || current.access_token != value.access_token
    {
        return Err("ZCode 账号凭据或套餐已变更，请重新刷新额度".to_string());
    }
    match payload.and_then(|payload| apply_quota_payload(&mut current, payload)) {
        Ok(()) => upsert_account_locked(current),
        Err(error) => {
            current.quota_query_last_error = Some(error.clone());
            current.quota_query_last_error_at = Some(now_ms());
            upsert_account_locked(current)?;
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

fn parse_import_accounts(
    content: &str,
    source_path: Option<&Path>,
) -> Result<Vec<ZcodeAccount>, String> {
    let root: Value =
        serde_json::from_str(content).map_err(|error| format!("JSON 解析失败: {}", error))?;
    let items: Vec<Value> = match root {
        Value::Array(items) => items,
        Value::Object(mut object) => {
            if let Some(Value::Array(accounts)) = object.remove("accounts") {
                accounts
            } else if object.contains_key(ZCODE_JWT_KEY)
                || object.keys().any(|key| key.starts_with("oauth:"))
            {
                let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
                let default = resolve_default_v2_dir_for_home(&home).join(CREDENTIALS_FILE);
                let path = source_path.unwrap_or(&default);
                return account_from_credential_map(&object, path, &home)
                    .map(|account| vec![account]);
            } else if object.get("provider").is_some_and(Value::is_object) {
                let accounts = api_key_accounts_from_values(&object)?;
                if accounts.is_empty() {
                    return Err("ZCode 配置文件中未找到可导入的 API Key".to_string());
                }
                return Ok(accounts);
            } else {
                vec![Value::Object(object)]
            }
        }
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

pub fn import_from_json(
    content: &str,
    source_path: Option<&Path>,
) -> Result<Vec<ZcodeAccount>, String> {
    parse_import_accounts(content, source_path)?
        .into_iter()
        .map(upsert_account)
        .collect()
}

pub fn import_from_file(path: &Path) -> Result<Vec<ZcodeAccount>, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("无法读取 ZCode 导入文件: {}", error))?;
    import_from_json(&content, Some(path))
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
            team_plan: None,
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

    fn seal_v2_payload(value: &str, key: &[u8; 32]) -> String {
        use aes_gcm::aead::Aead;
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let cipher = Aes256Gcm::new_from_slice(key).unwrap();
        let encrypted = cipher.encrypt(Nonce::from_slice(&nonce), value.as_bytes()).unwrap();
        let tag = encrypted[encrypted.len() - 16..].to_vec();
        let body = &encrypted[..encrypted.len() - 16];
        [
            URL_SAFE_NO_PAD.encode(nonce),
            URL_SAFE_NO_PAD.encode(tag),
            URL_SAFE_NO_PAD.encode(body),
        ]
        .join(".")
    }

    fn v2_env_ciphertext(value: &str, secret: &str, fingerprint: Option<String>) -> String {
        let key: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
        let fingerprint =
            fingerprint.unwrap_or_else(|| format!("{:x}", Sha256::digest(key))[..24].to_owned());
        format!(
            "{}{}:{}:{}",
            CREDENTIAL_V2_PREFIX, "env", fingerprint,
            seal_v2_payload(value, &key)
        )
    }

    #[test]
    fn v2_payload_round_trips_with_fixed_key() {
        let key: [u8; 32] = Sha256::digest(b"fixture-secret").into();
        let payload = seal_v2_payload("官方 v2 明文", &key);
        assert_eq!(decrypt_payload(&payload, &key).unwrap(), "官方 v2 明文");
    }

    #[test]
    fn v2_decrypt_reads_env_source_from_credential_secret_file() {
        let root = make_temp_dir("zcode-v2-env-secret");
        let credentials_path = root.join(CREDENTIALS_FILE);
        let secret = "a".repeat(48);
        let secret_path = root.join(CREDENTIAL_SECRET_FILE);
        fs::write(&secret_path, &secret).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let value = v2_env_ciphertext("token-value", &secret, None);
        assert_eq!(
            decrypt_credential_v2(&value, &credentials_path).unwrap(),
            "token-value"
        );
        // 指纹不匹配（密钥已更换）必须报错，不能静默返回错误明文。
        let mut mismatched = v2_env_ciphertext("token-value", &secret, None).into_bytes();
        // 把指纹第一个字符替换成不同的 hex 字符。
        let index = CREDENTIAL_V2_PREFIX.len() + "env".len() + 1;
        mismatched[index] = if mismatched[index] == b'0' { b'1' } else { b'0' };
        let error = decrypt_credential_v2(std::str::from_utf8(&mismatched).unwrap(), &credentials_path)
            .unwrap_err();
        assert!(error.contains("指纹不匹配"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v2_decrypt_rejects_bad_format_and_unknown_source() {
        let path = Path::new("/tmp/zcode-v2-format-test/credentials.json");
        assert!(decrypt_credential_v2("enc:v2:only:two", path).is_err());
        assert!(decrypt_credential_v2("enc:v2:os::payload", path).is_err());
        assert!(decrypt_credential_v2("enc:v2:bad:source:value", path).is_err());
        let bad_fingerprint = format!(
            "{}{}:{}:{}",
            CREDENTIAL_V2_PREFIX, "os", "zz", "payload"
        );
        assert!(decrypt_credential_v2(&bad_fingerprint, path).is_err());
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
    fn team_scope_is_imported_with_the_matching_credentials_profile() {
        let home = tempfile::tempdir().unwrap();
        let custom_data = tempfile::tempdir().unwrap();
        let settings_path = settings_path_for_home(home.path());
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        let mut settings = json!({
            "dataBaseDir": custom_data.path(),
            "providerFamilyDomain": "bigmodel",
            "providerFamilyConnectionSelections": {
                "bigmodel": {
                    "kind": "team-coding-plan",
                    "organizationId": "org-1",
                    "projectId": "project-1"
                }
            }
        });
        fs::write(&settings_path, settings.to_string()).unwrap();

        let mut team_account = sample_account();
        team_account.provider = "bigmodel".into();
        let credentials = resolve_default_v2_dir_for_home(home.path()).join(CREDENTIALS_FILE);
        write_account_to_credentials_path_with_home(&team_account, &credentials, home.path())
            .unwrap();
        let saved = account_from_credentials_path_with_home(&credentials, home.path()).unwrap();
        let round_trip: ZcodeAccount =
            serde_json::from_value(serde_json::to_value(saved).unwrap()).unwrap();
        assert_eq!(
            round_trip.team_plan,
            Some(ZcodeTeamPlan {
                organization_id: "org-1".into(),
                project_id: "project-1".into()
            })
        );

        let other_profile = tempfile::tempdir().unwrap();
        let other_credentials = other_profile.path().join(CREDENTIALS_FILE);
        write_account_to_credentials_path_with_home(&team_account, &other_credentials, home.path())
            .unwrap();
        let other = account_from_credentials_path_with_home(&other_credentials, home.path())
            .unwrap();
        assert!(other.team_plan.is_none());

        settings["providerFamilyDomain"] = json!("zai");
        fs::write(&settings_path, settings.to_string()).unwrap();
        let mismatched =
            account_from_credentials_path_with_home(&credentials, home.path()).unwrap();
        assert!(mismatched.team_plan.is_none());

        settings["providerFamilyDomain"] = json!("bigmodel");
        settings["providerFamilyConnectionSelections"]["bigmodel"]["kind"] =
            json!("personal-coding-plan");
        fs::write(&settings_path, settings.to_string()).unwrap();
        let personal = account_from_credentials_path_with_home(&credentials, home.path()).unwrap();
        assert!(personal.team_plan.is_none());
    }

    #[test]
    fn default_switch_writes_settings_to_home_and_credentials_to_custom_data() {
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let setting = settings_path_for_home(home.path());
        fs::create_dir_all(setting.parent().unwrap()).unwrap();
        fs::write(
            &setting,
            json!({"dataBaseDir": data.path(), "locale":"zh-CN", "providerFamilyDomain":"bigmodel"})
                .to_string(),
        )
        .unwrap();

        let oauth = sample_account();
        write_default_account_for_home(&oauth, home.path(), false).unwrap();
        let active_data = data.path().join(".zcode/v2");
        let credentials = active_data.join(CREDENTIALS_FILE);
        let restored = account_from_credentials_path_with_home(&credentials, home.path()).unwrap();
        assert_eq!(restored.access_token, oauth.access_token);
        assert_eq!(
            read_json_map(&setting).unwrap()["providerFamilyDomain"],
            "zai"
        );
        assert!(!active_data.join(SETTINGS_FILE).exists());
        assert!(!setting.with_file_name(CREDENTIALS_FILE).exists());

        let api_key = api_key_account("bigmodel", "fixture-key", None).unwrap();
        write_default_account_for_home(&api_key, home.path(), false).unwrap();
        let settings = read_json_map(&setting).unwrap();
        assert_eq!(settings["providerFamilyDomain"], "bigmodel");
        assert_eq!(settings["modelProviderFamilyModes"]["bigmodel"], "apiKey");
        assert_eq!(settings["locale"], "zh-CN");
        assert_eq!(settings["dataBaseDir"], json!(data.path()));
        assert_eq!(
            read_json_map(&active_data.join(CONFIG_FILE)).unwrap()["provider"]
                [api_key_provider_spec("bigmodel").0]["options"]["apiKey"],
            "fixture-key"
        );
    }

    #[test]
    fn bigmodel_oauth_switch_writes_zcode_profile_and_keeps_existing_family_domain() {
        let home = tempfile::tempdir().unwrap();
        let settings_path = settings_path_for_home(home.path());
        let settings = json!({
            "providerFamilyDomain": "zai",
            "providerFamilyDomainUpdatedAt": 123,
            "providerFamilyDomainMigrated": false,
            "modelProviderFamilyModes": {"zai": "apiKey", "bigmodel": "apiKey"},
        });
        write_json_map(
            &settings_path,
            settings.as_object().unwrap().clone(),
            SETTINGS_FILE,
        )
        .unwrap();
        let credentials_path = resolve_default_v2_dir_for_home(home.path()).join(CREDENTIALS_FILE);
        let old_credentials = json!({
            "oauth:zai_cn:access_token": "obsolete",
            "oauth:zai_cn:refresh_token": "obsolete",
            "oauth:zai_cn:user_info": "obsolete",
            "unrelated-key": "keep",
        });
        write_json_map(
            &credentials_path,
            old_credentials.as_object().unwrap().clone(),
            CREDENTIALS_FILE,
        )
        .unwrap();
        let mut account = sample_account();
        account.provider = "bigmodel".into();
        account.user_id = Some("business-user".into());
        account.zcode_jwt_token = format!(
            "e30.{}.fixture",
            URL_SAFE_NO_PAD.encode(br#"{"sub":"zcode-user"}"#),
        );
        let business_profile = json!({
            "customerName": "",
            "nickName": "Fixture nickname",
            "email": null,
            "id": 123,
        });
        account.user_info_raw = Some(business_profile.clone());

        write_default_account_for_home(&account, home.path(), true).unwrap();

        let saved = read_json_map(&credentials_path).unwrap();
        assert_eq!(
            decrypted_value(&saved, ACTIVE_PROVIDER_KEY, &credentials_path, home.path())
                .unwrap()
                .as_deref(),
            Some("bigmodel"),
        );
        assert!(!saved.keys().any(|name| name.starts_with("oauth:zai_cn:")));
        assert_eq!(saved["unrelated-key"], "keep");
        let profile = decrypted_value(&saved, "oauth:bigmodel:user_info", &credentials_path, home.path())
            .unwrap()
            .unwrap();
        let profile: Value = serde_json::from_str(&profile).unwrap();
        assert_eq!(profile["id"], "zcode-user");
        assert_eq!(profile["username"], "Fixture nickname");
        assert_eq!(profile["displayName"], "Fixture nickname");
        assert_eq!(profile["rawProfile"], business_profile);
        assert_eq!(profile["avatarUrl"], account.avatar_url.as_deref().unwrap());
        let restored =
            account_from_credentials_path_with_home(&credentials_path, home.path()).unwrap();
        assert_eq!(restored.access_token, account.access_token);
        assert_eq!(restored.zcode_jwt_token, account.zcode_jwt_token);
        assert_eq!(restored.user_id.as_deref(), Some("zcode-user"));

        let saved_settings = read_json_map(&settings_path).unwrap();
        assert_eq!(saved_settings["providerFamilyDomain"], "zai");
        assert_eq!(saved_settings["providerFamilyDomainUpdatedAt"], 123);
        assert_eq!(saved_settings["providerFamilyDomainMigrated"], false);
        assert_eq!(
            saved_settings["modelProviderFamilyModes"],
            json!({"zai": "apiKey", "bigmodel": "oauth"})
        );
    }

    #[test]
    fn bigmodel_preserves_an_existing_official_profile() {
        let mut account = sample_account();
        account.provider = "bigmodel".into();
        let profile = json!({
            "id": "official-id",
            "username": "official-name",
            "displayName": "Official display",
            "rawProfile": {"zcodeProfileSchemaVersion": 2},
        });
        account.user_info_raw = Some(profile.clone());
        assert_eq!(bigmodel_user_profile(&account).unwrap(), profile);
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
        write_auth_mode_to_settings_path("zai", ZcodeAuthMode::ApiKey, &settings_path, false)
            .unwrap();

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
    fn personal_and_team_quota_requests_use_their_respective_endpoints() {
        let client = reqwest::Client::new();
        let mut account = sample_account();
        let request = quota_request(&client, &account, "3.16.3", HeaderMap::new())
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            format!("{}?app_version=3.16.3", BILLING_BALANCE_URL)
        );
        assert_eq!(request.headers()["authorization"], "Bearer zcode-jwt-token");
        assert!(!request.headers().contains_key("x-bigmodel-authorization"));

        account.provider = "bigmodel".into();
        account.team_plan = Some(ZcodeTeamPlan {
            organization_id: "org-1".into(),
            project_id: "project-1".into(),
        });
        let request = quota_request(&client, &account, "3.16.3", HeaderMap::new())
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.url().as_str(), MCP_USAGE_URL);
        assert_eq!(request.headers()["authorization"], "Bearer zcode-jwt-token");
        assert_eq!(request.headers()["x-bigmodel-authorization"], "Bearer access-token");
        assert!(request.headers()["x-bigmodel-authorization"].is_sensitive());
        assert_eq!(request.headers()["bigmodel-organization"], "org-1");
        assert_eq!(request.headers()["bigmodel-project"], "project-1");
        assert_eq!(request.headers()["bigmodel-target-type"], "TEAM");
    }

    #[test]
    fn team_quota_shows_level_and_retains_values_on_incomplete_response() {
        let mut account = sample_account();
        account.provider = "bigmodel".into();
        account.team_plan = Some(ZcodeTeamPlan {
            organization_id: "org-1".into(),
            project_id: "project-1".into(),
        });
        apply_quota_payload(
            &mut account,
            json!({
                "code": 0,
                "data": {
                    "level": "pro",
                    "total_usage": {"used": 0, "limit": 1000, "remaining": 1000},
                    "next_refresh_at": 1_900_000_000,
                }
            }),
        )
        .unwrap();
        assert_eq!(account.plan_type.as_deref(), Some("pro"));
        assert_eq!(account.quota_total, Some(1000.0));
        assert_eq!(account.quota_used, Some(0.0));
        assert_eq!(account.quota_remaining, Some(1000.0));
        assert_eq!(account.quota_reset_at, Some(1_900_000_000));
        let report = account.quota_raw.clone();
        assert_eq!(report.as_ref().unwrap()[0]["total_units"], 1000.0);
        assert!(account.quota_query_last_error.is_none());

        assert!(apply_quota_payload(
            &mut account,
            json!({"code": 0, "data": {"total_usage": {"used": 0, "limit": 100}}}),
        )
        .is_err());
        assert_eq!(account.quota_raw, report);
        assert_eq!(account.quota_total, Some(1000.0));
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
            None,
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].id.is_empty());

        let mut invalid = serde_json::to_value(account).unwrap();
        invalid["zcode_jwt_token"] = Value::String(String::new());
        let error = parse_import_accounts(&invalid.to_string(), None).unwrap_err();
        assert!(error.contains("缺少必要 Token"));

        let api_key = api_key_account("bigmodel", "bigmodel-test-key", None).unwrap();
        let parsed = parse_import_accounts(&serde_json::to_string(&api_key).unwrap(), None).unwrap();
        assert_eq!(parsed[0].auth_mode, ZcodeAuthMode::ApiKey);
        assert_eq!(parsed[0].api_key.as_deref(), Some("bigmodel-test-key"));
    }

    fn fixture_v2_container(source: &str, key: &[u8; 32]) -> Value {
        let digest = format!("{:x}", Sha256::digest(key));
        let encrypt = |value: &str| {
            let payload = encrypt_payload(value, key).unwrap();
            format!("{CREDENTIAL_V2_PREFIX}{source}:{}:{payload}", &digest[..24])
        };
        json!({
            ACTIVE_PROVIDER_KEY: encrypt("bigmodel"),
            "oauth:bigmodel:access_token": encrypt("fixture-access"),
            "oauth:bigmodel:refresh_token": encrypt("fixture-refresh"),
            "oauth:bigmodel:user_info": encrypt(r#"{"id":"fixture-zcode-user","username":"fixture","displayName":"Fixture"}"#),
            ZCODE_JWT_KEY: encrypt("e30.eyJzdWIiOiJmaXh0dXJlLXpjb2RlLXVzZXIifQ.fixture"),
        })
    }

    #[test]
    fn v2_import_reads_secret_beside_selected_file_and_preserves_switch_profile() {
        let source = tempfile::tempdir().unwrap();
        let selected_path = source.path().join(CREDENTIALS_FILE);
        let secret = b"zcode-test-credential-file-secret";
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        {
            use std::io::Write;
            options.open(source.path().join(CREDENTIAL_SECRET_FILE)).unwrap()
                .write_all(secret).unwrap();
        }

        let key = Sha256::digest(secret).into();
        let values = fixture_v2_container("env", &key);
        fs::write(&selected_path, values.to_string()).unwrap();
        let target = tempfile::tempdir().unwrap();
        let local = account_from_credentials_path_with_home(&selected_path, target.path()).unwrap();
        let selected = parse_import_accounts(&values.to_string(), Some(&selected_path)).unwrap();
        assert_eq!(selected[0].id, local.id);
        assert_eq!(selected[0].user_id.as_deref(), Some("fixture-zcode-user"));
        assert_eq!(selected[0].refresh_token.as_deref(), Some("fixture-refresh"));

        write_default_account_for_home(&selected[0], target.path(), true).unwrap();
        let injected_path = resolve_default_v2_dir_for_home(target.path()).join(CREDENTIALS_FILE);
        let injected = account_from_credentials_path_with_home(&injected_path, target.path()).unwrap();
        assert_eq!(injected.user_info_raw, local.user_info_raw);
        assert_eq!(injected.access_token, "fixture-access");

        let wrong_key = fixture_v2_container("env", &[0; 32]);
        let error = parse_import_accounts(&wrong_key.to_string(), Some(&selected_path)).unwrap_err();
        assert!(error.contains("指纹不匹配"));
    }

    #[test]
    fn selected_zcode_config_imports_supported_api_keys() {
        let config = json!({"provider": {
            BIGMODEL_API_KEY_PROVIDER_ID: {"options": {"apiKey": "fixture-bigmodel"}},
            ZAI_API_KEY_PROVIDER_ID: {"options": {"apiKey": "fixture-zai"}},
            "unrelated": {"options": {"apiKey": "ignored"}},
        }});
        let accounts = parse_import_accounts(&config.to_string(), None).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].api_key.as_deref(), Some("fixture-zai"));
        assert_eq!(accounts[1].api_key.as_deref(), Some("fixture-bigmodel"));
    }

    #[test]
    #[ignore = "requires an unlocked native system credential store"]
    fn v2_import_reads_only_the_selected_files_native_keyring_entry() {
        let source = tempfile::tempdir().unwrap();
        let real_path = source.path().join(CREDENTIALS_FILE);
        let path_with_dot_dot = source.path().join("nested").join("..").join(CREDENTIALS_FILE);
        let account = zcode_keyring_account(&real_path).unwrap();
        let entry = keyring::Entry::new(CREDENTIAL_KEYRING_SERVICE, &account).unwrap();
        let fixture_key = [17; 32];
        let values = fixture_v2_container("os", &fixture_key);
        entry.set_password(&URL_SAFE_NO_PAD.encode(fixture_key)).unwrap();
        let parsed = parse_import_accounts(&values.to_string(), Some(&path_with_dot_dot));
        entry.delete_credential().unwrap();
        assert_eq!(parsed.unwrap()[0].access_token, "fixture-access");
    }

    #[test]
    fn import_parser_accepts_official_v1_credentials_container() {
        let root = make_temp_dir("zcode-import-container");
        // v1 密钥始终按真实主目录派生，测试加密必须与解析路径一致。
        let home = dirs::home_dir().unwrap();
        let mut container = Map::new();
        let mut put = |key: &str, value: &str| {
            container.insert(
                key.to_string(),
                Value::String(encrypt_credential(value, &home).unwrap()),
            );
        };
        put(ACTIVE_PROVIDER_KEY, "zai");
        put("oauth:zai:access_token", "access-token-value");
        put("oauth:zai:refresh_token", "refresh-token-value");
        put("oauth:zai:user_info", r#"{"user_id":"user-1","email":"u@z.ai"}"#);
        put(ZCODE_JWT_KEY, "jwt-token-value");

        let parsed = parse_import_accounts(&Value::Object(container).to_string(), None).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, account_id("zai", Some("user-1"), Some("u@z.ai")));
        assert_eq!(parsed[0].provider, "zai");
        assert_eq!(parsed[0].access_token, "access-token-value");
        assert_eq!(parsed[0].refresh_token.as_deref(), Some("refresh-token-value"));
        assert_eq!(parsed[0].zcode_jwt_token, "jwt-token-value");
        assert_eq!(parsed[0].user_id.as_deref(), Some("user-1"));
        assert_eq!(parsed[0].email, "u@z.ai");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn import_parser_accepts_official_config_api_key_container() {
        let config = json!({
            "provider": {
                "builtin:zai": {
                    "name": "Z.ai - API Key",
                    "options": { "apiKey": "zai-key-value" }
                },
                "builtin:bigmodel": {
                    "options": { "apiKey": "bigmodel-key-value" }
                }
            }
        });
        let parsed = parse_import_accounts(&config.to_string(), None).unwrap();
        assert_eq!(parsed.len(), 2);
        assert!(parsed
            .iter()
            .any(|account| account.provider == "zai"
                && account.api_key.as_deref() == Some("zai-key-value")));
        assert!(parsed
            .iter()
            .any(|account| account.provider == "bigmodel"
                && account.api_key.as_deref() == Some("bigmodel-key-value")));
    }

    #[test]
    fn import_parser_still_rejects_unrelated_objects() {
        let error = parse_import_accounts(&json!({"foo": 1}).to_string(), None).unwrap_err();
        assert!(error.contains("ZCode 导入账号格式无效"));
    }

    /// 真机冒烟测试：验证默认安装的 ZCode 3.18+ 凭据（enc:v2:os）可被解密导入。
    /// 只断言结构，不打印任何凭据值；系统凭据库缺少条目时跳过。
    #[test]
    #[ignore = "requires real ZCode credentials and system keyring on this machine"]
    fn real_local_credentials_smoke() {
        let path = default_credentials_path().unwrap();
        let account = account_from_credentials_path(&path).unwrap();
        assert!(matches!(account.provider.as_str(), "zai" | "bigmodel"));
        assert!(!account.access_token.trim().is_empty());
        assert!(!account.zcode_jwt_token.trim().is_empty());
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

    #[test]
    fn oauth_inference_requires_an_unambiguous_active_start_plan() {
        let mut account = sample_account();
        account.plan_type = Some("ZCode Start Plan".into());
        for plans in [json!([]), json!([{"name":"ZCode Start Plan","status":"expired"}]),
            json!([{"name":"Coding Plan","status":"active"}]),
            json!([{"name":"ZCode Start Plan","status":"active"},{"name":"Coding Plan","status":"active"}])] {
            account.subscription_raw = Some(plans);
            assert_eq!(oauth_inference_session(&account), Err("ZCODE_START_PLAN_REQUIRED"));
        }
        account.subscription_raw = Some(json!([{"plan_id":"zcode-v3-start-plan-0615","status":"active"}]));
        assert_eq!(oauth_inference_session(&account), Ok(account.zcode_jwt_token.as_str()));
        account.provider = "bigmodel".into();
        assert_eq!(oauth_inference_session(&account), Err("ZCODE_BIGMODEL_PROJECT_API_KEY_REQUIRED"));
    }
}
