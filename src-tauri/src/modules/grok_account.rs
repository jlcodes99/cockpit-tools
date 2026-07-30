use crate::models::grok::{
    GrokAccount, GrokAccountIndex, GrokAccountView, GrokAuthMode, GrokOAuthCompletePayload,
    GrokProductUsage, GrokQuota,
};
use crate::modules::{account, atomic_write, config, grok_oauth, logger, provider_current_state};
use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use toml_edit::{table, value, Document};
use uuid::Uuid;

const INDEX_FILE: &str = "grok_accounts.json";
const ACCOUNTS_DIR: &str = "grok_accounts";
const PROFILES_DIR: &str = "grok_profiles";
const DEFAULT_HOME_DIR: &str = ".grok";
const AUTH_FILE: &str = "auth.json";
const CONFIG_FILE: &str = "config.toml";
const COCKPIT_API_MODEL_PROFILE: &str = "cockpit-api";
const BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const CLI_USER_URL: &str = "https://cli-chat-proxy.grok.com/v1/user?include=subscription";
const SUBSCRIPTIONS_URL: &str = "https://grok.com/rest/subscriptions";
const TASK_USAGE_URL: &str = "https://grok.com/rest/tasks/usage";
const FALLBACK_GROK_CLIENT_VERSION: &str = "0.2.93";
/// billing / user / task_usage 传输层瞬时失败（SSL EOF、断连、超时）重试次数
const TRANSPORT_MAX_ATTEMPTS: usize = 3;
const FILE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

static ACCOUNT_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));
static TOKEN_LOCKS: std::sync::LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
const QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 6 * 60 * 60;

struct SecretFileLock {
    path: PathBuf,
}

impl Drop for SecretFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn now_ts() -> i64 {
    Utc::now().timestamp()
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn normalize_api_base_url(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = normalize_text(value) else {
        return Ok(None);
    };
    let parsed = url::Url::parse(&value).map_err(|_| {
        "第三方 API Base URL 格式无效，请输入完整的 http:// 或 https:// 地址".to_string()
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("第三方 API Base URL 仅支持 http:// 或 https:// 地址".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("第三方 API Base URL 不能包含用户名或密码".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("第三方 API Base URL 不能包含查询参数或片段".to_string());
    }
    Ok(Some(value.trim_end_matches('/').to_string()))
}

fn validate_api_provider_config(
    api_base_url: Option<&str>,
    api_model: Option<&str>,
) -> Result<(Option<String>, Option<String>), String> {
    let api_base_url = normalize_api_base_url(api_base_url)?;
    let api_model = normalize_text(api_model);
    match (api_base_url, api_model) {
        (None, None) => Ok((None, None)),
        (Some(base_url), Some(model)) => Ok((Some(base_url), Some(model))),
        (Some(_), None) => Err("第三方 API 缺少模型 ID".to_string()),
        (None, Some(_)) => Err("第三方 API 缺少 Base URL".to_string()),
    }
}

fn data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn accounts_dir() -> Result<PathBuf, String> {
    let path = data_dir()?.join(ACCOUNTS_DIR);
    ensure_secret_dir(&path)?;
    Ok(path)
}

pub fn profiles_dir() -> Result<PathBuf, String> {
    let path = data_dir()?.join(PROFILES_DIR);
    ensure_secret_dir(&path)?;
    Ok(path)
}

fn index_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join(INDEX_FILE))
}

pub fn default_grok_home() -> Result<PathBuf, String> {
    Ok(dirs::home_dir()
        .ok_or_else(|| "无法获取用户主目录".to_string())?
        .join(DEFAULT_HOME_DIR))
}

pub fn managed_profile_dir(account_id: &str) -> Result<PathBuf, String> {
    Ok(profiles_dir()?.join(normalize_id(account_id)?))
}

fn normalize_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value.contains("..")
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err("Grok 账号 ID 非法".to_string());
    }
    Ok(value.to_string())
}

fn account_path(account_id: &str) -> Result<PathBuf, String> {
    Ok(accounts_dir()?.join(format!("{}.json", normalize_id(account_id)?)))
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| format!("设置文件权限失败({}): {}", path.display(), error))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

fn ensure_secret_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("创建 Grok 凭据目录失败({}): {}", path.display(), error))?;
    set_mode(path, 0o700)
}

fn acquire_secret_lock(path: &Path) -> Result<SecretFileLock, String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("grok-secret");
    let lock_path = path.with_file_name(format!("{}.cockpit.lock", file_name));
    let started = Instant::now();
    loop {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&lock_path) {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                let _ = file.sync_all();
                set_mode(&lock_path, 0o600)?;
                return Ok(SecretFileLock { path: lock_path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let owner_pid = fs::read_to_string(&lock_path)
                    .ok()
                    .and_then(|value| value.trim().parse::<u32>().ok());
                let owner_is_gone = owner_pid
                    .map(|pid| !crate::modules::process::is_pid_running(pid))
                    .unwrap_or(false);
                let lock_is_expired = fs::metadata(&lock_path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .map(|elapsed| elapsed > Duration::from_secs(120))
                    .unwrap_or(false);
                // Never steal a lock from a live process merely because a slow
                // network refresh has exceeded the age threshold.
                let owner_is_stale = owner_is_gone || (owner_pid.is_none() && lock_is_expired);
                if owner_is_stale {
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                if started.elapsed() >= FILE_LOCK_TIMEOUT {
                    return Err(format!("等待 Grok 凭据文件锁超时: {}", lock_path.display()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                return Err(format!("创建 Grok 凭据文件锁失败: {}", error));
            }
        }
    }
}

fn acquire_store_lock() -> Result<SecretFileLock, String> {
    acquire_secret_lock(&data_dir()?.join("grok-store"))
}

fn open_secret_temp(path: &Path) -> Result<std::fs::File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| format!("创建 Grok 凭据临时文件失败({}): {}", path.display(), error))
}

fn write_secret_atomic(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法定位 Grok 凭据目录: {}", path.display()))?;
    ensure_secret_dir(parent)?;
    let _lock = acquire_secret_lock(path)?;

    write_secret_atomic_locked(path, content)
}

fn write_secret_atomic_locked(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法定位 Grok 凭据目录: {}", path.display()))?;

    if path.exists() {
        if let Ok(current) = fs::read_to_string(path) {
            if serde_json::from_str::<Value>(&current).is_ok() {
                let backup = path.with_extension("json.bak");
                let backup_temp = parent.join(format!(".{}.{}.backup", AUTH_FILE, Uuid::new_v4()));
                let mut file = open_secret_temp(&backup_temp)?;
                file.write_all(current.as_bytes())
                    .map_err(|error| format!("写入 Grok 凭据备份失败: {}", error))?;
                file.sync_all()
                    .map_err(|error| format!("同步 Grok 凭据备份失败: {}", error))?;
                fs::rename(&backup_temp, &backup)
                    .map_err(|error| format!("替换 Grok 凭据备份失败: {}", error))?;
                set_mode(&backup, 0o600)?;
            }
        }
    }

    replace_secret_atomic_locked(path, content)
}

fn replace_secret_atomic_locked(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法定位 Grok 凭据目录: {}", path.display()))?;
    let temp = parent.join(format!(".{}.{}.atomic", AUTH_FILE, Uuid::new_v4()));
    let mut file = open_secret_temp(&temp)?;
    if let Err(error) = file.write_all(content.as_bytes()) {
        let _ = fs::remove_file(&temp);
        return Err(format!("写入 Grok 凭据临时文件失败: {}", error));
    }
    if let Err(error) = file.sync_all() {
        let _ = fs::remove_file(&temp);
        return Err(format!("同步 Grok 凭据临时文件失败: {}", error));
    }
    drop(file);
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("原子替换 Grok 凭据失败: {}", error));
    }
    set_mode(path, 0o600)
}

fn load_index() -> Result<GrokAccountIndex, String> {
    let path = index_path()?;
    let details_dir = data_dir()?.join(ACCOUNTS_DIR);
    load_index_from_paths(&path, &details_dir)
}

fn load_index_from_paths(path: &Path, details_dir: &Path) -> Result<GrokAccountIndex, String> {
    if !path.exists() {
        return Ok(GrokAccountIndex::default());
    }
    let content =
        fs::read_to_string(&path).map_err(|error| format!("读取 Grok 账号索引失败: {}", error))?;
    if content.trim().is_empty() {
        return Ok(GrokAccountIndex::default());
    }
    match atomic_write::parse_json_with_auto_restore(&path, &content) {
        Ok(index) => {
            set_mode(path, 0o600)?;
            Ok(index)
        }
        Err(error) => {
            logger::log_warn(&format!(
                "[Grok Account] 索引损坏，按账号详情重建: path={}, error={}",
                path.display(),
                error
            ));
            let _ = atomic_write::quarantine_file(&path, "invalid-json");
            let index = rebuild_index_from_details_at(details_dir)?;
            save_index_at(path, &index)?;
            Ok(index)
        }
    }
}

fn save_index(index: &GrokAccountIndex) -> Result<(), String> {
    save_index_at(&index_path()?, index)
}

fn save_index_at(path: &Path, index: &GrokAccountIndex) -> Result<(), String> {
    let content = serde_json::to_string_pretty(index)
        .map_err(|error| format!("序列化 Grok 账号索引失败: {}", error))?;
    write_secret_atomic(path, &content)
}

pub fn load_account(account_id: &str) -> Option<GrokAccount> {
    let path = account_path(account_id).ok()?;
    load_account_from_path(&path, account_id)
}

fn write_account_content_to_path(path: &Path, account: &GrokAccount) -> Result<(), String> {
    let content = crate::modules::secure_account_storage::serialize_account_file("grok", account)?;
    write_secret_atomic(path, &content)
}

fn load_account_from_path(path: &Path, account_id: &str) -> Option<GrokAccount> {
    let content = fs::read_to_string(path).ok()?;
    match crate::modules::secure_account_storage::deserialize_account_file::<GrokAccount>(
        path, &content,
    ) {
        Ok((account, needs_rotation)) => {
            // Backup restore may use the shared atomic writer. Tighten secret permissions
            // immediately; the encryption rewrite remains deferred off the read path.
            if let Err(error) = set_mode(path, 0o600) {
                logger::log_warn(&format!(
                    "[Grok Account] 收紧账号详情权限失败: account_id={}, error={}",
                    account_id, error
                ));
            }
            if needs_rotation {
                // Grok secrets use write_secret_atomic (not shared CAS writer). Still
                // re-check source bytes before rewrite so delayed jobs cannot clobber
                // a newer token rotation or resurrect a deleted account file.
                let account_for_rewrite = account.clone();
                let path_buf = path.to_path_buf();
                let account_id_owned = account_id.to_string();
                let expected_content = content.clone();
                crate::modules::deferred_account_rewrite::schedule_account_rewrite(
                    "grok",
                    account_for_rewrite.id.clone(),
                    move || {
                        match fs::read_to_string(&path_buf) {
                            Ok(current) if current == expected_content => {}
                            Ok(_) => {
                                logger::log_info(&format!(
                                    "[Grok Account] 延迟迁移跳过：源文件已变化 account_id={}",
                                    account_id_owned
                                ));
                                return;
                            }
                            Err(_) => {
                                logger::log_info(&format!(
                                    "[Grok Account] 延迟迁移跳过：源文件不存在 account_id={}",
                                    account_id_owned
                                ));
                                return;
                            }
                        }
                        // 若 path 即正式账号文件，走全量 save（含 index/profile）
                        if account_path(&account_id_owned).ok().as_deref()
                            == Some(path_buf.as_path())
                        {
                            let _ = save_account_file(&account_for_rewrite);
                        } else {
                            let _ = write_account_content_to_path(&path_buf, &account_for_rewrite);
                        }
                    },
                );
            }
            Some(account)
        }
        Err(error) => {
            let backup = path.with_extension("json.bak");
            let restored = fs::read_to_string(&backup).ok().and_then(|backup_content| {
                crate::modules::secure_account_storage::deserialize_account_file::<GrokAccount>(
                    &backup,
                    &backup_content,
                )
                .ok()
                .map(|(account, _)| account)
            });
            if let Some(account) = restored {
                let write_result = if account_path(account_id).ok().as_deref() == Some(path) {
                    save_account_file(&account)
                } else {
                    write_account_content_to_path(path, &account)
                };
                match write_result {
                    Ok(()) => {
                        logger::log_warn(&format!(
                            "[Grok Account] 账号详情损坏，已从备份恢复: account_id={}",
                            account_id
                        ));
                        return Some(account);
                    }
                    Err(restore_error) => logger::log_warn(&format!(
                        "[Grok Account] 账号详情损坏且备份恢复失败: account_id={}, parse_error={}, restore_error={}",
                        account_id, error, restore_error
                    )),
                }
            } else {
                logger::log_warn(&format!(
                    "[Grok Account] 账号详情损坏且没有有效备份: account_id={}, error={}",
                    account_id, error
                ));
            }
            None
        }
    }
}

fn save_account_file(account: &GrokAccount) -> Result<(), String> {
    let path = account_path(&account.id)?;
    let content = crate::modules::secure_account_storage::serialize_account_file("grok", account)?;
    write_secret_atomic(&path, &content)
}

fn save_account_locked(account: &GrokAccount) -> Result<(), String> {
    save_account_file(account)?;
    let mut index = load_index()?;
    if let Some(existing) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *existing = account.summary();
    } else {
        index.accounts.push(account.summary());
    }
    save_index(&index)?;
    write_account_to_profile(account, &managed_profile_dir(&account.id)?)
}

pub fn list_accounts_checked() -> Result<Vec<GrokAccountView>, String> {
    let index = load_index()?;
    let mut accounts = Vec::new();
    let mut repaired = false;
    for summary in index.accounts {
        if let Some(account) = load_account(&summary.id) {
            // A previous local test run could persist its fixed fixture into the real
            // data directory. Remove only the complete fixture fingerprint, never by
            // email alone, so a real account cannot be mistaken for test data.
            if is_known_test_fixture(&account) {
                match remove_account(&account.id) {
                    Ok(()) => {
                        repaired = true;
                        logger::log_warn(
                            "[Grok Account] 已清理历史测试夹具账号: account_id=account-1",
                        );
                        continue;
                    }
                    Err(error) => logger::log_warn(&format!(
                        "[Grok Account] 清理历史测试夹具账号失败: account_id={}, error={}",
                        account.id, error
                    )),
                }
            }
            accounts.push(GrokAccountView::from(&account));
        } else {
            repaired = true;
        }
    }
    if repaired {
        rebuild_index()?;
    }
    accounts.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(accounts)
}

fn is_known_test_fixture(account: &GrokAccount) -> bool {
    account.id == "account-1"
        && account.email == "person@example.com"
        && account.auth_mode == GrokAuthMode::Oauth
        && account.user_id.as_deref() == Some("user-1")
        && account.principal_id.as_deref() == Some("principal-1")
        && account.access_token == "secret-access"
        && account.refresh_token.as_deref() == Some("secret-refresh")
        && account.created_at == 1
        && account.last_used == 1
}

fn rebuild_index() -> Result<(), String> {
    let index = rebuild_index_from_details()?;
    save_index(&index)
}

fn rebuild_index_from_details() -> Result<GrokAccountIndex, String> {
    rebuild_index_from_details_at(&accounts_dir()?)
}

fn rebuild_index_from_details_at(details_dir: &Path) -> Result<GrokAccountIndex, String> {
    ensure_secret_dir(details_dir)?;
    let mut index = GrokAccountIndex::default();
    for entry in
        fs::read_dir(details_dir).map_err(|error| format!("扫描 Grok 账号目录失败: {}", error))?
    {
        let entry = entry.map_err(|error| format!("读取 Grok 账号目录项失败: {}", error))?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if let Ok(content) = fs::read_to_string(entry.path()) {
            if let Ok(account) = serde_json::from_str::<GrokAccount>(&content) {
                index.accounts.push(account.summary());
            }
        }
    }
    Ok(index)
}

fn official_auth_object(account: &GrokAccount) -> Value {
    let mut object = account
        .auth_raw
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    object.insert(
        "key".to_string(),
        Value::String(account.access_token.clone()),
    );
    object.insert("auth_mode".to_string(), Value::String("oidc".to_string()));
    object.insert("email".to_string(), Value::String(account.email.clone()));
    insert_optional(
        &mut object,
        "refresh_token",
        account.refresh_token.as_deref(),
    );
    insert_optional(&mut object, "user_id", account.user_id.as_deref());
    insert_optional(&mut object, "principal_id", account.principal_id.as_deref());
    insert_optional(
        &mut object,
        "principal_type",
        account.principal_type.as_deref(),
    );
    insert_optional(&mut object, "team_id", account.team_id.as_deref());
    insert_optional(&mut object, "first_name", account.first_name.as_deref());
    insert_optional(&mut object, "last_name", account.last_name.as_deref());
    insert_optional(
        &mut object,
        "profile_image_asset_id",
        account.profile_image_asset_id.as_deref(),
    );
    object.insert(
        "coding_data_retention_opt_out".to_string(),
        Value::Bool(account.coding_data_retention_opt_out.unwrap_or(false)),
    );
    object.insert(
        "oidc_issuer".to_string(),
        Value::String(
            account
                .oidc_issuer
                .clone()
                .unwrap_or_else(|| grok_oauth::OIDC_ISSUER.to_string()),
        ),
    );
    object.insert(
        "oidc_client_id".to_string(),
        Value::String(
            account
                .oidc_client_id
                .clone()
                .unwrap_or_else(|| grok_oauth::OIDC_CLIENT_ID.to_string()),
        ),
    );
    if let Some(expires_at_raw) = account
        .expires_at_raw
        .clone()
        .filter(|value| !value.is_null())
        .or_else(|| {
            object
                .get("expires_at")
                .filter(|value| !value.is_null())
                .cloned()
        })
    {
        object.insert("expires_at".to_string(), expires_at_raw);
    } else if let Some(expires_at) = account.expires_at {
        if let Some(timestamp) = DateTime::from_timestamp(expires_at, 0) {
            object.insert(
                "expires_at".to_string(),
                Value::String(timestamp.to_rfc3339()),
            );
        }
    }
    Value::Object(object)
}

fn insert_optional(object: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = normalize_text(value) {
        object.insert(key.to_string(), Value::String(value));
    } else {
        object.remove(key);
    }
}

fn auth_registry_key(issuer: Option<&str>, client_id: Option<&str>) -> String {
    let issuer = normalize_text(issuer)
        .unwrap_or_else(|| grok_oauth::OIDC_ISSUER.to_string())
        .trim_end_matches('/')
        .to_string();
    let client_id =
        normalize_text(client_id).unwrap_or_else(|| grok_oauth::OIDC_CLIENT_ID.to_string());
    format!("{}::{}", issuer, client_id)
}

fn account_auth_registry_key(account: &GrokAccount) -> String {
    auth_registry_key(
        account.oidc_issuer.as_deref(),
        account.oidc_client_id.as_deref(),
    )
}

fn split_xai_auth_registry_key(key: &str) -> Option<(&str, &str)> {
    let (issuer, client_id) = key.rsplit_once("::")?;
    if issuer.trim_end_matches('/') != grok_oauth::OIDC_ISSUER || client_id.trim().is_empty() {
        return None;
    }
    Some((issuer, client_id))
}

fn auth_registry_for(account: &GrokAccount, existing: Option<Value>) -> Value {
    let mut registry = existing
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let matching_keys = registry
        .iter()
        .filter(|(key, value)| {
            split_xai_auth_registry_key(key).is_some() && auth_entry_matches_account(value, account)
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    for key in matching_keys {
        registry.remove(&key);
    }
    registry.insert(
        account_auth_registry_key(account),
        official_auth_object(account),
    );
    Value::Object(registry)
}

fn read_auth_registry(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("读取 Grok 默认凭据失败: {}", error))?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    let registry: Value = serde_json::from_str(&content)
        .map_err(|error| format!("解析 Grok 默认凭据失败: {}", error))?;
    if !registry.is_object() {
        return Err("Grok 默认凭据不是 registry 对象".to_string());
    }
    Ok(Some(registry))
}

fn auth_registry_entry(registry: &Value) -> Option<&Map<String, Value>> {
    if let Some(entry) = registry
        .get(grok_oauth::AUTH_REGISTRY_KEY)
        .and_then(Value::as_object)
    {
        return Some(entry);
    }
    registry.as_object()?.iter().find_map(|(key, value)| {
        split_xai_auth_registry_key(key)?;
        value.as_object()
    })
}

fn unique_account_match<'a, F>(
    accounts: &'a [GrokAccount],
    mut predicate: F,
) -> Option<&'a GrokAccount>
where
    F: FnMut(&GrokAccount) -> bool,
{
    let mut matches = accounts.iter().filter(|account| predicate(account));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn normalized_identity(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn compare_strong_identity(
    left_principal_id: Option<&str>,
    left_user_id: Option<&str>,
    right_principal_id: Option<&str>,
    right_user_id: Option<&str>,
) -> Option<bool> {
    let left = [
        normalized_identity(left_principal_id),
        normalized_identity(left_user_id),
    ];
    let right = [
        normalized_identity(right_principal_id),
        normalized_identity(right_user_id),
    ];
    if left.iter().all(Option::is_none) || right.iter().all(Option::is_none) {
        return None;
    }
    for (left_value, right_value) in left.iter().zip(right.iter()) {
        if let (Some(left_value), Some(right_value)) = (left_value, right_value) {
            if left_value != right_value {
                return Some(false);
            }
        }
    }
    Some(left.iter().flatten().any(|left_value| {
        right
            .iter()
            .flatten()
            .any(|right_value| left_value == right_value)
    }))
}

fn resolve_account_id_from_registry(accounts: &[GrokAccount], registry: &Value) -> Option<String> {
    let auth = auth_registry_entry(registry)?;
    if let Some(access_token) = string_field(auth, "key") {
        if let Some(account) =
            unique_account_match(accounts, |account| account.access_token == access_token)
        {
            return Some(account.id.clone());
        }
    }
    let principal_id = string_field(auth, "principal_id");
    let user_id = string_field(auth, "user_id");
    if principal_id.is_some() || user_id.is_some() {
        return unique_account_match(accounts, |account| {
            compare_strong_identity(
                principal_id.as_deref(),
                user_id.as_deref(),
                account.principal_id.as_deref(),
                account.user_id.as_deref(),
            ) == Some(true)
        })
        .map(|account| account.id.clone());
    }
    if let Some(email) = string_field(auth, "email") {
        if let Some(account) = unique_account_match(accounts, |account| {
            account.email.eq_ignore_ascii_case(&email)
        }) {
            return Some(account.id.clone());
        }
    }
    None
}

fn reconcile_current_account_id() -> Result<Option<String>, String> {
    let auth_path = default_grok_home()?.join(AUTH_FILE);
    let registry = read_auth_registry(&auth_path)?;
    let resolved = if let Some(registry) = registry.as_ref() {
        let accounts = load_index()?
            .accounts
            .into_iter()
            .filter_map(|summary| load_account(&summary.id))
            .collect::<Vec<_>>();
        resolve_account_id_from_registry(&accounts, registry)
    } else {
        None
    };
    let tracked = provider_current_state::get_current_account_id("grok")?;
    if tracked != resolved {
        provider_current_state::set_current_account_id("grok", resolved.as_deref())?;
    }
    Ok(resolved)
}

fn write_account_to_auth_path_if_token_matches(
    account: &GrokAccount,
    auth_path: &Path,
    expected_access_token: &str,
) -> Result<bool, String> {
    let parent = auth_path
        .parent()
        .ok_or_else(|| format!("无法定位 Grok 凭据目录: {}", auth_path.display()))?;
    ensure_secret_dir(parent)?;
    let _lock = acquire_secret_lock(auth_path)?;
    let Some(existing) = read_auth_registry(auth_path)? else {
        return Ok(false);
    };
    let matches_expected = auth_registry_entry(&existing)
        .and_then(|auth| auth.get("key"))
        .and_then(Value::as_str)
        .map(|token| token == expected_access_token)
        .unwrap_or(false);
    if !matches_expected {
        return Ok(false);
    }
    let content = serde_json::to_string_pretty(&auth_registry_for(account, Some(existing)))
        .map_err(|error| format!("序列化 Grok 默认凭据失败: {}", error))?;
    write_secret_atomic_locked(auth_path, &content)?;
    Ok(true)
}

fn write_empty_auth_file(auth_path: &Path) -> Result<(), String> {
    // API key auth uses XAI_API_KEY at launch time. Clear session auth so OAuth
    // tokens do not outrank the environment key (official credential priority).
    write_secret_atomic(auth_path, "{}")
}

fn write_api_key_profile_config(account: &GrokAccount, profile_dir: &Path) -> Result<(), String> {
    let (Some(base_url), Some(model)) = (
        account.api_base_url.as_deref(),
        account.api_model.as_deref(),
    ) else {
        return Ok(());
    };
    let config_path = profile_dir.join(CONFIG_FILE);
    let mut document = if config_path.exists() {
        fs::read_to_string(&config_path)
            .map_err(|error| format!("读取 Grok CLI 配置失败: {}", error))?
            .parse::<Document>()
            .map_err(|error| format!("解析 Grok CLI config.toml 失败: {}", error))?
    } else {
        Document::new()
    };
    if !document
        .as_table()
        .get("models")
        .is_some_and(toml_edit::Item::is_table)
    {
        document["models"] = table();
    }
    document["models"]["default"] = value(COCKPIT_API_MODEL_PROFILE);
    if !document
        .as_table()
        .get("model")
        .is_some_and(toml_edit::Item::is_table)
    {
        document["model"] = table();
    }
    document["model"][COCKPIT_API_MODEL_PROFILE] = table();
    let model_config = &mut document["model"][COCKPIT_API_MODEL_PROFILE];
    model_config["model"] = value(model);
    model_config["base_url"] = value(base_url);
    model_config["name"] = value(model);
    // Keep the secret out of config.toml. The launcher injects XAI_API_KEY only
    // into the selected account process, and Grok resolves it through env_key.
    model_config["env_key"] = value("XAI_API_KEY");
    write_secret_atomic(&config_path, &document.to_string())
}

fn write_account_to_official_auth_path(
    account: &GrokAccount,
    auth_path: &Path,
) -> Result<bool, String> {
    if account.is_api_key_auth() {
        if account.resolved_api_key().is_none() {
            return Err("Grok API Key 账号缺少 api_key".to_string());
        }
        // API Key 仅通过启动时的 XAI_API_KEY 生效，绝不清空或覆盖官方 OAuth 登录。
        return Ok(false);
    }

    let parent = auth_path
        .parent()
        .ok_or_else(|| format!("无法定位 Grok 凭据目录: {}", auth_path.display()))?;
    ensure_secret_dir(parent)?;
    let _lock = acquire_secret_lock(auth_path)?;
    let existing = read_auth_registry(auth_path)?;
    let content = serde_json::to_string_pretty(&auth_registry_for(account, existing))
        .map_err(|error| format!("序列化 Grok 默认凭据失败: {}", error))?;
    write_secret_atomic_locked(auth_path, &content)?;
    Ok(true)
}

pub fn write_account_to_profile(account: &GrokAccount, profile_dir: &Path) -> Result<(), String> {
    ensure_secret_dir(profile_dir)?;
    let auth_path = profile_dir.join(AUTH_FILE);
    if account.is_api_key_auth() {
        if account.resolved_api_key().is_none() {
            return Err("Grok API Key 账号缺少 api_key".to_string());
        }
        write_empty_auth_file(&auth_path)?;
        return write_api_key_profile_config(account, profile_dir);
    }
    let existing = fs::read_to_string(&auth_path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok());
    let content = serde_json::to_string_pretty(&auth_registry_for(account, existing))
        .map_err(|error| format!("序列化 Grok 官方凭据失败: {}", error))?;
    write_secret_atomic(&auth_path, &content)
}

/// 准备账号独立运行目录（GROK_HOME），不写官方默认 `~/.grok`，也不维护「当前账号」。
pub fn prepare_account_home(account_id: &str) -> Result<(String, PathBuf), String> {
    let _guard = ACCOUNT_LOCK.lock().map_err(|_| "获取 Grok 账号锁失败")?;
    let _store_guard = acquire_store_lock()?;
    let mut account =
        load_account(account_id).ok_or_else(|| format!("Grok 账号不存在: {}", account_id))?;
    account.last_used = now_ms();
    save_account_locked(&account)?;
    let home = managed_profile_dir(&account.id)?;
    write_account_to_profile(&account, &home)?;
    Ok((account.email, home))
}

/// 将默认实例切换到指定账号。OAuth 写入官方 auth.json；API Key 仅记录选择，
/// 实际凭据由默认实例启动命令通过 XAI_API_KEY 注入。
pub fn inject_to_default(account_id: &str) -> Result<String, String> {
    let _guard = ACCOUNT_LOCK.lock().map_err(|_| "获取 Grok 账号锁失败")?;
    let _store_guard = acquire_store_lock()?;
    let mut account =
        load_account(account_id).ok_or_else(|| format!("Grok 账号不存在: {}", account_id))?;
    account.last_used = now_ms();
    save_account_locked(&account)?;

    let auth_path = default_grok_home()?.join(AUTH_FILE);
    let wrote_official_auth = write_account_to_official_auth_path(&account, &auth_path)?;
    provider_current_state::set_current_account_id("grok", Some(account_id))?;
    if wrote_official_auth {
        logger::log_info(&format!(
            "[Grok Account] 已同步官方登录: account_id={}, auth_path={}",
            account_id,
            auth_path.display()
        ));
    } else {
        logger::log_info(&format!(
            "[Grok Account] API Key 账号已选中，官方 auth.json 保持不变: account_id={}",
            account_id
        ));
    }
    Ok(account.email)
}

fn account_from_auth_object(value: &Value) -> Result<GrokAccount, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "Grok auth.json 账号记录必须是对象".to_string())?;
    let access_token = object
        .get("key")
        .and_then(Value::as_str)
        .and_then(|value| normalize_text(Some(value)))
        .ok_or_else(|| "Grok auth.json 缺少 key".to_string())?;
    let email = object
        .get("email")
        .and_then(Value::as_str)
        .and_then(|value| normalize_text(Some(value)))
        .unwrap_or_else(|| "unknown@grok.local".to_string());
    let expires_at = object.get("expires_at").and_then(parse_timestamp);
    let created_at = object
        .get("create_time")
        .and_then(parse_timestamp)
        .map(|value| value * 1000)
        .unwrap_or_else(now_ms);
    Ok(GrokAccount {
        id: Uuid::new_v4().to_string(),
        email,
        auth_mode: GrokAuthMode::Oauth,
        tags: None,
        first_name: string_field(object, "first_name"),
        last_name: string_field(object, "last_name"),
        user_id: string_field(object, "user_id"),
        principal_id: string_field(object, "principal_id"),
        principal_type: string_field(object, "principal_type"),
        team_id: string_field(object, "team_id"),
        profile_image_asset_id: string_field(object, "profile_image_asset_id"),
        coding_data_retention_opt_out: object
            .get("coding_data_retention_opt_out")
            .and_then(Value::as_bool),
        access_token,
        api_key: None,
        api_base_url: None,
        api_model: None,
        refresh_token: string_field(object, "refresh_token"),
        id_token: None,
        token_type: Some("Bearer".to_string()),
        expires_at,
        expires_at_raw: object
            .get("expires_at")
            .filter(|value| !value.is_null())
            .cloned(),
        oidc_issuer: string_field(object, "oidc_issuer")
            .or_else(|| Some(grok_oauth::OIDC_ISSUER.to_string())),
        oidc_client_id: string_field(object, "oidc_client_id")
            .or_else(|| Some(grok_oauth::OIDC_CLIENT_ID.to_string())),
        token_endpoint: Some(grok_oauth::DEFAULT_TOKEN_ENDPOINT.to_string()),
        plan_type: None,
        quota: None,
        auth_raw: Some(value.clone()),
        billing_raw: None,
        subscription_raw: None,
        user_raw: None,
        task_usage_raw: None,
        has_grok_code_access: None,
        status: None,
        status_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        working_dir: None,
        created_at,
        last_used: now_ms(),
    })
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| normalize_text(Some(value)))
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().map(|value| {
            if value > 10_000_000_000 {
                value / 1000
            } else {
                value
            }
        }),
        Value::String(text) => text
            .parse::<i64>()
            .ok()
            .map(|value| {
                if value > 10_000_000_000 {
                    value / 1000
                } else {
                    value
                }
            })
            .or_else(|| {
                DateTime::parse_from_rfc3339(text)
                    .ok()
                    .map(|date| date.timestamp())
            }),
        _ => None,
    }
}

fn find_existing_account(candidate: &GrokAccount) -> Option<GrokAccount> {
    let index = load_index().ok()?;
    index.accounts.into_iter().find_map(|summary| {
        let account = load_account(&summary.id)?;
        accounts_match_for_upsert(candidate, &account).then_some(account)
    })
}

fn mask_api_key_email(api_key: &str) -> String {
    let trimmed = api_key.trim();
    let suffix: String = trimmed
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if suffix.is_empty() {
        "API_KEY".to_string()
    } else {
        format!("xai-****{}", suffix)
    }
}

fn api_key_account_display(
    api_key: &str,
    api_base_url: Option<&str>,
    api_model: Option<&str>,
) -> String {
    match (api_base_url, api_model) {
        (Some(base_url), Some(model)) => url::Url::parse(base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .map(|host| format!("{} @ {}", model, host))
            .unwrap_or_else(|| model.to_string()),
        _ => mask_api_key_email(api_key),
    }
}

fn api_key_account_id(
    api_key: &str,
    api_base_url: Option<&str>,
    api_model: Option<&str>,
) -> String {
    let identity = match (api_base_url, api_model) {
        (Some(base_url), Some(model)) => {
            format!("{}\n{}\n{}", api_key.trim(), base_url.trim(), model.trim())
        }
        _ => api_key.trim().to_string(),
    };
    format!("{:x}", md5::compute(identity.as_bytes()))
}

fn accounts_match_for_upsert(candidate: &GrokAccount, existing: &GrokAccount) -> bool {
    if candidate.auth_mode != existing.auth_mode {
        return false;
    }
    if candidate.is_api_key_auth() {
        return match (candidate.resolved_api_key(), existing.resolved_api_key()) {
            (Some(left), Some(right)) => {
                left == right
                    && candidate.api_base_url == existing.api_base_url
                    && candidate.api_model == existing.api_model
            }
            _ => false,
        };
    }
    if let Some(matches) = compare_strong_identity(
        candidate.principal_id.as_deref(),
        candidate.user_id.as_deref(),
        existing.principal_id.as_deref(),
        existing.user_id.as_deref(),
    ) {
        return matches;
    }
    candidate.email != "unknown@grok.local" && existing.email.eq_ignore_ascii_case(&candidate.email)
}

fn resolve_reauth_target(
    candidate: &GrokAccount,
    target_account_id: Option<&str>,
) -> Result<Option<GrokAccount>, String> {
    let Some(target_account_id) = target_account_id else {
        return Ok(None);
    };
    let target_account_id = normalize_id(target_account_id)?;
    let target = load_account(&target_account_id)
        .ok_or_else(|| format!("Grok 重新授权目标账号不存在: {}", target_account_id))?;
    if !target.email.trim().is_empty()
        && target.email != "unknown@grok.local"
        && !target.email.eq_ignore_ascii_case(&candidate.email)
    {
        return Err(format!(
            "Grok 重新授权账号邮箱不匹配: 目标账号为 {}，本次授权为 {}",
            target.email, candidate.email
        ));
    }
    Ok(Some(target))
}

fn upsert_candidate(
    mut candidate: GrokAccount,
    reauth_target_account_id: Option<&str>,
) -> Result<GrokAccount, String> {
    if candidate.is_api_key_auth() {
        let api_key = candidate
            .resolved_api_key()
            .ok_or_else(|| "Grok API Key 账号缺少 api_key".to_string())?;
        if api_key.contains(char::is_whitespace) {
            return Err("API Key 格式无效".to_string());
        }
        let (api_base_url, api_model) = validate_api_provider_config(
            candidate.api_base_url.as_deref(),
            candidate.api_model.as_deref(),
        )?;
        candidate.api_base_url = api_base_url;
        candidate.api_model = api_model;
    }
    let _guard = ACCOUNT_LOCK.lock().map_err(|_| "获取 Grok 账号锁失败")?;
    let _store_guard = acquire_store_lock()?;
    let existing = resolve_reauth_target(&candidate, reauth_target_account_id)?
        .or_else(|| find_existing_account(&candidate));
    if let Some(existing) = existing {
        if candidate.refresh_token.is_none() {
            candidate.refresh_token = existing.refresh_token.clone();
        }
        if candidate.id_token.is_none() {
            candidate.id_token = existing.id_token.clone();
        }
        if candidate.token_type.is_none() {
            candidate.token_type = existing.token_type.clone();
        }
        if candidate.expires_at.is_none() {
            candidate.expires_at = existing.expires_at;
        }
        if candidate.token_endpoint.is_none() {
            candidate.token_endpoint = existing.token_endpoint.clone();
        }
        if candidate.oidc_issuer.is_none() {
            candidate.oidc_issuer = existing.oidc_issuer.clone();
        }
        if candidate.oidc_client_id.is_none() {
            candidate.oidc_client_id = existing.oidc_client_id.clone();
        }
        let mut merged_auth = existing
            .auth_raw
            .as_ref()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(incoming) = candidate.auth_raw.as_ref().and_then(Value::as_object) {
            merged_auth.extend(incoming.clone());
        }
        candidate.auth_raw = (!merged_auth.is_empty()).then_some(Value::Object(merged_auth));
        candidate.id = existing.id;
        candidate.auth_mode = existing.auth_mode;
        if candidate.api_key.is_none() {
            candidate.api_key = existing.api_key.clone();
        }
        candidate.tags = existing.tags;
        candidate.created_at = existing.created_at;
        if !candidate.is_api_key_auth() {
            candidate.plan_type = existing.plan_type;
        }
        candidate.quota = existing.quota;
        candidate.billing_raw = existing.billing_raw;
        candidate.subscription_raw = existing.subscription_raw;
        candidate.user_raw = existing.user_raw;
        candidate.task_usage_raw = existing.task_usage_raw;
        candidate.has_grok_code_access = candidate
            .has_grok_code_access
            .or(existing.has_grok_code_access);
        candidate.quota_query_last_error = existing.quota_query_last_error;
        candidate.quota_query_last_error_at = existing.quota_query_last_error_at;
        candidate.usage_updated_at = existing.usage_updated_at;
        candidate.working_dir = existing.working_dir;
    }
    save_account_locked(&candidate)?;
    Ok(candidate)
}

fn oauth_account_candidate(payload: GrokOAuthCompletePayload) -> GrokAccount {
    let now = now_ms();
    let expires_at_raw = payload
        .auth_raw
        .get("expires_at")
        .filter(|value| !value.is_null())
        .cloned();
    GrokAccount {
        id: Uuid::new_v4().to_string(),
        email: payload.email,
        auth_mode: GrokAuthMode::Oauth,
        tags: None,
        first_name: payload.first_name,
        last_name: payload.last_name,
        user_id: payload.user_id,
        principal_id: payload.principal_id,
        principal_type: payload.principal_type,
        team_id: payload.team_id,
        profile_image_asset_id: payload.profile_image_asset_id,
        coding_data_retention_opt_out: payload.coding_data_retention_opt_out,
        access_token: payload.access_token,
        api_key: None,
        api_base_url: None,
        api_model: None,
        refresh_token: payload.refresh_token,
        id_token: payload.id_token,
        token_type: payload.token_type.or_else(|| Some("Bearer".to_string())),
        expires_at: payload.expires_at,
        expires_at_raw,
        oidc_issuer: Some(grok_oauth::OIDC_ISSUER.to_string()),
        oidc_client_id: Some(grok_oauth::OIDC_CLIENT_ID.to_string()),
        token_endpoint: Some(payload.token_endpoint),
        plan_type: None,
        quota: None,
        auth_raw: Some(payload.auth_raw),
        billing_raw: None,
        subscription_raw: None,
        user_raw: None,
        task_usage_raw: None,
        has_grok_code_access: None,
        status: None,
        status_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        working_dir: None,
        created_at: now,
        last_used: now,
    }
}

pub fn upsert_oauth(payload: GrokOAuthCompletePayload) -> Result<GrokAccount, String> {
    upsert_candidate(oauth_account_candidate(payload), None)
}

pub fn upsert_oauth_for_reauth(
    payload: GrokOAuthCompletePayload,
    target_account_id: &str,
) -> Result<GrokAccount, String> {
    upsert_candidate(oauth_account_candidate(payload), Some(target_account_id))
}

pub fn upsert_api_key(
    api_key: &str,
    api_base_url: Option<&str>,
    api_model: Option<&str>,
) -> Result<GrokAccountView, String> {
    let api_key = normalize_text(Some(api_key)).ok_or_else(|| "API Key 不能为空".to_string())?;
    if api_key.contains(char::is_whitespace) {
        return Err("API Key 格式无效".to_string());
    }
    let (api_base_url, api_model) = validate_api_provider_config(api_base_url, api_model)?;
    let now = now_ms();
    let candidate = GrokAccount {
        id: api_key_account_id(&api_key, api_base_url.as_deref(), api_model.as_deref()),
        email: api_key_account_display(&api_key, api_base_url.as_deref(), api_model.as_deref()),
        auth_mode: GrokAuthMode::ApiKey,
        tags: None,
        first_name: None,
        last_name: None,
        user_id: None,
        principal_id: None,
        principal_type: None,
        team_id: None,
        profile_image_asset_id: None,
        coding_data_retention_opt_out: None,
        access_token: String::new(),
        api_key: Some(api_key),
        api_base_url,
        api_model,
        refresh_token: None,
        id_token: None,
        token_type: None,
        expires_at: None,
        expires_at_raw: None,
        oidc_issuer: None,
        oidc_client_id: None,
        token_endpoint: None,
        plan_type: Some("API_KEY".to_string()),
        quota: None,
        auth_raw: None,
        billing_raw: None,
        subscription_raw: None,
        user_raw: None,
        task_usage_raw: None,
        has_grok_code_access: None,
        status: Some("normal".to_string()),
        status_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: Some(now),
        working_dir: None,
        created_at: now,
        last_used: now,
    };
    let account = upsert_candidate(candidate, None)?;
    Ok(GrokAccountView::from(&account))
}

fn parse_auth_registry(value: &Value) -> Result<GrokAccount, String> {
    if let Some(auth) = value.get(grok_oauth::AUTH_REGISTRY_KEY) {
        return account_from_auth_object(auth);
    }
    if let Some((registry_key, auth)) = value.as_object().and_then(|registry| {
        registry
            .iter()
            .find(|(key, auth)| split_xai_auth_registry_key(key).is_some() && auth.is_object())
    }) {
        let mut auth = auth.clone();
        if let (Some((issuer, client_id)), Some(object)) = (
            split_xai_auth_registry_key(registry_key),
            auth.as_object_mut(),
        ) {
            object
                .entry("oidc_issuer".to_string())
                .or_insert_with(|| Value::String(issuer.to_string()));
            object
                .entry("oidc_client_id".to_string())
                .or_insert_with(|| Value::String(client_id.to_string()));
        }
        return account_from_auth_object(&auth);
    }
    if value.get("key").is_some() {
        return account_from_auth_object(value);
    }
    if let Ok(account) = serde_json::from_value::<GrokAccount>(value.clone()) {
        validate_importable_account(&account)?;
        return Ok(account);
    }
    Err("未识别 Grok JSON 格式（支持官方 auth.json 或 Cockpit 导出账号数组）".to_string())
}

/// 是否具备可恢复的登录凭据（OAuth access/refresh 或 API Key）。
fn account_has_importable_credentials(account: &GrokAccount) -> bool {
    if account.is_api_key_auth() {
        return account.resolved_api_key().is_some();
    }
    normalize_text(Some(&account.access_token)).is_some()
        || account
            .refresh_token
            .as_deref()
            .and_then(|value| normalize_text(Some(value)))
            .is_some()
}

fn validate_importable_account(account: &GrokAccount) -> Result<(), String> {
    if account_has_importable_credentials(account) {
        return Ok(());
    }
    Err(
        "Grok 导入 JSON 缺少登录凭据（access_token / refresh_token / api_key），不能用于恢复账号；请使用官方 auth.json 或本应用导出的完整 JSON"
            .to_string(),
    )
}

fn import_full_accounts(accounts: Vec<GrokAccount>) -> Result<Vec<GrokAccountView>, String> {
    if accounts.is_empty() {
        return Err("导入数组为空".to_string());
    }
    let mut result = Vec::new();
    for (idx, account) in accounts.into_iter().enumerate() {
        validate_importable_account(&account)
            .map_err(|error| format!("第 {} 条记录解析失败: {}", idx + 1, error))?;
        let saved = upsert_candidate(account, None)
            .map_err(|error| format!("第 {} 条记录导入失败: {}", idx + 1, error))?;
        result.push(GrokAccountView::from(&saved));
    }
    Ok(result)
}

/// 尝试按 Cockpit 完整账号结构解析（导出格式）。
fn try_import_cockpit_export(value: &Value) -> Option<Result<Vec<GrokAccountView>, String>> {
    // 数组：[GrokAccount, ...]
    if let Ok(accounts) = serde_json::from_value::<Vec<GrokAccount>>(value.clone()) {
        if !accounts.is_empty()
            && accounts
                .iter()
                .any(|account| account_has_importable_credentials(account))
        {
            return Some(import_full_accounts(accounts));
        }
        // 可能是脱敏视图数组，交给后续路径给出明确错误
        if !accounts.is_empty() {
            return Some(Err(
                "Grok 导入 JSON 缺少登录凭据（access_token / refresh_token / api_key），不能用于恢复账号；请使用官方 auth.json 或本应用导出的完整 JSON"
                    .to_string(),
            ));
        }
    }
    // 单对象账号
    if let Ok(account) = serde_json::from_value::<GrokAccount>(value.clone()) {
        if account_has_importable_credentials(&account) {
            return Some(import_full_accounts(vec![account]));
        }
        // 可能是脱敏视图：若字段像账号但无凭据，直接报错
        if !account.id.trim().is_empty() || !account.email.trim().is_empty() {
            return Some(Err(
                "Grok 导入 JSON 缺少登录凭据（access_token / refresh_token / api_key），不能用于恢复账号；请使用官方 auth.json 或本应用导出的完整 JSON"
                    .to_string(),
            ));
        }
    }
    // 包装：{ "accounts": [...] } / { "items": [...] }
    if let Some(items) = value
        .get("accounts")
        .or_else(|| value.get("items"))
        .and_then(Value::as_array)
    {
        if let Ok(accounts) =
            serde_json::from_value::<Vec<GrokAccount>>(Value::Array(items.clone()))
        {
            return Some(import_full_accounts(accounts));
        }
    }
    None
}

pub fn import_from_local() -> Result<Vec<GrokAccountView>, String> {
    let path = default_grok_home()?.join(AUTH_FILE);
    if !path.exists() {
        return Err("未找到本机 Grok CLI 登录信息".to_string());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取本机 Grok auth.json 失败: {}", error))?;
    let accounts = import_from_json(&content)?;
    Ok(accounts)
}

pub fn import_from_json(content: &str) -> Result<Vec<GrokAccountView>, String> {
    let value: Value =
        serde_json::from_str(content).map_err(|error| format!("解析 Grok JSON 失败: {}", error))?;

    // 1) Cockpit 导出的完整账号（含凭据）— 与文件导入 / Token 粘贴共用
    if let Some(result) = try_import_cockpit_export(&value) {
        return result;
    }

    // 2) 官方 auth.json / 注册表对象 / 数组
    let values = if let Some(items) = value.as_array() {
        items.clone()
    } else {
        vec![value]
    };
    if values.is_empty() {
        return Err("导入数组为空".to_string());
    }
    let mut accounts = Vec::new();
    for (idx, value) in values.into_iter().enumerate() {
        let candidate = parse_auth_registry(&value)
            .map_err(|error| format!("第 {} 条记录解析失败: {}", idx + 1, error))?;
        let account = upsert_candidate(candidate, None)
            .map_err(|error| format!("第 {} 条记录导入失败: {}", idx + 1, error))?;
        accounts.push(GrokAccountView::from(&account));
    }
    Ok(accounts)
}

/// 导出完整账号 JSON（含凭据），可再导入恢复；与 Codex / WorkBuddy 等平台一致。
pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<GrokAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();
    if accounts.is_empty() {
        return Err("没有可导出的 Grok 账号".to_string());
    }
    serde_json::to_string_pretty(&accounts)
        .map_err(|error| format!("序列化 Grok 导出失败: {}", error))
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let id = normalize_id(account_id)?;
    // 删除时自动解绑默认/多开实例，无需用户先手动解绑
    let unbound = crate::modules::grok_instance::unbind_account(&id)?;
    if !unbound.is_empty() {
        logger::log_info(&format!(
            "删除 Grok 账号前已自动解绑实例: account_id={}, instances={}",
            id,
            unbound.join(", ")
        ));
    }
    let _guard = ACCOUNT_LOCK.lock().map_err(|_| "获取 Grok 账号锁失败")?;
    let _store_guard = acquire_store_lock()?;
    let path = account_path(&id)?;
    let backup_path = path.with_extension("json.bak");
    if backup_path.exists() {
        fs::remove_file(&backup_path)
            .map_err(|error| format!("删除 Grok 账号备份失败: {}", error))?;
    }
    if path.exists() {
        fs::remove_file(&path).map_err(|error| format!("删除 Grok 账号失败: {}", error))?;
    }
    let profile = managed_profile_dir(&id)?;
    if profile.exists() {
        fs::remove_dir_all(&profile)
            .map_err(|error| format!("删除 Grok profile 失败: {}", error))?;
    }
    let mut index = load_index()?;
    index.accounts.retain(|item| item.id != id);
    save_index(&index)?;
    Ok(())
}

fn remove_matching_default_auth(account: &GrokAccount) -> Result<(), String> {
    let auth_path = default_grok_home()?.join(AUTH_FILE);
    remove_matching_auth_scope(&auth_path, account, true)?;
    remove_matching_auth_scope(&auth_path.with_extension("json.bak"), account, false)?;
    Ok(())
}

fn auth_entry_matches_account(current: &Value, account: &GrokAccount) -> bool {
    let current_object = current.as_object();
    let same_token = current_object
        .and_then(|value| value.get("key"))
        .and_then(Value::as_str)
        .map(|value| value == account.access_token)
        .unwrap_or(false);
    if same_token {
        return true;
    }
    let current_principal_id = current_object
        .and_then(|value| value.get("principal_id"))
        .and_then(Value::as_str);
    let current_user_id = current_object
        .and_then(|value| value.get("user_id"))
        .and_then(Value::as_str);
    if let Some(matches) = compare_strong_identity(
        current_principal_id,
        current_user_id,
        account.principal_id.as_deref(),
        account.user_id.as_deref(),
    ) {
        return matches;
    }
    let same_email = current_object
        .and_then(|value| value.get("email"))
        .and_then(Value::as_str)
        .map(|value| value.eq_ignore_ascii_case(&account.email))
        .unwrap_or(false);
    same_email
}

fn remove_matching_auth_scope(
    path: &Path,
    account: &GrokAccount,
    create_backup: bool,
) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let _lock = acquire_secret_lock(path)?;
    let content =
        fs::read_to_string(path).map_err(|error| format!("读取 Grok 默认凭据失败: {}", error))?;
    let mut registry: Map<String, Value> = serde_json::from_str::<Value>(&content)
        .map_err(|error| format!("解析 Grok 默认凭据失败: {}", error))?
        .as_object()
        .cloned()
        .ok_or_else(|| "Grok 默认凭据不是 registry 对象".to_string())?;
    let matching_keys = registry
        .iter()
        .filter(|(key, current)| {
            split_xai_auth_registry_key(key).is_some()
                && auth_entry_matches_account(current, account)
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if matching_keys.is_empty() {
        return Ok(false);
    }
    for key in matching_keys {
        registry.remove(&key);
    }
    if registry.is_empty() {
        fs::remove_file(path).map_err(|error| format!("删除 Grok 默认凭据失败: {}", error))?;
    } else {
        let next = serde_json::to_string_pretty(&Value::Object(registry))
            .map_err(|error| format!("序列化 Grok 默认凭据失败: {}", error))?;
        if create_backup {
            write_secret_atomic_locked(path, &next)?;
        } else {
            replace_secret_atomic_locked(path, &next)?;
        }
    }
    Ok(true)
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for account_id in account_ids {
        remove_account(account_id)?;
    }
    Ok(())
}

pub fn update_tags(account_id: &str, tags: Vec<String>) -> Result<GrokAccountView, String> {
    let _guard = ACCOUNT_LOCK.lock().map_err(|_| "获取 Grok 账号锁失败")?;
    let _store_guard = acquire_store_lock()?;
    let mut account =
        load_account(account_id).ok_or_else(|| format!("Grok 账号不存在: {}", account_id))?;
    let mut normalized = Vec::new();
    for tag in tags {
        if let Some(tag) = normalize_text(Some(&tag)) {
            if !normalized.iter().any(|existing| existing == &tag) {
                normalized.push(tag);
            }
        }
    }
    account.tags = (!normalized.is_empty()).then_some(normalized);
    save_account_locked(&account)?;
    Ok(GrokAccountView::from(&account))
}

pub fn update_working_dir(
    account_id: &str,
    working_dir: Option<String>,
) -> Result<GrokAccountView, String> {
    let _guard = ACCOUNT_LOCK.lock().map_err(|_| "获取 Grok 账号锁失败")?;
    let _store_guard = acquire_store_lock()?;
    let mut account =
        load_account(account_id).ok_or_else(|| format!("Grok 账号不存在: {}", account_id))?;
    let normalized = working_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    if let Some(ref path) = normalized {
        if !Path::new(path).is_dir() {
            return Err(format!("Grok CLI 工作目录不存在: {}", path));
        }
    }
    account.working_dir = normalized;
    save_account_locked(&account)?;
    Ok(GrokAccountView::from(&account))
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(value)) => value.as_f64(),
        Some(Value::String(value)) => value.parse().ok(),
        Some(Value::Object(value)) => value.get("val").and_then(|item| number(Some(item))),
        _ => None,
    }
}

fn raw_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(|value| normalize_text(Some(value)))
}

fn active_subscription(value: &Value) -> Option<&Value> {
    value
        .get("subscriptions")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| {
                    matches!(
                        item.get("status").and_then(Value::as_str),
                        Some("SUBSCRIPTION_STATUS_ACTIVE" | "SubscriptionStatusActive")
                    )
                })
                .or_else(|| items.first())
        })
}

fn user_payload(value: &Value) -> &Value {
    value
        .get("user")
        .filter(|item| item.is_object())
        .unwrap_or(value)
}

fn user_subscription(value: &Value) -> Option<&Value> {
    let user = user_payload(value);
    user.get("subscription")
        .filter(|item| item.is_object())
        .or_else(|| active_subscription(user))
}

fn nested_number(value: &Value, key: &str) -> Option<f64> {
    match value {
        Value::Object(object) => object
            .get(key)
            .and_then(|item| number(Some(item)))
            .or_else(|| object.values().find_map(|item| nested_number(item, key))),
        Value::Array(items) => items.iter().find_map(|item| nested_number(item, key)),
        _ => None,
    }
}

fn credit_bag_amounts(value: &Value) -> Option<(Option<f64>, Option<f64>, Option<f64>)> {
    if let Some(items) = value.as_array() {
        return items.iter().find_map(credit_bag_amounts);
    }
    let object = value.as_object()?;
    let total = number(
        object
            .get("total")
            .or_else(|| object.get("limit"))
            .or_else(|| object.get("cap"))
            .or_else(|| object.get("allocation"))
            .or_else(|| object.get("amount")),
    );
    let used = number(
        object
            .get("used")
            .or_else(|| object.get("spent"))
            .or_else(|| object.get("consumed"))
            .or_else(|| object.get("usage")),
    );
    let remaining = number(
        object
            .get("remaining")
            .or_else(|| object.get("balance"))
            .or_else(|| object.get("left")),
    );
    if total.is_none() && used.is_none() && remaining.is_none() {
        return object
            .get("bags")
            .or_else(|| object.get("items"))
            .and_then(credit_bag_amounts);
    }
    let resolved_used = used.or_else(|| match (total, remaining) {
        (Some(total), Some(remaining)) => Some((total - remaining).max(0.0)),
        _ => None,
    });
    let resolved_remaining = remaining.or_else(|| match (total, resolved_used) {
        (Some(total), Some(used)) => Some((total - used).max(0.0)),
        _ => None,
    });
    Some((resolved_used, total, resolved_remaining))
}

fn credit_bag_usage_percent(value: &Value) -> Option<f64> {
    let (used, total, _) = credit_bag_amounts(value)?;
    if let Some(total) = total.filter(|value| value.is_finite() && *value > 0.0) {
        let used = used.filter(|value| value.is_finite())?;
        return Some(((used.max(0.0) / total) * 100.0).clamp(0.0, 100.0));
    }
    None
}

fn credit_usage_sources<'a>(billing: &'a Value, config: &'a Value) -> [Option<&'a Value>; 8] {
    [
        billing.get("credits"),
        billing.get("creditBalance"),
        billing.get("usage"),
        config.get("credits"),
        config.get("includedCredits"),
        config.get("subscriptionCredits"),
        config.get("weeklyCredits"),
        config.get("sharedPool"),
    ]
}

fn credit_usage_percent(billing: &Value, config: &Value) -> Option<f64> {
    credit_usage_sources(billing, config)
        .into_iter()
        .flatten()
        .find_map(credit_bag_usage_percent)
}

fn credit_usage_amounts(billing: &Value, config: &Value) -> (Option<f64>, Option<f64>) {
    credit_usage_sources(billing, config)
        .into_iter()
        .flatten()
        .find_map(|value| {
            let (used, total, _) = credit_bag_amounts(value)?;
            if used.is_some() || total.is_some() {
                Some((used, total))
            } else {
                None
            }
        })
        .unwrap_or((None, None))
}

fn product_usage_from_value(item: &Value) -> Option<GrokProductUsage> {
    let product = raw_string(item.get("product"))
        .or_else(|| raw_string(item.get("name")))
        .or_else(|| raw_string(item.get("productName")))?;
    let (used, total, remaining) = credit_bag_amounts(item).unwrap_or((None, None, None));
    let usage_percent = number(item.get("usagePercent"))
        .or_else(|| number(item.get("usedPercent")))
        .or_else(|| match (used, total) {
            (Some(used), Some(total)) if total > 0.0 => {
                Some(((used.max(0.0) / total) * 100.0).clamp(0.0, 100.0))
            }
            _ => None,
        });
    Some(GrokProductUsage {
        product,
        usage_percent,
        used,
        total,
        remaining,
    })
}

fn quota_from_payload(
    billing: &Value,
    subscriptions: Option<&Value>,
    cli_user: Option<&Value>,
    task_usage: Option<&Value>,
) -> GrokQuota {
    let config = billing.get("config").unwrap_or(billing);
    let period = config.get("currentPeriod").unwrap_or(&Value::Null);
    let subscription = cli_user
        .and_then(user_subscription)
        .or_else(|| subscriptions.and_then(active_subscription))
        .or_else(|| active_subscription(config));
    let subscription_tier = raw_string(config.get("subscription_tier"))
        .or_else(|| raw_string(config.get("subscriptionTier")))
        .or_else(|| subscription.and_then(|item| raw_string(item.get("tier"))))
        .or_else(|| {
            cli_user.and_then(|value| {
                let user = user_payload(value);
                raw_string(user.get("subscriptionTier"))
                    .or_else(|| raw_string(user.get("subscription_tier")))
            })
        });
    let products = config
        .get("productUsage")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(product_usage_from_value).collect())
        .unwrap_or_default();
    let (weekly_used, weekly_total) = credit_usage_amounts(billing, config);
    GrokQuota {
        period_type: raw_string(period.get("type")),
        period_start: raw_string(period.get("start"))
            .or_else(|| raw_string(config.get("billingPeriodStart"))),
        period_end: raw_string(period.get("end"))
            .or_else(|| raw_string(config.get("billingPeriodEnd"))),
        weekly_limit_percent: number(config.get("creditUsagePercent"))
            .or_else(|| credit_usage_percent(billing, config)),
        weekly_used,
        weekly_total,
        on_demand_used: number(config.get("onDemandUsed")),
        on_demand_cap: number(config.get("onDemandCap")),
        prepaid_balance: number(config.get("prepaidBalance")),
        frequent_usage: task_usage.and_then(|value| nested_number(value, "frequentUsage")),
        frequent_limit: task_usage.and_then(|value| nested_number(value, "frequentLimit")),
        occasional_usage: task_usage.and_then(|value| nested_number(value, "occasionalUsage")),
        occasional_limit: task_usage.and_then(|value| nested_number(value, "occasionalLimit")),
        subscription_tier,
        subscription_status: subscription.and_then(|item| raw_string(item.get("status"))),
        products,
    }
}

fn cli_proxy_get(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    client_version: &str,
) -> reqwest::RequestBuilder {
    // 与官方 Grok CLI / task_usage 一致：cli-chat-proxy 的 billing/user 需要 x-xai-token-auth
    client
        .get(url)
        .header(AUTHORIZATION, format!("Bearer {}", access_token))
        .header(ACCEPT, "application/json")
        .header("x-xai-token-auth", "xai-grok-cli")
        .header("x-grok-cli-version", client_version)
        .header("x-grok-client-version", client_version)
        .header("x-grok-client-surface", "grok-cli")
        .header("x-grok-client-identifier", "cockpit-tools")
        .header(USER_AGENT, format!("grok-cli/{}", client_version))
}

/// 传输层瞬时错误（SSL EOF / 断连 / 超时）重试；RequestBuilder 不可复用，需每次重建。
async fn send_with_transport_retry<F>(
    label: &str,
    max_attempts: usize,
    mut build: F,
) -> Result<reqwest::Response, String>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    let mut attempt = 0_usize;
    loop {
        attempt += 1;
        match build().send().await {
            Ok(response) => return Ok(response),
            Err(error) if attempt < max_attempts => {
                logger::log_warn(&format!(
                    "[Grok Account] {} 传输失败，第 {}/{} 次: {}",
                    label, attempt, max_attempts, error
                ));
                // 线性退避：0.5s / 1s / 1.5s…
                tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
            }
            Err(error) => {
                return Err(format!("{}失败: {}", label, error));
            }
        }
    }
}

async fn cli_user_for(
    client: &reqwest::Client,
    account: &GrokAccount,
    client_version: &str,
) -> Option<Value> {
    let access_token = account.access_token.clone();
    let client_version = client_version.to_string();
    let response =
        send_with_transport_retry("查询 Grok 用户信息", TRANSPORT_MAX_ATTEMPTS, || {
            cli_proxy_get(client, CLI_USER_URL, &access_token, &client_version)
        })
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().await.ok()
}

async fn subscriptions_for(account: &GrokAccount, client_version: &str) -> Option<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let access_token = account.access_token.clone();
    let client_version = client_version.to_string();
    let user_id = account.user_id.clone();
    let response = send_with_transport_retry("查询 Grok 订阅", TRANSPORT_MAX_ATTEMPTS, || {
        let mut request = client
            .get(SUBSCRIPTIONS_URL)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .header(ACCEPT, "application/json,text/plain,*/*")
            .header("x-xai-token-auth", "xai-grok-cli")
            .header("x-grok-client-version", &client_version)
            .header(USER_AGENT, format!("grok-cli/{}", client_version));
        if let Some(user_id) = user_id.as_deref() {
            request = request.header("x-userid", user_id);
        }
        request
    })
    .await
    .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().await.ok()
}

async fn task_usage_for(account: &GrokAccount) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("创建 Grok 任务配额客户端失败: {}", error))?;
    let access_token = account.access_token.clone();
    let response =
        send_with_transport_retry("查询 Grok 任务配额", TRANSPORT_MAX_ATTEMPTS, || {
            client
                .get(TASK_USAGE_URL)
                .header(AUTHORIZATION, format!("Bearer {}", access_token))
                .header(ACCEPT, "application/json")
                .header("x-xai-token-auth", "xai-grok-cli")
                .header(USER_AGENT, "Grok Build")
        })
        .await?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Grok 任务配额失败: {}", error))?;
    if !status.is_success() {
        return Err(format!("查询 Grok 任务配额返回 {}", status.as_u16()));
    }
    serde_json::from_str(&body).map_err(|error| format!("解析 Grok 任务配额失败: {}", error))
}

fn live_auth_path_for_account(account: &GrokAccount) -> Result<PathBuf, String> {
    if config::get_user_config().grok_sync_official_auth_on_switch
        && provider_current_state::get_current_account_id("grok")?.as_deref()
            == Some(account.id.as_str())
        && !account.is_api_key_auth()
    {
        return Ok(default_grok_home()?.join(AUTH_FILE));
    }
    Ok(managed_profile_dir(&account.id)?.join(AUTH_FILE))
}

/// access 仍可用的缓冲（秒）：未到 expires_at 且距过期大于该值时优先直接使用，不抢刷。
const ACCESS_USABLE_LEAD_SECS: i64 = 60;
/// 与 CLIProxyAPI RefreshLead 对齐：到期前 5 分钟主动 refresh。
const ACCESS_REFRESH_LEAD_SECS: i64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrokCredSource {
    Store,
    ManagedHome,
    OfficialHome,
}

impl GrokCredSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::ManagedHome => "managed_home",
            Self::OfficialHome => "official_home",
        }
    }

    /// 时间并列时的稳定次序（数值越大越优先）。
    fn tie_break_rank(self) -> u8 {
        match self {
            Self::OfficialHome => 3,
            Self::ManagedHome => 2,
            Self::Store => 1,
        }
    }
}

#[derive(Debug, Clone)]
struct LiveCredentialCandidate {
    source: GrokCredSource,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<i64>,
    expires_at_raw: Option<Value>,
    /// 文件 mtime 或账号文件 mtime（秒）。
    observed_at: i64,
    /// 匹配到的完整条目，用于合并 auth_raw。
    entry: Option<Map<String, Value>>,
    path: Option<PathBuf>,
}

fn path_mtime_secs(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| {
            time.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs() as i64)
        })
        .unwrap_or(0)
}

/// 在 registry 中找**同账号**条目：principal/user/email/token 与现有逻辑一致，绝不串号。
fn find_matching_auth_entry_in_registry<'a>(
    registry: &'a Value,
    account: &GrokAccount,
) -> Option<&'a Map<String, Value>> {
    let object = registry.as_object()?;
    let mut matches: Vec<&Map<String, Value>> = object
        .iter()
        .filter_map(|(key, value)| {
            // 只认 xAI OIDC registry 键，避免误读其它字段
            split_xai_auth_registry_key(key)?;
            let entry = value.as_object()?;
            auth_entry_matches_account(value, account).then_some(entry)
        })
        .collect();
    if matches.is_empty() {
        return None;
    }
    if matches.len() == 1 {
        return Some(matches[0]);
    }
    // 多条同账号（极少见）：取 expires_at 最晚的
    matches.sort_by(|left, right| {
        let left_exp = left
            .get("expires_at")
            .and_then(parse_timestamp)
            .unwrap_or(0);
        let right_exp = right
            .get("expires_at")
            .and_then(parse_timestamp)
            .unwrap_or(0);
        right_exp.cmp(&left_exp)
    });
    Some(matches[0])
}

fn live_candidate_from_entry(
    source: GrokCredSource,
    path: &Path,
    entry: &Map<String, Value>,
) -> Option<LiveCredentialCandidate> {
    let access_token = string_field(entry, "key")?;
    let refresh_token = string_field(entry, "refresh_token");
    let expires_at = entry.get("expires_at").and_then(parse_timestamp);
    let expires_at_raw = entry
        .get("expires_at")
        .filter(|value| !value.is_null())
        .cloned();
    Some(LiveCredentialCandidate {
        source,
        access_token,
        refresh_token,
        expires_at,
        expires_at_raw,
        observed_at: path_mtime_secs(path),
        entry: Some(entry.clone()),
        path: Some(path.to_path_buf()),
    })
}

fn live_candidate_from_auth_path(
    path: &Path,
    account: &GrokAccount,
    source: GrokCredSource,
) -> Option<LiveCredentialCandidate> {
    let registry = read_auth_registry(path).ok().flatten()?;
    let entry = find_matching_auth_entry_in_registry(&registry, account)?;
    live_candidate_from_entry(source, path, entry)
}

fn live_candidate_from_store(account: &GrokAccount) -> Option<LiveCredentialCandidate> {
    if account.access_token.trim().is_empty() && account.refresh_token.is_none() {
        return None;
    }
    let path = account_path(&account.id).ok();
    let observed_at = path
        .as_ref()
        .map(|p| path_mtime_secs(p))
        .filter(|t| *t > 0)
        .or_else(|| account.usage_updated_at.map(|ms| ms / 1000))
        .unwrap_or_else(|| account.last_used / 1000);
    Some(LiveCredentialCandidate {
        source: GrokCredSource::Store,
        access_token: account.access_token.clone(),
        refresh_token: account.refresh_token.clone(),
        expires_at: account.expires_at,
        expires_at_raw: account.expires_at_raw.clone(),
        observed_at,
        entry: account
            .auth_raw
            .as_ref()
            .and_then(Value::as_object)
            .cloned(),
        path,
    })
}

fn collect_live_credential_candidates(account: &GrokAccount) -> Vec<LiveCredentialCandidate> {
    let mut candidates = Vec::new();
    if let Some(store) = live_candidate_from_store(account) {
        candidates.push(store);
    }
    if let Ok(managed) = managed_profile_dir(&account.id) {
        let path = managed.join(AUTH_FILE);
        if let Some(candidate) =
            live_candidate_from_auth_path(&path, account, GrokCredSource::ManagedHome)
        {
            candidates.push(candidate);
        }
    }
    if let Ok(official) = default_grok_home() {
        let path = official.join(AUTH_FILE);
        if let Some(candidate) =
            live_candidate_from_auth_path(&path, account, GrokCredSource::OfficialHome)
        {
            candidates.push(candidate);
        }
    }
    candidates
}

fn access_still_usable(expires_at: Option<i64>, now: i64) -> bool {
    match expires_at {
        Some(exp) => exp > now + ACCESS_USABLE_LEAD_SECS,
        // 无过期信息时不视为「确定可用」，避免盲用旧 token
        None => false,
    }
}

fn pick_best_live_credential(
    candidates: &[LiveCredentialCandidate],
    now: i64,
) -> Option<&LiveCredentialCandidate> {
    if candidates.is_empty() {
        return None;
    }
    // 优先「access 仍可用」的集合；若没有，再退回全部（靠 RT 兜底 refresh）
    let usable: Vec<&LiveCredentialCandidate> = candidates
        .iter()
        .filter(|c| !c.access_token.trim().is_empty() && access_still_usable(c.expires_at, now))
        .collect();
    let pool: Vec<&LiveCredentialCandidate> = if usable.is_empty() {
        candidates.iter().collect()
    } else {
        usable
    };
    pool.into_iter().max_by(|left, right| {
        left.observed_at
            .cmp(&right.observed_at)
            .then_with(|| {
                left.expires_at
                    .unwrap_or(0)
                    .cmp(&right.expires_at.unwrap_or(0))
            })
            .then_with(|| {
                left.source
                    .tie_break_rank()
                    .cmp(&right.source.tie_break_rank())
            })
    })
}

fn apply_live_credential_candidate(
    account: &mut GrokAccount,
    candidate: &LiveCredentialCandidate,
) -> bool {
    let mut changed = false;
    if !candidate.access_token.is_empty() && candidate.access_token != account.access_token {
        account.access_token = candidate.access_token.clone();
        changed = true;
    }
    if let Some(refresh) = candidate.refresh_token.as_ref() {
        if account.refresh_token.as_deref() != Some(refresh.as_str()) {
            account.refresh_token = Some(refresh.clone());
            changed = true;
        }
    }
    if candidate.expires_at.is_some() && candidate.expires_at != account.expires_at {
        account.expires_at = candidate.expires_at;
        account.expires_at_raw = candidate.expires_at_raw.clone();
        changed = true;
    }
    if let Some(entry) = candidate.entry.as_ref() {
        let mut merged = account
            .auth_raw
            .as_ref()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, value) in entry {
            merged.insert(key.clone(), value.clone());
        }
        account.auth_raw = Some(Value::Object(merged));
    }
    changed
}

/// 从账号库 / 受管 home / 官方 ~/.grok 中，仅匹配**同一账号**后选取最新可用凭据并 adopt。
/// 返回是否改动了内存中的 account。
fn adopt_best_live_credentials(account: &mut GrokAccount) -> Result<bool, String> {
    if account.is_api_key_auth() {
        return Ok(false);
    }
    let candidates = collect_live_credential_candidates(account);
    let now = now_ts();
    let Some(best) = pick_best_live_credential(&candidates, now).cloned() else {
        return Ok(false);
    };
    let changed = apply_live_credential_candidate(account, &best);
    if changed {
        logger::log_info(&format!(
            "[Grok Account] 已采用最新同账号凭据: account_id={}, email={}, source={}, path={}, observed_at={}, expires_at={:?}",
            account.id,
            account.email,
            best.source.as_str(),
            best.path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "-".to_string()),
            best.observed_at,
            best.expires_at
        ));
    }
    Ok(changed)
}

/// 兼容旧调用名：行为升级为多源同账号 best-adopt。
fn adopt_live_tokens_from_account_home(account: &mut GrokAccount) -> Result<bool, String> {
    adopt_best_live_credentials(account)
}

async fn refresh_credentials(account: &mut GrokAccount, force: bool) -> Result<(), String> {
    // 先在多源中找同账号「最新可用」凭据（官方 CLI / 受管 home / 库可能已轮换 RT）
    let _ = adopt_best_live_credentials(account);
    let now = now_ts();
    // 已有仍可用的 access：查额度/注入优先直接用，避免与官方抢刷 RT
    if !force && access_still_usable(account.expires_at, now) {
        return Ok(());
    }
    let should_refresh = force
        || account
            .expires_at
            .map(|expires_at| expires_at <= now + ACCESS_REFRESH_LEAD_SECS)
            .unwrap_or(true);
    if !should_refresh {
        return Ok(());
    }
    let refresh_token = account
        .refresh_token
        .clone()
        .ok_or_else(|| "Grok refresh_token 为空，请重新授权".to_string())?;
    match grok_oauth::refresh_token(
        &refresh_token,
        account.token_endpoint.as_deref(),
        account.oidc_client_id.as_deref(),
    )
    .await
    {
        Ok(token) => {
            apply_refreshed_token(account, token);
            Ok(())
        }
        Err(error) => {
            // invalid_grant：其它源可能已轮换 RT，再收割一次同账号最新凭据后重试
            let normalized = error.to_ascii_lowercase();
            if normalized.contains("invalid_grant") || normalized.contains("401") {
                let previous_rt = refresh_token.clone();
                if adopt_best_live_credentials(account).unwrap_or(false) {
                    if let Some(rotated) = account.refresh_token.clone() {
                        if rotated != previous_rt {
                            // 若新 adopt 的 access 已可用，不必再 refresh
                            if !force && access_still_usable(account.expires_at, now_ts()) {
                                return Ok(());
                            }
                            let token = grok_oauth::refresh_token(
                                &rotated,
                                account.token_endpoint.as_deref(),
                                account.oidc_client_id.as_deref(),
                            )
                            .await?;
                            apply_refreshed_token(account, token);
                            return Ok(());
                        }
                    }
                    // RT 未变但 access 已更新且可用
                    if !force && access_still_usable(account.expires_at, now_ts()) {
                        return Ok(());
                    }
                }
            }
            Err(error)
        }
    }
}

fn apply_refreshed_token(account: &mut GrokAccount, token: grok_oauth::GrokTokenResponse) {
    account.access_token = token.access_token;
    if let Some(rotated) = normalize_text(token.refresh_token.as_deref()) {
        account.refresh_token = Some(rotated);
    }
    if let Some(id_token) = normalize_text(token.id_token.as_deref()) {
        account.id_token = Some(id_token);
    }
    if let Some(token_type) = normalize_text(token.token_type.as_deref()) {
        account.token_type = Some(token_type);
    }
    if let Some(expires_at) = token
        .expires_in
        .filter(|seconds| *seconds > 0)
        .map(|seconds| now_ts() + seconds)
    {
        account.expires_at = Some(expires_at);
        account.expires_at_raw = DateTime::from_timestamp(expires_at, 0)
            .map(|timestamp| Value::String(timestamp.to_rfc3339()));
    }
    account.auth_raw = Some(official_auth_object(account));
}

async fn query_quota(account: &mut GrokAccount) -> Result<(), String> {
    let client_version = crate::commands::grok::detect_grok_client_version()
        .await
        .unwrap_or_else(|| FALLBACK_GROK_CLIENT_VERSION.to_string());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("创建 Grok 配额客户端失败: {}", error))?;
    // billing 是主数据源：补上与 task_usage 相同的传输重试，降低 SSL EOF 等偶发失败
    let access_token = account.access_token.clone();
    let client_version_for_billing = client_version.clone();
    let response = send_with_transport_retry("查询 Grok 配额", TRANSPORT_MAX_ATTEMPTS, || {
        cli_proxy_get(
            &client,
            BILLING_URL,
            &access_token,
            &client_version_for_billing,
        )
    })
    .await?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Grok 配额响应失败: {}", error))?;
    if !status.is_success() {
        return Err(format!("查询 Grok 配额返回 {}", status.as_u16()));
    }
    let billing: Value =
        serde_json::from_str(&body).map_err(|error| format!("解析 Grok 配额失败: {}", error))?;
    let cli_user = cli_user_for(&client, account, &client_version).await;
    let subscriptions = if billing.get("subscriptions").is_some()
        || billing.pointer("/config/subscriptions").is_some()
        || cli_user.as_ref().and_then(user_subscription).is_some()
    {
        None
    } else {
        subscriptions_for(account, &client_version).await
    };
    let task_usage_result = task_usage_for(account).await;
    let quota = quota_from_payload(
        &billing,
        subscriptions.as_ref(),
        cli_user.as_ref(),
        task_usage_result.as_ref().ok(),
    );
    let has_usage_quota = quota.weekly_limit_percent.is_some()
        || quota
            .products
            .iter()
            .any(|item| item.usage_percent.is_some())
        || quota.on_demand_cap.is_some_and(|value| value > 0.0)
        || quota.prepaid_balance.is_some_and(|value| value > 0.0)
        || quota.frequent_limit.is_some_and(|value| value > 0.0)
        || quota.occasional_limit.is_some_and(|value| value > 0.0);
    account.plan_type = quota.subscription_tier.clone();
    account.quota = Some(quota);
    account.billing_raw = Some(billing);
    account.subscription_raw = subscriptions;
    account.task_usage_raw = task_usage_result.as_ref().ok().cloned();
    if let Some(user) = cli_user {
        if let Some(has_access) = user_payload(&user)
            .get("hasGrokCodeAccess")
            .or_else(|| user_payload(&user).get("has_grok_code_access"))
            .and_then(Value::as_bool)
        {
            account.has_grok_code_access = Some(has_access);
        }
        account.user_raw = Some(user);
    }
    account.usage_updated_at = Some(now_ms());
    account.quota_query_last_error = None;
    account.quota_query_last_error_at = None;
    account.status = None;
    account.status_reason = None;
    if !has_usage_quota {
        if let Err(error) = task_usage_result {
            return Err(error);
        }
    }
    Ok(())
}

fn save_refreshed_account(
    account: &GrokAccount,
    expected_default_access_token: &str,
) -> Result<(), String> {
    {
        let _guard = ACCOUNT_LOCK.lock().map_err(|_| "获取 Grok 账号锁失败")?;
        let _store_guard = acquire_store_lock()?;
        save_account_locked(account)?;
    }

    if config::get_user_config().grok_sync_official_auth_on_switch
        && !account.is_api_key_auth()
        && provider_current_state::get_current_account_id("grok")?.as_deref()
            == Some(account.id.as_str())
    {
        let auth_path = default_grok_home()?.join(AUTH_FILE);
        if !write_account_to_auth_path_if_token_matches(
            account,
            &auth_path,
            expected_default_access_token,
        )? {
            logger::log_info(&format!(
                "[Grok Account] 官方登录已被外部切换，跳过刷新回写: account_id={}",
                account.id
            ));
        }
    }
    Ok(())
}

fn token_lock_for(account_id: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let account_id = normalize_id(account_id)?;
    let mut locks = TOKEN_LOCKS
        .lock()
        .map_err(|_| "获取 Grok token 锁失败".to_string())?;
    Ok(locks
        .entry(account_id)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone())
}

fn acquire_token_refresh_file_lock(account_id: &str) -> Result<SecretFileLock, String> {
    let profile = managed_profile_dir(account_id)?;
    ensure_secret_dir(&profile)?;
    acquire_secret_lock(&profile.join("token-refresh"))
}

fn refresh_error_status(error: &str) -> &'static str {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("invalid_grant")
        || normalized.contains("refresh_token 为空")
        || normalized.contains("access_denied")
    {
        "reauth_required"
    } else {
        "error"
    }
}

const MAX_QUOTA_AUTH_RETRIES: usize = 1;

fn should_retry_quota_after_unauthorized(
    force_credentials: bool,
    retry_count: usize,
    result: &Result<(), String>,
) -> bool {
    !force_credentials
        && retry_count < MAX_QUOTA_AUTH_RETRIES
        && result
            .as_ref()
            .err()
            .map(|error| error.contains(" 401"))
            .unwrap_or(false)
}

async fn refresh_account_inner(
    account_id: &str,
    force_credentials: bool,
) -> Result<GrokAccountView, String> {
    let token_lock = token_lock_for(account_id)?;
    let _token_guard = token_lock.lock().await;
    let _file_guard = acquire_token_refresh_file_lock(account_id)?;
    let mut account =
        load_account(account_id).ok_or_else(|| format!("Grok 账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        if account.resolved_api_key().is_none() {
            account.status = Some("error".to_string());
            account.status_reason = Some("API Key 为空".to_string());
            save_account_locked(&account)?;
            return Err("Grok API Key 账号缺少 api_key".to_string());
        }
        account.status = Some("normal".to_string());
        account.status_reason = None;
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
        account.usage_updated_at = Some(now_ms());
        account.last_used = now_ms();
        save_account_locked(&account)?;
        return Ok(GrokAccountView::from(&account));
    }
    let previous_access_token = account.access_token.clone();
    if let Err(error) = refresh_credentials(&mut account, force_credentials).await {
        account.status = Some(refresh_error_status(&error).to_string());
        account.status_reason = Some(error.clone());
        save_refreshed_account(&account, &previous_access_token)?;
        return Err(error);
    }

    let mut quota_result = query_quota(&mut account).await;
    let mut quota_auth_retries = 0;
    while should_retry_quota_after_unauthorized(
        force_credentials,
        quota_auth_retries,
        &quota_result,
    ) {
        quota_auth_retries += 1;
        match refresh_credentials(&mut account, true).await {
            Ok(()) => quota_result = query_quota(&mut account).await,
            Err(error) => {
                account.status = Some(refresh_error_status(&error).to_string());
                account.status_reason = Some(error.clone());
                save_refreshed_account(&account, &previous_access_token)?;
                return Err(error);
            }
        }
    }
    if let Err(error) = quota_result {
        // 软失败：保留上次成功的 quota/plan 缓存，仅记录错误，避免后台刷新把界面刷成空
        account.quota_query_last_error = Some(error.clone());
        account.quota_query_last_error_at = Some(now_ms());
        logger::log_warn(&format!(
            "[Grok Account] 配额查询失败（保留缓存展示）: account_id={}, error={}",
            account.id, error
        ));
    }
    save_refreshed_account(&account, &previous_access_token)?;
    Ok(GrokAccountView::from(&account))
}

pub async fn prepare_account_for_injection(account_id: &str) -> Result<GrokAccount, String> {
    let token_lock = token_lock_for(account_id)?;
    let _token_guard = token_lock.lock().await;
    let _file_guard = acquire_token_refresh_file_lock(account_id)?;
    let mut account =
        load_account(account_id).ok_or_else(|| format!("Grok 账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        if account.resolved_api_key().is_none() {
            return Err("Grok API Key 账号缺少 api_key".to_string());
        }
        account.last_used = now_ms();
        save_account_locked(&account)?;
        return Ok(account);
    }
    let previous_access_token = account.access_token.clone();
    if let Err(error) = refresh_credentials(&mut account, false).await {
        account.status = Some(refresh_error_status(&error).to_string());
        account.status_reason = Some(error.clone());
        save_refreshed_account(&account, &previous_access_token)?;
        return Err(error);
    }
    save_refreshed_account(&account, &previous_access_token)?;
    Ok(account)
}

pub async fn refresh_account(account_id: &str) -> Result<GrokAccountView, String> {
    refresh_account_inner(account_id, false).await
}

pub async fn force_refresh_account(account_id: &str) -> Result<GrokAccountView, String> {
    refresh_account_inner(account_id, true).await
}

pub async fn refresh_all_accounts() -> Result<Vec<(String, Result<GrokAccountView, String>)>, String>
{
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    // 多账号时全串行易拖垮 IPC/超时；过高并发会触发上游限流与 token 旋转压力。
    // 每账号有独立 token 锁，跨账号限流并发是安全的。
    const MAX_CONCURRENT: usize = 3;
    let ids: Vec<String> = load_index()?
        .accounts
        .into_iter()
        .map(|item| item.id)
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let tasks: Vec<_> = ids
        .into_iter()
        .map(|account_id| {
            let semaphore = semaphore.clone();
            async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|error| format!("获取 Grok 刷新并发许可失败: {}", error))?;
                let result = refresh_account(&account_id).await;
                Ok::<(String, Result<GrokAccountView, String>), String>((account_id, result))
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

pub fn current_account_id() -> Result<Option<String>, String> {
    if !config::get_user_config().grok_sync_official_auth_on_switch {
        return Ok(None);
    }
    if let Some(tracked) = provider_current_state::get_current_account_id("grok")? {
        if load_account(&tracked).is_some_and(|account| account.is_api_key_auth()) {
            return Ok(Some(tracked));
        }
    }
    reconcile_current_account_id()
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(index_path()?.to_string_lossy().to_string())
}

fn remaining_percent_from_used_total(used: f64, total: f64) -> Option<i32> {
    if !used.is_finite() || !total.is_finite() || total <= 0.0 {
        return None;
    }
    let remaining = ((total - used).max(0.0) / total * 100.0).clamp(0.0, 100.0);
    Some(remaining.round() as i32)
}

fn remaining_percent_from_used_pct(used_percent: f64) -> i32 {
    (100.0 - used_percent.clamp(0.0, 100.0)).round() as i32
}

/// 与账号页/概览可见桶对齐：周额度 + productUsage + 任务/按量。
fn quota_remaining_metrics(account: &GrokAccountView) -> Vec<(String, i32)> {
    let Some(quota) = account.quota.as_ref() else {
        return Vec::new();
    };
    let mut metrics = Vec::new();
    // 周总池（creditUsagePercent / weeklyCredits）剩余
    if let Some(used_pct) = quota.weekly_limit_percent {
        metrics.push((
            "weekly".to_string(),
            remaining_percent_from_used_pct(used_pct),
        ));
    } else if let (Some(used), Some(total)) = (quota.weekly_used, quota.weekly_total) {
        if let Some(remaining) = remaining_percent_from_used_total(used, total) {
            metrics.push(("weekly".to_string(), remaining));
        }
    }
    for product in &quota.products {
        let remaining = match (product.used, product.total) {
            (Some(used), Some(total)) => remaining_percent_from_used_total(used, total),
            _ => product
                .usage_percent
                .map(remaining_percent_from_used_pct)
                .or_else(|| match (product.used, product.remaining, product.total) {
                    (None, Some(remaining), Some(total)) if total > 0.0 => {
                        remaining_percent_from_used_total((total - remaining).max(0.0), total)
                    }
                    _ => None,
                }),
        };
        if let Some(remaining) = remaining {
            metrics.push((product.product.clone(), remaining));
        }
    }
    if let (Some(used), Some(limit)) = (quota.frequent_usage, quota.frequent_limit) {
        if let Some(remaining) = remaining_percent_from_used_total(used, limit) {
            metrics.push(("frequent".to_string(), remaining));
        }
    }
    if let (Some(used), Some(limit)) = (quota.occasional_usage, quota.occasional_limit) {
        if let Some(remaining) = remaining_percent_from_used_total(used, limit) {
            metrics.push(("occasional".to_string(), remaining));
        }
    }
    if let (Some(used), Some(cap)) = (quota.on_demand_used, quota.on_demand_cap) {
        if let Some(remaining) = remaining_percent_from_used_total(used, cap) {
            metrics.push(("on-demand".to_string(), remaining));
        }
    }
    metrics
}

fn clear_quota_alert_cooldown(account_id: &str, threshold: i32) {
    if let Ok(mut state) = QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&format!("{}:{}", account_id, threshold));
    }
}

pub fn run_quota_alert_if_needed() -> Result<(), String> {
    let config = crate::modules::config::get_user_config();
    if !config.grok_quota_alert_enabled {
        return Ok(());
    }
    let threshold = config.grok_quota_alert_threshold.clamp(0, 100);
    let accounts = list_accounts_checked()?;
    let now = now_ts();

    // 无全局「当前账号」：对每个已有配额数据的账号独立预警。
    for current in &accounts {
        let metrics = quota_remaining_metrics(current);
        if metrics.is_empty() {
            clear_quota_alert_cooldown(&current.id, threshold);
            continue;
        }
        let lowest = metrics
            .iter()
            .map(|(_, remaining)| *remaining)
            .min()
            .unwrap_or(100);
        let low_products: Vec<String> = metrics
            .iter()
            .filter(|(_, remaining)| *remaining <= threshold)
            .map(|(name, _)| name.clone())
            .collect();
        if low_products.is_empty() {
            clear_quota_alert_cooldown(&current.id, threshold);
            continue;
        }

        let cooldown_key = format!("{}:{}", current.id, threshold);
        if let Ok(mut state) = QUOTA_ALERT_LAST_SENT.lock() {
            if state
                .get(&cooldown_key)
                .map(|sent_at| now - *sent_at < QUOTA_ALERT_COOLDOWN_SECONDS)
                .unwrap_or(false)
            {
                continue;
            }
            state.insert(cooldown_key, now);
        }

        let recommendation = accounts
            .iter()
            .filter(|account| account.id != current.id)
            .filter(|account| {
                account.quota_query_last_error.is_none()
                    && account
                        .status
                        .as_deref()
                        .map(|status| matches!(status, "normal" | "ok"))
                        .unwrap_or(true)
            })
            .filter_map(|account| {
                let minimum = quota_remaining_metrics(account)
                    .into_iter()
                    .map(|(_, remaining)| remaining)
                    .min()?;
                if minimum <= 0 {
                    return None;
                }
                Some((account, minimum))
            })
            .max_by_key(|(_, minimum)| *minimum)
            .map(|(account, _)| account);
        crate::modules::account::dispatch_quota_alert(
            &crate::modules::account::QuotaAlertPayload {
                platform: "grok".to_string(),
                current_account_id: current.id.clone(),
                current_email: current.email.clone(),
                threshold,
                threshold_display: None,
                lowest_percentage: lowest,
                low_models: low_products,
                recommended_account_id: recommendation.map(|account| account.id.clone()),
                recommended_email: recommendation.map(|account| account.email.clone()),
                triggered_at: now,
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        access_still_usable, account_from_auth_object, accounts_match_for_upsert,
        acquire_secret_lock, apply_refreshed_token, auth_entry_matches_account,
        auth_registry_entry, auth_registry_for, default_grok_home, ensure_secret_dir,
        find_matching_auth_entry_in_registry, list_accounts_checked, load_account,
        load_account_from_path, load_index_from_paths, parse_auth_registry,
        pick_best_live_credential, quota_from_payload, quota_remaining_metrics, remove_account,
        remove_matching_auth_scope, resolve_account_id_from_registry, save_account_locked,
        should_retry_quota_after_unauthorized, string_field, validate_api_provider_config,
        write_account_to_auth_path_if_token_matches, write_account_to_official_auth_path,
        write_account_to_profile, GrokCredSource, LiveCredentialCandidate,
    };
    use crate::models::grok::{
        GrokAccount, GrokAccountView, GrokAuthMode, GrokProductUsage, GrokQuota,
    };
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn sample_account() -> GrokAccount {
        GrokAccount {
            id: "account-1".to_string(),
            email: "person@example.com".to_string(),
            auth_mode: GrokAuthMode::Oauth,
            tags: None,
            first_name: None,
            last_name: None,
            user_id: Some("user-1".to_string()),
            principal_id: Some("principal-1".to_string()),
            principal_type: Some("user".to_string()),
            team_id: None,
            profile_image_asset_id: None,
            coding_data_retention_opt_out: Some(false),
            access_token: "secret-access".to_string(),
            api_key: None,
            api_base_url: None,
            api_model: None,
            refresh_token: Some("secret-refresh".to_string()),
            id_token: None,
            token_type: Some("Bearer".to_string()),
            expires_at: Some(1_900_000_000),
            expires_at_raw: None,
            oidc_issuer: None,
            oidc_client_id: None,
            token_endpoint: None,
            plan_type: None,
            quota: None,
            auth_raw: None,
            billing_raw: None,
            subscription_raw: None,
            user_raw: None,
            task_usage_raw: None,
            has_grok_code_access: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            working_dir: None,
            created_at: 1,
            last_used: 1,
        }
    }

    #[test]
    fn third_party_api_profile_uses_custom_model_without_persisting_key_in_config() {
        let temp = TestDir::new();
        let mut account = sample_account();
        account.auth_mode = GrokAuthMode::ApiKey;
        account.access_token.clear();
        account.refresh_token = None;
        account.api_key = Some("third-party-secret".to_string());
        account.api_base_url = Some("https://relay.example.com/v1".to_string());
        account.api_model = Some("grok-compatible".to_string());

        write_account_to_profile(&account, &temp.0).expect("write third-party profile");

        let config =
            std::fs::read_to_string(temp.0.join("config.toml")).expect("read generated config");
        assert!(config.contains("default = \"cockpit-api\""));
        assert!(config.contains("model = \"grok-compatible\""));
        assert!(config.contains("base_url = \"https://relay.example.com/v1\""));
        assert!(config.contains("env_key = \"XAI_API_KEY\""));
        assert!(!config.contains("third-party-secret"));
        assert_eq!(
            std::fs::read_to_string(temp.0.join("auth.json")).expect("read empty auth"),
            "{}"
        );
    }

    #[test]
    fn third_party_api_requires_valid_base_url_and_model_pair() {
        assert!(
            validate_api_provider_config(Some("https://relay.example.com/v1/"), Some("model"))
                .is_ok()
        );
        assert!(validate_api_provider_config(Some("https://relay.example.com/v1"), None).is_err());
        assert!(validate_api_provider_config(None, Some("model")).is_err());
        assert!(
            validate_api_provider_config(Some("ftp://relay.example.com/v1"), Some("model"))
                .is_err()
        );
        assert!(validate_api_provider_config(
            Some("https://user@relay.example.com/v1"),
            Some("model")
        )
        .is_err());
    }

    #[test]
    fn official_registry_preserves_unrelated_scopes() {
        let existing = json!({"custom-scope":{"key":"keep"}});
        let registry = auth_registry_for(&sample_account(), Some(existing));
        assert_eq!(registry["custom-scope"]["key"], "keep");
        assert_eq!(
            registry[crate::modules::grok_oauth::AUTH_REGISTRY_KEY]["key"],
            "secret-access"
        );
    }

    #[test]
    fn registry_uses_and_imports_account_oauth_client_id() {
        let mut account = sample_account();
        account.oidc_issuer = Some("https://auth.x.ai".to_string());
        account.oidc_client_id = Some("future-client-id".to_string());
        let registry = auth_registry_for(&account, None);
        let registry_key = "https://auth.x.ai::future-client-id";

        assert_eq!(registry[registry_key]["key"], "secret-access");
        let imported = parse_auth_registry(&registry).expect("parse dynamic registry key");
        assert_eq!(imported.oidc_issuer.as_deref(), Some("https://auth.x.ai"));
        assert_eq!(imported.oidc_client_id.as_deref(), Some("future-client-id"));
    }

    #[test]
    fn official_auth_round_trip_preserves_unknown_fields() {
        let raw = json!({
            "key": "original-access",
            "email": "person@example.com",
            "auth_mode": "oidc",
            "refresh_token": "original-refresh",
            "expires_at": "2030-03-17T17:46:40.123456Z",
            "future_field": {"nested": [1, 2, 3]}
        });
        let account = account_from_auth_object(&raw).expect("parse official auth object");
        let registry = auth_registry_for(&account, None);
        let official = &registry[crate::modules::grok_oauth::AUTH_REGISTRY_KEY];

        assert_eq!(official["key"], "original-access");
        assert_eq!(official["refresh_token"], "original-refresh");
        assert_eq!(official["expires_at"], "2030-03-17T17:46:40.123456Z");
        assert_eq!(official["future_field"], json!({"nested": [1, 2, 3]}));
    }

    #[test]
    fn official_sync_preserves_unrelated_registry_scopes() {
        let temp = TestDir::new();
        let auth_path = temp.0.join("auth.json");
        std::fs::write(
            &auth_path,
            serde_json::to_string_pretty(&json!({
                "custom-scope": {"key": "keep", "future": true}
            }))
            .expect("serialize seed registry"),
        )
        .expect("write seed registry");

        assert!(
            write_account_to_official_auth_path(&sample_account(), &auth_path)
                .expect("sync official auth")
        );
        let persisted: Value =
            serde_json::from_str(&std::fs::read_to_string(&auth_path).expect("read official auth"))
                .expect("parse official auth");
        assert_eq!(persisted["custom-scope"]["key"], "keep");
        assert_eq!(persisted["custom-scope"]["future"], true);
        assert_eq!(
            persisted[crate::modules::grok_oauth::AUTH_REGISTRY_KEY]["key"],
            "secret-access"
        );
    }

    #[test]
    fn api_key_official_sync_does_not_modify_existing_oauth_auth() {
        let temp = TestDir::new();
        let auth_path = temp.0.join("auth.json");
        let original = serde_json::to_string_pretty(&auth_registry_for(&sample_account(), None))
            .expect("serialize original auth");
        std::fs::write(&auth_path, &original).expect("write original auth");

        let mut api_key_account = sample_account();
        api_key_account.auth_mode = GrokAuthMode::ApiKey;
        api_key_account.access_token.clear();
        api_key_account.api_key = Some("xai-test-key".to_string());
        api_key_account.refresh_token = None;

        assert!(
            !write_account_to_official_auth_path(&api_key_account, &auth_path)
                .expect("select api key account")
        );
        assert_eq!(
            std::fs::read_to_string(&auth_path).expect("read preserved auth"),
            original
        );
    }

    #[test]
    fn deleting_auth_scope_scrubs_main_and_backup_without_removing_other_scopes() {
        let temp = TestDir::new();
        let auth_path = temp.0.join("auth.json");
        let backup_path = auth_path.with_extension("json.bak");
        let account = sample_account();
        let registry = auth_registry_for(&account, Some(json!({"custom-scope":{"key":"keep"}})));
        let content = serde_json::to_string_pretty(&registry).expect("serialize registry");
        std::fs::write(&auth_path, &content).expect("write auth registry");
        std::fs::write(&backup_path, &content).expect("write auth backup");

        assert!(
            remove_matching_auth_scope(&auth_path, &account, true).expect("remove main auth scope")
        );
        assert!(remove_matching_auth_scope(&backup_path, &account, false)
            .expect("remove backup auth scope"));

        for path in [&auth_path, &backup_path] {
            let persisted: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(path).expect("read scrubbed registry"),
            )
            .expect("parse scrubbed registry");
            assert!(persisted
                .get(crate::modules::grok_oauth::AUTH_REGISTRY_KEY)
                .is_none());
            assert_eq!(persisted["custom-scope"]["key"], "keep");
        }
    }

    #[test]
    fn external_login_change_is_reconciled_and_stale_refresh_does_not_overwrite_it() {
        let temp = TestDir::new();
        let auth_path = temp.0.join("auth.json");
        let account_a = sample_account();
        let mut account_b = sample_account();
        account_b.id = "account-2".to_string();
        account_b.email = "other@example.com".to_string();
        account_b.user_id = Some("user-2".to_string());
        account_b.principal_id = Some("principal-2".to_string());
        account_b.access_token = "external-b-access".to_string();
        account_b.refresh_token = Some("external-b-refresh".to_string());
        let external_registry = auth_registry_for(&account_b, None);
        std::fs::write(
            &auth_path,
            serde_json::to_string_pretty(&external_registry).expect("serialize external registry"),
        )
        .expect("write external registry");

        assert_eq!(
            resolve_account_id_from_registry(&[account_a.clone(), account_b], &external_registry),
            Some("account-2".to_string())
        );

        let mut refreshed_a = account_a;
        refreshed_a.access_token = "refreshed-a-access".to_string();
        let updated =
            write_account_to_auth_path_if_token_matches(&refreshed_a, &auth_path, "secret-access")
                .expect("compare and swap default auth");
        assert!(!updated);
        let persisted: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&auth_path).expect("read external registry after refresh"),
        )
        .expect("parse external registry after refresh");
        assert_eq!(
            persisted[crate::modules::grok_oauth::AUTH_REGISTRY_KEY]["key"],
            "external-b-access"
        );
    }

    #[test]
    fn external_principal_does_not_fall_back_to_a_same_email_account() {
        let account_a = sample_account();
        let mut external_account = sample_account();
        external_account.id = "external-account".to_string();
        external_account.principal_id = Some("external-principal".to_string());
        external_account.user_id = Some("external-user".to_string());
        external_account.access_token = "external-access".to_string();
        let external_registry = auth_registry_for(&external_account, None);

        assert_eq!(
            resolve_account_id_from_registry(&[account_a], &external_registry),
            None
        );
    }

    #[test]
    fn upsert_does_not_fall_back_to_email_when_strong_identity_conflicts() {
        let existing = sample_account();
        let mut different_principal = sample_account();
        different_principal.principal_id = Some("principal-2".to_string());
        assert!(!accounts_match_for_upsert(&different_principal, &existing));

        let mut different_user = sample_account();
        different_user.user_id = Some("user-2".to_string());
        assert!(!accounts_match_for_upsert(&different_user, &existing));
    }

    #[test]
    fn auth_cleanup_does_not_fall_back_to_email_when_strong_identity_conflicts() {
        let temp = TestDir::new();
        let auth_path = temp.0.join("auth.json");
        let account = sample_account();
        let mut external_account = sample_account();
        external_account.access_token = "external-access".to_string();
        external_account.principal_id = Some("principal-2".to_string());
        external_account.user_id = Some("user-2".to_string());
        let registry = auth_registry_for(&external_account, None);
        std::fs::write(
            &auth_path,
            serde_json::to_string_pretty(&registry).expect("serialize external registry"),
        )
        .expect("write external registry");

        assert!(!remove_matching_auth_scope(&auth_path, &account, true)
            .expect("compare external auth scope"));
        let persisted: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&auth_path).expect("read preserved external registry"),
        )
        .expect("parse preserved external registry");
        assert_eq!(
            persisted[crate::modules::grok_oauth::AUTH_REGISTRY_KEY]["key"],
            "external-access"
        );
    }

    #[test]
    fn current_account_token_refresh_updates_default_auth_with_matching_previous_token() {
        let temp = TestDir::new();
        let auth_path = temp.0.join("auth.json");
        let account = sample_account();
        let registry = auth_registry_for(&account, Some(json!({"custom-scope":{"key":"keep"}})));
        std::fs::write(
            &auth_path,
            serde_json::to_string_pretty(&registry).expect("serialize current registry"),
        )
        .expect("write current registry");

        let mut refreshed = account;
        refreshed.access_token = "refreshed-access".to_string();
        let updated =
            write_account_to_auth_path_if_token_matches(&refreshed, &auth_path, "secret-access")
                .expect("compare and swap current auth");
        assert!(updated);
        let persisted: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&auth_path).expect("read refreshed registry"),
        )
        .expect("parse refreshed registry");
        assert_eq!(
            persisted[crate::modules::grok_oauth::AUTH_REGISTRY_KEY]["key"],
            "refreshed-access"
        );
        assert_eq!(persisted["custom-scope"]["key"], "keep");
    }

    #[test]
    fn redacted_export_cannot_be_imported_as_an_empty_token_account() {
        let redacted =
            serde_json::to_value(crate::models::grok::GrokAccountView::from(&sample_account()))
                .expect("serialize redacted account");
        let error = parse_auth_registry(&redacted).expect_err("redacted export must be rejected");
        assert!(
            error.contains("缺少登录凭据") || error.contains("未识别"),
            "unexpected error: {error}"
        );
        let import_error = super::import_from_json(
            &serde_json::to_string(&vec![redacted]).expect("serialize redacted list"),
        )
        .expect_err("redacted list must be rejected");
        assert!(
            import_error.contains("缺少登录凭据"),
            "unexpected import error: {import_error}"
        );
    }

    #[test]
    fn full_export_json_can_be_reimported() {
        let account = sample_account();
        let exported = serde_json::to_string_pretty(&vec![&account]).expect("serialize export");
        let value: Value = serde_json::from_str(&exported).expect("parse export");
        let accounts: Vec<GrokAccount> =
            serde_json::from_value(value.clone()).expect("deserialize export as GrokAccount list");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].access_token, account.access_token);
        assert_eq!(accounts[0].refresh_token, account.refresh_token);
        super::validate_importable_account(&accounts[0])
            .expect("export credentials must be importable");
        // 导出数组应被识别为 Cockpit 完整导出格式
        assert!(
            matches!(
                super::try_import_cockpit_export(&value),
                Some(Err(_)) | Some(Ok(_))
            ),
            "export array should match cockpit export parser"
        );
        let via_registry = parse_auth_registry(&serde_json::to_value(&account).expect("to value"))
            .expect("full account object should parse");
        assert_eq!(via_registry.access_token, account.access_token);
    }

    #[test]
    fn refreshed_token_rotates_only_when_server_returns_replacement() {
        let mut account = sample_account();
        account.auth_raw = Some(json!({"future_field": "keep"}));
        apply_refreshed_token(
            &mut account,
            crate::modules::grok_oauth::GrokTokenResponse {
                access_token: "access-2".to_string(),
                refresh_token: Some("refresh-2".to_string()),
                id_token: Some("id-2".to_string()),
                token_type: Some("Bearer".to_string()),
                expires_in: None,
            },
        );
        assert_eq!(account.access_token, "access-2");
        assert_eq!(account.refresh_token.as_deref(), Some("refresh-2"));
        assert_eq!(account.auth_raw.as_ref().unwrap()["future_field"], "keep");

        apply_refreshed_token(
            &mut account,
            crate::modules::grok_oauth::GrokTokenResponse {
                access_token: "access-3".to_string(),
                refresh_token: None,
                id_token: None,
                token_type: None,
                expires_in: None,
            },
        );
        assert_eq!(account.access_token, "access-3");
        assert_eq!(account.refresh_token.as_deref(), Some("refresh-2"));
    }

    #[test]
    fn quota_401_retry_policy_allows_only_one_retry() {
        let unauthorized = Err("查询 Grok 配额返回 401".to_string());
        assert!(should_retry_quota_after_unauthorized(
            false,
            0,
            &unauthorized
        ));
        assert!(!should_retry_quota_after_unauthorized(
            false,
            1,
            &unauthorized
        ));
        assert!(!should_retry_quota_after_unauthorized(
            true,
            0,
            &unauthorized
        ));
        assert!(!should_retry_quota_after_unauthorized(
            false,
            0,
            &Err("查询 Grok 配额返回 500".to_string())
        ));
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cockpit-grok-account-test-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    struct EnvironmentGuard {
        previous_data_dir: Option<std::ffi::OsString>,
        previous_home: Option<std::ffi::OsString>,
    }

    #[cfg(unix)]
    impl EnvironmentGuard {
        fn new(root: &std::path::Path) -> Self {
            let data_dir = root.join("data");
            let home_dir = root.join("home");
            std::fs::create_dir_all(&data_dir).expect("create test data directory");
            std::fs::create_dir_all(&home_dir).expect("create test home directory");
            let previous_data_dir = std::env::var_os("COCKPIT_TOOLS_DATA_DIR");
            let previous_home = std::env::var_os("HOME");
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", data_dir);
            std::env::set_var("HOME", home_dir);
            Self {
                previous_data_dir,
                previous_home,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for EnvironmentGuard {
        fn drop(&mut self) {
            match self.previous_data_dir.as_ref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
            }
            match self.previous_home.as_ref() {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn removing_account_deletes_managed_profile_without_touching_unrelated_default_home() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock test environment");
        let temp = TestDir::new();
        let _environment = EnvironmentGuard::new(&temp.0);
        let account = sample_account();
        save_account_locked(&account).expect("save account fixture");
        let profile = super::managed_profile_dir(&account.id).expect("profile dir");
        assert!(profile.join("auth.json").exists());
        // 官方默认 home 若另有无关文件，删除账号不应要求清理它
        let default_home = default_grok_home().expect("resolve test Grok home");
        ensure_secret_dir(&default_home).expect("create default home");
        std::fs::write(default_home.join("auth.json"), "{}").expect("seed unrelated default auth");

        remove_account(&account.id).expect("remove account");

        assert!(super::load_account(&account.id).is_none());
        assert!(!profile.exists());
        assert!(default_home.join("auth.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn removing_bound_account_auto_unbinds_instances() {
        use crate::models::{
            codex::CodexAppSpeed, DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile,
            InstanceStore,
        };

        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock test environment");
        let temp = TestDir::new();
        let _environment = EnvironmentGuard::new(&temp.0);
        let account = sample_account();
        save_account_locked(&account).expect("save account fixture");

        let store = InstanceStore {
            instances: vec![InstanceProfile {
                id: "inst-333".to_string(),
                name: "333".to_string(),
                user_data_dir: temp.0.join("inst-333").to_string_lossy().to_string(),
                working_dir: None,
                extra_args: String::new(),
                bind_account_id: Some(account.id.clone()),
                launch_mode: InstanceLaunchMode::Cli,
                app_speed: CodexAppSpeed::Standard,
                created_at: 1,
                last_launched_at: None,
                last_pid: None,
            }],
            default_settings: DefaultInstanceSettings {
                bind_account_id: Some(account.id.clone()),
                follow_local_account: false,
                ..DefaultInstanceSettings::default()
            },
        };
        crate::modules::grok_instance::save_instance_store(&store).expect("save instance store");

        remove_account(&account.id).expect("remove bound account should auto-unbind");

        assert!(super::load_account(&account.id).is_none());
        let next = crate::modules::grok_instance::load_instance_store().expect("reload store");
        assert!(next.default_settings.bind_account_id.is_none());
        assert!(next.default_settings.follow_local_account);
        assert_eq!(next.instances.len(), 1);
        assert!(next.instances[0].bind_account_id.is_none());
        assert_eq!(next.instances[0].name, "333");
    }

    #[cfg(unix)]
    #[test]
    fn unix_secret_directory_and_reclaimed_lock_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TestDir::new();
        let secrets = temp.0.join("credentials");
        std::fs::create_dir_all(&secrets).expect("create permissive secret directory");
        std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o777))
            .expect("set initial secret permissions");
        ensure_secret_dir(&secrets).expect("secure credentials directory");
        assert_eq!(
            std::fs::metadata(&secrets)
                .expect("read credentials metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let target = secrets.join("auth.json");
        let lock_path = secrets.join("auth.json.cockpit.lock");
        std::fs::write(&lock_path, "0\n").expect("write stale lock");
        let lock = acquire_secret_lock(&target).expect("reclaim stale lock");
        assert_eq!(
            std::fs::metadata(&lock_path)
                .expect("read lock metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::read_to_string(&lock_path)
                .expect("read replacement lock")
                .trim(),
            std::process::id().to_string()
        );
        drop(lock);
        assert!(!lock_path.exists());
    }

    #[test]
    fn pick_best_prefers_newer_usable_official_over_stale_store() {
        let now = 1_700_000_000_i64;
        let store = LiveCredentialCandidate {
            source: GrokCredSource::Store,
            access_token: "store-access".into(),
            refresh_token: Some("store-rt".into()),
            expires_at: Some(now + 30), // 即将过期，视为不可直接用
            expires_at_raw: None,
            observed_at: now - 1000,
            entry: None,
            path: None,
        };
        let official = LiveCredentialCandidate {
            source: GrokCredSource::OfficialHome,
            access_token: "official-access".into(),
            refresh_token: Some("official-rt".into()),
            expires_at: Some(now + 3600),
            expires_at_raw: None,
            observed_at: now - 10,
            entry: None,
            path: None,
        };
        let managed = LiveCredentialCandidate {
            source: GrokCredSource::ManagedHome,
            access_token: "managed-access".into(),
            refresh_token: Some("managed-rt".into()),
            expires_at: Some(now + 1800),
            expires_at_raw: None,
            observed_at: now - 100,
            entry: None,
            path: None,
        };
        let candidates = vec![store, managed, official];
        let best = pick_best_live_credential(&candidates, now).expect("best");
        assert_eq!(best.access_token, "official-access");
        assert_eq!(best.source, GrokCredSource::OfficialHome);
        assert!(access_still_usable(best.expires_at, now));
    }

    #[test]
    fn find_matching_auth_entry_requires_same_account_identity() {
        let account = sample_account();
        // 两个合法 xAI registry 键：只能命中 principal/user/email 一致的那条
        let registry = json!({
            "https://auth.x.ai::client-a": {
                "email": "other@example.com",
                "principal_id": "other-principal",
                "user_id": "other-user",
                "key": "other-access",
                "refresh_token": "other-rt",
                "expires_at": "2099-01-01T00:00:00Z"
            },
            "https://auth.x.ai::client-b": {
                "email": "person@example.com",
                "principal_id": "principal-1",
                "user_id": "user-1",
                "key": "same-access",
                "refresh_token": "same-rt",
                "expires_at": "2099-01-01T00:00:00Z"
            }
        });
        let entry = find_matching_auth_entry_in_registry(&registry, &account)
            .expect("should match same principal/user");
        assert_eq!(string_field(entry, "key").as_deref(), Some("same-access"));
        assert_ne!(string_field(entry, "key").as_deref(), Some("other-access"));

        let stranger = GrokAccount {
            email: "nobody@example.com".into(),
            principal_id: Some("nope".into()),
            user_id: Some("nope".into()),
            access_token: "unrelated".into(),
            ..sample_account()
        };
        assert!(
            find_matching_auth_entry_in_registry(&registry, &stranger).is_none(),
            "must not adopt another account's credentials"
        );
    }

    #[test]
    fn corrupted_index_is_quarantined_and_rebuilt_from_account_details() {
        let temp = TestDir::new();
        let details = temp.0.join("grok_accounts");
        std::fs::create_dir_all(&details).expect("create account details directory");
        let account = sample_account();
        std::fs::write(
            details.join("account-1.json"),
            serde_json::to_string_pretty(&account).expect("serialize account detail"),
        )
        .expect("write account detail");
        let index_path = temp.0.join("grok_accounts.json");
        std::fs::write(&index_path, "{invalid-json").expect("write corrupted index");

        let index = load_index_from_paths(&index_path, &details)
            .expect("rebuild index from valid account details");
        assert_eq!(index.accounts.len(), 1);
        assert_eq!(index.accounts[0].id, "account-1");
        let persisted: crate::models::grok::GrokAccountIndex = serde_json::from_str(
            &std::fs::read_to_string(&index_path).expect("read rebuilt index"),
        )
        .expect("rebuilt index should be valid JSON");
        assert_eq!(persisted.accounts.len(), 1);
        assert!(std::fs::read_dir(&temp.0)
            .expect("scan quarantine files")
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .any(|name| name.starts_with("grok_accounts.json.invalid-json.")));
    }

    #[cfg(unix)]
    #[test]
    fn corrupted_account_detail_restores_private_backup() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TestDir::new();
        let path = temp.0.join("account-1.json");
        let backup = temp.0.join("account-1.json.bak");
        std::fs::write(&path, "{invalid-json").expect("write corrupted account");
        std::fs::write(
            &backup,
            serde_json::to_string_pretty(&sample_account()).expect("serialize account backup"),
        )
        .expect("write account backup");

        let restored = load_account_from_path(&path, "account-1").expect("restore account backup");
        assert_eq!(restored.id, "account-1");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("read restored account metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let reloaded = load_account_from_path(&path, "account-1").expect("reload restored account");
        assert_eq!(reloaded.id, "account-1");
        assert_eq!(reloaded.access_token, restored.access_token);
    }

    #[test]
    fn parses_billing_summary_without_translating_tier() {
        let billing = json!({
            "config": {
                "currentPeriod": {"type":"weekly", "start":"a", "end":"b"},
                "creditUsagePercent": 42.5,
                "subscription_tier": "SUBSCRIPTION_TIER_SUPERGROK",
                "productUsage": [{"product":"coding", "usagePercent":12.0}]
            }
        });
        let quota = quota_from_payload(&billing, None, None, None);
        assert_eq!(
            quota.subscription_tier.as_deref(),
            Some("SUBSCRIPTION_TIER_SUPERGROK")
        );
        assert_eq!(quota.weekly_limit_percent, Some(42.5));
        assert_eq!(quota.products[0].product, "coding");
    }

    #[test]
    fn parses_credit_bag_without_treating_zero_cap_as_exhausted() {
        let billing = json!({
            "config": {
                "onDemandCap": {"val": 0},
                "weeklyCredits": {
                    "total": {"val": 100},
                    "remaining": {"val": 75}
                }
            }
        });
        let quota = quota_from_payload(&billing, None, None, None);
        assert_eq!(quota.weekly_limit_percent, Some(25.0));
        assert_eq!(quota.weekly_used, Some(25.0));
        assert_eq!(quota.weekly_total, Some(100.0));
        assert_eq!(quota.on_demand_cap, Some(0.0));
    }

    #[test]
    fn parses_product_usage_absolute_amounts() {
        let billing = json!({
            "config": {
                "productUsage": [{
                    "product": "GrokBuild",
                    "usagePercent": 40.0,
                    "used": 40,
                    "total": 100,
                    "remaining": 60
                }]
            }
        });
        let quota = quota_from_payload(&billing, None, None, None);
        assert_eq!(quota.products[0].product, "GrokBuild");
        assert_eq!(quota.products[0].usage_percent, Some(40.0));
        assert_eq!(quota.products[0].used, Some(40.0));
        assert_eq!(quota.products[0].total, Some(100.0));
        assert_eq!(quota.products[0].remaining, Some(60.0));
    }

    #[test]
    fn uses_raw_subscription_from_cli_user() {
        let billing = json!({"config": {}});
        let cli_user = json!({
            "hasGrokCodeAccess": true,
            "subscription": {
                "tier": "SUBSCRIPTION_TIER_SUPERGROK_HEAVY",
                "status": "SUBSCRIPTION_STATUS_ACTIVE"
            }
        });
        let quota = quota_from_payload(&billing, None, Some(&cli_user), None);
        assert_eq!(
            quota.subscription_tier.as_deref(),
            Some("SUBSCRIPTION_TIER_SUPERGROK_HEAVY")
        );
        assert_eq!(
            quota.subscription_status.as_deref(),
            Some("SUBSCRIPTION_STATUS_ACTIVE")
        );
    }

    #[test]
    fn parses_task_usage_limits() {
        let billing = json!({"config": {}});
        let task_usage = json!({
            "frequentUsage": 2,
            "frequentLimit": 10,
            "occasionalUsage": 3,
            "occasionalLimit": 30
        });
        let quota = quota_from_payload(&billing, None, None, Some(&task_usage));
        assert_eq!(quota.frequent_usage, Some(2.0));
        assert_eq!(quota.frequent_limit, Some(10.0));
        assert_eq!(quota.occasional_usage, Some(3.0));
        assert_eq!(quota.occasional_limit, Some(30.0));
    }

    #[test]
    fn remaining_metrics_include_weekly_and_products() {
        let mut account = sample_account();
        account.quota = Some(GrokQuota {
            weekly_limit_percent: Some(40.0),
            products: vec![GrokProductUsage {
                product: "GrokBuild".to_string(),
                usage_percent: Some(25.0),
                used: None,
                total: None,
                remaining: None,
            }],
            ..Default::default()
        });
        let view = GrokAccountView::from(&account);
        let metrics = quota_remaining_metrics(&view);
        assert!(metrics
            .iter()
            .any(|(name, remaining)| { name == "weekly" && *remaining == 60 }));
        assert!(metrics
            .iter()
            .any(|(name, remaining)| { name == "GrokBuild" && *remaining == 75 }));
    }

    #[test]
    #[cfg(unix)]
    fn known_test_fixture_is_not_returned_from_account_list() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock test environment");
        let temp = TestDir::new();
        let _environment = EnvironmentGuard::new(&temp.0);
        save_account_locked(&sample_account()).expect("save account fixture");

        let listed = list_accounts_checked().expect("list accounts");

        assert!(listed.is_empty());
        assert!(load_account("account-1").is_none());
    }

    #[test]
    fn adopts_rotated_tokens_from_default_auth_when_identity_matches() {
        let mut account = sample_account();
        account.access_token = "old-access".to_string();
        account.refresh_token = Some("old-refresh".to_string());
        let registry = json!({
            "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828": {
                "key": "new-access",
                "refresh_token": "new-refresh",
                "email": "person@example.com",
                "user_id": "user-1",
                "principal_id": "principal-1",
                "expires_at": "2030-01-01T00:00:00Z"
            }
        });
        // 身份匹配 + 字段读取：刷新前吸收 CLI 已轮换凭据的前置条件
        let entry = auth_registry_entry(&registry).expect("entry");
        assert!(auth_entry_matches_account(
            &Value::Object(entry.clone()),
            &account
        ));
        assert_eq!(string_field(entry, "key").as_deref(), Some("new-access"));
        assert_eq!(
            string_field(entry, "refresh_token").as_deref(),
            Some("new-refresh")
        );
    }
}
