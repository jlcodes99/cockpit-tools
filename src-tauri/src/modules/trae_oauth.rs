use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tiny_http::{Header, Response, Server, StatusCode};
use url::Url;
use uuid::Uuid;

use crate::models::trae::{TraeImportPayload, TraeOAuthStartResponse};
use crate::modules::{config, device, logger, trae_account};

const OAUTH_TIMEOUT_SECONDS: i64 = 600;
const OAUTH_POLL_INTERVAL_MS: u64 = 250;
const OAUTH_STATE_FILE: &str = "trae_oauth_pending.json";
const CALLBACK_PATH: &str = "/authorize";
const TRAE_AUTHORIZATION_PATH: &str = "/authorization";
const TRAE_AUTH_CLIENT_ID: &str = "ono9krqynydwx5";
const TRAE_DEFAULT_PLUGIN_VERSION: &str = "local";
const TRAE_DEFAULT_DEVICE_ID: &str = "0";
const TRAE_DEFAULT_APP_TYPE: &str = "stable";

const TRAE_LOGIN_GUIDANCE_URLS: [&str; 3] = [
    "https://api.marscode.com/cloudide/api/v3/trae/GetLoginGuidance",
    "https://api.trae.ai/cloudide/api/v3/trae/GetLoginGuidance",
    "https://www.trae.ai/cloudide/api/v3/trae/GetLoginGuidance",
];

const TRAE_EXCHANGE_TOKEN_PATH: &str = "/cloudide/api/v3/trae/oauth/ExchangeToken";
const TRAE_GET_USER_INFO_PATH: &str = "/cloudide/api/v3/trae/GetUserInfo";
const TRAE_EXCHANGE_CLIENT_SECRET: &str = "-";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraeCallbackPayload {
    refresh_token: String,
    login_host: String,
    login_region: Option<String>,
    login_trace_id: Option<String>,
    cloudide_token: Option<String>,
    raw_query: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOAuthState {
    login_id: String,
    login_trace_id: String,
    callback_port: u16,
    callback_url: String,
    verification_uri: String,
    login_host: String,
    expires_at: i64,
    cancelled: bool,
    callback_result: Option<Result<TraeCallbackPayload, String>>,
}

#[derive(Debug, Clone, Default)]
struct TraeProductInfo {
    plugin_version: Option<String>,
    app_version: Option<String>,
    app_type: Option<String>,
}

#[derive(Debug, Clone)]
struct TraeLoginContext {
    plugin_version: String,
    machine_id: String,
    device_id: String,
    x_device_brand: String,
    x_device_type: String,
    x_os_version: String,
    x_env: String,
    x_app_version: String,
    x_app_type: String,
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
                "[Trae OAuth] 读取持久化登录状态失败，已忽略: {}",
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
        logger::log_warn(&format!("[Trae OAuth] 持久化登录状态失败，已忽略: {}", err));
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

fn extract_json_value<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for key in path {
        current = current.as_object()?.get(*key)?;
    }
    Some(current)
}

fn pick_string(root: &Value, paths: &[&[&str]]) -> Option<String> {
    for path in paths {
        if let Some(value) = extract_json_value(root, path) {
            if let Some(text) = value.as_str() {
                if let Some(normalized) = normalize_non_empty(Some(text)) {
                    return Some(normalized);
                }
            }
            if let Some(num) = value.as_i64() {
                return Some(num.to_string());
            }
            if let Some(num) = value.as_u64() {
                return Some(num.to_string());
            }
        }
    }
    None
}

fn pick_i64(root: &Value, paths: &[&[&str]]) -> Option<i64> {
    for path in paths {
        if let Some(value) = extract_json_value(root, path) {
            if let Some(num) = value.as_i64() {
                return Some(num);
            }
            if let Some(num) = value.as_u64() {
                if num <= i64::MAX as u64 {
                    return Some(num as i64);
                }
            }
            if let Some(text) = value.as_str() {
                if let Ok(parsed) = text.trim().parse::<i64>() {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

fn is_numeric_id(value: &str, min_len: usize, max_len: usize) -> bool {
    if value.len() < min_len || value.len() > max_len {
        return false;
    }
    value.chars().all(|ch| ch.is_ascii_digit())
}

fn normalize_device_id(value: Option<&str>) -> Option<String> {
    let normalized = normalize_non_empty(value)?;
    if !is_numeric_id(normalized.as_str(), 8, 24) {
        return None;
    }
    Some(normalized)
}

fn pick_storage_string(storage_root: Option<&Value>, keys: &[&str]) -> Option<String> {
    let obj = storage_root?.as_object()?;
    for key in keys {
        let Some(value) = obj.get(*key) else {
            continue;
        };
        if let Some(text) = value.as_str() {
            if let Some(normalized) = normalize_non_empty(Some(text)) {
                return Some(normalized);
            }
        }
        if let Some(num) = value.as_i64() {
            return Some(num.to_string());
        }
        if let Some(num) = value.as_u64() {
            return Some(num.to_string());
        }
    }
    None
}

fn parse_json_file(path: &Path) -> Option<Value> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&content).ok()
}

fn build_trae_product_file_candidates(base_path: &Path) -> Vec<PathBuf> {
    let mut app_roots: Vec<PathBuf> = Vec::new();
    let base_path_string = base_path.to_string_lossy().to_string();

    if let Some(app_idx) = base_path_string.find(".app") {
        app_roots.push(PathBuf::from(&base_path_string[..app_idx + 4]));
    }

    if base_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("Trae.app"))
        .unwrap_or(false)
    {
        app_roots.push(base_path.to_path_buf());
    }

    if app_roots.is_empty() {
        app_roots.push(base_path.to_path_buf());
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut candidates = Vec::new();
    for root in app_roots {
        let files = [
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("product.json"),
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("package.json"),
            root.join("resources").join("app").join("product.json"),
            root.join("resources").join("app").join("package.json"),
            root.join("product.json"),
            root.join("package.json"),
        ];
        for file in files {
            let key = file.to_string_lossy().to_string();
            if seen.contains(key.as_str()) {
                continue;
            }
            seen.insert(key);
            candidates.push(file);
        }
    }
    candidates
}

fn trae_product_base_paths() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let configured_path = config::get_user_config().trae_app_path.trim().to_string();
    if !configured_path.is_empty() {
        candidates.push(PathBuf::from(configured_path));
    }

    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from("/Applications/Trae.app"));
        candidates.push(PathBuf::from("/Applications/Trae.app/Contents"));
        candidates.push(PathBuf::from("/Applications/Trae.app/Contents/MacOS/Trae"));
        candidates.push(PathBuf::from(
            "/Applications/Trae.app/Contents/MacOS/Electron",
        ));
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(&local_app_data)
                    .join("Programs")
                    .join("Trae")
                    .join("Trae.exe"),
            );
            candidates.push(PathBuf::from(local_app_data).join("Programs").join("Trae"));
        }
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            candidates.push(PathBuf::from(&program_files).join("Trae").join("Trae.exe"));
            candidates.push(PathBuf::from(program_files).join("Trae"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/trae"));
        candidates.push(PathBuf::from("/usr/local/bin/trae"));
        candidates.push(PathBuf::from("/opt/trae/trae"));
        candidates.push(PathBuf::from("/opt/Trae"));
    }

    let mut dedup: HashSet<String> = HashSet::new();
    let mut output = Vec::new();
    for item in candidates {
        let key = item.to_string_lossy().to_string();
        if dedup.contains(key.as_str()) {
            continue;
        }
        dedup.insert(key);
        output.push(item);
    }
    output
}

fn read_trae_product_info(path: &Path) -> Option<TraeProductInfo> {
    let root = parse_json_file(path)?;
    let plugin_version = pick_string(
        &root,
        &[
            &["tronBuildVersion"],
            &["buildVersion"],
            &["productVersion"],
            &["version"],
        ],
    );
    let app_version = pick_string(&root, &[&["appVersion"], &["productVersion"], &["version"]]);
    let app_type = pick_string(&root, &[&["quality"]]).map(|value| value.to_lowercase());

    if plugin_version.is_none() && app_version.is_none() && app_type.is_none() {
        return None;
    }

    Some(TraeProductInfo {
        plugin_version,
        app_version,
        app_type,
    })
}

fn detect_trae_product_info() -> TraeProductInfo {
    for base_path in trae_product_base_paths() {
        for candidate in build_trae_product_file_candidates(base_path.as_path()) {
            if let Some(info) = read_trae_product_info(candidate.as_path()) {
                return info;
            }
        }
    }
    TraeProductInfo::default()
}

fn read_trae_storage_root() -> Option<Value> {
    let path = trae_account::get_default_trae_storage_path().ok()?;
    parse_json_file(path.as_path())
}

fn recent_trae_log_files() -> Vec<PathBuf> {
    let logs_root = match trae_account::get_default_trae_data_dir() {
        Ok(path) => path.join("logs"),
        Err(_) => return Vec::new(),
    };
    let entries = match fs::read_dir(logs_root) {
        Ok(iter) => iter,
        Err(_) => return Vec::new(),
    };

    let mut log_dirs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| path.is_dir())
        .collect();
    log_dirs.sort_by(|left, right| right.to_string_lossy().cmp(&left.to_string_lossy()));

    let mut files = Vec::new();
    for dir in log_dirs.into_iter().take(10) {
        let candidates = [
            dir.join("sharedprocess.log"),
            dir.join("main.log"),
            dir.join("window1").join("renderer.log"),
            dir.join("window1")
                .join("exthost")
                .join("trae.ai-code-completion")
                .join("Trae AI Code Client.log"),
            dir.join("window1")
                .join("exthost")
                .join("trae.ai-code-completion")
                .join("Trae AI Code Completion.log"),
            dir.join("window1")
                .join("exthost")
                .join("trae.ai-code-completion")
                .join("completion.log"),
        ];
        for file in candidates {
            if file.is_file() {
                files.push(file);
            }
        }
    }
    files
}

fn decode_url_component(raw: &str) -> String {
    match urlencoding::decode(raw) {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => raw.to_string(),
    }
}

fn extract_device_id_from_logs() -> Option<String> {
    let patterns = [
        r"resolve device_id:\s*([0-9]{8,24})",
        r#""device_id"\s*:\s*"([0-9]{8,24})""#,
        r#"device_id[:=]\s*"?(?:\s*)([0-9]{8,24})"#,
        r#""X-Device-Id"\s*:\s*"([0-9]{8,24})""#,
    ];

    for file in recent_trae_log_files() {
        let bytes = match fs::read(file) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let text = String::from_utf8_lossy(&bytes);

        for pattern in patterns {
            let regex = match Regex::new(pattern) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let mut candidate: Option<String> = None;
            for capture in regex.captures_iter(text.as_ref()) {
                if let Some(found) = capture.get(1) {
                    candidate = normalize_device_id(Some(found.as_str()));
                }
            }
            if let Some(device_id) = candidate {
                return Some(device_id);
            }
        }
    }

    None
}

fn extract_device_brand_from_logs() -> Option<String> {
    let patterns = [
        r#""device_model"\s*:\s*"([^"]+)""#,
        r#""X-Device-Brand"\s*:\s*"([^"]+)""#,
        r#"device_brand:\s*([A-Za-z0-9,%._+-]+)"#,
    ];

    for file in recent_trae_log_files() {
        let bytes = match fs::read(file) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let text = String::from_utf8_lossy(&bytes);

        for pattern in patterns {
            let regex = match Regex::new(pattern) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let mut candidate: Option<String> = None;
            for capture in regex.captures_iter(text.as_ref()) {
                if let Some(found) = capture.get(1) {
                    let decoded = decode_url_component(found.as_str());
                    candidate = normalize_non_empty(Some(decoded.as_str()));
                }
            }
            if let Some(brand) = candidate {
                return Some(brand);
            }
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn run_command_and_read_stdout(cmd: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    normalize_non_empty(Some(text.as_str()))
}

fn detect_device_type() -> String {
    #[cfg(target_os = "macos")]
    {
        return "mac".to_string();
    }
    #[cfg(target_os = "windows")]
    {
        return "windows".to_string();
    }
    #[cfg(target_os = "linux")]
    {
        return "linux".to_string();
    }
    #[allow(unreachable_code)]
    "unknown".to_string()
}

fn detect_os_version(device_type: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        if let Some(version) = run_command_and_read_stdout("sw_vers", &["-productVersion"]) {
            return format!("macOS {}", version);
        }
    }

    if let Some(version) =
        sysinfo::System::long_os_version().and_then(|raw| normalize_non_empty(Some(raw.as_str())))
    {
        return version;
    }

    if device_type == "mac" {
        return "macOS".to_string();
    }
    device_type.to_string()
}

fn detect_device_brand(device_type: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        if let Some(model) = run_command_and_read_stdout("sysctl", &["-n", "hw.model"]) {
            return model;
        }
    }

    if let Some(model) = extract_device_brand_from_logs() {
        return model;
    }

    if device_type == "mac" {
        return "Mac".to_string();
    }
    if device_type == "windows" {
        return "Windows".to_string();
    }
    if device_type == "linux" {
        return "Linux".to_string();
    }
    "unknown".to_string()
}

fn collect_trae_login_context() -> TraeLoginContext {
    let storage_root = read_trae_storage_root();
    let product_info = detect_trae_product_info();

    let plugin_version = product_info
        .plugin_version
        .or_else(|| pick_storage_string(storage_root.as_ref(), &["iCubeLastVersion"]))
        .unwrap_or_else(|| TRAE_DEFAULT_PLUGIN_VERSION.to_string());

    let app_version = product_info
        .app_version
        .or_else(|| pick_storage_string(storage_root.as_ref(), &["appVersion", "iCubeLastVersion"]))
        .unwrap_or_else(|| plugin_version.clone());

    let app_type = product_info
        .app_type
        .or_else(|| pick_storage_string(storage_root.as_ref(), &["quality", "appType"]))
        .map(|value| value.to_lowercase())
        .unwrap_or_else(|| TRAE_DEFAULT_APP_TYPE.to_string());

    let machine_id = pick_storage_string(
        storage_root.as_ref(),
        &["telemetry.machineId", "machine_id", "x_machine_id"],
    )
    .unwrap_or_else(device::get_service_machine_id);

    let device_id = pick_storage_string(
        storage_root.as_ref(),
        &[
            "device_id",
            "deviceId",
            "x_device_id",
            "iCubeDeviceId",
            "iCubeDeviceID",
            "icube.device_id",
        ],
    )
    .as_deref()
    .and_then(|value| normalize_device_id(Some(value)))
    .or_else(extract_device_id_from_logs)
    .unwrap_or_else(|| TRAE_DEFAULT_DEVICE_ID.to_string());

    let x_device_type = detect_device_type();
    let x_device_brand = detect_device_brand(x_device_type.as_str());
    let x_os_version = detect_os_version(x_device_type.as_str());
    let x_env = pick_storage_string(
        storage_root.as_ref(),
        &["ai_assistant.request.env", "ai_assistant.env", "x_env"],
    )
    .unwrap_or_default();

    TraeLoginContext {
        plugin_version,
        machine_id,
        device_id,
        x_device_brand,
        x_device_type,
        x_os_version,
        x_env,
        x_app_version: app_version,
        x_app_type: app_type,
    }
}

fn mask_id_for_log(value: &str) -> String {
    let normalized = normalize_non_empty(Some(value)).unwrap_or_default();
    if normalized.len() <= 12 {
        return normalized;
    }
    format!(
        "{}***{}",
        &normalized[..6],
        &normalized[normalized.len() - 4..]
    )
}

fn ensure_https_url(raw: &str) -> Result<Url, String> {
    let normalized =
        normalize_non_empty(Some(raw)).ok_or_else(|| "Trae 登录地址为空".to_string())?;
    let with_scheme = if normalized.starts_with("http://") || normalized.starts_with("https://") {
        normalized
    } else {
        format!("https://{}", normalized.trim_start_matches('/'))
    };
    Url::parse(with_scheme.as_str()).map_err(|e| format!("解析 Trae 登录地址失败: {}", e))
}

fn find_available_callback_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("分配 Trae OAuth 回调端口失败: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取 Trae OAuth 回调端口失败: {}", e))?
        .port();
    drop(listener);
    Ok(port)
}

fn parse_query_map(raw_query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(raw_query.as_bytes())
        .map(|(key, value)| (key.to_string(), value.to_string()))
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
            CALLBACK_PATH,
            trimmed.trim_start_matches('?')
        )
        .as_str(),
    )
    .map_err(|e| format!("回调链接格式无效: {}", e))
}

fn pick_query_value(params: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = params.get(*key) {
            if let Some(normalized) = normalize_non_empty(Some(value.as_str())) {
                return Some(normalized);
            }
        }
    }
    None
}

fn parse_bool_like(value: Option<&str>) -> Option<bool> {
    let normalized = normalize_non_empty(value)?;
    let lower = normalized.to_lowercase();
    if lower == "1" || lower == "true" || lower == "yes" {
        return Some(true);
    }
    if lower == "0" || lower == "false" || lower == "no" {
        return Some(false);
    }
    None
}

fn extract_cloudide_token(params: &HashMap<String, String>) -> Option<String> {
    if let Some(token) = pick_query_value(
        params,
        &[
            "x-cloudide-token",
            "xCloudideToken",
            "accessToken",
            "access_token",
            "token",
        ],
    ) {
        return Some(token);
    }

    let user_jwt = pick_query_value(params, &["userJwt", "user_jwt"])?;
    let parsed: Value = serde_json::from_str(user_jwt.as_str()).ok()?;
    pick_string(
        &parsed,
        &[
            &["Token"],
            &["token"],
            &["AccessToken"],
            &["accessToken"],
            &["access_token"],
        ],
    )
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&#39;")
}

fn callback_success_html() -> &'static str {
    r#"<!doctype html><html><head><meta charset="utf-8"><title>Trae Login</title></head><body><h2>Trae 登录回调已完成</h2><p>可以返回 Cockpit Tools。</p></body></html>"#
}

fn callback_pending_html() -> &'static str {
    r#"<!doctype html><html><head><meta charset="utf-8"><title>Trae Login</title></head><body><h2>正在解析授权结果…</h2><p id="hint">请稍候，页面将自动完成回调。</p><script>(function(){if(window.location.hash&&window.location.hash.length>1){var hash=window.location.hash.slice(1);var target=window.location.origin+window.location.pathname+'?'+hash;window.location.replace(target);return;}document.getElementById('hint').textContent='未检测到授权参数，请完成登录后重试。';})();</script></body></html>"#
}

fn callback_failure_html(message: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Trae Login</title></head><body><h2>Trae 登录回调失败</h2><p>{}</p></body></html>",
        escape_html(message)
    )
}

fn clear_pending_if_matches(login_id: &str) {
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

fn set_callback_result_if_matches(login_id: &str, result: Result<TraeCallbackPayload, String>) {
    if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
        if let Some(state) = guard.as_mut() {
            if state.login_id == login_id {
                state.callback_result = Some(result);
                persist_pending_login(Some(state));
            }
        }
    }
}

fn run_callback_server(
    login_id: String,
    callback_port: u16,
    fallback_login_host: String,
) -> Result<(), String> {
    let server = Server::http(format!("127.0.0.1:{}", callback_port))
        .map_err(|e| format!("启动 Trae OAuth 回调服务失败: {}", e))?;

    loop {
        let (should_stop, is_timeout) = {
            let guard = PENDING_OAUTH_STATE
                .lock()
                .map_err(|_| "获取 Trae OAuth 状态锁失败".to_string())?;
            match guard.as_ref() {
                Some(state) if state.login_id == login_id => {
                    let timeout = now_timestamp() > state.expires_at;
                    (state.cancelled, timeout)
                }
                _ => (true, false),
            }
        };

        if should_stop {
            break;
        }

        if is_timeout {
            set_callback_result_if_matches(
                &login_id,
                Err("Trae OAuth 登录已超时，请重试".to_string()),
            );
            break;
        }

        let request = match server.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(req)) => req,
            Ok(None) => continue,
            Err(err) => {
                set_callback_result_if_matches(
                    &login_id,
                    Err(format!("Trae OAuth 回调监听失败: {}", err)),
                );
                break;
            }
        };

        let full_url = format!("http://127.0.0.1{}", request.url());
        let parsed = match Url::parse(&full_url) {
            Ok(url) => url,
            Err(err) => {
                let _ = request.respond(
                    Response::from_string(callback_failure_html("回调 URL 解析失败"))
                        .with_status_code(StatusCode(400)),
                );
                set_callback_result_if_matches(
                    &login_id,
                    Err(format!("Trae OAuth 回调 URL 解析失败: {}", err)),
                );
                break;
            }
        };

        if parsed.path() != CALLBACK_PATH {
            let _ = request
                .respond(Response::from_string("Not Found").with_status_code(StatusCode(404)));
            continue;
        }

        let query_raw = parsed.query().unwrap_or("");
        let params = parse_query_map(query_raw);

        if let Some(error_code) =
            pick_query_value(&params, &["error", "error_code", "err", "errorCode"])
        {
            let error_desc = pick_query_value(
                &params,
                &[
                    "error_description",
                    "error_desc",
                    "errorDescription",
                    "message",
                ],
            );
            let message = if let Some(desc) = error_desc {
                format!("授权失败: {} ({})", error_code, desc)
            } else {
                format!("授权失败: {}", error_code)
            };
            let _ = request.respond(
                Response::from_string(callback_failure_html(message.as_str()))
                    .with_status_code(StatusCode(400)),
            );
            set_callback_result_if_matches(&login_id, Err(message));
            break;
        }

        let is_redirect = parse_bool_like(
            pick_query_value(&params, &["isRedirect", "is_redirect", "redirect"]).as_deref(),
        );
        if is_redirect != Some(true) {
            let message = "回调参数缺少 isRedirect=true".to_string();
            let _ = request.respond(
                Response::from_string(callback_failure_html(message.as_str()))
                    .with_status_code(StatusCode(400)),
            );
            set_callback_result_if_matches(&login_id, Err(message));
            break;
        }

        let refresh_token = pick_query_value(
            &params,
            &[
                "refreshToken",
                "refresh_token",
                "RefreshToken",
                "refresh-token",
            ],
        );

        if refresh_token.is_none() {
            let mut response =
                Response::from_string(callback_pending_html()).with_status_code(StatusCode(200));
            if let Ok(content_type) = Header::from_bytes(
                "Content-Type".as_bytes(),
                "text/html; charset=utf-8".as_bytes(),
            ) {
                response = response.with_header(content_type);
            }
            let _ = request.respond(response);

            if !query_raw.is_empty() {
                set_callback_result_if_matches(
                    &login_id,
                    Err("回调参数缺少 refreshToken".to_string()),
                );
                break;
            }
            continue;
        }

        let login_host = pick_query_value(
            &params,
            &[
                "loginHost",
                "login_host",
                "LoginHost",
                "host",
                "consoleHost",
            ],
        )
        .or_else(|| normalize_non_empty(Some(fallback_login_host.as_str())));
        let login_host = match login_host {
            Some(value) => value,
            None => {
                let message = "回调参数缺少 loginHost".to_string();
                let _ = request.respond(
                    Response::from_string(callback_failure_html(message.as_str()))
                        .with_status_code(StatusCode(400)),
                );
                set_callback_result_if_matches(&login_id, Err(message));
                break;
            }
        };

        let payload = TraeCallbackPayload {
            refresh_token: refresh_token.unwrap_or_default(),
            login_host,
            login_region: pick_query_value(
                &params,
                &["loginRegion", "login_region", "region", "Region"],
            ),
            login_trace_id: pick_query_value(
                &params,
                &["loginTraceID", "loginTraceId", "login_trace_id", "trace_id"],
            ),
            cloudide_token: extract_cloudide_token(&params),
            raw_query: params,
        };

        let mut response =
            Response::from_string(callback_success_html()).with_status_code(StatusCode(200));
        if let Ok(content_type) = Header::from_bytes(
            "Content-Type".as_bytes(),
            "text/html; charset=utf-8".as_bytes(),
        ) {
            response = response.with_header(content_type);
        }
        let _ = request.respond(response);
        set_callback_result_if_matches(&login_id, Ok(payload));
        break;
    }

    Ok(())
}

fn spawn_callback_server(login_id: String, callback_port: u16, fallback_login_host: String) {
    std::thread::spawn(move || {
        if let Err(err) = run_callback_server(login_id.clone(), callback_port, fallback_login_host)
        {
            logger::log_warn(&format!(
                "[Trae OAuth] 回调服务异常: login_id={}, error={}",
                login_id, err
            ));
        }
    });
}

fn ensure_callback_server_for_state(state: &PendingOAuthState) {
    if state.cancelled || now_timestamp() > state.expires_at {
        set_pending_login(None);
        return;
    }
    if state.callback_result.is_some() {
        return;
    }

    match TcpListener::bind(("127.0.0.1", state.callback_port)) {
        Ok(listener) => {
            drop(listener);
            spawn_callback_server(
                state.login_id.clone(),
                state.callback_port,
                state.login_host.clone(),
            );
            logger::log_info(&format!(
                "[Trae OAuth] 已恢复本地回调服务: login_id={}, port={}",
                state.login_id, state.callback_port
            ));
        }
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            logger::log_info(&format!(
                "[Trae OAuth] 本地回调端口已占用，视为监听中: login_id={}, port={}",
                state.login_id, state.callback_port
            ));
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Trae OAuth] 本地回调恢复失败: login_id={}, port={}, error={}",
                state.login_id, state.callback_port, err
            ));
        }
    }
}

fn extract_login_guidance_host(response: &Value) -> Option<String> {
    pick_string(
        response,
        &[
            &["Result", "LoginHost"],
            &["Result", "loginHost"],
            &["Result", "LoginURL"],
            &["Result", "loginUrl"],
            &["result", "LoginHost"],
            &["result", "loginHost"],
            &["result", "loginUrl"],
            &["data", "Result", "LoginHost"],
            &["data", "result", "loginHost"],
            &["data", "loginHost"],
            &["data", "loginUrl"],
            &["LoginHost"],
            &["loginHost"],
            &["loginUrl"],
        ],
    )
}

async fn request_login_guidance(login_trace_id: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut errors: Vec<String> = Vec::new();
    for endpoint in TRAE_LOGIN_GUIDANCE_URLS {
        let body = json!({
            "loginTraceID": login_trace_id,
            "login_trace_id": login_trace_id,
        });
        let request = client
            .post(endpoint)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("User-Agent", "Trae/1.0.0 antigravity-cockpit-tools")
            .json(&body);

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(err) => {
                errors.push(format!("{} => {}", endpoint, err));
                continue;
            }
        };

        let status = response.status();
        let text = match response.text().await {
            Ok(body_text) => body_text,
            Err(err) => {
                errors.push(format!("{} => 读取响应失败: {}", endpoint, err));
                continue;
            }
        };

        if !status.is_success() {
            errors.push(format!(
                "{} => HTTP {} (body_len={})",
                endpoint,
                status.as_u16(),
                text.len()
            ));
            continue;
        }

        let value: Value = match serde_json::from_str(text.as_str()) {
            Ok(parsed) => parsed,
            Err(err) => {
                errors.push(format!("{} => 解析 JSON 失败: {}", endpoint, err));
                continue;
            }
        };

        if let Some(login_host) = extract_login_guidance_host(&value) {
            return Ok(login_host);
        }

        errors.push(format!(
            "{} => 响应缺少 LoginHost 字段: {}",
            endpoint, value
        ));
    }

    Err(format!(
        "获取 Trae 登录引导地址失败: {}",
        errors.join(" | ")
    ))
}

fn build_verification_uri(
    login_host: &str,
    login_trace_id: &str,
    callback_url: &str,
    login_context: &TraeLoginContext,
) -> Result<String, String> {
    let mut url = ensure_https_url(login_host)?;
    url.set_path(TRAE_AUTHORIZATION_PATH);
    url.set_query(None);
    let mut query = String::new();
    let append_pair = |query: &mut String, key: &str, value: &str, should_encode: bool| {
        if !query.is_empty() {
            query.push('&');
        }
        query.push_str(key);
        query.push('=');
        if should_encode {
            query.push_str(urlencoding::encode(value).as_ref());
        } else {
            query.push_str(value);
        }
    };
    append_pair(&mut query, "login_version", "1", false);
    append_pair(&mut query, "auth_from", "trae", false);
    append_pair(&mut query, "login_channel", "native_ide", false);
    append_pair(
        &mut query,
        "plugin_version",
        login_context.plugin_version.as_str(),
        true,
    );
    append_pair(&mut query, "auth_type", "local", false);
    append_pair(&mut query, "client_id", TRAE_AUTH_CLIENT_ID, false);
    append_pair(&mut query, "redirect", "0", false);
    append_pair(&mut query, "login_trace_id", login_trace_id, true);
    append_pair(&mut query, "auth_callback_url", callback_url, false);
    append_pair(
        &mut query,
        "machine_id",
        login_context.machine_id.as_str(),
        true,
    );
    append_pair(
        &mut query,
        "device_id",
        login_context.device_id.as_str(),
        true,
    );
    append_pair(
        &mut query,
        "x_device_id",
        login_context.device_id.as_str(),
        true,
    );
    append_pair(
        &mut query,
        "x_machine_id",
        login_context.machine_id.as_str(),
        true,
    );
    append_pair(
        &mut query,
        "x_device_brand",
        login_context.x_device_brand.as_str(),
        true,
    );
    append_pair(
        &mut query,
        "x_device_type",
        login_context.x_device_type.as_str(),
        true,
    );
    append_pair(
        &mut query,
        "x_os_version",
        login_context.x_os_version.as_str(),
        true,
    );
    append_pair(&mut query, "x_env", login_context.x_env.as_str(), true);
    append_pair(
        &mut query,
        "x_app_version",
        login_context.x_app_version.as_str(),
        true,
    );
    append_pair(
        &mut query,
        "x_app_type",
        login_context.x_app_type.as_str(),
        true,
    );
    url.set_query(Some(query.as_str()));
    Ok(url.to_string())
}

fn infer_login_region(login_region: Option<&str>, login_host: &str) -> String {
    if let Some(region) = normalize_non_empty(login_region) {
        let lower = region.to_lowercase();
        if lower == "cn" || lower == "sg" || lower == "us" {
            return lower;
        }
        return lower;
    }

    let lower_host = login_host.to_lowercase();
    if lower_host.contains(".cn") {
        return "cn".to_string();
    }
    if lower_host.contains(".us") {
        return "us".to_string();
    }
    "sg".to_string()
}

fn dedup_keep_order(values: Vec<String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for value in values {
        if value.is_empty() || seen.contains(value.as_str()) {
            continue;
        }
        seen.insert(value.clone());
        out.push(value);
    }
    out
}

fn candidate_api_origins(login_host: &str) -> Vec<String> {
    let mut origins: Vec<String> = Vec::new();

    if let Ok(url) = ensure_https_url(login_host) {
        if let Some(host) = url.host_str() {
            origins.push(format!("{}://{}", url.scheme(), host));
            if let Some(stripped) = host.strip_prefix("www.") {
                origins.push(format!("{}://api.{}", url.scheme(), stripped));
            }
        }
    }

    origins.extend([
        "https://api.marscode.com".to_string(),
        "https://api.trae.ai".to_string(),
        "https://www.trae.ai".to_string(),
        "https://www.marscode.com".to_string(),
    ]);

    dedup_keep_order(origins)
}

fn build_api_urls(login_host: &str, path: &str) -> Vec<String> {
    let urls = candidate_api_origins(login_host)
        .into_iter()
        .map(|origin| format!("{}{}", origin.trim_end_matches('/'), path))
        .collect::<Vec<_>>();
    dedup_keep_order(urls)
}

fn extract_exchange_access_token(value: &Value) -> Option<String> {
    pick_string(
        value,
        &[
            &["Result", "AccessToken"],
            &["Result", "accessToken"],
            &["Result", "Token"],
            &["Result", "token"],
            &["result", "accessToken"],
            &["result", "access_token"],
            &["result", "Token"],
            &["result", "token"],
            &["data", "accessToken"],
            &["data", "access_token"],
            &["data", "Token"],
            &["data", "token"],
            &["Token"],
            &["accessToken"],
            &["access_token"],
            &["token"],
        ],
    )
}

fn extract_exchange_refresh_token(value: &Value) -> Option<String> {
    pick_string(
        value,
        &[
            &["Result", "RefreshToken"],
            &["Result", "refreshToken"],
            &["result", "refreshToken"],
            &["result", "refresh_token"],
            &["data", "refreshToken"],
            &["data", "refresh_token"],
            &["refreshToken"],
            &["refresh_token"],
        ],
    )
}

fn extract_exchange_token_type(value: &Value) -> Option<String> {
    pick_string(
        value,
        &[
            &["Result", "TokenType"],
            &["Result", "tokenType"],
            &["result", "tokenType"],
            &["result", "token_type"],
            &["data", "tokenType"],
            &["data", "token_type"],
            &["tokenType"],
            &["token_type"],
        ],
    )
}

fn extract_exchange_expires_at(value: &Value) -> Option<i64> {
    pick_i64(
        value,
        &[
            &["Result", "ExpiresAt"],
            &["Result", "expiresAt"],
            &["Result", "expiredAt"],
            &["result", "expiresAt"],
            &["result", "expires_at"],
            &["data", "expiresAt"],
            &["data", "expires_at"],
            &["expiresAt"],
            &["expires_at"],
        ],
    )
}

fn extract_error_message(value: &Value) -> Option<String> {
    pick_string(
        value,
        &[
            &["message"],
            &["msg"],
            &["error"],
            &["errorMsg"],
            &["error_msg"],
            &["ResponseMetadata", "Error", "Message"],
            &["Result", "Message"],
            &["result", "message"],
        ],
    )
}

async fn request_exchange_token(
    client: &reqwest::Client,
    login_host: &str,
    refresh_token: &str,
    cloudide_token: Option<&str>,
) -> Result<Value, String> {
    let urls = build_api_urls(login_host, TRAE_EXCHANGE_TOKEN_PATH);
    let mut errors: Vec<String> = Vec::new();

    for url in urls {
        let body = json!({
            "ClientID": TRAE_AUTH_CLIENT_ID,
            "RefreshToken": refresh_token,
            "ClientSecret": TRAE_EXCHANGE_CLIENT_SECRET,
            "UserID": "",
        });

        let mut request = client
            .post(url.as_str())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&body);
        if let Some(token) = normalize_non_empty(cloudide_token) {
            request = request.header("x-cloudide-token", token);
        }

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(err) => {
                errors.push(format!("{} => {}", url, err));
                continue;
            }
        };

        let status = response.status();
        let text = match response.text().await {
            Ok(body_text) => body_text,
            Err(err) => {
                errors.push(format!("{} => 读取响应失败: {}", url, err));
                continue;
            }
        };

        if !status.is_success() {
            errors.push(format!(
                "{} => HTTP {} (body_len={})",
                url,
                status.as_u16(),
                text.len()
            ));
            continue;
        }

        let value: Value = match serde_json::from_str(text.as_str()) {
            Ok(parsed) => parsed,
            Err(err) => {
                errors.push(format!("{} => 解析 JSON 失败: {}", url, err));
                continue;
            }
        };

        if extract_exchange_access_token(&value).is_some() {
            return Ok(value);
        }

        let msg =
            extract_error_message(&value).unwrap_or_else(|| "响应缺少 access token".to_string());
        errors.push(format!("{} => {}", url, msg));
    }

    Err(format!("Trae ExchangeToken 失败: {}", errors.join(" | ")))
}

async fn request_user_info(
    client: &reqwest::Client,
    login_host: &str,
    access_token: &str,
) -> Result<Value, String> {
    let urls = build_api_urls(login_host, TRAE_GET_USER_INFO_PATH);
    let mut errors: Vec<String> = Vec::new();

    for url in urls {
        let response = match client
            .post(url.as_str())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("x-cloudide-token", access_token)
            .json(&json!({}))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                errors.push(format!("{} => {}", url, err));
                continue;
            }
        };

        let status = response.status();
        let text = match response.text().await {
            Ok(body_text) => body_text,
            Err(err) => {
                errors.push(format!("{} => 读取响应失败: {}", url, err));
                continue;
            }
        };

        if !status.is_success() {
            errors.push(format!(
                "{} => HTTP {} (body_len={})",
                url,
                status.as_u16(),
                text.len()
            ));
            continue;
        }

        let value: Value = match serde_json::from_str(text.as_str()) {
            Ok(parsed) => parsed,
            Err(err) => {
                errors.push(format!("{} => 解析 JSON 失败: {}", url, err));
                continue;
            }
        };

        return Ok(value);
    }

    Err(format!("Trae GetUserInfo 失败: {}", errors.join(" | ")))
}

pub async fn start_login() -> Result<TraeOAuthStartResponse, String> {
    hydrate_pending_login_if_missing();
    if let Ok(guard) = PENDING_OAUTH_STATE.lock() {
        if let Some(state) = guard.as_ref() {
            if !state.cancelled && now_timestamp() <= state.expires_at {
                ensure_callback_server_for_state(state);
                return Ok(TraeOAuthStartResponse {
                    login_id: state.login_id.clone(),
                    verification_uri: state.verification_uri.clone(),
                    expires_in: (state.expires_at - now_timestamp()).max(0) as u64,
                    interval_seconds: (OAUTH_POLL_INTERVAL_MS / 1000).max(1),
                    callback_url: Some(state.callback_url.clone()),
                });
            }
        }
    }
    set_pending_login(None);

    let login_id = Uuid::new_v4().to_string();
    let login_trace_id = Uuid::new_v4().to_string();
    let login_context = collect_trae_login_context();
    let callback_port = find_available_callback_port()?;
    let callback_url = format!("http://127.0.0.1:{}{}", callback_port, CALLBACK_PATH);

    let login_host = request_login_guidance(login_trace_id.as_str()).await?;
    let verification_uri = build_verification_uri(
        login_host.as_str(),
        login_trace_id.as_str(),
        callback_url.as_str(),
        &login_context,
    )?;

    let state = PendingOAuthState {
        login_id: login_id.clone(),
        login_trace_id: login_trace_id.clone(),
        callback_port,
        callback_url: callback_url.clone(),
        verification_uri: verification_uri.clone(),
        login_host: login_host.clone(),
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS,
        cancelled: false,
        callback_result: None,
    };

    set_pending_login(Some(state));

    spawn_callback_server(login_id.clone(), callback_port, login_host.clone());

    logger::log_info(&format!(
        "[Trae OAuth] 登录会话已创建: login_id={}, trace_id={}, callback_url={}, plugin_version={}, x_app_version={}, x_app_type={}, machine_id={}, device_id={}",
        login_id,
        login_trace_id,
        callback_url,
        login_context.plugin_version,
        login_context.x_app_version,
        login_context.x_app_type,
        mask_id_for_log(login_context.machine_id.as_str()),
        mask_id_for_log(login_context.device_id.as_str())
    ));

    Ok(TraeOAuthStartResponse {
        login_id,
        verification_uri,
        expires_in: OAUTH_TIMEOUT_SECONDS as u64,
        interval_seconds: (OAUTH_POLL_INTERVAL_MS / 1000).max(1),
        callback_url: Some(callback_url),
    })
}

pub async fn complete_login(login_id: &str) -> Result<TraeImportPayload, String> {
    hydrate_pending_login_if_missing();
    let result = async {
        let (callback_payload, login_trace_id) = loop {
            let snapshot = {
                let guard = PENDING_OAUTH_STATE
                    .lock()
                    .map_err(|_| "获取 Trae OAuth 状态锁失败".to_string())?;
                let state = guard
                    .as_ref()
                    .ok_or_else(|| "没有进行中的 Trae OAuth 登录会话".to_string())?;

                if state.login_id != login_id {
                    return Err("Trae OAuth 登录会话已变更，请重新发起".to_string());
                }
                if state.cancelled {
                    return Err("Trae OAuth 登录已取消".to_string());
                }
                if now_timestamp() > state.expires_at {
                    return Err("Trae OAuth 登录已超时，请重试".to_string());
                }

                (
                    state.callback_result.clone(),
                    state.login_trace_id.clone(),
                    state.callback_url.clone(),
                    state.callback_port,
                    state.verification_uri.clone(),
                    state.login_host.clone(),
                )
            };

            if let Some(result) = snapshot.0 {
                break (result?, snapshot.1);
            }

            tokio::time::sleep(Duration::from_millis(OAUTH_POLL_INTERVAL_MS)).await;
        };

        if let Some(trace) = callback_payload.login_trace_id.as_deref() {
            if trace != login_trace_id {
                logger::log_warn(&format!(
                    "[Trae OAuth] 回调 trace 不匹配，继续处理: callback_trace={}, expected_trace={}",
                    trace, login_trace_id
                ));
            }
        }

        let login_region = infer_login_region(
            callback_payload.login_region.as_deref(),
            callback_payload.login_host.as_str(),
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        let exchange_response = request_exchange_token(
            &client,
            callback_payload.login_host.as_str(),
            callback_payload.refresh_token.as_str(),
            callback_payload.cloudide_token.as_deref(),
        )
        .await?;

        let access_token = extract_exchange_access_token(&exchange_response)
            .ok_or_else(|| "Trae ExchangeToken 响应缺少 access token".to_string())?;
        let refresh_token = extract_exchange_refresh_token(&exchange_response)
            .or_else(|| Some(callback_payload.refresh_token.clone()));
        let token_type = extract_exchange_token_type(&exchange_response);
        let expires_at = extract_exchange_expires_at(&exchange_response);

        let user_info_response = match request_user_info(
            &client,
            callback_payload.login_host.as_str(),
            access_token.as_str(),
        )
        .await
        {
            Ok(response) => Some(response),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Trae OAuth] GetUserInfo 失败，将使用降级信息保存账号: {}",
                    err
                ));
                None
            }
        };
        let callback_user_info = callback_payload
            .raw_query
            .get("userInfo")
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());

        let email = user_info_response
            .as_ref()
            .and_then(|value| {
                pick_string(
                    value,
                    &[
                        &["Result", "NonPlainTextEmail"],
                        &["Result", "Email"],
                        &["Result", "email"],
                        &["NonPlainTextEmail"],
                        &["result", "email"],
                        &["data", "email"],
                        &["data", "user", "email"],
                        &["email"],
                    ],
                )
            })
            .or_else(|| {
                callback_user_info.as_ref().and_then(|value| {
                    pick_string(
                        value,
                        &[&["NonPlainTextEmail"], &["Email"], &["email"]],
                    )
                })
            })
            .unwrap_or_else(|| "unknown".to_string());
        let user_id = user_info_response.as_ref().and_then(|value| {
            pick_string(
                value,
                &[
                    &["Result", "UserID"],
                    &["Result", "userId"],
                    &["Result", "UID"],
                    &["result", "userId"],
                    &["result", "uid"],
                    &["data", "userId"],
                    &["data", "uid"],
                    &["userId"],
                    &["uid"],
                ],
            )
        }).or_else(|| {
            callback_user_info
                .as_ref()
                .and_then(|value| pick_string(value, &[&["UserID"], &["userId"], &["uid"]]))
        });
        let nickname = user_info_response.as_ref().and_then(|value| {
            pick_string(
                value,
                &[
                    &["Result", "ScreenName"],
                    &["Result", "Nickname"],
                    &["Result", "nickname"],
                    &["Result", "Name"],
                    &["result", "nickname"],
                    &["result", "name"],
                    &["data", "nickname"],
                    &["data", "name"],
                    &["nickname"],
                    &["name"],
                ],
            )
        }).or_else(|| {
            callback_user_info.as_ref().and_then(|value| {
                pick_string(value, &[&["ScreenName"], &["Nickname"], &["Name"], &["name"]])
            })
        });

        let mut auth_raw = Map::new();
        auth_raw.insert(
            "accessToken".to_string(),
            Value::String(access_token.clone()),
        );
        if let Some(refresh) = refresh_token.as_ref() {
            auth_raw.insert("refreshToken".to_string(), Value::String(refresh.clone()));
        }
        auth_raw.insert(
            "loginHost".to_string(),
            Value::String(callback_payload.login_host.clone()),
        );
        auth_raw.insert(
            "loginRegion".to_string(),
            Value::String(login_region.clone()),
        );
        auth_raw.insert(
            "loginTraceID".to_string(),
            Value::String(login_trace_id.clone()),
        );
        auth_raw.insert(
            "callbackQuery".to_string(),
            serde_json::to_value(&callback_payload.raw_query).unwrap_or_else(|_| json!({})),
        );
        auth_raw.insert("exchangeResponse".to_string(), exchange_response.clone());

        let server_raw = json!({
            "loginHost": callback_payload.login_host,
            "loginRegion": login_region,
            "loginTraceID": login_trace_id,
        });

        Ok(TraeImportPayload {
            email,
            user_id,
            nickname,
            access_token,
            refresh_token,
            token_type,
            expires_at,
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: Some(Value::Object(auth_raw)),
            trae_profile_raw: user_info_response,
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: Some(server_raw),
            trae_usertag_raw: None,
            status: None,
            status_reason: None,
        })
    }
    .await;

    clear_pending_if_matches(login_id);
    result
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    hydrate_pending_login_if_missing();
    let current = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "获取 Trae OAuth 状态锁失败".to_string())?
        .as_ref()
        .cloned();

    let Some(current) = current.as_ref() else {
        return Ok(());
    };

    if let Some(target) = login_id {
        if current.login_id != target {
            return Ok(());
        }
    }

    logger::log_info(&format!(
        "[Trae OAuth] 取消登录会话: login_id={}",
        current.login_id
    ));

    set_pending_login(None);
    Ok(())
}

pub fn submit_callback_url(login_id: &str, callback_url: &str) -> Result<(), String> {
    hydrate_pending_login_if_missing();
    let (expires_at, cancelled, callback_port, fallback_login_host) = {
        let guard = PENDING_OAUTH_STATE
            .lock()
            .map_err(|_| "获取 Trae OAuth 状态锁失败".to_string())?;
        let state = guard
            .as_ref()
            .ok_or_else(|| "没有进行中的 Trae OAuth 登录会话".to_string())?;
        if state.login_id != login_id {
            return Err("Trae OAuth 登录会话已变更，请重新发起".to_string());
        }
        (
            state.expires_at,
            state.cancelled,
            state.callback_port,
            state.login_host.clone(),
        )
    };

    if cancelled {
        return Err("Trae OAuth 登录已取消".to_string());
    }
    if now_timestamp() > expires_at {
        return Err("Trae OAuth 登录已超时，请重试".to_string());
    }

    let parsed = parse_callback_url(callback_url, callback_port)?;
    if parsed.path() != CALLBACK_PATH {
        return Err(format!("回调链接路径无效，必须为 {}", CALLBACK_PATH));
    }

    let params = parse_query_map(parsed.query().unwrap_or_default());
    if let Some(error_code) =
        pick_query_value(&params, &["error", "error_code", "err", "errorCode"])
    {
        let error_desc = pick_query_value(
            &params,
            &[
                "error_description",
                "error_desc",
                "errorDescription",
                "message",
            ],
        );
        let message = if let Some(desc) = error_desc {
            format!("授权失败: {} ({})", error_code, desc)
        } else {
            format!("授权失败: {}", error_code)
        };
        set_callback_result_if_matches(login_id, Err(message.clone()));
        return Err(message);
    }

    let is_redirect = parse_bool_like(
        pick_query_value(&params, &["isRedirect", "is_redirect", "redirect"]).as_deref(),
    );
    if is_redirect != Some(true) {
        return Err("回调参数缺少 isRedirect=true".to_string());
    }

    let refresh_token = pick_query_value(
        &params,
        &[
            "refreshToken",
            "refresh_token",
            "RefreshToken",
            "refresh-token",
        ],
    )
    .ok_or_else(|| "回调参数缺少 refreshToken".to_string())?;

    let login_host = pick_query_value(
        &params,
        &[
            "loginHost",
            "login_host",
            "LoginHost",
            "host",
            "consoleHost",
        ],
    )
    .or_else(|| normalize_non_empty(Some(fallback_login_host.as_str())))
    .ok_or_else(|| "回调参数缺少 loginHost".to_string())?;

    let payload = TraeCallbackPayload {
        refresh_token,
        login_host,
        login_region: pick_query_value(
            &params,
            &["loginRegion", "login_region", "region", "Region"],
        ),
        login_trace_id: pick_query_value(
            &params,
            &["loginTraceID", "loginTraceId", "login_trace_id", "trace_id"],
        ),
        cloudide_token: extract_cloudide_token(&params),
        raw_query: params,
    };

    set_callback_result_if_matches(login_id, Ok(payload));
    logger::log_info(&format!(
        "[Trae OAuth] 已接收手动回调链接: login_id={}",
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
