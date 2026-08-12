//! Kimi Code multi-account store + official CLI inject.
//! Official single-slot: ~/.kimi-code/credentials/kimi-code.json + config.toml managed provider.

use crate::models::kimi::{
    KimiAccount, KimiAccountIndex, KimiAccountView, KimiOAuthCompletePayload, KimiQuota,
    KimiUsageRow,
};
use crate::modules::{atomic_write, config, kimi_oauth, logger, provider_current_state};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use toml_edit::{Document, Item, Table, Value as TomlValue};
use uuid::Uuid;

const INDEX_FILE: &str = "kimi_accounts.json";
const ACCOUNTS_DIR: &str = "kimi_accounts";
const PLATFORM: &str = "kimi";

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn data_dir() -> Result<PathBuf, String> {
    config::get_data_dir()
}

fn accounts_dir() -> Result<PathBuf, String> {
    let path = data_dir()?.join(ACCOUNTS_DIR);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("创建 Kimi 账号目录失败: {}", error))?;
    Ok(path)
}

fn index_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join(INDEX_FILE))
}

/// Same rules as Grok/CodeBuddy: block path traversal in account file names.
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
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    if !valid {
        return Err("账号 ID 非法，仅允许字母/数字/._-".to_string());
    }
    Ok(trimmed.to_string())
}

fn account_path(account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    Ok(accounts_dir()?.join(format!("{}.json", normalized)))
}

fn ensure_safe_account_id(raw: &str) -> String {
    match normalize_account_id(raw) {
        Ok(id) => id,
        Err(_) => format!("kimi-{}", Uuid::new_v4()),
    }
}

pub fn default_kimi_home() -> Result<PathBuf, String> {
    kimi_oauth::default_kimi_home()
}

fn official_credentials_path(home: &Path) -> PathBuf {
    home.join("credentials").join(kimi_oauth::CREDENTIAL_FILE_NAME)
}

fn official_config_path(home: &Path) -> PathBuf {
    home.join("config.toml")
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 Kimi HTTP 客户端失败: {}", error))
}

fn load_index() -> Result<KimiAccountIndex, String> {
    let path = index_path()?;
    if !path.exists() {
        return Ok(KimiAccountIndex::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取 Kimi 账号索引失败: {}", error))?;
    atomic_write::parse_json_with_auto_restore(&path, &content)
        .map_err(|error| format!("解析 Kimi 账号索引失败: {}", error))
}

fn save_index(index: &KimiAccountIndex) -> Result<(), String> {
    let path = index_path()?;
    let content = serde_json::to_string_pretty(index)
        .map_err(|error| format!("序列化 Kimi 账号索引失败: {}", error))?;
    atomic_write::write_string_atomic(&path, &content)
}

pub fn load_account(account_id: &str) -> Option<KimiAccount> {
    let path = account_path(account_id).ok()?;
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    match crate::modules::secure_account_storage::deserialize_account_file::<KimiAccount>(
        &path, &content,
    ) {
        Ok((account, needs_rotation)) => {
            if needs_rotation {
                let account_for_rewrite = account.clone();
                crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
                    "kimi",
                    account_for_rewrite.id.clone(),
                    path.clone(),
                    content.as_bytes(),
                    move || {
                        crate::modules::secure_account_storage::serialize_account_file(
                            "kimi",
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

fn save_account(account: &KimiAccount) -> Result<(), String> {
    let safe_id = ensure_safe_account_id(&account.id);
    let mut account = account.clone();
    account.id = safe_id;
    let path = account_path(&account.id)?;
    let content =
        crate::modules::secure_account_storage::serialize_account_file("kimi", &account)?;
    atomic_write::write_string_atomic(&path, &content)
        .map_err(|error| format!("保存 Kimi 账号失败: {}", error))?;

    let mut index = load_index()?;
    if let Some(existing) = index
        .accounts
        .iter_mut()
        .find(|item| item.id == account.id)
    {
        *existing = account.summary();
    } else {
        index.accounts.push(account.summary());
    }
    save_index(&index)
}

fn remove_from_index(account_id: &str) -> Result<(), String> {
    let mut index = load_index()?;
    index.accounts.retain(|item| item.id != account_id);
    save_index(&index)
}

pub fn list_accounts_checked() -> Result<Vec<KimiAccountView>, String> {
    let index = load_index()?;
    let mut views = Vec::new();
    for summary in index.accounts {
        if let Some(account) = load_account(&summary.id) {
            views.push(KimiAccountView::from(&account));
        }
    }
    views.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    Ok(views)
}

fn official_token_wire(account: &KimiAccount) -> Result<Value, String> {
    let refresh = account
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "账号缺少 refresh_token，无法写入官方凭据".to_string())?;
    if account.access_token.trim().is_empty() {
        return Err("账号缺少 access_token，无法写入官方凭据".to_string());
    }
    let expires_at = account.expires_at.unwrap_or(0);
    let expires_in = account.expires_in.unwrap_or(0);
    Ok(json!({
        "access_token": account.access_token,
        "refresh_token": refresh,
        "expires_at": expires_at,
        "scope": account.scope.clone().unwrap_or_default(),
        "token_type": account.token_type.clone().unwrap_or_else(|| "Bearer".to_string()),
        "expires_in": expires_in,
    }))
}

fn write_official_credentials(account: &KimiAccount, home: &Path) -> Result<(), String> {
    let cred_dir = home.join("credentials");
    std::fs::create_dir_all(&cred_dir).map_err(|error| {
        format!(
            "创建 Kimi credentials 目录失败: path={}, error={}",
            cred_dir.display(),
            error
        )
    })?;
    let path = official_credentials_path(home);
    let wire = official_token_wire(account)?;
    let content = serde_json::to_string_pretty(&wire)
        .map_err(|error| format!("序列化官方凭据失败: {}", error))?;
    atomic_write::write_string_atomic(&path, &format!("{}\n", content))?;
    Ok(())
}

fn ensure_managed_provider_config(home: &Path) -> Result<(), String> {
    let path = official_config_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "创建 Kimi config 目录失败: path={}, error={}",
                parent.display(),
                error
            )
        })?;
    }
    let raw = if path.exists() {
        std::fs::read_to_string(&path)
            .map_err(|error| format!("读取 Kimi config.toml 失败: {}", error))?
    } else {
        String::new()
    };
    let mut document: Document = if raw.trim().is_empty() {
        Document::new()
    } else {
        raw.parse::<Document>()
            .map_err(|error| format!("解析 Kimi config.toml 失败: {}", error))?
    };

    if !document
        .as_table()
        .get("providers")
        .map(Item::is_table)
        .unwrap_or(false)
    {
        document["providers"] = Item::Table(Table::new());
    }
    let providers = document["providers"]
        .as_table_mut()
        .ok_or_else(|| "Kimi config.toml providers 不是表".to_string())?;

    let provider_key = kimi_oauth::PROVIDER_NAME;
    if !providers
        .get(provider_key)
        .map(Item::is_table)
        .unwrap_or(false)
    {
        providers.insert(provider_key, Item::Table(Table::new()));
    }
    let provider = providers
        .get_mut(provider_key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| "无法写入 managed:kimi-code provider".to_string())?;

    provider.insert("type", Item::Value(TomlValue::from("kimi")));
    provider.insert(
        "base_url",
        Item::Value(TomlValue::from(kimi_oauth::API_BASE_URL)),
    );
    provider.insert("api_key", Item::Value(TomlValue::from("")));

    if !provider
        .get("oauth")
        .map(Item::is_table)
        .unwrap_or(false)
    {
        provider.insert("oauth", Item::Table(Table::new()));
    }
    let oauth = provider
        .get_mut("oauth")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| "无法写入 oauth 配置".to_string())?;
    oauth.insert("storage", Item::Value(TomlValue::from("file")));
    oauth.insert(
        "key",
        Item::Value(TomlValue::from(kimi_oauth::OAUTH_KEY)),
    );

    if document.get("default_model").is_none() {
        document["default_model"] = Item::Value(TomlValue::from("kimi-code/kimi-for-coding"));
    }

    atomic_write::write_string_atomic(&path, &document.to_string())?;
    Ok(())
}

pub fn write_account_to_official(account: &KimiAccount) -> Result<PathBuf, String> {
    let home = default_kimi_home()?;
    let _ = kimi_oauth::ensure_device_id(&home)?;
    write_official_credentials(account, &home)?;
    ensure_managed_provider_config(&home)?;
    Ok(home)
}

fn parse_official_credentials(value: &Value) -> Result<KimiAccount, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "官方凭据必须是 JSON 对象".to_string())?;
    let access_token = object
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "官方凭据缺少 access_token".to_string())?
        .to_string();
    let refresh_token = object
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let expires_at = object
        .get("expires_at")
        .and_then(|value| value.as_i64().or_else(|| value.as_f64().map(|v| v as i64)));
    let expires_in = object
        .get("expires_in")
        .and_then(|value| value.as_i64().or_else(|| value.as_f64().map(|v| v as i64)));
    let token_type = object
        .get("token_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let scope = object
        .get("scope")
        .and_then(Value::as_str)
        .map(str::to_string);
    let now = now_ts();
    let id = format!("kimi-{}", Uuid::new_v4());
    Ok(KimiAccount {
        id,
        email: "unknown@kimi.local".to_string(),
        tags: None,
        nickname: None,
        user_id: None,
        avatar: None,
        access_token,
        refresh_token,
        token_type,
        scope,
        expires_at,
        expires_in,
        device_id: None,
        plan_type: Some("Kimi Code".to_string()),
        quota: None,
        status: Some("active".to_string()),
        status_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        created_at: now,
        last_used: now,
    })
}

fn accounts_match(a: &KimiAccount, b: &KimiAccount) -> bool {
    if let (Some(uid_a), Some(uid_b)) = (
        normalize_text(a.user_id.as_deref()),
        normalize_text(b.user_id.as_deref()),
    ) {
        return uid_a == uid_b;
    }
    let email_a = a.email.trim().to_ascii_lowercase();
    let email_b = b.email.trim().to_ascii_lowercase();
    if email_a != "unknown@kimi.local"
        && email_b != "unknown@kimi.local"
        && email_a == email_b
    {
        return true;
    }
    if let (Some(rt_a), Some(rt_b)) = (
        normalize_text(a.refresh_token.as_deref()),
        normalize_text(b.refresh_token.as_deref()),
    ) {
        return rt_a == rt_b;
    }
    false
}

fn upsert_account(mut candidate: KimiAccount) -> Result<KimiAccount, String> {
    let index = load_index()?;
    for summary in index.accounts {
        if let Some(existing) = load_account(&summary.id) {
            if accounts_match(&candidate, &existing) {
                candidate.id = existing.id;
                candidate.created_at = existing.created_at;
                candidate.tags = existing.tags.or(candidate.tags);
                if candidate.email == "unknown@kimi.local" && existing.email != "unknown@kimi.local"
                {
                    candidate.email = existing.email;
                }
                if candidate.nickname.is_none() {
                    candidate.nickname = existing.nickname;
                }
                if candidate.user_id.is_none() {
                    candidate.user_id = existing.user_id;
                }
                if candidate.quota.is_none() {
                    candidate.quota = existing.quota;
                }
                break;
            }
        }
    }
    candidate.last_used = now_ts();
    save_account(&candidate)?;
    Ok(candidate)
}

pub fn upsert_oauth(payload: KimiOAuthCompletePayload) -> Result<KimiAccount, String> {
    let now = now_ts();
    let account = KimiAccount {
        id: format!("kimi-{}", Uuid::new_v4()),
        email: payload.email,
        tags: None,
        nickname: payload.nickname,
        user_id: payload.user_id,
        avatar: payload.avatar,
        access_token: payload.access_token,
        refresh_token: Some(payload.refresh_token),
        token_type: payload.token_type,
        scope: payload.scope,
        expires_at: Some(payload.expires_at),
        expires_in: Some(payload.expires_in),
        device_id: Some(payload.device_id),
        plan_type: payload.plan_type.or_else(|| Some("Kimi Code".to_string())),
        quota: None,
        status: Some("active".to_string()),
        status_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        created_at: now,
        last_used: now,
    };
    upsert_account(account)
}

pub fn upsert_oauth_for_reauth(
    payload: KimiOAuthCompletePayload,
    account_id: &str,
) -> Result<KimiAccount, String> {
    let mut existing =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    existing.access_token = payload.access_token;
    existing.refresh_token = Some(payload.refresh_token);
    existing.token_type = payload.token_type;
    existing.scope = payload.scope;
    existing.expires_at = Some(payload.expires_at);
    existing.expires_in = Some(payload.expires_in);
    existing.device_id = Some(payload.device_id);
    if let Some(nickname) = payload.nickname {
        existing.nickname = Some(nickname);
    }
    if let Some(user_id) = payload.user_id {
        existing.user_id = Some(user_id);
    }
    if payload.email != "unknown@kimi.local" {
        existing.email = payload.email;
    }
    if let Some(avatar) = payload.avatar {
        existing.avatar = Some(avatar);
    }
    existing.status = Some("active".to_string());
    existing.status_reason = None;
    existing.last_used = now_ts();
    save_account(&existing)?;
    Ok(existing)
}

pub fn import_from_local() -> Result<Vec<KimiAccountView>, String> {
    let home = default_kimi_home()?;
    let path = official_credentials_path(&home);
    if !path.exists() {
        return Err(format!(
            "未找到本机 Kimi Code 凭据: {}",
            path.display()
        ));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取本机 Kimi 凭据失败: {}", error))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|error| format!("解析本机 Kimi 凭据失败: {}", error))?;
    let mut account = parse_official_credentials(&value)?;
    if let Ok(device_id) = kimi_oauth::ensure_device_id(&home) {
        account.device_id = Some(device_id);
    }
    // Identity/quota filled on explicit refresh only (keeps import offline-safe).
    let saved = upsert_account(account)?;
    Ok(vec![KimiAccountView::from(&saved)])
}

pub fn import_from_json(content: &str) -> Result<Vec<KimiAccountView>, String> {
    let value: Value = serde_json::from_str(content)
        .map_err(|error| format!("解析导入 JSON 失败: {}", error))?;
    let mut imported = Vec::new();
    if value.is_array() {
        for item in value.as_array().into_iter().flatten() {
            if let Ok(mut account) = serde_json::from_value::<KimiAccount>(item.clone()) {
                if account.access_token.trim().is_empty() {
                    continue;
                }
                account.id = if account.id.trim().is_empty() {
                    format!("kimi-{}", Uuid::new_v4())
                } else {
                    ensure_safe_account_id(&account.id)
                };
                let saved = upsert_account(account)?;
                imported.push(KimiAccountView::from(&saved));
            } else if let Ok(account) = parse_official_credentials(item) {
                let saved = upsert_account(account)?;
                imported.push(KimiAccountView::from(&saved));
            }
        }
    } else if let Ok(mut account) = serde_json::from_value::<KimiAccount>(value.clone()) {
        if account.access_token.trim().is_empty() {
            return Err("导入 JSON 缺少 access_token".to_string());
        }
        account.id = if account.id.trim().is_empty() {
            format!("kimi-{}", Uuid::new_v4())
        } else {
            ensure_safe_account_id(&account.id)
        };
        let saved = upsert_account(account)?;
        imported.push(KimiAccountView::from(&saved));
    } else {
        let account = parse_official_credentials(&value)?;
        let saved = upsert_account(account)?;
        imported.push(KimiAccountView::from(&saved));
    }
    if imported.is_empty() {
        return Err("未识别可用的 Kimi 账号 JSON".to_string());
    }
    Ok(imported)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let mut accounts = Vec::new();
    for id in account_ids {
        if let Some(account) = load_account(id) {
            accounts.push(account);
        }
    }
    serde_json::to_string_pretty(&accounts).map_err(|error| format!("导出失败: {}", error))
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let path = account_path(account_id)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("删除 Kimi 账号失败: {}", error))?;
    }
    remove_from_index(account_id)?;
    if provider_current_state::get_current_account_id(PLATFORM)?.as_deref() == Some(account_id) {
        provider_current_state::set_current_account_id(PLATFORM, None)?;
    }
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        remove_account(id)?;
    }
    Ok(())
}

pub fn update_tags(account_id: &str, tags: Vec<String>) -> Result<KimiAccountView, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    account.tags = Some(tags);
    save_account(&account)?;
    Ok(KimiAccountView::from(&account))
}

pub fn inject_to_default(account_id: &str) -> Result<String, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    // If this is current on disk, re-read official file first (CLI may have rotated refresh).
    if let Ok(Some(current)) = provider_current_state::get_current_account_id(PLATFORM) {
        if current == account_id {
            if let Ok(disk) = read_official_into_account(&account) {
                account = disk;
            }
        }
    }
    let home = write_account_to_official(&account)?;
    account.last_used = now_ts();
    save_account(&account)?;
    provider_current_state::set_current_account_id(PLATFORM, Some(account_id))?;
    logger::log_info(&format!(
        "[Kimi Account] 已切号写入官方目录: account_id={}, home={}",
        account_id,
        home.display()
    ));
    Ok(account.email)
}

fn read_official_into_account(base: &KimiAccount) -> Result<KimiAccount, String> {
    let home = default_kimi_home()?;
    let path = official_credentials_path(&home);
    if !path.exists() {
        return Ok(base.clone());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取官方凭据失败: {}", error))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|error| format!("解析官方凭据失败: {}", error))?;
    let mut disk = parse_official_credentials(&value)?;
    disk.id = base.id.clone();
    disk.email = base.email.clone();
    disk.nickname = base.nickname.clone();
    disk.user_id = base.user_id.clone();
    disk.avatar = base.avatar.clone();
    disk.tags = base.tags.clone();
    disk.plan_type = base.plan_type.clone();
    disk.quota = base.quota.clone();
    disk.device_id = base.device_id.clone().or(disk.device_id);
    disk.created_at = base.created_at;
    Ok(disk)
}

fn apply_profile(account: &mut KimiAccount, profile: &Value) {
    if let Some(user_id) = profile
        .get("user_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        account.user_id = Some(user_id.to_string());
    }
    if let Some(nickname) = profile
        .get("nickname")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        account.nickname = Some(nickname.to_string());
        if account.email == "unknown@kimi.local" || account.email.ends_with("@kimi.local") {
            account.email = format!("{}@kimi.local", nickname);
        }
    }
    if let Some(email) = profile
        .get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        account.email = email.to_string();
    }
    if let Some(avatar) = profile
        .get("avatar")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        account.avatar = Some(avatar.to_string());
    }
    let level_name = profile
        .get("user_level_name")
        .and_then(Value::as_str)
        .map(str::to_string);
    if account.plan_type.is_none() {
        account.plan_type = level_name
            .clone()
            .or_else(|| Some("Kimi Code".to_string()));
    }
    let mut quota = account.quota.clone().unwrap_or_default();
    quota.user_level_name = level_name;
    quota.region = profile
        .get("region")
        .and_then(Value::as_str)
        .map(str::to_string);
    account.quota = Some(quota);
}

fn to_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|v| v as f64))
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

fn parse_usages(payload: &Value) -> KimiQuota {
    let mut quota = KimiQuota::default();
    if let Some(usage) = payload.get("usage") {
        quota.weekly_used = to_f64(usage.get("used").unwrap_or(&Value::Null));
        quota.weekly_limit = to_f64(usage.get("limit").unwrap_or(&Value::Null));
        quota.weekly_reset_at = usage
            .get("resetTime")
            .or_else(|| usage.get("reset_time"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if let Some(limits) = payload.get("limits").and_then(Value::as_array) {
        for item in limits {
            let detail = item.get("detail").unwrap_or(item);
            let window = item.get("window");
            let unit = window
                .and_then(|w| w.get("timeUnit").or_else(|| w.get("unit")))
                .and_then(Value::as_str)
                .map(|raw| match raw {
                    "TIME_UNIT_MINUTE" | "minute" => "minute",
                    "TIME_UNIT_HOUR" | "hour" => "hour",
                    "TIME_UNIT_DAY" | "day" => "day",
                    "TIME_UNIT_WEEK" | "week" => "week",
                    other => other,
                })
                .map(str::to_string);
            let duration = window
                .and_then(|w| w.get("duration"))
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|n| n as i64)));
            let used = to_f64(detail.get("used").unwrap_or(&Value::Null)).unwrap_or(0.0);
            let limit = to_f64(detail.get("limit").unwrap_or(&Value::Null)).unwrap_or(0.0);
            let reset_at = detail
                .get("resetTime")
                .or_else(|| detail.get("reset_time"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let name = item
                .get("name")
                .or_else(|| detail.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string);
            quota.limits.push(KimiUsageRow {
                name,
                window_unit: unit,
                window_duration: duration,
                used,
                limit,
                reset_at,
            });
        }
    }
    if let Some(wallet) = payload.get("boosterWallet").or_else(|| payload.get("booster_wallet"))
    {
        if let Some(balance) = wallet.get("balance") {
            if balance.get("type").and_then(Value::as_str) == Some("BOOSTER") {
                let amount = balance
                    .get("amount")
                    .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|n| n as i64)));
                let left = balance
                    .get("amountLeft")
                    .or_else(|| balance.get("amount_left"))
                    .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|n| n as i64)));
                // Official fixed-point: divide by 1_000_000 then round to cents.
                if let Some(amount) = amount {
                    quota.booster_total_cents = Some(((amount as f64) / 1_000_000.0).round() as i64);
                }
                if let Some(left) = left {
                    quota.booster_balance_cents =
                        Some(((left as f64) / 1_000_000.0).round() as i64);
                }
            }
        }
        quota.booster_currency = wallet
            .get("monthlyChargeLimit")
            .and_then(|v| v.get("currency"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    quota
}

#[derive(Debug)]
struct FetchJsonError {
    message: String,
    auth_failed: bool,
}

async fn fetch_json(url: &str, access_token: &str) -> Result<Value, FetchJsonError> {
    let client = http_client().map_err(|message| FetchJsonError {
        message,
        auth_failed: false,
    })?;
    let response = client
        .get(url)
        .header(AUTHORIZATION, format!("Bearer {}", access_token))
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| FetchJsonError {
            message: format!("请求 {} 失败: {}", url, error),
            auth_failed: false,
        })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| FetchJsonError {
            message: format!("读取 {} 响应失败: {}", url, error),
            auth_failed: false,
        })?;
    if !status.is_success() {
        let auth_failed = status.as_u16() == 401
            || status.as_u16() == 403
            || body.to_ascii_lowercase().contains("invalid_grant")
            || body.to_ascii_lowercase().contains("invalid_token");
        return Err(FetchJsonError {
            message: format!(
                "{} 返回 {}: {}",
                url,
                status.as_u16(),
                body.chars().take(180).collect::<String>()
            ),
            auth_failed,
        });
    }
    serde_json::from_str(&body).map_err(|error| FetchJsonError {
        message: format!("解析 {} 失败: {}", url, error),
        auth_failed: false,
    })
}

async fn ensure_fresh_account(mut account: KimiAccount) -> Result<KimiAccount, String> {
    if kimi_oauth::needs_refresh(account.expires_at, account.expires_in) {
        let refresh = account
            .refresh_token
            .clone()
            .ok_or_else(|| "缺少 refresh_token，请重新登录".to_string())?;
        match kimi_oauth::refresh_token(&refresh, account.device_id.as_deref()).await {
            Ok((token, expires_at, expires_in)) => {
                account.access_token = token.access_token;
                account.refresh_token = Some(token.refresh_token);
                account.token_type = token.token_type.or(account.token_type);
                account.scope = token.scope.or(account.scope);
                account.expires_at = Some(expires_at);
                account.expires_in = Some(expires_in);
                account.status = Some("active".to_string());
                account.status_reason = None;
                save_account(&account)?;
                // Write back if this is current official account.
                if provider_current_state::get_current_account_id(PLATFORM)?.as_deref()
                    == Some(account.id.as_str())
                {
                    let _ = write_account_to_official(&account);
                }
            }
            Err(error) => {
                account.status = Some("reauth_required".to_string());
                account.status_reason = Some(error.clone());
                save_account(&account)?;
                return Err(error);
            }
        }
    }
    Ok(account)
}

pub async fn refresh_account(account_id: &str) -> Result<KimiAccountView, String> {
    let account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let mut account = ensure_fresh_account(account).await?;

    match fetch_json(
        &format!("{}/me", kimi_oauth::API_BASE_URL),
        &account.access_token,
    )
    .await
    {
        Ok(profile) => {
            apply_profile(&mut account, &profile);
            account.quota_query_last_error = None;
            account.quota_query_last_error_at = None;
        }
        Err(error) => {
            account.quota_query_last_error = Some(error.message.clone());
            account.quota_query_last_error_at = Some(now_ms());
            if error.auth_failed {
                account.status = Some("reauth_required".to_string());
                account.status_reason = Some(error.message.clone());
            }
            logger::log_warn(&format!(
                "[Kimi Account] /me 失败: account_id={}, error={}",
                account_id, error.message
            ));
        }
    }

    match fetch_json(
        &format!("{}/usages", kimi_oauth::API_BASE_URL),
        &account.access_token,
    )
    .await
    {
        Ok(payload) => {
            let mut quota = parse_usages(&payload);
            if let Some(existing) = account.quota.as_ref() {
                if quota.user_level_name.is_none() {
                    quota.user_level_name = existing.user_level_name.clone();
                }
                if quota.region.is_none() {
                    quota.region = existing.region.clone();
                }
            }
            account.quota = Some(quota);
            account.usage_updated_at = Some(now_ms());
            account.quota_query_last_error = None;
            account.quota_query_last_error_at = None;
        }
        Err(error) => {
            account.quota_query_last_error = Some(error.message.clone());
            account.quota_query_last_error_at = Some(now_ms());
            if error.auth_failed {
                account.status = Some("reauth_required".to_string());
                account.status_reason = Some(error.message.clone());
            }
            logger::log_warn(&format!(
                "[Kimi Account] /usages 失败: account_id={}, error={}",
                account_id, error.message
            ));
        }
    }

    account.last_used = now_ts();
    save_account(&account)?;
    if provider_current_state::get_current_account_id(PLATFORM)?.as_deref()
        == Some(account.id.as_str())
    {
        let _ = write_account_to_official(&account);
    }
    Ok(KimiAccountView::from(&account))
}

pub async fn refresh_all_accounts() -> Result<Vec<(String, Result<KimiAccountView, String>)>, String>
{
    let index = load_index()?;
    let mut results = Vec::new();
    for summary in index.accounts {
        let result = refresh_account(&summary.id).await;
        results.push((summary.id, result));
    }
    Ok(results)
}

/// After OAuth only: fill identity via /me, skip /usages to avoid extra traffic.
pub async fn hydrate_profile_only(account_id: &str) -> Result<KimiAccountView, String> {
    let account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let mut account = ensure_fresh_account(account).await?;
    match fetch_json(
        &format!("{}/me", kimi_oauth::API_BASE_URL),
        &account.access_token,
    )
    .await
    {
        Ok(profile) => {
            apply_profile(&mut account, &profile);
            save_account(&account)?;
        }
        Err(error) if error.auth_failed => {
            account.status = Some("reauth_required".to_string());
            account.status_reason = Some(error.message);
            save_account(&account)?;
        }
        Err(_) => {}
    }
    Ok(KimiAccountView::from(&account))
}

pub fn current_account_id() -> Result<Option<String>, String> {
    provider_current_state::get_current_account_id(PLATFORM)
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(index_path()?.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    struct DataDirGuard {
        dir: PathBuf,
        previous_test: Option<String>,
        previous_data: Option<String>,
    }

    impl DataDirGuard {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "kimi-account-test-{}-{}-{}",
                label,
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            let previous_test = std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR").ok();
            let previous_data = std::env::var("COCKPIT_TOOLS_DATA_DIR").ok();
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &dir);
            Self {
                dir,
                previous_test,
                previous_data,
            }
        }
    }

    impl Drop for DataDirGuard {
        fn drop(&mut self) {
            match self.previous_test.as_ref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
            }
            match self.previous_data.as_ref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn normalize_account_id_rejects_traversal() {
        assert!(normalize_account_id("../evil").is_err());
        assert!(normalize_account_id("a/b").is_err());
        assert!(normalize_account_id("ok-id_1.2").is_ok());
    }

    #[test]
    fn export_import_roundtrip_preserves_tokens() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("roundtrip");

        let now = now_ts();
        let account = KimiAccount {
            id: "kimi-roundtrip-1".to_string(),
            email: "rt@kimi.local".to_string(),
            tags: None,
            nickname: Some("rt".to_string()),
            user_id: Some("u1".to_string()),
            avatar: None,
            access_token: "access-secret-xyz".to_string(),
            refresh_token: Some("refresh-secret-xyz".to_string()),
            token_type: Some("Bearer".to_string()),
            scope: None,
            expires_at: Some(now + 3600),
            expires_in: Some(3600),
            device_id: Some("dev-1".to_string()),
            plan_type: Some("MODERATO".to_string()),
            quota: None,
            status: Some("active".to_string()),
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: now,
            last_used: now,
        };
        save_account(&account).expect("save");

        let on_disk = std::fs::read_to_string(account_path("kimi-roundtrip-1").unwrap())
            .expect("read detail file");
        assert!(
            on_disk.contains("AES-256-GCM") || !on_disk.contains("access-secret-xyz"),
            "detail file must not store raw access token in plaintext"
        );

        let exported = export_accounts(&["kimi-roundtrip-1".to_string()]).expect("export");
        assert!(
            exported.contains("access-secret-xyz"),
            "export must include full credentials"
        );
        assert!(exported.contains("refresh-secret-xyz"));

        let views = list_accounts_checked().expect("list");
        assert_eq!(views.len(), 1);
        assert!(views[0].access_token.is_empty());

        remove_account("kimi-roundtrip-1").expect("delete");
        assert!(load_account("kimi-roundtrip-1").is_none());

        let imported = import_from_json(&exported).expect("import");
        assert_eq!(imported.len(), 1);
        let restored = load_account(&imported[0].id).expect("restored");
        assert_eq!(restored.access_token, "access-secret-xyz");
        assert_eq!(
            restored.refresh_token.as_deref(),
            Some("refresh-secret-xyz")
        );

        let evil = r#"[{"id":"../escape","email":"e@kimi.local","access_token":"a","refresh_token":"r","created_at":1,"last_used":1}]"#;
        let evil_import = import_from_json(evil).expect("evil import");
        assert!(!evil_import[0].id.contains(".."));
        assert!(!evil_import[0].id.contains('/'));
    }

    #[test]
    fn provider_current_state_accepts_kimi() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("current");
        provider_current_state::set_current_account_id("kimi", Some("kimi-acc-1"))
            .expect("set kimi current");
        assert_eq!(
            provider_current_state::get_current_account_id("kimi").expect("get kimi"),
            Some("kimi-acc-1".to_string())
        );
    }
}
