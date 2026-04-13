use base64::Engine;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use url::Url;

use crate::models::windsurf::{
    WindsurfAccount, WindsurfOAuthCompletePayload, WindsurfOAuthStartResponse,
};
use crate::modules::logger;

const WINDSURF_AUTH_BASE_URL: &str = "https://www.windsurf.com";
const WINDSURF_REGISTER_API_BASE_URL: &str = "https://register.windsurf.com";
const WINDSURF_DEFAULT_API_SERVER_URL: &str = "https://server.codeium.com";
const WINDSURF_CLIENT_ID: &str = "3GUryQ7ldAeKEuD2obYnppsnmj58eP5u";
const APP_USER_AGENT: &str = "antigravity-cockpit-tools";
const OAUTH_TIMEOUT_SECONDS: u64 = 600;
const OAUTH_STATE_FILE: &str = "windsurf_oauth_pending.json";
const FIREBASE_API_KEY: &str = "AIzaSyDsOl-1XpT5err0Tcnx8FFod1H8gVGIycY";
const FIREBASE_SIGN_IN_URL: &str =
    "https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword";

#[derive(Clone, Serialize, Deserialize)]
struct PendingOAuthState {
    login_id: String,
    state: String,
    auth_url: String,
    callback_url: String,
    port: u16,
    created_at: i64,
    expires_at: i64,
    access_token: Option<String>,
    callback_error: Option<String>,
}

#[derive(Debug, Clone)]
struct RegisterResult {
    api_key: String,
    api_server_url: String,
    name: Option<String>,
}

lazy_static::lazy_static! {
    static ref PENDING_OAUTH_STATE: Arc<Mutex<Option<PendingOAuthState>>> = Arc::new(Mutex::new(None));
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn load_pending_login_from_disk() -> Option<PendingOAuthState> {
    match crate::modules::oauth_pending_state::load::<PendingOAuthState>(OAUTH_STATE_FILE) {
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
                "[Windsurf OAuth] 读取持久化登录状态失败，已忽略: {}",
                err
            ));
            let _ = crate::modules::oauth_pending_state::clear(OAUTH_STATE_FILE);
            None
        }
    }
}

fn persist_pending_login(state: Option<&PendingOAuthState>) {
    let result = match state {
        Some(value) => crate::modules::oauth_pending_state::save(OAUTH_STATE_FILE, value),
        None => crate::modules::oauth_pending_state::clear(OAUTH_STATE_FILE),
    };
    if let Err(err) = result {
        logger::log_warn(&format!(
            "[Windsurf OAuth] 持久化登录状态失败，已忽略: {}",
            err
        ));
    }
}

fn hydrate_pending_login_if_missing() {
    if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
        if guard.is_none() {
            *guard = load_pending_login_from_disk();
        }
    }
}

fn set_pending_login(state: Option<PendingOAuthState>) {
    if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
        *guard = state.clone();
    }
    persist_pending_login(state.as_ref());
}

fn ensure_callback_server_for_state(state: &PendingOAuthState) {
    if state.expires_at <= now_timestamp() {
        clear_pending_if_matches(&state.login_id, &state.state);
        return;
    }
    if state.access_token.is_some() || state.callback_error.is_some() {
        return;
    }

    match TcpListener::bind(("127.0.0.1", state.port)) {
        Ok(listener) => {
            drop(listener);
            let callback_login_id = state.login_id.clone();
            let callback_state = state.state.clone();
            let callback_port = state.port;
            tokio::spawn(async move {
                if let Err(e) =
                    start_callback_server(callback_port, callback_login_id.clone(), callback_state)
                        .await
                {
                    logger::log_error(&format!(
                        "[Windsurf OAuth] 回调服务恢复失败: login_id={}, error={}",
                        callback_login_id, e
                    ));
                }
            });
            logger::log_info(&format!(
                "[Windsurf OAuth] 已恢复本地回调服务: login_id={}, port={}",
                state.login_id, state.port
            ));
        }
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            logger::log_info(&format!(
                "[Windsurf OAuth] 本地回调端口已占用，视为监听中: login_id={}, port={}",
                state.login_id, state.port
            ));
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Windsurf OAuth] 本地回调恢复失败: login_id={}, port={}, error={}",
                state.login_id, state.port, err
            ));
        }
    }
}

fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..24).map(|_| rng.gen::<u8>()).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
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
            let value = parts.next().unwrap_or("");
            Some((key.to_string(), decode_query_component(value)))
        })
        .collect()
}

fn parse_callback_url(raw_callback_url: &str, port: u16) -> Result<Url, String> {
    let trimmed = raw_callback_url.trim();
    if trimmed.is_empty() {
        return Err("回调链接不能为空".to_string());
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Url::parse(trimmed).map_err(|e| format!("回调链接格式无效: {}", e));
    }

    if trimmed.starts_with('/') {
        return Url::parse(format!("http://127.0.0.1:{}{}", port, trimmed).as_str())
            .map_err(|e| format!("回调链接格式无效: {}", e));
    }

    Url::parse(
        format!(
            "http://127.0.0.1:{}/windsurf-auth-callback?{}",
            port,
            trimmed.trim_start_matches('?')
        )
        .as_str(),
    )
    .map_err(|e| format!("回调链接格式无效: {}", e))
}

fn pick_string_from_object(obj: Option<&Value>, keys: &[&str]) -> Option<String> {
    let Some(obj) = obj.and_then(Value::as_object) else {
        return None;
    };

    for key in keys {
        if let Some(value) = obj.get(*key) {
            match value {
                Value::String(text) if !text.trim().is_empty() => {
                    return Some(text.trim().to_string())
                }
                Value::Number(num) => return Some(num.to_string()),
                _ => {}
            }
        }
    }
    None
}

fn pick_i64_from_object(obj: Option<&Value>, keys: &[&str]) -> Option<i64> {
    let Some(obj) = obj.and_then(Value::as_object) else {
        return None;
    };

    for key in keys {
        if let Some(value) = obj.get(*key) {
            if let Some(v) = value.as_i64() {
                return Some(v);
            }
            if let Some(v) = value.as_u64() {
                if v <= i64::MAX as u64 {
                    return Some(v as i64);
                }
            }
            if let Some(v) = value.as_str().and_then(|s| s.parse::<i64>().ok()) {
                return Some(v);
            }
        }
    }
    None
}

fn normalize_non_empty(input: Option<String>) -> Option<String> {
    input.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn resolve_api_server_url(
    auth_status_raw: Option<&Value>,
    api_server_url_hint: Option<&str>,
) -> String {
    normalize_non_empty(api_server_url_hint.map(|value| value.to_string()))
        .or_else(|| pick_string_from_object(auth_status_raw, &["apiServerUrl", "api_server_url"]))
        .unwrap_or_else(|| WINDSURF_DEFAULT_API_SERVER_URL.to_string())
}

fn parse_proto_timestamp_seconds(value: Option<&Value>) -> Option<i64> {
    let Some(value) = value else {
        return None;
    };
    if let Some(seconds) = value.as_i64() {
        return Some(seconds);
    }
    if let Some(seconds) = value.as_u64() {
        if seconds <= i64::MAX as u64 {
            return Some(seconds as i64);
        }
    }
    let obj = value.as_object()?;
    if let Some(seconds) = obj.get("seconds") {
        if let Some(v) = seconds.as_i64() {
            return Some(v);
        }
        if let Some(v) = seconds.as_u64() {
            if v <= i64::MAX as u64 {
                return Some(v as i64);
            }
        }
        if let Some(v) = seconds.as_str().and_then(|s| s.parse::<i64>().ok()) {
            return Some(v);
        }
    }
    None
}

fn sanitize_login(raw: &str) -> String {
    let mut text = raw.trim().to_lowercase();
    if text.is_empty() {
        return "windsurf_user".to_string();
    }
    text = text
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    text.trim_matches('_').to_string()
}

fn hash_to_u64(value: &str) -> u64 {
    let digest = md5::compute(value);
    let bytes = digest.0;
    let mut out: u64 = 0;
    for byte in bytes.iter().take(8) {
        out = (out << 8) | (*byte as u64);
    }
    out
}

fn build_synthetic_copilot_token(
    total_prompt: Option<i64>,
    total_flow: Option<i64>,
    reset_at: Option<i64>,
    plan_name: Option<&str>,
) -> String {
    let total_prompt = total_prompt.unwrap_or(0).max(0);
    let total_flow = total_flow.unwrap_or(0).max(0);
    let reset_at = reset_at.unwrap_or(0).max(0);
    let sku = plan_name
        .unwrap_or("windsurf")
        .trim()
        .to_lowercase()
        .replace(' ', "_");

    format!(
        "cq={};tq={};rd={}:0;sku={};source=windsurf",
        total_prompt, total_flow, reset_at, sku
    )
}

fn build_auth_url(redirect_uri: &str, state: &str) -> String {
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params.append_pair("response_type", "token");
    params.append_pair("client_id", WINDSURF_CLIENT_ID);
    params.append_pair("redirect_uri", redirect_uri);
    params.append_pair("state", state);
    params.append_pair("prompt", "login");
    params.append_pair("redirect_parameters_type", "query");
    params.append_pair("workflow", "onboarding");
    format!(
        "{}/windsurf/signin?{}",
        WINDSURF_AUTH_BASE_URL,
        params.finish()
    )
}

fn to_start_response(state: &PendingOAuthState) -> WindsurfOAuthStartResponse {
    let expires_in = (state.expires_at - now_timestamp()).max(0) as u64;
    WindsurfOAuthStartResponse {
        login_id: state.login_id.clone(),
        user_code: String::new(),
        verification_uri: state.auth_url.clone(),
        verification_uri_complete: Some(state.auth_url.clone()),
        expires_in,
        interval_seconds: 1,
        callback_url: Some(state.callback_url.clone()),
    }
}

fn clear_pending_if_matches(expected_login_id: &str, expected_state: &str) {
    let should_clear = if let Ok(guard) = PENDING_OAUTH_STATE.lock() {
        guard
            .as_ref()
            .map(|current| current.login_id == expected_login_id && current.state == expected_state)
            .unwrap_or(false)
    } else {
        false
    };
    if should_clear {
        set_pending_login(None);
    }
}

fn find_available_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("无法绑定本地 OAuth 回调端口: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取本地 OAuth 回调端口失败: {}", e))?
        .port();
    drop(listener);
    Ok(port)
}

fn notify_cancel(port: u16) {
    if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
        let _ = std::io::Write::write_all(
            &mut stream,
            b"GET /cancel HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        );
        let _ = std::io::Write::flush(&mut stream);
    }
}

fn oauth_success_html() -> &'static str {
    r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8" />
  <title>Windsurf 授权成功</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; display:flex; justify-content:center; align-items:center; height:100vh; margin:0; background:#0f172a; color:#e2e8f0; }
    .box { text-align:center; max-width:460px; padding:24px; border-radius:12px; background:#111827; border:1px solid #1f2937; }
    h1 { margin:0 0 10px; color:#22c55e; font-size:24px; }
    p { margin:0; opacity:.92; }
  </style>
</head>
<body>
  <div class="box">
    <h1>授权成功</h1>
    <p>你可以关闭此页面并返回 Antigravity Cockpit Tools。</p>
  </div>
</body>
</html>"#
}

fn oauth_fail_html(message: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8" />
  <title>Windsurf 授权失败</title>
  <style>
    body {{ font-family: -apple-system, BlinkMacSystemFont, sans-serif; display:flex; justify-content:center; align-items:center; height:100vh; margin:0; background:#0f172a; color:#e2e8f0; }}
    .box {{ text-align:center; max-width:520px; padding:24px; border-radius:12px; background:#111827; border:1px solid #1f2937; }}
    h1 {{ margin:0 0 10px; color:#ef4444; font-size:24px; }}
    p {{ margin:0; opacity:.92; word-break: break-word; }}
  </style>
</head>
<body>
  <div class="box">
    <h1>授权失败</h1>
    <p>{}</p>
  </div>
</body>
</html>"#,
        message
    )
}

async fn start_callback_server(
    port: u16,
    expected_login_id: String,
    expected_state: String,
) -> Result<(), String> {
    use tiny_http::{Header, Response, Server};

    let server = Server::http(format!("127.0.0.1:{}", port))
        .map_err(|e| format!("启动 Windsurf OAuth 本地回调服务失败: {}", e))?;
    let started = std::time::Instant::now();

    logger::log_info(&format!(
        "[Windsurf OAuth] 本地回调服务启动: login_id={}, port={}",
        expected_login_id, port
    ));

    loop {
        let should_stop = {
            let guard = PENDING_OAUTH_STATE
                .lock()
                .map_err(|_| "OAuth 状态锁不可用".to_string())?;
            match guard.as_ref() {
                Some(state) => state.login_id != expected_login_id || state.state != expected_state,
                None => true,
            }
        };

        if should_stop {
            break;
        }

        if started.elapsed().as_secs() > OAUTH_TIMEOUT_SECONDS {
            clear_pending_if_matches(&expected_login_id, &expected_state);
            logger::log_warn(&format!(
                "[Windsurf OAuth] 回调等待超时: login_id={}",
                expected_login_id
            ));
            break;
        }

        if let Ok(Some(request)) = server.try_recv() {
            let url = request.url().to_string();
            if url.starts_with("/cancel") {
                let _ = request.respond(Response::from_string("cancelled").with_status_code(200));
                clear_pending_if_matches(&expected_login_id, &expected_state);
                break;
            }

            if !url.starts_with("/windsurf-auth-callback") {
                let _ = request.respond(Response::from_string("Not Found").with_status_code(404));
                continue;
            }

            let query = url.split('?').nth(1).unwrap_or("");
            let params = parse_query_params(query);
            let state = params.get("state").cloned().unwrap_or_default();
            let access_token = params.get("access_token").cloned().unwrap_or_default();
            let error = params.get("error").cloned();
            let error_desc = params
                .get("error_description")
                .cloned()
                .unwrap_or_else(String::new);

            if state != expected_state {
                let html = oauth_fail_html("state 校验失败，请重新授权。");
                let _ = request.respond(
                    Response::from_string(html)
                        .with_status_code(400)
                        .with_header(
                            Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"text/html; charset=utf-8"[..],
                            )
                            .unwrap(),
                        ),
                );
                continue;
            }

            if let Some(error) = error {
                let message = if error_desc.is_empty() {
                    format!("授权失败: {}", error)
                } else {
                    format!("授权失败: {} ({})", error, error_desc)
                };
                if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
                    if let Some(current) = guard.as_mut() {
                        if current.login_id == expected_login_id && current.state == expected_state
                        {
                            current.callback_error = Some(message.clone());
                            persist_pending_login(Some(current));
                        }
                    }
                }
                let html = oauth_fail_html(&message);
                let _ = request.respond(
                    Response::from_string(html)
                        .with_status_code(400)
                        .with_header(
                            Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"text/html; charset=utf-8"[..],
                            )
                            .unwrap(),
                        ),
                );
                continue;
            }

            if access_token.trim().is_empty() {
                let message = "回调缺少 access_token，请重新授权。";
                if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
                    if let Some(current) = guard.as_mut() {
                        if current.login_id == expected_login_id && current.state == expected_state
                        {
                            current.callback_error = Some(message.to_string());
                            persist_pending_login(Some(current));
                        }
                    }
                }
                let html = oauth_fail_html(message);
                let _ = request.respond(
                    Response::from_string(html)
                        .with_status_code(400)
                        .with_header(
                            Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"text/html; charset=utf-8"[..],
                            )
                            .unwrap(),
                        ),
                );
                continue;
            }

            if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
                if let Some(current) = guard.as_mut() {
                    if current.login_id == expected_login_id && current.state == expected_state {
                        current.access_token = Some(access_token);
                        current.callback_error = None;
                        persist_pending_login(Some(current));
                    }
                }
            }

            let _ = request.respond(
                Response::from_string(oauth_success_html())
                    .with_status_code(200)
                    .with_header(
                        Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                            .unwrap(),
                    ),
            );
            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
    }

    Ok(())
}

async fn post_seat_management_json(
    base_url: &str,
    method: &str,
    body: Value,
) -> Result<Value, String> {
    let base = base_url.trim().trim_end_matches('/');
    let url = format!(
        "{}/exa.seat_management_pb.SeatManagementService/{}",
        base, method
    );
    let client = reqwest::Client::new();

    let response = client
        .post(url.clone())
        .header("User-Agent", APP_USER_AGENT)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 Windsurf {} 失败: {}", method, e))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .unwrap_or_else(|_| "<no-body>".to_string());

    if !status.is_success() {
        return Err(format!(
            "请求 Windsurf {} 失败: status={}, body_len={}",
            method,
            status,
            text.len()
        ));
    }

    serde_json::from_str::<Value>(&text).map_err(|e| {
        format!(
            "解析 Windsurf {} 响应失败: {} (body_len={})",
            method,
            e,
            text.len()
        )
    })
}

async fn register_user(firebase_id_token: &str) -> Result<RegisterResult, String> {
    let payload = json!({
        "firebase_id_token": firebase_id_token
    });
    let value =
        post_seat_management_json(WINDSURF_REGISTER_API_BASE_URL, "RegisterUser", payload).await?;

    let api_key = pick_string_from_object(Some(&value), &["apiKey", "api_key"])
        .ok_or_else(|| "RegisterUser 响应缺少 apiKey".to_string())?;
    let api_server_url = pick_string_from_object(Some(&value), &["apiServerUrl", "api_server_url"])
        .unwrap_or_else(|| WINDSURF_DEFAULT_API_SERVER_URL.to_string());
    let name = pick_string_from_object(Some(&value), &["name"]);

    Ok(RegisterResult {
        api_key,
        api_server_url,
        name,
    })
}

async fn get_one_time_auth_token(
    api_server_url: &str,
    firebase_id_token: &str,
) -> Result<String, String> {
    let payload = json!({
        "firebaseIdToken": firebase_id_token
    });
    let value = post_seat_management_json(api_server_url, "GetOneTimeAuthToken", payload).await?;
    pick_string_from_object(Some(&value), &["authToken", "auth_token"])
        .ok_or_else(|| "GetOneTimeAuthToken 响应缺少 authToken".to_string())
}

async fn get_current_user(api_server_url: &str, auth_token: &str) -> Result<Value, String> {
    let payload = json!({
        "authToken": auth_token,
        "includeSubscription": true
    });
    post_seat_management_json(api_server_url, "GetCurrentUser", payload).await
}

async fn get_plan_status(api_server_url: &str, auth_token: &str) -> Result<Value, String> {
    let payload = json!({
        "authToken": auth_token,
        "includeTopUpStatus": true
    });
    post_seat_management_json(api_server_url, "GetPlanStatus", payload).await
}

fn build_user_status_metadata(api_key: &str) -> Value {
    let normalized_os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };

    json!({
        "apiKey": api_key,
        "ideName": "Windsurf",
        // 服务端会校验该字段，缺失时 GetUserStatus 会返回 invalid_argument。
        "ideVersion": "1.0.0",
        "extensionName": "codeium.windsurf",
        "extensionVersion": "1.0.0",
        "locale": "zh-CN",
        "os": normalized_os,
        "disableTelemetry": false,
        "sessionId": format!("agtools-{}", now_timestamp()),
        "requestId": now_timestamp().to_string()
    })
}

async fn get_user_status_by_api_key(api_server_url: &str, api_key: &str) -> Result<Value, String> {
    let payload = json!({
        "metadata": build_user_status_metadata(api_key)
    });
    post_seat_management_json(api_server_url, "GetUserStatus", payload).await
}

fn merge_plan_snapshot(
    current_user: Option<&Value>,
    user_status_resp: Option<&Value>,
    plan_status_resp: Option<&Value>,
) -> (Option<Value>, Option<Value>, Option<Value>) {
    let user_status = user_status_resp
        .and_then(|value| value.get("userStatus"))
        .cloned();
    let plan_status_from_status = user_status
        .as_ref()
        .and_then(|value| value.get("planStatus"))
        .cloned();
    let plan_status = plan_status_resp
        .and_then(|value| value.get("planStatus"))
        .cloned()
        .or(plan_status_from_status);

    (user_status, plan_status, current_user.cloned())
}

fn build_payload_from_remote(
    source_token: String,
    token_type: Option<String>,
    api_key: String,
    api_server_url: String,
    auth_token: Option<String>,
    register_name: Option<String>,
    current_user_resp: Option<Value>,
    user_status_resp: Option<Value>,
    plan_status_resp: Option<Value>,
    auth_status_raw: Option<Value>,
) -> WindsurfOAuthCompletePayload {
    let current_user = current_user_resp
        .as_ref()
        .and_then(|value| value.get("user"));
    let (user_status, plan_status, current_user_snapshot) = merge_plan_snapshot(
        current_user_resp.as_ref(),
        user_status_resp.as_ref(),
        plan_status_resp.as_ref(),
    );

    let plan_info = plan_status
        .as_ref()
        .and_then(|value| value.get("planInfo"))
        .cloned()
        .or_else(|| {
            user_status_resp
                .as_ref()
                .and_then(|value| value.get("planInfo"))
                .cloned()
        });

    let name = pick_string_from_object(current_user, &["name"])
        .or_else(|| pick_string_from_object(user_status.as_ref(), &["name"]))
        .or(register_name);
    let email = pick_string_from_object(current_user, &["email"])
        .or_else(|| pick_string_from_object(user_status.as_ref(), &["email"]));
    let username = pick_string_from_object(current_user, &["username"])
        .or_else(|| pick_string_from_object(user_status.as_ref(), &["username"]));
    let user_id = pick_string_from_object(current_user, &["id"])
        .or_else(|| pick_string_from_object(user_status.as_ref(), &["id"]));

    let login_seed = username
        .clone()
        .or_else(|| {
            email
                .clone()
                .map(|value| value.split('@').next().unwrap_or("").to_string())
        })
        .or_else(|| name.clone())
        .unwrap_or_else(|| "windsurf_user".to_string());
    let github_login = sanitize_login(&login_seed);
    let github_id = hash_to_u64(
        user_id
            .as_deref()
            .or(email.as_deref())
            .unwrap_or_else(|| github_login.as_str()),
    );

    let available_prompt = pick_i64_from_object(
        plan_status.as_ref(),
        &["availablePromptCredits", "available_prompt_credits"],
    );
    let used_prompt = pick_i64_from_object(
        plan_status.as_ref(),
        &["usedPromptCredits", "used_prompt_credits"],
    );
    let available_flow = pick_i64_from_object(
        plan_status.as_ref(),
        &["availableFlowCredits", "available_flow_credits"],
    );
    let used_flow = pick_i64_from_object(
        plan_status.as_ref(),
        &["usedFlowCredits", "used_flow_credits"],
    );

    let total_prompt = match (available_prompt, used_prompt) {
        (Some(a), Some(u)) => Some((a + u).max(0)),
        (Some(a), None) => Some(a.max(0)),
        _ => None,
    };
    let total_flow = match (available_flow, used_flow) {
        (Some(a), Some(u)) => Some((a + u).max(0)),
        (Some(a), None) => Some(a.max(0)),
        _ => None,
    };

    let reset_at = plan_status
        .as_ref()
        .and_then(|value| value.get("planEnd"))
        .and_then(|value| parse_proto_timestamp_seconds(Some(value)));

    let plan_name = pick_string_from_object(plan_info.as_ref(), &["planName", "plan_name"])
        .or_else(|| pick_string_from_object(plan_info.as_ref(), &["teamsTier", "teams_tier"]));

    let copilot_token =
        build_synthetic_copilot_token(total_prompt, total_flow, reset_at, plan_name.as_deref());
    let copilot_limited_user_quotas = if available_prompt.is_some() || available_flow.is_some() {
        Some(json!({
            "completions": available_prompt.unwrap_or(0).max(0),
            "chat": available_flow.unwrap_or(0).max(0)
        }))
    } else {
        None
    };
    let copilot_quota_reset_date = reset_at
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|dt| dt.to_rfc3339());
    let copilot_quota_snapshots = Some(json!({
        "windsurfPlanStatus": plan_status,
        "windsurfPlanInfo": plan_info,
        "windsurfUserStatus": user_status,
        "windsurfCurrentUser": current_user_snapshot
    }));

    let windsurf_auth_status_raw = {
        let mut merged = auth_status_raw.unwrap_or_else(|| json!({}));
        if !merged.is_object() {
            merged = json!({});
        }
        if let Some(obj) = merged.as_object_mut() {
            obj.insert("apiKey".to_string(), Value::String(api_key.clone()));
            obj.insert(
                "apiServerUrl".to_string(),
                Value::String(api_server_url.clone()),
            );
            if let Some(value) = name.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()) {
                obj.insert("name".to_string(), Value::String(value.to_string()));
            }
            if let Some(value) = email.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()) {
                obj.insert("email".to_string(), Value::String(value.to_string()));
            }
        }
        Some(merged)
    };

    WindsurfOAuthCompletePayload {
        github_login,
        github_id,
        github_name: name,
        github_email: email,
        github_access_token: source_token,
        github_token_type: token_type,
        github_scope: None,
        copilot_token,
        copilot_plan: plan_name,
        copilot_chat_enabled: Some(true),
        copilot_expires_at: None,
        copilot_refresh_in: None,
        copilot_quota_snapshots,
        copilot_quota_reset_date,
        copilot_limited_user_quotas,
        copilot_limited_user_reset_date: reset_at,
        windsurf_api_key: Some(api_key),
        windsurf_api_server_url: Some(api_server_url),
        windsurf_auth_token: auth_token,
        windsurf_user_status: user_status_resp,
        windsurf_plan_status: plan_status_resp,
        windsurf_auth_status_raw,
    }
}

async fn build_payload_from_firebase_token(
    firebase_id_token: &str,
    auth_status_raw: Option<Value>,
) -> Result<WindsurfOAuthCompletePayload, String> {
    let register = register_user(firebase_id_token).await?;
    let auth_token =
        match get_one_time_auth_token(&register.api_server_url, firebase_id_token).await {
            Ok(token) => Some(token),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Windsurf OAuth] GetOneTimeAuthToken 失败（将继续尝试用 apiKey 拉用户态）: {}",
                    err
                ));
                None
            }
        };
    let current_user = if let Some(auth_token) = auth_token.as_deref() {
        match get_current_user(&register.api_server_url, auth_token).await {
            Ok(value) => Some(value),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Windsurf OAuth] GetCurrentUser 失败（已忽略）: {}",
                    err
                ));
                None
            }
        }
    } else {
        None
    };
    let plan_status = if let Some(auth_token) = auth_token.as_deref() {
        match get_plan_status(&register.api_server_url, auth_token).await {
            Ok(value) => Some(value),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Windsurf OAuth] GetPlanStatus 失败（已忽略）: {}",
                    err
                ));
                None
            }
        }
    } else {
        None
    };
    let user_status =
        match get_user_status_by_api_key(&register.api_server_url, &register.api_key).await {
            Ok(value) => Some(value),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Windsurf OAuth] GetUserStatus 失败（将导致邮箱/配额缺失）: {}",
                    err
                ));
                None
            }
        };

    Ok(build_payload_from_remote(
        firebase_id_token.to_string(),
        Some("Bearer".to_string()),
        register.api_key,
        register.api_server_url,
        auth_token,
        register.name,
        current_user,
        user_status,
        plan_status,
        auth_status_raw,
    ))
}

async fn build_payload_from_api_key(
    api_key: &str,
    auth_status_raw: Option<Value>,
    api_server_url_hint: Option<&str>,
) -> Result<WindsurfOAuthCompletePayload, String> {
    let api_server_url = resolve_api_server_url(auth_status_raw.as_ref(), api_server_url_hint);
    let user_status = match get_user_status_by_api_key(&api_server_url, api_key).await {
        Ok(value) => Some(value),
        Err(err) => {
            logger::log_warn(&format!(
                "[Windsurf OAuth] API Key 模式 GetUserStatus 失败（将导致邮箱/配额缺失）: {}",
                err
            ));
            None
        }
    };

    Ok(build_payload_from_remote(
        api_key.to_string(),
        Some("ApiKey".to_string()),
        api_key.to_string(),
        api_server_url,
        None,
        None,
        None,
        user_status,
        None,
        auth_status_raw,
    ))
}

pub async fn start_login() -> Result<WindsurfOAuthStartResponse, String> {
    hydrate_pending_login_if_missing();
    if let Ok(guard) = PENDING_OAUTH_STATE.lock() {
        if let Some(state) = guard.as_ref() {
            if state.expires_at > now_timestamp() {
                ensure_callback_server_for_state(state);
                logger::log_info(&format!(
                    "[Windsurf OAuth] 复用进行中的登录会话: login_id={}, port={}, age={}s",
                    state.login_id,
                    state.port,
                    now_timestamp() - state.created_at
                ));
                return Ok(to_start_response(state));
            }
        }
    }
    set_pending_login(None);

    let port = find_available_port()?;
    let login_id = generate_token();
    let state_token = generate_token();
    let callback_url = format!("http://127.0.0.1:{}/windsurf-auth-callback", port);
    let auth_url = build_auth_url(&callback_url, &state_token);
    let pending = PendingOAuthState {
        login_id: login_id.clone(),
        state: state_token.clone(),
        auth_url: auth_url.clone(),
        callback_url: callback_url.clone(),
        port,
        created_at: now_timestamp(),
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS as i64,
        access_token: None,
        callback_error: None,
    };

    set_pending_login(Some(pending.clone()));

    let callback_login_id = login_id.clone();
    let callback_state = state_token.clone();
    tokio::spawn(async move {
        if let Err(e) = start_callback_server(port, callback_login_id, callback_state).await {
            logger::log_error(&format!(
                "[Windsurf OAuth] 回调服务异常: login_id={}, error={}",
                login_id, e
            ));
        }
    });

    logger::log_info(&format!(
        "[Windsurf OAuth] 登录会话已创建: login_id={}, callback_url={}",
        pending.login_id, pending.callback_url
    ));
    Ok(to_start_response(&pending))
}

pub async fn complete_login(login_id: &str) -> Result<WindsurfOAuthCompletePayload, String> {
    hydrate_pending_login_if_missing();
    let token = loop {
        let state = {
            let guard = PENDING_OAUTH_STATE
                .lock()
                .map_err(|_| "OAuth 状态锁不可用".to_string())?;
            guard.clone()
        };
        let state = state.ok_or_else(|| "登录流程已取消，请重新发起授权".to_string())?;
        if state.login_id != login_id {
            return Err("登录会话已变更，请刷新后重试".to_string());
        }
        if state.expires_at <= now_timestamp() {
            clear_pending_if_matches(&state.login_id, &state.state);
            return Err("等待 Windsurf 授权超时，请重新发起授权".to_string());
        }
        if let Some(error) = state.callback_error {
            clear_pending_if_matches(&state.login_id, &state.state);
            return Err(error);
        }
        if let Some(token) = state.access_token.clone() {
            break (state, token);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
    };

    let (state, access_token) = token;
    let result = build_payload_from_firebase_token(&access_token, None).await;
    clear_pending_if_matches(&state.login_id, &state.state);
    result
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    hydrate_pending_login_if_missing();
    let current = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "OAuth 状态锁不可用".to_string())?
        .as_ref()
        .cloned();

    match (current.as_ref(), login_id) {
        (Some(state), Some(input)) if state.login_id != input => {
            return Err("登录会话不匹配，取消失败".to_string());
        }
        (Some(state), _) => {
            notify_cancel(state.port);
            set_pending_login(None);
        }
        (None, _) => {}
    }
    Ok(())
}

pub fn submit_callback_url(login_id: &str, callback_url: &str) -> Result<(), String> {
    hydrate_pending_login_if_missing();
    let (expected_state, port, expires_at) = {
        let guard = PENDING_OAUTH_STATE
            .lock()
            .map_err(|_| "OAuth 状态锁不可用".to_string())?;
        let state = guard
            .as_ref()
            .ok_or_else(|| "登录流程已取消，请重新发起授权".to_string())?;
        if state.login_id != login_id {
            return Err("登录会话已变更，请刷新后重试".to_string());
        }
        (state.state.clone(), state.port, state.expires_at)
    };

    if expires_at <= now_timestamp() {
        return Err("等待 Windsurf 授权超时，请重新发起授权".to_string());
    }

    let parsed = parse_callback_url(callback_url, port)?;
    if parsed.path() != "/windsurf-auth-callback" {
        return Err("回调链接路径无效，必须为 /windsurf-auth-callback".to_string());
    }

    let params = parse_query_params(parsed.query().unwrap_or_default());
    let state = params
        .get("state")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "回调链接中缺少 state 参数".to_string())?;
    if state != expected_state {
        return Err("回调 state 校验失败，请确认粘贴的是当前登录会话链接".to_string());
    }

    if let Some(error) = params
        .get("error")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        let error_desc = params
            .get("error_description")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
        let message = if error_desc.is_empty() {
            format!("授权失败: {}", error)
        } else {
            format!("授权失败: {} ({})", error, error_desc)
        };
        if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
            if let Some(current) = guard.as_mut() {
                if current.login_id == login_id && current.state == expected_state {
                    current.callback_error = Some(message.clone());
                    persist_pending_login(Some(current));
                }
            }
        }
        return Err(message);
    }

    let access_token = params
        .get("access_token")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "回调链接中缺少 access_token 参数".to_string())?
        .to_string();

    let mut guard = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "OAuth 状态锁不可用".to_string())?;
    let state = guard
        .as_mut()
        .ok_or_else(|| "登录流程已取消，请重新发起授权".to_string())?;
    if state.login_id != login_id {
        return Err("登录会话已变更，请刷新后重试".to_string());
    }
    state.access_token = Some(access_token);
    state.callback_error = None;
    persist_pending_login(Some(state));

    logger::log_info(&format!(
        "[Windsurf OAuth] 已接收手动回调链接: login_id={}",
        login_id
    ));
    Ok(())
}

pub fn restore_pending_oauth_listener() {
    hydrate_pending_login_if_missing();
    let pending = PENDING_OAUTH_STATE
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    if let Some(state) = pending {
        ensure_callback_server_for_state(&state);
    }
}

pub async fn build_payload_from_token(token: &str) -> Result<WindsurfOAuthCompletePayload, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err("Token 不能为空".to_string());
    }

    if trimmed.starts_with("sk-ws-") {
        return build_payload_from_api_key(trimmed, None, None).await;
    }

    if trimmed.starts_with("eyJ") {
        return build_payload_from_firebase_token(trimmed, None).await;
    }

    Err("Token 格式不支持：请使用 Windsurf API Key 或 Firebase JWT".to_string())
}

pub async fn build_payload_from_password(
    email: &str,
    password: &str,
) -> Result<WindsurfOAuthCompletePayload, String> {
    let email = email.trim();
    if email.is_empty() || password.is_empty() {
        return Err("邮箱和密码不能为空".to_string());
    }

    logger::log_info("[Windsurf PasswordLogin] 开始 Firebase 邮箱密码登录");

    let url = format!("{}?key={}", FIREBASE_SIGN_IN_URL, FIREBASE_API_KEY);
    let body = json!({
        "email": email,
        "password": password,
        "returnSecureToken": true,
        "clientType": "CLIENT_TYPE_WEB"
    });

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .header(
            "Sec-Ch-Ua",
            r#""Chromium";v="142", "Google Chrome";v="142", "Not_A Brand";v="99""#,
        )
        .header("Sec-Ch-Ua-Mobile", "?0")
        .header("Sec-Ch-Ua-Platform", r#""Windows""#)
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "cross-site")
        .header("X-Client-Version", "Chrome/JsCore/11.0.0/FirebaseCore-web")
        .header("Referer", "https://windsurf.com/")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Firebase 登录请求失败: {}", e))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .unwrap_or_else(|_| "<no-body>".to_string());

    if !status.is_success() {
        let error_msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .map(|message| message.to_string())
            })
            .unwrap_or_else(|| text.clone());

        let friendly = match error_msg.as_str() {
            "EMAIL_NOT_FOUND" => "邮箱不存在".to_string(),
            "INVALID_PASSWORD" | "INVALID_LOGIN_CREDENTIALS" => "邮箱或密码错误".to_string(),
            "USER_DISABLED" => "账号已被禁用".to_string(),
            "TOO_MANY_ATTEMPTS_TRY_LATER" => "尝试次数过多，请稍后再试".to_string(),
            _ => format!("Firebase 登录失败: {}", error_msg),
        };
        return Err(friendly);
    }

    let firebase_resp: Value =
        serde_json::from_str(&text).map_err(|e| format!("解析 Firebase 响应失败: {}", e))?;

    let id_token = firebase_resp
        .get("idToken")
        .and_then(Value::as_str)
        .ok_or_else(|| "Firebase 响应缺少 idToken".to_string())?;

    logger::log_info("[Windsurf PasswordLogin] Firebase 登录成功，开始获取账号信息");

    build_payload_from_firebase_token(id_token, None).await
}

pub async fn build_payload_from_local_auth_status(
    auth_status: Value,
) -> Result<WindsurfOAuthCompletePayload, String> {
    let api_key = pick_string_from_object(Some(&auth_status), &["apiKey", "api_key"])
        .ok_or_else(|| "本地 Windsurf 登录信息缺少 apiKey".to_string())?;

    let payload = if auth_status
        .get("firebaseIdToken")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        let firebase_token = auth_status
            .get("firebaseIdToken")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        build_payload_from_firebase_token(&firebase_token, Some(auth_status.clone())).await?
    } else {
        build_payload_from_api_key(&api_key, Some(auth_status.clone()), None).await?
    };

    Ok(payload)
}

pub async fn refresh_payload_for_account(
    account: &WindsurfAccount,
) -> Result<WindsurfOAuthCompletePayload, String> {
    let mut auth_status_hint = account
        .windsurf_auth_status_raw
        .clone()
        .unwrap_or_else(|| json!({}));
    if !auth_status_hint.is_object() {
        auth_status_hint = json!({});
    }
    if let Some(obj) = auth_status_hint.as_object_mut() {
        if let Some(api_key) = account
            .windsurf_api_key
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            obj.insert("apiKey".to_string(), Value::String(api_key.to_string()));
        }
        if let Some(api_server_url) = account
            .windsurf_api_server_url
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            obj.insert(
                "apiServerUrl".to_string(),
                Value::String(api_server_url.to_string()),
            );
        }
        if let Some(name) = account
            .github_name
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            obj.insert("name".to_string(), Value::String(name.to_string()));
        }
        if let Some(email) = account
            .github_email
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            obj.insert("email".to_string(), Value::String(email.to_string()));
        }
    }

    if let Some(api_key) = account
        .windsurf_api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return build_payload_from_api_key(
            api_key,
            Some(auth_status_hint.clone()),
            account.windsurf_api_server_url.as_deref(),
        )
        .await;
    }

    if account
        .github_token_type
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case("bearer"))
        .unwrap_or(false)
        && !account.github_access_token.trim().starts_with("sk-ws-")
    {
        return build_payload_from_firebase_token(
            &account.github_access_token,
            Some(auth_status_hint.clone()),
        )
        .await;
    }

    build_payload_from_token(&account.github_access_token).await
}
