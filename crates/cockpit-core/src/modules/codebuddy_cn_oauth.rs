use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use crate::models::codebuddy::{CodebuddyOAuthCompletePayload, CodebuddyOAuthStartResponse};
use crate::modules::logger;

const CODEBUDDY_API_ENDPOINT: &str = "https://www.codebuddy.cn";
const CODEBUDDY_API_PREFIX: &str = "/v2/plugin";
const CODEBUDDY_PLATFORM: &str = "ide";
const OAUTH_TIMEOUT_SECONDS: u64 = 600;
const OAUTH_POLL_INTERVAL_MS: u64 = 1500;

#[derive(Clone)]
struct PendingOAuthState {
    login_id: String,
    expires_at: i64,
    state: String,
    cancelled: bool,
}

lazy_static::lazy_static! {
    static ref PENDING_OAUTH_STATE: Arc<Mutex<Option<PendingOAuthState>>> = Arc::new(Mutex::new(None));
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn generate_login_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..16).map(|_| rng.gen::<u8>()).collect();
    format!(
        "cb_{}",
        bytes
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    )
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
}

fn normalize_product_code(value: Option<&str>) -> String {
    normalize_non_empty(value).unwrap_or_else(|| "p_tcaca".to_string())
}

fn normalize_user_resource_status(status: &[i32]) -> Vec<i32> {
    let mut normalized: Vec<i32> = status.iter().copied().filter(|v| *v >= 0).collect();
    if normalized.is_empty() {
        return vec![0, 3];
    }
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

fn build_default_user_resource_time_range() -> (String, String) {
    let now = chrono::Local::now();
    let begin = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let end = (now + chrono::Duration::days(365 * 101))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    (begin, end)
}

fn clear_pending_login(login_id: &str) -> Result<(), String> {
    let mut pending = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "获取锁失败".to_string())?;
    if pending
        .as_ref()
        .map(|s| s.login_id == login_id)
        .unwrap_or(false)
    {
        *pending = None;
    }
    Ok(())
}

pub fn clear_pending_oauth_login(login_id: &str) -> Result<(), String> {
    clear_pending_login(login_id)
}

async fn start_login_with_platform(platform: &str) -> Result<CodebuddyOAuthStartResponse, String> {
    let client = build_client()?;
    let url = format!(
        "{}{}/auth/state?platform={}",
        CODEBUDDY_API_ENDPOINT, CODEBUDDY_API_PREFIX, platform
    );

    logger::log_info(&format!("[CodeBuddy OAuth] 请求 auth/state: {}", url));

    let resp = client
        .post(&url)
        .json(&json!({}))
        .send()
        .await
        .map_err(|e| format!("请求 auth/state 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 auth/state 响应失败: {}", e))?;

    let data = body.get("data").ok_or_else(|| {
        let mut keys = body
            .as_object()
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        keys.sort();
        format!("auth/state 响应缺少 data 字段: body_keys={:?}", keys)
    })?;

    let state = data
        .get("state")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "auth/state 响应缺少 state".to_string())?
        .to_string();

    let auth_url = data
        .get("authUrl")
        .or_else(|| data.get("auth_url"))
        .or_else(|| data.get("url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let login_id = generate_login_id();

    let verification_uri = if auth_url.is_empty() {
        format!("{}/login?state={}", CODEBUDDY_API_ENDPOINT, state)
    } else {
        auth_url.clone()
    };

    {
        let mut pending = PENDING_OAUTH_STATE
            .lock()
            .map_err(|_| "获取锁失败".to_string())?;
        *pending = Some(PendingOAuthState {
            login_id: login_id.clone(),
            expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS as i64,
            state: state.clone(),
            cancelled: false,
        });
    }

    logger::log_info(&format!(
        "[CodeBuddy OAuth] 登录已启动: login_id={}, state={}",
        login_id, state
    ));

    Ok(CodebuddyOAuthStartResponse {
        login_id,
        verification_uri: verification_uri.clone(),
        verification_uri_complete: Some(verification_uri),
        expires_in: OAUTH_TIMEOUT_SECONDS,
        interval_seconds: OAUTH_POLL_INTERVAL_MS / 1000 + 1,
    })
}

pub async fn start_login() -> Result<CodebuddyOAuthStartResponse, String> {
    start_login_with_platform(CODEBUDDY_PLATFORM).await
}

pub async fn complete_login(login_id: &str) -> Result<CodebuddyOAuthCompletePayload, String> {
    let client = build_client()?;
    let start = now_timestamp();

    loop {
        let state_info = {
            let pending = PENDING_OAUTH_STATE
                .lock()
                .map_err(|_| "获取锁失败".to_string())?;
            match pending.as_ref() {
                None => return Err("没有待处理的登录请求".to_string()),
                Some(s) => {
                    if s.login_id != login_id {
                        return Err("login_id 不匹配".to_string());
                    }
                    if s.cancelled {
                        return Err("登录已取消".to_string());
                    }
                    if now_timestamp() > s.expires_at {
                        return Err("登录超时".to_string());
                    }
                    s.clone()
                }
            }
        };

        let url = format!(
            "{}{}/auth/token?state={}",
            CODEBUDDY_API_ENDPOINT, CODEBUDDY_API_PREFIX, state_info.state
        );

        match client.get(&url).send().await {
            Ok(resp) => {
                if let Ok(body) = resp.json::<Value>().await {
                    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);

                    if code == 0 || code == 200 {
                        if let Some(data) = body.get("data") {
                            let access_token = data
                                .get("accessToken")
                                .or_else(|| data.get("access_token"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            if !access_token.is_empty() {
                                logger::log_info("[CodeBuddy OAuth] 获取 token 成功");

                                let refresh_token = data
                                    .get("refreshToken")
                                    .or_else(|| data.get("refresh_token"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());

                                let expires_at = data
                                    .get("expiresAt")
                                    .or_else(|| data.get("expires_at"))
                                    .and_then(|v| v.as_i64());

                                let domain = data
                                    .get("domain")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());

                                let token_type = data
                                    .get("tokenType")
                                    .or_else(|| data.get("token_type"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());

                                let auth_raw = Some(data.clone());

                                let account_info = fetch_account_info(
                                    &client,
                                    &access_token,
                                    &state_info.state,
                                    domain.as_deref(),
                                )
                                .await;

                                let (
                                    uid,
                                    nickname,
                                    email,
                                    enterprise_id,
                                    enterprise_name,
                                    profile_raw,
                                ) = match account_info {
                                    Ok(info) => info,
                                    Err(e) => {
                                        logger::log_warn(&format!(
                                            "[CodeBuddy OAuth] 获取账号信息失败: {}",
                                            e
                                        ));
                                        (None, None, String::new(), None, None, None)
                                    }
                                };

                                return Ok(CodebuddyOAuthCompletePayload {
                                    email,
                                    uid,
                                    nickname,
                                    enterprise_id,
                                    enterprise_name,
                                    access_token,
                                    refresh_token,
                                    token_type,
                                    expires_at,
                                    domain,
                                    plan_type: None,
                                    dosage_notify_code: None,
                                    dosage_notify_zh: None,
                                    dosage_notify_en: None,
                                    payment_type: None,
                                    quota_raw: None,
                                    auth_raw,
                                    profile_raw,
                                    usage_raw: None,
                                    status: Some("normal".to_string()),
                                    status_reason: None,
                                    last_checkin_time: None,
                                    checkin_streak: 0,
                                    checkin_rewards: None,
                                });
                            }
                        }
                    }
                }
            }
            Err(e) => {
                logger::log_warn(&format!("[CodeBuddy OAuth] 轮询 token 请求失败: {}", e));
            }
        }

        if now_timestamp() - start > OAUTH_TIMEOUT_SECONDS as i64 {
            let mut pending = PENDING_OAUTH_STATE
                .lock()
                .map_err(|_| "获取锁失败".to_string())?;
            *pending = None;
            return Err("登录超时".to_string());
        }

        tokio::time::sleep(std::time::Duration::from_millis(OAUTH_POLL_INTERVAL_MS)).await;
    }
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    let mut pending = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "获取锁失败".to_string())?;
    if let Some(state) = pending.as_mut() {
        if login_id.is_none() || login_id == Some(state.login_id.as_str()) {
            state.cancelled = true;
            *pending = None;
        }
    }
    Ok(())
}

async fn fetch_account_info(
    client: &reqwest::Client,
    access_token: &str,
    state: &str,
    domain: Option<&str>,
) -> Result<
    (
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<Value>,
    ),
    String,
> {
    let url = format!(
        "{}{}/login/account?state={}",
        CODEBUDDY_API_ENDPOINT, CODEBUDDY_API_PREFIX, state
    );

    let mut req = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token));

    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 login/account 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 login/account 响应失败: {}", e))?;

    let data = body.get("data").cloned().unwrap_or(json!({}));

    let uid = data
        .get("uid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let nickname = data
        .get("nickname")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let email = data
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let enterprise_id = data
        .get("enterpriseId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let enterprise_name = data
        .get("enterpriseName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let email_final = if email.is_empty() {
        nickname.clone().or_else(|| uid.clone()).unwrap_or_default()
    } else {
        email
    };

    Ok((
        uid,
        nickname,
        email_final,
        enterprise_id,
        enterprise_name,
        Some(data),
    ))
}

pub async fn refresh_token(
    access_token: &str,
    refresh_token: &str,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}{}/auth/token/refresh",
        CODEBUDDY_API_ENDPOINT, CODEBUDDY_API_PREFIX
    );

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("X-Refresh-Token", refresh_token)
        .json(&json!({}));

    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("刷新 token 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析刷新响应失败: {}", e))?;

    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 && code != 200 {
        let msg = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("刷新 token 失败 (code={}): {}", code, msg));
    }

    body.get("data")
        .cloned()
        .ok_or_else(|| "刷新响应缺少 data 字段".to_string())
}

pub async fn fetch_dosage_notify(
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}/v2/billing/meter/get-dosage-notify",
        CODEBUDDY_API_ENDPOINT
    );

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json");

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }
    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 dosage notify 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 dosage 响应失败: {}", e))?;

    Ok(body)
}

pub async fn fetch_payment_type(
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}/v2/billing/meter/get-payment-type",
        CODEBUDDY_API_ENDPOINT
    );

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json");

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }
    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 payment type 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 payment type 响应失败: {}", e))?;

    Ok(body)
}

pub async fn fetch_user_resource_with_access_token(
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
    product_code: &str,
    status: &[i32],
    package_end_time_range_begin: &str,
    package_end_time_range_end: &str,
    page_number: i32,
    page_size: i32,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}/v2/billing/meter/get-user-resource",
        CODEBUDDY_API_ENDPOINT
    );

    let body = json!({
        "PageNumber": page_number,
        "PageSize": page_size,
        "ProductCode": product_code,
        "Status": status,
        "PackageEndTimeRangeBegin": package_end_time_range_begin,
        "PackageEndTimeRangeEnd": package_end_time_range_end
    });

    let mut req = client
        .post(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json");

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }
    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 user resource（Token）失败: {}", e))?;

    let status_code = resp.status();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let content_encoding = resp
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| {
            format!(
                "解析 user resource（Token）响应失败: {} (http={}, url={}, has_uid={}, has_enterprise_id={}, content_type={}, content_encoding={})",
                e,
                status_code.as_u16(),
                url,
                uid.is_some(),
                enterprise_id.is_some(),
                content_type,
                content_encoding
            )
        })?;

    if !status_code.is_success() {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!(
            "请求 user resource（Token）失败 (http={}): {}",
            status_code.as_u16(),
            message
        ));
    }

    if let Some(code) = body.get("code").and_then(|v| v.as_i64()) {
        if code != 0 && code != 200 {
            let message = body
                .get("message")
                .or_else(|| body.get("msg"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!(
                "请求 user resource（Token）失败 (code={}): {}",
                code, message
            ));
        }
    }

    Ok(body)
}

async fn fetch_user_resource_with_access_token_default(
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let product_code = normalize_product_code(None);
    let status = normalize_user_resource_status(&[]);
    let (package_end_time_range_begin, package_end_time_range_end) =
        build_default_user_resource_time_range();
    fetch_user_resource_with_access_token(
        access_token,
        uid,
        enterprise_id,
        domain,
        product_code.as_str(),
        &status,
        package_end_time_range_begin.as_str(),
        package_end_time_range_end.as_str(),
        1,
        100,
    )
    .await
}

async fn refresh_payload_for_account_inner(
    account: &crate::models::codebuddy::CodebuddyAccount,
    require_user_resource: bool,
) -> Result<(CodebuddyOAuthCompletePayload, Option<String>), String> {
    let mut new_access_token = account.access_token.clone();
    let mut new_refresh_token = account.refresh_token.clone();
    let mut new_expires_at = account.expires_at;
    let mut new_domain = account.domain.clone();

    if let Some(refresh_tk) = account.refresh_token.as_deref() {
        match refresh_token(&account.access_token, refresh_tk, account.domain.as_deref()).await {
            Ok(token_data) => {
                new_access_token = token_data
                    .get("accessToken")
                    .or_else(|| token_data.get("access_token"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&account.access_token)
                    .to_string();

                new_refresh_token = token_data
                    .get("refreshToken")
                    .or_else(|| token_data.get("refresh_token"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| account.refresh_token.clone());

                new_expires_at = token_data
                    .get("expiresAt")
                    .or_else(|| token_data.get("expires_at"))
                    .and_then(|v| v.as_i64())
                    .or(account.expires_at);

                new_domain = token_data
                    .get("domain")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| account.domain.clone());
            }
            Err(e) => {
                logger::log_warn(&format!(
                    "[CodeBuddy] Token 刷新失败，将使用现有 token 查询配额: {}",
                    e
                ));
            }
        }
    }

    let resolved_email =
        normalize_non_empty(Some(account.email.as_str())).unwrap_or_else(|| account.email.clone());
    let resolved_uid = account.uid.clone();
    let resolved_nickname = account.nickname.clone();
    let resolved_enterprise_id = account.enterprise_id.clone();
    let resolved_enterprise_name = account.enterprise_name.clone();
    let resolved_profile_raw = account.profile_raw.clone();

    let dosage = fetch_dosage_notify(
        &new_access_token,
        resolved_uid.as_deref(),
        resolved_enterprise_id.as_deref(),
        new_domain.as_deref(),
    )
    .await
    .ok();

    let payment = fetch_payment_type(
        &new_access_token,
        resolved_uid.as_deref(),
        resolved_enterprise_id.as_deref(),
        new_domain.as_deref(),
    )
    .await
    .ok();

    let mut quota_refresh_error: Option<String> = None;
    logger::log_info(&format!(
        "[CodeBuddy][IDE Token] 尝试刷新 user_resource: has_uid={}, has_enterprise_id={}, has_domain={}",
        resolved_uid.is_some(),
        resolved_enterprise_id.is_some(),
        new_domain.is_some()
    ));
    let user_resource = match fetch_user_resource_with_access_token_default(
        new_access_token.as_str(),
        resolved_uid.as_deref(),
        resolved_enterprise_id.as_deref(),
        new_domain.as_deref(),
    )
    .await
    {
        Ok(payload) => {
            logger::log_info("[CodeBuddy][IDE Token] 刷新 user_resource 成功");
            Some(payload)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[CodeBuddy][IDE Token] 刷新 user_resource 失败: {}",
                err
            ));
            quota_refresh_error = Some(err.clone());
            if require_user_resource {
                return Err(
                    "使用 IDE token 刷新 user_resource 失败，无法获取资源包配额".to_string()
                );
            }
            None
        }
    };

    let dosage_data = dosage.as_ref().and_then(|v| v.get("data"));
    let dosage_notify_code = dosage_data
        .and_then(|d| d.get("dosageNotifyCode"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => v.to_string(),
        });
    let dosage_notify_zh = dosage_data
        .and_then(|d| d.get("dosageNotifyZh"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let dosage_notify_en = dosage_data
        .and_then(|d| d.get("dosageNotifyEn"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let payment_data = payment.as_ref().and_then(|v| v.get("data"));
    let payment_type = payment_data
        .and_then(|d| {
            d.as_str().map(|s| s.to_string()).or_else(|| {
                d.get("paymentType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
        })
        .or_else(|| account.payment_type.clone());

    let mut combined_quota = serde_json::Map::new();
    if let Some(d) = &dosage {
        combined_quota.insert("dosage".to_string(), d.clone());
    }
    if let Some(p) = &payment {
        combined_quota.insert("payment".to_string(), p.clone());
    }
    if let Some(r) = &user_resource {
        combined_quota.insert("userResource".to_string(), r.clone());
    }

    let quota_raw = if combined_quota.is_empty() {
        account.quota_raw.clone()
    } else {
        Some(Value::Object(combined_quota))
    };

    let final_email =
        normalize_non_empty(Some(resolved_email.as_str())).unwrap_or_else(|| account.email.clone());

    Ok((
        CodebuddyOAuthCompletePayload {
            email: final_email,
            uid: resolved_uid,
            nickname: resolved_nickname,
            enterprise_id: resolved_enterprise_id,
            enterprise_name: resolved_enterprise_name,
            access_token: new_access_token,
            refresh_token: new_refresh_token,
            token_type: account.token_type.clone(),
            expires_at: new_expires_at,
            domain: new_domain,
            plan_type: account.plan_type.clone(),
            dosage_notify_code,
            dosage_notify_zh,
            dosage_notify_en,
            payment_type,
            quota_raw,
            auth_raw: account.auth_raw.clone(),
            profile_raw: resolved_profile_raw,
            usage_raw: user_resource.or_else(|| account.usage_raw.clone()),
            status: account.status.clone(),
            status_reason: account.status_reason.clone(),
            last_checkin_time: account.last_checkin_time,
            checkin_streak: account.checkin_streak,
            checkin_rewards: account.checkin_rewards.clone(),
        },
        quota_refresh_error,
    ))
}

pub async fn refresh_payload_for_account(
    account: &crate::models::codebuddy::CodebuddyAccount,
) -> Result<(CodebuddyOAuthCompletePayload, Option<String>), String> {
    refresh_payload_for_account_inner(account, false).await
}

pub async fn build_payload_from_token(
    access_token: &str,
) -> Result<CodebuddyOAuthCompletePayload, String> {
    let client = build_client()?;

    let url = format!(
        "{}{}/accounts",
        CODEBUDDY_API_ENDPOINT, CODEBUDDY_API_PREFIX
    );

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("请求 accounts 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 accounts 响应失败: {}", e))?;

    let accounts = body
        .get("data")
        .and_then(|d| d.get("accounts"))
        .and_then(|a| a.as_array());

    let account_data = accounts
        .and_then(|arr| {
            arr.iter().find(|a| {
                a.get("lastLogin")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
        })
        .or_else(|| accounts.and_then(|arr| arr.first()))
        .cloned()
        .unwrap_or(json!({}));

    let uid = account_data
        .get("uid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let nickname = account_data
        .get("nickname")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let email = account_data
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let enterprise_id = account_data
        .get("enterpriseId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let enterprise_name = account_data
        .get("enterpriseName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let dosage = fetch_dosage_notify(access_token, uid.as_deref(), enterprise_id.as_deref(), None)
        .await
        .ok();

    let payment = fetch_payment_type(access_token, uid.as_deref(), enterprise_id.as_deref(), None)
        .await
        .ok();

    let user_resource = fetch_user_resource_with_access_token_default(
        access_token,
        uid.as_deref(),
        enterprise_id.as_deref(),
        None,
    )
    .await
    .ok();

    let dosage_data = dosage.as_ref().and_then(|v| v.get("data"));
    let dosage_notify_code = dosage_data
        .and_then(|d| d.get("dosageNotifyCode"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => v.to_string(),
        });
    let dosage_notify_zh = dosage_data
        .and_then(|d| d.get("dosageNotifyZh"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let dosage_notify_en = dosage_data
        .and_then(|d| d.get("dosageNotifyEn"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let payment_data = payment.as_ref().and_then(|v| v.get("data"));
    let payment_type = payment_data.and_then(|d| {
        d.as_str().map(|s| s.to_string()).or_else(|| {
            d.get("paymentType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
    });

    let mut combined_quota = serde_json::Map::new();
    if let Some(payload) = dosage.as_ref() {
        combined_quota.insert("dosage".to_string(), payload.clone());
    }
    if let Some(payload) = payment.as_ref() {
        combined_quota.insert("payment".to_string(), payload.clone());
    }
    if let Some(payload) = user_resource.as_ref() {
        combined_quota.insert("userResource".to_string(), payload.clone());
    }
    let quota_raw = if combined_quota.is_empty() {
        None
    } else {
        Some(Value::Object(combined_quota))
    };

    let email_final = if email.is_empty() {
        nickname
            .clone()
            .or_else(|| uid.clone())
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        email
    };

    Ok(CodebuddyOAuthCompletePayload {
        email: email_final,
        uid,
        nickname,
        enterprise_id,
        enterprise_name,
        access_token: access_token.to_string(),
        refresh_token: None,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        domain: None,
        plan_type: None,
        dosage_notify_code,
        dosage_notify_zh,
        dosage_notify_en,
        payment_type,
        quota_raw,
        auth_raw: None,
        profile_raw: Some(account_data),
        usage_raw: user_resource,
        status: Some("normal".to_string()),
        status_reason: None,
        last_checkin_time: None,
        checkin_streak: 0,
        checkin_rewards: None,
    })
}

// 签到相关类型定义
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckinStatusResponse {
    // 官方响应可能是 snake_case 或 camelCase，两种都认。
    #[serde(default, alias = "todayCheckedIn")]
    pub today_checked_in: bool,
    // 缺省按 true：与「有 data 即活动可用」一致；显式 false 才视为关闭。
    // （serde 对 bool 的 default 是 false，会导致缺字段时被误判 inactive）
    #[serde(default = "default_checkin_active_true", alias = "Active")]
    pub active: bool,
    #[serde(default, alias = "streakDays")]
    pub streak_days: i64,
    #[serde(default, alias = "dailyCredit")]
    pub daily_credit: i64,
    #[serde(
        default,
        alias = "todayCredit",
        skip_serializing_if = "Option::is_none"
    )]
    pub today_credit: Option<i64>,
    #[serde(
        default,
        alias = "nextStreakDay",
        skip_serializing_if = "Option::is_none"
    )]
    pub next_streak_day: Option<i64>,
    #[serde(
        default,
        alias = "isStreakDay",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_streak_day: Option<bool>,
    #[serde(
        default,
        alias = "checkinDates",
        skip_serializing_if = "Option::is_none"
    )]
    pub checkin_dates: Option<Vec<String>>,
    #[serde(
        default,
        alias = "streakBonusDays",
        skip_serializing_if = "Option::is_none"
    )]
    pub streak_bonus_days: Option<i64>,
    #[serde(
        default,
        alias = "streakBonusCredit",
        skip_serializing_if = "Option::is_none"
    )]
    pub streak_bonus_credit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckinResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reward: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streak_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_streak_day: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_checkin_in: Option<i64>,
}

fn default_checkin_active_true() -> bool {
    true
}

// 签到 API 函数
// 官方 WorkBuddy CLI/Auth 使用 checkin-activity-status（Buddy 加油站）；
// 旧 BackendProvider 注释为 checkin-status。优先 activity 接口，失败再回退。
pub async fn get_checkin_status(
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<CheckinStatusResponse, String> {
    match fetch_checkin_status_from(
        "/v2/billing/meter/checkin-activity-status",
        access_token,
        uid,
        enterprise_id,
        domain,
    )
    .await
    {
        Ok(status) => Ok(status),
        Err(activity_err) => {
            match fetch_checkin_status_from(
                "/v2/billing/meter/checkin-status",
                access_token,
                uid,
                enterprise_id,
                domain,
            )
            .await
            {
                Ok(status) => Ok(status),
                Err(legacy_err) => Err(format!(
                    "查询签到状态失败: activity=({}) legacy=({})",
                    activity_err, legacy_err
                )),
            }
        }
    }
}

async fn fetch_checkin_status_from(
    path: &str,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<CheckinStatusResponse, String> {
    let client = build_client()?;
    let url = format!("{}{}", CODEBUDDY_API_ENDPOINT, path);

    // 与官方 buildHeaders(session) 对齐
    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&json!({}));

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }
    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 {} 失败: {}", path, e))?;

    let status_code = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 {} 响应失败: {}", path, e))?;

    if !status_code.is_success() {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!(
            "请求 {} 失败 (http={}): {}",
            path,
            status_code.as_u16(),
            message
        ));
    }

    // 与官方 WorkBuddy 一致：仅 code === 0 视为成功
    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("请求 {} 失败 (code={}): {}", path, code, message));
    }

    let data = body
        .get("data")
        .ok_or_else(|| format!("{} 响应缺少 data 字段", path))?;

    let status =
        parse_checkin_status_data(data).map_err(|e| format!("解析 {} data 失败: {}", path, e))?;

    Ok(status)
}

fn json_bool(value: &Value, snake: &str, camel: &str) -> Option<bool> {
    let raw = value.get(snake).or_else(|| value.get(camel))?;
    if let Some(b) = raw.as_bool() {
        return Some(b);
    }
    if let Some(n) = raw.as_i64() {
        return Some(n != 0);
    }
    if let Some(s) = raw.as_str() {
        let lower = s.trim().to_ascii_lowercase();
        if lower == "true" || lower == "1" {
            return Some(true);
        }
        if lower == "false" || lower == "0" {
            return Some(false);
        }
    }
    None
}

fn json_i64(value: &Value, snake: &str, camel: &str) -> Option<i64> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(|v| v.as_i64())
}

fn parse_checkin_status_data(data: &Value) -> Result<CheckinStatusResponse, String> {
    // 先宽松读 bool（兼容 0/1），再走 serde，避免 active 缺省为 false
    let today_checked_in = json_bool(data, "today_checked_in", "todayCheckedIn").unwrap_or(false);
    let active = json_bool(data, "active", "Active").unwrap_or(true);

    if let Ok(mut status) = serde_json::from_value::<CheckinStatusResponse>(data.clone()) {
        // 覆盖可能被 serde default=false 误伤的 active
        if data.get("active").is_none() && data.get("Active").is_none() {
            status.active = true;
        } else if let Some(a) = json_bool(data, "active", "Active") {
            status.active = a;
        }
        if let Some(t) = json_bool(data, "today_checked_in", "todayCheckedIn") {
            status.today_checked_in = t;
        }
        return Ok(status);
    }

    // 宽松解析：兼容字段命名差异，避免整段失败后前端仍显示「未签到」。
    let streak_days = json_i64(data, "streak_days", "streakDays").unwrap_or(0);
    let daily_credit = json_i64(data, "daily_credit", "dailyCredit").unwrap_or(0);
    let today_credit = json_i64(data, "today_credit", "todayCredit");
    let next_streak_day = json_i64(data, "next_streak_day", "nextStreakDay");
    let is_streak_day = json_bool(data, "is_streak_day", "isStreakDay");
    let streak_bonus_days = json_i64(data, "streak_bonus_days", "streakBonusDays");
    let streak_bonus_credit = json_i64(data, "streak_bonus_credit", "streakBonusCredit");
    let checkin_dates = data
        .get("checkin_dates")
        .or_else(|| data.get("checkinDates"))
        .and_then(|v| {
            v.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
        });

    Ok(CheckinStatusResponse {
        today_checked_in,
        active,
        streak_days,
        daily_credit,
        today_credit,
        next_streak_day,
        is_streak_day,
        checkin_dates,
        streak_bonus_days,
        streak_bonus_credit,
    })
}

pub async fn perform_checkin(
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<CheckinResponse, String> {
    let client = build_client()?;
    let url = format!("{}/v2/billing/meter/daily-checkin", CODEBUDDY_API_ENDPOINT);

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&json!({}));

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }
    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 daily-checkin 失败: {}", e))?;

    let status_code = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 daily-checkin 响应失败: {}", e))?;

    if !status_code.is_success() {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!(
            "请求 daily-checkin 失败 (http={}): {}",
            status_code.as_u16(),
            message
        ));
    }

    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    let api_msg = body
        .get("message")
        .or_else(|| body.get("msg"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown error")
        .to_string();

    // 与官方 WorkBuddy 一致：仅 code === 0 视为成功；
    // code != 0 时为业务错误（如已签到），包装为 success=false 返回给前端展示
    if code != 0 {
        return Ok(CheckinResponse {
            success: false,
            message: Some(api_msg),
            reward: None,
            credit: None,
            streak_days: None,
            is_streak_day: None,
            next_checkin_in: None,
        });
    }

    let data = body
        .get("data")
        .ok_or_else(|| "daily-checkin 响应缺少 data 字段".to_string())?;

    let success = data
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true); // code==0 时默认成功

    let message = data
        .get("message")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let reward = data.get("reward").cloned();

    let credit = data
        .get("credit")
        .or_else(|| data.get("today_credit"))
        .and_then(|v| v.as_i64());

    let streak_days = data.get("streak_days").and_then(|v| v.as_i64());

    let is_streak_day = data.get("is_streak_day").and_then(|v| v.as_bool());

    let next_checkin_in = data
        .get("nextCheckinIn")
        .or_else(|| data.get("next_checkin_in"))
        .and_then(|v| v.as_i64());

    Ok(CheckinResponse {
        success,
        message,
        reward,
        credit,
        streak_days,
        is_streak_day,
        next_checkin_in,
    })
}
