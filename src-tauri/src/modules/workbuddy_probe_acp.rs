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

/// 读流结束后的收尾判定：
/// - 已有错误/回复文本：以它们为准（错误可能来自限流，文本说明模型已实际回复）。
/// - prompt 正常结束（end_turn）：视为可用；模型实际执行者已由 update() 与
///   `usage.model` 比对过（防 set_model 静默失效）。
/// - prompt 未完成且无任何文本：若连 initialize/session/load 的 ack 都没有，
///   说明连接根本没建立成功 → 握手不完整；否则交给调用方按 NO_REPLY 处理。
fn finalize_outcome(outcome: &mut AcpOutcome) {
    if outcome.error.is_some() || !outcome.text.trim().is_empty() || outcome.prompt_done {
        return;
    }
    let session_ready =
        outcome.ack_ids.contains(&INIT_ID) && outcome.ack_ids.contains(&LOAD_ID);
    if !session_ready {
        outcome.error = Some(
            "WORKBUDDY_PROBE_HANDSHAKE_INCOMPLETE (no ack for initialize/session/load)".into(),
        );
    }
}

pub(super) struct AcpOutcome {
    pub(super) text: String,
    pub(super) error: Option<String>,
    /// 已确认收到成功响应的 JSON-RPC 请求 id（响应可能在 POST 内联，也可能在 SSE 流里）
    pub(super) ack_ids: Vec<u64>,
    /// session/prompt 的最终响应是否已收到（一轮对话结束）
    pub(super) prompt_done: bool,
    /// 本次探测请求的模型名（用于和 prompt 结果 usage.model 比对，防 set_model 失效）
    pub(super) expected_model: String,
}

impl AcpOutcome {
    pub(super) fn new(expected_model: &str) -> Self {
        Self {
            text: String::new(),
            error: None,
            ack_ids: Vec::new(),
            prompt_done: false,
            expected_model: expected_model.to_string(),
        }
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

        // 客户端 bootstrap 序列：initialize → session/load → session/set_model → session/prompt。
        // 少了 set_model，沙箱会用默认模型跑，探测结果就与所选模型无关（永远 OK）。
        //
        // 注意：真实后端对 JSON-RPC POST 返回 200 + 空响应体（受理），
        // 成功/错误响应统一走 SSE 流；个别部署也可能内联返回。
        // 因此这里只检查内联错误，不强制内联 ack——握手确认在读流阶段按 id 核对。
        let mut inline_bodies: Vec<String> = Vec::new();
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
            match send(method, id, params).await {
                Ok(response_text) => {
                    if let Some(err) = parse_acp_post_error(&response_text) {
                        outcome.error = Some(format!("{}: {}", method, err));
                        return;
                    }
                    // Require the model-selection acknowledgment before
                    // sending a prompt. Prompt replies may be direct SSE events.
                    if method != "session/prompt" && !rpc_acknowledged(&response_text, id) {
                        outcome.error = Some(format!(
                            "WORKBUDDY_PROBE_HANDSHAKE_INCOMPLETE ({}): {}",
                            method,
                            truncate_chars(response_text.trim(), 220)
                        ));
                        return;
                    }
                    inline_bodies.push(response_text);
                }
                Err(e) => {
                    outcome.error = Some(e);
                    return;
                }
            }
        }

        let mut decoder = ReplyDecoder::default();
        // 内联响应体也要过一遍解码器：登记 ack_id、解析 _meta 错误与内联更新。
        for body in &inline_bodies {
            if body.trim().is_empty() {
                continue;
            }
            if let Err(error) = decoder.feed(body.as_bytes(), true, session_id, &mut outcome) {
                outcome.error = Some(error);
                return;
            }
        }
        if outcome.error.is_some() {
            return;
        }
        // Some servers return prompt updates in the POST body while the GET
        // channel stays idle. Decode those updates before awaiting another byte.
        if outcome.prompt_done || !outcome.text.trim().is_empty() {
            finalize_outcome(&mut outcome);
            return;
        }
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if let Err(error) = decoder.feed(&bytes, false, session_id, &mut outcome) {
                        outcome.error = Some(error);
                        return;
                    }
                }
                Err(error) => {
                    outcome.error = Some(error.without_url().to_string());
                    return;
                }
            }
            if outcome.error.is_some() || outcome.prompt_done || !outcome.text.trim().is_empty() {
                break;
            }
        }
        if let Err(error) = decoder.feed(&[], true, session_id, &mut outcome) {
            outcome.error = Some(error);
        }
        finalize_outcome(&mut outcome);
    };

    if tokio::time::timeout(deadline, read_future).await.is_err() {
        outcome.error = Some("WORKBUDDY_PROBE_TIMEOUT".into());
    }
    if let Some(error) = &mut outcome.error {
        *error = error.replace(session_token, "<redacted>");
    }
    outcome
}

fn rpc_acknowledged(body: &str, id: u64) -> bool {
    let matches = |value: &Value| {
        (value["id"] == id || value["id"].as_str().is_some_and(|s| s == id.to_string()))
            && value.get("result").is_some()
    };
    // 整段解析：兼容 pretty-print 的 JSON（逐行解析会失败）
    if let Ok(value) = serde_json::from_str::<Value>(body.trim()) {
        if matches(&value) {
            return true;
        }
    }
    // 逐行解析：兼容 SSE（data: {...}）与多事件流
    body.lines().any(|line| {
        let text = line.strip_prefix("data:").unwrap_or(line).trim();
        serde_json::from_str::<Value>(text).is_ok_and(|value| matches(&value))
    })
}

#[derive(Default)]
struct ReplyDecoder {
    buffer: Vec<u8>,
    data: Vec<String>,
}

impl ReplyDecoder {
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
        // JSON-RPC 响应（带 id + result/error）：登记握手确认；错误响应直接失败。
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
                    .or_else(|| {
                        value["error"]["message"].as_str().map(str::to_string)
                    })
                    .unwrap_or_else(|| "WORKBUDDY_PROBE_REJECTED".into());
                outcome.error = Some(message);
                return;
            }
            if let Some(id) = id {
                if !outcome.ack_ids.contains(&id) {
                    outcome.ack_ids.push(id);
                }
                if id == PROMPT_ID {
                    outcome.prompt_done = true;
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
                        if !ran_n.is_empty()
                            && !want_n.is_empty()
                            && ran_n != want_n
                        {
                            outcome.error = Some(format!(
                                "WORKBUDDY_PROBE_MODEL_MISMATCH (server ran {ran})"
                            ));
                        }
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
    fn model_selection_requires_its_own_ack_and_quota_requires_error_evidence() {
        assert!(rpc_acknowledged(
            r#"{"jsonrpc":"2.0","id":3,"result":{}}"#,
            3
        ));
        assert!(!rpc_acknowledged(r#"{"id":2,"result":{}}"#, 3));
        assert!(!rpc_acknowledged(
            r#"{"id":3,"error":{"message":"unsupported"}}"#,
            3
        ));
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
        assert!(outcome.prompt_done);

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

    #[test]
    fn stream_without_any_ack_is_handshake_incomplete() {
        // POST 全空响应体 + SSE 无任何响应 → 握手不完整
        let mut outcome = AcpOutcome::new("hy4free");
        ReplyDecoder::default().feed(b"", true, "s1", &mut outcome).unwrap();
        assert!(!outcome.prompt_done);
        assert!(outcome.text.is_empty());
        assert!(outcome.ack_ids.is_empty());
    }
}
