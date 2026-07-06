use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;
use uuid::Uuid;

use crate::models::qoderwork_cn::{QoderworkCnAccount, QoderworkCnOAuthStartResponse};
use crate::modules::{logger, qoderwork_cn_account};

const OAUTH_TIMEOUT_SECONDS: i64 = 600;
const OAUTH_POLL_INTERVAL_MS: u64 = 1000;
const DEFAULT_LOGIN_BASE_URL: &str = "https://qoder.com.cn/device/selectAccounts";
const DEFAULT_OPENAPI_BASE_URL: &str = "https://openapi.qoder.com.cn";
const QODERWORK_CN_CLI_BROWSER_LOGIN_CLIENT_ID: &str = "e883ade2-e6e3-4d6d-adf7-f92ceff5fdcb";
const QODERWORK_CN_DEVICE_LOGIN_CHALLENGE_METHOD: &str = "S256";
const DEVICE_TOKEN_POLL_PATH: &str = "/api/v1/deviceToken/poll";
const USER_INFO_PATH: &str = "/api/v1/userinfo";
const CREDIT_USAGE_PATH: &str = "/api/v2/quota/usage";
const DEVICE_TOKEN_REFRESH_PATH: &str = "/api/v1/deviceToken/refresh";

#[derive(Debug, Clone)]
struct PendingOAuthState {
    login_id: String,
    expected_nonce: String,
    code_verifier: String,
    challenge_method: String,
    openapi_base_url: String,
    verification_uri: String,
    expires_at: i64,
    cancelled: bool,
}

#[derive(Debug, Deserialize)]
struct DeviceTokenPollResult {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    refresh_token_expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceTokenRefreshResult {
    #[serde(default)]
    device_token: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

lazy_static::lazy_static! {
    static ref PENDING_OAUTH_STATE: Arc<Mutex<Option<PendingOAuthState>>> = Arc::new(Mutex::new(None));
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
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

fn generate_pkce_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn generate_login_nonce() -> String {
    Uuid::new_v4().simple().to_string()
}

fn build_cli_device_login_url(
    login_base_url: &str,
    nonce: &str,
    challenge: &str,
    challenge_method: &str,
) -> Result<String, String> {
    let mut url = Url::parse(login_base_url)
        .map_err(|err| format!("解析 QoderWork CN 登录地址失败: {}", err))?;
    {
        let mut query_pairs = url.query_pairs_mut();
        query_pairs.append_pair("nonce", nonce);
        query_pairs.append_pair("challenge", challenge);
        query_pairs.append_pair("challenge_method", challenge_method);
        query_pairs.append_pair("client_id", QODERWORK_CN_CLI_BROWSER_LOGIN_CLIENT_ID);
    }
    Ok(url.to_string())
}

fn build_reqwest_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|err| format!("创建 QoderWork CN HTTP 客户端失败: {}", err))
}

async fn poll_device_token_once(
    client: &reqwest::Client,
    openapi_base_url: &str,
    nonce: &str,
    verifier: &str,
    challenge_method: &str,
) -> Result<Option<DeviceTokenPollResult>, String> {
    let response = client
        .get(format!("{}{}", openapi_base_url, DEVICE_TOKEN_POLL_PATH))
        .query(&[
            ("nonce", nonce),
            ("verifier", verifier),
            ("challenge_method", challenge_method),
        ])
        .send()
        .await
        .map_err(|err| format!("轮询 QoderWork CN device token 失败: {}", err))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "轮询 QoderWork CN device token 失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    let payload = response
        .json::<DeviceTokenPollResult>()
        .await
        .map_err(|err| format!("解析 QoderWork CN device token 响应失败: {}", err))?;
    if payload
        .token
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .is_some()
    {
        return Ok(Some(payload));
    }
    Ok(None)
}

async fn fetch_user_info(
    client: &reqwest::Client,
    openapi_base_url: &str,
    token: &str,
) -> Result<Value, String> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};

    let mut headers = HeaderMap::new();
    let bearer = format!("Bearer {}", token);
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&bearer).map_err(|e| format!("构造授权头失败: {}", e))?);
    headers.insert(ACCEPT, HeaderValue::from_str("application/json").unwrap());

    let response = client
        .get(format!("{}{}", openapi_base_url, USER_INFO_PATH))
        .headers(headers)
        .send()
        .await
        .map_err(|err| format!("请求 QoderWork CN userinfo 失败: {}", err))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("请求 QoderWork CN userinfo 失败: status={}, body_len={}", status, body.len()));
    }

    response.json::<Value>().await.map_err(|e| format!("解析 userinfo 响应失败: {}", e))
}

async fn fetch_credit_usage(
    client: &reqwest::Client,
    openapi_base_url: &str,
    token: &str,
) -> Result<Value, String> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};

    let mut headers = HeaderMap::new();
    let bearer = format!("Bearer {}", token);
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&bearer).map_err(|e| format!("构造授权头失败: {}", e))?);
    headers.insert(ACCEPT, HeaderValue::from_str("application/json").unwrap());

    let response = client
        .get(format!("{}{}", openapi_base_url, CREDIT_USAGE_PATH))
        .headers(headers)
        .send()
        .await
        .map_err(|err| format!("请求 QoderWork CN quota/usage 失败: {}", err))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("请求 QoderWork CN quota/usage 失败: status={}, body_len={}", status, body.len()));
    }

    response.json::<Value>().await.map_err(|e| format!("解析 quota/usage 响应失败: {}", e))
}

fn parse_expire_timestamp_ms(raw: Option<&str>) -> Option<i64> {
    let text = normalize_non_empty(raw)?;
    if let Ok(number) = text.parse::<i64>() {
        let millis = if number > 1_000_000_000_000 {
            number
        } else {
            number.saturating_mul(1000)
        };
        return Some(millis);
    }
    chrono::DateTime::parse_from_rfc3339(&text)
        .ok()
        .map(|value| value.timestamp_millis())
}

fn clear_pending_if_matches(login_id: &str) {
    if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
        if guard.as_ref().map(|s| s.login_id.as_str()) == Some(login_id) {
            *guard = None;
        }
    }
}

// ==================== 公开 API ====================

pub async fn start_login() -> Result<QoderworkCnOAuthStartResponse, String> {
    logger::log_info("[QoderWorkCN OAuth] 开始创建登录会话");
    let expected_nonce = generate_login_nonce();
    let code_verifier = generate_pkce_verifier();
    let challenge_method = QODERWORK_CN_DEVICE_LOGIN_CHALLENGE_METHOD.to_string();
    let code_challenge = generate_pkce_challenge(&code_verifier);
    let verification_uri = build_cli_device_login_url(
        DEFAULT_LOGIN_BASE_URL,
        &expected_nonce,
        &code_challenge,
        &challenge_method,
    )?;
    let login_id = Uuid::new_v4().to_string();

    logger::log_info(&format!(
        "[QoderWorkCN OAuth] 已生成 device login 链接: login_id={}, verification_uri_len={}",
        login_id,
        verification_uri.len()
    ));

    let state = PendingOAuthState {
        login_id: login_id.clone(),
        expected_nonce: expected_nonce.clone(),
        code_verifier,
        challenge_method: challenge_method.clone(),
        openapi_base_url: DEFAULT_OPENAPI_BASE_URL.to_string(),
        verification_uri: verification_uri.clone(),
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS,
        cancelled: false,
    };

    {
        let mut guard = PENDING_OAUTH_STATE
            .lock()
            .map_err(|_| "获取 QoderWork CN OAuth 状态锁失败".to_string())?;
        *guard = Some(state);
    }

    Ok(QoderworkCnOAuthStartResponse {
        login_id,
        verification_uri,
        expires_in: OAUTH_TIMEOUT_SECONDS as u64,
        interval_seconds: (OAUTH_POLL_INTERVAL_MS / 1000).max(1),
        callback_url: None,
    })
}

pub async fn complete_login(login_id: &str) -> Result<QoderworkCnAccount, String> {
    logger::log_info(&format!(
        "[QoderWorkCN OAuth] 开始等待登录完成: login_id={}",
        login_id
    ));
    let wait_started = Instant::now();
    let mut next_wait_log_at = Duration::from_secs(5);
    let client = build_reqwest_client()?;
    let mut last_poll_error: Option<String> = None;

    loop {
        let snapshot = {
            let guard = PENDING_OAUTH_STATE
                .lock()
                .map_err(|_| "获取 QoderWork CN OAuth 状态锁失败".to_string())?;
            let state = guard
                .as_ref()
                .ok_or_else(|| "没有进行中的 QoderWork CN OAuth 登录会话".to_string())?;

            if state.login_id != login_id {
                return Err("QoderWork CN OAuth 登录会话已变更，请重新发起".to_string());
            }
            if state.cancelled {
                return Err("QoderWork CN OAuth 登录已取消".to_string());
            }
            if now_timestamp() > state.expires_at {
                clear_pending_if_matches(login_id);
                return Err(
                    last_poll_error.unwrap_or_else(|| "QoderWork CN OAuth 登录已超时，请重试".to_string())
                );
            }

            (
                state.expected_nonce.clone(),
                state.code_verifier.clone(),
                state.challenge_method.clone(),
                state.openapi_base_url.clone(),
            )
        };

        match poll_device_token_once(&client, &snapshot.3, &snapshot.0, &snapshot.1, &snapshot.2)
            .await
        {
            Ok(Some(token_data)) => {
                logger::log_info(&format!(
                    "[QoderWorkCN OAuth] deviceToken/poll 命中: login_id={}, elapsed={}ms",
                    login_id,
                    wait_started.elapsed().as_millis()
                ));

                let access_token = normalize_non_empty(token_data.token.as_deref())
                    .ok_or_else(|| "QoderWork CN device token 响应缺少 token".to_string())?;
                let refresh_token = normalize_non_empty(token_data.refresh_token.as_deref());
                let token_expires_at = parse_expire_timestamp_ms(token_data.expires_at.as_deref());
                let user_id = normalize_non_empty(token_data.user_id.as_deref());

                // Fetch user info
                let user_info = match fetch_user_info(&client, &snapshot.3, &access_token).await {
                    Ok(value) => {
                        logger::log_info(&format!(
                            "[QoderWorkCN OAuth] userinfo 响应: {}",
                            serde_json::to_string_pretty(&value).unwrap_or_default()
                        ));
                        Some(value)
                    }
                    Err(err) => {
                        logger::log_warn(&format!(
                            "[QoderWorkCN OAuth] 获取 userinfo 失败: {}",
                            err
                        ));
                        None
                    }
                };

                // Fetch quota
                let quota_raw = match fetch_credit_usage(&client, &snapshot.3, &access_token).await {
                    Ok(value) => Some(value),
                    Err(err) => {
                        logger::log_warn(&format!(
                            "[QoderWorkCN OAuth] 获取 quota/usage 失败: {}",
                            err
                        ));
                        None
                    }
                };

                // Extract display name and email from user_info
                // Priority for display_name: userinfo.name (preferred) > userinfo.email > userinfo.username
                let display_name = user_info.as_ref().and_then(|info| {
                    info.get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                });

                // Priority for email: userinfo.email > userinfo.username > "unknown"
                let email = user_info
                    .as_ref()
                    .and_then(|info| {
                        info.get("email")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .or_else(|| {
                        user_info
                            .as_ref()
                            .and_then(|info| {
                                info.get("username")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                            })
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                // Priority for user_id: token_data.user_id > userinfo.id
                let final_user_id = user_id.or_else(|| {
                    user_info
                        .as_ref()
                        .and_then(|info| info.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                });

                let account = qoderwork_cn_account::upsert_account_from_payload(
                    email,
                    final_user_id,
                    display_name,
                    Some(access_token.clone()),
                    refresh_token.clone(),
                    token_expires_at,
                    quota_raw,
                )?;

                // 主动写入 QoderWork CN 认证文件（auth-v2.dat, .auth-cn/id, .auth-cn/user, .status.json）
                // 这样备份时就能捕获完整的会话数据，切换后 QoderWork CN 能保持登录状态
                let avatar_url = user_info.as_ref().and_then(|info| {
                    info.get("avatar").and_then(|v| v.as_str()).map(|s| s.to_string())
                });
                if let Err(err) = qoderwork_cn_account::write_oauth_session_files(
                    &access_token,
                    refresh_token.as_deref(),
                    token_expires_at,
                    account.user_id.as_deref(),
                    &account.email,
                    account.display_name.as_deref(),
                    avatar_url.as_deref(),
                    user_info.as_ref(),
                ) {
                    logger::log_warn(&format!(
                        "[QoderWorkCN OAuth] 写入会话文件失败: {}",
                        err
                    ));
                }

                // 备份当前会话文件到新创建的账号
                if let Err(err) = qoderwork_cn_account::backup_current_session_to(&account.id) {
                    logger::log_warn(&format!(
                        "[QoderWorkCN OAuth] 备份会话失败: account_id={}, error={}",
                        account.id, err
                    ));
                } else {
                    logger::log_info(&format!(
                        "[QoderWorkCN OAuth] 会话备份完成: account_id={}",
                        account.id
                    ));
                }

                clear_pending_if_matches(login_id);
                logger::log_info(&format!(
                    "[QoderWorkCN OAuth] 登录完成: login_id={}, account_id={}, email={}",
                    login_id, account.id, account.email
                ));
                return Ok(account);
            }
            Ok(None) => {}
            Err(err) => {
                last_poll_error = Some(err.clone());
                logger::log_warn(&format!(
                    "[QoderWorkCN OAuth] deviceToken/poll 失败: login_id={}, error={}",
                    login_id, err
                ));
            }
        }

        let elapsed = wait_started.elapsed();
        if elapsed >= next_wait_log_at {
            logger::log_info(&format!(
                "[QoderWorkCN OAuth] 等待 device token 中: login_id={}, elapsed={}s",
                login_id,
                elapsed.as_secs()
            ));
            next_wait_log_at += Duration::from_secs(5);
        }
        tokio::time::sleep(Duration::from_millis(OAUTH_POLL_INTERVAL_MS)).await;
    }
}

pub fn peek_pending_login() -> Option<QoderworkCnOAuthStartResponse> {
    let guard = PENDING_OAUTH_STATE.lock().ok()?;
    let state = guard.as_ref()?;
    if state.cancelled {
        return None;
    }
    let now = now_timestamp();
    if now > state.expires_at {
        return None;
    }

    Some(QoderworkCnOAuthStartResponse {
        login_id: state.login_id.clone(),
        verification_uri: state.verification_uri.clone(),
        expires_in: (state.expires_at - now).max(0) as u64,
        interval_seconds: (OAUTH_POLL_INTERVAL_MS / 1000).max(1),
        callback_url: None,
    })
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    let mut guard = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "获取 QoderWork CN OAuth 状态锁失败".to_string())?;

    let Some(current) = guard.as_ref() else {
        return Ok(());
    };

    if let Some(target) = login_id {
        if current.login_id != target {
            return Ok(());
        }
    }

    logger::log_info(&format!(
        "[QoderWorkCN OAuth] 取消登录会话: login_id={}",
        current.login_id
    ));

    *guard = None;
    Ok(())
}

/// 从 token 创建账号（用于 Token 导入）
pub async fn build_account_from_token(token: &str) -> Result<QoderworkCnAccount, String> {
    let client = build_reqwest_client()?;

    let user_info = match fetch_user_info(&client, DEFAULT_OPENAPI_BASE_URL, token).await {
        Ok(value) => Some(value),
        Err(err) => {
            logger::log_warn(&format!(
                "[QoderWorkCN OAuth] token 导入获取 userinfo 失败: {}",
                err
            ));
            None
        }
    };

    let quota_raw = match fetch_credit_usage(&client, DEFAULT_OPENAPI_BASE_URL, token).await {
        Ok(value) => Some(value),
        Err(err) => {
            logger::log_warn(&format!(
                "[QoderWorkCN OAuth] token 导入获取 quota/usage 失败: {}",
                err
            ));
            None
        }
    };

    let email = user_info
        .as_ref()
        .and_then(|info| info.get("email").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .or_else(|| {
            user_info
                .as_ref()
                .and_then(|info| info.get("username").and_then(|v| v.as_str()).map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "unknown".to_string());
    let user_id = user_info
        .as_ref()
        .and_then(|info| info.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()));
    let display_name = user_info
        .as_ref()
        .and_then(|info| info.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()));

    qoderwork_cn_account::upsert_account_from_payload(
        email,
        user_id,
        display_name,
        Some(token.to_string()),
        None,
        None,
        quota_raw,
    )
}

/// 刷新单个账号的 token 和配额
pub async fn refresh_account_from_openapi(account_id: &str) -> Result<QoderworkCnAccount, String> {
    let result = refresh_account_from_openapi_once(account_id).await;
    if let Err(err) = &result {
        let _ = qoderwork_cn_account::update_quota_query_error(account_id, Some(err.clone()));
    }
    result
}

async fn refresh_account_from_openapi_once(account_id: &str) -> Result<QoderworkCnAccount, String> {
    let target = qoderwork_cn_account::load_account(account_id)
        .ok_or_else(|| format!("QoderWork CN 账号不存在: {}", account_id))?;

    let access_token = target
        .access_token
        .as_ref()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "QoderWork CN 账号缺少 access token，请重新登录后再刷新".to_string())?
        .clone();

    let client = build_reqwest_client()?;

    // Try to refresh token first if we have a refresh_token
    let mut current_token = access_token.clone();
    if let Some(refresh_token) = target.refresh_token.as_ref().filter(|t| !t.is_empty()) {
        match refresh_token_api(&client, DEFAULT_OPENAPI_BASE_URL, refresh_token).await {
            Ok(refresh_result) => {
                if let Some(new_token) = refresh_result
                    .token
                    .or(refresh_result.device_token)
                {
                    current_token = new_token;
                }
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[QoderWorkCN Refresh] token 刷新失败，将使用现有 token: {}",
                    err
                ));
            }
        }
    }

    // Fetch user info
    let user_info = match fetch_user_info(&client, DEFAULT_OPENAPI_BASE_URL, &current_token).await {
        Ok(value) => Some(value),
        Err(err) => {
            logger::log_warn(&format!("[QoderWorkCN Refresh] 获取 userinfo 失败: {}", err));
            None
        }
    };

    // Fetch quota
    let mut quota_query_error: Option<String> = None;
    let quota_raw = match fetch_credit_usage(&client, DEFAULT_OPENAPI_BASE_URL, &current_token).await {
        Ok(value) => Some(value),
        Err(err) => {
            logger::log_warn(&format!("[QoderWorkCN Refresh] 获取 quota/usage 失败: {}", err));
            quota_query_error = Some(err.clone());
            target.quota_raw.clone()
        }
    };

    let email = user_info
        .as_ref()
        .and_then(|info| info.get("email").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .or_else(|| {
            user_info
                .as_ref()
                .and_then(|info| info.get("username").and_then(|v| v.as_str()).map(|s| s.to_string()))
        })
        .unwrap_or_else(|| target.email.clone());
    let user_id = user_info
        .as_ref()
        .and_then(|info| info.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .or_else(|| target.user_id.clone());
    let display_name = user_info
        .as_ref()
        .and_then(|info| info.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .or_else(|| target.display_name.clone());

    let refreshed = qoderwork_cn_account::upsert_account_from_payload(
        email,
        user_id,
        display_name,
        Some(current_token),
        target.refresh_token.clone(),
        target.token_expires_at,
        quota_raw,
    )?;

    if refreshed.id == target.id {
        if let Some(error_msg) = quota_query_error {
            if let Some(updated) =
                qoderwork_cn_account::update_quota_query_error(&refreshed.id, Some(error_msg))?
            {
                return Ok(updated);
            }
        }
    }

    Ok(refreshed)
}

async fn refresh_token_api(
    client: &reqwest::Client,
    openapi_base_url: &str,
    refresh_token: &str,
) -> Result<DeviceTokenRefreshResult, String> {
    let response = client
        .post(format!("{}{}", openapi_base_url, DEVICE_TOKEN_REFRESH_PATH))
        .json(&serde_json::json!({
            "refresh_token": refresh_token
        }))
        .send()
        .await
        .map_err(|err| format!("请求 token 刷新失败: {}", err))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("token 刷新失败: status={}, body_len={}", status, body.len()));
    }

    response
        .json::<DeviceTokenRefreshResult>()
        .await
        .map_err(|e| format!("解析 token 刷新响应失败: {}", e))
}

/// 批量刷新所有账号
pub async fn refresh_all_accounts_from_openapi() -> Result<i32, String> {
    let accounts = qoderwork_cn_account::list_accounts();
    if accounts.is_empty() {
        return Ok(0);
    }

    let mut refreshed_count = 0i32;
    for account in &accounts {
        if account.access_token.is_none() {
            continue;
        }
        match refresh_account_from_openapi(&account.id).await {
            Ok(_) => {
                refreshed_count += 1;
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[QoderWorkCN Refresh] 刷新账号失败: id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    Ok(refreshed_count)
}
