use crate::models::github_copilot::{
    GitHubCopilotOAuthCompletePayload, GitHubCopilotOAuthStartResponse,
};
use crate::modules::logger;
use base64::Engine;
use rand::Rng;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

const GITHUB_DEVICE_CODE_ENDPOINT: &str = "https://github.com/login/device/code";
const GITHUB_DEVICE_TOKEN_ENDPOINT: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_ENDPOINT: &str = "https://api.github.com/user";
const GITHUB_USER_EMAILS_ENDPOINT: &str = "https://api.github.com/user/emails";
const GITHUB_COPILOT_TOKEN_ENDPOINT: &str = "https://api.github.com/copilot_internal/v2/token";
const GITHUB_COPILOT_USER_INFO_ENDPOINT: &str = "https://api.github.com/copilot_internal/user";
const GITHUB_OAUTH_CLIENT_ID: &str = "01ab8ac9400c4e429b23";
const GITHUB_OAUTH_SCOPE: &str = "read:user user:email repo workflow";
const APP_USER_AGENT: &str = "antigravity-cockpit-tools";

#[derive(Debug, Clone)]
struct PendingDeviceLogin {
    login_id: String,
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    interval_seconds: u64,
    expires_at: i64,
}

lazy_static::lazy_static! {
    static ref PENDING_DEVICE_LOGIN: Arc<Mutex<Option<PendingDeviceLogin>>> = Arc::new(Mutex::new(None));
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    id: u64,
    login: String,
    name: Option<String>,
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubEmail {
    email: String,
    primary: Option<bool>,
    verified: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: Option<String>,
    expires_at: Option<i64>,
    refresh_in: Option<i64>,
    sku: Option<String>,
    chat_enabled: Option<bool>,
    limited_user_quotas: Option<serde_json::Value>,
    limited_user_reset_date: Option<i64>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotUserInfoResponse {
    copilot_plan: Option<String>,
    quota_snapshots: Option<serde_json::Value>,
    quota_reset_date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CopilotTokenBundle {
    pub token: String,
    pub plan: Option<String>,
    pub chat_enabled: Option<bool>,
    pub expires_at: Option<i64>,
    pub refresh_in: Option<i64>,
    pub quota_snapshots: Option<serde_json::Value>,
    pub quota_reset_date: Option<String>,
    pub limited_user_quotas: Option<serde_json::Value>,
    pub limited_user_reset_date: Option<i64>,
}

fn generate_login_id() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..24).map(|_| rng.gen::<u8>()).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn get_pending_login() -> Option<PendingDeviceLogin> {
    PENDING_DEVICE_LOGIN
        .lock()
        .ok()
        .and_then(|state| state.as_ref().cloned())
}

fn set_pending_login(state: Option<PendingDeviceLogin>) {
    if let Ok(mut guard) = PENDING_DEVICE_LOGIN.lock() {
        *guard = state;
    }
}

fn clear_pending_login_if_matches(login_id: &str) {
    if let Ok(mut guard) = PENDING_DEVICE_LOGIN.lock() {
        if guard.as_ref().map(|state| state.login_id.as_str()) == Some(login_id) {
            *guard = None;
        }
    }
}

fn get_pending_login_for(login_id: &str) -> Result<PendingDeviceLogin, String> {
    let state = get_pending_login().ok_or_else(|| "登录流程已取消，请重新发起授权".to_string())?;
    if state.login_id != login_id {
        return Err("登录会话已变更，请刷新后重试".to_string());
    }
    Ok(state)
}

async fn sleep_with_cancel_check(login_id: &str, total_secs: u64) -> Result<(), String> {
    let ticks = (total_secs.max(1) * 5) as usize;
    for _ in 0..ticks {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = get_pending_login_for(login_id)?;
    }
    Ok(())
}

fn to_start_response(state: &PendingDeviceLogin) -> GitHubCopilotOAuthStartResponse {
    let expires_in = (state.expires_at - now_timestamp()).max(0) as u64;
    GitHubCopilotOAuthStartResponse {
        login_id: state.login_id.clone(),
        user_code: state.user_code.clone(),
        verification_uri: state.verification_uri.clone(),
        verification_uri_complete: state.verification_uri_complete.clone(),
        expires_in,
        interval_seconds: state.interval_seconds,
    }
}

async fn request_device_code() -> Result<DeviceCodeResponse, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(GITHUB_DEVICE_CODE_ENDPOINT)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(ACCEPT, "application/json")
        .form(&[
            ("client_id", GITHUB_OAUTH_CLIENT_ID),
            ("scope", GITHUB_OAUTH_SCOPE),
        ])
        .send()
        .await
        .map_err(|e| format!("请求 GitHub 设备码失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<no-body>".to_string());
        return Err(format!(
            "请求 GitHub 设备码失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    response
        .json::<DeviceCodeResponse>()
        .await
        .map_err(|e| format!("解析 GitHub 设备码响应失败: {}", e))
}

pub async fn start_login() -> Result<GitHubCopilotOAuthStartResponse, String> {
    if let Some(existing) = get_pending_login() {
        if existing.expires_at > now_timestamp() {
            logger::log_info(&format!(
                "GitHub Copilot OAuth 复用登录会话: login_id={}",
                existing.login_id
            ));
            return Ok(to_start_response(&existing));
        }
    }

    let payload = request_device_code().await?;
    let login = PendingDeviceLogin {
        login_id: generate_login_id(),
        device_code: payload.device_code,
        user_code: payload.user_code,
        verification_uri: payload.verification_uri,
        verification_uri_complete: payload.verification_uri_complete,
        interval_seconds: payload.interval.unwrap_or(5).max(1),
        expires_at: now_timestamp() + payload.expires_in as i64,
    };

    logger::log_info(&format!(
        "GitHub Copilot OAuth 登录会话已创建: login_id={}, expires_in={}s, interval={}s",
        login.login_id,
        (login.expires_at - now_timestamp()).max(0),
        login.interval_seconds
    ));

    let response = to_start_response(&login);
    set_pending_login(Some(login));
    Ok(response)
}

async fn exchange_device_token(
    client: &reqwest::Client,
    device_code: &str,
) -> Result<DeviceTokenResponse, String> {
    let response = client
        .post(GITHUB_DEVICE_TOKEN_ENDPOINT)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(ACCEPT, "application/json")
        .form(&[
            ("client_id", GITHUB_OAUTH_CLIENT_ID),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("请求 GitHub access token 失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<no-body>".to_string());
        return Err(format!(
            "请求 GitHub access token 失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    response
        .json::<DeviceTokenResponse>()
        .await
        .map_err(|e| format!("解析 GitHub access token 响应失败: {}", e))
}

async fn fetch_github_user(
    client: &reqwest::Client,
    github_access_token: &str,
) -> Result<GitHubUser, String> {
    let response = client
        .get(GITHUB_USER_ENDPOINT)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(ACCEPT, "application/vnd.github+json")
        .header(AUTHORIZATION, format!("Bearer {}", github_access_token))
        .send()
        .await
        .map_err(|e| format!("请求 GitHub 用户信息失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<no-body>".to_string());
        return Err(format!(
            "请求 GitHub 用户信息失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    response
        .json::<GitHubUser>()
        .await
        .map_err(|e| format!("解析 GitHub 用户信息失败: {}", e))
}

async fn fetch_github_email(
    client: &reqwest::Client,
    github_access_token: &str,
) -> Result<Option<String>, String> {
    let response = client
        .get(GITHUB_USER_EMAILS_ENDPOINT)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(ACCEPT, "application/vnd.github+json")
        .header(AUTHORIZATION, format!("Bearer {}", github_access_token))
        .send()
        .await
        .map_err(|e| format!("请求 GitHub 邮箱列表失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<no-body>".to_string());
        return Err(format!(
            "请求 GitHub 邮箱列表失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    let emails = response
        .json::<Vec<GitHubEmail>>()
        .await
        .map_err(|e| format!("解析 GitHub 邮箱列表失败: {}", e))?;

    let primary = emails
        .iter()
        .find(|item| item.primary.unwrap_or(false) && item.verified.unwrap_or(false))
        .map(|item| item.email.clone());
    if primary.is_some() {
        return Ok(primary);
    }
    Ok(emails
        .iter()
        .find(|item| item.verified.unwrap_or(false))
        .map(|item| item.email.clone()))
}

async fn fetch_copilot_token(
    client: &reqwest::Client,
    github_access_token: &str,
) -> Result<CopilotTokenBundle, String> {
    let response = client
        .get(GITHUB_COPILOT_TOKEN_ENDPOINT)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(ACCEPT, "application/json")
        .header("X-GitHub-Api-Version", "2025-04-01")
        .header(AUTHORIZATION, format!("token {}", github_access_token))
        .send()
        .await
        .map_err(|e| format!("请求 Copilot token 失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<no-body>".to_string());
        return Err(format!(
            "请求 Copilot token 失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    let payload = response
        .json::<CopilotTokenResponse>()
        .await
        .map_err(|e| format!("解析 Copilot token 响应失败: {}", e))?;

    let token = payload.token.ok_or_else(|| {
        payload
            .message
            .unwrap_or_else(|| "Copilot token 缺失".to_string())
    })?;

    let user_info = fetch_copilot_user_info(client, github_access_token)
        .await
        .ok();

    Ok(CopilotTokenBundle {
        token,
        plan: user_info
            .as_ref()
            .and_then(|info| info.copilot_plan.clone())
            .or(payload.sku),
        chat_enabled: payload.chat_enabled,
        expires_at: payload.expires_at,
        refresh_in: payload.refresh_in,
        quota_snapshots: user_info
            .as_ref()
            .and_then(|info| info.quota_snapshots.clone()),
        quota_reset_date: user_info
            .as_ref()
            .and_then(|info| info.quota_reset_date.clone()),
        limited_user_quotas: payload.limited_user_quotas,
        limited_user_reset_date: payload.limited_user_reset_date,
    })
}

async fn fetch_copilot_user_info(
    client: &reqwest::Client,
    github_access_token: &str,
) -> Result<CopilotUserInfoResponse, String> {
    let response = client
        .get(GITHUB_COPILOT_USER_INFO_ENDPOINT)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(ACCEPT, "application/json")
        .header("X-GitHub-Api-Version", "2025-04-01")
        .header(AUTHORIZATION, format!("token {}", github_access_token))
        .send()
        .await
        .map_err(|e| format!("请求 Copilot user 信息失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<no-body>".to_string());
        return Err(format!(
            "请求 Copilot user 信息失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    response
        .json::<CopilotUserInfoResponse>()
        .await
        .map_err(|e| format!("解析 Copilot user 信息失败: {}", e))
}

pub async fn refresh_copilot_token(
    github_access_token: &str,
) -> Result<CopilotTokenBundle, String> {
    let client = reqwest::Client::new();
    fetch_copilot_token(&client, github_access_token).await
}

pub async fn complete_login(login_id: &str) -> Result<GitHubCopilotOAuthCompletePayload, String> {
    let pending = get_pending_login_for(login_id)?;
    if pending.expires_at <= now_timestamp() {
        clear_pending_login_if_matches(login_id);
        return Err("登录会话已过期，请重新发起授权".to_string());
    }

    logger::log_info(&format!(
        "GitHub Copilot OAuth 开始轮询授权结果: login_id={}",
        pending.login_id
    ));

    let client = reqwest::Client::new();
    let mut interval_seconds = pending.interval_seconds.max(1);

    let token_result = loop {
        let pending = get_pending_login_for(login_id)?;
        if now_timestamp() >= pending.expires_at {
            clear_pending_login_if_matches(login_id);
            return Err("等待 GitHub 授权超时，请重新发起授权".to_string());
        }

        let response = exchange_device_token(&client, &pending.device_code).await?;

        if let Some(access_token) = response.access_token {
            break (
                access_token,
                response.token_type.clone(),
                response.scope.clone(),
            );
        }

        match response.error.as_deref() {
            Some("authorization_pending") => {
                sleep_with_cancel_check(login_id, interval_seconds).await?;
            }
            Some("slow_down") => {
                interval_seconds += 5;
                sleep_with_cancel_check(login_id, interval_seconds).await?;
            }
            Some("expired_token") => {
                clear_pending_login_if_matches(login_id);
                return Err("授权码已过期，请重新发起授权".to_string());
            }
            Some("access_denied") => {
                clear_pending_login_if_matches(login_id);
                return Err("用户取消了 GitHub 授权".to_string());
            }
            Some(other) => {
                let detail = response
                    .error_description
                    .unwrap_or_else(|| "未知错误".to_string());
                clear_pending_login_if_matches(login_id);
                return Err(format!("GitHub 授权失败: {} ({})", other, detail));
            }
            None => {
                sleep_with_cancel_check(login_id, interval_seconds).await?;
            }
        }
    };

    let github_access_token = token_result.0;
    let _ = get_pending_login_for(login_id)?;
    let github_user = fetch_github_user(&client, &github_access_token).await?;
    let github_email = if github_user.email.is_some() {
        github_user.email.clone()
    } else {
        fetch_github_email(&client, &github_access_token).await?
    };
    let _ = get_pending_login_for(login_id)?;
    let copilot = fetch_copilot_token(&client, &github_access_token).await?;

    clear_pending_login_if_matches(login_id);

    logger::log_info(&format!(
        "GitHub Copilot OAuth 登录完成: login_id={}, github_login={}",
        pending.login_id, github_user.login
    ));

    Ok(GitHubCopilotOAuthCompletePayload {
        github_login: github_user.login,
        github_id: github_user.id,
        github_name: github_user.name,
        github_email,
        github_access_token,
        github_token_type: token_result.1,
        github_scope: token_result.2,
        copilot_token: copilot.token,
        copilot_plan: copilot.plan,
        copilot_chat_enabled: copilot.chat_enabled,
        copilot_expires_at: copilot.expires_at,
        copilot_refresh_in: copilot.refresh_in,
        copilot_quota_snapshots: copilot.quota_snapshots,
        copilot_quota_reset_date: copilot.quota_reset_date,
        copilot_limited_user_quotas: copilot.limited_user_quotas,
        copilot_limited_user_reset_date: copilot.limited_user_reset_date,
    })
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    let current = get_pending_login();
    match (current, login_id) {
        (Some(state), Some(id)) if state.login_id != id => {
            Err("登录会话不匹配，取消失败".to_string())
        }
        (Some(_), _) => {
            set_pending_login(None);
            Ok(())
        }
        (None, _) => Ok(()),
    }
}

pub async fn build_payload_from_github_access_token(
    github_access_token: &str,
) -> Result<GitHubCopilotOAuthCompletePayload, String> {
    let client = reqwest::Client::new();
    let github_user = fetch_github_user(&client, github_access_token).await?;
    let github_email = if github_user.email.is_some() {
        github_user.email.clone()
    } else {
        fetch_github_email(&client, github_access_token).await?
    };
    let copilot = fetch_copilot_token(&client, github_access_token).await?;

    Ok(GitHubCopilotOAuthCompletePayload {
        github_login: github_user.login,
        github_id: github_user.id,
        github_name: github_user.name,
        github_email,
        github_access_token: github_access_token.to_string(),
        github_token_type: None,
        github_scope: None,
        copilot_token: copilot.token,
        copilot_plan: copilot.plan,
        copilot_chat_enabled: copilot.chat_enabled,
        copilot_expires_at: copilot.expires_at,
        copilot_refresh_in: copilot.refresh_in,
        copilot_quota_snapshots: copilot.quota_snapshots,
        copilot_quota_reset_date: copilot.quota_reset_date,
        copilot_limited_user_quotas: copilot.limited_user_quotas,
        copilot_limited_user_reset_date: copilot.limited_user_reset_date,
    })
}
