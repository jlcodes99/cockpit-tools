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
use tauri::{AppHandle, Emitter};
use url::Url;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const SCOPES: &str = "openid profile email offline_access";
const ORIGINATOR: &str = "codex_vscode";
const OAUTH_CALLBACK_PORT: u16 = 1455;
const OAUTH_PORT_IN_USE_CODE: &str = "CODEX_OAUTH_PORT_IN_USE";
const OAUTH_STATE_FILE: &str = "codex_oauth_pending.json";
const OAUTH_TIMEOUT_SECONDS: i64 = 300;
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 300;

pub fn get_callback_port() -> u16 {
    OAUTH_CALLBACK_PORT
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOAuthLoginStartResponse {
    pub login_id: String,
    pub auth_url: String,
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
) -> Result<CodexTokens, String> {
    let redirect_uri = format!("http://localhost:{}/auth/callback", port);
    let client = reqwest::Client::new();

    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &redirect_uri),
        ("client_id", CLIENT_ID),
        ("code_verifier", code_verifier),
    ];

    logger::log_info("Codex OAuth 开始交换 Token");

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

pub async fn complete_oauth_login(login_id: &str) -> Result<CodexTokens, String> {
    hydrate_oauth_state_if_missing();
    let attempt_id = COMPLETE_ATTEMPT_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let started_at_ms = chrono::Utc::now().timestamp_millis();
    logger::log_info(&format!(
        "Codex OAuth 开始完成登录: attempt_id={}, login_id={}, started_at_ms={}",
        attempt_id, login_id, started_at_ms
    ));
    let (code, code_verifier, port) = {
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
        (code, state.code_verifier.clone(), state.port)
    };

    let tokens = match exchange_code_for_token_internal(&code, &code_verifier, port).await {
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

    notify_cancel(port);
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

pub fn is_token_expired(access_token: &str) -> bool {
    let parts: Vec<&str> = access_token.split('.').collect();
    if parts.len() != 3 {
        return true;
    }

    let payload_base64 = parts[1];
    let payload_bytes = match URL_SAFE_NO_PAD.decode(payload_base64) {
        Ok(bytes) => bytes,
        Err(_) => return true,
    };

    let payload_str = match String::from_utf8(payload_bytes) {
        Ok(s) => s,
        Err(_) => return true,
    };

    let payload: serde_json::Value = match serde_json::from_str(&payload_str) {
        Ok(v) => v,
        Err(_) => return true,
    };

    let exp = match payload.get("exp").and_then(|e| e.as_i64()) {
        Some(e) => e,
        None => return true,
    };

    let now = chrono::Utc::now().timestamp();
    exp < now + TOKEN_REFRESH_SKEW_SECONDS
}

pub async fn refresh_access_token(refresh_token: &str) -> Result<CodexTokens, String> {
    refresh_access_token_with_fallback(refresh_token, None).await
}

pub async fn refresh_access_token_with_fallback(
    refresh_token: &str,
    current_id_token: Option<&str>,
) -> Result<CodexTokens, String> {
    let client = reqwest::Client::new();

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", CLIENT_ID),
    ];

    logger::log_info("Codex Token 刷新中...");

    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token 刷新请求失败: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    if !status.is_success() {
        logger::log_error(&format!(
            "Token 刷新失败: status={}, body_len={}",
            status,
            body.len()
        ));
        return Err(format!(
            "Token 刷新失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    logger::log_info("Codex Token 刷新成功");

    let token_response: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("解析 Token 响应失败: {}", e))?;

    let id_token = token_response
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(|value| value.to_string())
        .or_else(|| {
            current_id_token
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
        })
        .ok_or("响应中缺少 id_token，且本地没有可复用的旧值")?;

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
