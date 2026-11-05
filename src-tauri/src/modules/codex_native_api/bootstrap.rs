//! Inspect only the bounded pre-output handshake. Never replay generated output.
use crate::modules::codex_native_api::deadline::{self, Deadline};
use super::{
    contract::ExecutionPolicy, contract::ServiceError, events::SseDecoder,
    lifecycle::stopping_error, server::AppState,
};
use axum::http::{header, StatusCode};
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) const MAX_EVENTS: usize = 16;
pub(super) const MAX_BYTES: usize = 64 << 10;
// A peer may wait for an interrupt after response.created. Never hide its ID
// indefinitely while waiting for a second event.
pub(super) const MAX_HOLD: std::time::Duration = std::time::Duration::from_millis(250);

/// Only known metadata with no model output or tool execution can be replayed.
pub(super) fn handshake(event: &Value) -> bool {
    matches!(event.get("type").and_then(Value::as_str), Some(
        "response.created" | "response.queued" | "response.in_progress" | "keepalive" | "codex.rate_limits"
        | "codex.response.metadata" | "response.metadata"
    )) && pre_execution(event) || empty_output_event(event)
}

fn empty_output_event(event: &Value) -> bool {
    fn empty_part(part: &Value) -> bool {
        let field = match part["type"].as_str() {
            Some("output_text" | "summary_text") => "text",
            Some("refusal") => "refusal",
            _ => return false,
        };
        if !part.get(field).is_none_or(|text| text == "") { return false; }
        let mut proof = part.clone();
        proof.as_object_mut().unwrap().remove(field);
        pre_execution(&proof)
    }
    let skeleton = match event["type"].as_str() {
        Some("response.content_part.added" | "response.content_part.done" |
            "response.reasoning_summary_part.added" | "response.reasoning_summary_part.done") =>
            event.get("part").filter(|part| empty_part(part)).map(|_| "part"),
        Some("response.output_item.added") => {
            let item = &event["item"];
            let empty = match item["type"].as_str() {
                Some("message") => item.get("content").and_then(Value::as_array).is_some_and(|parts| parts.iter().all(empty_part)),
                Some("reasoning") => item.get("encrypted_content").is_none_or(|value| value.is_null() || value == "")
                    && item.get("summary").and_then(Value::as_array).is_some_and(|parts| parts.iter().all(empty_part)),
                Some("function_call") => item.get("arguments").is_some_and(|value| value == ""),
                Some("custom_tool_call") => item.get("input").is_some_and(|value| value == ""),
                _ => false,
            };
            let mut proof = item.clone();
            if empty {
                let fields: &[&str] = match item["type"].as_str() {
                    Some("message") => &["content"],
                    Some("reasoning") => &["summary", "encrypted_content"],
                    Some("function_call") => &["arguments"],
                    Some("custom_tool_call") => &["input"],
                    _ => &[],
                };
                for field in fields { proof.as_object_mut().unwrap().remove(*field); }
            }
            (empty && pre_execution(&proof)).then_some("item")
        },
        _ => None,
    };
    if let Some(field) = skeleton {
        let mut metadata = event.clone();
        metadata.as_object_mut().unwrap().remove(field);
        return pre_execution(&metadata);
    }
    let field = match event["type"].as_str() {
        Some("response.output_text.delta" | "response.reasoning_summary_text.delta" |
            "response.reasoning_text.delta" | "response.function_call_arguments.delta" |
            "response.custom_tool_call_input.delta" | "response.refusal.delta") => "delta",
        Some("response.output_text.done" | "response.reasoning_summary_text.done" |
            "response.reasoning_text.done") => "text",
        Some("response.refusal.done") => "refusal",
        _ => return false,
    };
    if event.get(field) != Some(&Value::String(String::new())) { return false; }
    let mut metadata = event.clone();
    metadata.as_object_mut().unwrap().remove(field);
    pre_execution(&metadata)
}

/// The visibility timer does not certify execution. Keep a separate bounded
/// replay fence after exposing metadata needed by interrupt-capable clients.
pub(super) struct Fence { active: bool, events: usize, bytes: usize }
impl Fence {
    pub(super) fn new(enabled: bool) -> Self { Self { active:enabled,events:0,bytes:0 } }
    pub(super) fn active(&self) -> bool { self.active }
    pub(super) fn observe(&mut self, event: Option<&Value>, bytes: usize, grok: bool) -> Option<(u16,Value)> {
        if !self.active { return None; }
        self.bytes=self.bytes.saturating_add(bytes);
        if self.bytes>=MAX_BYTES { self.active=false;return None; }
        let Some(event)=event else {return None};
        if handshake(event) {
            self.events+=1;
            if self.events>=MAX_EVENTS {self.active=false;}
            return None;
        }
        self.active=false;
        if !pre_execution(event) {return None;}
        grok.then(||super::grok_errors::rejection(event)).flatten().or_else(||rejection(event))
            .or_else(||super::encrypted_reasoning::rejection(event))
    }
}

/// Structured input faults take precedence over contradictory capacity hints.
pub(super) fn rejection(event: &Value) -> Option<(u16, Value)> {
    if event["type"] == "response.incomplete"
        && event
            .pointer("/response/incomplete_details/reason")
            .and_then(Value::as_str)
            != Some("interrupted")
        && event
            .pointer("/response/output")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
        && event
            .pointer("/response/usage/output_tokens")
            .and_then(Value::as_u64)
            == Some(0)
    {
        return Some((
            502,
            json!({"error":{"code":"empty_incomplete_stream",
            "message":"Upstream terminated with an incomplete empty response (0 tokens)"}}),
        ));
    }
    if !matches!(
        event.get("type").and_then(Value::as_str),
        Some("error" | "response.error" | "response.failed")
    ) {
        return None;
    }
    if event
        .pointer("/response/output")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
        && super::official_fallback::code(event).as_deref()
            == Some("subscription_sharing_unsupported_capability")
        && super::capabilities::pre_execution_rejection(400, event)
    {
        return Some((400, event.clone()));
    }
    if super::quota_exhaustion::code(event).is_some() && !http_execution_observed(event) {
        return Some((429, event.clone()));
    }
    // A generic invalid_request_error may carry a contradictory capacity hint.
    // Classify its real status/body before synthesizing a retryable HTTP status.
    if super::upstream_errors::chat_failure(event, 502)
        .is_some_and(|error| super::upstream_errors::request_fault(error.status, event))
    {
        return None;
    }
    if event
        .pointer("/response/output")
        .and_then(Value::as_array)
        .is_some_and(|output| !output.is_empty())
    {
        return None;
    }
    let top_level = json!({"code":event.get("code"), "type":event.get("error_type"), "message":event.get("message")});
    let extracted = super::diagnostics::error_object(event);
    let error = if std::ptr::eq(extracted, event) {
        &top_level
    } else {
        extracted
    };
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let kind = error
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if [&*code, &*kind].iter().any(|value| {
        matches!(
            *value,
            "context_length_exceeded"
                | "context_too_large"
                | "invalid_value"
                | "invalid_argument"
                | "invalid_encrypted_content"
                | "thinking_signature_invalid"
                | "content_policy_violation"
                | "cyber_policy"
                | "invalid_prompt"
                | "unsupported_value"
                | "message_too_big"
                | "string_above_max_length"
                | "previous_response_not_found"
        )
    }) {
        return None;
    }
    let message = error
        .get("message")
        .or_else(|| event.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ["context window", "context length", "too many tokens"]
        .iter()
        .any(|text| message.contains(text))
    {
        return None;
    }
    let status = if let Some((_, status)) = super::official_fallback::explicit_rejection(event) {
        status
    } else if super::upstream_errors::model_unavailable(400, event, "") {
        404
    } else if super::transient_rate_limit::applies(event) {
        429
    } else if [&*code, &*kind].iter().any(|v| {
        matches!(
            *v,
            "usage_limit_reached" | "insufficient_quota" | "quota_exceeded"
        )
    }) {
        429
    } else if [&*code, &*kind].iter().any(|v| {
        matches!(
            *v,
            "account_deactivated"
                | "account_disabled"
                | "account_suspended"
                | "organization_deactivated"
        )
    }) {
        403
    } else {
        let capacity = message.contains("model is at capacity")
            || code.contains("model_at_capacity")
            || code.contains("model_is_at_capacity")
            || (message.contains("model") && message.contains("at capacity"));
        if !(capacity
            || super::overload::capacity_code(&kind)
            || super::overload::capacity_code(&code)
            || kind == "rate_limit_error"
            || code == "rate_limit_exceeded"
            || ((kind == "server_error" || code == "server_error")
                && message.contains("you can retry your request")))
        {
            return None;
        }
        // Capacity is transient availability, not evidence of exhausted quota.
        503
    };
    let mut body = json!({"error": error});
    // Preserve Retry-After/reset hints, including wrapped event headers.
    for key in [
        "headers",
        "retry_after",
        "retry_after_seconds",
        "resets_at",
        "resets_in_seconds",
    ] {
        if let Some(value) = event.get(key) {
            body[key] = value.clone();
        }
    }
    Some((status, body))
}

/// Return a synthetic HTTP rejection only before any non-handshake event.
/// This feeds the same status retry/cooldown path as a real non-2xx response.
pub(super) async fn probe(
    state: &Arc<AppState>,
    response: reqwest::Response,
    policy: &ExecutionPolicy,
    deadline: Deadline,
) -> Result<reqwest::Response, ServiceError> {
    probe_inner(
        state, response, policy, deadline, false, false, false, false,
    )
    .await
}

pub(super) async fn probe_chat(
    state: &Arc<AppState>,
    response: reqwest::Response,
    policy: &ExecutionPolicy,
    deadline: Deadline,
) -> Result<reqwest::Response, ServiceError> {
    probe_inner(state, response, policy, deadline, true, false, false, false).await
}

/// Official sharing can report quota only after its metadata handshake. Keep
/// that pre-business fence until a decision, bounded by the existing stream
/// idle/total deadlines and metadata limits. Never extend other providers' hold.
pub(super) async fn probe_official(
    state: &Arc<AppState>,
    response: reqwest::Response,
    policy: &ExecutionPolicy,
    deadline: Deadline,
    capability_fallback: bool,
) -> Result<reqwest::Response, ServiceError> {
    probe_inner(
        state,
        response,
        policy,
        deadline,
        false,
        false,
        capability_fallback,
        true,
    )
    .await
}

fn empty_output(event: &Value) -> bool {
    ["/response/output", "/output"].iter().all(|path| {
        event
            .pointer(path)
            .is_none_or(|value| value.is_null() || value.as_array().is_some_and(Vec::is_empty))
    }) && ["/response/usage/output_tokens", "/usage/output_tokens", "/response/usage/output_tokens_details/reasoning_tokens", "/usage/output_tokens_details/reasoning_tokens", "/usage/reasoning_tokens", "/response/usage/reasoning_tokens"]
        .iter()
        .all(|path| {
            event
                .pointer(path)
                .is_none_or(|value| value.is_null() || value.as_u64() == Some(0))
        })
}

const EXECUTION_FIELDS: &[&str] = &[
    "delta",
    "item",
    "content",
    "text",
    "arguments",
    "tool_calls",
    "function_call",
    "function_call_output",
    "output_text",
    "audio",
    "image",
    "partial_json",
    "encrypted_content",
    "summary",
    "thinking",
    "reasoning_content",
    "reasoning",
    "output_items",
    "tool_usage",
];

pub(super) fn pre_execution(event: &Value) -> bool {
    fn text_configuration(value: &Value) -> bool {
        let Some(fields) = value.as_object() else {
            return false;
        };
        fields.iter().all(|(key, value)| match key.as_str() {
            "verbosity" => {
                value.is_null()
                    || value
                        .as_str()
                        .is_some_and(|v| matches!(v, "low" | "medium" | "high"))
            }
            "format" => {
                value.is_null()
                    || value.as_object().is_some_and(|format| {
                        format.is_empty()
                            || format
                                .get("type")
                                .and_then(Value::as_str)
                                .is_some_and(|kind| {
                                    matches!(kind, "text" | "json_object" | "json_schema")
                                })
                                && format.keys().all(|key| {
                                    matches!(
                                        key.as_str(),
                                        "type" | "name" | "schema" | "strict" | "description"
                                    )
                                })
                    })
            }
            _ => false,
        })
    }
    fn metadata_only(value: &Value, response_envelope: bool) -> bool {
        let Some(fields) = value.as_object() else {
            return value.is_null();
        };
        // Response.text is output-format configuration; tool schema parameter
        // names are declarations. Neither represents generated model content.
        fields.iter().all(|(key, value)| {
            !EXECUTION_FIELDS.contains(&key.as_str())
                || value.is_null()
                || response_envelope && key == "text" && text_configuration(value)
                || response_envelope && key == "reasoning" && value.as_object().is_some_and(|fields| {
                    fields.iter().all(|(key,value)| match key.as_str() {
                        "effort" => value.is_null() || value.as_str().is_some_and(|v| matches!(v,"none"|"minimal"|"low"|"medium"|"high"|"xhigh"|"max"|"ultra")),
                        "summary" | "generate_summary" => value.is_null() || value.as_str().is_some_and(|v| matches!(v,"auto"|"concise"|"detailed")),
                        _ => false,
                    })
                })
        }) && empty_output(value)
            && value.get("usage").is_none_or(|usage| usage.is_null() || usage.as_object().is_some_and(|fields| {
                ["output_tokens","completion_tokens","reasoning_tokens"].iter().all(|key| fields.get(*key).is_none_or(|n|n.is_null() || n.as_u64()==Some(0)))
                    && fields.get("output_tokens_details").is_none_or(|details| details.is_null() || details.as_object().is_some_and(|fields|fields.values().all(|n|n.is_null() || n.as_u64()==Some(0))))
            }))
            && value
                .get("output_tokens")
                .is_none_or(|value| value.is_null() || value.as_u64() == Some(0))
    }
    metadata_only(event, event.get("object").and_then(Value::as_str) == Some("response"))
        && event
            .get("response")
            .is_none_or(|value| metadata_only(value, true))
        && ["/metadata", "/response/metadata"].iter().all(|path| {
            event
                .pointer(path)
                .is_none_or(|value| metadata_only(value, false))
        })
}

/// Non-2xx JSON may still carry a partial Responses result. A status code is
/// not permission to replay output/tool execution returned in that body.
pub(super) fn http_execution_observed(body: &Value) -> bool {
    [Some(body), body.get("body")].into_iter().flatten()
        .any(|value| value.is_object() && !pre_execution(value))
}

#[cfg(test)]
mod official_handshake_tests {
    use super::*;

    #[test]
    fn response_metadata_and_execution_fields_fail_closed() {
        let metadata = json!({"type":"response.metadata","metadata":{"synthetic":true}});
        assert!(handshake(&metadata));
        assert!(handshake(&metadata));
        for field in [
            "delta",
            "item",
            "content",
            "text",
            "arguments",
            "tool_calls",
            "function_call",
            "output_text",
        ] {
            let mut event = metadata.clone();
            event["metadata"][field] = Value::Null;
            assert!(handshake(&event), "null {field}");
            event["metadata"][field] = json!("executed");
            assert!(!handshake(&event), "non-null {field}");
        }
        for value in [json!(1), json!(-1), json!("0")] {
            let mut event = metadata.clone();
            event["metadata"]["usage"] = json!({"output_tokens":value});
            assert!(!handshake(&event));
        }
        assert!(!handshake(
            &json!({"type":"response.unknown_metadata"})
        ));
    }

    #[test]
    fn echoed_tool_parameter_names_are_declarations_not_execution() {
        let event = json!({"type":"response.created","response":{"output":[],"usage":{"output_tokens":0},
        "tools":[{"type":"function","name":"read_file","parameters":{"type":"object","properties":{
            "content":{"type":"string"},"text":{"type":"string"},"arguments":{"type":"string"}
        }}}]}});
        assert!(handshake(&event));
        let mut executing = event;
        executing["response"]["arguments"] = json!("{}");
        assert!(!handshake(&executing));
    }

    #[test]
    fn response_text_format_is_configuration_but_root_text_and_deltas_are_execution() {
        for text in [
            json!({}),
            json!({"format":{"type":"text"},"verbosity":"medium"}),
            json!({"format":{"type":"json_schema","name":"fixture","schema":{"properties":{"text":{"type":"string"}}},"strict":true}}),
        ] {
            let event = json!({"type":"response.created","response":{"output":[],"usage":null,
                "text":text,"reasoning":{"effort":"medium","summary":null},"audio":null}});
            assert!(handshake(&event));
        }
        for field in ["text", "delta", "item", "arguments"] {
            let mut event =
                json!({"type":"response.created","response":{"output":[],"usage":null}});
            event[field] = json!("executed");
            assert!(!handshake(&event), "root {field}");
            event[field] = Value::Null;
            event["response"][field] = json!("executed");
            assert!(!handshake(&event), "response {field}");
        }
    }
}

#[cfg(test)]
fn trace_official_gate(event: &Value, started: tokio::time::Instant) {
    if std::env::var_os("COCKPIT_NATIVE_POOL_LIVE_CONFIG").is_none() {
        return;
    }
    fn label(value: Option<&str>) -> &str {
        value
            .filter(|value| {
                value.len() <= 96
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            })
            .unwrap_or("unknown")
    }
    let code = super::official_fallback::code(event);
    let count = event
        .pointer("/response/output")
        .or_else(|| event.get("output"))
        .and_then(Value::as_array)
        .map(Vec::len);
    let output_tokens = event
        .pointer("/response/usage/output_tokens")
        .or_else(|| event.pointer("/usage/output_tokens"))
        .and_then(Value::as_u64);
    let keys = event
        .as_object()
        .map(|fields| {
            fields
                .keys()
                .take(32)
                .map(|key| label(Some(key)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let response_keys = event
        .get("response")
        .and_then(Value::as_object)
        .map(|fields| {
            fields
                .keys()
                .take(64)
                .map(|key| label(Some(key)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut execution_fields = Vec::new();
    for (position, value) in [
        ("root", Some(event)),
        ("response", event.get("response")),
        ("metadata", event.get("metadata")),
        ("response.metadata", event.pointer("/response/metadata")),
    ] {
        if let Some(fields) = value.and_then(Value::as_object) {
            for field in EXECUTION_FIELDS {
                if let Some(value) = fields.get(*field) {
                    let kind = match value {
                        Value::Null => "null",
                        Value::Bool(_) => "bool",
                        Value::Number(_) => "number",
                        Value::String(_) => "string",
                        Value::Array(_) => "array",
                        Value::Object(_) => "object",
                    };
                    execution_fields.push(format!(
                        "{position}.{field}:{kind}:null={}",
                        value.is_null()
                    ));
                }
            }
        }
    }
    eprintln!("[official_pre_business_gate] elapsed_ms={} type={} code={} output_items={count:?} output_tokens={output_tokens:?} keys={keys:?} response_keys={response_keys:?} execution_fields={execution_fields:?}",
        started.elapsed().as_millis(), label(event.get("type").and_then(Value::as_str)), label(code.as_deref()));
}

pub(super) async fn probe_provider(
    state: &Arc<AppState>,
    response: reqwest::Response,
    policy: &ExecutionPolicy,
    deadline: Deadline,
    grok: bool,
    capability_fallback: bool,
) -> Result<reqwest::Response, ServiceError> {
    probe_inner(
        state,
        response,
        policy,
        deadline,
        false,
        grok,
        capability_fallback,
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn probe_inner(
    state: &Arc<AppState>,
    response: reqwest::Response,
    policy: &ExecutionPolicy,
    deadline: Deadline,
    chat: bool,
    grok: bool,
    capability_fallback: bool,
    official: bool,
) -> Result<reqwest::Response, ServiceError> {
    #[cfg(test)]
    let gate_started = tokio::time::Instant::now();
    let status = response.status();
    let version = response.version();
    let mut headers = response.headers().clone();
    let mut source = deadline::timeout_at(
        deadline,
        super::upstream::raw_stream(state, response, policy.idle_timeout),
    )
    .await
    .map_err(|_| super::retry::total_timeout())??;
    let mut buffered = Vec::new();
    let mut size = 0;
    let mut count = 0;
    let mut decoder = SseDecoder::new(if chat { state.options.max_provider_event_bytes } else { state.options.max_event_bytes });
    let mut reject = None;
    let mut hold_deadline = None;
    let mut json_prefix = Vec::new();
    let responses_json = !source.is_sse && !chat && headers.get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(';').next().is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json")));
    if source.is_sse || chat || responses_json {
        'read: loop {
            let mut stop = state.stop.clone();
            let next = tokio::select! {
                _ = stop.wait() => return Err(stopping_error()),
                _ = deadline::sleep_until(hold_deadline.unwrap_or_else(tokio::time::Instant::now)), if hold_deadline.is_some() => break 'read,
                next = deadline::timeout_at(deadline.cap(tokio::time::Instant::now() + policy.idle_timeout), source.bytes.next()) =>
                    next.map_err(|_| if tokio::time::Instant::now() >= deadline { super::retry::total_timeout() }
                    else { ServiceError::new(504, "upstream_idle_timeout", "Upstream stream stopped responding") })?,
            };
            let events = match next {
                Some(Ok(bytes)) => {
                    let remaining = MAX_BYTES.saturating_sub(size);
                    let prefix = &bytes[..bytes.len().min(remaining)];
                    let events = if source.is_sse {
                        decoder.push_frames(prefix)?
                    } else {
                        json_prefix.extend_from_slice(prefix);
                        match serde_json::from_slice::<Value>(&json_prefix) {
                            Ok(value) => vec![super::events::SseFrame {
                                value: Some(value),
                                identity_event: None,
                                bytes: json_prefix.clone().into(),
                            }],
                            Err(_) => vec![],
                        }
                    };
                    size = size.saturating_add(bytes.len());
                    buffered.push(Ok(bytes));
                    events
                }
                Some(Err(error)) => {
                    return Err(super::diagnostics::transport_failure(
                        502,
                        "upstream_stream_error",
                        "Upstream stream disconnected before output",
                        &error,
                    ))
                }
                None => {
                    let events = if source.is_sse {
                        decoder.finish_frames()?
                    } else {
                        vec![]
                    };
                    if events.is_empty() {
                        if responses_json { break 'read; }
                        return Err(ServiceError::new(
                            502,
                            "incomplete_upstream_stream",
                            "Upstream stream ended before output",
                        ));
                    }
                    events
                }
            };
            for frame in events {
                if size >= MAX_BYTES || count >= MAX_EVENTS { break 'read; }
                // Comments and incomplete JSON do not expose a response ID.
                // Start the bounded hold only once a complete semantic event
                // can actually be forwarded; otherwise a slow first error can
                // escape the retry path before any response was published.
                if !official && frame.value.is_some() && hold_deadline.is_none() {
                    hold_deadline = Some(tokio::time::Instant::now() + MAX_HOLD);
                }
                if responses_json {
                    if let Some(value) = frame.value {
                        if super::quota_exhaustion::code(&value).is_some() && !http_execution_observed(&value) {
                            reject = Some((429, value));
                        }
                        break 'read;
                    }
                    continue;
                }
                if chat {
                    if let Some(error) = super::upstream_errors::chat_frame_failure(&frame) {
                        reject = Some((
                            error.status,
                            super::upstream_errors::canonical_chat_failure(&frame, &error),
                        ));
                        break 'read;
                    }
                    // Go's stream bootstrap stops at the first payload. Keep
                    // every choice, role delta, tool delta and usage payload;
                    // replaying any of them could duplicate output or billing.
                    if frame.value.is_some() || decoder.done {
                        break 'read;
                    }
                    count += 1;
                    if count > MAX_EVENTS {
                        break 'read;
                    }
                    continue;
                }
                let Some(event) = frame.value else { continue };
                #[cfg(test)]
                if official {
                    trace_official_gate(&event, gate_started);
                }
                // Positive/invalid output usage or a nonempty output is already
                // execution evidence even if the frame claims to be metadata.
                if !pre_execution(&event) && !empty_output_event(&event) {
                    break 'read;
                }
                // Only buffered pre-output events reach this point. Reuse the
                // same strict schema-rejection gate as HTTP errors and sockets;
                // normal parameter faults and generated output stay untouched.
                if capability_fallback && super::capabilities::pre_execution_rejection(400, &event)
                {
                    reject = Some((400, event));
                    break 'read;
                }
                if let Some(failure) = grok
                    .then(|| super::grok_errors::rejection(&event))
                    .flatten()
                    .or_else(|| rejection(&event))
                    .or_else(|| super::encrypted_reasoning::rejection(&event))
                {
                    reject = Some(failure);
                    break 'read;
                }
                if !handshake(&event) {
                    break 'read;
                }
                count += 1;
                if count >= MAX_EVENTS {
                    break 'read;
                }
            }
            if size >= MAX_BYTES {
                break;
            }
        }
    }
    headers.remove(header::CONTENT_LENGTH);
    headers.remove(header::CONTENT_ENCODING);
    let (status, body) = if let Some((status, error)) = reject {
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        (
            StatusCode::from_u16(status).unwrap(),
            reqwest::Body::from(error.to_string()),
        )
    } else {
        if source.is_sse {
            headers.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
        }
        (
            status,
            reqwest::Body::wrap_stream(stream::iter(buffered).chain(source.bytes)),
        )
    };
    let mut response = axum::http::Response::new(body);
    *response.status_mut() = status;
    *response.version_mut() = version;
    *response.headers_mut() = headers;
    Ok(reqwest::Response::from(response))
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
