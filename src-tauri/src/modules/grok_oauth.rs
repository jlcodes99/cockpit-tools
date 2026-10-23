use base64::Engine;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

use crate::models::grok::{GrokOAuthCompletePayload, GrokOAuthStartResponse};

pub const OIDC_ISSUER: &str = "https://auth.x.ai";
pub const OIDC_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const OIDC_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write workspaces:read workspaces:write";
pub const AUTH_REGISTRY_KEY: &str = "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828";
pub const DEFAULT_TOKEN_ENDPOINT: &str = "https://auth.x.ai/oauth2/token";

const DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const PENDING_FILE: &str = "grok_oauth_pending.json";
const DEFAULT_INTERVAL_SECONDS: u64 = 5;
const MAX_LOGIN_SECONDS: i64 = 30 * 60;
const MAX_CONSECUTIVE_POLL_TRANSPORT_ERRORS: u8 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingLogin {
    login_id: String,
    device_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    user_code: String,
    token_endpoint: String,
    userinfo_endpoint: Option<String>,
    expires_at: i64,
    interval_seconds: u64,
    cancelled: bool,
}

#[derive(Debug, Deserialize)]
struct DiscoveryResponse {
    device_authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    userinfo_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: i64,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrokTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: Option<String>,
    error_description: Option<String>,
}

enum PollResult {
    Pending,
    SlowDown,
    TransportFailure(String),
    Complete(GrokTokenResponse),
}

lazy_static::lazy_static! {
    static ref PENDING_LOGIN: Arc<Mutex<Option<PendingLogin>>> = Arc::new(Mutex::new(None));
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("创建 Grok OAuth 客户端失败: {}", error))
}

fn format_request_error(context: &str, error: &reqwest::Error) -> String {
    let category = if error.is_timeout() {
        "请求超时"
    } else if error.is_connect() {
        "连接失败"
    } else if error.is_body() {
        "响应传输失败"
    } else {
        "请求失败"
    };
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if !message.is_empty() && causes.last() != Some(&message) {
            causes.push(message);
        }
        source = cause.source();
    }
    if causes.is_empty() {
        format!("{}（{}）: {}", context, category, error)
    } else {
        format!(
            "{}（{}）: {}；原因: {}",
            context,
            category,
            error,
            causes.join(" -> ")
        )
    }
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn validate_xai_endpoint(raw: &str, field: &str) -> Result<String, String> {
    let parsed = url::Url::parse(raw.trim())
        .map_err(|error| format!("Grok OAuth {} URL 无效: {}", field, error))?;
    if parsed.scheme() != "https" {
        return Err(format!("Grok OAuth {} 必须使用 HTTPS", field));
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != "x.ai" && !host.ends_with(".x.ai") {
        return Err(format!("Grok OAuth {} 不属于 x.ai 域名", field));
    }
    Ok(parsed.to_string())
}

fn device_verification_url(
    verification_uri: &str,
    verification_uri_complete: Option<&str>,
    user_code: &str,
) -> Result<String, String> {
    if let Some(complete) = verification_uri_complete {
        return validate_xai_endpoint(complete, "verification_uri_complete");
    }
    // Follow the issuer's verification route, including any existing query.
    // Internal accounts-app sign-in parameters are not an OAuth contract.
    let validated = validate_xai_endpoint(verification_uri, "verification_uri")?;
    let mut url = url::Url::parse(&validated)
        .map_err(|error| format!("Grok OAuth verification_uri URL 无效: {}", error))?;
    url.query_pairs_mut().append_pair("user_code", user_code);
    Ok(url.to_string())
}

fn coding_data_retention_opt_out(claims: &Value) -> bool {
    // Match the official CLI: unknown sharing preferences remain opted out.
    claims
        .get("coding_data_retention_opt_out")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn set_pending(value: Option<PendingLogin>) -> Result<(), String> {
    if let Ok(mut guard) = PENDING_LOGIN.lock() {
        *guard = value.clone();
    }
    match value {
        Some(state) => crate::modules::oauth_pending_state::save(PENDING_FILE, &state),
        None => crate::modules::oauth_pending_state::clear(PENDING_FILE),
    }
}

fn hydrate_pending() {
    if let Ok(mut guard) = PENDING_LOGIN.lock() {
        if guard.is_none() {
            match crate::modules::oauth_pending_state::load::<PendingLogin>(PENDING_FILE) {
                Ok(Some(state)) if !state.cancelled && state.expires_at > now_ts() => {
                    *guard = Some(state);
                }
                Ok(Some(_)) => {
                    let _ = crate::modules::oauth_pending_state::clear(PENDING_FILE);
                }
                Ok(None) => {}
                Err(error) => {
                    crate::modules::logger::log_warn(&format!(
                        "[Grok OAuth] 读取 pending 状态失败，已忽略: {}",
                        error
                    ));
                }
            }
        }
    }
}

fn pending_for(login_id: &str) -> Result<PendingLogin, String> {
    hydrate_pending();
    let state = PENDING_LOGIN
        .lock()
        .map_err(|_| "获取 Grok OAuth 状态锁失败".to_string())?
        .clone()
        .ok_or_else(|| "Grok OAuth 登录流程不存在，请重新发起".to_string())?;
    if state.login_id != login_id {
        return Err("Grok OAuth 登录会话已变更，请重新发起".to_string());
    }
    if state.cancelled {
        return Err("Grok OAuth 登录已取消".to_string());
    }
    if now_ts() >= state.expires_at {
        return Err("Grok OAuth 登录已超时，请重试".to_string());
    }
    Ok(state)
}

async fn discover(client: &reqwest::Client) -> Result<DiscoveryResponse, String> {
    let response = client
        .get(DISCOVERY_URL)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| format_request_error("请求 Grok OAuth discovery 失败", &error))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Grok OAuth discovery 响应失败: {}", error))?;
    if !status.is_success() {
        return Err(format!("Grok OAuth discovery 返回 {}", status.as_u16()));
    }
    let mut result: DiscoveryResponse = serde_json::from_str(&body)
        .map_err(|error| format!("解析 Grok OAuth discovery 失败: {}", error))?;
    result.device_authorization_endpoint = validate_xai_endpoint(
        &result.device_authorization_endpoint,
        "device_authorization_endpoint",
    )?;
    result.token_endpoint = validate_xai_endpoint(&result.token_endpoint, "token_endpoint")?;
    result.userinfo_endpoint = result
        .userinfo_endpoint
        .as_deref()
        .map(|url| validate_xai_endpoint(url, "userinfo_endpoint"))
        .transpose()?;
    Ok(result)
}

pub async fn start_login() -> Result<GrokOAuthStartResponse, String> {
    let client = http_client()?;
    let discovery = discover(&client).await?;
    let response = client
        .post(&discovery.device_authorization_endpoint)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "application/json")
        .form(&[("client_id", OIDC_CLIENT_ID), ("scope", OIDC_SCOPE)])
        .send()
        .await
        .map_err(|error| format_request_error("发起 Grok device flow 失败", &error))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Grok device flow 响应失败: {}", error))?;
    if !status.is_success() {
        return Err(format!("Grok device flow 返回 {}", status.as_u16()));
    }
    let device: DeviceCodeResponse = serde_json::from_str(&body)
        .map_err(|error| format!("解析 Grok device flow 响应失败: {}", error))?;
    if device.device_code.trim().is_empty()
        || device.user_code.trim().is_empty()
        || device.verification_uri.trim().is_empty()
    {
        return Err("Grok device flow 响应缺少必要字段".to_string());
    }

    let expires_in = device.expires_in.clamp(1, MAX_LOGIN_SECONDS) as u64;
    let interval_seconds = device
        .interval
        .unwrap_or(DEFAULT_INTERVAL_SECONDS)
        .max(DEFAULT_INTERVAL_SECONDS);
    let verification_uri = validate_xai_endpoint(&device.verification_uri, "verification_uri")?;
    let verification_uri_complete = Some(device_verification_url(
        &verification_uri,
        device.verification_uri_complete.as_deref(),
        &device.user_code,
    )?);
    let state = PendingLogin {
        login_id: Uuid::new_v4().to_string(),
        device_code: device.device_code,
        verification_uri,
        verification_uri_complete,
        user_code: device.user_code,
        token_endpoint: discovery.token_endpoint,
        userinfo_endpoint: discovery.userinfo_endpoint,
        expires_at: now_ts() + expires_in as i64,
        interval_seconds,
        cancelled: false,
    };
    set_pending(Some(state.clone()))?;

    Ok(GrokOAuthStartResponse {
        login_id: state.login_id,
        verification_uri: state.verification_uri,
        verification_uri_complete: state.verification_uri_complete,
        user_code: state.user_code,
        expires_in,
        interval_seconds,
    })
}

async fn poll_once(client: &reqwest::Client, state: &PendingLogin) -> Result<PollResult, String> {
    let response = match client
        .post(&state.token_endpoint)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "application/json")
        .form(&[
            ("grant_type", DEVICE_GRANT_TYPE),
            ("device_code", state.device_code.as_str()),
            ("client_id", OIDC_CLIENT_ID),
        ])
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Ok(PollResult::TransportFailure(format_request_error(
                "轮询 Grok OAuth token 失败",
                &error,
            )))
        }
    };
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Grok OAuth token 响应失败: {}", error))?;
    if status.is_success() {
        let token: GrokTokenResponse = serde_json::from_str(&body)
            .map_err(|error| format!("解析 Grok OAuth token 失败: {}", error))?;
        if token.access_token.trim().is_empty() {
            return Err("Grok OAuth 未返回 access_token".to_string());
        }
        return Ok(PollResult::Complete(token));
    }

    let error: TokenErrorResponse = serde_json::from_str(&body).unwrap_or(TokenErrorResponse {
        error: None,
        error_description: None,
    });
    match error.error.as_deref() {
        Some("authorization_pending") => Ok(PollResult::Pending),
        Some("slow_down") => Ok(PollResult::SlowDown),
        Some("access_denied") => Err("Grok OAuth 授权已被拒绝".to_string()),
        Some("expired_token") => Err("Grok OAuth 验证码已过期".to_string()),
        Some(code) => Err(format!(
            "Grok OAuth 失败: {}{}",
            code,
            error
                .error_description
                .as_deref()
                .map(|value| format!(" ({})", value))
                .unwrap_or_default()
        )),
        None => Err(format!("Grok OAuth token 返回 {}", status.as_u16())),
    }
}

fn decode_jwt_claims(token: Option<&str>) -> Value {
    let Some(token) = token else {
        return json!({});
    };
    let Some(payload) = token.split('.').nth(1) else {
        return json!({});
    };
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or_else(|| json!({}))
}

fn claim_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .and_then(|text| normalize_text(Some(text)))
    })
}

async fn fetch_userinfo(
    client: &reqwest::Client,
    endpoint: Option<&str>,
    token: &str,
) -> Option<Value> {
    let endpoint = endpoint?;
    let response = client
        .get(endpoint)
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .header(ACCEPT, "application/json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().await.ok()
}

fn merge_claims(primary: Value, fallback: Value) -> Value {
    let mut result = fallback.as_object().cloned().unwrap_or_default();
    if let Some(object) = primary.as_object() {
        for (key, value) in object {
            result.insert(key.clone(), value.clone());
        }
    }
    Value::Object(result)
}

pub async fn complete_login(login_id: &str) -> Result<GrokOAuthCompletePayload, String> {
    let client = http_client()?;
    let mut interval = pending_for(login_id)?.interval_seconds;
    let mut consecutive_transport_errors = 0_u8;
    loop {
        let state = pending_for(login_id)?;
        match poll_once(&client, &state).await? {
            PollResult::Pending => {
                consecutive_transport_errors = 0;
            }
            PollResult::SlowDown => {
                consecutive_transport_errors = 0;
                interval = interval.saturating_add(DEFAULT_INTERVAL_SECONDS);
            }
            PollResult::TransportFailure(error) => {
                consecutive_transport_errors = consecutive_transport_errors.saturating_add(1);
                crate::modules::logger::log_warn(&format!(
                    "[Grok OAuth] token 轮询传输失败，第 {}/{} 次: {}",
                    consecutive_transport_errors, MAX_CONSECUTIVE_POLL_TRANSPORT_ERRORS, error
                ));
                if consecutive_transport_errors >= MAX_CONSECUTIVE_POLL_TRANSPORT_ERRORS {
                    return Err(error);
                }
            }
            PollResult::Complete(token) => {
                // Cancellation or a newer login may arrive while the token request is in flight.
                pending_for(login_id)?;
                let id_claims = decode_jwt_claims(token.id_token.as_deref());
                let access_claims = decode_jwt_claims(Some(&token.access_token));
                let userinfo = fetch_userinfo(
                    &client,
                    state.userinfo_endpoint.as_deref(),
                    &token.access_token,
                )
                .await
                .unwrap_or_else(|| json!({}));
                // The profile request can still be in flight when the user cancels
                // or starts a newer login, so validate the session once more before saving.
                pending_for(login_id)?;
                let claims = merge_claims(userinfo, merge_claims(id_claims, access_claims));
                let email = claim_text(&claims, &["email", "preferred_username"])
                    .unwrap_or_else(|| "unknown@grok.local".to_string());
                let expires_at = token
                    .expires_in
                    .filter(|seconds| *seconds > 0)
                    .map(|seconds| now_ts() + seconds);
                let principal_id = claim_text(&claims, &["principal_id", "sub"]);
                let user_id =
                    claim_text(&claims, &["user_id", "uid"]).or_else(|| principal_id.clone());
                let first_name = claim_text(&claims, &["first_name", "given_name"]);
                let last_name = claim_text(&claims, &["last_name", "family_name"]);
                let principal_type =
                    claim_text(&claims, &["principal_type"]).or_else(|| Some("User".to_string()));
                let team_id = claim_text(&claims, &["team_id"]);
                let profile_image_asset_id =
                    claim_text(&claims, &["profile_image_asset_id", "picture"]);
                let retention = Some(coding_data_retention_opt_out(&claims));
                let create_time = chrono::Utc::now().to_rfc3339();
                let expires_at_text = expires_at
                    .and_then(|value| chrono::DateTime::from_timestamp(value, 0))
                    .map(|value| value.to_rfc3339());
                let auth_raw = json!({
                    "key": token.access_token,
                    "auth_mode": "oidc",
                    "create_time": create_time,
                    "user_id": user_id,
                    "email": email,
                    "first_name": first_name,
                    "last_name": last_name,
                    "profile_image_asset_id": profile_image_asset_id,
                    "principal_type": principal_type,
                    "principal_id": principal_id,
                    "team_id": team_id,
                    "coding_data_retention_opt_out": retention.unwrap_or(true),
                    "refresh_token": token.refresh_token,
                    "expires_at": expires_at_text,
                    "oidc_issuer": OIDC_ISSUER,
                    "oidc_client_id": OIDC_CLIENT_ID,
                });
                let payload = GrokOAuthCompletePayload {
                    email,
                    first_name,
                    last_name,
                    user_id,
                    principal_id,
                    principal_type,
                    team_id,
                    profile_image_asset_id,
                    coding_data_retention_opt_out: retention,
                    access_token: auth_raw
                        .get("key")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    refresh_token: auth_raw
                        .get("refresh_token")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    id_token: token.id_token,
                    token_type: token.token_type,
                    expires_at,
                    token_endpoint: state.token_endpoint,
                    auth_raw,
                };
                set_pending(None)?;
                return Ok(payload);
            }
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    hydrate_pending();
    let should_cancel = {
        let guard = PENDING_LOGIN
            .lock()
            .map_err(|_| "获取 Grok OAuth 状态锁失败".to_string())?;
        match (
            guard.as_ref(),
            login_id.map(str::trim).filter(|id| !id.is_empty()),
        ) {
            (Some(state), Some(id)) => state.login_id == id,
            (Some(_), None) => true,
            _ => false,
        }
    };
    if should_cancel {
        set_pending(None)?;
    }
    Ok(())
}

pub async fn refresh_token(
    refresh_token: &str,
    token_endpoint: Option<&str>,
    client_id: Option<&str>,
) -> Result<GrokTokenResponse, String> {
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err("Grok refresh_token 为空，请重新授权".to_string());
    }
    let endpoint = validate_xai_endpoint(
        token_endpoint.unwrap_or(DEFAULT_TOKEN_ENDPOINT),
        "token_endpoint",
    )?;
    let client_id = client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(OIDC_CLIENT_ID);
    let client = http_client()?;
    let response = client
        .post(endpoint)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(|error| format_request_error("刷新 Grok token 失败", &error))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Grok token 刷新响应失败: {}", error))?;
    if !status.is_success() {
        let parsed: TokenErrorResponse =
            serde_json::from_str(&body).unwrap_or(TokenErrorResponse {
                error: None,
                error_description: None,
            });
        return Err(format!(
            "刷新 Grok token 失败: {}{}",
            parsed.error.unwrap_or_else(|| status.as_u16().to_string()),
            parsed
                .error_description
                .map(|value| format!(" ({})", value))
                .unwrap_or_default()
        ));
    }
    let token: GrokTokenResponse = serde_json::from_str(&body)
        .map_err(|error| format!("解析 Grok token 刷新响应失败: {}", error))?;
    if token.access_token.trim().is_empty() {
        return Err("刷新 Grok token 未返回 access_token".to_string());
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::{
        coding_data_retention_opt_out, decode_jwt_claims, device_verification_url,
        validate_xai_endpoint, OIDC_SCOPE,
    };
    use base64::Engine;
    use serde_json::json;

    #[test]
    fn oauth_scope_includes_official_workspace_permissions() {
        let scopes = OIDC_SCOPE.split_whitespace().collect::<Vec<_>>();
        for scope in [
            "offline_access",
            "grok-cli:access",
            "api:access",
            "conversations:read",
            "conversations:write",
            "workspaces:read",
            "workspaces:write",
        ] {
            assert!(scopes.contains(&scope), "missing official scope: {scope}");
        }
    }

    #[test]
    fn device_url_follows_issuer_route_and_preserves_query() {
        let result = device_verification_url(
            "https://auth.x.ai/oauth2/device?flow=unified#confirm",
            None,
            "ABCD EFGH&next=bad",
        )
        .expect("verification URL");
        let url = url::Url::parse(&result).expect("valid URL");
        assert_eq!(url.host_str(), Some("auth.x.ai"));
        assert_eq!(url.path(), "/oauth2/device");
        assert_eq!(url.fragment(), Some("confirm"));
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            vec![
                ("flow".into(), "unified".into()),
                ("user_code".into(), "ABCD EFGH&next=bad".into()),
            ]
        );
    }

    #[test]
    fn device_url_prefers_server_complete_url_and_rejects_untrusted_origins() {
        let complete = "https://accounts.x.ai/oauth2/device?user_code=SERVER-CODE";
        assert_eq!(
            device_verification_url("https://auth.x.ai/oauth2/device", Some(complete), "OTHER")
                .expect("complete URL"),
            complete
        );
        assert!(device_verification_url(
            "https://auth.x.ai/oauth2/device",
            Some("https://example.com/device"),
            "CODE"
        )
        .is_err());
        assert!(device_verification_url("http://auth.x.ai/device", None, "CODE").is_err());
    }

    #[test]
    fn unknown_retention_preference_is_opted_out_without_overriding_explicit_choice() {
        assert!(coding_data_retention_opt_out(&json!({})));
        assert!(coding_data_retention_opt_out(
            &json!({ "coding_data_retention_opt_out": null })
        ));
        assert!(coding_data_retention_opt_out(
            &json!({ "coding_data_retention_opt_out": true })
        ));
        assert!(!coding_data_retention_opt_out(
            &json!({ "coding_data_retention_opt_out": false })
        ));
    }

    #[test]
    fn validates_only_xai_https_endpoints() {
        assert!(validate_xai_endpoint("https://auth.x.ai/oauth/token", "token").is_ok());
        assert!(validate_xai_endpoint("http://auth.x.ai/oauth/token", "token").is_err());
        assert!(validate_xai_endpoint("https://example.com/oauth/token", "token").is_err());
    }

    #[test]
    fn decodes_jwt_claims_without_logging_token() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"email":"person@example.com"}"#);
        let token = format!("header.{}.signature", payload);
        assert_eq!(
            decode_jwt_claims(Some(&token))["email"],
            "person@example.com"
        );
    }
}
