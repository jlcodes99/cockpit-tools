//! Content-free request sizing; large inputs are measured in the parsing worker.
use std::{
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSize {
    /// Decoded client JSON bytes, including whitespace. Field sizes use compact JSON.
    pub total_bytes: u64,
    pub input_bytes: u64,
    pub tools_bytes: u64,
    pub instructions_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_text_characters: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponsesHistoryShape {
    pub input_items: usize,
    pub messages: usize,
    pub tool_calls: usize,
    pub tool_outputs: usize,
    pub compaction_items: usize,
    pub has_previous_response: bool,
    pub has_compaction_control: bool,
}

impl ResponsesHistoryShape {
    pub(super) fn from_body(body: &Value) -> Self {
        let mut shape = Self {
            has_previous_response: body["previous_response_id"].as_str().is_some_and(|id| !id.is_empty()),
            has_compaction_control: body["context_management"].as_array().is_some_and(|controls| {
                controls.iter().any(|control| control["type"] == "compaction")
            }),
            ..Default::default()
        };
        if let Some(items) = body["input"].as_array() {
            shape.input_items = items.len();
            for item in items {
                match item["type"].as_str() {
                    Some("compaction" | "compaction_summary" | "context_compaction") => shape.compaction_items += 1,
                    Some("message" | "agent_message") => shape.messages += 1,
                    Some("function_call_output" | "custom_tool_call_output" | "tool_search_output" | "mcp_approval_response") => shape.tool_outputs += 1,
                    Some("function_call" | "custom_tool_call" | "local_shell_call" | "shell_call" | "computer_call" | "mcp_call" | "tool_search_call" | "web_search_call" | "file_search_call" | "code_interpreter_call" | "image_generation_call") => shape.tool_calls += 1,
                    None if item["role"].is_string() => shape.messages += 1,
                    _ => {}
                }
            }
        } else if body["input"].is_string() {
            shape.input_items = 1;
            shape.messages = 1;
        }
        shape
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponsesHistoryRejection {
    pub client: ResponsesHistoryShape,
    pub upstream: ResponsesHistoryShape,
    pub compaction_request: bool,
    pub rebuilt_history: bool,
    pub recovered_continuation: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverloadRetry {
    pub retry: u32,
    pub wait_ms: u64,
    /// From the first capacity fallback until the first response bytes, or preparation.
    pub recovery_ms: u64,
}

#[derive(Default)]
struct ByteCounter(u64);
impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn text_characters(value: &Value) -> u64 {
    value.as_str().map_or(0, |text| text.chars().count() as u64)
}

fn message_content_characters(value: &Value) -> u64 {
    if let Some(parts) = value.as_array() {
        return parts.iter()
            .filter(|part| matches!(part["type"].as_str(), Some("text" | "input_text" | "output_text")))
            .map(|part| text_characters(&part["text"]))
            .sum();
    }
    text_characters(value)
}

fn message_characters(messages: &[Value]) -> u64 {
    messages.iter()
        .filter(|item| {
            matches!(item["role"].as_str(), Some("user" | "assistant" | "developer" | "system"))
                && matches!(item["type"].as_str(), None | Some("message" | "agent_message"))
        })
        .map(|item| message_content_characters(&item["content"]))
        .sum()
}

fn gemini_text_characters(content: &Value) -> u64 {
    content["parts"].as_array().into_iter().flatten()
        .filter(|part| part["thought"] != true)
        .map(|part| text_characters(&part["text"]))
        .sum()
}

fn input_text_characters(body: &Value, path: &str) -> Option<u64> {
    if !body.is_object() {
        return None;
    }
    if path.ends_with("/responses") || path.ends_with("/responses/compact") {
        let input = &body["input"];
        let input_chars = input.as_array().map_or_else(|| text_characters(input), |items| message_characters(items));
        return Some(input_chars + text_characters(&body["instructions"]));
    }
    if path.ends_with("/chat/completions") || matches!(path, "/v1/messages" | "/api/chat") {
        let chars = body["messages"].as_array().map_or(0, |messages| message_characters(messages));
        return Some(chars + if path == "/v1/messages" { message_content_characters(&body["system"]) } else { 0 });
    }
    if path.ends_with("/completions") {
        let prompt = &body["prompt"];
        return match prompt {
            Value::String(_) => Some(text_characters(prompt)),
            Value::Array(prompts) if prompts.iter().all(Value::is_string) => {
                Some(prompts.iter().map(text_characters).sum())
            }
            _ => None,
        };
    }
    if path.starts_with("/v1beta/models/")
        && (path.ends_with(":generateContent") || path.ends_with(":streamGenerateContent"))
    {
        let chars: u64 = body["contents"].as_array().into_iter().flatten().map(gemini_text_characters).sum();
        let system = body.get("systemInstruction").or_else(|| body.get("system_instruction"));
        return Some(chars + system.map_or(0, gemini_text_characters));
    }
    None
}

pub(super) fn measure(body: &Value, total_bytes: usize, path: &str) -> RequestSize {
    fn field(body: &Value, key: &str) -> u64 {
        let Some(value) = body.get(key) else { return 0 };
        let mut counter = ByteCounter::default();
        // Serializing a Value to an infallible counting writer cannot fail.
        serde_json::to_writer(&mut counter, value).expect("count JSON bytes");
        counter.0
    }
    RequestSize {
        total_bytes: total_bytes as u64,
        input_bytes: field(body, "input"),
        tools_bytes: field(body, "tools"),
        instructions_bytes: field(body, "instructions"),
        input_text_characters: input_text_characters(body, path),
    }
}

#[derive(Default)]
struct Bucket {
    requests: AtomicU64,
    failures: AtomicU64,
    attempts: AtomicU64,
    rejected_attempts: AtomicU64,
}

#[derive(Default)]
pub(super) struct SizeBuckets([Bucket; 4]);

impl SizeBuckets {
    pub(super) fn record(
        &self,
        context: &super::diagnostics::DiagnosticContext,
        success: bool,
        status: u16,
    ) {
        let Some(size) = context
            .attempts
            .first()
            .and_then(|attempt| attempt.request_size.as_ref())
        else {
            return;
        };
        // Client cancellations are not evidence of upstream admission failure.
        if status == 499 {
            return;
        }
        let upstream_bytes = context
            .attempts
            .first()
            .and_then(|attempt| attempt.upstream_request_bytes)
            .unwrap_or(size.total_bytes);
        let (index, label) = match upstream_bytes {
            0..100_000 => (0, "<100KB"),
            100_000..200_000 => (1, "100-200KB"),
            200_000..500_000 => (2, "200-500KB"),
            _ => (3, ">=500KB"),
        };
        let bucket = &self.0[index];
        let requests = bucket.requests.fetch_add(1, Ordering::Relaxed) + 1;
        let failures = bucket
            .failures
            .fetch_add(u64::from(!success), Ordering::Relaxed)
            + u64::from(!success);
        let attempts = bucket
            .attempts
            .fetch_add(context.attempts.len() as u64, Ordering::Relaxed)
            + context.attempts.len() as u64;
        let rejected = context
            .attempts
            .iter()
            .filter(|attempt| !attempt.success)
            .count() as u64;
        let rejected = bucket
            .rejected_attempts
            .fetch_add(rejected, Ordering::Relaxed)
            + rejected;
        tracing::info!(target: "codex_api", request_id=%context.request_id,
            size_bucket=label, request_bytes=size.total_bytes, upstream_request_bytes=upstream_bytes, input_bytes=size.input_bytes,
            tools_bytes=size.tools_bytes, instructions_bytes=size.instructions_bytes,
            attempts=context.attempts.len(), success, bucket_requests=requests,
            bucket_failures=failures, bucket_attempts=attempts, bucket_rejected_attempts=rejected,
            bucket_failure_rate=failures as f64 / requests as f64,
            bucket_mean_attempts=attempts as f64 / requests as f64,
            "[NativeAPI][request_size]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_history_shape_keeps_presence_and_counts_without_content() {
        let shape = ResponsesHistoryShape::from_body(&serde_json::json!({
            "previous_response_id":"private-response-reference",
            "context_management":[{"type":"compaction","compact_threshold":200000}],
            "input":[
                {"type":"compaction","encrypted_content":"private-summary"},
                {"type":"reasoning","encrypted_content":"private-reasoning"},
                {"type":"message","role":"user","content":"private-message"},
                {"type":"custom_tool_call","name":"private-name"},
                {"type":"custom_tool_call_output","output":"private-output"}
            ]
        }));
        assert_eq!(shape, ResponsesHistoryShape {
            input_items: 5, messages: 1, tool_calls: 1, tool_outputs: 1,
            compaction_items: 1, has_previous_response: true, has_compaction_control: true,
        });
        assert!(!serde_json::to_string(&shape).unwrap().contains("private-"));
    }

    #[test]
    fn sizing_counts_utf8_and_json_escaping_without_retaining_content() {
        let body = serde_json::json!({"instructions":"中文\n\"x\"", "tools":[{"name":"工具"}],
            "input":[{"role":"user","content":"秘密"}]});
        let size = measure(&body, 1234, "/v1/responses");
        assert_eq!(size.total_bytes, 1234);
        assert_eq!(
            size.input_bytes,
            serde_json::to_vec(&body["input"]).unwrap().len() as u64
        );
        assert_eq!(
            size.tools_bytes,
            serde_json::to_vec(&body["tools"]).unwrap().len() as u64
        );
        assert_eq!(
            size.instructions_bytes,
            serde_json::to_vec(&body["instructions"]).unwrap().len() as u64
        );
        assert!(!serde_json::to_string(&size).unwrap().contains("秘密"));
        assert_eq!(measure(&serde_json::json!({}), 2, "/v1/responses").input_bytes, 0);
    }

    #[test]
    fn input_text_characters_follow_each_client_protocol() {
        let responses = serde_json::json!({
            "instructions":"规则", "previous_response_id":"hidden-response-id",
            "input":[
                {"type":"message","role":"user","content":[
                    {"type":"input_text","text":"汉🙂"},
                    {"type":"input_image","image_url":"data:image/png;base64,attachment"},
                    {"type":"input_file","file_data":"file payload"}
                ]},
                {"role":"assistant","content":"é"},
                {"type":"function_call","arguments":"ignored arguments"},
                {"type":"function_call_output","output":"ignored result"}
            ],
            "tools":[{"name":"tool","description":"ignored schema"}]
        });
        assert_eq!(measure(&responses, 0, "/v1/responses").input_text_characters, Some(6));
        assert_eq!(measure(&serde_json::json!({"input":[{"role":"user","content":[{"type":"input_image","image_url":"abc"}]}]}), 0, "/responses").input_text_characters, Some(0));

        let chat = serde_json::json!({"messages":[
            {"role":"user","content":"hé"},
            {"role":"assistant","content":"好"},
            {"role":"tool","content":"ignored"}
        ], "system":"ignored"});
        assert_eq!(measure(&chat, 0, "/v1/chat/completions").input_text_characters, Some(3));
        assert_eq!(measure(&chat, 0, "/api/chat").input_text_characters, Some(3));

        let claude = serde_json::json!({"system":[{"type":"text","text":"rules"}],
            "messages":[{"role":"user","content":[
                {"type":"text","text":"你好"},
                {"type":"tool_result","content":"ignored"},
                {"type":"image","source":{"data":"ignored"}}
            ]}]});
        assert_eq!(measure(&claude, 0, "/v1/messages").input_text_characters, Some(7));

        let gemini = serde_json::json!({
            "system_instruction":{"parts":[{"text":"甲乙"}]},
            "contents":[{"parts":[{"text":"🦀"},{"text":"ignored","thought":true},{"inlineData":{"data":"ignored"}}]}]
        });
        assert_eq!(measure(&gemini, 0, "/v1beta/models/example:streamGenerateContent").input_text_characters, Some(3));
        assert_eq!(measure(&serde_json::json!({"prompt":["a","中"]}), 0, "/v1/completions").input_text_characters, Some(2));
        assert_eq!(measure(&serde_json::json!({"prompt":[1,2]}), 0, "/v1/completions").input_text_characters, None);
        assert_eq!(measure(&serde_json::json!({"input":"text"}), 0, "/v1/embeddings").input_text_characters, None);
    }

    #[test]
    fn buckets_distinguish_recovered_attempts_final_failures_and_cancellations() {
        use super::super::diagnostics::{DiagnosticAttempt, DiagnosticContext};
        let buckets = SizeBuckets::default();
        let context = DiagnosticContext {
            attempts: vec![
                DiagnosticAttempt {
                    request_size: Some(RequestSize {
                        total_bytes: 500_000,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                DiagnosticAttempt {
                    success: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        buckets.record(&context, true, 200);
        buckets.record(&context, false, 503);
        buckets.record(&context, false, 499);
        assert_eq!(buckets.0[3].requests.load(Ordering::Relaxed), 2);
        assert_eq!(buckets.0[3].failures.load(Ordering::Relaxed), 1);
        assert_eq!(buckets.0[3].attempts.load(Ordering::Relaxed), 4);
        assert_eq!(buckets.0[3].rejected_attempts.load(Ordering::Relaxed), 2);
        assert_eq!(buckets.0[2].requests.load(Ordering::Relaxed), 0);
        let mut rewritten = context;
        rewritten.attempts[0].upstream_request_bytes = Some(99_999);
        buckets.record(&rewritten, true, 200);
        assert_eq!(
            buckets.0[0].requests.load(Ordering::Relaxed),
            1,
            "bucket by the bytes actually sent upstream"
        );
    }
}
