use base64::Engine;
use rand::Rng;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tiny_http::{Header, Response, Server, StatusCode};
use url::Url;

use crate::models::gemini::{GeminiOAuthCompletePayload, GeminiOAuthStartResponse};
use crate::modules::logger;

const GEMINI_OAUTH_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GEMINI_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const GEMINI_OAUTH_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const GEMINI_OAUTH_CLIENT_SECRET: &str = "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";
const OAUTH_TIMEOUT_SECONDS: i64 = 300;
const OAUTH_CALLBACK_PATH: &str = "/oauth2callback";
const OAUTH_POLL_INTERVAL_SECONDS: u64 = 1;
const HTTP_REDIRECT_STATUS: u16 = 301;
const OAUTH_STATE_FILE: &str = "gemini_oauth_pending.json";
const SIGN_IN_SUCCESS_URL: &str =
    "https://developers.google.com/gemini-code-assist/auth_success_gemini";
const SIGN_IN_FAILURE_URL: &str =
    "https://developers.google.com/gemini-code-assist/auth_failure_gemini";

const OAUTH_SCOPES: [&str; 3] = [
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOAuthState {
    login_id: String,
    callback_port: u16,
    callback_url: String,
    auth_url: String,
    state_token: String,
    expires_at: i64,
    cancelled: bool,
    manual_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfoResponse {
    id: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

lazy_static::lazy_static! {
    static ref PENDING_OAUTH_STATE: Arc<Mutex<Option<PendingOAuthState>>> = Arc::new(Mutex::new(None));
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn now_timestamp_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..24).map(|_| rng.gen::<u8>()).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
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

pub fn gemini_oauth_client_id() -> &'static str {
    GEMINI_OAUTH_CLIENT_ID
}

pub fn gemini_oauth_client_secret() -> &'static str {
    GEMINI_OAUTH_CLIENT_SECRET
}

fn find_available_callback_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("分配 Gemini OAuth 回调端口失败: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取 Gemini OAuth 回调端口失败: {}", e))?
        .port();
    drop(listener);
    Ok(port)
}

fn build_auth_url(callback_url: &str, state_token: &str) -> Result<String, String> {
    let mut url = Url::parse(GEMINI_OAUTH_AUTH_URL)
        .map_err(|e| format!("构建 Gemini OAuth URL 失败: {}", e))?;
    let scope = OAUTH_SCOPES.join(" ");

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", gemini_oauth_client_id())
        .append_pair("redirect_uri", callback_url)
        .append_pair("access_type", "offline")
        .append_pair("scope", &scope)
        .append_pair("state", state_token);

    Ok(url.to_string())
}

fn get_pending_login() -> Option<PendingOAuthState> {
    hydrate_pending_login_if_missing();
    PENDING_OAUTH_STATE
        .lock()
        .ok()
        .and_then(|state| state.as_ref().cloned())
}

fn set_pending_login(state: Option<PendingOAuthState>) {
    if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
        *guard = state.clone();
    }
    let result = match state.as_ref() {
        Some(value) => crate::modules::oauth_pending_state::save(OAUTH_STATE_FILE, value),
        None => crate::modules::oauth_pending_state::clear(OAUTH_STATE_FILE),
    };
    if let Err(err) = result {
        logger::log_warn(&format!(
            "[Gemini OAuth] 持久化登录状态失败，已忽略: {}",
            err
        ));
    }
}

fn load_pending_login_from_disk() -> Option<PendingOAuthState> {
    match crate::modules::oauth_pending_state::load::<PendingOAuthState>(OAUTH_STATE_FILE) {
        Ok(Some(state)) => {
            if state.cancelled || now_timestamp() > state.expires_at {
                let _ = crate::modules::oauth_pending_state::clear(OAUTH_STATE_FILE);
                None
            } else {
                Some(state)
            }
        }
        Ok(None) => None,
        Err(err) => {
            logger::log_warn(&format!(
                "[Gemini OAuth] 读取持久化登录状态失败，已忽略: {}",
                err
            ));
            let _ = crate::modules::oauth_pending_state::clear(OAUTH_STATE_FILE);
            None
        }
    }
}

fn hydrate_pending_login_if_missing() {
    if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
        if guard.is_none() {
            *guard = load_pending_login_from_disk();
        }
    }
}

fn get_pending_login_for(login_id: &str) -> Result<PendingOAuthState, String> {
    hydrate_pending_login_if_missing();
    let state =
        get_pending_login().ok_or_else(|| "Gemini OAuth 登录流程不存在，请重新发起".to_string())?;
    if state.login_id != login_id {
        return Err("Gemini OAuth 登录会话已变更，请重新发起".to_string());
    }
    if state.cancelled {
        return Err("Gemini OAuth 登录已取消".to_string());
    }
    if now_timestamp() > state.expires_at {
        return Err("Gemini OAuth 登录已超时，请重试".to_string());
    }
    Ok(state)
}

fn clear_pending_login_if_matches(login_id: &str) {
    let should_clear = if let Ok(guard) = PENDING_OAUTH_STATE.lock() {
        guard
            .as_ref()
            .map(|state| state.login_id.as_str())
            .map(|id| id == login_id)
            .unwrap_or(false)
    } else {
        false
    };
    if should_clear {
        set_pending_login(None);
    }
}

fn parse_query_pairs(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn parse_callback_url(raw_callback_url: &str, callback_port: u16) -> Result<Url, String> {
    let trimmed = raw_callback_url.trim();
    if trimmed.is_empty() {
        return Err("回调链接不能为空".to_string());
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Url::parse(trimmed).map_err(|e| format!("回调链接格式无效: {}", e));
    }

    if trimmed.starts_with('/') {
        return Url::parse(format!("http://127.0.0.1:{}{}", callback_port, trimmed).as_str())
            .map_err(|e| format!("回调链接格式无效: {}", e));
    }

    Url::parse(
        format!(
            "http://127.0.0.1:{}{}?{}",
            callback_port,
            OAUTH_CALLBACK_PATH,
            trimmed.trim_start_matches('?')
        )
        .as_str(),
    )
    .map_err(|e| format!("回调链接格式无效: {}", e))
}

fn parse_jwt_claim_string(token: &str, key: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let payload_b64 = parts[1].replace('-', "+").replace('_', "/");
    let padded = match payload_b64.len() % 4 {
        2 => format!("{}==", payload_b64),
        3 => format!("{}=", payload_b64),
        _ => payload_b64,
    };

    let payload = base64::engine::general_purpose::STANDARD
        .decode(padded)
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    normalize_non_empty(value.get(key).and_then(|item| item.as_str()))
}

fn respond_text(request: tiny_http::Request, status: StatusCode, body: &str) {
    let mut response = Response::from_string(body.to_string()).with_status_code(status);
    if let Ok(header) = Header::from_bytes(
        "Content-Type".as_bytes(),
        "text/plain; charset=utf-8".as_bytes(),
    ) {
        response.add_header(header);
    }
    let _ = request.respond(response);
}

fn respond_redirect(request: tiny_http::Request, location: &str) {
    let mut response =
        Response::from_string(String::new()).with_status_code(StatusCode(HTTP_REDIRECT_STATUS));
    if let Ok(header) = Header::from_bytes("Location".as_bytes(), location.as_bytes()) {
        response.add_header(header);
    }
    let _ = request.respond(response);
}

fn wait_for_oauth_code_blocking(
    login_id: String,
    callback_port: u16,
    expected_state: String,
    expires_at: i64,
) -> Result<String, String> {
    let server = Server::http(format!("127.0.0.1:{}", callback_port))
        .map_err(|e| format!("启动 Gemini OAuth 回调监听失败: {}", e))?;

    loop {
        if now_timestamp() > expires_at {
            return Err("Gemini OAuth 登录等待超时，请重试".to_string());
        }

        if let Ok(guard) = PENDING_OAUTH_STATE.lock() {
            match guard.as_ref() {
                Some(state) if state.login_id == login_id && !state.cancelled => {
                    if let Some(code) = state
                        .manual_code
                        .as_deref()
                        .and_then(|value| normalize_non_empty(Some(value)))
                    {
                        return Ok(code);
                    }
                }
                Some(_) => return Err("Gemini OAuth 登录会话已变更，请重试".to_string()),
                None => return Err("Gemini OAuth 登录已取消".to_string()),
            }
        }

        let request = match server.recv_timeout(Duration::from_secs(OAUTH_POLL_INTERVAL_SECONDS)) {
            Ok(Some(req)) => req,
            Ok(None) => continue,
            Err(err) => return Err(format!("Gemini OAuth 回调监听失败: {}", err)),
        };

        let full_url = format!("http://127.0.0.1{}", request.url());
        let parsed = match Url::parse(&full_url) {
            Ok(url) => url,
            Err(_) => {
                respond_redirect(request, SIGN_IN_FAILURE_URL);
                continue;
            }
        };

        if parsed.path() != OAUTH_CALLBACK_PATH {
            respond_redirect(request, SIGN_IN_FAILURE_URL);
            continue;
        }

        let params = parse_query_pairs(&parsed);

        if let Some(error) = params.get("error") {
            let desc = params
                .get("error_description")
                .cloned()
                .unwrap_or_else(|| "No details".to_string());
            respond_redirect(request, SIGN_IN_FAILURE_URL);
            return Err(format!("Google OAuth 错误: {} ({})", error, desc));
        }

        if params.get("state").map(String::as_str) != Some(expected_state.as_str()) {
            respond_text(request, StatusCode(400), "State mismatch.");
            return Err("Gemini OAuth state 校验失败，可能存在 CSRF 风险".to_string());
        }

        let code = match params.get("code") {
            Some(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => {
                respond_redirect(request, SIGN_IN_FAILURE_URL);
                return Err("Google OAuth 回调缺少 code 参数".to_string());
            }
        };

        respond_redirect(request, SIGN_IN_SUCCESS_URL);
        return Ok(code);
    }
}

async fn exchange_code_for_tokens(
    code: &str,
    redirect_uri: &str,
) -> Result<GoogleTokenResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client
        .post(GEMINI_OAUTH_TOKEN_URL)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[
            ("code", code),
            ("client_id", gemini_oauth_client_id()),
            ("client_secret", gemini_oauth_client_secret()),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("请求 Google OAuth token 失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<empty-body>".to_string());
        return Err(format!(
            "Google OAuth token 交换失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    let payload = response
        .json::<GoogleTokenResponse>()
        .await
        .map_err(|e| format!("解析 Google OAuth token 响应失败: {}", e))?;

    if payload.access_token.is_none() {
        return Err(format!(
            "Google OAuth token 响应缺少 access_token: error={:?}, desc={:?}",
            payload.error, payload.error_description
        ));
    }

    Ok(payload)
}

async fn fetch_google_userinfo(access_token: &str) -> Option<GoogleUserInfoResponse> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let response = client
        .get(GOOGLE_USERINFO_URL)
        .header(AUTHORIZATION, format!("Bearer {}", access_token))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<GoogleUserInfoResponse>().await.ok()
}

pub async fn start_login() -> Result<GeminiOAuthStartResponse, String> {
    hydrate_pending_login_if_missing();
    if let Some(existing) = get_pending_login() {
        if existing.expires_at > now_timestamp() && !existing.cancelled {
            logger::log_info(&format!(
                "[Gemini OAuth] 复用登录会话: login_id={}",
                existing.login_id
            ));
            return Ok(GeminiOAuthStartResponse {
                login_id: existing.login_id,
                verification_uri: existing.auth_url,
                expires_in: (existing.expires_at - now_timestamp()).max(0) as u64,
                interval_seconds: OAUTH_POLL_INTERVAL_SECONDS,
                callback_url: Some(existing.callback_url),
            });
        }
    }
    set_pending_login(None);

    let callback_port = find_available_callback_port()?;
    let callback_url = format!("http://127.0.0.1:{}{}", callback_port, OAUTH_CALLBACK_PATH);
    let state_token = generate_token();
    let auth_url = build_auth_url(&callback_url, &state_token)?;
    let login_id = generate_token();

    let pending = PendingOAuthState {
        login_id: login_id.clone(),
        callback_port,
        callback_url: callback_url.clone(),
        auth_url: auth_url.clone(),
        state_token,
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS,
        cancelled: false,
        manual_code: None,
    };

    set_pending_login(Some(pending));

    logger::log_info(&format!(
        "[Gemini OAuth] 登录会话已创建: login_id={}, callback_url={}",
        login_id, callback_url
    ));

    Ok(GeminiOAuthStartResponse {
        login_id,
        verification_uri: auth_url,
        expires_in: OAUTH_TIMEOUT_SECONDS as u64,
        interval_seconds: OAUTH_POLL_INTERVAL_SECONDS,
        callback_url: Some(callback_url),
    })
}

pub async fn complete_login(login_id: &str) -> Result<GeminiOAuthCompletePayload, String> {
    let state = get_pending_login_for(login_id)?;

    let code = tokio::task::spawn_blocking({
        let login_id = login_id.to_string();
        let state_token = state.state_token.clone();
        let callback_port = state.callback_port;
        let expires_at = state.expires_at;
        move || wait_for_oauth_code_blocking(login_id, callback_port, state_token, expires_at)
    })
    .await
    .map_err(|e| format!("等待 Gemini OAuth 回调任务失败: {}", e))??;

    let token_payload = exchange_code_for_tokens(&code, &state.callback_url).await?;
    let access_token = token_payload
        .access_token
        .clone()
        .ok_or_else(|| "Google OAuth token 响应缺少 access_token".to_string())?;

    let user_info = fetch_google_userinfo(&access_token).await;

    let email = normalize_non_empty(user_info.as_ref().and_then(|info| info.email.as_deref()))
        .or_else(|| {
            token_payload
                .id_token
                .as_deref()
                .and_then(|token| parse_jwt_claim_string(token, "email"))
        })
        .unwrap_or_else(|| "unknown@gmail.com".to_string());

    let auth_id = normalize_non_empty(user_info.as_ref().and_then(|info| info.id.as_deref()))
        .or_else(|| {
            token_payload
                .id_token
                .as_deref()
                .and_then(|token| parse_jwt_claim_string(token, "sub"))
        });

    let name = normalize_non_empty(user_info.as_ref().and_then(|info| info.name.as_deref()))
        .or_else(|| {
            token_payload
                .id_token
                .as_deref()
                .and_then(|token| parse_jwt_claim_string(token, "name"))
        });

    let expiry_date = token_payload
        .expires_in
        .map(|seconds| now_timestamp_ms() + seconds.saturating_mul(1000));

    let mut auth_raw = serde_json::Map::new();
    auth_raw.insert(
        "access_token".to_string(),
        serde_json::Value::String(access_token.clone()),
    );
    if let Some(refresh_token) = token_payload.refresh_token.clone() {
        auth_raw.insert(
            "refresh_token".to_string(),
            serde_json::Value::String(refresh_token),
        );
    }
    if let Some(id_token) = token_payload.id_token.clone() {
        auth_raw.insert("id_token".to_string(), serde_json::Value::String(id_token));
    }
    if let Some(token_type) = token_payload.token_type.clone() {
        auth_raw.insert(
            "token_type".to_string(),
            serde_json::Value::String(token_type),
        );
    }
    if let Some(scope) = token_payload.scope.clone() {
        auth_raw.insert("scope".to_string(), serde_json::Value::String(scope));
    }
    if let Some(expiry_date) = expiry_date {
        auth_raw.insert("expiry_date".to_string(), serde_json::json!(expiry_date));
    }
    auth_raw.insert(
        "email".to_string(),
        serde_json::Value::String(email.clone()),
    );
    if let Some(auth_id) = auth_id.clone() {
        auth_raw.insert("sub".to_string(), serde_json::Value::String(auth_id));
    }

    clear_pending_login_if_matches(login_id);

    Ok(GeminiOAuthCompletePayload {
        email,
        auth_id,
        name,
        access_token,
        refresh_token: token_payload.refresh_token,
        id_token: token_payload.id_token,
        token_type: token_payload.token_type,
        scope: token_payload.scope,
        expiry_date,
        selected_auth_type: Some("oauth-personal".to_string()),
        project_id: None,
        tier_id: None,
        plan_name: None,
        gemini_auth_raw: Some(serde_json::Value::Object(auth_raw)),
        gemini_usage_raw: None,
        status: None,
        status_reason: None,
    })
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    hydrate_pending_login_if_missing();
    if let Some(state) = get_pending_login() {
        if login_id.is_none() || login_id == Some(state.login_id.as_str()) {
            set_pending_login(None);
        }
    }
    Ok(())
}

pub fn submit_callback_url(login_id: &str, callback_url: &str) -> Result<(), String> {
    hydrate_pending_login_if_missing();
    let (state_token, callback_port, expires_at) = {
        let guard = PENDING_OAUTH_STATE
            .lock()
            .map_err(|_| "Gemini OAuth 状态锁不可用".to_string())?;
        let state = guard
            .as_ref()
            .ok_or_else(|| "Gemini OAuth 登录流程不存在，请重新发起".to_string())?;
        if state.login_id != login_id {
            return Err("Gemini OAuth 登录会话已变更，请重新发起".to_string());
        }
        if state.cancelled {
            return Err("Gemini OAuth 登录已取消".to_string());
        }
        (
            state.state_token.clone(),
            state.callback_port,
            state.expires_at,
        )
    };

    if now_timestamp() > expires_at {
        return Err("Gemini OAuth 登录已超时，请重试".to_string());
    }

    let parsed = parse_callback_url(callback_url, callback_port)?;
    if parsed.path() != OAUTH_CALLBACK_PATH {
        return Err(format!("回调链接路径无效，必须为 {}", OAUTH_CALLBACK_PATH));
    }

    let params = parse_query_pairs(&parsed);
    if let Some(error) = params.get("error") {
        let desc = params
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| "No details".to_string());
        return Err(format!("Google OAuth 错误: {} ({})", error, desc));
    }

    if params.get("state").map(String::as_str) != Some(state_token.as_str()) {
        return Err("Gemini OAuth state 校验失败，可能存在 CSRF 风险".to_string());
    }

    let code = params
        .get("code")
        .and_then(|value| normalize_non_empty(Some(value.as_str())))
        .ok_or_else(|| "Google OAuth 回调缺少 code 参数".to_string())?;

    let mut guard = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "Gemini OAuth 状态锁不可用".to_string())?;
    let state = guard
        .as_mut()
        .ok_or_else(|| "Gemini OAuth 登录流程不存在，请重新发起".to_string())?;
    if state.login_id != login_id {
        return Err("Gemini OAuth 登录会话已变更，请重新发起".to_string());
    }
    state.manual_code = Some(code);
    let snapshot = state.clone();
    drop(guard);
    set_pending_login(Some(snapshot));

    logger::log_info(&format!(
        "[Gemini OAuth] 已接收手动回调链接: login_id={}",
        login_id
    ));
    Ok(())
}

pub fn restore_pending_oauth_state() {
    hydrate_pending_login_if_missing();
}
