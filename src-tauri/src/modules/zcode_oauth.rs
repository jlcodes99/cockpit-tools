use rand::RngCore;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use url::Url;
use uuid::Uuid;

use crate::models::zcode::{ZcodeAccount, ZcodeAuthMode, ZcodeOAuthStartResponse};
use crate::modules::{logger, zcode_account};

const OAUTH_TIMEOUT_SECONDS: i64 = 300;
const ZCODE_TOKEN_URL: &str = "https://zcode.z.ai/api/v1/oauth/token";
const BIGMODEL_AUTHORIZE_URL: &str = "https://bigmodel.cn/login";
const BIGMODEL_USER_INFO_URL: &str = "https://open.bigmodel.cn/api/biz/customer/getCustomerInfo";
const BIGMODEL_REDIRECT_URI: &str = "zcode://oauth/callback";
const ZAI_AUTHORIZE_URL: &str = "https://chat.z.ai/api/oauth/authorize";
const ZAI_USER_INFO_URL: &str = "https://chat.z.ai/api/oauth/userinfo";
const ZAI_BUSINESS_LOGIN_URL: &str = "https://api.z.ai/api/auth/z/login";
// OAuth redirect contract verified against ZCode 3.16.3. Both providers now
// return through the official web bridge to the shared desktop callback.
const DESKTOP_OAUTH_REDIRECT_URI: &str =
    "https://zcode.z.ai/app/oauth/login?redirect=zcode%3A%2F%2Foauth%2Fcallback&app_version=3.16.3";
const ZAI_CLIENT_ID: &str = "client_P8X5CMWmlaRO9gyO-KSqtg";
const ZCODE_OAUTH_WINDOW_LABEL: &str = "zcode-oauth";

#[derive(Debug, Clone)]
struct PendingOAuthState {
    login_id: String,
    provider: String,
    state: String,
    expires_at: i64,
    result: Option<ZcodeAccount>,
    error: Option<String>,
    processing: bool,
    cancelled: bool,
}

lazy_static::lazy_static! {
    static ref PENDING_OAUTH: Arc<Mutex<Option<PendingOAuthState>>> = Arc::new(Mutex::new(None));
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn normalize_provider(provider: &str) -> Result<&'static str, String> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "zai" => Ok("zai"),
        "bigmodel" => Ok("bigmodel"),
        _ => Err("不支持的 ZCode OAuth provider".to_string()),
    }
}

fn build_authorize_url(provider: &str, state: &str) -> Result<(String, String), String> {
    if provider == "zai" {
        let mut url = Url::parse(ZAI_AUTHORIZE_URL).map_err(|error| error.to_string())?;
        url.query_pairs_mut()
            .append_pair("redirect_uri", DESKTOP_OAUTH_REDIRECT_URI)
            .append_pair("response_type", "code")
            .append_pair("client_id", ZAI_CLIENT_ID)
            .append_pair("state", state);
        return Ok((url.to_string(), BIGMODEL_REDIRECT_URI.to_string()));
    }

    let mut url = Url::parse(BIGMODEL_AUTHORIZE_URL).map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("redirect", DESKTOP_OAUTH_REDIRECT_URI)
        .append_pair("appId", "zcode")
        .append_pair("state", state);
    Ok((url.to_string(), BIGMODEL_REDIRECT_URI.to_string()))
}

fn is_supported_authorize_url(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    matches!(
        (url.host_str(), url.path()),
        (Some("chat.z.ai"), "/api/oauth/authorize") | (Some("bigmodel.cn"), "/login")
    )
}

fn is_zcode_callback_url(url: &Url) -> bool {
    url.scheme().eq_ignore_ascii_case("zcode")
        && matches!(
            (url.host_str(), url.path()),
            (Some("zai-auth"), "/callback") | (Some("oauth"), "/callback")
        )
}

fn authorize_url_matches_pending(url: &Url, pending: &PendingOAuthState) -> bool {
    if !is_supported_authorize_url(url) {
        return false;
    }
    let query_value = |key: &str| {
        url.query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
    };
    if query_value("state").as_deref() != Some(pending.state.as_str()) {
        return false;
    }

    if pending.provider == "zai" {
        url.host_str() == Some("chat.z.ai")
            && url.path() == "/api/oauth/authorize"
            && query_value("client_id").as_deref() == Some(ZAI_CLIENT_ID)
            && query_value("response_type").as_deref() == Some("code")
            && query_value("redirect_uri").as_deref() == Some(DESKTOP_OAUTH_REDIRECT_URI)
    } else {
        url.host_str() == Some("bigmodel.cn")
            && url.path() == "/login"
            && query_value("appId").as_deref() == Some("zcode")
            && query_value("redirect").as_deref() == Some(DESKTOP_OAUTH_REDIRECT_URI)
    }
}

pub fn open_oauth_window(app: &AppHandle, auth_url: &str, incognito: bool) -> Result<(), String> {
    let parsed = Url::parse(auth_url.trim())
        .map_err(|error| format!("ZCode OAuth 授权地址无效: {}", error))?;
    let pending = PENDING_OAUTH
        .lock()
        .map_err(|_| "获取 ZCode OAuth 状态锁失败".to_string())?
        .as_ref()
        .cloned()
        .ok_or_else(|| "ZCode OAuth 登录会话不存在或已结束".to_string())?;
    if pending.expires_at <= now_ts() {
        return Err("ZCode OAuth 登录已过期".to_string());
    }
    if !authorize_url_matches_pending(&parsed, &pending) {
        return Err("ZCode OAuth 授权地址与当前登录会话不匹配".to_string());
    }

    if let Some(window) = app.get_webview_window(ZCODE_OAUTH_WINDOW_LABEL) {
        window
            .destroy()
            .map_err(|error| format!("重置 ZCode OAuth 授权窗口失败: {}", error))?;
    }

    let callback_app = app.clone();
    WebviewWindowBuilder::new(app, ZCODE_OAUTH_WINDOW_LABEL, WebviewUrl::External(parsed))
        .title("ZCode OAuth")
        .inner_size(920.0, 720.0)
        .min_inner_size(640.0, 560.0)
        .center()
        .incognito(incognito)
        .on_navigation(move |url| {
            if is_zcode_callback_url(url) {
                let callback_url = url.to_string();
                let app = callback_app.clone();
                logger::log_info("[ZCode OAuth] 内置授权窗口已拦截 zcode:// 回调");
                tauri::async_runtime::spawn(async move {
                    handle_deep_link(&callback_url).await;
                    if let Some(window) = app.get_webview_window(ZCODE_OAUTH_WINDOW_LABEL) {
                        let _ = window.close();
                    }
                });
                return false;
            }

            let allowed = matches!(url.scheme(), "https" | "about");
            if !allowed {
                logger::log_warn(&format!(
                    "[ZCode OAuth] 已阻止授权窗口导航到非 HTTPS 地址: scheme={}",
                    url.scheme()
                ));
            }
            allowed
        })
        .build()
        .map_err(|error| format!("创建 ZCode OAuth 授权窗口失败: {}", error))?;

    logger::log_info(&format!(
        "[ZCode OAuth] 已打开内置授权窗口: provider={}, incognito={}",
        pending.provider, incognito
    ));

    Ok(())
}

pub fn close_oauth_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(ZCODE_OAUTH_WINDOW_LABEL) {
        window
            .close()
            .map_err(|error| format!("关闭 ZCode OAuth 授权窗口失败: {}", error))?;
    }
    Ok(())
}

pub fn start_oauth_login(provider: &str) -> Result<ZcodeOAuthStartResponse, String> {
    let provider = normalize_provider(provider)?;
    cancel_oauth_login(None)?;
    let login_id = Uuid::new_v4().to_string();
    let state = generate_state();
    let (auth_url, callback_url) = build_authorize_url(provider, &state)?;
    let pending = PendingOAuthState {
        login_id: login_id.clone(),
        provider: provider.to_string(),
        state,
        expires_at: now_ts() + OAUTH_TIMEOUT_SECONDS,
        result: None,
        error: None,
        processing: false,
        cancelled: false,
    };
    *PENDING_OAUTH
        .lock()
        .map_err(|_| "获取 ZCode OAuth 状态锁失败".to_string())? = Some(pending);
    logger::log_info(&format!(
        "[ZCode OAuth] 登录会话已创建: login_id={}, provider={}",
        login_id, provider
    ));
    Ok(ZcodeOAuthStartResponse {
        login_id,
        provider: provider.to_string(),
        verification_uri: auth_url,
        expires_in: OAUTH_TIMEOUT_SECONDS as u64,
        interval_seconds: 1,
        callback_url,
    })
}

fn get_pending(login_id: &str) -> Result<PendingOAuthState, String> {
    let guard = PENDING_OAUTH
        .lock()
        .map_err(|_| "获取 ZCode OAuth 状态锁失败".to_string())?;
    let pending = guard
        .as_ref()
        .ok_or_else(|| "ZCode OAuth 登录会话不存在或已结束".to_string())?;
    if pending.login_id != login_id {
        return Err("ZCode OAuth 登录会话已变更".to_string());
    }
    Ok(pending.clone())
}

fn parse_callback_url(callback_url: &str, pending: &PendingOAuthState) -> Result<String, String> {
    let url = Url::parse(callback_url.trim())
        .map_err(|error| format!("ZCode OAuth 回调链接无效: {}", error))?;
    if url.scheme() != "zcode" {
        return Err("ZCode OAuth 回调协议必须是 zcode://".to_string());
    }
    let supported_host = url.host_str() == Some("oauth")
        || (pending.provider == "zai" && url.host_str() == Some("zai-auth"));
    if !supported_host || url.path() != "/callback" {
        return Err("ZCode OAuth 回调地址与登录 provider 不匹配".to_string());
    }
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| "ZCode OAuth 回调缺少 state".to_string())?;
    if state != pending.state {
        return Err("ZCode OAuth state 不匹配或已过期".to_string());
    }
    if let Some(error) = url
        .query_pairs()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.into_owned())
    {
        return Err(format!("ZCode OAuth 授权失败: {}", error));
    }
    url.query_pairs()
        .find(|(key, _)| key == "code" || key == "authCode")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "ZCode OAuth 回调缺少 code/authCode".to_string())
}

fn pick_string(value: &Value, paths: &[&[&str]]) -> Option<String> {
    for path in paths {
        let mut current = value;
        let mut found = true;
        for key in *path {
            if let Some(next) = current.get(*key) {
                current = next;
            } else {
                found = false;
                break;
            }
        }
        if found {
            if let Some(text) = current
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                return Some(text.to_string());
            }
        }
    }
    None
}

#[derive(Debug)]
struct OAuthTokenEnvelope {
    zcode_jwt_token: String,
    provider_access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    user_info: Value,
}

fn parse_token_envelope(
    provider: &str,
    token_response: &Value,
) -> Result<OAuthTokenEnvelope, String> {
    let code = token_response.get("code").and_then(Value::as_i64);
    let success = if provider == "zai" {
        code == Some(0)
    } else {
        code.is_none() || code == Some(0)
    };
    if !success {
        return Err(token_response
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("ZCode OAuth Token 交换失败")
            .to_string());
    }

    let zcode_jwt_token = pick_string(token_response, &[&["data", "token"]])
        .ok_or_else(|| "ZCode OAuth 响应缺少 data.token".to_string())?;
    if provider == "zai" {
        let provider_access_token = pick_string(
            token_response,
            &[
                &["data", "zai", "access_token"],
                &["data", "zai", "accessToken"],
            ],
        )
        .ok_or_else(|| "Z.ai OAuth 响应缺少 access_token".to_string())?;
        return Ok(OAuthTokenEnvelope {
            zcode_jwt_token,
            provider_access_token,
            refresh_token: None,
            expires_in: token_response
                .pointer("/data/expires_in")
                .and_then(Value::as_i64),
            user_info: token_response
                .pointer("/data/user")
                .cloned()
                .unwrap_or_else(|| json!({})),
        });
    }

    let provider_access_token = pick_string(
        token_response,
        &[
            &["data", "bigmodel", "access_token"],
            &["data", "bigmodel", "accessToken"],
            &["data", "access_token"],
            &["data", "accessToken"],
        ],
    )
    .ok_or_else(|| "BigModel OAuth 响应缺少 access_token".to_string())?;
    let refresh_token = pick_string(
        token_response,
        &[
            &["data", "bigmodel", "refresh_token"],
            &["data", "bigmodel", "refreshToken"],
        ],
    );
    Ok(OAuthTokenEnvelope {
        zcode_jwt_token,
        provider_access_token,
        refresh_token,
        expires_in: None,
        user_info: json!({}),
    })
}

fn parse_zai_business_token(response: &Value) -> Result<String, String> {
    let code_is_success = match response.get("code") {
        None | Some(Value::Null) => true,
        Some(Value::Number(code)) => code.as_i64().is_some_and(|code| code == 0 || code == 200),
        Some(Value::String(code)) => matches!(code.trim(), "0" | "200"),
        _ => false,
    };
    if !code_is_success || response.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(response
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("Z.ai 业务 Token 交换失败")
            .to_string());
    }
    pick_string(
        response,
        &[&["data", "access_token"], &["data", "accessToken"]],
    )
    .ok_or_else(|| "Z.ai 业务 Token 响应缺少 access_token".to_string())
}

async fn read_json_response(response: reqwest::Response, context: &str) -> Result<Value, String> {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{}失败: HTTP {}", context, status));
    }
    serde_json::from_str(&text).map_err(|error| format!("解析{}响应失败: {}", context, error))
}

async fn exchange_callback(
    pending: &PendingOAuthState,
    code: &str,
) -> Result<ZcodeAccount, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 ZCode OAuth 客户端失败: {}", error))?;
    let token_response = read_json_response(
        client
            .post(ZCODE_TOKEN_URL)
            .json(&json!({
                "provider": pending.provider,
                "code": code,
                "redirect_uri": DESKTOP_OAUTH_REDIRECT_URI,
                "state": pending.state,
            }))
            .send()
            .await
            .map_err(|error| format!("请求 ZCode OAuth Token 失败: {}", error))?,
        "ZCode OAuth Token 交换",
    )
    .await?;
    let envelope = parse_token_envelope(&pending.provider, &token_response)?;
    let zcode_jwt_token = envelope.zcode_jwt_token;
    let (access_token, refresh_token, expires_at, mut user_info) = if pending.provider == "zai" {
        let business_response = read_json_response(
            client
                .post(ZAI_BUSINESS_LOGIN_URL)
                .json(&json!({ "token": envelope.provider_access_token }))
                .send()
                .await
                .map_err(|error| format!("请求 Z.ai 业务 Token 失败: {}", error))?,
            "Z.ai 业务 Token 交换",
        )
        .await?;
        let access_token = parse_zai_business_token(&business_response)?;
        let expires_at = envelope.expires_in.map(|seconds| now_ts() + seconds);
        (access_token, None, expires_at, envelope.user_info)
    } else {
        (
            envelope.provider_access_token,
            envelope.refresh_token,
            None,
            envelope.user_info,
        )
    };

    if user_info.as_object().is_none_or(|value| value.is_empty()) {
        let request = if pending.provider == "zai" {
            client.get(ZAI_USER_INFO_URL).bearer_auth(&access_token)
        } else {
            client
                .get(BIGMODEL_USER_INFO_URL)
                .header(reqwest::header::AUTHORIZATION, &access_token)
        };
        if let Ok(response) = request.send().await {
            if let Ok(payload) = read_json_response(response, "ZCode 用户信息").await {
                user_info = payload.get("data").cloned().unwrap_or(payload);
            }
        }
    }

    let user_id = pick_string(
        &user_info,
        &[&["user_id"], &["id"], &["customerNumber"], &["sub"]],
    );
    let email =
        pick_string(&user_info, &[&["email"]]).unwrap_or_else(|| "unknown@zcode.local".to_string());
    let display_name = pick_string(
        &user_info,
        &[
            &["name"],
            &["displayName"],
            &["username"],
            &["nickName"],
            &["customerName"],
        ],
    );
    let avatar_url = pick_string(&user_info, &[&["avatar"], &["avatarUrl"], &["picture"]]);
    let now = now_ts();
    let account = ZcodeAccount {
        id: String::new(),
        auth_mode: ZcodeAuthMode::Oauth,
        provider: pending.provider.clone(),
        email,
        user_id,
        display_name,
        avatar_url,
        access_token,
        refresh_token,
        zcode_jwt_token,
        api_key: None,
        expires_at,
        plan_type: None,
        quota_total: None,
        quota_used: None,
        quota_remaining: None,
        quota_reset_at: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        tags: None,
        user_info_raw: Some(user_info),
        subscription_raw: None,
        quota_raw: None,
        created_at: now,
        last_used: now,
    };
    let saved = zcode_account::upsert_account(account)?;
    zcode_account::refresh_account_quota(&saved.id)
        .await
        .or(Ok(saved))
}

pub async fn submit_callback_url(login_id: &str, callback_url: &str) -> Result<(), String> {
    let pending = get_pending(login_id)?;
    if pending.expires_at <= now_ts() {
        return Err("ZCode OAuth 登录已过期".to_string());
    }
    let code = parse_callback_url(callback_url, &pending)?;
    {
        let mut guard = PENDING_OAUTH
            .lock()
            .map_err(|_| "获取 ZCode OAuth 状态锁失败".to_string())?;
        let current = guard
            .as_mut()
            .filter(|current| current.login_id == login_id)
            .ok_or_else(|| "ZCode OAuth 登录会话已变更".to_string())?;
        if current.processing || current.result.is_some() {
            return Ok(());
        }
        current.processing = true;
    }
    let result = exchange_callback(&pending, &code).await;
    match &result {
        Ok(_) => logger::log_info(&format!(
            "[ZCode OAuth] Token 交换及账号保存成功: provider={}",
            pending.provider
        )),
        Err(error) => logger::log_error(&format!(
            "[ZCode OAuth] Token 交换或账号保存失败: provider={}, error={}",
            pending.provider, error
        )),
    }
    let mut guard = PENDING_OAUTH
        .lock()
        .map_err(|_| "获取 ZCode OAuth 状态锁失败".to_string())?;
    if let Some(current) = guard
        .as_mut()
        .filter(|current| current.login_id == login_id)
    {
        current.processing = false;
        match result {
            Ok(account) => {
                current.error = None;
                current.result = Some(account);
            }
            Err(error) if current.result.is_none() => current.error = Some(error),
            Err(_) => {}
        }
    }
    Ok(())
}

pub async fn handle_deep_link(callback_url: &str) -> bool {
    if !callback_url
        .trim()
        .to_ascii_lowercase()
        .starts_with("zcode://")
    {
        return false;
    }
    let pending = match PENDING_OAUTH.lock() {
        Ok(guard) => guard.as_ref().cloned(),
        Err(_) => None,
    };
    let Some(pending) = pending else {
        logger::log_warn("[ZCode OAuth] 收到回调，但没有进行中的登录会话");
        return true;
    };
    if let Err(error) = submit_callback_url(&pending.login_id, callback_url).await {
        if let Ok(mut guard) = PENDING_OAUTH.lock() {
            if let Some(current) = guard
                .as_mut()
                .filter(|current| current.login_id == pending.login_id)
            {
                current.error = Some(error.clone());
            }
        }
        logger::log_error(&format!("[ZCode OAuth] 处理回调失败: {}", error));
    }
    true
}

pub async fn complete_oauth_login(login_id: &str) -> Result<ZcodeAccount, String> {
    loop {
        let pending = get_pending(login_id)?;
        if pending.cancelled {
            return Err("ZCode OAuth 登录已取消".to_string());
        }
        if pending.expires_at <= now_ts() {
            cancel_oauth_login(Some(login_id))?;
            return Err("ZCode OAuth 登录已超时".to_string());
        }
        if let Some(error) = pending.error {
            cancel_oauth_login(Some(login_id))?;
            return Err(error);
        }
        if let Some(account) = pending.result {
            cancel_oauth_login(Some(login_id))?;
            return Ok(account);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub fn cancel_oauth_login(login_id: Option<&str>) -> Result<(), String> {
    let mut guard = PENDING_OAUTH
        .lock()
        .map_err(|_| "获取 ZCode OAuth 状态锁失败".to_string())?;
    if login_id.is_none() || guard.as_ref().map(|pending| pending.login_id.as_str()) == login_id {
        *guard = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(provider: &str, state: &str) -> PendingOAuthState {
        PendingOAuthState {
            login_id: "login-1".to_string(),
            provider: provider.to_string(),
            state: state.to_string(),
            expires_at: now_ts() + 60,
            result: None,
            error: None,
            processing: false,
            cancelled: false,
        }
    }

    #[test]
    fn authorize_urls_use_official_redirects() {
        let (zai, zai_redirect) = build_authorize_url("zai", "state-1").unwrap();
        assert_eq!(zai_redirect, BIGMODEL_REDIRECT_URI);
        assert!(zai.contains("client_id="));
        assert!(zai.contains("state=state-1"));

        let (bigmodel, bigmodel_redirect) = build_authorize_url("bigmodel", "state-2").unwrap();
        assert_eq!(bigmodel_redirect, BIGMODEL_REDIRECT_URI);
        assert!(bigmodel.contains("appId=zcode"));
    }

    #[test]
    fn generated_state_matches_official_32_byte_hex_format() {
        let state = generate_state();
        assert_eq!(state.len(), 64);
        assert!(state.chars().all(|value| value.is_ascii_hexdigit()));
    }

    #[test]
    fn parses_official_zai_token_envelope() {
        let parsed = parse_token_envelope(
            "zai",
            &json!({
                "code": 0,
                "data": {
                    "token": "zcode-jwt",
                    "zai": { "access_token": "oauth-access" },
                    "expires_in": 3600,
                    "user": { "user_id": "user-1", "email": "user@example.com" }
                }
            }),
        )
        .unwrap();
        assert_eq!(parsed.zcode_jwt_token, "zcode-jwt");
        assert_eq!(parsed.provider_access_token, "oauth-access");
        assert_eq!(parsed.expires_in, Some(3600));
        assert_eq!(parsed.user_info["user_id"], "user-1");
        assert!(parsed.refresh_token.is_none());
    }

    #[test]
    fn parses_official_bigmodel_token_envelope_and_aliases() {
        let parsed = parse_token_envelope(
            "bigmodel",
            &json!({
                "data": {
                    "token": "zcode-jwt",
                    "bigmodel": {
                        "accessToken": "business-access",
                        "refreshToken": "refresh-token"
                    }
                }
            }),
        )
        .unwrap();
        assert_eq!(parsed.zcode_jwt_token, "zcode-jwt");
        assert_eq!(parsed.provider_access_token, "business-access");
        assert_eq!(parsed.refresh_token.as_deref(), Some("refresh-token"));
        assert!(parsed.expires_in.is_none());
    }

    #[test]
    fn zai_token_envelope_requires_success_code_and_surfaces_message() {
        let error = parse_token_envelope(
            "zai",
            &json!({ "code": 401, "msg": "authorization code expired" }),
        )
        .unwrap_err();
        assert_eq!(error, "authorization code expired");

        let missing_code = parse_token_envelope(
            "zai",
            &json!({ "data": { "token": "jwt", "zai": { "access_token": "token" } } }),
        )
        .unwrap_err();
        assert!(missing_code.contains("Token 交换失败"));
    }

    #[test]
    fn parses_zai_business_token_success_and_failure_envelopes() {
        assert_eq!(
            parse_zai_business_token(
                &json!({ "code": 200, "success": true, "data": { "accessToken": "business-token" } })
            )
            .unwrap(),
            "business-token"
        );
        assert_eq!(
            parse_zai_business_token(
                &json!({ "code": "0", "data": { "access_token": "business-token-2" } })
            )
            .unwrap(),
            "business-token-2"
        );
        let error = parse_zai_business_token(
            &json!({ "code": 0, "success": false, "msg": "oauth required" }),
        )
        .unwrap_err();
        assert_eq!(error, "oauth required");
    }

    #[test]
    fn embedded_window_only_accepts_official_authorize_urls() {
        assert!(is_supported_authorize_url(
            &Url::parse("https://chat.z.ai/api/oauth/authorize?client_id=test&state=state-1")
                .unwrap()
        ));
        assert!(is_supported_authorize_url(
            &Url::parse("https://bigmodel.cn/login?appId=zcode&state=state-1").unwrap()
        ));
        assert!(!is_supported_authorize_url(
            &Url::parse("https://chat.z.ai/auth/oauth/authorize").unwrap()
        ));
        assert!(!is_supported_authorize_url(
            &Url::parse("http://chat.z.ai/api/oauth/authorize").unwrap()
        ));
    }

    #[test]
    fn embedded_window_authorize_url_must_match_pending_provider_and_state() {
        let zai_pending = pending("zai", "state-1");
        let (zai_url, _) = build_authorize_url("zai", "state-1").unwrap();
        assert!(authorize_url_matches_pending(
            &Url::parse(&zai_url).unwrap(),
            &zai_pending
        ));

        let (wrong_state, _) = build_authorize_url("zai", "state-2").unwrap();
        assert!(!authorize_url_matches_pending(
            &Url::parse(&wrong_state).unwrap(),
            &zai_pending
        ));

        let (wrong_provider, _) = build_authorize_url("bigmodel", "state-1").unwrap();
        assert!(!authorize_url_matches_pending(
            &Url::parse(&wrong_provider).unwrap(),
            &zai_pending
        ));
    }

    #[test]
    fn embedded_window_recognizes_only_zcode_callback_routes() {
        assert!(is_zcode_callback_url(
            &Url::parse("zcode://zai-auth/callback?code=value&state=state-1").unwrap()
        ));
        assert!(is_zcode_callback_url(
            &Url::parse("zcode://oauth/callback?authCode=value&state=state-1").unwrap()
        ));
        assert!(!is_zcode_callback_url(
            &Url::parse("zcode://oauth/other?code=value").unwrap()
        ));
        assert!(!is_zcode_callback_url(
            &Url::parse("https://oauth/callback?code=value").unwrap()
        ));
    }

    #[test]
    fn callback_accepts_matching_state_and_provider_route() {
        let zai = pending("zai", "state+with/chars");
        assert_eq!(
            parse_callback_url(
                "zcode://zai-auth/callback?code=zai-code&state=state%2Bwith%2Fchars",
                &zai,
            )
            .unwrap(),
            "zai-code"
        );

        let bigmodel = pending("bigmodel", "bigmodel-state");
        assert_eq!(
            parse_callback_url(
                "zcode://oauth/callback?authCode=bigmodel-code&state=bigmodel-state",
                &bigmodel,
            )
            .unwrap(),
            "bigmodel-code"
        );
    }

    #[test]
    fn current_zai_login_uses_web_bridge_and_shared_callback() {
        let (authorize, callback) = build_authorize_url("zai", "state-1").unwrap();
        let authorize = Url::parse(&authorize).unwrap();
        let bridge = authorize
            .query_pairs()
            .find(|(key, _)| key == "redirect_uri")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(bridge, DESKTOP_OAUTH_REDIRECT_URI);
        assert_eq!(callback, "zcode://oauth/callback");
        assert_eq!(
            parse_callback_url(
                "zcode://oauth/callback?code=new-code&state=state-1",
                &pending("zai", "state-1")
            )
            .unwrap(),
            "new-code"
        );
    }

    #[test]
    fn callback_rejects_missing_or_mismatched_state() {
        let pending = pending("zai", "expected-state");
        let missing =
            parse_callback_url("zcode://zai-auth/callback?code=value", &pending).unwrap_err();
        assert!(missing.contains("缺少 state"));

        let mismatch = parse_callback_url(
            "zcode://zai-auth/callback?code=value&state=unexpected-state",
            &pending,
        )
        .unwrap_err();
        assert!(mismatch.contains("state 不匹配"));
    }

    #[test]
    fn callback_rejects_route_for_another_provider() {
        let pending = pending("bigmodel", "state-1");
        let error = parse_callback_url(
            "zcode://zai-auth/callback?code=value&state=state-1",
            &pending,
        )
        .unwrap_err();
        assert!(error.contains("provider 不匹配"));
    }

    #[test]
    fn callback_surfaces_oauth_error_before_missing_code() {
        let pending = pending("zai", "state-1");
        let error = parse_callback_url(
            "zcode://zai-auth/callback?error=access_denied&state=state-1",
            &pending,
        )
        .unwrap_err();
        assert!(error.contains("access_denied"));
    }
}
