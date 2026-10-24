use serde_json::Value;
use std::sync::{Arc, Mutex};

use crate::models::minimax_code::{MinimaxCodeOAuthCompletePayload, MinimaxCodeOAuthStartResponse};
use crate::modules::logger;

const ACCOUNT_ORIGIN_CN: &str = "https://account.minimax.cn";
const ACCOUNT_ORIGIN_EN: &str = "https://account.minimax.io";
const AGENT_ORIGIN_CN: &str = "https://agent.minimax.cn";
const AGENT_ORIGIN_EN: &str = "https://agent.minimax.io";
const DEVICE_CODE_PATH: &str = "/oauth2/device/code";
const TOKEN_PATH: &str = "/oauth2/token";
const USER_INFO_PATH: &str = "/v1/api/user/info";
const CLIENT_ID: &str = "mcode-public";
const SCOPES: &str = "agent.default";
const AUDIENCE: &str = "agent-backend";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const OAUTH_TIMEOUT_SECONDS: i64 = 900;
const OAUTH_POLL_INTERVAL_MS: u64 = 5000;

#[derive(Clone)]
struct PendingOAuthState {
    login_id: String,
    device_code: String,
    code_verifier: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    region: String,
    expires_at: i64,
    cancelled: bool,
}

lazy_static::lazy_static! {
    static ref PENDING_OAUTH_STATE: Arc<Mutex<Option<PendingOAuthState>>> = Arc::new(Mutex::new(None));
}

fn now_timestamp() -> i64 { chrono::Utc::now().timestamp() }

pub fn resolve_account_origin(region: &str) -> &'static str {
    if region == "en" { ACCOUNT_ORIGIN_EN } else { ACCOUNT_ORIGIN_CN }
}
pub fn resolve_agent_origin(region: &str) -> &'static str {
    if region == "en" { AGENT_ORIGIN_EN } else { AGENT_ORIGIN_CN }
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("MiniMax Code")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))
}

fn generate_login_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..16).map(|_| rng.gen::<u8>()).collect();
    format!("mmc_{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn base64_url_safe_no_pad(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_verifier() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.gen::<u8>()).collect();
    base64_url_safe_no_pad(&bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

pub fn clear_pending_oauth_login(login_id: &str) -> Result<(), String> {
    let mut pending = PENDING_OAUTH_STATE.lock().map_err(|_| "获取锁失败".to_string())?;
    if pending.as_ref().map(|s| s.login_id == login_id).unwrap_or(false) {
        *pending = None;
    }
    Ok(())
}

/// Step 1: request a device code + user code.
pub async fn start_login(region: &str) -> Result<MinimaxCodeOAuthStartResponse, String> {
    let client = build_client()?;
    let origin = resolve_account_origin(region);
    let url = format!("{origin}{DEVICE_CODE_PATH}");

    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    let body = client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("client_id", CLIENT_ID),
            ("scope", SCOPES),
            ("audience", AUDIENCE),
            ("code_challenge", &code_challenge),
            ("code_challenge_method", "S256"),
        ])
        .send()
        .await
        .map_err(|e| format!("请求 device/code 失败: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| format!("解析 device/code 响应失败: {e}"))?;

    let device_code = body.get("device_code").and_then(|v| v.as_str())
        .ok_or_else(|| "device/code 响应缺少 device_code".to_string())?.to_string();
    let user_code = body.get("user_code").and_then(|v| v.as_str())
        .ok_or_else(|| "device/code 响应缺少 user_code".to_string())?.to_string();
    let verification_uri = body.get("verification_uri").or_else(|| body.get("verification_url"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "device/code 响应缺少 verification_uri".to_string())?.to_string();
    let verification_uri_complete = body.get("verification_uri_complete").and_then(|v| v.as_str()).map(|s| s.to_string());
    let expires_in = body.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(OAUTH_TIMEOUT_SECONDS) as u64;
    let interval_seconds = body.get("interval").and_then(|v| v.as_i64()).unwrap_or(5).max(1) as u64;

    let login_id = generate_login_id();
    {
        let mut pending = PENDING_OAUTH_STATE.lock().map_err(|_| "获取锁失败".to_string())?;
        *pending = Some(PendingOAuthState {
            login_id: login_id.clone(),
            device_code,
            code_verifier,
            user_code: user_code.clone(),
            verification_uri: verification_uri.clone(),
            verification_uri_complete: verification_uri_complete.clone(),
            region: region.to_string(),
            expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS,
            cancelled: false,
        });
    }

    Ok(MinimaxCodeOAuthStartResponse {
        login_id,
        verification_uri,
        verification_uri_complete,
        user_code,
        expires_in,
        interval_seconds,
    })
}

/// Step 2: poll for the token, then fetch user info + quota.
pub async fn complete_login(login_id: &str) -> Result<MinimaxCodeOAuthCompletePayload, String> {
    let client = build_client()?;
    let start = now_timestamp();
    let mut last_error: Option<String> = None;

    loop {
        let snapshot = {
            let pending = PENDING_OAUTH_STATE.lock().map_err(|_| "获取锁失败".to_string())?;
            match pending.as_ref() {
                None => return Err("没有待处理的登录请求".to_string()),
                Some(s) => {
                    if s.login_id != login_id { return Err("login_id 不匹配".to_string()); }
                    if s.cancelled { return Err("登录已取消".to_string()); }
                    if now_timestamp() > s.expires_at { return Err("登录超时".to_string()); }
                    (s.device_code.clone(), s.code_verifier.clone(), s.region.clone())
                }
            }
        };

        let origin = resolve_account_origin(&snapshot.2);
        let url = format!("{origin}{TOKEN_PATH}");
        let body = client
            .post(&url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", DEVICE_GRANT_TYPE),
                ("device_code", snapshot.0.as_str()),
                ("client_id", CLIENT_ID),
                ("code_verifier", snapshot.1.as_str()),
            ])
            .send()
            .await
            .map_err(|e| format!("请求 token 失败: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("解析 token 响应失败: {e}"))?;

        if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
            if err == "authorization_pending" || err == "slow_down" {
                last_error = Some(err.to_string());
            } else {
                let desc = body.get("error_description").and_then(|v| v.as_str()).unwrap_or("");
                return Err(format!("OAuth 失败 ({err}): {desc}"));
            }
        } else {
            let access_token = body.get("access_token").and_then(|v| v.as_str())
                .ok_or_else(|| "token 响应缺少 access_token".to_string())?.to_string();
            let refresh_token = body.get("refresh_token").and_then(|v| v.as_str()).map(|s| s.to_string());
            let expires_at = body.get("expires_in").and_then(|v| v.as_i64())
                .map(|secs| chrono::Utc::now().timestamp() + secs);
            let token_type = body.get("token_type").and_then(|v| v.as_str()).map(|s| s.to_string());

            clear_pending_oauth_login(login_id)?;

            let (uid, nickname, email) = fetch_user_info(&client, &access_token, &snapshot.2).await
                .unwrap_or((None, None, String::new()));
            let quota_raw = fetch_credit_details(&client, &access_token, &snapshot.2).await.ok();

            return Ok(MinimaxCodeOAuthCompletePayload {
                email: if email.is_empty() { "unknown".to_string() } else { email },
                uid, nickname, access_token, refresh_token, token_type, expires_at,
                region: Some(snapshot.2.clone()),
                auth_file: None,
                quota_raw, auth_raw: None, profile_raw: None,
                status: Some("normal".to_string()),
                status_reason: None,
            });
        }

        if now_timestamp() - start > OAUTH_TIMEOUT_SECONDS {
            clear_pending_oauth_login(login_id)?;
            return Err(last_error.map(|e| format!("登录超时 ({e})")).unwrap_or_else(|| "登录超时".to_string()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(OAUTH_POLL_INTERVAL_MS)).await;
    }
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    let mut pending = PENDING_OAUTH_STATE.lock().map_err(|_| "获取锁失败".to_string())?;
    if let Some(state) = pending.as_mut() {
        if login_id.is_none() || login_id == Some(state.login_id.as_str()) {
            state.cancelled = true;
            *pending = None;
        }
    }
    Ok(())
}

pub fn peek_pending_login() -> Option<MinimaxCodeOAuthStartResponse> {
    let pending = PENDING_OAUTH_STATE.lock().ok()?;
    let state = pending.as_ref()?;
    if state.cancelled || now_timestamp() > state.expires_at { return None; }
    Some(MinimaxCodeOAuthStartResponse {
        login_id: state.login_id.clone(),
        verification_uri: state.verification_uri.clone(),
        verification_uri_complete: state.verification_uri_complete.clone(),
        user_code: state.user_code.clone(),
        expires_in: OAUTH_TIMEOUT_SECONDS as u64,
        interval_seconds: OAUTH_POLL_INTERVAL_MS / 1000,
    })
}

pub async fn fetch_user_info(
    client: &reqwest::Client,
    access_token: &str,
    region: &str,
) -> Result<(Option<String>, Option<String>, String), String> {
    let origin = resolve_agent_origin(region);
    let url = format!("{origin}{USER_INFO_PATH}");
    let body: Value = client
        .get(&url)
        .bearer_auth(access_token)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("请求 user/info 失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("解析 user/info 响应失败: {e}"))?;

    let data = body.get("data").cloned().unwrap_or(body);
    let uid = data.get("uid").or_else(|| data.get("userId")).and_then(|v| v.as_str()).map(|s| s.to_string());
    let nickname = data.get("nickname").or_else(|| data.get("userName")).and_then(|v| v.as_str()).map(|s| s.to_string());
    let email = data.get("email").or_else(|| data.get("userEmail")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    Ok((uid, nickname, email))
}

pub async fn fetch_credit_details(
    client: &reqwest::Client,
    access_token: &str,
    region: &str,
) -> Result<Value, String> {
    let origin = resolve_agent_origin(region);
    let tz = crate::modules::minimax_code_api::iana_timezone();
    let url = format!("{origin}/minimax-cloud/api/v1/credit/details?timezone_id={tz}");
    let body: Value = client
        .get(&url)
        .bearer_auth(access_token)
        .header("accept", "application/json")
        .header("accept-language", "zh-CN,zh;q=0.9")
        .send()
        .await
        .map_err(|e| format!("请求 credit/details 失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("解析 credit/details 响应失败: {e}"))?;
    Ok(body)
}

/// Build a payload from a manually-pasted access token.
pub async fn build_payload_from_token(access_token: &str, region: Option<&str>) -> Result<MinimaxCodeOAuthCompletePayload, String> {
    let client = build_client()?;
    let region = region.unwrap_or("cn").to_string();
    let (uid, nickname, email) = fetch_user_info(&client, access_token, &region).await
        .unwrap_or((None, None, String::new()));
    let quota_raw = fetch_credit_details(&client, access_token, &region).await.ok();
    Ok(MinimaxCodeOAuthCompletePayload {
        email: if email.is_empty() { "unknown".to_string() } else { email },
        uid, nickname,
        access_token: access_token.to_string(),
        refresh_token: None,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        region: Some(region),
        auth_file: None,
        quota_raw, auth_raw: None, profile_raw: None,
        status: Some("normal".to_string()),
        status_reason: None,
    })
}

/// Refresh an access token using the refresh token.
pub async fn refresh_token(
    access_token: &str,
    refresh_token: &str,
    region: &str,
) -> Result<Value, String> {
    let client = build_client()?;
    let origin = resolve_account_origin(region);
    let url = format!("{origin}{TOKEN_PATH}");
    let body: Value = client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ])
        .send()
        .await
        .map_err(|e| format!("刷新 token 失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("解析刷新响应失败: {e}"))?;
    if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
        return Err(format!("刷新 token 失败 ({err})"));
    }
    Ok(body)
}

/// Build a payload from the MiniMax client's local `auth.json` (used by local import).
pub fn payload_from_local_record(id: &str, rec: &Value, auth_file: &str) -> MinimaxCodeOAuthCompletePayload {
    let access = rec.get("accessToken").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let refresh = rec.get("refreshToken").and_then(|v| v.as_str()).map(|s| s.to_string());
    let token_type = rec.get("tokenType").and_then(|v| v.as_str()).map(|s| s.to_string());
    let expires_at = rec.get("expiresAtMs").and_then(|v| v.as_i64());
    MinimaxCodeOAuthCompletePayload {
        email: id.to_string(),
        uid: None,
        nickname: None,
        access_token: access,
        refresh_token: refresh,
        token_type,
        expires_at,
        region: Some("cn".to_string()),
        auth_file: Some(auth_file.to_string()),
        quota_raw: None,
        auth_raw: Some(rec.clone()),
        profile_raw: None,
        status: Some("normal".to_string()),
        status_reason: None,
    }
}
