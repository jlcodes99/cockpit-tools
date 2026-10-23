use crate::modules::codex_native_api::deadline::{self, Deadline};
use std::{collections::VecDeque, sync::Arc};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::HeaderMap,
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::Message as UpstreamMessage;

use super::{
    contract::{Caller, ExecutionPolicy, ServiceError},
    events::ResponseObservation,
    lifecycle::{stopping_error, RequestTracker},
    mapping::{self, Prepared},
    server::AppState,
};

#[path = "websocket_history.rs"]
mod history;
#[path = "websocket_protocol.rs"]
mod protocol;
#[path = "websocket_transport.rs"]
mod transport_bridge;

pub(super) async fn upgrade(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ServiceError> {
    let mut tracker = state.begin()?;
    let capture = tokio::time::timeout(
        state.options.request_timeout,
        state.host.payload_logging_enabled(),
    )
    .await
    .unwrap_or(false);
    tracker.context("GET", "/v1/responses", &headers, capture);
    let caller = match state.authorize(&headers).await {
        Ok(caller) => caller,
        Err(error) => {
            tracker.fail(&error).await;
            return Err(error);
        }
    };
    tracker.authorized(&caller, "");
    let policy = match super::retry::execution_policy(&state).await {
        Ok(policy) => policy,
        Err(error) => {
            tracker.fail(&error).await;
            return Err(error);
        }
    };
    let mut stop = state.stop.clone();
    let enabled = tokio::select! {
        _ = stop.wait() => Err(stopping_error()),
        result = tokio::time::timeout(state.options.request_timeout, state.host.websockets_enabled()) => result
            .map_err(|_| ServiceError::new(504, "websocket_policy_timeout", "WebSocket policy lookup timed out")).and_then(|v|v),
    };
    let enabled = match enabled {
        Ok(value) => value,
        Err(error) => {
            tracker.fail(&error).await;
            return Err(error);
        }
    };
    if !enabled {
        let error = ServiceError::new(
            426,
            "websockets_disabled",
            "Use HTTP Responses because WebSockets are disabled",
        );
        tracker.transport_fallback(&error).await;
        return Err(error);
    }
    // The handshake has no model. Select only after the first create message,
    // otherwise a heterogeneous pool can become pinned to an incompatible account.
    tracker.idle();
    let max = state.options.max_body_bytes;
    Ok(ws
        .max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| async move {
            run(state, caller, headers, socket, tracker, policy).await;
        }))
}

async fn run(
    state: Arc<AppState>,
    caller: Caller,
    mut headers: HeaderMap,
    mut socket: WebSocket,
    mut tracker: RequestTracker,
    mut policy: ExecutionPolicy,
) {
    let client_headers = headers.clone();
    let first = match first_create(&state, &mut socket).await {
        Ok(body) => body,
        Err(error) => {
            // Shutdown may race a client's Close frame. Until a valid create
            // starts a turn, either event closes only the idle transport.
            if matches!(error.code, "websocket_idle_closed" | "service_stopping") {
                tracker.complete_local().await;
            } else {
                tracker.fail(&error).await;
                send_error(&state, &mut socket, &error).await;
            }
            return;
        }
    };
    if let Err(error) = tracker.start_turn() {
        tracker.fail(&error).await;
        send_error(&state, &mut socket, &error).await;
        return;
    }
    if let Err(error) = refresh_rebuild_policy(&state, &tracker, &mut policy).await {
        tracker.fail(&error).await;
        send_error(&state, &mut socket, &error).await;
        return;
    }
    let mut history = history::History::default();
    let mut requested_model = first
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let execution = super::execution::Execution::new(&tracker, &policy);
    let mut prepared = match execution
        .run(super::execution::prepare(
            &state,
            mapping::prepare_websocket(&state, &caller, &headers, first),
        ))
        .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            tracker.fail(&error).await;
            send_error(&state, &mut socket, &error).await;
            return;
        }
    };
    tracker.routed(&caller, &prepared.route, &requested_model);
    tracker.routing_identity(&prepared.request);
    tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
    tracker.prepared_metadata(&prepared);
    // Generic WS clients may omit all Codex metadata. Their connection still
    // represents one logical thread, including retries and shorthand creates.
    for (name, value) in [
        ("session-id", prepared.request.session_id.as_deref()),
        ("thread-id", prepared.request.thread_id.as_deref()),
    ] {
        if !headers.contains_key(name) {
            if let Some(value) =
                value.and_then(|value| value.parse::<axum::http::HeaderValue>().ok())
            {
                headers.insert(name, value.clone());
                prepared.source_headers.insert(name, value);
            }
        }
    }
    let mut turn_deadline = execution.total_deadline;
    let mut pending_messages = VecDeque::<String>::new();
    let opened = match while_connected(
        &state,
        &mut socket,
        &mut pending_messages,
        turn_deadline,
        transport_bridge::open(
            &state,
            &caller,
            &mut prepared,
            &mut tracker,
            &policy,
            turn_deadline,
            &mut history,
        ),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            tracker.fail(&error).await;
            send_error(&state, &mut socket, &error).await;
            return;
        }
    };
    let (mut client_sink, mut client_stream) = socket.split();
    let mut upstream_sink = opened.sink;
    let mut upstream_stream = opened.stream;
    let mut upstream_http = opened.http;
    let mut upstream_local = opened.local;
    let mut connection_identity = opened.identity;
    let mut opening_retries = opened.opening_retries;
    let mut active = true;
    let mut response_id: Option<String> = None;
    let mut completed_ids: VecDeque<String> = VecDeque::new();
    let mut last_completed_id: Option<String> = None;
    let mut last_local_checkpoint_id: Option<String> = None;
    let mut interrupt_pending = false;
    let mut observation = ResponseObservation::default();
    let mut output_items = super::output_items::OutputCollector::new();
    // Publishing an idle handshake makes response.interrupt possible, but does
    // not prove execution. Retain the bounded pre-business fence after both
    // fresh opens and reused response.create sends, even after MAX_HOLD.
    let mut probe_reused = pre_business_enabled(&prepared, upstream_local);
    let mut reused_handshake = Vec::<(Value, String)>::new();
    let mut reused_bytes = 0usize;
    let mut reused_events = 0usize;
    let mut reused_hold_deadline = tokio::time::Instant::now();
    let mut client_deadline = tokio::time::Instant::now() + policy.websocket_idle_timeout;
    let mut upstream_deadline = client_deadline;
    let mut heartbeat_deadline = tokio::time::Instant::now() + policy.websocket_heartbeat;
    let result: Result<(), ServiceError> = async {
        loop {
            let mut stop = state.stop.clone();
            tokio::select! {
                _ = stop.wait() => return Err(stopping_error()),
                _ = deadline::sleep_until(client_deadline), if !active => return Err(ServiceError::new(408, "websocket_idle_timeout", "Client WebSocket is idle")),
                _ = deadline::sleep_until(upstream_deadline), if active => return Err(ServiceError::new(504, "upstream_idle_timeout", "Upstream WebSocket stopped responding")),
                _ = deadline::sleep_until(turn_deadline), if active => return Err(super::retry::total_timeout()),
                _ = deadline::sleep_until(reused_hold_deadline), if probe_reused && !reused_handshake.is_empty() => {
                    // Make the active ID available for response.interrupt without
                    // treating elapsed time as evidence of model/tool execution.
                    let mut events = Vec::new();
                    for (mut event, text) in std::mem::take(&mut reused_handshake) {
                        if !upstream_http { prepared.multi_agent_context.restore(&mut event); }
                        let text = event.to_string();
                        if let Some(context) = prepared.grok_context.as_mut().filter(|_| !upstream_http) {
                            events.extend(super::grok_wire::restore_event(context, event)?.into_iter().map(|event| { let text = event.to_string(); (event, text) }));
                        } else { events.push((event, text)); }
                    }
                    for (event, text) in events {
                        mapping::observe_event(&state, &prepared, &event).await?;
                        history.observe(&prepared, &event);
                        if event["type"] == "response.created" {
                            response_id = event.pointer("/response/id").and_then(Value::as_str).map(str::to_string);
                        }
                        tracker.first_response();
                        tracker.payload("upstream_response", "websocket", &HeaderMap::new(), &text);
                        timed_client_until(&state, turn_deadline.instant(), client_sink.send(Message::Text(text.into()))).await?;
                    }
                },
                _ = deadline::sleep_until(heartbeat_deadline), if !policy.websocket_heartbeat.is_zero() => {
                    heartbeat_deadline = tokio::time::Instant::now() + policy.websocket_heartbeat;
                    if let Some(sink) = &mut upstream_sink { timed_until(&state, active.then_some(turn_deadline).and_then(Deadline::instant), sink.send(UpstreamMessage::Ping(Vec::new().into()))).await?; }
                    timed_client_until(&state, active.then_some(turn_deadline).and_then(Deadline::instant), client_sink.send(Message::Ping(Vec::new().into()))).await?;
                },
                next = async {
                    if !active {
                        if let Some(text) = pending_messages.pop_front() {
                            return Some(Ok(Message::Text(text.into())));
                        }
                    }
                    client_stream.next().await
                } => {
                    client_deadline = tokio::time::Instant::now() + policy.websocket_idle_timeout;
                    match next {
                        None | Some(Ok(Message::Close(_))) | Some(Err(_)) => {
                            return if active { Err(ServiceError::new(499, "client_disconnected", "Client WebSocket disconnected")) } else { Ok(()) };
                        }
                        Some(Ok(Message::Ping(value))) => {
                            timed_client_until(&state, active.then_some(turn_deadline).and_then(Deadline::instant), client_sink.send(Message::Pong(value))).await?;
                        }
                        Some(Ok(Message::Pong(_))) => {}
                        Some(Ok(Message::Binary(_))) => {
                            let error = ServiceError::new(400, "invalid_websocket_message", "Responses WebSocket requires JSON text messages");
                            timed_client_until(&state, active.then_some(turn_deadline).and_then(Deadline::instant), client_sink.send(Message::Text(error_event(&error).to_string().into()))).await?;
                        },
                        Some(Ok(Message::Text(text))) => {
                            let body: Value = match super::request_body::websocket_json(&state, text.clone()).await {
                                Ok(body) => body,
                                Err(error) => {
                                    timed_client_until(&state, active.then_some(turn_deadline).and_then(Deadline::instant), client_sink.send(Message::Text(error_event(&error).to_string().into()))).await?;
                                    continue;
                                }
                            };
                            match body.get("type").and_then(Value::as_str) {
                                Some("response.create" | "response.append") => {
                                    if active {
                                        if let Some(error) = enqueue_request(&mut pending_messages, text.as_str()) {
                                            timed_client_until(&state, turn_deadline.instant(), client_sink.send(Message::Text(error_event(&error).to_string().into()))).await?;
                                        }
                                        continue;
                                    }
                                    let body = match protocol::normalize_create(body, Some(prepared.replay_body()), last_completed_id.as_deref().or(last_local_checkpoint_id.as_deref())).and_then(|body| history.resolve_prewarm(body)) {
                                        Ok(body) => body,
                                        Err(error) if error.status == 400 => {
                                            timed_client_until(&state, None, client_sink.send(Message::Text(error_event(&error).to_string().into()))).await?;
                                            continue;
                                        }
                                        Err(error) => return Err(error),
                                    };
                                    let (body, interrupted_owner) = history.resolve_local(&prepared, body)?;
                                    requested_model = body["model"].as_str().unwrap_or_default().to_owned();
                                    tracker.start_turn()?;
                                    refresh_rebuild_policy(&state, &tracker, &mut policy).await?;
                                    let execution = super::execution::Execution::new(&tracker, &policy);
                                    turn_deadline = execution.total_deadline;
                                    let capture = deadline::timeout_at((tokio::time::Instant::now()+state.options.request_timeout).min(execution.prepare_deadline()), state.host.payload_logging_enabled()).await.unwrap_or(false);
                                    tracker.context("GET", "/v1/responses", &client_headers, capture);
                                    tracker.authorized(&caller, &requested_model);
                                    let next = execution.run(super::execution::prepare(&state, prepare_next_turn(&state, &caller, &headers, body, interrupted_owner.as_ref().map(|owner| owner.account.as_str()), &prepared, &history))).await?;
                                    if interrupted_owner.as_ref().is_some_and(|owner| !owner.matches(&next)) {
                                        return Err(ServiceError::new(409, "continuation_owner_mismatch", "Partial HTTP history cannot move to a different account, owner or thread"));
                                    }
                                    let previous_multi_agent_context = prepared.multi_agent_context.clone();
                                    let reconnect = upstream_http || websocket_route_changed(&prepared, &next) || !transport_bridge::passthrough(&state, &caller, &next).await?;
                                    prepared = next;
                                    tracker.routed(&caller, &prepared.route, &requested_model);
                                    tracker.routing_identity(&prepared.request);
                                    tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                                    tracker.prepared_metadata(&prepared);
                                    active = true; response_id = None; interrupt_pending = false; observation = ResponseObservation::default(); last_local_checkpoint_id = None;
                                    output_items = super::output_items::OutputCollector::new();
                                    probe_reused = pre_business_enabled(&prepared, upstream_local);
                                    opening_retries = OpeningRetries::default();
                                    reused_handshake.clear();
                                    reused_bytes = 0;
                                    reused_events = 0;
                                    reused_hold_deadline = tokio::time::Instant::now() + super::bootstrap::MAX_HOLD;
                                    upstream_deadline = tokio::time::Instant::now() + policy.websocket_initial_timeout;
                                    if reconnect {
                                        if let Some(sink) = &mut upstream_sink { let _ = tokio::time::timeout(std::time::Duration::from_secs(1), sink.close()).await; }
                                        let opened = while_split_connected(
                                            &state, &mut client_sink, &mut client_stream, &mut pending_messages,
                                            turn_deadline,
                                            transport_bridge::open(&state, &caller, &mut prepared, &mut tracker, &policy, turn_deadline, &mut history),
                                        ).await?;
                                        upstream_sink = opened.sink;
                                        upstream_stream = opened.stream;
                                        upstream_http = opened.http;
                                        upstream_local = opened.local;
                                        connection_identity = opened.identity;
                                        opening_retries = opened.opening_retries;
                                        probe_reused = pre_business_enabled(&prepared, upstream_local);
                                        reused_hold_deadline = tokio::time::Instant::now();
                                    } else {
                                        let mut wire_headers = prepared.headers.clone();
                                        let wire_body = prepare_ws_body(&mut prepared, &mut wire_headers)?;
                                        if !super::multi_agent_compat::has_namespace_conflict(&prepared.body) {
                                            prepared.multi_agent_context.optimized_namespace |= previous_multi_agent_context.optimized_namespace;
                                        }
                                        if wire_body.get("previous_response_id").and_then(Value::as_str).is_some_and(|id| !id.trim().is_empty()) {
                                            prepared.multi_agent_context.inherit_plaintext(&previous_multi_agent_context);
                                        }
                                        let sink = upstream_sink.as_mut().ok_or_else(|| ServiceError::new(500, "websocket_transport_unavailable", "Upstream transport is unavailable"))?;
                                        tracker.check_quota_before_send()?;
                                        history.begin(&prepared);
                                        timed_until(&state, turn_deadline.instant(), async {
                                            if tokio::time::Instant::now() >= turn_deadline { return Err(tokio_tungstenite::tungstenite::Error::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "WebSocket write timed out"))); }
                                            tracker.begin_attempt("websocket");
                                            tracker.capability_fallback(prepared.capability_fallback.clone());
                                            if let Some(identity) = &connection_identity { tracker.upstream_identity(identity.clone()); }
                                            tracker.payload("upstream_request", "websocket", &wire_headers, &wire_body.to_string());
                                            sink.send(UpstreamMessage::Text(wire_body.to_string().into())).await
                                        }).await?;
                                    }
                                }
                                Some("response.interrupt") => {
                                    let target = body.get("response_id").and_then(Value::as_str);
                                    if target.is_some_and(|id| completed_ids.iter().any(|completed| completed == id)) && (!active || target != response_id.as_deref()) { continue; }
                                    if !active || target.is_none() || target != response_id.as_deref() {
                                        let error = ServiceError::new(409, "unknown_interrupt_response", "Interrupt must refer to this connection's active response");
                                        timed_client_until(&state, active.then_some(turn_deadline).and_then(Deadline::instant), client_sink.send(Message::Text(error_event(&error).to_string().into()))).await?;
                                        continue;
                                    }
                                    if upstream_http {
                                        let id = target.unwrap().to_owned();
                                        let incomplete = history.interrupted(&prepared, &id)?;
                                        // Dropping the boxed HTTP body cancels this turn only. Never
                                        // promote a local interruption to a durable completed response.
                                        upstream_stream = futures_util::stream::empty().boxed();
                                        last_completed_id = None;
                                        last_local_checkpoint_id = Some(id.clone());
                                        if completed_ids.back() != Some(&id) { completed_ids.push_back(id); }
                                        if completed_ids.len() > 256 { completed_ids.pop_front(); }
                                        active = false;
                                        interrupt_pending = false;
                                        response_id = None;
                                        tracker.payload("upstream_response", "websocket", &HeaderMap::new(), &incomplete.to_string());
                                        tracker.complete(499, false, observation.usage.take(), Some("client_interrupted".into())).await;
                                        prepared.route.permit = None;
                                        client_deadline = tokio::time::Instant::now() + policy.websocket_idle_timeout;
                                        timed_client_until(&state, None, client_sink.send(Message::Text(incomplete.to_string().into()))).await?;
                                        continue;
                                    }
                                    if !interrupt_pending {
                                        // An explicit client interrupt is not a retryable rejection.
                                        probe_reused = false;
                                        if let Some(sink) = &mut upstream_sink { timed_until(&state, turn_deadline.instant(), sink.send(UpstreamMessage::Text(text.to_string().into()))).await?; }
                                        interrupt_pending = true;
                                    }
                                }
                                _ => {
                                    let error = ServiceError::new(400, "unsupported_websocket_event", "Expected response.create or response.interrupt");
                                    timed_client_until(&state, active.then_some(turn_deadline).and_then(Deadline::instant), client_sink.send(Message::Text(error_event(&error).to_string().into()))).await?;
                                },
                            }
                        }
                    }
                }
                next = upstream_stream.next(), if active || !upstream_http => {
                    upstream_deadline = tokio::time::Instant::now() + policy.websocket_idle_timeout;
                    match next {
                        Some(Ok(UpstreamMessage::Close(Some(frame)))) if u16::from(frame.code) == 1009 => {
                            return Err(message_too_big_error(&frame.reason));
                        }
                        None | Some(Ok(UpstreamMessage::Close(_))) => {
                            return if active { Err(ServiceError::new(502, "incomplete_upstream_stream", "Upstream closed before completing the response")) } else { Ok(()) };
                        }
                        Some(Err(error)) => return Err(error),
                        Some(Ok(UpstreamMessage::Ping(value))) => { if let Some(sink) = &mut upstream_sink { timed_until(&state, turn_deadline.instant(), sink.send(UpstreamMessage::Pong(value))).await?; } }
                        Some(Ok(UpstreamMessage::Pong(_))) | Some(Ok(UpstreamMessage::Frame(_))) => {}
                        Some(Ok(UpstreamMessage::Binary(_))) => return Err(ServiceError::new(502, "invalid_upstream_event", "Upstream Responses WebSocket returned a binary message")),
                        Some(Ok(UpstreamMessage::Text(text))) => {
                            let event: Value = serde_json::from_str(text.as_str()).map_err(|_| ServiceError::new(502, "invalid_upstream_event", "Upstream returned invalid JSON"))?;
                            if let Some(error) = super::upstream_errors::chat_failure(&event, 0)
                                .filter(|error| websocket_error_close(error).is_some()) {
                                return Err(error);
                            }
                            if probe_reused {
                                reused_bytes = reused_bytes.saturating_add(text.len());
                                reused_events = reused_events.saturating_add(1);
                                if reused_events >= super::bootstrap::MAX_EVENTS || reused_bytes >= super::bootstrap::MAX_BYTES {
                                    probe_reused = false;
                                }
                            }
                            if probe_reused {
                                if prepared.capability_fallback_enabled && super::bootstrap::pre_execution(&event) && super::capabilities::pre_execution_rejection(400, &event) {
                                    tracker.attempt_result(400, false, Some("unsupported_capability"));
                                    if mapping::retry_capability(&state, &mut prepared).await? {
                                        tracker.routed(&caller, &prepared.route, &requested_model);
                                        tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                                        if let Some(sink) = &mut upstream_sink { let _ = tokio::time::timeout(std::time::Duration::from_secs(1), sink.close()).await; }
                                        let opened = while_split_connected(&state, &mut client_sink, &mut client_stream, &mut pending_messages, turn_deadline,
                                            transport_bridge::open(&state, &caller, &mut prepared, &mut tracker, &policy, turn_deadline, &mut history)).await?;
                                        upstream_sink = opened.sink; upstream_stream = opened.stream;
                                        upstream_http = opened.http; upstream_local = opened.local; connection_identity = opened.identity;
                                        opening_retries = opened.opening_retries;
                                        response_id = None;
                                        probe_reused = pre_business_enabled(&prepared, upstream_local); reused_handshake.clear();
                                        reused_events = 0; reused_bytes = 0;
                                        reused_hold_deadline = tokio::time::Instant::now();
                                        continue;
                                    }
                                }
                                if let Some((error, wait)) = super::grok_errors::applies(&prepared.route).then(|| super::grok_errors::rejection(&event)).flatten().or_else(|| super::bootstrap::rejection(&event))
                                    .and_then(|_| first_event_rejection(&UpstreamMessage::Text(text.clone()), super::grok_errors::applies(&prepared.route))) {
                                    tracker.attempt_error(&error);
                                    // HTTP bridge opening already shares the HTTP retry engine;
                                    // its published-metadata continuation must share its actual
                                    // account budget as well, rather than restart socket counters.
                                    let retry = if upstream_http {
                                        super::retry::recover_before_output(&state, &mut prepared, &mut tracker, &event, error.status, wait, &policy, turn_deadline).await?
                                    } else {
                                        retry_opening_failure(&state, &mut prepared, &mut tracker, &policy, turn_deadline,
                                            &mut opening_retries, &error, OpeningFailure::Status, wait).await?
                                    };
                                    if retry {
                                        if let Some(sink) = &mut upstream_sink { let _ = tokio::time::timeout(std::time::Duration::from_secs(1), sink.close()).await; }
                                        let recovery_policy = if upstream_http {
                                            super::retry::remaining_credential_policy(&prepared, &tracker, &policy)
                                        } else { policy.clone() };
                                        let attempts = tracker.upstream_attempt_count();
                                        let reopened = while_split_connected(&state, &mut client_sink, &mut client_stream,
                                            &mut pending_messages, turn_deadline,
                                            transport_bridge::open_with_retries(&state, &caller, &mut prepared, &mut tracker, &recovery_policy, turn_deadline, &mut history, std::mem::take(&mut opening_retries))).await;
                                        let opened = match reopened {
                                            Err(_) if !state.stop.stopped() && tokio::time::Instant::now() < turn_deadline && tracker.upstream_attempt_count() == attempts => return Err(error),
                                            value => value?,
                                        };
                                        upstream_sink = opened.sink;
                                        upstream_stream = opened.stream;
                                        upstream_http = opened.http;
                                        upstream_local = opened.local;
                                        connection_identity = opened.identity;
                                        opening_retries = opened.opening_retries;
                                        response_id = None;
                                        probe_reused = pre_business_enabled(&prepared, upstream_local);
                                        reused_handshake.clear();
                                        reused_events = 0; reused_bytes = 0;
                                        reused_hold_deadline = tokio::time::Instant::now();
                                        continue;
                                    }
                                }
                                if super::bootstrap::handshake(&event) {
                                    reused_handshake.push((event, text.to_string()));
                                    continue;
                                }
                                probe_reused = false;
                            }
                            tracker.first_response();
                            tracker.payload("upstream_response", "websocket", &HeaderMap::new(), text.as_str());
                            let mut events = Vec::new();
                            for (mut event, text) in std::mem::take(&mut reused_handshake).into_iter().chain(std::iter::once((event, text.to_string()))) {
                                if !upstream_http { prepared.multi_agent_context.restore(&mut event); }
                        let text = event.to_string();
                        if let Some(context) = prepared.grok_context.as_mut().filter(|_| !upstream_http) {
                                    events.extend(super::grok_wire::restore_event(context, event)?.into_iter().map(|event| { let text = event.to_string(); (event, text) }));
                                } else { events.push((event, text)); }
                            }
                            for (mut event, mut text) in events {
                                let mut downstream = None;
                                if interrupt_pending && protocol::is_interrupt_rejection(&event) {
                                    interrupt_pending = false;
                                    timed_client_until(&state, turn_deadline.instant(), client_sink.send(Message::Text(text.into()))).await?;
                                    continue;
                                }
                                let original = (!upstream_local && matches!(event["type"].as_str(),
                                    Some("response.completed" | "response.done" | "response.incomplete")))
                                    .then(|| event.clone());
                                let identity_restored = !upstream_local && output_items.can_restore_identity(&event);
                                let output_changed = !upstream_local && output_items.observe(&mut event)?;
                                let presentation_only = output_changed && !identity_restored;
                                if presentation_only {
                                    // Repair the client transcript without treating a missing or
                                    // contradictory terminal as proof for cross-account replay.
                                    history.invalidate_rebuild_proof();
                                    prepared.binding.abort_rebuild();
                                }
                                let usage_changed = super::output_usage::normalize_usage_details(&mut event);
                                if output_changed || usage_changed { text = event.to_string(); }
                                if !upstream_local {
                                    mapping::observe_event(&state, &prepared,
                                        original.as_ref().filter(|_| presentation_only).unwrap_or(&event)).await?;
                                    history.observe(&prepared, &event);
                                }
                                if event.get("type").and_then(Value::as_str) == Some("response.created") {
                                    response_id = event.pointer("/response/id").and_then(Value::as_str).map(str::to_string);
                                }
                                let partial_checkpoint = if upstream_http && !upstream_local {
                                    history.incomplete_anchor(&prepared, &event)
                                } else { None };
                                tracker.quota_event(&event);
                                observation.observe(&event);
                                tracker.capture_response_error(observation.error_message.as_deref(), observation.credential_scope);
                                tracker.capture_request_fault(observation.request_scoped);
                                tracker.response_tier(observation.service_tier.as_deref());
                                tracker.capture_usage(&observation.usage);
                                if active && observation.terminal {
                                    last_completed_id = observation.succeeded.then(|| event.pointer("/response/id").and_then(Value::as_str).or(response_id.as_deref()).map(str::to_owned)).flatten();
                                    if let Some(id) = response_id.as_deref().or_else(|| event.pointer("/response/id").and_then(Value::as_str)) {
                                        if completed_ids.back().is_none_or(|previous| previous != id) { completed_ids.push_back(id.to_owned()); }
                                        if completed_ids.len() > 256 { completed_ids.pop_front(); }
                                    }
                                    interrupt_pending = false;
                                    last_local_checkpoint_id = partial_checkpoint;
                                    tracker.set_retry_after(observation.retry_after);
                                    if upstream_local { tracker.complete_local().await; }
                                    else {
                                        downstream = Some(tracker.defer_downstream());
                                        tracker.complete(observation.http_status.unwrap_or(200), observation.succeeded, observation.usage.take(), observation.error.take()).await;
                                    }
                                    prepared.route.permit = None;
                                    active = false;
                                    if upstream_http { upstream_stream = futures_util::stream::empty().boxed(); }
                                    client_deadline = tokio::time::Instant::now() + policy.websocket_idle_timeout;
                                }
                                match timed_client_until(&state, active.then_some(turn_deadline).and_then(Deadline::instant), client_sink.send(Message::Text(text.into()))).await {
                                    Ok(()) => { if let Some(outcome) = downstream.as_mut() { outcome.success(); } },
                                    Err(error) => { if let Some(outcome) = downstream.as_mut() { outcome.fail(&error); } return Err(error); },
                                }
                            }
                        }
                    }
                }
            }
        }
    }.await;
    if state.stop.stopped() {
        if let Err(error) = result {
            tracker.fail(&error).await;
        }
        return;
    }
    if let Err(error) = result {
        tracker.fail(&error).await;
        let message = websocket_error_close(&error).map_or_else(
            || Message::Text(error_event(&error).to_string().into()),
            |frame| Message::Close(Some(frame)),
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), client_sink.send(message))
            .await;
    }
    if let Some(sink) = &mut upstream_sink {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), sink.close()).await;
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), client_sink.close()).await;
}

async fn while_connected<T>(
    state: &AppState,
    socket: &mut WebSocket,
    pending: &mut VecDeque<String>,
    deadline: Deadline,
    future: impl std::future::Future<Output = Result<T, ServiceError>>,
) -> Result<T, ServiceError> {
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            next = socket.recv() => match next {
                Some(Ok(Message::Ping(value))) => timed_until(state, deadline.instant(), socket.send(Message::Pong(value))).await?,
                Some(Ok(Message::Pong(_))) => {},
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return Err(ServiceError::new(499, "client_disconnected", "Client WebSocket disconnected")),
                Some(Ok(Message::Text(text))) => {
                    if let Some(error) = opening_control(state, text, pending).await {
                        timed_client_until(state, deadline.instant(), socket.send(Message::Text(error_event(&error).to_string().into()))).await?;
                    }
                },
                Some(Ok(Message::Binary(_))) => {
                    let error = ServiceError::new(400, "invalid_websocket_message", "Responses WebSocket requires JSON text messages");
                    timed_client_until(state, deadline.instant(), socket.send(Message::Text(error_event(&error).to_string().into()))).await?;
                },
            }
        }
    }
}

fn pre_business_enabled(prepared: &Prepared, local: bool) -> bool {
    !local && !prepared.route.chatgpt_plan
        && prepared.route.wire_api == super::contract::WireApi::Responses
        && prepared.raw_chat.is_none()
}

fn websocket_route_changed(previous: &Prepared, next: &Prepared) -> bool {
    websocket_transport_changed(&previous.route, &next.route)
        || websocket_connection_headers_changed(&previous.headers, &next.headers)
}

fn websocket_transport_changed(
    previous: &super::contract::Route,
    next: &super::contract::Route,
) -> bool {
    let stable_assertion = previous
        .transport_identity
        .as_deref()
        .is_some_and(|identity| !identity.is_empty())
        && previous.transport_identity == next.transport_identity;
    let endpoint_changed = match (&previous.websocket_endpoint, &next.websocket_endpoint) {
        (None, None) => false,
        (Some(previous), Some(next)) => {
            previous.base_url != next.base_url
                || websocket_host_headers_changed(
                    &previous.headers,
                    &next.headers,
                    stable_assertion,
                )
        }
        _ => true,
    };
    previous.account_id != next.account_id
        || previous.continuity_id() != next.continuity_id()
        || previous.official_registration_id != next.official_registration_id
        || previous.owner_id != next.owner_id
        || previous.transport_identity != next.transport_identity
        || previous.installation_profile != next.installation_profile
        || previous.proxy_url != next.proxy_url
        || previous.base_url != next.base_url
        || previous.wire_api != next.wire_api
        || previous.chatgpt != next.chatgpt
        || endpoint_changed
        || websocket_host_headers_changed(&previous.headers, &next.headers, stable_assertion)
}

fn websocket_host_headers_changed(
    previous: &HeaderMap,
    next: &HeaderMap,
    stable_assertion: bool,
) -> bool {
    if previous == next {
        return false;
    }
    let is_assertion = |headers: &HeaderMap| {
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("AgentAssertion "))
    };
    if stable_assertion && is_assertion(previous) && is_assertion(next) {
        // The host's digest covers runtime/task/private-key generation. A fresh
        // timestamp signature alone does not require closing the same session.
        let mut previous = previous.clone();
        let mut next = next.clone();
        previous.remove("authorization");
        next.remove("authorization");
        return previous != next;
    }
    // Hosts without a transport digest still cannot keep a socket authenticated
    // with the previous Bearer token after routing returns new credentials.
    true
}

fn websocket_connection_headers_changed(previous: &HeaderMap, next: &HeaderMap) -> bool {
    [
        "session-id",
        "thread-id",
        "x-codex-installation-id",
        "installation-id",
        "user-agent",
        "originator",
        "version",
        "openai-beta",
        "x-codex-beta-features",
        "chatgpt-account-id",
        "x-openai-fedramp",
    ]
    .into_iter()
    .any(|name| previous.get_all(name).iter().ne(next.get_all(name).iter()))
}
async fn while_split_connected<T>(
    state: &AppState,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    pending: &mut VecDeque<String>,
    deadline: Deadline,
    future: impl std::future::Future<Output = Result<T, ServiceError>>,
) -> Result<T, ServiceError> {
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            next = stream.next() => match next {
                Some(Ok(Message::Ping(value))) => timed_until(state, deadline.instant(), sink.send(Message::Pong(value))).await?,
                Some(Ok(Message::Pong(_))) => {},
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return Err(ServiceError::new(499, "client_disconnected", "Client WebSocket disconnected")),
                Some(Ok(Message::Text(text))) => {
                    if let Some(error) = opening_control(state, text, pending).await {
                        timed_client_until(state, deadline.instant(), sink.send(Message::Text(error_event(&error).to_string().into()))).await?;
                    }
                },
                Some(Ok(Message::Binary(_))) => {
                    let error = ServiceError::new(400, "invalid_websocket_message", "Responses WebSocket requires JSON text messages");
                    timed_client_until(state, deadline.instant(), sink.send(Message::Text(error_event(&error).to_string().into()))).await?;
                },
            }
        }
    }
}

async fn open_response(
    state: &Arc<AppState>,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: Deadline,
    mut retries: OpeningRetries,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<reqwest::Upgraded>,
        Vec<UpstreamMessage>,
        super::diagnostics::HeaderIdentity,
        OpeningRetries,
    ),
    ServiceError,
> {
    if prepared.route.wire_api != super::contract::WireApi::Responses {
        return Err(ServiceError::new(
            426,
            "websockets_unsupported",
            "Use HTTP Responses for this provider account",
        ));
    }
    // Candidate exclusions can come from preparation without an upstream send.
    let mut previous_open_deadline = None;
    loop {
        if state.stop.stopped() {
            return Err(stopping_error());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(super::retry::total_timeout());
        }
        if previous_open_deadline == Some(prepared.execution.open_deadline) {
            prepared.execution = prepared.execution.renew_open();
        }
        previous_open_deadline = Some(prepared.execution.open_deadline);
        if prepared.route.wire_api != super::contract::WireApi::Responses {
            return Err(ServiceError::new(
                426,
                "websockets_unsupported",
                "The replacement account does not support Responses WebSockets",
            ));
        }
        let mut headers = prepared.headers.clone();
        let wire_body = prepare_ws_body(prepared, &mut headers)?;
        let endpoint = prepared.route.websocket_endpoint.as_ref();
        let base_url = endpoint.map_or(prepared.route.base_url.as_str(), |endpoint| {
            endpoint.base_url.as_str()
        });
        let url = format!("{}/responses", base_url.trim_end_matches('/'));
        if let Some(endpoint) = endpoint {
            for name in prepared.route.headers.keys() {
                headers.remove(name);
            }
            headers.extend(endpoint.headers.clone());
        }
        let headers = websocket_headers(&headers, prepared.route.chatgpt);
        let wire_headers = headers.clone();
        let upstream_identity = super::diagnostics::capture_header_identity(&headers);
        tracker.check_quota_before_send()?;
        let connected = super::transport::connect_websocket_tracked(
            &prepared.route,
            &url,
            headers,
            state.options.max_event_bytes,
            policy.websocket_connect_timeout.min(
                deadline.cap(prepared.execution.prepare_deadline())
                    .saturating_duration_since(tokio::time::Instant::now()),
            ),
            state.stop.clone(),
            |headers| {
                tracker.begin_attempt("websocket");
                tracker.capability_fallback(prepared.capability_fallback.clone());
                tracker.upstream_headers(headers);
            },
        )
        .await;
        let (mut upstream, response_headers) = match connected {
            Ok(value) => value,
            Err(error) => {
                if error.upstream_not_sent {
                    tracker.discard_unsent_attempt();
                } else {
                    tracker.attempt_error(&error);
                }
                tracker.set_retry_after(error.retry_after);
                let kind = if error.upstream_not_sent { OpeningFailure::Send } else { match error.code {
                    "websocket_upstream_rejected" | "upstream_model_unavailable" => {
                        OpeningFailure::Status
                    }
                    "websocket_connect_failed" | "websocket_connect_timeout" => {
                        OpeningFailure::Send
                    }
                    _ => OpeningFailure::Fatal,
                }};
                if retry_opening_failure(
                    state,
                    prepared,
                    tracker,
                    policy,
                    deadline,
                    &mut retries,
                    &error,
                    kind,
                    error.retry_after,
                )
                .await?
                {
                    continue;
                }
                return Err(if error.upstream_not_sent {
                    retries.last_upstream_failure.clone().unwrap_or(error)
                } else { error });
            }
        };
        tracker.quota_headers(&response_headers);
        tracker.upstream_response_headers(&response_headers);
        mapping::observe_headers(state, prepared, &response_headers).await?;
        tracker.payload(
            "upstream_request",
            "websocket",
            &wire_headers,
            &wire_body.to_string(),
        );
        let initial = async {
            tracker.check_quota_before_send()?;
            timed_until(state, deadline.instant(), upstream.send(UpstreamMessage::Text(wire_body.to_string().into()))).await?;
            let initial_deadline = deadline.cap(tokio::time::Instant::now() + policy.websocket_initial_timeout);
            let mut buffered = Vec::new();
            let mut size = 0usize;
            let mut hold_deadline = None;
            loop {
                let mut stop = state.stop.clone();
                let next = tokio::select! {
                    _ = stop.wait() => return Err(stopping_error()),
                    _ = deadline::sleep_until(hold_deadline.unwrap_or(initial_deadline)), if hold_deadline.is_some() => return Ok(buffered),
                    result = deadline::timeout_at(initial_deadline, upstream.next()) => result.map_err(|_| {
                        if tokio::time::Instant::now() >= deadline { super::retry::total_timeout() }
                        else { ServiceError::new(504, "websocket_initial_timeout", "Upstream WebSocket did not send its first event") }
                    })?,
                };
                match next {
                    Some(Ok(message @ UpstreamMessage::Text(_))) => {
                        let UpstreamMessage::Text(text) = &message else { unreachable!() };
                        let event: Value = serde_json::from_str(text.as_str()).map_err(|_| ServiceError::new(502, "invalid_upstream_event", "Upstream returned invalid JSON"))?;
                        size = size.saturating_add(text.len());
                        let hold = super::bootstrap::handshake(&event);
                        buffered.push(message);
                        hold_deadline.get_or_insert(tokio::time::Instant::now() + super::bootstrap::MAX_HOLD);
                        if !hold || buffered.len() > super::bootstrap::MAX_EVENTS || size >= super::bootstrap::MAX_BYTES {
                            return Ok(buffered);
                        }
                    },
                    Some(Ok(UpstreamMessage::Ping(value))) => timed_until(state, Some(initial_deadline), upstream.send(UpstreamMessage::Pong(value))).await?,
                    Some(Ok(UpstreamMessage::Pong(_))) | Some(Ok(UpstreamMessage::Frame(_))) => {},
                    Some(Ok(UpstreamMessage::Binary(_))) => return Err(ServiceError::new(502, "invalid_upstream_event", "Upstream Responses WebSocket returned a binary message")),
                    Some(Ok(UpstreamMessage::Close(Some(frame)))) if u16::from(frame.code) == 1009 => return Err(message_too_big_error(&frame.reason)),
                    Some(Err(error)) => return Err(initial_frame_error(&error)),
                    _ => return Err(ServiceError::new(502, "incomplete_upstream_stream", "Upstream closed before its first response event")),
                }
            }
        }.await;
        match initial {
            Ok(initial) => {
                let last = initial.last().expect("opening always buffers a message");
                let within_fence = initial.len() <= super::bootstrap::MAX_EVENTS
                    && initial.iter().map(|message| match message { UpstreamMessage::Text(text) => text.len(), _ => 0 }).sum::<usize>() < super::bootstrap::MAX_BYTES;
                let capability_rejection = within_fence && prepared.capability_fallback_enabled
                    && initial.iter().enumerate().all(|(index, message)| {
                        let UpstreamMessage::Text(text) = message else {
                            return false;
                        };
                        let Ok(event) = serde_json::from_str::<Value>(text.as_str()) else {
                            return false;
                        };
                        if index + 1 == initial.len() {
                            super::bootstrap::pre_execution(&event) && super::capabilities::pre_execution_rejection(400, &event)
                        } else {
                            super::bootstrap::handshake(&event)
                        }
                    });
                if capability_rejection {
                    tracker.attempt_result(400, false, Some("unsupported_capability"));
                    if mapping::retry_capability(state, prepared).await? {
                        tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
                        tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                        // Re-open through the shared HTTP bridge so a replacement
                        // need not support the source's authenticated socket.
                        return Err(ServiceError::new(
                            426,
                            "websocket_upstream_rejected",
                            "The replacement account will use HTTP Responses",
                        ));
                    }
                }
                let retryable = within_fence && (initial.len() == 1
                    || match last {
                        UpstreamMessage::Text(text) => serde_json::from_str::<Value>(text.as_str())
                            .ok()
                            .is_some_and(|event| {
                                super::bootstrap::rejection(&event).is_some()
                                    || (super::grok_errors::applies(&prepared.route)
                                        && super::grok_errors::rejection(&event).is_some())
                            }),
                        _ => false,
                    });
                if let Some((error, wait)) = retryable
                    .then(|| {
                        first_event_rejection(last, super::grok_errors::applies(&prepared.route))
                    })
                    .flatten()
                {
                    tracker.attempt_error(&error);
                    tracker.set_retry_after(wait);
                    if let UpstreamMessage::Text(text) = last {
                        tracker.payload(
                            "upstream_response",
                            "websocket",
                            &HeaderMap::new(),
                            text.as_str(),
                        );
                    }
                    if retry_opening_failure(
                        state,
                        prepared,
                        tracker,
                        policy,
                        deadline,
                        &mut retries,
                        &error,
                        OpeningFailure::Status,
                        wait,
                    )
                    .await?
                    {
                        continue;
                    }
                }
                return Ok((upstream, initial, upstream_identity, retries));
            }
            Err(error) => {
                tracker.attempt_error(&error);
                let kind = match error.code {
                    "invalid_upstream_event" | "message_too_big" => OpeningFailure::Fatal,
                    "websocket_send_failed" | "websocket_send_timeout" => OpeningFailure::Send,
                    _ => OpeningFailure::Bootstrap,
                };
                if !retry_opening_failure(
                    state,
                    prepared,
                    tracker,
                    policy,
                    deadline,
                    &mut retries,
                    &error,
                    kind,
                    None,
                )
                .await?
                {
                    return Err(error);
                }
            }
        }
    }
}

#[derive(Default)]
struct OpeningRetries {
    refreshed: bool,
    credentials: usize,
    send: u8,
    status: u8,
    bootstrap: u8,
    cooldown_round: bool,
    last_upstream_failure: Option<ServiceError>,
}

#[derive(Clone, Copy)]
enum OpeningFailure {
    Send,
    Status,
    Bootstrap,
    Fatal,
}

#[allow(clippy::too_many_arguments)]
async fn retry_opening_failure(
    state: &AppState,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: Deadline,
    retries: &mut OpeningRetries,
    error: &ServiceError,
    kind: OpeningFailure,
    wait: Option<std::time::Duration>,
) -> Result<bool, ServiceError> {
    retries.credentials = retries.credentials.max(tracker.upstream_account_ids().len().saturating_sub(1));
    if !error.upstream_not_sent {
        retries.last_upstream_failure = Some(error.clone().with_retry_after(wait.max(error.retry_after)));
    }
    if state.stop.stopped() {
        return Err(stopping_error());
    }
    if tokio::time::Instant::now() >= deadline {
        return Err(super::retry::total_timeout());
    }
    if matches!(kind, OpeningFailure::Fatal) {
        return Ok(false);
    }
    if error.code == super::upstream_errors::REQUEST_SCOPED {
        return Ok(false);
    }
    let model_unavailable = error.code == "upstream_model_unavailable"
        || super::upstream_errors::model_unavailable_message(
            error.status,
            &error.message,
            &prepared.route.model,
        );
    if matches!(kind, OpeningFailure::Status)
        && error.status == 401
        && !retries.refreshed
        && !model_unavailable
        && super::retry::can_refresh_credentials(prepared)
    {
        tracker.release_route();
        let mut stop = state.stop.clone();
        let refresh = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            result = deadline::timeout_at(deadline, mapping::refresh(state, prepared)) => result.map_err(|_| super::retry::total_timeout())?,
        };
        retries.refreshed = true;
        match refresh {
            Ok(()) => {
                tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
                tracker.routing_identity(&prepared.request);
                tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                tracker.prepared_metadata(&prepared);
                return Ok(true);
            }
            Err(error)
                if state.stop.stopped()
                    || tokio::time::Instant::now() >= deadline
                    || matches!(error.code, "refresh_owner_changed" | "official_fallback_authorization_changed") =>
            {
                return Err(error)
            }
            Err(_) => {}
        }
    }
    let backoff = match kind {
        OpeningFailure::Send if !error.upstream_not_sent && retries.send < policy.send_retries => {
            retries.send += 1;
            Some((
                policy
                    .retry_base_delay
                    .saturating_mul(1u32 << u32::from(retries.send.saturating_sub(1).min(8))),
                policy.send_retry_max_delay,
            ))
        }
        OpeningFailure::Status
            if matches!(error.status, 408 | 500 | 502 | 503 | 504)
                && !model_unavailable
                && retries.status < policy.status_retries =>
        {
            retries.status += 1;
            Some((
                wait.unwrap_or_else(|| {
                    policy
                        .status_retry_base_delay
                        .saturating_mul(1u32 << u32::from(retries.status.saturating_sub(1).min(8)))
                }),
                policy.status_retry_max_delay,
            ))
        }
        OpeningFailure::Bootstrap if retries.bootstrap < policy.bootstrap_retries => {
            retries.bootstrap += 1;
            Some((policy.retry_base_delay, policy.retry_max_delay))
        }
        _ => None,
    };
    if let Some((delay, maximum)) = backoff {
        super::retry::delay(state, delay, maximum, deadline).await?;
        super::retry::renew_transport_after_tls(state, prepared, tracker, error).await?;
        return Ok(true);
    }
    if matches!(kind, OpeningFailure::Status)
        && (model_unavailable
            || matches!(
                error.status,
                401 | 402 | 403 | 408 | 429 | 500 | 502 | 503 | 504
            ))
    {
        let mut stop = state.stop.clone();
        tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            result = deadline::timeout_at(deadline, state.host.route_failed_attempt(&prepared.route, &prepared.route.model, if model_unavailable { 404 } else { error.status }, wait, error.credential_scope, Some(&error.message))) => result.map_err(|_| super::retry::total_timeout())?,
        }
    }
    // Authentication failures and throttling must never spend network retries.
    // A self-contained opening can move to another route; opaque continuation
    // requests have a hard preferred account and cannot cross owner boundaries.
    if (model_unavailable
        || matches!(
            error.status,
            401 | 402 | 403 | 408 | 429 | 500 | 502 | 503 | 504
        ))
        && (error.upstream_not_sent || super::retry::credential_limit_allows(policy, retries.credentials))
        && super::retry::can_switch_account(prepared)
    {
        tracker.release_route();
        let mut stop = state.stop.clone();
        let replacement = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            result = deadline::timeout_at(deadline, mapping::retry_other_account(state, prepared)) => result.map_err(|_| super::retry::total_timeout())?,
        };
        match replacement {
            Ok(()) => {
                if !error.upstream_not_sent { retries.credentials += 1; }
                retries.send = 0;
                retries.status = 0;
                retries.bootstrap = 0;
                retries.refreshed = false;
                tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
                tracker.routing_identity(&prepared.request);
                tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                tracker.prepared_metadata(&prepared);
                return Ok(true);
            }
            Err(error) if state.stop.stopped() || tokio::time::Instant::now() >= deadline => {
                return Err(error)
            }
            // Return the original upstream failure if the alternate pool has
            // been exhausted, including the original first-event JSON bytes.
            Err(_) => {}
        }
    }
    if matches!(kind, OpeningFailure::Status) && !retries.cooldown_round {
        retries.cooldown_round = true;
        if super::retry::cooldown_round(state, prepared, tracker, policy, deadline, error.status)
            .await?
        {
            retries.credentials = 0;
            retries.send = 0;
            retries.status = 0;
            retries.bootstrap = 0;
            retries.refreshed = false;
            return Ok(true);
        }
    }
    Ok(false)
}

fn first_event_rejection(
    message: &UpstreamMessage,
    grok: bool,
) -> Option<(ServiceError, Option<std::time::Duration>)> {
    let UpstreamMessage::Text(text) = message else {
        return None;
    };
    let mut event: Value = serde_json::from_str(text.as_str()).ok()?;
    if grok {
        super::grok_errors::normalize_event(&mut event);
    }
    if !matches!(
        super::events::response_terminal_kind(&event),
        Some("error" | "response.failed" | "response.incomplete" | "response.cancelled")
    ) || !super::bootstrap::pre_execution(&event)
    {
        return None;
    }
    let mut observation = ResponseObservation::default();
    observation.observe(&event);
    // Observation uses 502 as a reporting fallback for an unknown error.
    // That fallback alone is not evidence that an opening can be retried.
    // Reuse the shared parser for both integers and numeric strings. A zero
    // fallback distinguishes absent status from a real upstream 502.
    let status = super::upstream_errors::chat_failure(&event, 0)
        .map(|error| error.status)
        .filter(|status| (400..600).contains(status))
        .or_else(|| super::bootstrap::rejection(&event).map(|(status, _)| status))
        .or_else(|| {
            (observation.request_scoped
                || super::upstream_errors::model_unavailable(400, &event, ""))
            .then_some(observation.http_status)
            .flatten()
        })?;
    (400..600).contains(&status).then(|| {
        (
            ServiceError::new(
                status,
                if super::upstream_errors::request_fault(status, &event) {
                    super::upstream_errors::REQUEST_SCOPED
                } else if super::upstream_errors::model_unavailable(status, &event, "") {
                    "upstream_model_unavailable"
                } else {
                    "upstream_websocket_error"
                },
                observation
                    .error_message
                    .as_deref()
                    .unwrap_or("Upstream rejected the opening response request"),
            )
            .with_credential_scope(observation.credential_scope)
            .with_error_details(super::diagnostics::error_object(&event).clone()),
            observation.retry_after,
        )
    })
}

fn prepare_ws_body(
    prepared: &mut Prepared,
    headers: &mut HeaderMap,
) -> Result<Value, ServiceError> {
    if prepared.route.chatgpt_plan {
        return Err(ServiceError::new(
            400,
            "subscription_sharing_unsupported_capability",
            "ChatGPT plan accounts support HTTP Responses inference only",
        ));
    }
    let mut body = prepared.body.clone();
    if prepared.route.chatgpt {
        body["store"] = Value::Bool(false);
        body["stream"] = Value::Bool(true);
    }
    super::multi_agent_transport::sanitize_reasoning(prepared, &mut body);
    if super::grok_errors::applies(&prepared.route) {
        super::multi_agent_transport::prepare(prepared, &mut body)?;
    }
    prepared.grok_context = if super::grok_errors::applies(&prepared.route) {
        let (wire_body, context) = super::grok_wire::prepare(body)?;
        body = wire_body;
        prepared.binding.retain_transmitted_input_blobs(&body);
        Some(context)
    } else {
        None
    };
    super::codex_wire::normalize_websocket(
        headers,
        &mut body,
        prepared.route.chatgpt && !prepared.route.headers.contains_key("x-xai-token-auth"),
    )?;
    if !super::grok_errors::applies(&prepared.route) {
        super::multi_agent_transport::prepare(prepared, &mut body)?;
    }
    let codex_backend = prepared.route.chatgpt && !super::grok_errors::applies(&prepared.route);
    if codex_backend { super::codex_cpa_wire::sanitize_input_ids(&mut body); }
    super::codex_wire::finalize_routing_hint(headers, &body, codex_backend, &prepared.route.headers)?;
    prepared.multi_agent_context.capture_final(&body);
    prepared.binding.retain_transmitted_input_blobs(&body);
    Ok(body)
}

fn websocket_headers(headers: &HeaderMap, chatgpt: bool) -> HeaderMap {
    let mut headers = headers.clone();
    headers.remove("x-codex-turn-state");
    if chatgpt {
        let mut beta = headers
            .get("openai-beta")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty() && !item.starts_with("responses_websockets="))
            .map(str::to_string)
            .collect::<Vec<_>>();
        beta.push("responses_websockets=2026-02-06".to_string());
        if let Ok(value) = beta.join(", ").parse() {
            headers.insert("openai-beta", value);
        }
    }
    headers
}

fn enqueue_request(pending: &mut VecDeque<String>, text: &str) -> Option<ServiceError> {
    const MAX_PENDING: usize = 8;
    const MAX_PENDING_BYTES: usize = 32 * 1024 * 1024;
    let bytes = pending.iter().map(String::len).sum::<usize>();
    // Bound pipelining without treating one valid large conversation as a
    // request-size violation. Further creates wait until it leaves the queue.
    if pending.len() >= MAX_PENDING
        || (!pending.is_empty() && bytes.saturating_add(text.len()) > MAX_PENDING_BYTES)
    {
        return Some(ServiceError::new(
            409,
            "websocket_queue_full",
            "The pending response queue is full; wait for an active response to complete",
        ));
    }
    pending.push_back(text.to_owned());
    None
}
async fn opening_control(
    state: &AppState,
    text: axum::extract::ws::Utf8Bytes,
    pending: &mut VecDeque<String>,
) -> Option<ServiceError> {
    match super::request_body::websocket_json(state, text.clone()).await {
        Err(error) => Some(error),
        Ok(body) => match body.get("type").and_then(Value::as_str) {
            Some("response.create" | "response.append") => enqueue_request(pending, text.as_str()),
            Some("response.interrupt") => Some(ServiceError::new(
                409,
                "unknown_interrupt_response",
                "Wait for the active response ID before interrupting it",
            )),
            _ => Some(ServiceError::new(
                400,
                "unsupported_websocket_event",
                "Expected response.create or response.interrupt",
            )),
        },
    }
}

async fn first_create(
    state: &AppState,
    socket: &mut WebSocket,
) -> Result<Value, ServiceError> {
    let deadline = Deadline::NONE;
    loop {
        let mut stop = state.stop.clone();
        let next = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            result = deadline::timeout_at(deadline, socket.recv()) => result.map_err(|_| ServiceError::new(408, "websocket_start_timeout", "No response.create was received before the deadline"))?,
        };
        match next {
            Some(Ok(Message::Text(text))) => {
                let parsed = super::request_body::websocket_json(state, text).await;
                match parsed.and_then(|body| protocol::normalize_create(body, None, None)) {
                    Ok(body) => return Ok(body),
                    Err(error) if error.status == 400 => {
                        timed_client_until(
                            state,
                            deadline.instant(),
                            socket.send(Message::Text(error_event(&error).to_string().into())),
                        )
                        .await?;
                    }
                    Err(error) => return Err(error),
                }
            }
            Some(Ok(Message::Binary(_))) => {
                let error = ServiceError::new(
                    400,
                    "invalid_websocket_message",
                    "Responses WebSocket requires JSON text messages",
                );
                timed_client_until(
                    state,
                    deadline.instant(),
                    socket.send(Message::Text(error_event(&error).to_string().into())),
                )
                .await?;
            }
            Some(Ok(Message::Ping(value))) => {
                timed_client_until(state, deadline.instant(), socket.send(Message::Pong(value)))
                    .await?;
            }
            Some(Ok(Message::Pong(_))) => {}
            _ => {
                return Err(ServiceError::new(
                    499,
                    "websocket_idle_closed",
                    "The idle WebSocket connection closed before its first request",
                ))
            }
        }
    }
}

async fn timed<T, E: std::error::Error + 'static>(
    state: &AppState,
    future: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, ServiceError> {
    timed_until(state, None, future).await
}

fn initial_frame_error(error: &tokio_tungstenite::tungstenite::Error) -> ServiceError {
    super::diagnostics::transport_failure(
        502,
        "incomplete_upstream_stream",
        "Upstream closed before its first response event",
        error,
    )
}

async fn timed_client_until<T, E: std::error::Error + 'static>(
    state: &AppState,
    deadline: Option<tokio::time::Instant>,
    future: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, ServiceError> {
    timed_until(state, deadline, future)
        .await
        .map_err(|mut error| {
            if error.code == "websocket_send_failed" {
                error.status = 499;
                error.code = "client_disconnected";
            }
            error
        })
}

async fn timed_until<T, E: std::error::Error + 'static>(
    state: &AppState,
    deadline: Option<tokio::time::Instant>,
    future: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, ServiceError> {
    timed_write(
        state.stop.clone(),
        state.options.request_timeout,
        deadline,
        future,
    )
    .await
}

#[cfg(test)]
#[path = "websocket_unsent_dispatch_tests.rs"]
mod unsent_dispatch_tests;

async fn timed_write<T, E: std::error::Error + 'static>(
    mut stop: super::lifecycle::StopSignal,
    write_timeout: std::time::Duration,
    deadline: Option<tokio::time::Instant>,
    future: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, ServiceError> {
    let write_deadline = (tokio::time::Instant::now() + write_timeout)
        .min(deadline.unwrap_or_else(|| tokio::time::Instant::now() + write_timeout));
    if tokio::time::Instant::now() >= write_deadline {
        return Err(ServiceError::new(504, "websocket_send_timeout", "WebSocket write timed out"));
    }
    tokio::select! {
        biased;
        _ = stop.wait() => Err(stopping_error()),
        result = deadline::timeout_at(write_deadline, future) => result
            .map_err(|_| ServiceError::new(504, "websocket_send_timeout", "WebSocket write timed out"))?
            .map_err(|error| super::diagnostics::transport_failure(502, "websocket_send_failed", "WebSocket write failed", &error)),
    }
}

fn error_event(error: &ServiceError) -> Value {
    let mut event = super::output_error::json_error(error);
    event["type"] = json!("error");
    event["status"] = json!(error.status);
    if let Some(delay) = error.retry_after {
        event["retry_after"] = json!(delay.as_secs_f64());
    }
    event
}

fn message_too_big_error(reason: &str) -> ServiceError {
    let reason = if reason.is_empty() {
        "message too big"
    } else {
        reason
    };
    ServiceError::new(413, "message_too_big", reason)
        .with_error_details(json!({"code":"message_too_big", "message":reason}))
}

/// Preserve only the protocol signal that the Go handler uses for SSE fallback.
/// A generic 413 is not evidence that this connection needs that fallback.
fn websocket_error_close(error: &ServiceError) -> Option<axum::extract::ws::CloseFrame> {
    if error.status != 413
        || (error.code != "message_too_big"
            && error
                .response_error
                .as_ref()
                .and_then(|value| value.get("code"))
                .and_then(Value::as_str)
                != Some("message_too_big"))
    {
        return None;
    }
    let reason = error
        .response_error
        .as_ref()
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("message too big");
    let mut reason = super::diagnostics::sanitize_error_text(reason);
    let mut end = reason.len().min(123);
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    reason.truncate(end);
    Some(axum::extract::ws::CloseFrame {
        code: 1009,
        reason: reason.into(),
    })
}

async fn send_error(state: &AppState, socket: &mut WebSocket, error: &ServiceError) {
    let message = websocket_error_close(error).map_or_else(
        || Message::Text(error_event(error).to_string().into()),
        |frame| Message::Close(Some(frame)),
    );
    let _ = timed(state, socket.send(message)).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), socket.close()).await;
}

#[cfg(test)]
#[path = "websocket_bridge_tests.rs"]
mod bridge_tests;
#[cfg(test)]
#[path = "websocket_retry_tests.rs"]
mod retry_tests;

#[cfg(test)]
#[path = "websocket_opening_write_deadline_tests.rs"]
mod opening_write_deadline_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parity22_runtime_close_reason_is_bounded_and_generic_413_does_not_signal_fallback() {
        let error = message_too_big_error(&format!("{} Bearer private-token", "界".repeat(80)));
        let frame = websocket_error_close(&error).unwrap();
        assert_eq!(frame.code, 1009);
        assert!(frame.reason.len() <= 123);
        assert!(!frame.reason.contains("private-token"));
        assert!(
            websocket_error_close(&ServiceError::new(413, "request_too_large", "too large"))
                .is_none()
        );
    }
    #[test]
    fn websocket_error_envelope_preserves_public_details_and_retry_without_secrets() {
        let error = ServiceError::new(429, "upstream_error", "fallback")
            .with_error_details(json!({"type":"rate_limit_error","code":"rate_limit_exceeded","param":"model","message":"retry Bearer private-value","extension":{"access_token":"private-token","count":3}}))
            .with_retry_after(Some(std::time::Duration::from_millis(1500)));
        let event = error_event(&error);
        assert_eq!(event["type"], "error");
        assert_eq!(event["status"], 429);
        assert_eq!(event["retry_after"], 1.5);
        assert_eq!(event["error"]["type"], "rate_limit_error");
        assert_eq!(event["error"]["code"], "rate_limit_exceeded");
        assert_eq!(event["error"]["param"], "model");
        assert_eq!(event["error"]["extension"]["count"], 3);
        assert!(!event.to_string().contains("private-"));
        let isolation =
            ServiceError::new(409, "upstream_state_owner_changed", "State owner changed");
        let event = error_event(&isolation);
        assert_eq!(event["error"]["code"], "upstream_state_owner_changed");
        assert!(event.get("retry_after").is_none());
    }

    #[test]
    fn handshake_preserves_other_beta_features_without_turn_state() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "openai-beta",
            "other_feature=1, responses_websockets=old".parse().unwrap(),
        );
        headers.insert(
            "x-codex-turn-state",
            "private-continuation".parse().unwrap(),
        );
        let result = websocket_headers(&headers, true);
        assert_eq!(
            result["openai-beta"],
            "other_feature=1, responses_websockets=2026-02-06"
        );
        assert!(!result.contains_key("x-codex-turn-state"));
    }
}

#[cfg(test)]
mod control_queue_tests {
    use super::*;
    #[test]
    fn pending_queue_is_fifo_bounded_and_keeps_raw_requests_until_dequeue() {
        let mut queue = VecDeque::new();
        for index in 0..8 {
            let text = serde_json::json!({"type":"response.append","input":[],"extension":index})
                .to_string();
            assert!(enqueue_request(&mut queue, &text).is_none());
        }
        assert_eq!(
            enqueue_request(&mut queue, "{}").unwrap().code,
            "websocket_queue_full"
        );
        assert_eq!(queue.len(), 8);
        for index in 0..8 {
            let body: Value = serde_json::from_str(&queue.pop_front().unwrap()).unwrap();
            assert_eq!(body["extension"], index);
            assert!(body.get("model").is_none());
            assert!(body.get("previous_response_id").is_none());
        }
        let large = "x".repeat(32 * 1024 * 1024 + 1);
        assert!(enqueue_request(&mut queue, &large).is_none());
        assert_eq!(queue.front(), Some(&large));
        assert_eq!(
            enqueue_request(&mut queue, "{}").unwrap().code,
            "websocket_queue_full"
        );
        assert_eq!(queue.len(), 1);
    }
}

// Refresh the internal host capability within the existing preparation budget.
// The application always enables rebuilding; other socket policy stays unchanged.
async fn refresh_rebuild_policy(
    state: &AppState,
    tracker: &RequestTracker,
    policy: &mut ExecutionPolicy,
) -> Result<(), ServiceError> {
    let deadline = super::execution::Execution::new(tracker, policy).prepare_deadline();
    let current = deadline::timeout_at(deadline, super::retry::execution_policy(state))
        .await
        .map_err(|_| {
            ServiceError::new(
                504,
                "execution_policy_timeout",
                "Execution policy lookup timed out",
            )
        })??;
    policy.account_switch_rebuild = current.account_switch_rebuild;
    Ok(())
}

async fn prepare_next_turn(
    state: &AppState,
    caller: &Caller,
    headers: &HeaderMap,
    body: Value,
    preferred: Option<&str>,
    previous: &Prepared,
    history: &history::History,
) -> Result<Prepared, ServiceError> {
    let enabled = super::execution::current()
        .is_some_and(|execution| execution.policy.account_switch_rebuild);
    let original = body.clone();
    match mapping::prepare_websocket_preferred(state, caller, headers, body, preferred).await {
        Ok(next) => Ok(next),
        Err(error)
            if enabled
                && preferred.is_none()
                && matches!(
                    error.code,
                    "account_binding_unavailable" | "account_cooling" | "no_available_account"
                ) =>
        {
            let Some(full) = history.completed_replay(previous, &original)? else {
                return Err(error);
            };
            let mut next =
                mapping::prepare_websocket_preferred(state, caller, headers, full.clone(), None)
                    .await?;
            // Keep the original client message for diagnostics and a separate
            // full logical request for retries and connection-local history.
            next.source_body = original;
            if !next.rebuild_applied {
                next.bridge_body = Some(full);
            }
            Ok(next)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "websocket_transport_identity_tests.rs"]
mod transport_identity_tests;

#[cfg(test)]
#[path = "websocket_tls_error_tests.rs"]
mod tls_error_tests;
