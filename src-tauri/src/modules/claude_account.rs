use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

use crate::models::claude::{ClaudeAccount, ClaudeAccountIndex};

const ACCOUNTS_INDEX_FILE: &str = "claude_accounts.json";
const ACCOUNTS_DIR: &str = "claude_accounts";
const PROFILES_DIR: &str = "claude_profiles";

static CLAUDE_ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

fn now_ts() -> i64 {
    Utc::now().timestamp()
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

fn normalize_bool_flag(value: Option<bool>, default_value: bool) -> bool {
    value.unwrap_or(default_value)
}

fn normalize_tags(tags: &[String]) -> Option<Vec<String>> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();
        if seen.insert(lower) {
            normalized.push(trimmed.to_string());
        }
    }
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
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

fn normalize_login_mode(
    value: Option<&str>,
    login_hint_email: Option<&str>,
    anthropic_base_url: Option<&str>,
    anthropic_auth_token: Option<&str>,
) -> Result<String, String> {
    let normalized = value
        .map(|raw| raw.trim().to_ascii_lowercase())
        .filter(|raw| !raw.is_empty())
        .unwrap_or_else(|| "claudeai".to_string());

    match normalized.as_str() {
        "claudeai" | "console" | "sso" => Ok(normalized),
        "email" => {
            if normalize_non_empty(login_hint_email).is_none() {
                return Err("选择 email 登录模式时必须填写邮箱提示".to_string());
            }
            Ok(normalized)
        }
        "auth_token" => {
            if normalize_non_empty(anthropic_base_url).is_none() {
                return Err("选择 Auth Token 模式时必须填写 ANTHROPIC_BASE_URL".to_string());
            }
            if normalize_non_empty(anthropic_auth_token).is_none() {
                return Err("选择 Auth Token 模式时必须填写 ANTHROPIC_AUTH_TOKEN".to_string());
            }
            Ok(normalized)
        }
        _ => Err(format!("不支持的 Claude 登录模式: {}", normalized)),
    }
}

fn normalize_status_string(value: Option<&str>) -> Option<String> {
    let normalized = normalize_non_empty(value)?;
    if normalized.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(normalized)
    }
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let dir = crate::modules::account::get_data_dir()?.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Claude 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_profiles_root_dir() -> Result<PathBuf, String> {
    let dir = crate::modules::account::get_data_dir()?.join(PROFILES_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Claude profile 目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(get_accounts_index_path()?.to_string_lossy().to_string())
}

fn resolve_account_file_path(account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    Ok(get_accounts_dir()?.join(format!("{}.json", normalized)))
}

fn save_account_file(account: &ClaudeAccount) -> Result<(), String> {
    let path = resolve_account_file_path(&account.id)?;
    let content = serde_json::to_string_pretty(account)
        .map_err(|e| format!("序列化 Claude 账号失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存 Claude 账号失败: path={}, error={}", path.display(), e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| {
            format!(
                "删除 Claude 账号文件失败: path={}, error={}",
                path.display(),
                e
            )
        })?;
    }
    Ok(())
}

fn load_account_index_checked() -> Result<ClaudeAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        return Ok(ClaudeAccountIndex::new());
    }
    let content = fs::read_to_string(&path).map_err(|e| {
        format!(
            "读取 Claude 账号索引失败: path={}, error={}",
            path.display(),
            e
        )
    })?;
    if content.trim().is_empty() {
        return Ok(ClaudeAccountIndex::new());
    }
    crate::modules::atomic_write::parse_json_with_auto_restore::<ClaudeAccountIndex>(
        &path, &content,
    )
    .map_err(|e| {
        format!(
            "解析 Claude 账号索引失败: path={}, error={}",
            path.display(),
            e
        )
    })
}

fn save_account_index(index: &ClaudeAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content = serde_json::to_string_pretty(index)
        .map_err(|e| format!("序列化 Claude 账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content).map_err(|e| {
        format!(
            "保存 Claude 账号索引失败: path={}, error={}",
            path.display(),
            e
        )
    })
}

fn sort_accounts(accounts: &mut [ClaudeAccount]) {
    accounts.sort_by(|left, right| {
        right
            .last_used
            .cmp(&left.last_used)
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn save_index_from_accounts(accounts: &[ClaudeAccount]) -> Result<(), String> {
    let mut summaries: Vec<_> = accounts.iter().map(|account| account.summary()).collect();
    summaries.sort_by(|left, right| {
        right
            .last_used
            .cmp(&left.last_used)
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    save_account_index(&ClaudeAccountIndex {
        version: "1.0".to_string(),
        accounts: summaries,
    })
}

fn collect_account_ids_from_details() -> Result<Vec<String>, String> {
    let dir = get_accounts_dir()?;
    let mut ids = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| {
        format!(
            "读取 Claude 账号详情目录失败: path={}, error={}",
            dir.display(),
            e
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取 Claude 账号详情项失败: {}", e))?;
        let path = entry.path();
        if path.extension() != Some(OsStr::new("json")) {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
            ids.push(stem.to_string());
        }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub fn load_account(account_id: &str) -> Option<ClaudeAccount> {
    let path = resolve_account_file_path(account_id).ok()?;
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore::<ClaudeAccount>(&path, &content)
        .ok()
}

pub fn load_account_checked(account_id: &str) -> Result<ClaudeAccount, String> {
    load_account(account_id).ok_or_else(|| format!("Claude 账号不存在: {}", account_id))
}

pub fn list_accounts_checked() -> Result<Vec<ClaudeAccount>, String> {
    let index = load_account_index_checked()?;
    let mut ids: Vec<String> = index
        .accounts
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    for detail_id in collect_account_ids_from_details()? {
        if !ids.iter().any(|id| id == &detail_id) {
            ids.push(detail_id);
        }
    }

    let mut accounts: Vec<ClaudeAccount> = ids
        .into_iter()
        .filter_map(|account_id| load_account(&account_id))
        .collect();
    sort_accounts(&mut accounts);
    save_index_from_accounts(&accounts)?;
    Ok(accounts)
}

fn resolve_managed_profile_dir(account_id: &str) -> Result<PathBuf, String> {
    Ok(get_profiles_root_dir()?.join(normalize_account_id(account_id)?))
}

fn is_managed_profile_dir(path: &Path) -> Result<bool, String> {
    let managed_root = get_profiles_root_dir()?;
    Ok(path.starts_with(managed_root))
}

fn push_unique_search_dir(search_dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !search_dirs.iter().any(|existing| existing == &dir) {
        search_dirs.push(dir);
    }
}

#[cfg(target_os = "macos")]
fn extend_claude_search_dirs_for_macos(search_dirs: &mut Vec<PathBuf>, home_dir: Option<&Path>) {
    push_unique_search_dir(search_dirs, PathBuf::from("/opt/homebrew/bin"));
    push_unique_search_dir(search_dirs, PathBuf::from("/usr/local/bin"));

    if let Some(home) = home_dir {
        push_unique_search_dir(search_dirs, home.join(".bun/bin"));
        push_unique_search_dir(search_dirs, home.join(".local/bin"));
    }
}

pub fn resolve_claude_binary_path() -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("CLAUDE_CLI_PATH") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.exists() && path.is_file() {
                return Ok(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    let binary_names = ["claude.exe", "claude.cmd", "claude.bat"];
    #[cfg(not(target_os = "windows"))]
    let binary_names = ["claude"];

    let mut search_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();

    #[cfg(target_os = "macos")]
    {
        extend_claude_search_dirs_for_macos(&mut search_dirs, dirs::home_dir().as_deref());
    }

    for dir in search_dirs {
        for binary_name in binary_names {
            let candidate = dir.join(binary_name);
            if candidate.exists() && candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err("未找到 Claude CLI，请确认 `claude` 已安装，或设置 CLAUDE_CLI_PATH".to_string())
}

pub fn build_command_env_pairs(account: &ClaudeAccount) -> Result<Vec<(String, String)>, String> {
    let mut envs = vec![("CLAUDE_CONFIG_DIR".to_string(), account.config_dir.clone())];

    if account.login_mode == "auth_token" {
        let base_url = account
            .anthropic_base_url
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
            .ok_or_else(|| "当前 profile 缺少 ANTHROPIC_BASE_URL".to_string())?;
        let auth_token = account
            .anthropic_auth_token
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
            .ok_or_else(|| "当前 profile 缺少 ANTHROPIC_AUTH_TOKEN".to_string())?;
        envs.push(("ANTHROPIC_BASE_URL".to_string(), base_url));
        envs.push(("ANTHROPIC_AUTH_TOKEN".to_string(), auth_token));
        if account.disable_nonessential_traffic {
            envs.push((
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_string(),
                "1".to_string(),
            ));
        }
    }

    Ok(envs)
}

fn read_auth_status(account: &ClaudeAccount) -> Result<Value, String> {
    let binary = resolve_claude_binary_path()?;
    let mut command = Command::new(binary);
    for (key, value) in build_command_env_pairs(account)? {
        command.env(key, value);
    }
    let output = command
        .args(["auth", "status", "--json"])
        .output()
        .map_err(|e| format!("执行 `claude auth status --json` 失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("读取 Claude 登录状态失败: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<Value>(&stdout)
        .map_err(|e| format!("解析 Claude 登录状态 JSON 失败: {}", e))
}

fn apply_auth_status(account: &mut ClaudeAccount, status: &Value) {
    account.logged_in = status
        .get("loggedIn")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    account.auth_method = normalize_status_string(status.get("authMethod").and_then(Value::as_str));
    account.api_provider = normalize_non_empty(status.get("apiProvider").and_then(Value::as_str));
    account.org_id = normalize_non_empty(status.get("orgId").and_then(Value::as_str));
    account.org_name = normalize_non_empty(status.get("orgName").and_then(Value::as_str));
    account.subscription_type =
        normalize_non_empty(status.get("subscriptionType").and_then(Value::as_str));

    if let Some(email) = normalize_non_empty(status.get("email").and_then(Value::as_str)) {
        account.email = email;
    } else if account.email.trim().is_empty() {
        if let Some(hint) = account.login_hint_email.clone() {
            account.email = hint;
        }
    }

    account.status_raw = Some(status.clone());
    account.last_synced_at = Some(now_ts());
}

pub fn create_account(
    name: Option<String>,
    login_mode: Option<String>,
    login_hint_email: Option<String>,
    anthropic_base_url: Option<String>,
    anthropic_auth_token: Option<String>,
    disable_nonessential_traffic: Option<bool>,
) -> Result<ClaudeAccount, String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁".to_string())?;

    let normalized_name = normalize_non_empty(name.as_deref());
    let normalized_hint = normalize_non_empty(login_hint_email.as_deref());
    let normalized_base_url = normalize_non_empty(anthropic_base_url.as_deref());
    let normalized_auth_token = normalize_non_empty(anthropic_auth_token.as_deref());
    let normalized_mode = normalize_login_mode(
        login_mode.as_deref(),
        normalized_hint.as_deref(),
        normalized_base_url.as_deref(),
        normalized_auth_token.as_deref(),
    )?;
    let account_id = Uuid::new_v4().to_string();
    let config_dir = resolve_managed_profile_dir(&account_id)?;
    fs::create_dir_all(&config_dir).map_err(|e| format!("创建 Claude profile 目录失败: {}", e))?;

    let now = now_ts();
    let mut account = ClaudeAccount {
        id: account_id,
        email: normalized_hint.clone().unwrap_or_default(),
        name: normalized_name,
        tags: None,
        config_dir: config_dir.to_string_lossy().to_string(),
        login_mode: normalized_mode,
        login_hint_email: normalized_hint,
        anthropic_base_url: normalized_base_url,
        anthropic_auth_token: normalized_auth_token,
        disable_nonessential_traffic: normalize_bool_flag(disable_nonessential_traffic, false),
        logged_in: false,
        auth_method: None,
        api_provider: None,
        org_id: None,
        org_name: None,
        subscription_type: None,
        status_raw: None,
        created_at: now,
        last_used: now,
        last_synced_at: None,
    };

    if let Ok(status) = read_auth_status(&account) {
        apply_auth_status(&mut account, &status);
    }

    save_account_file(&account)?;
    let mut accounts = list_accounts_checked()?;
    accounts.retain(|item| item.id != account.id);
    accounts.push(account.clone());
    sort_accounts(&mut accounts);
    save_index_from_accounts(&accounts)?;
    Ok(account)
}

pub fn refresh_account(account_id: &str) -> Result<ClaudeAccount, String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁".to_string())?;

    let mut account = load_account_checked(account_id)?;
    let status = read_auth_status(&account)?;
    apply_auth_status(&mut account, &status);
    save_account_file(&account)?;

    let mut accounts = list_accounts_checked()?;
    accounts.retain(|item| item.id != account.id);
    accounts.push(account.clone());
    sort_accounts(&mut accounts);
    save_index_from_accounts(&accounts)?;
    Ok(account)
}

pub fn refresh_all_accounts() -> Result<Vec<ClaudeAccount>, String> {
    let account_ids: Vec<String> = list_accounts_checked()?
        .into_iter()
        .map(|account| account.id)
        .collect();
    let mut refreshed = Vec::new();
    for account_id in account_ids {
        refreshed.push(refresh_account(&account_id)?);
    }
    Ok(refreshed)
}

pub fn mark_account_used(account_id: &str) -> Result<ClaudeAccount, String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁".to_string())?;

    let mut account = load_account_checked(account_id)?;
    account.last_used = now_ts();
    save_account_file(&account)?;

    let mut accounts = list_accounts_checked()?;
    accounts.retain(|item| item.id != account.id);
    accounts.push(account.clone());
    sort_accounts(&mut accounts);
    save_index_from_accounts(&accounts)?;
    Ok(account)
}

pub fn set_current_account(account_id: &str) -> Result<ClaudeAccount, String> {
    let account = mark_account_used(account_id)?;
    crate::modules::provider_current_state::set_current_account_id("claude", Some(account_id))?;
    Ok(account)
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<ClaudeAccount, String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁".to_string())?;

    let mut account = load_account_checked(account_id)?;
    account.tags = normalize_tags(&tags);
    save_account_file(&account)?;

    let mut accounts = list_accounts_checked()?;
    accounts.retain(|item| item.id != account.id);
    accounts.push(account.clone());
    sort_accounts(&mut accounts);
    save_index_from_accounts(&accounts)?;
    Ok(account)
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁".to_string())?;

    let account = load_account_checked(account_id)?;
    delete_account_file(account_id)?;

    let profile_dir = PathBuf::from(&account.config_dir);
    if profile_dir.exists() && is_managed_profile_dir(&profile_dir)? {
        fs::remove_dir_all(&profile_dir).map_err(|e| {
            format!(
                "删除 Claude profile 目录失败: path={}, error={}",
                profile_dir.display(),
                e
            )
        })?;
    }

    let mut accounts = list_accounts_checked()?;
    accounts.retain(|item| item.id != account_id);
    save_index_from_accounts(&accounts)?;

    if crate::modules::provider_current_state::get_current_account_id("claude")?.as_deref()
        == Some(account_id)
    {
        crate::modules::provider_current_state::set_current_account_id("claude", None)?;
    }
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for account_id in account_ids {
        remove_account(account_id)?;
    }
    Ok(())
}

fn ensure_import_config_dir(account: &mut ClaudeAccount) -> Result<(), String> {
    let config_dir = normalize_non_empty(Some(&account.config_dir));
    let mut reused_existing_dir = false;
    let resolved_dir = if let Some(existing_dir) = config_dir {
        let path = PathBuf::from(&existing_dir);
        if path.exists() && path.is_dir() {
            reused_existing_dir = true;
            path
        } else {
            resolve_managed_profile_dir(&account.id)?
        }
    } else {
        resolve_managed_profile_dir(&account.id)?
    };

    fs::create_dir_all(&resolved_dir)
        .map_err(|e| format!("准备 Claude profile 目录失败: {}", e))?;
    account.config_dir = resolved_dir.to_string_lossy().to_string();
    if !reused_existing_dir {
        account.logged_in = false;
        account.auth_method = None;
        account.api_provider = None;
        account.org_id = None;
        account.org_name = None;
        account.subscription_type = None;
        account.status_raw = None;
    }
    Ok(())
}

fn import_accounts_from_value(value: Value) -> Result<Vec<ClaudeAccount>, String> {
    let raw_accounts = match value {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("accounts")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "Claude 导入 JSON 缺少 accounts 数组".to_string())?,
        _ => {
            return Err(
                "Claude 导入 JSON 格式不正确，必须为数组或包含 accounts 字段的对象".to_string(),
            )
        }
    };

    let existing_ids: HashSet<String> = list_accounts_checked()?
        .into_iter()
        .map(|account| account.id)
        .collect();
    let mut seen_ids = existing_ids;
    let mut imported_accounts = Vec::new();

    for item in raw_accounts {
        let mut account: ClaudeAccount =
            serde_json::from_value(item).map_err(|e| format!("解析 Claude 导入账号失败: {}", e))?;
        let mut account_id =
            normalize_account_id(&account.id).unwrap_or_else(|_| Uuid::new_v4().to_string());
        while seen_ids.contains(&account_id) {
            account_id = Uuid::new_v4().to_string();
        }
        seen_ids.insert(account_id.clone());
        account.id = account_id;
        account.name = normalize_non_empty(account.name.as_deref());
        account.login_hint_email = normalize_non_empty(account.login_hint_email.as_deref());
        account.anthropic_base_url = normalize_non_empty(account.anthropic_base_url.as_deref());
        account.anthropic_auth_token = normalize_non_empty(account.anthropic_auth_token.as_deref());
        account.login_mode = normalize_login_mode(
            Some(&account.login_mode),
            account.login_hint_email.as_deref(),
            account.anthropic_base_url.as_deref(),
            account.anthropic_auth_token.as_deref(),
        )?;
        account.tags = account.tags.as_ref().and_then(|tags| normalize_tags(tags));
        account.disable_nonessential_traffic =
            normalize_bool_flag(Some(account.disable_nonessential_traffic), false);
        if account.created_at <= 0 {
            account.created_at = now_ts();
        }
        if account.last_used <= 0 {
            account.last_used = account.created_at;
        }
        ensure_import_config_dir(&mut account)?;
        save_account_file(&account)?;
        imported_accounts.push(account);
    }

    let mut accounts = list_accounts_checked()?;
    for account in &imported_accounts {
        accounts.retain(|item| item.id != account.id);
        accounts.push(account.clone());
    }
    sort_accounts(&mut accounts);
    save_index_from_accounts(&accounts)?;
    Ok(imported_accounts)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<ClaudeAccount>, String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁".to_string())?;
    let value: Value = serde_json::from_str(json_content)
        .map_err(|e| format!("解析 Claude 导入 JSON 失败: {}", e))?;
    import_accounts_from_value(value)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let selected_ids: HashSet<&str> = account_ids.iter().map(|id| id.as_str()).collect();
    let accounts = list_accounts_checked()?;
    let selected: Vec<ClaudeAccount> = if selected_ids.is_empty() {
        accounts
    } else {
        accounts
            .into_iter()
            .filter(|account| selected_ids.contains(account.id.as_str()))
            .collect()
    };
    serde_json::to_string_pretty(&selected).map_err(|e| format!("导出 Claude 账号失败: {}", e))
}

pub fn build_login_command_args(account: &ClaudeAccount) -> Result<(PathBuf, Vec<String>), String> {
    let binary = resolve_claude_binary_path()?;
    let mut args = vec!["auth".to_string(), "login".to_string()];

    match account.login_mode.as_str() {
        "claudeai" => args.push("--claudeai".to_string()),
        "console" => args.push("--console".to_string()),
        "sso" => args.push("--sso".to_string()),
        "email" => {
            args.push("--claudeai".to_string());
            let hint = account
                .login_hint_email
                .as_ref()
                .and_then(|value| normalize_non_empty(Some(value)))
                .ok_or_else(|| "当前 profile 缺少 email 登录提示".to_string())?;
            args.push("--email".to_string());
            args.push(hint);
        }
        "auth_token" => {
            return Err("当前 profile 使用环境变量认证，不需要执行 `claude auth login`".to_string())
        }
        other => return Err(format!("不支持的 Claude 登录模式: {}", other)),
    }

    if account.login_mode != "email" {
        if let Some(hint) = account
            .login_hint_email
            .as_ref()
            .and_then(|value| normalize_non_empty(Some(value)))
        {
            args.push("--email".to_string());
            args.push(hint);
        }
    }

    Ok((binary, args))
}

pub fn build_launch_command_args(
    _account: &ClaudeAccount,
) -> Result<(PathBuf, Vec<String>), String> {
    let binary = resolve_claude_binary_path()?;
    Ok((binary, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_account(login_mode: &str) -> ClaudeAccount {
        ClaudeAccount {
            id: "test-account".to_string(),
            email: String::new(),
            name: Some("API Token Profile".to_string()),
            tags: None,
            config_dir: "/tmp/cockpit-claude-test".to_string(),
            login_mode: login_mode.to_string(),
            login_hint_email: None,
            anthropic_base_url: Some("https://api.example.com".to_string()),
            anthropic_auth_token: Some("test-auth-token".to_string()),
            disable_nonessential_traffic: true,
            logged_in: false,
            auth_method: None,
            api_provider: None,
            org_id: None,
            org_name: None,
            subscription_type: None,
            status_raw: None,
            created_at: 0,
            last_used: 0,
            last_synced_at: None,
        }
    }

    #[test]
    fn auth_token_mode_requires_base_url_and_token() {
        assert!(normalize_login_mode(
            Some("auth_token"),
            None,
            Some("https://api.example.com"),
            Some("test-auth-token"),
        )
        .is_ok());
        assert!(
            normalize_login_mode(Some("auth_token"), None, None, Some("test-auth-token")).is_err()
        );
        assert!(normalize_login_mode(
            Some("auth_token"),
            None,
            Some("https://api.example.com"),
            None,
        )
        .is_err());
    }

    #[test]
    fn auth_token_mode_builds_runtime_envs() {
        let account = build_test_account("auth_token");
        let envs = build_command_env_pairs(&account).expect("expected envs");
        assert!(envs
            .iter()
            .any(|(key, value)| key == "CLAUDE_CONFIG_DIR" && value == "/tmp/cockpit-claude-test"));
        assert!(envs
            .iter()
            .any(|(key, value)| key == "ANTHROPIC_BASE_URL" && value == "https://api.example.com"));
        assert!(envs
            .iter()
            .any(|(key, value)| key == "ANTHROPIC_AUTH_TOKEN" && value == "test-auth-token"));
        assert!(envs.iter().any(|(key, value)| {
            key == "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC" && value == "1"
        }));
    }

    #[test]
    fn auth_token_mode_does_not_build_login_command() {
        let account = build_test_account("auth_token");
        let error = build_login_command_args(&account).expect_err("expected login command error");
        assert!(error.contains("环境变量认证"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_search_dirs_include_bun_bin_under_home() {
        let home = PathBuf::from("/tmp/cockpit-claude-home");
        let mut search_dirs = vec![PathBuf::from("/opt/homebrew/bin")];
        extend_claude_search_dirs_for_macos(&mut search_dirs, Some(home.as_path()));

        assert!(search_dirs.contains(&home.join(".bun/bin")));
    }
}
