use crate::models::codex::CodexTokens;
use crate::modules::logger;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use url::Url;
use uuid::Uuid;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const ORIGINATOR: &str = "Codex Desktop";
const OAUTH_CALLBACK_PORT: u16 = 1455;
/// 官方 Codex 登录服务在 1455 被占用时会回退到 1457，这里保持一致。
const OAUTH_FALLBACK_CALLBACK_PORT: u16 = 1457;
const OAUTH_PORT_IN_USE_CODE: &str = "CODEX_OAUTH_PORT_IN_USE";
const OAUTH_STATE_FILE: &str = "codex_oauth_pending.json";
const OAUTH_WINDOW_LABEL: &str = "codex-oauth-incognito";
const OAUTH_TIMEOUT_SECONDS: i64 = 10 * 60;
const OFFICIAL_HOSTED_AUTH_ENDPOINT: &str = "https://chatgpt.com/codex/desktop-auth";
const OFFICIAL_CLIENT_IDENTITY_FILE: &str = "codex-official-client-identity.txt";
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 300;
pub const ID_TOKEN_REFRESH_LEAD_SECONDS: i64 = 10 * 60;
const TOKEN_REFRESH_TIMEOUT: Duration = Duration::from_secs(25);
const DEVICE_USER_CODE_ENDPOINT: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_ENDPOINT: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const DEVICE_EXCHANGE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const DEVICE_TIMEOUT_SECONDS: u64 = 15 * 60;
const DEVICE_DEFAULT_POLL_SECONDS: u64 = 5;

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
    /// 本次登录实际使用的出口；None 表示走原有默认授权出口。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<CodexOAuthLoginProxyUse>,
}

/// 本次登录使用的出口描述：只含展示摘要与可编辑地址，绝不含凭据。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOAuthLoginProxyUse {
    /// `account`：按重新授权账号的生效出口解析；`explicit`：沿用用户填写的地址。
    pub source: String,
    /// 出口外观（协议 / 服务器 / 端口，或资源名称），与账号响应使用同一套脱敏规则。
    pub summary: serde_json::Value,
    /// 可回填输入框的地址；资源绑定快照不回填，由后端按登录会话直接使用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
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
    #[serde(default)]
    proxy_required: bool,
}

struct PendingOAuthProxy {
    login_id: String,
    raw_url: String,
    browser_url: String,
    request_url: String,
    tunnel: Option<crate::modules::codex_proxy_engine::NodeTunnel>,
}

pub struct CodexOAuthCompletion {
    pub tokens: CodexTokens,
    pub proxy_url: Option<String>,
}

lazy_static::lazy_static! {
    static ref OAUTH_STATE: Arc<Mutex<Option<OAuthState>>> = Arc::new(Mutex::new(None));
    static ref OAUTH_PROXY: Mutex<Option<PendingOAuthProxy>> = Mutex::new(None);
    static ref COMPLETE_ATTEMPT_SEQ: AtomicU64 = AtomicU64::new(0);
    static ref OFFICIAL_CLIENT_STABLE_ID: Mutex<Option<String>> = Mutex::new(None);
}
static OAUTH_START_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    if state.is_none() {
        if let Ok(mut pending) = OAUTH_PROXY.lock() {
            pending.take();
        }
    }
}

fn active_proxy(login_id: &str) -> Result<(String, String, String), String> {
    let guard = OAUTH_PROXY.lock().map_err(|_| "PROXY_OAUTH_UNAVAILABLE")?;
    let proxy = guard
        .as_ref()
        .filter(|proxy| {
            proxy.login_id == login_id
                && proxy
                    .tunnel
                    .as_ref()
                    .is_none_or(|tunnel| tunnel.is_running())
        })
        .ok_or("PROXY_OAUTH_UNAVAILABLE")?;
    Ok((
        proxy.raw_url.clone(),
        proxy.browser_url.clone(),
        proxy.request_url.clone(),
    ))
}

/// 只读解析：重新授权账号当前的生效出口（账号独立绑定 > 统一代理 > 默认出口）。
///
/// 直接调用 `codex_account_proxy::configured_url`，不复制也不修改优先级实现；
/// 账号不在本地时不借用其他账号的代理，读取失败交由调用方在授权弹框内提示。
async fn resolve_reauth_egress_proxy(account_id: &str) -> Result<Option<String>, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Ok(None);
    }
    let account = crate::modules::codex_proxy_runtime::load(account_id).await?;
    Ok(crate::modules::codex_account_proxy::configured_url(&account)?
        .map(|value| value.into_owned()))
}

/// 出口外观复用账号响应的脱敏摘要；资源绑定快照含凭据且可能很大，绝不回传前端。
fn oauth_proxy_use(source: &str, value: &str) -> CodexOAuthLoginProxyUse {
    struct Redacted<'a>(&'a str);
    impl Serialize for Redacted<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            crate::modules::codex_account_proxy::serialize_summary(
                &Some(self.0.to_string()),
                serializer,
            )
        }
    }
    let summary = serde_json::to_value(Redacted(value))
        .unwrap_or_else(|_| serde_json::json!({ "protocol": "UNKNOWN" }));
    let input = (!value.starts_with(crate::modules::codex_proxy_catalog_binding::PREFIX))
        .then(|| value.to_string());
    CodexOAuthLoginProxyUse {
        source: source.to_string(),
        summary,
        input,
    }
}

fn oauth_proxy_matches_request(requested: Option<&str>, normalized: &str) -> bool {
    requested.is_some_and(|value| {
        crate::modules::codex_proxy_runtime::normalize_binding(value)
            .is_ok_and(|value| value == normalized)
    })
}

async fn prepare_proxy(login_id: String, input: String) -> Result<PendingOAuthProxy, String> {
    let raw_url = crate::modules::codex_proxy_runtime::normalize_binding(&input)?;
    let parsed = Url::parse(&raw_url).map_err(|_| "PROXY_INVALID_URL")?;
    if matches!(parsed.scheme(), "http" | "socks5")
        && parsed.username().is_empty()
        && parsed.password().is_none()
    {
        return Ok(PendingOAuthProxy {
            login_id,
            raw_url: raw_url.clone(),
            browser_url: raw_url.clone(),
            request_url: raw_url,
            tunnel: None,
        });
    }
    let tunnel = crate::modules::codex_proxy_engine::start_desktop(&raw_url).await?;
    let loopback = tunnel.proxy_url().to_string();
    Ok(PendingOAuthProxy {
        login_id,
        raw_url,
        browser_url: loopback.clone(),
        request_url: loopback,
        tunnel: Some(tunnel),
    })
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
    let mut last_error: Option<String> = None;

    for port in [OAUTH_CALLBACK_PORT, OAUTH_FALLBACK_CALLBACK_PORT] {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                drop(listener);
                return Ok(port);
            }
            Err(error) if error.kind() == ErrorKind::AddrInUse => {
                logger::log_warn(&format!(
                    "Codex OAuth 回调端口被占用，尝试下一个端口: port={}",
                    port
                ));
                last_error = None;
            }
            Err(error) => {
                last_error = Some(format!("无法绑定端口 {}: {}", port, error));
            }
        }
    }

    match last_error {
        Some(error) => Err(error),
        None => Err(format!(
            "{}:{}",
            OAUTH_PORT_IN_USE_CODE, OAUTH_CALLBACK_PORT
        )),
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
        .connect_timeout(TOKEN_REFRESH_TIMEOUT)
        .timeout(TOKEN_REFRESH_TIMEOUT)
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

async fn poll_device_token(
    login_id: String,
    device_auth_id: String,
    user_code: String,
    poll_interval_seconds: u64,
    app_handle: AppHandle,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(DEVICE_TIMEOUT_SECONDS);
    let client = match reqwest::Client::builder()
        .connect_timeout(TOKEN_REFRESH_TIMEOUT)
        .timeout(TOKEN_REFRESH_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            emit_device_auth_error(
                &app_handle,
                &login_id,
                format!("创建 HTTP 客户端失败: {}", error),
            );
            return;
        }
    };
    loop {
        let still_active = OAUTH_STATE.lock().unwrap().as_ref().is_some_and(|state| {
            state.login_id == login_id
                && state.device_auth_id.as_deref() == Some(device_auth_id.as_str())
        });
        if !still_active {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            clear_oauth_state_for_login_id(&login_id);
            let _ = app_handle.emit(
                "codex-oauth-login-timeout",
                CodexOAuthLoginTimeoutEvent {
                    login_id: login_id.clone(),
                    callback_url: DEVICE_VERIFICATION_URL.to_string(),
                    timeout_seconds: DEVICE_TIMEOUT_SECONDS,
                },
            );
            return;
        }
        let response = client
            .post(DEVICE_TOKEN_ENDPOINT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "device_auth_id": device_auth_id,
                "user_code": user_code,
            }))
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                let body = match response.text().await {
                    Ok(body) => body,
                    Err(error) => {
                        logger::log_warn(&format!("读取 Codex 设备令牌响应失败: {}", error));
                        continue;
                    }
                };
                let value: serde_json::Value = match serde_json::from_str(&body) {
                    Ok(value) => value,
                    Err(error) => {
                        logger::log_warn(&format!("解析 Codex 设备令牌响应失败: {}", error));
                        continue;
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
                        &app_handle,
                        &login_id,
                        "Codex 设备令牌响应缺少 authorization_code/code_verifier/code_challenge"
                            .to_string(),
                    );
                    return;
                }
                let mut guard = OAUTH_STATE.lock().unwrap();
                if let Some(state) = guard.as_mut().filter(|state| {
                    state.login_id == login_id
                        && state.device_auth_id.as_deref() == Some(device_auth_id.as_str())
                }) {
                    state.code = Some(code.to_string());
                    state.code_verifier = verifier.to_string();
                    state.exchange_redirect_uri = Some(DEVICE_EXCHANGE_REDIRECT_URI.to_string());
                    persist_state_to_disk(Some(state));
                    drop(guard);
                    let _ = app_handle.emit(
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
                    &app_handle,
                    &login_id,
                    format!("Codex 设备授权轮询失败: status={}", response.status()),
                );
                return;
            }
            Err(error) => logger::log_warn(&format!("Codex 设备授权轮询请求失败: {}", error)),
        }
        tokio::time::sleep(Duration::from_secs(poll_interval_seconds.max(1))).await;
    }
}

fn emit_device_auth_error(app_handle: &AppHandle, login_id: &str, error: String) {
    logger::log_warn(&format!(
        "Codex 设备授权失败: login_id={}, error={}",
        login_id, error
    ));
    clear_oauth_state_for_login_id(login_id);
    let _ = app_handle.emit(
        "codex-device-auth-error",
        CodexDeviceAuthErrorEvent {
            login_id: login_id.to_string(),
            error,
        },
    );
}

pub async fn start_device_auth(
    app_handle: AppHandle,
) -> Result<CodexDeviceAuthStartResponse, String> {
    let _start_guard = OAUTH_START_GATE
        .try_lock()
        .map_err(|_| "CODEX_OAUTH_START_BUSY")?;
    hydrate_oauth_state_if_missing();
    if OAUTH_STATE.lock().unwrap().is_some() {
        return Err("Codex OAuth 登录会话已存在，请先取消当前流程".to_string());
    }
    let (device_auth_id, user_code, poll_interval_seconds) = request_device_user_code().await?;
    let login_id = generate_base64url_token();
    let state = OAuthState {
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
        device_poll_interval_seconds: Some(poll_interval_seconds),
        exchange_redirect_uri: Some(DEVICE_EXCHANGE_REDIRECT_URI.to_string()),
        proxy_required: false,
    };
    set_oauth_state(Some(state));
    tokio::spawn(poll_device_token(
        login_id.clone(),
        device_auth_id,
        user_code.clone(),
        poll_interval_seconds,
        app_handle,
    ));
    Ok(CodexDeviceAuthStartResponse {
        login_id,
        user_code,
        verification_url: DEVICE_VERIFICATION_URL.to_string(),
        poll_interval_seconds,
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
    let mut url = Url::parse(AUTH_ENDPOINT).expect("valid Codex OAuth authorize endpoint");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", CLIENT_ID);
        query.append_pair("redirect_uri", redirect_uri);
        query.append_pair("scope", SCOPES);
        query.append_pair("code_challenge", code_challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("id_token_add_organizations", "true");
        query.append_pair("codex_cli_simplified_flow", "true");
        query.append_pair("codex_streamlined_login", "true");
        query.append_pair("state", state);
        query.append_pair("originator", ORIGINATOR);
        query.append_pair("codex_app_version", &official_client_version());
        let stable_id = official_client_stable_id();
        query.append_pair("source_surface_stable_id", &stable_id);
        query.append_pair("codex_origin_stable_id", &stable_id);
    }
    hosted_auth_url(&url.to_string())
}

/// 用户设置优先；留空时使用远端配置缓存，并在无缓存时回退内置默认值。
pub(crate) fn official_client_version() -> String {
    let configured = crate::modules::config::get_user_config().codex_oauth_app_version;
    if let Some(version) =
        crate::modules::remote_config::normalize_codex_oauth_app_version(&configured)
    {
        return version;
    }
    crate::modules::remote_config::cached_codex_oauth_app_version()
}

/// 为两个 stable ID 提供同一个持久化值；它们用于官方登录包装的客户端关联标识。
fn official_client_stable_id() -> String {
    if let Ok(mut cached) = OFFICIAL_CLIENT_STABLE_ID.lock() {
        if let Some(value) = cached.as_ref() {
            return value.clone();
        }

        let path = crate::modules::account::get_data_dir()
            .ok()
            .map(|dir| dir.join(OFFICIAL_CLIENT_IDENTITY_FILE));
        if let Some(path) = path.as_ref() {
            if let Ok(value) = fs::read_to_string(path) {
                let value = value.trim();
                if !value.is_empty() {
                    let value = value.to_string();
                    *cached = Some(value.clone());
                    return value;
                }
            }
        }

        let value = Uuid::new_v4().to_string();
        if let Some(path) = path {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(path, &value);
        }
        *cached = Some(value.clone());
        value
    } else {
        Uuid::new_v4().to_string()
    }
}

/// 将授权地址包装成官方桌面使用的 hosted login 地址。
fn hosted_auth_url(auth_url: &str) -> String {
    format!(
        "{}?authorize_url={}&codex_streamlined_login=true&no_universal_links=1",
        OFFICIAL_HOSTED_AUTH_ENDPOINT,
        urlencoding::encode(auth_url)
    )
}

fn authorize_url_matches_pending(url: &Url, pending: &OAuthState) -> bool {
    Url::parse(&pending.auth_url).is_ok_and(|expected| expected == *url)
}

fn is_callback_navigation(url: &Url, callback_port: u16) -> bool {
    url.scheme() == "http"
        && url.host_str() == Some("localhost")
        && url.port() == Some(callback_port)
        && url.path() == "/auth/callback"
}

pub fn open_incognito_oauth_window(app: &AppHandle, auth_url: &str) -> Result<(), String> {
    hydrate_oauth_state_if_missing();
    let parsed = Url::parse(auth_url.trim())
        .map_err(|error| format!("Codex OAuth 授权地址无效: {}", error))?;
    let pending = OAUTH_STATE
        .lock()
        .map_err(|_| "获取 Codex OAuth 状态锁失败".to_string())?
        .as_ref()
        .cloned()
        .ok_or_else(|| "Codex OAuth 登录会话不存在或已结束".to_string())?;
    if pending.expires_at <= now_timestamp() {
        return Err("Codex OAuth 登录已超时，请重新发起授权".to_string());
    }
    if !authorize_url_matches_pending(&parsed, &pending) {
        return Err("Codex OAuth 授权地址与当前登录会话不匹配".to_string());
    }

    if let Some(window) = app.get_webview_window(OAUTH_WINDOW_LABEL) {
        window
            .destroy()
            .map_err(|error| format!("重置 Codex OAuth 无痕窗口失败: {}", error))?;
    }

    let browser_proxy = if pending.proxy_required {
        if cfg!(target_os = "macos") {
            return Err("PROXY_OAUTH_BROWSER_UNSUPPORTED".into());
        }
        let (_, browser_url, _) = active_proxy(&pending.login_id)?;
        Some(Url::parse(&browser_url).map_err(|_| "PROXY_OAUTH_UNAVAILABLE")?)
    } else {
        None
    };
    let callback_port = pending.port;
    let mut builder =
        WebviewWindowBuilder::new(app, OAUTH_WINDOW_LABEL, WebviewUrl::External(parsed));
    if let Some(proxy) = browser_proxy {
        #[cfg(windows)]
        {
            // WebView2 can reuse a browser process for the same data folder,
            // which would ignore new proxy flags or affect the main window.
            let isolated = app
                .path()
                .app_cache_dir()
                .map_err(|_| "PROXY_OAUTH_UNAVAILABLE")?
                .join("oauth-proxy-webview")
                .join(&pending.login_id);
            builder = builder.data_directory(isolated);
        }
        builder = builder.proxy_url(proxy).on_new_window(|_, _| {
            // A separate popup may not inherit this WebView's proxy. Keep the
            // opted-in flow fail closed instead of opening an uncontrolled tab.
            tauri::webview::NewWindowResponse::Deny
        });
    }
    builder
        .title("Codex OAuth")
        .inner_size(920.0, 720.0)
        .min_inner_size(640.0, 560.0)
        .center()
        .incognito(true)
        .on_navigation(move |url| {
            if is_callback_navigation(url, callback_port) {
                logger::log_info("Codex OAuth 无痕窗口正在访问本地回调地址");
                return true;
            }
            let allowed = matches!(url.scheme(), "https" | "about");
            if !allowed {
                logger::log_warn(&format!(
                    "Codex OAuth 无痕窗口已阻止非 HTTPS 导航: scheme={}",
                    url.scheme()
                ));
            }
            allowed
        })
        .build()
        .map_err(|error| format!("创建 Codex OAuth 无痕窗口失败: {}", error))?;

    logger::log_info(&format!(
        "Codex OAuth 无痕窗口已打开: login_id={}, port={}",
        pending.login_id, pending.port
    ));
    Ok(())
}

pub fn close_oauth_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(OAUTH_WINDOW_LABEL) {
        window
            .destroy()
            .map_err(|error| format!("关闭 Codex OAuth 无痕窗口失败: {}", error))?;
    }
    Ok(())
}

fn to_start_response(
    state: &OAuthState,
    proxy: Option<CodexOAuthLoginProxyUse>,
) -> CodexOAuthLoginStartResponse {
    CodexOAuthLoginStartResponse {
        login_id: state.login_id.clone(),
        auth_url: state.auth_url.clone(),
        proxy,
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

fn clear_oauth_state_for_login_id(expected_login_id: &str) {
    let should_clear = OAUTH_STATE
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|state| state.login_id == expected_login_id);
    if should_clear {
        set_oauth_state(None);
    }
}

pub async fn start_oauth_login(
    app_handle: AppHandle,
    proxy_url: Option<String>,
    reauth_account_id: Option<String>,
) -> Result<CodexOAuthLoginStartResponse, String> {
    let _start_guard = OAUTH_START_GATE
        .try_lock()
        .map_err(|_| "CODEX_OAUTH_START_BUSY")?;
    // 已有账号重新授权且未显式填写地址时，默认沿用该账号当前生效出口；
    // 首次添加没有账号 ID，因此不会带入任何账号代理。解析失败按错误返回，不静默改走直连。
    let (requested_proxy, proxy_source) = match proxy_url.filter(|value| !value.trim().is_empty()) {
        Some(value) => (Some(value), Some("explicit")),
        None => {
            let account_id = reauth_account_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            match account_id {
                Some(account_id) => match resolve_reauth_egress_proxy(account_id).await? {
                    Some(value) => (Some(value), Some("account")),
                    None => (None, None),
                },
                None => (None, None),
            }
        }
    };
    let proxy_use = requested_proxy
        .as_deref()
        .map(|value| oauth_proxy_use(proxy_source.unwrap_or("explicit"), value));
    if requested_proxy.is_some() && cfg!(target_os = "macos") {
        // The current macOS build lacks Tauri's macos-proxy feature. Using its
        // WebView would silently bypass the selected route.
        return Err("PROXY_OAUTH_BROWSER_UNSUPPORTED".into());
    }
    hydrate_oauth_state_if_missing();
    {
        let oauth_state = OAUTH_STATE.lock().unwrap();
        if let Some(state) = oauth_state.as_ref() {
            if state.expires_at <= now_timestamp() {
                let expected_state = state.state.clone();
                let expected_login_id = state.login_id.clone();
                drop(oauth_state);
                clear_oauth_state_if_matches(&expected_state, &expected_login_id);
            } else if state.proxy_required && active_proxy(&state.login_id).is_err() {
                // A restored session has no in-memory proxy lease. Discard it
                // before any browser or token request can use a direct route.
                let expected_state = state.state.clone();
                let expected_login_id = state.login_id.clone();
                drop(oauth_state);
                clear_oauth_state_if_matches(&expected_state, &expected_login_id);
            } else {
                if state.proxy_required != requested_proxy.is_some()
                    || (state.proxy_required
                        && active_proxy(&state.login_id).map_or(true, |(raw, _, _)| {
                            !oauth_proxy_matches_request(requested_proxy.as_deref(), &raw)
                        }))
                {
                    return Err("PROXY_OAUTH_SESSION_ACTIVE".into());
                }
                ensure_callback_listener_for_state(&app_handle, state);
                logger::log_info(&format!(
                    "Codex OAuth 复用进行中的登录会话: login_id={}, port={}, redirect_uri={}",
                    state.login_id, state.port, state.redirect_uri
                ));
                return Ok(to_start_response(state, proxy_use));
            }
        }
    }

    let port = find_available_port()?;
    let code_verifier = generate_base64url_token();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state_token = generate_base64url_token();
    let login_id = generate_base64url_token();
    let prepared_proxy = if let Some(input) = requested_proxy {
        Some(prepare_proxy(login_id.clone(), input).await?)
    } else {
        None
    };
    if OAUTH_STATE
        .lock()
        .map_err(|_| "PROXY_OAUTH_UNAVAILABLE")?
        .is_some()
    {
        return Err("PROXY_OAUTH_SESSION_ACTIVE".into());
    }
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
        proxy_required: prepared_proxy.is_some(),
    };

    *OAUTH_PROXY.lock().map_err(|_| "PROXY_OAUTH_UNAVAILABLE")? = prepared_proxy;
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

    Ok(CodexOAuthLoginStartResponse {
        login_id,
        auth_url,
        proxy: proxy_use,
    })
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
                    if let Err(error) = close_oauth_window(&app_handle) {
                        logger::log_warn(&error);
                    }
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
        if let Err(error) = close_oauth_window(&app_handle) {
            logger::log_warn(&error);
        }
    }

    Ok(())
}

async fn exchange_code_for_token_internal(
    code: &str,
    code_verifier: &str,
    port: u16,
    exchange_redirect_uri: Option<&str>,
    proxy_url: Option<&str>,
) -> Result<CodexTokens, String> {
    let redirect_uri = resolve_exchange_redirect_uri(port, exchange_redirect_uri);
    let mut builder = reqwest::Client::builder()
        .connect_timeout(TOKEN_REFRESH_TIMEOUT)
        .timeout(TOKEN_REFRESH_TIMEOUT);
    if let Some(proxy_url) = proxy_url {
        let proxy = reqwest::Proxy::all(proxy_url).map_err(|_| "PROXY_OAUTH_UNAVAILABLE")?;
        builder = builder.no_proxy().proxy(proxy);
    }
    let client = builder.build().map_err(|_| "PROXY_OAUTH_UNAVAILABLE")?;

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
        .map_err(|e| {
            if proxy_url.is_some() {
                "PROXY_OAUTH_REQUEST_FAILED".to_string()
            } else {
                format!("Token 请求失败: {}", e)
            }
        })?;

    let status = response.status();
    let body = response.text().await.map_err(|e| {
        if proxy_url.is_some() {
            "PROXY_OAUTH_REQUEST_FAILED".to_string()
        } else {
            format!("读取响应失败: {}", e)
        }
    })?;

    if !status.is_success() {
        let response_summary = serde_json::from_str::<serde_json::Value>(&body)
            .map(|value| crate::modules::codex_auth_diagnostic::oauth_response_summary(&value))
            .unwrap_or_else(|_| serde_json::json!({"body_type":"non_json"}));
        crate::modules::codex_auth_diagnostic::log_event(
            "oauth_token_exchange_failed",
            serde_json::json!({
                "status": status.as_u16(),
                "body_length": body.len(),
                "response": response_summary,
            }),
        );
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

    crate::modules::codex_auth_diagnostic::log_event(
        "oauth_token_exchange_response",
        serde_json::json!({
            "status": status.as_u16(),
            "body_length": body.len(),
            "response": crate::modules::codex_auth_diagnostic::oauth_response_summary(&token_response),
        }),
    );

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

pub async fn complete_oauth_login(login_id: &str) -> Result<CodexOAuthCompletion, String> {
    hydrate_oauth_state_if_missing();
    let attempt_id = COMPLETE_ATTEMPT_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let started_at_ms = chrono::Utc::now().timestamp_millis();
    logger::log_info(&format!(
        "Codex OAuth 开始完成登录: attempt_id={}, login_id={}, started_at_ms={}",
        attempt_id, login_id, started_at_ms
    ));
    let (code, code_verifier, port, exchange_redirect_uri, proxy_required) = {
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
            state.proxy_required,
        )
    };

    let (raw_proxy, request_proxy) = if proxy_required {
        let (raw, _, request) = active_proxy(login_id)?;
        (Some(raw), Some(request))
    } else {
        (None, None)
    };

    let tokens = match exchange_code_for_token_internal(
        &code,
        &code_verifier,
        port,
        exchange_redirect_uri.as_deref(),
        request_proxy.as_deref(),
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

    crate::modules::codex_auth_diagnostic::log_event(
        "oauth_login_tokens_received",
        serde_json::json!({
            "attempt_id": attempt_id,
            "login_id": login_id,
            "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&tokens),
        }),
    );

    set_oauth_state(None);

    logger::log_info(&format!(
        "Codex OAuth 完成并清理状态: attempt_id={}, login_id={}, duration_ms={}",
        attempt_id,
        login_id,
        chrono::Utc::now().timestamp_millis() - started_at_ms
    ));
    Ok(CodexOAuthCompletion {
        tokens,
        proxy_url: raw_proxy,
    })
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

pub fn is_jwt_token_expired(token: &str) -> bool {
    let Some(exp) = jwt_token_expiration_timestamp(token) else {
        return true;
    };

    let now = chrono::Utc::now().timestamp();
    exp < now + TOKEN_REFRESH_SKEW_SECONDS
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

pub fn jwt_token_expiration_timestamp(token: &str) -> Option<i64> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload_base64 = parts[1];
    let payload_bytes = match URL_SAFE_NO_PAD.decode(payload_base64) {
        Ok(bytes) => bytes,
        Err(_) => return None,
    };

    let payload_str = match String::from_utf8(payload_bytes) {
        Ok(s) => s,
        Err(_) => return None,
    };

    let payload: serde_json::Value = match serde_json::from_str(&payload_str) {
        Ok(v) => v,
        Err(_) => return None,
    };

    payload.get("exp").and_then(|e| e.as_i64())
}

pub fn is_token_expired(access_token: &str) -> bool {
    let token = access_token.trim();
    if token.is_empty() {
        return true;
    }

    // Some short-lived Codex credentials are opaque ChatGPT access tokens
    // (`at-...`) rather than JWTs. They do not carry a local `exp` claim, so
    // treating them as expired makes Cockpit immediately force an OAuth refresh
    // and reject access-token-only imports. Let the first real upstream request
    // decide whether the opaque token is still accepted; 401 handling will mark
    // it invalid when it actually dies. Other malformed/non-JWT values still
    // fail closed instead of being treated as valid.
    if token.split('.').count() != 3 {
        return !token.starts_with("at-");
    }

    is_jwt_token_expired(token)
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
        if !is_id_token_expired(id_token) {
            return Ok(id_token.to_string());
        }
    }

    if let Some(id_token) = current_id_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // OAuth refresh responses are allowed to omit id_token. Keep the old
        // value long enough to persist a possibly rotated refresh_token. The
        // client-runtime preparation layer validates id_token afterwards and
        // blocks stale auth.json projection when the old value is no longer usable.
        return Ok(id_token.to_string());
    }

    // Codex app-server authentication is based on access_token. Persist the
    // rotated token chain even when the response omits the optional OIDC token;
    // desktop runtime preparation will reject this empty value before launch.
    Ok(String::new())
}

pub async fn refresh_access_token_with_fallback(
    refresh_token: &str,
    current_id_token: Option<&str>,
) -> Result<CodexTokens, String> {
    refresh_access_token_with_account_proxy(refresh_token, current_id_token, None).await
}

pub async fn refresh_access_token_with_account_proxy(
    refresh_token: &str,
    current_id_token: Option<&str>,
    account: Option<&crate::models::codex::CodexAccount>,
) -> Result<CodexTokens, String> {
    let builder = reqwest::Client::builder()
        .connect_timeout(TOKEN_REFRESH_TIMEOUT)
        .timeout(TOKEN_REFRESH_TIMEOUT);
    let builder = match account {
        Some(account) => {
            crate::modules::codex_proxy_runtime::client_builder(account, builder).await?
        }
        None => builder,
    };
    let client = builder
        .build()
        .map_err(|e| format!("创建 Token 刷新客户端失败: {}", e))?;

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
        let response_summary = serde_json::from_str::<serde_json::Value>(&body)
            .map(|value| crate::modules::codex_auth_diagnostic::oauth_response_summary(&value))
            .unwrap_or_else(|_| serde_json::json!({"body_type":"non_json"}));
        crate::modules::codex_auth_diagnostic::log_event(
            "oauth_token_refresh_failed",
            serde_json::json!({
                "status": status.as_u16(),
                "error_code": error_code,
                "body_length": body.len(),
                "response": response_summary,
                "refresh_token": crate::modules::codex_auth_diagnostic::token_summary(refresh_token, "refresh_token"),
            }),
        );
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

    let refreshed_tokens = CodexTokens {
        id_token,
        access_token,
        refresh_token: new_refresh_token,
    };
    crate::modules::codex_auth_diagnostic::log_event(
        "oauth_token_refresh_response",
        serde_json::json!({
            "status": status.as_u16(),
            "body_length": body.len(),
            "response": crate::modules::codex_auth_diagnostic::oauth_response_summary(&token_response),
            "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&refreshed_tokens),
        }),
    );

    Ok(refreshed_tokens)
}

#[cfg(test)]
mod tests {
    use super::{
        authorize_url_matches_pending, build_auth_url, find_available_port, is_callback_navigation,
        is_id_token_expired, is_id_token_refresh_due, is_token_expired, parse_device_poll_interval,
        resolve_exchange_redirect_uri, resolve_refreshed_id_token, OAuthState,
        DEVICE_DEFAULT_POLL_SECONDS, DEVICE_EXCHANGE_REDIRECT_URI, ID_TOKEN_REFRESH_LEAD_SECONDS,
        OAUTH_CALLBACK_PORT, OAUTH_FALLBACK_CALLBACK_PORT, OAUTH_PORT_IN_USE_CODE, ORIGINATOR,
        TOKEN_REFRESH_SKEW_SECONDS,
    };
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::net::TcpListener;

    fn make_jwt(exp: i64) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::json!({ "exp": exp }).to_string());
        format!("{}.{}.sig", header, payload)
    }

    #[test]
    fn oauth_originator_matches_current_desktop_client() {
        assert_eq!(ORIGINATOR, "Codex Desktop");
    }

    /// 官方 codex 登录服务在 1455 被占用时回退 1457；两个端口都不可用时才报端口占用。
    #[test]
    fn oauth_callback_port_falls_back_when_primary_port_is_taken() {
        let Ok(occupied) = TcpListener::bind(("127.0.0.1", OAUTH_CALLBACK_PORT)) else {
            // 本机 1455 已被其它进程占用，说明前置条件不成立，跳过该用例。
            return;
        };
        let result = find_available_port();
        drop(occupied);

        match result {
            Ok(port) => assert_eq!(
                port, OAUTH_FALLBACK_CALLBACK_PORT,
                "1455 被占用时必须回退到 {}",
                OAUTH_FALLBACK_CALLBACK_PORT
            ),
            Err(error) => assert!(
                error.contains(OAUTH_PORT_IN_USE_CODE),
                "1455/1457 都不可用时必须返回端口占用错误，实际: {error}"
            ),
        }
    }

    #[test]
    fn browser_oauth_uses_official_hosted_url_without_app_server() {
        let auth_url = build_auth_url(
            "http://localhost:1455/auth/callback",
            "challenge-1",
            "state-1",
        );
        let hosted = url::Url::parse(&auth_url).expect("parse hosted auth url");
        assert_eq!(hosted.host_str(), Some("chatgpt.com"));
        assert_eq!(hosted.path(), "/codex/desktop-auth");

        let hosted_params = hosted
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            hosted_params
                .get("codex_streamlined_login")
                .map(|value| value.as_ref()),
            Some("true")
        );
        assert_eq!(
            hosted_params
                .get("no_universal_links")
                .map(|value| value.as_ref()),
            Some("1")
        );

        let authorize_url = hosted_params
            .get("authorize_url")
            .expect("hosted URL contains authorize_url");
        let authorize = url::Url::parse(authorize_url).expect("parse authorize URL");
        assert_eq!(authorize.host_str(), Some("auth.openai.com"));
        assert_eq!(authorize.path(), "/oauth/authorize");
        let params = authorize
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            params.get("redirect_uri").map(|value| value.as_ref()),
            Some("http://localhost:1455/auth/callback")
        );
        assert_eq!(
            params.get("code_challenge").map(|value| value.as_ref()),
            Some("challenge-1")
        );
        assert_eq!(
            params.get("state").map(|value| value.as_ref()),
            Some("state-1")
        );
        assert_eq!(
            params.get("originator").map(|value| value.as_ref()),
            Some("Codex Desktop")
        );
        assert!(params.contains_key("codex_app_version"));
        let source_id = params
            .get("source_surface_stable_id")
            .expect("source stable ID");
        assert_eq!(params.get("codex_origin_stable_id"), Some(source_id));
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

    #[tokio::test]
    async fn explicit_http_oauth_proxy_needs_no_node_engine() {
        let prepared = super::prepare_proxy("login-1".into(), "http://127.0.0.1:18080".into())
            .await
            .expect("direct proxy");
        assert_eq!(prepared.login_id, "login-1");
        assert_eq!(prepared.browser_url, prepared.request_url);
        assert!(prepared.browser_url.starts_with("http://127.0.0.1:18080"));
        assert!(prepared.tunnel.is_none());
    }

    #[tokio::test]
    async fn oauth_proxy_session_reuse_compares_normalized_addresses() {
        let prepared = super::prepare_proxy("login-1".into(), "http://127.0.0.1:18080".into())
            .await
            .expect("direct proxy");
        assert_eq!(prepared.raw_url, "http://127.0.0.1:18080/");
        for requested in ["http://127.0.0.1:18080", "http://127.0.0.1:18080/"] {
            assert!(super::oauth_proxy_matches_request(Some(requested), &prepared.raw_url));
        }
        for requested in [None, Some("http://127.0.0.1:18081"), Some("invalid")] {
            assert!(!super::oauth_proxy_matches_request(requested, &prepared.raw_url));
        }
    }

    fn reauth_proxy_account() -> crate::models::codex::CodexAccount {
        crate::models::codex::CodexAccount::new(
            "reauth-account".into(),
            "reauth@example.com".into(),
            crate::models::codex::CodexTokens {
                access_token: "access".into(),
                refresh_token: Some("refresh".into()),
                id_token: "id".into(),
            },
        )
    }

    /// 重新授权默认出口的优先级：账号独立绑定 > 统一代理 > 原有默认出口。
    #[test]
    fn reauth_proxy_priority_matches_account_proxy_policy() {
        use crate::modules::codex_account_proxy::{configured_url, has_effective_proxy};
        // 有独立绑定：直接使用账号代理，统一代理不会覆盖它。
        let mut bound = reauth_proxy_account();
        bound.egress_proxy_url = Some("socks5://127.0.0.1:1080".into());
        assert_eq!(
            configured_url(&bound).unwrap().as_deref(),
            Some("socks5://127.0.0.1:1080")
        );
        assert!(has_effective_proxy(&bound, true));
        // 无独立绑定：只有统一代理开启时才解析出统一出口。
        let unbound = reauth_proxy_account();
        assert!(has_effective_proxy(&unbound, true));
        // 两者都没有：保持原有默认授权路径，不借用其他账号的代理。
        assert!(!has_effective_proxy(&unbound, false));
        assert_eq!(crate::modules::codex_account_proxy::resolve_effective(&unbound, None), None);
    }

    /// 没有账号 ID（首次添加）或 ID 为空时不解析任何账号代理。
    #[tokio::test]
    async fn reauth_proxy_resolution_needs_a_real_account_id() {
        assert_eq!(super::resolve_reauth_egress_proxy("   ").await, Ok(None));
    }

    #[test]
    fn oauth_proxy_use_redacts_direct_proxy_credentials() {
        let proxy = super::oauth_proxy_use("account", "http://user:TOPSECRET@proxy.example:8080");
        assert_eq!(proxy.source, "account");
        assert_eq!(proxy.summary["protocol"], "HTTP");
        assert_eq!(proxy.summary["server"], "proxy.example");
        assert_eq!(proxy.summary["port"], 8080);
        assert!(!proxy.summary.to_string().contains("TOPSECRET"));
        // 直接地址可以回填输入框，用户随后可改。
        assert_eq!(
            proxy.input.as_deref(),
            Some("http://user:TOPSECRET@proxy.example:8080")
        );
    }

    #[test]
    fn oauth_proxy_use_keeps_resource_snapshots_backend_side() {
        let catalog = crate::modules::codex_proxy_subscription_parser::ParsedCatalog {
            nodes: vec![crate::modules::codex_proxy_subscription_parser::ParsedNode {
                id: "node-1".into(),
                name: "Alpha".into(),
                protocol: "HTTP".into(),
                outbound: Some(serde_json::json!({
                    "type": "http",
                    "server": "proxy.example",
                    "server_port": 8080,
                    "username": "user",
                    "password": "TOP_SECRET",
                })),
                error: None,
            }],
            groups: Vec::new(),
        };
        let snapshot = crate::modules::codex_proxy_catalog_binding::encode(
            "source-1",
            "Demo",
            "node-1",
            &catalog,
            &std::collections::BTreeMap::new(),
        )
        .expect("binding snapshot");
        let proxy = super::oauth_proxy_use("account", &snapshot);
        assert_eq!(proxy.summary["protocol"], "RESOURCE");
        assert_eq!(proxy.summary["name"], "Alpha");
        assert!(!proxy.summary.to_string().contains("TOP_SECRET"));
        // 资源快照可能很大且不是可编辑地址，只显示摘要，实际出口由后端按会话使用。
        assert!(proxy.input.is_none());
    }

    #[test]
    fn pending_oauth_state_records_only_proxy_requirement() {
        let mut state = pending("https://auth.openai.com/oauth/authorize");
        state.proxy_required = true;
        let serialized = serde_json::to_string(&state).expect("serialize");
        assert!(serialized.contains("\"proxy_required\":true"));
        assert!(!serialized.contains("proxy_url"));
        let legacy = serialized.replace(",\"proxy_required\":true", "");
        let restored: OAuthState = serde_json::from_str(&legacy).expect("legacy state");
        assert!(!restored.proxy_required);
    }

    fn pending(auth_url: &str) -> OAuthState {
        OAuthState {
            login_id: "login-1".to_string(),
            auth_url: auth_url.to_string(),
            redirect_uri: "http://localhost:1455/auth/callback".to_string(),
            code_verifier: "verifier".to_string(),
            state: "state-1".to_string(),
            port: 1455,
            expires_at: chrono::Utc::now().timestamp() + 60,
            code: None,
            device_auth_id: None,
            device_user_code: None,
            device_poll_interval_seconds: None,
            exchange_redirect_uri: None,
            proxy_required: false,
        }
    }

    #[test]
    fn incognito_window_accepts_only_current_hosted_authorize_url() {
        let url = "https://chatgpt.com/codex/desktop-auth?authorize_url=https%3A%2F%2Fauth.openai.com%2Foauth%2Fauthorize%3Fstate%3Dstate-1";
        let pending = pending(url);
        assert!(authorize_url_matches_pending(
            &url::Url::parse(url).unwrap(),
            &pending
        ));
        assert!(!authorize_url_matches_pending(
            &url::Url::parse("https://example.com/oauth/authorize?state=state-1").unwrap(),
            &pending
        ));
        assert!(!authorize_url_matches_pending(
            &url::Url::parse("https://auth.openai.com/oauth/authorize?state=other").unwrap(),
            &pending
        ));
    }

    #[test]
    fn incognito_window_allows_only_exact_local_callback_route() {
        assert!(is_callback_navigation(
            &url::Url::parse("http://localhost:1455/auth/callback?code=x&state=y").unwrap(),
            1455
        ));
        assert!(!is_callback_navigation(
            &url::Url::parse("http://localhost:1456/auth/callback?code=x&state=y").unwrap(),
            1455
        ));
        assert!(!is_callback_navigation(
            &url::Url::parse("http://localhost:1455/other").unwrap(),
            1455
        ));
    }

    #[test]
    fn opaque_access_token_is_not_locally_expired() {
        assert!(!is_token_expired("at-short-lived-opaque-token"));
    }

    #[test]
    fn unknown_non_jwt_access_token_is_expired() {
        assert!(is_token_expired("opaque-without-known-prefix"));
    }

    #[test]
    fn empty_access_token_is_expired() {
        assert!(is_token_expired("   "));
    }

    #[test]
    fn malformed_jwt_access_token_is_expired() {
        assert!(is_token_expired("not-valid.jwt.token"));
    }

    #[test]
    fn jwt_shaped_at_access_token_is_expired_when_malformed() {
        assert!(is_token_expired("at-not.valid.jwt"));
    }

    #[test]
    fn expired_jwt_access_token_is_expired() {
        let expired = chrono::Utc::now().timestamp() - 3600;
        assert!(is_token_expired(&make_jwt(expired)));
    }

    #[test]
    fn fresh_jwt_access_token_is_not_expired() {
        let fresh = chrono::Utc::now().timestamp() + TOKEN_REFRESH_SKEW_SECONDS + 3600;
        assert!(!is_token_expired(&make_jwt(fresh)));
    }

    #[test]
    fn expired_id_token_is_expired() {
        let expired = chrono::Utc::now().timestamp() - 3600;
        assert!(is_id_token_expired(&make_jwt(expired)));
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
    fn refreshed_id_token_prefers_fresh_response_value() {
        let fresh = make_jwt(chrono::Utc::now().timestamp() + TOKEN_REFRESH_SKEW_SECONDS + 3600);
        let old = make_jwt(chrono::Utc::now().timestamp() - 3600);
        let response = serde_json::json!({ "id_token": fresh });

        assert_eq!(
            resolve_refreshed_id_token(&response, Some(&old)).expect("resolve response id_token"),
            response["id_token"].as_str().unwrap()
        );
    }

    #[test]
    fn refreshed_id_token_reuses_only_fresh_current_value() {
        let fresh = make_jwt(chrono::Utc::now().timestamp() + TOKEN_REFRESH_SKEW_SECONDS + 3600);
        let response = serde_json::json!({});

        assert_eq!(
            resolve_refreshed_id_token(&response, Some(&fresh)).expect("reuse fresh id_token"),
            fresh
        );
    }

    #[test]
    fn refreshed_id_token_keeps_current_value_when_response_omits_it() {
        let expired = make_jwt(chrono::Utc::now().timestamp() - 3600);
        let resolved = resolve_refreshed_id_token(&serde_json::json!({}), Some(&expired))
            .expect("current id_token can be retained until runtime validation");

        assert_eq!(resolved, expired);
    }

    #[test]
    fn refreshed_id_token_omits_expired_response_value_without_losing_rotated_chain() {
        let expired = make_jwt(chrono::Utc::now().timestamp() - 3600);
        let response = serde_json::json!({ "id_token": expired });

        assert_eq!(
            resolve_refreshed_id_token(&response, None)
                .expect("access refresh result remains persistable"),
            ""
        );
    }
}
