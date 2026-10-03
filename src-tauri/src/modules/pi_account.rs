//! pi coding agent account profiles.
//!
//! A pi account is a profile holding one or more provider credentials (the raw
//! `auth.json` entries) plus default provider/model settings. Switching merges
//! those credentials into the target `auth.json` and writes defaults into
//! `settings.json`; unrelated providers in the target files are preserved.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::models::pi::{PiAccount, PiAccountView, PiProviderCredential};
use crate::modules::atomic_write::{write_secret_string_atomic, write_string_atomic};
use crate::modules::{account, logger, pi_auth_lock, provider_current_state};

const INDEX_FILE: &str = "pi_accounts.json";
const PROFILES_DIR: &str = "pi_profiles";
const AUTH_FILE: &str = "auth.json";
const SETTINGS_FILE: &str = "settings.json";
const MODELS_FILE: &str = "models.json";
const PLATFORM: &str = "pi";
pub const PI_HOME_ENV: &str = "PI_CODING_AGENT_DIR";

static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Default, Serialize, Deserialize)]
struct PiAccountStore {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    accounts: Vec<PiAccount>,
}

fn default_version() -> u32 {
    1
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn index_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(index_path()?.to_string_lossy().to_string())
}

pub fn profiles_dir() -> Result<PathBuf, String> {
    let path = account::get_data_dir()?.join(PROFILES_DIR);
    fs::create_dir_all(&path).map_err(|e| format!("创建 pi profile 目录失败: {}", e))?;
    Ok(path)
}

/// Official pi agent dir: `$PI_CODING_AGENT_DIR` or `~/.pi/agent`.
pub fn default_pi_home() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os(PI_HOME_ENV) {
        let path = PathBuf::from(value);
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }
    Ok(dirs::home_dir()
        .ok_or_else(|| "无法获取用户主目录".to_string())?
        .join(".pi")
        .join("agent"))
}

fn normalize_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(format!("非法的 pi 账号 ID: {}", value));
    }
    Ok(value.to_string())
}

pub fn managed_profile_dir(account_id: &str) -> Result<PathBuf, String> {
    Ok(profiles_dir()?.join(normalize_id(account_id)?))
}

fn load_store() -> Result<PiAccountStore, String> {
    let path = index_path()?;
    if !path.exists() {
        return Ok(PiAccountStore::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读取 pi 账号索引失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(PiAccountStore::default());
    }
    serde_json::from_str(&content).map_err(|e| format!("解析 pi 账号索引失败: {}", e))
}

fn save_store(store: &PiAccountStore) -> Result<(), String> {
    let path = index_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建数据目录失败: {}", e))?;
    }
    let content =
        serde_json::to_string_pretty(store).map_err(|e| format!("序列化 pi 账号失败: {}", e))?;
    write_secret_string_atomic(&path, &content)
}

fn with_store<T>(f: impl FnOnce(&mut PiAccountStore) -> Result<T, String>) -> Result<T, String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "pi 账号锁已损坏".to_string())?;
    let mut store = load_store()?;
    let result = f(&mut store)?;
    save_store(&store)?;
    Ok(result)
}

pub fn list_accounts() -> Result<Vec<PiAccountView>, String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "pi 账号锁已损坏".to_string())?;
    Ok(load_store()?
        .accounts
        .iter()
        .map(PiAccountView::from)
        .collect())
}

pub fn load_account(account_id: &str) -> Option<PiAccount> {
    let _guard = STORE_LOCK.lock().ok()?;
    load_store()
        .ok()?
        .accounts
        .into_iter()
        .find(|account| account.id == account_id)
}

pub fn delete_accounts(account_ids: &[String]) -> Result<(), String> {
    with_store(|store| {
        store
            .accounts
            .retain(|account| !account_ids.iter().any(|id| id == &account.id));
        Ok(())
    })?;
    for id in account_ids {
        if let Ok(dir) = managed_profile_dir(id) {
            if dir.exists() {
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }
    if let Some(current) = provider_current_state::get_current_account_id(PLATFORM)? {
        if account_ids.iter().any(|id| id == &current) {
            provider_current_state::set_current_account_id(PLATFORM, None)?;
        }
    }
    Ok(())
}

fn update_account(
    account_id: &str,
    f: impl FnOnce(&mut PiAccount),
) -> Result<PiAccountView, String> {
    with_store(|store| {
        let account = store
            .accounts
            .iter_mut()
            .find(|account| account.id == account_id)
            .ok_or_else(|| format!("pi 账号不存在: {}", account_id))?;
        f(account);
        Ok(PiAccountView::from(&*account))
    })
}

pub fn update_tags(account_id: &str, tags: Vec<String>) -> Result<PiAccountView, String> {
    let tags: Vec<String> = tags
        .into_iter()
        .filter_map(|tag| normalize_text(Some(&tag)))
        .collect();
    update_account(account_id, |account| {
        account.tags = (!tags.is_empty()).then_some(tags);
    })
}

pub fn update_working_dir(
    account_id: &str,
    working_dir: Option<String>,
) -> Result<PiAccountView, String> {
    let working_dir = normalize_text(working_dir.as_deref());
    update_account(account_id, |account| account.working_dir = working_dir)
}

pub fn update_defaults(
    account_id: &str,
    display_name: Option<String>,
    default_provider: Option<String>,
    default_model: Option<String>,
    default_thinking_level: Option<String>,
) -> Result<PiAccountView, String> {
    update_account(account_id, |account| {
        if let Some(name) = normalize_text(display_name.as_deref()) {
            account.email = name;
        }
        account.default_provider = normalize_text(default_provider.as_deref());
        account.default_model = normalize_text(default_model.as_deref());
        account.default_thinking_level = normalize_text(default_thinking_level.as_deref());
    })
}

/// Stable fingerprint of the credential set, used to dedupe imports.
fn credentials_fingerprint(credentials: &[PiProviderCredential]) -> String {
    let mut parts: Vec<String> = credentials
        .iter()
        .map(|cred| {
            let identity = cred
                .entry
                .get("key")
                .or_else(|| cred.entry.get("accountId"))
                .or_else(|| cred.entry.get("refresh"))
                .map(Value::to_string)
                .unwrap_or_default();
            format!("{}:{}", cred.provider, identity)
        })
        .collect();
    parts.sort();
    let digest = Sha256::digest(parts.join("|").as_bytes());
    digest
        .iter()
        .take(8)
        .map(|b| format!("{:02x}", b))
        .collect()
}

fn mask_secret(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("****{}", tail)
}

fn default_display_name(credentials: &[PiProviderCredential]) -> String {
    let providers: Vec<&str> = credentials.iter().map(|c| c.provider.as_str()).collect();
    if providers.is_empty() {
        return "pi".to_string();
    }
    if credentials.len() == 1 {
        if let Some(key) = credentials[0].entry.get("key").and_then(Value::as_str) {
            return format!("{} {}", credentials[0].provider, mask_secret(key));
        }
    }
    providers.join(", ")
}

/// Insert a new account or update the one with the same credential fingerprint.
fn upsert(mut candidate: PiAccount) -> Result<PiAccountView, String> {
    if candidate.credentials.is_empty() {
        return Err("pi 账号至少需要一个 provider 凭据".to_string());
    }
    let fingerprint = credentials_fingerprint(&candidate.credentials);
    with_store(|store| {
        if let Some(existing) = store
            .accounts
            .iter_mut()
            .find(|account| credentials_fingerprint(&account.credentials) == fingerprint)
        {
            existing.credentials = candidate.credentials;
            if !candidate.custom_providers.is_empty() {
                existing.custom_providers = candidate.custom_providers;
            }
            if candidate.default_provider.is_some() {
                existing.default_provider = candidate.default_provider;
            }
            if candidate.default_model.is_some() {
                existing.default_model = candidate.default_model;
            }
            if candidate.default_thinking_level.is_some() {
                existing.default_thinking_level = candidate.default_thinking_level;
            }
            return Ok(PiAccountView::from(&*existing));
        }
        if store
            .accounts
            .iter()
            .any(|account| account.id == candidate.id)
        {
            candidate.id = format!("pi_{}", uuid::Uuid::new_v4().simple());
        }
        let view = PiAccountView::from(&candidate);
        store.accounts.push(candidate);
        Ok(view)
    })
}

fn new_account(
    credentials: Vec<PiProviderCredential>,
    display_name: Option<String>,
    default_provider: Option<String>,
    default_model: Option<String>,
    default_thinking_level: Option<String>,
) -> PiAccount {
    let now = now_ts();
    PiAccount {
        id: format!("pi_{}", credentials_fingerprint(&credentials)),
        email: display_name.unwrap_or_else(|| default_display_name(&credentials)),
        tags: None,
        credentials,
        custom_providers: Vec::new(),
        default_provider,
        default_model,
        default_thinking_level,
        working_dir: None,
        created_at: now,
        last_used: now,
    }
}

fn normalize_provider_id(provider: &str) -> Result<String, String> {
    let provider = provider.trim();
    if provider.is_empty()
        || !provider
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!("非法的 provider: {}", provider));
    }
    Ok(provider.to_string())
}

pub fn add_with_api_key(
    provider: &str,
    api_key: &str,
    display_name: Option<String>,
    default_model: Option<String>,
) -> Result<PiAccountView, String> {
    let provider = normalize_provider_id(provider)?;
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    let credentials = vec![PiProviderCredential {
        provider: provider.clone(),
        entry: serde_json::json!({ "type": "api_key", "key": api_key }),
    }];
    upsert(new_account(
        credentials,
        normalize_text(display_name.as_deref()),
        Some(provider),
        normalize_text(default_model.as_deref()),
        None,
    ))
}

fn read_json_object(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let content =
        fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    if content.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&content) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!("{} 不是 JSON 对象", path.display())),
        Err(e) => Err(format!("解析 {} 失败: {}", path.display(), e)),
    }
}

fn credentials_from_auth(auth: &Map<String, Value>) -> Vec<PiProviderCredential> {
    auth.iter()
        .filter(|(_, entry)| entry.get("type").and_then(Value::as_str).is_some())
        .map(|(provider, entry)| PiProviderCredential {
            provider: provider.clone(),
            entry: entry.clone(),
        })
        .collect()
}

fn string_setting(settings: &Map<String, Value>, key: &str) -> Option<String> {
    normalize_text(settings.get(key).and_then(Value::as_str))
}

/// Import the current `~/.pi/agent/auth.json` (+ settings defaults) as one account.
pub fn import_from_local() -> Result<Vec<PiAccountView>, String> {
    let home = default_pi_home()?;
    let auth = read_json_object(&home.join(AUTH_FILE))?;
    let credentials = credentials_from_auth(&auth);
    if credentials.is_empty() {
        return Err(format!(
            "未在 {} 找到 pi 凭据，请先运行 pi 并执行 /login",
            home.join(AUTH_FILE).display()
        ));
    }
    let settings = read_json_object(&home.join(SETTINGS_FILE)).unwrap_or_default();
    let view = upsert(new_account(
        credentials,
        None,
        string_setting(&settings, "defaultProvider"),
        string_setting(&settings, "defaultModel"),
        string_setting(&settings, "defaultThinkingLevel"),
    ))?;
    Ok(vec![view])
}

/// Import accounts exported by [`export_accounts`] (array or single object).
pub fn import_from_json(json_content: &str) -> Result<Vec<PiAccountView>, String> {
    let value: Value =
        serde_json::from_str(json_content).map_err(|e| format!("解析 JSON 失败: {}", e))?;
    let items = match value {
        Value::Array(items) => items,
        other => vec![other],
    };
    let mut results = Vec::new();
    for item in items {
        // Accept either an exported PiAccount or a raw pi auth.json object.
        if let Ok(account) = serde_json::from_value::<PiAccount>(item.clone()) {
            results.push(upsert(account)?);
            continue;
        }
        if let Value::Object(map) = &item {
            let credentials = credentials_from_auth(map);
            if !credentials.is_empty() {
                results.push(upsert(new_account(credentials, None, None, None, None))?);
                continue;
            }
        }
        return Err("无法识别的 pi 账号 JSON".to_string());
    }
    Ok(results)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "pi 账号锁已损坏".to_string())?;
    let accounts: Vec<PiAccount> = load_store()?
        .accounts
        .into_iter()
        .filter(|account| account_ids.is_empty() || account_ids.contains(&account.id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

fn backup_file(path: &Path) {
    if !path.exists() {
        return;
    }
    // Single rolling backup so credential copies don't accumulate.
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let backup = path.with_file_name(format!("{}.cockpit-backup", file_name));
    // Backups hold credentials too: keep them owner-only like the original.
    let result = fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|content| write_secret_string_atomic(&backup, &content));
    if let Err(e) = result {
        logger::log_warn(&format!("[pi] 备份 {} 失败: {}", path.display(), e));
    }
}

/// Merge the account's credentials and defaults into `home`.
pub fn write_account_to_home(account: &PiAccount, home: &Path) -> Result<(), String> {
    fs::create_dir_all(home).map_err(|e| format!("创建 {} 失败: {}", home.display(), e))?;

    let auth_path = home.join(AUTH_FILE);
    let auth_lock = pi_auth_lock::lock(&auth_path)?;
    let mut auth = read_json_object(&auth_path)?;
    backup_file(&auth_path);
    for cred in &account.credentials {
        auth.insert(cred.provider.clone(), cred.entry.clone());
    }
    let content = serde_json::to_string_pretty(&Value::Object(auth))
        .map_err(|e| format!("序列化 auth.json 失败: {}", e))?;
    write_secret_string_atomic(&auth_path, &content)?;
    drop(auth_lock);

    if !account.custom_providers.is_empty() {
        let models_path = home.join(MODELS_FILE);
        let mut models = read_json_object(&models_path)?;
        let mut providers = match models.remove("providers") {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };
        for custom in &account.custom_providers {
            providers.insert(custom.provider.clone(), custom.config.clone());
        }
        models.insert("providers".into(), Value::Object(providers));
        backup_file(&models_path);
        let content = serde_json::to_string_pretty(&Value::Object(models))
            .map_err(|e| format!("序列化 models.json 失败: {}", e))?;
        write_string_atomic(&models_path, &content)?;
    }

    let settings_path = home.join(SETTINGS_FILE);
    let mut settings = read_json_object(&settings_path).unwrap_or_default();
    let mut changed = false;
    for (key, value) in [
        ("defaultProvider", &account.default_provider),
        ("defaultModel", &account.default_model),
        ("defaultThinkingLevel", &account.default_thinking_level),
    ] {
        if let Some(value) = value {
            settings.insert(key.to_string(), Value::String(value.clone()));
            changed = true;
        }
    }
    if changed {
        backup_file(&settings_path);
        let content = serde_json::to_string_pretty(&Value::Object(settings))
            .map_err(|e| format!("序列化 settings.json 失败: {}", e))?;
        write_string_atomic(&settings_path, &content)?;
    }
    Ok(())
}

/// pi refreshes OAuth tokens in place; pull them back before switching away.
fn sync_back_credentials(account_id: &str, home: &Path) {
    match pi_auth_lock::lock(&home.join(AUTH_FILE)) {
        Ok(_guard) => sync_back_unlocked(account_id, home),
        Err(e) => logger::log_warn(&format!("[pi] 回写 OAuth 凭据跳过: {}", e)),
    }
}

/// Is `live` the same OAuth login as `stored`?
///
/// Codex entries carry `accountId`; Anthropic entries carry no identity, so
/// for them we rely on `home` belonging to this account (managed profile dir,
/// or the official home while this account is current). A `/login` to another
/// Anthropic account inside that home cannot be told apart.
fn same_oauth_identity(stored: &Value, live: &Value) -> bool {
    if live.get("type").and_then(Value::as_str) != Some("oauth") {
        return false;
    }
    match (stored.get("accountId"), live.get("accountId")) {
        (Some(a), Some(b)) => a == b,
        (None, None) => true,
        _ => false,
    }
}

/// `live` is at least as fresh as `stored`; a missing `expires` is unknown.
fn is_not_older(stored: &Value, live: &Value) -> bool {
    match (
        stored.get("expires").and_then(Value::as_i64),
        live.get("expires").and_then(Value::as_i64),
    ) {
        (Some(stored), Some(live)) => live >= stored,
        (None, _) => true,
        (Some(_), None) => false,
    }
}

/// Caller must hold the `auth.json` lock for `home`.
fn sync_back_unlocked(account_id: &str, home: &Path) {
    let Ok(auth) = read_json_object(&home.join(AUTH_FILE)) else {
        return;
    };
    let result = with_store(|store| {
        if let Some(account) = store.accounts.iter_mut().find(|a| a.id == account_id) {
            for cred in account.credentials.iter_mut() {
                if cred.kind() != "oauth" {
                    continue;
                }
                let Some(live) = auth.get(&cred.provider) else {
                    continue;
                };
                // Never replace a newer token (e.g. one cockpit just refreshed).
                if same_oauth_identity(&cred.entry, live) && is_not_older(&cred.entry, live) {
                    cred.entry = live.clone();
                }
            }
        }
        Ok(())
    });
    if let Err(e) = result {
        logger::log_warn(&format!("[pi] 回写 OAuth 凭据失败: {}", e));
    }
}

fn touch_last_used(account_id: &str) {
    let _ = update_account(account_id, |account| account.last_used = now_ts());
}

/// Write the account into the official pi home and mark it current.
pub fn inject_to_default(account_id: &str) -> Result<String, String> {
    let home = default_pi_home()?;
    if let Some(previous) = provider_current_state::get_current_account_id(PLATFORM)? {
        if previous != account_id {
            sync_back_credentials(&previous, &home);
        }
    }
    let account =
        load_account(account_id).ok_or_else(|| format!("pi 账号不存在: {}", account_id))?;
    write_account_to_home(&account, &home)?;
    provider_current_state::set_current_account_id(PLATFORM, Some(account_id))?;
    touch_last_used(account_id);
    Ok(account.email)
}

/// Prepare an isolated `PI_CODING_AGENT_DIR` for the account.
pub fn prepare_account_home(account_id: &str) -> Result<(String, PathBuf), String> {
    let dir = managed_profile_dir(account_id)?;
    sync_back_credentials(account_id, &dir);
    let account =
        load_account(account_id).ok_or_else(|| format!("pi 账号不存在: {}", account_id))?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建 {} 失败: {}", dir.display(), e))?;
    // Seed theme/enabledModels etc. from the official home on first use.
    let settings_path = dir.join(SETTINGS_FILE);
    if !settings_path.exists() {
        if let Ok(home) = default_pi_home() {
            let source = home.join(SETTINGS_FILE);
            if source.exists() {
                let _ = fs::copy(&source, &settings_path);
            }
        }
    }
    write_account_to_home(&account, &dir)?;
    touch_last_used(account_id);
    Ok((account.email, dir))
}

pub fn current_account_id() -> Result<Option<String>, String> {
    let tracked = provider_current_state::get_current_account_id(PLATFORM)?;
    Ok(tracked.filter(|id| load_account(id).is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::pi::PiCustomProvider;

    fn cred(provider: &str, entry: Value) -> PiProviderCredential {
        PiProviderCredential {
            provider: provider.to_string(),
            entry,
        }
    }

    #[test]
    fn fingerprint_is_order_independent() {
        let a = cred(
            "anthropic",
            serde_json::json!({"type":"api_key","key":"k1"}),
        );
        let b = cred("openai", serde_json::json!({"type":"api_key","key":"k2"}));
        assert_eq!(
            credentials_fingerprint(&[a.clone(), b.clone()]),
            credentials_fingerprint(&[b, a])
        );
    }

    #[test]
    fn write_account_merges_and_preserves_other_providers() {
        let dir = std::env::temp_dir().join(format!("pi_test_{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(AUTH_FILE),
            r#"{"deepseek":{"type":"api_key","key":"old"},"anthropic":{"type":"api_key","key":"x"}}"#,
        )
        .unwrap();
        fs::write(dir.join(SETTINGS_FILE), r#"{"theme":"dark"}"#).unwrap();
        let account = new_account(
            vec![cred(
                "anthropic",
                serde_json::json!({"type":"api_key","key":"new"}),
            )],
            None,
            Some("anthropic".into()),
            Some("claude-sonnet".into()),
            None,
        );
        write_account_to_home(&account, &dir).unwrap();
        let auth = read_json_object(&dir.join(AUTH_FILE)).unwrap();
        assert_eq!(auth["deepseek"]["key"], "old");
        assert_eq!(auth["anthropic"]["key"], "new");
        let settings = read_json_object(&dir.join(SETTINGS_FILE)).unwrap();
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["defaultProvider"], "anthropic");
        assert_eq!(settings["defaultModel"], "claude-sonnet");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gateway_provider_merges_into_models_json() {
        let dir = std::env::temp_dir().join(format!("pi_test_{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(MODELS_FILE),
            r#"{"providers":{"ollama":{"baseUrl":"http://localhost:11434/v1","api":"openai-completions","models":[{"id":"llama"}]}}}"#,
        )
        .unwrap();
        let mut account = new_account(
            vec![cred(
                "my-gw",
                serde_json::json!({"type":"api_key","key":"sk"}),
            )],
            None,
            Some("my-gw".into()),
            Some("m1".into()),
            None,
        );
        account.custom_providers = vec![PiCustomProvider {
            provider: "my-gw".into(),
            config: serde_json::json!({"baseUrl":"https://gw.example/v1","api":"openai-completions","models":[{"id":"m1"}]}),
        }];
        write_account_to_home(&account, &dir).unwrap();
        let models = read_json_object(&dir.join(MODELS_FILE)).unwrap();
        assert_eq!(models["providers"]["ollama"]["models"][0]["id"], "llama");
        assert_eq!(
            models["providers"]["my-gw"]["baseUrl"],
            "https://gw.example/v1"
        );
        assert!(models["providers"]["my-gw"].get("apiKey").is_none());
        let auth = read_json_object(&dir.join(AUTH_FILE)).unwrap();
        assert_eq!(auth["my-gw"]["key"], "sk");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn display_name_masks_key() {
        let name = default_display_name(&[cred(
            "openai",
            serde_json::json!({"type":"api_key","key":"sk-1234567890abcd"}),
        )]);
        assert_eq!(name, "openai ****abcd");
    }
}
