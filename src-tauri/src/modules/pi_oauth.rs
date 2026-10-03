//! Built-in OAuth login for pi providers. Mirrors pi's own `/login` flows
//! (pi-ai `auth/oauth/*.js`) so the stored `auth.json` entries are identical.

use std::sync::Mutex;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::models::pi::PiAccountView;
use crate::modules::{logger, pi_account};

const LOGIN_TIMEOUT_SECS: i64 = 600;

const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const ANTHROPIC_PORT: u16 = 53692;
const ANTHROPIC_SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_PORT: u16 = 1455;
const CODEX_SCOPES: &str = "openid profile email offline_access";
const CODEX_JWT_CLAIM: &str = "https://api.openai.com/auth";

const COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const COPILOT_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const COPILOT_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const COPILOT_DEFAULT_BASE_URL: &str = "https://api.individual.githubcopilot.com";
const COPILOT_API_VERSION: &str = "2026-06-01";
const COPILOT_HEADERS: [(&str, &str); 4] = [
    ("User-Agent", "GitHubCopilotChat/0.35.0"),
    ("Editor-Version", "vscode/1.107.0"),
    ("Editor-Plugin-Version", "copilot-chat/0.35.0"),
    ("Copilot-Integration-Id", "vscode-chat"),
];

const KIMI_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_DEVICE_CODE_URL: &str = "https://auth.kimi.com/api/oauth/device_authorization";
const KIMI_TOKEN_URL: &str = "https://auth.kimi.com/api/oauth/token";

const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const XAI_DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";

const OPENROUTER_AUTHORIZE_URL: &str = "https://openrouter.ai/auth";
const OPENROUTER_KEYS_URL: &str = "https://openrouter.ai/api/v1/auth/keys";

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Provider {
    Anthropic,
    OpenAiCodex,
    GithubCopilot,
    KimiCoding,
    Xai,
    OpenRouter,
}

impl Provider {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "anthropic" => Ok(Self::Anthropic),
            "openai-codex" => Ok(Self::OpenAiCodex),
            "github-copilot" => Ok(Self::GithubCopilot),
            "kimi-coding" => Ok(Self::KimiCoding),
            "xai" => Ok(Self::Xai),
            "openrouter" => Ok(Self::OpenRouter),
            other => Err(format!("暂不支持内置登录的提供商: {}", other)),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAiCodex => "openai-codex",
            Self::GithubCopilot => "github-copilot",
            Self::KimiCoding => "kimi-coding",
            Self::Xai => "xai",
            Self::OpenRouter => "openrouter",
        }
    }

    fn is_device_flow(self) -> bool {
        matches!(self, Self::GithubCopilot | Self::KimiCoding | Self::Xai)
    }

    /// Fixed loopback port; 0 means an ephemeral port (OpenRouter).
    fn port(self) -> u16 {
        match self {
            Self::Anthropic => ANTHROPIC_PORT,
            Self::OpenAiCodex => CODEX_PORT,
            _ => 0,
        }
    }

    fn callback_path(self) -> String {
        match self {
            Self::Anthropic => "/callback".to_string(),
            Self::OpenAiCodex => "/auth/callback".to_string(),
            _ => format!("/oauth/callback/{}", random_token(12)),
        }
    }
}

struct PendingLogin {
    login_id: String,
    provider: Provider,
    verifier: String,
    state: String,
    redirect_uri: String,
    expires_at: i64,
    code: Option<String>,
    error: Option<String>,
    /// Device-code flows only.
    device_code: Option<String>,
    interval_secs: u64,
}

static PENDING: Mutex<Option<PendingLogin>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiOAuthStartResponse {
    pub login_id: String,
    pub provider: String,
    pub auth_url: String,
    pub verification_uri: String,
    /// Device-code flows only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    pub expires_in: i64,
    pub interval_seconds: i64,
    /// Redirect URI; shown so the user can paste the final URL manually.
    /// Absent for device-code flows.
    pub callback_url: Option<String>,
    /// False when the callback port was busy; manual paste is then required.
    pub listening: bool,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn random_token(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn build_auth_url(provider: Provider, challenge: &str, state: &str, redirect_uri: &str) -> String {
    let (base, params): (&str, Vec<(&str, &str)>) = match provider {
        Provider::Anthropic => (
            ANTHROPIC_AUTHORIZE_URL,
            vec![
                ("code", "true"),
                ("client_id", ANTHROPIC_CLIENT_ID),
                ("response_type", "code"),
                ("redirect_uri", redirect_uri),
                ("scope", ANTHROPIC_SCOPES),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
                ("state", state),
            ],
        ),
        Provider::OpenAiCodex => (
            CODEX_AUTHORIZE_URL,
            vec![
                ("response_type", "code"),
                ("client_id", CODEX_CLIENT_ID),
                ("redirect_uri", redirect_uri),
                ("scope", CODEX_SCOPES),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
                ("state", state),
                ("id_token_add_organizations", "true"),
                ("codex_cli_simplified_flow", "true"),
                ("originator", "pi"),
            ],
        ),
        // OpenRouter returns only `code` (no state) to `callback_url`.
        _ => (
            OPENROUTER_AUTHORIZE_URL,
            vec![
                ("callback_url", redirect_uri),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
            ],
        ),
    };
    let mut url = url::Url::parse(base).expect("static authorize url");
    url.query_pairs_mut().extend_pairs(params);
    url.to_string()
}

/// Whether `login_id` is still the active, unresolved login.
fn is_waiting(login_id: &str) -> bool {
    let guard = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    matches!(
        guard.as_ref(),
        Some(p) if p.login_id == login_id && p.code.is_none() && p.error.is_none() && p.expires_at > now()
    )
}

/// Record a callback result for `login_id`. Returns a message for the browser.
fn resolve(
    login_id: &str,
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    let mut guard = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let pending = guard
        .as_mut()
        .filter(|p| p.login_id == login_id)
        .ok_or_else(|| "登录会话已失效，请重新开始".to_string())?;
    if let Some(error) = error {
        pending.error = Some(error.clone());
        return Err(error);
    }
    if let Some(state) = state.as_deref() {
        if state != pending.state {
            return Err("OAuth state 不匹配".to_string());
        }
    }
    let code = code
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| "缺少授权码".to_string())?;
    pending.code = Some(code.trim().to_string());
    Ok(())
}

fn callback_html(ok: bool, message: &str) -> String {
    let color = if ok { "#16a34a" } else { "#dc2626" };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Cockpit Tools · pi</title></head>\
         <body style=\"font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#0f172a;color:#e2e8f0\">\
         <div style=\"text-align:center\"><h2 style=\"color:{color}\">{message}</h2><p>可以关闭此页面并返回 Cockpit Tools。</p></div></body></html>"
    )
}

fn bind_callback_server(port: u16) -> Option<tiny_http::Server> {
    match tiny_http::Server::http(("127.0.0.1", port)) {
        Ok(server) => Some(server),
        Err(e) => {
            logger::log_warn(&format!(
                "[pi][OAuth] 回调端口 {} 被占用，改用手动粘贴: {}",
                port, e
            ));
            None
        }
    }
}

fn spawn_callback_server(server: tiny_http::Server, login_id: String, callback_path: String) {
    std::thread::spawn(move || {
        while is_waiting(&login_id) {
            let request = match server.recv_timeout(Duration::from_millis(200)) {
                Ok(Some(request)) => request,
                Ok(None) => continue,
                Err(_) => break,
            };
            let url = url::Url::parse(&format!("http://localhost{}", request.url())).ok();
            let Some(url) = url.filter(|u| u.path() == callback_path) else {
                let _ = request.respond(tiny_http::Response::empty(404));
                continue;
            };
            let query = |key: &str| {
                url.query_pairs()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.into_owned())
            };
            let result = resolve(&login_id, query("code"), query("state"), query("error"));
            let html = match &result {
                Ok(()) => callback_html(true, "授权成功"),
                Err(e) => callback_html(false, e),
            };
            let header = tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8")
                .expect("static header");
            let _ = request.respond(tiny_http::Response::from_string(html).with_header(header));
            if result.is_ok() {
                break;
            }
        }
    });
}

pub async fn start_login(provider: &str) -> Result<PiOAuthStartResponse, String> {
    let provider = Provider::parse(provider)?;
    cancel_login(None);
    if provider.is_device_flow() {
        return start_device_login(provider).await;
    }
    let login_id = random_token(16);
    let verifier = random_token(32);
    // pi uses the verifier as state for Anthropic; a separate random state is
    // equally valid and keeps the verifier out of the browser URL.
    let state = random_token(16);
    let callback_path = provider.callback_path();
    let server = bind_callback_server(provider.port());
    let listening = server.is_some();
    let port = match (&server, provider.port()) {
        (Some(server), 0) => server
            .server_addr()
            .to_ip()
            .map(|addr| addr.port())
            .ok_or_else(|| "无法获取回调端口".to_string())?,
        (None, 0) => return Err("无法启动本地回调服务".to_string()),
        (_, port) => port,
    };
    // OpenRouter's own flow uses 127.0.0.1; the others register localhost.
    let host = if provider == Provider::OpenRouter {
        "127.0.0.1"
    } else {
        "localhost"
    };
    let redirect_uri = format!("http://{}:{}{}", host, port, callback_path);
    let auth_url = build_auth_url(provider, &code_challenge(&verifier), &state, &redirect_uri);
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(PendingLogin {
        login_id: login_id.clone(),
        provider,
        verifier,
        state,
        redirect_uri: redirect_uri.clone(),
        expires_at: now() + LOGIN_TIMEOUT_SECS,
        code: None,
        error: None,
        device_code: None,
        interval_secs: 0,
    });
    if let Some(server) = server {
        spawn_callback_server(server, login_id.clone(), callback_path);
    }
    logger::log_info(&format!(
        "[pi][OAuth] 开始登录 provider={} listening={}",
        provider.id(),
        listening
    ));
    Ok(PiOAuthStartResponse {
        login_id,
        provider: provider.id().to_string(),
        verification_uri: auth_url.clone(),
        auth_url,
        user_code: None,
        expires_in: LOGIN_TIMEOUT_SECS,
        interval_seconds: 1,
        callback_url: Some(redirect_uri),
        listening,
    })
}

/// Only http(s) verification URIs are opened in the browser.
fn trusted_url(value: Option<&str>) -> Option<String> {
    let url = url::Url::parse(value?).ok()?;
    matches!(url.scheme(), "https" | "http").then(|| url.to_string())
}

/// POST a form and parse the JSON body regardless of status (device-flow
/// errors such as `authorization_pending` come back as 4xx JSON).
async fn post_form_json(
    client: &reqwest::Client,
    url: &str,
    fields: &[(&str, &str)],
    copilot: bool,
) -> Result<(u16, Value), String> {
    let mut request = client
        .post(url)
        .header("Accept", "application/json")
        .form(fields);
    if copilot {
        request = request.header("User-Agent", COPILOT_HEADERS[0].1);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;
    let body = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
    Ok((status, body))
}

async fn start_device_login(provider: Provider) -> Result<PiOAuthStartResponse, String> {
    let client = crate::utils::http::create_client(30);
    let (url, fields): (&str, Vec<(&str, &str)>) = match provider {
        Provider::GithubCopilot => (
            COPILOT_DEVICE_CODE_URL,
            vec![("client_id", COPILOT_CLIENT_ID), ("scope", "read:user")],
        ),
        Provider::KimiCoding => (KIMI_DEVICE_CODE_URL, vec![("client_id", KIMI_CLIENT_ID)]),
        _ => (
            XAI_DEVICE_CODE_URL,
            vec![
                ("client_id", XAI_CLIENT_ID),
                ("scope", XAI_SCOPE),
                ("referrer", "pi"),
            ],
        ),
    };
    let (status, body) =
        post_form_json(&client, url, &fields, provider == Provider::GithubCopilot).await?;
    let device_code = body.get("device_code").and_then(Value::as_str);
    let user_code = body.get("user_code").and_then(Value::as_str);
    let verification_uri = trusted_url(body.get("verification_uri").and_then(Value::as_str));
    let (Some(device_code), Some(user_code), Some(verification_uri)) =
        (device_code, user_code, verification_uri)
    else {
        return Err(format!("设备码请求失败 ({}): {}", status, body));
    };
    let complete_uri = trusted_url(
        body.get("verification_uri_complete")
            .and_then(Value::as_str),
    );
    let interval = body
        .get("interval")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .unwrap_or(5);
    let expires_in = body
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|v| *v > 0)
        .unwrap_or(900);
    let login_id = random_token(16);
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(PendingLogin {
        login_id: login_id.clone(),
        provider,
        verifier: String::new(),
        state: String::new(),
        redirect_uri: String::new(),
        expires_at: now() + expires_in,
        code: None,
        error: None,
        device_code: Some(device_code.to_string()),
        interval_secs: interval,
    });
    logger::log_info(&format!(
        "[pi][OAuth] 开始设备码登录 provider={}",
        provider.id()
    ));
    let open_uri = complete_uri.unwrap_or_else(|| verification_uri.clone());
    Ok(PiOAuthStartResponse {
        login_id,
        provider: provider.id().to_string(),
        auth_url: open_uri.clone(),
        verification_uri: open_uri,
        user_code: Some(user_code.to_string()),
        expires_in,
        interval_seconds: interval as i64,
        callback_url: None,
        listening: false,
    })
}

pub fn cancel_login(login_id: Option<&str>) {
    let mut guard = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if login_id.is_none() || guard.as_ref().map(|p| p.login_id.as_str()) == login_id {
        *guard = None;
    }
}

/// Parse a pasted redirect URL, `code#state`, query string, or bare code.
fn parse_manual_input(input: &str) -> (Option<String>, Option<String>) {
    let value = input.trim();
    if let Ok(url) = url::Url::parse(value) {
        let get = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        if get("code").is_some() {
            return (get("code"), get("state"));
        }
    }
    if let Some((code, state)) = value.split_once('#') {
        return (Some(code.to_string()), Some(state.to_string()));
    }
    if value.contains("code=") {
        let pairs: Vec<(String, String)> =
            url::form_urlencoded::parse(value.trim_start_matches('?').as_bytes())
                .into_owned()
                .collect();
        let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        return (get("code"), get("state"));
    }
    (Some(value.to_string()), None)
}

pub fn submit_callback(login_id: &str, input: &str) -> Result<(), String> {
    let (code, state) = parse_manual_input(input);
    resolve(login_id, code, state, None)
}

struct ReadyCode {
    provider: Provider,
    code: String,
    state: String,
    verifier: String,
    redirect_uri: String,
}

/// Wait until the callback (or a manual paste) delivers the code.
async fn wait_for_code(login_id: &str) -> Result<ReadyCode, String> {
    loop {
        {
            let mut guard = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            let pending = guard
                .as_ref()
                .filter(|p| p.login_id == login_id)
                .ok_or_else(|| "登录已取消".to_string())?;
            if let Some(error) = pending.error.clone() {
                *guard = None;
                return Err(format!("授权失败: {}", error));
            }
            if let Some(code) = pending.code.clone() {
                let ready = ReadyCode {
                    provider: pending.provider,
                    code,
                    state: pending.state.clone(),
                    verifier: pending.verifier.clone(),
                    redirect_uri: pending.redirect_uri.clone(),
                };
                *guard = None;
                return Ok(ready);
            }
            if pending.expires_at <= now() {
                *guard = None;
                return Err("登录超时，请重试".to_string());
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

async fn read_token_json(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("读取令牌响应失败: {}", e))?;
    if !status.is_success() {
        return Err(format!("令牌交换失败 ({}): {}", status.as_u16(), text));
    }
    serde_json::from_str(&text).map_err(|e| format!("令牌响应不是 JSON: {}", e))
}

fn token_fields(data: &Value) -> Result<(String, String, i64), String> {
    let access = data.get("access_token").and_then(Value::as_str);
    let refresh = data.get("refresh_token").and_then(Value::as_str);
    let expires_in = data.get("expires_in").and_then(Value::as_i64);
    match (access, refresh, expires_in) {
        (Some(a), Some(r), Some(e)) => Ok((a.to_string(), r.to_string(), e)),
        _ => Err("令牌响应缺少 access_token / refresh_token / expires_in".to_string()),
    }
}

fn jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Exchange the code and build the pi `auth.json` entry plus a display name.
async fn exchange(ready: &ReadyCode) -> Result<(Value, Option<String>), String> {
    let client = crate::utils::http::create_client(30);
    let now_ms = chrono::Utc::now().timestamp_millis();
    match ready.provider {
        Provider::Anthropic => {
            let response = client
                .post(ANTHROPIC_TOKEN_URL)
                .header("Accept", "application/json")
                .json(&json!({
                    "grant_type": "authorization_code",
                    "client_id": ANTHROPIC_CLIENT_ID,
                    "code": ready.code,
                    "state": ready.state,
                    "redirect_uri": ready.redirect_uri,
                    "code_verifier": ready.verifier,
                }))
                .send()
                .await
                .map_err(|e| format!("令牌交换请求失败: {}", e))?;
            let data = read_token_json(response).await?;
            let (access, refresh, expires_in) = token_fields(&data)?;
            let email = data
                .pointer("/account/email_address")
                .and_then(Value::as_str)
                .map(str::to_string);
            let entry = json!({
                "type": "oauth",
                "refresh": refresh,
                "access": access,
                // Same 5-minute safety margin pi applies.
                "expires": now_ms + expires_in * 1000 - 5 * 60 * 1000,
            });
            Ok((entry, email))
        }
        Provider::OpenAiCodex => {
            let response = client
                .post(CODEX_TOKEN_URL)
                .form(&[
                    ("grant_type", "authorization_code"),
                    ("client_id", CODEX_CLIENT_ID),
                    ("code", ready.code.as_str()),
                    ("code_verifier", ready.verifier.as_str()),
                    ("redirect_uri", ready.redirect_uri.as_str()),
                ])
                .send()
                .await
                .map_err(|e| format!("令牌交换请求失败: {}", e))?;
            let data = read_token_json(response).await?;
            let (access, refresh, expires_in) = token_fields(&data)?;
            let account_id = jwt_payload(&access)
                .and_then(|p| {
                    p.get(CODEX_JWT_CLAIM)?
                        .get("chatgpt_account_id")?
                        .as_str()
                        .map(str::to_string)
                })
                .ok_or_else(|| "无法从令牌解析 ChatGPT accountId".to_string())?;
            let email = data
                .get("id_token")
                .and_then(Value::as_str)
                .and_then(jwt_payload)
                .and_then(|p| p.get("email")?.as_str().map(str::to_string));
            let entry = json!({
                "type": "oauth",
                "access": access,
                "refresh": refresh,
                "expires": now_ms + expires_in * 1000,
                "accountId": account_id,
            });
            Ok((entry, email))
        }
        _ => {
            let response = client
                .post(OPENROUTER_KEYS_URL)
                .header("Accept", "application/json")
                .json(&json!({
                    "code": ready.code,
                    "code_verifier": ready.verifier,
                    "code_challenge_method": "S256",
                }))
                .send()
                .await
                .map_err(|e| format!("令牌交换请求失败: {}", e))?;
            let data = read_token_json(response).await?;
            let key = data
                .get("key")
                .and_then(Value::as_str)
                .filter(|k| !k.is_empty())
                .ok_or_else(|| "OpenRouter 响应缺少 key".to_string())?;
            // OpenRouter issues a permanent API key; pi stores it the same way.
            let entry = json!({
                "type": "oauth",
                "access": key,
                "refresh": "",
                "expires": 9_007_199_254_740_991i64,
            });
            Ok((entry, None))
        }
    }
}

struct DeviceGrant {
    provider: Provider,
    device_code: String,
    interval_secs: u64,
    expires_at: i64,
}

fn take_device_grant(login_id: &str) -> Result<Option<DeviceGrant>, String> {
    let guard = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let pending = guard
        .as_ref()
        .filter(|p| p.login_id == login_id)
        .ok_or_else(|| "登录已取消".to_string())?;
    Ok(pending.device_code.clone().map(|device_code| DeviceGrant {
        provider: pending.provider,
        device_code,
        interval_secs: pending.interval_secs,
        expires_at: pending.expires_at,
    }))
}

/// RFC 8628 polling; returns the raw token response.
async fn poll_device_token(login_id: &str, grant: &DeviceGrant) -> Result<Value, String> {
    let client = crate::utils::http::create_client(30);
    let (url, client_id) = match grant.provider {
        Provider::GithubCopilot => (COPILOT_ACCESS_TOKEN_URL, COPILOT_CLIENT_ID),
        Provider::KimiCoding => (KIMI_TOKEN_URL, KIMI_CLIENT_ID),
        _ => (XAI_TOKEN_URL, XAI_CLIENT_ID),
    };
    let mut interval = grant.interval_secs.max(1);
    loop {
        // Sleep in short slices so cancellation is noticed quickly.
        let wake = std::time::Instant::now() + Duration::from_secs(interval);
        while std::time::Instant::now() < wake {
            if !is_waiting(login_id) {
                return Err("登录已取消".to_string());
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        if now() >= grant.expires_at {
            return Err("设备码已过期，请重试".to_string());
        }
        let (status, body) = post_form_json(
            &client,
            url,
            &[
                ("client_id", client_id),
                ("device_code", grant.device_code.as_str()),
                ("grant_type", DEVICE_GRANT),
            ],
            grant.provider == Provider::GithubCopilot,
        )
        .await?;
        if body.get("access_token").and_then(Value::as_str).is_some() {
            return Ok(body);
        }
        match body.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => {}
            Some("slow_down") => {
                interval = body
                    .get("interval")
                    .and_then(Value::as_u64)
                    .filter(|v| *v > 0)
                    .unwrap_or(interval + 5);
            }
            Some("access_denied") | Some("authorization_denied") => {
                return Err("用户拒绝了授权".to_string())
            }
            Some("expired_token") => return Err("设备码已过期，请重试".to_string()),
            _ => return Err(format!("设备码轮询失败 ({}): {}", status, body)),
        }
    }
}

/// Copilot model ids usable by this account; enables `unconfigured` ones.
async fn copilot_models(client: &reqwest::Client, token: &str, base_url: &str) -> Vec<String> {
    let mut request = client
        .get(format!("{}/models", base_url))
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .header("X-GitHub-Api-Version", COPILOT_API_VERSION);
    for (k, v) in COPILOT_HEADERS {
        request = request.header(k, v);
    }
    let Ok(response) = request.send().await else {
        return Vec::new();
    };
    let Ok(data) = response.json::<Value>().await else {
        return Vec::new();
    };
    let items = data
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut ids = Vec::new();
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        if item.pointer("/capabilities/supports/tool_calls") == Some(&Value::Bool(false)) {
            continue;
        }
        let picker = item.get("model_picker_enabled") == Some(&Value::Bool(true));
        let policy = item.pointer("/policy/state").and_then(Value::as_str);
        if !picker || policy == Some("disabled") {
            continue;
        }
        if policy == Some("unconfigured") {
            let mut enable = client
                .post(format!("{}/models/{}/policy", base_url, id))
                .header("Authorization", format!("Bearer {}", token))
                .header("openai-intent", "chat-policy")
                .header("x-interaction-type", "chat-policy")
                .json(&json!({ "state": "enabled" }));
            for (k, v) in COPILOT_HEADERS {
                enable = enable.header(k, v);
            }
            let _ = enable.send().await;
        }
        ids.push(id.to_string());
    }
    ids
}

fn copilot_base_url(token: &str) -> String {
    token
        .split(';')
        .find_map(|part| part.strip_prefix("proxy-ep="))
        .map(|host| match host.strip_prefix("proxy.") {
            Some(rest) => format!("https://api.{}", rest),
            None => format!("https://{}", host),
        })
        .unwrap_or_else(|| COPILOT_DEFAULT_BASE_URL.to_string())
}

async fn finish_device_login(
    grant: &DeviceGrant,
    data: Value,
) -> Result<(Value, Option<String>), String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let access = data
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    match grant.provider {
        Provider::GithubCopilot => {
            let client = crate::utils::http::create_client(30);
            let mut request = client
                .get(COPILOT_TOKEN_URL)
                .header("Accept", "application/json")
                .header("Authorization", format!("Bearer {}", access));
            for (k, v) in COPILOT_HEADERS {
                request = request.header(k, v);
            }
            let response = request
                .send()
                .await
                .map_err(|e| format!("获取 Copilot 令牌失败: {}", e))?;
            let token_data = read_token_json(response).await?;
            let token = token_data
                .get("token")
                .and_then(Value::as_str)
                .ok_or_else(|| "Copilot 令牌响应缺少 token".to_string())?;
            let expires_at = token_data
                .get("expires_at")
                .and_then(Value::as_i64)
                .ok_or_else(|| "Copilot 令牌响应缺少 expires_at".to_string())?;
            let models = copilot_models(&client, token, &copilot_base_url(token)).await;
            let mut user = client
                .get("https://api.github.com/user")
                .header("Accept", "application/json")
                .header("Authorization", format!("Bearer {}", access));
            user = user.header("User-Agent", COPILOT_HEADERS[0].1);
            let login = match user.send().await {
                Ok(r) => r.json::<Value>().await.ok().and_then(|u| {
                    u.get("email")
                        .and_then(Value::as_str)
                        .or_else(|| u.get("login").and_then(Value::as_str))
                        .map(str::to_string)
                }),
                Err(_) => None,
            };
            let mut entry = json!({
                "type": "oauth",
                // pi keeps the GitHub token as `refresh` and the Copilot token as `access`.
                "refresh": access,
                "access": token,
                "expires": expires_at * 1000 - REFRESH_SKEW_MS,
            });
            if !models.is_empty() {
                entry["availableModelIds"] = json!(models);
            }
            Ok((entry, login))
        }
        _ => {
            let refresh = data
                .get("refresh_token")
                .and_then(Value::as_str)
                .ok_or_else(|| "令牌响应缺少 refresh_token".to_string())?;
            let expires_in = data
                .get("expires_in")
                .and_then(Value::as_i64)
                .unwrap_or(3600);
            let skew = if grant.provider == Provider::Xai {
                REFRESH_SKEW_MS
            } else {
                0
            };
            let email = data
                .get("id_token")
                .and_then(Value::as_str)
                .and_then(jwt_payload)
                .and_then(|p| p.get("email")?.as_str().map(str::to_string));
            let entry = json!({
                "type": "oauth",
                "access": access,
                "refresh": refresh,
                "expires": now_ms + expires_in * 1000 - skew,
            });
            Ok((entry, email))
        }
    }
}

pub async fn complete_login(login_id: &str) -> Result<PiAccountView, String> {
    if let Some(grant) = take_device_grant(login_id)? {
        let result = poll_device_token(login_id, &grant).await;
        cancel_login(Some(login_id));
        let (entry, email) = finish_device_login(&grant, result?).await?;
        logger::log_info(&format!(
            "[pi][OAuth] 登录成功 provider={}",
            grant.provider.id()
        ));
        let display_name = email.unwrap_or_else(|| grant.provider.id().to_string());
        return pi_account::add_with_oauth(grant.provider.id(), entry, Some(display_name));
    }
    let ready = wait_for_code(login_id).await?;
    let (entry, email) = exchange(&ready).await?;
    logger::log_info(&format!(
        "[pi][OAuth] 登录成功 provider={}",
        ready.provider.id()
    ));
    let display_name = email.unwrap_or_else(|| ready.provider.id().to_string());
    pi_account::add_with_oauth(ready.provider.id(), entry, Some(display_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manual_callback_inputs() {
        assert_eq!(
            parse_manual_input("http://localhost:53692/callback?code=abc&state=xyz"),
            (Some("abc".into()), Some("xyz".into()))
        );
        assert_eq!(
            parse_manual_input("abc#xyz"),
            (Some("abc".into()), Some("xyz".into()))
        );
        assert_eq!(
            parse_manual_input("code=abc&state=xyz"),
            (Some("abc".into()), Some("xyz".into()))
        );
        assert_eq!(parse_manual_input(" abc "), (Some("abc".into()), None));
    }

    #[test]
    fn builds_pkce_auth_urls() {
        let url = build_auth_url(
            Provider::OpenAiCodex,
            "ch",
            "st",
            "http://localhost:1455/auth/callback",
        );
        assert!(url.starts_with(CODEX_AUTHORIZE_URL));
        assert!(url.contains("originator=pi"));
        assert!(url.contains("code_challenge=ch"));
        let url = build_auth_url(
            Provider::Anthropic,
            "ch",
            "st",
            "http://localhost:53692/callback",
        );
        assert!(url.contains("code=true"));
        assert_eq!(code_challenge("abc").len(), 43);
        let url = build_auth_url(
            Provider::OpenRouter,
            "ch",
            "",
            "http://127.0.0.1:5000/oauth/callback/x",
        );
        assert!(url.starts_with(OPENROUTER_AUTHORIZE_URL));
        assert!(url.contains("callback_url="));
    }

    #[test]
    fn derives_copilot_base_url() {
        assert_eq!(
            copilot_base_url("tid=1;exp=2;proxy-ep=proxy.individual.githubcopilot.com;x=y"),
            "https://api.individual.githubcopilot.com"
        );
        assert_eq!(copilot_base_url("tid=1"), COPILOT_DEFAULT_BASE_URL);
    }
}
