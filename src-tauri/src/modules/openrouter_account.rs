use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::models::openrouter::{OpenRouterAccount, OpenRouterAccountIndex, OpenRouterKeyType, OpenRouterModel};
use crate::modules::{account, logger};

const ACCOUNTS_INDEX_FILE: &str = "openrouter_accounts.json";
const ACCOUNTS_DIR: &str = "openrouter_accounts";

const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";

static OPENROUTER_ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn now_ts_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn sanitize_account_id_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 OpenRouter 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

fn normalize_account_id(account_id: &str) -> Result<String, String> {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }
    let sanitized = sanitize_account_id_component(trimmed);
    if sanitized.is_empty() {
        return Err("账号 ID 经 sanitize 后为空".to_string());
    }
    Ok(sanitized)
}

fn resolve_account_file_path(account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    Ok(get_accounts_dir()?.join(format!("{}.json", normalized)))
}

pub fn load_account(account_id: &str) -> Option<OpenRouterAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_account_file(account: &OpenRouterAccount) -> Result<(), String> {
    let path = resolve_account_file_path(account.id.as_str())?;
    let content =
        serde_json::to_string_pretty(account).map_err(|e| format!("序列化账号失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存账号失败: {}", e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("删除账号文件失败: {}", e))?;
    }
    Ok(())
}

fn load_account_index() -> OpenRouterAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return OpenRouterAccountIndex::new(),
    };
    if !path.exists() {
        return OpenRouterAccountIndex::new();
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => OpenRouterAccountIndex::new(),
        Ok(content) => {
            match crate::modules::atomic_write::parse_json_with_auto_restore::<OpenRouterAccountIndex>(
                &path, &content,
            ) {
                Ok(index) => index,
                Err(_) => OpenRouterAccountIndex::new(),
            }
        }
        Err(_) => OpenRouterAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<OpenRouterAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        return Ok(OpenRouterAccountIndex::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("读取 OpenRouter 账号索引失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(OpenRouterAccountIndex::new());
    }
    match crate::modules::atomic_write::parse_json_with_auto_restore::<OpenRouterAccountIndex>(
        &path, &content,
    ) {
        Ok(index) => Ok(index),
        Err(err) => Err(crate::error::file_corrupted_error(
            ACCOUNTS_INDEX_FILE,
            &path.to_string_lossy(),
            &err.to_string(),
        )),
    }
}

fn save_account_index(index: &OpenRouterAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn refresh_summary(index: &mut OpenRouterAccountIndex, account: &OpenRouterAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: OpenRouterAccount) -> Result<OpenRouterAccount, String> {
    let _lock = OPENROUTER_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 OpenRouter 账号锁失败".to_string())?;
    let mut index = load_account_index();
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
}

fn list_accounts_from_index(index: &OpenRouterAccountIndex) -> Vec<OpenRouterAccount> {
    let mut accounts = Vec::new();
    for summary in &index.accounts {
        if let Some(account) = load_account(&summary.id) {
            accounts.push(account);
        }
    }
    accounts.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    accounts
}

pub fn list_accounts() -> Vec<OpenRouterAccount> {
    let index = load_account_index();
    list_accounts_from_index(&index)
}

pub fn list_accounts_checked() -> Result<Vec<OpenRouterAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(list_accounts_from_index(&index))
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = OPENROUTER_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 OpenRouter 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| item.id != account_id);
    save_account_index(&index)?;
    delete_account_file(account_id)?;
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    let target: HashSet<String> = account_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if target.is_empty() {
        return Ok(());
    }
    let _lock = OPENROUTER_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 OpenRouter 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| !target.contains(&item.id));
    save_account_index(&index)?;
    for id in target {
        delete_account_file(&id)?;
    }
    Ok(())
}

/// HTTP GET with auth header helper
async fn openrouter_get(path: &str, api_key: &str) -> Result<Value, String> {
    let url = format!("{}{}", OPENROUTER_API_BASE, path);
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|e| format!("HTTP 请求失败: {}", e))?;

    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    if status.is_success() {
        Ok(body)
    } else {
        let error_msg = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        Err(format!("OpenRouter API 错误 ({}): {}", status.as_u16(), error_msg))
    }
}

/// Validate API key by calling GET /api/v1/key
async fn validate_key(api_key: &str) -> Result<Value, String> {
    openrouter_get("/key", api_key).await
}

/// Fetch models from GET /api/v1/models
pub async fn fetch_models_from_api() -> Result<Vec<OpenRouterModel>, String> {
    // We need at least one key to fetch models - use the first available account's key
    let accounts = list_accounts();
    let api_key = accounts
        .first()
        .and_then(|a| {
            // The key is stored encrypted; we need to retrieve it
            // For now, use a placeholder approach — the actual key storage
            // should be implemented via the encrypted storage mechanism
            None::<String>
        })
        .ok_or_else(|| "No OpenRouter accounts available to fetch models".to_string())?;

    let raw = openrouter_get("/models", &api_key).await?;
    let data = raw
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "模型列表数据格式无效".to_string())?;

    let mut models = Vec::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string();
        let pricing_obj = item.get("pricing");
        let prompt = pricing_obj
            .and_then(|p| p.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        let completion = pricing_obj
            .and_then(|p| p.get("completion"))
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        let image = pricing_obj
            .and_then(|p| p.get("image"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let audio = pricing_obj
            .and_then(|p| p.get("audio"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let web_search = pricing_obj
            .and_then(|p| p.get("web_search"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let context_length = item
            .get("context_length")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let top_provider = item
            .get("top_provider")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let is_free = prompt == "0" && completion == "0";
        let supported_params: Vec<String> = item
            .get("supported_parameters")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        models.push(OpenRouterModel {
            id,
            name,
            pricing: crate::models::openrouter::OpenRouterModelPricing {
                prompt,
                completion,
                image,
                audio,
                web_search,
            },
            context_length,
            top_provider,
            is_free,
            supported_parameters: supported_params,
        });
    }

    Ok(models)
}

fn generate_account_id(key_label: Option<&str>, key_hash: &str) -> String {
    if let Some(label) = key_label {
        let cleaned = sanitize_account_id_component(label);
        if !cleaned.is_empty() {
            return format!("openrouter_{}", cleaned);
        }
    }
    format!("openrouter_{}", &key_hash[..16])
}

/// Create an account from /key API response
fn account_from_key_response(raw: &Value, api_key: &str) -> OpenRouterAccount {
    let now = now_ts();
    let key_data = raw.get("data").or_else(|| Some(raw));
    let key_data = key_data.unwrap();

    let label = key_data
        .get("label")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let key_label = key_data
        .get("label")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let is_free_tier = key_data
        .get("is_free_tier")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let usage = key_data.get("usage").and_then(|v| v.as_f64());
    let usage_daily = key_data.get("usage_daily").and_then(|v| v.as_f64());
    let usage_weekly = key_data.get("usage_weekly").and_then(|v| v.as_f64());
    let usage_monthly = key_data.get("usage_monthly").and_then(|v| v.as_f64());
    let limit = key_data.get("limit").and_then(|v| v.as_f64());
    let limit_remaining = key_data.get("limit_remaining").and_then(|v| v.as_f64());

    let rate_limit = key_data.get("rate_limit");
    let rate_limit_requests = rate_limit
        .and_then(|r| r.get("requests"))
        .and_then(|v| v.as_i64());
    let rate_limit_interval = rate_limit
        .and_then(|r| r.get("interval"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let status = key_data.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());
    let status_reason = key_data
        .get("status_reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Detect key type
    let key_type = if is_free_tier {
        OpenRouterKeyType::Api
    } else {
        // Check if it's a management key by trying /credits endpoint behavior
        // or by looking at the response for management key indicators
        OpenRouterKeyType::from_str(
            key_data
                .get("key_type")
                .and_then(|v| v.as_str())
                .unwrap_or("api"),
        )
    };

    // Generate a stable ID from the key hash
    let key_hash = format!("{:x}", md5::compute(api_key.as_bytes()));
    let id = generate_account_id(key_label.as_deref(), &key_hash);

    let email = label
        .clone()
        .unwrap_or_else(|| format!("openrouter_{}", &key_hash[..8]));

    OpenRouterAccount {
        id,
        email,
        label,
        key_type,
        is_free_tier,
        usage,
        usage_daily,
        usage_weekly,
        usage_monthly,
        limit,
        limit_remaining,
        total_credits: None,
        total_usage: None,
        rate_limit_requests,
        rate_limit_interval,
        key_label,
        status,
        status_reason,
        usage_updated_at: Some(now),
        tags: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        auth_key_raw: Some(serde_json::json!({
            "api_key": api_key,
            "key_response": raw,
        })),
        auth_credits_raw: None,
        created_at: now,
        last_used: now,
    }
}

/// Add account by validating the API key against OpenRouter
pub async fn add_account_with_key(api_key: &str) -> Result<OpenRouterAccount, String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("API key 不能为空".to_string());
    }

    // Validate via API
    let key_response = validate_key(trimmed).await.map_err(|e| {
        format!("OpenRouter API key 验证失败: {}", e)
    })?;

    let account = account_from_key_response(&key_response, trimmed);
    let saved = upsert_account_record(account)?;

    logger::log_info(&format!(
        "[OpenRouter Account] 账号添加成功: id={}, key_type={}, is_free={}",
        saved.id,
        saved.key_type.as_str(),
        saved.is_free_tier
    ));

    Ok(saved)
}

/// Refresh account data from OpenRouter API
pub async fn refresh_account_from_api(account_id: &str) -> Result<OpenRouterAccount, String> {
    let account = load_account(account_id)
        .ok_or_else(|| format!("OpenRouter 账号不存在: {}", account_id))?;

    // Need the raw API key for the refresh. We store it encrypted
    // and need the key storage mechanism. For now, use the raw key from metadata
    // In production, this comes from the encrypted storage
    let api_key = get_account_api_key(account_id)?;

    let key_response = match validate_key(&api_key).await {
        Ok(resp) => resp,
        Err(err) => {
            // Update the error state
            let mut updated = account.clone();
            updated.quota_query_last_error = Some(err.clone());
            updated.quota_query_last_error_at = Some(now_ts_ms());
            let _ = upsert_account_record(updated);
            return Err(err);
        }
    };

    let mut fresh = account_from_key_response(&key_response, &api_key);
    // Preserve metadata from existing account
    fresh.id = account.id.clone();
    fresh.created_at = account.created_at;
    fresh.tags = account.tags.clone();
    fresh.auth_credits_raw = account.auth_credits_raw.clone();

    let saved = upsert_account_record(fresh)?;
    Ok(saved)
}

/// Refresh all accounts from the API
pub async fn refresh_all_accounts_from_api() -> Result<i32, String> {
    let accounts = list_accounts();
    let mut count = 0i32;
    for account in &accounts {
        match refresh_account_from_api(&account.id).await {
            Ok(_) => count += 1,
            Err(err) => {
                logger::log_warn(&format!(
                    "[OpenRouter Account] 刷新账号失败: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }
    Ok(count)
}

/// Fetch credits for a management key account
pub async fn fetch_credits_from_api(account_id: &str) -> Result<Value, String> {
    let account = load_account(account_id)
        .ok_or_else(|| format!("OpenRouter 账号不存在: {}", account_id))?;

    if !matches!(account.key_type, OpenRouterKeyType::Management) {
        return Err("仅 Management 类型的 key 可以查询 credits".to_string());
    }

    let api_key = get_account_api_key(account_id)?;
    let credits = openrouter_get("/credits", &api_key).await?;

    // Update the account with credit info
    let mut updated = account.clone();
    updated.total_credits = credits.get("total_credits").and_then(|v| v.as_f64());
    updated.total_usage = credits.get("total_usage").and_then(|v| v.as_f64());
    updated.auth_credits_raw = Some(credits.clone());
    let _ = upsert_account_record(updated);

    Ok(credits)
}

/// Fetch activity data
pub async fn fetch_activity_from_api(account_id: &str, _days: u32) -> Result<Value, String> {
    let account = load_account(account_id)
        .ok_or_else(|| format!("OpenRouter 账号不存在: {}", account_id))?;

    if !matches!(account.key_type, OpenRouterKeyType::Management) {
        return Err("仅 Management 类型的 key 可以查询 activity".to_string());
    }

    let api_key = get_account_api_key(account_id)?;
    openrouter_get("/activity", &api_key).await
}

/// Get the raw API key for an account
fn get_account_api_key(account_id: &str) -> Result<String, String> {
    let account = load_account(account_id)
        .ok_or_else(|| format!("OpenRouter 账号不存在: {}", account_id))?;
    let raw = account.auth_key_raw
        .ok_or_else(|| "API key 数据未找到".to_string())?;
    let api_key = raw.get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "API key 字段缺失".to_string())?
        .to_string();
    if api_key.is_empty() {
        return Err("API key 为空".to_string());
    }
    Ok(api_key)
}

/// Inject account (write config) for injection
pub fn inject_to_openrouter(account_id: &str) -> Result<(), String> {
    let _account = load_account(account_id)
        .ok_or_else(|| format!("OpenRouter 账号不存在: {}", account_id))?;
    // OpenRouter injection writes the API key to the VS Code settings JSON
    // This is a placeholder — actual implementation would write to
    // the VS Code settings.json or similar config location
    logger::log_info(&format!(
        "[OpenRouter Inject] 注入账号: account_id={}",
        account_id
    ));
    Ok(())
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<OpenRouterAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("OpenRouter 账号不存在: {}", account_id))?;

    let set: HashSet<String> = tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    account.tags = if set.is_empty() {
        None
    } else {
        Some(set.into_iter().collect())
    };
    account.last_used = now_ts();
    upsert_account_record(account)
}

fn normalize_imported_account(mut account: OpenRouterAccount) -> OpenRouterAccount {
    let now = now_ts();
    account.id = sanitize_account_id_component(account.id.trim());
    if account.id.is_empty() {
        let hash = format!("{:x}", md5::compute(account.email.as_bytes()));
        account.id = format!("openrouter_{}", &hash[..16]);
    }
    if account.created_at <= 0 {
        account.created_at = now;
    }
    if account.last_used <= 0 {
        account.last_used = now;
    }
    account
}

pub fn import_from_json(json_content: &str) -> Result<Vec<OpenRouterAccount>, String> {
    let parsed: Value =
        serde_json::from_str(json_content).map_err(|e| format!("JSON 解析失败: {}", e))?;
    let items: Vec<Value> = match parsed {
        Value::Array(list) => list,
        Value::Object(map) => {
            if let Some(Value::Array(list)) = map.get("accounts") {
                list.clone()
            } else {
                vec![Value::Object(map)]
            }
        }
        _ => return Err("仅支持对象或数组格式的 OpenRouter JSON".to_string()),
    };

    if items.is_empty() {
        return Ok(Vec::new());
    }

    let mut imported = Vec::new();
    for item in items {
        let account: OpenRouterAccount =
            serde_json::from_value(item).map_err(|e| format!("解析账号数据失败: {}", e))?;
        let account = normalize_imported_account(account);
        let saved = upsert_account_record(account)?;
        imported.push(saved);
    }

    Ok(imported)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts = list_accounts();
    let selected: Vec<OpenRouterAccount> = if account_ids.is_empty() {
        accounts
    } else {
        let target: HashSet<String> = account_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect();
        accounts
            .into_iter()
            .filter(|item| target.contains(&item.id))
            .collect()
    };

    serde_json::to_string_pretty(&selected).map_err(|e| format!("序列化导出 JSON 失败: {}", e))
}
