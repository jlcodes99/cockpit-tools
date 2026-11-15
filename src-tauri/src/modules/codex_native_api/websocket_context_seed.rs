//! Populate a cold Responses socket without one oversized history message.
use super::*;
use std::{collections::HashSet, io::Write, ops::Range};

// Keep ordinary requests on the existing path below this size. Once a request
// needs preloading, use smaller chunks so the upstream can acknowledge each one.
pub(super) const MESSAGE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const CHUNK_TARGET_BYTES: usize = 8 * 1024 * 1024;
// Unlike an ordinary opening fence, a preload waits for a completed empty
// response. Codex echoes the tool catalog in created/in_progress/completed;
// three valid envelopes can exceed 64 KiB without containing generated output.
const PRELOAD_EVENT_BYTES: usize = 4 * 1024 * 1024;

/// A stale/missing live quota confirmation is not proof of exhausted quota.
/// Keep this socket and its parent while the normal single-flight monitor
/// refreshes; every send still requires the unchanged reserve gate to pass.
pub(super) async fn check_quota(
    state: &AppState,
    tracker: &RequestTracker,
    deadline: Deadline,
) -> Result<(), ServiceError> {
    let until = deadline.cap(tokio::time::Instant::now() + std::time::Duration::from_secs(12));
    let mut waiting = false;
    loop {
        match tracker.check_quota_before_send() {
            Ok(()) => return Ok(()),
            Err(error) => {
                if error.code != "quota_reserved"
                    || error
                        .response_error
                        .as_ref()
                        .and_then(|e| e.get("confirmation_pending"))
                        != Some(&Value::Bool(true))
                    || tokio::time::Instant::now() >= until
                {
                    return Err(error);
                }
                if !waiting {
                    tracing::info!(target:"codex_api",request_id=tracker.diagnostic_request_id(),
                        "[NativeAPI][quota] confirmation=waiting");
                    waiting = true;
                }
                tracker.request_quota_refresh();
                let wake =
                    until.min(tokio::time::Instant::now() + std::time::Duration::from_secs(2));
                let mut stop = state.stop.clone();
                tokio::select! {
                    _ = stop.wait() => return Err(stopping_error()),
                    _ = tokio::time::sleep_until(wake) => {},
                }
            }
        }
    }
}

pub(super) fn eligible(prepared: &Prepared, body: &Value) -> bool {
    supports(prepared) && body.get("previous_response_id").is_none_or(Value::is_null)
}

pub(super) fn supports(prepared: &Prepared) -> bool {
    prepared.raw_chat.is_none()
        && !prepared.route.chatgpt_plan
        && !super::super::grok_errors::applies(&prepared.route)
        && super::super::multi_agent_compat::is_codex_client(
            prepared
                .source_headers
                .get("user-agent")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        )
}

pub(super) struct Plan {
    pub ranges: Vec<Range<usize>>,
}

fn unavailable() -> ServiceError {
    ServiceError::new(426, "websocket_context_seed_unavailable",
        "The full history requires the HTTP bridge because bounded WebSocket preloading is unavailable")
}

fn outcome_unknown(reason: &'static str) -> ServiceError {
    ServiceError::new(
        502,
        "websocket_context_seed_outcome_unknown",
        "Upstream did not confirm a non-generating WebSocket preload; the request was not replayed",
    )
    .with_error_details(json!({"preload_failure":reason}))
}

fn unsupported_generate(event: &Value) -> bool {
    event["type"] == "error"
        && event["status"] == 400
        && matches!(
            event["error"]["code"].as_str(),
            Some("unsupported_parameter" | "unknown_parameter")
        )
        && event["error"]["param"] == "generate"
        && ["response", "id", "output", "usage"]
            .iter()
            .all(|key| event[*key].is_null())
        && ["output", "result", "usage"]
            .iter()
            .all(|key| event["error"][*key].is_null())
        && ["executed", "execution_started"]
            .iter()
            .all(|key| event["error"].get(*key).is_none_or(|value| value == false))
}

fn json_bytes(value: &Value) -> usize {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).expect("JSON values are serializable");
    counter.0
}

fn envelope(body: &Value) -> Value {
    Value::Object(
        body.as_object()
            .expect("normalized create")
            .iter()
            .filter(|(key, _)| key.as_str() != "input")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

impl Plan {
    pub fn build(body: &Value, limit: usize) -> Result<Option<Self>, ServiceError> {
        let Some(input) = body.get("input").and_then(Value::as_array) else {
            return Ok(None);
        };
        let sizes: Vec<usize> = input.iter().map(json_bytes).collect();
        // Reserve space for an upstream response ID and generate:false.
        let overhead = json_bytes(&envelope(body)).saturating_add(1024);
        let total = sizes
            .iter()
            .fold(overhead, |n, size| n.saturating_add(*size + 1));
        if total <= limit {
            return Ok(None);
        }
        let budget = limit.checked_sub(overhead).ok_or_else(unavailable)?;
        let mut pending = HashSet::<String>::new();
        let mut groups = Vec::<(Range<usize>, usize)>::new();
        let mut start = 0;
        let mut bytes = 0usize;
        for (index, (item, size)) in input.iter().zip(sizes).enumerate() {
            bytes = bytes.saturating_add(size + 1);
            match item["type"].as_str() {
                Some("function_call" | "custom_tool_call") => {
                    let id = item["call_id"].as_str().ok_or_else(unavailable)?;
                    if !pending.insert(id.to_owned()) {
                        return Err(unavailable());
                    }
                }
                Some("function_call_output" | "custom_tool_call_output") => {
                    // Codex host receipts have no model-issued call. Preserve
                    // their native WebSocket envelope and do not invent an ID.
                    if !super::super::responses_host_results::is_standalone(item)
                        && !item["call_id"]
                            .as_str()
                            .is_some_and(|id| pending.remove(id))
                    {
                        return Err(unavailable());
                    }
                }
                _ => {}
            }
            if bytes > budget {
                return Err(unavailable());
            }
            if pending.is_empty() {
                groups.push((start..index + 1, bytes));
                start = index + 1;
                bytes = 0;
            }
        }
        if !pending.is_empty() {
            return Err(unavailable());
        }
        // An indivisible image or tool group may exceed the target. Keep it
        // intact as long as it is within the existing message budget.
        let target = CHUNK_TARGET_BYTES
            .min(limit)
            .checked_sub(overhead)
            .filter(|target| *target > 0)
            .unwrap_or(budget);
        let mut ranges = Vec::new();
        let mut start = 0;
        let mut end = 0;
        let mut bytes = 0usize;
        for (group, size) in groups {
            if start < end && bytes.saturating_add(size) > target {
                ranges.push(start..end);
                start = group.start;
                bytes = 0;
            }
            end = group.end;
            bytes += size;
        }
        if end > start {
            ranges.push(start..end);
        }
        if ranges.len() < 2 {
            return Err(unavailable());
        }
        Ok(Some(Self { ranges }))
    }

    fn body(&self, source: &Value, index: usize, previous: Option<&str>, warm: bool) -> Value {
        let mut body = envelope(source);
        body["input"] =
            Value::Array(source["input"].as_array().unwrap()[self.ranges[index].clone()].to_vec());
        if let Some(id) = previous {
            body["previous_response_id"] = json!(id);
        }
        if warm {
            body["generate"] = json!(false);
        }
        body
    }

    pub fn final_body(&self, source: &Value, previous: &str) -> Value {
        self.body(source, self.ranges.len() - 1, Some(previous), false)
    }
}

pub(super) struct Seed {
    pub previous_id: String,
    pub bytes: usize,
    pub largest_message: usize,
    pub input_tokens: u64,
    pub cached_tokens: u64,
}

/// Newer Codex envelopes describe input-image processing and reasoning mode
/// even on generate:false responses. Normalize only these configuration fields
/// and provably zero hosted-tool counters for the existing no-output proof.
fn preload_event(mut event: Value) -> Value {
    fn option(value: &Value) -> bool {
        value.is_null()
            || value.as_str().is_some_and(|text| {
                !text.is_empty()
                    && text.len() <= 32
                    && text
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            })
    }
    fn zero_usage(value: &Value) -> bool {
        value.is_null()
            || value.as_f64() == Some(0.0)
            || value
                .as_object()
                .is_some_and(|fields| fields.values().all(zero_usage))
    }
    let Some(response) = event.get_mut("response").and_then(Value::as_object_mut) else {
        return event;
    };
    if response.get("generate") != Some(&Value::Bool(false))
        || !response
            .get("output")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    {
        return event;
    }
    if response
        .get("image")
        .and_then(Value::as_object)
        .is_some_and(|fields| {
            fields
                .iter()
                .all(|(key, value)| key == "context_compression" && option(value))
        })
    {
        response.remove("image");
    }
    if let Some(reasoning) = response.get_mut("reasoning").and_then(Value::as_object_mut) {
        for field in ["context", "mode"] {
            if reasoning.get(field).is_some_and(option) {
                reasoning.remove(field);
            }
        }
    }
    if response.get("tool_usage").is_some_and(zero_usage) {
        response.remove("tool_usage");
    }
    event
}

pub(super) async fn prime(
    state: &AppState,
    socket: &mut super::super::websocket_io::UpstreamSocket,
    body: &Value,
    plan: &Plan,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: Deadline,
) -> Result<Seed, ServiceError> {
    let (mut sink, mut source) = (&mut *socket).split();
    prime_parts(
        state,
        &mut sink,
        &mut source,
        body,
        plan,
        tracker,
        policy,
        deadline,
    )
    .await
}

/// Keep a proven parent on its live socket, including an empty client prewarm.
/// Closing that socket before seeding would invalidate the very ID we need.
pub(super) async fn prime_parts<S, R, E>(
    state: &AppState,
    sink: &mut S,
    source: &mut R,
    body: &Value,
    plan: &Plan,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: Deadline,
) -> Result<Seed, ServiceError>
where
    S: futures_util::Sink<UpstreamMessage, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin
        + Send,
    R: futures_util::Stream<Item = Result<UpstreamMessage, E>> + Unpin + Send,
    E: Send,
{
    let mut seed = Seed {
        previous_id: String::new(),
        bytes: 0,
        largest_message: 0,
        input_tokens: 0,
        cached_tokens: 0,
    };
    for index in 0..plan.ranges.len() - 1 {
        check_quota(state, tracker, deadline).await?;
        let text = plan
            .body(
                body,
                index,
                (!seed.previous_id.is_empty()).then_some(seed.previous_id.as_str()),
                true,
            )
            .to_string();
        seed.bytes = seed.bytes.saturating_add(text.len());
        seed.largest_message = seed.largest_message.max(text.len());
        tracker.upstream_request_bytes(seed.bytes);
        timed_until(
            state,
            deadline.instant(),
            sink.send(UpstreamMessage::Text(text.into())),
        )
        .await
        .map_err(|error| {
            if state.stop.stopped() {
                error
            } else {
                outcome_unknown(error.code)
            }
        })?;
        let until = deadline.cap(tokio::time::Instant::now() + policy.websocket_initial_timeout);
        let mut events = 0usize;
        let mut bytes = 0usize;
        let mut response_started = false;
        loop {
            let mut stop = state.stop.clone();
            let next = tokio::select! {
                _ = stop.wait() => return Err(stopping_error()),
                result = deadline::timeout_at(until, source.next()) => result.map_err(|_| outcome_unknown("response_timeout"))?,
            };
            match next {
                Some(Ok(UpstreamMessage::Text(text))) => {
                    bytes = bytes.saturating_add(text.len());
                    let mut completed = false;
                    let mut rejected = false;
                    for document in
                        super::super::event_documents::websocket(UpstreamMessage::Text(text))
                    {
                        let UpstreamMessage::Text(text) = document else {
                            unreachable!()
                        };
                        events += 1;
                        if bytes > PRELOAD_EVENT_BYTES || events > 64 {
                            tracing::info!(target:"codex_api",request_id=tracker.diagnostic_request_id(),reason="event_budget",event_bytes=bytes,event_count=events,"[NativeAPI][websocket] bootstrap=seed_unconfirmed");
                            return Err(outcome_unknown("event_budget"));
                        }
                        let event = preload_event(serde_json::from_str(&text).map_err(|_| {
                        tracing::info!(target:"codex_api",reason="event_json","[NativeAPI][websocket] bootstrap=seed_unconfirmed");
                        outcome_unknown("invalid_event_json")
                    })?);
                        tracker.quota_event(&event);
                        if !super::super::bootstrap::pre_execution(&event)
                            && !super::super::bootstrap::handshake(&event)
                        {
                            // Never replay possible model/tool output on another transport.
                            return Err(ServiceError::new(502, "websocket_context_seed_output",
                            "Upstream generated unexpected output during non-generating context preload"));
                        }
                        match event["type"].as_str() {
                            Some(
                                "response.created" | "response.queued" | "response.in_progress",
                            ) => {
                                if completed || rejected {
                                    return Err(outcome_unknown("unexpected_response_start"));
                                }
                                response_started = true;
                            }
                            Some("response.completed" | "response.done") => {
                                let response = &event["response"];
                                if completed
                                    || rejected
                                    || response["status"] != "completed"
                                    || !response["output"].as_array().is_some_and(Vec::is_empty)
                                {
                                    tracing::info!(target:"codex_api",reason="terminal_shape","[NativeAPI][websocket] bootstrap=seed_unconfirmed");
                                    return Err(outcome_unknown("invalid_completion"));
                                }
                                seed.previous_id = response["id"]
                                    .as_str()
                                    .filter(|id| !id.is_empty() && id.len() <= 512)
                                    .ok_or_else(|| outcome_unknown("missing_response_id"))?
                                    .to_owned();
                                seed.input_tokens = seed.input_tokens.saturating_add(
                                    response
                                        .pointer("/usage/input_tokens")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0),
                                );
                                seed.cached_tokens = seed.cached_tokens.saturating_add(
                                    response
                                        .pointer("/usage/input_tokens_details/cached_tokens")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0),
                                );
                                completed = true;
                            }
                            Some(
                                "error"
                                | "response.error"
                                | "response.failed"
                                | "response.incomplete"
                                | "response.cancelled",
                            ) => {
                                if !response_started
                                    && !completed
                                    && !rejected
                                    && unsupported_generate(&event)
                                {
                                    rejected = true;
                                    continue;
                                }
                                tracing::info!(target:"codex_api",reason="upstream_rejected",event_type=event["type"].as_str().unwrap_or(""),error_code=event.pointer("/error/code").or_else(|| event.pointer("/response/error/code")).and_then(|value| value.as_str()).filter(|code|code.len()<=64 && code.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'_')).unwrap_or(""),"[NativeAPI][websocket] bootstrap=seed_unconfirmed");
                                return Err(outcome_unknown("upstream_rejection"));
                            }
                            _ if super::super::bootstrap::handshake(&event)
                                || !super::super::response_wire::visible(&event, true) => {}
                            _ => {
                                tracing::info!(target:"codex_api",reason="event_type",event_type=event["type"].as_str().filter(|kind|kind.len()<=80 && kind.bytes().all(|c|c.is_ascii_alphanumeric()||matches!(c,b'_'|b'.'))).unwrap_or(""),"[NativeAPI][websocket] bootstrap=seed_unconfirmed");
                                return Err(outcome_unknown("unexpected_event"));
                            }
                        }
                    }
                    // A single upstream frame can contain both the terminal
                    // event and additional events. Validate the whole frame.
                    if rejected {
                        return Err(unavailable());
                    }
                    if completed {
                        break;
                    }
                }
                Some(Ok(UpstreamMessage::Ping(value))) => {
                    timed_until(state, Some(until), sink.send(UpstreamMessage::Pong(value)))
                        .await?;
                }
                Some(Ok(UpstreamMessage::Pong(_) | UpstreamMessage::Frame(_))) => {}
                Some(Err(_)) => return Err(outcome_unknown("receive_failed")),
                Some(Ok(UpstreamMessage::Binary(_))) => {
                    return Err(outcome_unknown("unexpected_binary_message"))
                }
                _ => {
                    tracing::info!(target:"codex_api",reason="connection_closed","[NativeAPI][websocket] bootstrap=seed_unconfirmed");
                    return Err(outcome_unknown("connection_closed"));
                }
            }
        }
    }
    // The final, generating chunk has the same reserve fence as every preload.
    check_quota(state, tracker, deadline).await?;
    Ok(seed)
}

#[cfg(test)]
#[path = "websocket_context_seed_tests.rs"]
mod tests;
