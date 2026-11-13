use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, Instant};
use url::Url;
use uuid::Uuid;

use crate::models::qoder::{QoderAccount, QoderOAuthStartResponse};
use crate::modules::{logger, qoder_account, qoder_identity, qoder_instance};

const REAUTH_REQUIRED: &str = "QODER_REAUTH_REQUIRED";
const OPENAPI_UNAVAILABLE: &str = "QODER_SESSION_ENDPOINT_UNAVAILABLE";
const UNAUTHORIZED: &str = "QODER_OPENAPI_UNAUTHORIZED";
const RENEWAL_WINDOW_MS: i64 = 3 * 24 * 60 * 60 * 1000;
type RefreshLock = tokio::sync::Mutex<()>;
static REFRESH_LOCKS: LazyLock<Mutex<HashMap<String, Weak<RefreshLock>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const OAUTH_TIMEOUT_SECONDS: i64 = 600;
const OAUTH_POLL_INTERVAL_MS: u64 = 1000;
const DEFAULT_LOGIN_BASE_URL: &str = "https://qoder.com/device/selectAccounts";
const DEFAULT_OPENAPI_BASE_URL: &str = "https://openapi.qoder.sh";
const QODER_IDE_REDIRECT_URI: &str = "qoder-ide://aicoding.aicoding-agent/login-success";
const QODER_DEVICE_LOGIN_CHALLENGE_METHOD: &str = "S256";
const DEVICE_TOKEN_REFRESH_PATH: &str = "/api/v1/deviceToken/refresh";
const DEVICE_TOKEN_POLL_PATH: &str = "/api/v1/deviceToken/poll";
const USER_INFO_PATH: &str = "/api/v1/userinfo";
const USER_STATUS_PATH: &str = "/api/v3/user/status";
const DATA_POLICY_PATH: &str = "/api/v2/config/getDataPolicy";
const USER_PLAN_PATH: &str = "/api/v2/user/plan";
const CREDIT_USAGE_PATH: &str = "/sash/api/v2/me/usage";
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
    redirect_uri: &str,
    nonce: &str,
    challenge: &str,
    challenge_method: &str,
    machine_token: Option<&str>,
    machine_id: Option<&str>,
) -> Result<String, String> {
    let mut url =
        Url::parse(login_base_url).map_err(|err| format!("解析 Qoder 登录地址失败: {}", err))?;
    {
        let mut query_pairs = url.query_pairs_mut();
        query_pairs.append_pair("nonce", nonce);
        query_pairs.append_pair("challenge", challenge);
        query_pairs.append_pair("challenge_method", challenge_method);
        query_pairs.append_pair("redirect_uri", redirect_uri);
        if let Some(machine_token) =
            machine_token.and_then(|value| normalize_non_empty(Some(value)))
        {
            query_pairs.append_pair("machine_token", &machine_token);
        }
        if let Some(machine_id) = machine_id.and_then(|value| normalize_non_empty(Some(value))) {
            query_pairs.append_pair("machine_id", &machine_id);
        }
    }
    Ok(url.to_string())
}

fn installed_ide_redirect_uri() -> &'static str {
    let Some(executable) = crate::modules::process::resolve_qoder_launch_path().ok() else {
        return QODER_IDE_REDIRECT_URI;
    };
    let legacy = build_qoder_product_file_candidates(&executable)
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "product.json"))
        .filter_map(|path| fs::read(path).ok())
        .find_map(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .is_some_and(|product| {
            product.get("deeplinkUrlProtocol").is_none()
                && product.get("urlProtocol").and_then(Value::as_str) == Some("qoder")
        });
    if legacy {
        "qoder://aicoding.aicoding-agent/login-success"
    } else {
        QODER_IDE_REDIRECT_URI
    }
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

    if whitelist_status.as_deref() == Some("LoginExpire") {
        return Err(UNAUTHORIZED.into());
    }
    if !has_user_id {
        return Err("Qoder 用户状态缺少 id，无法确认登录身份".to_string());
    }

    match whitelist_status.as_deref() {
        Some("NoIpPermission") => Err("企业设置了 IP 白名单，当前 IP 无法登录".to_string()),
        Some("AppDisable") => Err("Qoder 应用已被停用，无法登录".to_string()),
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
        let root = if base_path.is_file() {
            base_path.parent().unwrap_or(base_path)
        } else {
            base_path
        };
        app_roots.push(root.to_path_buf());
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
    let executable = crate::modules::process::resolve_qoder_launch_path().ok()?;
    build_qoder_product_file_candidates(&executable)
        .iter()
        .find_map(|path| read_version_from_json_file(path))
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
    if qoder_identity::field(Some(user_status), "orgId").is_some() {
        copy_optional_field(user_status, map, "orgId", "orgId");
    }
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
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let same_origin = attempt
                .previous()
                .first()
                .is_some_and(|url| url.origin() == attempt.url().origin());
            if same_origin && attempt.previous().len() < 10 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
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
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(UNAUTHORIZED.into());
    }
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
    _machine_info: Option<&QoderMachineInfo>,
) -> Result<Value, String> {
    // Official 1.38 uses raw Authorization headers for the usage endpoint.
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", token)
            .parse()
            .map_err(|_| "QODER_USAGE_INVALID_AUTH_HEADER".to_string())?,
    );
    let value =
        fetch_openapi_json(client, openapi_base_url, CREDIT_USAGE_PATH, headers, &[]).await?;
    if !crate::modules::qoder_usage::is_envelope(&value)
        && value.get("totalUsagePercentage").is_none()
        && value.get("userQuota").is_none()
    {
        return Err("QODER_USAGE_RESPONSE_INVALID".into());
    }
    Ok(value)
}

fn get_string_at_path(root: &Value, path: &[&str]) -> Option<String> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    value_to_string(current)
}

pub(crate) fn access_token_from_info(user_info: Option<&Value>) -> Option<String> {
    let user_info = user_info?;
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

fn ensure_refresh_identity_consistent(
    target: &QoderAccount,
    user_status: &Value,
) -> Result<(), String> {
    let target_user_id = target
        .user_id
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)));
    let status_user_id = user_status
        .get("id")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_non_empty(Some(value)));

    if let (Some(target_uid), Some(status_uid)) = (target_user_id.as_ref(), status_user_id.as_ref())
    {
        if !target_uid.eq_ignore_ascii_case(status_uid) {
            return Err(format!(
                "官方接口返回账号与目标账号不一致: target_user_id={}, actual_user_id={}",
                target_uid, status_uid
            ));
        }
    } else {
        let target_email =
            normalize_non_empty(Some(target.email.as_str())).map(|value| value.to_lowercase());
        let status_email = user_status
            .get("email")
            .and_then(|value| value.as_str())
            .and_then(|value| normalize_non_empty(Some(value)))
            .map(|value| value.to_lowercase());
        if let (Some(left), Some(right)) = (target_email.as_ref(), status_email.as_ref()) {
            if left != right {
                return Err(format!(
                    "官方接口返回账号与目标账号不一致: target_email={}, actual_email={}",
                    left, right
                ));
            }
        }
    }

    let scope = qoder_identity::scope(
        target.auth_user_info_raw.as_ref(),
        target.auth_user_plan_raw.as_ref(),
    );
    let returned = qoder_identity::scope(Some(user_status), None);
    if (scope.1.is_some() && user_status.get("orgId").is_some() && scope.1 != returned.1)
        || matches!((scope.0, returned.0), (Some(left), Some(right)) if left != right)
    {
        return Err("QODER_SESSION_CHANGED".into());
    }
    Ok(())
}

fn build_refresh_user_info_raw(
    target: &QoderAccount,
    access_token: &str,
    user_status: &Value,
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
    }

    merge_user_status_into_user_info(&mut user_info, user_status, data_policy);
    user_info
}

fn refresh_lock(account_id: &str) -> Result<Arc<RefreshLock>, String> {
    let mut locks = REFRESH_LOCKS.lock().map_err(|_| "QODER_ACCOUNT_BUSY")?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(account_id).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(RefreshLock::new(()));
    locks.insert(account_id.to_string(), Arc::downgrade(&lock));
    Ok(lock)
}

pub(crate) fn token_expiry(info: Option<&Value>, key: &str) -> Option<i64> {
    let value = info?.get(key)?;
    let raw = value_to_string(value)?;
    parse_expire_timestamp_ms(Some(&raw))?.parse().ok()
}

fn session_endpoint(account: &QoderAccount) -> Result<String, String> {
    let (source, _) = qoder_identity::scope(
        account.auth_user_info_raw.as_ref(),
        account.auth_user_plan_raw.as_ref(),
    );
    if matches!(source, Some("qodercn" | "dedicated_qodercn")) {
        if let Some(origin) = account.verified_openapi_origin.as_deref() {
            return Ok(origin.to_string());
        }
    }
    match source {
        None | Some("qoder") => Ok(DEFAULT_OPENAPI_BASE_URL.to_string()),
        Some("dedicated_qodercn") => {
            let root = qoder_instance::get_default_qoder_user_data_dir()?;
            if !qoder_account::local_session_matches(&root, account)? {
                return Err(OPENAPI_UNAVAILABLE.into());
            }
            let config = fs::read(root.join("User/app.json")).map_err(|_| OPENAPI_UNAVAILABLE)?;
            let config: Value = serde_json::from_slice(&config).map_err(|_| OPENAPI_UNAVAILABLE)?;
            let configured = config
                .pointer("/config/general/dedicatedUrl")
                .and_then(Value::as_str)
                .ok_or(OPENAPI_UNAVAILABLE)?;
            dedicated_openapi_endpoint(configured).ok_or_else(|| OPENAPI_UNAVAILABLE.into())
        }
        Some("qodercn") => {
            let root = qoder_instance::get_default_qoder_user_data_dir()?;
            if !qoder_account::local_session_matches(&root, account)? {
                return Err(OPENAPI_UNAVAILABLE.into());
            }
            let executable = crate::modules::process::resolve_qoder_launch_path()?;
            let flags = build_qoder_product_file_candidates(&executable)
                .into_iter()
                .filter_map(|path| fs::read(path.with_file_name("featureFlags.json")).ok())
                .find_map(|raw| serde_json::from_slice::<Value>(&raw).ok())
                .ok_or(OPENAPI_UNAVAILABLE)?;
            let url = flags
                .get("defaultOpenApiBaseUrl")
                .and_then(Value::as_str)
                .and_then(|value| Url::parse(value).ok())
                .ok_or(OPENAPI_UNAVAILABLE)?;
            let hostname = url.host_str().unwrap_or_default();
            let official = ["qoder.cn", "qoder.com.cn"]
                .iter()
                .any(|suffix| hostname == *suffix || hostname.ends_with(&format!(".{suffix}")));
            if url.scheme() == "https" && official {
                return Ok(url.origin().ascii_serialization());
            }
            Err(OPENAPI_UNAVAILABLE.into())
        }
        // CN 1.0 sessions use Cosy rather than the device-token OpenAPI.
        _ => Err(OPENAPI_UNAVAILABLE.into()),
    }
}

fn dedicated_openapi_endpoint(configured: &str) -> Option<String> {
    let mut url = Url::parse(configured.trim()).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if let Some(url::Host::Domain(host)) = url.host() {
        if let Some((first, rest)) = host.split_once('.') {
            url.set_host(Some(&format!("{first}-openapi.{rest}")))
                .ok()?;
        }
    }
    Some(url.origin().ascii_serialization())
}

#[derive(Deserialize)]
struct RenewedDeviceToken {
    device_token: String,
    refresh_token: String,
    expires_at: String,
    refresh_token_expires_at: Option<String>,
}

async fn request_renewed_token(
    client: &reqwest::Client,
    endpoint: &str,
    access: &str,
    refresh_token: &str,
) -> Result<RenewedDeviceToken, String> {
    let response = client
        .post(format!("{endpoint}{DEVICE_TOKEN_REFRESH_PATH}"))
        .bearer_auth(access)
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&serde_json::json!({ "refresh_token": refresh_token }))
        .send()
        .await
        .map_err(|_| "QODER_RENEWAL_FAILED")?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(REAUTH_REQUIRED.into());
    }
    if status == reqwest::StatusCode::BAD_REQUEST {
        let body = response.text().await.unwrap_or_default();
        if body.contains("invalid refresh_token") || body.contains("invalid_grant") {
            return Err(REAUTH_REQUIRED.into());
        }
        return Err(format!("QODER_RENEWAL_FAILED: {status}"));
    }
    if !status.is_success() {
        return Err(format!("QODER_RENEWAL_FAILED: {status}"));
    }

    response
        .json()
        .await
        .map_err(|_| "QODER_RENEWAL_FAILED".into())
}

async fn renew_device_token(
    client: &reqwest::Client,
    endpoint: &str,
    account: &QoderAccount,
) -> Result<QoderAccount, String> {
    let info = account.auth_user_info_raw.as_ref();
    let refresh_token = qoder_identity::field(info, "refreshToken").ok_or(REAUTH_REQUIRED)?;
    if token_expiry(info, "refreshTokenExpireTime")
        .is_some_and(|expires| expires <= chrono::Utc::now().timestamp_millis())
    {
        return Err(REAUTH_REQUIRED.into());
    }
    let access = access_token_from_info(info).ok_or(REAUTH_REQUIRED)?;
    let tokens = request_renewed_token(client, endpoint, &access, refresh_token).await?;
    let mut updated = info
        .and_then(Value::as_object)
        .cloned()
        .ok_or(REAUTH_REQUIRED)?;
    let device_token =
        normalize_non_empty(Some(&tokens.device_token)).ok_or("QODER_RENEWAL_FAILED")?;
    let refresh_token =
        normalize_non_empty(Some(&tokens.refresh_token)).ok_or("QODER_RENEWAL_FAILED")?;
    let expiry =
        parse_expire_timestamp_ms(Some(&tokens.expires_at)).ok_or("QODER_RENEWAL_FAILED")?;
    updated.insert("token".into(), Value::String(device_token.clone()));
    updated.insert("securityOauthToken".into(), Value::String(device_token));
    updated.insert("refreshToken".into(), Value::String(refresh_token));
    updated.insert("expireTime".into(), Value::String(expiry));
    updated.remove("refreshTokenExpireTime");
    insert_string_field(
        &mut updated,
        "refreshTokenExpireTime",
        parse_expire_timestamp_ms(tokens.refresh_token_expires_at.as_deref()),
    );

    // Persist the rotated token before making further requests that can fail.
    let verified_origin = (endpoint != DEFAULT_OPENAPI_BASE_URL).then(|| endpoint.to_string());
    let account = account.clone();
    tokio::task::spawn_blocking(move || {
        qoder_account::save_refresh(
            &account,
            Value::Object(updated),
            None,
            None,
            account.quota_query_last_error.clone(),
            verified_origin,
        )
    })
    .await
    .map_err(|_| "QODER_ACCOUNT_BUSY")?
}

async fn load_refresh_target(account_id: &str) -> Result<QoderAccount, String> {
    let id = account_id.to_string();
    tokio::task::spawn_blocking(move || {
        qoder_account::load_account(&id).ok_or_else(|| "QODER_SESSION_CHANGED".to_string())
    })
    .await
    .map_err(|_| "QODER_ACCOUNT_BUSY")?
}

async fn account_endpoint(account: &QoderAccount) -> Result<String, String> {
    let account = account.clone();
    tokio::task::spawn_blocking(move || session_endpoint(&account))
        .await
        .map_err(|_| "QODER_ACCOUNT_BUSY")?
}

pub(crate) async fn prepare_account_for_switch(account_id: &str) -> Result<(), String> {
    let target = load_refresh_target(account_id).await?;
    let info = target.auth_user_info_raw.as_ref();
    access_token_from_info(info).ok_or(REAUTH_REQUIRED)?;
    if !token_expiry(info, "expireTime")
        .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp_millis())
    {
        return Ok(());
    }
    let lock = refresh_lock(account_id)?;
    let _guard = lock.lock().await;
    let target = load_refresh_target(account_id).await?;
    let info = target.auth_user_info_raw.as_ref();
    access_token_from_info(info).ok_or(REAUTH_REQUIRED)?;
    if token_expiry(info, "expireTime")
        .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp_millis())
    {
        let endpoint = account_endpoint(&target).await?;
        renew_device_token(&build_reqwest_client()?, &endpoint, &target).await?;
    }
    Ok(())
}

async fn refresh_account_from_openapi_once(
    mut target: QoderAccount,
) -> Result<QoderAccount, String> {
    let endpoint = account_endpoint(&target).await?;
    let client = build_reqwest_client()?;
    let machine_info = tokio::task::spawn_blocking(read_qoder_machine_info_cache)
        .await
        .map_err(|_| "QODER_ACCOUNT_BUSY")?
        .map_err(|error| logger::log_warn(&format!("[Qoder Refresh] 读取机器信息失败: {error}")))
        .ok()
        .flatten();

    let now = chrono::Utc::now().timestamp_millis();
    let info = target.auth_user_info_raw.as_ref();
    let expiry = token_expiry(info, "expireTime");
    let refresh_expiry = token_expiry(info, "refreshTokenExpireTime");
    let mut renewed = expiry.is_some_and(|expiry| expiry <= now)
        || (expiry.is_some_and(|expiry| expiry - now < RENEWAL_WINDOW_MS)
            && qoder_identity::field(info, "refreshToken").is_some()
            && refresh_expiry.is_none_or(|expiry| expiry - now >= RENEWAL_WINDOW_MS));
    if renewed {
        target = renew_device_token(&client, &endpoint, &target).await?;
    }
    let mut access =
        access_token_from_info(target.auth_user_info_raw.as_ref()).ok_or(REAUTH_REQUIRED)?;
    let (user_status, data_policy) = loop {
        match fetch_qoder_user_status_bundle(&client, &endpoint, &access, machine_info.as_ref())
            .await
        {
            Err(error) if error == UNAUTHORIZED && !renewed => {
                target = renew_device_token(&client, &endpoint, &target).await?;
                access = access_token_from_info(target.auth_user_info_raw.as_ref())
                    .ok_or(REAUTH_REQUIRED)?;
                renewed = true;
            }
            Err(error) if error == UNAUTHORIZED => return Err(REAUTH_REQUIRED.into()),
            result => break result?,
        }
    };
    ensure_refresh_identity_consistent(&target, &user_status)?;
    let user_info =
        build_refresh_user_info_raw(&target, &access, &user_status, data_policy.as_ref());

    let (plan, usage) = tokio::join!(
        fetch_qoder_user_plan(&client, &endpoint, &access, machine_info.as_ref()),
        fetch_qoder_credit_usage(&client, &endpoint, &access, machine_info.as_ref())
    );
    let plan = plan
        .map_err(|error| logger::log_warn(&format!("[Qoder Refresh] 套餐查询失败: {error}")))
        .ok();
    let (usage, quota_error) = match usage {
        Ok(usage) => (Some(usage), None),
        Err(error) => (None, Some(error)),
    };
    let verified_origin = (endpoint != DEFAULT_OPENAPI_BASE_URL).then_some(endpoint);
    tokio::task::spawn_blocking(move || {
        qoder_account::save_refresh(
            &target,
            user_info,
            plan,
            usage,
            quota_error,
            verified_origin,
        )
    })
    .await
    .map_err(|_| "QODER_ACCOUNT_BUSY")?
}

pub async fn refresh_account_from_openapi(account_id: &str) -> Result<QoderAccount, String> {
    let lock = refresh_lock(account_id)?;
    let _guard = lock.lock().await;
    let target = load_refresh_target(account_id).await?;
    let result = refresh_account_from_openapi_once(target.clone()).await;
    if let Err(error) = &result {
        if error != "QODER_SESSION_CHANGED" {
            let error = error.clone();
            let _ = tokio::task::spawn_blocking(move || {
                qoder_account::mark_quota_query_error(&target, error)
            })
            .await;
        }
    }
    result
}

pub async fn refresh_all_accounts_from_openapi() -> Result<i32, String> {
    let account_ids = tokio::task::spawn_blocking(qoder_account::list_account_ids)
        .await
        .map_err(|_| "QODER_ACCOUNT_BUSY")??;
    let mut success_count = 0;
    for account_id in account_ids {
        match refresh_account_from_openapi(&account_id).await {
            Ok(account) if account.quota_query_last_error.is_none() => success_count += 1,
            Ok(_) => {}
            Err(error) => logger::log_warn(&format!(
                "[Qoder Refresh] 批量刷新失败: account_id={account_id}, error={error}"
            )),
        }
    }

    Ok(success_count)
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
    tokio::task::spawn_blocking(start_login_blocking)
        .await
        .map_err(|_| "QODER_ACCOUNT_BUSY")?
}

fn start_login_blocking() -> Result<QoderOAuthStartResponse, String> {
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
    let machine_id = machine_info
        .as_ref()
        .and_then(|value| value.machine_id.as_deref());
    let cached_machine_id = if machine_id.is_none() {
        read_qoder_cached_machine_id()
            .map_err(|error| {
                logger::log_warn(&format!("[Qoder OAuth] 读取机器标识失败: {error}"));
            })
            .ok()
            .flatten()
    } else {
        None
    };
    let login_machine_id = machine_id.or(cached_machine_id.as_deref());
    let expected_nonce = generate_login_nonce();
    let code_verifier = generate_pkce_verifier();
    let challenge_method = QODER_DEVICE_LOGIN_CHALLENGE_METHOD.to_string();
    let code_challenge = generate_pkce_challenge(&code_verifier);
    let verification_uri = build_cli_device_login_url(
        &login_base_url,
        installed_ide_redirect_uri(),
        &expected_nonce,
        &code_challenge,
        &challenge_method,
        machine_info.as_ref().map(|info| info.token.as_str()),
        login_machine_id,
    )?;
    let login_machine_id_source = if machine_id.is_some() {
        "machine_info"
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
    let completion = refresh_lock(&format!("login:{login_id}"))?;
    let _completion = completion.try_lock().map_err(|_| "QODER_ACCOUNT_BUSY")?;
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
                drop(guard);
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
                            "[Qoder OAuth] 获取 /sash/api/v2/me/usage 失败，将以缺省快照继续: {}",
                            err
                        ));
                        None
                    }
                };

                let id = login_id.to_string();
                let account = tokio::task::spawn_blocking(move || {
                    let state = {
                        let mut pending = PENDING_OAUTH_STATE
                            .lock()
                            .map_err(|_| "QODER_ACCOUNT_BUSY")?;
                        if !pending.as_ref().is_some_and(|state| {
                            state.login_id == id
                                && !state.cancelled
                                && now_timestamp() <= state.expires_at
                        }) {
                            return Err("QODER_SESSION_CHANGED".into());
                        }
                        pending.take()
                    };
                    let result = qoder_account::upsert_account_from_snapshot(
                        user_info_raw,
                        user_plan_raw,
                        credit_usage_raw,
                    );
                    if result.is_err() {
                        if let Ok(mut pending) = PENDING_OAUTH_STATE.lock() {
                            if pending.is_none() {
                                *pending = state;
                            }
                        }
                    }
                    result
                })
                .await
                .map_err(|_| "QODER_ACCOUNT_BUSY")??;
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
            QODER_IDE_REDIRECT_URI,
            "test-nonce",
            "test-challenge",
            QODER_DEVICE_LOGIN_CHALLENGE_METHOD,
            Some("test-machine-token"),
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
        assert!(query.contains(&(
            "machine_token".to_string(),
            "test-machine-token".to_string()
        )));
        assert!(!query.iter().any(|(key, _)| key == "client_id"));
    }

    #[test]
    fn dedicated_openapi_is_derived_from_the_configured_https_origin() {
        assert_eq!(
            dedicated_openapi_endpoint("https://tenant.example.invalid:8443/setup").as_deref(),
            Some("https://tenant-openapi.example.invalid:8443")
        );
        assert!(dedicated_openapi_endpoint("http://tenant.example.invalid").is_none());
        assert!(dedicated_openapi_endpoint("https://user@tenant.example.invalid").is_none());
    }

    #[tokio::test]
    async fn renews_device_credentials_and_distinguishes_reauthorization() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let responses = [
                (
                    "200 OK",
                    r#"{"device_token":"new-device","refresh_token":"new-refresh","expires_at":"2031-01-01T00:00:00Z","refresh_token_expires_at":"2031-02-01T00:00:00Z"}"#,
                ),
                ("400 Bad Request", r#"{"error":"invalid_grant"}"#),
                ("503 Service Unavailable", r#"{"error":"unavailable"}"#),
            ];
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 512];
                loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    let request = std::str::from_utf8(&request).unwrap();
                    if request.ends_with(r#"{"refresh_token":"test-refresh"}"#) {
                        assert!(request.starts_with("POST /api/v1/deviceToken/refresh HTTP/1.1"));
                        assert!(request.contains("authorization: Bearer test-device"));
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let renewed = request_renewed_token(&client, &endpoint, "test-device", "test-refresh")
            .await
            .unwrap();
        assert_eq!(renewed.device_token, "new-device");
        assert_eq!(renewed.refresh_token, "new-refresh");
        assert!(parse_expire_timestamp_ms(Some(&renewed.expires_at)).is_some());
        assert_eq!(
            request_renewed_token(&client, &endpoint, "test-device", "test-refresh")
                .await
                .err()
                .as_deref(),
            Some(REAUTH_REQUIRED)
        );
        assert_eq!(
            request_renewed_token(&client, &endpoint, "test-device", "test-refresh")
                .await
                .err()
                .as_deref(),
            Some("QODER_RENEWAL_FAILED: 503 Service Unavailable")
        );
        server.await.unwrap();
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
