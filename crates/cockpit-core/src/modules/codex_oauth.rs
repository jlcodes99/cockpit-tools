use crate::models::codex::CodexTokens;
use crate::modules::logger;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use url::Url;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const ORIGINATOR: &str = "Codex Desktop";
const OAUTH_CALLBACK_PORT: u16 = 1455;
const OAUTH_PORT_IN_USE_CODE: &str = "CODEX_OAUTH_PORT_IN_USE";
const OAUTH_STATE_FILE: &str = "codex_oauth_pending.json";
const OAUTH_TIMEOUT_SECONDS: i64 = 300;
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 300;
pub const ID_TOKEN_REFRESH_LEAD_SECONDS: i64 = 10 * 60;
const DEVICE_USER_CODE_ENDPOINT: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_ENDPOINT: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const DEVICE_EXCHANGE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const DEVICE_TIMEOUT_SECONDS: u64 = 15 * 60;
const DEVICE_DEFAULT_POLL_SECONDS: u64 = 5;
const DEVICE_REQUEST_TIMEOUT_SECONDS: u64 = 25;

pub fn get_callback_port() -> u16 {
    OAUTH_CALLBACK_PORT
}

fn apply_codex_auth_identity_headers(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    request
        .header(
            "User-Agent",
            format!(
                "{}/{} ({}; {})",
                ORIGINATOR,
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        )
        .header("originator", ORIGINATOR)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOAuthLoginStartResponse {
    pub login_id: String,
    pub auth_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceAuthStartResponse {
    pub login_id: String,
    pub user_code: String,
    pub verification_url: String,
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexDeviceAuthErrorEvent {
    login_id: String,
    error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexOAuthLoginCallbackEvent {
    login_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexOAuthLoginTimeoutEvent {
    login_id: String,
    callback_url: String,
    timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthState {
    login_id: String,
    auth_url: String,
    redirect_uri: String,
    code_verifier: String,
    state: String,
    port: u16,
    expires_at: i64,
    code: Option<String>,
    #[serde(default)]
    device_auth_id: Option<String>,
    #[serde(default)]
    device_user_code: Option<String>,
    #[serde(default)]
    device_poll_interval_seconds: Option<u64>,
    #[serde(default)]
    exchange_redirect_uri: Option<String>,
}

lazy_static::lazy_static! {
    static ref OAUTH_STATE: Arc<Mutex<Option<OAuthState>>> = Arc::new(Mutex::new(None));
    static ref COMPLETE_ATTEMPT_SEQ: AtomicU64 = AtomicU64::new(0);
}

fn generate_base64url_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.gen::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_challenge(code_verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let result = hasher.finalize();
    URL_SAFE_NO_PAD.encode(result)
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn extract_token_error_code(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value
        .get("error")
        .and_then(|item| item.as_str())
        .or_else(|| {
            value
                .get("error")
                .and_then(|item| item.get("code"))
                .and_then(|item| item.as_str())
        })
        .or_else(|| value.get("code").and_then(|item| item.as_str()))
        .map(|item| item.to_string())
}

fn load_pending_state_from_disk() -> Option<OAuthState> {
    match crate::modules::oauth_pending_state::load::<OAuthState>(OAUTH_STATE_FILE) {
        Ok(Some(state)) => {
            if state.expires_at <= now_timestamp() {
                let _ = crate::modules::oauth_pending_state::clear(OAUTH_STATE_FILE);
                None
            } else {
                Some(state)
            }
        }
        Ok(None) => None,
        Err(err) => {
            logger::log_warn(&format!(
                "Codex OAuth 读取持久化 pending 状态失败，已忽略: {}",
                err
            ));
            let _ = crate::modules::oauth_pending_state::clear(OAUTH_STATE_FILE);
            None
        }
    }
}

fn persist_state_to_disk(state: Option<&OAuthState>) {
    let result = match state {
        // Device auth is tied to an in-process poller. Do not restore a stale
        // device code after restart as if it were a local callback flow.
        Some(value) if value.device_auth_id.is_some() => {
            crate::modules::oauth_pending_state::clear(OAUTH_STATE_FILE)
        }
        Some(value) => crate::modules::oauth_pending_state::save(OAUTH_STATE_FILE, value),
        None => crate::modules::oauth_pending_state::clear(OAUTH_STATE_FILE),
    };
    if let Err(err) = result {
        logger::log_warn(&format!("Codex OAuth 写入持久化 pending 状态失败: {}", err));
    }
}

fn hydrate_oauth_state_if_missing() {
    let mut guard = OAUTH_STATE.lock().unwrap();
    if guard.is_none() {
        *guard = load_pending_state_from_disk();
    }
}

fn set_oauth_state(state: Option<OAuthState>) {
    {
        let mut guard = OAUTH_STATE.lock().unwrap();
        *guard = state.clone();
    }
    persist_state_to_disk(state.as_ref());
}

fn ensure_callback_listener_for_state(app_handle: &AppHandle, state: &OAuthState) {
    if state.device_auth_id.is_some() {
        return;
    }
    if state.expires_at <= now_timestamp() {
        clear_oauth_state_if_matches(&state.state, &state.login_id);
        return;
    }

    match TcpListener::bind(("127.0.0.1", state.port)) {
        Ok(listener) => {
            drop(listener);
            let expected_state = state.state.clone();
            let expected_login_id = state.login_id.clone();
            let callback_url = state.redirect_uri.clone();
            let app_handle_clone = app_handle.clone();
            let port = state.port;
            tokio::spawn(async move {
                if let Err(e) = start_callback_server(
                    port,
                    expected_state,
                    expected_login_id,
                    callback_url,
                    app_handle_clone,
                )
                .await
                {
                    logger::log_error(&format!("OAuth 回调服务器错误: {}", e));
                }
            });
            logger::log_info(&format!(
                "Codex OAuth 已恢复回调监听: login_id={}, port={}",
                state.login_id, state.port
            ));
        }
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            logger::log_info(&format!(
                "Codex OAuth 回调端口已占用，视为监听中: login_id={}, port={}",
                state.login_id, state.port
            ));
        }
        Err(err) => {
            logger::log_warn(&format!(
                "Codex OAuth 回调监听恢复失败: login_id={}, port={}, error={}",
                state.login_id, state.port, err
            ));
        }
    }
}

fn find_available_port() -> Result<u16, String> {
    match TcpListener::bind(("127.0.0.1", OAUTH_CALLBACK_PORT)) {
        Ok(listener) => {
            drop(listener);
            Ok(OAUTH_CALLBACK_PORT)
        }
        Err(e) if e.kind() == ErrorKind::AddrInUse => Err(format!(
            "{}:{}",
            OAUTH_PORT_IN_USE_CODE, OAUTH_CALLBACK_PORT
        )),
        Err(e) => Err(format!("无法绑定端口 {}: {}", OAUTH_CALLBACK_PORT, e)),
    }
}

fn parse_device_poll_interval(value: Option<&serde_json::Value>) -> u64 {
    value
        .and_then(|item| item.as_u64().or_else(|| item.as_str()?.parse().ok()))
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEVICE_DEFAULT_POLL_SECONDS)
}

async fn request_device_user_code() -> Result<(String, String, u64), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(DEVICE_REQUEST_TIMEOUT_SECONDS))
        .timeout(Duration::from_secs(DEVICE_REQUEST_TIMEOUT_SECONDS))
        .build()
        .map_err(|error| format!("创建 Codex 设备授权 HTTP 客户端失败: {}", error))?;
    let response = client
        .post(DEVICE_USER_CODE_ENDPOINT)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "client_id": CLIENT_ID }))
        .send()
        .await
        .map_err(|error| format!("请求 Codex 设备授权码失败: {}", error))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Codex 设备授权响应失败: {}", error))?;
    if !status.is_success() {
        return Err(format!("Codex 设备授权码请求失败: status={}", status));
    }
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("解析 Codex 设备授权响应失败: {}", error))?;
    let device_auth_id = value
        .get("device_auth_id")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let user_code = value
        .get("user_code")
        .or_else(|| value.get("usercode"))
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if device_auth_id.is_empty() || user_code.is_empty() {
        return Err("Codex 设备授权响应缺少 device_auth_id 或 user_code".to_string());
    }
    Ok((
        device_auth_id,
        user_code,
        parse_device_poll_interval(value.get("interval")),
    ))
}

fn clear_oauth_state_for_login_id(login_id: &str) {
    let should_clear = OAUTH_STATE
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|state| state.login_id == login_id);
    if should_clear {
        set_oauth_state(None);
    }
}

fn emit_device_auth_error(app: &AppHandle, login_id: &str, error: String) {
    clear_oauth_state_for_login_id(login_id);
    let _ = app.emit(
        "codex-device-auth-error",
        CodexDeviceAuthErrorEvent {
            login_id: login_id.to_string(),
            error,
        },
    );
}

async fn poll_device_token(
    app: AppHandle,
    login_id: String,
    device_auth_id: String,
    user_code: String,
    interval: u64,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(DEVICE_TIMEOUT_SECONDS);
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(DEVICE_REQUEST_TIMEOUT_SECONDS))
        .timeout(Duration::from_secs(DEVICE_REQUEST_TIMEOUT_SECONDS))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            emit_device_auth_error(&app, &login_id, format!("创建 HTTP 客户端失败: {}", error));
            return;
        }
    };
    loop {
        let active = OAUTH_STATE.lock().unwrap().as_ref().is_some_and(|state| {
            state.login_id == login_id
                && state.device_auth_id.as_deref() == Some(device_auth_id.as_str())
        });
        if !active {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            clear_oauth_state_for_login_id(&login_id);
            let _ = app.emit(
                "codex-oauth-login-timeout",
                CodexOAuthLoginTimeoutEvent {
                    login_id,
                    callback_url: DEVICE_VERIFICATION_URL.to_string(),
                    timeout_seconds: DEVICE_TIMEOUT_SECONDS,
                },
            );
            return;
        }
        match client
            .post(DEVICE_TOKEN_ENDPOINT)
            .header("Accept", "application/json")
            .json(&serde_json::json!({ "device_auth_id": device_auth_id, "user_code": user_code }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let value: serde_json::Value = match response.json().await {
                    Ok(value) => value,
                    Err(error) => {
                        emit_device_auth_error(
                            &app,
                            &login_id,
                            format!("解析 Codex 设备令牌响应失败: {}", error),
                        );
                        return;
                    }
                };
                let code = value
                    .get("authorization_code")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .trim();
                let verifier = value
                    .get("code_verifier")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .trim();
                let challenge = value
                    .get("code_challenge")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .trim();
                if code.is_empty() || verifier.is_empty() || challenge.is_empty() {
                    emit_device_auth_error(
                        &app,
                        &login_id,
                        "Codex 设备令牌响应缺少必要字段".to_string(),
                    );
                    return;
                }
                let mut guard = OAUTH_STATE.lock().unwrap();
                if let Some(state) = guard.as_mut().filter(|state| state.login_id == login_id) {
                    state.code = Some(code.to_string());
                    state.code_verifier = verifier.to_string();
                    persist_state_to_disk(Some(state));
                    drop(guard);
                    let _ = app.emit(
                        "codex-oauth-login-completed",
                        CodexOAuthLoginCallbackEvent { login_id },
                    );
                }
                return;
            }
            Ok(response)
                if response.status() == reqwest::StatusCode::FORBIDDEN
                    || response.status() == reqwest::StatusCode::NOT_FOUND => {}
            Ok(response) => {
                emit_device_auth_error(
                    &app,
                    &login_id,
                    format!("Codex 设备授权轮询失败: status={}", response.status()),
                );
                return;
            }
            Err(error) => logger::log_warn(&format!("Codex 设备授权轮询请求失败: {}", error)),
        }
        tokio::time::sleep(Duration::from_secs(interval.max(1))).await;
    }
}

pub async fn start_device_auth(app: AppHandle) -> Result<CodexDeviceAuthStartResponse, String> {
    hydrate_oauth_state_if_missing();
    if OAUTH_STATE.lock().unwrap().is_some() {
        return Err("Codex OAuth 登录会话已存在，请先取消当前流程".to_string());
    }
    let (device_auth_id, user_code, interval) = request_device_user_code().await?;
    let login_id = generate_base64url_token();
    set_oauth_state(Some(OAuthState {
        login_id: login_id.clone(),
        auth_url: DEVICE_VERIFICATION_URL.to_string(),
        redirect_uri: DEVICE_EXCHANGE_REDIRECT_URI.to_string(),
        code_verifier: String::new(),
        state: generate_base64url_token(),
        port: 0,
        expires_at: now_timestamp() + DEVICE_TIMEOUT_SECONDS as i64,
        code: None,
        device_auth_id: Some(device_auth_id.clone()),
        device_user_code: Some(user_code.clone()),
        device_poll_interval_seconds: Some(interval),
        exchange_redirect_uri: Some(DEVICE_EXCHANGE_REDIRECT_URI.to_string()),
    }));
    tokio::spawn(poll_device_token(
        app,
        login_id.clone(),
        device_auth_id,
        user_code.clone(),
        interval,
    ));
    Ok(CodexDeviceAuthStartResponse {
        login_id,
        user_code,
        verification_url: DEVICE_VERIFICATION_URL.to_string(),
        poll_interval_seconds: interval,
    })
}

fn notify_cancel(port: u16) {
    if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
        let _ = stream
            .write_all(b"GET /cancel HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        let _ = stream.flush();
    }
}

fn decode_query_component(value: &str) -> String {
    urlencoding::decode(value)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| value.to_string())
}

fn parse_query_params(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?.trim();
            if key.is_empty() {
                return None;
            }
            let raw_value = parts.next().unwrap_or("");
            Some((key.to_string(), decode_query_component(raw_value)))
        })
        .collect()
}

fn parse_callback_url(callback_url: &str, port: u16) -> Result<Url, String> {
    let trimmed = callback_url.trim();
    if trimmed.is_empty() {
        return Err("回调链接不能为空".to_string());
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Url::parse(trimmed).map_err(|e| format!("回调链接格式无效: {}", e));
    }

    if trimmed.starts_with('/') {
        return Url::parse(format!("http://localhost:{}{}", port, trimmed).as_str())
            .map_err(|e| format!("回调链接格式无效: {}", e));
    }

    Url::parse(
        format!(
            "http://localhost:{}/auth/callback?{}",
            port,
            trimmed.trim_start_matches('?')
        )
        .as_str(),
    )
    .map_err(|e| format!("回调链接格式无效: {}", e))
}

fn build_auth_url(redirect_uri: &str, code_challenge: &str, state: &str) -> String {
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true&state={}&originator={}",
        AUTH_ENDPOINT,
        CLIENT_ID,
        urlencoding::encode(redirect_uri),
        urlencoding::encode(SCOPES),
        code_challenge,
        state,
        urlencoding::encode(ORIGINATOR)
    )
}

fn to_start_response(state: &OAuthState) -> CodexOAuthLoginStartResponse {
    CodexOAuthLoginStartResponse {
        login_id: state.login_id.clone(),
        auth_url: state.auth_url.clone(),
    }
}

fn clear_oauth_state_if_matches(expected_state: &str, expected_login_id: &str) {
    let should_clear = {
        let oauth_state = OAUTH_STATE.lock().unwrap();
        oauth_state
            .as_ref()
            .is_some_and(|s| s.state == expected_state && s.login_id == expected_login_id)
    };
    if should_clear {
        set_oauth_state(None);
    }
}

pub async fn start_oauth_login(
    app_handle: AppHandle,
) -> Result<CodexOAuthLoginStartResponse, String> {
    hydrate_oauth_state_if_missing();
    {
        let oauth_state = OAUTH_STATE.lock().unwrap();
        if let Some(state) = oauth_state.as_ref() {
            if state.expires_at <= now_timestamp() {
                let expected_state = state.state.clone();
                let expected_login_id = state.login_id.clone();
                drop(oauth_state);
                clear_oauth_state_if_matches(&expected_state, &expected_login_id);
            } else {
                ensure_callback_listener_for_state(&app_handle, state);
                logger::log_info(&format!(
                    "Codex OAuth 复用进行中的登录会话: login_id={}, port={}, redirect_uri={}",
                    state.login_id, state.port, state.redirect_uri
                ));
                return Ok(to_start_response(state));
            }
        }
    }

    let port = find_available_port()?;
    let code_verifier = generate_base64url_token();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state_token = generate_base64url_token();
    let login_id = generate_base64url_token();
    let redirect_uri = format!("http://localhost:{}/auth/callback", port);
    let auth_url = build_auth_url(&redirect_uri, &code_challenge, &state_token);

    let oauth_state = OAuthState {
        login_id: login_id.clone(),
        auth_url: auth_url.clone(),
        redirect_uri: redirect_uri.clone(),
        code_verifier: code_verifier.clone(),
        state: state_token.clone(),
        port,
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS,
        code: None,
        device_auth_id: None,
        device_user_code: None,
        device_poll_interval_seconds: None,
        exchange_redirect_uri: None,
    };

    set_oauth_state(Some(oauth_state));

    let app_handle_clone = app_handle.clone();
    let expected_state = state_token.clone();
    let expected_login_id = login_id.clone();
    let callback_url = redirect_uri.clone();
    tokio::spawn(async move {
        if let Err(e) = start_callback_server(
            port,
            expected_state,
            expected_login_id,
            callback_url,
            app_handle_clone,
        )
        .await
        {
            logger::log_error(&format!("OAuth 回调服务器错误: {}", e));
        }
    });

    logger::log_info(&format!(
        "Codex OAuth 登录会话已创建: login_id={}, port={}, redirect_uri={}",
        login_id, port, redirect_uri
    ));

    Ok(CodexOAuthLoginStartResponse { login_id, auth_url })
}

async fn start_callback_server(
    port: u16,
    expected_state: String,
    expected_login_id: String,
    callback_url: String,
    app_handle: AppHandle,
) -> Result<(), String> {
    use tiny_http::{Response, Server};

    let server = Server::http(format!("127.0.0.1:{}", port))
        .map_err(|e| format!("启动服务器失败: {}", e))?;
    let timeout = std::time::Duration::from_secs(OAUTH_TIMEOUT_SECONDS as u64);

    logger::log_info(&format!(
        "Codex OAuth 回调服务器启动: login_id={}, port={}, timeout_seconds={}",
        expected_login_id,
        port,
        timeout.as_secs()
    ));

    let start = std::time::Instant::now();
    let mut clear_state_on_exit = false;

    loop {
        let should_stop = {
            let oauth_state = OAUTH_STATE.lock().unwrap();
            match oauth_state.as_ref() {
                Some(state) => state.state != expected_state || state.login_id != expected_login_id,
                None => true,
            }
        };

        if should_stop {
            logger::log_info(&format!(
                "Codex OAuth 已取消或状态已变更，停止回调监听: login_id={}",
                expected_login_id
            ));
            break;
        }

        if start.elapsed() > timeout {
            logger::log_error(&format!(
                "Codex OAuth 回调超时: login_id={}, callback_url={}, elapsed={}s",
                expected_login_id,
                callback_url,
                start.elapsed().as_secs()
            ));
            clear_state_on_exit = true;
            break;
        }

        if let Ok(Some(request)) = server.try_recv() {
            let url = request.url().to_string();

            if url.starts_with("/auth/callback") {
                let has_query = url.contains('?');
                logger::log_info(&format!(
                    "Codex OAuth 收到回调请求: login_id={}, path=/auth/callback, has_query={}",
                    expected_login_id, has_query
                ));
                let query = url.split('?').nth(1).unwrap_or("");
                let params = parse_query_params(query);
                let code = params.get("code").cloned().unwrap_or_default();
                let state = params.get("state").cloned().unwrap_or_default();
                logger::log_info(&format!(
                    "Codex OAuth 回调参数检查: login_id={}, has_code={}, has_state={}",
                    expected_login_id,
                    !code.is_empty(),
                    !state.is_empty()
                ));

                if state != expected_state {
                    logger::log_warn(&format!(
                        "Codex OAuth 回调 state 不匹配: login_id={}, expected_state={}, actual_state={}",
                        expected_login_id, expected_state, state
                    ));
                    let response = Response::from_string("State mismatch").with_status_code(400);
                    let _ = request.respond(response);
                    continue;
                }

                if code.is_empty() {
                    let mut param_keys = params.keys().cloned().collect::<Vec<_>>();
                    param_keys.sort();
                    logger::log_warn(&format!(
                        "Codex OAuth 回调缺少 code: login_id={}, param_keys={:?}",
                        expected_login_id, param_keys
                    ));
                    let response = Response::from_string("Missing code").with_status_code(400);
                    let _ = request.respond(response);
                    continue;
                }

                let html = r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>授权成功</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); }
        .container { text-align: center; color: white; }
        h1 { font-size: 2.5rem; margin-bottom: 1rem; }
        p { font-size: 1.2rem; opacity: 0.9; }
    </style>
</head>
<body>
    <div class="container">
        <h1>✅ 授权成功</h1>
        <p>您可以关闭此窗口并返回应用</p>
    </div>
</body>
</html>"#;

                let response = Response::from_string(html).with_header(
                    tiny_http::Header::from_bytes(
                        &b"Content-Type"[..],
                        &b"text/html; charset=utf-8"[..],
                    )
                    .unwrap(),
                );
                let _ = request.respond(response);

                let login_id = {
                    let mut oauth_state = OAUTH_STATE.lock().unwrap();
                    if let Some(state_data) = oauth_state.as_mut() {
                        if state_data.state == expected_state
                            && state_data.login_id == expected_login_id
                        {
                            state_data.code = Some(code.clone());
                            persist_state_to_disk(Some(state_data));
                            Some(state_data.login_id.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(login_id) = login_id {
                    let _ = app_handle.emit(
                        "codex-oauth-login-completed",
                        CodexOAuthLoginCallbackEvent { login_id },
                    );
                    // 兼容新前端页面（GitHub Copilot 账号管理）使用的事件名。
                    // 目前仍复用 Codex OAuth 的后端实现，因此在这里双发事件，前端可逐步迁移。
                    let _ = app_handle.emit(
                        "ghcp-oauth-login-completed",
                        CodexOAuthLoginCallbackEvent {
                            login_id: expected_login_id.clone(),
                        },
                    );
                    logger::log_info(&format!(
                        "Codex OAuth 回调校验通过并已通知前端: login_id={}",
                        expected_login_id
                    ));
                }

                break;
            } else if url.starts_with("/cancel") {
                let response = Response::from_string("Login cancelled").with_status_code(200);
                let _ = request.respond(response);
                clear_oauth_state_if_matches(&expected_state, &expected_login_id);
                logger::log_info(&format!(
                    "Codex OAuth 收到本地取消请求: login_id={}",
                    expected_login_id
                ));
                break;
            } else {
                let response = Response::from_string("Not Found").with_status_code(404);
                let _ = request.respond(response);
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    if clear_state_on_exit {
        clear_oauth_state_if_matches(&expected_state, &expected_login_id);
        logger::log_info(&format!(
            "Codex OAuth 已在超时后清理状态: login_id={}",
            expected_login_id
        ));
        let _ = app_handle.emit(
            "codex-oauth-login-timeout",
            CodexOAuthLoginTimeoutEvent {
                login_id: expected_login_id.clone(),
                callback_url: callback_url.clone(),
                timeout_seconds: timeout.as_secs(),
            },
        );
        let _ = app_handle.emit(
            "ghcp-oauth-login-timeout",
            CodexOAuthLoginTimeoutEvent {
                login_id: expected_login_id.clone(),
                callback_url: callback_url.clone(),
                timeout_seconds: timeout.as_secs(),
            },
        );
        logger::log_info(&format!(
            "Codex OAuth 已发送超时事件到前端: login_id={}, callback_url={}, timeout_seconds={}",
            expected_login_id,
            callback_url,
            timeout.as_secs()
        ));
    }

    Ok(())
}

async fn exchange_code_for_token_internal(
    code: &str,
    code_verifier: &str,
    port: u16,
    exchange_redirect_uri: Option<&str>,
) -> Result<CodexTokens, String> {
    let redirect_uri = resolve_exchange_redirect_uri(port, exchange_redirect_uri);
    let client = reqwest::Client::new();

    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &redirect_uri),
        ("client_id", CLIENT_ID),
        ("code_verifier", code_verifier),
    ];

    logger::log_info("Codex OAuth 开始交换 Token");

    // 官方 authorization-code exchange 使用 raw auth client，不附加运行时 originator headers。
    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token 请求失败: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    if !status.is_success() {
        logger::log_error(&format!(
            "Token 交换失败: status={}, body_len={}",
            status,
            body.len()
        ));
        return Err(format!(
            "Token 交换失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    logger::log_info("Codex OAuth Token 交换成功");

    let token_response: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("解析 Token 响应失败: {}", e))?;

    let id_token = token_response
        .get("id_token")
        .and_then(|v| v.as_str())
        .ok_or("响应中缺少 id_token")?
        .to_string();

    let access_token = token_response
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("响应中缺少 access_token")?
        .to_string();

    let refresh_token = token_response
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(CodexTokens {
        id_token,
        access_token,
        refresh_token,
    })
}

fn resolve_exchange_redirect_uri(port: u16, exchange_redirect_uri: Option<&str>) -> String {
    exchange_redirect_uri
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("http://localhost:{}/auth/callback", port))
}

pub async fn complete_oauth_login(login_id: &str) -> Result<CodexTokens, String> {
    hydrate_oauth_state_if_missing();
    let attempt_id = COMPLETE_ATTEMPT_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let started_at_ms = chrono::Utc::now().timestamp_millis();
    logger::log_info(&format!(
        "Codex OAuth 开始完成登录: attempt_id={}, login_id={}, started_at_ms={}",
        attempt_id, login_id, started_at_ms
    ));
    let (code, code_verifier, port, exchange_redirect_uri) = {
        let oauth_state = OAUTH_STATE.lock().unwrap();
        let state = oauth_state
            .as_ref()
            .ok_or("OAuth 状态不存在，请重新发起授权")?;
        if state.expires_at <= now_timestamp() {
            return Err("OAuth 登录已超时，请重新发起授权".to_string());
        }
        if state.login_id != login_id {
            logger::log_warn(&format!(
                "Codex OAuth loginId 不匹配: attempt_id={}, requested={}, current={}",
                attempt_id, login_id, state.login_id
            ));
            return Err("OAuth loginId 不匹配".to_string());
        }

        let code = state
            .code
            .clone()
            .ok_or("授权尚未完成，请先在浏览器中授权")?;
        logger::log_info(&format!(
            "Codex OAuth 准备完成登录: attempt_id={}, login_id={}",
            attempt_id, login_id
        ));
        (
            code,
            state.code_verifier.clone(),
            state.port,
            state.exchange_redirect_uri.clone(),
        )
    };

    let tokens = match exchange_code_for_token_internal(
        &code,
        &code_verifier,
        port,
        exchange_redirect_uri.as_deref(),
    )
    .await
    {
        Ok(tokens) => tokens,
        Err(e) => {
            let finished_ms = chrono::Utc::now().timestamp_millis();
            logger::log_error(&format!(
                "Codex OAuth 完成登录失败: attempt_id={}, login_id={}, duration_ms={}, error={}",
                attempt_id,
                login_id,
                finished_ms - started_at_ms,
                e
            ));
            return Err(e);
        }
    };

    set_oauth_state(None);

    logger::log_info(&format!(
        "Codex OAuth 完成并清理状态: attempt_id={}, login_id={}, duration_ms={}",
        attempt_id,
        login_id,
        chrono::Utc::now().timestamp_millis() - started_at_ms
    ));
    Ok(tokens)
}

pub fn cancel_oauth_flow_for(login_id: Option<&str>) -> Result<(), String> {
    hydrate_oauth_state_if_missing();
    let port = {
        let oauth_state = OAUTH_STATE.lock().unwrap();
        let Some(current) = oauth_state.as_ref() else {
            logger::log_info("Codex OAuth 取消请求已忽略：当前无活动流程");
            return Ok(());
        };
        logger::log_info(&format!(
            "Codex OAuth 收到取消请求: current_login_id={}, current_port={}",
            current.login_id, current.port,
        ));

        if let Some(login_id) = login_id {
            if current.login_id != login_id {
                logger::log_warn(&format!(
                    "Codex OAuth 取消失败，loginId 不匹配: requested={}, current={}",
                    login_id, current.login_id
                ));
                return Err("OAuth loginId 不匹配".to_string());
            }
        }

        let port = current.port;
        port
    };
    set_oauth_state(None);

    if port > 0 {
        notify_cancel(port);
    }
    logger::log_info(&format!(
        "Codex OAuth 流程已取消: login_id={}",
        login_id.unwrap_or("<none>")
    ));
    Ok(())
}

pub fn submit_callback_url(login_id: &str, callback_url: &str) -> Result<(), String> {
    hydrate_oauth_state_if_missing();
    let (expected_state, port) = {
        let guard = OAUTH_STATE.lock().unwrap();
        let state = guard
            .as_ref()
            .ok_or_else(|| "OAuth 状态不存在，请重新发起授权".to_string())?;
        if state.login_id != login_id {
            return Err("OAuth loginId 不匹配".to_string());
        }
        (state.state.clone(), state.port)
    };

    let parsed = parse_callback_url(callback_url, port)?;
    if parsed.path() != "/auth/callback" {
        return Err("回调链接路径无效，必须为 /auth/callback".to_string());
    }

    let params = parse_query_params(parsed.query().unwrap_or_default());
    let code = params
        .get("code")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "回调链接中缺少 code 参数".to_string())?
        .to_string();
    let state = params
        .get("state")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "回调链接中缺少 state 参数".to_string())?;

    if state != expected_state {
        return Err("回调 state 校验失败，请确认粘贴的是当前登录会话链接".to_string());
    }

    let mut guard = OAUTH_STATE.lock().unwrap();
    let current = guard
        .as_mut()
        .ok_or_else(|| "OAuth 状态不存在，请重新发起授权".to_string())?;
    if current.login_id != login_id {
        return Err("OAuth loginId 不匹配".to_string());
    }
    current.code = Some(code);
    persist_state_to_disk(Some(current));

    logger::log_info(&format!(
        "Codex OAuth 已接收手动回调链接: login_id={}",
        login_id
    ));
    Ok(())
}

pub fn restore_pending_oauth_listener(app_handle: AppHandle) {
    hydrate_oauth_state_if_missing();
    let state = {
        let guard = OAUTH_STATE.lock().unwrap();
        guard.as_ref().cloned()
    };
    if let Some(pending) = state.as_ref() {
        ensure_callback_listener_for_state(&app_handle, pending);
    }
}

fn is_jwt_token_expired(token: &str) -> bool {
    let Some(exp) = jwt_token_expiration_timestamp(token) else {
        return true;
    };

    exp < chrono::Utc::now().timestamp() + TOKEN_REFRESH_SKEW_SECONDS
}

fn jwt_token_expiration_timestamp(token: &str) -> Option<i64> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let payload_str = String::from_utf8(payload_bytes).ok()?;
    let payload: serde_json::Value = serde_json::from_str(&payload_str).ok()?;
    payload.get("exp").and_then(|value| value.as_i64())
}

pub fn is_id_token_expired(id_token: &str) -> bool {
    is_jwt_token_expired(id_token.trim())
}

pub fn is_id_token_refresh_due(id_token: &str) -> bool {
    let Some(exp) = jwt_token_expiration_timestamp(id_token.trim()) else {
        return true;
    };

    exp <= chrono::Utc::now().timestamp() + ID_TOKEN_REFRESH_LEAD_SECONDS
}

pub fn is_token_expired(access_token: &str) -> bool {
    is_jwt_token_expired(access_token)
}

pub async fn refresh_access_token(refresh_token: &str) -> Result<CodexTokens, String> {
    refresh_access_token_with_fallback(refresh_token, None).await
}

fn resolve_refreshed_id_token(
    token_response: &serde_json::Value,
    current_id_token: Option<&str>,
) -> Result<String, String> {
    if let Some(id_token) = token_response
        .get("id_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if is_id_token_expired(id_token) {
            return Err("Token 刷新响应中的 id_token 已过期或无效".to_string());
        }
        return Ok(id_token.to_string());
    }

    if let Some(id_token) = current_id_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| !is_id_token_expired(value))
    {
        return Ok(id_token.to_string());
    }

    Err("响应中缺少 id_token，且本地 id_token 已过期或无效".to_string())
}

pub async fn refresh_access_token_with_fallback(
    refresh_token: &str,
    current_id_token: Option<&str>,
) -> Result<CodexTokens, String> {
    let client = reqwest::Client::new();

    logger::log_info("Codex Token 刷新中...");

    let response = apply_codex_auth_identity_headers(client.post(TOKEN_ENDPOINT))
        .json(&serde_json::json!({
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .map_err(|e| format!("Token 刷新请求失败: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    if !status.is_success() {
        let error_code = extract_token_error_code(&body);
        logger::log_error(&format!(
            "Token 刷新失败: status={}, error_code={:?}, body_len={}",
            status,
            error_code,
            body.len()
        ));
        let mut message = format!("Token 刷新失败: status={}", status);
        if let Some(code) = error_code {
            message.push_str(&format!(", error_code={}", code));
        }
        message.push_str(&format!(", body_len={}", body.len()));
        return Err(message);
    }

    logger::log_info("Codex Token 刷新成功");

    let token_response: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("解析 Token 响应失败: {}", e))?;

    let id_token = resolve_refreshed_id_token(&token_response, current_id_token)?;

    let access_token = token_response
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("响应中缺少 access_token")?
        .to_string();

    let new_refresh_token = token_response
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| Some(refresh_token.to_string()));

    Ok(CodexTokens {
        id_token,
        access_token,
        refresh_token: new_refresh_token,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        is_id_token_refresh_due, parse_device_poll_interval, resolve_exchange_redirect_uri,
        resolve_refreshed_id_token, DEVICE_DEFAULT_POLL_SECONDS, DEVICE_EXCHANGE_REDIRECT_URI,
        ID_TOKEN_REFRESH_LEAD_SECONDS, ORIGINATOR, TOKEN_REFRESH_SKEW_SECONDS,
    };
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    fn make_jwt(exp: i64) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::json!({ "exp": exp }).to_string());
        format!("{}.{}.sig", header, payload)
    }

    #[test]
    fn oauth_originator_matches_current_desktop_client() {
        assert_eq!(ORIGINATOR, "Codex Desktop");
    }

    #[test]
    fn device_auth_uses_official_exchange_redirect_and_poll_interval() {
        assert_eq!(
            resolve_exchange_redirect_uri(0, Some(DEVICE_EXCHANGE_REDIRECT_URI)),
            DEVICE_EXCHANGE_REDIRECT_URI
        );
        assert_eq!(
            parse_device_poll_interval(None),
            DEVICE_DEFAULT_POLL_SECONDS
        );
        assert_eq!(parse_device_poll_interval(Some(&serde_json::json!("7"))), 7);
        assert_eq!(
            resolve_exchange_redirect_uri(1455, None),
            "http://localhost:1455/auth/callback"
        );
    }

    #[test]
    fn id_token_is_due_within_refresh_lead_time() {
        let due = chrono::Utc::now().timestamp() + ID_TOKEN_REFRESH_LEAD_SECONDS - 30;
        assert!(is_id_token_refresh_due(&make_jwt(due)));
    }

    #[test]
    fn id_token_is_not_due_beyond_refresh_lead_time() {
        let fresh = chrono::Utc::now().timestamp() + ID_TOKEN_REFRESH_LEAD_SECONDS + 3600;
        assert!(!is_id_token_refresh_due(&make_jwt(fresh)));
    }

    #[test]
    fn refreshed_id_token_replaces_expired_current_value() {
        let fresh = make_jwt(chrono::Utc::now().timestamp() + TOKEN_REFRESH_SKEW_SECONDS + 3600);
        let expired = make_jwt(chrono::Utc::now().timestamp() - 3600);
        let response = serde_json::json!({ "id_token": fresh });

        assert_eq!(
            resolve_refreshed_id_token(&response, Some(&expired))
                .expect("replace expired id_token"),
            response["id_token"].as_str().unwrap()
        );
    }

    #[test]
    fn refreshed_id_token_reuses_only_fresh_current_value() {
        let fresh = make_jwt(chrono::Utc::now().timestamp() + TOKEN_REFRESH_SKEW_SECONDS + 3600);
        assert_eq!(
            resolve_refreshed_id_token(&serde_json::json!({}), Some(&fresh))
                .expect("reuse fresh id_token"),
            fresh
        );
    }

    #[test]
    fn refreshed_id_token_rejects_missing_value_when_current_is_expired() {
        let expired = make_jwt(chrono::Utc::now().timestamp() - 3600);
        assert!(resolve_refreshed_id_token(&serde_json::json!({}), Some(&expired)).is_err());
    }

    #[test]
    fn refreshed_id_token_rejects_expired_response_value() {
        let expired = make_jwt(chrono::Utc::now().timestamp() - 3600);
        assert!(
            resolve_refreshed_id_token(&serde_json::json!({ "id_token": expired }), None).is_err()
        );
    }
}
