use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex as TokioMutex, Notify};
use tokio::time::{timeout, Duration};
use tokio_rustls::TlsAcceptor;

const START_CASCADE_PATH: &str = "/exa.language_server_pb.LanguageServerService/StartCascade";
const SEND_USER_CASCADE_MESSAGE_PATH: &str =
    "/exa.language_server_pb.LanguageServerService/SendUserCascadeMessage";
const GET_CASCADE_TRAJECTORY_PATH: &str =
    "/exa.language_server_pb.LanguageServerService/GetCascadeTrajectory";
const DELETE_CASCADE_TRAJECTORY_PATH: &str =
    "/exa.language_server_pb.LanguageServerService/DeleteCascadeTrajectory";
pub const INTERNAL_PREPARE_START_CONTEXT_PATH: &str = "/__ag_internal__/wakeup/prepareStartContext";
pub const INTERNAL_HEALTH_CHECK_PATH: &str = "/__ag_internal__/wakeup/health";

const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP_REQUEST_BYTES: usize = 512 * 1024;
// 对齐官方扩展 waitForLanguageServerStart 的 60s 等待窗口，降低冷启动慢时误判失败。
const OFFICIAL_LS_START_TIMEOUT: Duration = Duration::from_secs(60);
const OFFICIAL_LS_EXTENSION_BIND_TIMEOUT: Duration = Duration::from_secs(8);
const LOCAL_GATEWAY_BIND_TIMEOUT: Duration = Duration::from_secs(8);
const LOCAL_GATEWAY_TLS_SETUP_TIMEOUT: Duration = Duration::from_secs(8);
const OFFICIAL_LS_CLOUD_CODE_DAILY: &str = "https://daily-cloudcode-pa.googleapis.com";
const OFFICIAL_LS_CLOUD_CODE_PROD: &str = "https://cloudcode-pa.googleapis.com";
const OFFICIAL_LS_DEFAULT_APP_DATA_DIR: &str = "antigravity";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static LOCAL_GATEWAY_BASE_URL: OnceLock<TokioMutex<Option<String>>> = OnceLock::new();
static LOCAL_GATEWAY_START_LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();

fn local_gateway_store() -> &'static TokioMutex<Option<String>> {
    LOCAL_GATEWAY_BASE_URL.get_or_init(|| TokioMutex::new(None))
}

fn local_gateway_start_lock() -> &'static TokioMutex<()> {
    LOCAL_GATEWAY_START_LOCK.get_or_init(|| TokioMutex::new(()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OfficialLsVersionMode {
    Gte1216,
    Lt1216,
}

impl OfficialLsVersionMode {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "gte_1_21_6" | ">=1.21.6" => Some(Self::Gte1216),
            "lt_1_21_6" | "<1.21.6" => Some(Self::Lt1216),
            _ => None,
        }
    }

    fn requires_random_port(self) -> bool {
        matches!(self, Self::Lt1216)
    }
}

fn official_ls_version_mode_cell() -> &'static Mutex<OfficialLsVersionMode> {
    static MODE: OnceLock<Mutex<OfficialLsVersionMode>> = OnceLock::new();
    MODE.get_or_init(|| Mutex::new(OfficialLsVersionMode::Gte1216))
}

fn current_official_ls_version_mode() -> OfficialLsVersionMode {
    official_ls_version_mode_cell()
        .lock()
        .map(|guard| *guard)
        .unwrap_or(OfficialLsVersionMode::Gte1216)
}

fn official_ls_requires_random_port() -> bool {
    current_official_ls_version_mode().requires_random_port()
}

pub fn set_official_ls_version_mode(mode: Option<&str>) -> Result<(), String> {
    let next_mode = mode
        .and_then(OfficialLsVersionMode::from_str)
        .unwrap_or(OfficialLsVersionMode::Gte1216);
    let mut guard = official_ls_version_mode_cell()
        .lock()
        .map_err(|_| "官方 LS 版本模式锁失败".to_string())?;
    *guard = next_mode;
    Ok(())
}

#[derive(Debug, Clone)]
struct PreparedStartContext {
    account_id: String,
    prepared_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareStartContextRequest {
    account_id: String,
}

fn pending_start_contexts() -> &'static Mutex<VecDeque<PreparedStartContext>> {
    static PENDING: OnceLock<Mutex<VecDeque<PreparedStartContext>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn pop_prepared_start_context() -> Result<Option<PreparedStartContext>, String> {
    let mut guard = pending_start_contexts()
        .lock()
        .map_err(|_| "准备上下文锁失败".to_string())?;
    while let Some(front) = guard.front() {
        if Utc::now()
            .signed_duration_since(front.prepared_at)
            .num_seconds()
            > 60
        {
            let _ = guard.pop_front();
            continue;
        }
        break;
    }
    Ok(guard.pop_front())
}

fn parse_json_object_body(body: &[u8], name: &str) -> Result<Value, (u16, String)> {
    if body.is_empty() {
        return Ok(json!({}));
    }

    let payload: Value =
        serde_json::from_slice(body).map_err(|e| (400, format!("{} 请求体无效: {}", name, e)))?;
    if !payload.is_object() {
        return Err((400, format!("{} 请求体必须为 JSON object", name)));
    }
    Ok(payload)
}

fn extract_required_cascade_id(payload: &Value, name: &str) -> Result<String, (u16, String)> {
    let cascade_id = payload
        .get("cascadeId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| (400, format!("{} 缺少 cascadeId", name)))?;
    Ok(cascade_id.to_string())
}

fn get_official_ls_session(cascade_id: &str) -> Result<Arc<OfficialLsCascadeSession>, String> {
    let guard = official_ls_sessions()
        .lock()
        .map_err(|_| "官方 LS 会话映射锁失败".to_string())?;
    guard
        .get(cascade_id)
        .cloned()
        .ok_or_else(|| format!("会话不存在: {}", cascade_id))
}

fn remove_official_ls_session(
    cascade_id: &str,
) -> Result<Option<Arc<OfficialLsCascadeSession>>, String> {
    let mut guard = official_ls_sessions()
        .lock()
        .map_err(|_| "官方 LS 会话映射锁失败".to_string())?;
    Ok(guard.remove(cascade_id))
}

async fn shutdown_official_ls_session(session: &Arc<OfficialLsCascadeSession>) {
    let mut process_guard = session.process.lock().await;
    if let Some(mut process) = process_guard.take() {
        process.shutdown().await;
    }
}

async fn start_official_ls_cascade_session(
    prepared: PreparedStartContext,
    start_body: &Value,
) -> Result<(Value, Arc<OfficialLsCascadeSession>), String> {
    let (_account, token) = ensure_wakeup_account_token(&prepared.account_id).await?;
    let mut ls = start_official_ls_process(&prepared.account_id, &token).await?;
    let client = build_official_ls_local_client(30)?;
    let base_url = format!("https://127.0.0.1:{}", ls.started.https_port);
    let start_resp = match post_json_to_official_ls(
        &client,
        &base_url,
        &ls.ls_csrf_token,
        START_CASCADE_PATH,
        start_body,
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            ls.shutdown().await;
            return Err(err);
        }
    };

    start_resp
        .get("cascadeId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "官方 LS StartCascade 未返回 cascadeId".to_string())?;

    let session = Arc::new(OfficialLsCascadeSession {
        account_id: prepared.account_id,
        client,
        base_url,
        ls_csrf_token: ls.ls_csrf_token.clone(),
        process: TokioMutex::new(Some(ls)),
    });

    Ok((start_resp, session))
}

async fn proxy_official_ls_session_json_request(
    session: &OfficialLsCascadeSession,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    post_json_to_official_ls(
        &session.client,
        &session.base_url,
        &session.ls_csrf_token,
        path,
        body,
    )
    .await
}

fn json_response(status_code: u16, status_text: &str, body: &Value) -> Vec<u8> {
    let body_bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n",
        status_code,
        status_text,
        body_bytes.len()
    );
    let mut resp = headers.into_bytes();
    resp.extend_from_slice(&body_bytes);
    resp
}

fn text_response(status_code: u16, status_text: &str, body: &str, content_type: &str) -> Vec<u8> {
    let body_bytes = body.as_bytes();
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n",
        status_code, status_text, content_type, body_bytes.len()
    );
    let mut resp = headers.into_bytes();
    resp.extend_from_slice(body_bytes);
    resp
}

fn options_response() -> Vec<u8> {
    text_response(200, "OK", "", "text/plain; charset=utf-8")
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|idx| idx + 4)
}

fn parse_content_length(header_bytes: &[u8]) -> Result<usize, String> {
    let header_text = String::from_utf8_lossy(header_bytes);
    for line in header_text.lines() {
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse::<usize>()
                .map_err(|e| format!("非法 Content-Length: {}", e));
        }
    }
    Ok(0)
}

async fn read_http_request<R>(stream: &mut R) -> Result<Vec<u8>, String>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let mut header_end: Option<usize> = None;
    let mut content_length: usize = 0;

    loop {
        let bytes_read = timeout(REQUEST_READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| "读取网关请求超时".to_string())?
            .map_err(|e| format!("读取网关请求失败: {}", e))?;

        if bytes_read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..bytes_read]);
        if buffer.len() > MAX_HTTP_REQUEST_BYTES {
            return Err("请求体过大".to_string());
        }

        if header_end.is_none() {
            if let Some(end) = find_header_end(&buffer) {
                content_length = parse_content_length(&buffer[..end])?;
                header_end = Some(end);
            }
        }

        if let Some(end) = header_end {
            if buffer.len() >= end.saturating_add(content_length) {
                return Ok(buffer[..(end + content_length)].to_vec());
            }
        }
    }

    Err("请求不完整".to_string())
}

#[derive(Debug)]
struct ParsedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn parse_http_request(raw: &[u8]) -> Result<ParsedRequest, String> {
    let Some(header_end) = find_header_end(raw) else {
        return Err("缺少 HTTP 头结束标记".to_string());
    };
    let header_text = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().ok_or_else(|| "请求行为空".to_string())?.trim();

    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "请求行缺少 method".to_string())?
        .to_string();
    let target = parts
        .next()
        .ok_or_else(|| "请求行缺少 target".to_string())?
        .to_string();

    let mut headers = HashMap::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    Ok(ParsedRequest {
        method,
        target,
        headers,
        body: raw[header_end..].to_vec(),
    })
}

fn normalize_path(target: &str) -> String {
    if target.starts_with("http://") || target.starts_with("https://") {
        if let Ok(url) = url::Url::parse(target) {
            return url.path().to_string();
        }
    }
    if let Ok(url) = url::Url::parse(&format!("http://localhost{}", target)) {
        return url.path().to_string();
    }
    target.to_string()
}

fn rpc_method_name_from_path(path: &str) -> &str {
    let last = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path);
    last.split(':').next().unwrap_or(last)
}

fn path_matches_rpc_method(path: &str, method_name: &str) -> bool {
    rpc_method_name_from_path(path) == method_name
}

#[derive(Debug, Clone, Copy)]
struct OfficialLsStartedInfo {
    https_port: u16,
    http_port: u16,
    lsp_port: u16,
}

struct OfficialLsExtensionServerState {
    csrf_token: String,
    uss_oauth_topic_bytes: Vec<u8>,
    empty_topic_bytes: Vec<u8>,
    started_sender: Mutex<Option<oneshot::Sender<OfficialLsStartedInfo>>>,
    shutdown_notify: Arc<Notify>,
}

struct OfficialLsExtensionServerHandle {
    port: u16,
    csrf_token: String,
    started_receiver: oneshot::Receiver<OfficialLsStartedInfo>,
    shutdown_notify: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

struct OfficialLsProcessHandle {
    child: Child,
    stdout_task: Option<tokio::task::JoinHandle<()>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
    extension_server: OfficialLsExtensionServerHandle,
    started: OfficialLsStartedInfo,
    ls_csrf_token: String,
}

struct OfficialLsCascadeSession {
    account_id: String,
    client: reqwest::Client,
    base_url: String,
    ls_csrf_token: String,
    process: TokioMutex<Option<OfficialLsProcessHandle>>,
}

fn official_ls_sessions() -> &'static Mutex<HashMap<String, Arc<OfficialLsCascadeSession>>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<OfficialLsCascadeSession>>>> =
        OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

const APP_PATH_NOT_FOUND_PREFIX: &str = "APP_PATH_NOT_FOUND:";

fn app_path_missing_error(app: &str) -> String {
    format!("{}{}", APP_PATH_NOT_FOUND_PREFIX, app)
}

#[cfg(target_os = "macos")]
fn normalize_macos_app_root(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let path_str = path.to_string_lossy();
    let app_idx = path_str.find(".app")?;
    let root = &path_str[..app_idx + 4];
    let root_path = std::path::PathBuf::from(root);
    if root_path.exists() {
        Some(root_path)
    } else {
        None
    }
}

fn resolve_configured_antigravity_root(path_str: &str) -> Option<std::path::PathBuf> {
    let raw = path_str.trim();
    if raw.is_empty() {
        return None;
    }

    let original_path = std::path::PathBuf::from(raw);
    let resolved_path =
        std::fs::canonicalize(&original_path).unwrap_or_else(|_| original_path.clone());

    #[cfg(target_os = "macos")]
    {
        if let Some(root) = normalize_macos_app_root(&original_path)
            .or_else(|| normalize_macos_app_root(&resolved_path))
        {
            return Some(root);
        }
    }

    if !resolved_path.exists() {
        return None;
    }
    if resolved_path.is_file() {
        return resolved_path.parent().map(std::path::Path::to_path_buf);
    }
    if resolved_path.is_dir() {
        return Some(resolved_path);
    }
    None
}

fn antigravity_extension_dir(root: &std::path::Path) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        return root
            .join("Contents")
            .join("Resources")
            .join("app")
            .join("extensions")
            .join("antigravity");
    }
    #[cfg(not(target_os = "macos"))]
    {
        return root
            .join("resources")
            .join("app")
            .join("extensions")
            .join("antigravity");
    }
}

fn antigravity_extension_bin_dir(root: &std::path::Path) -> std::path::PathBuf {
    antigravity_extension_dir(root).join("bin")
}

fn find_official_ls_binary_under(root: &std::path::Path) -> Option<String> {
    let bin_dir = antigravity_extension_bin_dir(root);

    #[cfg(target_os = "windows")]
    let preferred = [
        "language_server_windows_x64.exe",
        "language_server_windows_arm64.exe",
        "language_server_windows.exe",
    ];
    #[cfg(target_os = "macos")]
    let preferred = [
        "language_server_macos_arm",
        "language_server_macos_x64",
        "language_server_macos",
        "language_server_darwin_arm64",
        "language_server_darwin_x64",
        "language_server_darwin",
        "language_server",
    ];
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let preferred = [
        "language_server_linux_x64",
        "language_server_linux_arm64",
        "language_server_linux",
        "language_server",
    ];

    for name in preferred {
        let candidate = bin_dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }

    let mut dynamic_candidates: Vec<std::path::PathBuf> = Vec::new();
    let bin_dir = antigravity_extension_bin_dir(root);
    let entries = std::fs::read_dir(bin_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if !lower.starts_with("language_server") {
            continue;
        }
        #[cfg(target_os = "windows")]
        if !lower.ends_with(".exe") {
            continue;
        }
        dynamic_candidates.push(path);
    }

    dynamic_candidates.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    dynamic_candidates
        .into_iter()
        .next()
        .map(|path| path.to_string_lossy().to_string())
}

fn resolve_official_ls_binary_from_config() -> Result<String, String> {
    let user_config = crate::modules::config::get_user_config();
    let antigravity_path = user_config.antigravity_app_path.trim();
    let root = resolve_configured_antigravity_root(antigravity_path)
        .ok_or_else(|| app_path_missing_error("antigravity"))?;
    find_official_ls_binary_under(&root).ok_or_else(|| app_path_missing_error("antigravity"))
}

fn official_ls_binary_path() -> Result<String, String> {
    if let Ok(v) = std::env::var("AG_WAKEUP_OFFICIAL_LS_BINARY_PATH") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    resolve_official_ls_binary_from_config()
}

pub fn ensure_official_ls_binary_ready() -> Result<String, String> {
    official_ls_binary_path()
}

fn official_ls_cloud_code_endpoint(token: &crate::models::token::TokenData) -> &'static str {
    if token.is_gcp_tos == Some(true) {
        OFFICIAL_LS_CLOUD_CODE_PROD
    } else {
        OFFICIAL_LS_CLOUD_CODE_DAILY
    }
}

fn official_ls_app_data_dir_name() -> String {
    if let Ok(v) = std::env::var("AG_WAKEUP_OFFICIAL_LS_APP_DATA_DIR") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    OFFICIAL_LS_DEFAULT_APP_DATA_DIR.to_string()
}

pub(crate) fn official_antigravity_root_for_version() -> Option<std::path::PathBuf> {
    let user_config = crate::modules::config::get_user_config();
    if let Some(root) = resolve_configured_antigravity_root(user_config.antigravity_app_path.trim())
    {
        return Some(root);
    }

    #[cfg(target_os = "macos")]
    {
        let default_root = std::path::PathBuf::from("/Applications/Antigravity.app");
        if default_root.exists() {
            return Some(default_root);
        }
    }

    None
}

fn official_antigravity_info_plist_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return official_antigravity_root_for_version()
            .map(|root| root.join("Contents").join("Info.plist"));
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn read_json_string_field(path: &std::path::Path, key: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&content).ok()?;
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn official_antigravity_product_json_path() -> Option<std::path::PathBuf> {
    let root = official_antigravity_root_for_version()?;
    #[cfg(target_os = "macos")]
    let candidates = [
        root.join("Contents")
            .join("Resources")
            .join("app")
            .join("product.json"),
        root.join("resources").join("app").join("product.json"),
        root.join("app").join("product.json"),
    ];
    #[cfg(not(target_os = "macos"))]
    let candidates = [
        root.join("resources").join("app").join("product.json"),
        root.join("app").join("product.json"),
    ];

    for path in candidates {
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn parse_version_from_plutil_output(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("\"CFBundleShortVersionString\"") {
            continue;
        }
        let Some(version) = line.split("=>").nth(1) else {
            continue;
        };
        let version = version.trim().trim_matches('"');
        if !version.is_empty() {
            return Some(version.to_string());
        }
    }
    None
}

fn official_antigravity_extension_path() -> String {
    let user_config = crate::modules::config::get_user_config();
    if let Some(root) = resolve_configured_antigravity_root(user_config.antigravity_app_path.trim())
    {
        let ext_path = antigravity_extension_dir(&root);
        if ext_path.exists() {
            return ext_path.to_string_lossy().to_string();
        }
        return root.to_string_lossy().to_string();
    }

    let default_path =
        "/Applications/Antigravity.app/Contents/Resources/app/extensions/antigravity";
    if std::path::Path::new(default_path).exists() {
        return default_path.to_string();
    }
    "/Applications/Antigravity.app".to_string()
}

pub(crate) fn official_antigravity_app_version() -> String {
    if let Ok(v) = std::env::var("AG_WAKEUP_OFFICIAL_APP_VERSION") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Some(product_json_path) = official_antigravity_product_json_path() {
        if let Some(version) = read_json_string_field(&product_json_path, "ideVersion")
            .or_else(|| read_json_string_field(&product_json_path, "version"))
        {
            return version;
        }
    }

    let plist_path = official_antigravity_info_plist_path();
    if let Some(plist_path) = plist_path {
        let output = std::process::Command::new("plutil")
            .arg("-p")
            .arg(&plist_path)
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                if let Some(version) = parse_version_from_plutil_output(&text) {
                    return version;
                }
            }
        }
    }

    // 未读取到官方安装版本时，不再返回写死版本号。
    String::new()
}

fn build_official_ls_metadata_bytes() -> Vec<u8> {
    use crate::utils::protobuf::{encode_len_delim_field, encode_varint};

    let mut out = Vec::new();
    let push_str = |buf: &mut Vec<u8>, field_num: u32, value: &str| {
        if value.is_empty() {
            return;
        }
        buf.extend(encode_len_delim_field(field_num, value.as_bytes()));
    };

    // exa.codeium_common_pb.Metadata
    // 1=ide_name, 7=ide_version, 12=extension_name, 17=extension_path, 4=locale, 24=device_fingerprint
    push_str(&mut out, 1, "Antigravity");
    push_str(&mut out, 7, &official_antigravity_app_version());
    push_str(&mut out, 12, "antigravity");
    push_str(&mut out, 17, &official_antigravity_extension_path());
    push_str(
        &mut out,
        4,
        &std::env::var("LANG")
            .ok()
            .and_then(|v| v.split('.').next().map(|s| s.replace('_', "-")))
            .unwrap_or_else(|| "zh-CN".to_string()),
    );
    push_str(&mut out, 24, &uuid::Uuid::new_v4().to_string());

    // ensure the message is not empty to avoid LS startup error
    if out.is_empty() {
        out.extend(encode_varint(0));
    }

    out
}

fn build_uss_oauth_topic_bytes(token: &crate::models::token::TokenData) -> Vec<u8> {
    let expiry = token.expiry_timestamp.max(0);
    let oauth_info = crate::utils::protobuf::create_oauth_info(
        &token.access_token,
        &token.refresh_token,
        expiry,
    );
    let oauth_info_b64 = general_purpose::STANDARD.encode(oauth_info);

    // exa.unified_state_sync_pb.Topic
    // Topic.data -> map<string, Row>
    // Row.value stores base64(oauth_info)
    let row = crate::utils::protobuf::encode_string_field(1, &oauth_info_b64);
    let entry = [
        crate::utils::protobuf::encode_string_field(1, "oauthTokenInfoSentinelKey"),
        crate::utils::protobuf::encode_len_delim_field(2, &row),
    ]
    .concat();
    crate::utils::protobuf::encode_len_delim_field(1, &entry)
}

fn build_unified_state_sync_update_initial_state(topic_bytes: &[u8]) -> Vec<u8> {
    // exa.extension_server_pb.UnifiedStateSyncUpdate: field 1 = initial_state (Topic)
    crate::utils::protobuf::encode_len_delim_field(1, topic_bytes)
}

fn parse_official_ls_started_request(body: &[u8]) -> Result<OfficialLsStartedInfo, String> {
    let mut offset = 0usize;
    let mut https_port: Option<u16> = None;
    let mut http_port: Option<u16> = None;
    let mut lsp_port: Option<u16> = None;

    while offset < body.len() {
        let (tag, new_offset) = crate::utils::protobuf::read_varint(body, offset)?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;

        match (field_num, wire_type) {
            (1, 0) => {
                let (v, end) = crate::utils::protobuf::read_varint(body, new_offset)?;
                https_port = u16::try_from(v).ok();
                offset = end;
                continue;
            }
            (2, 0) => {
                let (v, end) = crate::utils::protobuf::read_varint(body, new_offset)?;
                lsp_port = u16::try_from(v).ok();
                offset = end;
                continue;
            }
            (5, 0) => {
                let (v, end) = crate::utils::protobuf::read_varint(body, new_offset)?;
                http_port = u16::try_from(v).ok();
                offset = end;
                continue;
            }
            _ => {}
        }

        offset = crate::utils::protobuf::skip_field(body, new_offset, wire_type)?;
    }

    Ok(OfficialLsStartedInfo {
        https_port: https_port
            .ok_or_else(|| "LanguageServerStarted 缺少 https_port".to_string())?,
        http_port: http_port.unwrap_or(0),
        lsp_port: lsp_port.unwrap_or(0),
    })
}

fn parse_subscribe_topic_from_connect_body(body: &[u8]) -> Result<String, String> {
    let payload = decode_connect_request_first_message(body)?;
    let mut offset = 0usize;
    while offset < payload.len() {
        let (tag, new_offset) = crate::utils::protobuf::read_varint(payload, offset)?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;
        if field_num == 1 && wire_type == 2 {
            let (len, content_offset) = crate::utils::protobuf::read_varint(payload, new_offset)?;
            let len = len as usize;
            let end = content_offset + len;
            if end > payload.len() {
                return Err("SubscribeToUnifiedStateSyncTopic 请求体长度非法".to_string());
            }
            let topic = std::str::from_utf8(&payload[content_offset..end])
                .map_err(|e| format!("topic UTF-8 解码失败: {}", e))?;
            return Ok(topic.to_string());
        }
        offset = crate::utils::protobuf::skip_field(payload, new_offset, wire_type)?;
    }
    Err("SubscribeToUnifiedStateSyncTopic 缺少 topic".to_string())
}

fn decode_connect_request_first_message(body: &[u8]) -> Result<&[u8], String> {
    if body.len() < 5 {
        return Err("Connect 请求体过短".to_string());
    }
    let flags = body[0];
    if flags & 0x01 != 0 {
        return Err("暂不支持压缩的 Connect 请求".to_string());
    }
    let len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
    let start = 5usize;
    let end = start + len;
    if end > body.len() {
        return Err("Connect 请求帧长度非法".to_string());
    }
    Ok(&body[start..end])
}

fn encode_connect_envelope(flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(flags);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn encode_connect_message_envelope(payload: &[u8]) -> Vec<u8> {
    encode_connect_envelope(0, payload)
}

fn encode_connect_end_ok_envelope() -> Vec<u8> {
    encode_connect_envelope(0x02, br#"{}"#)
}

fn binary_http_response(
    status_code: u16,
    status_text: &str,
    content_type: &str,
    body: &[u8],
) -> Vec<u8> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_code,
        status_text,
        content_type,
        body.len()
    );
    let mut resp = headers.into_bytes();
    resp.extend_from_slice(body);
    resp
}

fn extension_unary_response(request_content_type: &str, proto_body: &[u8]) -> Vec<u8> {
    let content_type_lc = request_content_type.to_ascii_lowercase();
    if content_type_lc.starts_with("application/connect+proto") {
        let body = encode_connect_message_envelope(proto_body);
        return binary_http_response(200, "OK", "application/connect+proto", &body);
    }

    binary_http_response(
        200,
        "OK",
        if request_content_type.is_empty() {
            "application/proto"
        } else {
            request_content_type
        },
        proto_body,
    )
}

fn chunked_http_stream_headers(status_code: u16, status_text: &str, content_type: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n",
        status_code, status_text, content_type
    )
    .into_bytes()
}

fn encode_chunked_bytes(payload: &[u8]) -> Vec<u8> {
    let mut out = format!("{:X}\r\n", payload.len()).into_bytes();
    out.extend_from_slice(payload);
    out.extend_from_slice(b"\r\n");
    out
}

fn encode_chunked_final() -> Vec<u8> {
    b"0\r\n\r\n".to_vec()
}

fn build_official_ls_local_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .no_proxy()
        // lgtm[rs/disabled-certificate-check] 仅连接官方 LS 的本地 HTTPS（127.0.0.1），使用自签名证书，非公网 TLS 校验放宽
        .danger_accept_invalid_certs(true)
        // 本地回环 HTTPS 连接统一关闭 SNI，避免服务端记录 IP-SNI 非法告警。
        .tls_sni(false)
        .build()
        .map_err(|e| format!("创建官方 LS 本地客户端失败: {}", e))
}

async fn post_json_to_official_ls(
    client: &reqwest::Client,
    base_url: &str,
    csrf_token: &str,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let resp = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("x-codeium-csrf-token", csrf_token)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("官方 LS 请求失败: {} ({})", e, path))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "官方 LS 返回错误: {} - {} ({})",
            status, text, path
        ));
    }

    resp.json::<Value>()
        .await
        .map_err(|e| format!("官方 LS 响应解析失败: {} ({})", e, path))
}

enum OfficialLsExtensionAction {
    Close(Vec<u8>),
    HoldStream {
        content_type: String,
        first_message: Vec<u8>,
        shutdown_notify: Arc<Notify>,
    },
}

async fn handle_official_ls_extension_connection<S>(
    mut stream: S,
    state: Arc<OfficialLsExtensionServerState>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let action = match read_http_request(&mut stream).await {
        Ok(raw) => match parse_http_request(&raw) {
            Ok(parsed) => route_official_ls_extension_request(parsed, state.clone()).await,
            Err(err) => OfficialLsExtensionAction::Close(text_response(
                400,
                "Bad Request",
                &err,
                "text/plain; charset=utf-8",
            )),
        },
        Err(err) => OfficialLsExtensionAction::Close(text_response(
            400,
            "Bad Request",
            &err,
            "text/plain; charset=utf-8",
        )),
    };

    match action {
        OfficialLsExtensionAction::Close(resp) => {
            let _ = stream.write_all(&resp).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
        }
        OfficialLsExtensionAction::HoldStream {
            content_type,
            first_message,
            shutdown_notify,
        } => {
            let headers = chunked_http_stream_headers(200, "OK", &content_type);
            let _ = stream.write_all(&headers).await;
            let _ = stream
                .write_all(&encode_chunked_bytes(&encode_connect_message_envelope(
                    &first_message,
                )))
                .await;
            let _ = stream.flush().await;

            shutdown_notify.notified().await;

            let _ = stream
                .write_all(&encode_chunked_bytes(&encode_connect_end_ok_envelope()))
                .await;
            let _ = stream.write_all(&encode_chunked_final()).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
        }
    }
}

async fn route_official_ls_extension_request(
    parsed: ParsedRequest,
    state: Arc<OfficialLsExtensionServerState>,
) -> OfficialLsExtensionAction {
    let path = normalize_path(&parsed.target);
    let method = parsed.method.to_ascii_uppercase();
    let content_type = parsed
        .headers
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| "application/proto".to_string());
    let request_csrf = parsed
        .headers
        .get("x-codeium-csrf-token")
        .cloned()
        .unwrap_or_default();

    if method == "OPTIONS" {
        return OfficialLsExtensionAction::Close(text_response(
            200,
            "OK",
            "",
            "text/plain; charset=utf-8",
        ));
    }
    if method != "POST" {
        return OfficialLsExtensionAction::Close(text_response(
            405,
            "Method Not Allowed",
            "Only POST is supported",
            "text/plain; charset=utf-8",
        ));
    }
    if request_csrf != state.csrf_token {
        return OfficialLsExtensionAction::Close(text_response(
            403,
            "Forbidden",
            "Invalid CSRF token",
            "text/plain; charset=utf-8",
        ));
    }

    if path_matches_rpc_method(&path, "LanguageServerStarted") {
        match parse_official_ls_started_request(&parsed.body) {
            Ok(started) => {
                if let Ok(mut guard) = state.started_sender.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(started);
                    }
                }
                return OfficialLsExtensionAction::Close(extension_unary_response(
                    &content_type,
                    &[],
                ));
            }
            Err(err) => {
                crate::modules::logger::log_error(&format!(
                    "[WakeupGateway] 官方 LS LanguageServerStarted 解析失败: {}",
                    err
                ));
                return OfficialLsExtensionAction::Close(text_response(
                    400,
                    "Bad Request",
                    &err,
                    "text/plain; charset=utf-8",
                ));
            }
        }
    }

    if path_matches_rpc_method(&path, "SubscribeToUnifiedStateSyncTopic") {
        let topic = match parse_subscribe_topic_from_connect_body(&parsed.body) {
            Ok(v) => v,
            Err(err) => {
                crate::modules::logger::log_error(&format!(
                    "[WakeupGateway] 官方 LS SubscribeToUnifiedStateSyncTopic 解析失败: {}",
                    err
                ));
                return OfficialLsExtensionAction::Close(text_response(
                    400,
                    "Bad Request",
                    &err,
                    "text/plain; charset=utf-8",
                ));
            }
        };

        let topic_bytes = match topic.as_str() {
            "uss-oauth" => &state.uss_oauth_topic_bytes,
            "uss-enterprisePreferences" | "uss-agentPreferences" => &state.empty_topic_bytes,
            _ => &state.empty_topic_bytes,
        };
        let update = build_unified_state_sync_update_initial_state(topic_bytes);

        return OfficialLsExtensionAction::HoldStream {
            content_type: "application/connect+proto".to_string(),
            first_message: update,
            shutdown_notify: state.shutdown_notify.clone(),
        };
    }

    // Minimal unary implementations to keep the official LS alive for wakeup usage.
    if path_matches_rpc_method(&path, "IsAgentManagerEnabled") {
        let body = [
            crate::utils::protobuf::encode_varint((1 << 3) as u64),
            vec![1u8],
        ]
        .concat();
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &body));
    }

    // 官方扩展会周期性探测 Chrome DevTools MCP URL；对唤醒场景返回空字符串即可。
    if path_matches_rpc_method(&path, "GetChromeDevtoolsMcpUrl") {
        let body = crate::utils::protobuf::encode_string_field(1, "");
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &body));
    }

    // 唤醒场景不提供终端 shell 能力，返回默认值（false/empty）。
    if path_matches_rpc_method(&path, "CheckTerminalShellSupport") {
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &[]));
    }

    // 唤醒场景不使用浏览器 onboarding，返回默认端口 0。
    if path_matches_rpc_method(&path, "GetBrowserOnboardingPort") {
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &[]));
    }

    let empty_ok_paths = [
        "/PushUnifiedStateSyncUpdate",
        "/GetSecretValue",
        "/StoreSecretValue",
        "/LogEvent",
        "/RecordError",
        "/RestartUserStatusUpdater",
        "/OpenSetting",
        "/PlaySound",
        "/BroadcastConversationDeletion",
    ];

    if empty_ok_paths.iter().any(|suffix| {
        path.ends_with(suffix) || path_matches_rpc_method(&path, suffix.trim_start_matches('/'))
    }) {
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &[]));
    }

    crate::modules::logger::log_warn(&format!(
        "[WakeupGateway] 官方 LS 调用了未实现扩展接口: {}",
        path
    ));
    OfficialLsExtensionAction::Close(binary_http_response(
        200,
        "OK",
        if content_type.is_empty() {
            "application/proto"
        } else {
            &content_type
        },
        &[],
    ))
}

async fn run_official_ls_extension_server(
    listener: TcpListener,
    state: Arc<OfficialLsExtensionServerState>,
) {
    loop {
        tokio::select! {
            _ = state.shutdown_notify.notified() => {
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        tokio::spawn(async move {
                            handle_official_ls_extension_connection(stream, state).await;
                        });
                    }
                    Err(err) => {
                        crate::modules::logger::log_error(&format!(
                            "[WakeupGateway] 官方 LS 扩展服务 accept 失败: {}",
                            err
                        ));
                        break;
                    }
                }
            }
        }
    }
}

async fn start_official_ls_extension_server(
    token: &crate::models::token::TokenData,
) -> Result<OfficialLsExtensionServerHandle, String> {
    let listener = timeout(
        OFFICIAL_LS_EXTENSION_BIND_TIMEOUT,
        TcpListener::bind("127.0.0.1:0"),
    )
    .await
    .map_err(|_| {
        format!(
            "启动官方 LS 扩展服务失败（绑定端口超时，>{}s）",
            OFFICIAL_LS_EXTENSION_BIND_TIMEOUT.as_secs()
        )
    })?
    .map_err(|e| format!("启动官方 LS 扩展服务失败（绑定端口）: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取官方 LS 扩展服务端口失败: {}", e))?
        .port();
    let csrf_token = uuid::Uuid::new_v4().to_string();
    let (started_sender, started_receiver) = oneshot::channel();
    let shutdown_notify = Arc::new(Notify::new());
    let state = Arc::new(OfficialLsExtensionServerState {
        csrf_token: csrf_token.clone(),
        uss_oauth_topic_bytes: build_uss_oauth_topic_bytes(token),
        empty_topic_bytes: Vec::new(),
        started_sender: Mutex::new(Some(started_sender)),
        shutdown_notify: shutdown_notify.clone(),
    });
    let task = tokio::spawn(run_official_ls_extension_server(listener, state));

    Ok(OfficialLsExtensionServerHandle {
        port,
        csrf_token,
        started_receiver,
        shutdown_notify,
        task,
    })
}

fn spawn_ls_log_task<R>(reader: R, tag: &'static str) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            crate::modules::logger::log_info(&format!(
                "[WakeupGateway][OfficialLS][{}] {}",
                tag, trimmed
            ));
        }
    })
}

impl OfficialLsProcessHandle {
    async fn shutdown(&mut self) {
        self.extension_server.shutdown_notify.notify_waiters();
        self.extension_server.task.abort();

        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }

        let _ = self.child.start_kill();
        let _ = timeout(Duration::from_secs(2), self.child.wait()).await;
    }
}

async fn start_official_ls_process(
    account_id: &str,
    token: &crate::models::token::TokenData,
) -> Result<OfficialLsProcessHandle, String> {
    let binary_path = official_ls_binary_path()?;
    let mut extension_server = start_official_ls_extension_server(token).await?;
    let ls_csrf = uuid::Uuid::new_v4().to_string();
    let cloud_code_endpoint = official_ls_cloud_code_endpoint(token);
    // 对齐官方扩展：app_data_dir 使用固定 IDE 级目录（默认 antigravity），而非按账号拆分。
    let app_data_dir = official_ls_app_data_dir_name();

    let mut cmd = Command::new(&binary_path);
    cmd.arg("--enable_lsp");
    if official_ls_requires_random_port() {
        cmd.arg("--random_port");
    }
    cmd.arg("--csrf_token")
        .arg(&ls_csrf)
        .arg("--extension_server_port")
        .arg(extension_server.port.to_string())
        .arg("--extension_server_csrf_token")
        .arg(&extension_server.csrf_token)
        .arg("--cloud_code_endpoint")
        .arg(cloud_code_endpoint)
        .arg("--app_data_dir")
        .arg(&app_data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动官方 Language Server 失败: {}", e))?;

    if let Some(stdout) = child.stdout.take() {
        crate::modules::logger::log_info(&format!(
            "[WakeupGateway] 官方 LS 已启动进程，等待回调: account_id={}",
            account_id
        ));
        let stdout_task = spawn_ls_log_task(stdout, "stdout");
        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| spawn_ls_log_task(stderr, "stderr"));

        if let Some(mut stdin) = child.stdin.take() {
            let metadata = build_official_ls_metadata_bytes();
            stdin
                .write_all(&metadata)
                .await
                .map_err(|e| format!("写入官方 LS 初始 Metadata 失败: {}", e))?;
            let _ = stdin.shutdown().await;
        }

        let started = timeout(
            OFFICIAL_LS_START_TIMEOUT,
            &mut extension_server.started_receiver,
        )
        .await
        .map_err(|_| "等待官方 LS LanguageServerStarted 超时".to_string())?
        .map_err(|_| "官方 LS LanguageServerStarted 通知通道已关闭".to_string())?;

        crate::modules::logger::log_info(&format!(
            "[WakeupGateway] 官方 LS 启动完成: https_port={}, http_port={}, lsp_port={}",
            started.https_port, started.http_port, started.lsp_port
        ));

        Ok(OfficialLsProcessHandle {
            child,
            stdout_task: Some(stdout_task),
            stderr_task,
            extension_server,
            started,
            ls_csrf_token: ls_csrf,
        })
    } else {
        let _ = child.start_kill();
        Err("官方 LS stdout 不可用".to_string())
    }
}

async fn ensure_wakeup_account_token(
    account_id: &str,
) -> Result<
    (
        crate::models::account::Account,
        crate::models::token::TokenData,
    ),
    String,
> {
    let mut account = crate::modules::account::load_account(account_id)?;
    let token = crate::modules::oauth::ensure_fresh_token(&account.token).await?;
    if token.access_token != account.token.access_token
        || token.refresh_token != account.token.refresh_token
        || token.expiry_timestamp != account.token.expiry_timestamp
        || token.project_id != account.token.project_id
        || token.is_gcp_tos != account.token.is_gcp_tos
    {
        account.token = token.clone();
        let _ = crate::modules::account::save_account(&account);
    }
    Ok((account, token))
}

async fn handle_prepare_start_context(body: &[u8]) -> Result<Value, (u16, String)> {
    let req: PrepareStartContextRequest = serde_json::from_slice(body)
        .map_err(|e| (400, format!("prepareStartContext 请求体无效: {}", e)))?;

    let account_id = req.account_id.trim();
    if account_id.is_empty() {
        return Err((400, "缺少 accountId".to_string()));
    }

    let ctx = PreparedStartContext {
        account_id: account_id.to_string(),
        prepared_at: Utc::now(),
    };

    let mut guard = pending_start_contexts()
        .lock()
        .map_err(|_| (500, "准备上下文锁失败".to_string()))?;
    guard.push_back(ctx);

    Ok(json!({}))
}

async fn handle_health_check() -> Result<Value, (u16, String)> {
    Ok(json!({ "ok": true }))
}

async fn handle_start_cascade(body: &[u8]) -> Result<Value, (u16, String)> {
    let payload = parse_json_object_body(body, "StartCascade")?;
    let prepared = pop_prepared_start_context()
        .map_err(|e| (500, e))?
        .ok_or_else(|| {
            (
                400,
                "缺少账号上下文，请先调用内部 prepareStartContext".to_string(),
            )
        })?;

    let (start_resp, session) = start_official_ls_cascade_session(prepared, &payload)
        .await
        .map_err(|e| (500, e))?;

    let cascade_id = start_resp
        .get("cascadeId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| (500, "官方 LS StartCascade 未返回 cascadeId".to_string()))?
        .to_string();

    let insert_ok = if let Ok(mut guard) = official_ls_sessions().lock() {
        guard.insert(cascade_id, session.clone());
        true
    } else {
        false
    };
    if !insert_ok {
        shutdown_official_ls_session(&session).await;
        return Err((500, "官方 LS 会话映射锁失败".to_string()));
    }

    Ok(start_resp)
}

async fn handle_send_user_cascade_message(body: &[u8]) -> Result<Value, (u16, String)> {
    let payload = parse_json_object_body(body, "SendUserCascadeMessage")?;
    let cascade_id = extract_required_cascade_id(&payload, "SendUserCascadeMessage")?;
    let session = get_official_ls_session(&cascade_id).map_err(|e| {
        if e.starts_with("会话不存在:") {
            (404, e)
        } else {
            (500, e)
        }
    })?;

    proxy_official_ls_session_json_request(&session, SEND_USER_CASCADE_MESSAGE_PATH, &payload)
        .await
        .map_err(|e| (500, e))
}

async fn handle_get_cascade_trajectory(body: &[u8]) -> Result<Value, (u16, String)> {
    let payload = parse_json_object_body(body, "GetCascadeTrajectory")?;
    let cascade_id = extract_required_cascade_id(&payload, "GetCascadeTrajectory")?;
    let session = get_official_ls_session(&cascade_id).map_err(|e| {
        if e.starts_with("会话不存在:") {
            (404, e)
        } else {
            (500, e)
        }
    })?;

    proxy_official_ls_session_json_request(&session, GET_CASCADE_TRAJECTORY_PATH, &payload)
        .await
        .map_err(|e| (500, e))
}

async fn handle_delete_cascade_trajectory(body: &[u8]) -> Result<Value, (u16, String)> {
    let payload = parse_json_object_body(body, "DeleteCascadeTrajectory")?;
    let cascade_id = extract_required_cascade_id(&payload, "DeleteCascadeTrajectory")?;
    let session = remove_official_ls_session(&cascade_id)
        .map_err(|e| (500, e))?
        .ok_or_else(|| (404, format!("会话不存在: {}", cascade_id)))?;

    let proxy_result =
        proxy_official_ls_session_json_request(&session, DELETE_CASCADE_TRAJECTORY_PATH, &payload)
            .await;

    crate::modules::logger::log_info(&format!(
        "[WakeupGateway] 清理官方 LS 会话: cascade_id={}, account_id={}",
        cascade_id, session.account_id
    ));
    shutdown_official_ls_session(&session).await;

    proxy_result.map_err(|e| (500, e))
}

async fn route_request(parsed: ParsedRequest) -> Vec<u8> {
    let path = normalize_path(&parsed.target);
    if parsed.method.eq_ignore_ascii_case("OPTIONS") {
        return options_response();
    }
    if !parsed.method.eq_ignore_ascii_case("POST") {
        return json_response(
            405,
            "Method Not Allowed",
            &json!({ "error": "Only POST is supported" }),
        );
    }

    let result = match path.as_str() {
        INTERNAL_HEALTH_CHECK_PATH => handle_health_check().await,
        INTERNAL_PREPARE_START_CONTEXT_PATH => handle_prepare_start_context(&parsed.body).await,
        START_CASCADE_PATH => handle_start_cascade(&parsed.body).await,
        SEND_USER_CASCADE_MESSAGE_PATH => handle_send_user_cascade_message(&parsed.body).await,
        GET_CASCADE_TRAJECTORY_PATH => handle_get_cascade_trajectory(&parsed.body).await,
        DELETE_CASCADE_TRAJECTORY_PATH => handle_delete_cascade_trajectory(&parsed.body).await,
        _ => Err((404, format!("Unknown path: {}", path))),
    };

    match result {
        Ok(body) => json_response(200, "OK", &body),
        Err((status, message)) => {
            let status_text = match status {
                400 => "Bad Request",
                404 => "Not Found",
                405 => "Method Not Allowed",
                _ => "Internal Server Error",
            };
            json_response(status, status_text, &json!({ "error": message }))
        }
    }
}

async fn handle_connection<S>(mut stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let response = match read_http_request(&mut stream).await {
        Ok(raw) => match parse_http_request(&raw) {
            Ok(parsed) => route_request(parsed).await,
            Err(err) => json_response(400, "Bad Request", &json!({ "error": err })),
        },
        Err(err) => json_response(400, "Bad Request", &json!({ "error": err })),
    };

    let _ = stream.write_all(&response).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

fn build_tls_acceptor() -> Result<TlsAcceptor, String> {
    let certified =
        generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .map_err(|e| format!("生成本地 TLS 证书失败: {}", e))?;

    let cert_der: Vec<u8> = certified.cert.der().to_vec();
    let key_der: Vec<u8> = certified.key_pair.serialize_der();

    let certs = vec![CertificateDer::from(cert_der)];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("创建 TLS 配置失败: {}", e))?;
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

async fn run_gateway_server(listener: TcpListener, tls_acceptor: TlsAcceptor) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let acceptor = tls_acceptor.clone();
                tokio::spawn(async move {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            handle_connection(tls_stream).await;
                        }
                        Err(err) => {
                            crate::modules::logger::log_error(&format!(
                                "[WakeupGateway] TLS 握手失败: {}",
                                err
                            ));
                        }
                    }
                });
            }
            Err(err) => {
                crate::modules::logger::log_error(&format!(
                    "[WakeupGateway] accept 失败，网关停止: {}",
                    err
                ));
                break;
            }
        }
    }
}

pub async fn ensure_local_gateway_started() -> Result<String, String> {
    let store = local_gateway_store();
    {
        let guard = store.lock().await;
        if let Some(base_url) = guard.as_ref() {
            return Ok(base_url.clone());
        }
    }

    let _start_guard = local_gateway_start_lock().lock().await;
    {
        let guard = store.lock().await;
        if let Some(base_url) = guard.as_ref() {
            return Ok(base_url.clone());
        }
    }

    let listener = timeout(LOCAL_GATEWAY_BIND_TIMEOUT, TcpListener::bind("127.0.0.1:0"))
        .await
        .map_err(|_| {
            format!(
                "启动本地唤醒网关失败（绑定端口超时，>{}s）",
                LOCAL_GATEWAY_BIND_TIMEOUT.as_secs()
            )
        })?
        .map_err(|e| format!("启动本地唤醒网关失败（绑定端口）: {}", e))?;

    let tls_acceptor = timeout(
        LOCAL_GATEWAY_TLS_SETUP_TIMEOUT,
        tokio::task::spawn_blocking(build_tls_acceptor),
    )
    .await
    .map_err(|_| {
        format!(
            "启动本地唤醒网关失败（TLS 初始化超时，>{}s）",
            LOCAL_GATEWAY_TLS_SETUP_TIMEOUT.as_secs()
        )
    })?
    .map_err(|e| format!("启动本地唤醒网关失败（TLS 初始化任务异常）: {}", e))??;

    let port = listener
        .local_addr()
        .map_err(|e| format!("读取本地唤醒网关端口失败: {}", e))?
        .port();
    let base_url = format!("https://127.0.0.1:{}", port);
    crate::modules::logger::log_info(&format!("[WakeupGateway] 本地网关已启动: {}", base_url));
    tokio::spawn(run_gateway_server(listener, tls_acceptor));

    let mut guard = store.lock().await;
    if let Some(existing) = guard.as_ref() {
        return Ok(existing.clone());
    }
    *guard = Some(base_url.clone());
    Ok(base_url)
}

pub async fn clear_local_gateway_base_url_cache() {
    let store = local_gateway_store();
    let mut guard = store.lock().await;
    if let Some(current) = guard.take() {
        crate::modules::logger::log_warn(&format!(
            "[WakeupGateway] 清理本地网关地址缓存: {}",
            current
        ));
    }
}
