use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;
use uuid::Uuid;

use crate::models::qoder::{QoderAccount, QoderOAuthStartResponse};
use crate::modules::qoder_channel::QoderChannel;
use crate::modules::{config, logger, qoder_account, qoder_instance};

const OAUTH_TIMEOUT_SECONDS: i64 = 600;
const OAUTH_POLL_INTERVAL_MS: u64 = 1000;
const DEFAULT_LOGIN_BASE_URL: &str = "https://qoder.com/device/selectAccounts";
pub const DEFAULT_GLOBAL_OPENAPI_BASE_URL: &str = "https://openapi.qoder.sh";
pub const DEFAULT_CN_OPENAPI_BASE_URL: &str = "https://openapi.qoder.com.cn";
pub const DEFAULT_OPENAPI_BASE_URL: &str = DEFAULT_GLOBAL_OPENAPI_BASE_URL;
const QODER_IDE_REDIRECT_URI: &str = "qoder://aicoding.aicoding-agent/login-success";
const QODER_DEVICE_LOGIN_CHALLENGE_METHOD: &str = "S256";
const DEVICE_TOKEN_POLL_PATH: &str = "/api/v1/deviceToken/poll";
const DEVICE_TOKEN_REFRESH_PATH: &str = "/api/v1/deviceToken/refresh";
const USER_INFO_PATH: &str = "/api/v1/userinfo";
const USER_STATUS_PATH: &str = "/api/v3/user/status";
const DATA_POLICY_PATH: &str = "/api/v2/config/getDataPolicy";
const USER_PLAN_PATH: &str = "/api/v2/user/plan";
const CREDIT_USAGE_PATH: &str = "/api/v2/quota/usage";
const AUTH_STATUS_AUTHORIZED: i64 = 2;
const AUTH_STATUS_IP_BANNED_ERROR: i64 = 6;
const AUTH_STATUS_APP_DISABLED_ERROR: i64 = 7;
const AUTH_STATUS_LOGIN_EXPIRED: i64 = 3;
const WHITELIST_NOT_WHITELIST: i64 = 1;
const WHITELIST_WAIT_PASS: i64 = 2;
const WHITELIST_PASS: i64 = 3;
const WHITELIST_NO_LICENCE: i64 = 5;
const WHITELIST_ORG_EXPIRED: i64 = 6;
const WHITELIST_NOT_ALLOW: i64 = 7;

#[derive(Debug, Clone)]
struct PendingOAuthState {
    login_id: String,
    expected_nonce: String,
    code_verifier: String,
    challenge_method: String,
    openapi_base_url: String,
    machine_info: Option<QoderMachineInfo>,
    verification_uri: String,
    expires_at: i64,
    cancelled: bool,
}

#[derive(Debug, Clone)]
struct QoderMachineInfo {
    token: String,
    machine_type: Option<String>,
    machine_code: Option<String>,
    machine_id: Option<String>,
    machine_hostname: Option<String>,
    machine_os: Option<String>,
    cosy_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QoderMachineTokenCache {
    #[serde(default)]
    token: Option<String>,
    #[serde(default, rename = "type")]
    machine_type: Option<String>,
    #[serde(default, rename = "code")]
    machine_code: Option<String>,
    #[serde(default, rename = "id")]
    machine_id: Option<String>,
    #[serde(default, rename = "hostname")]
    machine_hostname: Option<String>,
    #[serde(default, rename = "os")]
    machine_os: Option<String>,
    #[serde(default, rename = "version")]
    cosy_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QoderDeviceTokenPollResult {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    refresh_token_expires_at: Option<String>,
}

lazy_static::lazy_static! {
    static ref PENDING_OAUTH_STATE: Arc<Mutex<Option<PendingOAuthState>>> = Arc::new(Mutex::new(None));
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
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

fn generate_pkce_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn generate_login_nonce() -> String {
    Uuid::new_v4().simple().to_string()
}

fn normalize_url_origin_and_path(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    let host = url.host_str()?;
    let mut normalized = format!("{}://{}", url.scheme(), host);
    if let Some(port) = url.port() {
        normalized.push(':');
        normalized.push_str(&port.to_string());
    }
    normalized.push_str(url.path());
    Some(normalized)
}

fn resolve_qoder_cli_login_endpoint() -> String {
    normalize_url_origin_and_path(DEFAULT_LOGIN_BASE_URL)
        .unwrap_or_else(|| DEFAULT_LOGIN_BASE_URL.to_string())
}

fn build_cli_device_login_url(
    login_base_url: &str,
    nonce: &str,
    challenge: &str,
    challenge_method: &str,
    machine_id: Option<&str>,
) -> Result<String, String> {
    let mut url =
        Url::parse(login_base_url).map_err(|err| format!("解析 Qoder 登录地址失败: {}", err))?;
    {
        let mut query_pairs = url.query_pairs_mut();
        query_pairs.append_pair("nonce", nonce);
        query_pairs.append_pair("challenge", challenge);
        query_pairs.append_pair("challenge_method", challenge_method);
        query_pairs.append_pair("redirect_uri", QODER_IDE_REDIRECT_URI);
        if let Some(machine_id) = machine_id.and_then(|value| normalize_non_empty(Some(value))) {
            query_pairs.append_pair("machine_id", &machine_id);
        }
    }
    Ok(url.to_string())
}

fn parse_expire_timestamp_ms(raw: Option<&str>) -> Option<String> {
    let text = normalize_non_empty(raw)?;
    if let Ok(number) = text.parse::<i64>() {
        let millis = if number > 1_000_000_000_000 {
            number
        } else {
            number.saturating_mul(1000)
        };
        return Some(millis.to_string());
    }

    chrono::DateTime::parse_from_rfc3339(&text)
        .ok()
        .map(|value| value.timestamp_millis().to_string())
}

fn insert_string_field(map: &mut serde_json::Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(text) = value {
        map.insert(key.to_string(), Value::String(text));
    }
}

fn insert_i64_field(map: &mut serde_json::Map<String, Value>, key: &str, value: i64) {
    map.insert(
        key.to_string(),
        Value::Number(serde_json::Number::from(value)),
    );
}

fn copy_optional_field(
    from: &Value,
    to: &mut serde_json::Map<String, Value>,
    source_key: &str,
    target_key: &str,
) {
    if let Some(value) = from.get(source_key) {
        to.insert(target_key.to_string(), value.clone());
    }
}

fn calculate_auth_status(user_status: &Value) -> (i64, i64) {
    let has_user_id = user_status
        .get("id")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_non_empty(Some(value)))
        .is_some();
    if !has_user_id {
        return (AUTH_STATUS_LOGIN_EXPIRED, WHITELIST_NOT_WHITELIST);
    }

    match user_status
        .get("whitelistStatus")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_non_empty(Some(value)))
        .as_deref()
    {
        Some("NoIpPermission") => (AUTH_STATUS_IP_BANNED_ERROR, WHITELIST_NOT_WHITELIST),
        Some("AppDisable") => (AUTH_STATUS_APP_DISABLED_ERROR, WHITELIST_NOT_WHITELIST),
        Some("LoginExpire") => (AUTH_STATUS_LOGIN_EXPIRED, WHITELIST_NOT_WHITELIST),
        Some("PASS") => (AUTH_STATUS_AUTHORIZED, WHITELIST_PASS),
        Some("WAIT") => (AUTH_STATUS_AUTHORIZED, WHITELIST_WAIT_PASS),
        Some("NoLicense") => (AUTH_STATUS_AUTHORIZED, WHITELIST_NO_LICENCE),
        Some("NoQuota") | Some("EXPIRED") => (AUTH_STATUS_AUTHORIZED, WHITELIST_ORG_EXPIRED),
        Some("NotAllow") | Some("NOT_ALLOW") => (AUTH_STATUS_AUTHORIZED, WHITELIST_NOT_ALLOW),
        _ => (AUTH_STATUS_AUTHORIZED, WHITELIST_NOT_WHITELIST),
    }
}

fn ensure_user_status_allowed(user_status: &Value) -> Result<(), String> {
    let whitelist_status = user_status
        .get("whitelistStatus")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_non_empty(Some(value)));
    let has_user_id = user_status
        .get("id")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_non_empty(Some(value)))
        .is_some();

    if !has_user_id {
        return Err("Qoder 用户状态缺少 id，无法确认登录身份".to_string());
    }

    match whitelist_status.as_deref() {
        Some("NoIpPermission") => Err("企业设置了 IP 白名单，当前 IP 无法登录".to_string()),
        Some("AppDisable") => Err("Qoder 应用已被停用，无法登录".to_string()),
        Some("LoginExpire") => Err("Qoder 登录已失效，请重试".to_string()),
        Some("NotAllow") | Some("NOT_ALLOW") => Err("当前账号暂无 Qoder 使用权限".to_string()),
        _ => Ok(()),
    }
}

fn build_cosy_machine_os() -> String {
    let arch = match std::env::consts::ARCH {
        "arm64" => "aarch64",
        value => value,
    };
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        value => value,
    };
    format!("{}_{}", arch, os)
}

fn build_qoder_product_file_candidates(base_path: &Path) -> Vec<PathBuf> {
    let mut app_roots: Vec<PathBuf> = Vec::new();
    for ancestor in base_path.ancestors() {
        let Some(name) = ancestor.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if name.eq_ignore_ascii_case("Qoder.app") || name.eq_ignore_ascii_case("Qoder IDE.app") {
            app_roots.push(ancestor.to_path_buf());
            break;
        }
    }

    if base_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("Qoder.app"))
        .unwrap_or(false)
    {
        app_roots.push(base_path.to_path_buf());
    }

    if app_roots.is_empty() {
        app_roots.push(base_path.to_path_buf());
    }

    let mut candidates = Vec::new();
    for root in app_roots {
        candidates.push(
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("product.json"),
        );
        candidates.push(
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("package.json"),
        );
        candidates.push(root.join("resources").join("app").join("product.json"));
        candidates.push(root.join("resources").join("app").join("package.json"));
        candidates.push(root.join("product.json"));
        candidates.push(root.join("package.json"));
    }
    candidates
}

fn read_version_from_json_file(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let parsed = serde_json::from_str::<Value>(&content).ok()?;
    parsed
        .get("productVersion")
        .and_then(|value| value.as_str())
        .or_else(|| parsed.get("version").and_then(|value| value.as_str()))
        .and_then(|value| normalize_non_empty(Some(value)))
}

fn detect_qoder_product_version() -> Option<String> {
    let mut base_paths: Vec<PathBuf> = Vec::new();
    let configured_path = config::get_user_config().qoder_app_path.trim().to_string();
    if !configured_path.is_empty() {
        base_paths.push(PathBuf::from(configured_path));
    }

    #[cfg(target_os = "macos")]
    {
        base_paths.push(PathBuf::from("/Applications/Qoder IDE.app"));
        base_paths.push(PathBuf::from(
            "/Applications/Qoder IDE.app/Contents/MacOS/Qoder IDE",
        ));
        base_paths.push(PathBuf::from("/Applications/Qoder.app"));
        base_paths.push(PathBuf::from(
            "/Applications/Qoder.app/Contents/MacOS/Qoder",
        ));
        base_paths.push(PathBuf::from(
            "/Applications/Qoder.app/Contents/MacOS/Electron",
        ));
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            base_paths.push(
                PathBuf::from(&local_app_data)
                    .join("Programs")
                    .join("Qoder")
                    .join("Qoder.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            base_paths.push(PathBuf::from(program_files).join("Qoder").join("Qoder.exe"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        base_paths.push(PathBuf::from("/usr/share/qoder"));
        base_paths.push(PathBuf::from("/opt/Qoder"));
        if let Some(home) = dirs::home_dir() {
            base_paths.push(home.join(".local").join("share").join("Qoder"));
        }
    }

    for base_path in base_paths {
        for candidate in build_qoder_product_file_candidates(&base_path) {
            if let Some(version) = read_version_from_json_file(&candidate) {
                return Some(version);
            }
        }
    }

    None
}

fn build_qoder_headers(
    token: &str,
    machine_info: Option<&QoderMachineInfo>,
) -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};

    let mut headers = HeaderMap::new();
    let bearer = format!("Bearer {}", token);
    if let Ok(value) = HeaderValue::from_str(&bearer) {
        headers.insert(AUTHORIZATION, value);
    }
    if let Ok(value) = HeaderValue::from_str("application/json") {
        headers.insert(ACCEPT, value);
    }
    let cosy_version = machine_info
        .and_then(|value| value.cosy_version.clone())
        .or_else(detect_qoder_product_version);
    if let Some(version) = cosy_version.as_deref() {
        if let Ok(value) = HeaderValue::from_str(&version) {
            headers.insert("Cosy-Version", value);
        }
    }
    if let Some(machine_token) = machine_info
        .map(|value| value.token.as_str())
        .and_then(|value| normalize_non_empty(Some(value)))
    {
        if let Ok(value) = HeaderValue::from_str(&machine_token) {
            headers.insert("Cosy-MachineToken", value);
        }
    }
    if let Some(machine_type) = machine_info
        .and_then(|value| value.machine_type.as_deref())
        .and_then(|value| normalize_non_empty(Some(value)))
    {
        if let Ok(value) = HeaderValue::from_str(&machine_type) {
            headers.insert("Cosy-MachineType", value);
        }
    }
    let machine_os = machine_info
        .and_then(|value| value.machine_os.as_deref())
        .and_then(|value| normalize_non_empty(Some(value)))
        .unwrap_or_else(build_cosy_machine_os);
    if let Ok(value) = HeaderValue::from_str(&machine_os) {
        headers.insert("Cosy-MachineOS", value);
    }
    for (header, value) in [
        (
            "Cosy-MachineCode",
            machine_info.and_then(|v| v.machine_code.as_deref()),
        ),
        (
            "Cosy-MachineId",
            machine_info.and_then(|v| v.machine_id.as_deref()),
        ),
        (
            "Cosy-MachineHostname",
            machine_info.and_then(|v| v.machine_hostname.as_deref()),
        ),
    ] {
        if let Some(value) = value.and_then(|value| normalize_non_empty(Some(value))) {
            if let Ok(header_value) = HeaderValue::from_str(&value) {
                headers.insert(header, header_value);
            }
        }
    }
    if let Ok(value) = HeaderValue::from_str("0") {
        headers.insert("Cosy-ClientType", value);
    }
    logger::log_info(&format!(
        "[Qoder OAuth] 构造请求头: has_cosy_version={}, has_machine_token={}, has_machine_type={}, machine_os={}",
        cosy_version.is_some(),
        machine_info.is_some_and(|value| !value.token.is_empty()),
        machine_info.and_then(|value| value.machine_type.as_ref()).is_some(),
        machine_os
    ));
    headers
}

fn build_initial_user_info_raw(
    token_data: &QoderDeviceTokenPollResult,
    user_info_response: Option<&Value>,
) -> Value {
    let mut user_info = serde_json::Map::new();
    insert_string_field(
        &mut user_info,
        "id",
        normalize_non_empty(token_data.user_id.as_deref()).or_else(|| {
            user_info_response
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str())
                .and_then(|value| normalize_non_empty(Some(value)))
        }),
    );
    insert_string_field(
        &mut user_info,
        "token",
        normalize_non_empty(token_data.token.as_deref()),
    );
    insert_string_field(
        &mut user_info,
        "refreshToken",
        normalize_non_empty(token_data.refresh_token.as_deref()),
    );
    insert_string_field(
        &mut user_info,
        "expireTime",
        parse_expire_timestamp_ms(token_data.expires_at.as_deref()),
    );
    insert_string_field(
        &mut user_info,
        "refreshTokenExpireTime",
        parse_expire_timestamp_ms(token_data.refresh_token_expires_at.as_deref()),
    );
    if let Some(value) = user_info_response {
        copy_optional_field(value, &mut user_info, "name", "name");
        copy_optional_field(value, &mut user_info, "email", "email");
        copy_optional_field(value, &mut user_info, "avatarUrl", "avatarUrl");
        copy_optional_field(value, &mut user_info, "avatar_url", "avatarUrl");
    }
    Value::Object(user_info)
}

fn merge_user_status_into_user_info(
    user_info: &mut Value,
    user_status: &Value,
    data_policy: Option<&Value>,
) {
    let Some(map) = user_info.as_object_mut() else {
        return;
    };

    copy_optional_field(user_status, map, "id", "id");
    copy_optional_field(user_status, map, "name", "name");
    copy_optional_field(user_status, map, "email", "email");
    copy_optional_field(user_status, map, "avatarUrl", "avatarUrl");
    copy_optional_field(user_status, map, "userType", "userType");
    copy_optional_field(user_status, map, "userTag", "userTag");
    copy_optional_field(user_status, map, "isSubAccount", "isSubAccount");
    copy_optional_field(user_status, map, "quota", "quota");
    copy_optional_field(user_status, map, "isQuotaExceeded", "isQuotaExceeded");
    copy_optional_field(user_status, map, "orgId", "orgId");
    copy_optional_field(user_status, map, "orgName", "orgName");
    copy_optional_field(user_status, map, "yxUid", "yxUid");
    copy_optional_field(user_status, map, "staffId", "staffId");
    copy_optional_field(user_status, map, "cloudType", "cloudType");
    copy_optional_field(
        user_status,
        map,
        "isPrivacyPolicyModifiable",
        "isPrivacyPolicyModifiable",
    );
    copy_optional_field(
        user_status,
        map,
        "isPrivacyPolicyVisible",
        "isPrivacyPolicyVisible",
    );

    let (status, whitelist) = calculate_auth_status(user_status);
    insert_i64_field(map, "status", status);
    insert_i64_field(map, "whitelist", whitelist);

    if let Some(policy) = data_policy {
        let agreed = policy
            .get("success")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            && policy
                .get("result")
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str())
                == Some("AGREE");
        map.insert("privacyPolicyAgreed".to_string(), Value::Bool(agreed));
    }
}

fn build_reqwest_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|err| format!("创建 Qoder HTTP 客户端失败: {}", err))
}

async fn poll_device_token_once(
    client: &reqwest::Client,
    openapi_base_url: &str,
    nonce: &str,
    verifier: &str,
    challenge_method: &str,
) -> Result<Option<QoderDeviceTokenPollResult>, String> {
    let response = client
        .get(format!("{}{}", openapi_base_url, DEVICE_TOKEN_POLL_PATH))
        .query(&[
            ("nonce", nonce),
            ("verifier", verifier),
            ("challenge_method", challenge_method),
        ])
        .send()
        .await
        .map_err(|err| format!("轮询 Qoder device token 失败: {}", err))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "轮询 Qoder device token 失败: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    let payload = response
        .json::<QoderDeviceTokenPollResult>()
        .await
        .map_err(|err| format!("解析 Qoder device token 响应失败: {}", err))?;
    if payload
        .token
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .is_some()
    {
        return Ok(Some(payload));
    }
    Ok(None)
}

async fn fetch_openapi_json(
    client: &reqwest::Client,
    openapi_base_url: &str,
    path: &str,
    mut headers: reqwest::header::HeaderMap,
    query: &[(&str, String)],
) -> Result<Value, String> {
    use reqwest::header::{HeaderValue, ACCEPT};

    if !headers.contains_key(ACCEPT) {
        if let Ok(value) = HeaderValue::from_str("application/json") {
            headers.insert(ACCEPT, value);
        }
    }
    let request = client
        .get(format!("{}{}", openapi_base_url, path))
        .headers(headers)
        .query(query);
    let response = request
        .send()
        .await
        .map_err(|err| format!("请求 Qoder OpenAPI 失败 ({}): {}", path, err))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "请求 Qoder OpenAPI 失败 ({}): status={}, body_len={}",
            path,
            status,
            body.len()
        ));
    }
    response
        .json::<Value>()
        .await
        .map_err(|err| format!("解析 Qoder OpenAPI 响应失败 ({}): {}", path, err))
}

async fn fetch_qoder_user_info(
    client: &reqwest::Client,
    openapi_base_url: &str,
    token: &str,
    machine_info: Option<&QoderMachineInfo>,
) -> Result<Value, String> {
    let headers = build_qoder_headers(token, machine_info);
    fetch_openapi_json(client, openapi_base_url, USER_INFO_PATH, headers, &[]).await
}

async fn fetch_qoder_user_status_bundle(
    client: &reqwest::Client,
    openapi_base_url: &str,
    token: &str,
    machine_info: Option<&QoderMachineInfo>,
) -> Result<(Value, Option<Value>), String> {
    let status_headers = build_qoder_headers(token, machine_info);
    let status = fetch_openapi_json(
        client,
        openapi_base_url,
        USER_STATUS_PATH,
        status_headers.clone(),
        &[],
    )
    .await?;
    ensure_user_status_allowed(&status)?;

    let data_policy = fetch_openapi_json(
        client,
        openapi_base_url,
        DATA_POLICY_PATH,
        status_headers,
        &[("requestId", Uuid::new_v4().to_string())],
    )
    .await
    .ok();
    Ok((status, data_policy))
}

async fn fetch_qoder_user_plan(
    client: &reqwest::Client,
    openapi_base_url: &str,
    token: &str,
    machine_info: Option<&QoderMachineInfo>,
) -> Result<Value, String> {
    let headers = build_qoder_headers(token, machine_info);
    fetch_openapi_json(client, openapi_base_url, USER_PLAN_PATH, headers, &[]).await
}

async fn fetch_qoder_credit_usage(
    client: &reqwest::Client,
    openapi_base_url: &str,
    token: &str,
    machine_info: Option<&QoderMachineInfo>,
) -> Result<Value, String> {
    let headers = build_qoder_headers(token, machine_info);
    fetch_openapi_json(client, openapi_base_url, CREDIT_USAGE_PATH, headers, &[]).await
}

fn get_string_at_path(root: &Value, path: &[&str]) -> Option<String> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    value_to_string(current)
}

fn extract_access_token_from_account(account: &QoderAccount) -> Option<String> {
    let user_info = account.auth_user_info_raw.as_ref()?;
    let candidate_paths: &[&[&str]] = &[
        &["token"],
        &["securityOauthToken"],
        &["accessToken"],
        &["access_token"],
        &["result", "token"],
        &["data", "token"],
        &["result", "accessToken"],
        &["data", "accessToken"],
    ];
    for path in candidate_paths {
        if let Some(value) = get_string_at_path(user_info, path) {
            return Some(value);
        }
    }
    None
}

fn extract_refresh_token_from_account(account: &QoderAccount) -> Option<String> {
    let user_info = account.auth_user_info_raw.as_ref()?;
    let candidate_paths: &[&[&str]] = &[
        &["refreshToken"],
        &["refresh_token"],
        &["securityRefreshToken"],
        &["data", "refreshToken"],
        &["data", "refresh_token"],
        &["result", "refreshToken"],
        &["result", "refresh_token"],
    ];
    for path in candidate_paths {
        if let Some(value) = get_string_at_path(user_info, path) {
            return Some(value);
        }
    }
    None
}

fn is_unauthorized_error(err: &str) -> bool {
    err.contains("401")
        || err.contains("403")
        || err.contains("Unauthorized")
        || err.contains("UNAUTHORIZED")
}

#[derive(Debug, Clone)]
struct RefreshedTokenInfo {
    access_token: String,
    refresh_token: String,
    expires_at: Option<String>,
    refresh_token_expires_at: Option<String>,
}

async fn request_device_token_refresh(
    client: &reqwest::Client,
    openapi_base_url: &str,
    refresh_token: &str,
    access_token: Option<&str>,
    machine_info: Option<&QoderMachineInfo>,
) -> Result<RefreshedTokenInfo, String> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};

    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str("application/json") {
        headers.insert(ACCEPT, value.clone());
        headers.insert(CONTENT_TYPE, value);
    }
    if let Some(tok) = access_token {
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", tok)) {
            headers.insert(AUTHORIZATION, value);
        }
    }
    if let Some(version) = machine_info
        .and_then(|v| v.cosy_version.clone())
        .or_else(detect_qoder_product_version)
    {
        if let Ok(value) = HeaderValue::from_str(&version) {
            headers.insert("Cosy-Version", value);
        }
    }
    if let Some(machine_token) = machine_info
        .map(|v| v.token.as_str())
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        if let Ok(value) = HeaderValue::from_str(&machine_token) {
            headers.insert("Cosy-MachineToken", value);
        }
    }

    let body = serde_json::json!({
        "refresh_token": refresh_token,
        "refreshToken": refresh_token,
    });

    let url = format!("{}{}", openapi_base_url, DEVICE_TOKEN_REFRESH_PATH);
    let resp = client
        .post(&url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 Token 刷新接口网络失败 ({}): {}", url, e))?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        let is_unauthorized = status == reqwest::StatusCode::UNAUTHORIZED
            || (status == reqwest::StatusCode::BAD_REQUEST
                && (body_text.contains("invalid refresh_token")
                    || body_text.contains("invalid_grant")
                    || body_text.contains("User not authenticated")));
        if is_unauthorized {
            return Err("登录凭据已完全失效（阿里专网会话超时），请在 Qoder 中重新登录".to_string());
        }
        return Err(format!(
            "Token 刷新失败 (status={}, body={})",
            status, body_text
        ));
    }

    let payload: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 Token 刷新响应失败: {}", e))?;

    let candidate_tokens: &[&[&str]] = &[
        &["token"],
        &["device_token"],
        &["accessToken"],
        &["access_token"],
        &["data", "token"],
        &["data", "device_token"],
        &["data", "accessToken"],
        &["result", "token"],
    ];
    let candidate_refresh: &[&[&str]] = &[
        &["refresh_token"],
        &["refreshToken"],
        &["data", "refresh_token"],
        &["data", "refreshToken"],
        &["result", "refresh_token"],
    ];

    let new_token = candidate_tokens
        .iter()
        .find_map(|path| get_string_at_path(&payload, path))
        .ok_or_else(|| "Token 刷新响应未包含有效 access token".to_string())?;

    let new_refresh_token = candidate_refresh
        .iter()
        .find_map(|path| get_string_at_path(&payload, path))
        .unwrap_or_else(|| refresh_token.to_string());

    let expires_at = get_string_at_path(&payload, &["expires_at"])
        .or_else(|| get_string_at_path(&payload, &["data", "expires_at"]))
        .and_then(|v| parse_expire_timestamp_ms(Some(&v)));

    let refresh_token_expires_at = get_string_at_path(&payload, &["refresh_token_expires_at"])
        .or_else(|| get_string_at_path(&payload, &["data", "refresh_token_expires_at"]))
        .and_then(|v| parse_expire_timestamp_ms(Some(&v)));

    Ok(RefreshedTokenInfo {
        access_token: new_token,
        refresh_token: new_refresh_token,
        expires_at,
        refresh_token_expires_at,
    })
}

fn update_account_token_info(
    account: &mut QoderAccount,
    refreshed_info: &RefreshedTokenInfo,
) {
    let mut user_info = account
        .auth_user_info_raw
        .clone()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    if !user_info.is_object() {
        user_info = Value::Object(serde_json::Map::new());
    }
    if let Some(map) = user_info.as_object_mut() {
        map.insert("token".to_string(), Value::String(refreshed_info.access_token.clone()));
        map.insert("refreshToken".to_string(), Value::String(refreshed_info.refresh_token.clone()));
        map.insert("refresh_token".to_string(), Value::String(refreshed_info.refresh_token.clone()));
        if let Some(expires_at) = refreshed_info.expires_at.as_ref() {
            map.insert("expireTime".to_string(), Value::String(expires_at.clone()));
            map.insert("expiresAt".to_string(), Value::String(expires_at.clone()));
        }
        if let Some(rt_expires_at) = refreshed_info.refresh_token_expires_at.as_ref() {
            map.insert("refreshTokenExpireTime".to_string(), Value::String(rt_expires_at.clone()));
            map.insert("refreshTokenExpiresAt".to_string(), Value::String(rt_expires_at.clone()));
        }
    }
    account.auth_user_info_raw = Some(user_info);
}

pub fn is_cn_account(target: &QoderAccount, channel: Option<QoderChannel>) -> bool {
    // 1. 若显式指定为国内渠道，毫无疑问是国内
    if let Some(ch) = channel {
        if ch.is_cn() {
            return true;
        }
    }

    // 2. 特别注意：阿里巴巴集团员工的 SAML 企业组织账号（如 @qoder.alibaba-inc.com、@qoderwork.alibaba-inc.com）
    // 走的是 Qoder Global 全球统一服务体系（由新加坡节点 openapi.qoder.sh 服务，source=sso.saml.organization），
    // 绝非国内阿里云公有云，绝不能误判为 CN！
    let email = target.email.to_lowercase();
    if email.contains("alibaba-inc.com") {
        return false;
    }

    // 3. 账号特征审查：国内版灵码 / 阿里云专属域名与特征
    if email.contains("qoder.cn")
        || email.ends_with(".cn")
        || email.contains("aliyun")
        || email.contains("alipay")
        || email.contains("taobao")
    {
        return true;
    }

    if let Some(ref uid) = target.user_id {
        let u = uid.to_lowercase();
        if u.starts_with("aliyun-") {
            return true;
        }
    }

    if let Some(ref name) = target.display_name {
        let n = name.to_lowercase();
        if n.starts_with("aliyun-") {
            return true;
        }
    }

    false
}

fn get_candidate_openapi_base_urls(
    target: &QoderAccount,
    channel: Option<QoderChannel>,
) -> Vec<&'static str> {
    if is_cn_account(target, channel) {
        vec![DEFAULT_CN_OPENAPI_BASE_URL, DEFAULT_GLOBAL_OPENAPI_BASE_URL]
    } else {
        vec![DEFAULT_GLOBAL_OPENAPI_BASE_URL, DEFAULT_CN_OPENAPI_BASE_URL]
    }
}

fn build_refresh_user_info_raw(
    target: &QoderAccount,
    access_token: &str,
    user_info_fetched: Option<&Value>,
    user_status: Option<&Value>,
    data_policy: Option<&Value>,
) -> Value {
    let mut user_info = target
        .auth_user_info_raw
        .clone()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    if !user_info.is_object() {
        user_info = Value::Object(serde_json::Map::new());
    }

    if let Some(map) = user_info.as_object_mut() {
        map.insert("token".to_string(), Value::String(access_token.to_string()));
        if get_string_from_object(map, &["securityOauthToken"]).is_none() {
            map.insert(
                "securityOauthToken".to_string(),
                Value::String(access_token.to_string()),
            );
        }
        if get_string_from_object(map, &["email"]).is_none() {
            map.insert("email".to_string(), Value::String(target.email.clone()));
        }
        if get_string_from_object(map, &["id", "uid"]).is_none() {
            if let Some(user_id) = target
                .user_id
                .as_deref()
                .and_then(|value| normalize_non_empty(Some(value)))
            {
                map.insert("id".to_string(), Value::String(user_id));
            }
        }
        if get_string_from_object(map, &["name"]).is_none() {
            if let Some(display_name) = target
                .display_name
                .as_deref()
                .and_then(|value| normalize_non_empty(Some(value)))
            {
                map.insert("name".to_string(), Value::String(display_name));
            }
        }
        if let Some(fetched) = user_info_fetched {
            copy_optional_field(fetched, map, "name", "name");
            copy_optional_field(fetched, map, "email", "email");
            copy_optional_field(fetched, map, "avatarUrl", "avatarUrl");
            copy_optional_field(fetched, map, "avatar_url", "avatarUrl");
            copy_optional_field(fetched, map, "userTag", "userTag");
            copy_optional_field(fetched, map, "user_tag", "userTag");
        }
    }

    if let Some(status) = user_status {
        merge_user_status_into_user_info(&mut user_info, status, data_policy);
    }
    user_info
}

pub async fn refresh_account_from_openapi_for_channel(
    channel: Option<QoderChannel>,
    account_id: &str,
) -> Result<QoderAccount, String> {
    let (ch, mut target) = if let Some(ch) = channel {
        let acc = qoder_account::load_account_for_channel(ch, account_id)
            .ok_or_else(|| format!("{} 账号不存在: {}", ch.display_name(), account_id))?;
        (ch, acc)
    } else {
        qoder_account::find_account_channel(account_id)
            .ok_or_else(|| format!("Qoder 账号不存在: {}", account_id))?
    };

    let client = build_reqwest_client()?;
    let machine_info = match read_qoder_machine_info_cache() {
        Ok(value) => value,
        Err(err) => {
            logger::log_warn(&format!(
                "[Qoder Refresh] 读取官方 machine token 缓存失败，将继续尝试无机器标识链路: {}",
                err
            ));
            None
        }
    };

    let mut current_access_token = extract_access_token_from_account(&target);
    let refresh_token = extract_refresh_token_from_account(&target);

    if current_access_token.is_none() && refresh_token.is_none() {
        return Err("Qoder 账号缺少凭据 (token 与 refreshToken 均为空)，请重新登录".to_string());
    }

    let candidate_base_urls = get_candidate_openapi_base_urls(&target, Some(ch));
    let mut last_error = String::new();

    for base_url in candidate_base_urls {
        let mut active_token = current_access_token.clone();

        // 1. 若当前没有 access_token 但有 refresh_token，直接在当前节点先刷新
        if active_token.is_none() {
            if let Some(ref rt) = refresh_token {
                match request_device_token_refresh(&client, base_url, rt, current_access_token.as_deref(), machine_info.as_ref()).await {
                    Ok(info) => {
                        update_account_token_info(&mut target, &info);
                        let _ = qoder_account::save_account_file_for_channel(ch, &target);
                        active_token = Some(info.access_token.clone());
                        current_access_token = Some(info.access_token);
                    }
                    Err(err) => {
                        last_error = format!("{} 换新 Token 失败: {}", base_url, err);
                        continue;
                    }
                }
            }
        }

        let Some(ref tok) = active_token else {
            continue;
        };

        // 2. 尝试请求额度配额 /api/v2/quota/usage
        let credit_res = fetch_qoder_credit_usage(&client, base_url, tok, machine_info.as_ref()).await;
        let final_token = match credit_res {
            Ok(credit_val) => (tok.clone(), Some(credit_val)),
            Err(ref err) if is_unauthorized_error(err) => {
                // 401 Unauthorized！立即使用 refresh_token 换新！
                if let Some(ref rt) = refresh_token {
                    logger::log_info(&format!(
                        "[Qoder Refresh] 请求配额遇到 401，尝试使用 refresh_token 在 {} 自动换新...",
                        base_url
                    ));
                    match request_device_token_refresh(&client, base_url, rt, Some(tok), machine_info.as_ref()).await {
                        Ok(info) => {
                            update_account_token_info(&mut target, &info);
                            let _ = qoder_account::save_account_file_for_channel(ch, &target);
                            let new_tok = info.access_token;
                            current_access_token = Some(new_tok.clone());
                            // 用新 token 重试请求配额
                            match fetch_qoder_credit_usage(&client, base_url, &new_tok, machine_info.as_ref()).await {
                                Ok(new_credit) => (new_tok, Some(new_credit)),
                                Err(retry_err) => {
                                    last_error = format!("换新 Token 后请求配额仍失败: {}", retry_err);
                                    continue;
                                }
                            }
                        }
                        Err(ref_err) => {
                            last_error = format!("{} 换新 Token 失败: {}", base_url, ref_err);
                            continue;
                        }
                    }
                } else {
                    last_error = format!("{} 凭据已失效且未找到 refresh_token，请重新登录", base_url);
                    continue;
                }
            }
            Err(other_err) => {
                last_error = format!("{} 请求配额失败: {}", base_url, other_err);
                continue;
            }
        };

        let (access_token_ok, credit_usage_raw) = final_token;

        // 3. 请求用户套餐 user/plan
        let user_plan_raw = match fetch_qoder_user_plan(&client, base_url, &access_token_ok, machine_info.as_ref()).await {
            Ok(val) => Some(val),
            Err(err) => {
                logger::log_warn(&format!("[Qoder Refresh] 获取 /api/v2/user/plan 失败，沿用本地缓存: {}", err));
                target.auth_user_plan_raw.clone()
            }
        };

        // 4. 请求用户信息 userinfo
        let user_info_fetched = fetch_qoder_user_info(&client, base_url, &access_token_ok, machine_info.as_ref()).await.ok();

        // 5. 请求状态探针 user/status（非强制！若失败完全不阻断）
        let (user_status, data_policy) = match fetch_qoder_user_status_bundle(&client, base_url, &access_token_ok, machine_info.as_ref()).await {
            Ok(bundle) => (Some(bundle.0), bundle.1),
            Err(err) => {
                logger::log_warn(&format!("[Qoder Refresh] 获取 /api/v3/user/status 状态探针失败（已降级容错）: {}", err));
                (None, None)
            }
        };

        let user_info_raw = build_refresh_user_info_raw(
            &target,
            &access_token_ok,
            user_info_fetched.as_ref(),
            user_status.as_ref(),
            data_policy.as_ref(),
        );

        let mut refreshed = qoder_account::merge_account_snapshot(
            target.clone(),
            Some(user_info_raw),
            user_plan_raw,
            credit_usage_raw,
        );
        refreshed.quota_query_last_error = None;
        refreshed.quota_query_last_error_at = None;
        let saved = qoder_account::upsert_account_record_for_channel(ch, refreshed)?;
        return Ok(saved);
    }

    let final_err = if last_error.is_empty() {
        "刷新 Qoder 额度失败，请检查网络或重新登录".to_string()
    } else {
        last_error
    };
    let _ = qoder_account::update_quota_query_error_for_channel(ch, account_id, Some(final_err.clone()));
    Err(final_err)
}

pub async fn refresh_account_from_openapi(account_id: &str) -> Result<QoderAccount, String> {
    refresh_account_from_openapi_for_channel(None, account_id).await
}

pub async fn refresh_all_accounts_for_channel(channel: QoderChannel) -> Result<i32, String> {
    let accounts = qoder_account::list_accounts_for_channel(channel);
    if accounts.is_empty() {
        return Ok(0);
    }
    let mut success_count = 0;
    for account in accounts {
        match refresh_account_from_openapi_for_channel(Some(channel), &account.id).await {
            Ok(_) => success_count += 1,
            Err(err) => {
                logger::log_warn(&format!(
                    "[Qoder Refresh] [{}] 批量刷新失败: account_id={}, email={}, error={}",
                    channel.as_str(), account.id, account.email, err
                ));
            }
        }
    }
    Ok(success_count)
}

pub async fn refresh_all_accounts_from_openapi() -> Result<i32, String> {
    let mut total_success = 0;
    for &ch in &QoderChannel::ALL {
        if let Ok(count) = refresh_all_accounts_for_channel(ch).await {
            total_success += count;
        }
    }
    Ok(total_success)
}

fn clear_pending_if_matches(login_id: &str) {
    if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
        if guard.as_ref().map(|state| state.login_id.as_str()) == Some(login_id) {
            *guard = None;
        }
    }
}

fn default_cosy_info_path() -> Result<PathBuf, String> {
    let user_data = qoder_instance::get_default_qoder_user_data_dir()?;
    Ok(user_data.join("SharedClientCache"))
}

fn read_qoder_machine_info_cache() -> Result<Option<QoderMachineInfo>, String> {
    let cache_path = default_cosy_info_path()?
        .join("cache")
        .join("machine_token.json");
    if !cache_path.exists() {
        logger::log_warn(&format!(
            "[Qoder OAuth] 未找到官方 machine token 缓存，将跳过机器标识注入: {}",
            cache_path.to_string_lossy()
        ));
        return Ok(None);
    }

    let content = fs::read_to_string(&cache_path)
        .map_err(|err| format!("读取 Qoder machine_token.json 失败: {}", err))?;
    let parsed = serde_json::from_str::<QoderMachineTokenCache>(&content)
        .map_err(|err| format!("解析 Qoder machine_token.json 失败: {}", err))?;

    let token = parsed
        .token
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)));
    let machine_type = parsed
        .machine_type
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)));
    let machine_code = parsed
        .machine_code
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)));
    let machine_id = parsed
        .machine_id
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)));
    let machine_hostname = parsed
        .machine_hostname
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)));
    let machine_os = parsed
        .machine_os
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)));
    let cosy_version = parsed
        .cosy_version
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)));

    logger::log_info(&format!(
        "[Qoder OAuth] 官方 machine token 缓存已加载: path={}, has_token={}, has_machine_type={}",
        cache_path.to_string_lossy(),
        token.is_some(),
        machine_type.is_some()
    ));

    Ok(token.map(|token| QoderMachineInfo {
        token,
        machine_type,
        machine_code,
        machine_id,
        machine_hostname,
        machine_os,
        cosy_version,
    }))
}

fn read_qoder_cached_machine_id() -> Result<Option<String>, String> {
    let cache_path = default_cosy_info_path()?.join("cache").join("id");
    if !cache_path.exists() {
        logger::log_warn(&format!(
            "[Qoder OAuth] 未找到官方 machine id 缓存，将继续使用无机器标识链路: {}",
            cache_path.to_string_lossy()
        ));
        return Ok(None);
    }

    let raw = fs::read_to_string(&cache_path)
        .map_err(|err| format!("读取 Qoder cache/id 失败: {}", err))?;
    let machine_id = normalize_non_empty(Some(raw.trim()));
    logger::log_info(&format!(
        "[Qoder OAuth] 官方 machine id 缓存已加载: path={}, has_machine_id={}",
        cache_path.to_string_lossy(),
        machine_id.is_some()
    ));
    Ok(machine_id)
}

fn summarize_url_for_log(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(url) => {
            let host = url.host_str().unwrap_or("<unknown>");
            let path = url.path();
            let query_keys = url
                .query_pairs()
                .map(|(key, _)| key.to_string())
                .collect::<Vec<String>>();
            if query_keys.is_empty() {
                format!("{}://{}{} (len={})", url.scheme(), host, path, raw.len())
            } else {
                format!(
                    "{}://{}{}?keys={} (len={})",
                    url.scheme(),
                    host,
                    path,
                    query_keys.join(","),
                    raw.len()
                )
            }
        }
        Err(_) => format!("<invalid-url len={}>", raw.len()),
    }
}

fn get_object_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a Value> {
    for key in keys {
        if let Some(value) = object.get(*key) {
            if !value.is_null() {
                return Some(value);
            }
        }
    }
    None
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => normalize_non_empty(Some(text.as_str())),
        Value::Number(number) => {
            let text = number.to_string();
            normalize_non_empty(Some(text.as_str()))
        }
        Value::Bool(flag) => Some(if *flag { "true" } else { "false" }.to_string()),
        _ => None,
    }
}

fn get_string_from_object(
    object: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    get_object_field(object, keys).and_then(value_to_string)
}
pub async fn start_login() -> Result<QoderOAuthStartResponse, String> {
    logger::log_info("[Qoder OAuth] 开始创建登录会话");
    let login_base_url = resolve_qoder_cli_login_endpoint();
    let machine_info = match read_qoder_machine_info_cache() {
        Ok(value) => value,
        Err(err) => {
            logger::log_warn(&format!(
                "[Qoder OAuth] 读取官方 machine token 缓存失败，将继续使用无机器标识链路: {}",
                err
            ));
            None
        }
    };
    let login_machine_id = if let Some(machine_token) = machine_info
        .as_ref()
        .and_then(|value| normalize_non_empty(Some(value.token.as_str())))
    {
        Some(machine_token)
    } else {
        match read_qoder_cached_machine_id() {
            Ok(value) => value,
            Err(err) => {
                logger::log_warn(&format!(
                    "[Qoder OAuth] 读取官方 machine id 缓存失败，将继续使用无机器标识链路: {}",
                    err
                ));
                None
            }
        }
    };
    let expected_nonce = generate_login_nonce();
    let code_verifier = generate_pkce_verifier();
    let challenge_method = QODER_DEVICE_LOGIN_CHALLENGE_METHOD.to_string();
    let code_challenge = generate_pkce_challenge(&code_verifier);
    let verification_uri = build_cli_device_login_url(
        &login_base_url,
        &expected_nonce,
        &code_challenge,
        &challenge_method,
        login_machine_id.as_deref(),
    )?;
    let login_machine_id_source = if machine_info.is_some() {
        "machine_token"
    } else if login_machine_id.is_some() {
        "cache_id"
    } else {
        "none"
    };
    let login_id = Uuid::new_v4().to_string();
    logger::log_info(&format!(
        "[Qoder OAuth] 已生成官方 CLI device login 链接: login_id={}, login_base_url={}, verification_uri={}, nonce_len={}, has_machine_token={}, has_machine_type={}, has_login_machine_id={}, login_machine_id_source={}",
        login_id,
        summarize_url_for_log(&login_base_url),
        summarize_url_for_log(&verification_uri),
        expected_nonce.len(),
        machine_info.is_some(),
        machine_info
            .as_ref()
            .and_then(|value| value.machine_type.as_deref())
            .is_some(),
        login_machine_id.is_some(),
        login_machine_id_source
    ));

    let state = PendingOAuthState {
        login_id: login_id.clone(),
        expected_nonce: expected_nonce.clone(),
        code_verifier,
        challenge_method: challenge_method.clone(),
        openapi_base_url: DEFAULT_OPENAPI_BASE_URL.to_string(),
        machine_info,
        verification_uri: verification_uri.clone(),
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS,
        cancelled: false,
    };

    {
        let mut guard = PENDING_OAUTH_STATE
            .lock()
            .map_err(|_| "获取 Qoder OAuth 状态锁失败".to_string())?;
        *guard = Some(state);
    }

    logger::log_info(&format!(
        "[Qoder OAuth] 登录会话已创建: login_id={}, redirect_uri={}, expires_in={}s",
        login_id, QODER_IDE_REDIRECT_URI, OAUTH_TIMEOUT_SECONDS
    ));

    Ok(QoderOAuthStartResponse {
        login_id,
        verification_uri,
        expires_in: OAUTH_TIMEOUT_SECONDS as u64,
        interval_seconds: (OAUTH_POLL_INTERVAL_MS / 1000).max(1),
        callback_url: None,
    })
}

pub async fn complete_login(login_id: &str) -> Result<QoderAccount, String> {
    logger::log_info(&format!(
        "[Qoder OAuth] 开始等待回调完成: login_id={}",
        login_id
    ));
    let wait_started = Instant::now();
    let mut next_wait_log_at = Duration::from_secs(5);
    let client = build_reqwest_client()?;
    let mut last_poll_error: Option<String> = None;

    loop {
        let snapshot = {
            let guard = PENDING_OAUTH_STATE
                .lock()
                .map_err(|_| "获取 Qoder OAuth 状态锁失败".to_string())?;
            let state = guard
                .as_ref()
                .ok_or_else(|| "没有进行中的 Qoder OAuth 登录会话".to_string())?;

            if state.login_id != login_id {
                return Err("Qoder OAuth 登录会话已变更，请重新发起".to_string());
            }
            if state.cancelled {
                return Err("Qoder OAuth 登录已取消".to_string());
            }
            if now_timestamp() > state.expires_at {
                clear_pending_if_matches(login_id);
                return Err(
                    last_poll_error.unwrap_or_else(|| "Qoder OAuth 登录已超时，请重试".to_string())
                );
            }

            (
                state.expected_nonce.clone(),
                state.code_verifier.clone(),
                state.challenge_method.clone(),
                state.openapi_base_url.clone(),
                state.machine_info.clone(),
            )
        };

        match poll_device_token_once(&client, &snapshot.3, &snapshot.0, &snapshot.1, &snapshot.2)
            .await
        {
            Ok(Some(token_data)) => {
                logger::log_info(&format!(
                    "[Qoder OAuth] deviceToken/poll 命中: login_id={}, elapsed={}ms",
                    login_id,
                    wait_started.elapsed().as_millis()
                ));

                let access_token = normalize_non_empty(token_data.token.as_deref())
                    .ok_or_else(|| "Qoder device token 响应缺少 token".to_string())?;

                let user_info_response = match fetch_qoder_user_info(
                    &client,
                    &snapshot.3,
                    &access_token,
                    snapshot.4.as_ref(),
                )
                .await
                {
                    Ok(value) => Some(value),
                    Err(err) => {
                        logger::log_warn(&format!(
                            "[Qoder OAuth] 获取 /userinfo 失败，将继续使用 user/status: {}",
                            err
                        ));
                        None
                    }
                };

                let (user_status, data_policy) = fetch_qoder_user_status_bundle(
                    &client,
                    &snapshot.3,
                    &access_token,
                    snapshot.4.as_ref(),
                )
                .await?;

                let mut user_info_raw =
                    build_initial_user_info_raw(&token_data, user_info_response.as_ref());
                merge_user_status_into_user_info(
                    &mut user_info_raw,
                    &user_status,
                    data_policy.as_ref(),
                );

                let user_plan_raw = match fetch_qoder_user_plan(
                    &client,
                    &snapshot.3,
                    &access_token,
                    snapshot.4.as_ref(),
                )
                .await
                {
                    Ok(value) => Some(value),
                    Err(err) => {
                        logger::log_warn(&format!(
                            "[Qoder OAuth] 获取 /api/v2/user/plan 失败，将以缺省快照继续: {}",
                            err
                        ));
                        None
                    }
                };

                let credit_usage_raw = match fetch_qoder_credit_usage(
                    &client,
                    &snapshot.3,
                    &access_token,
                    snapshot.4.as_ref(),
                )
                .await
                {
                    Ok(value) => Some(value),
                    Err(err) => {
                        logger::log_warn(&format!(
                            "[Qoder OAuth] 获取 /api/v2/quota/usage 失败，将以缺省快照继续: {}",
                            err
                        ));
                        None
                    }
                };

                let account = qoder_account::upsert_account_from_snapshot(
                    user_info_raw,
                    user_plan_raw,
                    credit_usage_raw,
                )?;
                clear_pending_if_matches(login_id);
                logger::log_info(&format!(
                    "[Qoder OAuth] 登录完成并入库成功: login_id={}, account_id={}, email={}",
                    login_id, account.id, account.email
                ));
                return Ok(account);
            }
            Ok(None) => {}
            Err(err) => {
                last_poll_error = Some(err.clone());
                logger::log_warn(&format!(
                    "[Qoder OAuth] deviceToken/poll 失败，等待重试: login_id={}, error={}",
                    login_id, err
                ));
            }
        }

        let elapsed = wait_started.elapsed();
        if elapsed >= next_wait_log_at {
            logger::log_info(&format!(
                "[Qoder OAuth] 等待 device token 中: login_id={}, elapsed={}s",
                login_id,
                elapsed.as_secs()
            ));
            next_wait_log_at += Duration::from_secs(5);
        }
        tokio::time::sleep(Duration::from_millis(OAUTH_POLL_INTERVAL_MS)).await;
    }
}

pub fn peek_pending_login() -> Option<QoderOAuthStartResponse> {
    let guard = PENDING_OAUTH_STATE.lock().ok()?;
    let state = guard.as_ref()?;
    if state.cancelled {
        return None;
    }
    let now = now_timestamp();
    if now > state.expires_at {
        return None;
    }

    Some(QoderOAuthStartResponse {
        login_id: state.login_id.clone(),
        verification_uri: state.verification_uri.clone(),
        expires_in: (state.expires_at - now).max(0) as u64,
        interval_seconds: (OAUTH_POLL_INTERVAL_MS / 1000).max(1),
        callback_url: None,
    })
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    let mut guard = PENDING_OAUTH_STATE
        .lock()
        .map_err(|_| "获取 Qoder OAuth 状态锁失败".to_string())?;

    let Some(current) = guard.as_ref() else {
        return Ok(());
    };

    if let Some(target) = login_id {
        if current.login_id != target {
            return Ok(());
        }
    }

    logger::log_info(&format!(
        "[Qoder OAuth] 取消登录会话: login_id={}",
        current.login_id
    ));

    *guard = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // lgtm[rs/hardcoded-credentials] test-nonce 和 test-challenge 是单元测试专用占位字符串，不用于生产认证
    fn builds_current_ide_device_login_url() {
        let url = build_cli_device_login_url(
            DEFAULT_LOGIN_BASE_URL,
            "test-nonce",
            "test-challenge",
            QODER_DEVICE_LOGIN_CHALLENGE_METHOD,
            Some("test-machine-id"),
        )
        .expect("build login url");

        let parsed = Url::parse(&url).expect("parse login url");
        let query = parsed
            .query_pairs()
            .into_owned()
            .collect::<Vec<(String, String)>>();

        assert!(query.contains(&("nonce".to_string(), "test-nonce".to_string())));
        assert!(query.contains(&("challenge".to_string(), "test-challenge".to_string())));
        assert!(query.contains(&(
            "challenge_method".to_string(),
            QODER_DEVICE_LOGIN_CHALLENGE_METHOD.to_string()
        )));
        assert!(query.contains(&(
            "redirect_uri".to_string(),
            QODER_IDE_REDIRECT_URI.to_string()
        )));
        assert!(query.contains(&("machine_id".to_string(), "test-machine-id".to_string())));
        assert!(!query.iter().any(|(key, _)| key == "client_id"));
    }

    #[test]
    fn builds_current_official_cosy_headers_from_machine_cache() {
        let machine = QoderMachineInfo {
            token: "machine-token".to_string(),
            machine_type: Some("machine-type".to_string()),
            machine_code: Some("machine-code".to_string()),
            machine_id: Some("machine-id".to_string()),
            machine_hostname: Some("machine-hostname".to_string()),
            machine_os: Some("aarch64_darwin".to_string()),
            cosy_version: Some("1.27.1".to_string()),
        };
        let headers = build_qoder_headers("access-token", Some(&machine));

        assert_eq!(headers["Cosy-Version"], "1.27.1");
        assert_eq!(headers["Cosy-MachineToken"], "machine-token");
        assert_eq!(headers["Cosy-MachineType"], "machine-type");
        assert_eq!(headers["Cosy-MachineCode"], "machine-code");
        assert_eq!(headers["Cosy-MachineId"], "machine-id");
        assert_eq!(headers["Cosy-MachineHostname"], "machine-hostname");
        assert_eq!(headers["Cosy-MachineOS"], "aarch64_darwin");
        assert_eq!(headers["Cosy-ClientType"], "0");
    }
}
