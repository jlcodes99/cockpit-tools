//! Normalize a Chat upstream into Responses before identity inspection and client adapters.
use super::{
    contract::{Route, ServiceError, WireApi},
    diagnostics::transport_failure,
    events::SseDecoder,
    lifecycle::stopping_error,
    server::AppState,
};
use axum::{body::Bytes, http::header};
use futures_util::{stream, StreamExt};
use serde_json::Value;
use std::{collections::VecDeque, sync::Arc, time::Duration};

pub(super) type ByteStream =
    futures_util::stream::BoxStream<'static, Result<Bytes, std::io::Error>>;
pub(super) struct CanonicalStream {
    pub bytes: ByteStream,
    pub is_sse: bool,
}

pub(super) async fn canonical_stream(
    state: &Arc<AppState>,
    response: reqwest::Response,
    route: &Route,
    compact: bool,
    grok_context: Option<super::grok_wire::Context>,
    plan_tools_context: Option<super::chatgpt_plan_tools::Context>,
    multi_agent_context: super::multi_agent_compat::Context,
    request_body: &Value,
    idle_timeout: Duration,
) -> Result<CanonicalStream, ServiceError> {
    let chat = route.wire_api == WireApi::Chat && !compact;
    let chat_context = if chat {
        super::chat_upstream::ToolContext::from_responses(request_body)
    } else {
        super::chat_upstream::ToolContext::default()
    };
    let convert = response.status().is_success()
        && (chat
            || grok_context.is_some()
            || plan_tools_context.is_some()
            || multi_agent_context.is_active());
    let CanonicalStream {
        bytes: mut upstream,
        is_sse,
    } = raw_stream(state, response, idle_timeout).await?;
    if !convert {
        return Ok(CanonicalStream {
            bytes: upstream,
            is_sse,
        });
    }
    if !is_sse {
        let mut data = Vec::new();
        loop {
            let mut stop = state.stop.clone();
            let next = tokio::select! {
                _ = stop.wait() => return Err(stopping_error()),
                value = tokio::time::timeout(idle_timeout,upstream.next()) => value.map_err(|_| ServiceError::new(504,"upstream_idle_timeout","Upstream response stopped responding"))?,
            };
            match next {
                Some(Ok(bytes))
                    if data.len().saturating_add(bytes.len())
                        <= state.options.max_response_bytes =>
                {
                    data.extend_from_slice(&bytes)
                }
                Some(Ok(_)) => {
                    return Err(ServiceError::new(
                        502,
                        "upstream_response_too_large",
                        "Upstream response exceeds the configured size limit",
                    ))
                }
                Some(Err(error)) => {
                    return Err(transport_failure(
                        502,
                        "upstream_stream_error",
                        "Unable to read the upstream response",
                        &error,
                    ))
                }
                None => break,
            }
        }
        let mut value: Value = serde_json::from_slice(&data).map_err(|_| {
            ServiceError::new(
                502,
                "invalid_chat_upstream_response",
                "Upstream returned invalid JSON",
            )
        })?;
        multi_agent_context.restore(&mut value);
        let mut canonical = if chat {
            super::chat_upstream::chat_to_response_with_context(
                &value,
                &route.model,
                &chat_context,
            )?
        } else if let Some(mut context) = plan_tools_context.clone() {
            super::chatgpt_plan_tools::restore_event(&mut context, value)?
                .pop()
                .ok_or_else(|| {
                    ServiceError::new(
                        502,
                        "invalid_upstream_response",
                        "Upstream returned no response",
                    )
                })?
        } else if let Some(mut context) = grok_context {
            super::grok_wire::restore_event(&mut context, value)?
                .pop()
                .ok_or_else(|| {
                    ServiceError::new(502, "invalid_grok_response", "Grok returned no response")
                })?
        } else {
            value
        };
        // Adapters can restore echoed tool declarations from their own wire
        // context. Keep the client-facing namespace canonical there as well.
        multi_agent_context.restore(&mut canonical);
        if !compact
            && super::grok_errors::applies(route)
            && super::grok_wire::compaction::triggered(request_body)
        {
            let events = super::grok_wire::compaction::events(canonical, &route.model)?;
            let data: String = events
                .iter()
                .map(|event| {
                    format!(
                        "event: {}\ndata: {event}\n\n",
                        event["type"].as_str().unwrap()
                    )
                })
                .collect();
            return Ok(CanonicalStream {
                is_sse: true,
                bytes: stream::once(async move { Ok(Bytes::from(data)) }).boxed(),
            });
        }
        return Ok(CanonicalStream {
            is_sse: false,
            bytes: stream::once(async move { Ok(Bytes::from(canonical.to_string())) }).boxed(),
        });
    }
    let mut state = ChatStream::new(
        upstream,
        if chat {
            Transform::Chat(super::chat_upstream::ResponsesStreamAdapter::with_context(
                route.model.clone(),
                chat_context,
            ))
        } else if let Some(context) = plan_tools_context {
            Transform::Plan(context)
        } else if let Some(context) = grok_context {
            Transform::Grok(context)
        } else {
            Transform::Responses
        },
        if route.wire_api == super::contract::WireApi::Chat && !route.chatgpt {
            state.options.max_provider_event_bytes
        } else {
            state.options.max_event_bytes
        },
    );
    state.multi_agent_context = multi_agent_context;
    let bytes = stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.pending.pop_front() {
                return Some((item, state));
            }
            if state.finished {
                return None;
            }
            let mut structured_failure = None;
            let result: Result<(), ServiceError> = match state.source.next().await {
                Some(Ok(bytes)) => match state.decoder.push_frames(&bytes) {
                    Ok(events) => {
                        let mut failure = None;
                        for frame in events {
                            if matches!(&state.adapter, Transform::Chat(_)) {
                                if let Some(error) =
                                    super::upstream_errors::chat_frame_failure(&frame)
                                {
                                    structured_failure =
                                        Some(super::upstream_errors::canonical_chat_failure(
                                            &frame, &error,
                                        ));
                                    failure = Some(error);
                                    break;
                                }
                            }
                            let Some(event) = frame.value else {
                                continue;
                            };
                            match state.push(event) {
                                Ok(events) => enqueue(&mut state.pending, events),
                                Err(error) => {
                                    failure = Some(error);
                                    break;
                                }
                            }
                        }
                        if failure.is_none() && state.decoder.done {
                            state.finished = true;
                            match state.adapter.finish_done() {
                                Ok(events) => enqueue(&mut state.pending, events),
                                Err(error) => failure = Some(error),
                            }
                        }
                        if failure.is_none()
                            && state.pending.is_empty()
                            && !state.finished
                            && !bytes.is_empty()
                        {
                            state
                                .pending
                                .push_back(Ok(Bytes::from_static(b": upstream activity\n\n")));
                        }
                        failure.map_or(Ok(()), Err)
                    }
                    Err(error) => Err(error),
                },
                Some(Err(error)) => Err(transport_failure(
                    502,
                    "upstream_stream_error",
                    "Upstream response stream disconnected",
                    &error,
                )),
                None => {
                    state.finished = true;
                    match state.decoder.finish_frames() {
                        Err(error) => Err(error),
                        Ok(events) => {
                            let mut result = Ok(());
                            for frame in events {
                                if matches!(&state.adapter, Transform::Chat(_)) {
                                    if let Some(error) =
                                        super::upstream_errors::chat_frame_failure(&frame)
                                    {
                                        structured_failure =
                                            Some(super::upstream_errors::canonical_chat_failure(
                                                &frame, &error,
                                            ));
                                        result = Err(error);
                                        break;
                                    }
                                }
                                let Some(event) = frame.value else {
                                    continue;
                                };
                                match state.push(event) {
                                    Ok(events) => enqueue(&mut state.pending, events),
                                    Err(error) => {
                                        result = Err(error);
                                        break;
                                    }
                                }
                            }
                            if result.is_ok() {
                                let final_events = if state.decoder.done {
                                    state.adapter.finish_done()
                                } else {
                                    state.adapter.finish()
                                };
                                result =
                                    final_events.map(|events| enqueue(&mut state.pending, events));
                            }
                            result
                        }
                    }
                }
            };
            if let Err(error) = result {
                state.finished = true;
                if let Some(event) = structured_failure {
                    enqueue(&mut state.pending, vec![event]);
                } else {
                    state
                        .pending
                        .push_back(Err(std::io::Error::other(error)));
                }
            }
        }
    })
    .boxed();
    Ok(CanonicalStream {
        bytes,
        is_sse: true,
    })
}

/// Identify HTTP JSON/SSE without translating native Chat choices or options.
pub(super) async fn raw_stream(
    state: &Arc<AppState>,
    response: reqwest::Response,
    idle_timeout: Duration,
) -> Result<CanonicalStream, ServiceError> {
    let mut is_sse = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
        });
    let success = response.status().is_success();
    let mut bytes = response
        .bytes_stream()
        .map(|value| value.map_err(std::io::Error::other))
        .boxed();
    if success && !is_sse {
        (bytes, is_sse) = detect_sse(state, bytes, idle_timeout).await?;
    }
    Ok(CanonicalStream { bytes, is_sse })
}

async fn detect_sse(
    state: &AppState,
    mut source: ByteStream,
    idle_timeout: Duration,
) -> Result<(ByteStream, bool), ServiceError> {
    const PREFIX_LIMIT: usize = 4096;
    let mut prefix = Vec::new();
    let mut buffered = VecDeque::new();
    let is_sse = loop {
        let mut stop = state.stop.clone();
        let next = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            value = tokio::time::timeout(idle_timeout, source.next()) => value
                .map_err(|_| ServiceError::new(504, "upstream_idle_timeout", "Upstream response stopped responding"))?,
        };
        match next {
            Some(Ok(bytes)) => {
                // Retain original chunks for byte-for-byte forwarding, but only
                // inspect a fixed-size prefix, even if one chunk is very large.
                let remaining = PREFIX_LIMIT.saturating_sub(prefix.len());
                prefix.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
                if !bytes.is_empty() {
                    buffered.push_back(Ok(bytes));
                }
                if let Some(is_sse) = sse_prefix(&prefix) {
                    break is_sse;
                }
                if prefix.len() == PREFIX_LIMIT {
                    return Err(ServiceError::new(
                        502,
                        "unknown_upstream_format",
                        "Upstream response format could not be identified within its prefix limit",
                    ));
                }
            }
            Some(Err(error)) => {
                return Err(transport_failure(
                    502,
                    "upstream_stream_error",
                    "Unable to read the upstream response",
                    &error,
                ));
            }
            None => break false,
        }
    };
    Ok((stream::iter(buffered).chain(source).boxed(), is_sse))
}

fn sse_prefix(prefix: &[u8]) -> Option<bool> {
    let first = prefix.iter().position(|byte| !byte.is_ascii_whitespace())?;
    let prefix = &prefix[first..];
    if prefix.starts_with(b":") {
        return Some(true);
    }
    for field in [b"data:".as_slice(), b"event:", b"id:", b"retry:"] {
        if prefix.starts_with(field) {
            return Some(true);
        }
        if field.starts_with(prefix) {
            return None;
        }
    }
    Some(false)
}

fn enqueue(queue: &mut VecDeque<Result<Bytes, std::io::Error>>, events: Vec<Value>) {
    queue.extend(events.into_iter().map(|event| {
        Ok(Bytes::from(format!(
            "event: {}\ndata: {}\n\n",
            event["type"].as_str().unwrap_or("message"),
            event
        )))
    }));
}

struct ChatStream {
    source: ByteStream,
    decoder: SseDecoder,
    adapter: Transform,
    multi_agent_context: super::multi_agent_compat::Context,
    pending: VecDeque<Result<Bytes, std::io::Error>>,
    finished: bool,
}

impl ChatStream {
    fn new(source: ByteStream, adapter: Transform, limit: usize) -> Self {
        Self {
            source,
            // Preserve output evidence until the HTTP/WS delivery collector.
            decoder: SseDecoder::without_output_repair(limit),
            adapter,
            multi_agent_context: Default::default(),
            pending: VecDeque::new(),
            finished: false,
        }
    }
    fn push(&mut self, mut event: Value) -> Result<Vec<Value>, ServiceError> {
        self.multi_agent_context.restore(&mut event);
        let mut events = self.adapter.push(&event)?;
        for event in &mut events {
            self.multi_agent_context.restore(event);
        }
        Ok(events)
    }
}

enum Transform {
    Responses,
    Chat(super::chat_upstream::ResponsesStreamAdapter),
    Grok(super::grok_wire::Context),
    Plan(super::chatgpt_plan_tools::Context),
}
impl Transform {
    fn push(&mut self, value: &Value) -> Result<Vec<Value>, ServiceError> {
        match self {
            Self::Responses => Ok(vec![value.clone()]),
            Self::Chat(adapter) => adapter.push(value),
            Self::Grok(context) => super::grok_wire::restore_event(context, value.clone()),
            Self::Plan(context) => super::chatgpt_plan_tools::restore_event(context, value.clone()),
        }
    }
    fn finish(&mut self) -> Result<Vec<Value>, ServiceError> {
        match self {
            Self::Chat(adapter) => adapter.finish(),
            Self::Grok(_) | Self::Plan(_) | Self::Responses => Ok(Vec::new()),
        }
    }
    fn finish_done(&mut self) -> Result<Vec<Value>, ServiceError> {
        match self {
            Self::Chat(adapter) => adapter.finish_done(),
            Self::Grok(_) | Self::Plan(_) | Self::Responses => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod format_tests {
    use super::sse_prefix;

    #[test]
    fn detects_complete_fields_without_guessing_from_partial_prefixes() {
        for prefix in [b"".as_slice(), b" \r\n", b"d", b"dat", b"ev", b"retr"] {
            assert_eq!(sse_prefix(prefix), None, "{prefix:?}");
        }
        for prefix in [
            b"data:".as_slice(),
            b"event:",
            b"id:",
            b"retry:",
            b": ping",
            b"\r\ndata:",
        ] {
            assert_eq!(sse_prefix(prefix), Some(true), "{prefix:?}");
        }
        for prefix in [
            b" {\"id\":".as_slice(),
            b"[]",
            b"<html>",
            b"eventual",
            b"null",
        ] {
            assert_eq!(sse_prefix(prefix), Some(false), "{prefix:?}");
        }
    }
}

#[cfg(test)]
mod output_repair_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn grok_conversion_defers_output_repair_to_http_or_websocket_delivery() {
        let (request, context) = super::super::grok_wire::prepare(
            json!({"model":"grok","tools":[{"type":"custom","name":"shell"}],"input":[]}),
        )
        .unwrap();
        let tool_name = request["tools"][0]["name"].as_str().unwrap();
        let reasoning =
            json!({"type":"reasoning","encrypted_content":"private-cipher","summary":[]});
        let wire_tool = json!({"type":"function_call","name":tool_name,"call_id":"c","arguments":"{\"input\":\"pwd\"}"});
        let wire = format!(
            "data: {}\n\ndata: {}\n\ndata: {}\n\n",
            json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
            json!({"type":"response.output_item.done","output_index":1,"item":wire_tool}),
            json!({"type":"response.completed","response":{"id":"grok-response","status":"completed","output":[]}})
        );
        let mut conversion =
            ChatStream::new(stream::empty().boxed(), Transform::Grok(context), 4096);
        let frames = conversion.decoder.push_frames(wire.as_bytes()).unwrap();
        for frame in frames {
            let events = conversion.adapter.push(&frame.value.unwrap()).unwrap();
            enqueue(&mut conversion.pending, events);
        }
        let bytes: Vec<u8> = conversion
            .pending
            .into_iter()
            .flat_map(|chunk| chunk.unwrap().to_vec())
            .collect();
        let mut ws_decoder = SseDecoder::without_output_repair(4096);
        let mut ws_events = ws_decoder.push(&bytes).unwrap();
        assert_eq!(ws_events.last().unwrap()["response"]["output"], json!([]));
        let restored_tool =
            json!({"type":"custom_tool_call","name":"shell","call_id":"c","input":"pwd"});
        assert_eq!(ws_events[1]["item"], restored_tool);
        let mut collector = super::super::output_items::OutputCollector::new();
        let changed: Vec<_> = ws_events
            .iter_mut()
            .map(|event| collector.observe(event).unwrap())
            .collect();
        assert_eq!(changed, vec![false, false, true]);
        assert_eq!(
            ws_events.last().unwrap()["response"]["output"],
            json!([reasoning, restored_tool])
        );
        let mut http_decoder = SseDecoder::new(4096);
        let http_events = http_decoder.push(&bytes).unwrap();
        assert_eq!(
            http_events.last().unwrap()["response"]["output"],
            json!([reasoning, restored_tool])
        );
    }
}
