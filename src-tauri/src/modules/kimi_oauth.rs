//! Kimi Code OAuth device flow (RFC 8628).
//! Aligned with MoonshotAI/kimi-code packages/oauth.

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::error::Error as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

use crate::models::kimi::KimiOAuthStartResponse;

pub const OAUTH_HOST: &str = "https://auth.kimi.com";
pub const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
pub const API_BASE_URL: &str = "https://api.kimi.com/coding/v1";
pub const CREDENTIAL_FILE_NAME: &str = "kimi-code.json";
pub const OAUTH_KEY: &str = "oauth/kimi-code";
pub const PROVIDER_NAME: &str = "managed:kimi-code";
pub const X_MSH_PLATFORM: &str = "cockpit_tools";
pub const X_MSH_VERSION: &str = "1.0.0";

const DEVICE_AUTH_URL: &str = "https://auth.kimi.com/api/oauth/device_authorization";
const TOKEN_URL: &str = "https://auth.kimi.com/api/oauth/token";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const PENDING_FILE: &str = "kimi_oauth_pending.json";
const DEFAULT_INTERVAL_SECONDS: u64 = 5;
const MAX_LOGIN_SECONDS: i64 = 15 * 60;
const MAX_CONSECUTIVE_POLL_TRANSPORT_ERRORS: u8 = 3;
const REFRESH_LEAD_SECONDS: i64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingLogin {
    login_id: String,
    device_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    user_code: String,
    device_id: String,
    expires_at: i64,
    interval_seconds: u64,
    cancelled: bool,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    #[serde(default)]
    verification_uri: Option<String>,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KimiTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    pub expires_in: i64,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: Option<String>,
    error_description: Option<String>,
}

enum PollResult {
    Pending,
    SlowDown,
    TransportFailure(String),
    Complete(KimiTokenResponse),
}

lazy_static::lazy_static! {
    static ref PENDING_LOGIN: Arc<Mutex<Option<PendingLogin>>> = Arc::new(Mutex::new(None));
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("创建 Kimi OAuth 客户端失败: {}", error))
}

fn format_request_error(context: &str, error: &reqwest::Error) -> String {
    let category = if error.is_timeout() {
        "请求超时"
    } else if error.is_connect() {
        "连接失败"
    } else if error.is_body() {
        "响应传输失败"
    } else {
        "请求失败"
    };
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if !message.is_empty() && causes.last() != Some(&message) {
            causes.push(message);
        }
        source = cause.source();
    }
    if causes.is_empty() {
        format!("{}（{}）: {}", context, category, error)
    } else {
        format!(
            "{}（{}）: {}；原因: {}",
            context,
            category,
            error,
            causes.join(" -> ")
        )
    }
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

pub fn default_kimi_home() -> Result<std::path::PathBuf, String> {
    if let Ok(override_home) = std::env::var("KIMI_CODE_HOME") {
        let trimmed = override_home.trim();
        if !trimmed.is_empty() {
            return Ok(std::path::PathBuf::from(trimmed));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| "无法定位用户主目录".to_string())?;
    Ok(home.join(".kimi-code"))
}

pub fn ensure_device_id(home: &std::path::Path) -> Result<String, String> {
    let path = home.join("device_id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    std::fs::create_dir_all(home).map_err(|error| {
        format!(
            "创建 Kimi Code 数据目录失败: path={}, error={}",
            home.display(),
            error
        )
    })?;
    let id = Uuid::new_v4().to_string();
    crate::modules::atomic_write::write_string_atomic(&path, &format!("{}\n", id))?;
    Ok(id)
}

fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "cockpit".to_string())
}

fn device_headers(device_id: &str) -> Vec<(String, String)> {
    let hostname = device_name();
    let model = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    vec![
        ("X-Msh-Platform".to_string(), X_MSH_PLATFORM.to_string()),
        ("X-Msh-Version".to_string(), X_MSH_VERSION.to_string()),
        ("X-Msh-Device-Name".to_string(), hostname),
        ("X-Msh-Device-Model".to_string(), model),
        (
            "X-Msh-Os-Version".to_string(),
            std::env::consts::OS.to_string(),
        ),
        ("X-Msh-Device-Id".to_string(), device_id.to_string()),
    ]
}

fn apply_device_headers(
    request: reqwest::RequestBuilder,
    device_id: &str,
) -> reqwest::RequestBuilder {
    let mut request = request;
    for (key, value) in device_headers(device_id) {
        request = request.header(key, value);
    }
    request
}

fn set_pending(value: Option<PendingLogin>) -> Result<(), String> {
    if let Ok(mut guard) = PENDING_LOGIN.lock() {
        *guard = value.clone();
    }
    match value {
        Some(state) => crate::modules::oauth_pending_state::save(PENDING_FILE, &state),
        None => crate::modules::oauth_pending_state::clear(PENDING_FILE),
    }
}

fn hydrate_pending() {
    if let Ok(mut guard) = PENDING_LOGIN.lock() {
        if guard.is_none() {
            match crate::modules::oauth_pending_state::load::<PendingLogin>(PENDING_FILE) {
                Ok(Some(state)) if !state.cancelled && state.expires_at > now_ts() => {
                    *guard = Some(state);
                }
                Ok(Some(_)) => {
                    let _ = crate::modules::oauth_pending_state::clear(PENDING_FILE);
                }
                Ok(None) => {}
                Err(error) => {
                    crate::modules::logger::log_warn(&format!(
                        "[Kimi OAuth] 读取 pending 状态失败，已忽略: {}",
                        error
                    ));
                }
            }
        }
    }
}

fn pending_for(login_id: &str) -> Result<PendingLogin, String> {
    hydrate_pending();
    let state = PENDING_LOGIN
        .lock()
        .map_err(|_| "获取 Kimi OAuth 状态锁失败".to_string())?
        .clone()
        .ok_or_else(|| "Kimi OAuth 登录流程不存在，请重新发起".to_string())?;
    if state.login_id != login_id {
        return Err("Kimi OAuth 登录会话已变更，请重新发起".to_string());
    }
    if state.cancelled {
        return Err("Kimi OAuth 登录已取消".to_string());
    }
    if now_ts() >= state.expires_at {
        return Err("Kimi OAuth 登录已超时，请重试".to_string());
    }
    Ok(state)
}

pub async fn start_login() -> Result<KimiOAuthStartResponse, String> {
    let home = default_kimi_home()?;
    let device_id = ensure_device_id(&home)?;
    let client = http_client()?;
    let mut request = client
        .post(DEVICE_AUTH_URL)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "application/json")
        .form(&[("client_id", CLIENT_ID)]);
    request = apply_device_headers(request, &device_id);
    let response = request
        .send()
        .await
        .map_err(|error| format_request_error("发起 Kimi device flow 失败", &error))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Kimi device flow 响应失败: {}", error))?;
    if !status.is_success() {
        return Err(format!(
            "Kimi device flow 返回 {}：{}",
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        ));
    }
    let device: DeviceCodeResponse = serde_json::from_str(&body)
        .map_err(|error| format!("解析 Kimi device flow 响应失败: {}", error))?;
    if device.device_code.trim().is_empty() || device.user_code.trim().is_empty() {
        return Err("Kimi device flow 响应缺少必要字段".to_string());
    }
    let verification_uri_complete = normalize_text(device.verification_uri_complete.as_deref());
    let verification_uri = normalize_text(device.verification_uri.as_deref())
        .or_else(|| verification_uri_complete.clone())
        .ok_or_else(|| "Kimi device flow 响应缺少 verification_uri".to_string())?;
    let expires_in = device
        .expires_in
        .unwrap_or(MAX_LOGIN_SECONDS)
        .clamp(1, MAX_LOGIN_SECONDS) as u64;
    let interval_seconds = device
        .interval
        .unwrap_or(DEFAULT_INTERVAL_SECONDS)
        .max(1);
    let state = PendingLogin {
        login_id: Uuid::new_v4().to_string(),
        device_code: device.device_code,
        verification_uri: verification_uri.clone(),
        verification_uri_complete: verification_uri_complete.clone(),
        user_code: device.user_code.clone(),
        device_id,
        expires_at: now_ts() + expires_in as i64,
        interval_seconds,
        cancelled: false,
    };
    set_pending(Some(state.clone()))?;
    Ok(KimiOAuthStartResponse {
        login_id: state.login_id,
        verification_uri,
        verification_uri_complete,
        user_code: device.user_code,
        expires_in,
        interval_seconds,
    })
}

async fn poll_once(client: &reqwest::Client, state: &PendingLogin) -> Result<PollResult, String> {
    let mut request = client
        .post(TOKEN_URL)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "application/json")
        .form(&[
            ("grant_type", DEVICE_GRANT_TYPE),
            ("device_code", state.device_code.as_str()),
            ("client_id", CLIENT_ID),
        ]);
    request = apply_device_headers(request, &state.device_id);
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return Ok(PollResult::TransportFailure(format_request_error(
                "轮询 Kimi OAuth token 失败",
                &error,
            )));
        }
    };
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Kimi OAuth token 响应失败: {}", error))?;

    // Official server may return 200 for pending states with error field.
    if let Ok(error) = serde_json::from_str::<TokenErrorResponse>(&body) {
        match error.error.as_deref() {
            Some("authorization_pending") => return Ok(PollResult::Pending),
            Some("slow_down") => return Ok(PollResult::SlowDown),
            Some("access_denied") => return Err("Kimi OAuth 授权已被拒绝".to_string()),
            Some("expired_token") => return Err("Kimi OAuth 验证码已过期".to_string()),
            Some(code) if !status.is_success() || code != "success" => {
                if !status.is_success() || error.error_description.is_some() {
                    return Err(format!(
                        "Kimi OAuth 失败: {}{}",
                        code,
                        error
                            .error_description
                            .as_deref()
                            .map(|value| format!(" ({})", value))
                            .unwrap_or_default()
                    ));
                }
            }
            _ => {}
        }
    }

    if status.is_success() {
        if let Ok(token) = serde_json::from_str::<KimiTokenResponse>(&body) {
            if !token.access_token.trim().is_empty() && !token.refresh_token.trim().is_empty() {
                return Ok(PollResult::Complete(token));
            }
        }
        // Success status but no token yet — treat as pending when body has no access_token.
        if body.contains("authorization_pending") {
            return Ok(PollResult::Pending);
        }
        return Err(format!(
            "Kimi OAuth token 响应无效: {}",
            body.chars().take(200).collect::<String>()
        ));
    }

    let error: TokenErrorResponse = serde_json::from_str(&body).unwrap_or(TokenErrorResponse {
        error: None,
        error_description: None,
    });
    match error.error.as_deref() {
        Some("authorization_pending") => Ok(PollResult::Pending),
        Some("slow_down") => Ok(PollResult::SlowDown),
        Some("access_denied") => Err("Kimi OAuth 授权已被拒绝".to_string()),
        Some("expired_token") => Err("Kimi OAuth 验证码已过期".to_string()),
        Some(code) => Err(format!(
            "Kimi OAuth 失败: {}{}",
            code,
            error
                .error_description
                .as_deref()
                .map(|value| format!(" ({})", value))
                .unwrap_or_default()
        )),
        None => Err(format!("Kimi OAuth token 返回 {}", status.as_u16())),
    }
}

pub async fn complete_login(
    login_id: &str,
) -> Result<(KimiTokenResponse, String, i64, i64), String> {
    let client = http_client()?;
    let mut interval = pending_for(login_id)?.interval_seconds;
    let mut consecutive_transport_errors = 0_u8;
    loop {
        let state = pending_for(login_id)?;
        match poll_once(&client, &state).await? {
            PollResult::Pending => {
                consecutive_transport_errors = 0;
            }
            PollResult::SlowDown => {
                consecutive_transport_errors = 0;
                interval = interval.saturating_add(5);
            }
            PollResult::TransportFailure(error) => {
                consecutive_transport_errors = consecutive_transport_errors.saturating_add(1);
                crate::modules::logger::log_warn(&format!(
                    "[Kimi OAuth] token 轮询传输失败，第 {}/{} 次: {}",
                    consecutive_transport_errors, MAX_CONSECUTIVE_POLL_TRANSPORT_ERRORS, error
                ));
                if consecutive_transport_errors >= MAX_CONSECUTIVE_POLL_TRANSPORT_ERRORS {
                    return Err(error);
                }
            }
            PollResult::Complete(token) => {
                pending_for(login_id)?;
                let expires_in = if token.expires_in > 0 {
                    token.expires_in
                } else {
                    3600
                };
                let expires_at = now_ts() + expires_in;
                let device_id = state.device_id;
                set_pending(None)?;
                return Ok((token, device_id, expires_at, expires_in));
            }
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    hydrate_pending();
    let should_cancel = {
        let guard = PENDING_LOGIN
            .lock()
            .map_err(|_| "获取 Kimi OAuth 状态锁失败".to_string())?;
        match (
            guard.as_ref(),
            login_id.map(str::trim).filter(|id| !id.is_empty()),
        ) {
            (Some(state), Some(id)) => state.login_id == id,
            (Some(_), None) => true,
            _ => false,
        }
    };
    if should_cancel {
        set_pending(None)?;
    }
    Ok(())
}

pub fn needs_refresh(expires_at: Option<i64>, expires_in: Option<i64>) -> bool {
    let Some(expires_at) = expires_at else {
        return false;
    };
    let lead = expires_in
        .filter(|value| *value > 0)
        .map(|value| (value / 2).max(REFRESH_LEAD_SECONDS))
        .unwrap_or(REFRESH_LEAD_SECONDS);
    now_ts() + lead >= expires_at
}

pub async fn refresh_token(
    refresh_token: &str,
    device_id: Option<&str>,
) -> Result<(KimiTokenResponse, i64, i64), String> {
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err("Kimi refresh_token 为空，请重新授权".to_string());
    }
    let home = default_kimi_home()?;
    let device_id = match normalize_text(device_id) {
        Some(value) => value,
        None => ensure_device_id(&home)?,
    };
    let client = http_client()?;
    let mut request = client
        .post(TOKEN_URL)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ]);
    request = apply_device_headers(request, &device_id);
    let response = request
        .send()
        .await
        .map_err(|error| format_request_error("刷新 Kimi token 失败", &error))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Kimi token 刷新响应失败: {}", error))?;
    if !status.is_success() {
        let parsed: TokenErrorResponse =
            serde_json::from_str(&body).unwrap_or(TokenErrorResponse {
                error: None,
                error_description: None,
            });
        return Err(format!(
            "刷新 Kimi token 失败: {}{}",
            parsed.error.unwrap_or_else(|| status.as_u16().to_string()),
            parsed
                .error_description
                .map(|value| format!(" ({})", value))
                .unwrap_or_default()
        ));
    }
    let token: KimiTokenResponse = serde_json::from_str(&body)
        .map_err(|error| format!("解析 Kimi token 刷新响应失败: {}", error))?;
    if token.access_token.trim().is_empty() || token.refresh_token.trim().is_empty() {
        return Err("刷新 Kimi token 未返回完整凭据".to_string());
    }
    let expires_in = if token.expires_in > 0 {
        token.expires_in
    } else {
        3600
    };
    Ok((token, now_ts() + expires_in, expires_in))
}

#[cfg(test)]
mod tests {
    use super::needs_refresh;

    #[test]
    fn refresh_lead_uses_half_lifetime() {
        let now = chrono::Utc::now().timestamp();
        assert!(needs_refresh(Some(now + 100), Some(3600)));
        assert!(!needs_refresh(Some(now + 2000), Some(3600)));
    }
}
