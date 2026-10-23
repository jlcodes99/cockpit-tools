use crate::modules::codex_native_api::deadline::{self, Deadline};
use std::{io::Read, sync::Arc};

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
};
use futures_util::{stream, StreamExt};
use serde_json::Value;

use super::{
    contract::{ExecutionPolicy, ServiceError},
    diagnostics::transport_failure,
    events::{retry_after_headers, ResponseObservation, SseDecoder, SseFrame},
    lifecycle::{stopping_error, RequestTracker},
    mapping::{self, Prepared},
    server::{downstream_response, AppState},
};

#[path = "http_pre_business.rs"]
mod pre_business;

pub(super) async fn responses(State(state): State<Arc<AppState>>, request: Request) -> Response {
    handle(state, request, false)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

pub(super) async fn compact(State(state): State<Arc<AppState>>, request: Request) -> Response {
    handle(state, request, true)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn handle(
    state: Arc<AppState>,
    request: Request,
    compact: bool,
) -> Result<Response, ServiceError> {
    let input = read_request(&state, request).await?;
    dispatch(state, input, compact).await
}

pub(super) struct Input {
    pub tracker: RequestTracker,
    pub caller: super::contract::Caller,
    pub headers: HeaderMap,
    pub body: Value,
    pub raw_chat: Option<super::native_chat::RawChat>,
}

pub(super) async fn read_request(
    state: &AppState,
    request: Request,
) -> Result<Input, ServiceError> {
    let mut tracker = state.begin()?;
    let (parts, body) = request.into_parts();
    let capture = tokio::time::timeout(
        state.options.request_timeout,
        state.host.payload_logging_enabled(),
    )
    .await
    .unwrap_or(false);
    tracker.context(
        parts.method.as_str(),
        parts.uri.path(),
        &parts.headers,
        capture,
    );
    let result=async {
        let caller = state.authorize(&parts.headers).await?;
        tracker.authorized(&caller,"");
        let mut stop = state.stop.clone();
        let bytes = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            result = tokio::time::timeout(state.options.request_timeout, to_bytes(body, state.options.max_body_bytes)) =>
                result.map_err(|_| ServiceError::new(408, "request_body_timeout", "Timed out reading request body"))?
                    .map_err(super::request_body::read_error)?,
        };
        let bytes = decode_body(state, bytes, &parts.headers).await?;
        let small = bytes.len() <= 64 * 1024;
        let parse = move || {
            serde_json::from_slice(&bytes).map_err(|_| ServiceError::new(400,"invalid_json","Request body must be a JSON object"))
        };
        let body: Value = if small { parse()? } else {
            super::blocking::run(state.stop.clone(), state.options.request_timeout, parse).await?
        };
        if !body.is_object() { return Err(ServiceError::new(400,"invalid_json","Request body must be a JSON object")); }
        Ok((caller,body))
    }.await;
    match result {
        Ok((caller, body)) => {
            tracker.authorized(&caller, body["model"].as_str().unwrap_or(""));
            tracker.payload("client_request", "http", &parts.headers, &body.to_string());
            Ok(Input {
                tracker,
                caller,
                headers: parts.headers,
                body,
                raw_chat: None,
            })
        }
        Err(error) => {
            tracker.fail(&error).await;
            Err(error)
        }
    }
}

pub(super) async fn dispatch(
    state: Arc<AppState>,
    mut input: Input,
    compact: bool,
) -> Result<Response, ServiceError> {
    let mut policy = match super::retry::execution_policy(&state).await {
        Ok(policy) => policy,
        Err(error) => {
            input.tracker.fail(&error).await;
            return Err(error);
        }
    };
    if mapping::image_generation_requested(&input.body)
        || super::images::is_forced_generation(&input.body)
    {
        policy = policy.for_images();
    }
    let immediate =
        !compact && input.body["stream"].as_bool().unwrap_or(false) && policy.immediate_sse;
    let codex_client = super::output_error::is_codex_client(&input.headers);
    let keepalive = policy.keepalive;
    // The outer stream covers selection and credentials, so selected transports
    // must not wrap a second immediate stream around their own opening stage.
    policy.immediate_sse = false;
    let execution = super::execution::Execution::new(&input.tracker, &policy);
    let total_deadline = execution.total_deadline;
    let protocol = super::native_chat::DeferredProtocol::default();
    if immediate {
        let deferred = protocol.clone();
        let mut response = immediate_stream_for_client(
            Box::pin(async move {
                execution
                    .run(dispatch_scoped(state, input, compact, protocol))
                    .await
            }),
            keepalive,
            total_deadline,
            codex_client,
        );
        response.extensions_mut().insert(deferred);
        return Ok(response);
    }
    execution
        .run(dispatch_scoped(state, input, compact, protocol))
        .await
}

async fn dispatch_scoped(
    state: Arc<AppState>,
    mut input: Input,
    compact: bool,
    protocol: super::native_chat::DeferredProtocol,
) -> Result<Response, ServiceError> {
    mapping::apply_request_image_policy(&input.headers, &mut input.body);
    if let Some(raw) = &mut input.raw_chat {
        mapping::apply_request_image_policy(&input.headers, &mut raw.body);
    }
    if !compact && super::images::is_forced_generation(&input.body) {
        return super::images::responses(state, input).await;
    }
    dispatch_responses(state, input, compact, protocol).await
}

pub(super) async fn dispatch_responses(
    state: Arc<AppState>,
    input: Input,
    compact: bool,
    protocol: super::native_chat::DeferredProtocol,
) -> Result<Response, ServiceError> {
    let Input {
        mut tracker,
        caller,
        headers,
        body,
        raw_chat,
    } = input;
    let client_stream = !compact && body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let requested_model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let prepared = if compact {
        mapping::prepare_compact(&state, &caller, &headers, body, None).await
    } else {
        mapping::prepare(&state, &caller, &headers, body, None).await
    };
    let mut prepared = match prepared {
        Ok(value) => value,
        Err(error) => {
            tracker.fail(&error).await;
            return Err(error);
        }
    };
    tracker.routed(&caller, &prepared.route, &requested_model);
    tracker.routing_identity(&prepared.request);
    tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
    tracker.prepared_metadata(&prepared);
    if let Some(raw) = raw_chat {
        if prepared.route.wire_api == super::contract::WireApi::Chat {
            protocol.native();
            prepared.raw_chat = Some(raw.clone());
            return super::native_chat::forward(state, prepared, tracker, raw).await;
        }
        if let Some(error) = raw.conversion_error {
            tracker.fail(&error).await;
            return Err(error);
        }
    }

    match forward(state, prepared, compact, client_stream, tracker).await {
        Ok(response) => Ok(response),
        Err(error) => Err(error),
    }
}

pub(super) async fn dispatch_selected(
    state: Arc<AppState>,
    input: Input,
    selected: super::contract::ImageRoute,
    request: super::contract::ImageRouteRequest,
) -> Result<Response, ServiceError> {
    let Input {
        mut tracker,
        caller,
        headers,
        body,
        ..
    } = input;
    let client_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let requested_model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let prepared =
        match mapping::prepare_selected(&state, &caller, &headers, body, selected, request).await {
            Ok(prepared) => prepared,
            Err(error) => {
                tracker.fail(&error).await;
                return Err(error);
            }
        };
    tracker.routed(&caller, &prepared.route, &requested_model);
    tracker.routing_identity(&prepared.request);
    tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
    tracker.prepared_metadata(&prepared);
    forward(state, prepared, false, client_stream, tracker).await
}

pub(super) async fn decode_body(
    state: &AppState,
    bytes: Bytes,
    headers: &HeaderMap,
) -> Result<Vec<u8>, ServiceError> {
    decode_body_with_limit(state, bytes, headers, state.options.max_body_bytes).await
}

pub(super) async fn decode_body_with_limit(
    state: &AppState,
    bytes: Bytes,
    headers: &HeaderMap,
    limit: usize,
) -> Result<Vec<u8>, ServiceError> {
    let encodings = headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .map(|value| value.to_str())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            ServiceError::new(
                415,
                "unsupported_content_encoding",
                "Invalid content encoding",
            )
        })?
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if encodings
        .iter()
        .any(|encoding| !matches!(encoding.as_str(), "identity" | "gzip" | "zstd"))
    {
        return Err(ServiceError::new(
            415,
            "unsupported_content_encoding",
            "Supported content encodings are identity, gzip and zstd",
        ));
    }
    if encodings.iter().all(|encoding| encoding == "identity") {
        return Ok(bytes.to_vec());
    }
    super::blocking::run(
        state.stop.clone(),
        state.options.request_timeout,
        move || {
            let mut input = bytes.to_vec();
            // Content-Encoding lists the order of application, so undo it in reverse.
            // Bound every intermediate layer as well as the final JSON body.
            for encoding in encodings.iter().rev() {
                if encoding == "identity" {
                    continue;
                }
                let reader: Box<dyn Read + '_> = if encoding == "gzip" {
                    Box::new(flate2::read::GzDecoder::new(input.as_slice()))
                } else {
                    Box::new(
                        zstd::stream::read::Decoder::new(input.as_slice()).map_err(|_| {
                            ServiceError::new(
                                400,
                                "invalid_compressed_body",
                                "Invalid compressed request body",
                            )
                        })?,
                    )
                };
                let mut output = Vec::new();
                reader
                    .take((limit as u64).saturating_add(1))
                    .read_to_end(&mut output)
                    .map_err(|_| {
                        ServiceError::new(
                            400,
                            "invalid_compressed_body",
                            "Invalid compressed request body",
                        )
                    })?;
                if output.len() > limit {
                    return Err(ServiceError::new(
                        413,
                        "request_too_large",
                        "Decompressed request exceeds the configured size limit",
                    ));
                }
                input = output;
            }
            Ok(input)
        },
    )
    .await
}

pub(super) async fn forward(
    state: Arc<AppState>,
    prepared: Prepared,
    compact: bool,
    client_stream: bool,
    mut tracker: RequestTracker,
) -> Result<Response, ServiceError> {
    let policy = if let Some(policy) = super::execution::policy() {
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
    let mut policy =
        if prepared.request.image_generation_requested || prepared.image_route_request.is_some() {
            policy.for_images()
        } else {
            policy
        };
    if client_stream && policy.immediate_sse {
        return Ok(immediate_response(
            state, prepared, compact, tracker, policy,
        ));
    }
    forward_with_policy(state, prepared, compact, client_stream, tracker, policy).await
}

async fn forward_with_policy(
    state: Arc<AppState>,
    mut prepared: Prepared,
    compact: bool,
    client_stream: bool,
    mut tracker: RequestTracker,
    policy: ExecutionPolicy,
) -> Result<Response, ServiceError> {
    let deadline = prepared
        .execution
        .total_deadline
        .min(tracker.deadline(policy.total_timeout));
    let HttpResponseStream {
        bytes: upstream,
        status,
        mut headers,
        is_sse,
    } = match open_response_stream(
        &state,
        &mut prepared,
        compact,
        client_stream,
        &mut tracker,
        &policy,
        deadline,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            tracker.fail(&error).await;
            return Err(error);
        }
    };
    if prepared.route.chatgpt_plan && status.is_success() && !is_sse {
        let error = ServiceError::new(502, "incomplete_upstream_stream",
            "ChatGPT plan inference requires a Responses event stream");
        tracker.fail(&error).await;
        return Err(error);
    }
    if status.is_success()
        && (is_sse || (!compact && prepared.route.wire_api == super::contract::WireApi::Chat))
    {
        headers.remove(header::CONTENT_LENGTH);
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(if is_sse {
                "text/event-stream"
            } else {
                "application/json"
            }),
        );
    }
    if is_sse && status.is_success() && client_stream {
        let event_limit = state.options.max_event_bytes;
        let fence = super::bootstrap::Fence::new(!prepared.route.chatgpt_plan);
        let stream_state = StreamingResponse {
            state,
            prepared,
            upstream,
            decoder: SseDecoder::new(event_limit),
            observation: ResponseObservation {
                retry_after: retry_after_headers(&headers),
                ..Default::default()
            },
            tracker,
            fence,
            recovery_generation: 0,
            finished: false,
            sequence: 0,
            pending_error: None,
            idle_deadline: tokio::time::Instant::now() + policy.idle_timeout,
            keepalive_deadline: tokio::time::Instant::now() + policy.keepalive,
            deadline,
            policy,
        };
        let body = Body::from_stream(stream::unfold(stream_state, |mut current| async move {
            loop {
                if current.finished {
                    return None;
                }
                let mut stop = current.state.stop.clone();
                let next = if let Some(error) = current.pending_error.take() {
                    Err(error)
                } else {
                    tokio::select! {
                        _ = stop.wait() => Err(stopping_error()),
                        _ = deadline::sleep_until(current.deadline) => Err(super::retry::total_timeout()),
                        _ = deadline::sleep_until(current.idle_deadline) => Err(ServiceError::new(504, "upstream_idle_timeout", "Upstream stream stopped responding")),
                        _ = deadline::sleep_until(current.keepalive_deadline), if !current.policy.keepalive.is_zero() => {
                            current.keepalive_deadline = tokio::time::Instant::now() + current.policy.keepalive;
                            return Some((Ok::<_, std::io::Error>(Bytes::from_static(b": keep-alive\n\n")), current));
                        },
                        next = current.upstream.next() => Ok(next),
                    }
                };
                let outcome = match next {
                    Ok(Some(Ok(bytes))) => {
                        if !bytes.is_empty() {
                            current.tracker.first_response();
                            current.idle_deadline =
                                tokio::time::Instant::now() + current.policy.idle_timeout;
                        }
                        match current.decoder.push_frames(&bytes) {
                            Ok(frames) => current.observe_frames(frames).await,
                            Err(error) => Err(error),
                        }
                    }
                    Ok(None) => { let generation=current.recovery_generation; match current.decoder.finish_frames() {
                        Ok(frames) => match current.observe_frames(frames).await {
                            Ok(bytes) if current.observation.terminal || generation!=current.recovery_generation => Ok(bytes),
                            Ok(bytes) => {
                                let error = ServiceError::new(
                                    502,
                                    "incomplete_upstream_stream",
                                    "Upstream stream ended before a terminal response",
                                );
                                if bytes.is_empty() {
                                    Err(error)
                                } else {
                                    current.pending_error = Some(error);
                                    Ok(bytes)
                                }
                            }
                            Err(error) => Err(error),
                        },
                        Err(error) => Err(error),
                    }},
                    Ok(Some(Err(error))) => Err(transport_failure(
                        502,
                        "upstream_stream_error",
                        "Upstream stream disconnected",
                        &error,
                    )),
                    Err(error) => Err(error),
                };
                match outcome {
                    Ok(bytes) => {
                        if current.observation.terminal {
                            current.complete().await;
                        }
                        if !bytes.is_empty() {
                            return Some((Ok(bytes), current));
                        }
                        if current.finished {
                            return None;
                        }
                    }
                    Err(error) => {
                        current.finished = true;
                        current.tracker.fail(&error).await;
                        let codex =
                            super::output_error::is_codex_client(&current.prepared.source_headers);
                        return Some((
                            Ok(super::output_error::responses_sse(
                                &error,
                                codex,
                                current.sequence,
                            )),
                            current,
                        ));
                    }
                }
            }
        }));
        return Ok(downstream_response(status, &headers, body));
    }
    if is_sse && status.is_success() {
        let mut observation =
            match collect_sse(&state, &mut prepared, upstream, &mut tracker, &policy, deadline,
                retry_after_headers(&headers)).await {
                Ok(observation) => observation,
                Err(error) => {
                    tracker.fail(&error).await;
                    return Err(error);
                }
            };
        let Some(response) = observation.response.take().filter(|_| observation.terminal) else {
            let error = ServiceError::new(
                502,
                "incomplete_upstream_stream",
                "Upstream stream ended without a final response",
            );
            tracker.fail(&error).await;
            return Err(error);
        };
        if !observation.succeeded && response.get("error").is_some_and(|error| !error.is_null()) {
            let event = serde_json::json!({"type":"response.failed","response":response});
            let error =
                super::output_error::event_failure(&event, observation.http_status.unwrap_or(502));
            tracker.fail(&error).await;
            return Err(error);
        }
        tracker.payload("upstream_response", "http", &headers, &response.to_string());
        tracker.set_retry_after(observation.retry_after);
        tracker.response_tier(observation.service_tier.as_deref());
        tracker.capture_response_error(
            observation.error_message.as_deref(),
            observation.credential_scope,
        );
        tracker.capture_request_fault(observation.request_scoped);
        tracker
            .complete(
                observation.http_status.unwrap_or(status.as_u16()),
                observation.succeeded,
                observation.usage,
                observation.error,
            )
            .await;
        let mut response_headers = headers;
        response_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        return Ok(downstream_response(
            status,
            &response_headers,
            Body::from(response.to_string()),
        ));
    }
    let bytes = match read_bounded(&state, upstream, &mut tracker, &policy, deadline).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracker.fail(&error).await;
            return Err(error);
        }
    };
    tracker.payload(
        "upstream_response",
        "http",
        &headers,
        &String::from_utf8_lossy(&bytes),
    );
    let mut value = serde_json::from_slice::<Value>(&bytes).ok();
    let mut normalized = false;
    if !compact {
        if let Some(value) = &mut value {
            normalized = super::output_usage::normalize_usage_details(value);
        }
    }
    if let Some(value) = &value {
        tracker.capture_upstream_error(status.as_u16(), value);
        if status.is_success() {
            if super::upstream_errors::chat_failure(value, 502).is_some() {
                let error = super::output_error::event_failure(value, 502);
                tracker.fail(&error).await;
                return Err(error);
            }
        }
    } else if !status.is_success() {
        tracker.capture_upstream_error(
            status.as_u16(),
            &Value::String(String::from_utf8_lossy(&bytes).into_owned()),
        );
    }
    if headers
        .get("cf-mitigated")
        .is_some_and(|value| value == "challenge")
    {
        tracker.capture_response_error(Some("cloudflare challenge"), false);
        tracker.capture_request_fault(false);
    }
    if status.is_success()
        && !value.as_ref().is_some_and(|value| {
            value
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
                && value.get("output").is_some_and(Value::is_array)
        })
    {
        let error = ServiceError::new(
            502,
            "invalid_upstream_response",
            "Upstream did not return a valid Responses object or event stream",
        );
        tracker.fail(&error).await;
        return Err(error);
    }
    if let Some(value) = &value {
        tracker.response_tier(value.get("service_tier").and_then(Value::as_str));
        if let Err(error) = mapping::observe_event(&state, &prepared, value).await {
            tracker.fail(&error).await;
            return Err(error);
        }
    }
    let failed_terminal = value
        .as_ref()
        .and_then(|value| super::events::response_terminal_kind(value))
        .filter(|kind| !matches!(*kind, "response.completed" | "response.done"));
    tracker
        .complete(
            status.as_u16(),
            status.is_success() && failed_terminal.is_none(),
            value.as_ref().and_then(|v| v.get("usage")).cloned(),
            failed_terminal.map(str::to_owned).or_else(|| {
                (!status.is_success()).then(|| format!("upstream_http_{}", status.as_u16()))
            }),
        )
        .await;
    let bytes = if normalized {
        value.unwrap().to_string().into_bytes()
    } else {
        bytes
    };
    Ok(downstream_response(status, &headers, Body::from(bytes)))
}

pub(super) struct HttpResponseStream {
    pub status: axum::http::StatusCode,
    pub headers: HeaderMap,
    pub bytes: super::upstream::ByteStream,
    pub is_sse: bool,
}

/// Open and normalize HTTP upstreams without consuming the request/session owner.
/// Both HTTP delivery and the downstream WebSocket bridge share this retry fence.
#[allow(clippy::too_many_arguments)]
pub(super) async fn open_response_stream(
    state: &Arc<AppState>,
    prepared: &mut Prepared,
    compact: bool,
    client_stream: bool,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: Deadline,
) -> Result<HttpResponseStream, ServiceError> {
    let mut bootstrap_attempts = 0;
    loop {
        let response = super::retry::send(state, prepared, tracker, compact, policy).await?;
        let status = response.status();
        let headers = response.headers().clone();
        tracker.quota_headers(&headers);
        tracker.set_retry_after(retry_after_headers(&headers));
        mapping::observe_headers(state, prepared, &headers).await?;
        let mut stop = state.stop.clone();
        let mut upstream = tokio::select! {
            _ = stop.wait() => Err(stopping_error()),
            result = deadline::timeout_at(deadline, super::upstream::canonical_stream(state, response, &prepared.route, compact, prepared.grok_context.clone(), prepared.plan_tools_context.clone(), prepared.multi_agent_context.clone(), &prepared.body, policy.idle_timeout)) => result.map_err(|_| super::retry::total_timeout()).and_then(|value| value),
        }?;
        if upstream.is_sse && status.is_success() && client_stream {
            match prefetch(state, upstream.bytes, policy, deadline).await {
                Ok(bytes) => upstream.bytes = bytes,
                Err(error)
                    if bootstrap_attempts < policy.bootstrap_retries
                        && !state.stop.stopped()
                        && tokio::time::Instant::now() < deadline =>
                {
                    tracker.attempt_error(&error);
                    bootstrap_attempts += 1;
                    super::retry::delay(
                        state,
                        policy.retry_base_delay,
                        policy.retry_max_delay,
                        deadline,
                    )
                    .await?;
                    super::retry::renew_transport_after_tls(state, prepared, tracker, &error).await?;
                    prepared.execution = prepared.execution.renew_open();
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        return Ok(HttpResponseStream {
            status,
            headers,
            bytes: upstream.bytes,
            is_sse: upstream.is_sse,
        });
    }
}

struct StreamingResponse {
    state: Arc<AppState>,
    prepared: Prepared,
    upstream: super::upstream::ByteStream,
    decoder: SseDecoder,
    observation: ResponseObservation,
    tracker: RequestTracker,
    finished: bool,
    fence: super::bootstrap::Fence,
    recovery_generation: u64,
    sequence: u64,
    pending_error: Option<ServiceError>,
    policy: ExecutionPolicy,
    deadline: Deadline,
    idle_deadline: tokio::time::Instant,
    keepalive_deadline: tokio::time::Instant,
}

impl StreamingResponse {
    async fn observe_frames(&mut self, frames: Vec<SseFrame>) -> Result<Bytes, ServiceError> {
        let mut bytes = Vec::new();
        for frame in frames {
            if let Some(opened)=pre_business::recover(&self.state,&mut self.prepared,&mut self.tracker,
                &mut self.fence,&frame,self.observation.retry_after,&self.policy,self.deadline,true).await? {
                self.recovery_generation=self.recovery_generation.saturating_add(1);
                self.upstream=opened.bytes;
                self.decoder=SseDecoder::new(self.state.options.max_event_bytes);
                self.observation=ResponseObservation { retry_after:retry_after_headers(&opened.headers),..Default::default() };
                self.idle_deadline=tokio::time::Instant::now()+self.policy.idle_timeout;
                return Ok(bytes.into());
            }
            if let Some(event) = frame.value {
                if self.prepared.route.chatgpt_plan {
                    super::chatgpt_plan_wire::validate_event(frame.identity_event.as_ref().unwrap_or(&event))?;
                }
                observe_late_official_rejection(
                    &self.state, &self.prepared, frame.identity_event.as_ref().unwrap_or(&event),
                    self.observation.retry_after, self.deadline,
                ).await?;
                let event_sequence = event
                    .get("sequence_number")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.sequence);
                self.sequence = event
                    .get("sequence_number")
                    .and_then(Value::as_u64)
                    .map_or(self.sequence.saturating_add(1), |sequence| {
                        self.sequence.max(sequence.saturating_add(1))
                    });
                // Public repair cannot certify an empty or contradictory upstream terminal.
                // Keep server-issued IDs and ownership, but prove rebuild from original output.
                if frame.identity_event.is_some() {
                    self.prepared.binding.abort_rebuild();
                }
                mapping::observe_event(
                    &self.state,
                    &self.prepared,
                    frame.identity_event.as_ref().unwrap_or(&event),
                )
                .await?;
                self.tracker.quota_event(&event);
                self.observation.observe(&event);
                self.tracker
                    .response_tier(self.observation.service_tier.as_deref());
                self.tracker.capture_usage(&self.observation.usage);
                if matches!(
                    super::events::response_terminal_kind(&event),
                    Some("error" | "response.failed")
                ) {
                    let error = super::output_error::event_failure(
                        &event,
                        self.observation.http_status.unwrap_or(502),
                    );
                    let codex = super::output_error::is_codex_client(&self.prepared.source_headers);
                    bytes.extend_from_slice(&super::output_error::responses_sse(
                        &error,
                        codex,
                        event_sequence,
                    ));
                    break;
                }
            }
            bytes.extend_from_slice(&frame.bytes);
            if self.observation.terminal {
                break;
            }
        }
        Ok(bytes.into())
    }

    async fn complete(&mut self) {
        if let Some(response) = &self.observation.response {
            self.tracker.payload(
                "upstream_response",
                "http",
                &HeaderMap::new(),
                &response.to_string(),
            );
        }
        self.tracker.set_retry_after(self.observation.retry_after);
        self.tracker.capture_response_error(
            self.observation.error_message.as_deref(),
            self.observation.credential_scope,
        );
        self.tracker
            .capture_request_fault(self.observation.request_scoped);
        self.tracker
            .complete(
                self.observation.http_status.unwrap_or(200),
                self.observation.succeeded,
                self.observation.usage.take(),
                self.observation.error.take(),
            )
            .await;
        self.finished = true;
    }
}

async fn read_bounded(
    state: &AppState,
    mut stream: super::upstream::ByteStream,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: Deadline,
) -> Result<Vec<u8>, ServiceError> {
    let mut bytes = Vec::new();
    let mut stop = state.stop.clone();
    loop {
        let next = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            _ = deadline::sleep_until(deadline) => return Err(super::retry::total_timeout()),
            result = tokio::time::timeout(policy.idle_timeout, stream.next()) =>
                result.map_err(|_| ServiceError::new(504, "upstream_idle_timeout", "Upstream response stopped responding"))?,
        };
        match next {
            Some(Ok(chunk)) => {
                if !chunk.is_empty() {
                    tracker.first_response();
                }
                if bytes.len().saturating_add(chunk.len()) > state.options.max_response_bytes {
                    return Err(ServiceError::new(
                        502,
                        "upstream_response_too_large",
                        "Upstream response exceeds the configured size limit",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            Some(Err(error)) => {
                return Err(transport_failure(
                    502,
                    "upstream_stream_error",
                    "Unable to read the upstream response",
                    &error,
                ))
            }
            None => return Ok(bytes),
        }
    }
}

async fn collect_sse(
    state: &Arc<AppState>,
    prepared: &mut Prepared,
    mut stream: super::upstream::ByteStream,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: Deadline,
    initial_retry_after: Option<std::time::Duration>,
) -> Result<ResponseObservation, ServiceError> {
    let mut decoder = SseDecoder::new(state.options.max_event_bytes);
    let mut observation = ResponseObservation {
        retry_after: initial_retry_after,
        ..Default::default()
    };
    let mut fence=super::bootstrap::Fence::new(!prepared.route.chatgpt_plan);
    let mut total = 0usize;
    let mut stop = state.stop.clone();
    'read: loop {
        let next = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            _ = deadline::sleep_until(deadline) => return Err(super::retry::total_timeout()),
            result = tokio::time::timeout(policy.idle_timeout, stream.next()) =>
                result.map_err(|_| ServiceError::new(504, "upstream_idle_timeout", "Upstream response stopped responding"))?,
        };
        match next {
            Some(Ok(chunk)) => {
                if !chunk.is_empty() {
                    tracker.first_response();
                }
                total = total.saturating_add(chunk.len());
                if total > state.options.max_response_bytes {
                    return Err(ServiceError::new(
                        502,
                        "upstream_response_too_large",
                        "Upstream response exceeds the configured size limit",
                    ));
                }
                for frame in decoder.push_frames(&chunk)? {
                    if let Some(opened)=pre_business::recover(state,prepared,tracker,&mut fence,&frame,
                        observation.retry_after,policy,deadline,false).await? {
                        stream=opened.bytes;
                        decoder=SseDecoder::new(state.options.max_event_bytes);
                        observation=ResponseObservation { retry_after:retry_after_headers(&opened.headers),..Default::default() };
                        total=0;
                        continue 'read;
                    }
                    let Some(event) = frame.value else { continue };
                    if prepared.route.chatgpt_plan {
                        super::chatgpt_plan_wire::validate_event(frame.identity_event.as_ref().unwrap_or(&event))?;
                    }
                    observe_late_official_rejection(
                        state, prepared, frame.identity_event.as_ref().unwrap_or(&event),
                        observation.retry_after, deadline,
                    ).await?;
                    if frame.identity_event.is_some() {
                        prepared.binding.abort_rebuild();
                    }
                    mapping::observe_event(
                        state,
                        prepared,
                        frame.identity_event.as_ref().unwrap_or(&event),
                    )
                    .await?;
                    tracker.quota_event(&event);
                    observation.observe(&event);
                    if observation.terminal && observation.response.is_none() {
                        observation.response =
                            Some(event.get("response").unwrap_or(&event).clone());
                    }
                }
                if observation.terminal {
                    return Ok(observation);
                }
            }
            Some(Err(error)) => {
                return Err(transport_failure(
                    502,
                    "upstream_stream_error",
                    "Unable to read the upstream response",
                    &error,
                ))
            }
            None => {
                for frame in decoder.finish_frames()? {
                    if let Some(opened)=pre_business::recover(state,prepared,tracker,&mut fence,&frame,
                        observation.retry_after,policy,deadline,false).await? {
                        stream=opened.bytes;
                        decoder=SseDecoder::new(state.options.max_event_bytes);
                        observation=ResponseObservation { retry_after:retry_after_headers(&opened.headers),..Default::default() };
                        total=0;
                        continue 'read;
                    }
                    let Some(event) = frame.value else { continue };
                    if prepared.route.chatgpt_plan {
                        super::chatgpt_plan_wire::validate_event(frame.identity_event.as_ref().unwrap_or(&event))?;
                    }
                    observe_late_official_rejection(
                        state, prepared, frame.identity_event.as_ref().unwrap_or(&event),
                        observation.retry_after, deadline,
                    ).await?;
                    if frame.identity_event.is_some() {
                        prepared.binding.abort_rebuild();
                    }
                    mapping::observe_event(
                        state,
                        prepared,
                        frame.identity_event.as_ref().unwrap_or(&event),
                    )
                    .await?;
                    tracker.quota_event(&event);
                    observation.observe(&event);
                    if observation.terminal && observation.response.is_none() {
                        observation.response =
                            Some(event.get("response").unwrap_or(&event).clone());
                    }
                }
                if observation.terminal {
                    return Ok(observation);
                }
                return Err(ServiceError::new(
                    502,
                    "incomplete_upstream_stream",
                    "Upstream stream ended without a final response",
                ));
            }
        }
    }
}

// A late terminal rejection is real route-health evidence even after business
// output makes replay unsafe. This observer never selects or resends a route.
async fn observe_late_official_rejection(
    state: &AppState,
    prepared: &Prepared,
    event: &Value,
    previous_retry_after: Option<std::time::Duration>,
    deadline: Deadline,
) -> Result<(), ServiceError> {
    if !prepared.route.chatgpt_plan || !matches!(
        super::events::response_terminal_kind(event), Some("error" | "response.failed")
    ) {
        return Ok(());
    }
    let Some((code, status)) = super::official_fallback::explicit_rejection(event) else {
        return Ok(());
    };
    let failure = super::contract::OfficialFallbackFailure {
        trigger_stage: "upstream", status, code,
        message: super::diagnostics::upstream_error_metadata_status(status, event).0,
        retry_after: previous_retry_after.max(super::events::retry_after_event(event)),
    };
    let mut stop = state.stop.clone();
    let observation_deadline = deadline.min(tokio::time::Instant::now() + state.options.request_timeout);
    tokio::select! {
        _ = stop.wait() => Err(stopping_error()),
        result = deadline::timeout_at(observation_deadline,
            state.host.observe_official_failure(&prepared.route, &prepared.route.model, &failure)) => {
            if result.is_err() {
                crate::modules::logger::log_warn(
                    "[NativeAPI] official_health_observation_timeout; preserving upstream rejection",
                );
            }
            Ok(())
        }
    }
}

pub(super) async fn prefetch(
    state: &AppState,
    mut bytes: super::upstream::ByteStream,
    policy: &ExecutionPolicy,
    deadline: Deadline,
) -> Result<super::upstream::ByteStream, ServiceError> {
    let mut stop = state.stop.clone();
    let idle = tokio::time::Instant::now() + policy.idle_timeout;
    loop {
        let next = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            _ = deadline::sleep_until(deadline) => return Err(super::retry::total_timeout()),
            _ = deadline::sleep_until(idle) => return Err(ServiceError::new(504, "upstream_idle_timeout", "Upstream stream stopped responding")),
            next = bytes.next() => next,
        };
        match next {
            Some(Ok(chunk)) if chunk.is_empty() => continue,
            Some(Ok(chunk)) => {
                return Ok(stream::once(async move { Ok(chunk) }).chain(bytes).boxed())
            }
            Some(Err(error)) => {
                return Err(transport_failure(
                    502,
                    "upstream_stream_error",
                    "Upstream stream disconnected before its first event",
                    &error,
                ))
            }
            None => {
                return Err(ServiceError::new(
                    502,
                    "incomplete_upstream_stream",
                    "Upstream stream ended before its first event",
                ))
            }
        }
    }
}

struct ImmediateResponse {
    operation: Option<futures_util::future::BoxFuture<'static, Result<Response, ServiceError>>>,
    stream: Option<super::upstream::ByteStream>,
    initial: bool,
    keepalive: std::time::Duration,
    next_keepalive: tokio::time::Instant,
    total_deadline: Deadline,
    codex_client: bool,
}

fn immediate_response(
    state: Arc<AppState>,
    prepared: Prepared,
    compact: bool,
    tracker: RequestTracker,
    policy: ExecutionPolicy,
) -> Response {
    let keepalive = policy.keepalive;
    let deadline = prepared.execution.total_deadline;
    let codex_client = super::output_error::is_codex_client(&prepared.source_headers);
    immediate_stream_for_client(
        Box::pin(forward_with_policy(
            state, prepared, compact, true, tracker, policy,
        )),
        keepalive,
        deadline,
        codex_client,
    )
}

pub(super) fn immediate_stream(
    operation: futures_util::future::BoxFuture<'static, Result<Response, ServiceError>>,
    keepalive: std::time::Duration,
    total_deadline: Deadline,
) -> Response {
    immediate_stream_for_client(operation, keepalive, total_deadline, false)
}

fn immediate_stream_for_client(
    operation: futures_util::future::BoxFuture<'static, Result<Response, ServiceError>>,
    keepalive: std::time::Duration,
    total_deadline: Deadline,
    codex_client: bool,
) -> Response {
    let current = ImmediateResponse {
        operation: Some(operation),
        stream: None,
        initial: true,
        keepalive,
        next_keepalive: tokio::time::Instant::now() + keepalive,
        total_deadline,
        codex_client,
    };
    let bytes = stream::unfold(current, |mut current| async move {
        if current.initial {
            current.initial = false;
            return Some((
                Ok::<_, std::io::Error>(Bytes::from_static(b": keep-alive\n\n")),
                current,
            ));
        }
        if let Some(operation) = current.operation.as_mut() {
            let response = tokio::select! {
                response = operation => response,
                _ = deadline::sleep_until(current.next_keepalive), if !current.keepalive.is_zero() => {
                    current.next_keepalive = tokio::time::Instant::now() + current.keepalive;
                    return Some((Ok(Bytes::from_static(b": keep-alive\n\n")), current));
                }
            };
            current.operation = None;
            match response {
                Ok(response) if response.status().is_success() => {
                    current.stream = Some(
                        response
                            .into_body()
                            .into_data_stream()
                            .map(|item| item.map_err(std::io::Error::other))
                            .boxed(),
                    );
                }
                Ok(response) => {
                    let bytes = immediate_error_response(
                        response,
                        current.total_deadline,
                        current.codex_client,
                    )
                    .await;
                    return Some((Ok(bytes), current));
                }
                Err(error) => {
                    return Some((
                        Ok(super::output_error::responses_sse(
                            &error,
                            current.codex_client,
                            0,
                        )),
                        current,
                    ))
                }
            }
        }
        let next = current.stream.as_mut()?.next().await?;
        Some((next, current))
    });
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    downstream_response(
        axum::http::StatusCode::OK,
        &headers,
        Body::from_stream(bytes),
    )
}

async fn immediate_error_response(
    response: Response,
    total_deadline: Deadline,
    codex_client: bool,
) -> Bytes {
    let status = response.status().as_u16();
    let retry_header = response.headers().get(header::RETRY_AFTER).cloned();
    let header_delay = retry_after_headers(response.headers());
    let deadline =
        total_deadline.cap(tokio::time::Instant::now() + std::time::Duration::from_secs(5));
    let body =
        match deadline::timeout_at(deadline, to_bytes(response.into_body(), 64 * 1024)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => Bytes::from_static(b"Unable to read the bounded upstream error response"),
            Err(_) => Bytes::from_static(b"Upstream error response read timed out"),
        };
    let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let source = value
        .pointer("/response/error")
        .or_else(|| value.get("error"))
        .or_else(|| value.get("detail"))
        .unwrap_or(&value);
    let message = source
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| source.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| String::from_utf8_lossy(&body).into_owned());
    let message = super::diagnostics::sanitize_error_text(&message);
    let body_delay = super::events::retry_after_event(&value);
    let delay = match (header_delay, body_delay) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    let error = ServiceError::new(status, "upstream_http_error", message)
        .with_error_details(source.clone())
        .with_retry_after(delay);
    let sequence = value
        .get("sequence_number")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut event = super::output_error::responses_event(&error, codex_client, sequence);
    if let Some(value) = retry_header.and_then(|value| value.to_str().ok().map(str::to_owned)) {
        event["headers"] = serde_json::json!({"retry-after":value});
    }
    super::output_error::event_sse(&event)
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod transport_tests;

#[cfg(test)]
#[path = "response_format_tests.rs"]
mod response_format_tests;

#[cfg(test)]
#[path = "preparation_timeout_tests.rs"]
mod preparation_timeout_tests;

#[cfg(test)]
#[path = "official_late_rejection_tests.rs"]
mod official_late_rejection_tests;
