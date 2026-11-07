//! Public Responses envelopes. Observe upstream state before filtering private events.
use serde_json::{json, Value};

#[derive(Default)]
pub(super) struct ResponseWire {
    created_at: Option<Value>,
    id: Option<Value>,
    model: Option<Value>,
    sequence: u64,
}

impl ResponseWire {
    pub(super) fn stamp_response(&mut self, response: &mut Value) -> bool {
        let Some(object) = response.as_object_mut() else {
            return false;
        };
        for (key, slot) in [("id", &mut self.id), ("model", &mut self.model)] {
            if let Some(value) = object.get(key).filter(|value| value.is_string()) {
                *slot = Some(value.clone());
            }
        }
        if let Some(value) = object.get("created_at").filter(|value| !value.is_null()) {
            self.created_at = Some(value.clone());
            return false;
        }
        let timestamp = self
            .created_at
            .get_or_insert_with(|| json!(chrono::Utc::now().timestamp()));
        object.insert("created_at".into(), timestamp.clone());
        true
    }

    pub(super) fn stamp_event(&mut self, event: &mut Value) -> bool {
        let sequence = event.get("sequence_number").and_then(Value::as_u64);
        if let Some(sequence) = sequence {
            self.sequence = self.sequence.max(sequence.saturating_add(1));
        }
        if !matches!(
            event["type"].as_str(),
            Some(
                "response.created"
                    | "response.queued"
                    | "response.in_progress"
                    | "response.completed"
                    | "response.done"
                    | "response.incomplete"
                    | "response.failed"
                    | "response.cancelled"
            )
        ) {
            return false;
        }
        let mut changed = false;
        if event["type"] == "response.failed" {
            if let Some(response) = event.get_mut("response").and_then(Value::as_object_mut) {
                for (key, value) in [
                    (
                        "id",
                        self.id
                            .clone()
                            .unwrap_or_else(|| json!(format!("resp_{}", uuid::Uuid::now_v7()))),
                    ),
                    ("object", json!("response")),
                    ("status", json!("failed")),
                    ("output", json!([])),
                ] {
                    if response.get(key).is_none_or(Value::is_null) {
                        response.insert(key.into(), value);
                        changed = true;
                    }
                }
            }
        }
        let Some(response) = event.get_mut("response").filter(|value| value.is_object()) else {
            return false;
        };
        changed |= self.stamp_response(response);
        if sequence.is_none() {
            event["sequence_number"] = json!(self.sequence);
            self.sequence = self.sequence.saturating_add(1);
            changed = true;
        }
        changed
    }

    pub(super) fn failure(
        &mut self,
        error: &super::contract::ServiceError,
        codex: bool,
        sequence: u64,
    ) -> Value {
        let mut event = super::output_error::responses_event(error, codex, sequence);
        if codex {
            // Reuse this turn's identity and timestamp, including a bare error
            // that replaces an upstream response.failed frame.
            let response = event["response"].as_object_mut().unwrap();
            if let Some(id) = &self.id {
                response.insert("id".into(), id.clone());
            }
            if let Some(model) = &self.model {
                response.insert("model".into(), model.clone());
            }
            if let Some(timestamp) = &self.created_at {
                response.insert("created_at".into(), timestamp.clone());
            }
        }
        event
    }
}

pub(super) fn visible(event: &Value, codex: bool) -> bool {
    visible_kind(event["type"].as_str().unwrap_or(""), codex)
}

pub(super) fn visible_frame(bytes: &[u8], codex: bool) -> bool {
    // The caller separately checks the already-decoded JSON type. Do not parse
    // large data lines a second time just to inspect the SSE event name.
    for line in bytes.split(|byte| *byte == b'\n') {
        if let Some(kind) = line.trim_ascii().strip_prefix(b"event:") {
            if !visible_kind(String::from_utf8_lossy(kind).trim(), codex) {
                return false;
            }
        }
    }
    true
}

pub(super) fn rewrite_frame(bytes: &[u8], event: &Value) -> axum::body::Bytes {
    // Keep data-only streams data-only and retain comments, ids and CRLF framing.
    let newline = if bytes.windows(2).any(|part| part == b"\r\n") {
        &b"\r\n"[..]
    } else {
        &b"\n"[..]
    };
    let mut wire = Vec::with_capacity(bytes.len().saturating_add(128));
    for line in bytes.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if !line.trim_ascii().is_empty() && !line.trim_ascii_start().starts_with(b"data:") {
            wire.extend_from_slice(line);
            wire.extend_from_slice(newline);
        }
    }
    wire.extend_from_slice(b"data: ");
    serde_json::to_writer(&mut wire, event).expect("JSON values serialize into a byte buffer");
    wire.extend_from_slice(newline);
    wire.extend_from_slice(newline);
    wire.into()
}

pub(super) fn visible_kind(kind: &str, codex: bool) -> bool {
    let kind = kind.trim();
    if matches!(kind, "error" | "response.error" | "response.failed") {
        return true;
    }
    if kind.starts_with("responsesapi.") || kind == "response.subscription_usage" {
        return false;
    }
    if codex {
        kind != "codex.rate_limits"
    } else {
        !kind.starts_with("codex.") && kind != "response.metadata"
    }
}

#[cfg(test)]
#[path = "response_wire_tests.rs"]
mod tests;
