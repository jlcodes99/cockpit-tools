use crate::models::codex::{CodexAccount, CodexQuota, CodexQuotaErrorInfo};
use crate::modules::{codex_account, logger};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::json;

// 使用 wham/usage 端点（Quotio 使用的）
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
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
const SUBSCRIPTION_RETRY_INTERVAL_SECONDS: i64 = 30 * 60;

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

/// 使用率响应
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageResponse {
    #[serde(rename = "plan_type")]
    plan_type: Option<String>,
    #[serde(rename = "rate_limit")]
    rate_limit: Option<RateLimitInfo>,
    #[serde(rename = "code_review_rate_limit")]
    code_review_rate_limit: Option<RateLimitInfo>,
}

fn normalize_remaining_percentage(window: &WindowInfo) -> i32 {
    let used = window.used_percent.unwrap_or(0).clamp(0, 100);
    100 - used
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
    let client = reqwest::Client::new();

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", account.tokens.access_token))
            .map_err(|e| format!("构建 Authorization 头失败: {}", e))?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

    // 添加 ChatGPT-Account-Id 头（关键！）
    let account_id = account.account_id.clone().or_else(|| {
        codex_account::extract_chatgpt_account_id_from_access_token(&account.tokens.access_token)
    });

    if let Some(ref acc_id) = account_id {
        if !acc_id.is_empty() {
            headers.insert(
                "ChatGPT-Account-Id",
                HeaderValue::from_str(acc_id)
                    .map_err(|e| format!("构建 Account-Id 头失败: {}", e))?,
            );
        }
    }

    logger::log_info(&format!(
        "Codex 配额请求: {} (account_id: {:?})",
        USAGE_URL, account_id
    ));

    let response = client
        .get(USAGE_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;

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
            "Codex 配额接口返回非成功状态: url={}, status={}, request-id={}, x-request-id={}, cf-ray={}, detail_code={:?}, body_len={}",
            USAGE_URL,
            status,
            request_id,
            x_request_id,
            cf_ray,
            detail_code,
            body_len
        ));

        let mut error_message = format!("API 返回错误 {}", status);
        if let Some(code) = detail_code {
            error_message.push_str(&format!(" [error_code:{}]", code));
        }
        error_message.push_str(&format!(" [body_len:{}]", body_len));
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
                normalize_remaining_percentage(primary),
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
                normalize_remaining_percentage(secondary),
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
    let mut account = codex_account::prepare_account_for_injection(account_id).await?;
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
    refresh_account_quota_once(account_id, RefreshQuotaOptions::default()).await
}

pub async fn refresh_account_quota_with_options(
    account_id: &str,
    options: RefreshQuotaOptions,
) -> Result<CodexQuota, String> {
    refresh_account_quota_once(account_id, options).await
}

pub async fn refresh_account_subscription_info(
    account_id: &str,
    force: bool,
) -> Result<CodexAccount, String> {
    let mut account = codex_account::prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() {
        return Err("API Key 账号不支持刷新订阅信息".to_string());
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

/// 刷新所有账号配额
pub async fn refresh_all_quotas() -> Result<Vec<(String, Result<CodexQuota, String>)>, String> {
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    const MAX_CONCURRENT: usize = 5;
    let accounts: Vec<_> = codex_account::list_accounts()
        .into_iter()
        .filter(|account| !account.is_api_key_auth() || is_new_api_account(account))
        .collect();

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let tasks: Vec<_> = accounts
        .into_iter()
        .map(|account| {
            let account_id = account.id;
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
