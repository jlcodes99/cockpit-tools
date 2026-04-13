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
use crate::modules::{config, logger, qoder_account, qoder_instance};

const OAUTH_TIMEOUT_SECONDS: i64 = 600;
const OAUTH_POLL_INTERVAL_MS: u64 = 1000;
const DEFAULT_LOGIN_BASE_URL: &str = "https://qoder.com/device/selectAccounts";
const DEFAULT_OPENAPI_BASE_URL: &str = "https://openapi.qoder.sh";
const QODER_CLI_BROWSER_LOGIN_CLIENT_ID: &str = "e883ade2-e6e3-4d6d-adf7-f92ceff5fdcb";
const QODER_DEVICE_LOGIN_CHALLENGE_METHOD: &str = "S256";
const DEVICE_TOKEN_POLL_PATH: &str = "/api/v1/deviceToken/poll";
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
    machine_token: Option<String>,
    machine_type: Option<String>,
    verification_uri: String,
    expires_at: i64,
    cancelled: bool,
}

#[derive(Debug, Clone)]
struct QoderMachineInfo {
    token: String,
    machine_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QoderMachineTokenCache {
    #[serde(default)]
    token: Option<String>,
    #[serde(default, rename = "type")]
    machine_type: Option<String>,
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
        query_pairs.append_pair("client_id", QODER_CLI_BROWSER_LOGIN_CLIENT_ID);
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
        if name.eq_ignore_ascii_case("Qoder.app") {
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

fn build_qoder_status_headers(
    token: &str,
    machine_token: Option<&str>,
    machine_type: Option<&str>,
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
    let cosy_version = detect_qoder_product_version();
    if let Some(version) = cosy_version.as_deref() {
        if let Ok(value) = HeaderValue::from_str(&version) {
            headers.insert("Cosy-Version", value);
        }
    }
    if let Some(machine_token) = machine_token.and_then(|value| normalize_non_empty(Some(value))) {
        if let Ok(value) = HeaderValue::from_str(&machine_token) {
            headers.insert("Cosy-MachineToken", value);
        }
    }
    if let Some(machine_type) = machine_type.and_then(|value| normalize_non_empty(Some(value))) {
        if let Ok(value) = HeaderValue::from_str(&machine_type) {
            headers.insert("Cosy-MachineType", value);
        }
    }
    if let Ok(value) = HeaderValue::from_str(&build_cosy_machine_os()) {
        headers.insert("Cosy-MachineOS", value);
    }
    if let Ok(value) = HeaderValue::from_str("0") {
        headers.insert("Cosy-ClientType", value);
    }
    logger::log_info(&format!(
        "[Qoder OAuth] 构造状态请求头: has_cosy_version={}, has_machine_token={}, has_machine_type={}, machine_os={}",
        cosy_version.is_some(),
        machine_token.is_some(),
        machine_type.is_some(),
        build_cosy_machine_os()
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
) -> Result<Value, String> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

    let mut headers = HeaderMap::new();
    let bearer = format!("Bearer {}", token);
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&bearer)
            .map_err(|err| format!("构造 Qoder userinfo 授权头失败: {}", err))?,
    );
    fetch_openapi_json(client, openapi_base_url, USER_INFO_PATH, headers, &[]).await
}

async fn fetch_qoder_user_status_bundle(
    client: &reqwest::Client,
    openapi_base_url: &str,
    token: &str,
    machine_token: Option<&str>,
    machine_type: Option<&str>,
) -> Result<(Value, Option<Value>), String> {
    let status_headers = build_qoder_status_headers(token, machine_token, machine_type);
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
) -> Result<Value, String> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

    let mut headers = HeaderMap::new();
    let bearer = format!("Bearer {}", token);
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&bearer)
            .map_err(|err| format!("构造 Qoder user plan 授权头失败: {}", err))?,
    );
    fetch_openapi_json(client, openapi_base_url, USER_PLAN_PATH, headers, &[]).await
}

async fn fetch_qoder_credit_usage(
    client: &reqwest::Client,
    openapi_base_url: &str,
    token: &str,
) -> Result<Value, String> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

    let mut headers = HeaderMap::new();
    let bearer = format!("Bearer {}", token);
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&bearer)
            .map_err(|err| format!("构造 Qoder credit usage 授权头失败: {}", err))?,
    );
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

async fn fetch_qoder_user_status_bundle_with_fallback(
    client: &reqwest::Client,
    openapi_base_url: &str,
    token: &str,
    machine_info: Option<&QoderMachineInfo>,
) -> Result<(Value, Option<Value>), String> {
    let first_attempt = fetch_qoder_user_status_bundle(
        client,
        openapi_base_url,
        token,
        machine_info.map(|item| item.token.as_str()),
        machine_info.and_then(|item| item.machine_type.as_deref()),
    )
    .await;

    if first_attempt.is_ok() {
        return first_attempt;
    }

    let first_error = first_attempt
        .err()
        .unwrap_or_else(|| "未知错误".to_string());

    if machine_info.is_some() {
        logger::log_warn(&format!(
            "[Qoder Refresh] user/status 首次请求失败，尝试无机器标识重试: {}",
            first_error
        ));
        if let Ok(value) =
            fetch_qoder_user_status_bundle(client, openapi_base_url, token, None, None).await
        {
            return Ok(value);
        }
    }

    Err(first_error)
}

async fn refresh_account_from_openapi_once(account_id: &str) -> Result<QoderAccount, String> {
    let target = qoder_account::load_account(account_id)
        .ok_or_else(|| format!("Qoder 账号不存在: {}", account_id))?;

    let access_token = extract_access_token_from_account(&target)
        .ok_or_else(|| "Qoder 账号缺少 access token，请重新登录后再刷新".to_string())?;

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

    let (user_status, data_policy) = fetch_qoder_user_status_bundle_with_fallback(
        &client,
        DEFAULT_OPENAPI_BASE_URL,
        &access_token,
        machine_info.as_ref(),
    )
    .await?;

    ensure_refresh_identity_consistent(&target, &user_status)?;

    let user_info_raw =
        build_refresh_user_info_raw(&target, &access_token, &user_status, data_policy.as_ref());

    let user_plan_raw =
        match fetch_qoder_user_plan(&client, DEFAULT_OPENAPI_BASE_URL, &access_token).await {
            Ok(value) => Some(value),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Qoder Refresh] 获取 /api/v2/user/plan 失败，将沿用本地缓存: {}",
                    err
                ));
                target.auth_user_plan_raw.clone()
            }
        };

    let mut quota_query_error: Option<String> = None;
    let credit_usage_raw =
        match fetch_qoder_credit_usage(&client, DEFAULT_OPENAPI_BASE_URL, &access_token).await {
            Ok(value) => Some(value),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Qoder Refresh] 获取 /api/v2/quota/usage 失败，将沿用本地缓存: {}",
                    err
                ));
                quota_query_error = Some(err.clone());
                target.auth_credit_usage_raw.clone()
            }
        };

    let refreshed = qoder_account::upsert_account_from_snapshot(
        user_info_raw,
        user_plan_raw,
        credit_usage_raw,
    )?;
    let refreshed = if refreshed.id == target.id {
        qoder_account::update_quota_query_error(&refreshed.id, quota_query_error)?
            .unwrap_or(refreshed)
    } else {
        refreshed
    };
    if refreshed.id != target.id {
        return Err(format!(
            "刷新结果账号不一致: target_id={}, actual_id={}",
            target.id, refreshed.id
        ));
    }
    Ok(refreshed)
}

pub async fn refresh_account_from_openapi(account_id: &str) -> Result<QoderAccount, String> {
    let result = crate::modules::refresh_retry::retry_once_with_delay(
        "Qoder Refresh",
        account_id,
        || async { refresh_account_from_openapi_once(account_id).await },
    )
    .await;
    if let Err(err) = &result {
        let _ = qoder_account::update_quota_query_error(account_id, Some(err.clone()));
    }
    result
}

pub async fn refresh_all_accounts_from_openapi() -> Result<i32, String> {
    let accounts = qoder_account::list_accounts();
    if accounts.is_empty() {
        return Ok(0);
    }

    let mut success_count: i32 = 0;
    for account in accounts {
        match refresh_account_from_openapi(&account.id).await {
            Ok(_) => {
                success_count += 1;
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[Qoder Refresh] 批量刷新失败: account_id={}, email={}, error={}",
                    account.id, account.email, err
                ));
            }
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

    logger::log_info(&format!(
        "[Qoder OAuth] 官方 machine token 缓存已加载: path={}, has_token={}, has_machine_type={}",
        cache_path.to_string_lossy(),
        token.is_some(),
        machine_type.is_some()
    ));

    Ok(token.map(|token| QoderMachineInfo {
        token,
        machine_type,
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
        machine_token: machine_info.as_ref().map(|value| value.token.clone()),
        machine_type: machine_info.and_then(|value| value.machine_type),
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
        "[Qoder OAuth] 登录会话已创建: login_id={}, client_id={}, expires_in={}s",
        login_id, QODER_CLI_BROWSER_LOGIN_CLIENT_ID, OAUTH_TIMEOUT_SECONDS
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
                state.machine_token.clone(),
                state.machine_type.clone(),
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

                let user_info_response =
                    match fetch_qoder_user_info(&client, &snapshot.3, &access_token).await {
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
                    snapshot.4.as_deref(),
                    snapshot.5.as_deref(),
                )
                .await?;

                let mut user_info_raw =
                    build_initial_user_info_raw(&token_data, user_info_response.as_ref());
                merge_user_status_into_user_info(
                    &mut user_info_raw,
                    &user_status,
                    data_policy.as_ref(),
                );

                let user_plan_raw =
                    match fetch_qoder_user_plan(&client, &snapshot.3, &access_token).await {
                        Ok(value) => Some(value),
                        Err(err) => {
                            logger::log_warn(&format!(
                                "[Qoder OAuth] 获取 /api/v2/user/plan 失败，将以缺省快照继续: {}",
                                err
                            ));
                            None
                        }
                    };

                let credit_usage_raw =
                    match fetch_qoder_credit_usage(&client, &snapshot.3, &access_token).await {
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
    fn builds_cli_device_login_url_without_redirect_uri() {
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
            "client_id".to_string(),
            QODER_CLI_BROWSER_LOGIN_CLIENT_ID.to_string()
        )));
        assert!(query.contains(&("machine_id".to_string(), "test-machine-id".to_string())));
        assert!(!query.iter().any(|(key, _)| key == "redirect_uri"));
    }
}
