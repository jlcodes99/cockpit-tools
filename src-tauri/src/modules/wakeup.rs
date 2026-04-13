use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use tokio::sync::watch;

use crate::modules;

const CLOUD_CODE_BASE_URLS: [&str; 3] = [
    "https://daily-cloudcode-pa.googleapis.com",
    "https://cloudcode-pa.googleapis.com",
    "https://daily-cloudcode-pa.sandbox.googleapis.com",
];
const STREAM_PATH: &str = "/v1internal:streamGenerateContent?alt=sse";
const FETCH_MODELS_PATH: &str = "/v1internal:fetchAvailableModels";
const USER_AGENT: &str = "antigravity";
const ANTIGRAVITY_SYSTEM_PROMPT: &str = "You are Antigravity, a powerful agentic AI coding assistant designed by the Google Deepmind team working on Advanced Agentic Coding.You are pair programming with a USER to solve their coding task. The task may require creating a new codebase, modifying or debugging an existing codebase, or simply answering a question.**Absolute paths only****Proactiveness**";
const DEFAULT_ATTEMPTS: usize = 2;
const BACKOFF_BASE_MS: u64 = 500;
const BACKOFF_MAX_MS: u64 = 4000;
const WAKEUP_ERROR_JSON_PREFIX: &str = "AG_WAKEUP_ERROR_JSON:";
const CLIENT_GATEWAY_POLL_INTERVAL_MS: u64 = 250;
const CLIENT_GATEWAY_MAX_POLL_ROUNDS: usize = 240; // 240 * 250ms = 60s
const CLIENT_GATEWAY_UPSTREAM_RETRY_DELAY_MS: u64 = 1200;
const WAKEUP_CANCELLED_MESSAGE: &str = "唤醒测试已取消";
static BASE_URL_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static WAKEUP_CANCEL_SCOPES: OnceLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeupResponse {
    pub reply: String,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub trace_id: Option<String>,
    pub response_id: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug)]
struct StreamParseResult {
    reply: String,
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
    trace_id: Option<String>,
    response_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WakeupUiErrorPayload {
    version: u8,
    kind: String,
    message: String,
    error_code: Option<i64>,
    validation_url: Option<String>,
    appeal_url: Option<String>,
    trajectory_id: Option<String>,
    error_message_json: Option<String>,
    step_json: Option<String>,
}

fn random_suffix(len: usize) -> String {
    let charset: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| charset[rng.gen_range(0..charset.len())] as char)
        .collect()
}

fn wakeup_cancel_scopes() -> &'static Mutex<HashMap<String, watch::Sender<bool>>> {
    WAKEUP_CANCEL_SCOPES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_wakeup_cancel_receiver(
    cancel_scope_id: Option<&str>,
) -> Result<Option<watch::Receiver<bool>>, String> {
    let Some(cancel_scope_id) = cancel_scope_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let mut scopes = wakeup_cancel_scopes()
        .lock()
        .map_err(|_| "唤醒取消状态锁已损坏".to_string())?;

    let sender = scopes
        .entry(cancel_scope_id.to_string())
        .or_insert_with(|| {
            let (sender, _) = watch::channel(false);
            sender
        })
        .clone();

    Ok(Some(sender.subscribe()))
}

fn wakeup_cancelled_error() -> String {
    WAKEUP_CANCELLED_MESSAGE.to_string()
}

fn is_wakeup_cancelled(cancel_rx: Option<&watch::Receiver<bool>>) -> bool {
    cancel_rx
        .map(|receiver| *receiver.borrow())
        .unwrap_or(false)
}

fn is_wakeup_cancelled_message(message: &str) -> bool {
    message.trim() == WAKEUP_CANCELLED_MESSAGE
}

async fn await_with_cancel<T, F>(
    cancel_rx: Option<&watch::Receiver<bool>>,
    future: F,
) -> Result<T, String>
where
    F: Future<Output = T>,
{
    if let Some(cancel_rx) = cancel_rx {
        if *cancel_rx.borrow() {
            return Err(wakeup_cancelled_error());
        }

        let mut cancel_rx = cancel_rx.clone();
        tokio::select! {
            result = future => Ok(result),
            changed = cancel_rx.changed() => {
                match changed {
                    Ok(_) if *cancel_rx.borrow() => Err(wakeup_cancelled_error()),
                    Ok(_) => Err(wakeup_cancelled_error()),
                    Err(_) => Err(wakeup_cancelled_error()),
                }
            }
        }
    } else {
        Ok(future.await)
    }
}

async fn sleep_with_cancel(
    duration: std::time::Duration,
    cancel_rx: Option<&watch::Receiver<bool>>,
) -> Result<(), String> {
    await_with_cancel(cancel_rx, tokio::time::sleep(duration)).await?;
    Ok(())
}

pub fn cancel_wakeup_scope(cancel_scope_id: &str) -> Result<(), String> {
    let cancel_scope_id = cancel_scope_id.trim();
    if cancel_scope_id.is_empty() {
        return Ok(());
    }

    let sender = {
        let mut scopes = wakeup_cancel_scopes()
            .lock()
            .map_err(|_| "唤醒取消状态锁已损坏".to_string())?;
        scopes.remove(cancel_scope_id)
    };

    if let Some(sender) = sender {
        let _ = sender.send(true);
    }

    Ok(())
}

pub fn release_wakeup_scope(cancel_scope_id: &str) -> Result<(), String> {
    let cancel_scope_id = cancel_scope_id.trim();
    if cancel_scope_id.is_empty() {
        return Ok(());
    }

    let mut scopes = wakeup_cancel_scopes()
        .lock()
        .map_err(|_| "唤醒取消状态锁已损坏".to_string())?;
    scopes.remove(cancel_scope_id);
    Ok(())
}

fn format_prompt_for_log(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }
    const MAX_LEN: usize = 60;
    let mut preview = trimmed.chars().take(MAX_LEN).collect::<String>();
    if trimmed.chars().count() > MAX_LEN {
        preview.push_str("...");
    }
    preview
}

fn generate_session_id() -> String {
    let timestamp = chrono::Utc::now().timestamp_millis();
    format!("sess_{}_{}", timestamp, random_suffix(6))
}

fn generate_request_id() -> String {
    let timestamp = chrono::Utc::now().timestamp_millis();
    format!("req_{}_{}", timestamp, random_suffix(6))
}

fn build_request_body(
    project_id: &str,
    model: &str,
    prompt: &str,
    max_output_tokens: u32,
) -> serde_json::Value {
    let request_id = generate_request_id();
    let session_id = generate_session_id();
    let mut generation_config = json!({ "temperature": 0 });
    if max_output_tokens > 0 {
        if let Some(obj) = generation_config.as_object_mut() {
            obj.insert("maxOutputTokens".to_string(), json!(max_output_tokens));
        }
    }

    json!({
        "project": project_id,
        "requestId": request_id,
        "model": model,
        "userAgent": "antigravity",
        "requestType": "agent",
        "request": {
            "contents": [
                { "role": "user", "parts": [ { "text": prompt } ] }
            ],
            "session_id": session_id,
            "systemInstruction": {
                "parts": [ { "text": ANTIGRAVITY_SYSTEM_PROMPT } ]
            },
            "generationConfig": generation_config
        }
    })
}

fn get_backoff_delay_ms(attempt: usize) -> u64 {
    if attempt < 2 {
        return 0;
    }
    let raw = BACKOFF_BASE_MS.saturating_mul(2u64.saturating_pow((attempt - 2) as u32));
    let jitter = rand::thread_rng().gen_range(0..100);
    std::cmp::min(raw + jitter, BACKOFF_MAX_MS)
}

fn get_base_url_order() -> Vec<&'static str> {
    let lock = BASE_URL_ORDER.get_or_init(|| Mutex::new(CLOUD_CODE_BASE_URLS.to_vec()));
    match lock.lock() {
        Ok(list) => list.clone(),
        Err(_) => CLOUD_CODE_BASE_URLS.to_vec(),
    }
}

fn promote_base_url(base: &'static str) {
    let lock = BASE_URL_ORDER.get_or_init(|| Mutex::new(CLOUD_CODE_BASE_URLS.to_vec()));
    if let Ok(mut list) = lock.lock() {
        if let Some(pos) = list.iter().position(|item| *item == base) {
            list.remove(pos);
            list.insert(0, base);
        }
    }
}

fn truncate_log_text(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }
    let mut preview = text.chars().take(max_len).collect::<String>();
    preview.push_str("...");
    preview
}

fn summarize_body_len(text: &str) -> usize {
    text.len()
}

fn process_stream_object(
    obj: &serde_json::Value,
    reply_parts: &mut Vec<String>,
    prompt_tokens: &mut Option<u32>,
    completion_tokens: &mut Option<u32>,
    total_tokens: &mut Option<u32>,
    trace_id: &mut Option<String>,
    response_id: &mut Option<String>,
) {
    let candidate = obj
        .get("response")
        .and_then(|value| value.get("candidates"))
        .and_then(|value| value.get(0))
        .or_else(|| obj.get("candidates").and_then(|value| value.get(0)));

    if let Some(parts) = candidate
        .and_then(|value| value.get("content"))
        .and_then(|value| value.get("parts"))
        .and_then(|value| value.as_array())
    {
        for part in parts {
            if part.get("thought").and_then(|value| value.as_bool()) == Some(true) {
                continue;
            }
            if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                if !text.is_empty() {
                    reply_parts.push(text.to_string());
                }
            }
        }
    }

    if prompt_tokens.is_none() || completion_tokens.is_none() || total_tokens.is_none() {
        let usage = obj
            .get("response")
            .and_then(|value| value.get("usageMetadata"))
            .or_else(|| obj.get("usageMetadata"));
        if let Some(usage) = usage {
            if prompt_tokens.is_none() {
                *prompt_tokens = usage
                    .get("promptTokenCount")
                    .and_then(|value| value.as_u64())
                    .map(|value| value as u32);
            }
            if completion_tokens.is_none() {
                *completion_tokens = usage
                    .get("candidatesTokenCount")
                    .and_then(|value| value.as_u64())
                    .map(|value| value as u32);
            }
            if total_tokens.is_none() {
                *total_tokens = usage
                    .get("totalTokenCount")
                    .and_then(|value| value.as_u64())
                    .map(|value| value as u32);
            }
        }
    }

    if trace_id.is_none() {
        *trace_id = obj
            .get("traceId")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
    }
    if response_id.is_none() {
        *response_id = obj
            .get("response")
            .and_then(|value| value.get("responseId"))
            .or_else(|| obj.get("responseId"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
    }
}

fn parse_stream_result(text: &str) -> Result<StreamParseResult, String> {
    let mut reply_parts: Vec<String> = Vec::new();
    let mut prompt_tokens: Option<u32> = None;
    let mut completion_tokens: Option<u32> = None;
    let mut total_tokens: Option<u32> = None;
    let mut trace_id: Option<String> = None;
    let mut response_id: Option<String> = None;
    let mut got_event = false;
    let mut last_data: Option<serde_json::Value> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let payload = if trimmed.starts_with("data:") {
            let payload = trimmed.trim_start_matches("data:").trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            Some(payload)
        } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
            Some(trimmed)
        } else {
            None
        };

        if let Some(payload) = payload {
            got_event = true;
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
                process_stream_object(
                    &value,
                    &mut reply_parts,
                    &mut prompt_tokens,
                    &mut completion_tokens,
                    &mut total_tokens,
                    &mut trace_id,
                    &mut response_id,
                );
                last_data = Some(value);
            }
        }
    }

    if !got_event {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            got_event = true;
            process_stream_object(
                &value,
                &mut reply_parts,
                &mut prompt_tokens,
                &mut completion_tokens,
                &mut total_tokens,
                &mut trace_id,
                &mut response_id,
            );
        }
    }

    if !got_event {
        return Err("Cloud Code stream received no data".to_string());
    }

    if reply_parts.is_empty() {
        if let Some(value) = last_data.as_ref() {
            process_stream_object(
                value,
                &mut reply_parts,
                &mut prompt_tokens,
                &mut completion_tokens,
                &mut total_tokens,
                &mut trace_id,
                &mut response_id,
            );
        }
    }

    let reply = if reply_parts.is_empty() {
        "(无回复)".to_string()
    } else {
        reply_parts.join("")
    };
    if completion_tokens.is_none() {
        completion_tokens = Some(0);
    }

    Ok(StreamParseResult {
        reply,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        trace_id,
        response_id,
    })
}

async fn send_stream_request(
    client: &reqwest::Client,
    access_token: &str,
    body: &serde_json::Value,
    cancel_rx: Option<&watch::Receiver<bool>>,
) -> Result<StreamParseResult, String> {
    let mut last_error: Option<String> = None;
    for base in get_base_url_order() {
        for attempt in 1..=DEFAULT_ATTEMPTS {
            if is_wakeup_cancelled(cancel_rx) {
                return Err(wakeup_cancelled_error());
            }
            let url = format!("{}{}", base, STREAM_PATH);
            crate::modules::logger::log_info(&format!(
                "[Wakeup] 发送请求: url={}, attempt={}/{}",
                url, attempt, DEFAULT_ATTEMPTS
            ));
            let response = await_with_cancel(
                cancel_rx,
                client
                    .post(&url)
                    .bearer_auth(access_token)
                    .header(reqwest::header::USER_AGENT, USER_AGENT)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .header(reqwest::header::ACCEPT_ENCODING, "gzip")
                    .json(body)
                    .send(),
            )
            .await?;

            match response {
                Ok(res) => {
                    let status = res.status();
                    if status.is_success() {
                        let text = await_with_cancel(cancel_rx, res.text())
                            .await?
                            .unwrap_or_default();
                        crate::modules::logger::log_info(&format!(
                            "[Wakeup] stream响应已接收: body_len={}",
                            summarize_body_len(&text)
                        ));
                        match parse_stream_result(&text) {
                            Ok(parsed) => {
                                promote_base_url(base);
                                crate::modules::logger::log_info(&format!(
                                    "[Wakeup] 请求成功: url={}, status={}",
                                    url, status
                                ));
                                return Ok(parsed);
                            }
                            Err(err) => {
                                last_error = Some(err.clone());
                                crate::modules::logger::log_warn(&format!(
                                    "[Wakeup] 解析响应失败: url={}, error={}",
                                    url, err
                                ));
                                if attempt < DEFAULT_ATTEMPTS {
                                    let delay = get_backoff_delay_ms(attempt + 1);
                                    if delay > 0 {
                                        crate::modules::logger::log_info(&format!(
                                            "[Wakeup] 准备重试: delay={}ms",
                                            delay
                                        ));
                                        sleep_with_cancel(
                                            std::time::Duration::from_millis(delay),
                                            cancel_rx,
                                        )
                                        .await?;
                                    }
                                    continue;
                                }
                            }
                        }
                    } else {
                        if status == reqwest::StatusCode::UNAUTHORIZED {
                            crate::modules::logger::log_error("[Wakeup] 授权失效 (401)");
                            return Err("Authorization expired".to_string());
                        }
                        if status == reqwest::StatusCode::FORBIDDEN {
                            crate::modules::logger::log_error("[Wakeup] 无权限 (403)");
                            return Err("Cloud Code access forbidden".to_string());
                        }
                        let text = await_with_cancel(cancel_rx, res.text())
                            .await?
                            .unwrap_or_default();
                        let retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                            || status.as_u16() >= 500;
                        let message = format!("唤醒请求失败: {} - {}", status, text);
                        last_error = Some(message.clone());
                        crate::modules::logger::log_warn(&format!(
                            "[Wakeup] 请求失败: url={}, status={}, retryable={}",
                            url, status, retryable
                        ));
                        if retryable && attempt < DEFAULT_ATTEMPTS {
                            let delay = get_backoff_delay_ms(attempt + 1);
                            if delay > 0 {
                                crate::modules::logger::log_info(&format!(
                                    "[Wakeup] 准备重试: delay={}ms",
                                    delay
                                ));
                                sleep_with_cancel(
                                    std::time::Duration::from_millis(delay),
                                    cancel_rx,
                                )
                                .await?;
                            }
                            continue;
                        }
                    }
                }
                Err(err) => {
                    last_error = Some(format!("唤醒请求失败: {}", err));
                    crate::modules::logger::log_warn(&format!(
                        "[Wakeup] 网络错误: url={}, error={}",
                        url, err
                    ));
                    if attempt < DEFAULT_ATTEMPTS {
                        let delay = get_backoff_delay_ms(attempt + 1);
                        if delay > 0 {
                            crate::modules::logger::log_info(&format!(
                                "[Wakeup] 准备重试: delay={}ms",
                                delay
                            ));
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        }
                        continue;
                    }
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "唤醒请求失败".to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WakeupTransportMode {
    LegacyCloudCode,
    ClientGateway,
}

fn resolve_wakeup_transport_mode() -> WakeupTransportMode {
    match std::env::var("AG_WAKEUP_TRANSPORT_MODE")
        .ok()
        .unwrap_or_else(|| "client_gateway".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "client_gateway" | "client-gateway" | "gateway" => WakeupTransportMode::ClientGateway,
        _ => WakeupTransportMode::LegacyCloudCode,
    }
}

pub fn wakeup_requires_official_ls() -> bool {
    matches!(
        resolve_wakeup_transport_mode(),
        WakeupTransportMode::ClientGateway
    )
}

pub fn set_official_ls_version_mode(mode: Option<&str>) -> Result<(), String> {
    crate::modules::wakeup_gateway::set_official_ls_version_mode(mode)
}

pub fn ensure_wakeup_runtime_ready() -> Result<Option<String>, String> {
    if !wakeup_requires_official_ls() {
        return Ok(None);
    }
    crate::modules::wakeup_gateway::ensure_official_ls_binary_ready().map(Some)
}

fn gateway_start_bind_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn build_gateway_client(base_url: &str, timeout_secs: u64) -> Result<reqwest::Client, String> {
    let mut builder =
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_secs));
    builder = builder.no_proxy();
    if base_url.starts_with("https://127.0.0.1:") || base_url.starts_with("https://localhost:") {
        // 本地自签名网关证书（协议形态对齐客户端，校验在此放宽）
        // lgtm[rs/disabled-certificate-check] 仅用于本地回环（127.0.0.1/localhost）自签名网关证书，非生产 TLS 校验放宽
        builder = builder
            .danger_accept_invalid_certs(true)
            // 回环地址使用 IP 时不发送 SNI，避免 rustls 告警 Illegal SNI extension。
            .tls_sni(false);
    }
    builder
        .build()
        .map_err(|e| format!("创建网关 HTTP 客户端失败: {}", e))
}

fn classify_gateway_transport_error(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        return "timeout";
    }

    let lower = err.to_string().to_lowercase();
    if lower.contains("tls")
        || lower.contains("ssl")
        || lower.contains("certificate")
        || lower.contains("handshake")
    {
        return "tls";
    }

    if err.is_connect() {
        return "connect";
    }

    "send"
}

fn is_local_gateway_recoverable_error_message(message: &str) -> bool {
    let lower = message.to_lowercase();
    if !(lower.contains("网关")
        && (lower.contains("url=https://127.0.0.1:")
            || lower.contains("url=https://localhost:")
            || lower.contains("url=http://127.0.0.1:")
            || lower.contains("url=http://localhost:")))
    {
        return false;
    }

    let transport_error = lower.contains("请求失败")
        && (lower.contains("[connect]")
            || lower.contains("[timeout]")
            || lower.contains("[tls]")
            || lower.contains("connection refused")
            || lower.contains("connection reset")
            || lower.contains("timed out")
            || lower.contains("dns error")
            || lower.contains("error sending request"));
    if transport_error {
        return true;
    }

    lower.contains("languageserverstarted")
        && (lower.contains("超时") || lower.contains("通知通道已关闭"))
}

fn parse_wakeup_ui_error_payload_from_message(message: &str) -> Option<WakeupUiErrorPayload> {
    let payload = message.strip_prefix(WAKEUP_ERROR_JSON_PREFIX)?.trim();
    if payload.is_empty() {
        return None;
    }
    serde_json::from_str::<WakeupUiErrorPayload>(payload).ok()
}

fn is_gateway_upstream_retryable_error_message(message: &str) -> bool {
    if let Some(payload) = parse_wakeup_ui_error_payload_from_message(message) {
        if payload.kind.eq_ignore_ascii_case("temporary") {
            return true;
        }
        if matches!(
            payload.error_code,
            Some(4 | 8 | 13 | 14 | 408 | 429 | 500 | 502 | 503 | 504)
        ) {
            return true;
        }
    }

    let lower = message.to_ascii_lowercase();
    lower.contains("internal (code 500)")
        || lower.contains("service unavailable")
        || lower.contains("deadline exceeded")
}

fn is_cascade_status_running(status: &str) -> bool {
    let normalized = status.trim().to_ascii_uppercase();
    !normalized.is_empty() && normalized.contains("RUNNING")
}

async fn post_gateway_json(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
    op_name: &str,
    cancel_rx: Option<&watch::Receiver<bool>>,
) -> Result<serde_json::Value, String> {
    let resp = await_with_cancel(
        cancel_rx,
        client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(body)
            .send(),
    )
    .await?
    .map_err(|e| {
        let kind = classify_gateway_transport_error(&e);
        let message = format!("网关 {} 请求失败[{}]: {} (url={})", op_name, kind, e, url);
        crate::modules::logger::log_error(&format!("[Wakeup] {}", message));
        message
    })?;

    let status = resp.status();
    let text = await_with_cancel(cancel_rx, resp.text())
        .await?
        .unwrap_or_default();
    if !status.is_success() {
        crate::modules::logger::log_error(&format!(
            "[Wakeup] 网关 {} 返回错误: url={}, status={}, body_len={}",
            op_name,
            url,
            status,
            summarize_body_len(&text)
        ));
        return Err(format!(
            "网关 {} 返回错误: status={}, body_len={}",
            op_name,
            status,
            summarize_body_len(&text)
        ));
    }

    serde_json::from_str::<serde_json::Value>(&text).map_err(|e| {
        let message = format!("网关 {} 响应解析失败: {} (url={})", op_name, e, url);
        crate::modules::logger::log_error(&format!(
            "[Wakeup] {}，body_len={}",
            message,
            summarize_body_len(&text)
        ));
        message
    })
}

async fn resolve_requested_model_for_official_ls(
    account_id: &str,
    model: &str,
    cancel_rx: Option<&watch::Receiver<bool>>,
) -> Result<serde_json::Value, String> {
    let trimmed = model.trim();
    if let Ok(num) = trimmed.parse::<i64>() {
        return Ok(json!({ "model": num }));
    }

    let mut account = match modules::load_account(account_id) {
        Ok(v) => v,
        Err(err) => {
            return Err(format!(
                "requestedModel 严格模式解析失败（读取账号失败）: account_id={}, err={}",
                account_id, err
            ));
        }
    };

    let token = match await_with_cancel(
        cancel_rx,
        modules::oauth::ensure_fresh_token(&account.token),
    )
    .await?
    {
        Ok(v) => {
            if v.access_token != account.token.access_token
                || v.refresh_token != account.token.refresh_token
                || v.expiry_timestamp != account.token.expiry_timestamp
                || v.project_id != account.token.project_id
                || v.is_gcp_tos != account.token.is_gcp_tos
            {
                account.token = v.clone();
                let _ = modules::save_account(&account);
            }
            v
        }
        Err(err) => {
            return Err(format!(
                "requestedModel 严格模式解析失败（刷新 token 失败）: account_id={}, err={}",
                account_id, err
            ));
        }
    };

    let client = crate::utils::http::create_client(15);
    let payload = json!({});
    let mut last_error: Option<String> = None;

    for base in CLOUD_CODE_BASE_URLS {
        for attempt in 1..=DEFAULT_ATTEMPTS {
            if is_wakeup_cancelled(cancel_rx) {
                return Err(wakeup_cancelled_error());
            }
            let url = format!("{}{}", base, FETCH_MODELS_PATH);
            let response = await_with_cancel(
                cancel_rx,
                client
                    .post(&url)
                    .bearer_auth(&token.access_token)
                    .header(reqwest::header::USER_AGENT, USER_AGENT)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .header(reqwest::header::ACCEPT_ENCODING, "gzip")
                    .json(&payload)
                    .send(),
            )
            .await?;

            match response {
                Ok(res) => {
                    let status = res.status();
                    if status.is_success() {
                        match await_with_cancel(cancel_rx, res.json::<AvailableModelsResponse>())
                            .await?
                        {
                            Ok(parsed) => {
                                if let Some(models) = extract_available_models_map(&parsed) {
                                    if let Some(meta) = models.get(trimmed) {
                                        if let Some(model_constant) = meta
                                            .model_constant
                                            .as_deref()
                                            .map(str::trim)
                                            .filter(|v| !v.is_empty())
                                        {
                                            if let Ok(num) = model_constant.parse::<i64>() {
                                                crate::modules::logger::log_info(&format!(
                                                    "[Wakeup] requestedModel 解析: {} -> modelConstant({})",
                                                    trimmed, num
                                                ));
                                                return Ok(json!({ "model": num }));
                                            }
                                            if let Some(num) =
                                                parse_codeium_model_enum_name(model_constant)
                                            {
                                                crate::modules::logger::log_info(&format!(
                                                    "[Wakeup] requestedModel 解析: {} -> model({})（由 enum 名 {} 映射）",
                                                    trimmed, num, model_constant
                                                ));
                                                return Ok(json!({ "model": num }));
                                            }

                                            last_error = Some(format!(
                                                "requestedModel 严格模式解析失败（模型常量无法映射）: model={}, model_constant={}",
                                                trimmed, model_constant
                                            ));
                                        } else {
                                            last_error = Some(format!(
                                                "requestedModel 严格模式解析失败（模型缺少 model_constant）: model={}",
                                                trimmed
                                            ));
                                        }
                                    } else {
                                        last_error = Some(format!(
                                            "requestedModel 严格模式解析失败（fetchAvailableModels 未返回该模型）: model={}",
                                            trimmed
                                        ));
                                    }
                                } else {
                                    last_error = Some(format!(
                                        "requestedModel 严格模式解析失败（fetchAvailableModels 响应缺少 models）: model={}",
                                        trimmed
                                    ));
                                }
                                break;
                            }
                            Err(err) => {
                                last_error = Some(format!("解析模型列表失败: {}", err));
                            }
                        }
                    } else {
                        let text = await_with_cancel(cancel_rx, res.text())
                            .await?
                            .unwrap_or_default();
                        let retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                            || status.as_u16() >= 500;

                        // 对 403 尝试解析 TOS_VIOLATION / VALIDATION_REQUIRED 并编码为结构化错误
                        if status == reqwest::StatusCode::FORBIDDEN {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                                let details = parsed.get("error").and_then(|e| e.get("details"));
                                let appeal_url = extract_appeal_url_from_error_details(details);
                                let validation_url =
                                    extract_validation_url_from_error_details(details);
                                let message = parsed
                                    .get("error")
                                    .and_then(|e| e.get("message"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_string();

                                let detail = GatewayTrajectoryErrorDetail {
                                    message: if message.is_empty() {
                                        format!("获取模型列表失败: {}", status)
                                    } else {
                                        message
                                    },
                                    error_code: Some(403),
                                    validation_url,
                                    appeal_url,
                                    trajectory_id: None,
                                    error_message_json: text.clone(),
                                    step_json: String::new(),
                                };
                                last_error = Some(encode_wakeup_ui_error_payload(&detail));
                                break;
                            }
                        }

                        last_error = Some(format!("获取模型列表失败: {} - {}", status, text));
                        if retryable && attempt < DEFAULT_ATTEMPTS {
                            let delay = get_backoff_delay_ms(attempt + 1);
                            if delay > 0 {
                                sleep_with_cancel(
                                    std::time::Duration::from_millis(delay),
                                    cancel_rx,
                                )
                                .await?;
                            }
                            continue;
                        }
                    }
                }
                Err(err) => {
                    last_error = Some(format!("获取模型列表失败: {}", err));
                    if attempt < DEFAULT_ATTEMPTS {
                        let delay = get_backoff_delay_ms(attempt + 1);
                        if delay > 0 {
                            sleep_with_cancel(std::time::Duration::from_millis(delay), cancel_rx)
                                .await?;
                        }
                        continue;
                    }
                }
            }
        }
    }

    if let Some(err) = last_error {
        crate::modules::logger::log_warn(&format!("[Wakeup] {}", err));
        return Err(err);
    } else {
        crate::modules::logger::log_warn(&format!(
            "[Wakeup] requestedModel 严格模式解析失败（未知原因）: model={}",
            trimmed
        ));
        return Err(format!(
            "requestedModel 严格模式解析失败（未知原因）: model={}",
            trimmed
        ));
    }
}

fn parse_codeium_model_enum_name(model_constant: &str) -> Option<i64> {
    let trimmed = model_constant.trim();
    if let Some(idx) = parse_placeholder_model_index(trimmed) {
        // 官方 exa.codeium_common_pb.Model 枚举中 PLACEHOLDER_M0 从 1000 开始连续递增。
        return Some(1000 + idx);
    }

    match trimmed {
        // 目前先补已确认枚举；后续如出现更多非 placeholder 枚举名，再按需补充。
        "MODEL_OPENAI_GPT_OSS_120B_MEDIUM" | "OPENAI_GPT_OSS_120B_MEDIUM" => Some(342),
        _ => None,
    }
}

fn parse_placeholder_model_index(raw: &str) -> Option<i64> {
    // fetchAvailableModels 的枚举 JSON 名在不同接口/版本里可能带前缀或尾随字符，
    // 这里按子串 PLACEHOLDER_M<digits> 做宽松匹配，避免误回退成 alias。
    let marker = "PLACEHOLDER_M";
    let start = raw.find(marker)? + marker.len();
    let digits: String = raw[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<i64>().ok()
}

fn normalize_local_gateway_base_url(base_url: String) -> String {
    if let Some(rest) = base_url.strip_prefix("https://localhost:") {
        return format!("https://127.0.0.1:{}", rest);
    }
    if let Some(rest) = base_url.strip_prefix("http://localhost:") {
        return format!("http://127.0.0.1:{}", rest);
    }
    base_url
}

fn build_client_like_cascade_config(
    requested_model: serde_json::Value,
    max_output_tokens: u32,
) -> serde_json::Value {
    let max_tokens = if max_output_tokens > 0 {
        max_output_tokens
    } else {
        8192
    };

    json!({
        "plannerConfig": {
            "requestedModel": requested_model,
            "maxOutputTokens": max_tokens,
        },
        "checkpointConfig": {
            "maxOutputTokens": max_tokens,
        }
    })
}

fn summarize_gateway_trajectory_for_log(get_resp: &serde_json::Value) -> String {
    let status = get_resp
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("-");

    let Some(steps) = get_resp
        .get("trajectory")
        .and_then(|v| v.get("steps"))
        .and_then(|v| v.as_array())
    else {
        return format!("status={}, steps=0", status);
    };

    let last_step_case = steps.last().and_then(step_case_name).unwrap_or("-");

    let planner_keys = steps
        .iter()
        .rev()
        .find(|step| step_case_name(step) == Some("plannerResponse"))
        .and_then(|step| step_case_value(step, "plannerResponse"))
        .and_then(|v| v.as_object())
        .map(|obj| {
            let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
            keys.sort_unstable();
            keys.join(",")
        })
        .unwrap_or_else(|| "-".to_string());

    format!(
        "status={}, steps={}, last_step_case={}, planner_keys={}",
        status,
        steps.len(),
        last_step_case,
        planner_keys
    )
}

fn step_case_name(step: &serde_json::Value) -> Option<&str> {
    if let Some(case_name) = step
        .get("step")
        .and_then(|v| v.get("case"))
        .and_then(|v| v.as_str())
    {
        return Some(case_name);
    }

    // 官方 LS 原生 JSON（proto oneof）通常直接把 oneof 字段展开到 step 顶层。
    for key in [
        "plannerResponse",
        "errorMessage",
        "userInput",
        "toolCall",
        "checkpoint",
        "commandStatus",
        "notifyUser",
        "ephemeralMessage",
    ] {
        if step.get(key).is_some() {
            return Some(key);
        }
    }

    None
}

fn step_case_value<'a>(
    step: &'a serde_json::Value,
    case_name: &str,
) -> Option<&'a serde_json::Value> {
    if step_case_name(step) == Some(case_name) {
        if let Some(v) = step.get("step").and_then(|v| v.get("value")) {
            return Some(v);
        }
        if let Some(v) = step.get(case_name) {
            return Some(v);
        }
    }
    None
}

#[derive(Debug)]
struct GatewayTrajectoryErrorDetail {
    message: String,
    error_code: Option<i64>,
    validation_url: Option<String>,
    appeal_url: Option<String>,
    trajectory_id: Option<String>,
    error_message_json: String,
    step_json: String,
}

fn classify_gateway_error_kind(
    error_code: Option<i64>,
    appeal_url: &Option<String>,
) -> &'static str {
    if error_code == Some(403) && appeal_url.is_some() {
        return "tos_violation";
    }
    match error_code {
        Some(403) => "verification_required",
        Some(429) => "quota",
        // gRPC canonical codes: RESOURCE_EXHAUSTED=8, DEADLINE_EXCEEDED=4, INTERNAL=13, UNAVAILABLE=14
        // 同时覆盖 HTTP 侧常见的临时失败码。
        Some(8) | Some(4) | Some(13) | Some(14) | Some(408) | Some(500) | Some(502) | Some(503)
        | Some(504) => "temporary",
        _ => "generic",
    }
}

fn encode_wakeup_ui_error_payload(detail: &GatewayTrajectoryErrorDetail) -> String {
    let payload = WakeupUiErrorPayload {
        version: 1,
        kind: classify_gateway_error_kind(detail.error_code, &detail.appeal_url).to_string(),
        message: detail.message.clone(),
        error_code: detail.error_code,
        validation_url: detail.validation_url.clone(),
        appeal_url: detail.appeal_url.clone(),
        trajectory_id: detail.trajectory_id.clone(),
        error_message_json: Some(detail.error_message_json.clone()),
        step_json: Some(detail.step_json.clone()),
    };
    match serde_json::to_string(&payload) {
        Ok(text) => format!("{}{}", WAKEUP_ERROR_JSON_PREFIX, text),
        Err(_) => detail.message.clone(),
    }
}

fn extract_wakeup_response_from_gateway_trajectory(
    get_resp: &serde_json::Value,
    duration_ms: u64,
) -> Option<WakeupResponse> {
    let steps = get_resp
        .get("trajectory")
        .and_then(|v| v.get("steps"))
        .and_then(|v| v.as_array())?;

    for step in steps.iter().rev() {
        if step_case_name(step) != Some("plannerResponse") {
            continue;
        }

        let value = step_case_value(step, "plannerResponse")?;
        let reply = value
            .get("modifiedResponse")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("response").and_then(|v| v.as_str()))
            .or_else(|| {
                value
                    .get("response")
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
            })
            .or_else(|| {
                value
                    .get("response")
                    .and_then(|v| v.get("candidates"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.get("content"))
                    .and_then(|v| v.get("parts"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| {
                        arr.iter()
                            .find_map(|part| part.get("text").and_then(|v| v.as_str()))
                    })
            })?;
        let reply = reply.trim();
        if reply.is_empty() {
            continue;
        }

        return Some(WakeupResponse {
            reply: reply.to_string(),
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            trace_id: None,
            response_id: None,
            duration_ms,
        });
    }

    None
}

fn extract_gateway_error_from_trajectory(
    get_resp: &serde_json::Value,
) -> Option<GatewayTrajectoryErrorDetail> {
    let steps = get_resp
        .get("trajectory")
        .and_then(|v| v.get("steps"))
        .and_then(|v| v.as_array())?;
    let trajectory_id = get_resp
        .get("trajectory")
        .and_then(|v| v.get("trajectoryId"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);

    for step in steps.iter().rev() {
        if step_case_name(step) != Some("errorMessage") {
            continue;
        }

        let error_value = step_case_value(step, "errorMessage")?;
        let error_obj = error_value
            .get("error")
            .filter(|v| v.is_object())
            .unwrap_or(error_value);

        let msg = error_obj
            .get("userErrorMessage")
            .or_else(|| error_obj.get("message"))
            .or_else(|| error_obj.get("shortError"))
            .or_else(|| error_obj.get("fullError"))
            .or_else(|| error_value.get("userErrorMessage"))
            .or_else(|| error_value.get("message"))
            .and_then(|v| v.as_str())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "官方 LS 返回错误".to_string());

        let error_code = error_obj
            .get("errorCode")
            .or_else(|| error_obj.get("code"))
            .or_else(|| error_value.get("errorCode"))
            .or_else(|| error_value.get("code"))
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
            });
        let details_value = error_obj
            .get("details")
            .or_else(|| error_value.get("details"));
        let validation_url = extract_validation_url_from_error_details(details_value);
        let appeal_url = extract_appeal_url_from_error_details(details_value);
        let error_message_json =
            serde_json::to_string(error_value).unwrap_or_else(|_| "{}".to_string());
        let step_json = serde_json::to_string(step).unwrap_or_else(|_| "{}".to_string());

        return Some(GatewayTrajectoryErrorDetail {
            message: msg,
            error_code,
            validation_url,
            appeal_url,
            trajectory_id: trajectory_id.clone(),
            error_message_json,
            step_json,
        });
    }

    None
}

fn extract_validation_url_from_error_details(
    details: Option<&serde_json::Value>,
) -> Option<String> {
    let details = details?;
    let parsed = match details {
        serde_json::Value::String(text) => serde_json::from_str::<serde_json::Value>(text).ok()?,
        other => other.clone(),
    };

    let error_details = parsed
        .get("error")
        .and_then(|v| v.get("details"))
        .and_then(|v| v.as_array())?;

    for item in error_details {
        let ty = item
            .get("@type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let reason = item
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = item
            .get("metadata")
            .and_then(|v| v.get("validation_url"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if ty == "type.googleapis.com/google.rpc.ErrorInfo" && reason == "VALIDATION_REQUIRED" {
            if let Some(url) = url {
                return Some(url.to_string());
            }
        }
    }

    None
}

fn extract_appeal_url_from_error_details(details: Option<&serde_json::Value>) -> Option<String> {
    let details = details?;
    let parsed = match details {
        serde_json::Value::String(text) => serde_json::from_str::<serde_json::Value>(text).ok()?,
        other => other.clone(),
    };

    let error_details = parsed
        .get("error")
        .and_then(|v| v.get("details"))
        .and_then(|v| v.as_array())
        .or_else(|| parsed.as_array())?;

    for item in error_details {
        let ty = item
            .get("@type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let reason = item
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if ty == "type.googleapis.com/google.rpc.ErrorInfo" && reason == "TOS_VIOLATION" {
            let url = item
                .get("metadata")
                .and_then(|v| v.get("appeal_url"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty());
            if let Some(url) = url {
                return Some(url.to_string());
            }
        }
    }

    None
}

async fn resolve_client_gateway_base_url() -> Result<(String, bool), String> {
    if let Ok(value) = std::env::var("AG_WAKEUP_GATEWAY_BASE_URL") {
        let trimmed = value.trim().trim_end_matches('/').to_string();
        if !trimmed.is_empty() {
            return Ok((normalize_local_gateway_base_url(trimmed), true));
        }
    }

    Ok((
        normalize_local_gateway_base_url(
            modules::wakeup_gateway::ensure_local_gateway_started().await?,
        ),
        false,
    ))
}

async fn trigger_wakeup_via_client_gateway_once(
    account_id: &str,
    model: &str,
    prompt: &str,
    max_output_tokens: u32,
    base_url: &str,
    cancel_rx: Option<&watch::Receiver<bool>>,
) -> Result<WakeupResponse, String> {
    let client = build_gateway_client(base_url, 30)?;
    post_gateway_json(
        &client,
        &format!(
            "{}{}",
            base_url,
            modules::wakeup_gateway::INTERNAL_HEALTH_CHECK_PATH
        ),
        &json!({}),
        "HealthCheck",
        cancel_rx,
    )
    .await?;

    let service_base = format!("{}/exa.language_server_pb.LanguageServerService", base_url);
    let start_resp: serde_json::Value;

    {
        let _bind_guard = gateway_start_bind_lock().lock().await;
        let prepare_url = format!(
            "{}{}",
            base_url,
            modules::wakeup_gateway::INTERNAL_PREPARE_START_CONTEXT_PATH
        );

        await_with_cancel(
            cancel_rx,
            client
                .post(&prepare_url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&json!({
                    "accountId": account_id,
                    "model": model,
                    "maxOutputTokens": max_output_tokens,
                }))
                .send(),
        )
        .await?
        .map_err(|e| {
            let kind = classify_gateway_transport_error(&e);
            let message = format!(
                "网关 prepareStartContext 请求失败[{}]: {} (url={})",
                kind, e, prepare_url
            );
            crate::modules::logger::log_error(&format!("[Wakeup] {}", message));
            message
        })?
        .error_for_status()
        .map_err(|e| {
            format!(
                "网关 prepareStartContext 返回错误: {} (url={})",
                e, prepare_url
            )
        })?;

        start_resp = post_gateway_json(
            &client,
            &format!("{}/StartCascade", service_base),
            &json!({}),
            "StartCascade",
            cancel_rx,
        )
        .await?;
    }

    let cascade_id = start_resp
        .get("cascadeId")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| "网关 StartCascade 未返回 cascadeId".to_string())?
        .to_string();
    crate::modules::logger::log_info(&format!(
        "[Wakeup] 网关 StartCascade 成功: cascade_id={}",
        cascade_id
    ));

    let wakeup_result: Result<WakeupResponse, String> = async {
        let requested_model =
            resolve_requested_model_for_official_ls(account_id, model, cancel_rx).await?;

        let send_body = json!({
            "cascadeId": cascade_id,
            "items": [{ "text": prompt }],
            "cascadeConfig": build_client_like_cascade_config(requested_model, max_output_tokens),
        });

        post_gateway_json(
            &client,
            &format!("{}/SendUserCascadeMessage", service_base),
            &send_body,
            "SendUserCascadeMessage",
            cancel_rx,
        )
        .await?;
        crate::modules::logger::log_info(&format!(
            "[Wakeup] 网关 SendUserCascadeMessage 成功: cascade_id={}",
            cascade_id
        ));

        let get_body = json!({ "cascadeId": cascade_id });
        let started_at = std::time::Instant::now();
        let mut last_status = String::new();
        let mut last_trajectory_summary = String::new();
        let mut last_running_error: Option<GatewayTrajectoryErrorDetail> = None;

        for poll_idx in 0..CLIENT_GATEWAY_MAX_POLL_ROUNDS {
            let get_resp: serde_json::Value = post_gateway_json(
                &client,
                &format!("{}/GetCascadeTrajectory", service_base),
                &get_body,
                "GetCascadeTrajectory",
                cancel_rx,
            )
            .await?;

            last_status = get_resp
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            if poll_idx == 0 || poll_idx % 8 == 0 {
                let summary = summarize_gateway_trajectory_for_log(&get_resp);
                if summary != last_trajectory_summary {
                    crate::modules::logger::log_info(&format!(
                        "[Wakeup] 网关轨迹轮询#{}: {}",
                        poll_idx + 1,
                        summary
                    ));
                    last_trajectory_summary = summary;
                }
            }

            if let Some(parsed) = extract_wakeup_response_from_gateway_trajectory(
                &get_resp,
                started_at.elapsed().as_millis() as u64,
            ) {
                if last_running_error.is_some() {
                    crate::modules::logger::log_info(&format!(
                        "[Wakeup] 网关轨迹在中间错误后恢复成功: cascade_id={}",
                        cascade_id
                    ));
                }
                crate::modules::logger::log_info(&format!(
                    "[Wakeup] 网关唤醒成功: cascade_id={}, duration={}ms, reply={}",
                    cascade_id,
                    parsed.duration_ms,
                    truncate_log_text(&parsed.reply, 1000)
                ));
                return Ok(parsed);
            }

            if let Some(err) = extract_gateway_error_from_trajectory(&get_resp) {
                if is_cascade_status_running(&last_status) {
                    if poll_idx == 0 || poll_idx % 8 == 0 {
                        crate::modules::logger::log_warn(&format!(
                            "[Wakeup] 网关轨迹出现中间错误但状态仍为 RUNNING，继续等待恢复: cascade_id={}, status={}, error_code={:?}, message={}",
                            cascade_id,
                            last_status,
                            err.error_code,
                            truncate_log_text(&err.message, 500)
                        ));
                    }
                    last_running_error = Some(err);
                    sleep_with_cancel(
                        std::time::Duration::from_millis(CLIENT_GATEWAY_POLL_INTERVAL_MS),
                        cancel_rx,
                    )
                    .await?;
                    continue;
                }

                if err.error_code == Some(403) {
                    crate::modules::logger::log_error(&format!(
                        "[Wakeup] 网关轨迹错误(403): cascade_id={}, status={}, message={}, validation_url={:?}, errorMessage_len={}, step_len={}",
                        cascade_id,
                        if last_status.is_empty() { "-" } else { &last_status },
                        err.message,
                        err.validation_url,
                        err.error_message_json.len(),
                        err.step_json.len()
                    ));
                } else {
                    crate::modules::logger::log_error(&format!(
                        "[Wakeup] 网关轨迹错误: cascade_id={}, status={}, error_code={:?}, message={}, errorMessage_len={}",
                        cascade_id,
                        if last_status.is_empty() { "-" } else { &last_status },
                        err.error_code,
                        err.message,
                        err.error_message_json.len()
                    ));
                }
                return Err(encode_wakeup_ui_error_payload(&err));
            }

            sleep_with_cancel(
                std::time::Duration::from_millis(CLIENT_GATEWAY_POLL_INTERVAL_MS),
                cancel_rx,
            )
            .await?;
        }

        if let Some(err) = last_running_error {
            crate::modules::logger::log_error(&format!(
                "[Wakeup] 网关轨迹持续错误且未恢复: cascade_id={}, last_status={}, error_code={:?}, message={}",
                cascade_id,
                if last_status.is_empty() { "-" } else { &last_status },
                err.error_code,
                truncate_log_text(&err.message, 500)
            ));
            return Err(encode_wakeup_ui_error_payload(&err));
        }

        let message = if last_status.is_empty() {
            "网关未返回唤醒结果（轨迹中未出现 plannerResponse.modifiedResponse）".to_string()
        } else {
            format!("网关未在超时时间内返回唤醒结果，最后状态={}", last_status)
        };
        crate::modules::logger::log_error(&format!(
            "[Wakeup] 网关唤醒失败(超时/无结果): cascade_id={}, error={}",
            cascade_id, message
        ));
        Err(message)
    }
    .await;

    if matches!(&wakeup_result, Err(err) if is_wakeup_cancelled_message(err)) {
        let cleanup_client = client.clone();
        let cleanup_url = format!("{}/DeleteCascadeTrajectory", service_base);
        let cleanup_cascade_id = cascade_id.clone();
        tokio::spawn(async move {
            let delete_resp = cleanup_client
                .post(cleanup_url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&json!({ "cascadeId": cleanup_cascade_id }))
                .send()
                .await;

            if let Err(err) = delete_resp {
                crate::modules::logger::log_warn(&format!(
                    "[Wakeup] 取消后异步删除网关轨迹失败: cascade_id={}, error={}",
                    cascade_id, err
                ));
            }
        });
        return wakeup_result;
    }

    let delete_resp = client
        .post(format!("{}/DeleteCascadeTrajectory", service_base))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&json!({ "cascadeId": cascade_id }))
        .send()
        .await;

    if let Err(err) = delete_resp {
        crate::modules::logger::log_error(&format!(
            "[Wakeup] 删除网关轨迹失败: cascade_id={}, error={}",
            cascade_id, err
        ));
    }

    if let Err(err) = &wakeup_result {
        crate::modules::logger::log_error(&format!(
            "[Wakeup] 网关唤醒失败: cascade_id={}, error={}",
            cascade_id, err
        ));
    }

    wakeup_result
}

async fn trigger_wakeup_via_client_gateway(
    account_id: &str,
    model: &str,
    prompt: &str,
    max_output_tokens: u32,
    cancel_rx: Option<&watch::Receiver<bool>>,
) -> Result<WakeupResponse, String> {
    crate::modules::logger::log_info(&format!(
        "[Wakeup] 开始唤醒(网关): account_id={}, model={}, max_tokens={}, prompt={}",
        account_id,
        model,
        max_output_tokens,
        format_prompt_for_log(prompt)
    ));

    for attempt in 1..=2 {
        let (base_url, from_env) = resolve_client_gateway_base_url().await?;
        crate::modules::logger::log_info(&format!(
            "[Wakeup] 使用 client-like 网关通道（官方协议）: base_url={}, attempt={}",
            base_url, attempt
        ));

        match trigger_wakeup_via_client_gateway_once(
            account_id,
            model,
            prompt,
            max_output_tokens,
            &base_url,
            cancel_rx,
        )
        .await
        {
            Ok(resp) => return Ok(resp),
            Err(err) => {
                if is_wakeup_cancelled_message(&err) {
                    return Err(err);
                }
                if attempt == 1 && !from_env && is_local_gateway_recoverable_error_message(&err) {
                    crate::modules::logger::log_warn(&format!(
                        "[Wakeup] 检测到本地网关可恢复错误（传输或官方 LS 启动超时），尝试重建网关并重试一次: {}",
                        err
                    ));
                    modules::wakeup_gateway::clear_local_gateway_base_url_cache().await;
                    continue;
                }
                if attempt == 1 && is_gateway_upstream_retryable_error_message(&err) {
                    crate::modules::logger::log_warn(&format!(
                        "[Wakeup] 检测到上游临时错误，等待后自动重试一次: {}",
                        err
                    ));
                    sleep_with_cancel(
                        std::time::Duration::from_millis(CLIENT_GATEWAY_UPSTREAM_RETRY_DELAY_MS),
                        cancel_rx,
                    )
                    .await?;
                    continue;
                }
                return Err(err);
            }
        }
    }

    Err("网关重试后仍失败".to_string())
}

/// 旧版直连 Cloud Code 唤醒（保留给兼容模式与网关内部调用）
pub(crate) async fn trigger_wakeup_direct(
    account_id: &str,
    model: &str,
    prompt: &str,
    max_output_tokens: u32,
    cancel_rx: Option<&watch::Receiver<bool>>,
) -> Result<WakeupResponse, String> {
    let mut account = modules::load_account(account_id)?;
    crate::modules::logger::log_info(&format!(
        "[Wakeup] 开始唤醒: email={}, model={}, max_tokens={}, prompt={}",
        account.email,
        model,
        max_output_tokens,
        format_prompt_for_log(prompt)
    ));
    let mut token = await_with_cancel(
        cancel_rx,
        modules::oauth::ensure_fresh_token(&account.token),
    )
    .await??;

    let (project_id, _, _) = await_with_cancel(
        cancel_rx,
        modules::quota::fetch_project_id_for_token(&token, &account.email),
    )
    .await?;
    let final_project_id = project_id
        .clone()
        .or_else(|| token.project_id.clone())
        .ok_or_else(|| "project_id 缺失，无法执行唤醒请求".to_string())?;
    crate::modules::logger::log_info(&format!("[Wakeup] 项目ID: {}", final_project_id));

    if token.project_id.is_none() && project_id.is_some() {
        token.project_id = project_id.clone();
    }

    if token.access_token != account.token.access_token
        || token.expiry_timestamp != account.token.expiry_timestamp
        || token.project_id != account.token.project_id
    {
        account.token = token.clone();
        let _ = modules::save_account(&account);
    }

    let client = crate::utils::http::create_client(15);
    let body = build_request_body(&final_project_id, model, prompt, max_output_tokens);
    let started = std::time::Instant::now();

    match send_stream_request(&client, &token.access_token, &body, cancel_rx).await {
        Ok(parsed) => {
            let duration_ms = started.elapsed().as_millis() as u64;
            crate::modules::logger::log_info(&format!(
                "[Wakeup] 唤醒完成: duration={}ms",
                duration_ms
            ));
            Ok(WakeupResponse {
                reply: parsed.reply,
                prompt_tokens: parsed.prompt_tokens,
                completion_tokens: parsed.completion_tokens,
                total_tokens: parsed.total_tokens,
                trace_id: parsed.trace_id,
                response_id: parsed.response_id,
                duration_ms,
            })
        }
        Err(err) => {
            if is_wakeup_cancelled_message(&err) {
                crate::modules::logger::log_info("[Wakeup] 唤醒测试已取消");
            } else {
                crate::modules::logger::log_error(&format!("[Wakeup] 唤醒失败: {}", err));
            }
            Err(err)
        }
    }
}

/// 触发单个账号的唤醒请求（根据配置/环境变量分发通道）
pub async fn trigger_wakeup(
    account_id: &str,
    model: &str,
    prompt: &str,
    max_output_tokens: u32,
    cancel_scope_id: Option<&str>,
) -> Result<WakeupResponse, String> {
    // 执行前检查：唤醒链路依赖官方 LS 二进制，未就绪时不再发起网络请求。
    let _ = ensure_wakeup_runtime_ready()?;
    let cancel_rx = get_wakeup_cancel_receiver(cancel_scope_id)?;

    match resolve_wakeup_transport_mode() {
        WakeupTransportMode::LegacyCloudCode => {
            crate::modules::logger::log_info("[Wakeup] 通道=legacy_cloudcode");
            trigger_wakeup_direct(
                account_id,
                model,
                prompt,
                max_output_tokens,
                cancel_rx.as_ref(),
            )
            .await
        }
        WakeupTransportMode::ClientGateway => {
            crate::modules::logger::log_info("[Wakeup] 通道=client_gateway");
            trigger_wakeup_via_client_gateway(
                account_id,
                model,
                prompt,
                max_output_tokens,
                cancel_rx.as_ref(),
            )
            .await
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableModel {
    pub id: String,
    pub display_name: String,
    pub model_constant: Option<String>,
    pub recommended: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AvailableModelsResponse {
    payload: Option<AvailableModelsPayload>,
    #[serde(rename = "agentModelSorts")]
    agent_model_sorts: Option<Vec<AvailableAgentModelSort>>,
    models: Option<HashMap<String, AvailableModelMeta>>,
}

#[derive(Debug, Deserialize)]
struct AvailableModelsPayload {
    #[serde(rename = "agentModelSorts")]
    agent_model_sorts: Option<Vec<AvailableAgentModelSort>>,
    models: Option<HashMap<String, AvailableModelMeta>>,
}

#[derive(Debug, Deserialize)]
struct AvailableAgentModelSort {
    groups: Option<Vec<AvailableAgentModelGroup>>,
}

#[derive(Debug, Deserialize)]
struct AvailableAgentModelGroup {
    #[serde(rename = "modelIds")]
    model_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct AvailableModelMeta {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "model")]
    model_constant: Option<String>,
    #[serde(rename = "recommended")]
    recommended: Option<bool>,
}

fn extract_available_models_map(
    response: &AvailableModelsResponse,
) -> Option<&HashMap<String, AvailableModelMeta>> {
    response
        .payload
        .as_ref()
        .and_then(|payload| payload.models.as_ref())
        .or(response.models.as_ref())
}

fn extract_agent_model_sorts(
    response: &AvailableModelsResponse,
) -> Option<&Vec<AvailableAgentModelSort>> {
    response
        .payload
        .as_ref()
        .and_then(|payload| payload.agent_model_sorts.as_ref())
        .or(response.agent_model_sorts.as_ref())
}

fn extract_ordered_model_ids(response: &AvailableModelsResponse) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    if let Some(sorts) = extract_agent_model_sorts(response) {
        for sort in sorts {
            if let Some(groups) = sort.groups.as_ref() {
                for group in groups {
                    if let Some(model_ids) = group.model_ids.as_ref() {
                        for id in model_ids {
                            let trimmed = id.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            if seen.insert(trimmed.to_string()) {
                                ids.push(trimmed.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    ids
}

fn has_abnormal_account_marker(account: &crate::models::Account) -> bool {
    if account.disabled {
        return true;
    }
    account
        .disabled_reason
        .as_deref()
        .map(str::trim)
        .is_some_and(|reason| !reason.is_empty())
}

fn build_models_from_available_models_response(
    response: &AvailableModelsResponse,
) -> Vec<AvailableModel> {
    let mut models = Vec::new();
    if let Some(entries) = extract_available_models_map(response) {
        let ordered_ids = extract_ordered_model_ids(response);
        for id in ordered_ids {
            if let Some(meta) = entries.get(&id) {
                models.push(AvailableModel {
                    id: id.clone(),
                    display_name: meta.display_name.clone().unwrap_or_else(|| id.clone()),
                    model_constant: meta.model_constant.clone(),
                    recommended: meta.recommended,
                });
            }
        }
    }
    models
}

fn build_local_fallback_models(accounts: &[crate::models::Account]) -> Vec<AvailableModel> {
    let mut result: Vec<AvailableModel> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for account in accounts {
        let Some(quota) = account.quota.as_ref() else {
            continue;
        };
        for item in &quota.models {
            let id = item.name.trim();
            if id.is_empty() {
                continue;
            }
            let dedupe_key = id.to_ascii_lowercase();
            if !seen.insert(dedupe_key) {
                continue;
            }

            let display_name = item
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(id)
                .to_string();

            result.push(AvailableModel {
                id: id.to_string(),
                display_name,
                model_constant: None,
                recommended: Some(true),
            });
        }
    }

    result
}

async fn fetch_available_models_from_access_token(
    access_token: &str,
) -> Result<AvailableModelsResponse, String> {
    let payload = json!({});
    let client = crate::utils::http::create_client(15);
    let mut last_error: Option<String> = None;

    for base in CLOUD_CODE_BASE_URLS {
        for attempt in 1..=DEFAULT_ATTEMPTS {
            let url = format!("{}{}", base, FETCH_MODELS_PATH);
            let response = client
                .post(url)
                .bearer_auth(access_token)
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT_ENCODING, "gzip")
                .json(&payload)
                .send()
                .await;

            match response {
                Ok(res) => {
                    if res.status().is_success() {
                        let parsed: AvailableModelsResponse = res
                            .json()
                            .await
                            .map_err(|e| format!("解析模型列表失败: {}", e))?;
                        return Ok(parsed);
                    }
                    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                        return Err("Authorization expired".to_string());
                    }
                    if res.status() == reqwest::StatusCode::FORBIDDEN {
                        return Err("Cloud Code access forbidden".to_string());
                    }
                    let status = res.status();
                    let text = res.text().await.unwrap_or_default();
                    let retryable =
                        status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.as_u16() >= 500;
                    last_error = Some(format!("获取模型列表失败: {} - {}", status, text));
                    if retryable && attempt < DEFAULT_ATTEMPTS {
                        let delay = get_backoff_delay_ms(attempt + 1);
                        if delay > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        }
                        continue;
                    }
                }
                Err(err) => {
                    last_error = Some(format!("获取模型列表失败: {}", err));
                    if attempt < DEFAULT_ATTEMPTS {
                        let delay = get_backoff_delay_ms(attempt + 1);
                        if delay > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        }
                        continue;
                    }
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "获取模型列表失败".to_string()))
}

/// 获取可用模型列表（用于唤醒配置）
pub async fn fetch_available_models() -> Result<Vec<AvailableModel>, String> {
    let all_accounts = modules::list_accounts()?;
    if all_accounts.is_empty() {
        return Err("未找到可用账号".to_string());
    }

    let mut normal_accounts: Vec<crate::models::Account> = all_accounts
        .iter()
        .filter(|account| !has_abnormal_account_marker(account))
        .cloned()
        .collect();

    let current_account_id = modules::get_current_account_id().ok().flatten();
    if let Some(current_id) = current_account_id.as_deref() {
        if let Some(pos) = normal_accounts
            .iter()
            .position(|account| account.id == current_id)
        {
            if pos > 0 {
                let current = normal_accounts.remove(pos);
                normal_accounts.insert(0, current);
            }
        }
    }

    let local_fallback_models = build_local_fallback_models(&all_accounts);

    if normal_accounts.is_empty() {
        if !local_fallback_models.is_empty() {
            crate::modules::logger::log_warn(
                "[Wakeup] fetch_available_models 未找到无异常标识账号，回退本地模型列表",
            );
            return Ok(local_fallback_models);
        }
        return Err("未找到无异常标识账号，且本地模型列表为空".to_string());
    }

    let mut last_error: Option<String> = None;
    for account in normal_accounts {
        let token = match modules::oauth::ensure_fresh_token(&account.token).await {
            Ok(value) => {
                if value.access_token != account.token.access_token
                    || value.refresh_token != account.token.refresh_token
                    || value.expiry_timestamp != account.token.expiry_timestamp
                    || value.project_id != account.token.project_id
                    || value.is_gcp_tos != account.token.is_gcp_tos
                {
                    let mut updated = account.clone();
                    updated.token = value.clone();
                    let _ = modules::save_account(&updated);
                }
                value
            }
            Err(err) => {
                let message = format!("账号 {} 刷新 token 失败: {}", account.email, err);
                crate::modules::logger::log_warn(&format!("[Wakeup] {}", message));
                last_error = Some(message);
                continue;
            }
        };

        match fetch_available_models_from_access_token(&token.access_token).await {
            Ok(response) => {
                let models = build_models_from_available_models_response(&response);
                if !models.is_empty() {
                    return Ok(models);
                }
                let message = format!(
                    "账号 {} 模型列表为空：官方接口未返回可用模型",
                    account.email
                );
                crate::modules::logger::log_warn(&format!("[Wakeup] {}", message));
                last_error = Some(message);
            }
            Err(err) => {
                let message = format!("账号 {} 获取模型列表失败: {}", account.email, err);
                crate::modules::logger::log_warn(&format!("[Wakeup] {}", message));
                last_error = Some(message);
            }
        }
    }

    if !local_fallback_models.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[Wakeup] fetch_available_models 所有无异常标识账号获取失败，回退本地模型列表，last_error={}",
            last_error.clone().unwrap_or_else(|| "-".to_string())
        ));
        return Ok(local_fallback_models);
    }

    Err(last_error.unwrap_or_else(|| {
        "获取模型列表失败：无异常标识账号均不可用，且本地模型列表为空".to_string()
    }))
}
