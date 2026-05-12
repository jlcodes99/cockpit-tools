//! Windsurf Devin Auth (2026-04+) 协议实现
//!
//! Windsurf 在 2026-04 把账号体系从 Firebase (sk-ws-XXX 格式) 迁移到了 Devin Auth
//! (auth1_XXX / devin-session-token$XXX 格式)。新注册的账号都属于 Devin 体系，
//! 而老的 Firebase 邮密登录端点对它们返回 EMAIL_NOT_FOUND。
//!
//! 本模块实现 Devin Auth 的协议级登录与刷新流程（2026-05 实测验证）。
//!
//! # 流程
//!
//! ```text
//! 邮箱+密码                               (login_with_password)
//!   ↓
//! POST /_devin-auth/password/login        → auth1_token (长期凭证)
//!   ↓
//! WindsurfPostAuth(auth1)                 → session_token + account_id + org_id
//!   ↓
//! GetOneTimeAuthToken(session)            → ott
//!   ↓
//! RegisterUser(ott)                       → ide_token (机器绑定，能发消息)
//!   ↓
//! GetCurrentUser(session)                 → user_status_proto (UI 显示)
//! ```
//!
//! 切号场景：只需要 `auth1_token`，调一次 `full_refresh_from_auth1` 拿全所需字段。

use base64::Engine;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

use crate::modules::logger;
use crate::utils::protobuf::{encode_string_field, read_varint, skip_field};

// ========== 端点 ==========

const PASSWORD_LOGIN_URL: &str = "https://windsurf.com/_devin-auth/password/login";
const CONNECTIONS_URL: &str = "https://windsurf.com/_devin-auth/connections";
const POST_AUTH_URL: &str =
    "https://windsurf.com/_backend/exa.seat_management_pb.SeatManagementService/WindsurfPostAuth";
const GET_OTT_URL: &str =
    "https://windsurf.com/_backend/exa.seat_management_pb.SeatManagementService/GetOneTimeAuthToken";
const REGISTER_USER_URL: &str =
    "https://register.windsurf.com/exa.seat_management_pb.SeatManagementService/RegisterUser";
const GET_CURRENT_USER_URL: &str =
    "https://windsurf.com/_backend/exa.seat_management_pb.SeatManagementService/GetCurrentUser";
/// GetUserStatus: Devin 号查配额/计划，必须用 self-serve 域 + Bearer ide_token
pub const GET_USER_STATUS_URL: &str =
    "https://server.self-serve.windsurf.com/exa.seat_management_pb.SeatManagementService/GetUserStatus";

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36";
const SEC_CH_UA: &str = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;

// ========== 公共数据结构 ==========

#[derive(Debug, Clone)]
pub struct DevinPasswordLoginResult {
    pub auth1_token: String,
    pub user_id: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DevinFullRefreshResult {
    pub ide_token: String,
    pub session_token: String,
    pub auth1_token: String,
    pub account_id: String,
    pub org_id: String,
    /// UserStatus protobuf 的 base64 编码（写入 windsurfAuthStatus.userStatusProtoBinaryBase64）
    pub user_status_proto_b64: Option<String>,
}

// ========== protobuf 辅助：解析所有 length-delimited string 字段 ==========

/// 从 proto 字节流里取出指定 field_num 的第一个 string 值。
/// 仅支持 wire type 0 (varint) 和 2 (length-delimited)，足够当前需求。
fn proto_get_first_string(data: &[u8], target_field: u32) -> Option<String> {
    let mut offset = 0usize;
    while offset < data.len() {
        let (tag, new_offset) = read_varint(data, offset).ok()?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;
        if wire_type == 2 {
            let (length, content_offset) = read_varint(data, new_offset).ok()?;
            let length = length as usize;
            if content_offset + length > data.len() {
                return None;
            }
            if field_num == target_field {
                return String::from_utf8(data[content_offset..content_offset + length].to_vec())
                    .ok();
            }
            offset = content_offset + length;
        } else {
            offset = skip_field(data, new_offset, wire_type).ok()?;
        }
    }
    None
}

// ========== HTTP client ==========

fn build_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("构建 HTTP client 失败: {}", e))
}

// ========== 1. 邮密登录: /_devin-auth/password/login ==========

#[derive(Debug, Deserialize)]
struct PasswordLoginResponse {
    token: Option<String>,
    user_id: Option<String>,
    email: Option<String>,
}

/// 协议级邮密登录（无 OTP、无浏览器）。返回 auth1_token。
pub async fn login_with_password(
    email: &str,
    password: &str,
) -> Result<DevinPasswordLoginResult, String> {
    let email = email.trim();
    if email.is_empty() || password.is_empty() {
        return Err("邮箱和密码不能为空".to_string());
    }

    logger::log_info(&format!(
        "[Windsurf Devin] /_devin-auth/password/login (email={})",
        email
    ));

    let client = build_client()?;
    let resp = client
        .post(PASSWORD_LOGIN_URL)
        .header("Accept", "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Content-Type", "application/json")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/account/login")
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-origin")
        .header("sec-ch-ua", SEC_CH_UA)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", "\"Windows\"")
        .json(&json!({
            "email": email,
            "password": password,
            "product": "Windsurf",
        }))
        .send()
        .await
        .map_err(|e| format!("Devin 邮密登录请求失败: {}", e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .unwrap_or_else(|_| "<no-body>".to_string());

    if !status.is_success() {
        let friendly = if status.as_u16() == 401 || status.as_u16() == 403 {
            "邮箱或密码错误".to_string()
        } else if status.as_u16() == 404 {
            "邮箱未注册".to_string()
        } else if status.as_u16() == 429 {
            "请求过于频繁，请稍后再试".to_string()
        } else {
            format!(
                "Devin 登录失败 (HTTP {}): {}",
                status.as_u16(),
                truncate(&text, 200)
            )
        };
        return Err(friendly);
    }

    let parsed: PasswordLoginResponse = serde_json::from_str(&text).map_err(|e| {
        format!(
            "解析 Devin 登录响应失败: {} (body: {})",
            e,
            truncate(&text, 200)
        )
    })?;

    let auth1 = parsed
        .token
        .filter(|t| t.starts_with("auth1_"))
        .ok_or_else(|| format!("Devin 登录响应未含 auth1_token: {}", truncate(&text, 200)))?;

    Ok(DevinPasswordLoginResult {
        auth1_token: auth1,
        user_id: parsed.user_id,
        email: parsed.email,
    })
}

// ========== 2. WindsurfPostAuth: auth1 → session + account + org ==========

async fn windsurf_post_auth(
    client: &Client,
    auth1_token: &str,
) -> Result<(String, String, String, String), String> {
    let body = encode_string_field(1, auth1_token);

    let mut last_err = String::new();
    for attempt in 1..=3u32 {
        logger::log_info(&format!(
            "[Windsurf Devin] WindsurfPostAuth (attempt {}/3)",
            attempt
        ));
        let resp = client
            .post(POST_AUTH_URL)
            .header("Content-Type", "application/proto")
            .header("connect-protocol-version", "1")
            .header("Accept", "application/proto")
            .header("Origin", "https://windsurf.com")
            .header("Referer", "https://windsurf.com/account/login")
            // 关键: 服务端 2026-05 起强制要求此 header
            .header("x-devin-auth1-token", auth1_token)
            .body(body.clone())
            .send()
            .await;
        match resp {
            Ok(r) => {
                let status = r.status();
                if !status.is_success() {
                    let body_text = r.text().await.unwrap_or_default();
                    last_err = format!("HTTP {}: {}", status.as_u16(), truncate(&body_text, 200));
                    if attempt < 3 {
                        // 4xx 大概率 auth1 未就绪 (服务端索引需要 1-2 秒)，等更久
                        let wait_ms = if status.as_u16() >= 400 && status.as_u16() < 500 {
                            (3000 * attempt) as u64
                        } else {
                            (2000 * attempt) as u64
                        };
                        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                        continue;
                    }
                    return Err(format!("WindsurfPostAuth 失败: {}", last_err));
                }
                let raw = r
                    .bytes()
                    .await
                    .map_err(|e| format!("读取响应失败: {}", e))?;
                let session_token = proto_get_first_string(&raw, 1)
                    .filter(|s| s.starts_with("devin-session-token$"))
                    .ok_or_else(|| "WindsurfPostAuth 响应未含 session_token".to_string())?;
                let auth1_back =
                    proto_get_first_string(&raw, 3).unwrap_or_else(|| auth1_token.to_string());
                let account_id = proto_get_first_string(&raw, 4)
                    .ok_or_else(|| "WindsurfPostAuth 响应未含 account_id".to_string())?;
                let org_id = proto_get_first_string(&raw, 5)
                    .ok_or_else(|| "WindsurfPostAuth 响应未含 org_id".to_string())?;
                return Ok((session_token, auth1_back, account_id, org_id));
            }
            Err(e) => {
                last_err = format!("请求异常: {}", e);
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_millis(2000)).await;
                    continue;
                }
                return Err(format!("WindsurfPostAuth 失败: {}", last_err));
            }
        }
    }
    Err(format!("WindsurfPostAuth 重试 3 次仍失败: {}", last_err))
}

// ========== 3. GetOneTimeAuthToken: session → ott ==========

async fn get_one_time_auth_token(
    client: &Client,
    session_token: &str,
    auth1: &str,
    account_id: &str,
    org_id: &str,
) -> Result<String, String> {
    let body = encode_string_field(1, session_token);

    let mut last_err = String::new();
    for attempt in 1..=3u32 {
        let resp = client
            .post(GET_OTT_URL)
            .header("Content-Type", "application/proto")
            .header("connect-protocol-version", "1")
            .header("Accept", "application/proto")
            .header("Origin", "https://windsurf.com")
            .header("Referer", "https://windsurf.com/editor/auth-success")
            .header("x-auth-token", session_token)
            .header("x-devin-session-token", session_token)
            .header("x-devin-account-id", account_id)
            .header("x-devin-primary-org-id", org_id)
            .header("x-devin-auth1-token", auth1)
            .body(body.clone())
            .send()
            .await;
        match resp {
            Ok(r) => {
                let status = r.status();
                if !status.is_success() {
                    let body_text = r.text().await.unwrap_or_default();
                    last_err = format!("HTTP {}: {}", status.as_u16(), truncate(&body_text, 200));
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_millis((2000 * attempt) as u64)).await;
                        continue;
                    }
                    return Err(format!("GetOneTimeAuthToken 失败: {}", last_err));
                }
                let raw = r
                    .bytes()
                    .await
                    .map_err(|e| format!("读取响应失败: {}", e))?;
                let ott = proto_get_first_string(&raw, 1)
                    .filter(|s| s.starts_with("ott$"))
                    .ok_or_else(|| "GetOTT 响应未含 ott".to_string())?;
                return Ok(ott);
            }
            Err(e) => {
                last_err = format!("请求异常: {}", e);
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_millis(2000)).await;
                    continue;
                }
                return Err(format!("GetOneTimeAuthToken 失败: {}", last_err));
            }
        }
    }
    Err(format!("GetOneTimeAuthToken 重试 3 次仍失败: {}", last_err))
}

// ========== 4. RegisterUser: ott → ide_token ==========

async fn register_user(client: &Client, ott: &str) -> Result<String, String> {
    let body = encode_string_field(1, ott);

    let mut last_err = String::new();
    for attempt in 1..=3u32 {
        let resp = client
            .post(REGISTER_USER_URL)
            .header("Content-Type", "application/proto")
            .header("connect-protocol-version", "1")
            // 关键：UA 必须用 IDE 真实使用的 Go connect 客户端版本。
            // 服务端在 RegisterUser 时把 ide_token 与 UA 绑定，后续 IDE 发消息
            // UA 不匹配会被服务端 deny (error 12 — 能登录但不能对话)。
            // 不要带 x-devin-* headers，register.windsurf.com 不识别。
            .header("User-Agent", "connect-go/1.18.1 (go1.26.1)")
            .body(body.clone())
            .send()
            .await;
        match resp {
            Ok(r) => {
                let status = r.status();
                if !status.is_success() {
                    let body_text = r.text().await.unwrap_or_default();
                    last_err = format!("HTTP {}: {}", status.as_u16(), truncate(&body_text, 200));
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_millis((2000 * attempt) as u64)).await;
                        continue;
                    }
                    return Err(format!("RegisterUser 失败: {}", last_err));
                }
                let raw = r
                    .bytes()
                    .await
                    .map_err(|e| format!("读取响应失败: {}", e))?;
                // 找第一个 "devin-session-token$" 开头的 ASCII 字符串
                let needle = b"devin-session-token$";
                if let Some(idx) = find_subslice(&raw, needle) {
                    let mut end = idx;
                    while end < raw.len() && (0x20..0x7F).contains(&raw[end]) {
                        end += 1;
                    }
                    let ide_token = String::from_utf8(raw[idx..end].to_vec())
                        .map_err(|e| format!("ide_token 不是合法 UTF-8: {}", e))?;
                    return Ok(ide_token);
                }
                last_err = "RegisterUser 响应未含 ide_token".to_string();
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    continue;
                }
                return Err(last_err);
            }
            Err(e) => {
                last_err = format!("请求异常: {}", e);
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_millis(2000)).await;
                    continue;
                }
                return Err(format!("RegisterUser 失败: {}", last_err));
            }
        }
    }
    Err(format!("RegisterUser 重试 3 次仍失败: {}", last_err))
}

// ========== 5. GetCurrentUser: session → UserStatusProto ==========

async fn get_current_user_proto(
    client: &Client,
    session_token: &str,
    auth1: &str,
    account_id: &str,
    org_id: &str,
) -> Result<Vec<u8>, String> {
    // body = proto field 1 (string session) + 0x10 0x01 0x20 0x01 (field 2/4 varint=1)
    let mut body = encode_string_field(1, session_token);
    body.extend_from_slice(&[0x10, 0x01, 0x20, 0x01]);

    let resp = client
        .post(GET_CURRENT_USER_URL)
        .header("Content-Type", "application/proto")
        .header("connect-protocol-version", "1")
        .header("Accept", "application/proto")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/")
        .header("x-auth-token", session_token)
        .header("x-devin-session-token", session_token)
        .header("x-devin-account-id", account_id)
        .header("x-devin-primary-org-id", org_id)
        .header("x-devin-auth1-token", auth1)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("GetCurrentUser 请求失败: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "GetCurrentUser HTTP {}: {}",
            status.as_u16(),
            truncate(&text, 200)
        ));
    }
    let raw = resp
        .bytes()
        .await
        .map_err(|e| format!("读取 GetCurrentUser 响应失败: {}", e))?;
    Ok(raw.to_vec())
}

// ========== 组合：完整刷新 ==========

/// 用 auth1_token 跑完整 4 步链路，拿到 IDE 切号注入需要的所有字段
///
/// 任何一步失败都返回 Err（GetCurrentUser 除外，它失败不致命），
/// 避免产出"能登录但不能对话"的脏号。
pub async fn full_refresh_from_auth1(auth1_token: &str) -> Result<DevinFullRefreshResult, String> {
    let auth1 = auth1_token.trim();
    if !auth1.starts_with("auth1_") {
        return Err(format!(
            "auth1_token 格式错误，应以 'auth1_' 开头，实际: {}",
            truncate(auth1, 30)
        ));
    }

    let client = build_client()?;

    // Step 1: WindsurfPostAuth
    let (session_token, auth1_back, account_id, org_id) =
        windsurf_post_auth(&client, auth1).await?;
    logger::log_info(&format!(
        "[Windsurf Devin] PostAuth ok: account={}, org={}",
        account_id, org_id
    ));

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Step 2: GetOneTimeAuthToken
    let ott =
        get_one_time_auth_token(&client, &session_token, &auth1_back, &account_id, &org_id).await?;
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Step 3: RegisterUser → ide_token (机器绑定)
    let ide_token = register_user(&client, &ott).await?;
    logger::log_info(&format!(
        "[Windsurf Devin] RegisterUser ok: ide_token len={}",
        ide_token.len()
    ));

    // Step 4: GetCurrentUser → UserStatusProto（失败不致命）
    // 注意：必须用 RegisterUser 产出的 ide_token（机器绑定）调用，
    // 用 session_token 调可能会拿到旧/错的 UserStatus。
    tokio::time::sleep(Duration::from_millis(500)).await;
    let user_status_proto_b64 = match get_current_user_proto(
        &client,
        &ide_token,
        &auth1_back,
        &account_id,
        &org_id,
    )
    .await
    {
        Ok(bytes) => Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        Err(err) => {
            logger::log_warn(&format!(
                "[Windsurf Devin] GetCurrentUser 失败（不致命）: {}",
                err
            ));
            None
        }
    };

    Ok(DevinFullRefreshResult {
        ide_token,
        session_token,
        auth1_token: auth1_back,
        account_id,
        org_id,
        user_status_proto_b64,
    })
}

// ========== GetUserStatus: 查 Devin 账号的计划/配额 ==========

/// 用 ide_token 查询当前账号的 plan + 配额
///
/// 端点是 server.self-serve.windsurf.com（Devin 号专用），鉴权用 `Authorization: Bearer <ide_token>`。
/// 返回 raw JSON Value，调用方按需提取 userStatus / planStatus / planInfo 等字段。
///
/// # 关键响应字段（实测）
///
/// ```text
/// userStatus.planStatus.availablePromptCredits  // 剩余 (× 100)
/// userStatus.planStatus.availableFlowCredits    // flow 剩余 (× 100)
/// userStatus.planStatus.usedPromptCredits       // 已用 (× 100)
/// userStatus.planStatus.planEnd                 // 计划结束时间
/// userStatus.planStatus.dailyRemainingPercent / dailyQuotaRemainingPercent
/// userStatus.planStatus.weeklyRemainingPercent / weeklyQuotaRemainingPercent
/// userStatus.planStatus.dailyResetAtUnix / dailyQuotaResetAtUnix
/// userStatus.planStatus.weeklyResetAtUnix / weeklyQuotaResetAtUnix
/// userStatus.userUsedPromptCredits              // 备用已用字段
/// planInfo.planName                             // FREE | PRO | TEAM | ENTERPRISE
/// planInfo.monthlyPromptCredits                 // 月度总额度
/// planInfo.monthlyFlowCredits                   // 月度 flow 总额度
/// ```
pub async fn fetch_devin_user_status(ide_token: &str) -> Result<serde_json::Value, String> {
    let token = ide_token.trim();
    if token.is_empty() {
        return Err("ide_token 不能为空".to_string());
    }
    let client = build_client()?;
    let body = json!({
        "metadata": {
            "ide_name": "WINDSURF",
            "ide_version": "1.0.0",
            "extension_version": "1.0.0",
            "api_key": token,
        }
    });
    let resp = client
        .post(GET_USER_STATUS_URL)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("Authorization", format!("Bearer {}", token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("GetUserStatus 请求失败: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "GetUserStatus HTTP {}: {}",
            status.as_u16(),
            truncate(&text, 200)
        ));
    }
    serde_json::from_str::<serde_json::Value>(&text).map_err(|e| {
        format!(
            "解析 GetUserStatus 响应失败: {} (body_len={})",
            e,
            text.len()
        )
    })
}

// ========== 辅助：查询邮箱可用 auth_method（诊断用） ==========

#[derive(Debug, Deserialize)]
struct ConnectionsAuthMethod {
    method: Option<String>,
    has_password: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ConnectionsResponse {
    auth_method: Option<ConnectionsAuthMethod>,
}

#[derive(Debug, Clone)]
pub struct ConnectionsInfo {
    pub method: String,
    pub has_password: bool,
}

/// 调 /_devin-auth/connections 询问服务端：这个邮箱可用哪种 auth_method
pub async fn query_connections(email: &str) -> Result<ConnectionsInfo, String> {
    let email = email.trim();
    if email.is_empty() {
        return Err("邮箱不能为空".to_string());
    }
    let client = build_client()?;
    let resp = client
        .post(CONNECTIONS_URL)
        .header("Content-Type", "application/json")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/account/login")
        .json(&json!({"product": "windsurf", "email": email}))
        .send()
        .await
        .map_err(|e| format!("查询 connections 失败: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "connections HTTP {}: {}",
            status.as_u16(),
            truncate(&body, 200)
        ));
    }
    let parsed: ConnectionsResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析 connections 响应失败: {}", e))?;
    let am = parsed.auth_method.unwrap_or(ConnectionsAuthMethod {
        method: Some("email".to_string()),
        has_password: Some(false),
    });
    Ok(ConnectionsInfo {
        method: am.method.unwrap_or_else(|| "email".to_string()),
        has_password: am.has_password.unwrap_or(false),
    })
}

// ========== 工具 ==========

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...({} more chars)", &s[..max], s.len() - max)
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
