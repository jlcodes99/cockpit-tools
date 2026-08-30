// Claude 账号模块：Claude account models, storage paths, indexing and deduplication。
// 通过 include! 保持原 modules::claude_account 作用域和私有调用关系。
use crate::models::claude::{
    ClaudeAccount, ClaudeAccountIndex, ClaudeAuthMode, ClaudeDesktopGatewayModel,
    ClaudeDesktopGatewayModelMapping, ClaudeDesktopGatewayModelsResult,
    ClaudeDesktopLoginStartResponse, ClaudeOAuthStartResponse, ClaudeQuota, ClaudeQuotaErrorInfo,
};
use crate::modules::{account, atomic_write, logger};
#[cfg(target_os = "macos")]
use aes::Aes128;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
#[cfg(target_os = "macos")]
use cbc::cipher::block_padding::Pkcs7;
#[cfg(target_os = "macos")]
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
#[cfg(target_os = "macos")]
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(target_os = "macos")]
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use url::{form_urlencoded, Url};

const ACCOUNTS_INDEX_FILE: &str = "claude_accounts.json";
const ACCOUNTS_DIR: &str = "claude_accounts";
const CLAUDE_OAUTH_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
const CLAUDE_OAUTH_MANUAL_REDIRECT_URL: &str = "https://platform.claude.com/oauth/code/callback";
const CLAUDE_OAUTH_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_OAUTH_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_OAUTH_PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const CLAUDE_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";
const CLAUDE_TOKEN_EXPIRY_BUFFER_MS: i64 = 5 * 60 * 1000;
const CLAUDE_OAUTH_TIMEOUT_SECONDS: i64 = 600;
const CLAUDE_OAUTH_STATE_FILE: &str = "claude_oauth_pending.json";
const CLAUDE_CODE_CREDENTIALS_FILE: &str = ".credentials.json";
const CLAUDE_CODE_CONFIG_FILE: &str = ".config.json";
const CLAUDE_CODE_GLOBAL_CONFIG_FILE: &str = ".claude.json";
const CLAUDE_CODE_SETTINGS_FILE: &str = "settings.json";
const CLAUDE_CODE_SETTINGS_MANAGED_ENV_KEYS_FILE: &str =
    "claude_cli_settings_managed_env_keys.json";
const CLAUDE_CODE_KEYCHAIN_SERVICE_PREFIX: &str = "Claude Code";
const CLAUDE_CODE_KEYCHAIN_CREDENTIALS_SUFFIX: &str = "-credentials";
const CLAUDE_CODE_API_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "CLAUDE_CODE_ATTRIBUTION_HEADER",
];
const CLAUDE_OAUTH_SCOPES: [&str; 6] = [
    "org:create_api_key",
    "user:profile",
    "user:inference",
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
];
const CLAUDE_DESKTOP_LOGIN_STATE_FILE: &str = "claude_desktop_login_pending.json";
const CLAUDE_DESKTOP_PROFILES_DIR: &str = "claude_desktop_profiles";
const CLAUDE_DESKTOP_LOGIN_DIR: &str = "claude_desktop_login";
const CLAUDE_DESKTOP_CONFIG_FILE_NAME: &str = "claude_desktop_config.json";
const CLAUDE_DESKTOP_CONFIG_LIBRARY_DIR: &str = "configLibrary";
const CLAUDE_DESKTOP_THREEP_DIR_NAME: &str = "Claude-3p";
const CLAUDE_DESKTOP_AUTH_HELPER_SCRIPT: &str = "scripts/claude-desktop-auth-helper.cjs";
const CLAUDE_DESKTOP_AUTH_STATUS_FILE: &str = "claude_desktop_auth_status.json";
const CLAUDE_DESKTOP_AUTH_EXPORT_FILE: &str = "claude_desktop_auth_export.json";
const CLAUDE_DESKTOP_COOKIE_EXPORT_FILE: &str = "claude_desktop_cookie_probe_cookies.json";
const CLAUDE_DESKTOP_LOGIN_PROGRESS_EVENT: &str = "claude:desktop-login-progress";
const CLAUDE_DESKTOP_ELECTRON_RUNTIME_DIR: &str = "electron_runtime";
const CLAUDE_DESKTOP_ELECTRON_VERSION: &str = "42.4.0";
const CLAUDE_DESKTOP_BUNDLE_ID_MACOS: &str = "com.anthropic.claudefordesktop";
const CLAUDE_DESKTOP_LOGIN_TIMEOUT_SECONDS: i64 = 30 * 60;
const CLAUDE_DESKTOP_AUTH_EXPORT_WAIT_SECONDS: u64 = 8;
const CLAUDE_DESKTOP_HIDDEN_PROBE_COOLDOWN_SECONDS: u64 = 10 * 60;
const CLAUDE_DESKTOP_REQUIRED_COOKIE_NAMES: &[&str] = &["sessionKey", "lastActiveOrg"];
const CHROMIUM_EPOCH_OFFSET_MS: i64 = 11_644_473_600_000;
const CLAUDE_DESKTOP_LOCAL_PROFILE_MAX_FILES: usize = 600;
const CLAUDE_DESKTOP_LOCAL_PROFILE_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const CLAUDE_DESKTOP_LOCAL_PROFILE_SCAN_DIRS: &[&str] = &[
    "IndexedDB",
    "Local Storage",
    "Session Storage",
    "Cache/Cache_Data",
];
const CLAUDE_DESKTOP_PROFILE_ITEMS: &[&str] = &[
    "Local State",
    "Preferences",
    "Cookies",
    "Cookies-journal",
    "Network",
    "DIPS",
    "DIPS-wal",
    "SharedStorage",
    "SharedStorage-wal",
    "WebStorage",
    "Local Storage",
    "IndexedDB",
    "Session Storage",
    "Service Worker",
    "ant-did",
    "config.json",
    CLAUDE_DESKTOP_CONFIG_FILE_NAME,
];
static CLAUDE_ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));
static CLAUDE_PENDING_OAUTH_LOGIN: std::sync::LazyLock<Mutex<Option<PendingClaudeOAuthState>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));
static CLAUDE_PENDING_DESKTOP_LOGIN: std::sync::LazyLock<
    Mutex<Option<PendingClaudeDesktopLoginState>>,
> = std::sync::LazyLock::new(|| Mutex::new(None));
static CLAUDE_DESKTOP_ELECTRON_RUNTIME_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));
static CLAUDE_DESKTOP_HIDDEN_PROBE_ATTEMPTS: std::sync::LazyLock<Mutex<HashMap<String, Instant>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static EMAIL_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)[a-z0-9._%+\-]{1,64}@[a-z0-9.\-]{2,253}\.[a-z]{2,24}")
        .expect("valid email regex")
});
static UUID_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
        .expect("valid uuid regex")
});
#[cfg(target_os = "macos")]
type Aes128CbcDec = cbc::Decryptor<Aes128>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingClaudeOAuthState {
    login_id: String,
    state: String,
    code_verifier: String,
    auth_url: String,
    expires_at: i64,
    cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingClaudeDesktopLoginState {
    login_id: String,
    user_data_dir: PathBuf,
    #[serde(default)]
    status_file: PathBuf,
    #[serde(default)]
    export_file: PathBuf,
    #[serde(default)]
    helper_pid: Option<u32>,
    expires_at: i64,
    cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClaudeDesktopAuthCookieExport {
    cookies: Vec<ClaudeDesktopAuthCookie>,
    #[serde(default, rename = "webProfile")]
    web_profile: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClaudeDesktopAuthCookie {
    name: String,
    value: String,
    domain: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    secure: bool,
    #[serde(default, rename = "httpOnly")]
    http_only: bool,
    #[serde(default, rename = "expirationDate")]
    expiration_date: Option<f64>,
    #[serde(default, rename = "sameSite")]
    same_site: Option<String>,
}

#[derive(Debug, Clone)]
struct ClaudeDesktopProfileMetadata {
    source: String,
    has_session_key: bool,
    has_last_active_org: bool,
    last_active_org: Option<String>,
    session_expires_at: Option<i64>,
    cookie_names: Vec<String>,
    web_profile: Option<Value>,
}

#[derive(Debug, Clone, Default)]
struct ClaudeDesktopLocalProfile {
    email: Option<String>,
    account_uuid: Option<String>,
    full_name: Option<String>,
    display_name: Option<String>,
    organization_uuid: Option<String>,
    organization_name: Option<String>,
    source: Option<String>,
}

impl ClaudeDesktopLocalProfile {
    fn score(&self) -> i32 {
        let mut score = 0;
        if self.email.is_some() {
            score += 100;
        }
        if self.account_uuid.is_some() {
            score += 20;
        }
        if self.organization_uuid.is_some() {
            score += 10;
        }
        if self.organization_name.is_some() {
            score += 5;
        }
        if self.display_name.is_some() || self.full_name.is_some() {
            score += 3;
        }
        score
    }

    fn has_identity(&self) -> bool {
        self.email.is_some()
            || self.account_uuid.is_some()
            || self.organization_uuid.is_some()
            || self.organization_name.is_some()
    }
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
        (!trimmed.is_empty()).then_some(trimmed.to_string())
    })
}

fn generate_random_url_token(byte_len: usize) -> String {
    let mut bytes = vec![0u8; byte_len.max(16)];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn read_string_path(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    normalize_non_empty(current.as_str())
}

fn read_i64_value(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|v| v as i64)),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn read_f64_value(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn read_bool_value(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::String(text)) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn parse_reset_seconds(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => {
            let raw = number
                .as_i64()
                .or_else(|| number.as_f64().map(|v| v as i64))?;
            if raw <= 0 {
                None
            } else if raw > 10_000_000_000 {
                Some(raw / 1000)
            } else {
                Some(raw)
            }
        }
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(raw) = trimmed.parse::<i64>() {
                return if raw > 10_000_000_000 {
                    Some(raw / 1000)
                } else {
                    Some(raw)
                };
            }
            chrono::DateTime::parse_from_rfc3339(trimmed)
                .ok()
                .map(|dt| dt.timestamp())
        }
        _ => None,
    }
}

fn clamp_percentage(value: Option<f64>) -> i32 {
    let raw = value.unwrap_or(0.0);
    if !raw.is_finite() {
        return 0;
    }
    raw.round().clamp(0.0, 100.0) as i32
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let dir = get_data_dir()?.join(ACCOUNTS_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("创建 Claude 账号目录失败: {}", e))?;
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(get_accounts_index_path()?.to_string_lossy().to_string())
}

fn account_file_path(account_id: &str) -> Result<PathBuf, String> {
    Ok(get_accounts_dir()?.join(format!("{}.json", account_id)))
}

fn load_index() -> Result<ClaudeAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        return Ok(ClaudeAccountIndex::new());
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 Claude 账号索引失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(ClaudeAccountIndex::new());
    }
    atomic_write::parse_json_with_auto_restore::<ClaudeAccountIndex>(&path, &content)
        .map_err(|e| format!("解析 Claude 账号索引失败: {}", e))
}

fn save_index(index: &ClaudeAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content = serde_json::to_string_pretty(index)
        .map_err(|e| format!("序列化 Claude 账号索引失败: {}", e))?;
    atomic_write::write_string_atomic(&path, &content)
}

fn write_account_file(account: &ClaudeAccount) -> Result<(), String> {
    let path = account_file_path(&account.id)?;
    let content =
        crate::modules::secure_account_storage::serialize_account_file("claude", account)?;
    atomic_write::write_string_atomic(&path, &content)
}

fn load_account_file(account_id: &str) -> Option<ClaudeAccount> {
    let path = account_file_path(account_id).ok()?;
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    match crate::modules::secure_account_storage::deserialize_account_file::<ClaudeAccount>(
        &path, &content,
    ) {
        Ok((account, needs_rotation)) => {
            if needs_rotation {
                let account_for_rewrite = account.clone();
                crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
                    "claude",
                    account_for_rewrite.id.clone(),
                    path.clone(),
                    content.as_bytes(),
                    move || {
                        crate::modules::secure_account_storage::serialize_account_file(
                            "claude",
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

pub fn load_account(account_id: &str) -> Option<ClaudeAccount> {
    load_account_file(account_id)
}

pub fn list_accounts() -> Vec<ClaudeAccount> {
    list_accounts_checked().unwrap_or_default()
}

fn normalized_account_uuid(account: &ClaudeAccount) -> Option<String> {
    account
        .account_uuid
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .map(|value| value.to_ascii_lowercase())
}

fn normalized_account_email(account: &ClaudeAccount) -> Option<String> {
    normalize_non_empty(Some(account.email.as_str()))
        .filter(|value| value.contains('@'))
        .map(|value| value.to_ascii_lowercase())
}

fn is_real_email(value: &str) -> bool {
    value
        .split_once('@')
        .map(|(_, domain)| domain.contains('.'))
        .unwrap_or(false)
}

fn desktop_accounts_same_identity(a: &ClaudeAccount, b: &ClaudeAccount) -> bool {
    if a.auth_mode != ClaudeAuthMode::DesktopOAuth || b.auth_mode != ClaudeAuthMode::DesktopOAuth {
        return false;
    }
    match (normalized_account_uuid(a), normalized_account_uuid(b)) {
        (Some(left), Some(right)) => left == right,
        _ => match (normalized_account_email(a), normalized_account_email(b)) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        },
    }
}

fn cli_accounts_same_identity(a: &ClaudeAccount, b: &ClaudeAccount) -> bool {
    if a.auth_mode == ClaudeAuthMode::DesktopOAuth || b.auth_mode == ClaudeAuthMode::DesktopOAuth {
        return false;
    }
    match (normalized_account_uuid(a), normalized_account_uuid(b)) {
        (Some(left), Some(right)) => left == right,
        _ => match (normalized_account_email(a), normalized_account_email(b)) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        },
    }
}

fn merge_tags(left: Option<Vec<String>>, right: Option<Vec<String>>) -> Option<Vec<String>> {
    let mut tags = BTreeSet::new();
    for tag in left
        .into_iter()
        .flatten()
        .chain(right.into_iter().flatten())
    {
        let normalized = tag.trim();
        if !normalized.is_empty() {
            tags.insert(normalized.to_string());
        }
    }
    (!tags.is_empty()).then(|| tags.into_iter().collect())
}

fn choose_desktop_duplicate_base<'a>(
    left: &'a ClaudeAccount,
    right: &'a ClaudeAccount,
    current_id: Option<&str>,
) -> &'a ClaudeAccount {
    if current_id == Some(left.id.as_str()) {
        return left;
    }
    if current_id == Some(right.id.as_str()) {
        return right;
    }
    let left_score = (left.last_used, left.created_at);
    let right_score = (right.last_used, right.created_at);
    if right_score > left_score {
        right
    } else {
        left
    }
}

fn merge_desktop_account_fields(base: &ClaudeAccount, incoming: &ClaudeAccount) -> ClaudeAccount {
    let mut merged = base.clone();
    if is_real_email(&incoming.email) || !is_real_email(&merged.email) {
        merged.email = incoming.email.clone();
    }
    if incoming.account_uuid.is_some() {
        merged.account_uuid = incoming.account_uuid.clone();
    }
    if incoming.organization_uuid.is_some() {
        merged.organization_uuid = incoming.organization_uuid.clone();
    }
    if incoming
        .organization_name
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .map(|value| !value.eq_ignore_ascii_case("Claude"))
        .unwrap_or(false)
    {
        merged.organization_name = incoming.organization_name.clone();
    }
    if incoming.plan_type.is_some() {
        merged.plan_type = incoming.plan_type.clone();
    } else if merged
        .plan_type
        .as_deref()
        .map(is_desktop_plan_placeholder)
        .unwrap_or(false)
    {
        merged.plan_type = None;
    }
    if incoming.avatar_url.is_some() {
        merged.avatar_url = incoming.avatar_url.clone();
    }
    if incoming.profile_updated_at.is_some() {
        merged.profile_updated_at = incoming.profile_updated_at;
    }
    if incoming.quota.is_some() {
        merged.quota = incoming.quota.clone();
    }
    if incoming.usage_updated_at.is_some() {
        merged.usage_updated_at = incoming.usage_updated_at;
    }
    merged.quota_error = incoming.quota_error.clone();
    merged.status = incoming.status.clone();
    merged.status_reason = incoming.status_reason.clone();
    if incoming.desktop_profile_dir.is_some() {
        merged.desktop_profile_dir = incoming.desktop_profile_dir.clone();
    }
    if incoming.desktop_profile_imported_at.is_some() {
        merged.desktop_profile_imported_at = incoming.desktop_profile_imported_at;
    }
    if incoming.claude_credentials_raw.is_some() {
        merged.claude_credentials_raw = incoming.claude_credentials_raw.clone();
    }
    if incoming.claude_config_raw.is_some() {
        merged.claude_config_raw = incoming.claude_config_raw.clone();
    }
    if incoming.claude_usage_raw.is_some() {
        merged.claude_usage_raw = incoming.claude_usage_raw.clone();
    }
    merged.tags = merge_tags(merged.tags.take(), incoming.tags.clone());
    if incoming.account_note.is_some() {
        merged.account_note = incoming.account_note.clone();
    }
    merged.created_at = merged.created_at.min(incoming.created_at);
    merged.last_used = merged.last_used.max(incoming.last_used);
    merged
}

fn remove_desktop_snapshot_if_unused(snapshot: Option<&str>, keep_snapshot: Option<&str>) {
    let Some(snapshot) = snapshot.and_then(|value| normalize_non_empty(Some(value))) else {
        return;
    };
    if keep_snapshot
        .and_then(|value| normalize_non_empty(Some(value)))
        .map(|keep| keep == snapshot)
        .unwrap_or(false)
    {
        return;
    }
    let snapshot_path = PathBuf::from(snapshot);
    if snapshot_path.exists() {
        if let Err(error) = remove_path_if_exists(&snapshot_path) {
            logger::log_warn(&format!(
                "[Claude] 删除重复账号快照失败: path={}, error={}",
                snapshot_path.display(),
                error
            ));
        }
    }
}

fn delete_account_file_silent(account_id: &str) {
    if let Ok(path) = account_file_path(account_id) {
        if path.exists() {
            if let Err(error) = crate::modules::atomic_write::remove_file_locked(&path) {
                logger::log_warn(&format!(
                    "[Claude] 删除重复账号文件失败: path={}, error={}",
                    path.display(),
                    error
                ));
            }
        }
    }
}

fn find_existing_desktop_account(incoming: &ClaudeAccount) -> Option<ClaudeAccount> {
    let index = load_index().ok()?;
    index
        .accounts
        .into_iter()
        .filter(|summary| summary.id != incoming.id)
        .filter_map(|summary| load_account_file(&summary.id))
        .find(|account| desktop_accounts_same_identity(account, incoming))
}

fn save_desktop_account_with_dedupe(incoming: ClaudeAccount) -> Result<ClaudeAccount, String> {
    let old_snapshot = incoming.desktop_profile_dir.clone();
    let Some(existing) = find_existing_desktop_account(&incoming) else {
        return save_account_and_index(incoming);
    };
    let existing_snapshot = existing.desktop_profile_dir.clone();
    let merged = merge_desktop_account_fields(&existing, &incoming);
    let saved = save_account_and_index(merged)?;
    remove_desktop_snapshot_if_unused(
        existing_snapshot.as_deref(),
        saved.desktop_profile_dir.as_deref(),
    );
    if saved.desktop_profile_dir.as_deref() != old_snapshot.as_deref() {
        remove_desktop_snapshot_if_unused(
            old_snapshot.as_deref(),
            saved.desktop_profile_dir.as_deref(),
        );
    }
    Ok(saved)
}

fn dedupe_desktop_accounts(accounts: Vec<ClaudeAccount>) -> Result<Vec<ClaudeAccount>, String> {
    let current_id =
        crate::modules::provider_current_state::get_current_account_id("claude_desktop_account")
            .ok()
            .flatten();
    let mut kept: Vec<ClaudeAccount> = Vec::with_capacity(accounts.len());
    let mut removed_ids = Vec::new();
    let mut rewired_current: Option<String> = None;

    for account in accounts {
        let Some(index) = kept
            .iter()
            .position(|existing| desktop_accounts_same_identity(existing, &account))
        else {
            kept.push(account);
            continue;
        };

        let existing = kept.remove(index);
        let base =
            choose_desktop_duplicate_base(&existing, &account, current_id.as_deref()).clone();
        let other = if base.id == existing.id {
            account
        } else {
            existing
        };
        let old_base_snapshot = base.desktop_profile_dir.clone();
        let other_snapshot = other.desktop_profile_dir.clone();
        let mut merged = merge_desktop_account_fields(&base, &other);
        merged.id = base.id.clone();
        if current_id.as_deref() == Some(other.id.as_str()) {
            rewired_current = Some(base.id.clone());
        }
        remove_desktop_snapshot_if_unused(
            other_snapshot.as_deref(),
            merged.desktop_profile_dir.as_deref(),
        );
        if merged.desktop_profile_dir.as_deref() != old_base_snapshot.as_deref() {
            remove_desktop_snapshot_if_unused(
                old_base_snapshot.as_deref(),
                merged.desktop_profile_dir.as_deref(),
            );
        }
        delete_account_file_silent(&other.id);
        removed_ids.push(other.id.clone());
        kept.push(merged);
    }

    if removed_ids.is_empty() {
        return Ok(kept);
    }

    for account in &kept {
        write_account_file(account)?;
    }
    let mut index = ClaudeAccountIndex::new();
    index.accounts = kept.iter().map(|account| account.summary()).collect();
    index.accounts.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    save_index(&index)?;
    if let Some(next_current) = rewired_current {
        let _ = crate::modules::provider_current_state::set_current_account_id(
            "claude_desktop_account",
            Some(next_current.as_str()),
        );
    }
    logger::log_info(&format!(
        "[Claude] 已合并重复账号: removed={}",
        removed_ids.join(",")
    ));
    Ok(kept)
}

pub fn list_accounts_checked() -> Result<Vec<ClaudeAccount>, String> {
    let index = load_index()?;
    let mut accounts = Vec::new();
    for summary in index.accounts {
        if let Some(account) = load_account_file(&summary.id) {
            let mut account = account;
            let mut should_save = false;
            match repair_desktop_profile_dir(&mut account) {
                Ok(true) => should_save = true,
                Ok(false) => {}
                Err(error) => logger::log_warn(&format!(
                    "[Claude] Desktop profile 路径自动修复失败: account_id={}, error={}",
                    account.id, error
                )),
            }
            if normalize_account_plan_from_snapshots(&mut account) {
                should_save = true;
            }
            if account.auth_mode == ClaudeAuthMode::DesktopOAuth
                && !desktop_account_has_real_profile_data(&account)
            {
                if let Some(snapshot_dir) = account
                    .desktop_profile_dir
                    .as_deref()
                    .and_then(|value| normalize_non_empty(Some(value)))
                    .map(PathBuf::from)
                {
                    if apply_desktop_local_profile(&mut account, &snapshot_dir) {
                        account.quota_error = None;
                        account.status_reason = None;
                        should_save = true;
                    }
                }
            }
            if slim_claude_account_snapshots(&mut account) {
                should_save = true;
            }
            if normalize_cached_desktop_quota_from_raw(&mut account) {
                should_save = true;
            }
            if should_save {
                if let Err(error) = save_account_and_index(account.clone()) {
                    logger::log_warn(&format!(
                        "[Claude] 账号自动迁移保存失败: account_id={}, error={}",
                        account.id, error
                    ));
                }
            }
            accounts.push(account);
        }
    }
    dedupe_desktop_accounts(accounts)
}

fn save_account_and_index(mut account: ClaudeAccount) -> Result<ClaudeAccount, String> {
    if account.auth_mode == ClaudeAuthMode::DesktopOAuth {
        if let Err(error) = repair_desktop_profile_dir(&mut account) {
            logger::log_warn(&format!(
                "[Claude] 保存前修复 Desktop profile 路径失败: account_id={}, error={}",
                account.id, error
            ));
        }
    }
    slim_claude_account_snapshots(&mut account);
    write_account_file(&account)?;
    let mut index = load_index()?;
    index.accounts.retain(|item| item.id != account.id);
    index.accounts.push(account.summary());
    index.accounts.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    save_index(&index)?;
    Ok(account)
}

fn to_oauth_start_response(state: &PendingClaudeOAuthState) -> ClaudeOAuthStartResponse {
    ClaudeOAuthStartResponse {
        login_id: state.login_id.clone(),
        verification_uri: state.auth_url.clone(),
        expires_in: state
            .expires_at
            .saturating_sub(now_ts())
            .max(0)
            .try_into()
            .unwrap_or(0),
        interval_seconds: 1,
    }
}

fn to_desktop_login_start_response(
    state: &PendingClaudeDesktopLoginState,
) -> ClaudeDesktopLoginStartResponse {
    ClaudeDesktopLoginStartResponse {
        login_id: state.login_id.clone(),
        user_data_dir: state.user_data_dir.to_string_lossy().to_string(),
        expires_in: state
            .expires_at
            .saturating_sub(now_ts())
            .max(0)
            .try_into()
            .unwrap_or(0),
        interval_seconds: 2,
    }
}

fn get_desktop_profiles_dir() -> Result<PathBuf, String> {
    let dir = get_data_dir()?.join(CLAUDE_DESKTOP_PROFILES_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("创建 Claude 账号快照目录失败: {}", e))?;
    Ok(dir)
}

fn desktop_profile_has_valid_cookies(profile_dir: &Path) -> bool {
    if !profile_dir.exists() {
        return false;
    }
    desktop_cookie_path_candidates(profile_dir)
        .into_iter()
        .any(|cookies_path| {
            cookies_path.exists()
                && matches!(
                    cookies_db_has_required_desktop_session(&cookies_path),
                    Ok(true)
                )
        })
}

fn desktop_profile_snapshot_id_from_path(path: &Path) -> Option<String> {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    components
        .windows(2)
        .rev()
        .find_map(|pair| {
            pair.first()
                .filter(|name| name.eq_ignore_ascii_case(CLAUDE_DESKTOP_PROFILES_DIR))
                .and_then(|_| pair.get(1))
                .and_then(|snapshot| normalize_non_empty(Some(snapshot.as_str())))
        })
        .or_else(|| {
            path.file_name()
                .and_then(|value| value.to_str())
                .and_then(|value| normalize_non_empty(Some(value)))
                .filter(|value| value.starts_with("claude_desktop_"))
        })
        .or_else(|| {
            let parts = path
                .to_string_lossy()
                .split(['/', '\\'])
                .map(str::to_string)
                .collect::<Vec<_>>();
            parts.windows(2).rev().find_map(|pair| {
                pair.first()
                    .filter(|name| name.eq_ignore_ascii_case(CLAUDE_DESKTOP_PROFILES_DIR))
                    .and_then(|_| pair.get(1))
                    .and_then(|snapshot| normalize_non_empty(Some(snapshot.as_str())))
            })
        })
}

fn desktop_profile_repair_candidates(account: &ClaudeAccount) -> Result<Vec<PathBuf>, String> {
    let profiles_dir = get_desktop_profiles_dir()?;
    let mut candidates = Vec::new();
    if let Some(raw_path) = account
        .desktop_profile_dir
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
    {
        let original = PathBuf::from(raw_path);
        candidates.push(original.clone());
        if let Some(snapshot_id) = desktop_profile_snapshot_id_from_path(&original) {
            candidates.push(profiles_dir.join(snapshot_id));
        }
    }
    candidates.push(profiles_dir.join(&account.id));

    let mut seen = HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.to_string_lossy().to_string()));
    Ok(candidates)
}

fn repair_desktop_profile_dir(account: &mut ClaudeAccount) -> Result<bool, String> {
    if account.auth_mode != ClaudeAuthMode::DesktopOAuth {
        return Ok(false);
    }
    let current = account
        .desktop_profile_dir
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .map(PathBuf::from);
    if current
        .as_ref()
        .map(|path| desktop_profile_has_valid_cookies(path))
        .unwrap_or(false)
    {
        return Ok(false);
    }

    for candidate in desktop_profile_repair_candidates(account)? {
        if !desktop_profile_has_valid_cookies(&candidate) {
            continue;
        }
        let repaired = candidate.to_string_lossy().to_string();
        if account.desktop_profile_dir.as_deref() != Some(repaired.as_str()) {
            logger::log_info(&format!(
                "[Claude] 已修复 Desktop profile 路径: account_id={}, path={}",
                account.id,
                candidate.display()
            ));
            account.desktop_profile_dir = Some(repaired);
            return Ok(true);
        }
        return Ok(false);
    }
    Ok(false)
}

fn resolve_valid_desktop_profile_dir(account: &mut ClaudeAccount) -> Result<PathBuf, String> {
    let _ = repair_desktop_profile_dir(account)?;
    let profile_dir = account
        .desktop_profile_dir
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .map(PathBuf::from)
        .ok_or_else(|| "Claude 账号缺少 profile 快照，请重新登录或重新导入。".to_string())?;
    if desktop_profile_has_valid_cookies(&profile_dir) {
        return Ok(profile_dir);
    }
    Err(format!(
        "Claude profile 快照不可用，请重新登录或重新导入: {}",
        profile_dir.display()
    ))
}

fn get_desktop_login_root_dir() -> Result<PathBuf, String> {
    let dir = get_data_dir()?.join(CLAUDE_DESKTOP_LOGIN_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("创建 Claude 登录工作目录失败: {}", e))?;
    Ok(dir)
}

pub fn get_default_claude_desktop_user_data_dir() -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("CLAUDE_DESKTOP_USER_DATA_DIR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let data_dir = dirs::data_dir().ok_or_else(|| "无法获取系统应用数据目录".to_string())?;
    let standard_dir = data_dir.join("Claude");
    if standard_dir.exists() {
        return Ok(standard_dir);
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(store_dir) = find_windows_store_claude_desktop_user_data_dir() {
            return Ok(store_dir);
        }
    }

    Ok(standard_dir)
}

#[derive(Debug, Clone)]
struct ClaudeDesktopGatewayConfigPaths {
    normal_config_path: PathBuf,
    threep_config_path: PathBuf,
    config_library_dir: PathBuf,
}

impl ClaudeDesktopGatewayConfigPaths {
    fn config_library_meta_path(&self) -> PathBuf {
        self.config_library_dir.join("_meta.json")
    }
}

fn get_default_claude_desktop_threep_user_data_dir(normal_dir: &Path) -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("CLAUDE_DESKTOP_THREEP_USER_DATA_DIR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    #[cfg(target_os = "windows")]
    {
        let _ = normal_dir;
        let local_data_dir =
            dirs::data_local_dir().ok_or_else(|| "无法获取系统本地应用数据目录".to_string())?;
        return Ok(local_data_dir.join(CLAUDE_DESKTOP_THREEP_DIR_NAME));
    }

    #[cfg(not(target_os = "windows"))]
    {
        if normal_dir
            .file_name()
            .and_then(|value| value.to_str())
            .map(|name| name.eq_ignore_ascii_case("Claude"))
            .unwrap_or(false)
        {
            if let Some(parent) = normal_dir.parent() {
                return Ok(parent.join(CLAUDE_DESKTOP_THREEP_DIR_NAME));
            }
        }

        let data_dir = dirs::data_dir().ok_or_else(|| "无法获取系统应用数据目录".to_string())?;
        Ok(data_dir.join(CLAUDE_DESKTOP_THREEP_DIR_NAME))
    }
}

fn desktop_gateway_config_paths_from_dirs(
    normal_dir: &Path,
    threep_dir: &Path,
) -> ClaudeDesktopGatewayConfigPaths {
    ClaudeDesktopGatewayConfigPaths {
        normal_config_path: normal_dir.join(CLAUDE_DESKTOP_CONFIG_FILE_NAME),
        threep_config_path: threep_dir.join(CLAUDE_DESKTOP_CONFIG_FILE_NAME),
        config_library_dir: threep_dir.join(CLAUDE_DESKTOP_CONFIG_LIBRARY_DIR),
    }
}

fn get_default_claude_desktop_gateway_config_paths(
) -> Result<ClaudeDesktopGatewayConfigPaths, String> {
    let normal_dir = get_default_claude_desktop_user_data_dir()?;
    let threep_dir = get_default_claude_desktop_threep_user_data_dir(&normal_dir)?;
    Ok(desktop_gateway_config_paths_from_dirs(
        &normal_dir,
        &threep_dir,
    ))
}

fn validate_desktop_deployment_mode(config_path: &Path, expected_mode: &str) -> Result<(), String> {
    let config = read_config_file(config_path)?
        .ok_or_else(|| format!("Claude Desktop 配置未写入: {}", config_path.display()))?;
    let actual_mode = config
        .get("deploymentMode")
        .and_then(Value::as_str)
        .unwrap_or("");
    if actual_mode.eq_ignore_ascii_case(expected_mode) {
        return Ok(());
    }
    Err(format!(
        "Claude Desktop deploymentMode 校验失败: path={}, expected={}, actual={}",
        config_path.display(),
        expected_mode,
        actual_mode
    ))
}

fn validate_desktop_gateway_meta(meta_path: &Path, expected_config_id: &str) -> Result<(), String> {
    let meta = read_config_file(meta_path)?
        .ok_or_else(|| format!("Claude Gateway _meta.json 未写入: {}", meta_path.display()))?;
    let applied_id = meta.get("appliedId").and_then(Value::as_str).unwrap_or("");
    if applied_id != expected_config_id {
        return Err(format!(
            "Claude Gateway appliedId 校验失败: path={}, expected={}, actual={}",
            meta_path.display(),
            expected_config_id,
            applied_id
        ));
    }
    let has_entry = meta
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|id| id == expected_config_id)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if has_entry {
        return Ok(());
    }
    Err(format!(
        "Claude Gateway entries 校验失败: path={}, missing_id={}",
        meta_path.display(),
        expected_config_id
    ))
}

#[cfg(target_os = "windows")]
fn find_windows_store_claude_desktop_user_data_dir() -> Option<PathBuf> {
    let packages_dir = dirs::data_local_dir()?.join("Packages");
    let entries = fs::read_dir(packages_dir).ok()?;
    let mut candidates = Vec::new();

    for entry in entries.flatten() {
        let package_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !package_name.starts_with("claude_") && !package_name.contains("anthropic") {
            continue;
        }
        let profile_dir = entry
            .path()
            .join("LocalCache")
            .join("Roaming")
            .join("Claude");
        if !profile_dir.exists() {
            continue;
        }
        let has_cookies = desktop_cookies_path(&profile_dir).exists();
        let modified_at = fs::metadata(&profile_dir)
            .and_then(|metadata| metadata.modified())
            .ok();
        candidates.push((has_cookies, modified_at, profile_dir));
    }

    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    candidates.into_iter().map(|(_, _, path)| path).next()
}

pub fn get_default_claude_code_config_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    Ok(home.join(".claude"))
}

fn get_effective_claude_code_config_dir(config_dir: Option<&Path>) -> Result<PathBuf, String> {
    match config_dir {
        Some(path) => Ok(path.to_path_buf()),
        None => get_default_claude_code_config_dir(),
    }
}

fn get_claude_code_credentials_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CLAUDE_CODE_CREDENTIALS_FILE)
}

fn get_claude_code_settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CLAUDE_CODE_SETTINGS_FILE)
}

fn get_claude_code_settings_managed_env_keys_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(CLAUDE_CODE_SETTINGS_MANAGED_ENV_KEYS_FILE))
}

fn get_claude_code_global_config_path(config_dir: &Path) -> Result<PathBuf, String> {
    let config_json = config_dir.join(CLAUDE_CODE_CONFIG_FILE);
    if config_json.exists() {
        return Ok(config_json);
    }
    if config_dir != get_default_claude_code_config_dir()?.as_path() {
        return Ok(config_dir.join(CLAUDE_CODE_GLOBAL_CONFIG_FILE));
    }
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    Ok(home.join(CLAUDE_CODE_GLOBAL_CONFIG_FILE))
}

fn set_pending_oauth_login(state: Option<PendingClaudeOAuthState>) {
    if let Ok(mut guard) = CLAUDE_PENDING_OAUTH_LOGIN.lock() {
        *guard = state.clone();
    }
    let result = match state.as_ref() {
        Some(value) => crate::modules::oauth_pending_state::save(CLAUDE_OAUTH_STATE_FILE, value),
        None => crate::modules::oauth_pending_state::clear(CLAUDE_OAUTH_STATE_FILE),
    };
    if let Err(error) = result {
        logger::log_warn(&format!(
            "[Claude OAuth] 持久化 OAuth pending 状态失败，已忽略: {}",
            error
        ));
    }
}

fn load_pending_oauth_login_from_disk() -> Option<PendingClaudeOAuthState> {
    match crate::modules::oauth_pending_state::load::<PendingClaudeOAuthState>(
        CLAUDE_OAUTH_STATE_FILE,
    ) {
        Ok(Some(state)) => {
            if state.cancelled || now_ts() > state.expires_at {
                let _ = crate::modules::oauth_pending_state::clear(CLAUDE_OAUTH_STATE_FILE);
                None
            } else {
                Some(state)
            }
        }
        Ok(None) => None,
        Err(error) => {
            logger::log_warn(&format!(
                "[Claude OAuth] 读取 OAuth pending 状态失败，已忽略: {}",
                error
            ));
            let _ = crate::modules::oauth_pending_state::clear(CLAUDE_OAUTH_STATE_FILE);
            None
        }
    }
}

fn hydrate_pending_oauth_login_if_missing() {
    if let Ok(mut guard) = CLAUDE_PENDING_OAUTH_LOGIN.lock() {
        if guard.is_none() {
            *guard = load_pending_oauth_login_from_disk();
        }
    }
}

fn get_pending_oauth_login_for(login_id: &str) -> Result<PendingClaudeOAuthState, String> {
    hydrate_pending_oauth_login_if_missing();
    let state = CLAUDE_PENDING_OAUTH_LOGIN
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
        .ok_or_else(|| "Claude OAuth 授权流程不存在，请重新开始".to_string())?;
    if state.login_id != login_id {
        return Err("Claude OAuth 授权会话已变更，请重新开始".to_string());
    }
    if state.cancelled {
        return Err("Claude OAuth 授权已取消".to_string());
    }
    if now_ts() > state.expires_at {
        clear_pending_oauth_login_if_matches(login_id);
        return Err("Claude OAuth 授权已超时，请重新开始".to_string());
    }
    Ok(state)
}

fn clear_pending_oauth_login_if_matches(login_id: &str) {
    let should_clear = CLAUDE_PENDING_OAUTH_LOGIN
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|state| state.login_id == login_id))
        .unwrap_or(false);
    if should_clear {
        set_pending_oauth_login(None);
    }
}

fn build_oauth_authorize_url(state: &str, code_challenge: &str) -> Result<String, String> {
    let mut url = Url::parse(CLAUDE_OAUTH_AUTHORIZE_URL)
        .map_err(|e| format!("构建 Claude OAuth 授权地址失败: {}", e))?;
    url.query_pairs_mut()
        .append_pair("code", "true")
        .append_pair("client_id", CLAUDE_OAUTH_CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", CLAUDE_OAUTH_MANUAL_REDIRECT_URL)
        .append_pair("scope", &CLAUDE_OAUTH_SCOPES.join(" "))
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
    Ok(url.to_string())
}

fn clean_authorization_code(raw: &str) -> (String, Option<String>) {
    let mut code = raw.trim();
    let mut state = None;
    if let Some((before, after)) = code.split_once('#') {
        code = before;
        state = normalize_non_empty(Some(after));
    }
    if let Some((before, _after)) = code.split_once('&') {
        code = before;
    }
    (code.trim().to_string(), state)
}

fn is_claude_oauth_authorize_url(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    (host.eq_ignore_ascii_case("claude.com") || host.eq_ignore_ascii_case("www.claude.com"))
        && url.path().eq_ignore_ascii_case("/cai/oauth/authorize")
}

fn oauth_authorize_url_input_error() -> String {
    "你粘贴的是 OAuth 授权入口链接，不是授权完成后的 code。请先在浏览器完成授权，然后复制最终页面地址或页面显示的 code。".to_string()
}

fn parse_oauth_callback_input(input: &str) -> Result<(String, Option<String>), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("授权回调链接或 code 不能为空".to_string());
    }

    let mut query_like = None;
    if let Ok(url) = Url::parse(trimmed) {
        if is_claude_oauth_authorize_url(&url) {
            return Err(oauth_authorize_url_input_error());
        }
        let pairs: std::collections::HashMap<String, String> = url
            .query_pairs()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        if pairs
            .get("code")
            .map(|value| value == "true")
            .unwrap_or(false)
            && pairs.get("client_id").is_some()
        {
            return Err(oauth_authorize_url_input_error());
        }
        if let Some(code) = pairs
            .get("code")
            .and_then(|value| normalize_non_empty(Some(value.as_str())))
        {
            let (code, state_from_code) = clean_authorization_code(&code);
            return Ok((code, pairs.get("state").cloned().or(state_from_code)));
        }
        if let Some(fragment) = normalize_non_empty(url.fragment()) {
            query_like = Some(fragment);
        }
    } else if trimmed.starts_with("code=")
        || trimmed.starts_with("state=")
        || trimmed.contains("&code=")
        || trimmed.contains("?code=")
    {
        query_like = Some(
            trimmed
                .split_once('?')
                .map(|(_, query)| query)
                .unwrap_or_else(|| trimmed.trim_start_matches('?'))
                .to_string(),
        );
    }

    if let Some(query) = query_like {
        let pairs: std::collections::HashMap<String, String> =
            form_urlencoded::parse(query.as_bytes())
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect();
        if let Some(code) = pairs
            .get("code")
            .and_then(|value| normalize_non_empty(Some(value.as_str())))
        {
            let (code, state_from_code) = clean_authorization_code(&code);
            return Ok((code, pairs.get("state").cloned().or(state_from_code)));
        }
    }

    let (raw_code, raw_state) = clean_authorization_code(trimmed);
    let code = normalize_non_empty(Some(raw_code.trim_start_matches("code=")))
        .ok_or_else(|| "授权 code 不能为空".to_string())?;
    Ok((code, raw_state))
}

fn set_pending_desktop_login(state: Option<PendingClaudeDesktopLoginState>) {
    if let Ok(mut guard) = CLAUDE_PENDING_DESKTOP_LOGIN.lock() {
        *guard = state.clone();
    }
    let result = match state.as_ref() {
        Some(value) => {
            crate::modules::oauth_pending_state::save(CLAUDE_DESKTOP_LOGIN_STATE_FILE, value)
        }
        None => crate::modules::oauth_pending_state::clear(CLAUDE_DESKTOP_LOGIN_STATE_FILE),
    };
    if let Err(error) = result {
        logger::log_warn(&format!(
            "[Claude] 持久化登录 pending 状态失败，已忽略: {}",
            error
        ));
    }
}

fn load_pending_desktop_login_from_disk() -> Option<PendingClaudeDesktopLoginState> {
    match crate::modules::oauth_pending_state::load::<PendingClaudeDesktopLoginState>(
        CLAUDE_DESKTOP_LOGIN_STATE_FILE,
    ) {
        Ok(Some(state)) => {
            if state.cancelled || now_ts() > state.expires_at {
                let _ = crate::modules::oauth_pending_state::clear(CLAUDE_DESKTOP_LOGIN_STATE_FILE);
                None
            } else {
                Some(state)
            }
        }
        Ok(None) => None,
        Err(error) => {
            logger::log_warn(&format!(
                "[Claude] 读取登录 pending 状态失败，已忽略: {}",
                error
            ));
            let _ = crate::modules::oauth_pending_state::clear(CLAUDE_DESKTOP_LOGIN_STATE_FILE);
            None
        }
    }
}

fn hydrate_pending_desktop_login_if_missing() {
    if let Ok(mut guard) = CLAUDE_PENDING_DESKTOP_LOGIN.lock() {
        if guard.is_none() {
            *guard = load_pending_desktop_login_from_disk();
        }
    }
}

fn get_pending_desktop_login_for(login_id: &str) -> Result<PendingClaudeDesktopLoginState, String> {
    hydrate_pending_desktop_login_if_missing();
    let state = CLAUDE_PENDING_DESKTOP_LOGIN
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
        .ok_or_else(|| "Claude 登录流程不存在，请重新开始".to_string())?;
    if state.login_id != login_id {
        return Err("Claude 登录会话已变更，请重新开始".to_string());
    }
    if state.cancelled {
        return Err("Claude 登录已取消".to_string());
    }
    if now_ts() > state.expires_at {
        clear_pending_desktop_login_if_matches(login_id);
        return Err("Claude 登录已超时，请重新开始".to_string());
    }
    Ok(state)
}

fn clear_pending_desktop_login_if_matches(login_id: &str) {
    let should_clear = CLAUDE_PENDING_DESKTOP_LOGIN
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|state| state.login_id == login_id))
        .unwrap_or(false);
    if should_clear {
        set_pending_desktop_login(None);
    }
}

