// Deliberately separate from wakeup: bounded streaming, no tools, no model fallback,
// no API-service startup and no automatic generation retries.
pub const PELICAN_DELIVERY_INSTRUCTIONS: &str = "Return a complete standalone HTML document in your response. Do not use Markdown fences or external dependencies.";
const PELICAN_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const PELICAN_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const PELICAN_TOTAL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
static PELICAN_PREPARATION_SLOTS: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(6)));

pub struct PelicanChatOutput {
    pub reply: String,
    pub usage: Option<Value>,
    pub response_id: Option<String>,
    pub response_model: Option<String>,
    pub quota: Option<PelicanQuotaSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PelicanQuotaSnapshot {
    pub used_percent: f64,
    pub remaining_percent: i32,
    pub window_minutes: Option<i64>,
    pub reset_at: Option<i64>,
}

fn pelican_header_number(headers: &reqwest::header::HeaderMap, name: &str) -> Option<f64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<f64>().ok())
}

fn pelican_quota_snapshot_from_headers(
    headers: &reqwest::header::HeaderMap,
) -> Option<PelicanQuotaSnapshot> {
    let used_percent = pelican_header_number(headers, "x-codex-primary-used-percent")?;
    let used_percent = used_percent.clamp(0.0, 100.0);
    let reset_at = pelican_header_number(headers, "x-codex-primary-reset-after-seconds")
        .filter(|seconds| *seconds >= 0.0)
        .map(|seconds| chrono::Utc::now().timestamp() + seconds.round() as i64);
    Some(PelicanQuotaSnapshot {
        used_percent,
        remaining_percent: (100.0 - used_percent).round().clamp(0.0, 100.0) as i32,
        window_minutes: pelican_header_number(headers, "x-codex-primary-window-minutes")
            .filter(|minutes| *minutes > 0.0)
            .map(|minutes| minutes.round() as i64),
        reset_at,
    })
}

pub fn pelican_quota_snapshot_from_codex_quota(quota: &CodexQuota) -> Option<PelicanQuotaSnapshot> {
    if quota.hourly_window_present == Some(false) {
        return None;
    }
    let remaining_percent = quota.hourly_percentage.clamp(0, 100);
    Some(PelicanQuotaSnapshot {
        used_percent: (100 - remaining_percent) as f64,
        remaining_percent,
        window_minutes: quota.hourly_window_minutes,
        reset_at: quota.hourly_reset_time,
    })
}

#[derive(Default)]
struct PelicanSseDecoder {
    pending: Vec<u8>,
    scanned: usize,
    data: String,
    received: usize,
    reply: String,
    completed: Option<Value>,
}

impl PelicanSseDecoder {
    fn push(&mut self, bytes: &[u8], on_delta: &impl Fn(String)) -> Result<(), String> {
        self.received = self.received.saturating_add(bytes.len());
        if self.received > PELICAN_MAX_RESPONSE_BYTES {
            return Err("PELICAN_RESPONSE_TOO_LARGE".into());
        }
        self.pending.extend_from_slice(bytes);
        let mut consumed = 0;
        while let Some(relative_end) = self.pending[self.scanned..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let end = self.scanned + relative_end;
            let line = std::str::from_utf8(&self.pending[consumed..=end])
                .map_err(|_| "PELICAN_STREAM_INCOMPLETE".to_string())?
                .trim_end_matches(['\r', '\n'])
                .to_owned();
            consumed = end + 1;
            self.scanned = consumed;
            if line.is_empty() {
                self.event(on_delta)?;
            } else if let Some(value) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
            }
        }
        // Compact once per chunk, and never rescan the old part of a long line.
        self.pending.drain(..consumed);
        self.scanned = self.pending.len();
        Ok(())
    }

    fn event(&mut self, on_delta: &impl Fn(String)) -> Result<(), String> {
        let data = std::mem::take(&mut self.data);
        if data.is_empty() || data.trim() == "[DONE]" {
            return Ok(());
        }
        let event: Value =
            serde_json::from_str(&data).map_err(|_| "PELICAN_STREAM_INCOMPLETE".to_string())?;
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta" | "response.refusal.delta") => {
                if self.completed.is_none() {
                    if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                        self.reply.push_str(delta);
                        on_delta(delta.to_string());
                    }
                }
            }
            Some("response.completed") => {
                let response = event.get("response").ok_or("PELICAN_STREAM_INCOMPLETE")?;
                if response
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s != "completed")
                {
                    return Err("PELICAN_STREAM_INCOMPLETE".into());
                }
                self.completed = Some(response.clone());
            }
            Some("response.failed" | "response.incomplete" | "error") => {
                return Err("PELICAN_STREAM_INCOMPLETE".into());
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(mut self, on_delta: &impl Fn(String)) -> Result<PelicanChatOutput, String> {
        // Accept an unterminated final SSE line, but never infer success from EOF/[DONE].
        if !self.pending.is_empty() {
            self.push(b"\n", on_delta)?;
        }
        self.event(on_delta)?;
        let response = self.completed.ok_or("PELICAN_STREAM_INCOMPLETE")?;
        let final_text = pelican_final_text(&response);
        if self.reply.is_empty() && !final_text.is_empty() {
            on_delta(final_text.clone());
        }
        let reply = if final_text.is_empty() {
            self.reply
        } else {
            final_text
        };
        if reply.is_empty() {
            return Err("PELICAN_STREAM_INCOMPLETE".into());
        }
        Ok(PelicanChatOutput {
            reply,
            usage: response.get("usage").filter(|v| !v.is_null()).cloned(),
            response_id: response
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            response_model: response
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned),
            quota: None,
        })
    }
}

fn pelican_final_text(response: &Value) -> String {
    response
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|part| match part.get("type").and_then(Value::as_str) {
            Some("output_text") => part.get("text").and_then(Value::as_str),
            Some("refusal") => part.get("refusal").and_then(Value::as_str),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn pelican_read_error_body(mut response: reqwest::Response) -> Result<String, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = timeout(PELICAN_IDLE_TIMEOUT, response.chunk())
        .await
        .map_err(|_| "PELICAN_TIMEOUT".to_string())?
        .map_err(|_| "PELICAN_STREAM_INCOMPLETE".to_string())?
    {
        let remaining = 64 * 1024 - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if bytes.len() >= 64 * 1024 {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 构造鹈鹕请求：与本地 API 服务共用同一套 Codex 客户端特征，
/// 包括 prompt cache key、session/conversation 头与 client_metadata。
fn build_pelican_request(
    model: &str,
    effort: &str,
    prompt: &str,
    account_id: &str,
) -> Result<(Vec<u8>, HashMap<String, String>), String> {
    let session_id = stable_uuid_from_text(&format!("agtools:codex:pelican:{account_id}"));
    let window_id = format!("{session_id}:0");
    let installation_id =
        stable_uuid_from_text(&format!("agtools:codex:installation:{account_id}"));
    let turn_id = stable_uuid_from_text(&format!("agtools:codex:turn:{account_id}:{session_id}"));
    let turn_metadata = build_codex_turn_metadata(&session_id, &turn_id);
    let mut body_value = json!({
        "model": model,
        "input": [{"type":"message", "role":"user", "content":[{"type":"input_text", "text":prompt}]}],
        "instructions": PELICAN_DELIVERY_INSTRUCTIONS,
        "reasoning": {"effort":effort, "summary":"auto"},
        "store":false, "stream":true,
    });
    if let Some(object) = body_value.as_object_mut() {
        object.insert(
            "prompt_cache_key".to_string(),
            Value::String(session_id.clone()),
        );
        object.insert(
            "client_metadata".to_string(),
            json!({
                "x-codex-installation-id": installation_id,
                "x-codex-window-id": window_id,
                "x-codex-turn-metadata": turn_metadata,
            }),
        );
    }
    let body = serde_json::to_vec(&body_value).map_err(|e| e.to_string())?;
    let headers = HashMap::from([
        ("accept".to_string(), "text/event-stream".to_string()),
        ("content-type".to_string(), "application/json".to_string()),
        ("session-id".to_string(), session_id.clone()),
        ("conversation_id".to_string(), session_id.clone()),
        ("x-client-request-id".to_string(), session_id),
    ]);
    Ok((body, headers))
}

pub async fn run_pelican_chat(
    account_id: &str,
    model: &str,
    effort: &str,
    prompt: &str,
    cancel: watch::Receiver<bool>,
    on_delta: impl Fn(String) + Send + Sync + 'static,
) -> Result<PelicanChatOutput, String> {
    pelican_with_cancel(
        cancel,
        PELICAN_TOTAL_TIMEOUT,
        pelican_chat_inner(account_id, model, effort, prompt, on_delta),
    )
    .await
}

async fn pelican_with_cancel<T>(
    mut cancel: watch::Receiver<bool>,
    total_timeout: Duration,
    operation: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    if *cancel.borrow() {
        return Err("PELICAN_CANCELLED".into());
    }
    // Dropping the HTTP future closes the stream; no cancelled task keeps consuming
    // output locally. Upstream may already have billed tokens produced before closure.
    tokio::select! {
        biased;
        _ = cancel.changed() => Err("PELICAN_CANCELLED".into()),
        result = timeout(total_timeout, operation) => {
            result.map_err(|_| "PELICAN_TIMEOUT".to_string())?
        }
    }
}

async fn pelican_chat_inner(
    account_id: &str,
    model: &str,
    effort: &str,
    prompt: &str,
    on_delta: impl Fn(String) + Send + Sync + 'static,
) -> Result<PelicanChatOutput, String> {
    let id = account_id.to_owned();
    let runtime_handle = tokio::runtime::Handle::current();
    // Blocking disk/syscall work cannot be forcibly cancelled. Keep its permit in
    // the closure so repeated cancel/restart cannot accumulate orphan preparations.
    let preparation_permit = PELICAN_PREPARATION_SLOTS
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "pelican.error.stateUnavailable".to_string())?;
    // Existing account preparation contains synchronous local credential I/O.
    // Keep it off the async executor and away from the main-window interaction path.
    let mut account = timeout(
        Duration::from_secs(90),
        tokio::task::spawn_blocking(move || {
            let _permit = preparation_permit;
            runtime_handle.block_on(async move {
                // The inner deadline also stops detached async preparation after a caller
                // cancels. Existing per-account refresh locks prevent duplicate refreshes.
                timeout(Duration::from_secs(85), prepare_direct_codex_account(&id))
                    .await
                    .map_err(|_| "PELICAN_TIMEOUT".to_string())?
            })
        }),
    )
    .await
    .map_err(|_| "PELICAN_TIMEOUT".to_string())?
    .map_err(|e| format!("Pelican account preparation: {e}"))??;
    if account.is_api_key_auth() || account.is_web_session_auth() {
        return Err("PELICAN_UNSUPPORTED_ACCOUNT".into());
    }
    let (body, mut headers) = build_pelican_request(model, effort, prompt, account_id)?;
    for name in CODEX_OFFICIAL_EMPTY_HEADERS {
        headers
            .entry((*name).to_string())
            .or_insert_with(String::new);
    }
    if account
        .agent_identity
        .as_ref()
        .is_some_and(|identity| identity.chatgpt_account_is_fedramp)
    {
        headers.insert("x-openai-fedramp".into(), "true".into());
    }
    // Use account-bound or global/system proxy settings, never API Service settings.
    let mut timeouts = CodexLocalAccessTimeouts::default();
    // A retry after an uncertain send could generate twice. Retests are user actions.
    timeouts.upstream_send_retry_attempts = 0;
    let connect_timeout = duration_from_millis(
        timeouts.legacy_upstream_connect_timeout_ms,
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    );
    let target = resolve_upstream_target(RESPONSES_PATH)?;
    let mut expected_task_id: Option<String> = None;
    for attempt in 0..=1 {
        let response = timeout(PELICAN_IDLE_TIMEOUT, async {
            if account.is_agent_identity_auth() {
                let (updated, auth_headers, task_id) =
                    codex_agent_identity::build_authentication_headers_with_base_url(
                        &account,
                        expected_task_id.as_deref(),
                        codex_agent_identity::AGENT_IDENTITY_AUTH_API_BASE_URL,
                    )
                    .await?;
                account = updated;
                expected_task_id = Some(task_id);
                let authorization = auth_headers
                    .get(AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or("PELICAN_UNSUPPORTED_ACCOUNT")?;
                send_upstream_request_with_authorization_url(
                    "POST",
                    &format!(
                        "{}{}",
                        UPSTREAM_CODEX_BASE_URL.trim_end_matches('/'),
                        target
                    ),
                    &target,
                    &headers,
                    &body,
                    &account,
                    authorization,
                    None,
                    connect_timeout,
                    &timeouts,
                    CodexLocalAccessImageGenerationMode::Disabled,
                    CodexLocalAccessRequestKind::Text,
                )
                .await
            } else {
                send_upstream_request(
                    "POST",
                    &target,
                    &headers,
                    &body,
                    &account,
                    None,
                    connect_timeout,
                    &timeouts,
                    CodexLocalAccessImageGenerationMode::Disabled,
                    CodexLocalAccessRequestKind::Text,
                )
                .await
            }
        })
        .await
        .map_err(|_| "PELICAN_TIMEOUT".to_string())??;
        let status = response.status();
        if !status.is_success() {
            let raw = pelican_read_error_body(response).await?;
            // Only retry an explicitly rejected Agent Identity task, before generation starts.
            if attempt == 0
                && account.is_agent_identity_auth()
                && codex_agent_identity::is_task_invalid_response(status, &raw)
            {
                continue;
            }
            let safe = pelican_redact_error(&account, &raw);
            let detail =
                extract_upstream_error_message(&safe).unwrap_or_else(|| status.to_string());
            return Err(format!(
                "HTTP {}: {}",
                status.as_u16(),
                truncate_diagnostic_text(&detail, 1200)
            ));
        }
        let result = pelican_consume_response(response, PELICAN_IDLE_TIMEOUT, &on_delta).await?;
        return Ok(result);
    }
    Err("PELICAN_STREAM_INCOMPLETE".into())
}

fn pelican_redact_error(account: &CodexAccount, raw: &str) -> String {
    let mut safe = codex_agent_identity::redact_sensitive_body(account, raw);
    for secret in [
        Some(account.tokens.access_token.as_str()),
        Some(account.tokens.id_token.as_str()),
        account.tokens.refresh_token.as_deref(),
        account.openai_api_key.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|secret| !secret.is_empty())
    {
        safe = safe.replace(secret, "[redacted]");
    }
    safe
}

async fn pelican_consume_response(
    mut response: reqwest::Response,
    idle_timeout: Duration,
    on_delta: &impl Fn(String),
) -> Result<PelicanChatOutput, String> {
    let quota = pelican_quota_snapshot_from_headers(response.headers());
    let mut decoder = PelicanSseDecoder::default();
    while let Some(chunk) = timeout(idle_timeout, response.chunk())
        .await
        .map_err(|_| "PELICAN_TIMEOUT".to_string())?
        .map_err(|_| "PELICAN_STREAM_INCOMPLETE".to_string())?
    {
        for part in chunk.chunks(64 * 1024) {
            decoder.push(part, on_delta)?;
            tokio::task::yield_now().await;
        }
        if decoder.completed.is_some() {
            break;
        }
    }
    let mut output = decoder.finish(on_delta)?;
    output.quota = quota;
    Ok(output)
}

#[cfg(test)]
#[path = "codex_pelican_transport_tests.rs"]
mod pelican_transport_tests;
