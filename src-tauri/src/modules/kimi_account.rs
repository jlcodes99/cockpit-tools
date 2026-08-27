//! Kimi Code multi-account store + official CLI inject.
//! Official single-slot: ~/.kimi-code/credentials/kimi-code.json + config.toml managed provider.

use crate::models::kimi::{
    KimiAccount, KimiAccountIndex, KimiAccountView, KimiOAuthCompletePayload, KimiQuota,
    KimiUsageRow,
};
use crate::modules::{atomic_write, config, kimi_oauth, logger, provider_current_state};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use toml_edit::{Document, Item, Table, Value as TomlValue};
use uuid::Uuid;

const INDEX_FILE: &str = "kimi_accounts.json";
const ACCOUNTS_DIR: &str = "kimi_accounts";
const PLATFORM: &str = "kimi";
const QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 6 * 60 * 60;

static QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

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
    std::fs::create_dir_all(&path).map_err(|error| format!("创建 Kimi 账号目录失败: {}", error))?;
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
    home.join("credentials")
        .join(kimi_oauth::CREDENTIAL_FILE_NAME)
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
    let content = crate::modules::secure_account_storage::serialize_account_file("kimi", &account)?;
    atomic_write::write_string_atomic(&path, &content)
        .map_err(|error| format!("保存 Kimi 账号失败: {}", error))?;

    let mut index = load_index()?;
    if let Some(existing) = index.accounts.iter_mut().find(|item| item.id == account.id) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiOfficialModel {
    pub id: String,
    pub display_name: Option<String>,
    pub max_context_size: i64,
    pub capabilities: Vec<String>,
    pub support_efforts: Vec<String>,
    pub default_effort: Option<String>,
}

impl KimiOfficialModel {
    pub fn alias(&self) -> String {
        format!("kimi-code/{}", self.id)
    }
}

fn toml_string_array(values: &[String]) -> Item {
    let mut array = toml_edit::Array::new();
    for value in values {
        array.push(value.as_str());
    }
    Item::Value(TomlValue::Array(array))
}

fn ensure_table<'a>(parent: &'a mut Table, key: &str) -> Result<&'a mut Table, String> {
    if !parent.get(key).map(Item::is_table).unwrap_or(false) {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| format!("无法写入表 {}", key))
}

fn write_oauth_ref(table: &mut Table) {
    if !table.get("oauth").map(Item::is_table).unwrap_or(false) {
        table.insert("oauth", Item::Table(Table::new()));
    }
    if let Some(oauth) = table.get_mut("oauth").and_then(Item::as_table_mut) {
        oauth.insert("storage", Item::Value(TomlValue::from("file")));
        oauth.insert("key", Item::Value(TomlValue::from(kimi_oauth::OAUTH_KEY)));
    }
}

pub fn parse_official_models(payload: &Value) -> Result<Vec<KimiOfficialModel>, String> {
    let items = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Kimi /models 响应缺少 data 数组".to_string())?;
    let mut models = Vec::new();
    for item in items {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Kimi /models 条目缺少 id".to_string())?;
        let context = item
            .get("context_length")
            .and_then(|value| value.as_i64().or_else(|| value.as_f64().map(|n| n as i64)))
            .filter(|value| *value > 0)
            .unwrap_or(262144);
        let mut capabilities = vec!["thinking".to_string(), "always_thinking".to_string()];
        if item
            .get("supports_image_in")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            capabilities.push("image_in".to_string());
        }
        if item
            .get("supports_video_in")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            capabilities.push("video_in".to_string());
        }
        capabilities.push("tool_use".to_string());
        let is_k3 = id == "k3" || id.starts_with("k3-");
        models.push(KimiOfficialModel {
            id: id.to_string(),
            display_name: item
                .get("display_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            max_context_size: context,
            capabilities,
            support_efforts: if is_k3 {
                vec!["low".to_string(), "high".to_string(), "max".to_string()]
            } else {
                Vec::new()
            },
            default_effort: is_k3.then(|| "high".to_string()),
        });
    }
    Ok(models)
}

pub fn apply_official_managed_config(
    home: &Path,
    models: &[KimiOfficialModel],
) -> Result<String, String> {
    if models.is_empty() {
        return Err(
            "Kimi /models 未返回可用模型，已中止切号，未写入残缺 default_model".to_string(),
        );
    }
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

    let provider_key = kimi_oauth::PROVIDER_NAME;
    {
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
        let provider = ensure_table(providers, provider_key)?;
        provider.insert("type", Item::Value(TomlValue::from("kimi")));
        provider.insert(
            "base_url",
            Item::Value(TomlValue::from(kimi_oauth::API_BASE_URL)),
        );
        provider.insert("api_key", Item::Value(TomlValue::from("")));
        write_oauth_ref(provider);
    }

    {
        if !document
            .as_table()
            .get("models")
            .map(Item::is_table)
            .unwrap_or(false)
        {
            document["models"] = Item::Table(Table::new());
        }
        let model_table = document["models"]
            .as_table_mut()
            .ok_or_else(|| "Kimi config.toml models 不是表".to_string())?;
        let stale: Vec<String> = model_table
            .iter()
            .filter_map(|(key, item)| {
                item.as_table()
                    .and_then(|table| table.get("provider"))
                    .and_then(Item::as_str)
                    .filter(|provider| *provider == provider_key)
                    .map(|_| key.to_string())
            })
            .collect();
        for key in stale {
            model_table.remove(&key);
        }
        for model in models {
            let alias = model.alias();
            let entry = ensure_table(model_table, &alias)?;
            entry.insert("provider", Item::Value(TomlValue::from(provider_key)));
            entry.insert("model", Item::Value(TomlValue::from(model.id.as_str())));
            entry.insert(
                "max_context_size",
                Item::Value(TomlValue::from(model.max_context_size)),
            );
            entry.insert("capabilities", toml_string_array(&model.capabilities));
            if let Some(display_name) = model.display_name.as_deref() {
                entry.insert("display_name", Item::Value(TomlValue::from(display_name)));
            }
            if !model.support_efforts.is_empty() {
                entry.insert("support_efforts", toml_string_array(&model.support_efforts));
            }
            if let Some(effort) = model.default_effort.as_deref() {
                entry.insert("default_effort", Item::Value(TomlValue::from(effort)));
            }
        }
    }

    let default_model = models
        .iter()
        .find(|model| model.id == "kimi-for-coding")
        .or_else(|| models.first())
        .map(KimiOfficialModel::alias)
        .ok_or_else(|| {
            "Kimi /models 未返回可用模型，已中止切号，未写入残缺 default_model".to_string()
        })?;
    document["default_model"] = Item::Value(TomlValue::from(default_model.as_str()));

    if !document
        .as_table()
        .get("thinking")
        .map(Item::is_table)
        .unwrap_or(false)
    {
        document["thinking"] = Item::Table(Table::new());
    }
    if let Some(thinking) = document["thinking"].as_table_mut() {
        thinking.insert("enabled", Item::Value(TomlValue::from(true)));
    }

    {
        if !document
            .as_table()
            .get("services")
            .map(Item::is_table)
            .unwrap_or(false)
        {
            document["services"] = Item::Table(Table::new());
        }
        let services = document["services"]
            .as_table_mut()
            .ok_or_else(|| "Kimi config.toml services 不是表".to_string())?;
        for (name, url) in [
            ("moonshot_search", "https://api.kimi.com/coding/v1/search"),
            ("moonshot_fetch", "https://api.kimi.com/coding/v1/fetch"),
        ] {
            let service = ensure_table(services, name)?;
            service.insert("base_url", Item::Value(TomlValue::from(url)));
            service.insert("api_key", Item::Value(TomlValue::from("")));
            write_oauth_ref(service);
        }
    }

    atomic_write::write_string_atomic(&path, &document.to_string())?;
    Ok(default_model)
}

pub fn write_official_credentials_only(account: &KimiAccount) -> Result<PathBuf, String> {
    let home = default_kimi_home()?;
    let _ = kimi_oauth::ensure_device_id_with(&home, account.device_id.as_deref())?;
    write_official_credentials(account, &home)?;
    Ok(home)
}

pub fn write_account_to_official(
    account: &KimiAccount,
    models: &[KimiOfficialModel],
) -> Result<PathBuf, String> {
    let home = default_kimi_home()?;
    let _ = kimi_oauth::ensure_device_id_with(&home, account.device_id.as_deref())?;
    // Config first: empty/failed models never reach here, so we never leave a stub
    // default_model. Credentials are written only after the models table is on disk.
    apply_official_managed_config(&home, models)?;
    write_official_credentials(account, &home)?;
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
    let user_id = object
        .get("user_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let now = now_ts();
    let id = format!("kimi-{}", Uuid::new_v4());
    Ok(KimiAccount {
        id,
        email: "unknown@kimi.local".to_string(),
        tags: None,
        nickname: None,
        user_id,
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
    if email_a != "unknown@kimi.local" && email_b != "unknown@kimi.local" && email_a == email_b {
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

fn official_identity_matches(base: &KimiAccount, disk: &KimiAccount) -> bool {
    if let (Some(base_uid), Some(disk_uid)) = (
        normalize_text(base.user_id.as_deref()),
        normalize_text(disk.user_id.as_deref()),
    ) {
        return base_uid == disk_uid;
    }
    if let (Some(base_rt), Some(disk_rt)) = (
        normalize_text(base.refresh_token.as_deref()),
        normalize_text(disk.refresh_token.as_deref()),
    ) {
        return base_rt == disk_rt;
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
        return Err(format!("未找到本机 Kimi Code 凭据: {}", path.display()));
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
    let value: Value =
        serde_json::from_str(content).map_err(|error| format!("解析导入 JSON 失败: {}", error))?;
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
        std::fs::remove_file(&path).map_err(|error| format!("删除 Kimi 账号失败: {}", error))?;
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

pub async fn inject_to_default(account_id: &str) -> Result<String, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if let Ok(Some(current)) = provider_current_state::get_current_account_id(PLATFORM) {
        if current == account_id {
            if let Ok(disk) = read_official_into_account(&account) {
                account = disk;
            }
        }
    }
    let account = ensure_fresh_account(account).await?;
    let models = fetch_official_models(&account.access_token).await?;
    if models.is_empty() {
        return Err(
            "Kimi /models 未返回可用模型，已中止切号，未写入残缺 default_model".to_string(),
        );
    }
    let home = write_account_to_official(&account, &models)?;
    let mut account = account;
    account.last_used = now_ts();
    save_account(&account)?;
    provider_current_state::set_current_account_id(PLATFORM, Some(account_id))?;
    logger::log_info(&format!(
        "[Kimi Account] 已切号写入官方目录: account_id={}, home={}, models={}",
        account_id,
        home.display(),
        models.len()
    ));
    Ok(account.email)
}

/// Mark Cockpit current account without writing official credentials/config.
pub fn mark_current_without_official_write(account_id: &str) -> Result<String, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    account.last_used = now_ts();
    save_account(&account)?;
    provider_current_state::set_current_account_id(PLATFORM, Some(account_id))?;
    logger::log_info(&format!(
        "[Kimi Account] 已切换当前账号（未写入官方目录）: account_id={}",
        account_id
    ));
    Ok(account.email)
}

fn remaining_percent_from_used_total(used: f64, total: f64) -> Option<i32> {
    if !used.is_finite() || !total.is_finite() || total <= 0.0 {
        return None;
    }
    let remaining = ((total - used).max(0.0) / total * 100.0).clamp(0.0, 100.0);
    Some(remaining.round() as i32)
}

fn quota_remaining_metrics(account: &KimiAccountView) -> Vec<(String, i32)> {
    let Some(quota) = account.quota.as_ref() else {
        return Vec::new();
    };
    let mut metrics = Vec::new();
    if let (Some(used), Some(limit)) = (quota.weekly_used, quota.weekly_limit) {
        if let Some(remaining) = remaining_percent_from_used_total(used, limit) {
            metrics.push(("weekly".to_string(), remaining));
        }
    }
    for row in &quota.limits {
        if let Some(remaining) = remaining_percent_from_used_total(row.used, row.limit) {
            let name = row
                .name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("quota")
                .to_string();
            metrics.push((name, remaining));
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
    let user_config = config::get_user_config();
    if !user_config.kimi_quota_alert_enabled {
        return Ok(());
    }
    let threshold = user_config.kimi_quota_alert_threshold.clamp(0, 100);
    let accounts = list_accounts_checked()?;
    let now = now_ts();
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
                platform: "kimi".to_string(),
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

fn read_official_into_account(base: &KimiAccount) -> Result<KimiAccount, String> {
    let home = default_kimi_home()?;
    let path = official_credentials_path(&home);
    if !path.exists() {
        return Ok(base.clone());
    }
    let content =
        std::fs::read_to_string(&path).map_err(|error| format!("读取官方凭据失败: {}", error))?;
    let value: Value =
        serde_json::from_str(&content).map_err(|error| format!("解析官方凭据失败: {}", error))?;
    let disk = parse_official_credentials(&value)?;
    if !official_identity_matches(base, &disk) {
        return Ok(base.clone());
    }
    let mut disk = disk;
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
        account.plan_type = level_name.clone().or_else(|| Some("Kimi Code".to_string()));
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
    if let Some(wallet) = payload
        .get("boosterWallet")
        .or_else(|| payload.get("booster_wallet"))
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
                    quota.booster_total_cents =
                        Some(((amount as f64) / 1_000_000.0).round() as i64);
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

async fn fetch_official_models(access_token: &str) -> Result<Vec<KimiOfficialModel>, String> {
    match fetch_json(
        &format!("{}/models", kimi_oauth::API_BASE_URL),
        access_token,
    )
    .await
    {
        Ok(payload) => parse_official_models(&payload),
        Err(error) => {
            if error.message.contains(" 402") || error.message.contains(" 403") {
                Err(format!(
                    "Kimi 会员权益无法校验（{}）。切号已中止，未写入残缺 default_model",
                    error.message
                ))
            } else {
                Err(error.message)
            }
        }
    }
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
        .header("X-Msh-Platform", kimi_oauth::X_MSH_PLATFORM)
        .header("X-Msh-Version", kimi_oauth::X_MSH_VERSION)
        .send()
        .await
        .map_err(|error| FetchJsonError {
            message: format!("请求 {} 失败: {}", url, error),
            auth_failed: false,
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| FetchJsonError {
        message: format!("读取 {} 响应失败: {}", url, error),
        auth_failed: false,
    })?;
    if !status.is_success() {
        let auth_failed = status.as_u16() == 401
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
                    let _ = write_official_credentials_only(&account);
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
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
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
    if config::get_user_config().kimi_sync_official_config_on_switch
        && provider_current_state::get_current_account_id(PLATFORM)?.as_deref()
            == Some(account.id.as_str())
    {
        let _ = write_official_credentials_only(&account);
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
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
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
    use crate::models::kimi::KimiOAuthCompletePayload;
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

    fn isolated_kimi_home(label: &str) -> PathBuf {
        let home =
            std::env::temp_dir().join(format!("cockpit-kimi-{}-{}", label, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).expect("create isolated home");
        home
    }

    #[test]
    fn apply_config_refuses_empty_models_and_leaves_no_default_model() {
        let home = isolated_kimi_home("empty");
        let error = apply_official_managed_config(&home, &[]).expect_err("empty models");
        assert!(error.contains("未写入残缺 default_model"));
        assert!(!official_config_path(&home).exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn apply_config_writes_models_table_and_matching_default() {
        let home = isolated_kimi_home("models");
        let payload = serde_json::json!({
            "data": [
                {
                    "id": "kimi-for-coding",
                    "display_name": "K2.7 Coding",
                    "context_length": 262144,
                    "supports_image_in": true,
                    "supports_video_in": true
                },
                {
                    "id": "k3",
                    "display_name": "K3",
                    "context_length": 262144,
                    "supports_image_in": true,
                    "supports_video_in": true
                }
            ]
        });
        let models = parse_official_models(&payload).expect("parse");
        let default_model = apply_official_managed_config(&home, &models).expect("apply");
        assert_eq!(default_model, "kimi-code/kimi-for-coding");
        let raw = std::fs::read_to_string(official_config_path(&home)).expect("read");
        assert!(raw.contains("default_model = \"kimi-code/kimi-for-coding\""));
        assert!(raw.contains("[models.\"kimi-code/kimi-for-coding\"]"));
        assert!(raw.contains("[models.\"kimi-code/k3\"]"));
        assert!(raw.contains("provider = \"managed:kimi-code\""));
        assert!(raw.contains("[services.moonshot_search]"));
        assert!(raw.contains("[services.moonshot_fetch]"));
        assert!(
            raw.contains("kimi-code/kimi-for-coding"),
            "default_model alias must exist in models table"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn apply_config_empty_models_leaves_existing_stub_untouched() {
        let home = isolated_kimi_home("stub");
        let path = official_config_path(&home);
        std::fs::write(&path, "default_model = \"kimi-code/kimi-for-coding\"\n")
            .expect("seed stub");
        let before = std::fs::read_to_string(&path).expect("read stub");
        let error = apply_official_managed_config(&home, &[]).expect_err("empty models");
        assert!(error.contains("未写入残缺 default_model"));
        let after = std::fs::read_to_string(&path).expect("read after fail");
        assert_eq!(before, after);
        assert!(!after.contains("[models."));
        let _ = std::fs::remove_dir_all(&home);
    }

    struct KimiHomeGuard {
        dir: PathBuf,
        previous: Option<String>,
    }

    impl KimiHomeGuard {
        fn new(label: &str) -> Self {
            let dir = isolated_kimi_home(label);
            let previous = std::env::var("KIMI_CODE_HOME").ok();
            std::env::set_var("KIMI_CODE_HOME", dir.to_string_lossy().as_ref());
            Self { dir, previous }
        }
    }

    impl Drop for KimiHomeGuard {
        fn drop(&mut self) {
            match self.previous.as_ref() {
                Some(value) => std::env::set_var("KIMI_CODE_HOME", value),
                None => std::env::remove_var("KIMI_CODE_HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn sample_account(id: &str, access: &str, refresh: &str) -> KimiAccount {
        let now = now_ts();
        KimiAccount {
            id: id.to_string(),
            email: "sample@kimi.local".to_string(),
            tags: None,
            nickname: Some("sample".to_string()),
            user_id: Some("uid-sample".to_string()),
            avatar: None,
            access_token: access.to_string(),
            refresh_token: Some(refresh.to_string()),
            token_type: Some("Bearer".to_string()),
            scope: None,
            expires_at: Some(now + 7200),
            expires_in: Some(7200),
            device_id: Some("device-sample".to_string()),
            plan_type: Some("MODERATO".to_string()),
            quota: None,
            status: Some("active".to_string()),
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: now,
            last_used: now,
        }
    }

    fn sample_models() -> Vec<KimiOfficialModel> {
        let payload = serde_json::json!({
            "data": [{
                "id": "kimi-for-coding",
                "display_name": "K2.7 Coding",
                "context_length": 262144,
                "supports_image_in": true,
                "supports_video_in": true
            }]
        });
        parse_official_models(&payload).expect("parse models")
    }

    #[test]
    fn update_tags_persists() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("tags");
        save_account(&sample_account("kimi-tags-1", "acc-tags", "ref-tags")).expect("save");
        let view = update_tags("kimi-tags-1", vec!["work".to_string(), "vip".to_string()])
            .expect("update tags");
        assert_eq!(view.tags.as_ref().map(|t| t.len()), Some(2));
        let stored = load_account("kimi-tags-1").expect("load");
        assert_eq!(
            stored.tags.as_ref().map(|t| t.join(",")),
            Some("work,vip".to_string())
        );
    }

    #[test]
    fn remove_account_clears_current_state() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("remove");
        save_account(&sample_account("kimi-del-1", "acc-del", "ref-del")).expect("save");
        provider_current_state::set_current_account_id(PLATFORM, Some("kimi-del-1"))
            .expect("set current");
        remove_account("kimi-del-1").expect("remove");
        assert!(load_account("kimi-del-1").is_none());
        assert_eq!(current_account_id().expect("current"), None);
    }

    #[test]
    fn remove_accounts_batch() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("batch-del");
        save_account(&sample_account("kimi-del-a", "a1", "r1")).expect("save a");
        save_account(&sample_account("kimi-del-b", "a2", "r2")).expect("save b");
        remove_accounts(&["kimi-del-a".to_string(), "kimi-del-b".to_string()])
            .expect("batch remove");
        assert!(load_account("kimi-del-a").is_none());
        assert!(load_account("kimi-del-b").is_none());
        assert!(list_accounts_checked().expect("list").is_empty());
    }

    #[test]
    fn import_from_json_skips_empty_tokens_in_array() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("import-skip");
        let json = r#"[
            {"id":"kimi-empty","email":"e@kimi.local","access_token":"","created_at":1,"last_used":1},
            {"id":"kimi-good","email":"g@kimi.local","access_token":"good-access","refresh_token":"good-refresh","created_at":1,"last_used":1}
        ]"#;
        let imported = import_from_json(json).expect("import");
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].id, "kimi-good");
    }

    #[test]
    fn import_from_json_official_credentials_object() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("import-official");
        let json = r#"{"access_token":"off-access","refresh_token":"off-refresh","expires_at":999999,"token_type":"Bearer"}"#;
        let imported = import_from_json(json).expect("import official");
        assert_eq!(imported.len(), 1);
        let stored = load_account(&imported[0].id).expect("stored");
        assert_eq!(stored.access_token, "off-access");
        assert_eq!(stored.refresh_token.as_deref(), Some("off-refresh"));
    }

    #[test]
    fn import_from_json_missing_token_errors() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("import-missing");
        let err = import_from_json(r#"{"email":"e@kimi.local","created_at":1,"last_used":1}"#)
            .expect_err("missing token");
        assert!(err.contains("access_token") || err.contains("未识别"));
    }

    #[test]
    fn import_from_local_reads_isolated_credentials() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("import-local");
        let _home = KimiHomeGuard::new("local-import");
        let cred_dir = default_kimi_home().expect("home").join("credentials");
        std::fs::create_dir_all(&cred_dir).expect("cred dir");
        let cred_path = cred_dir.join(kimi_oauth::CREDENTIAL_FILE_NAME);
        std::fs::write(
            &cred_path,
            r#"{
                "access_token": "local-access",
                "refresh_token": "local-refresh",
                "expires_at": 999999,
                "token_type": "Bearer"
            }"#,
        )
        .expect("write creds");
        let imported = import_from_local().expect("import local");
        assert_eq!(imported.len(), 1);
        let stored = load_account(&imported[0].id).expect("stored");
        assert_eq!(stored.access_token, "local-access");
    }

    #[test]
    fn write_account_to_official_writes_credentials_and_config() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("write-official");
        let _home = KimiHomeGuard::new("write-official");
        let account = sample_account("kimi-write-1", "wire-access", "wire-refresh");
        let models = sample_models();
        let home = write_account_to_official(&account, &models).expect("write official");
        let cred_path = home
            .join("credentials")
            .join(kimi_oauth::CREDENTIAL_FILE_NAME);
        let cred_raw = std::fs::read_to_string(&cred_path).expect("read creds");
        assert!(cred_raw.contains("wire-access"));
        assert!(cred_raw.contains("wire-refresh"));
        let config_raw = std::fs::read_to_string(official_config_path(&home)).expect("read config");
        assert!(config_raw.contains("default_model = \"kimi-code/kimi-for-coding\""));
        assert!(config_raw.contains("[models.\"kimi-code/kimi-for-coding\"]"));
        let device_id = std::fs::read_to_string(home.join("device_id")).expect("device_id");
        assert_eq!(device_id.trim(), "device-sample");
    }

    #[test]
    fn read_official_into_account_skips_mismatched_identity() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("merge-mismatch");
        let _home = KimiHomeGuard::new("merge-mismatch");
        let base = sample_account("kimi-base-1", "base-access", "base-refresh");
        save_account(&base).expect("save");
        write_official_credentials(
            &sample_account("kimi-other", "other-access", "other-refresh"),
            &_home.dir,
        )
        .expect("write other");
        let merged = read_official_into_account(&base).expect("merge");
        assert_eq!(merged.access_token, "base-access");
        assert_eq!(merged.refresh_token.as_deref(), Some("base-refresh"));
    }

    #[test]
    fn read_official_into_account_merges_same_refresh_token() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("merge-same");
        let _home = KimiHomeGuard::new("merge-same");
        let base = sample_account("kimi-base-2", "old-access", "same-refresh");
        let mut disk = sample_account("kimi-disk", "fresh-access", "same-refresh");
        disk.expires_at = Some(base.expires_at.unwrap_or(0) + 100);
        write_official_credentials(&disk, &_home.dir).expect("write same");
        let merged = read_official_into_account(&base).expect("merge");
        assert_eq!(merged.access_token, "fresh-access");
        assert_eq!(merged.refresh_token.as_deref(), Some("same-refresh"));
        assert_eq!(merged.id, base.id);
        assert_eq!(merged.email, base.email);
    }

    #[test]
    fn official_identity_matches_rejects_different_user_id() {
        let mut base = sample_account("kimi-a", "a", "ra");
        let mut disk = sample_account("kimi-b", "b", "rb");
        base.user_id = Some("uid-a".to_string());
        disk.user_id = Some("uid-b".to_string());
        assert!(!official_identity_matches(&base, &disk));
        disk.user_id = Some("uid-a".to_string());
        assert!(official_identity_matches(&base, &disk));
    }

    #[test]
    fn official_token_wire_rejects_missing_refresh() {
        let account = KimiAccount {
            id: "x".to_string(),
            email: "e@kimi.local".to_string(),
            tags: None,
            nickname: None,
            user_id: None,
            avatar: None,
            access_token: "a".to_string(),
            refresh_token: None,
            token_type: None,
            scope: None,
            expires_at: None,
            expires_in: None,
            device_id: None,
            plan_type: None,
            quota: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: 1,
            last_used: 1,
        };
        let err = official_token_wire(&account).expect_err("no refresh");
        assert!(err.contains("refresh_token"));
    }

    #[test]
    fn parse_usages_extracts_weekly_and_limits() {
        let payload = serde_json::json!({
            "usage": { "used": 10, "limit": 100, "resetTime": "2026-01-01T00:00:00Z" },
            "limits": [{
                "name": "5 hour",
                "window": { "timeUnit": "TIME_UNIT_HOUR", "duration": 5 },
                "detail": { "used": 3, "limit": 50, "resetTime": "2026-01-02T00:00:00Z" }
            }]
        });
        let quota = parse_usages(&payload);
        assert_eq!(quota.weekly_used, Some(10.0));
        assert_eq!(quota.weekly_limit, Some(100.0));
        assert_eq!(quota.limits.len(), 1);
        assert_eq!(quota.limits[0].used, 3.0);
        assert_eq!(quota.limits[0].limit, 50.0);
        assert_eq!(quota.limits[0].window_unit.as_deref(), Some("hour"));
        assert_eq!(quota.limits[0].window_duration, Some(5));
    }

    #[test]
    fn apply_profile_updates_email_and_level() {
        let mut account = sample_account("kimi-profile", "a", "r");
        account.email = "unknown@kimi.local".to_string();
        let profile = serde_json::json!({
            "user_id": "uid-42",
            "nickname": "coder",
            "email": "real@example.com",
            "user_level_name": "ALLEGRO",
            "region": "cn"
        });
        apply_profile(&mut account, &profile);
        assert_eq!(account.user_id.as_deref(), Some("uid-42"));
        assert_eq!(account.email, "real@example.com");
        assert_eq!(
            account
                .quota
                .as_ref()
                .and_then(|q| q.user_level_name.as_deref()),
            Some("ALLEGRO")
        );
        assert_eq!(
            account.quota.as_ref().and_then(|q| q.region.as_deref()),
            Some("cn")
        );
    }

    #[test]
    fn upsert_oauth_for_reauth_updates_tokens() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("reauth");
        save_account(&sample_account(
            "kimi-reauth-1",
            "old-access",
            "old-refresh",
        ))
        .expect("save");
        let payload = KimiOAuthCompletePayload {
            access_token: "new-access".to_string(),
            refresh_token: "new-refresh".to_string(),
            token_type: Some("Bearer".to_string()),
            scope: None,
            expires_at: now_ts() + 3600,
            expires_in: 3600,
            device_id: "dev-reauth".to_string(),
            email: "reauth@kimi.local".to_string(),
            nickname: Some("reauth".to_string()),
            user_id: Some("uid-reauth".to_string()),
            avatar: None,
            plan_type: Some("Kimi Code".to_string()),
        };
        let updated = upsert_oauth_for_reauth(payload, "kimi-reauth-1").expect("reauth");
        assert_eq!(updated.access_token, "new-access");
        assert_eq!(updated.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(updated.status.as_deref(), Some("active"));
    }

    #[test]
    fn accounts_index_path_points_under_data_dir() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = DataDirGuard::new("index-path");
        let path = accounts_index_path_string().expect("index path");
        assert!(path.contains("kimi_accounts.json"));
        assert!(path.starts_with(dir.dir.to_string_lossy().as_ref()));
    }

    #[test]
    fn list_accounts_checked_hides_tokens_in_views() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = DataDirGuard::new("list-hide");
        save_account(&sample_account(
            "kimi-list-1",
            "hidden-access",
            "hidden-refresh",
        ))
        .expect("save");
        let views = list_accounts_checked().expect("list");
        assert_eq!(views.len(), 1);
        assert!(views[0].access_token.is_empty());
    }

    #[test]
    fn parse_official_models_adds_k3_efforts() {
        let payload = serde_json::json!({
            "data": [{
                "id": "k3",
                "display_name": "K3",
                "context_length": 262144,
                "supports_image_in": true,
                "supports_video_in": false
            }]
        });
        let models = parse_official_models(&payload).expect("parse");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].support_efforts, vec!["low", "high", "max"]);
        assert_eq!(models[0].default_effort.as_deref(), Some("high"));
    }

    #[test]
    fn apply_config_stale_managed_models_removed_on_reapply() {
        let home = isolated_kimi_home("stale");
        let stale_payload = serde_json::json!({
            "data": [{
                "id": "old-model",
                "context_length": 1000,
                "supports_image_in": false,
                "supports_video_in": false
            }]
        });
        let stale_models = parse_official_models(&stale_payload).expect("stale parse");
        apply_official_managed_config(&home, &stale_models).expect("apply stale");
        let fresh_models = sample_models();
        apply_official_managed_config(&home, &fresh_models).expect("apply fresh");
        let raw = std::fs::read_to_string(official_config_path(&home)).expect("read");
        assert!(!raw.contains("kimi-code/old-model"));
        assert!(raw.contains("kimi-code/kimi-for-coding"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn mark_current_without_official_write_skips_credentials() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _home = KimiHomeGuard::new("no-sync");
        let _dir = DataDirGuard::new("no-sync-data");
        let account = sample_account("kimi-no-sync", "access-no-sync", "refresh-no-sync");
        save_account(&account).expect("save");
        let email = mark_current_without_official_write(&account.id).expect("mark");
        assert_eq!(email, account.email);
        assert_eq!(
            current_account_id().expect("current").as_deref(),
            Some(account.id.as_str())
        );
        assert!(!official_credentials_path(&_home.dir).exists());
    }

    #[test]
    fn quota_remaining_metrics_from_weekly_and_limits() {
        let mut view = KimiAccountView::from(&sample_account("kimi-q", "a", "r"));
        view.quota = Some(KimiQuota {
            weekly_used: Some(80.0),
            weekly_limit: Some(100.0),
            weekly_reset_at: None,
            limits: vec![KimiUsageRow {
                name: Some("daily".to_string()),
                window_unit: None,
                window_duration: None,
                used: 1.0,
                limit: 4.0,
                reset_at: None,
            }],
            booster_balance_cents: None,
            booster_total_cents: None,
            booster_currency: None,
            user_level_name: None,
            region: None,
        });
        let metrics = quota_remaining_metrics(&view);
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0], ("weekly".to_string(), 20));
        assert_eq!(metrics[1], ("daily".to_string(), 75));
    }
}
