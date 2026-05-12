use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::opencode::{
    OpenCodeAccount, OpenCodeAccountIndex, OpenCodeImportPayload, OpenCodeTier,
};
use crate::modules::{account, logger};

const ACCOUNTS_INDEX_FILE: &str = "opencode_accounts.json";
const ACCOUNTS_DIR: &str = "opencode_accounts";
const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go/v1/";
const OPENCODE_ZEN_BASE_URL: &str = "https://opencode.ai/zen/v1/";

lazy_static::lazy_static! {
    static ref OPENCODE_ACCOUNT_INDEX_LOCK: Mutex<()> = Mutex::new(());
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------------------------------------------------------------------------
// Data directory
// ---------------------------------------------------------------------------

fn data_dir() -> Result<PathBuf, String> {
    account::accounts_dir().map(|d| d.join(ACCOUNTS_DIR))
}

fn accounts_index_path() -> Result<PathBuf, String> {
    data_dir().map(|d| d.join(ACCOUNTS_INDEX_FILE))
}

pub(crate) fn accounts_index_path_string() -> Result<String, String> {
    accounts_index_path().map(|p| p.to_string_lossy().to_string())
}

fn account_file_path(account_id: &str) -> Result<PathBuf, String> {
    data_dir().map(|d| d.join(format!("{}.json", account_id)))
}

// ---------------------------------------------------------------------------
// Check API access — validate token by hitting models endpoint
// ---------------------------------------------------------------------------

fn check_api_access(tier: &OpenCodeTier, token: &str) -> Result<(), String> {
    let base_url = match tier {
        OpenCodeTier::Go => OPENCODE_GO_BASE_URL,
        OpenCodeTier::Zen => OPENCODE_ZEN_BASE_URL,
        OpenCodeTier::Free => return Ok(()), // Free tier needs no validation
    };

    let url = format!("{}models", base_url);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .map_err(|e| format!("API request failed: {}", e))?;

    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        Err(format!(
            "API returned status {}: {}",
            status,
            response.text().unwrap_or_default()
        ))
    }
}

// ---------------------------------------------------------------------------
// Fetch usage/quota data
// ---------------------------------------------------------------------------

fn fetch_go_usage(token: &str) -> Result<Option<serde_json::Value>, String> {
    let url = format!("{}models", OPENCODE_GO_BASE_URL);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .map_err(|e| format!("Go usage fetch failed: {}", e))?;

    if !response.status().is_success() {
        logger::log_warn(&format!(
            "[OpenCode] Go usage fetch returned status {}",
            response.status()
        ));
        return Ok(None);
    }

    // Parse rate limit headers for dollar-value tracking
    let usage_5h_str = response
        .headers()
        .get("x-ratelimit-usage-5h")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok());
    let usage_weekly_str = response
        .headers()
        .get("x-ratelimit-usage-weekly")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok());
    let usage_monthly_str = response
        .headers()
        .get("x-ratelimit-usage-monthly")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok());

    let limit_5h_str = response
        .headers()
        .get("x-ratelimit-limit-5h")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok());
    let limit_weekly_str = response
        .headers()
        .get("x-ratelimit-limit-weekly")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok());
    let limit_monthly_str = response
        .headers()
        .get("x-ratelimit-limit-monthly")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok());

    if usage_5h_str.is_none() && usage_monthly_str.is_none() {
        return Ok(None);
    }

    let mut usage = serde_json::Map::new();
    if let Some(v) = usage_5h_str {
        usage.insert("usage_5h_dollars".to_string(), serde_json::json!(v));
    }
    if let Some(v) = usage_weekly_str {
        usage.insert("usage_weekly_dollars".to_string(), serde_json::json!(v));
    }
    if let Some(v) = usage_monthly_str {
        usage.insert("usage_monthly_dollars".to_string(), serde_json::json!(v));
    }
    if let Some(v) = limit_5h_str {
        usage.insert("limit_5h".to_string(), serde_json::json!(v));
    } else {
        usage.insert("limit_5h".to_string(), serde_json::json!(12.0));
    }
    if let Some(v) = limit_weekly_str {
        usage.insert("limit_weekly".to_string(), serde_json::json!(v));
    } else {
        usage.insert("limit_weekly".to_string(), serde_json::json!(30.0));
    }
    if let Some(v) = limit_monthly_str {
        usage.insert("limit_monthly".to_string(), serde_json::json!(v));
    } else {
        usage.insert("limit_monthly".to_string(), serde_json::json!(60.0));
    }

    Ok(Some(serde_json::Value::Object(usage)))
}

fn fetch_zen_usage(_token: &str) -> Result<Option<serde_json::Value>, String> {
    // Zen doesn't have a public balance API via REST.
    // Balance info is only available from the console at https://opencode.ai/console.
    // We return a minimal placeholder with zero balance and auto-reload flag.
    // Users should check their balance via the console URL.
    let usage = serde_json::json!({
        "balance_dollars": 0.0,
        "auto_reload_enabled": false,
        "monthly_spend_limit": null,
        "console_url": "https://opencode.ai/console"
    });
    Ok(Some(usage))
}

async fn fetch_go_usage_async(token: &str) -> Result<Option<serde_json::Value>, String> {
    // Run the blocking call in a blocking thread pool
    let token = token.to_string();
    tokio::task::spawn_blocking(move || fetch_go_usage(&token))
        .await
        .map_err(|e| format!("Async task failed: {}", e))?
}

async fn fetch_zen_usage_async(token: &str) -> Result<Option<serde_json::Value>, String> {
    let token = token.to_string();
    tokio::task::spawn_blocking(move || fetch_zen_usage(&token))
        .await
        .map_err(|e| format!("Async task failed: {}", e))?
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

fn ensure_data_dir() -> Result<PathBuf, String> {
    let dir = data_dir()?;
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create data dir: {}", e))?;
    Ok(dir)
}

fn read_index() -> Result<OpenCodeAccountIndex, String> {
    let path = accounts_index_path()?;
    if !path.exists() {
        return Ok(OpenCodeAccountIndex::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read index: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse index: {}", e))
}

fn write_index(index: &OpenCodeAccountIndex) -> Result<(), String> {
    let path = accounts_index_path()?;
    let content = serde_json::to_string_pretty(index)
        .map_err(|e| format!("Failed to serialize index: {}", e))?;
    fs::write(&path, content).map_err(|e| format!("Failed to write index: {}", e))
}

pub(crate) fn list_accounts_checked() -> Result<Vec<OpenCodeAccount>, String> {
    let _lock = OPENCODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let index = read_index()?;
    let mut accounts = Vec::new();
    for summary in &index.accounts {
        if let Some(account) = load_account(&summary.id) {
            accounts.push(account);
        }
    }
    Ok(accounts)
}

pub(crate) fn load_account(account_id: &str) -> Option<OpenCodeAccount> {
    let path = account_file_path(account_id).ok()?;
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_account(account: &OpenCodeAccount) -> Result<(), String> {
    let path = account_file_path(&account.id)?;
    let content = serde_json::to_string_pretty(account)
        .map_err(|e| format!("Failed to serialize account: {}", e))?;
    fs::write(&path, content).map_err(|e| format!("Failed to write account: {}", e))
}

pub(crate) fn upsert_account(payload: OpenCodeImportPayload) -> Result<OpenCodeAccount, String> {
    let _lock = OPENCODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    ensure_data_dir()?;

    // Validate API key by checking access (skip for free tier)
    match &payload.tier {
        OpenCodeTier::Free => {}
        _ => {
            check_api_access(&payload.tier, &payload.access_token).map_err(|e| {
                format!("API key validation failed: {}", e)
            })?;
        }
    }

    let mut index = read_index()?;
    let now = now_ts();

    let account = OpenCodeAccount {
        id: uuid::Uuid::new_v4().to_string(),
        email: payload.email,
        name: payload.name,
        tags: None,
        access_token: payload.access_token,
        tier: payload.tier,
        plan_name: payload.plan_name,
        subscription_status: payload.subscription_status,
        usage_raw: payload.usage_raw,
        status: payload.status,
        status_reason: payload.status_reason,
        created_at: now,
        last_used: now,
    };

    // Mark previous accounts that share the same email as non-current
    for summary in &mut index.accounts {
        if summary.email == account.email {
            // Keep existing, just update last_used
        }
    }

    save_account(&account)?;
    index.accounts.push(account.summary());
    write_index(&index)?;

    logger::log_info(&format!(
        "[OpenCode] 创建账号: id={}, email={}, tier={}",
        account.id, account.email, account.tier
    ));

    Ok(account)
}

pub(crate) fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = OPENCODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let mut index = read_index()?;
    index.accounts.retain(|a| a.id != account_id);
    write_index(&index)?;

    // Delete the account file
    if let Ok(path) = account_file_path(account_id) {
        let _ = fs::remove_file(path);
    }

    logger::log_info(&format!("[OpenCode] 删除账号: id={}", account_id));
    Ok(())
}

pub(crate) fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        remove_account(id)?;
    }
    Ok(())
}

pub(crate) fn update_account_tags(
    account_id: &str,
    tags: Vec<String>,
) -> Result<OpenCodeAccount, String> {
    let _lock = OPENCODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let mut account = load_account(account_id)
        .ok_or_else(|| format!("OpenCode account not found: {}", account_id))?;
    let clean_tags: Vec<String> = tags.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    account.tags = if clean_tags.is_empty() {
        None
    } else {
        Some(clean_tags)
    };
    account.last_used = now_ts();
    save_account(&account)?;
    Ok(account)
}

// ---------------------------------------------------------------------------
// Import / Export
// ---------------------------------------------------------------------------

pub(crate) fn import_from_json(json_content: &str) -> Result<Vec<OpenCodeAccount>, String> {
    #[derive(Deserialize)]
    struct ImportItem {
        access_token: String,
        #[serde(default = "default_email")]
        email: String,
        #[serde(default)]
        tier: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        plan_name: Option<String>,
        #[serde(default)]
        subscription_status: Option<String>,
        #[serde(default)]
        usage_raw: Option<serde_json::Value>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        status_reason: Option<String>,
    }

    fn default_email() -> String {
        "unknown".to_string()
    }

    let items: Vec<ImportItem> = serde_json::from_str(json_content)
        .map_err(|e| format!("JSON parse error: {}", e))?;

    let mut accounts = Vec::new();
    for item in items {
        let tier = match item.tier.as_deref().unwrap_or("go") {
            "zen" => OpenCodeTier::Zen,
            "free" => OpenCodeTier::Free,
            _ => OpenCodeTier::Go,
        };

        let payload = OpenCodeImportPayload {
            email: item.email,
            name: item.name,
            access_token: item.access_token,
            tier,
            plan_name: item.plan_name,
            subscription_status: item.subscription_status,
            usage_raw: item.usage_raw,
            status: item.status,
            status_reason: item.status_reason,
        };
        accounts.push(upsert_account(payload)?);
    }
    Ok(accounts)
}

pub(crate) fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    #[derive(Serialize)]
    struct ExportItem {
        access_token: String,
        email: String,
        tier: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tags: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        plan_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        subscription_status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage_raw: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status_reason: Option<String>,
    }

    let _lock = OPENCODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let mut items = Vec::new();
    for id in account_ids {
        if let Some(account) = load_account(id) {
            items.push(ExportItem {
                access_token: account.access_token,
                email: account.email,
                tier: account.tier.to_string(),
                name: account.name,
                tags: account.tags,
                plan_name: account.plan_name,
                subscription_status: account.subscription_status,
                usage_raw: account.usage_raw,
                status: account.status,
                status_reason: account.status_reason,
            });
        }
    }

    serde_json::to_string_pretty(&items)
        .map_err(|e| format!("Failed to serialize export: {}", e))
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

pub(crate) async fn refresh_account_async(
    account_id: &str,
) -> Result<OpenCodeAccount, String> {
    let _lock = OPENCODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let mut account = load_account(account_id)
        .ok_or_else(|| format!("OpenCode account not found: {}", account_id))?;

    let usage = match &account.tier {
        OpenCodeTier::Go => fetch_go_usage_async(&account.access_token).await?,
        OpenCodeTier::Zen => fetch_zen_usage_async(&account.access_token).await?,
        OpenCodeTier::Free => None,
    };

    if let Some(ref usage_data) = usage {
        account.usage_raw = Some(usage_data.clone());
    }

    account.last_used = now_ts();
    save_account(&account)?;

    // Update index
    let mut index = read_index()?;
    if let Some(summary) = index.accounts.iter_mut().find(|a| a.id == account_id) {
        *summary = account.summary();
    }
    write_index(&index)?;

    Ok(account)
}

pub(crate) async fn refresh_all_tokens(
) -> Result<Vec<(String, Result<OpenCodeAccount, String>)>, String> {
    let accounts = list_accounts_checked()?;
    let mut results = Vec::new();

    for account in accounts {
        let result = refresh_account_async(&account.id).await;
        results.push((account.id, result));
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Inject to VS Code
// ---------------------------------------------------------------------------

pub(crate) fn inject_to_opencode(account_id: &str) -> Result<(), String> {
    let account = load_account(account_id)
        .ok_or_else(|| format!("OpenCode account not found: {}", account_id))?;

    // For OpenCode, injection writes the API key to VS Code settings
    // Similar to how other token-based providers inject their credentials
    let config = crate::modules::config::get_user_config();

    let vscode_path = config.vscode_app_path.trim();
    if vscode_path.is_empty() {
        return Err("APP_PATH_NOT_FOUND:vscode".to_string());
    }

    let base_url = match &account.tier {
        OpenCodeTier::Go => OPENCODE_GO_BASE_URL,
        OpenCodeTier::Zen => OPENCODE_ZEN_BASE_URL,
        OpenCodeTier::Free => return Err("Free tier cannot be injected".to_string()),
    };

    crate::modules::vscode_inject::inject_settings_json(&crate::modules::vscode_paths::VSCodeSettingsInjection {
        vscode_path: vscode_path.to_string(),
        settings: serde_json::json!({
            "opencode.api_key": account.access_token,
            "opencode.base_url": base_url,
            "opencode.tier": account.tier.to_string(),
        }),
    })?;

    logger::log_info(&format!(
        "[OpenCode] 已注入 VS Code 配置: account_id={}, email={}, tier={}",
        account.id, account.email, account.tier
    ));

    Ok(())
}
