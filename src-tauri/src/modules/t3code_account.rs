use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::models::t3code::{T3CodeAccount, T3CodeAccountIndex, T3CodeAuthMode};
use crate::modules::t3code_app_injection::ObservedClerkAccount;
use crate::modules::{account, atomic_write};
use tauri::Emitter;

const ACCOUNTS_DIR: &str = "t3code_accounts";
const ACCOUNTS_INDEX_FILE: &str = "t3code_accounts.json";
const CLERK_DIR_NAME: &str = "clerk";
const CLERK_TOKEN_FILE_NAME: &str = "clerk-tokens.json";
const CLERK_JWT_KEY: &str = "__clerk_client_jwt";

#[derive(Clone, Debug)]
struct ObservedStateEntry {
    instance_id: String,
    clerk: ObservedClerkAccount,
    received_at: Instant,
    confirmed_account_id: Option<String>,
}

const OBSERVATION_TTL: Duration = Duration::from_secs(12);

static T3CODE_ACCOUNT_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));
static OBSERVED_BY_STATE_DIR: std::sync::LazyLock<Mutex<HashMap<PathBuf, ObservedStateEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn remember_observed_clerk_state(
    instance_id: &str,
    state_dir: &Path,
    clerk: &ObservedClerkAccount,
) {
    if let Ok(mut map) = OBSERVED_BY_STATE_DIR.lock() {
        let confirmed_account_id = map.get(state_dir).and_then(|previous| {
            (same_live_identity(&previous.clerk, clerk) && previous.instance_id == instance_id)
                .then(|| previous.confirmed_account_id.clone())
                .flatten()
        });
        let changed = map
            .get(state_dir)
            .is_some_and(|previous| previous.confirmed_account_id != confirmed_account_id);
        map.insert(
            state_dir.to_path_buf(),
            ObservedStateEntry {
                instance_id: instance_id.to_owned(),
                clerk: clerk.clone(),
                received_at: Instant::now(),
                confirmed_account_id,
            },
        );
        drop(map);
        if changed {
            emit_accounts_updated(instance_id);
        }
    }
}

fn same_live_identity(a: &ObservedClerkAccount, b: &ObservedClerkAccount) -> bool {
    a.signed_in
        && b.signed_in
        && a.user_id.is_some()
        && a.user_id == b.user_id
        && a.session_id.is_some()
        && a.session_id == b.session_id
}

fn emit_accounts_updated(instance_id: &str) {
    if let Some(app) = crate::get_app_handle() {
        let _ = app.emit("t3code-accounts-updated", instance_id);
    }
}

pub(crate) fn forget_observed_clerk_state(instance_id: Option<&str>) {
    let removed = OBSERVED_BY_STATE_DIR.lock().ok().is_some_and(|mut map| {
        let previous = map.len();
        map.retain(|_, entry| instance_id.is_some_and(|id| entry.instance_id != id));
        previous != map.len()
    });
    if removed {
        emit_accounts_updated(instance_id.unwrap_or("__default__"));
    }
}

pub(crate) fn confirmed_current_account_id() -> Option<String> {
    let dir = default_state_dir().ok()?;
    OBSERVED_BY_STATE_DIR
        .lock()
        .ok()?
        .get(&dir)
        .filter(|entry| entry.received_at.elapsed() < OBSERVATION_TTL)
        .and_then(|entry| entry.confirmed_account_id.clone())
}

fn observed_clerk_for_state_dir(state_dir: &Path) -> Option<ObservedClerkAccount> {
    OBSERVED_BY_STATE_DIR
        .lock()
        .ok()
        .and_then(|map| {
            map.get(state_dir)
                .filter(|entry| entry.received_at.elapsed() < OBSERVATION_TTL)
                .map(|entry| entry.clerk.clone())
        })
        .filter(|clerk| clerk.signed_in)
}

fn token_fingerprint(jwt: &str) -> Option<String> {
    let trimmed = jwt.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(trimmed.as_bytes());
    Some(format!("{:x}", hasher.finalize()))
}

fn is_placeholder_email(email: &str) -> bool {
    let trimmed = email.trim();
    !trimmed.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && !domain.is_empty()
            && !domain.contains('@')
            && !trimmed.chars().any(char::is_whitespace)
    }) || (trimmed.starts_with("clerk-") && trimmed.ends_with("@t3code.cloud"))
}

fn normalize_plan_type(value: Option<&str>) -> Option<String> {
    normalize_string(value).filter(|plan| !plan.eq_ignore_ascii_case("T3 Connect"))
}

fn read_state_dir_clerk_token(state_dir: &Path) -> Option<(String, Value)> {
    let clerk_path = clerk_tokens_path_for_state_dir(state_dir);
    let raw_text = fs::read_to_string(&clerk_path).ok()?;
    let parsed_json = serde_json::from_str::<Value>(&raw_text).ok()?;
    let clerk_jwt = parsed_json
        .get(CLERK_JWT_KEY)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    (!clerk_jwt.is_empty()).then_some((clerk_jwt, parsed_json))
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn normalize_string(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
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

fn account_id(user_id: Option<&str>, email: Option<&str>, environment_id: Option<&str>) -> String {
    let identity = normalize_string(user_id)
        .or_else(|| normalize_string(email))
        .or_else(|| normalize_string(environment_id))
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    format!("t3code_{:x}", md5::compute(identity))
}

fn accounts_dir() -> Result<PathBuf, String> {
    let path = account::get_data_dir()?.join(ACCOUNTS_DIR);
    fs::create_dir_all(&path).map_err(|error| format!("创建 T3 Code 账号目录失败: {}", error))?;
    Ok(path)
}

fn index_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(
        crate::modules::provider_account_index::storage_path::<T3CodeAccountIndex>(&index_path()?)?
            .to_string_lossy()
            .into_owned(),
    )
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
        return Err("T3 Code 账号 ID 非法".to_string());
    }
    Ok(trimmed)
}

fn account_path(account_id: &str) -> Result<PathBuf, String> {
    Ok(accounts_dir()?.join(format!("{}.json", safe_account_id(account_id)?)))
}

fn load_index() -> Result<T3CodeAccountIndex, String> {
    crate::modules::provider_account_index::checked(
        &index_path()?,
        crate::modules::provider_index_maintenance::schedule,
        |_| None,
        |_| false,
    )
}

fn save_index(index: &T3CodeAccountIndex) -> Result<(), String> {
    crate::modules::provider_account_index::save(&index_path()?, index)
}

pub fn load_account(account_id: &str) -> Option<T3CodeAccount> {
    let path = account_path(account_id).ok()?;
    let content = fs::read_to_string(&path).ok()?;
    match crate::modules::secure_account_storage::deserialize_account_file::<T3CodeAccount>(
        &path, &content,
    ) {
        Ok((account, needs_rotation)) => {
            if needs_rotation {
                let account_for_rewrite = account.clone();
                crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
                    "t3code",
                    account_for_rewrite.id.clone(),
                    path.clone(),
                    content.as_bytes(),
                    move || {
                        crate::modules::secure_account_storage::serialize_account_file(
                            "t3code",
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

fn save_account_file(account: &T3CodeAccount) -> Result<(), String> {
    let content =
        crate::modules::secure_account_storage::serialize_account_file("t3code", account)?;
    atomic_write::write_string_atomic(&account_path(&account.id)?, &content)
        .map_err(|error| format!("保存 T3 Code 账号失败: {}", error))
}

pub fn default_t3_base_dir() -> Result<PathBuf, String> {
    if let Some(custom) = std::env::var_os("T3CODE_HOME")
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(custom));
    }
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    Ok(home.join(".t3"))
}

pub fn default_state_dir() -> Result<PathBuf, String> {
    Ok(default_t3_base_dir()?.join("userdata"))
}

pub fn state_dir_for_instance_root(root: &Path) -> PathBuf {
    root.join("t3").join("userdata")
}

pub fn clerk_tokens_path_for_state_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(CLERK_DIR_NAME).join(CLERK_TOKEN_FILE_NAME)
}

pub fn settings_path_for_state_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("settings.json")
}

#[derive(Debug, Default)]
struct ParsedClerkJwt {
    subject: Option<String>,
    email: Option<String>,
    name: Option<String>,
    session_id: Option<String>,
    expires_at: Option<i64>,
}

fn decode_jwt_payload(token: &str) -> Option<Value> {
    let mut parts = token.trim().split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .or_else(|_| {
            let padded = format!("{}{}", payload, "=".repeat((4 - payload.len() % 4) % 4));
            base64::engine::general_purpose::URL_SAFE.decode(padded.as_bytes())
        })
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn parse_clerk_jwt(token: &str) -> ParsedClerkJwt {
    let Some(payload) = decode_jwt_payload(token) else {
        return ParsedClerkJwt::default();
    };
    let subject = payload
        .get("sub")
        .and_then(Value::as_str)
        .and_then(|v| normalize_string(Some(v)));
    let email = payload
        .get("email")
        .or_else(|| payload.get("primary_email"))
        .or_else(|| payload.get("user_email"))
        .and_then(Value::as_str)
        .and_then(|v| normalize_string(Some(v)));
    let name = payload
        .get("name")
        .or_else(|| payload.get("username"))
        .and_then(Value::as_str)
        .and_then(|v| normalize_string(Some(v)));
    let session_id = payload
        .get("sid")
        .or_else(|| payload.get("jti"))
        .and_then(Value::as_str)
        .and_then(|v| normalize_string(Some(v)));
    let expires_at = payload.get("exp").and_then(Value::as_i64);
    ParsedClerkJwt {
        subject,
        email,
        name,
        session_id,
        expires_at,
    }
}

pub fn upsert_account(value: T3CodeAccount) -> Result<T3CodeAccount, String> {
    let _guard = T3CODE_ACCOUNT_LOCK
        .lock()
        .map_err(|_| "获取 T3 Code 账号锁失败".to_string())?;
    upsert_account_locked(value)
}

fn upsert_account_locked(mut value: T3CodeAccount) -> Result<T3CodeAccount, String> {
    value.email = normalize_string(Some(&value.email))
        .filter(|email| !is_placeholder_email(email))
        .unwrap_or_default();
    value.user_id = normalize_string(value.user_id.as_deref());
    value.display_name = normalize_string(value.display_name.as_deref());
    value.avatar_url = normalize_string(value.avatar_url.as_deref());
    value.session_id = normalize_string(value.session_id.as_deref());
    value.environment_id = normalize_string(value.environment_id.as_deref());
    value.plan_type = match value.auth_mode {
        T3CodeAuthMode::Clerk => normalize_plan_type(value.plan_type.as_deref()),
        T3CodeAuthMode::LocalProfile => {
            normalize_plan_type(value.plan_type.as_deref()).or_else(|| Some("Local".to_string()))
        }
    };
    value.tags = normalize_tags(value.tags.unwrap_or_default());

    if value.id.trim().is_empty() {
        let token_fp = token_fingerprint(&value.clerk_jwt);
        value.id = account_id(
            value.user_id.as_deref(),
            (!value.email.is_empty()).then_some(value.email.as_str()),
            token_fp.as_deref().or(value.environment_id.as_deref()),
        );
    }

    if let Some(existing) = load_account(&value.id) {
        if value.tags.is_none() {
            value.tags = existing.tags;
        }
        if value.display_name.is_none() {
            value.display_name = existing.display_name;
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

    save_account_file(&value)?;
    let mut index = load_index()?;
    let summary = value.summary();
    if let Some(existing) = index.accounts.iter_mut().find(|item| item.id == value.id) {
        *existing = summary;
    } else {
        index.accounts.push(summary);
    }
    if index.current_account_id.is_none() {
        index.current_account_id = Some(value.id.clone());
    }
    save_index(&index)?;
    Ok(value)
}

pub fn list_accounts_checked() -> Result<Vec<T3CodeAccount>, String> {
    let index = load_index()?;
    let mut accounts = Vec::with_capacity(index.accounts.len());
    for summary in &index.accounts {
        if let Some(account) = load_account(&summary.id) {
            accounts.push(account);
        }
    }
    Ok(accounts)
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    remove_accounts(&[account_id.to_string()])
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    let _guard = T3CODE_ACCOUNT_LOCK
        .lock()
        .map_err(|_| "获取 T3 Code 账号锁失败".to_string())?;
    let remove_set: HashSet<&str> = account_ids.iter().map(String::as_str).collect();
    for id in &remove_set {
        if let Ok(path) = account_path(id) {
            let _ = fs::remove_file(path);
        }
    }
    let mut index = load_index()?;
    index
        .accounts
        .retain(|item| !remove_set.contains(item.id.as_str()));
    if index
        .current_account_id
        .as_deref()
        .is_some_and(|current| remove_set.contains(current))
    {
        index.current_account_id = index.accounts.first().map(|item| item.id.clone());
    }
    save_index(&index)
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<T3CodeAccount, String> {
    let _guard = T3CODE_ACCOUNT_LOCK
        .lock()
        .map_err(|_| "获取 T3 Code 账号锁失败".to_string())?;
    let mut account = load_account(account_id).ok_or_else(|| "T3 Code 账号不存在".to_string())?;
    account.tags = normalize_tags(tags);
    upsert_account_locked(account)
}

fn find_matching_existing_account(
    accounts: &[T3CodeAccount],
    user_id: Option<&str>,
    token_fp: Option<&str>,
) -> Option<T3CodeAccount> {
    if let Some(uid) = user_id {
        if let Some(found) = accounts
            .iter()
            .find(|acc| acc.user_id.as_deref() == Some(uid))
        {
            return Some(found.clone());
        }
    }
    let fp = token_fp?;
    accounts
        .iter()
        .find(|acc| {
            let different_user =
                user_id
                    .zip(acc.user_id.as_deref())
                    .is_some_and(|(incoming, stored)| {
                        incoming != stored
                            && (stored.starts_with("user_") || acc.identity_verified_at.is_some())
                    });
            acc.auth_mode == T3CodeAuthMode::Clerk
                && !different_user
                && token_fingerprint(&acc.clerk_jwt).as_deref() == Some(fp)
        })
        .cloned()
}

fn matching_observed_account(
    index: &T3CodeAccountIndex,
    observed: &ObservedClerkAccount,
    disk: Option<&(String, Value)>,
    pending_id: Option<&str>,
) -> Option<T3CodeAccount> {
    let user_id = observed.user_id.as_deref()?;
    if let Some(((jwt, _), fingerprint)) =
        disk.and_then(|entry| token_fingerprint(&entry.0).map(|fp| (entry, fp)))
    {
        let claims = parse_clerk_jwt(jwt);
        let ids = [
            account_id(
                claims.subject.as_deref(),
                claims.email.as_deref(),
                Some(&fingerprint),
            ),
            account_id(None, None, Some(&fingerprint)),
        ];
        let matching_backup = pending_id
            .into_iter()
            .chain(ids.iter().map(String::as_str))
            .filter_map(load_account)
            .find(|account| {
                let different_user = account.user_id.as_deref().is_some_and(|stored| {
                    stored != user_id
                        && (stored.starts_with("user_") || account.identity_verified_at.is_some())
                });
                let different_session = account.session_id.as_deref().is_some_and(|stored| {
                    account.identity_verified_at.is_none()
                        && stored.starts_with("sess_")
                        && Some(stored) != observed.session_id.as_deref()
                });
                account.auth_mode == T3CodeAuthMode::Clerk
                    && !different_user
                    && !different_session
                    && token_fingerprint(&account.clerk_jwt).as_deref() == Some(&fingerprint)
            });
        if matching_backup.is_some() {
            return matching_backup;
        }
    }

    index
        .accounts
        .iter()
        .filter(|item| {
            item.auth_mode == T3CodeAuthMode::Clerk && item.user_id.as_deref() == Some(user_id)
        })
        .filter_map(|item| load_account(&item.id))
        .find(|account| account.identity_verified_at.is_some())
}

fn apply_clerk_snapshot(account: &mut T3CodeAccount, observed: &ObservedClerkAccount) {
    if let Some(email) =
        normalize_string(observed.email.as_deref()).filter(|email| !is_placeholder_email(email))
    {
        if account.identity_verified_at.is_none() || account.email != email {
            account.identity_verified_at = Some(now_ts());
        }
        account.email = email;
    }
    account.user_id = observed.user_id.clone();
    account.session_id = observed.session_id.clone();
    account.expires_at = observed.expires_at;
    account.display_name =
        normalize_string(observed.display_name.as_deref()).or(account.display_name.take());
    account.avatar_url =
        normalize_string(observed.avatar_url.as_deref()).or(account.avatar_url.take());

    if let Some(((checked, used), total)) = observed
        .relay_verified_at
        .zip(observed.tunnels_used)
        .zip(observed.tunnels_limit)
        .filter(|((checked, used), total)| {
            *checked > 0
                && used.is_finite()
                && *used >= 0.0
                && used.fract() == 0.0
                && total.is_finite()
                && *total >= 0.0
                && total.fract() == 0.0
        })
    {
        if account
            .relay_verified_at
            .is_none_or(|previous| checked >= previous)
        {
            account.relay_verified_at = Some(checked);
            account.usage_updated_at = Some(checked);
            account.plan_type = observed.plan_type.clone();
            account.t3chat_linked = observed.t3chat_linked;
            account.quota_used = Some(used);
            account.quota_total = Some(total);
            account.quota_remaining = Some((total - used).max(0.0));
        }
    }
    account.quota_reset_at = None;
    if let Some(failed) = observed.relay_failed_at.filter(|failed| {
        account
            .relay_verified_at
            .is_none_or(|checked| *failed >= checked)
    }) {
        account.quota_query_last_error_at = Some(failed);
        account.quota_query_last_error = Some(clerk_sync_message("syncUnavailable"));
    } else if account.relay_verified_at.is_some_and(|checked| {
        observed.relay_verified_at == Some(checked)
            && account
                .quota_query_last_error_at
                .is_none_or(|failed| checked >= failed)
    }) {
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
    }
}

fn clerk_sync_message(key: &str) -> String {
    crate::modules::i18n::translate(
        &crate::modules::config::get_user_config().language,
        &format!("t3code.{key}"),
        &[],
    )
}

pub(crate) fn sync_observed_clerk_account(
    state_dir: &Path,
    observed: &ObservedClerkAccount,
) -> Result<bool, String> {
    if !observed_clerk_for_state_dir(state_dir)
        .is_some_and(|latest| same_live_identity(&latest, observed))
    {
        return Ok(false);
    }
    let disk = read_state_dir_clerk_token(state_dir);
    let is_default = default_state_dir().ok().as_deref() == Some(state_dir);
    let _guard = T3CODE_ACCOUNT_LOCK
        .lock()
        .map_err(|_| "获取 T3 Code 账号锁失败".to_string())?;
    if !observed_clerk_for_state_dir(state_dir)
        .is_some_and(|latest| same_live_identity(&latest, observed))
    {
        return Ok(false);
    }
    let index = load_index()?;
    let pending_id = is_default
        .then_some(index.current_account_id.as_deref())
        .flatten();
    let Some(mut account) = matching_observed_account(&index, observed, disk.as_ref(), pending_id)
    else {
        return Ok(false);
    };
    let old = account.clone();
    apply_clerk_snapshot(&mut account, observed);
    if let Some((jwt, raw)) = disk {
        let claims = parse_clerk_jwt(&jwt);
        let exact_backup = token_fingerprint(&jwt) == token_fingerprint(&account.clerk_jwt);
        let same_session = claims.session_id.is_some()
            && claims.session_id == observed.session_id
            && claims.subject == observed.user_id;
        if exact_backup || same_session {
            account.clerk_jwt = jwt;
            account.clerk_tokens_raw = Some(raw);
        }
    }
    let changed = old != account;
    if changed {
        upsert_account_locked(account.clone())?;
    }
    let confirmed = OBSERVED_BY_STATE_DIR.lock().ok().is_some_and(|mut map| {
        if let Some(entry) = map.get_mut(state_dir).filter(|entry| {
            entry.received_at.elapsed() < OBSERVATION_TTL
                && same_live_identity(&entry.clerk, observed)
        }) {
            if entry.confirmed_account_id.as_deref() != Some(account.id.as_str()) {
                entry.confirmed_account_id = Some(account.id);
                return true;
            }
        }
        false
    });
    Ok(changed || confirmed)
}

pub fn import_from_state_dir(
    state_dir: &Path,
    custom_label: Option<&str>,
) -> Result<T3CodeAccount, String> {
    if !state_dir.exists() {
        return Err(format!(
            "未找到本机 T3 Code 数据目录: {}",
            state_dir.display()
        ));
    }
    let clerk_path = clerk_tokens_path_for_state_dir(state_dir);
    let environment_id = fs::read_to_string(state_dir.join("environment-id"))
        .ok()
        .and_then(|v| normalize_string(Some(&v)));
    let settings_raw = fs::read_to_string(settings_path_for_state_dir(state_dir))
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok());

    let mut clerk_jwt = String::new();
    let mut clerk_tokens_raw = None;
    if clerk_path.exists() {
        let raw_text = fs::read_to_string(&clerk_path)
            .map_err(|error| format!("读取 T3 Code Clerk 凭据失败: {}", error))?;
        if let Ok(parsed_json) = serde_json::from_str::<Value>(&raw_text) {
            clerk_jwt = parsed_json
                .get(CLERK_JWT_KEY)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            clerk_tokens_raw = Some(parsed_json);
        }
    }
    if clerk_jwt.is_empty() {
        return Err(
            "未检测到本机 T3 Code 登录凭据，请先在 T3 Code 左下角登录 T3 Connect 后再导入"
                .to_string(),
        );
    }

    let parsed = parse_clerk_jwt(&clerk_jwt);
    let observed = observed_clerk_for_state_dir(state_dir);
    let token_fp = token_fingerprint(&clerk_jwt);

    let obs_user_id = observed
        .as_ref()
        .and_then(|o| normalize_string(o.user_id.as_deref()));
    let obs_email = observed
        .as_ref()
        .and_then(|o| normalize_string(o.email.as_deref()));
    let obs_name = observed
        .as_ref()
        .and_then(|o| normalize_string(o.display_name.as_deref()));
    let obs_avatar = observed
        .as_ref()
        .and_then(|o| normalize_string(o.avatar_url.as_deref()));
    let obs_session = observed
        .as_ref()
        .and_then(|o| normalize_string(o.session_id.as_deref()));
    let obs_exp = observed.as_ref().and_then(|o| o.expires_at);

    let candidate_user_id =
        obs_user_id.or_else(|| parsed.subject.filter(|id| id.starts_with("user_")));
    let candidate_email = obs_email.or(parsed.email);

    let existing_accounts = list_accounts_checked().unwrap_or_default();
    let existing = find_matching_existing_account(
        &existing_accounts,
        observed
            .as_ref()
            .and_then(|account| account.user_id.as_deref()),
        token_fp.as_deref(),
    );

    let user_id = candidate_user_id.or_else(|| {
        existing.as_ref().and_then(|acc| {
            acc.user_id
                .clone()
                .filter(|uid| Some(uid.as_str()) != acc.environment_id.as_deref())
        })
    });
    let email = existing
        .as_ref()
        .filter(|account| account.identity_verified_at.is_some())
        .map(|account| account.email.clone())
        .or(candidate_email)
        .or_else(|| {
            existing
                .as_ref()
                .map(|acc| acc.email.clone())
                .filter(|e| !is_placeholder_email(e))
        })
        .unwrap_or_default();
    let display_name = normalize_string(custom_label)
        .or(obs_name)
        .or(parsed.name)
        .or_else(|| existing.as_ref().and_then(|acc| acc.display_name.clone()));
    let avatar_url =
        obs_avatar.or_else(|| existing.as_ref().and_then(|acc| acc.avatar_url.clone()));
    let session_id = obs_session
        .or(parsed.session_id)
        .or_else(|| existing.as_ref().and_then(|acc| acc.session_id.clone()));
    let expires_at = obs_exp
        .or(parsed.expires_at)
        .or_else(|| existing.as_ref().and_then(|acc| acc.expires_at));
    let id = existing
        .as_ref()
        .map(|acc| acc.id.clone())
        .unwrap_or_else(|| {
            account_id(
                observed
                    .as_ref()
                    .and_then(|account| account.user_id.as_deref()),
                None,
                token_fp.as_deref().or(environment_id.as_deref()),
            )
        });
    let now = now_ts();
    let mut account = T3CodeAccount {
        id,
        auth_mode: T3CodeAuthMode::Clerk,
        email,
        user_id,
        display_name,
        avatar_url,
        clerk_jwt,
        session_id,
        environment_id,
        expires_at,
        identity_verified_at: existing.as_ref().and_then(|a| a.identity_verified_at),
        relay_verified_at: existing.as_ref().and_then(|a| a.relay_verified_at),
        t3chat_linked: existing.as_ref().and_then(|a| a.t3chat_linked),
        plan_type: existing.as_ref().and_then(|a| a.plan_type.clone()),
        quota_total: existing.as_ref().and_then(|a| a.quota_total),
        quota_used: existing.as_ref().and_then(|a| a.quota_used),
        quota_remaining: existing.as_ref().and_then(|a| a.quota_remaining),
        quota_reset_at: None,
        quota_query_last_error: existing
            .as_ref()
            .and_then(|a| a.quota_query_last_error.clone()),
        quota_query_last_error_at: existing.as_ref().and_then(|a| a.quota_query_last_error_at),
        usage_updated_at: existing.as_ref().and_then(|a| a.relay_verified_at),
        tags: existing.and_then(|acc| acc.tags),
        clerk_tokens_raw,
        settings_snapshot_raw: settings_raw,
        created_at: now,
        last_used: now,
    };
    if let Some(observed) = &observed {
        apply_clerk_snapshot(&mut account, observed);
    }
    let saved = upsert_account(account)?;
    if let Some(observed) = &observed {
        if sync_observed_clerk_account(state_dir, observed)? {
            emit_accounts_updated("__default__");
        }
    }
    Ok(saved)
}

pub async fn import_from_local() -> Result<Vec<T3CodeAccount>, String> {
    tokio::task::spawn_blocking(|| {
        let state_dir = default_state_dir()?;
        Ok(vec![import_from_state_dir(&state_dir, None)?])
    })
    .await
    .map_err(|error| format!("导入本机 T3 Code 账号失败: {}", error))?
}

pub fn import_token_or_clerk_json(
    raw_input: &str,
    account_name: Option<&str>,
) -> Result<T3CodeAccount, String> {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        return Err("请输入 T3 Code Clerk JWT 或 clerk-tokens.json 内容".to_string());
    }
    let (clerk_jwt, clerk_tokens_raw) = if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let jwt = value
            .get(CLERK_JWT_KEY)
            .or_else(|| value.get("clerk_jwt"))
            .or_else(|| value.get("token"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if jwt.is_empty() {
            return Err("JSON 中未包含 __clerk_client_jwt 字段".to_string());
        }
        (jwt, Some(value))
    } else {
        let jwt = trimmed.to_string();
        (jwt.clone(), Some(json!({ CLERK_JWT_KEY: jwt })))
    };

    let parsed = parse_clerk_jwt(&clerk_jwt);
    let token_fp = token_fingerprint(&clerk_jwt);
    let email = parsed
        .email
        .filter(|email| !is_placeholder_email(email))
        .unwrap_or_default();
    let user_id = parsed.subject.filter(|id| id.starts_with("user_"));
    let existing_accounts = list_accounts_checked().unwrap_or_default();
    let existing = find_matching_existing_account(&existing_accounts, None, token_fp.as_deref());
    let id = existing
        .as_ref()
        .map(|acc| acc.id.clone())
        .unwrap_or_else(|| account_id(None, None, token_fp.as_deref()));
    let now = now_ts();
    let account = T3CodeAccount {
        id,
        auth_mode: T3CodeAuthMode::Clerk,
        email: if let Some(known) = existing
            .as_ref()
            .filter(|acc| acc.identity_verified_at.is_some())
        {
            known.email.clone()
        } else if !email.is_empty() {
            email
        } else {
            existing
                .as_ref()
                .map(|acc| acc.email.clone())
                .filter(|e| !is_placeholder_email(e))
                .unwrap_or_default()
        },
        user_id: existing
            .as_ref()
            .and_then(|acc| acc.user_id.clone())
            .or(user_id),
        display_name: normalize_string(account_name)
            .or(parsed.name)
            .or_else(|| existing.as_ref().and_then(|acc| acc.display_name.clone())),
        avatar_url: existing.as_ref().and_then(|acc| acc.avatar_url.clone()),
        clerk_jwt,
        session_id: parsed
            .session_id
            .or_else(|| existing.as_ref().and_then(|acc| acc.session_id.clone())),
        environment_id: existing.as_ref().and_then(|acc| acc.environment_id.clone()),
        expires_at: parsed
            .expires_at
            .or_else(|| existing.as_ref().and_then(|acc| acc.expires_at)),
        identity_verified_at: existing.as_ref().and_then(|acc| acc.identity_verified_at),
        relay_verified_at: existing.as_ref().and_then(|acc| acc.relay_verified_at),
        t3chat_linked: existing.as_ref().and_then(|acc| acc.t3chat_linked),
        plan_type: existing
            .as_ref()
            .and_then(|acc| normalize_plan_type(acc.plan_type.as_deref())),
        quota_total: existing.as_ref().and_then(|acc| acc.quota_total),
        quota_used: existing.as_ref().and_then(|acc| acc.quota_used),
        quota_remaining: existing.as_ref().and_then(|acc| acc.quota_remaining),
        quota_reset_at: None,
        quota_query_last_error: existing
            .as_ref()
            .and_then(|acc| acc.quota_query_last_error.clone()),
        quota_query_last_error_at: existing
            .as_ref()
            .and_then(|acc| acc.quota_query_last_error_at),
        usage_updated_at: existing.as_ref().and_then(|acc| acc.relay_verified_at),
        tags: existing.and_then(|acc| acc.tags),
        clerk_tokens_raw,
        settings_snapshot_raw: None,
        created_at: now,
        last_used: now,
    };
    upsert_account(account)
}

fn import_account_backup(mut item: T3CodeAccount) -> Result<T3CodeAccount, String> {
    let _guard = T3CODE_ACCOUNT_LOCK
        .lock()
        .map_err(|_| "获取 T3 Code 账号锁失败".to_string())?;
    let existing = load_account(&item.id);
    let same_backup = existing.as_ref().filter(|account| {
        token_fingerprint(&item.clerk_jwt).is_some_and(|fingerprint| {
            token_fingerprint(&account.clerk_jwt).as_deref() == Some(&fingerprint)
        })
    });
    if existing.is_some() && same_backup.is_none() {
        let fingerprint = token_fingerprint(&item.clerk_jwt);
        item.id = account_id(None, None, fingerprint.as_deref());
    }
    item.identity_verified_at = same_backup.and_then(|account| account.identity_verified_at);
    item.relay_verified_at = same_backup.and_then(|account| account.relay_verified_at);
    item.t3chat_linked = same_backup.and_then(|account| account.t3chat_linked);
    item.usage_updated_at = item.relay_verified_at;
    item.quota_reset_at = None;
    item.quota_query_last_error =
        same_backup.and_then(|account| account.quota_query_last_error.clone());
    item.quota_query_last_error_at =
        same_backup.and_then(|account| account.quota_query_last_error_at);
    if let Some(account) = same_backup {
        item.email = account.email.clone();
        item.user_id = account.user_id.clone();
        item.plan_type = account.plan_type.clone();
        item.quota_used = account.quota_used;
        item.quota_remaining = account.quota_remaining;
        item.quota_total = account.quota_total;
    }
    upsert_account_locked(item)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<T3CodeAccount>, String> {
    let trimmed = json_content.trim();
    if trimmed.is_empty() {
        return Err("导入的 JSON 内容为空".to_string());
    }
    if let Ok(accounts) = serde_json::from_str::<Vec<T3CodeAccount>>(trimmed) {
        let mut saved = Vec::with_capacity(accounts.len());
        for item in accounts {
            saved.push(import_account_backup(item)?);
        }
        return Ok(saved);
    }
    if let Ok(single) = serde_json::from_str::<T3CodeAccount>(trimmed) {
        return Ok(vec![import_account_backup(single)?]);
    }
    let imported = import_token_or_clerk_json(trimmed, None)?;
    Ok(vec![imported])
}

pub fn import_from_file(path: &Path) -> Result<Vec<T3CodeAccount>, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("读取 T3 Code 导入文件失败: {}", error))?;
    import_from_json(&content)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let all = list_accounts_checked()?;
    let filter: HashSet<&str> = account_ids.iter().map(String::as_str).collect();
    let selected: Vec<T3CodeAccount> = if filter.is_empty() {
        all
    } else {
        all.into_iter()
            .filter(|item| filter.contains(item.id.as_str()))
            .collect()
    };
    serde_json::to_string_pretty(&selected)
        .map_err(|error| format!("导出 T3 Code 账号失败: {}", error))
}

#[cfg(unix)]
fn set_unix_permissions(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_unix_permissions(_path: &Path, _mode: u32) {}

pub fn inject_to_state_dir(account_id: &str, state_dir: &Path) -> Result<T3CodeAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "T3 Code 账号不存在".to_string())?;
    fs::create_dir_all(state_dir)
        .map_err(|error| format!("创建 T3 Code 状态目录失败: {}", error))?;
    set_unix_permissions(state_dir, 0o700);

    let clerk_dir = state_dir.join(CLERK_DIR_NAME);
    fs::create_dir_all(&clerk_dir)
        .map_err(|error| format!("创建 T3 Code Clerk 目录失败: {}", error))?;
    set_unix_permissions(&clerk_dir, 0o700);

    let token_path = clerk_tokens_path_for_state_dir(state_dir);
    if !account.clerk_jwt.trim().is_empty() || account.clerk_tokens_raw.is_some() {
        let payload = account
            .clerk_tokens_raw
            .clone()
            .unwrap_or_else(|| json!({ CLERK_JWT_KEY: account.clerk_jwt }));
        let content = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("序列化 T3 Code Clerk 凭据失败: {}", error))?;
        atomic_write::write_string_atomic(&token_path, &content)
            .map_err(|error| format!("写入 T3 Code Clerk 凭据失败: {}", error))?;
        set_unix_permissions(&token_path, 0o600);
    }

    account.last_used = now_ts();
    let saved = upsert_account(account)?;
    let mut index = load_index()?;
    index.current_account_id = Some(saved.id.clone());
    save_index(&index)?;
    Ok(saved)
}

pub fn inject_to_default(account_id: &str) -> Result<T3CodeAccount, String> {
    let state_dir = default_state_dir()?;
    inject_to_state_dir(account_id, &state_dir)
}

pub fn inject_to_instance_root(account_id: &str, root: &Path) -> Result<T3CodeAccount, String> {
    let state_dir = state_dir_for_instance_root(root);
    inject_to_state_dir(account_id, &state_dir)
}

fn find_live_state_for_account(
    account: &T3CodeAccount,
) -> Option<(PathBuf, String, ObservedClerkAccount)> {
    let map = OBSERVED_BY_STATE_DIR.lock().ok()?;
    map.iter().find_map(|(dir, entry)| {
        (entry.received_at.elapsed() < OBSERVATION_TTL
            && entry.clerk.signed_in
            && (entry.confirmed_account_id.as_deref() == Some(account.id.as_str())
                || (account.identity_verified_at.is_some()
                    && account
                        .user_id
                        .as_ref()
                        .is_some_and(|id| entry.clerk.user_id.as_ref() == Some(id)))))
        .then(|| (dir.clone(), entry.instance_id.clone(), entry.clerk.clone()))
    })
}

pub async fn refresh_account_quota(account_id: &str) -> Result<T3CodeAccount, String> {
    let id = account_id.to_owned();
    let (dir, instance, original) = tokio::task::spawn_blocking({
        let id = id.clone();
        move || {
            let account = load_account(&id).ok_or_else(|| "T3 Code 账号不存在".to_string())?;
            find_live_state_for_account(&account)
                .ok_or_else(|| clerk_sync_message("syncUnavailable"))
        }
    })
    .await
    .map_err(|error| error.to_string())??;
    let token = crate::modules::t3code_app_injection::request_clerk_refresh(&instance)
        .ok_or_else(|| clerk_sync_message("syncUnavailable"))?;
    for _ in 0..56 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let observed = observed_clerk_for_state_dir(&dir)
            .filter(|updated| same_live_identity(updated, &original))
            .ok_or_else(|| clerk_sync_message("syncUnavailable"))?;
        if observed.force_refresh_handled.as_deref() != Some(token.as_str()) {
            continue;
        }
        return tokio::task::spawn_blocking(move || {
            sync_observed_clerk_account(&dir, &observed)?;
            if observed.relay_failed_at.is_some() || observed.relay_verified_at.is_none() {
                return Err(clerk_sync_message("syncUnavailable"));
            }
            load_account(&id)
                .filter(|account| {
                    account.user_id == observed.user_id
                        && account.relay_verified_at == observed.relay_verified_at
                })
                .ok_or_else(|| clerk_sync_message("syncUnavailable"))
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    Err(clerk_sync_message("syncUnavailable"))
}

pub async fn refresh_all_accounts() -> Result<i32, String> {
    let accounts = tokio::task::spawn_blocking(list_accounts_checked)
        .await
        .map_err(|error| format!("读取 T3 Code 账号列表失败: {}", error))??;
    let mut count = 0;
    for item in accounts {
        if refresh_account_quota(&item.id).await.is_ok() {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_observation() -> ObservedClerkAccount {
        ObservedClerkAccount {
            signed_in: true,
            user_id: Some("user_fixture".to_string()),
            session_id: Some("sess_fixture".to_string()),
            email: Some("verified@example.test".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn clerk_observation_requires_same_live_user_and_session() {
        let live = live_observation();
        assert!(same_live_identity(&live, &live));
        assert!(!same_live_identity(
            &live,
            &ObservedClerkAccount {
                user_id: Some("user_other".to_string()),
                ..live.clone()
            }
        ));
        assert!(!same_live_identity(
            &live,
            &ObservedClerkAccount {
                session_id: Some("sess_other".to_string()),
                ..live.clone()
            }
        ));
        assert!(!same_live_identity(
            &live,
            &ObservedClerkAccount {
                signed_in: false,
                ..live.clone()
            }
        ));
    }

    #[test]
    fn clerk_snapshot_updates_identity_and_requires_complete_fresh_quota() {
        let mut account = serde_json::from_value::<T3CodeAccount>(json!({
            "id": "t3code_fixture",
            "email": "",
            "created_at": 1,
            "last_used": 1,
            "quota_reset_at": 200
        }))
        .unwrap();
        let mut live = live_observation();
        live.relay_verified_at = Some(100);
        live.tunnels_limit = Some(3.0);
        apply_clerk_snapshot(&mut account, &live);
        assert_eq!(account.email, "verified@example.test");
        assert!(account.identity_verified_at.is_some());
        assert_eq!(account.relay_verified_at, None);
        assert_eq!(account.quota_reset_at, None);

        live.tunnels_used = Some(0.0);
        live.plan_type = Some("free".to_string());
        live.t3chat_linked = Some(true);
        apply_clerk_snapshot(&mut account, &live);
        assert_eq!(account.relay_verified_at, Some(100));
        assert_eq!(account.usage_updated_at, Some(100));
        assert_eq!(account.quota_used, Some(0.0));
        assert_eq!(account.quota_total, Some(3.0));
        assert_eq!(account.plan_type.as_deref(), Some("free"));

        live.relay_verified_at = Some(99);
        live.tunnels_used = Some(1.0);
        apply_clerk_snapshot(&mut account, &live);
        assert_eq!(account.quota_used, Some(0.0));
        assert_eq!(account.relay_verified_at, Some(100));
    }

    #[test]
    fn parses_clerk_jwt_and_normalizes_placeholder_identity() {
        let claims = json!({
            "sub": "user_2t3code99",
            "email": "dev@t3tools.test",
            "sid": "sess_123",
            "exp": 1999999999i64
        });
        let encoded_claims = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{}.sig", encoded_claims);
        let parsed = parse_clerk_jwt(&fake_jwt);
        assert_eq!(parsed.subject.as_deref(), Some("user_2t3code99"));
        assert_eq!(parsed.email.as_deref(), Some("dev@t3tools.test"));
        assert_eq!(parsed.session_id.as_deref(), Some("sess_123"));
        assert_eq!(parsed.expires_at, Some(1999999999));

        assert!(is_placeholder_email(""));
        assert!(is_placeholder_email("clerk-1940abcd@t3code.cloud"));
        assert!(!is_placeholder_email("real.user@gmail.com"));
        assert_eq!(normalize_plan_type(Some("T3 Connect")), None);
        assert_eq!(normalize_plan_type(Some("FREE")).as_deref(), Some("FREE"));
        assert!(token_fingerprint("enc:djEwabc").is_some());
    }
}
