//! Preserve native Chat/legacy Completions options when the selected provider
//! already speaks that protocol. OAuth routes still use the Responses adapter.
use crate::modules::codex_native_api::deadline::{self, Deadline};
use super::{
    contract::{ServiceError, WireApi},
    diagnostics::transport_failure,
    events::{retry_after_headers, SseDecoder, SseFrame},
    lifecycle::{stopping_error, RequestTracker},
    mapping::Prepared,
    server::{downstream_response, AppState},
};
use axum::{
    body::{Body, Bytes},
    response::Response,
};
use futures_util::{stream, StreamExt};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone)]
pub(super) struct RawChat {
    pub body: Value,
    pub endpoint: &'static str,
    pub conversion_error: Option<ServiceError>,
}
#[derive(Clone)]
pub(super) struct Passthrough;

/// Immediate SSE starts before selection knows which upstream protocol will
/// be used. Adapters read this flag only after receiving a real data frame.
#[derive(Clone, Default)]
pub(super) struct DeferredProtocol(pub std::sync::Arc<std::sync::atomic::AtomicBool>);
impl DeferredProtocol {
    pub(super) fn is_native(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(super) fn native(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

pub(super) async fn forward(
    state: Arc<AppState>,
    prepared: Prepared,
    mut tracker: RequestTracker,
    raw: RawChat,
) -> Result<Response, ServiceError> {
    if prepared.route.wire_api != WireApi::Chat || prepared.route.chatgpt {
        let error = ServiceError::new(
            400,
            "native_chat_protocol_required",
            "Native Chat requests require a Chat provider account",
        );
        tracker.fail(&error).await;
        return Err(error);
    }
    let streaming = raw.body["stream"].as_bool().unwrap_or(false);
    let mut policy = if let Some(policy) = super::execution::policy() {
        policy
    } else {
        match super::retry::execution_policy(&state).await {
            Ok(policy) => policy,
            Err(error) => {
                tracker.fail(&error).await;
                return Err(error);
            }
        }
    };
    if streaming && policy.immediate_sse {
        let keepalive = policy.keepalive;
        let deadline = prepared.execution.total_deadline;
        let mut response = super::http::immediate_stream(
            Box::pin(forward_with_policy(state, prepared, tracker, raw, policy)),
            keepalive,
            deadline,
        );
        response.extensions_mut().insert(Passthrough);
        return Ok(response);
    }
    forward_with_policy(state, prepared, tracker, raw, policy).await
}

async fn forward_with_policy(
    state: Arc<AppState>,
    mut prepared: Prepared,
    mut tracker: RequestTracker,
    raw: RawChat,
    policy: super::contract::ExecutionPolicy,
) -> Result<Response, ServiceError> {
    let streaming = raw.body["stream"].as_bool().unwrap_or(false);
    let expected_choices = raw.body["n"].as_u64().unwrap_or(1).max(1) as usize;
    let deadline = prepared
        .execution
        .total_deadline
        .min(tracker.deadline(policy.total_timeout));
    let mut bootstrap_attempts = 0;
    let (status, mut headers, source, is_sse) = loop {
        let response = match super::retry::send_native_chat(
            &state,
            &mut prepared,
            &mut tracker,
            &policy,
            &raw.body,
            raw.endpoint,
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                tracker.fail(&error).await;
                return Err(error);
            }
        };
        let status = response.status();
        let headers = response.headers().clone();
        tracker.set_retry_after(retry_after_headers(&headers));
        if let Err(error) = super::mapping::observe_headers(&state, &prepared, &headers).await {
            tracker.fail(&error).await;
            return Err(error);
        }
        let mut stop = state.stop.clone();
        let canonical = tokio::select! {
            _ = stop.wait() => Err(stopping_error()),
            result = deadline::timeout_at(deadline, super::upstream::raw_stream(&state, response, policy.idle_timeout)) => result.map_err(|_| super::retry::total_timeout()).and_then(|value| value),
        };
        let canonical = match canonical {
            Ok(value) => value,
            Err(error) => {
                tracker.fail(&error).await;
                return Err(error);
            }
        };
        let mut source = canonical.bytes;
        let is_sse = canonical.is_sse;
        if streaming && is_sse && status.is_success() {
            match super::http::prefetch(&state, source, &policy, deadline).await {
                Ok(value) => source = value,
                Err(error)
                    if bootstrap_attempts < policy.bootstrap_retries
                        && !state.stop.stopped()
                        && tokio::time::Instant::now() < deadline =>
                {
                    tracker.attempt_error(&error);
                    bootstrap_attempts += 1;
                    if let Err(error) = super::retry::delay(
                        &state,
                        policy.retry_base_delay,
                        policy.retry_max_delay,
                        deadline,
                    )
                    .await
                    {
                        tracker.fail(&error).await;
                        return Err(error);
                    }
                    prepared.execution = prepared.execution.renew_open();
                    continue;
                }
                Err(error) => {
                    tracker.fail(&error).await;
                    return Err(error);
                }
            }
        }
        break (status, headers, source, is_sse);
    };
    if is_sse && status.is_success() {
        headers.remove(axum::http::header::CONTENT_LENGTH);
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
    }
    let mut result = if streaming && is_sse && status.is_success() {
        let current = StreamState {
            state: state.clone(),
            source,
            tracker,
            decoder: SseDecoder::new(state.options.max_provider_event_bytes),
            usage: None,
            terminal: false,
            finished: false,
            idle: policy.idle_timeout,
            idle_deadline: tokio::time::Instant::now() + policy.idle_timeout,
            deadline,
            keepalive: policy.keepalive,
            keepalive_deadline: tokio::time::Instant::now() + policy.keepalive,
            finished_choices: BTreeSet::new(),
            expected_choices,
            pending_error: None,
        };
        let stream = stream::unfold(current, |mut current| async move {
            loop {
                if current.finished {
                    return None;
                }
                let mut stop = current.state.stop.clone();
                let next = tokio::select! {
                    _ = stop.wait() => Err(stopping_error()),
                    _ = deadline::sleep_until(current.deadline) => Err(super::retry::total_timeout()),
                    _ = deadline::sleep_until(current.idle_deadline) => Err(ServiceError::new(504,"upstream_idle_timeout","Upstream stream stopped responding")),
                    _ = deadline::sleep_until(current.keepalive_deadline), if !current.keepalive.is_zero() => {
                        current.keepalive_deadline = tokio::time::Instant::now() + current.keepalive;
                        return Some((Ok::<_, std::io::Error>(Bytes::from_static(b": keep-alive\n\n")), current));
                    },
                    value = current.source.next() => Ok(value),
                };
                let eof = matches!(next, Ok(None));
                let frames = match next {
                    Ok(Some(Ok(bytes))) => {
                        if !bytes.is_empty() {
                            current.tracker.first_response();
                            current.idle_deadline = tokio::time::Instant::now() + current.idle;
                        }
                        current.decoder.push_frames(&bytes)
                    }
                    Ok(None) => current.decoder.finish_frames(),
                    Ok(Some(Err(error))) => Err(transport_failure(
                        502,
                        "upstream_stream_error",
                        "Chat upstream disconnected",
                        &error,
                    )),
                    Err(error) => Err(error),
                };
                let result = frames.and_then(|frames| current.observe_frames(frames));
                match result {
                    Ok(bytes) => {
                        if let Some(error) = current.pending_error.take() {
                            current.tracker.fail(&error).await;
                            current.finished = true;
                            return Some((Ok(bytes), current));
                        }
                        if eof || current.decoder.done {
                            if current.terminal {
                                current
                                    .tracker
                                    .complete(200, true, current.usage.take(), None)
                                    .await;
                                current.finished = true;
                            } else {
                                let error = ServiceError::new(
                                    502,
                                    "incomplete_upstream_stream",
                                    "Chat upstream closed before completing all response choices",
                                );
                                current.tracker.fail(&error).await;
                                current.finished = true;
                                let mut output = bytes.to_vec();
                                output.extend_from_slice(
                                    format!(
                                        "data: {}\n\n",
                                        super::output_error::json_error(&error)
                                    )
                                    .as_bytes(),
                                );
                                return Some((Ok(Bytes::from(output)), current));
                            }
                        }
                        if !bytes.is_empty() {
                            return Some((Ok(bytes), current));
                        }
                        if current.finished {
                            return None;
                        }
                    }
                    Err(error) => {
                        current.tracker.fail(&error).await;
                        current.finished = true;
                        return Some((
                            Ok(Bytes::from(format!(
                                "data: {}\n\n",
                                super::output_error::json_error(&error)
                            ))),
                            current,
                        ));
                    }
                }
            }
        });
        downstream_response(status, &headers, Body::from_stream(stream))
    } else {
        let mut source = source;
        let mut bytes = Vec::new();
        loop {
            let mut stop = state.stop.clone();
            let result = tokio::select! {
                _ = stop.wait() => Err(stopping_error()),
                _ = deadline::sleep_until(deadline) => Err(super::retry::total_timeout()),
                value = tokio::time::timeout(policy.idle_timeout, source.next()) => value
                    .map_err(|_| ServiceError::new(504, "upstream_idle_timeout", "Upstream response stopped responding")),
            };
            let item = match result {
                Ok(Some(Ok(item))) => item,
                Ok(None) => break,
                result => {
                    let error = match result {
                        Err(error) => error,
                        Ok(Some(Err(error))) => transport_failure(
                            502,
                            "upstream_stream_error",
                            "Upstream disconnected",
                            &error,
                        ),
                        _ => {
                            ServiceError::new(502, "upstream_stream_error", "Upstream disconnected")
                        }
                    };
                    tracker.fail(&error).await;
                    return Err(error);
                }
            };
            if bytes.len().saturating_add(item.len()) > state.options.max_response_bytes {
                let error = ServiceError::new(
                    502,
                    "upstream_response_too_large",
                    "Upstream response exceeds its size limit",
                );
                tracker.fail(&error).await;
                return Err(error);
            }
            if !item.is_empty() {
                tracker.first_response();
            }
            bytes.extend_from_slice(&item);
        }
        let value: Option<Value> = serde_json::from_slice(&bytes).ok();
        if let Some(value) = &value {
            tracker.capture_upstream_error(status.as_u16(), value);
            if super::upstream_errors::chat_failure(value, status.as_u16()).is_some() {
                let error = chat_error(value, status.as_u16());
                tracker.attempt_error(&error);
            }
        }
        if headers
            .get("cf-mitigated")
            .is_some_and(|value| value == "challenge")
        {
            tracker.capture_response_error(Some("cloudflare challenge"), false);
            tracker.capture_request_fault(false);
        }
        let success = status.is_success()
            && value.as_ref().is_some_and(|value| {
                super::upstream_errors::chat_failure(value, status.as_u16()).is_none()
                    && value
                        .get("choices")
                        .and_then(Value::as_array)
                        .is_some_and(|choices| !choices.is_empty())
            });
        if status.is_success() && !success {
            let error = value
                .as_ref()
                .filter(|value| {
                    super::upstream_errors::chat_failure(value, status.as_u16()).is_some()
                })
                .map(|value| chat_error(value, status.as_u16()))
                .unwrap_or_else(|| {
                    ServiceError::new(
                        502,
                        "invalid_chat_upstream_response",
                        "Chat upstream returned no valid choices",
                    )
                });
            tracker.fail(&error).await;
            return Err(error);
        }
        let usage = value
            .as_ref()
            .and_then(|value| value.get("usage"))
            .map(normalize_usage);
        tracker.payload(
            "upstream_response",
            "http",
            &headers,
            &String::from_utf8_lossy(&bytes),
        );
        tracker
            .complete(
                status.as_u16(),
                success,
                usage,
                (!status.is_success()).then(|| "upstream_http_error".into()),
            )
            .await;
        downstream_response(status, &headers, Body::from(bytes))
    };
    result.extensions_mut().insert(Passthrough);
    Ok(result)
}
fn normalize_usage(value: &Value) -> Value {
    let mut usage = value.clone();
    if let Some(object) = usage.as_object_mut() {
        if let Some(input) = value.get("prompt_tokens") {
            object.insert("input_tokens".into(), input.clone());
        }
        if let Some(output) = value.get("completion_tokens") {
            object.insert("output_tokens".into(), output.clone());
        }
        if let Some(details) = value.get("prompt_tokens_details") {
            object.insert("input_tokens_details".into(), details.clone());
        }
        if let Some(details) = value.get("completion_tokens_details") {
            object.insert("output_tokens_details".into(), details.clone());
        }
    }
    usage
}
struct StreamState {
    state: Arc<AppState>,
    source: super::upstream::ByteStream,
    tracker: RequestTracker,
    decoder: SseDecoder,
    usage: Option<Value>,
    terminal: bool,
    finished: bool,
    idle: std::time::Duration,
    idle_deadline: tokio::time::Instant,
    deadline: Deadline,
    keepalive: std::time::Duration,
    keepalive_deadline: tokio::time::Instant,
    finished_choices: BTreeSet<u64>,
    expected_choices: usize,
    pending_error: Option<ServiceError>,
}

impl StreamState {
    fn observe_frames(&mut self, frames: Vec<SseFrame>) -> Result<Bytes, ServiceError> {
        let mut bytes = Vec::new();
        for frame in frames {
            if let Some(error) = super::upstream_errors::chat_frame_failure(&frame) {
                bytes.extend_from_slice(
                    format!("data: {}\n\n", super::output_error::json_error(&error)).as_bytes(),
                );
                self.pending_error = Some(error);
                break;
            }
            if let Some(event) = frame.value {
                if let Some(usage) = event.get("usage").filter(|value| !value.is_null()) {
                    self.usage = Some(normalize_usage(usage));
                    self.tracker.capture_usage(&self.usage);
                }
                if let Some(choices) = event.get("choices").and_then(Value::as_array) {
                    for choice in choices {
                        if choice
                            .get("finish_reason")
                            .is_some_and(|value| !value.is_null())
                        {
                            let index = choice
                                .get("index")
                                .and_then(Value::as_u64)
                                .or_else(|| (self.expected_choices == 1).then_some(0))
                                .ok_or_else(|| {
                                    ServiceError::new(
                                        502,
                                        "invalid_chat_upstream_response",
                                        "A multi-choice Chat chunk requires choice indices",
                                    )
                                })?;
                            self.finished_choices.insert(index);
                        }
                    }
                }
            }
            // A premature [DONE] must not advertise success before the error.
            let done = std::str::from_utf8(&frame.bytes).is_ok_and(|text| {
                text.lines().any(|line| {
                    line.strip_prefix("data:")
                        .is_some_and(|data| data.trim() == "[DONE]")
                })
            });
            if !done || self.finished_choices.len() >= self.expected_choices {
                bytes.extend_from_slice(&frame.bytes);
            }
        }
        self.terminal = self.finished_choices.len() >= self.expected_choices;
        Ok(bytes.into())
    }
}
fn chat_error(value: &Value, fallback: u16) -> ServiceError {
    super::upstream_errors::chat_failure(value, fallback).unwrap_or_else(|| {
        let (message, scope) = super::diagnostics::upstream_error_metadata_status(fallback, value);
        ServiceError::new(
            fallback,
            if super::upstream_errors::request_fault(fallback, value) {
                super::upstream_errors::REQUEST_SCOPED
            } else {
                "upstream_error"
            },
            message.unwrap_or_else(|| "Chat upstream returned an error".into()),
        )
        .with_credential_scope(scope)
    })
}

#[cfg(test)]
#[path = "native_chat_bootstrap_tests.rs"]
mod bootstrap_tests;
