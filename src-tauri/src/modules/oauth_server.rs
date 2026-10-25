use crate::modules::oauth;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::time::{timeout, Duration};

struct OAuthFlowState {
    auth_url: String,
    redirect_uri: String,
    expected_state: String,
    cancel_tx: watch::Sender<bool>,
    code_tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<Result<String, String>>>>>,
    code_rx: Option<oneshot::Receiver<Result<String, String>>>,
}

static OAUTH_FLOW_STATE: OnceLock<Mutex<Option<OAuthFlowState>>> = OnceLock::new();
const OAUTH_CALLBACK_PATH: &str = "/oauth-callback";
const MAX_HTTP_REQUEST_BYTES: usize = 32 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
const OAUTH_FLOW_WAIT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

fn get_oauth_flow_state() -> &'static Mutex<Option<OAuthFlowState>> {
    OAUTH_FLOW_STATE.get_or_init(|| Mutex::new(None))
}

fn oauth_success_html() -> &'static str {
    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
    <html>\
    <body style='font-family: sans-serif; text-align: center; padding: 50px; background: #0d1117; color: #fff;'>\
        <h1 style='color: #4ade80;'>✅ 授权成功!</h1>\
        <p>您可以关闭此窗口返回应用。</p>\
        <script>setTimeout(function() { window.close(); }, 2000);</script>\
    </body>\
    </html>"
}

fn oauth_fail_html(message: &str) -> String {
    format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
    <html>\
    <body style='font-family: sans-serif; text-align: center; padding: 50px; background: #0d1117; color: #fff;'>\
        <h1 style='color: #f87171;'>❌ 授权失败</h1>\
        <p>{}</p>\
    </body>\
    </html>",
        message
    )
}

fn oauth_not_found_response() -> &'static str {
    "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nNot Found"
}

fn oauth_options_response() -> &'static str {
    "HTTP/1.1 200 OK\r\n\
    Access-Control-Allow-Origin: *\r\n\
    Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
    Access-Control-Allow-Headers: Content-Type\r\n\
    Content-Length: 0\r\n\r\n"
}

fn clear_oauth_flow_state() {
    if let Ok(mut lock) = get_oauth_flow_state().lock() {
        *lock = None;
    }
}

fn extract_code_from_callback_url(
    callback_url: &Url,
    expected_state: &str,
) -> Result<String, String> {
    let mut code = None;
    let mut state = None;
    for (key, value) in callback_url.query_pairs() {
        match key.as_ref() {
            "code" if code.is_none() => code = Some(value.into_owned()),
            "state" if state.is_none() => state = Some(value.into_owned()),
            _ => {}
        }
    }

    let Some(code) = code.filter(|value| !value.trim().is_empty()) else {
        return Err("未能在回调中获取 Authorization Code".to_string());
    };

    let Some(state) = state.filter(|value| !value.trim().is_empty()) else {
        return Err("未能在回调中获取 OAuth state".to_string());
    };

    if state != expected_state {
        return Err("OAuth state 校验失败".to_string());
    }

    Ok(code)
}

fn parse_manual_callback_url(raw_callback_url: &str, redirect_uri: &str) -> Result<Url, String> {
    let trimmed = raw_callback_url.trim();
    if trimmed.is_empty() {
        return Err("回调链接不能为空".to_string());
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Url::parse(trimmed).map_err(|e| format!("OAuth 回调 URL 解析失败: {}", e));
    }

    let redirect =
        Url::parse(redirect_uri).map_err(|e| format!("OAuth redirect_uri 无效: {}", e))?;
    let host = redirect
        .host_str()
        .ok_or_else(|| "OAuth redirect_uri 缺少 host".to_string())?;
    let origin = match redirect.port() {
        Some(port) => format!("{}://{}:{}", redirect.scheme(), host, port),
        None => format!("{}://{}", redirect.scheme(), host),
    };

    if trimmed.starts_with('/') {
        return Url::parse(format!("{}{}", origin, trimmed).as_str())
            .map_err(|e| format!("OAuth 回调 URL 解析失败: {}", e));
    }

    Url::parse(
        format!(
            "{}{}?{}",
            origin,
            OAUTH_CALLBACK_PATH,
            trimmed.trim_start_matches('?')
        )
        .as_str(),
    )
    .map_err(|e| format!("OAuth 回调 URL 解析失败: {}", e))
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Result<String, String> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];

    loop {
        let bytes_read = timeout(REQUEST_READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| "读取 OAuth 回调请求超时".to_string())?
            .map_err(|e| format!("读取 OAuth 回调请求失败: {}", e))?;

        if bytes_read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..bytes_read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n")
            || buffer.len() >= MAX_HTTP_REQUEST_BYTES
        {
            break;
        }
    }

    if buffer.is_empty() {
        return Err("OAuth 回调请求为空".to_string());
    }

    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn parse_request_target(request: &str) -> Result<(String, String), String> {
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| "OAuth 回调请求行为空".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "OAuth 回调请求缺少 method".to_string())?;
    let target = parts
        .next()
        .ok_or_else(|| "OAuth 回调请求缺少 target".to_string())?;

    Ok((method.to_string(), target.to_string()))
}

async fn process_callback_request(
    stream: &mut tokio::net::TcpStream,
    port: u16,
    expected_state: &str,
) -> Option<Result<String, String>> {
    let request = match read_http_request(stream).await {
        Ok(request) => request,
        Err(err) => {
            let response = oauth_fail_html("回调请求读取失败，请返回应用重试。");
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
            return Some(Err(err));
        }
    };

    let (method, target) = match parse_request_target(&request) {
        Ok(parsed) => parsed,
        Err(err) => {
            let response = oauth_fail_html("回调请求格式无效，请返回应用重试。");
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
            return Some(Err(err));
        }
    };

    if method.eq_ignore_ascii_case("OPTIONS") {
        let _ = stream.write_all(oauth_options_response().as_bytes()).await;
        let _ = stream.flush().await;
        return None;
    }

    let callback_url = match if target.starts_with("http://") || target.starts_with("https://") {
        Url::parse(&target)
    } else {
        Url::parse(&format!("http://localhost:{}{}", port, target))
    } {
        Ok(url) => url,
        Err(_) => {
            let response = oauth_fail_html("回调 URL 解析失败，请返回应用重试。");
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
            return Some(Err("OAuth 回调 URL 解析失败".to_string()));
        }
    };

    if callback_url.path() != OAUTH_CALLBACK_PATH {
        let _ = stream
            .write_all(oauth_not_found_response().as_bytes())
            .await;
        let _ = stream.flush().await;
        return None;
    }

    let code = match extract_code_from_callback_url(&callback_url, expected_state) {
        Ok(code) => code,
        Err(err) if err == "未能在回调中获取 Authorization Code" => {
            let response = oauth_fail_html("未能获取授权 code，请返回应用重试。");
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
            return Some(Err(err));
        }
        Err(err) if err == "未能在回调中获取 OAuth state" => {
            let response = oauth_fail_html("未能获取授权状态 state，请返回应用重试。");
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
            return Some(Err(err));
        }
        Err(err) => {
            let response = oauth_fail_html("授权状态校验失败，请返回应用重新发起授权。");
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
            return Some(Err(err));
        }
    };

    let _ = stream.write_all(oauth_success_html().as_bytes()).await;
    let _ = stream.flush().await;

    Some(Ok(code))
}

// Listen on both loopback families before advertising localhost. A single-family
// machine gets an explicit IP so the browser cannot choose an unbound address.
struct OAuthCallbackListeners {
    ipv4: Option<TcpListener>,
    ipv6: Option<TcpListener>,
    port: u16,
}

fn callback_redirect_uri(port: u16, has_ipv4: bool, has_ipv6: bool) -> String {
    let host = match (has_ipv4, has_ipv6) {
        (true, true) => "localhost",
        (true, false) => "127.0.0.1",
        (false, true) => "[::1]",
        (false, false) => unreachable!("OAuth needs a bound loopback listener"),
    };
    format!("http://{}:{}{}", host, port, OAUTH_CALLBACK_PATH)
}

async fn bind_callback_listeners() -> Result<OAuthCallbackListeners, String> {
    let (first, is_ipv6) = match TcpListener::bind("[::1]:0").await {
        Ok(listener) => (listener, true),
        Err(_) => (
            TcpListener::bind("127.0.0.1:0")
                .await
                .map_err(|e| format!("无法绑定本地端口: {}", e))?,
            false,
        ),
    };
    let port = first
        .local_addr()
        .map_err(|e| format!("无法获取本地端口: {}", e))?
        .port();
    let other_address = if is_ipv6 {
        format!("127.0.0.1:{}", port)
    } else {
        format!("[::1]:{}", port)
    };
    let other = TcpListener::bind(&other_address).await.ok();
    let (ipv4, ipv6) = if is_ipv6 {
        (other, Some(first))
    } else {
        (Some(first), other)
    };
    Ok(OAuthCallbackListeners { ipv4, ipv6, port })
}

async fn accept_callback(
    listener: &Option<TcpListener>,
) -> std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)> {
    match listener {
        Some(listener) => listener.accept().await,
        None => std::future::pending().await,
    }
}

async fn ensure_oauth_flow_prepared(app_handle: &tauri::AppHandle) -> Result<String, String> {
    use tauri::Emitter;

    if let Ok(state) = get_oauth_flow_state().lock() {
        if let Some(s) = state.as_ref() {
            return Ok(s.auth_url.clone());
        }
    }

    let listeners = bind_callback_listeners().await?;
    let port = listeners.port;
    let redirect_uri =
        callback_redirect_uri(port, listeners.ipv4.is_some(), listeners.ipv6.is_some());
    let state_token = uuid::Uuid::new_v4().to_string();
    let auth_url = oauth::get_auth_url(&redirect_uri, Some(&state_token));

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (code_tx, code_rx) = oneshot::channel::<Result<String, String>>();

    let code_tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(code_tx)));
    let app_handle_clone = app_handle.clone();

    let tx = code_tx.clone();
    let mut rx = cancel_rx;
    let expected_state = state_token.clone();
    tokio::spawn(async move {
        loop {
            let accept_result = tokio::select! {
                res = accept_callback(&listeners.ipv4) => Some(res),
                res = accept_callback(&listeners.ipv6) => Some(res),
                _ = rx.changed() => None,
            };

            let Some(accept_result) = accept_result else {
                break;
            };

            let Ok((mut stream, _)) = accept_result else {
                continue;
            };

            let result = process_callback_request(&mut stream, port, &expected_state).await;
            if let Some(result) = result {
                if let Some(sender) = tx.lock().await.take() {
                    let _ = app_handle_clone.emit("oauth-callback-received", ());
                    let _ = sender.send(result);
                }
                break;
            }
        }
    });

    if let Ok(mut state) = get_oauth_flow_state().lock() {
        *state = Some(OAuthFlowState {
            auth_url: auth_url.clone(),
            redirect_uri,
            expected_state: state_token.clone(),
            cancel_tx,
            code_tx: code_tx.clone(),
            code_rx: Some(code_rx),
        });
    }

    let _ = app_handle.emit("oauth-url-generated", &auth_url);

    Ok(auth_url)
}

/// 预生成 OAuth URL
pub async fn prepare_oauth_url(app_handle: tauri::AppHandle) -> Result<String, String> {
    ensure_oauth_flow_prepared(&app_handle).await
}

/// 取消当前的 OAuth 流程
pub fn cancel_oauth_flow() {
    if let Ok(mut state) = get_oauth_flow_state().lock() {
        if let Some(s) = state.take() {
            let _ = s.cancel_tx.send(true);
        }
    }
}

pub async fn submit_oauth_callback_url(
    app_handle: tauri::AppHandle,
    callback_url: &str,
) -> Result<(), String> {
    use tauri::Emitter;

    let (redirect_uri, expected_state, code_tx) = {
        let lock = get_oauth_flow_state()
            .lock()
            .map_err(|_| "OAuth 状态锁被污染".to_string())?;
        let state = lock
            .as_ref()
            .ok_or_else(|| "OAuth 状态不存在，请先发起授权".to_string())?;
        (
            state.redirect_uri.clone(),
            state.expected_state.clone(),
            state.code_tx.clone(),
        )
    };

    let parsed = parse_manual_callback_url(callback_url, &redirect_uri)?;
    if parsed.path() != OAUTH_CALLBACK_PATH {
        return Err(format!("回调链接路径无效，必须为 {}", OAUTH_CALLBACK_PATH));
    }

    let code = extract_code_from_callback_url(&parsed, expected_state.as_str())?;

    let mut tx = code_tx.lock().await;
    let sender = tx
        .take()
        .ok_or_else(|| "OAuth 回调已处理，请勿重复提交".to_string())?;
    sender
        .send(Ok(code))
        .map_err(|_| "OAuth 回调发送失败，请重新发起授权".to_string())?;
    let _ = app_handle.emit("oauth-callback-received", ());
    Ok(())
}

/// 启动 OAuth 流程并等待回调
pub async fn start_oauth_flow(
    app_handle: tauri::AppHandle,
) -> Result<oauth::TokenResponse, String> {
    let auth_url = ensure_oauth_flow_prepared(&app_handle).await?;

    use tauri_plugin_opener::OpenerExt;
    app_handle
        .opener()
        .open_url(&auth_url, None::<String>)
        .map_err(|e| {
            cancel_oauth_flow();
            format!("无法打开浏览器: {}", e)
        })?;

    let (code_rx, redirect_uri) = {
        let mut lock = get_oauth_flow_state()
            .lock()
            .map_err(|_| "OAuth 状态锁被污染".to_string())?;
        let Some(state) = lock.as_mut() else {
            return Err("OAuth 状态不存在".to_string());
        };
        let rx = state
            .code_rx
            .take()
            .ok_or_else(|| "OAuth 授权已在进行中".to_string())?;
        (rx, state.redirect_uri.clone())
    };

    let callback_result = timeout(OAUTH_FLOW_WAIT_TIMEOUT, code_rx).await;
    let code = match callback_result {
        Ok(Ok(Ok(code))) => code,
        Ok(Ok(Err(e))) => {
            clear_oauth_flow_state();
            return Err(e);
        }
        Ok(Err(_)) => {
            clear_oauth_flow_state();
            return Err("等待 OAuth 回调失败".to_string());
        }
        Err(_) => {
            cancel_oauth_flow();
            return Err("等待 OAuth 回调超时，请重试".to_string());
        }
    };

    clear_oauth_flow_state();

    oauth::exchange_code(&code, &redirect_uri).await
}

/// 完成 OAuth 流程（不打开浏览器）

pub async fn complete_oauth_flow(
    app_handle: tauri::AppHandle,
) -> Result<oauth::TokenResponse, String> {
    let _ = ensure_oauth_flow_prepared(&app_handle).await?;

    let (code_rx, redirect_uri) = {
        let mut lock = get_oauth_flow_state()
            .lock()
            .map_err(|_| "OAuth 状态锁被污染".to_string())?;
        let Some(state) = lock.as_mut() else {
            return Err("OAuth 状态不存在".to_string());
        };
        let rx = state
            .code_rx
            .take()
            .ok_or_else(|| "OAuth 授权已在进行中".to_string())?;
        (rx, state.redirect_uri.clone())
    };

    let callback_result = timeout(OAUTH_FLOW_WAIT_TIMEOUT, code_rx).await;
    let code = match callback_result {
        Ok(Ok(Ok(code))) => code,
        Ok(Ok(Err(e))) => {
            clear_oauth_flow_state();
            return Err(e);
        }
        Ok(Err(_)) => {
            clear_oauth_flow_state();
            return Err("等待 OAuth 回调失败".to_string());
        }
        Err(_) => {
            cancel_oauth_flow();
            return Err("等待 OAuth 回调超时，请重试".to_string());
        }
    };

    clear_oauth_flow_state();

    oauth::exchange_code(&code, &redirect_uri).await
}

#[cfg(test)]
#[path = "oauth_server_tests.rs"]
mod tests;
