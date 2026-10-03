//! Usage / quota lookup for pi account credentials.
//!
//! Expired tokens are refreshed and written back to the store and to every
//! live `auth.json`, since providers rotate refresh tokens.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, USER_AGENT};
use serde::Serialize;
use serde_json::Value;

use crate::models::pi::PiAccount;
use crate::modules::pi_account;

const TIMEOUT_SECS: u64 = 20;
const EXPIRED: &str = "TOKEN_EXPIRED";

#[derive(Debug, Clone, Serialize)]
pub struct PiUsageWindow {
    pub key: String,
    pub label: String,
    /// 0-100, percentage already used.
    pub used_percent: f64,
    /// Unix seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<i64>,
    /// Extra text such as `120 / 300`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PiProviderUsage {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub windows: Vec<PiUsageWindow>,
    /// Free-form balance text (OpenRouter credits).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
    /// Gateway usage summary (same shape as the Codex model-provider panel).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// true when this provider has no known usage endpoint.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unsupported: bool,
}

impl PiProviderUsage {
    fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_string(),
            plan: None,
            windows: Vec::new(),
            balance: None,
            gateway: None,
            error: None,
            unsupported: false,
        }
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))
}

fn str_field<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn is_expired(entry: &Value) -> bool {
    entry
        .get("expires")
        .and_then(Value::as_i64)
        .is_some_and(|ms| ms <= chrono::Utc::now().timestamp_millis())
}

async fn get_json(request: reqwest::RequestBuilder) -> Result<Value, String> {
    let response = request
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(format!("{}: HTTP {}", EXPIRED, status.as_u16()));
    }
    if !status.is_success() {
        let snippet: String = body.chars().take(300).collect();
        return Err(format!("HTTP {}: {}", status.as_u16(), snippet));
    }
    serde_json::from_str(&body).map_err(|e| format!("解析响应失败: {}", e))
}

fn parse_time(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n
            .as_i64()
            .map(|v| if v > 10_000_000_000 { v / 1000 } else { v }),
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.timestamp())
            .ok()
            .or_else(|| {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|d| d.and_utc().timestamp())
            }),
        _ => None,
    }
}

fn clamp(v: f64) -> f64 {
    v.clamp(0.0, 100.0)
}

// ---- Anthropic (Claude subscription) ----

fn parse_anthropic(raw: &Value, out: &mut PiProviderUsage) {
    let windows = [
        ("five_hour", "5h"),
        ("seven_day", "7d"),
        ("seven_day_sonnet", "7d Sonnet"),
        ("seven_day_opus", "7d Opus"),
    ];
    for (key, label) in windows {
        let Some(item) = raw.get(key).filter(|v| v.is_object()) else {
            continue;
        };
        let Some(used) = item.get("utilization").and_then(Value::as_f64) else {
            continue;
        };
        out.windows.push(PiUsageWindow {
            key: key.to_string(),
            label: label.to_string(),
            used_percent: clamp(used),
            reset_at: parse_time(item.get("resets_at")),
            detail: None,
        });
    }
    if let Some(extra) = raw.get("extra_usage") {
        if extra.get("is_enabled").and_then(Value::as_bool) == Some(true) {
            if let Some(used) = extra.get("utilization").and_then(Value::as_f64) {
                out.windows.push(PiUsageWindow {
                    key: "extra_usage".to_string(),
                    label: "Extra".to_string(),
                    used_percent: clamp(used),
                    reset_at: None,
                    detail: None,
                });
            }
        }
    }
}

async fn query_anthropic(entry: &Value, out: &mut PiProviderUsage) -> Result<(), String> {
    let token = str_field(entry, "access").ok_or("缺少 access token")?;
    let raw = get_json(
        client()?
            .get("https://api.anthropic.com/api/oauth/usage")
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header(USER_AGENT, "pi-coding-agent"),
    )
    .await?;
    parse_anthropic(&raw, out);
    Ok(())
}

// ---- OpenAI Codex (ChatGPT subscription) ----

fn parse_codex(raw: &Value, out: &mut PiProviderUsage) {
    out.plan = str_field(raw, "plan_type").map(str::to_string);
    let Some(rate) = raw.get("rate_limit") else {
        return;
    };
    for (key, label) in [("primary_window", "5h"), ("secondary_window", "7d")] {
        let Some(item) = rate.get(key).filter(|v| v.is_object()) else {
            continue;
        };
        let Some(used) = item.get("used_percent").and_then(Value::as_f64) else {
            continue;
        };
        let reset_at = parse_time(item.get("reset_at")).or_else(|| {
            item.get("reset_after_seconds")
                .and_then(Value::as_i64)
                .map(|s| chrono::Utc::now().timestamp() + s)
        });
        let label = item
            .get("limit_window_seconds")
            .and_then(Value::as_i64)
            .map(|s| {
                if s >= 86400 {
                    format!("{}d", (s + 43200) / 86400)
                } else {
                    format!("{}h", (s + 1800) / 3600)
                }
            })
            .unwrap_or_else(|| label.to_string());
        out.windows.push(PiUsageWindow {
            key: key.to_string(),
            label,
            used_percent: clamp(used),
            reset_at,
            detail: None,
        });
    }
}

async fn query_codex(entry: &Value, out: &mut PiProviderUsage) -> Result<(), String> {
    let token = str_field(entry, "access").ok_or("缺少 access token")?;
    let mut request = client()?
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .header(USER_AGENT, "codex_cli_rs")
        .header("originator", "codex_cli_rs");
    if let Some(account_id) = str_field(entry, "accountId") {
        request = request.header("ChatGPT-Account-Id", account_id);
    }
    let raw = get_json(request).await?;
    parse_codex(&raw, out);
    Ok(())
}

// ---- GitHub Copilot ----

fn parse_copilot(raw: &Value, out: &mut PiProviderUsage) {
    out.plan = str_field(raw, "copilot_plan").map(str::to_string);
    let reset_at = parse_time(raw.get("quota_reset_date"));
    let Some(snapshots) = raw.get("quota_snapshots").and_then(Value::as_object) else {
        return;
    };
    let mut keys: Vec<&String> = snapshots.keys().collect();
    // Premium first, then chat / completions.
    keys.sort_by_key(|k| match k.as_str() {
        "premium_models" | "premium_interactions" => 0,
        "chat" => 1,
        "completions" => 2,
        _ => 3,
    });
    let mut seen_premium = false;
    for key in keys {
        let snap = &snapshots[key];
        if snap.get("unlimited").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let premium = key.starts_with("premium");
        if premium && seen_premium {
            continue;
        }
        let Some(remaining_pct) = snap.get("percent_remaining").and_then(Value::as_f64) else {
            continue;
        };
        seen_premium |= premium;
        let entitlement = snap.get("entitlement").and_then(Value::as_f64);
        let remaining = snap.get("remaining").and_then(Value::as_f64);
        let detail = match (remaining, entitlement) {
            (Some(r), Some(e)) if e > 0.0 => Some(format!("{} / {}", r.max(0.0) as i64, e as i64)),
            _ => None,
        };
        out.windows.push(PiUsageWindow {
            key: key.clone(),
            label: match key.as_str() {
                "premium_models" | "premium_interactions" => "Premium".to_string(),
                "chat" => "Chat".to_string(),
                "completions" => "Completions".to_string(),
                other => other.to_string(),
            },
            used_percent: clamp(100.0 - remaining_pct),
            reset_at,
            detail,
        });
    }
}

async fn query_copilot(entry: &Value, out: &mut PiProviderUsage) -> Result<(), String> {
    // The GitHub token (stored as `refresh`) does not expire like the Copilot token.
    let token = str_field(entry, "refresh").ok_or("缺少 GitHub token")?;
    let raw = get_json(
        client()?
            .get("https://api.github.com/copilot_internal/user")
            .header(AUTHORIZATION, format!("token {}", token))
            .header(USER_AGENT, "GitHubCopilotChat/0.35.0")
            .header("Accept", "application/json"),
    )
    .await?;
    parse_copilot(&raw, out);
    Ok(())
}

// ---- OpenRouter ----

async fn query_openrouter(key: &str, out: &mut PiProviderUsage) -> Result<(), String> {
    let raw = get_json(
        client()?
            .get("https://openrouter.ai/api/v1/key")
            .header(AUTHORIZATION, format!("Bearer {}", key)),
    )
    .await?;
    let data = raw.get("data").unwrap_or(&raw);
    let usage = data.get("usage").and_then(Value::as_f64).unwrap_or(0.0);
    match data.get("limit").and_then(Value::as_f64) {
        Some(limit) if limit > 0.0 => {
            out.windows.push(PiUsageWindow {
                key: "credits".to_string(),
                label: "Credits".to_string(),
                used_percent: clamp(usage / limit * 100.0),
                reset_at: None,
                detail: Some(format!("${:.2} / ${:.2}", usage, limit)),
            });
        }
        _ => out.balance = Some(format!("${:.2}", usage)),
    }
    if data.get("is_free_tier").and_then(Value::as_bool) == Some(true) {
        out.plan = Some("free".to_string());
    }
    Ok(())
}

// ---- Gateways ----

fn gateway_base_candidates(base_url: &str) -> Vec<String> {
    let base = base_url.trim().trim_end_matches('/').to_string();
    let mut out = vec![base.clone()];
    if let Ok(url) = url::Url::parse(&base) {
        let origin = url.origin().ascii_serialization();
        for candidate in [origin.clone(), format!("{}/v1", origin)] {
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

fn is_unavailable(error: &str) -> bool {
    error.contains("PROVIDER_USAGE_DETECT_FAILED")
        || error.contains("PROVIDER_USAGE_HTTP_404")
        || error.contains("PROVIDER_USAGE_TYPE_UNSUPPORTED")
}

async fn query_gateway(base_url: &str, key: &str, out: &mut PiProviderUsage) {
    let mut last_error = None;
    for candidate in gateway_base_candidates(base_url) {
        match crate::commands::codex::codex_query_model_provider_usage(
            candidate,
            key.to_string(),
            None,
        )
        .await
        {
            Ok(summary) => {
                out.gateway = serde_json::to_value(summary).ok();
                return;
            }
            Err(error) => {
                let unavailable = is_unavailable(&error);
                last_error = Some(error);
                if !unavailable {
                    break;
                }
            }
        }
    }
    match last_error {
        Some(error) if !is_unavailable(&error) => out.error = Some(error),
        _ => out.unsupported = true,
    }
}

/// Refresh expired Anthropic / Codex tokens and write them back (store and
/// live `auth.json`), so pi keeps working with the rotated refresh token.
fn needs_refresh(account: &PiAccount) -> bool {
    account.credentials.iter().any(|cred| {
        matches!(cred.provider.as_str(), "anthropic" | "openai-codex")
            && cred.kind() == "oauth"
            && is_expired(&cred.entry)
    })
}

/// One refresh at a time per account inside cockpit (auto refresh, manual
/// query and tray may overlap). Refresh tokens rotate, so a second concurrent
/// refresh with the same token would fail or lose the rotated one.
fn account_refresh_lock(account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.entry(account_id.to_string()).or_default().clone()
}

/// Refresh expired Anthropic / Codex tokens and write them back.
///
/// Holds every live `auth.json` lock (shared with pi) across the refresh, and
/// re-reads pi's tokens under it first: if pi already rotated the token we use
/// that instead of refreshing again.
async fn refresh_expired_locked(account_id: &str) -> Result<(), String> {
    let account_lock = account_refresh_lock(account_id);
    let _account_guard = account_lock.lock().await;

    let id = account_id.to_string();
    let file_locks = tokio::task::spawn_blocking(move || pi_account::lock_live_auth(&id))
        .await
        .map_err(|e| e.to_string())??;
    pi_account::sync_live_credentials_locked(account_id);
    let mut account =
        pi_account::load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    // pi stops honouring our lock after its stale window; bound the network.
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        refresh_expired(account_id, &mut account),
    )
    .await;
    drop(file_locks);
    result.map_err(|_| "刷新令牌超时".to_string())
}

/// Caller must hold [`pi_account::lock_live_auth`].
async fn refresh_expired(account_id: &str, account: &mut PiAccount) {
    for cred in account.credentials.iter_mut() {
        if !matches!(cred.provider.as_str(), "anthropic" | "openai-codex")
            || cred.kind() != "oauth"
            || !is_expired(&cred.entry)
        {
            continue;
        }
        let previous = str_field(&cred.entry, "refresh")
            .unwrap_or_default()
            .to_string();
        match crate::modules::pi_oauth::refresh_entry(&cred.provider, &cred.entry).await {
            Ok(Some(entry)) => {
                if let Err(e) = pi_account::save_refreshed_credential(
                    account_id,
                    &cred.provider,
                    &previous,
                    entry.clone(),
                ) {
                    crate::modules::logger::log_warn(&format!("[pi] 保存刷新令牌失败: {}", e));
                }
                cred.entry = entry;
            }
            Ok(None) => {}
            Err(e) => crate::modules::logger::log_warn(&format!(
                "[pi] 刷新 {} 令牌失败: {}",
                cred.provider, e
            )),
        }
    }
}

/// Query usage for every credential on `account_id`.
pub async fn query_account_usage(account_id: &str) -> Result<Vec<PiProviderUsage>, String> {
    let stale = pi_account::load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    if needs_refresh(&stale) {
        // Syncs pi's tokens under the lock before deciding to refresh.
        if let Err(e) = refresh_expired_locked(account_id).await {
            crate::modules::logger::log_warn(&format!("[pi] 刷新令牌跳过: {}", e));
        }
    } else {
        pi_account::sync_live_credentials(account_id);
    }
    let account: PiAccount =
        pi_account::load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    let mut results = Vec::new();
    for cred in &account.credentials {
        let mut out = PiProviderUsage::new(&cred.provider);
        let entry = &cred.entry;
        let gateway_base = account
            .custom_providers
            .iter()
            .find(|p| p.provider == cred.provider)
            .and_then(|p| str_field(&p.config, "baseUrl").map(str::to_string));

        if let Some(base_url) = gateway_base {
            match str_field(entry, "key") {
                Some(key) => query_gateway(&base_url, key, &mut out).await,
                None => out.unsupported = true,
            }
            results.push(out);
            continue;
        }

        let oauth = entry.get("type").and_then(Value::as_str) == Some("oauth");
        let result = match cred.provider.as_str() {
            "openrouter" => match str_field(entry, if oauth { "access" } else { "key" }) {
                Some(key) => {
                    let key = key.to_string();
                    query_openrouter(&key, &mut out).await
                }
                None => Err("缺少 key".to_string()),
            },
            "github-copilot" if oauth => query_copilot(entry, &mut out).await,
            "anthropic" if oauth => {
                if is_expired(entry) {
                    Err(EXPIRED.to_string())
                } else {
                    query_anthropic(entry, &mut out).await
                }
            }
            "openai-codex" if oauth => {
                if is_expired(entry) {
                    Err(EXPIRED.to_string())
                } else {
                    query_codex(entry, &mut out).await
                }
            }
            _ => {
                out.unsupported = true;
                Ok(())
            }
        };
        if let Err(error) = result {
            out.error = Some(error);
        }
        results.push(out);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_anthropic_usage() {
        let mut out = PiProviderUsage::new("anthropic");
        parse_anthropic(
            &json!({"five_hour":{"utilization":12.0,"resets_at":"2026-01-01T00:00:00Z"},"seven_day":{"utilization":40.0}}),
            &mut out,
        );
        assert_eq!(out.windows.len(), 2);
        assert_eq!(out.windows[0].used_percent, 12.0);
        assert!(out.windows[0].reset_at.is_some());
    }

    #[test]
    fn parses_copilot_snapshots() {
        let mut out = PiProviderUsage::new("github-copilot");
        parse_copilot(
            &json!({"copilot_plan":"individual","quota_reset_date":"2026-02-01","quota_snapshots":{
                "chat":{"unlimited":true},
                "premium_interactions":{"entitlement":300,"remaining":240,"percent_remaining":80.0}
            }}),
            &mut out,
        );
        assert_eq!(out.windows.len(), 1);
        assert_eq!(out.windows[0].used_percent, 20.0);
        assert_eq!(out.windows[0].detail.as_deref(), Some("240 / 300"));
    }

    #[test]
    fn gateway_candidates_include_origin() {
        let c = gateway_base_candidates("https://x.com/api/v1/");
        assert_eq!(
            c,
            vec!["https://x.com/api/v1", "https://x.com", "https://x.com/v1"]
        );
    }
}

/// Refresh tokens and usage for every pi account (web report scheduler).
pub async fn refresh_all_accounts() -> Result<(), String> {
    for account in pi_account::list_accounts()? {
        if let Err(e) = query_account_usage(&account.id).await {
            crate::modules::logger::log_warn(&format!("[pi] 刷新用量失败 {}: {}", account.id, e));
        }
    }
    Ok(())
}
