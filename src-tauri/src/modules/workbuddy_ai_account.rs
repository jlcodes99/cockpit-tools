use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{LazyLock, Mutex},
};

use chrono::{NaiveDate, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::models::workbuddy::WorkbuddyAccount;

use super::{atomic_write, secure_account_storage, workbuddy_account, workbuddy_auth_crypto};

const PLATFORM: &str = "workbuddy_ai";
const PENDING_ROLLOVER_AMBIGUITY_SECS: i64 = 30 * 60;
const PENDING_ROLLOVER_MAX_CLOCK_SKEW_SECS: i64 = 60;
static SWITCH: Mutex<()> = Mutex::new(());
static WRITE: Mutex<()> = Mutex::new(());

// Refresh tokens can rotate server-side. A second request with the same
// account's old token must not race the first request.
static REFRESHING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

pub struct RefreshGuard(String);

impl RefreshGuard {
    pub fn acquire(id: &str) -> Result<Self, String> {
        let mut pending = REFRESHING.lock().map_err(|error| error.to_string())?;
        if !pending.insert(id.to_owned()) {
            return Err("WORKBUDDY_AI_REFRESH_IN_PROGRESS".into());
        }
        Ok(Self(id.to_owned()))
    }
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = REFRESHING.lock() {
            pending.remove(&self.0);
        }
    }
}

fn directory() -> Result<PathBuf, String> {
    Ok(super::account::get_data_dir()?.join("workbuddy_ai_accounts"))
}

fn account_path(id: &str) -> Result<PathBuf, String> {
    let suffix = id
        .strip_prefix("workbuddy_ai_")
        .ok_or("WORKBUDDY_AI_INVALID_ID")?;
    if suffix.len() != 64 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("WORKBUDDY_AI_INVALID_ID".into());
    }
    Ok(directory()?.join(format!("{id}.json")))
}

pub(super) fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn load(id: &str) -> Result<WorkbuddyAccount, String> {
    let path = account_path(id)?;
    let content = read_optional(&path)?.ok_or("WORKBUDDY_AI_ACCOUNT_NOT_FOUND")?;
    secure_account_storage::deserialize_encrypted_snapshot(&content)
}

pub fn list() -> Result<Vec<WorkbuddyAccount>, String> {
    let entries = match fs::read_dir(directory()?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut accounts = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            if let Some(id) = path.file_stem().and_then(|id| id.to_str()) {
                accounts.push(load(id)?);
            }
        }
    }
    accounts.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(accounts)
}

fn write(account: &WorkbuddyAccount) -> Result<(), String> {
    let path = account_path(&account.id)?;
    let content = secure_account_storage::serialize_account_file(PLATFORM, account)?;
    atomic_write::write_secret_string_atomic_if_changed(&path, &content).map(|_| ())
}

fn identity(session: &Value) -> Result<String, String> {
    let account = &session["account"];
    let uid = account["uid"]
        .as_str()
        .filter(|uid| !uid.trim().is_empty())
        .ok_or("WORKBUDDY_AI_MISSING_ACCOUNT_ID")?;
    let enterprise = account["enterpriseId"].as_str().filter(|id| !id.is_empty());
    let identity = serde_json::to_vec(&(uid, enterprise)).map_err(|error| error.to_string())?;
    Ok(format!("{PLATFORM}_{:x}", Sha256::digest(identity)))
}

pub fn store_session(session: Value) -> Result<WorkbuddyAccount, String> {
    store_session_inner(session, None)
}

pub fn store_refreshed_session(
    previous: &WorkbuddyAccount,
    session: Value,
) -> Result<WorkbuddyAccount, String> {
    store_session_inner(session, Some(previous))
}

fn store_session_inner(
    session: Value,
    refreshing: Option<&WorkbuddyAccount>,
) -> Result<WorkbuddyAccount, String> {
    let id = identity(&session)?;
    let token = session["auth"]["accessToken"]
        .as_str()
        .filter(|token| !token.trim().is_empty())
        .ok_or("WORKBUDDY_AI_MISSING_ACCESS_TOKEN")?
        .to_string();
    if !session["accounts"].is_array() || !session["allAccounts"].is_array() {
        return Err("WORKBUDDY_AI_INVALID_SESSION".into());
    }
    let payload = workbuddy_account::build_local_import_payload(token, Some(session), None);
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    let path = account_path(&id)?;
    let previous = read_optional(&path)?
        .map(|content| {
            secure_account_storage::deserialize_encrypted_snapshot::<WorkbuddyAccount>(&content)
        })
        .transpose()?;
    if let Some(expected) = refreshing {
        if id != expected.id
            || !previous
                .as_ref()
                .is_some_and(|current| current.auth_raw == expected.auth_raw)
        {
            return Err("WORKBUDDY_AI_ACCOUNT_CHANGED".into());
        }
    }
    let now = chrono::Utc::now().timestamp();
    let mut account: WorkbuddyAccount = serde_json::from_value(serde_json::json!({
        "id": id,
        "email": payload.email,
        "access_token": payload.access_token,
        "created_at": previous.as_ref().map_or(now, |value| value.created_at),
        "last_used": now,
    }))
    .map_err(|error| error.to_string())?;
    account.uid = payload.uid;
    account.nickname = payload.nickname;
    account.enterprise_id = payload.enterprise_id;
    account.enterprise_name = payload.enterprise_name;
    account.refresh_token = payload.refresh_token;
    account.token_type = payload.token_type;
    account.expires_at = payload.expires_at;
    account.domain = payload.domain;
    account.plan_type = payload.profile_raw.as_ref().and_then(|raw| {
        raw["editionType"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    });
    account.auth_raw = payload.auth_raw;
    account.profile_raw = payload.profile_raw;
    account.status = payload.status;
    if let Some(previous) = previous {
        account.tags = previous.tags;
        account.last_switched_at = previous.last_switched_at;
        account.quota_raw = previous.quota_raw;
        account.quota_query_last_error = previous.quota_query_last_error;
        account.quota_query_last_error_at = previous.quota_query_last_error_at;
        account.usage_updated_at = previous.usage_updated_at;
        account.last_checkin_time = previous.last_checkin_time;
        account.checkin_streak = previous.checkin_streak;
        account.checkin_rewards = previous.checkin_rewards;
    }
    write(&account)?;
    Ok(account)
}

pub fn update_quota(
    id: &str,
    access_token: &str,
    result: Result<Value, String>,
) -> Result<WorkbuddyAccount, String> {
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    let mut account = load(id)?;
    if account.access_token != access_token {
        return Ok(account);
    }
    match result {
        Ok(value) => {
            account.quota_raw = Some(serde_json::json!({"userResource": value}));
            account.usage_updated_at = Some(chrono::Utc::now().timestamp_millis());
            account.quota_query_last_error = None;
            account.quota_query_last_error_at = None;
        }
        Err(error) => {
            account.quota_query_last_error = Some(error);
            account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
        }
    }
    write(&account)?;
    Ok(account)
}

pub fn update_tags(id: &str, tags: Vec<String>) -> Result<WorkbuddyAccount, String> {
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    let mut account = load(id)?;
    account.tags = Some(tags);
    write(&account)?;
    Ok(account)
}

pub(crate) fn is_adjacent_next_day(prev_date: &str, next_date: &str) -> bool {
    let Ok(prev) = NaiveDate::parse_from_str(prev_date, "%Y-%m-%d") else {
        return false;
    };
    let Ok(next) = NaiveDate::parse_from_str(next_date, "%Y-%m-%d") else {
        return false;
    };
    prev.succ_opt() == Some(next)
}

pub fn has_pending_checkin_turn(account: &WorkbuddyAccount, activity_date: &str) -> bool {
    account
        .checkin_rewards
        .as_ref()
        .and_then(|v| v.get("pendingActivityDate"))
        .and_then(Value::as_str)
        .is_some_and(|date| date == activity_date)
}

/// Checks whether the account has an unresolved pending chat turn that either matches `today_date`
/// directly OR was reserved for the immediately preceding server activity date (`today_date - 1d`)
/// within the rollover ambiguity window while that preceding date is still absent from
/// `active_dates` (meaning an in-flight chat POST may have crossed midnight into `today_date`).
pub fn has_pending_checkin_turn_for_heatmap(
    account: &WorkbuddyAccount,
    today_date: &str,
    active_dates: &[String],
) -> bool {
    if has_pending_checkin_turn(account, today_date) {
        return true;
    }
    let Some(rewards) = account.checkin_rewards.as_ref().and_then(Value::as_object) else {
        return false;
    };
    let Some(pending_date) = rewards.get("pendingActivityDate").and_then(Value::as_str) else {
        return false;
    };
    if !is_adjacent_next_day(pending_date, today_date) {
        return false;
    }
    if active_dates.iter().any(|d| d == pending_date) {
        return false;
    }
    rewards
        .get("pendingCreatedAt")
        .and_then(Value::as_i64)
        .is_some_and(|created_at| {
            let now = Utc::now().timestamp();
            created_at <= now.saturating_add(PENDING_ROLLOVER_MAX_CLOCK_SKEW_SECS)
                && now.saturating_sub(created_at) <= PENDING_ROLLOVER_AMBIGUITY_SECS
        })
}

pub fn has_confirmed_checkin_date(account: &WorkbuddyAccount, activity_date: &str) -> bool {
    account
        .checkin_rewards
        .as_ref()
        .and_then(|v| v.get("activityDate"))
        .and_then(Value::as_str)
        .is_some_and(|date| date == activity_date)
}

pub fn has_recorded_checkin_for_heatmap(
    account: &WorkbuddyAccount,
    today_date: &str,
    active_dates: &[String],
) -> bool {
    has_confirmed_checkin_date(account, today_date)
        || has_pending_checkin_turn_for_heatmap(account, today_date, active_dates)
}

pub(crate) fn apply_pending_checkin_marker(
    account: &mut WorkbuddyAccount,
    activity_date: &str,
    model_id: &str,
) {
    let mut rewards = account
        .checkin_rewards
        .take()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    rewards.insert(
        "pendingActivityDate".to_string(),
        Value::String(activity_date.to_string()),
    );
    rewards.insert(
        "pendingModelId".to_string(),
        Value::String(model_id.to_string()),
    );
    rewards.insert(
        "pendingCreatedAt".to_string(),
        Value::Number(Utc::now().timestamp().into()),
    );
    account.checkin_rewards = Some(Value::Object(rewards));
}

pub(crate) fn clear_pending_checkin_marker(account: &mut WorkbuddyAccount, activity_date: &str) {
    let Some(rewards) = account
        .checkin_rewards
        .as_mut()
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let matches_date = rewards
        .get("pendingActivityDate")
        .and_then(Value::as_str)
        .is_some_and(|d| d == activity_date);
    if matches_date {
        rewards.remove("pendingActivityDate");
        rewards.remove("pendingModelId");
        rewards.remove("pendingCreatedAt");
    }
}

pub fn mark_pending_checkin_turn(
    id: &str,
    activity_date: &str,
    model_id: &str,
) -> Result<WorkbuddyAccount, String> {
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    let mut account = load(id)?;
    apply_pending_checkin_marker(&mut account, activity_date, model_id);
    write(&account)?;
    Ok(account)
}

pub fn clear_pending_checkin_turn(
    id: &str,
    activity_date: &str,
) -> Result<WorkbuddyAccount, String> {
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    let mut account = load(id)?;
    clear_pending_checkin_marker(&mut account, activity_date);
    write(&account)?;
    Ok(account)
}

#[cfg(test)]
pub(crate) fn write_snapshot_to_path(
    path: &Path,
    account: &WorkbuddyAccount,
) -> Result<(), String> {
    let content = serde_json::to_string(account).map_err(|error| error.to_string())?;
    atomic_write::write_secret_string_atomic_if_changed(path, &content).map(|_| ())
}

#[cfg(test)]
pub(crate) fn load_snapshot_from_path(path: &Path) -> Result<WorkbuddyAccount, String> {
    let content = read_optional(path)?.ok_or("WORKBUDDY_AI_ACCOUNT_NOT_FOUND")?;
    serde_json::from_str(&content).map_err(|error| error.to_string())
}

pub(crate) fn apply_checkin_info_update(
    account: &mut WorkbuddyAccount,
    last_checkin_time: Option<i64>,
    checkin_streak: Option<i32>,
    checkin_rewards: Option<Value>,
) {
    if let Some(ts) = last_checkin_time {
        account.last_checkin_time = Some(ts);
    }
    if let Some(streak) = checkin_streak {
        account.checkin_streak = Some(streak);
    }
    if let Some(mut incoming) = checkin_rewards {
        if let (Some(prev), Some(next)) = (
            account.checkin_rewards.as_ref().and_then(Value::as_object),
            incoming.as_object_mut(),
        ) {
            let incoming_date = next
                .get("activityDate")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let same_confirmed_day = prev
                .get("activityDate")
                .and_then(Value::as_str)
                .is_some_and(|d| incoming_date.as_deref() == Some(d));
            let matching_pending_day = prev
                .get("pendingActivityDate")
                .and_then(Value::as_str)
                .zip(incoming_date.as_deref())
                .is_some_and(|(pending_date, inc)| {
                    pending_date == inc || is_adjacent_next_day(pending_date, inc)
                });

            let incoming_model_missing = next
                .get("modelId")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty);

            if incoming_model_missing {
                let fallback_model = if same_confirmed_day {
                    prev.get("modelId")
                        .filter(|v| v.as_str().map(str::trim).is_some_and(|s| !s.is_empty()))
                        .or_else(|| {
                            if matching_pending_day {
                                prev.get("pendingModelId").filter(|v| {
                                    v.as_str().map(str::trim).is_some_and(|s| !s.is_empty())
                                })
                            } else {
                                None
                            }
                        })
                } else if matching_pending_day {
                    // Pending confirmation for a new day (including midnight rollover D -> D+1)
                    // only inherits the matching pendingModelId; never copy yesterday's confirmed modelId.
                    prev.get("pendingModelId")
                        .filter(|v| v.as_str().map(str::trim).is_some_and(|s| !s.is_empty()))
                } else {
                    None
                };
                if let Some(prev_model) = fallback_model {
                    next.insert("modelId".to_string(), prev_model.clone());
                }
            }

            // Clear pending marker once confirmed; preserve only if a newer date is pending.
            let preserve_newer_pending = prev
                .get("pendingActivityDate")
                .and_then(Value::as_str)
                .zip(incoming_date.as_deref())
                .is_some_and(|(pending, confirmed)| pending > confirmed);
            if preserve_newer_pending {
                if let Some(p_date) = prev.get("pendingActivityDate") {
                    next.insert("pendingActivityDate".to_string(), p_date.clone());
                }
                if let Some(p_model) = prev.get("pendingModelId") {
                    next.insert("pendingModelId".to_string(), p_model.clone());
                }
                if let Some(p_created) = prev.get("pendingCreatedAt") {
                    next.insert("pendingCreatedAt".to_string(), p_created.clone());
                }
            } else {
                next.remove("pendingActivityDate");
                next.remove("pendingModelId");
                next.remove("pendingCreatedAt");
            }
        }
        account.checkin_rewards = Some(incoming);
    }
}

pub fn update_checkin_info(
    id: &str,
    last_checkin_time: Option<i64>,
    checkin_streak: Option<i32>,
    checkin_rewards: Option<Value>,
) -> Result<WorkbuddyAccount, String> {
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    let mut account = load(id)?;
    apply_checkin_info_update(
        &mut account,
        last_checkin_time,
        checkin_streak,
        checkin_rewards,
    );
    write(&account)?;
    Ok(account)
}

pub fn delete(ids: Vec<String>) -> Result<(), String> {
    let paths = ids
        .iter()
        .map(|id| account_path(id))
        .collect::<Result<Vec<_>, _>>()?;
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    for path in paths {
        atomic_write::remove_file_locked(&path)?;
    }
    Ok(())
}

pub fn import_json(content: &str) -> Result<Vec<WorkbuddyAccount>, String> {
    let value: Value = serde_json::from_str(content).map_err(|error| error.to_string())?;
    let items = match value {
        Value::Array(items) => items,
        item => vec![item],
    };
    let accounts = items
        .into_iter()
        .map(|value| {
            let account: WorkbuddyAccount =
                serde_json::from_value(value).map_err(|error| error.to_string())?;
            let session = account
                .auth_raw
                .as_ref()
                .ok_or("WORKBUDDY_AI_MISSING_SESSION")?;
            if identity(session)? != account.id
                || session["auth"]["accessToken"].as_str() != Some(account.access_token.as_str())
            {
                return Err("WORKBUDDY_AI_INVALID_BACKUP".into());
            }
            Ok(account)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    for account in &accounts {
        write(account)?;
    }
    Ok(accounts)
}

fn auth_path() -> Result<PathBuf, String> {
    workbuddy_account::get_workbuddy_shared_auth_dir()
        .map(|dir| dir.join("workbuddy-desktop-ai.info"))
        .ok_or_else(|| "WORKBUDDY_AI_AUTH_PATH_UNAVAILABLE".into())
}

fn decode(raw: &str, prepare: bool) -> Result<Value, String> {
    let value: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    if prepare && workbuddy_auth_crypto::contains_encrypted_wrapper(&value) {
        workbuddy_auth_crypto::prepare_international()?;
    }
    workbuddy_auth_crypto::decode_for_edition(&value, true)
}

pub fn current_id() -> Result<Option<String>, String> {
    let path = auth_path()?;
    if workbuddy_account::workbuddy_logout_marker_path(&path).exists() {
        return Ok(None);
    }
    let Some(raw) = read_optional(&path)? else {
        return Ok(None);
    };
    // Identity fields are clear even when the credential fields are encrypted.
    let session = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    let id = identity(&session)?;
    Ok(account_path(&id)?.is_file().then_some(id))
}

pub fn import_local() -> Result<WorkbuddyAccount, String> {
    let path = auth_path()?;
    if workbuddy_account::workbuddy_logout_marker_path(&path).exists() {
        return Err("WORKBUDDY_AI_NO_LOCAL_LOGIN".into());
    }
    let raw = read_optional(&path)?.ok_or("WORKBUDDY_AI_NO_LOCAL_LOGIN")?;
    let session = decode(&raw, true)?;
    if workbuddy_account::workbuddy_logout_marker_path(&path).exists() {
        return Err("WORKBUDDY_AI_NO_LOCAL_LOGIN".into());
    }
    store_session(session)
}

pub(crate) const MISSING_APP_PATH: &str = "APP_PATH_NOT_FOUND:workbuddy_ai";
pub(crate) const SWITCHED_MISSING_APP_PATH: &str = "APP_PATH_MISSING_AFTER_SWITCH:workbuddy_ai";

pub(crate) fn executable_at(path: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let bundle = if path.is_dir() {
            path
        } else {
            path.ancestors().nth(3)?
        };
        let executable = bundle.join("Contents/MacOS/Electron");
        if !executable.is_file() || (path != bundle && path != executable) {
            return None;
        }
        let info = plist::Value::from_file(bundle.join("Contents/Info.plist"))
            .ok()?
            .into_dictionary()?;
        return (info.get("CFBundleIdentifier")?.as_string()? == "com.workbuddy.workbuddy-ai")
            .then_some(executable);
    }
    #[cfg(target_os = "windows")]
    {
        let executable = if path.is_dir() {
            path.join("WorkBuddyAI.exe")
        } else {
            path.to_owned()
        };
        return (executable.is_file()
            && executable
                .file_name()?
                .to_str()?
                .eq_ignore_ascii_case("WorkBuddyAI.exe"))
        .then_some(executable);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        None
    }
}

pub(crate) fn detect_executable() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let system = PathBuf::from("/Applications/WorkBuddy AI.app");
        let user = dirs::home_dir().map(|home| home.join("Applications/WorkBuddy AI.app"));
        return std::iter::once(system)
            .chain(user)
            .find_map(|path| executable_at(&path));
    }
    #[cfg(target_os = "windows")]
    {
        let local =
            std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Programs"));
        let system = ["ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(std::env::var_os);
        return local
            .into_iter()
            .chain(system.map(PathBuf::from))
            .flat_map(|root| ["WorkBuddy AI", "workbuddy-ai"].map(|name| root.join(name)))
            .find_map(|path| executable_at(&path));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    None
}

pub(super) fn resolve_executable() -> Result<PathBuf, String> {
    let config = super::config::get_user_config();
    let path = config.workbuddy_ai_app_path.trim();
    if !path.is_empty() {
        return executable_at(Path::new(path)).ok_or_else(|| MISSING_APP_PATH.into());
    }
    detect_executable().ok_or_else(|| MISSING_APP_PATH.into())
}

pub fn launch_current() -> Result<(), String> {
    let _switch = SWITCH
        .try_lock()
        .map_err(|_| "WORKBUDDY_AI_SWITCH_IN_PROGRESS")?;
    launch(&resolve_executable()?)
}

#[cfg(target_os = "macos")]
fn stop(executable: &Path) -> Result<(), String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    for (pid, process) in system.processes() {
        if process.exe() == Some(executable)
            && !process.cmd().iter().skip(1).any(|arg| {
                let arg = arg.to_string_lossy();
                arg.ends_with("/daemon-app-server-entry.js")
                    || arg.contains("/app.asar.unpacked/cli/bin/")
            })
            && !process
                .cmd()
                .iter()
                .any(|arg| arg.to_string_lossy().starts_with("--type="))
        {
            super::process::close_pid(pid.as_u32(), 10)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn stop(executable: &Path) -> Result<(), String> {
    super::process::close_processes_by_exact_exe_paths(&[executable.to_path_buf()], 10).map(|_| ())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn stop(_executable: &Path) -> Result<(), String> {
    Err("WORKBUDDY_AI_APP_NOT_FOUND".into())
}

fn launch(executable: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bundle = executable
            .ancestors()
            .nth(3)
            .ok_or("WORKBUDDY_AI_APP_NOT_FOUND")?;
        let status = Command::new("/usr/bin/open")
            .arg("-a")
            .arg(bundle)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err("WORKBUDDY_AI_LAUNCH_FAILED".into());
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new(executable)
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = executable;
        return Err("WORKBUDDY_AI_APP_NOT_FOUND".into());
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    Ok(())
}

struct ClientWriteLock(PathBuf);

impl ClientWriteLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        let parent = path.parent().ok_or("WORKBUDDY_AI_AUTH_PATH_UNAVAILABLE")?;
        let basename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("WORKBUDDY_AI_AUTH_PATH_UNAVAILABLE")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let lock = parent.join(format!(".{basename}.workbuddy.lock"));
        fs::create_dir(&lock).map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                "WORKBUDDY_AI_CLIENT_SESSION_BUSY".to_string()
            } else {
                error.to_string()
            }
        })?;
        Ok(Self(lock))
    }
}

impl Drop for ClientWriteLock {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir(&self.0) {
            super::logger::log_warn(&format!(
                "[WorkBuddy AI] Could not release login file lock: {error}"
            ));
        }
    }
}

fn write_client_session(path: &Path, session: &Value) -> Result<(), String> {
    let _client_lock = ClientWriteLock::acquire(path)?;
    let old = read_optional(path)?;
    let marker = workbuddy_account::workbuddy_logout_marker_path(path);
    let previous_marker = read_optional(&marker)?;
    let encrypt = if let Some(raw) = old.as_deref() {
        let previous: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
        let encrypted = workbuddy_auth_crypto::contains_encrypted_wrapper(&previous);
        if encrypted {
            decode(raw, true)?;
        }
        encrypted
    } else {
        false
    };
    let written = if encrypt {
        workbuddy_auth_crypto::encode_for_edition(session, true)?
    } else {
        session.clone()
    };
    let content = serde_json::to_string_pretty(&written).map_err(|error| error.to_string())?;
    if !atomic_write::replace_private_string_if_matches(path, old.as_deref(), Some(&content))? {
        return Err("WORKBUDDY_AI_CLIENT_SESSION_CHANGED".into());
    }
    let clear_marker =
        atomic_write::replace_string_if_matches(&marker, previous_marker.as_deref(), None);
    if !matches!(clear_marker, Ok(true)) {
        atomic_write::replace_private_string_if_matches(path, Some(&content), old.as_deref())?;
        clear_marker?;
        return Err("WORKBUDDY_AI_CLIENT_SESSION_CHANGED".into());
    }
    if read_optional(path)?.as_deref() != Some(content.as_str()) || marker.exists() {
        return Err("WORKBUDDY_AI_CLIENT_SESSION_CHANGED".into());
    }
    Ok(())
}

pub fn switch(id: &str) -> Result<(), String> {
    let _switch = SWITCH
        .try_lock()
        .map_err(|_| "WORKBUDDY_AI_SWITCH_IN_PROGRESS")?;
    let executable = resolve_executable()?;
    let path = auth_path()?;
    load(id)?;
    stop(&executable)?;
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    let account = load(id)?;
    let session = account
        .auth_raw
        .as_ref()
        .ok_or("WORKBUDDY_AI_MISSING_SESSION")?;
    let session = session.clone();
    drop(_guard);
    write_client_session(&path, &session)?;
    let _guard = WRITE.lock().map_err(|error| error.to_string())?;
    if let Ok(mut account) = load(id) {
        account.last_switched_at = Some(chrono::Utc::now().timestamp());
        if let Err(error) = write(&account) {
            super::logger::log_warn(&format!(
                "[WorkBuddy AI] Could not record account switch: {error}"
            ));
        }
    }
    drop(_guard);
    if !executable.is_file() {
        return Err(SWITCHED_MISSING_APP_PATH.into());
    }
    launch(&executable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(uid: &str, enterprise_id: Option<&str>, token: &str) -> Value {
        let account = json!({"uid": uid, "enterpriseId": enterprise_id, "nickname": "Fixture"});
        json!({
            "account": account,
            "accounts": [account],
            "allAccounts": [account],
            "auth": {"accessToken": token, "refreshToken": "fixture-refresh"},
        })
    }

    #[test]
    fn identity_is_stable_across_refresh_and_separates_tenants() {
        let personal = identity(&session("alice", None, "old")).unwrap();
        assert_eq!(
            personal,
            identity(&session("alice", Some(""), "new")).unwrap()
        );
        assert_ne!(
            personal,
            identity(&session("alice", Some("team"), "old")).unwrap()
        );
        assert_ne!(personal, identity(&session("bob", None, "old")).unwrap());
        assert!(account_path("workbuddy_domestic").is_err());
        assert!(account_path("workbuddy_ai_../domestic").is_err());
    }

    #[test]
    fn switching_preserves_official_encryption_and_clears_the_logout_marker() {
        workbuddy_auth_crypto::with_test_key(|| {
            let directory = tempfile::tempdir().unwrap();
            let international = directory.path().join("workbuddy-desktop-ai.info");
            let domestic = directory.path().join("workbuddy-desktop.info");
            let previous = session("alice", None, "old");
            let target = session("bob", None, "new");
            fs::write(&domestic, "domestic fixture").unwrap();
            fs::write(
                &international,
                workbuddy_auth_crypto::encode_for_edition(&previous, true)
                    .unwrap()
                    .to_string(),
            )
            .unwrap();
            let marker = workbuddy_account::workbuddy_logout_marker_path(&international);
            fs::write(&marker, "logged out").unwrap();

            write_client_session(&international, &target).unwrap();

            let written = fs::read_to_string(&international).unwrap();
            assert!(workbuddy_auth_crypto::contains_encrypted_wrapper(
                &serde_json::from_str(&written).unwrap()
            ));
            assert_eq!(decode(&written, false).unwrap(), target);
            assert!(!marker.exists());
            assert_eq!(fs::read_to_string(domestic).unwrap(), "domestic fixture");
            assert!(!directory
                .path()
                .join("workbuddy-desktop-ai.info.bak")
                .exists());
        });
    }

    #[test]
    fn a_client_write_lock_keeps_the_session_intact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workbuddy-desktop-ai.info");
        let old = session("alice", None, "old").to_string();
        fs::write(&path, &old).unwrap();
        let lock = ClientWriteLock::acquire(&path).unwrap();
        let new = session("bob", None, "new");

        assert_eq!(
            write_client_session(&path, &new).unwrap_err(),
            "WORKBUDDY_AI_CLIENT_SESSION_BUSY"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), old);
        drop(lock);
        write_client_session(&path, &new).unwrap();
        assert_eq!(
            decode(&fs::read_to_string(&path).unwrap(), false).unwrap(),
            new
        );
    }

    #[test]
    fn invalid_existing_session_stays_intact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workbuddy-desktop-ai.info");
        fs::write(&path, "broken fixture").unwrap();
        assert!(write_client_session(&path, &session("alice", None, "token")).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "broken fixture");
    }

    #[test]
    fn checkin_info_update_preserves_streak_and_same_day_model() {
        let mut account: WorkbuddyAccount = serde_json::from_value(json!({
            "id": "workbuddy_ai_0000000000000000000000000000000000000000000000000000000000000001",
            "email": "tester@example.com",
            "access_token": "tok",
            "created_at": 1,
            "last_used": 1,
            "checkin_streak": 12,
            "checkin_rewards": {
                "activityDate": "2026-11-24",
                "modelId": "hy4flash-free"
            }
        }))
        .unwrap();

        apply_checkin_info_update(
            &mut account,
            Some(1_700_000_123),
            None,
            Some(json!({
                "activityDate": "2026-11-24",
                "modelId": null
            })),
        );

        assert_eq!(account.checkin_streak, Some(12));
        let rewards = account.checkin_rewards.unwrap();
        assert_eq!(rewards["modelId"], "hy4flash-free");
    }

    #[test]
    fn pending_same_day_confirmation_does_not_inherit_yesterday_model() {
        let mut account: WorkbuddyAccount = serde_json::from_value(json!({
            "id": "workbuddy_ai_0000000000000000000000000000000000000000000000000000000000000001",
            "email": "tester@example.com",
            "access_token": "tok",
            "created_at": 1,
            "last_used": 1,
            "checkin_streak": 7,
            "checkin_rewards": {
                "activityDate": "2026-11-23",
                "modelId": "yesterday-model"
            }
        }))
        .unwrap();

        apply_pending_checkin_marker(&mut account, "2026-11-24", "today-free-model");
        assert!(has_pending_checkin_turn(&account, "2026-11-24"));
        assert!(has_recorded_checkin_for_heatmap(
            &account,
            "2026-11-24",
            &[]
        ));

        // Confirm today with modelId=None:
        // must inherit today's pendingModelId, NOT yesterday's modelId.
        apply_checkin_info_update(
            &mut account,
            Some(1_700_000_200),
            Some(8),
            Some(json!({
                "activityDate": "2026-11-24",
                "modelId": null
            })),
        );

        let rewards = account.checkin_rewards.as_ref().unwrap();
        assert_eq!(rewards["activityDate"], "2026-11-24");
        assert_eq!(rewards["modelId"], "today-free-model");
        assert!(rewards.get("pendingActivityDate").is_none());
        assert!(rewards.get("pendingModelId").is_none());
        assert!(rewards.get("pendingCreatedAt").is_none());
        assert!(has_confirmed_checkin_date(&account, "2026-11-24"));
        assert!(has_recorded_checkin_for_heatmap(
            &account,
            "2026-11-24",
            &[]
        ));
    }

    #[test]
    fn rollover_confirmation_promotes_adjacent_pending_model_and_ignores_stale_gap() {
        let mut account: WorkbuddyAccount = serde_json::from_value(json!({
            "id": "workbuddy_ai_0000000000000000000000000000000000000000000000000000000000000001",
            "email": "tester@example.com",
            "access_token": "tok",
            "created_at": 1,
            "last_used": 1,
            "checkin_streak": 4,
            "checkin_rewards": {
                "activityDate": "2026-11-23",
                "modelId": "prior-confirmed-model"
            }
        }))
        .unwrap();

        apply_pending_checkin_marker(&mut account, "2026-11-24", "hy4flash-free");
        apply_checkin_info_update(
            &mut account,
            Some(1_700_000_900),
            Some(5),
            Some(json!({
                "activityDate": "2026-11-25",
                "modelId": null
            })),
        );

        let rewards = account.checkin_rewards.as_ref().unwrap();
        assert_eq!(rewards["activityDate"], "2026-11-25");
        assert_eq!(rewards["modelId"], "hy4flash-free");
        assert!(rewards.get("pendingActivityDate").is_none());
        assert!(rewards.get("pendingModelId").is_none());
        assert!(rewards.get("pendingCreatedAt").is_none());

        // Non-adjacent confirmation (2 days later) must not inherit an unrelated pendingModelId.
        apply_pending_checkin_marker(&mut account, "2026-11-25", "day25-model");
        apply_checkin_info_update(
            &mut account,
            Some(1_700_100_000),
            Some(6),
            Some(json!({
                "activityDate": "2026-11-27",
                "modelId": null
            })),
        );
        let rewards = account.checkin_rewards.as_ref().unwrap();
        assert_eq!(rewards["activityDate"], "2026-11-27");
        assert!(rewards["modelId"].is_null());
        assert!(rewards.get("pendingActivityDate").is_none());
    }

    #[test]
    fn pending_rollover_window_handles_extreme_timestamps_without_overflow() {
        let mut account: WorkbuddyAccount = serde_json::from_value(json!({
            "id": "workbuddy_ai_0000000000000000000000000000000000000000000000000000000000000001",
            "email": "tester@example.com",
            "access_token": "tok",
            "created_at": 1,
            "last_used": 1,
        }))
        .unwrap();
        let now = Utc::now().timestamp();

        for (ts, expected) in [
            (i64::MIN, false),
            (i64::MAX, false),
            (now + 120, false),
            (now + 30, true),
            (now - 60, true),
            (now - PENDING_ROLLOVER_AMBIGUITY_SECS - 1, false),
        ] {
            account.checkin_rewards = Some(json!({
                "pendingActivityDate": "2026-11-24",
                "pendingModelId": "hy4flash-free",
                "pendingCreatedAt": ts,
            }));
            assert_eq!(
                has_pending_checkin_turn_for_heatmap(&account, "2026-11-25", &[]),
                expected,
                "unexpected result for pendingCreatedAt={ts}"
            );
        }
    }
}
