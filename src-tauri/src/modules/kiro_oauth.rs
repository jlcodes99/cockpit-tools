use base64::Engine;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use url::Url;

use crate::models::kiro::{KiroAccount, KiroOAuthCompletePayload, KiroOAuthStartResponse};
use crate::modules::{kiro_account, logger};

const KIRO_AUTH_PORTAL_URL: &str = "https://app.kiro.dev/signin";
const KIRO_TOKEN_ENDPOINT: &str = "https://prod.us-east-1.auth.desktop.kiro.dev/oauth/token";
const KIRO_REFRESH_ENDPOINT: &str = "https://prod.us-east-1.auth.desktop.kiro.dev/refreshToken";
const KIRO_AWS_OIDC_TOKEN_ENDPOINT_FMT: &str = "https://oidc.{region}.amazonaws.com/token";
const KIRO_RUNTIME_DEFAULT_ENDPOINT: &str = "https://q.us-east-1.amazonaws.com";
const KIRO_ACCOUNT_STATUS_NORMAL: &str = "normal";
const KIRO_ACCOUNT_STATUS_BANNED: &str = "banned";
const KIRO_ACCOUNT_STATUS_ERROR: &str = "error";
const OAUTH_TIMEOUT_SECONDS: u64 = 600;
const OAUTH_POLL_INTERVAL_MS: u64 = 250;
const OAUTH_STATE_FILE: &str = "kiro_oauth_pending.json";
const CALLBACK_PORT_CANDIDATES: [u16; 10] = [
    3128, 4649, 6588, 8008, 9091, 49153, 50153, 51153, 52153, 53153,
];

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OAuthCallbackData {
    login_option: String,
    code: Option<String>,
    issuer_url: Option<String>,
    idc_region: Option<String>,
    path: String,
    client_id: Option<String>,
    scopes: Option<String>,
    login_hint: Option<String>,
    audience: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PendingOAuthState {
    login_id: String,
    expires_at: i64,
    verification_uri: String,
    verification_uri_complete: String,
    callback_url: String,
    callback_port: u16,
    state_token: String,
    code_verifier: String,
    callback_result: Option<Result<OAuthCallbackData, String>>,
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
                "[Kiro OAuth] 读取持久化登录状态失败，已忽略: {}",
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
        logger::log_warn(&format!("[Kiro OAuth] 持久化登录状态失败，已忽略: {}", err));
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
        set_pending_login(None);
        return;
    }
    if state.callback_result.is_some() {
        return;
    }

    match TcpListener::bind(("127.0.0.1", state.callback_port)) {
        Ok(listener) => {
            drop(listener);
            let expected_login_id = state.login_id.clone();
            let expected_state = state.state_token.clone();
            let callback_port = state.callback_port;
            tokio::spawn(async move {
                if let Err(err) = start_callback_server(
                    callback_port,
                    expected_login_id.clone(),
                    expected_state.clone(),
                )
                .await
                {
                    logger::log_error(&format!(
                        "[Kiro OAuth] 回调服务恢复失败: login_id={}, error={}",
                        expected_login_id, err
                    ));
                    set_callback_result_for_login(
                        &expected_login_id,
                        &expected_state,
                        Err(format!("本地回调服务异常: {}", err)),
                    );
                }
            });
            logger::log_info(&format!(
                "[Kiro OAuth] 已恢复本地回调服务: login_id={}, port={}",
                state.login_id, state.callback_port
            ));
        }
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            logger::log_info(&format!(
                "[Kiro OAuth] 本地回调端口已占用，视为监听中: login_id={}, port={}",
                state.login_id, state.callback_port
            ));
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Kiro OAuth] 本地回调恢复失败: login_id={}, port={}, error={}",
                state.login_id, state.callback_port, err
            ));
        }
    }
}

fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..24).map(|_| rng.gen::<u8>()).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_challenge(code_verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let digest = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
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

fn normalize_email(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() || !trimmed.contains('@') {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn set_payload_status(
    payload: &mut KiroOAuthCompletePayload,
    status: &str,
    reason: Option<String>,
) {
    payload.status = Some(status.to_string());
    payload.status_reason = reason.and_then(|raw| normalize_non_empty(Some(raw.as_str())));
}

fn parse_runtime_error_reason(body: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(body).ok()?;
    let direct_reason = pick_string(
        Some(&parsed),
        &[
            &["reason"],
            &["message"],
            &["errorMessage"],
            &["error", "message"],
            &["error", "reason"],
            &["detail"],
            &["details"],
        ],
    );
    if let Some(reason) = direct_reason.and_then(|raw| normalize_non_empty(Some(raw.as_str()))) {
        return Some(reason);
    }

    if let Some(code) = pick_string(
        Some(&parsed),
        &[&["error"], &["code"], &["errorCode"], &["error", "code"]],
    )
    .and_then(|raw| normalize_non_empty(Some(raw.as_str())))
    {
        return Some(code);
    }

    None
}

fn parse_banned_reason_from_error(err: &str) -> Option<String> {
    err.strip_prefix("BANNED:")
        .and_then(|raw| normalize_non_empty(Some(raw)))
}

fn get_path_value<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for key in path {
        current = current.as_object()?.get(*key)?;
    }
    Some(current)
}

fn pick_string(root: Option<&Value>, paths: &[&[&str]]) -> Option<String> {
    let root = root?;
    for path in paths {
        if let Some(value) = get_path_value(root, path) {
            if let Some(text) = value.as_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
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

fn pick_number(root: Option<&Value>, paths: &[&[&str]]) -> Option<f64> {
    let root = root?;
    for path in paths {
        if let Some(value) = get_path_value(root, path) {
            if let Some(num) = value.as_f64() {
                if num.is_finite() {
                    return Some(num);
                }
            }
            if let Some(text) = value.as_str() {
                if let Ok(num) = text.trim().parse::<f64>() {
                    if num.is_finite() {
                        return Some(num);
                    }
                }
            }
        }
    }
    None
}

fn parse_timestamp(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if let Some(seconds) = value.as_i64() {
        return normalize_timestamp(seconds);
    }
    if let Some(seconds) = value.as_u64() {
        return normalize_timestamp(seconds as i64);
    }
    if let Some(seconds) = value.as_f64() {
        if seconds.is_finite() {
            return normalize_timestamp(seconds.round() as i64);
        }
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(num) = trimmed.parse::<i64>() {
            return normalize_timestamp(num);
        }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
            return Some(dt.timestamp());
        }
        if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
            return Some(parsed.and_utc().timestamp());
        }
        if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y/%m/%d %H:%M:%S") {
            return Some(parsed.and_utc().timestamp());
        }
    }
    None
}

fn normalize_timestamp(raw: i64) -> Option<i64> {
    if raw <= 0 {
        return None;
    }
    if raw > 10_000_000_000 {
        return Some(raw / 1000);
    }
    Some(raw)
}

fn resolve_usage_root<'a>(usage: Option<&'a Value>) -> Option<&'a Value> {
    let usage = usage?;
    if let Some(state) = get_path_value(usage, &["kiro.resourceNotifications.usageState"]) {
        return Some(state);
    }
    if let Some(state) = get_path_value(usage, &["usageState"]) {
        return Some(state);
    }
    Some(usage)
}

fn pick_usage_breakdown<'a>(usage: Option<&'a Value>) -> Option<&'a Value> {
    let usage = usage?;
    let list = get_path_value(usage, &["usageBreakdownList"])
        .and_then(|value| value.as_array())
        .or_else(|| {
            get_path_value(usage, &["usageBreakdowns"]).and_then(|value| value.as_array())
        })?;
    if list.is_empty() {
        return None;
    }

    list.iter()
        .find(|item| {
            item.as_object()
                .and_then(|obj| obj.get("type"))
                .and_then(|value| value.as_str())
                .map(|value| value.eq_ignore_ascii_case("credit"))
                .unwrap_or(false)
        })
        .or_else(|| list.first())
}

fn days_until(timestamp: Option<i64>) -> Option<i64> {
    let ts = timestamp?;
    let now = now_timestamp();
    if ts <= now {
        return Some(0);
    }
    Some(((ts - now) as f64 / 86_400.0).ceil() as i64)
}

fn parse_profile_arn_region(profile_arn: &str) -> Option<String> {
    let mut segments = profile_arn.split(':');
    let prefix = segments.next()?.trim();
    if !prefix.eq_ignore_ascii_case("arn") {
        return None;
    }
    let _partition = segments.next()?;
    let _service = segments.next()?;
    let region = segments.next()?.trim();
    if region.is_empty() {
        None
    } else {
        Some(region.to_string())
    }
}

fn runtime_endpoint_for_region(region: Option<&str>) -> String {
    let region = region.unwrap_or("us-east-1").trim().to_ascii_lowercase();
    match region.as_str() {
        "us-east-1" => "https://q.us-east-1.amazonaws.com".to_string(),
        "eu-central-1" => "https://q.eu-central-1.amazonaws.com".to_string(),
        "us-gov-east-1" => "https://q-fips.us-gov-east-1.amazonaws.com".to_string(),
        "us-gov-west-1" => "https://q-fips.us-gov-west-1.amazonaws.com".to_string(),
        "us-iso-east-1" => "https://q.us-iso-east-1.c2s.ic.gov".to_string(),
        "us-isob-east-1" => "https://q.us-isob-east-1.sc2s.sgov.gov".to_string(),
        "us-isof-south-1" => "https://q.us-isof-south-1.csp.hci.ic.gov".to_string(),
        "us-isof-east-1" => "https://q.us-isof-east-1.csp.hci.ic.gov".to_string(),
        _ => KIRO_RUNTIME_DEFAULT_ENDPOINT.to_string(),
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

fn parse_callback_url(raw_callback_url: &str, callback_port: u16) -> Result<Url, String> {
    let trimmed = raw_callback_url.trim();
    if trimmed.is_empty() {
        return Err("回调链接不能为空".to_string());
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Url::parse(trimmed).map_err(|e| format!("回调链接格式无效: {}", e));
    }

    if trimmed.starts_with('/') {
        return Url::parse(format!("http://localhost:{}{}", callback_port, trimmed).as_str())
            .map_err(|e| format!("回调链接格式无效: {}", e));
    }

    Url::parse(
        format!(
            "http://localhost:{}/oauth/callback?{}",
            callback_port,
            trimmed.trim_start_matches('?')
        )
        .as_str(),
    )
    .map_err(|e| format!("回调链接格式无效: {}", e))
}

fn auth_success_redirect_url() -> String {
    format!(
        "{}?auth_status=success&redirect_from=KiroIDE",
        KIRO_AUTH_PORTAL_URL
    )
}

fn auth_error_redirect_url(message: &str) -> String {
    format!(
        "{}?auth_status=error&redirect_from=KiroIDE&error_message={}",
        KIRO_AUTH_PORTAL_URL,
        urlencoding::encode(message)
    )
}

fn is_mwinit_tool_available() -> bool {
    #[cfg(target_os = "windows")]
    let checker = "where.exe";
    #[cfg(not(target_os = "windows"))]
    let checker = "which";

    let mut cmd = std::process::Command::new(checker);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    cmd.arg("mwinit")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn build_portal_auth_url(
    state_token: &str,
    code_challenge: &str,
    redirect_uri: &str,
    from_amazon_internal: bool,
) -> String {
    let mut url = format!(
        "{}?state={}&code_challenge={}&code_challenge_method=S256&redirect_uri={}&redirect_from=KiroIDE",
        KIRO_AUTH_PORTAL_URL,
        urlencoding::encode(state_token),
        urlencoding::encode(code_challenge),
        urlencoding::encode(redirect_uri),
    );
    if from_amazon_internal {
        url.push_str("&from_amazon_internal=true");
    }
    url
}

fn find_available_callback_port() -> Result<u16, String> {
    for port in CALLBACK_PORT_CANDIDATES {
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            drop(listener);
            return Ok(port);
        }
    }
    Err("本地回调端口已被占用，请关闭占用进程后重试".to_string())
}

fn set_callback_result_for_login(
    expected_login_id: &str,
    expected_state: &str,
    result: Result<OAuthCallbackData, String>,
) {
    if let Ok(mut guard) = PENDING_OAUTH_STATE.lock() {
        if let Some(state) = guard.as_mut() {
            if state.login_id == expected_login_id && state.state_token == expected_state {
                state.callback_result = Some(result);
                persist_pending_login(Some(state));
            }
        }
    }
}

fn extract_profile_arn_from_account(account: &KiroAccount) -> Option<String> {
    extract_profile_arn(
        account.kiro_auth_token_raw.as_ref(),
        account.kiro_profile_raw.as_ref(),
    )
}

fn extract_profile_arn_from_payload(payload: &KiroOAuthCompletePayload) -> Option<String> {
    extract_profile_arn(
        payload.kiro_auth_token_raw.as_ref(),
        payload.kiro_profile_raw.as_ref(),
    )
}

fn provider_from_login_option(login_option: &str) -> Option<String> {
    match login_option.trim().to_ascii_lowercase().as_str() {
        "google" => Some("Google".to_string()),
        "github" => Some("Github".to_string()),
        _ => None,
    }
}

fn ensure_expires_at_from_expires_in(token: &mut Value) {
    let Some(obj) = token.as_object_mut() else {
        return;
    };
    if obj.contains_key("expiresAt") || obj.contains_key("expires_at") {
        return;
    }

    let expires_in_seconds = obj
        .get("expiresIn")
        .or_else(|| obj.get("expires_in"))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().map(|n| n as i64))
                .or_else(|| {
                    value
                        .as_str()
                        .and_then(|raw| raw.trim().parse::<i64>().ok())
                })
        })
        .unwrap_or(0);
    if expires_in_seconds <= 0 {
        return;
    }

    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in_seconds);
    obj.insert(
        "expiresAt".to_string(),
        Value::String(expires_at.to_rfc3339()),
    );
}

fn normalize_ascii_lower(value: Option<&str>) -> Option<String> {
    normalize_non_empty(value).map(|raw| raw.to_ascii_lowercase())
}

fn resolve_idc_region(auth_token: &Value, account: &KiroAccount) -> Option<String> {
    pick_string(
        Some(auth_token),
        &[&["idc_region"], &["idcRegion"], &["region"]],
    )
    .and_then(|value| normalize_non_empty(Some(value.as_str())))
    .or_else(|| {
        account
            .idc_region
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
    })
    .or_else(|| {
        extract_profile_arn(Some(auth_token), account.kiro_profile_raw.as_ref())
            .and_then(|profile_arn| parse_profile_arn_region(profile_arn.as_str()))
    })
}

fn resolve_idc_client_id(auth_token: &Value, account: &KiroAccount) -> Option<String> {
    pick_string(
        Some(auth_token),
        &[
            &["client_id"],
            &["clientId"],
            &["clientRegistration", "clientId"],
            &["registration", "clientId"],
            &["oidcClient", "clientId"],
        ],
    )
    .and_then(|value| normalize_non_empty(Some(value.as_str())))
    .or_else(|| {
        account
            .client_id
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
    })
}

fn resolve_idc_client_secret(auth_token: &Value) -> Option<String> {
    pick_string(
        Some(auth_token),
        &[
            &["client_secret"],
            &["clientSecret"],
            &["clientRegistration", "clientSecret"],
            &["clientRegistration", "client_secret"],
            &["registration", "clientSecret"],
            &["oidcClient", "clientSecret"],
        ],
    )
    .and_then(|value| normalize_non_empty(Some(value.as_str())))
}

fn should_prefer_idc_refresh(auth_token: &Value, account: &KiroAccount) -> bool {
    let auth_method_is_idc = normalize_ascii_lower(
        pick_string(Some(auth_token), &[&["authMethod"], &["auth_method"]]).as_deref(),
    )
    .map(|value| value == "idc")
    .unwrap_or(false);

    let provider_is_idc = normalize_ascii_lower(
        pick_string(
            Some(auth_token),
            &[&["provider"], &["loginProvider"], &["login_option"]],
        )
        .as_deref(),
    )
    .map(|value| {
        matches!(
            value.as_str(),
            "enterprise" | "builderid" | "internal" | "awsidc" | "external_idp"
        )
    })
    .unwrap_or(false);

    let login_provider_is_idc = normalize_ascii_lower(account.login_provider.as_deref())
        .map(|value| {
            matches!(
                value.as_str(),
                "enterprise" | "builderid" | "internal" | "awsidc"
            )
        })
        .unwrap_or(false);

    let has_idc_material = resolve_idc_region(auth_token, account).is_some()
        && resolve_idc_client_id(auth_token, account).is_some()
        && resolve_idc_client_secret(auth_token).is_some();

    auth_method_is_idc || provider_is_idc || login_provider_is_idc || has_idc_material
}

fn build_token_exchange_redirect_uri(
    base_callback_url: &str,
    callback: &OAuthCallbackData,
) -> String {
    let callback_path = if callback.path.starts_with('/') {
        callback.path.clone()
    } else {
        format!("/{}", callback.path)
    };
    format!(
        "{}{}?login_option={}",
        base_callback_url.trim_end_matches('/'),
        callback_path,
        urlencoding::encode(callback.login_option.as_str()),
    )
}

fn inject_callback_context_into_token(token: &mut Value, callback: &OAuthCallbackData) {
    if !token.is_object() {
        *token = json!({});
    }
    let Some(obj) = token.as_object_mut() else {
        return;
    };

    if !callback.login_option.trim().is_empty() {
        obj.entry("login_option".to_string())
            .or_insert_with(|| Value::String(callback.login_option.clone()));
    }

    if let Some(provider) = provider_from_login_option(&callback.login_option) {
        obj.entry("provider".to_string())
            .or_insert_with(|| Value::String(provider.clone()));
        obj.entry("loginProvider".to_string())
            .or_insert_with(|| Value::String(provider.clone()));
        obj.entry("authMethod".to_string())
            .or_insert_with(|| Value::String("social".to_string()));
    }

    if let Some(value) = callback.issuer_url.as_ref() {
        obj.entry("issuer_url".to_string())
            .or_insert_with(|| Value::String(value.clone()));
    }
    if let Some(value) = callback.idc_region.as_ref() {
        obj.entry("idc_region".to_string())
            .or_insert_with(|| Value::String(value.clone()));
    }
    if let Some(value) = callback.client_id.as_ref() {
        obj.entry("client_id".to_string())
            .or_insert_with(|| Value::String(value.clone()));
    }
    if let Some(value) = callback.scopes.as_ref() {
        obj.entry("scopes".to_string())
            .or_insert_with(|| Value::String(value.clone()));
    }
    if let Some(value) = callback.login_hint.as_ref() {
        obj.entry("login_hint".to_string())
            .or_insert_with(|| Value::String(value.clone()));
    }
    if let Some(value) = callback.audience.as_ref() {
        obj.entry("audience".to_string())
            .or_insert_with(|| Value::String(value.clone()));
    }

    // Auth service returns expiresIn; convert to expiresAt to match Kiro local cache shape.
    ensure_expires_at_from_expires_in(token);
}

fn unwrap_token_response(mut response: Value) -> Value {
    if let Some(data) = response
        .as_object_mut()
        .and_then(|obj| obj.remove("data"))
        .filter(|value| value.is_object())
    {
        data
    } else {
        response
    }
}

async fn exchange_code_for_token(
    callback: &OAuthCallbackData,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<Value, String> {
    let code = callback
        .code
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .ok_or_else(|| "Kiro 回调缺少 code，无法完成登录".to_string())?;

    let response = reqwest::Client::new()
        .post(KIRO_TOKEN_ENDPOINT)
        .header("Content-Type", "application/json")
        .json(&json!({
            "code": code,
            "code_verifier": code_verifier,
            "redirect_uri": redirect_uri
        }))
        .send()
        .await
        .map_err(|e| format!("请求 Kiro oauth/token 接口失败: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<no-body>".to_string());
    if !status.is_success() {
        return Err(format!(
            "Kiro oauth/token 接口返回异常: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    let mut token = unwrap_token_response(serde_json::from_str::<Value>(&body).map_err(|e| {
        format!(
            "解析 Kiro oauth/token 响应失败: {} (body_len={})",
            e,
            body.len()
        )
    })?);
    inject_callback_context_into_token(&mut token, callback);
    Ok(token)
}

async fn start_callback_server(
    callback_port: u16,
    expected_login_id: String,
    expected_state: String,
) -> Result<(), String> {
    use tiny_http::{Header, Response, Server};

    let server = Server::http(format!("127.0.0.1:{}", callback_port))
        .map_err(|e| format!("启动 Kiro OAuth 回调服务失败: {}", e))?;
    let started = std::time::Instant::now();

    logger::log_info(&format!(
        "[Kiro OAuth] 本地回调服务启动: login_id={}, port={}",
        expected_login_id, callback_port
    ));

    loop {
        let should_stop = {
            let guard = PENDING_OAUTH_STATE
                .lock()
                .map_err(|_| "OAuth 状态锁不可用".to_string())?;
            match guard.as_ref() {
                Some(state) => {
                    state.login_id != expected_login_id || state.state_token != expected_state
                }
                None => true,
            }
        };
        if should_stop {
            break;
        }

        if started.elapsed().as_secs() > OAUTH_TIMEOUT_SECONDS {
            set_callback_result_for_login(
                &expected_login_id,
                &expected_state,
                Err("等待 Kiro 登录超时，请重新发起授权".to_string()),
            );
            break;
        }

        if let Ok(Some(request)) = server.try_recv() {
            let raw_url = request.url().to_string();
            let (path, query) = match raw_url.split_once('?') {
                Some((path, query)) => (path, query),
                None => (raw_url.as_str(), ""),
            };

            if path == "/cancel" {
                set_callback_result_for_login(
                    &expected_login_id,
                    &expected_state,
                    Err("登录已取消".to_string()),
                );
                let _ = request.respond(Response::from_string("cancelled").with_status_code(200));
                break;
            }

            if path != "/oauth/callback" && path != "/signin/callback" {
                let _ = request.respond(Response::from_string("Not Found").with_status_code(404));
                continue;
            }

            let params = parse_query_params(query);
            let error_code = params.get("error").cloned();
            let error_description = params
                .get("error_description")
                .cloned()
                .unwrap_or_else(String::new);
            if let Some(error_code) = error_code {
                let message = if error_description.trim().is_empty() {
                    format!("授权失败: {}", error_code)
                } else {
                    format!("授权失败: {} ({})", error_code, error_description)
                };
                set_callback_result_for_login(
                    &expected_login_id,
                    &expected_state,
                    Err(message.clone()),
                );
                let redirect = auth_error_redirect_url(&message);
                let response = Header::from_bytes(&b"Location"[..], redirect.as_bytes())
                    .ok()
                    .map(|header| Response::empty(302).with_header(header))
                    .unwrap_or_else(|| Response::empty(400));
                let _ = request.respond(response);
                break;
            }

            let callback_state = params.get("state").cloned().unwrap_or_default();
            if callback_state.is_empty() || callback_state != expected_state {
                let message = "授权状态校验失败，请重新发起登录".to_string();
                set_callback_result_for_login(
                    &expected_login_id,
                    &expected_state,
                    Err(message.clone()),
                );
                let redirect = auth_error_redirect_url(&message);
                let response = Header::from_bytes(&b"Location"[..], redirect.as_bytes())
                    .ok()
                    .map(|header| Response::empty(302).with_header(header))
                    .unwrap_or_else(|| Response::empty(400));
                let _ = request.respond(response);
                break;
            }

            let login_option = params
                .get("login_option")
                .or_else(|| params.get("loginOption"))
                .and_then(|value| normalize_non_empty(Some(value.as_str())))
                .unwrap_or_default()
                .to_ascii_lowercase();

            let callback = OAuthCallbackData {
                login_option,
                code: params
                    .get("code")
                    .and_then(|value| normalize_non_empty(Some(value.as_str()))),
                issuer_url: params
                    .get("issuer_url")
                    .or_else(|| params.get("issuerUrl"))
                    .and_then(|value| normalize_non_empty(Some(value.as_str()))),
                idc_region: params
                    .get("idc_region")
                    .or_else(|| params.get("idcRegion"))
                    .and_then(|value| normalize_non_empty(Some(value.as_str()))),
                path: path.to_string(),
                client_id: params
                    .get("client_id")
                    .or_else(|| params.get("clientId"))
                    .and_then(|value| normalize_non_empty(Some(value.as_str()))),
                scopes: params
                    .get("scopes")
                    .or_else(|| params.get("scope"))
                    .and_then(|value| normalize_non_empty(Some(value.as_str()))),
                login_hint: params
                    .get("login_hint")
                    .or_else(|| params.get("loginHint"))
                    .and_then(|value| normalize_non_empty(Some(value.as_str()))),
                audience: params
                    .get("audience")
                    .and_then(|value| normalize_non_empty(Some(value.as_str()))),
            };

            logger::log_info(&format!(
                "[Kiro OAuth] 收到回调: login_id={}, path={}, login_option={}, has_code={}",
                expected_login_id,
                callback.path,
                callback.login_option,
                callback
                    .code
                    .as_ref()
                    .map(|v| !v.is_empty())
                    .unwrap_or(false)
            ));

            set_callback_result_for_login(&expected_login_id, &expected_state, Ok(callback));
            let redirect = auth_success_redirect_url();
            let response = Header::from_bytes(&b"Location"[..], redirect.as_bytes())
                .ok()
                .map(|header| Response::empty(302).with_header(header))
                .unwrap_or_else(|| Response::empty(200));
            let _ = request.respond(response);
            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
    }

    Ok(())
}

fn decode_jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .ok()?;
    serde_json::from_slice::<Value>(&decoded).ok()
}

fn extract_usage_payload(
    usage: Option<&Value>,
) -> (
    Option<String>,
    Option<String>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<i64>,
    Option<i64>,
) {
    let usage = resolve_usage_root(usage);

    let plan_name = pick_string(
        usage,
        &[
            &["planName"],
            &["currentPlanName"],
            &["subscriptionInfo", "subscriptionName"],
            &["subscriptionInfo", "subscriptionTitle"],
            &["usageBreakdowns", "planName"],
            &["freeTrialUsage", "planName"],
            &["plan", "name"],
        ],
    );

    let plan_tier = pick_string(
        usage,
        &[
            &["planTier"],
            &["tier"],
            &["subscriptionInfo", "type"],
            &["usageBreakdowns", "tier"],
            &["plan", "tier"],
        ],
    );

    let mut credits_total = pick_number(
        usage,
        &[
            &["estimatedUsage", "total"],
            &["estimatedUsage", "creditsTotal"],
            &["usageBreakdowns", "plan", "totalCredits"],
            &["usageBreakdowns", "covered", "total"],
            &["usageBreakdownList", "0", "usageLimitWithPrecision"],
            &["usageBreakdownList", "0", "usageLimit"],
            &["credits", "total"],
            &["totalCredits"],
        ],
    );

    let mut credits_used = pick_number(
        usage,
        &[
            &["estimatedUsage", "used"],
            &["estimatedUsage", "creditsUsed"],
            &["usageBreakdowns", "plan", "usedCredits"],
            &["usageBreakdowns", "covered", "used"],
            &["usageBreakdownList", "0", "currentUsageWithPrecision"],
            &["usageBreakdownList", "0", "currentUsage"],
            &["credits", "used"],
            &["usedCredits"],
        ],
    );

    let mut bonus_total = pick_number(
        usage,
        &[
            &["bonusCredits", "total"],
            &["bonus", "total"],
            &["usageBreakdowns", "bonus", "total"],
            &[
                "usageBreakdownList",
                "0",
                "freeTrialInfo",
                "usageLimitWithPrecision",
            ],
            &["usageBreakdownList", "0", "freeTrialInfo", "usageLimit"],
        ],
    );

    let mut bonus_used = pick_number(
        usage,
        &[
            &["bonusCredits", "used"],
            &["bonus", "used"],
            &["usageBreakdowns", "bonus", "used"],
            &[
                "usageBreakdownList",
                "0",
                "freeTrialInfo",
                "currentUsageWithPrecision",
            ],
            &["usageBreakdownList", "0", "freeTrialInfo", "currentUsage"],
        ],
    );

    let mut usage_reset_at = parse_timestamp(
        usage
            .and_then(|value| get_path_value(value, &["resetAt"]))
            .or_else(|| usage.and_then(|value| get_path_value(value, &["resetTime"])))
            .or_else(|| usage.and_then(|value| get_path_value(value, &["resetOn"])))
            .or_else(|| usage.and_then(|value| get_path_value(value, &["nextDateReset"])))
            .or_else(|| {
                usage.and_then(|value| get_path_value(value, &["usageBreakdowns", "resetAt"]))
            }),
    );

    let mut bonus_expire_days = pick_number(
        usage,
        &[
            &["bonusCredits", "expiryDays"],
            &["bonusCredits", "expireDays"],
            &["bonus", "expiryDays"],
            &["usageBreakdownList", "0", "freeTrialInfo", "daysRemaining"],
        ],
    )
    .map(|value| value.round() as i64);

    let breakdown = pick_usage_breakdown(usage);
    let free_trial = breakdown.and_then(|value| {
        get_path_value(value, &["freeTrialUsage"])
            .or_else(|| get_path_value(value, &["freeTrialInfo"]))
    });

    let plan_name = plan_name.or_else(|| {
        pick_string(
            breakdown,
            &[
                &["displayName"],
                &["displayNamePlural"],
                &["type"],
                &["unit"],
            ],
        )
    });

    let plan_tier =
        plan_tier.or_else(|| pick_string(breakdown, &[&["currency"], &["type"], &["unit"]]));

    if credits_total.is_none() {
        credits_total = pick_number(
            breakdown,
            &[
                &["usageLimitWithPrecision"],
                &["usageLimit"],
                &["limit"],
                &["total"],
                &["totalCredits"],
            ],
        );
    }
    if credits_used.is_none() {
        credits_used = pick_number(
            breakdown,
            &[
                &["currentUsageWithPrecision"],
                &["currentUsage"],
                &["used"],
                &["usedCredits"],
            ],
        );
    }

    if bonus_total.is_none() {
        bonus_total = pick_number(
            free_trial,
            &[
                &["usageLimitWithPrecision"],
                &["usageLimit"],
                &["limit"],
                &["total"],
                &["totalCredits"],
            ],
        );
    }
    if bonus_used.is_none() {
        bonus_used = pick_number(
            free_trial,
            &[
                &["currentUsageWithPrecision"],
                &["currentUsage"],
                &["used"],
                &["usedCredits"],
            ],
        );
    }

    if usage_reset_at.is_none() {
        usage_reset_at = parse_timestamp(
            breakdown
                .and_then(|value| get_path_value(value, &["resetDate"]))
                .or_else(|| breakdown.and_then(|value| get_path_value(value, &["resetAt"]))),
        );
    }

    if bonus_expire_days.is_none() {
        bonus_expire_days = pick_number(
            free_trial,
            &[&["daysRemaining"], &["expiryDays"], &["expireDays"]],
        )
        .map(|value| value.round() as i64)
        .or_else(|| {
            days_until(parse_timestamp(
                free_trial.and_then(|value| get_path_value(value, &["expiryDate"])),
            ))
        })
        .or_else(|| {
            days_until(parse_timestamp(
                free_trial.and_then(|value| get_path_value(value, &["freeTrialExpiry"])),
            ))
        });
    }

    (
        plan_name,
        plan_tier,
        credits_total,
        credits_used,
        bonus_total,
        bonus_used,
        usage_reset_at,
        bonus_expire_days,
    )
}

fn extract_profile_arn(auth_token: Option<&Value>, profile: Option<&Value>) -> Option<String> {
    pick_string(
        profile,
        &[
            &["arn"],
            &["profileArn"],
            &["profile", "arn"],
            &["account", "arn"],
        ],
    )
    .or_else(|| pick_string(auth_token, &[&["profileArn"], &["profile_arn"], &["arn"]]))
}

fn extract_profile_name(auth_token: Option<&Value>, profile: Option<&Value>) -> Option<String> {
    pick_string(
        profile,
        &[
            &["name"],
            &["profileName"],
            &["provider"],
            &["loginProvider"],
        ],
    )
    .or_else(|| pick_string(auth_token, &[&["provider"], &["loginProvider"]]))
}

pub(crate) fn build_payload_from_snapshot(
    auth_token: Value,
    profile: Option<Value>,
    usage: Option<Value>,
) -> Result<KiroOAuthCompletePayload, String> {
    let access_token = pick_string(
        Some(&auth_token),
        &[
            &["accessToken"],
            &["access_token"],
            &["token"],
            &["idToken"],
            &["id_token"],
            &["accessTokenJwt"],
        ],
    )
    .ok_or_else(|| "Kiro 本地授权信息缺少 access token".to_string())?;

    let refresh_token = pick_string(
        Some(&auth_token),
        &[&["refreshToken"], &["refresh_token"], &["refreshTokenJwt"]],
    );
    let token_type = pick_string(
        Some(&auth_token),
        &[&["tokenType"], &["token_type"], &["authType"]],
    )
    .or_else(|| Some("Bearer".to_string()));

    let expires_at = parse_timestamp(
        get_path_value(&auth_token, &["expiresAt"])
            .or_else(|| get_path_value(&auth_token, &["expires_at"]))
            .or_else(|| get_path_value(&auth_token, &["expiry"]))
            .or_else(|| get_path_value(&auth_token, &["expiration"])),
    );

    let profile_arn = extract_profile_arn(Some(&auth_token), profile.as_ref());
    let profile_name = extract_profile_name(Some(&auth_token), profile.as_ref());

    let id_token_claims = pick_string(
        Some(&auth_token),
        &[
            &["idToken"],
            &["id_token"],
            &["idTokenJwt"],
            &["id_token_jwt"],
        ],
    )
    .and_then(|raw| decode_jwt_claims(&raw));
    let access_token_claims = pick_string(
        Some(&auth_token),
        &[
            &["accessToken"],
            &["access_token"],
            &["token"],
            &["accessTokenJwt"],
        ],
    )
    .and_then(|raw| decode_jwt_claims(&raw));

    let email = normalize_email(pick_string(
        profile.as_ref(),
        &[
            &["email"],
            &["user", "email"],
            &["account", "email"],
            &["primaryEmail"],
        ],
    ))
    .or_else(|| {
        normalize_email(pick_string(
            Some(&auth_token),
            &[&["email"], &["userEmail"]],
        ))
    })
    .or_else(|| {
        normalize_email(pick_string(
            id_token_claims.as_ref(),
            &[&["email"], &["upn"], &["preferred_username"]],
        ))
    })
    .or_else(|| {
        normalize_email(pick_string(
            access_token_claims.as_ref(),
            &[&["email"], &["upn"], &["preferred_username"]],
        ))
    })
    .or_else(|| {
        normalize_email(pick_string(
            Some(&auth_token),
            &[&["login_hint"], &["loginHint"]],
        ))
    })
    .unwrap_or_default();

    let user_id = pick_string(
        profile.as_ref(),
        &[
            &["userId"],
            &["user_id"],
            &["id"],
            &["sub"],
            &["account", "id"],
        ],
    )
    .or_else(|| {
        pick_string(
            Some(&auth_token),
            &[&["userId"], &["user_id"], &["sub"], &["accountId"]],
        )
    })
    .or_else(|| {
        pick_string(
            id_token_claims.as_ref(),
            &[&["sub"], &["user_id"], &["uid"]],
        )
    })
    .or_else(|| {
        pick_string(
            access_token_claims.as_ref(),
            &[&["sub"], &["user_id"], &["uid"]],
        )
    })
    .or_else(|| profile_arn.clone());

    let login_provider = pick_string(
        profile.as_ref(),
        &[
            &["loginProvider"],
            &["provider"],
            &["authProvider"],
            &["signedInWith"],
        ],
    )
    .or_else(|| {
        pick_string(
            Some(&auth_token),
            &[&["login_option"], &["provider"], &["loginProvider"]],
        )
    })
    .or_else(|| profile_name.clone())
    .map(|raw| provider_from_login_option(&raw).unwrap_or(raw));

    let idc_region = pick_string(
        Some(&auth_token),
        &[&["idc_region"], &["idcRegion"], &["region"]],
    );
    let issuer_url = pick_string(
        Some(&auth_token),
        &[&["issuer_url"], &["issuerUrl"], &["issuer"]],
    );
    let client_id = pick_string(Some(&auth_token), &[&["client_id"], &["clientId"]]);
    let scopes = pick_string(Some(&auth_token), &[&["scopes"], &["scope"]]);
    let login_hint = pick_string(Some(&auth_token), &[&["login_hint"], &["loginHint"]])
        .or_else(|| normalize_non_empty(Some(email.as_str())));

    let mut normalized_profile = profile.unwrap_or_else(|| json!({}));
    if !normalized_profile.is_object() {
        normalized_profile = json!({});
    }
    if let Some(obj) = normalized_profile.as_object_mut() {
        if let Some(arn) = profile_arn.clone() {
            obj.entry("arn".to_string())
                .or_insert_with(|| Value::String(arn));
        }
        if let Some(name) = profile_name.clone().or_else(|| login_provider.clone()) {
            obj.entry("name".to_string())
                .or_insert_with(|| Value::String(name));
        }
    }

    let normalized_profile = if normalized_profile
        .as_object()
        .map(|obj| !obj.is_empty())
        .unwrap_or(false)
    {
        Some(normalized_profile)
    } else {
        None
    };

    let (
        plan_name,
        plan_tier,
        credits_total,
        credits_used,
        bonus_total,
        bonus_used,
        usage_reset_at,
        bonus_expire_days,
    ) = extract_usage_payload(usage.as_ref());

    Ok(KiroOAuthCompletePayload {
        email,
        user_id,
        login_provider,
        access_token,
        refresh_token,
        token_type,
        expires_at,
        idc_region,
        issuer_url,
        client_id,
        scopes,
        login_hint,
        plan_name,
        plan_tier,
        credits_total,
        credits_used,
        bonus_total,
        bonus_used,
        usage_reset_at,
        bonus_expire_days,
        kiro_auth_token_raw: Some(auth_token),
        kiro_profile_raw: normalized_profile,
        kiro_usage_raw: usage,
        status: None,
        status_reason: None,
    })
}

pub fn build_payload_from_local_files() -> Result<KiroOAuthCompletePayload, String> {
    let auth_token = kiro_account::read_local_auth_token_json()?.ok_or_else(|| {
        "未在本机找到 Kiro 登录信息（~/.aws/sso/cache/kiro-auth-token.json）".to_string()
    })?;
    let profile = kiro_account::read_local_profile_json()?;
    let usage = kiro_account::read_local_usage_snapshot()?;
    build_payload_from_snapshot(auth_token, profile, usage)
}

async fn refresh_token_via_remote(refresh_token: &str) -> Result<Value, String> {
    let response = reqwest::Client::new()
        .post(KIRO_REFRESH_ENDPOINT)
        .header("Content-Type", "application/json")
        .json(&json!({
            "refreshToken": refresh_token
        }))
        .send()
        .await
        .map_err(|e| format!("请求 Kiro refreshToken 接口失败: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<no-body>".to_string());

    if !status.is_success() {
        return Err(format!(
            "Kiro refreshToken 接口返回异常: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    let mut token = unwrap_token_response(
        serde_json::from_str::<Value>(&body)
            .map_err(|e| format!("解析 Kiro refreshToken 响应失败: {}", e))?,
    );
    ensure_expires_at_from_expires_in(&mut token);
    Ok(token)
}

async fn refresh_token_via_idc_oidc(
    refresh_token: &str,
    auth_token: &Value,
    account: &KiroAccount,
) -> Result<Value, String> {
    let region = resolve_idc_region(auth_token, account)
        .ok_or_else(|| "缺少 idc_region，无法执行 AWS IAM Identity Center 刷新".to_string())?;
    let client_id = resolve_idc_client_id(auth_token, account)
        .ok_or_else(|| "缺少 client_id，无法执行 AWS IAM Identity Center 刷新".to_string())?;
    let client_secret = resolve_idc_client_secret(auth_token)
        .ok_or_else(|| "缺少 client_secret，无法执行 AWS IAM Identity Center 刷新".to_string())?;

    let endpoint = KIRO_AWS_OIDC_TOKEN_ENDPOINT_FMT.replace("{region}", region.as_str());
    let response = reqwest::Client::new()
        .post(endpoint.as_str())
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("请求 AWS IAM Identity Center OIDC 刷新接口失败: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<no-body>".to_string());
    if !status.is_success() {
        return Err(format!(
            "AWS IAM Identity Center OIDC 刷新接口返回异常: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    let mut token = unwrap_token_response(
        serde_json::from_str::<Value>(&body)
            .map_err(|e| format!("解析 AWS IAM Identity Center OIDC 刷新响应失败: {}", e))?,
    );
    if !token.is_object() {
        token = json!({});
    }
    if let Some(obj) = token.as_object_mut() {
        obj.entry("refreshToken".to_string())
            .or_insert_with(|| Value::String(refresh_token.to_string()));
        obj.entry("idc_region".to_string())
            .or_insert_with(|| Value::String(region.clone()));
        obj.entry("idcRegion".to_string())
            .or_insert_with(|| Value::String(region.clone()));
        obj.entry("region".to_string())
            .or_insert_with(|| Value::String(region.clone()));
        obj.entry("client_id".to_string())
            .or_insert_with(|| Value::String(client_id.clone()));
        obj.entry("clientId".to_string())
            .or_insert_with(|| Value::String(client_id));
        obj.entry("client_secret".to_string())
            .or_insert_with(|| Value::String(client_secret.clone()));
        obj.entry("clientSecret".to_string())
            .or_insert_with(|| Value::String(client_secret));
        obj.entry("authMethod".to_string())
            .or_insert_with(|| Value::String("IdC".to_string()));

        if let Some(provider) = pick_string(
            Some(auth_token),
            &[&["provider"], &["loginProvider"], &["login_option"]],
        )
        .and_then(|value| normalize_non_empty(Some(value.as_str())))
        {
            obj.entry("provider".to_string())
                .or_insert_with(|| Value::String(provider.clone()));
            obj.entry("loginProvider".to_string())
                .or_insert_with(|| Value::String(provider));
        }
        if let Some(issuer_url) = pick_string(
            Some(auth_token),
            &[&["issuer_url"], &["issuerUrl"], &["issuer"]],
        )
        .and_then(|value| normalize_non_empty(Some(value.as_str())))
        {
            obj.entry("issuer_url".to_string())
                .or_insert_with(|| Value::String(issuer_url.clone()));
            obj.entry("issuerUrl".to_string())
                .or_insert_with(|| Value::String(issuer_url));
        }
    }
    ensure_expires_at_from_expires_in(&mut token);
    Ok(token)
}

async fn fetch_usage_limits_via_runtime(
    access_token: &str,
    profile_arn: &str,
    is_email_required: bool,
) -> Result<Value, String> {
    let region = parse_profile_arn_region(profile_arn);
    let endpoint = runtime_endpoint_for_region(region.as_deref());
    let mut url = format!(
        "{}/getUsageLimits?origin=AI_EDITOR&profileArn={}&resourceType=AGENTIC_REQUEST",
        endpoint.trim_end_matches('/'),
        urlencoding::encode(profile_arn),
    );
    if is_email_required {
        url.push_str("&isEmailRequired=true");
    }

    let response = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token.trim()))
        .send()
        .await
        .map_err(|e| format!("请求 Kiro runtime usage 接口失败: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<no-body>".to_string());

    if !status.is_success() {
        let reason = parse_runtime_error_reason(&body)
            .or_else(|| (status == reqwest::StatusCode::FORBIDDEN).then(|| body.clone()));
        if let Some(reason) = reason {
            return Err(format!("BANNED:{}", reason));
        }
        return Err(format!(
            "Kiro runtime usage 接口返回异常: status={}, body_len={}",
            status,
            body.len()
        ));
    }

    serde_json::from_str::<Value>(&body)
        .map_err(|e| format!("解析 Kiro runtime usage 响应失败: {}", e))
}

fn merge_refreshed_auth_token_into_payload(
    payload: &mut KiroOAuthCompletePayload,
    auth_token: Value,
) {
    if let Some(value) = pick_string(
        Some(&auth_token),
        &[
            &["accessToken"],
            &["access_token"],
            &["token"],
            &["idToken"],
            &["id_token"],
            &["accessTokenJwt"],
        ],
    )
    .and_then(|v| normalize_non_empty(Some(v.as_str())))
    {
        payload.access_token = value;
    }
    if let Some(value) = pick_string(
        Some(&auth_token),
        &[&["refreshToken"], &["refresh_token"], &["refreshTokenJwt"]],
    )
    .and_then(|v| normalize_non_empty(Some(v.as_str())))
    {
        payload.refresh_token = Some(value);
    }
    if let Some(value) = pick_string(
        Some(&auth_token),
        &[&["tokenType"], &["token_type"], &["authType"]],
    )
    .and_then(|v| normalize_non_empty(Some(v.as_str())))
    {
        payload.token_type = Some(value);
    }
    if let Some(expires_at) = parse_timestamp(
        get_path_value(&auth_token, &["expiresAt"])
            .or_else(|| get_path_value(&auth_token, &["expires_at"]))
            .or_else(|| get_path_value(&auth_token, &["expiry"]))
            .or_else(|| get_path_value(&auth_token, &["expiration"])),
    ) {
        payload.expires_at = Some(expires_at);
    } else if let Some(expires_in) =
        pick_number(Some(&auth_token), &[&["expiresIn"], &["expires_in"]])
    {
        let now = now_timestamp();
        payload.expires_at = Some(now + expires_in.round() as i64);
    }

    if let Some(value) = pick_string(
        Some(&auth_token),
        &[&["provider"], &["loginProvider"], &["login_option"]],
    )
    .and_then(|v| normalize_non_empty(Some(v.as_str())))
    {
        payload.login_provider = Some(provider_from_login_option(&value).unwrap_or(value));
    }
    if let Some(value) = pick_string(
        Some(&auth_token),
        &[&["idc_region"], &["idcRegion"], &["region"]],
    )
    .and_then(|v| normalize_non_empty(Some(v.as_str())))
    {
        payload.idc_region = Some(value);
    }
    if let Some(value) = pick_string(
        Some(&auth_token),
        &[&["issuer_url"], &["issuerUrl"], &["issuer"]],
    )
    .and_then(|v| normalize_non_empty(Some(v.as_str())))
    {
        payload.issuer_url = Some(value);
    }
    if let Some(value) = pick_string(Some(&auth_token), &[&["client_id"], &["clientId"]])
        .and_then(|v| normalize_non_empty(Some(v.as_str())))
    {
        payload.client_id = Some(value);
    }
    if let Some(value) = pick_string(Some(&auth_token), &[&["scopes"], &["scope"]])
        .and_then(|v| normalize_non_empty(Some(v.as_str())))
    {
        payload.scopes = Some(value);
    }
    if let Some(value) = pick_string(Some(&auth_token), &[&["login_hint"], &["loginHint"]])
        .and_then(|v| normalize_non_empty(Some(v.as_str())))
    {
        payload.login_hint = Some(value);
    }

    let mut merged_auth = payload
        .kiro_auth_token_raw
        .clone()
        .unwrap_or_else(|| json!({}));
    if !merged_auth.is_object() {
        merged_auth = json!({});
    }
    if let (Some(target), Some(source)) = (merged_auth.as_object_mut(), auth_token.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    payload.kiro_auth_token_raw = Some(merged_auth);

    if let Some(profile_arn) = pick_string(
        Some(&auth_token),
        &[&["profileArn"], &["profile_arn"], &["arn"]],
    ) {
        let profile_value = normalize_non_empty(Some(profile_arn.as_str())).unwrap_or(profile_arn);
        let mut profile_raw = payload
            .kiro_profile_raw
            .clone()
            .unwrap_or_else(|| json!({}));
        if !profile_raw.is_object() {
            profile_raw = json!({});
        }
        if let Some(obj) = profile_raw.as_object_mut() {
            obj.entry("arn".to_string())
                .or_insert_with(|| Value::String(profile_value));
        }
        payload.kiro_profile_raw = Some(profile_raw);
    }
}

fn apply_runtime_usage_to_payload(payload: &mut KiroOAuthCompletePayload, usage: Value) {
    if let Some(email) = normalize_email(pick_string(
        Some(&usage),
        &[&["userInfo", "email"], &["email"]],
    )) {
        payload.email = email.clone();
        if payload
            .login_hint
            .as_deref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .is_none()
        {
            payload.login_hint = Some(email);
        }
    }

    if let Some(user_id) = pick_string(
        Some(&usage),
        &[&["userInfo", "userId"], &["userId"], &["user_id"], &["sub"]],
    )
    .and_then(|value| normalize_non_empty(Some(value.as_str())))
    {
        payload.user_id = Some(user_id);
    }

    if let Some(provider) = pick_string(
        Some(&usage),
        &[
            &["userInfo", "provider", "label"],
            &["userInfo", "provider", "name"],
            &["userInfo", "provider", "id"],
            &["userInfo", "providerId"],
            &["provider", "label"],
            &["provider", "name"],
            &["provider", "id"],
        ],
    )
    .and_then(|value| normalize_non_empty(Some(value.as_str())))
    {
        payload.login_provider = Some(provider_from_login_option(&provider).unwrap_or(provider));
    }

    let (
        plan_name,
        plan_tier,
        credits_total,
        credits_used,
        bonus_total,
        bonus_used,
        usage_reset_at,
        bonus_expire_days,
    ) = extract_usage_payload(Some(&usage));

    if let Some(value) = plan_name {
        payload.plan_name = Some(value);
    }
    if let Some(value) = plan_tier {
        payload.plan_tier = Some(value);
    }
    if let Some(value) = credits_total {
        payload.credits_total = Some(value);
    }
    if let Some(value) = credits_used {
        payload.credits_used = Some(value);
    }
    if let Some(value) = bonus_total {
        payload.bonus_total = Some(value);
    }
    if let Some(value) = bonus_used {
        payload.bonus_used = Some(value);
    }
    if let Some(value) = usage_reset_at {
        payload.usage_reset_at = Some(value);
    }
    if let Some(value) = bonus_expire_days {
        payload.bonus_expire_days = Some(value);
    }

    payload.kiro_usage_raw = Some(usage);
}

pub async fn enrich_payload_with_runtime_usage(
    mut payload: KiroOAuthCompletePayload,
) -> KiroOAuthCompletePayload {
    let Some(initial_profile_arn) = extract_profile_arn_from_payload(&payload) else {
        return payload;
    };

    let first_try = fetch_usage_limits_via_runtime(
        payload.access_token.as_str(),
        initial_profile_arn.as_str(),
        true,
    )
    .await;

    match first_try {
        Ok(usage) => {
            apply_runtime_usage_to_payload(&mut payload, usage);
            set_payload_status(&mut payload, KIRO_ACCOUNT_STATUS_NORMAL, None);
            return payload;
        }
        Err(err) => {
            if let Some(reason) = parse_banned_reason_from_error(&err) {
                set_payload_status(&mut payload, KIRO_ACCOUNT_STATUS_BANNED, Some(reason));
                return payload;
            }
            set_payload_status(&mut payload, KIRO_ACCOUNT_STATUS_ERROR, Some(err.clone()));
            logger::log_warn(&format!(
                "[Kiro Refresh] runtime usage 首次请求失败，准备尝试 refresh token: {}",
                err
            ));
        }
    }

    let Some(refresh_token) = payload
        .refresh_token
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
    else {
        return payload;
    };

    match refresh_token_via_remote(&refresh_token).await {
        Ok(auth_token) => {
            merge_refreshed_auth_token_into_payload(&mut payload, auth_token);
        }
        Err(err) => {
            set_payload_status(&mut payload, KIRO_ACCOUNT_STATUS_ERROR, Some(err.clone()));
            logger::log_warn(&format!(
                "[Kiro Refresh] refresh token 失败，跳过 runtime usage 回填: {}",
                err
            ));
            return payload;
        }
    }

    let profile_arn = extract_profile_arn_from_payload(&payload).unwrap_or(initial_profile_arn);
    match fetch_usage_limits_via_runtime(payload.access_token.as_str(), profile_arn.as_str(), true)
        .await
    {
        Ok(usage) => {
            apply_runtime_usage_to_payload(&mut payload, usage);
            set_payload_status(&mut payload, KIRO_ACCOUNT_STATUS_NORMAL, None);
        }
        Err(err) => {
            if let Some(reason) = parse_banned_reason_from_error(&err) {
                set_payload_status(&mut payload, KIRO_ACCOUNT_STATUS_BANNED, Some(reason));
            } else {
                set_payload_status(&mut payload, KIRO_ACCOUNT_STATUS_ERROR, Some(err.clone()));
            }
            logger::log_warn(&format!(
                "[Kiro Refresh] runtime usage 二次请求失败: {}",
                err
            ));
        }
    }

    payload
}

fn merge_account_context_into_auth_token(auth_token: &mut Value, account: &KiroAccount) {
    if !auth_token.is_object() {
        *auth_token = json!({});
    }
    let Some(target) = auth_token.as_object_mut() else {
        return;
    };

    if let Some(source) = account
        .kiro_auth_token_raw
        .as_ref()
        .and_then(|value| value.as_object())
    {
        for (key, value) in source {
            target.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    if !account.access_token.trim().is_empty() {
        target
            .entry("accessToken".to_string())
            .or_insert_with(|| Value::String(account.access_token.clone()));
    }
    if let Some(value) = account
        .refresh_token
        .as_deref()
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        target
            .entry("refreshToken".to_string())
            .or_insert_with(|| Value::String(value));
    }
    if let Some(value) = account
        .token_type
        .as_deref()
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        target
            .entry("tokenType".to_string())
            .or_insert_with(|| Value::String(value));
    }
    if let Some(value) = account
        .login_provider
        .as_deref()
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        target
            .entry("provider".to_string())
            .or_insert_with(|| Value::String(value.clone()));
        target
            .entry("loginProvider".to_string())
            .or_insert_with(|| Value::String(value));
    }
    if let Some(value) = account
        .idc_region
        .as_deref()
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        target
            .entry("idc_region".to_string())
            .or_insert_with(|| Value::String(value.clone()));
        target
            .entry("idcRegion".to_string())
            .or_insert_with(|| Value::String(value));
    }
    if let Some(value) = account
        .issuer_url
        .as_deref()
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        target
            .entry("issuer_url".to_string())
            .or_insert_with(|| Value::String(value.clone()));
        target
            .entry("issuerUrl".to_string())
            .or_insert_with(|| Value::String(value));
    }
    if let Some(value) = account
        .client_id
        .as_deref()
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        target
            .entry("client_id".to_string())
            .or_insert_with(|| Value::String(value.clone()));
        target
            .entry("clientId".to_string())
            .or_insert_with(|| Value::String(value));
    }
    if let Some(value) = account
        .scopes
        .as_deref()
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        target
            .entry("scopes".to_string())
            .or_insert_with(|| Value::String(value.clone()));
        target
            .entry("scope".to_string())
            .or_insert_with(|| Value::String(value));
    }
    if let Some(value) = account
        .login_hint
        .as_deref()
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        target
            .entry("login_hint".to_string())
            .or_insert_with(|| Value::String(value.clone()));
        target
            .entry("loginHint".to_string())
            .or_insert_with(|| Value::String(value));
    }
    if !account.email.trim().is_empty() {
        target
            .entry("email".to_string())
            .or_insert_with(|| Value::String(account.email.clone()));
    }
    if let Some(value) = account
        .user_id
        .as_deref()
        .and_then(|v| normalize_non_empty(Some(v)))
    {
        target
            .entry("userId".to_string())
            .or_insert_with(|| Value::String(value.clone()));
        target
            .entry("user_id".to_string())
            .or_insert_with(|| Value::String(value));
    }
    if let Some(profile_arn) = extract_profile_arn_from_account(account) {
        target
            .entry("profileArn".to_string())
            .or_insert_with(|| Value::String(profile_arn));
    }
}

fn pick_profile_and_usage_for_refresh(
    account: &KiroAccount,
    _auth_token: &Value,
) -> (Option<Value>, Option<Value>) {
    // 刷新逻辑仅依赖当前账号 JSON，不再读取 Kiro 本地快照文件。
    (
        account.kiro_profile_raw.clone(),
        account.kiro_usage_raw.clone(),
    )
}

pub async fn refresh_payload_for_account(
    account: &KiroAccount,
) -> Result<KiroOAuthCompletePayload, String> {
    // 刷新仅依赖账号 JSON 里的 refresh token + runtime usage 查询。
    if let Some(refresh_token) = account
        .refresh_token
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
    {
        let mut account_auth_token = account
            .kiro_auth_token_raw
            .clone()
            .unwrap_or_else(|| json!({}));
        merge_account_context_into_auth_token(&mut account_auth_token, account);

        let prefer_idc = should_prefer_idc_refresh(&account_auth_token, account);
        let mut refresh_error_messages: Vec<String> = Vec::new();
        let mut refreshed_auth_token: Option<Value> = None;

        if prefer_idc {
            match refresh_token_via_idc_oidc(&refresh_token, &account_auth_token, account).await {
                Ok(token) => {
                    refreshed_auth_token = Some(token);
                }
                Err(err) => {
                    logger::log_warn(&format!(
                        "[Kiro Refresh] AWS IAM Identity Center OIDC 刷新失败: {}",
                        err
                    ));
                    refresh_error_messages
                        .push(format!("AWS IAM Identity Center OIDC 刷新失败: {}", err));
                }
            }
        }

        if refreshed_auth_token.is_none() {
            match refresh_token_via_remote(&refresh_token).await {
                Ok(token) => {
                    refreshed_auth_token = Some(token);
                }
                Err(err) => {
                    logger::log_warn(&format!("[Kiro Refresh] refreshToken 接口失败: {}", err));
                    refresh_error_messages.push(format!("Kiro refreshToken 接口失败: {}", err));
                }
            }
        }

        if let Some(mut auth_token) = refreshed_auth_token {
            merge_account_context_into_auth_token(&mut auth_token, account);
            let (profile, usage) = pick_profile_and_usage_for_refresh(account, &auth_token);
            let payload = build_payload_from_snapshot(auth_token, profile, usage)?;
            return Ok(enrich_payload_with_runtime_usage(payload).await);
        }

        if refresh_error_messages.is_empty() {
            return Err("刷新 Kiro 登录态失败: 未命中可用刷新策略".to_string());
        }
        return Err(format!(
            "刷新 Kiro 登录态失败: {}",
            refresh_error_messages.join("；")
        ));
    }

    Err("账号缺少 refresh_token，无法刷新 Kiro 登录态".to_string())
}

pub async fn start_login() -> Result<KiroOAuthStartResponse, String> {
    hydrate_pending_login_if_missing();
    if let Ok(guard) = PENDING_OAUTH_STATE.lock() {
        if let Some(state) = guard.as_ref() {
            if state.expires_at > now_timestamp() && state.callback_result.is_none() {
                ensure_callback_server_for_state(state);
                return Ok(KiroOAuthStartResponse {
                    login_id: state.login_id.clone(),
                    user_code: String::new(),
                    verification_uri: state.verification_uri.clone(),
                    verification_uri_complete: Some(state.verification_uri_complete.clone()),
                    expires_in: (state.expires_at - now_timestamp()).max(0) as u64,
                    interval_seconds: 1,
                    callback_url: Some(state.callback_url.clone()),
                });
            }
        }
    }
    set_pending_login(None);

    let callback_port = find_available_callback_port()?;
    let callback_url = format!("http://localhost:{}", callback_port);
    let state_token = generate_token();
    let code_verifier = generate_token();
    let code_challenge = generate_code_challenge(&code_verifier);
    let verification_uri_complete = build_portal_auth_url(
        &state_token,
        &code_challenge,
        &callback_url,
        is_mwinit_tool_available(),
    );

    let pending = PendingOAuthState {
        login_id: generate_token(),
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS as i64,
        verification_uri: KIRO_AUTH_PORTAL_URL.to_string(),
        verification_uri_complete,
        callback_url: callback_url.clone(),
        callback_port,
        state_token: state_token.clone(),
        code_verifier,
        callback_result: None,
    };

    set_pending_login(Some(pending.clone()));

    let expected_login_id = pending.login_id.clone();
    let expected_state = state_token.clone();
    let callback_port = pending.callback_port;
    tokio::spawn(async move {
        if let Err(err) = start_callback_server(
            callback_port,
            expected_login_id.clone(),
            expected_state.clone(),
        )
        .await
        {
            logger::log_error(&format!(
                "[Kiro OAuth] 本地回调服务异常: login_id={}, error={}",
                expected_login_id, err
            ));
            set_callback_result_for_login(
                &expected_login_id,
                &expected_state,
                Err(format!("本地回调服务异常: {}", err)),
            );
        }
    });

    logger::log_info(&format!(
        "[Kiro OAuth] 登录会话已创建: login_id={}, callback_url={}, expires_in={}s",
        pending.login_id, pending.callback_url, OAUTH_TIMEOUT_SECONDS
    ));

    Ok(KiroOAuthStartResponse {
        login_id: pending.login_id,
        user_code: String::new(),
        verification_uri: pending.verification_uri.clone(),
        verification_uri_complete: Some(pending.verification_uri_complete),
        expires_in: OAUTH_TIMEOUT_SECONDS,
        interval_seconds: 1,
        callback_url: Some(callback_url),
    })
}

pub async fn complete_login(login_id: &str) -> Result<KiroOAuthCompletePayload, String> {
    hydrate_pending_login_if_missing();
    loop {
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
            let _ = cancel_login(Some(login_id));
            return Err("等待 Kiro 登录超时，请重新发起授权".to_string());
        }

        if let Some(result) = state.callback_result.clone() {
            let _ = cancel_login(Some(login_id));
            let callback = result?;

            let login_option = callback.login_option.trim().to_ascii_lowercase();
            if callback.code.is_none() {
                let reason = match login_option.as_str() {
                    "builderid" | "awsidc" | "internal" => {
                        "当前登录方式需要 Kiro 客户端后续认证流程，暂不支持直接导入，请改用 Google/GitHub 登录。"
                    }
                    "external_idp" => {
                        "当前登录方式为 External IdP，未返回授权 code，暂不支持自动导入。"
                    }
                    _ => "回调缺少授权 code，无法完成登录。",
                };
                return Err(reason.to_string());
            }

            let redirect_uri = build_token_exchange_redirect_uri(&state.callback_url, &callback);
            let auth_token =
                exchange_code_for_token(&callback, &state.code_verifier, &redirect_uri).await?;
            let payload = build_payload_from_snapshot(auth_token, None, None)?;
            return Ok(enrich_payload_with_runtime_usage(payload).await);
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(OAUTH_POLL_INTERVAL_MS)).await;
    }
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
        (Some(_), _) => {
            set_pending_login(None);
        }
        (None, _) => {}
    }
    Ok(())
}

pub fn submit_callback_url(login_id: &str, callback_url: &str) -> Result<(), String> {
    hydrate_pending_login_if_missing();
    let (expected_state, callback_port, expires_at) = {
        let guard = PENDING_OAUTH_STATE
            .lock()
            .map_err(|_| "OAuth 状态锁不可用".to_string())?;
        let state = guard
            .as_ref()
            .ok_or_else(|| "登录流程已取消，请重新发起授权".to_string())?;
        if state.login_id != login_id {
            return Err("登录会话已变更，请刷新后重试".to_string());
        }
        (
            state.state_token.clone(),
            state.callback_port,
            state.expires_at,
        )
    };

    if expires_at <= now_timestamp() {
        return Err("等待 Kiro 登录超时，请重新发起授权".to_string());
    }

    let parsed = parse_callback_url(callback_url, callback_port)?;
    let path = parsed.path();
    if path != "/oauth/callback" && path != "/signin/callback" {
        return Err("回调链接路径无效，必须为 /oauth/callback 或 /signin/callback".to_string());
    }

    let params = parse_query_params(parsed.query().unwrap_or_default());
    let error_code = params.get("error").cloned();
    let error_description = params
        .get("error_description")
        .cloned()
        .unwrap_or_else(String::new);
    if let Some(error_code) = error_code {
        let message = if error_description.trim().is_empty() {
            format!("授权失败: {}", error_code)
        } else {
            format!("授权失败: {} ({})", error_code, error_description)
        };
        set_callback_result_for_login(login_id, &expected_state, Err(message.clone()));
        return Err(message);
    }

    let callback_state = params.get("state").cloned().unwrap_or_default();
    if callback_state.is_empty() || callback_state != expected_state {
        return Err("授权状态校验失败，请确认粘贴的是当前登录会话链接".to_string());
    }

    let login_option = params
        .get("login_option")
        .or_else(|| params.get("loginOption"))
        .and_then(|value| normalize_non_empty(Some(value.as_str())))
        .unwrap_or_default()
        .to_ascii_lowercase();

    let callback = OAuthCallbackData {
        login_option,
        code: params
            .get("code")
            .and_then(|value| normalize_non_empty(Some(value.as_str()))),
        issuer_url: params
            .get("issuer_url")
            .or_else(|| params.get("issuerUrl"))
            .and_then(|value| normalize_non_empty(Some(value.as_str()))),
        idc_region: params
            .get("idc_region")
            .or_else(|| params.get("idcRegion"))
            .and_then(|value| normalize_non_empty(Some(value.as_str()))),
        path: path.to_string(),
        client_id: params
            .get("client_id")
            .or_else(|| params.get("clientId"))
            .and_then(|value| normalize_non_empty(Some(value.as_str()))),
        scopes: params
            .get("scopes")
            .or_else(|| params.get("scope"))
            .and_then(|value| normalize_non_empty(Some(value.as_str()))),
        login_hint: params
            .get("login_hint")
            .or_else(|| params.get("loginHint"))
            .and_then(|value| normalize_non_empty(Some(value.as_str()))),
        audience: params
            .get("audience")
            .and_then(|value| normalize_non_empty(Some(value.as_str()))),
    };

    set_callback_result_for_login(login_id, &expected_state, Ok(callback));
    logger::log_info(&format!(
        "[Kiro OAuth] 已接收手动回调链接: login_id={}",
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

pub async fn build_payload_from_token(token: &str) -> Result<KiroOAuthCompletePayload, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err("Token 不能为空".to_string());
    }

    let mut snapshot = json!({
        "accessToken": trimmed,
        "tokenType": "Bearer"
    });
    if let Some(obj) = snapshot.as_object_mut() {
        if let Some(claims) = decode_jwt_claims(trimmed) {
            if let Some(email) = pick_string(
                Some(&claims),
                &[&["email"], &["upn"], &["preferred_username"]],
            ) {
                obj.insert("email".to_string(), Value::String(email.clone()));
                obj.insert("login_hint".to_string(), Value::String(email));
            }
            if let Some(user_id) = pick_string(Some(&claims), &[&["sub"], &["user_id"], &["uid"]]) {
                obj.insert("userId".to_string(), Value::String(user_id.clone()));
                obj.insert("sub".to_string(), Value::String(user_id));
            }
            if let Some(name) = pick_string(Some(&claims), &[&["name"], &["nickname"]]) {
                obj.insert("provider".to_string(), Value::String(name));
            }
        }
    }

    let payload = build_payload_from_snapshot(snapshot, None, None)?;
    Ok(enrich_payload_with_runtime_usage(payload).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_payload_from_snapshot_supports_kiro_raw_json_shape() {
        let auth_token = json!({
            "email": "3493729266@qq.com",
            "accessToken": "test_access_token",
            "refreshToken": "test_refresh_token",
            "expiresAt": "2026/02/19 02:01:47",
            "provider": "Github",
            "userId": "user-123",
            "profileArn": "arn:aws:codewhisperer:us-east-1:699475941385:profile/EHGA3GRVQMUK"
        });
        let usage = json!({
            "nextDateReset": 1772323200,
            "subscriptionInfo": {
                "subscriptionTitle": "KIRO FREE",
                "type": "Q_DEVELOPER_STANDALONE_FREE"
            },
            "usageBreakdownList": [
                {
                    "usageLimitWithPrecision": 50,
                    "currentUsageWithPrecision": 0,
                    "freeTrialInfo": {
                        "currentUsageWithPrecision": 189.24,
                        "usageLimitWithPrecision": 500,
                        "freeTrialExpiry": 4_102_444_800_i64
                    }
                }
            ],
            "userInfo": {
                "email": "3493729266@qq.com",
                "userId": "user-123"
            }
        });

        let payload = build_payload_from_snapshot(auth_token, None, Some(usage))
            .expect("payload should parse");

        let expected_expires_at =
            chrono::NaiveDateTime::parse_from_str("2026/02/19 02:01:47", "%Y/%m/%d %H:%M:%S")
                .expect("valid datetime")
                .and_utc()
                .timestamp();

        assert_eq!(payload.email, "3493729266@qq.com");
        assert_eq!(payload.user_id.as_deref(), Some("user-123"));
        assert_eq!(payload.login_provider.as_deref(), Some("Github"));
        assert_eq!(payload.access_token, "test_access_token");
        assert_eq!(payload.refresh_token.as_deref(), Some("test_refresh_token"));
        assert_eq!(payload.expires_at, Some(expected_expires_at));
        assert_eq!(payload.plan_name.as_deref(), Some("KIRO FREE"));
        assert_eq!(
            payload.plan_tier.as_deref(),
            Some("Q_DEVELOPER_STANDALONE_FREE")
        );
        assert_eq!(payload.credits_total, Some(50.0));
        assert_eq!(payload.credits_used, Some(0.0));
        assert_eq!(payload.bonus_total, Some(500.0));
        assert!(
            payload
                .bonus_used
                .map(|value| (value - 189.24).abs() < 0.0001)
                .unwrap_or(false),
            "bonus_used should parse from freeTrialInfo.currentUsageWithPrecision"
        );
        assert_eq!(payload.usage_reset_at, Some(1772323200));
        assert!(
            payload.bonus_expire_days.unwrap_or(-1) > 0,
            "bonus_expire_days should derive from freeTrialExpiry"
        );
    }

    #[test]
    fn ensure_expires_at_from_expires_in_should_fill_rfc3339() {
        let mut token = json!({
            "accessToken": "token",
            "expiresIn": 3600
        });
        ensure_expires_at_from_expires_in(&mut token);
        let expires_at = token
            .get("expiresAt")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        assert!(
            chrono::DateTime::parse_from_rfc3339(expires_at).is_ok(),
            "expiresAt should be valid rfc3339"
        );
    }

    #[test]
    fn should_prefer_idc_refresh_for_enterprise_account() {
        let account = KiroAccount {
            id: "kiro_test".to_string(),
            email: "tester@example.com".to_string(),
            user_id: None,
            login_provider: Some("Enterprise".to_string()),
            tags: None,
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: None,
            idc_region: Some("us-east-1".to_string()),
            issuer_url: Some("https://example.awsapps.com/start".to_string()),
            client_id: Some("client-id".to_string()),
            scopes: None,
            login_hint: None,
            plan_name: None,
            plan_tier: None,
            credits_total: None,
            credits_used: None,
            bonus_total: None,
            bonus_used: None,
            usage_reset_at: None,
            bonus_expire_days: None,
            kiro_auth_token_raw: Some(json!({
                "authMethod": "IdC",
                "provider": "Enterprise",
                "idc_region": "us-east-1",
                "client_id": "client-id",
                "client_secret": "client-secret"
            })),
            kiro_profile_raw: None,
            kiro_usage_raw: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: 0,
            last_used: 0,
        };
        let auth_token = account
            .kiro_auth_token_raw
            .clone()
            .unwrap_or_else(|| json!({}));
        assert!(should_prefer_idc_refresh(&auth_token, &account));
        assert_eq!(
            resolve_idc_client_secret(&auth_token).as_deref(),
            Some("client-secret")
        );
    }
}
