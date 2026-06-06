use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use rand::rngs::OsRng;
use rsa::pkcs1::{DecodeRsaPrivateKey, EncodeRsaPrivateKey, EncodeRsaPublicKey};
use rsa::{Oaep, Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use url::Url;
use uuid::Uuid;

use crate::models::zed::{ZedAccount, ZedOAuthStartResponse};
use crate::modules::{logger, oauth_pending_state, zed_account};

const OAUTH_PENDING_FILE: &str = "zed_oauth_pending.json";
const OAUTH_TIMEOUT_SECONDS: i64 = 600;
const OAUTH_POLL_INTERVAL_MS: u64 = 1000;
const CALLBACK_HOST: &str = "127.0.0.1";
const ZED_NATIVE_SIGNIN_URL: &str = "https://zed.dev/native_app_signin";

static PENDING_OAUTH_STATE: std::sync::LazyLock<Arc<Mutex<Option<PendingOAuthState>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(None)));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOAuthState {
    login_id: String,
    verification_uri: String,
    callback_url: String,
    port: u16,
    public_key_b64: String,
    private_key_der_b64: String,
    expires_at: i64,
    cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn generate_login_id() -> String {
    format!("zed_{}", Uuid::new_v4().simple())
}

fn pending_state_to_response(state: &PendingOAuthState) -> ZedOAuthStartResponse {
    ZedOAuthStartResponse {
        login_id: state.login_id.clone(),
        verification_uri: state.verification_uri.clone(),
        expires_in: OAUTH_TIMEOUT_SECONDS as u64,
        interval_seconds: OAUTH_POLL_INTERVAL_MS / 1000 + 1,
        callback_url: Some(state.callback_url.clone()),
    }
}

fn load_pending_state_from_disk() -> Result<Option<PendingOAuthState>, String> {
    oauth_pending_state::load(OAUTH_PENDING_FILE)
}

fn persist_pending_state(state: Option<&PendingOAuthState>) -> Result<(), String> {
    match state {
        Some(value) => oauth_pending_state::save(OAUTH_PENDING_FILE, value),
        None => oauth_pending_state::clear(OAUTH_PENDING_FILE),
    }
}

fn replace_pending_state(next: Option<PendingOAuthState>) -> Result<(), String> {
    let mut guard = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "获取 Zed OAuth 状态锁失败".to_string())?;
    *guard = next.clone();
    persist_pending_state(next.as_ref())
}

fn active_pending_state() -> Option<PendingOAuthState> {
    let Ok(guard) = PENDING_OAUTH_STATE.lock() else {
        return None;
    };
    guard.clone()
}

fn html_success() -> &'static str {
    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<html><body style='font-family:sans-serif;background:#0f172a;color:#e2e8f0;padding:32px;text-align:center;'>\
<h2 style='color:#22c55e;'>Zed 登录已完成</h2>\
<p>可以关闭此窗口并返回 Cockpit Tools。</p>\
<script>setTimeout(function(){ window.close(); }, 1200);</script>\
</body></html>"
}

fn decode_url_safe_base64(value: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(value.as_bytes())
        .or_else(|_| URL_SAFE.decode(value.as_bytes()))
        .map_err(|e| format!("解析 Zed OAuth Base64 失败: {}", e))
}

fn build_key_pair() -> Result<(String, String), String> {
    let private_key = RsaPrivateKey::new(&mut OsRng, 2048)
        .map_err(|e| format!("生成 Zed RSA 私钥失败: {}", e))?;
    let public_key = RsaPublicKey::from(&private_key);

    let private_der = private_key
        .to_pkcs1_der()
        .map_err(|e| format!("编码 Zed RSA 私钥失败: {}", e))?;
    let public_der = public_key
        .to_pkcs1_der()
        .map_err(|e| format!("编码 Zed RSA 公钥失败: {}", e))?;

    Ok((
        URL_SAFE_NO_PAD.encode(private_der.as_bytes()),
        URL_SAFE_NO_PAD.encode(public_der.as_bytes()),
    ))
}

fn decrypt_access_token(
    private_key_der_b64: &str,
    encrypted_token: &str,
) -> Result<String, String> {
    let private_key_der = decode_url_safe_base64(private_key_der_b64)?;
    let private_key = RsaPrivateKey::from_pkcs1_der(&private_key_der)
        .map_err(|e| format!("解析 Zed RSA 私钥失败: {}", e))?;
    let encrypted = decode_url_safe_base64(encrypted_token)?;

    let decrypted = private_key
        .decrypt(Oaep::new::<Sha256>(), &encrypted)
        .or_else(|_| private_key.decrypt(Pkcs1v15Encrypt, &encrypted))
        .map_err(|e| format!("解密 Zed access_token 失败: {}", e))?;

    String::from_utf8(decrypted).map_err(|e| format!("Zed access_token 不是有效 UTF-8: {}", e))
}

fn read_http_request(stream: &mut TcpStream) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("设置 Zed OAuth 回调读取超时失败: {}", e))?;

    let mut buffer = [0u8; 8192];
    let read = stream
        .read(&mut buffer)
        .map_err(|e| format!("读取 Zed OAuth 回调失败: {}", e))?;
    if read == 0 {
        return Err("Zed OAuth 回调内容为空".to_string());
    }

    Ok(String::from_utf8_lossy(&buffer[..read]).into_owned())
}

fn parse_query_from_request(request: &str, port: u16) -> Result<CallbackQuery, String> {
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| "Zed OAuth 回调缺少请求行".to_string())?;
    let mut parts = request_line.split_whitespace();
    let _method = parts
        .next()
        .ok_or_else(|| "Zed OAuth 回调缺少 method".to_string())?;
    let target = parts
        .next()
        .ok_or_else(|| "Zed OAuth 回调缺少 target".to_string())?;
    let url = if target.starts_with("http://") || target.starts_with("https://") {
        target.to_string()
    } else {
        format!("http://{}:{}{}", CALLBACK_HOST, port, target)
    };
    let parsed =
        url::Url::parse(&url).map_err(|e| format!("解析 Zed OAuth 回调 URL 失败: {}", e))?;
    let query = parsed.query().unwrap_or_default();
    serde_urlencoded::from_str::<CallbackQuery>(query)
        .map_err(|e| format!("解析 Zed OAuth 回调参数失败: {}", e))
}

fn parse_callback_url(callback_url: &str, port: u16) -> Result<Url, String> {
    let trimmed = callback_url.trim();
    if trimmed.is_empty() {
        return Err("回调地址不能为空".to_string());
    }

    let normalized = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.starts_with('/') {
        format!("http://{}:{}{}", CALLBACK_HOST, port, trimmed)
    } else {
        format!("http://{}:{}/{}", CALLBACK_HOST, port, trimmed)
    };

    let parsed =
        Url::parse(&normalized).map_err(|e| format!("解析 Zed OAuth 回调 URL 失败: {}", e))?;
    if !parsed.has_host() {
        return Err("回调地址缺少主机".to_string());
    }
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("回调地址协议无效".to_string());
    }
    let parsed_port = parsed
        .port_or_known_default()
        .ok_or_else(|| "回调地址缺少端口".to_string())?;
    if parsed_port != port {
        return Err(format!("回调地址端口不匹配，期望 {}", port));
    }

    let host = parsed.host_str().unwrap_or_default();
    if host != CALLBACK_HOST && host != "localhost" {
        return Err("回调地址主机必须为 127.0.0.1 或 localhost".to_string());
    }

    Ok(parsed)
}

fn validate_callback_query(query: &CallbackQuery) -> Result<(), String> {
    if let Some(error) = query
        .error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let description = query
            .error_description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
        if description.is_empty() {
            return Err(format!("授权失败: {}", error));
        }
        return Err(format!("授权失败: {} ({})", error, description));
    }
    Ok(())
}

fn write_response(stream: &mut TcpStream, payload: &str) {
    let _ = stream.write_all(payload.as_bytes());
    let _ = stream.flush();
}

fn update_completed_credentials(
    login_id: &str,
    user_id: String,
    access_token: String,
) -> Result<(), String> {
    let mut guard = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "获取 Zed OAuth 状态锁失败".to_string())?;
    let Some(state) = guard.as_mut() else {
        return Err("Zed OAuth 状态不存在".to_string());
    };
    if state.login_id != login_id {
        return Err("Zed OAuth login_id 不匹配".to_string());
    }

    state.completed_user_id = Some(user_id);
    state.completed_access_token = Some(access_token);
    persist_pending_state(guard.as_ref())
}

fn handle_callback_stream(
    mut stream: TcpStream,
    login_id: &str,
    port: u16,
    private_key_der_b64: &str,
) -> Result<(), String> {
    let request = read_http_request(&mut stream)?;
    let query = parse_query_from_request(&request, port)?;
    validate_callback_query(&query)?;

    let user_id = query
        .user_id
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .ok_or_else(|| "Zed OAuth 回调缺少 user_id".to_string())?;
    let encrypted_token = query
        .access_token
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .ok_or_else(|| "Zed OAuth 回调缺少 access_token".to_string())?;

    let access_token = decrypt_access_token(private_key_der_b64, &encrypted_token)?;
    update_completed_credentials(login_id, user_id, access_token)?;
    write_response(&mut stream, html_success());
    Ok(())
}

fn spawn_listener(listener: TcpListener, login_id: String, port: u16, private_key_der_b64: String) {
    thread::spawn(move || {
        if let Err(err) = listener.set_nonblocking(true) {
            logger::log_warn(&format!(
                "[Zed OAuth] 设置本地监听为 nonblocking 失败: login_id={}, err={}",
                login_id, err
            ));
            return;
        }

        loop {
            let Some(state) = active_pending_state() else {
                break;
            };
            if state.login_id != login_id {
                break;
            }
            if state.cancelled || now_timestamp() > state.expires_at {
                break;
            }
            if state.completed_user_id.is_some() && state.completed_access_token.is_some() {
                break;
            }

            match listener.accept() {
                Ok((stream, _addr)) => {
                    match handle_callback_stream(stream, &login_id, port, &private_key_der_b64) {
                        Ok(()) => logger::log_info(&format!(
                            "[Zed OAuth] 已接收浏览器回调: login_id={}",
                            login_id
                        )),
                        Err(err) => logger::log_warn(&format!(
                            "[Zed OAuth] 处理浏览器回调失败: login_id={}, err={}",
                            login_id, err
                        )),
                    }
                    break;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(200));
                }
                Err(err) => {
                    logger::log_warn(&format!(
                        "[Zed OAuth] 本地监听 accept 失败: login_id={}, err={}",
                        login_id, err
                    ));
                    break;
                }
            }
        }
    });
}

fn bind_listener(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind((CALLBACK_HOST, port))
        .map_err(|e| format!("绑定 Zed OAuth 本地端口失败 ({}): {}", port, e))
}

fn build_pending_state(listener: &TcpListener) -> Result<PendingOAuthState, String> {
    let port = listener
        .local_addr()
        .map_err(|e| format!("获取 Zed OAuth 本地端口失败: {}", e))?
        .port();
    let login_id = generate_login_id();
    let (private_key_der_b64, public_key_b64) = build_key_pair()?;
    let verification_uri = format!(
        "{}?native_app_port={}&native_app_public_key={}",
        ZED_NATIVE_SIGNIN_URL, port, public_key_b64
    );
    let callback_url = format!("http://{}:{}/", CALLBACK_HOST, port);

    Ok(PendingOAuthState {
        login_id,
        verification_uri,
        callback_url,
        port,
        public_key_b64,
        private_key_der_b64,
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS,
        cancelled: false,
        completed_user_id: None,
        completed_access_token: None,
    })
}

pub async fn start_login() -> Result<ZedOAuthStartResponse, String> {
    let listener = bind_listener(0)?;
    let state = build_pending_state(&listener)?;
    replace_pending_state(Some(state.clone()))?;

    spawn_listener(
        listener,
        state.login_id.clone(),
        state.port,
        state.private_key_der_b64.clone(),
    );

    logger::log_info(&format!(
        "[Zed OAuth] 登录已启动: login_id={}, port={}",
        state.login_id, state.port
    ));
    Ok(pending_state_to_response(&state))
}

pub fn peek_pending_login() -> Option<ZedOAuthStartResponse> {
    let state = active_pending_state().or_else(|| load_pending_state_from_disk().ok().flatten())?;
    if state.cancelled || now_timestamp() > state.expires_at {
        let _ = replace_pending_state(None);
        return None;
    }
    let _ = replace_pending_state(Some(state.clone()));
    Some(pending_state_to_response(&state))
}

pub async fn complete_login(login_id: &str) -> Result<ZedAccount, String> {
    loop {
        let state =
            active_pending_state().or_else(|| load_pending_state_from_disk().ok().flatten());
        let Some(state) = state else {
            return Err("没有待处理的 Zed 登录请求".to_string());
        };

        if state.login_id != login_id {
            return Err("Zed OAuth login_id 不匹配".to_string());
        }
        if state.cancelled {
            let _ = replace_pending_state(None);
            return Err("Zed 登录已取消".to_string());
        }
        if now_timestamp() > state.expires_at {
            let _ = replace_pending_state(None);
            return Err("Zed 登录超时".to_string());
        }

        if let (Some(user_id), Some(access_token)) = (
            state.completed_user_id.clone(),
            state.completed_access_token.clone(),
        ) {
            match zed_account::upsert_account_from_credentials(&user_id, &access_token).await {
                Ok(account) => {
                    let _ = replace_pending_state(None);
                    return Ok(account);
                }
                Err(err) => return Err(err),
            }
        }

        tokio::time::sleep(Duration::from_millis(OAUTH_POLL_INTERVAL_MS)).await;
    }
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    let pending = active_pending_state().or_else(|| load_pending_state_from_disk().ok().flatten());
    let Some(state) = pending else {
        return Ok(());
    };

    if let Some(expected) = login_id {
        if state.login_id != expected {
            return Err("Zed OAuth login_id 不匹配".to_string());
        }
    }

    replace_pending_state(None)?;
    logger::log_info(&format!(
        "[Zed OAuth] 登录已取消: login_id={}",
        state.login_id
    ));
    Ok(())
}

pub fn submit_callback_url(login_id: &str, callback_url: &str) -> Result<(), String> {
    let state = active_pending_state().or_else(|| load_pending_state_from_disk().ok().flatten());
    let Some(state) = state else {
        return Err("登录流程已取消，请重新发起授权".to_string());
    };
    if state.login_id != login_id {
        return Err("Zed OAuth login_id 不匹配".to_string());
    }
    if state.cancelled {
        return Err("Zed 登录已取消".to_string());
    }
    if now_timestamp() > state.expires_at {
        return Err("Zed 登录超时".to_string());
    }

    let parsed = parse_callback_url(callback_url, state.port)?;
    let query = serde_urlencoded::from_str::<CallbackQuery>(parsed.query().unwrap_or_default())
        .map_err(|e| format!("解析 Zed OAuth 回调参数失败: {}", e))?;
    validate_callback_query(&query)?;

    let user_id = query
        .user_id
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .ok_or_else(|| "回调链接中缺少 user_id 参数".to_string())?;
    let encrypted_token = query
        .access_token
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .ok_or_else(|| "回调链接中缺少 access_token 参数".to_string())?;

    let access_token = decrypt_access_token(&state.private_key_der_b64, &encrypted_token)?;
    update_completed_credentials(login_id, user_id, access_token)?;
    logger::log_info(&format!(
        "[Zed OAuth] 已接收手动回调链接: login_id={}",
        login_id
    ));
    Ok(())
}

pub fn restore_pending_oauth_listener() {
    let restored = match load_pending_state_from_disk() {
        Ok(state) => state,
        Err(err) => {
            logger::log_warn(&format!("[Zed OAuth] 恢复 pending 状态失败: {}", err));
            return;
        }
    };
    let Some(state) = restored else {
        return;
    };

    if state.cancelled || now_timestamp() > state.expires_at {
        let _ = replace_pending_state(None);
        return;
    }

    if let Err(err) = replace_pending_state(Some(state.clone())) {
        logger::log_warn(&format!("[Zed OAuth] 恢复内存状态失败: {}", err));
        return;
    }

    if state.completed_user_id.is_some() && state.completed_access_token.is_some() {
        logger::log_info(&format!(
            "[Zed OAuth] 已恢复已完成但未消费的登录状态: login_id={}",
            state.login_id
        ));
        return;
    }

    match bind_listener(state.port) {
        Ok(listener) => {
            spawn_listener(
                listener,
                state.login_id.clone(),
                state.port,
                state.private_key_der_b64.clone(),
            );
            logger::log_info(&format!(
                "[Zed OAuth] 已恢复本地监听: login_id={}, port={}",
                state.login_id, state.port
            ));
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Zed OAuth] 恢复本地监听失败: login_id={}, port={}, err={}",
                state.login_id, state.port, err
            ));
        }
    }
}
