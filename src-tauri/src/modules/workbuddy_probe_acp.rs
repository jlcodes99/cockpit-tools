use super::truncate_chars;
use serde_json::Value;

/// JSON-RPC 请求 id：initialize=1, session/load=2, session/set_model=3, session/prompt=4
const INIT_ID: u64 = 1;
const LOAD_ID: u64 = 2;
const SET_MODEL_ID: u64 = 3;
const PROMPT_ID: u64 = 4;

/// 归一化模型名用于比对：小写且只保留字母数字（hy4free ↔ hy4-free）。
fn normalized_model_name(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PromptResult {
    Pending,
    Acknowledged,
    Finished,
}

pub(super) struct AcpOutcome {
    pub(super) text: String,
    pub(super) error: Option<String>,
    prompt_result: PromptResult,
    /// 本次探测请求的模型名（用于和 prompt 结果 usage.model 比对，防 set_model 失效）
    pub(super) expected_model: String,
}

impl AcpOutcome {
    pub(super) fn new(expected_model: &str) -> Self {
        Self {
            text: String::new(),
            error: None,
            prompt_result: PromptResult::Pending,
            expected_model: expected_model.to_string(),
        }
    }

    pub(super) fn has_prompt_result(&self) -> bool {
        self.prompt_result != PromptResult::Pending
    }
}

/// 读取 ACP SSE 流：累积 agent_message_chunk 的文本，捕获错误事件与限流文案。
/// 统一的限流文案判定（服务端文案可能只有 "429 超出频率限制"，不含 quota 等字样）。
pub(super) fn looks_rate_limited(text: &str) -> bool {
    let low = text.to_lowercase();
    text.contains("上限")
        || text.contains("限流")
        || text.contains("额度")
        || text.contains("频率限制")
        || text.contains("使用量已超出")
        || text.contains("429")
        || low.contains("quota")
        || low.contains("rate limit")
        || low.contains("exceed")
        || low.contains("too many")
}

/// 扫描 ACP POST 响应（JSON 或 SSE 行）里的错误/限流文案。
/// 限流通常在 `session/prompt` 的响应里，包在
/// `result._meta["codebuddy.ai/errorMessage"]` 的嵌套 JSON 字符串中。
pub(super) fn parse_acp_post_error(body: &str) -> Option<String> {
    for line in body.lines() {
        let payload = line.strip_prefix("data:").unwrap_or(line).trim();
        if payload.is_empty() || payload.starts_with(':') || payload == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
            // 优先取客户端同款的错误展示文案（嵌套 JSON 字符串）。
            if let Some(raw) = value
                .get("result")
                .and_then(|r| r.get("_meta"))
                .and_then(|m| m.get("codebuddy.ai/errorMessage"))
                .and_then(|v| v.as_str())
            {
                let cleaned = serde_json::from_str::<serde_json::Value>(raw)
                    .ok()
                    .and_then(|inner| {
                        inner
                            .get("message")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| raw.to_string());
                return Some(cleaned);
            }
            if let Some(err) = value.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or(payload)
                    .to_string();
                return Some(msg);
            }
            // 业务错误码：code != 0 且带 message/msg
            let code = match value.get("code") {
                None => 0,
                Some(code) => match code
                    .as_i64()
                    .or_else(|| code.as_str().and_then(|code| code.parse().ok()))
                {
                    Some(code) => code,
                    None => return Some("WORKBUDDY_PROBE_INVALID_RESPONSE".into()),
                },
            };
            let msg = value
                .get("message")
                .or_else(|| value.get("msg"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if code != 0 {
                return Some(format!("code={code} {msg}"));
            }
        }
    }
    None
}

pub(super) async fn read_acp_reply(
    client: &reqwest::Client,
    api_url: &str,
    session_token: &str,
    session_id: &str,
    model: &str,
    enterprise_id: Option<&str>,
    timeout_secs: u64,
) -> AcpOutcome {
    use futures_util::StreamExt;

    let mut outcome = AcpOutcome::new(model);

    let mut headers = reqwest::header::HeaderMap::new();
    if let Ok(value) = format!("Bearer {}", session_token).parse() {
        headers.insert("Authorization", value);
    }
    headers.insert(
        "Accept",
        reqwest::header::HeaderValue::from_static("text/event-stream"),
    );
    if let Some(eid) = enterprise_id {
        if let Ok(value) = eid.parse() {
            headers.insert("X-Enterprise-Id", value);
        }
    }

    let base = api_url.trim_end_matches('/').to_string();
    let mut attempts: Vec<(String, bool)> = Vec::new();
    if base.ends_with("/acp") {
        attempts.push((base.clone(), false));
    }
    attempts.push((format!("{}/acp", base), false));
    attempts.push((format!("{}/acp/bootstrap", base), true));

    let deadline = std::time::Duration::from_secs(timeout_secs);

    let read_future = async {
        let mut resp = None;
        let mut last_err = String::new();
        for (url, is_post) in attempts {
            let request = if is_post {
                client
                    .post(&url)
                    .headers(headers.clone())
                    .header("Content-Type", "application/json")
                    .body("{}")
            } else {
                client.get(&url).headers(headers.clone())
            };
            match request.send().await {
                Ok(r) => {
                    let status = r.status().as_u16();
                    if (200..300).contains(&status) {
                        resp = Some(r);
                        break;
                    }
                    if status == 404 || status == 405 {
                        last_err = format!("{} -> HTTP {}", url, status);
                        continue;
                    }
                    let body = r.text().await.unwrap_or_default();
                    outcome.error = Some(format!(
                        "ACP HTTP {}: {}",
                        status,
                        truncate_chars(&body, 300)
                    ));
                    return;
                }
                Err(e) => {
                    last_err = format!("ACP: {}", e.without_url());
                }
            }
        }
        let Some(resp) = resp else {
            outcome.error = Some(format!("ACP: {last_err}"));
            return;
        };

        // ACP 连接 ID：后续 JSON-RPC 请求要带上。
        let connection_id = resp
            .headers()
            .get("Acp-Connection-Id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let acp_url = resp.url().to_string();
        // 按 ACP 协议握手并发送 prompt（与客户端 AcpClient 相同的方法序列）。
        let send = |method: &'static str, id: u64, params: serde_json::Value| {
            let client = client.clone();
            let url = acp_url.clone();
            let mut h = headers.clone();
            h.insert("Content-Type", "application/json".parse().unwrap());
            h.insert(
                "Accept",
                "application/json, text/event-stream".parse().unwrap(),
            );
            if let Some(cid) = &connection_id {
                if let Ok(v) = cid.parse() {
                    h.insert("Acp-Connection-Id", v);
                }
            }
            async move {
                let body = serde_json::json!({
                    "jsonrpc": "2.0", "id": id, "method": method, "params": params
                });
                match client.post(&url).headers(h).json(&body).send().await {
                    Ok(r) => {
                        let status = r.status().as_u16();
                        let response_text = r.text().await.unwrap_or_default();
                        if !(200..300).contains(&status) {
                            return Err(format!(
                                "{} HTTP {}: {}",
                                method,
                                status,
                                truncate_chars(&response_text, 200)
                            ));
                        }
                        Ok(response_text)
                    }
                    Err(e) => Err(format!("{method}: {}", e.without_url())),
                }
            }
        };

        // The backend may acknowledge setup requests with an empty HTTP 200
        // and only send the prompt result on the event stream.
        let mut stream = resp.bytes_stream();
        let mut decoder = ReplyDecoder::default();
        let calls: [(&'static str, u64, serde_json::Value); 4] = [
            (
                "initialize",
                INIT_ID,
                serde_json::json!({ "protocolVersion": 1, "clientCapabilities": {} }),
            ),
            (
                "session/load",
                LOAD_ID,
                serde_json::json!({ "sessionId": session_id, "cwd": "/", "mcpServers": [] }),
            ),
            (
                "session/set_model",
                SET_MODEL_ID,
                serde_json::json!({ "sessionId": session_id, "modelId": model }),
            ),
            (
                "session/prompt",
                PROMPT_ID,
                serde_json::json!({
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": "Reply with OK." }]
                }),
            ),
        ];
        for (method, id, params) in calls {
            let body = match send(method, id, params).await {
                Ok(body) => body,
                Err(error) => {
                    outcome.error = Some(error);
                    return;
                }
            };
            if let Err(error) = ReplyDecoder::decode_post(&body, session_id, &mut outcome) {
                outcome.error = Some(format!("{method}: {error}"));
                return;
            }

            if outcome.error.is_some() || (id == PROMPT_ID && !outcome.text.trim().is_empty()) {
                return;
            }
        }

        // Read through the final result even after a text chunk: the final reply
        // can report a quota error or that a different model actually ran.
        while outcome.error.is_none()
            && outcome.prompt_result != PromptResult::Finished
            && !(outcome.has_prompt_result() && !outcome.text.trim().is_empty())
        {
            let decoded = match stream.next().await {
                Some(Ok(bytes)) => decoder.feed(&bytes, false, session_id, &mut outcome),
                Some(Err(error)) => Err(error.without_url().to_string()),
                None => {
                    if let Err(error) = decoder.feed(&[], true, session_id, &mut outcome) {
                        outcome.error = Some(error);
                    }
                    break;
                }
            };
            if let Err(error) = decoded {
                outcome.error = Some(error);
            }
        }
    };

    if tokio::time::timeout(deadline, read_future).await.is_err()
        && outcome.error.is_none()
        && !outcome.has_prompt_result()
        && outcome.text.trim().is_empty()
    {
        outcome.error = Some("WORKBUDDY_PROBE_TIMEOUT".into());
    }
    if let Some(error) = &mut outcome.error {
        *error = error.replace(session_token, "<redacted>");
    }
    outcome
}

#[derive(Default)]
struct ReplyDecoder {
    buffer: Vec<u8>,
    data: Vec<String>,
}

impl ReplyDecoder {
    fn decode_post(body: &str, session: &str, outcome: &mut AcpOutcome) -> Result<(), String> {
        let body = body.trim();
        if body.is_empty() {
            return Ok(());
        }
        if let Ok(value) = serde_json::from_str::<Value>(body) {
            Self::update(&value, session, outcome);
            return Ok(());
        }
        Self::default().feed(body.as_bytes(), true, session, outcome)
    }

    fn feed(
        &mut self,
        bytes: &[u8],
        end: bool,
        session: &str,
        outcome: &mut AcpOutcome,
    ) -> Result<(), String> {
        // A minimal probe has no legitimate unbounded SSE event. Keep raw bytes
        // until the complete line arrives, including a split UTF-8 code point.
        const MAX_EVENT_BYTES: usize = 1024 * 1024;
        if self.buffer.len().saturating_add(bytes.len()) > MAX_EVENT_BYTES {
            return Err("WORKBUDDY_PROBE_INVALID_RESPONSE".into());
        }
        self.buffer.extend_from_slice(bytes);
        if end
            && !self.buffer.ends_with(
                b"
",
            )
        {
            self.buffer.push(b'\n');
        }
        while let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
            let line = self.buffer.drain(..=pos).collect::<Vec<_>>();
            let text = std::str::from_utf8(&line)
                .map_err(|_| "WORKBUDDY_PROBE_INVALID_RESPONSE")?
                .trim_end_matches(['\r', '\n']);
            if text.is_empty() {
                self.flush(session, outcome)?;
            } else if let Some(data) = text.strip_prefix("data:") {
                if self.data.iter().map(String::len).sum::<usize>() + data.len() > MAX_EVENT_BYTES {
                    return Err("WORKBUDDY_PROBE_INVALID_RESPONSE".into());
                }
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).to_string());
            } else if text.starts_with('{') {
                let value =
                    serde_json::from_str(text).map_err(|_| "WORKBUDDY_PROBE_INVALID_RESPONSE")?;
                Self::update(&value, session, outcome);
            }
        }
        if end {
            self.flush(session, outcome)?;
        }
        Ok(())
    }

    fn flush(&mut self, session: &str, outcome: &mut AcpOutcome) -> Result<(), String> {
        let data = std::mem::take(&mut self.data).join(
            "
",
        );
        if data.is_empty() || data == "[DONE]" {
            return Ok(());
        }
        let value = serde_json::from_str(&data).map_err(|_| "WORKBUDDY_PROBE_INVALID_RESPONSE")?;
        Self::update(&value, session, outcome);
        Ok(())
    }

    fn update(value: &Value, session: &str, outcome: &mut AcpOutcome) {
        // JSON-RPC 错误直接失败，prompt 的终态结果用于校验实际模型。
        // id 可能是数字或字符串，两种都接受。
        if value.get("id").is_some()
            && (value.get("result").is_some() || value.get("error").is_some())
        {
            let id = match &value["id"] {
                Value::Number(number) => number.as_u64(),
                Value::String(text) => text.parse::<u64>().ok(),
                _ => None,
            };
            if value.get("error").is_some() {
                let message = parse_acp_post_error(&value.to_string())
                    .or_else(|| value["error"]["message"].as_str().map(str::to_string))
                    .unwrap_or_else(|| "WORKBUDDY_PROBE_REJECTED".into());
                outcome.error = Some(message);
                return;
            }
            if id == Some(PROMPT_ID) {
                outcome.prompt_result = if value
                    .pointer("/result/stopReason")
                    .and_then(Value::as_str)
                    .is_some()
                {
                    PromptResult::Finished
                } else {
                    // Some backends send a bare acknowledgement before the text update.
                    PromptResult::Acknowledged
                };
                // prompt 结果内嵌的限流/错误信息（result._meta.errorMessage）
                if let Some(err) = parse_acp_post_error(&value.to_string()) {
                    outcome.error = Some(err);
                    return;
                }
                // 服务端在 usage.model 里报告实际执行的模型。
                // set_model 若被静默忽略，这里会暴露成「跑了别的模型」。
                let ran = value
                    .pointer("/result/usage/model")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !ran.is_empty() && !outcome.expected_model.is_empty() {
                    let (ran_n, want_n) = (
                        normalized_model_name(ran),
                        normalized_model_name(&outcome.expected_model),
                    );
                    if !ran_n.is_empty() && !want_n.is_empty() && ran_n != want_n {
                        outcome.error =
                            Some(format!("WORKBUDDY_PROBE_MODEL_MISMATCH (server ran {ran})"));
                    }
                }
            }
            return;
        }
        if let Some(error) = parse_acp_post_error(&value.to_string()) {
            outcome.error = Some(error);
            return;
        }
        if value
            .pointer("/params/sessionId")
            .and_then(Value::as_str)
            .is_some_and(|id| id != session)
        {
            return;
        }
        let Some(update) = value
            .pointer("/params/update")
            .or_else(|| value.get("update"))
        else {
            return;
        };
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk" | "agentMessageChunk" | "agent_message") => {
                if update["content"]["type"] == "text" {
                    if let Some(text) = update["content"]["text"].as_str() {
                        outcome.text.push_str(text);
                    }
                }
            }
            Some("error") => {
                outcome.error = Some(
                    update["message"]
                        .as_str()
                        .unwrap_or("WORKBUDDY_PROBE_REJECTED")
                        .to_string(),
                )
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_accepts_empty_handshake_responses_and_checks_prompt_result() {
        use axum::{body::Body, http::header::CONTENT_TYPE, response::IntoResponse, routing::get};
        use futures_util::StreamExt;
        use std::sync::{Arc, Mutex};

        let success =
            r#"{"id":4,"result":{"stopReason":"end_turn","usage":{"model":"probe-model"}}}"#;
        for (text_before_result, prompt_result, expected_error) in [
            (false, success, None),
            (true, success, None),
            (
                true,
                r#"{"id":4,"result":{"stopReason":"end_turn","usage":{"model":"different"}}}"#,
                Some("WORKBUDDY_PROBE_MODEL_MISMATCH"),
            ),
            (
                true,
                r#"{"id":4,"error":{"code":-32000,"message":"429 quota exhausted"}}"#,
                Some("429 quota exhausted"),
            ),
        ] {
            let prompt_sent = Arc::new(tokio::sync::Notify::new());
            let stream_ready = Arc::clone(&prompt_sent);
            let calls = Arc::new(Mutex::new(Vec::new()));
            let posted_calls = Arc::clone(&calls);
            let app = axum::Router::new().route(
                "/acp",
                get(move || {
                    let stream_ready = Arc::clone(&stream_ready);
                    async move {
                        let text = "data: {\"params\":{\"sessionId\":\"probe-session\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"OK\"}}}}\n\n";
                        let updates = futures_util::stream::once(async move {
                            stream_ready.notified().await;
                            Ok::<_, std::io::Error>(if text_before_result { text } else { "" }.to_string())
                        }).chain(futures_util::stream::once(async move {
                            Ok(format!("data: {prompt_result}\n\n"))
                        }));
                        ([(CONTENT_TYPE, "text/event-stream")], Body::from_stream(updates)).into_response()
                    }
                }).post(move |axum::Json(request): axum::Json<Value>| {
                    let method = request["method"].as_str().unwrap();
                    posted_calls.lock().unwrap().push(method.to_string());
                    if method == "session/prompt" {
                        prompt_sent.notify_one();
                    }
                    async { axum::http::StatusCode::OK }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let result = read_acp_reply(
                &client,
                &url,
                "synthetic-token",
                "probe-session",
                "probe-model",
                None,
                2,
            )
            .await;
            server.abort();
            assert_eq!(
                *calls.lock().unwrap(),
                [
                    "initialize",
                    "session/load",
                    "session/set_model",
                    "session/prompt"
                ]
            );
            if let Some(expected) = expected_error {
                assert!(
                    result
                        .error
                        .as_deref()
                        .is_some_and(|error| error.contains(expected)),
                    "{:?}",
                    result.error
                );
            } else {
                assert!(result.error.is_none(), "{:?}", result.error);
                assert!(result.has_prompt_result());
                assert_eq!(result.text, if text_before_result { "OK" } else { "" });
            }
        }
    }

    #[test]
    fn reply_decoder_handles_split_utf8_multiline_events_and_ignores_other_sessions() {
        let text = "data: {\"params\":{\"sessionId\":\"chosen\",\ndata: \"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"可用\"}}}}\n\n";
        let bytes = text.as_bytes();
        let split = bytes.iter().position(|byte| *byte >= 128).unwrap() + 1;
        let mut result = AcpOutcome::new("chosen");
        let mut decoder = ReplyDecoder::default();
        decoder
            .feed(&bytes[..split], false, "chosen", &mut result)
            .unwrap();
        decoder
            .feed(&bytes[split..], false, "chosen", &mut result)
            .unwrap();
        assert_eq!(result.text, "可用");
        let mut other = AcpOutcome::new("chosen");
        ReplyDecoder::default()
            .feed(bytes, true, "other", &mut other)
            .unwrap();
        assert!(other.text.is_empty());
    }

    #[test]
    fn quota_requires_error_evidence() {
        assert!(parse_acp_post_error(r#"{"id":3,"result":{"model":"quota-model"}}"#).is_none());
        let body = r#"{"id":4,"result":{"_meta":{"codebuddy.ai/errorMessage":"{\"message\":\"quota exhausted\"}"}}}"#;
        assert!(parse_acp_post_error(body).is_some());
    }

    #[test]
    fn prompt_result_reports_usage_model_and_flags_mismatch() {
        // 真实后端形态：POST 空响应体，SSE 流只推 id=4 的最终 result（含 usage.model）
        let mut outcome = AcpOutcome::new("hy4free");
        ReplyDecoder::default()
            .feed(
                br#"data: {"jsonrpc":"2.0","id":4,"result":{"_meta":{"traceparent":"00-x"},"stopReason":"end_turn","usage":{"model":"hy4-free","outputTokens":18}}}"#,
                true,
                "s1",
                &mut outcome,
            )
            .unwrap();
        assert!(outcome.error.is_none(), "alias naming must not fail");
        assert!(outcome.has_prompt_result());

        let mut mismatch = AcpOutcome::new("deepseek-v4.1-flash");
        ReplyDecoder::default()
            .feed(
                br#"data: {"jsonrpc":"2.0","id":4,"result":{"stopReason":"end_turn","usage":{"model":"hy3","outputTokens":5}}}"#,
                true,
                "s1",
                &mut mismatch,
            )
            .unwrap();
        assert!(mismatch
            .error
            .unwrap_or_default()
            .starts_with("WORKBUDDY_PROBE_MODEL_MISMATCH"));
    }

    #[test]
    fn prompt_error_response_surfaces_message() {
        let payload = r#"data: {"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"您的使用量已超出频率限制，将在 2026-11-13T18:00:00+08:00 重置"}}"#;
        let mut outcome = AcpOutcome::new("hy4free");
        ReplyDecoder::default()
            .feed(payload.as_bytes(), true, "s1", &mut outcome)
            .unwrap();
        let message = outcome.error.unwrap_or_default();
        assert!(message.contains("频率限制"));
    }
}
