//! Bounded Responses SSE framing, inspection and compatibility repair.
use axum::body::Bytes;
use serde_json::Value;

use super::contract::ServiceError;

#[derive(Debug)]
pub(super) struct SseFrame {
    pub value: Option<Value>,
    /// Preserve the upstream terminal as history evidence when public output is repaired.
    /// This metadata is never serialized onto the client wire.
    pub identity_event: Option<Value>,
    pub bytes: Bytes,
}

pub(super) struct SseDecoder {
    pending: Vec<u8>,
    limit: usize,
    scan_from: usize,
    data_lines: IndependentDataLines,
    output: Option<super::output_items::OutputCollector>,
    terminal: bool,
    pub done: bool,
}

impl SseDecoder {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            pending: Vec::new(),
            limit,
            scan_from: 0,
            data_lines: IndependentDataLines::default(),
            output: Some(super::output_items::OutputCollector::new()),
            terminal: false,
            done: false,
        }
    }

    /// The WebSocket bridge restores output after framing, where it can retain
    /// the original terminal's eligibility for automatic history rebuilding.
    pub(super) fn without_output_repair(limit: usize) -> Self {
        Self {
            output: None,
            ..Self::new(limit)
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>, ServiceError> {
        Ok(self
            .push_frames(bytes)?
            .into_iter()
            .filter_map(|frame| frame.value)
            .collect())
    }

    pub(super) fn finish(&mut self) -> Result<Vec<Value>, ServiceError> {
        Ok(self
            .finish_frames()?
            .into_iter()
            .filter_map(|frame| frame.value)
            .collect())
    }

    /// Keep complete frames from this transport chunk even if a later frame
    /// fails validation. Emit the bounded protocol failure immediately after
    /// them, rather than discarding data or waiting for another network chunk.
    pub(super) fn push_frames(&mut self, bytes: &[u8]) -> Result<Vec<SseFrame>, ServiceError> {
        if self.terminal || self.done {
            return Ok(Vec::new());
        }
        let mut frames = Vec::new();
        if let Err(error) = self.push_frames_inner(bytes, &mut frames) {
            if !frames.iter().any(|frame| frame.value.is_some()) {
                return Err(error);
            }
            let mut event = super::output_error::responses_event(&error, false, 0);
            event.as_object_mut().unwrap().remove("sequence_number");
            let bytes = super::output_error::event_sse(&event);
            frames.push(SseFrame {
                value: Some(event),
                identity_event: None,
                bytes,
            });
            self.pending.clear();
            self.scan_from = 0;
            self.data_lines = IndependentDataLines::default();
            self.terminal = true;
        }
        Ok(frames)
    }

    /// Complete JSON plus an explicit event field can be framed immediately.
    /// Data-only tail frames wait for the next field boundary or a clean EOF.
    fn push_frames_inner(
        &mut self,
        mut bytes: &[u8],
        frames: &mut Vec<SseFrame>,
    ) -> Result<(), ServiceError> {
        if !self.pending.is_empty() && starts_field(bytes) {
            let fields = fields(&self.pending);
            // Preserve the old missing-field-newline compatibility for two
            // complete data-only chunks. Actual newline-delimited data uses
            // the incremental scanner below, independent of TCP boundaries.
            if !self.pending.contains(&b'\n')
                && fields.event.is_none()
                && self.pending.len() <= self.limit
                && bytes.len() <= self.limit
                && data_object(&self.pending)
                && !bytes.contains(&b'\n')
                && data_object(bytes)
            {
                let pending = std::mem::take(&mut self.pending);
                frames.push(self.decode_frame(&pending, b"\n\n")?);
                self.scan_from = 0;
                self.data_lines = IndependentDataLines::default();
            } else if !self.pending.ends_with(b"\n")
                && !self.pending.ends_with(b"\r")
                && !bytes.starts_with(b"\n")
                && !bytes.starts_with(b"\r")
                && (self
                    .pending
                    .rsplit(|byte| *byte == b'\n')
                    .next()
                    .is_some_and(data_object)
                    || (fields.data.is_empty()
                        && complete_event_field(&self.pending)
                        && trim_ascii(bytes).starts_with(b"data:")))
            {
                self.pending.push(b'\n');
            }
        }
        while !bytes.is_empty() && !self.terminal && !self.done {
            // A chunk may contain many small events. Bound the pending frame,
            // rather than reject a large transport chunk containing valid frames.
            let remaining = self
                .pending_limit()
                .saturating_add(4)
                .saturating_sub(self.pending.len());
            if remaining == 0 {
                return Err(event_too_large());
            }
            let count = remaining.min(bytes.len());
            self.pending.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            let mut consumed = 0;
            loop {
                let standard = frame_end(&self.pending[self.scan_from..])
                    .map(|(offset, separator)| (self.scan_from + offset, separator, false));
                let separate = self
                    .data_lines
                    .boundary(&self.pending[consumed..], false)
                    .map(|(offset, separator)| (consumed + offset, separator, true));
                let Some((end, separator, repaired)) = [standard, separate]
                    .into_iter()
                    .flatten()
                    .min_by_key(|(end, _, _)| *end)
                else {
                    break;
                };
                self.check_line_size(&self.pending[consumed..end])?;
                let raw = self.pending[consumed..end].to_vec();
                let delimiter = if repaired {
                    b"\n\n".to_vec()
                } else {
                    self.pending[end..end + separator].to_vec()
                };
                frames.push(self.decode_frame(&raw, &delimiter)?);
                consumed = end + separator;
                self.data_lines = IndependentDataLines::default();
                self.scan_from = consumed;
                if self.terminal || self.done {
                    break;
                }
            }
            self.pending.drain(..consumed);
            self.scan_from = self.pending.len().saturating_sub(3);
            if self.terminal || self.done {
                self.pending.clear();
                self.data_lines = IndependentDataLines::default();
                self.scan_from = 0;
                break;
            }
            if self.pending.len() > self.pending_limit() {
                return Err(event_too_large());
            }
        }
        let fields = fields(&self.pending);
        if !self.pending.is_empty() && fields.event.is_some() && fields.valid_data() {
            let pending = std::mem::take(&mut self.pending);
            frames.push(self.decode_frame(&pending, b"\n\n")?);
            self.scan_from = 0;
            self.data_lines = IndependentDataLines::default();
        }
        Ok(())
    }

    pub(super) fn finish_frames(&mut self) -> Result<Vec<SseFrame>, ServiceError> {
        if self.terminal || self.done || !self.has_partial_event() {
            self.pending.clear();
            self.data_lines = IndependentDataLines::default();
            self.scan_from = 0;
            return Ok(Vec::new());
        }
        let mut frames = Vec::new();
        while let Some((end, separator)) = self.data_lines.boundary(&self.pending, true) {
            self.check_line_size(&self.pending[..end])?;
            let raw = self.pending[..end].to_vec();
            frames.push(self.decode_frame(&raw, b"\n\n")?);
            self.pending.drain(..end + separator);
            self.data_lines = IndependentDataLines::default();
            self.scan_from = 0;
            if self.terminal || self.done {
                self.pending.clear();
                return Ok(frames);
            }
        }
        let fields = fields(&self.pending);
        if !fields.data.is_empty() && !fields.valid_data() {
            return Err(ServiceError::new(
                502,
                "incomplete_upstream_stream",
                "Upstream stream ended within an event",
            ));
        }
        let pending = std::mem::take(&mut self.pending);
        self.scan_from = 0;
        self.data_lines = IndependentDataLines::default();
        frames.push(self.decode_frame(&pending, b"\n\n")?);
        Ok(frames)
    }

    fn pending_limit(&self) -> usize {
        // Confirming independent lines needs at most one following bounded
        // frame, not an unbounded aggregate or a larger individual event.
        let prefix = self
            .data_lines
            .first
            .filter(|(end, _)| *end <= self.limit)
            .map_or(0, |(end, separator)| end.saturating_add(separator));
        // Go's Scanner limits the data line; a preceding event field does not
        // consume that line's budget. Retain a finite pending buffer as well.
        let metadata = self.pending.split_inclusive(|byte| *byte == b'\n')
            .take_while(|line| !line.starts_with(b"data:"))
            .map(|line| line.len()).sum::<usize>();
        let metadata = if metadata < self.pending.len() { metadata } else { 0 };
        self.limit.saturating_add(prefix).saturating_add(metadata)
    }

    fn check_line_size(&self, bytes: &[u8]) -> Result<(), ServiceError> {
        if bytes.split(|byte| *byte == b'\n').any(|line| line.len() >= self.limit) {
            return Err(event_too_large());
        }
        Ok(())
    }

    pub(super) fn has_partial_event(&self) -> bool {
        self.pending.iter().any(|byte| !byte.is_ascii_whitespace())
    }

    fn decode_frame(&mut self, raw: &[u8], separator: &[u8]) -> Result<SseFrame, ServiceError> {
        let fields = fields(raw);
        let mut wire = Vec::with_capacity(raw.len() + separator.len());
        wire.extend_from_slice(raw);
        wire.extend_from_slice(separator);
        if fields.data.is_empty() {
            return Ok(SseFrame {
                value: None,
                identity_event: None,
                bytes: wire.into(),
            });
        }
        if fields.data == b"[DONE]" {
            self.done = true;
            return Ok(SseFrame {
                value: None,
                identity_event: None,
                bytes: wire.into(),
            });
        }
        let mut value: Value = serde_json::from_slice(&fields.data).map_err(|_| {
            ServiceError::new(
                502,
                "invalid_upstream_event",
                "Upstream returned an invalid Responses event",
            )
        })?;
        let mut changed = false;
        let mut preserve_failure_wire = false;
        let terminal = response_terminal_kind(&value);
        let flat_error = super::upstream_errors::flat_error_details(&value);
        if let Some(object) = value.as_object_mut() {
            let kind = object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let declared_failure = fields.event.as_deref().is_some_and(failure_kind);
            let payload_failure = terminal.is_some_and(failure_kind);
            if payload_failure || declared_failure {
                let failure = if failure_kind(&kind) {
                    kind.to_owned()
                } else if declared_failure {
                    fields.event.clone().unwrap()
                } else {
                    terminal.unwrap_or("error").into()
                };
                let failure = if failure == "response.error" {
                    "error".into()
                } else {
                    failure
                };
                if kind != failure {
                    // `type` may be an upstream error classification rather
                    // than the SSE event kind. Preserve it before normalization.
                    if declared_failure && !kind.is_empty() && !failure_kind(&kind) {
                        preserve_failure_wire = true;
                        let nested = object.get("error").is_some_and(|error| !error.is_null())
                            || object
                                .get("response")
                                .and_then(|response| response.get("error"))
                                .is_some_and(|error| !error.is_null());
                        if nested {
                            object
                                .insert("original_error_type".into(), Value::String(kind.clone()));
                        } else {
                            object.insert("error".into(), flat_error.clone());
                        }
                    }
                    object.insert("type".into(), Value::String(failure));
                    changed = true;
                }
            } else if kind.is_empty() {
                if let Some(event) = fields.event.as_ref().filter(|event| !event.is_empty()) {
                    object.insert("type".into(), Value::String(event.clone()));
                    changed = true;
                }
            }
        }
        let identity_restored = self
            .output
            .as_ref()
            .is_some_and(|output| output.can_restore_identity(&value));
        let original = (self.output.is_some()
            && matches!(
                value["type"].as_str(),
                Some("response.completed" | "response.done" | "response.incomplete")
            ))
        .then(|| value.clone());
        let repaired_output = self
            .output
            .as_mut()
            .map(|output| output.observe(&mut value))
            .transpose()?
            .unwrap_or(false);
        changed |= repaired_output;
        if value.get("response").is_some()
            || value["type"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("response."))
            || value["object"].as_str() == Some("response")
        {
            changed |= super::output_usage::normalize_usage_details(&mut value);
        }
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "response.completed" && !completed_response(&value) {
            return Err(ServiceError::new(
                502,
                "invalid_upstream_event",
                "Completed response requires a completed status and a nonempty response ID",
            ));
        }
        self.terminal = response_terminal_kind(&value).is_some();
        if changed && (!preserve_failure_wire || repaired_output) {
            wire.clear();
            for line in raw.split(|byte| *byte == b'\n') {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                let trimmed = trim_ascii(line);
                if trimmed.is_empty() || trimmed.starts_with(b"data:") {
                    continue;
                }
                if trimmed.starts_with(b"event:") && !kind.is_empty() {
                    wire.extend_from_slice(b"event: ");
                    wire.extend_from_slice(kind.as_bytes());
                } else {
                    wire.extend_from_slice(line);
                }
                wire.push(b'\n');
            }
            wire.extend_from_slice(b"data: ");
            let mut wire_value = value.clone();
            if let Some(object) = wire_value.as_object_mut() {
                object.remove("original_error_type");
            }
            wire.extend_from_slice(wire_value.to_string().as_bytes());
            wire.extend_from_slice(b"\n\n");
        }
        // Go bounds upstream lines, not a terminal rebuilt from prior output items.
        if !repaired_output { self.check_line_size(&wire)?; }
        Ok(SseFrame {
            value: Some(value),
            identity_event: original.filter(|_| repaired_output && !identity_restored),
            bytes: wire.into(),
        })
    }
}

struct Fields {
    data: Vec<u8>,
    event: Option<String>,
}
impl Fields {
    fn valid_data(&self) -> bool {
        !self.data.is_empty()
            && (self.data == b"[DONE]" || serde_json::from_slice::<Value>(&self.data).is_ok())
    }
}
fn fields(raw: &[u8]) -> Fields {
    let mut result = Fields {
        data: Vec::new(),
        event: None,
    };
    let mut found = false;
    for line in raw.split(|byte| *byte == b'\n') {
        let line = trim_ascii(line);
        if let Some(data) = line.strip_prefix(b"data:") {
            if found {
                result.data.push(b'\n');
            }
            result
                .data
                .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
            found = true;
        } else if let Some(event) = line.strip_prefix(b"event:") {
            result.event = std::str::from_utf8(trim_ascii(event))
                .ok()
                .map(str::to_owned);
        }
    }
    result
}
fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}
fn starts_field(bytes: &[u8]) -> bool {
    let bytes = trim_ascii(bytes);
    [b"data:".as_slice(), b"event:", b"id:", b"retry:", b":"]
        .into_iter()
        .any(|prefix| bytes.starts_with(prefix))
}

fn complete_event_field(raw: &[u8]) -> bool {
    raw.rsplit(|byte| *byte == b'\n')
        .next()
        .map(trim_ascii)
        .and_then(|line| line.strip_prefix(b"event:"))
        .map(trim_ascii)
        .is_some_and(|kind| {
            !kind.is_empty()
                && std::str::from_utf8(kind).is_ok()
                && !kind.iter().any(|byte| byte.is_ascii_whitespace())
        })
}

/// Some Responses peers omit the blank separator between data-only JSON
/// events. Repair complete object lines or an existing following [DONE]; an incomplete
/// JSON line still belongs to the ordinary multiline SSE field. Event/id/retry
/// and comment fields are handled exclusively by standard SSE framing.
#[derive(Default)]
struct IndependentDataLines {
    scan_from: usize,
    line_start: usize,
    first: Option<(usize, usize)>,
    disabled: bool,
    #[cfg(test)]
    scanned_bytes: usize,
    #[cfg(test)]
    parsed_lines: usize,
}

fn data_object(line: &[u8]) -> bool {
    trim_ascii(line)
        .strip_prefix(b"data:")
        .and_then(|data| serde_json::from_slice::<Value>(trim_ascii(data)).ok())
        .is_some_and(|value| value.is_object())
}

fn data_done(line: &[u8]) -> bool {
    trim_ascii(line)
        .strip_prefix(b"data:")
        .is_some_and(|data| trim_ascii(data) == b"[DONE]")
}

impl IndependentDataLines {
    fn boundary(&mut self, raw: &[u8], eof: bool) -> Option<(usize, usize)> {
        if self.disabled {
            return None;
        }
        while self.scan_from < raw.len() {
            let offset = raw[self.scan_from..].iter().position(|byte| *byte == b'\n');
            let scanned = offset.map_or(raw.len() - self.scan_from, |offset| offset + 1);
            #[cfg(test)]
            {
                self.scanned_bytes += scanned;
            }
            self.scan_from += scanned;
            let Some(_) = offset else {
                break;
            };
            let newline = self.scan_from - 1;
            #[cfg(test)]
            {
                self.parsed_lines += 1;
            }
            let line = &raw[self.line_start..newline];
            if !data_object(line) && !(self.first.is_some() && data_done(line)) {
                self.disabled = true;
                return None;
            }
            if let Some(first) = self.first {
                return Some(first);
            }
            self.first = Some(if newline > 0 && raw[newline - 1] == b'\r' {
                (newline - 1, 2)
            } else {
                (newline, 1)
            });
            self.line_start = self.scan_from;
        }
        if eof && self.first.is_some() && self.line_start < raw.len() {
            #[cfg(test)]
            {
                self.parsed_lines += 1;
            }
            if data_object(&raw[self.line_start..]) || data_done(&raw[self.line_start..]) {
                return self.first;
            }
            self.disabled = true;
        }
        None
    }
}
/// Shared terminal classification for SSE, WebSocket and identity observation.
/// A failure status is authoritative even when an upstream uses a generic or
/// stale event type. A created event is never a successful checkpoint.
pub(super) fn response_terminal_kind(event: &Value) -> Option<&'static str> {
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let response = event.get("response").unwrap_or(event);
    if event.get("error").is_some_and(|error| !error.is_null())
        || response.get("error").is_some_and(|error| !error.is_null())
    {
        return Some("error");
    }
    match response.get("status").and_then(Value::as_str) {
        Some("failed") => return Some("response.failed"),
        Some("incomplete") => return Some("response.incomplete"),
        Some("cancelled") => return Some("response.cancelled"),
        _ => {}
    }
    match kind {
        "error" | "response.error" => Some("error"),
        "response.failed" => Some("response.failed"),
        "response.incomplete" => Some("response.incomplete"),
        "response.cancelled" => Some("response.cancelled"),
        "response.completed" => Some("response.completed"),
        "response.done" => Some("response.done"),
        "" | "response" if response.get("status").and_then(Value::as_str) == Some("completed") => {
            Some("response.completed")
        }
        _ => None,
    }
}

fn failure_kind(kind: &str) -> bool {
    matches!(
        kind,
        "response.failed"
            | "response.incomplete"
            | "response.cancelled"
            | "response.error"
            | "error"
    )
}
fn completed_response(event: &Value) -> bool {
    let response = event.get("response").unwrap_or(event);
    response.get("status").and_then(Value::as_str) == Some("completed")
        && response
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty())
        && !event.get("error").is_some_and(|error| !error.is_null())
        && !response.get("error").is_some_and(|error| !error.is_null())
}

fn event_too_large() -> ServiceError {
    ServiceError::new(
        502,
        "upstream_event_too_large",
        "Upstream event exceeds the configured size limit",
    )
}
fn frame_end(bytes: &[u8]) -> Option<(usize, usize)> {
    for index in 0..bytes.len().saturating_sub(1) {
        if bytes[index..].starts_with(b"\n\n") {
            return Some((index, 2));
        }
        if bytes[index..].starts_with(b"\r\n\r\n") {
            return Some((index, 4));
        }
    }
    None
}

#[derive(Default)]
pub(super) struct ResponseObservation {
    pub terminal: bool,
    pub succeeded: bool,
    pub usage: Option<Value>,
    pub response: Option<Value>,
    pub error: Option<String>,
    pub error_message: Option<String>,
    pub credential_scope: bool,
    pub request_scoped: bool,
    pub retry_after: Option<std::time::Duration>,
    pub http_status: Option<u16>,
    pub service_tier: Option<String>,
}

impl ResponseObservation {
    pub(super) fn observe(&mut self, event: &Value) {
        if self.terminal {
            return;
        }
        if let Some(usage) = event
            .pointer("/response/usage")
            .filter(|usage| !usage.is_null())
        {
            self.usage = Some(usage.clone());
        }
        if let Some(tier) = event
            .pointer("/response/service_tier")
            .and_then(Value::as_str)
        {
            self.service_tier = Some(tier.to_string());
        }
        if let Some(value) = retry_after_event(event) {
            self.retry_after = Some(self.retry_after.map_or(value, |current| current.max(value)));
        }
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (message, credential_scope) = super::diagnostics::upstream_error_metadata(event);
        if message.is_some() {
            self.error_message = message;
        }
        self.credential_scope |= credential_scope;
        let error_code = super::diagnostics::error_object(event)
            .get("code")
            .and_then(Value::as_str);
        let model_unavailable = matches!(kind, "error" | "response.error" | "response.failed")
            && super::upstream_errors::model_unavailable(400, event, "");
        if model_unavailable {
            self.http_status = Some(404);
        }
        if let Some(code) = error_code {
            self.http_status = match code {
                "rate_limit_exceeded" | "insufficient_quota" | "usage_limit_reached" => Some(429),
                "invalid_api_key"
                | "invalid_token"
                | "invalid_grant"
                | "reauthorization_required" => Some(401),
                "account_deactivated" | "account_suspended" => Some(403),
                _ => super::chatgpt_plan_wire::error_status(code).or(self.http_status),
            };
        }
        if let Some((status, _)) = super::bootstrap::rejection(event) {
            self.http_status = self.http_status.or(Some(status));
        }
        if let Some(status) = event
            .get("status")
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
        {
            self.http_status = Some(status);
        }
        if let Some(status) =
            super::upstream_errors::chat_failure(event, self.http_status.unwrap_or(502))
                .map(|error| error.status)
        {
            self.http_status = Some(status);
            let (message, scope) =
                super::diagnostics::upstream_error_metadata_status(status, event);
            if message.is_some() {
                self.error_message = message;
            }
            self.credential_scope |= scope;
            self.request_scoped = super::upstream_errors::request_fault(status, event);
        }
        let terminal = response_terminal_kind(event);
        if terminal.is_some_and(|kind| kind != "error") {
            self.terminal = true;
            self.succeeded = terminal
                .is_some_and(|kind| matches!(kind, "response.completed" | "response.done"))
                && completed_response(event);
            self.response = Some(event.get("response").unwrap_or(event).clone());
            if !self.succeeded {
                self.error = Some(
                    if model_unavailable {
                        "upstream_model_unavailable"
                    } else {
                        error_code.unwrap_or(terminal.unwrap_or(kind))
                    }
                    .to_string(),
                );
            }
        } else if terminal == Some("error") {
            self.terminal = true;
            self.error = Some(
                if model_unavailable {
                    "upstream_model_unavailable"
                } else {
                    error_code.unwrap_or("upstream_error")
                }
                .to_string(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declared_failures_preserve_original_classification_and_native_wire_bytes() {
        for original in [
            json!({"type":"invalid_request_error","status":503,"message":"Malformed request"}),
            json!({"type":"authentication_error","code":"invalid_request_error","status":401,"message":"Expired key"}),
            json!({"type":"invalid_request_error","status":503,"error":{"type":"overloaded_error","code":"server_error","message":"Malformed request"}}),
            json!({"type":"authentication_error","status":401,"response":{"error":{"code":"invalid_request_error","message":"Expired key"}}}),
        ] {
            let raw = format!("event: error\ndata: {original}\n\n");
            let mut decoder = SseDecoder::new(4096);
            let frames = decoder.push_frames(raw.as_bytes()).unwrap();
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].bytes.as_ref(), raw.as_bytes());
            let value = frames[0].value.as_ref().unwrap();
            assert_eq!(value["type"], "error");
            if let Some(nested) = original.get("error") {
                assert_eq!(value.get("error"), Some(nested));
            }
            if let Some(nested) = original.pointer("/response/error") {
                assert_eq!(value.pointer("/response/error"), Some(nested));
            }
            let error = super::super::upstream_errors::chat_frame_failure(&frames[0]).unwrap();
            let canonical =
                super::super::upstream_errors::canonical_chat_failure(&frames[0], &error);
            let request_fault = original["type"] == "invalid_request_error";
            assert_eq!(
                super::super::upstream_errors::request_fault(error.status, &canonical),
                request_fault
            );
            assert_eq!(
                super::super::diagnostics::upstream_error_metadata_status(error.status, &canonical)
                    .1,
                !request_fault
            );
            assert!(decoder.finish_frames().unwrap().is_empty());
        }
        let raw=b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"kept\"},\"finish_reason\":null}]}\n\n";
        let mut decoder = SseDecoder::new(4096);
        assert_eq!(decoder.push_frames(raw).unwrap()[0].bytes.as_ref(), raw);
    }

    #[test]
    fn split_crlf_events_preserve_unknown_response_fields() {
        let mut decoder = SseDecoder::new(4096);
        assert!(decoder
            .push(b": ping\r\n\r\ndata: {\"type\":\"response.com")
            .unwrap()
            .is_empty());
        let events = decoder
            .push(b"pleted\",\r\ndata: \"response\":{\"id\":\"r1\",\"status\":\"completed\",\"unknown\":true}}\r\n\r\n")
            .unwrap();
        let mut observation = ResponseObservation::default();
        observation.observe(&events[0]);
        assert!(observation.succeeded);
        assert_eq!(
            observation.response,
            Some(json!({"id":"r1","status":"completed","unknown":true}))
        );
        assert!(!decoder.has_partial_event());
    }

    #[test]
    fn wrapped_rate_limit_exposes_status_and_retry_after() {
        let mut observation = ResponseObservation::default();
        observation.observe(&json!({"type":"error","status":429,"headers":{"Retry-After":"90"},"error":{"code":"rate_limit_exceeded"}}));
        assert_eq!(observation.http_status, Some(429));
        assert_eq!(
            observation.retry_after,
            Some(std::time::Duration::from_secs(90))
        );
        assert_eq!(observation.error.as_deref(), Some("rate_limit_exceeded"));
        assert!(!observation.succeeded);
        let future = (chrono::Utc::now() + chrono::Duration::seconds(120))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        let delay = parse_retry_after(&future).unwrap().as_secs();
        assert!((118..=120).contains(&delay));
    }

    #[test]
    fn retry_hints_preserve_nested_seconds_and_take_the_longest_delay() {
        for wrapper in [
            "root",
            "error",
            "response",
            "response/error",
            "body",
            "body/error",
            "detail",
        ] {
            for field in ["retry_after", "retry_after_seconds"] {
                for value in [json!(0), json!(2), json!(1.25), json!("2.5")] {
                    let mut event = json!({});
                    let mut target = &mut event;
                    if wrapper != "root" {
                        for part in wrapper.split('/') {
                            target[part] = json!({});
                            target = &mut target[part];
                        }
                    }
                    target[field] = value.clone();
                    let expected = value
                        .as_f64()
                        .unwrap_or_else(|| value.as_str().unwrap().parse().unwrap());
                    assert_eq!(
                        retry_after_event(&event),
                        Some(std::time::Duration::from_secs_f64(expected)),
                        "{event}"
                    );
                }
            }
        }
        for value in [
            json!(-1),
            json!("NaN"),
            json!("inf"),
            json!(1e100),
            json!(true),
            json!(null),
            json!({}),
        ] {
            assert_eq!(retry_after_event(&json!({"retry_after":value})), None);
        }
        assert_eq!(
            retry_after_event(
                &json!({"retry_after":1, "error":{"retry_after_seconds":3.5}, "headers":{"Retry-After":"2"}})
            ),
            Some(std::time::Duration::from_millis(3500))
        );
        let mut observation = ResponseObservation::default();
        observation.observe(&json!({"type":"response.failed", "response":{"error":{"code":"server_is_overloaded","retry_after":7}}}));
        assert_eq!(
            observation.retry_after,
            Some(std::time::Duration::from_secs(7))
        );
    }

    #[test]
    fn oversized_or_truncated_events_are_not_success() {
        let mut decoder = SseDecoder::new(16);
        assert!(decoder.push(b"data: {").unwrap().is_empty());
        assert!(decoder.has_partial_event());
        assert_eq!(
            decoder.push(b"1234567890123456").unwrap_err().code,
            "upstream_event_too_large"
        );
        let mut observation = ResponseObservation::default();
        observation.observe(&json!({"type":"response.incomplete","response":{"id":"r1"}}));
        assert!(observation.terminal);
        assert!(!observation.succeeded);
    }

    #[test]
    fn first_terminal_event_cannot_be_reversed_by_a_later_completion() {
        let mut observation = ResponseObservation::default();
        observation.observe(&json!({"type":"error","error":{"code":"invalid_token"}}));
        observation.observe(
            &json!({"type":"response.completed","response":{"id":"late","status":"completed"}}),
        );
        assert!(observation.terminal);
        assert!(!observation.succeeded);
        assert_eq!(observation.error.as_deref(), Some("invalid_token"));
        assert_eq!(observation.http_status, Some(401));
        assert!(observation.response.is_none());
    }

    #[test]
    fn done_event_requires_a_completed_response() {
        for status in ["completed", "failed", "incomplete"] {
            let mut observation = ResponseObservation::default();
            observation
                .observe(&json!({"type":"response.done","response":{"id":"r","status":status}}));
            assert!(observation.terminal);
            assert_eq!(observation.succeeded, status == "completed");
        }
    }
}

pub(super) fn retry_after_headers(headers: &axum::http::HeaderMap) -> Option<std::time::Duration> {
    headers
        .get(axum::http::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after)
}

fn parse_retry_after(value: &str) -> Option<std::time::Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(std::time::Duration::from_secs(seconds));
    }
    let deadline = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some(
        (deadline.with_timezone(&chrono::Utc) - chrono::Utc::now())
            .to_std()
            .unwrap_or_default(),
    )
}

pub(super) fn retry_after_event(event: &Value) -> Option<std::time::Duration> {
    fn seconds(value: &Value) -> Option<std::time::Duration> {
        if let Some(seconds) = value.as_u64() {
            return Some(std::time::Duration::from_secs(seconds));
        }
        let seconds = value
            .as_f64()
            .or_else(|| value.as_str()?.trim().parse::<f64>().ok())?;
        std::time::Duration::try_from_secs_f64(seconds).ok()
    }
    let mut delay: Option<std::time::Duration> = None;
    let mut retain = |next: std::time::Duration| {
        delay = Some(delay.map_or(next, |current| current.max(next)));
    };
    for headers in [
        event.get("headers"),
        event.pointer("/error/headers"),
        event.pointer("/response/error/headers"),
        event.pointer("/body/error/headers"),
        event.pointer("/body/headers"),
    ] {
        if let Some(headers) = headers.and_then(Value::as_object) {
            for (name, value) in headers {
                if name.eq_ignore_ascii_case("retry-after") {
                    if let Some(next) = value.as_str().and_then(parse_retry_after) {
                        retain(next);
                    } else if let Some(seconds) = value.as_u64() {
                        retain(std::time::Duration::from_secs(seconds));
                    }
                }
            }
        }
    }
    for error in [
        Some(event),
        Some(super::diagnostics::error_object(event)),
        event.get("error"),
        event.get("response"),
        event.pointer("/response/error"),
        event.get("body"),
        event.pointer("/body/error"),
    ] {
        let Some(error) = error else {
            continue;
        };
        for key in ["retry_after", "retry_after_seconds"] {
            if let Some(next) = error.get(key).and_then(seconds) {
                retain(next);
            }
        }
        if let Some(seconds) = error.get("resets_in_seconds").and_then(Value::as_u64) {
            retain(std::time::Duration::from_secs(seconds));
        }
        if let Some(reset) = error.get("resets_at").and_then(Value::as_i64) {
            let seconds = reset.saturating_sub(chrono::Utc::now().timestamp()).max(0) as u64;
            retain(std::time::Duration::from_secs(seconds));
        }
    }
    delay
}

#[cfg(test)]
mod sparse_identity_tests {
    use super::*;
    use serde_json::json;

    fn item() -> Value {
        json!({"id":"observed","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"synthetic answer"}],"unknown":{"keep":true}})
    }

    fn frame(decoder: &mut SseDecoder, event: &Value) -> SseFrame {
        decoder
            .push_frames(format!("data: {event}\n\n").as_bytes())
            .unwrap()
            .pop()
            .unwrap()
    }

    #[test]
    fn complete_done_can_supply_identity_evidence_for_a_successful_sparse_terminal() {
        for kind in ["response.completed", "response.done"] {
            for missing in [false, true] {
                let mut decoder = SseDecoder::new(8192);
                let done =
                    json!({"type":"response.output_item.done","output_index":0,"item":item()});
                let observed = frame(&mut decoder, &done);
                assert_eq!(observed.value.as_ref(), Some(&done));
                assert!(observed.identity_event.is_none());
                let mut terminal = json!({"type":kind,"response":{"id":"response","status":"completed","output":[]}});
                if missing {
                    terminal["response"]
                        .as_object_mut()
                        .unwrap()
                        .remove("output");
                }
                let terminal = frame(&mut decoder, &terminal);
                assert!(terminal.identity_event.is_none());
                assert_eq!(
                    terminal.value.as_ref().unwrap()["response"]["output"],
                    json!([item()])
                );
                assert!(String::from_utf8_lossy(&terminal.bytes).contains("synthetic answer"));
            }
        }
    }

    #[test]
    fn partial_or_presentation_repaired_output_keeps_the_original_identity_event() {
        for mode in [
            "unindexed",
            "gap",
            "conflict",
            "partial",
            "missing-id",
            "nonempty-tool",
            "incomplete",
        ] {
            let mut decoder = SseDecoder::new(8192);
            let mut done =
                json!({"type":"response.output_item.done","output_index":0,"item":item()});
            if mode == "unindexed" {
                done.as_object_mut().unwrap().remove("output_index");
            }
            if mode == "gap" {
                done["output_index"] = json!(1);
            }
            if mode == "partial" {
                done["item"]["content"] = json!([]);
            }
            if mode == "nonempty-tool" {
                done["item"] = json!({"id":"call","type":"function_call","call_id":"call","name":"tool","arguments":"{}"});
            }
            frame(&mut decoder, &done);
            if mode == "conflict" {
                done["item"]["content"][0]["text"] = json!("different");
                frame(&mut decoder, &done);
            }
            let mut terminal = json!({"type":"response.completed","response":{"id":"response","status":"completed","output":[]}});
            if mode == "missing-id" {
                let mut missing = item();
                missing.as_object_mut().unwrap().remove("id");
                terminal["response"]["output"] = json!([missing]);
            }
            if mode == "nonempty-tool" {
                terminal["response"]["output"] = json!([{"id":"call","type":"function_call","call_id":"call","name":"tool","arguments":"{\"truncated\":true}"}]);
            }
            if mode == "incomplete" {
                terminal["type"] = json!("response.incomplete");
                terminal["response"]["status"] = json!("incomplete");
            }
            let observed = frame(&mut decoder, &terminal);
            assert_eq!(observed.identity_event.as_ref(), Some(&terminal), "{mode}");
        }
    }

    #[test]
    fn eof_or_failed_terminal_cannot_turn_done_items_into_success() {
        for failed in [false, true] {
            let mut decoder = SseDecoder::new(8192);
            frame(
                &mut decoder,
                &json!({"type":"response.output_item.done","output_index":0,"item":item()}),
            );
            if failed {
                let terminal = json!({"type":"response.failed","response":{"id":"response","status":"failed","error":{"code":"synthetic_failure"},"output":[]}});
                let observed = frame(&mut decoder, &terminal);
                assert_eq!(observed.value.as_ref(), Some(&terminal));
                assert!(observed.identity_event.is_none());
                assert_eq!(observed.value.unwrap()["response"]["output"], json!([]));
            }
            assert!(decoder.finish_frames().unwrap().is_empty());
        }
    }
}

#[cfg(test)]
#[path = "sse_framing_tests.rs"]
mod sse_framing_tests;

#[cfg(test)]
#[path = "events_partial_error_tests.rs"]
mod partial_error_tests;
