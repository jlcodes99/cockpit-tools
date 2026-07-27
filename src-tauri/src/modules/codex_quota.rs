use crate::models::codex::{CodexAccount, CodexQuota, CodexQuotaErrorInfo, CodexResetCredit};
use crate::modules::{codex_account, codex_agent_identity, logger};
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT,
};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;

// 使用 wham/usage 端点（Quotio 使用的）
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const RESET_CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const RESET_CREDITS_CONSUME_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";
const SUBSCRIPTION_ACCOUNTS_CHECK_URL: &str =
    "https://chatgpt.com/backend-api/accounts/check/v4-2023-04-27";
const SUBSCRIPTIONS_URL: &str = "https://chatgpt.com/backend-api/subscriptions";
const COCKPIT_API_PROVIDER_ID: &str = "cockpit_api";
const LEGACY_NEW_API_PROVIDER_ID: &str = "new_api";
const COCKPIT_API_PLAN_TYPE: &str = "Cockpit Api";
const LEGACY_NEW_API_EXCLUSIVE_PLAN_TYPE: &str = "NEW_API_EXCLUSIVE";
const COCKPIT_API_BASE_URL: &str = "https://chongcodex.cn/v1";
const CHATGPT_WEB_REFERER: &str = "https://chatgpt.com/";
const CHATGPT_WEB_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36";
const RESET_CREDITS_MOCK_JSON_ENV: &str = "CODEX_RESET_CREDITS_MOCK_JSON";
const SUBSCRIPTION_RETRY_INTERVAL_SECONDS: i64 = 30 * 60;
const HTTP_ERROR_BODY_DISPLAY_MAX_CHARS: usize = 4000;

fn get_header_value(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string()
}

fn extract_detail_code_from_body(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;

    if let Some(code) = value
        .get("detail")
        .and_then(|detail| detail.get("code"))
        .and_then(|code| code.as_str())
    {
        return Some(code.to_string());
    }

    if let Some(code) = value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(|code| code.as_str())
    {
        return Some(code.to_string());
    }

    if let Some(code) = value.get("code").and_then(|code| code.as_str()) {
        return Some(code.to_string());
    }

    None
}

fn normalize_http_error_body_for_display(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }

    let mut compact = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    let char_count = compact.chars().count();
    if char_count > HTTP_ERROR_BODY_DISPLAY_MAX_CHARS {
        compact = compact
            .chars()
            .take(HTTP_ERROR_BODY_DISPLAY_MAX_CHARS)
            .collect::<String>();
        compact.push_str("...(truncated)");
    }
    compact
}

fn append_http_error_diagnostics(message: &mut String, headers: &HeaderMap, body: &str) {
    message.push_str(&format!(
        " [request-id:{}] [x-request-id:{}] [cf-ray:{}] [body:{}]",
        get_header_value(headers, "request-id"),
        get_header_value(headers, "x-request-id"),
        get_header_value(headers, "cf-ray"),
        normalize_http_error_body_for_display(body)
    ));
}

fn extract_error_code_from_message(message: &str) -> Option<String> {
    let marker = "[error_code:";
    if let Some(start) = message.find(marker) {
        let code_start = start + marker.len();
        let end = message[code_start..].find(']')?;
        return Some(message[code_start..code_start + end].to_string());
    }

    let marker = "error_code=";
    let start = message.find(marker)?;
    let code_start = start + marker.len();
    let tail = &message[code_start..];
    let end = tail
        .find(|ch: char| ch == ',' || ch == ']' || ch.is_whitespace())
        .unwrap_or(tail.len());
    let code = tail[..end].trim();
    if code.is_empty() {
        None
    } else {
        Some(code.to_string())
    }
}

fn write_quota_error(account: &mut CodexAccount, message: String) {
    account.quota_error = Some(CodexQuotaErrorInfo {
        code: extract_error_code_from_message(&message),
        message,
        timestamp: chrono::Utc::now().timestamp(),
    });
}

/// 使用率窗口（5小时/周）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowInfo {
    #[serde(rename = "used_percent")]
    used_percent: Option<i32>,
    #[serde(rename = "limit_window_seconds")]
    limit_window_seconds: Option<i64>,
    #[serde(rename = "reset_after_seconds")]
    reset_after_seconds: Option<i64>,
    #[serde(rename = "reset_at")]
    reset_at: Option<i64>,
}

/// 速率限制信息
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RateLimitInfo {
    allowed: Option<bool>,
    #[serde(rename = "limit_reached")]
    limit_reached: Option<bool>,
    #[serde(rename = "primary_window")]
    primary_window: Option<WindowInfo>,
    #[serde(rename = "secondary_window")]
    secondary_window: Option<WindowInfo>,
}

/// 主动重置次数
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResetCreditsInfo {
    available_count: Option<i64>,
}

/// 使用率响应
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageResponse {
    #[serde(rename = "plan_type")]
    plan_type: Option<String>,
    #[serde(rename = "rate_limit")]
    rate_limit: Option<RateLimitInfo>,
    #[serde(rename = "code_review_rate_limit")]
    code_review_rate_limit: Option<RateLimitInfo>,
    #[serde(rename = "rate_limit_reset_credits")]
    rate_limit_reset_credits: Option<ResetCreditsInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexResetCreditsSnapshot {
    available_count: Option<i64>,
    credits: Vec<CodexResetCredit>,
    next_expires_at: Option<i64>,
}

fn normalize_remaining_percentage(window: &WindowInfo) -> Result<i32, String> {
    let used = window
        .used_percent
        .ok_or_else(|| "配额窗口缺少 used_percent".to_string())?;
    if !(0..=100).contains(&used) {
        return Err(format!("配额窗口 used_percent 超出范围: {}", used));
    }
    Ok(100 - used)
}

fn normalize_window_minutes(window: &WindowInfo) -> Option<i64> {
    let seconds = window.limit_window_seconds?;
    if seconds <= 0 {
        return None;
    }
    Some((seconds + 59) / 60)
}

fn normalize_reset_time(window: &WindowInfo) -> Option<i64> {
    if let Some(reset_at) = window.reset_at {
        return Some(reset_at);
    }

    let reset_after_seconds = window.reset_after_seconds?;
    if reset_after_seconds < 0 {
        return None;
    }

    Some(chrono::Utc::now().timestamp() + reset_after_seconds)
}

/// 配额查询结果（包含 plan_type）
pub struct FetchQuotaResult {
    pub quota: CodexQuota,
    pub plan_type: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RefreshQuotaOptions {
    pub force_subscription_refresh: bool,
}

#[derive(Debug, Clone, Copy)]
struct SubscriptionRefreshOptions {
    force: bool,
}

#[derive(Debug, Clone)]
struct SubscriptionStatusSnapshot {
    account_id: Option<String>,
    plan_type: Option<String>,
    subscription_active_until: Option<String>,
}

#[derive(Debug, Clone)]
struct AccountCheckRecord {
    key: Option<String>,
    node: serde_json::Value,
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn parse_reset_credit_timestamp_value(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::Number(number) => {
            let mut timestamp = number
                .as_i64()
                .or_else(|| number.as_u64().and_then(|raw| i64::try_from(raw).ok()))?;
            if timestamp > 1_000_000_000_000 {
                timestamp /= 1000;
            }
            Some(timestamp)
        }
        serde_json::Value::String(text) => parse_subscription_timestamp(text),
        _ => None,
    }
}

fn extract_reset_credit_timestamp(
    record: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<i64> {
    for key in keys {
        if let Some(timestamp) = parse_reset_credit_timestamp_value(record.get(*key)) {
            return Some(timestamp);
        }
    }
    None
}

fn extract_reset_credit_string(
    record: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    for key in keys {
        if let Some(value) = normalize_optional_json_scalar(record.get(*key)) {
            return Some(value);
        }
    }
    None
}

fn normalize_reset_credit_status(status: Option<&str>, expires_at: Option<i64>) -> Option<String> {
    let normalized = status.and_then(|value| normalize_optional_ref(Some(value)));
    if let Some(value) = normalized {
        return Some(value.to_ascii_lowercase());
    }

    if expires_at.is_some_and(|timestamp| timestamp <= now_timestamp()) {
        return Some("expired".to_string());
    }

    None
}

fn is_available_reset_credit(credit: &CodexResetCredit) -> bool {
    let status = credit
        .status
        .as_deref()
        .or(credit.raw_status.as_deref())
        .unwrap_or("available")
        .trim()
        .to_ascii_lowercase();
    if matches!(
        status.as_str(),
        "redeemed" | "used" | "consumed" | "expired"
    ) {
        return false;
    }

    credit
        .expires_at
        .map(|timestamp| timestamp > now_timestamp())
        .unwrap_or(true)
}

fn parse_reset_credit_record(value: &serde_json::Value) -> Option<CodexResetCredit> {
    let record = value.as_object()?;
    let raw_status = extract_reset_credit_string(record, &["status", "state"]);
    let expires_at =
        extract_reset_credit_timestamp(record, &["expires_at", "expire_at", "expiresAt"]);
    let status = normalize_reset_credit_status(raw_status.as_deref(), expires_at);

    Some(CodexResetCredit {
        id: extract_reset_credit_string(record, &["id", "credit_id", "creditId"]),
        status,
        reset_type: extract_reset_credit_string(record, &["type", "reset_type", "resetType"]),
        granted_at: extract_reset_credit_timestamp(
            record,
            &["granted_at", "created_at", "grantedAt"],
        ),
        expires_at,
        redeemed_at: extract_reset_credit_timestamp(
            record,
            &["redeemed_at", "used_at", "consumed_at", "redeemedAt"],
        ),
        raw_status,
    })
}

fn parse_reset_credits_snapshot(payload: serde_json::Value) -> CodexResetCreditsSnapshot {
    let credits = payload
        .get("credits")
        .or_else(|| payload.get("data").and_then(|data| data.get("credits")))
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(parse_reset_credit_record)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let available_count = payload
        .get("available_count")
        .or_else(|| payload.get("availableCount"))
        .or_else(|| {
            payload.get("data").and_then(|data| {
                data.get("available_count")
                    .or_else(|| data.get("availableCount"))
            })
        })
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
        })
        .or_else(|| {
            Some(
                credits
                    .iter()
                    .filter(|credit| is_available_reset_credit(credit))
                    .count() as i64,
            )
        });

    let next_expires_at = credits
        .iter()
        .filter(|credit| is_available_reset_credit(credit))
        .filter_map(|credit| credit.expires_at)
        .min();

    CodexResetCreditsSnapshot {
        available_count,
        credits,
        next_expires_at,
    }
}

fn mock_reset_credits_payload() -> Option<serde_json::Value> {
    if !cfg!(debug_assertions) {
        return None;
    }

    if let Ok(raw) = std::env::var(RESET_CREDITS_MOCK_JSON_ENV) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            match serde_json::from_str(trimmed) {
                Ok(payload) => return Some(payload),
                Err(error) => {
                    logger::log_warn(&format!("Codex reset credit mock JSON 解析失败: {}", error))
                }
            }
        }
    }

    None
}

fn current_chatgpt_timezone_offset_min() -> i32 {
    -(chrono::Local::now().offset().local_minus_utc() / 60)
}

fn normalize_optional_ref(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_optional_json_scalar(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(text) => normalize_optional_ref(Some(text)),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn parse_subscription_timestamp(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        let mut timestamp = trimmed.parse::<i64>().ok()?;
        if timestamp > 1_000_000_000_000 {
            timestamp /= 1000;
        }
        return Some(timestamp);
    }

    chrono::DateTime::parse_from_rfc3339(trimmed)
        .ok()
        .map(|parsed| parsed.timestamp())
}

fn subscription_missing_or_expired(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return true;
    };
    let Some(timestamp) = parse_subscription_timestamp(raw) else {
        return true;
    };
    timestamp <= now_timestamp()
}

fn mark_subscription_retry_pending(account: &mut CodexAccount, error: Option<String>) {
    let now = now_timestamp();
    account.subscription_query_last_attempt_at = Some(now);
    account.subscription_query_next_retry_at = Some(now + SUBSCRIPTION_RETRY_INTERVAL_SECONDS);
    account.subscription_query_last_error =
        error.and_then(|message| normalize_optional_ref(Some(&message)));
}

fn clear_subscription_retry_pending(account: &mut CodexAccount) {
    account.subscription_query_next_retry_at = None;
    account.subscription_query_last_error = None;
}

fn normalize_subscription_retry_state(account: &mut CodexAccount) {
    if !subscription_missing_or_expired(account.subscription_active_until.as_deref()) {
        clear_subscription_retry_pending(account);
    }
}

fn should_attempt_subscription_refresh(
    account: &CodexAccount,
    options: SubscriptionRefreshOptions,
) -> bool {
    if !subscription_missing_or_expired(account.subscription_active_until.as_deref())
        && !options.force
    {
        return false;
    }

    if options.force {
        return true;
    }

    let now = now_timestamp();
    account
        .subscription_query_next_retry_at
        .map(|next_retry_at| next_retry_at <= now)
        .unwrap_or(true)
}

fn extract_account_record_field(
    record: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    for key in keys {
        if let Some(value) = normalize_optional_json_scalar(record.get(*key)) {
            return Some(value);
        }
    }
    None
}

fn collect_subscription_account_records(payload: &serde_json::Value) -> Vec<AccountCheckRecord> {
    let mut records = Vec::new();

    if let Some(accounts_value) = payload.get("accounts") {
        if let Some(array) = accounts_value.as_array() {
            for item in array {
                if item.is_object() {
                    records.push(AccountCheckRecord {
                        key: None,
                        node: item.clone(),
                    });
                }
            }
        } else if let Some(object) = accounts_value.as_object() {
            for (key, value) in object {
                if value.is_object() {
                    records.push(AccountCheckRecord {
                        key: Some(key.clone()),
                        node: value.clone(),
                    });
                }
            }
        }
    }

    if records.is_empty() {
        if let Some(array) = payload.as_array() {
            for item in array {
                if item.is_object() {
                    records.push(AccountCheckRecord {
                        key: None,
                        node: item.clone(),
                    });
                }
            }
        }
    }

    records
}

fn parse_account_check_snapshot(
    payload: &serde_json::Value,
    account: &CodexAccount,
) -> Result<SubscriptionStatusSnapshot, String> {
    let records = collect_subscription_account_records(payload);
    if records.is_empty() {
        return Err("accounts/check 返回里没有可用账号".to_string());
    }

    let preferred_account_id =
        normalize_optional_ref(account.account_id.as_deref()).or_else(|| {
            codex_account::extract_chatgpt_account_id_from_access_token(
                &account.tokens.access_token,
            )
        });
    let ordering_first_key = payload
        .get("account_ordering")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_optional_ref(Some(value)));

    let selected = records
        .iter()
        .find(|item| {
            let Some(record) = item.node.as_object() else {
                return false;
            };
            let account_record = record
                .get("account")
                .and_then(|value| value.as_object())
                .unwrap_or(record);
            let candidate_id = extract_account_record_field(
                account_record,
                &["account_id", "id", "chatgpt_account_id", "workspace_id"],
            );
            candidate_id == preferred_account_id
        })
        .or_else(|| {
            records.iter().find(|item| {
                item.key
                    .as_deref()
                    .and_then(|value| normalize_optional_ref(Some(value)))
                    == ordering_first_key
            })
        })
        .unwrap_or(&records[0]);

    let record = selected
        .node
        .as_object()
        .ok_or_else(|| "accounts/check 账号记录格式不正确".to_string())?;
    let account_record = record
        .get("account")
        .and_then(|value| value.as_object())
        .unwrap_or(record);
    let entitlement = record
        .get("entitlement")
        .and_then(|value| value.as_object());

    let account_id = extract_account_record_field(
        account_record,
        &["account_id", "id", "chatgpt_account_id", "workspace_id"],
    );
    let plan_type = entitlement
        .and_then(|value| extract_account_record_field(value, &["subscription_plan"]))
        .or_else(|| extract_account_record_field(account_record, &["plan_type", "planType"]));
    let subscription_active_until = entitlement
        .and_then(|value| extract_account_record_field(value, &["expires_at"]))
        .or_else(|| extract_account_record_field(account_record, &["expires_at"]));

    Ok(SubscriptionStatusSnapshot {
        account_id,
        plan_type,
        subscription_active_until,
    })
}

fn parse_subscription_snapshot(
    payload: &serde_json::Value,
    fallback_account_id: &str,
) -> SubscriptionStatusSnapshot {
    SubscriptionStatusSnapshot {
        account_id: normalize_optional_ref(Some(fallback_account_id)),
        plan_type: normalize_optional_json_scalar(
            payload
                .get("subscription_plan")
                .or_else(|| payload.get("plan_type")),
        ),
        subscription_active_until: normalize_optional_json_scalar(
            payload
                .get("active_until")
                .or_else(|| payload.get("expires_at")),
        ),
    }
}

fn build_subscription_headers(
    account: &CodexAccount,
    target_path: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", account.tokens.access_token))
            .map_err(|e| format!("构建 Authorization 头失败: {}", e))?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(REFERER, HeaderValue::from_static(CHATGPT_WEB_REFERER));
    headers.insert(USER_AGENT, HeaderValue::from_static(CHATGPT_WEB_USER_AGENT));
    headers.insert(
        "x-openai-target-path",
        HeaderValue::from_str(target_path)
            .map_err(|e| format!("构建 x-openai-target-path 头失败: {}", e))?,
    );
    headers.insert(
        "x-openai-target-route",
        HeaderValue::from_str(target_path)
            .map_err(|e| format!("构建 x-openai-target-route 头失败: {}", e))?,
    );

    if let Some(account_id) = normalize_optional_ref(chatgpt_account_id) {
        headers.insert(
            "ChatGPT-Account-Id",
            HeaderValue::from_str(&account_id)
                .map_err(|e| format!("构建 ChatGPT-Account-Id 头失败: {}", e))?,
        );
    }

    Ok(headers)
}

async fn fetch_subscription_account_check(
    account: &CodexAccount,
) -> Result<SubscriptionStatusSnapshot, String> {
    let client = reqwest::Client::new();
    let headers =
        build_subscription_headers(account, "/backend-api/accounts/check/v4-2023-04-27", None)?;
    let timezone_offset_min = current_chatgpt_timezone_offset_min();

    let response = client
        .get(SUBSCRIPTION_ACCOUNTS_CHECK_URL)
        .query(&[("timezone_offset_min", timezone_offset_min)])
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("请求订阅账号信息失败: {}", e))?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取订阅账号信息响应失败: {}", e))?;
    let body_len = body.len();

    logger::log_info(&format!(
        "Codex 订阅账号信息响应: url={}, status={}, request-id={}, x-request-id={}, cf-ray={}, body_len={}",
        SUBSCRIPTION_ACCOUNTS_CHECK_URL,
        status,
        get_header_value(&headers, "request-id"),
        get_header_value(&headers, "x-request-id"),
        get_header_value(&headers, "cf-ray"),
        body_len
    ));

    if !status.is_success() {
        let detail_code = extract_detail_code_from_body(&body);
        let mut error_message = format!("订阅账号信息接口返回错误 {}", status);
        if let Some(code) = detail_code {
            error_message.push_str(&format!(" [error_code:{}]", code));
        }
        error_message.push_str(&format!(" [body_len:{}]", body_len));
        append_http_error_diagnostics(&mut error_message, &headers, &body);
        return Err(error_message);
    }

    let payload: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("订阅账号信息 JSON 解析失败: {}", e))?;
    parse_account_check_snapshot(&payload, account)
}

async fn fetch_subscriptions_snapshot(
    account: &CodexAccount,
    account_id: &str,
) -> Result<SubscriptionStatusSnapshot, String> {
    let client = reqwest::Client::new();
    let headers = build_subscription_headers(account, "/backend-api/subscriptions", None)?;

    let response = client
        .get(SUBSCRIPTIONS_URL)
        .query(&[("account_id", account_id)])
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("请求订阅信息失败: {}", e))?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取订阅信息响应失败: {}", e))?;
    let body_len = body.len();

    logger::log_info(&format!(
        "Codex 订阅信息响应: url={}, status={}, request-id={}, x-request-id={}, cf-ray={}, body_len={}",
        SUBSCRIPTIONS_URL,
        status,
        get_header_value(&headers, "request-id"),
        get_header_value(&headers, "x-request-id"),
        get_header_value(&headers, "cf-ray"),
        body_len
    ));

    if !status.is_success() {
        let detail_code = extract_detail_code_from_body(&body);
        let mut error_message = format!("订阅信息接口返回错误 {}", status);
        if let Some(code) = detail_code {
            error_message.push_str(&format!(" [error_code:{}]", code));
        }
        error_message.push_str(&format!(" [body_len:{}]", body_len));
        append_http_error_diagnostics(&mut error_message, &headers, &body);
        return Err(error_message);
    }

    let payload: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("订阅信息 JSON 解析失败: {}", e))?;
    Ok(parse_subscription_snapshot(&payload, account_id))
}

async fn fetch_subscription_status_snapshot(
    account: &CodexAccount,
) -> Result<SubscriptionStatusSnapshot, String> {
    let mut snapshot = fetch_subscription_account_check(account).await?;

    let should_query_subscriptions =
        subscription_missing_or_expired(snapshot.subscription_active_until.as_deref());
    if !should_query_subscriptions {
        return Ok(snapshot);
    }

    let account_id = snapshot
        .account_id
        .clone()
        .or_else(|| normalize_optional_ref(account.account_id.as_deref()))
        .or_else(|| {
            codex_account::extract_chatgpt_account_id_from_access_token(
                &account.tokens.access_token,
            )
        })
        .ok_or_else(|| "未获取到 account_id，无法请求 subscriptions".to_string())?;

    let subscriptions = fetch_subscriptions_snapshot(account, &account_id).await?;
    snapshot.account_id = Some(account_id);
    if subscriptions.plan_type.is_some() {
        snapshot.plan_type = subscriptions.plan_type;
    }
    if subscriptions.subscription_active_until.is_some() {
        snapshot.subscription_active_until = subscriptions.subscription_active_until;
    }
    Ok(snapshot)
}

async fn refresh_subscription_state(
    account: &mut CodexAccount,
    options: SubscriptionRefreshOptions,
) -> Result<bool, String> {
    normalize_subscription_retry_state(account);
    if !should_attempt_subscription_refresh(account, options) {
        return Ok(false);
    }

    let snapshot = match fetch_subscription_status_snapshot(account).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            mark_subscription_retry_pending(account, Some(error.clone()));
            return Err(error);
        }
    };

    let mut changed = false;
    if snapshot.account_id.is_some() && account.account_id != snapshot.account_id {
        account.account_id = snapshot.account_id.clone();
        changed = true;
    }

    let previous_plan = account.plan_type.clone();
    let previous_subscription = account.subscription_active_until.clone();
    sync_subscription_from_token(
        account,
        snapshot.plan_type.clone(),
        snapshot.subscription_active_until.clone(),
    );
    changed = changed
        || previous_plan != account.plan_type
        || previous_subscription != account.subscription_active_until;

    account.subscription_query_last_attempt_at = Some(now_timestamp());
    if subscription_missing_or_expired(account.subscription_active_until.as_deref()) {
        mark_subscription_retry_pending(account, Some("订阅接口未返回有效订阅时间".to_string()));
    } else {
        account.subscription_query_last_success_at = Some(now_timestamp());
        clear_subscription_retry_pending(account);
    }

    Ok(changed)
}

async fn refresh_account_tokens(account: &mut CodexAccount, reason: &str) -> Result<(), String> {
    logger::log_info(&format!(
        "Codex 账号 {} 触发强制 Token 刷新: {}",
        account.email, reason
    ));

    let refreshed = codex_account::force_refresh_managed_account(&account.id, reason)
        .await
        .map_err(|e| format!("{}，刷新 Token 失败: {}", reason, e))?;
    *account = refreshed;
    Ok(())
}

/// 查询单个账号的配额
pub async fn fetch_quota(account: &CodexAccount) -> Result<FetchQuotaResult, String> {
    let account_id = account.account_id.clone().or_else(|| {
        codex_account::extract_chatgpt_account_id_from_access_token(&account.tokens.access_token)
    });

    logger::log_info(&format!(
        "Codex 配额请求: {} (account_id: {:?})",
        USAGE_URL, account_id
    ));

    let response = send_codex_api_request(account, Method::GET, USAGE_URL, None).await?;
    let status = response.status;
    let headers = response.headers;
    let body = response.body;

    let request_id = get_header_value(&headers, "request-id");
    let x_request_id = get_header_value(&headers, "x-request-id");
    let cf_ray = get_header_value(&headers, "cf-ray");
    let body_len = body.len();

    logger::log_info(&format!(
        "Codex 配额响应元信息: url={}, status={}, request-id={}, x-request-id={}, cf-ray={}, body_len={}",
        USAGE_URL, status, request_id, x_request_id, cf_ray, body_len
    ));

    if !status.is_success() {
        let detail_code = extract_detail_code_from_body(&body);

        logger::log_error(&format!(
            "Codex 配额接口返回非成功状态: url={}, status={}, request-id={}, x-request-id={}, cf-ray={}, detail_code={:?}, body_len={}, body={}",
            USAGE_URL,
            status,
            request_id,
            x_request_id,
            cf_ray,
            detail_code,
            body_len,
            normalize_http_error_body_for_display(&body)
        ));

        let mut error_message = format!("API 返回错误 {}", status);
        if let Some(code) = detail_code {
            error_message.push_str(&format!(" [error_code:{}]", code));
        }
        error_message.push_str(&format!(" [body_len:{}]", body_len));
        append_http_error_diagnostics(&mut error_message, &headers, &body);
        return Err(error_message);
    }

    // 解析响应
    let usage: UsageResponse =
        serde_json::from_str(&body).map_err(|e| format!("解析 JSON 失败: {}", e))?;

    let quota = parse_quota_from_usage(&usage, &body)?;
    let plan_type = usage.plan_type.clone();

    Ok(FetchQuotaResult { quota, plan_type })
}

/// 从使用率响应中解析配额信息
fn parse_quota_from_usage(usage: &UsageResponse, raw_body: &str) -> Result<CodexQuota, String> {
    let rate_limit = usage.rate_limit.as_ref();
    let primary_window = rate_limit.and_then(|r| r.primary_window.as_ref());
    let secondary_window = rate_limit.and_then(|r| r.secondary_window.as_ref());

    // Primary window = 5小时配额（session）
    let (hourly_percentage, hourly_reset_time, hourly_window_minutes) =
        if let Some(primary) = primary_window {
            (
                normalize_remaining_percentage(primary)?,
                normalize_reset_time(primary),
                normalize_window_minutes(primary),
            )
        } else {
            (100, None, None)
        };

    // Secondary window = 周配额
    let (weekly_percentage, weekly_reset_time, weekly_window_minutes) =
        if let Some(secondary) = secondary_window {
            (
                normalize_remaining_percentage(secondary)?,
                normalize_reset_time(secondary),
                normalize_window_minutes(secondary),
            )
        } else {
            (100, None, None)
        };

    // 保存原始响应
    let raw_data: Option<serde_json::Value> = serde_json::from_str(raw_body).ok();

    Ok(CodexQuota {
        hourly_percentage,
        hourly_reset_time,
        hourly_window_minutes,
        hourly_window_present: Some(primary_window.is_some()),
        weekly_percentage,
        weekly_reset_time,
        weekly_window_minutes,
        weekly_window_present: Some(secondary_window.is_some()),
        reset_credits_available: usage
            .rate_limit_reset_credits
            .as_ref()
            .and_then(|credits| credits.available_count),
        reset_credits: Vec::new(),
        reset_credits_next_expires_at: None,
        raw_data,
    })
}

fn is_new_api_account(account: &CodexAccount) -> bool {
    account
        .api_provider_id
        .as_deref()
        .map(|value| {
            let value = value.trim();
            value.eq_ignore_ascii_case(COCKPIT_API_PROVIDER_ID)
                || value.eq_ignore_ascii_case(LEGACY_NEW_API_PROVIDER_ID)
        })
        .unwrap_or(false)
        || is_cockpit_api_base_url(account.api_base_url.as_deref())
        || account
            .plan_type
            .as_deref()
            .map(|value| {
                let value = value.trim();
                value.eq_ignore_ascii_case(COCKPIT_API_PLAN_TYPE)
                    || value.eq_ignore_ascii_case(LEGACY_NEW_API_EXCLUSIVE_PLAN_TYPE)
            })
            .unwrap_or(false)
}

pub fn supports_quota_refresh(account: &CodexAccount) -> bool {
    !account.is_api_key_auth() || is_new_api_account(account)
}

fn normalize_api_base_url_for_match(raw: Option<&str>) -> Option<String> {
    let parsed = reqwest::Url::parse(raw?.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    let port = parsed
        .port()
        .map(|value| format!(":{}", value))
        .unwrap_or_default();
    let path = parsed.path().trim_end_matches('/');
    Some(format!("{}://{}{}{}", parsed.scheme(), host, port, path).to_ascii_lowercase())
}

fn is_cockpit_api_base_url(raw: Option<&str>) -> bool {
    let Some(actual) = normalize_api_base_url_for_match(raw) else {
        return false;
    };
    let Some(expected) = normalize_api_base_url_for_match(Some(COCKPIT_API_BASE_URL)) else {
        return false;
    };
    actual == expected
}

fn build_new_api_profile_url(account: &CodexAccount) -> Result<String, String> {
    let base_url = account
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("Cockpit Api 账号缺少 Base URL")?;
    let mut parsed = reqwest::Url::parse(base_url)
        .map_err(|err| format!("Cockpit Api Base URL 无效: {}", err))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Cockpit Api Base URL 仅支持 http/https".to_string());
    }
    parsed.set_path("/api/cockpit-tools/token-profile");
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

fn read_i64(value: &serde_json::Value, key: &str) -> i64 {
    value
        .get(key)
        .and_then(|item| {
            item.as_i64()
                .or_else(|| item.as_u64().and_then(|raw| i64::try_from(raw).ok()))
        })
        .unwrap_or(0)
}

fn read_bool(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
}

fn new_api_percentage(available: i64, total: i64, unlimited: bool) -> i32 {
    if unlimited {
        return 100;
    }
    if total <= 0 {
        return 0;
    }
    let percentage = (available.max(0) as f64 / total.max(1) as f64) * 100.0;
    percentage.round().clamp(0.0, 100.0) as i32
}

async fn fetch_new_api_quota(account: &CodexAccount) -> Result<FetchQuotaResult, String> {
    let api_key = account
        .openai_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("Cockpit Api 账号缺少 OPENAI_API_KEY")?;
    let profile_url = build_new_api_profile_url(account)?;
    let client = reqwest::Client::new();
    let response = client
        .get(&profile_url)
        .bearer_auth(api_key)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|err| format!("请求 Cockpit Api 额度失败: {}", err))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("读取 Cockpit Api 额度响应失败: {}", err))?;
    if !status.is_success() {
        return Err(format!("Cockpit Api 额度接口返回 HTTP {}", status.as_u16()));
    }

    let root: serde_json::Value = serde_json::from_str(&body)
        .map_err(|err| format!("解析 Cockpit Api 额度 JSON 失败: {}", err))?;
    if root.get("success").and_then(|item| item.as_bool()) == Some(false) {
        let message = root
            .get("message")
            .and_then(|item| item.as_str())
            .unwrap_or("Cockpit Api 额度接口返回失败");
        return Err(message.to_string());
    }
    let data = root.get("data").unwrap_or(&root);
    let usage = data.get("usage").ok_or("Cockpit Api 额度响应缺少 usage")?;
    let total = read_i64(usage, "total_granted");
    let used = read_i64(usage, "total_used");
    let available = read_i64(usage, "total_available");
    let unlimited = read_bool(usage, "unlimited_quota");
    let percentage = new_api_percentage(available, total, unlimited);
    let expires_at = read_i64(usage, "expires_at");
    let reset_time = if expires_at > 0 {
        Some(expires_at)
    } else {
        None
    };

    Ok(FetchQuotaResult {
        quota: CodexQuota {
            hourly_percentage: percentage,
            hourly_reset_time: reset_time,
            hourly_window_minutes: None,
            hourly_window_present: Some(true),
            weekly_percentage: 0,
            weekly_reset_time: None,
            weekly_window_minutes: None,
            weekly_window_present: Some(false),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: Some(json!({
                "provider": "cockpit-api",
                "object": "codex_cockpit_api_quota",
                "profile": data,
                "usage": usage,
                "total_granted": total,
                "total_used": used,
                "total_available": available,
                "unlimited_quota": unlimited
            })),
        },
        plan_type: Some(
            data.get("plan_type")
                .and_then(|item| item.as_str())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| COCKPIT_API_PLAN_TYPE.to_string()),
        ),
    })
}

fn build_codex_api_headers(
    account: &CodexAccount,
    account_id: Option<&str>,
) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    if !account.is_agent_identity_auth() {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", account.tokens.access_token))
                .map_err(|e| format!("构建 Authorization 头失败: {}", e))?,
        );
    }
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(REFERER, HeaderValue::from_static(CHATGPT_WEB_REFERER));
    headers.insert(USER_AGENT, HeaderValue::from_static(CHATGPT_WEB_USER_AGENT));
    headers.insert("OpenAI-Beta", HeaderValue::from_static("codex-1"));
    headers.insert("oai-language", HeaderValue::from_static("zh-CN"));
    headers.insert("originator", HeaderValue::from_static("Codex Desktop"));
    headers.insert("sec-fetch-site", HeaderValue::from_static("none"));
    headers.insert("sec-fetch-mode", HeaderValue::from_static("no-cors"));
    headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
    headers.insert("priority", HeaderValue::from_static("u=4, i"));

    if account
        .agent_identity
        .as_ref()
        .is_some_and(|identity| identity.chatgpt_account_is_fedramp)
    {
        headers.insert("x-openai-fedramp", HeaderValue::from_static("true"));
    }

    if let Some(account_id) = normalize_optional_ref(account_id) {
        headers.insert(
            "ChatGPT-Account-Id",
            HeaderValue::from_str(&account_id)
                .map_err(|e| format!("构建 ChatGPT-Account-Id 头失败: {}", e))?,
        );
    }

    Ok(headers)
}

struct CodexApiResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

async fn send_codex_api_request(
    account: &CodexAccount,
    method: Method,
    url: &str,
    json_body: Option<&serde_json::Value>,
) -> Result<CodexApiResponse, String> {
    send_codex_api_request_with_agent_auth_base_url(
        account,
        method,
        url,
        json_body,
        codex_agent_identity::AGENT_IDENTITY_AUTH_API_BASE_URL,
    )
    .await
}

async fn send_codex_api_request_with_agent_auth_base_url(
    account: &CodexAccount,
    method: Method,
    url: &str,
    json_body: Option<&serde_json::Value>,
    agent_auth_base_url: &str,
) -> Result<CodexApiResponse, String> {
    let account_id = account.account_id.clone().or_else(|| {
        codex_account::extract_chatgpt_account_id_from_access_token(&account.tokens.access_token)
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("创建 Codex 上游客户端失败: {}", error))?;
    let mut current = account.clone();
    let mut expected_task_id: Option<String> = None;

    for attempt in 0..=1 {
        let mut headers = build_codex_api_headers(&current, account_id.as_deref())?;
        let mut assertion_task_id = None;
        if current.is_agent_identity_auth() {
            let (updated, auth_headers, task_id) =
                codex_agent_identity::build_authentication_headers_with_base_url(
                    &current,
                    expected_task_id.as_deref(),
                    agent_auth_base_url,
                )
                .await?;
            current = updated;
            headers.extend(auth_headers);
            assertion_task_id = Some(task_id);
        }

        let mut request = client.request(method.clone(), url).headers(headers);
        if let Some(body) = json_body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("请求失败: {}", error))?;
        let status = response.status();
        let headers = response.headers().clone();
        let raw_body = response
            .text()
            .await
            .map_err(|error| format!("读取响应失败: {}", error))?;

        if current.is_agent_identity_auth()
            && attempt == 0
            && codex_agent_identity::is_task_invalid_response(status, &raw_body)
        {
            expected_task_id = assertion_task_id;
            continue;
        }

        let body = if status.is_success() {
            raw_body
        } else {
            codex_agent_identity::redact_sensitive_body(&current, &raw_body)
        };
        return Ok(CodexApiResponse {
            status,
            headers,
            body,
        });
    }

    Err("Agent Identity task 恢复后请求仍失败".to_string())
}

async fn fetch_reset_credits(account: &CodexAccount) -> Result<CodexResetCreditsSnapshot, String> {
    if let Some(payload) = mock_reset_credits_payload() {
        logger::log_info("Codex reset credit 查询使用显式 mock JSON");
        return Ok(parse_reset_credits_snapshot(payload));
    }

    let response = send_codex_api_request(account, Method::GET, RESET_CREDITS_URL, None)
        .await
        .map_err(|error| format!("请求主动重置次数明细失败: {}", error))?;
    let status = response.status;
    let headers = response.headers;
    let body = response.body;

    logger::log_info(&format!(
        "Codex 主动重置次数明细响应: url={}, status={}, request-id={}, x-request-id={}, cf-ray={}, body_len={}",
        RESET_CREDITS_URL,
        status,
        get_header_value(&headers, "request-id"),
        get_header_value(&headers, "x-request-id"),
        get_header_value(&headers, "cf-ray"),
        body.len()
    ));

    if !status.is_success() {
        let detail_code = extract_detail_code_from_body(&body);
        let mut error_message = format!("主动重置次数明细接口返回错误 {}", status);
        if let Some(code) = detail_code {
            error_message.push_str(&format!(" [error_code:{}]", code));
        }
        error_message.push_str(&format!(" [body_len:{}]", body.len()));
        append_http_error_diagnostics(&mut error_message, &headers, &body);
        return Err(error_message);
    }

    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("主动重置次数明细 JSON 解析失败: {}", e))?;
    Ok(parse_reset_credits_snapshot(payload))
}

pub async fn fetch_account_reset_credits(
    account_id: &str,
) -> Result<CodexResetCreditsSnapshot, String> {
    let mut account = codex_account::prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() {
        return Err("API Key 账号不支持主动重置额度".to_string());
    }

    if !account.is_agent_identity_auth()
        && crate::modules::codex_oauth::is_token_expired(&account.tokens.access_token)
    {
        refresh_account_tokens(&mut account, "查询主动重置记录前 Token 已过期").await?;
        sync_subscription_expiry_from_current_id_token(&mut account);
        normalize_subscription_retry_state(&mut account);
        codex_account::save_account(&account)?;
    }

    match fetch_reset_credits(&account).await {
        Ok(snapshot) => Ok(snapshot),
        Err(error) if !account.is_agent_identity_auth() && is_unauthorized_error(&error) => {
            refresh_account_tokens(&mut account, "主动重置记录接口返回 401").await?;
            sync_subscription_expiry_from_current_id_token(&mut account);
            normalize_subscription_retry_state(&mut account);
            codex_account::save_account(&account)?;
            fetch_reset_credits(&account).await
        }
        Err(error) => Err(error),
    }
}

async fn post_reset_credit_once(
    account: &CodexAccount,
    redeem_request_id: &str,
) -> Result<(), String> {
    let request_body = json!({ "redeem_request_id": redeem_request_id });
    let response = send_codex_api_request(
        account,
        Method::POST,
        RESET_CREDITS_CONSUME_URL,
        Some(&request_body),
    )
    .await
    .map_err(|error| format!("请求主动重置失败: {}", error))?;
    let status = response.status;
    let headers = response.headers;
    let body = response.body;

    logger::log_info(&format!(
        "Codex 主动重置响应: url={}, status={}, request-id={}, x-request-id={}, cf-ray={}, body_len={}",
        RESET_CREDITS_CONSUME_URL,
        status,
        get_header_value(&headers, "request-id"),
        get_header_value(&headers, "x-request-id"),
        get_header_value(&headers, "cf-ray"),
        body.len()
    ));

    if status.is_success() {
        return Ok(());
    }

    let detail_code = extract_detail_code_from_body(&body);
    let mut error_message = format!("主动重置接口返回错误 {}", status);
    if let Some(code) = detail_code {
        error_message.push_str(&format!(" [error_code:{}]", code));
    }
    error_message.push_str(&format!(" [body_len:{}]", body.len()));
    append_http_error_diagnostics(&mut error_message, &headers, &body);
    Err(error_message)
}

fn is_unauthorized_error(message: &str) -> bool {
    message.contains("401") || message.contains(&StatusCode::UNAUTHORIZED.to_string())
}

pub async fn consume_reset_credit(account_id: &str) -> Result<(), String> {
    let mut account = codex_account::prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() {
        return Err("API Key 账号不支持主动重置额度".to_string());
    }
    if mock_reset_credits_payload().is_some() {
        logger::log_info(&format!(
            "Codex 主动重置使用显式 mock JSON 空操作: account_id={}",
            account_id
        ));
        return Ok(());
    }

    if !account.is_agent_identity_auth()
        && crate::modules::codex_oauth::is_token_expired(&account.tokens.access_token)
    {
        refresh_account_tokens(&mut account, "主动重置前 Token 已过期").await?;
        sync_subscription_expiry_from_current_id_token(&mut account);
        normalize_subscription_retry_state(&mut account);
        codex_account::save_account(&account)?;
    }

    let redeem_request_id = uuid::Uuid::new_v4().to_string();
    match post_reset_credit_once(&account, &redeem_request_id).await {
        Ok(()) => Ok(()),
        Err(error) if !account.is_agent_identity_auth() && is_unauthorized_error(&error) => {
            refresh_account_tokens(&mut account, "主动重置接口返回 401").await?;
            sync_subscription_expiry_from_current_id_token(&mut account);
            normalize_subscription_retry_state(&mut account);
            codex_account::save_account(&account)?;
            post_reset_credit_once(&account, &redeem_request_id).await
        }
        Err(error) => Err(error),
    }
}

/// 从 id_token 中提取订阅标识并同步更新账号和索引
fn sync_subscription_from_token(
    account: &mut CodexAccount,
    plan_type: Option<String>,
    subscription_active_until: Option<String>,
) {
    let mut changed = false;
    if let Some(ref new_plan) = plan_type {
        let old_plan = account.plan_type.clone();
        if account.plan_type.as_deref() != Some(new_plan) {
            logger::log_info(&format!(
                "Codex 账号 {} 订阅标识已更新: {:?} -> {:?}",
                account.email, old_plan, plan_type
            ));
            account.plan_type = plan_type;
            changed = true;
        }
    }

    if let Some(ref next_expiry) = subscription_active_until {
        if account.subscription_active_until.as_deref() != Some(next_expiry) {
            account.subscription_active_until = Some(next_expiry.clone());
            changed = true;
        }
    }

    if changed {
        if let Err(e) = codex_account::update_account_plan_type_in_index(
            &account.id,
            &account.plan_type,
            &account.subscription_active_until,
        ) {
            logger::log_warn(&format!("更新索引 plan_type 失败: {}", e));
        }
    }
}

fn sync_subscription_expiry_from_current_id_token(account: &mut CodexAccount) {
    if let Ok((_, _, _, subscription_active_until, _, _)) =
        codex_account::extract_user_info(&account.tokens.id_token)
    {
        sync_subscription_from_token(account, None, subscription_active_until);
    }
}

/// 刷新账号配额并保存（包含 token 自动刷新）
async fn refresh_account_quota_once(
    account_id: &str,
    options: RefreshQuotaOptions,
) -> Result<CodexQuota, String> {
    let mut account = match codex_account::prepare_account_for_injection(account_id).await {
        Ok(account) => account,
        Err(error) => {
            if let Some(mut stored_account) = codex_account::load_account(account_id) {
                write_quota_error(&mut stored_account, error.clone());
                if let Err(save_error) = codex_account::save_account(&stored_account) {
                    logger::log_warn(&format!(
                        "准备 Codex 账号失败后写入配额错误失败: {}",
                        save_error
                    ));
                }
            }
            return Err(error);
        }
    };
    if account.is_api_key_auth() {
        if is_new_api_account(&account) {
            let result = match fetch_new_api_quota(&account).await {
                Ok(result) => result,
                Err(e) => {
                    write_quota_error(&mut account, e.clone());
                    if let Err(save_err) = codex_account::save_account(&account) {
                        logger::log_warn(&format!("写入 Cockpit Api 配额错误失败: {}", save_err));
                    }
                    return Err(e);
                }
            };
            if result.plan_type.is_some() {
                sync_subscription_from_token(&mut account, result.plan_type.clone(), None);
            }
            normalize_subscription_retry_state(&mut account);
            account.quota = Some(result.quota.clone());
            account.quota_error = None;
            account.usage_updated_at = Some(now_timestamp());
            codex_account::save_account(&account)?;
            return Ok(result.quota);
        }
        account.quota = None;
        account.quota_error = None;
        account.usage_updated_at = None;
        let _ = codex_account::save_account(&account);
        return Err("API Key 账号不支持刷新配额，请在网页端查看。".to_string());
    }
    if account.is_agent_identity_auth() {
        let result = match fetch_quota(&account).await {
            Ok(result) => result,
            Err(error) => {
                write_quota_error(&mut account, error.clone());
                if let Err(save_error) = codex_account::save_account(&account) {
                    logger::log_warn(&format!("写入 Agent Identity 配额错误失败: {}", save_error));
                }
                return Err(error);
            }
        };
        account = codex_account::load_account(&account.id).unwrap_or(account);
        if result.plan_type.is_some() {
            sync_subscription_from_token(&mut account, result.plan_type.clone(), None);
        }
        account.quota = Some(result.quota.clone());
        account.quota_error = None;
        account.usage_updated_at = Some(now_timestamp());
        codex_account::save_account(&account)?;
        return Ok(result.quota);
    }

    // 检查 token 是否过期，如果过期则刷新
    if crate::modules::codex_oauth::is_token_expired(&account.tokens.access_token) {
        match refresh_account_tokens(&mut account, "Token 已过期").await {
            Ok(()) => {
                logger::log_info(&format!("账号 {} 的 Token 刷新成功", account.email));

                sync_subscription_expiry_from_current_id_token(&mut account);
                normalize_subscription_retry_state(&mut account);

                codex_account::save_account(&account)?;
            }
            Err(e) => {
                logger::log_error(&format!("账号 {} Token 刷新失败: {}", account.email, e));
                let message = e;
                write_quota_error(&mut account, message.clone());
                if let Err(save_err) = codex_account::save_account(&account) {
                    logger::log_warn(&format!("写入 Codex 配额错误失败: {}", save_err));
                }
                return Err(message);
            }
        }
    }

    let subscription_options = SubscriptionRefreshOptions {
        force: options.force_subscription_refresh,
    };
    let result = match fetch_quota(&account).await {
        Ok(result) => result,
        Err(e) => {
            if let Err(subscription_error) =
                refresh_subscription_state(&mut account, subscription_options).await
            {
                logger::log_warn(&format!(
                    "Codex 账号 {} 刷新配额失败后补拉订阅信息失败: {}",
                    account.email, subscription_error
                ));
            }
            write_quota_error(&mut account, e.clone());
            if let Err(save_err) = codex_account::save_account(&account) {
                logger::log_warn(&format!("写入 Codex 配额错误失败: {}", save_err));
            }
            return Err(e);
        }
    };

    // 从 usage 响应中的 plan_type 更新订阅标识
    if result.plan_type.is_some() {
        sync_subscription_from_token(&mut account, result.plan_type.clone(), None);
    }

    if let Err(subscription_error) =
        refresh_subscription_state(&mut account, subscription_options).await
    {
        logger::log_warn(&format!(
            "Codex 账号 {} 刷新订阅信息失败，保留现有订阅标识: {}",
            account.email, subscription_error
        ));
    }

    account.quota = Some(result.quota.clone());
    account.quota_error = None;
    account.usage_updated_at = Some(now_timestamp());
    codex_account::save_account(&account)?;

    Ok(result.quota)
}

pub async fn refresh_account_quota(account_id: &str) -> Result<CodexQuota, String> {
    let result = refresh_account_quota_once(account_id, RefreshQuotaOptions::default()).await;
    crate::modules::codex_local_access::reevaluate_bound_oauth_quota_reserve_after_refresh(
        account_id,
        result.is_ok(),
    )
    .await;
    result
}

pub async fn refresh_account_quota_with_options(
    account_id: &str,
    options: RefreshQuotaOptions,
) -> Result<CodexQuota, String> {
    let result = refresh_account_quota_once(account_id, options).await;
    crate::modules::codex_local_access::reevaluate_bound_oauth_quota_reserve_after_refresh(
        account_id,
        result.is_ok(),
    )
    .await;
    result
}

pub async fn probe_import_account_quota(account: &CodexAccount) -> Result<CodexQuota, String> {
    if account.is_agent_identity_auth() {
        return fetch_quota(account).await.map(|result| result.quota);
    }
    if account.is_api_key_auth() {
        if is_new_api_account(account) {
            return fetch_new_api_quota(account)
                .await
                .map(|result| result.quota);
        }
        return Err("API Key 账号不支持自动查询额度".to_string());
    }

    if crate::modules::codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Err("access_token 已过期，无法在导入前查询额度".to_string());
    }

    fetch_quota(account).await.map(|result| result.quota)
}

pub async fn refresh_account_subscription_info(
    account_id: &str,
    force: bool,
) -> Result<CodexAccount, String> {
    let mut account = codex_account::prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() {
        return Err("API Key 账号不支持刷新订阅信息".to_string());
    }
    if account.is_agent_identity_auth() {
        let quota = fetch_quota(&account).await?;
        account = codex_account::load_account(&account.id).unwrap_or(account);
        if quota.plan_type.is_some() {
            sync_subscription_from_token(&mut account, quota.plan_type, None);
            codex_account::save_account(&account)?;
        }
        return Ok(account);
    }

    if crate::modules::codex_oauth::is_token_expired(&account.tokens.access_token) {
        refresh_account_tokens(&mut account, "订阅信息刷新前 Token 已过期").await?;
        sync_subscription_expiry_from_current_id_token(&mut account);
        normalize_subscription_retry_state(&mut account);
    }

    match refresh_subscription_state(&mut account, SubscriptionRefreshOptions { force }).await {
        Ok(_) => {
            codex_account::save_account(&account)?;
            Ok(account)
        }
        Err(error) => {
            if let Err(save_err) = codex_account::save_account(&account) {
                logger::log_warn(&format!("写入订阅刷新状态失败: {}", save_err));
            }
            Err(error)
        }
    }
}

const CODEX_QUOTA_REFRESH_MAX_CONCURRENT: usize = 5;

/// 按账号 ID 列表限流并发刷新配额（分组/勾选批量共用）。
///
/// `respect_group_quota_refresh=true`：跳过分组策略为「不刷新」的账号。
/// 显式「刷新分组」应传 `false`。
pub async fn refresh_quotas_for_account_ids(
    account_ids: &[String],
) -> Result<Vec<(String, Result<CodexQuota, String>)>, String> {
    refresh_quotas_for_account_ids_with_options(account_ids, true).await
}

pub async fn refresh_quotas_for_account_ids_with_options(
    account_ids: &[String],
    respect_group_quota_refresh: bool,
) -> Result<Vec<(String, Result<CodexQuota, String>)>, String> {
    use futures::future::join_all;
    use std::collections::HashSet;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    if account_ids.is_empty() {
        return Ok(Vec::new());
    }

    let effective_ids: Vec<String> = if respect_group_quota_refresh {
        codex_account::filter_account_ids_by_quota_refresh_policy(account_ids)
    } else {
        account_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect()
    };

    // 去重并保持输入顺序，避免重复刷新同一账号
    let mut seen = HashSet::new();
    let unique_ids: Vec<String> = effective_ids
        .into_iter()
        .filter_map(|id| {
            if seen.insert(id.clone()) {
                Some(id)
            } else {
                None
            }
        })
        .collect();

    if unique_ids.is_empty() {
        return Ok(Vec::new());
    }

    let semaphore = Arc::new(Semaphore::new(CODEX_QUOTA_REFRESH_MAX_CONCURRENT));
    let tasks: Vec<_> = unique_ids
        .into_iter()
        .map(|account_id| {
            let semaphore = semaphore.clone();
            async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|e| format!("获取 Codex 刷新并发许可失败: {}", e))?;
                let result = refresh_account_quota(&account_id).await;
                Ok::<(String, Result<CodexQuota, String>), String>((account_id, result))
            }
        })
        .collect();

    let mut results = Vec::with_capacity(tasks.len());
    for task in join_all(tasks).await {
        match task {
            Ok(item) => results.push(item),
            Err(err) => return Err(err),
        }
    }
    Ok(results)
}

/// 刷新所有账号配额（自动跳过分组「不刷新」账号）
pub async fn refresh_all_quotas() -> Result<Vec<(String, Result<CodexQuota, String>)>, String> {
    let disabled = codex_account::load_quota_refresh_disabled_account_ids();
    let account_ids: Vec<String> = codex_account::list_accounts()
        .into_iter()
        .filter(supports_quota_refresh)
        .filter(|account| !disabled.contains(&account.id))
        .map(|account| account.id)
        .collect();
    refresh_quotas_for_account_ids_with_options(&account_ids, false).await
}

#[cfg(test)]
mod tests {
    use super::{
        build_codex_api_headers, normalize_http_error_body_for_display,
        normalize_remaining_percentage, parse_reset_credits_snapshot,
        send_codex_api_request_with_agent_auth_base_url, WindowInfo,
        HTTP_ERROR_BODY_DISPLAY_MAX_CHARS,
    };
    use crate::models::codex::{CodexAccount, CodexAgentIdentity, CodexTokens};
    use base64::{engine::general_purpose, Engine as _};
    use ed25519_dalek::{pkcs8::EncodePrivateKey, SigningKey};
    use rand::rngs::OsRng;
    use reqwest::Method;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn agent_identity_test_account() -> CodexAccount {
        let signing_key = SigningKey::generate(&mut OsRng);
        let private_key = signing_key.to_pkcs8_der().expect("encode PKCS#8");
        let mut account = CodexAccount::new(
            format!("agent-quota-{}", uuid::Uuid::new_v4()),
            "agent@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.account_id = Some("team-test".to_string());
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-test".to_string(),
            agent_private_key: general_purpose::STANDARD.encode(private_key.as_bytes()),
            task_id: Some("task-old".to_string()),
            account_id: "team-test".to_string(),
            chatgpt_user_id: "user-test".to_string(),
            email: Some(account.email.clone()),
            plan_type: Some("k12".to_string()),
            chatgpt_account_is_fedramp: true,
        });
        account
    }

    fn assertion_task_id(request: &str) -> Option<String> {
        let authorization = request.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim())
        })?;
        let encoded = authorization.strip_prefix("AgentAssertion ")?;
        let payload = general_purpose::URL_SAFE_NO_PAD.decode(encoded).ok()?;
        serde_json::from_slice::<serde_json::Value>(&payload)
            .ok()?
            .get("task_id")?
            .as_str()
            .map(str::to_string)
    }

    async fn read_test_http_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 2048];
        let header_end = loop {
            let read = stream.read(&mut chunk).await.expect("read request");
            assert!(read > 0, "connection closed before request headers");
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut chunk).await.expect("read request body");
            assert!(read > 0, "connection closed before request body");
            bytes.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn displays_empty_http_error_body_explicitly() {
        assert_eq!(normalize_http_error_body_for_display(" \n\t "), "<empty>");
    }

    #[test]
    fn rejects_missing_or_invalid_window_usage_percent() {
        let window = |used_percent| WindowInfo {
            used_percent,
            limit_window_seconds: Some(300),
            reset_after_seconds: None,
            reset_at: None,
        };

        assert!(normalize_remaining_percentage(&window(None)).is_err());
        assert!(normalize_remaining_percentage(&window(Some(-1))).is_err());
        assert!(normalize_remaining_percentage(&window(Some(101))).is_err());
        assert_eq!(
            normalize_remaining_percentage(&window(Some(37))).unwrap(),
            63
        );
    }

    #[test]
    fn compacts_and_truncates_http_error_body_for_display() {
        let body = format!(
            " first\n\nsecond   {} ",
            "x".repeat(HTTP_ERROR_BODY_DISPLAY_MAX_CHARS)
        );
        let display = normalize_http_error_body_for_display(&body);

        assert!(display.starts_with("first second "));
        assert!(display.ends_with("...(truncated)"));
        assert!(display.chars().count() <= HTTP_ERROR_BODY_DISPLAY_MAX_CHARS + 14);
    }

    #[test]
    fn parses_reset_credit_details_and_next_expiry() {
        let snapshot = parse_reset_credits_snapshot(json!({
            "available_count": 1,
            "credits": [
                {
                    "id": "credit-1",
                    "status": "available",
                    "type": "rate_limit_reset",
                    "granted_at": "2099-06-19T00:00:00Z",
                    "expires_at": "2099-06-25T08:30:00Z"
                },
                {
                    "id": "credit-2",
                    "status": "redeemed",
                    "granted_at": 1781846400,
                    "expires_at": 1782451200,
                    "redeemed_at": 1781900000
                }
            ]
        }));

        assert_eq!(snapshot.available_count, Some(1));
        assert_eq!(snapshot.credits.len(), 2);
        assert_eq!(snapshot.credits[0].id.as_deref(), Some("credit-1"));
        assert_eq!(snapshot.credits[0].status.as_deref(), Some("available"));
        assert_eq!(snapshot.credits[0].granted_at, Some(4085510400));
        assert_eq!(snapshot.credits[0].expires_at, Some(4086059400));
        assert_eq!(snapshot.next_expires_at, Some(4086059400));
    }

    #[test]
    fn derives_reset_credit_count_when_available_count_missing() {
        let future = chrono::Utc::now().timestamp() + 3600;
        let past = chrono::Utc::now().timestamp() - 3600;
        let snapshot = parse_reset_credits_snapshot(json!({
            "credits": [
                { "id": "available", "expires_at": future },
                { "id": "expired", "expires_at": past },
                { "id": "used", "status": "used", "expires_at": future }
            ]
        }));

        assert_eq!(snapshot.available_count, Some(1));
        assert_eq!(snapshot.next_expires_at, Some(future));
        assert_eq!(snapshot.credits[1].status.as_deref(), Some("expired"));
    }

    #[tokio::test]
    async fn agent_identity_quota_request_uses_common_headers_and_recovers_task_once() {
        let account = agent_identity_test_account();
        let headers = build_codex_api_headers(&account, account.account_id.as_deref())
            .expect("build common headers");
        assert!(headers.get("authorization").is_none());
        assert_eq!(headers.get("openai-beta").unwrap(), "codex-1");
        assert_eq!(headers.get("originator").unwrap(), "Codex Desktop");
        assert_eq!(headers.get("chatgpt-account-id").unwrap(), "team-test");
        assert_eq!(headers.get("x-openai-fedramp").unwrap(), "true");

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let base_url = format!("http://{}", listener.local_addr().expect("local address"));
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let captured = Arc::clone(&requests);
        let server = tokio::spawn(async move {
            let mut usage_calls = 0;
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().await.expect("accept request");
                let request = read_test_http_request(&mut stream).await;
                let request_line = request.lines().next().unwrap_or_default().to_string();
                captured.lock().expect("capture request").push(request);
                let (status, body) = if request_line.contains("/task/register") {
                    ("200 OK", r#"{"task_id":"task-new"}"#)
                } else if usage_calls == 0 {
                    usage_calls += 1;
                    (
                        "401 Unauthorized",
                        r#"{"error":{"code":"invalid_task_id"}}"#,
                    )
                } else {
                    usage_calls += 1;
                    ("200 OK", r#"{"ok":true}"#)
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write response");
            }
        });

        let response = send_codex_api_request_with_agent_auth_base_url(
            &account,
            Method::GET,
            &format!("{base_url}/usage"),
            None,
            &base_url,
        )
        .await
        .expect("recover task and retry quota request");
        assert!(response.status.is_success());
        server.await.expect("mock server");

        let requests = requests.lock().expect("captured requests");
        assert_eq!(requests.len(), 3);
        let usage_requests = requests
            .iter()
            .filter(|request| {
                request
                    .lines()
                    .next()
                    .is_some_and(|line| line.contains("/usage"))
            })
            .collect::<Vec<_>>();
        assert_eq!(usage_requests.len(), 2);
        assert_eq!(
            assertion_task_id(usage_requests[0]),
            Some("task-old".to_string())
        );
        assert_eq!(
            assertion_task_id(usage_requests[1]),
            Some("task-new".to_string())
        );
        assert!(usage_requests.iter().all(|request| {
            let lower = request.to_ascii_lowercase();
            lower.contains("openai-beta: codex-1")
                && lower.contains("originator: codex desktop")
                && lower.contains("chatgpt-account-id: team-test")
                && lower.contains("x-openai-fedramp: true")
        }));
    }
}
