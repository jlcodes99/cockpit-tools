use crate::models::github_copilot::{
    GitHubCopilotOAuthCompletePayload, GitHubCopilotOAuthStartResponse,
};
use crate::modules::logger;
use base64::Engine;
use rand::RngCore;
use reqwest::{
    header::{ACCEPT, AUTHORIZATION, USER_AGENT},
    StatusCode,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Write,
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const GITHUB_AUTHORIZATION_ENDPOINT: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_ENDPOINT: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_ENDPOINT: &str = "https://api.github.com/user";
const GITHUB_USER_EMAILS_ENDPOINT: &str = "https://api.github.com/user/emails";
const GITHUB_COPILOT_TOKEN_ENDPOINT: &str = "https://api.github.com/copilot_internal/v2/token";
const GITHUB_COPILOT_USER_INFO_ENDPOINT: &str = "https://api.github.com/copilot_internal/user";
const GITHUB_OAUTH_CLIENT_ID: &str = "01ab8ac9400c4e429b23";
const GITHUB_OAUTH_CLIENT_SECRET: &str = "2af589bb2ffd03a29cc0df83f767e3f6693f14cd";
const GITHUB_OAUTH_REDIRECT_URI: &str = "https://vscode.dev/redirect";
const GITHUB_OAUTH_SCOPE: &str = "read:user repo user:email workflow";
const GITHUB_OAUTH_GET_STARTED_WITH: &str = "copilot-vscode";
const GITHUB_API_VERSION: &str = "2025-04-01";
const OAUTH_TIMEOUT_SECONDS: i64 = 300;
const OAUTH_POLL_INTERVAL_MS: u64 = 200;
const APP_USER_AGENT: &str = "antigravity-cockpit-tools";

#[derive(Debug, Clone)]
struct PendingOAuthLogin {
    login_id: String,
    auth_url: String,
    code_verifier: String,
    port: u16,
    code: Option<String>,
    error: Option<String>,
    expires_at: i64,
}

lazy_static::lazy_static! {
    static ref PENDING_OAUTH_LOGIN: Arc<Mutex<Option<PendingOAuthLogin>>> = Arc::new(Mutex::new(None));
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
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
    token_based_billing: Option<bool>,
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
    pub token_based_billing: Option<bool>,
    pub quota_snapshots: Option<serde_json::Value>,
    pub quota_reset_date: Option<String>,
    pub limited_user_quotas: Option<serde_json::Value>,
    pub limited_user_reset_date: Option<i64>,
}

fn generate_base64_token(byte_len: usize) -> String {
    let mut bytes = vec![0u8; byte_len];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_hex_token(byte_len: usize) -> String {
    let mut bytes = vec![0u8; byte_len];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn generate_login_id() -> String {
    let mut bytes = vec![0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_challenge(code_verifier: &str) -> String {
    let digest = Sha256::digest(code_verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn github_access_token_auth_header(github_access_token: &str) -> String {
    format!("Bearer {}", github_access_token)
}

fn github_copilot_auth_header(github_access_token: &str) -> String {
    format!("token {}", github_access_token)
}

fn is_github_email_permission_error(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
    )
}

fn get_pending_login() -> Option<PendingOAuthLogin> {
    PENDING_OAUTH_LOGIN
        .lock()
        .ok()
        .and_then(|state| state.as_ref().cloned())
}

fn set_pending_login(state: Option<PendingOAuthLogin>) {
    if let Ok(mut guard) = PENDING_OAUTH_LOGIN.lock() {
        *guard = state;
    }
}

fn clear_pending_login_if_matches(login_id: &str) {
    if let Ok(mut guard) = PENDING_OAUTH_LOGIN.lock() {
        if guard.as_ref().map(|state| state.login_id.as_str()) == Some(login_id) {
            *guard = None;
        }
    }
}

fn get_pending_login_for(login_id: &str) -> Result<PendingOAuthLogin, String> {
    let state = get_pending_login().ok_or_else(|| "登录流程已取消，请重新发起授权".to_string())?;
    if state.login_id != login_id {
        return Err("登录会话已变更，请刷新后重试".to_string());
    }
    Ok(state)
}

fn to_start_response(state: &PendingOAuthLogin) -> GitHubCopilotOAuthStartResponse {
    let expires_in = (state.expires_at - now_timestamp()).max(0) as u64;
    GitHubCopilotOAuthStartResponse {
        login_id: state.login_id.clone(),
        user_code: String::new(),
        verification_uri: state.auth_url.clone(),
        verification_uri_complete: Some(state.auth_url.clone()),
        expires_in,
        interval_seconds: 1,
    }
}

fn build_authorization_url(code_challenge: &str, callback_url: &str) -> String {
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params.append_pair("client_id", GITHUB_OAUTH_CLIENT_ID);
    params.append_pair("redirect_uri", GITHUB_OAUTH_REDIRECT_URI);
    params.append_pair("scope", GITHUB_OAUTH_SCOPE);
    params.append_pair("state", callback_url);
    params.append_pair("code_challenge", code_challenge);
    params.append_pair("code_challenge_method", "S256");
    params.append_pair("get_started_with", GITHUB_OAUTH_GET_STARTED_WITH);
    params.append_pair("prompt", "select_account");
    format!("{}?{}", GITHUB_AUTHORIZATION_ENDPOINT, params.finish())
}

fn find_available_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("查找本地回调端口失败: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取本地回调端口失败: {}", e))?
        .port();
    Ok(port)
}

fn notify_cancel(port: u16) {
    if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
        let _ = stream.write_all(b"GET /cancel HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    }
}

fn parse_query_params(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            (
                percent_decode_query_component(key),
                percent_decode_query_component(value),
            )
        })
        .collect()
}

fn percent_decode_query_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }

        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn set_callback_result_if_matches(login_id: &str, result: Result<String, String>) {
    if let Ok(mut guard) = PENDING_OAUTH_LOGIN.lock() {
        if let Some(state) = guard.as_mut() {
            if state.login_id == login_id {
                match result {
                    Ok(code) => state.code = Some(code),
                    Err(err) => state.error = Some(err),
                }
            }
        }
    }
}

fn callback_success_html() -> &'static str {
    r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>授权成功</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: linear-gradient(135deg, #0969da 0%, #1f883d 100%); }
        .container { text-align: center; color: white; }
        h1 { font-size: 2.4rem; margin-bottom: 1rem; }
        p { font-size: 1.1rem; opacity: 0.92; }
    </style>
</head>
<body>
    <div class="container">
        <h1>授权成功</h1>
        <p>GitHub Copilot 授权已完成，您可以关闭此窗口并返回应用。</p>
    </div>
</body>
</html>"#
}

async fn start_callback_server(
    port: u16,
    expected_login_id: String,
    expected_callback_url: String,
    expected_nonce: String,
) -> Result<(), String> {
    use tiny_http::{Header, Response, Server};

    let server = Server::http(format!("127.0.0.1:{}", port))
        .map_err(|e| format!("启动 GitHub Copilot OAuth 回调服务失败: {}", e))?;
    let started = Instant::now();
    let timeout = Duration::from_secs(OAUTH_TIMEOUT_SECONDS as u64);

    logger::log_info(&format!(
        "GitHub Copilot OAuth 回调服务已启动: login_id={}, port={}",
        expected_login_id, port
    ));

    loop {
        let should_stop = {
            let state = PENDING_OAUTH_LOGIN.lock().unwrap();
            match state.as_ref() {
                Some(current) => current.login_id != expected_login_id,
                None => true,
            }
        };
        if should_stop {
            break;
        }

        if started.elapsed() > timeout {
            set_callback_result_if_matches(
                &expected_login_id,
                Err("等待 GitHub 授权超时，请重新发起授权".to_string()),
            );
            break;
        }

        if let Ok(Some(request)) = server.try_recv() {
            let url = request.url().to_string();
            if url.starts_with("/callback") {
                let query = url.split_once('?').map(|(_, query)| query).unwrap_or("");
                let params = parse_query_params(query);
                let code = params.get("code").cloned().unwrap_or_default();
                let state = params.get("state").cloned().unwrap_or_default();
                let nonce = params.get("nonce").cloned().unwrap_or_default();
                let error = params.get("error").cloned();
                let error_description = params.get("error_description").cloned();

                if let Some(error) = error {
                    let message = error_description.unwrap_or(error);
                    set_callback_result_if_matches(
                        &expected_login_id,
                        Err(format!("GitHub 授权失败: {}", message)),
                    );
                    let _ = request.respond(
                        Response::from_string("GitHub authorization failed").with_status_code(400),
                    );
                    break;
                }

                if state != expected_callback_url {
                    logger::log_warn(&format!(
                        "GitHub Copilot OAuth 回调 state 不匹配: login_id={}",
                        expected_login_id
                    ));
                    let _ = request
                        .respond(Response::from_string("State mismatch").with_status_code(400));
                    continue;
                }

                if nonce != expected_nonce {
                    logger::log_warn(&format!(
                        "GitHub Copilot OAuth 回调 nonce 不匹配: login_id={}",
                        expected_login_id
                    ));
                    let _ = request
                        .respond(Response::from_string("Nonce mismatch").with_status_code(400));
                    continue;
                }

                if code.is_empty() {
                    let _ = request
                        .respond(Response::from_string("Missing code").with_status_code(400));
                    continue;
                }

                let response = Response::from_string(callback_success_html()).with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                        .unwrap(),
                );
                let _ = request.respond(response);
                set_callback_result_if_matches(&expected_login_id, Ok(code));
                break;
            } else if url.starts_with("/cancel") {
                let _ =
                    request.respond(Response::from_string("Login cancelled").with_status_code(200));
                break;
            } else {
                let _ = request.respond(Response::from_string("Not Found").with_status_code(404));
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    logger::log_info(&format!(
        "GitHub Copilot OAuth 回调服务已停止: login_id={}",
        expected_login_id
    ));
    Ok(())
}

pub async fn start_login() -> Result<GitHubCopilotOAuthStartResponse, String> {
    if let Some(existing) = get_pending_login() {
        logger::log_info(&format!(
            "GitHub Copilot OAuth 发现进行中的登录会话，将创建新会话并覆盖旧会话: login_id={}",
            existing.login_id
        ));
        notify_cancel(existing.port);
    }

    let login_id = generate_login_id();
    let port = find_available_port()?;
    let nonce = generate_base64_token(16);
    let callback_url = format!(
        "http://127.0.0.1:{}/callback?nonce={}",
        port,
        urlencoding::encode(&nonce)
    );
    let code_verifier = generate_hex_token(32);
    let code_challenge = generate_code_challenge(&code_verifier);
    let auth_url = build_authorization_url(&code_challenge, &callback_url);

    let login = PendingOAuthLogin {
        login_id: login_id.clone(),
        auth_url,
        code_verifier,
        port,
        code: None,
        error: None,
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS,
    };

    set_pending_login(Some(login.clone()));
    tokio::spawn(start_callback_server(
        port,
        login_id.clone(),
        callback_url,
        nonce,
    ));

    logger::log_info(&format!(
        "GitHub Copilot OAuth 登录会话已创建: login_id={}, port={}, redirect_uri={}",
        login.login_id, login.port, GITHUB_OAUTH_REDIRECT_URI
    ));

    Ok(to_start_response(&login))
}

async fn exchange_code_for_token(
    client: &reqwest::Client,
    code: &str,
    code_verifier: &str,
) -> Result<OAuthTokenResponse, String> {
    let response = client
        .post(GITHUB_TOKEN_ENDPOINT)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(ACCEPT, "application/json")
        .form(&[
            ("client_id", GITHUB_OAUTH_CLIENT_ID),
            ("client_secret", GITHUB_OAUTH_CLIENT_SECRET),
            ("code", code),
            ("redirect_uri", GITHUB_OAUTH_REDIRECT_URI),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("请求 GitHub OAuth access token 失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<no-body>".to_string());
        return Err(format!(
            "请求 GitHub OAuth access token 失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    response
        .json::<OAuthTokenResponse>()
        .await
        .map_err(|e| format!("解析 GitHub OAuth access token 响应失败: {}", e))
}

async fn fetch_github_user(
    client: &reqwest::Client,
    github_access_token: &str,
) -> Result<GitHubUser, String> {
    let response = client
        .get(GITHUB_USER_ENDPOINT)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(ACCEPT, "application/vnd.github+json")
        .header(
            AUTHORIZATION,
            github_access_token_auth_header(github_access_token),
        )
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
        .header(
            AUTHORIZATION,
            github_access_token_auth_header(github_access_token),
        )
        .send()
        .await
        .map_err(|e| format!("请求 GitHub 邮箱列表失败: {}", e))?;

    if is_github_email_permission_error(response.status()) {
        logger::log_warn(&format!(
            "GitHub 邮箱列表无权限，将继续保存无邮箱账号: status={}",
            response.status()
        ));
        return Ok(None);
    }

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
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .header(
            AUTHORIZATION,
            github_copilot_auth_header(github_access_token),
        )
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
        token_based_billing: user_info.as_ref().and_then(|info| info.token_based_billing),
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
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .header(
            AUTHORIZATION,
            github_copilot_auth_header(github_access_token),
        )
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
    logger::log_info(&format!(
        "GitHub Copilot OAuth 开始等待浏览器回调: login_id={}",
        login_id
    ));

    let client = reqwest::Client::new();
    let (code, code_verifier) = loop {
        let pending = get_pending_login_for(login_id)?;
        if now_timestamp() >= pending.expires_at {
            clear_pending_login_if_matches(login_id);
            return Err("等待 GitHub 授权超时，请重新发起授权".to_string());
        }

        if let Some(error) = pending.error.clone() {
            clear_pending_login_if_matches(login_id);
            return Err(error);
        }

        if let Some(code) = pending.code.clone() {
            break (code, pending.code_verifier.clone());
        }

        tokio::time::sleep(Duration::from_millis(OAUTH_POLL_INTERVAL_MS)).await;
    };

    let token_result = exchange_code_for_token(&client, &code, &code_verifier).await?;
    if let Some(error) = token_result.error.clone() {
        let detail = token_result
            .error_description
            .unwrap_or_else(|| "未知错误".to_string());
        clear_pending_login_if_matches(login_id);
        return Err(format!("GitHub 授权失败: {} ({})", error, detail));
    }

    let github_access_token = token_result.access_token.clone().ok_or_else(|| {
        clear_pending_login_if_matches(login_id);
        "GitHub OAuth access token 缺失".to_string()
    })?;

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
        login_id, github_user.login
    ));

    Ok(GitHubCopilotOAuthCompletePayload {
        github_login: github_user.login,
        github_id: github_user.id,
        github_name: github_user.name,
        github_email,
        github_access_token,
        github_token_type: token_result.token_type,
        github_scope: token_result.scope,
        copilot_token: copilot.token,
        copilot_plan: copilot.plan,
        copilot_chat_enabled: copilot.chat_enabled,
        copilot_expires_at: copilot.expires_at,
        copilot_refresh_in: copilot.refresh_in,
        copilot_token_based_billing: copilot.token_based_billing,
        copilot_quota_snapshots: copilot.quota_snapshots,
        copilot_quota_reset_date: copilot.quota_reset_date,
        copilot_limited_user_quotas: copilot.limited_user_quotas,
        copilot_limited_user_reset_date: copilot.limited_user_reset_date,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_auth_url_matches_vscode_copilot_login_flow() {
        let callback_url = "http://127.0.0.1:61280/callback?nonce=HNvQeHsrB2DLlrPuR9eVLQ%3D%3D";
        let auth_url = build_authorization_url("challenge-value", callback_url);

        assert!(auth_url.starts_with("https://github.com/login/oauth/authorize?"));
        assert!(auth_url.contains("client_id=01ab8ac9400c4e429b23"));
        assert!(auth_url.contains("redirect_uri=https%3A%2F%2Fvscode.dev%2Fredirect"));
        assert!(auth_url.contains("scope=read%3Auser+repo+user%3Aemail+workflow"));
        assert!(auth_url.contains("state=http%3A%2F%2F127.0.0.1%3A61280%2Fcallback%3Fnonce%3DHNvQeHsrB2DLlrPuR9eVLQ%253D%253D"));
        assert!(auth_url.contains("code_challenge=challenge-value"));
        assert!(auth_url.contains("code_challenge_method=S256"));
        assert!(auth_url.contains("get_started_with=copilot-vscode"));
        assert!(auth_url.contains("prompt=select_account"));
    }

    #[test]
    fn code_challenge_uses_s256_base64url_without_padding() {
        assert_eq!(
            generate_code_challenge("test"),
            "n4bQgYhMfWWaL-qgxVrQFaO_TxsrC4Is0V1sFbDwCgg"
        );
    }

    #[test]
    fn callback_query_parser_preserves_plus_in_nonce() {
        let params = parse_query_params(
            "nonce=IMLVvLVydtWkp+wqTOhBw%3D%3D&code=d5829a1c9dfcdcb845b9&state=http%3A%2F%2F127.0.0.1%3A61915%2Fcallback%3Fnonce%3DIMLVvLVydtWkp%2BwqTOhBw%253D%253D",
        );

        assert_eq!(
            params.get("nonce").map(String::as_str),
            Some("IMLVvLVydtWkp+wqTOhBw==")
        );
        assert_eq!(
            params.get("code").map(String::as_str),
            Some("d5829a1c9dfcdcb845b9")
        );
    }

    #[test]
    fn generated_nonce_is_url_safe() {
        let nonce = generate_base64_token(16);

        assert!(!nonce.contains('+'));
        assert!(!nonce.contains('/'));
        assert!(!nonce.contains('='));
    }

    #[test]
    fn github_internal_api_uses_token_auth_scheme() {
        let header = github_copilot_auth_header("gho_example");
        assert_eq!(header, "token gho_example");
        assert!(!header.starts_with("Bearer "));
    }

    #[test]
    fn github_email_permission_errors_do_not_abort_login() {
        assert!(is_github_email_permission_error(StatusCode::UNAUTHORIZED));
        assert!(is_github_email_permission_error(StatusCode::FORBIDDEN));
        assert!(is_github_email_permission_error(StatusCode::NOT_FOUND));
        assert!(!is_github_email_permission_error(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
    }
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    let current = get_pending_login();
    match (current, login_id) {
        (Some(state), Some(id)) if state.login_id != id => {
            Err("登录会话不匹配，取消失败".to_string())
        }
        (Some(state), _) => {
            set_pending_login(None);
            notify_cancel(state.port);
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
        copilot_token_based_billing: copilot.token_based_billing,
        copilot_quota_snapshots: copilot.quota_snapshots,
        copilot_quota_reset_date: copilot.quota_reset_date,
        copilot_limited_user_quotas: copilot.limited_user_quotas,
        copilot_limited_user_reset_date: copilot.limited_user_reset_date,
    })
}
