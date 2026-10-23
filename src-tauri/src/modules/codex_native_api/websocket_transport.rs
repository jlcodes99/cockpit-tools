//! A downstream Responses socket can use either upstream WebSocket or HTTP/SSE.
use crate::modules::codex_native_api::deadline::{self, Deadline};
use super::super::{
    contract::{Caller, ExecutionPolicy, ServiceError},
    events::SseDecoder,
    http::{open_response_stream, HttpResponseStream},
    lifecycle::{stopping_error, RequestTracker},
    mapping::{self, Prepared},
    server::AppState,
};
use super::{history::History, open_response, OpeningRetries};
use futures_util::{stream, stream::BoxStream, StreamExt};
use serde_json::{json, Value};
use std::{collections::VecDeque, sync::Arc};
use tokio_tungstenite::tungstenite::protocol::Message as UpstreamMessage;

pub(super) type NativeSocket = tokio_tungstenite::WebSocketStream<reqwest::Upgraded>;
pub(super) struct Transport {
    pub sink: Option<futures_util::stream::SplitSink<NativeSocket, UpstreamMessage>>,
    pub stream: BoxStream<'static, Result<UpstreamMessage, ServiceError>>,
    pub http: bool,
    pub local: bool,
    pub identity: Option<super::super::diagnostics::HeaderIdentity>,
    pub(super) opening_retries: OpeningRetries,
}

pub(super) async fn passthrough(
    state: &AppState,
    caller: &Caller,
    prepared: &Prepared,
) -> Result<bool, ServiceError> {
    // Public plan credentials use stateless HTTP/SSE even when the caller uses
    // a socket. Keep the connection's authenticated history in this gateway.
    if prepared.route.chatgpt_plan {
        return Ok(false);
    }
    if super::super::grok_errors::applies(&prepared.route)
        && (super::super::grok_wire::compaction::triggered(&prepared.body)
            || prepared
                .body
                .get("previous_response_id")
                .is_some_and(Value::is_string))
    {
        // A compact response was created through HTTP, outside the upstream
        // socket's state. Expand the known local checkpoint before forwarding.
        return Ok(false);
    }
    let mut stop = state.stop.clone();
    tokio::select! {
        _=stop.wait()=>Err(stopping_error()),
        result=deadline::timeout_at((tokio::time::Instant::now()+state.options.request_timeout).min(prepared.execution.prepare_deadline()),state.host.responses_websocket_passthrough(caller,&prepared.route))=>result.map_err(|_| {
            if tokio::time::Instant::now()>=prepared.execution.total_deadline { super::super::retry::total_timeout() }
            else {ServiceError::new(504,"request_policy_timeout","WebSocket transport policy timed out")}
        })?,
    }
}

pub(super) async fn open(
    state: &Arc<AppState>,
    caller: &Caller,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: Deadline,
    history: &mut History,
) -> Result<Transport, ServiceError> {
    open_with_retries(
        state,
        caller,
        prepared,
        tracker,
        policy,
        deadline,
        history,
        OpeningRetries::default(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn open_with_retries(
    state: &Arc<AppState>,
    caller: &Caller,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: Deadline,
    history: &mut History,
    retries: OpeningRetries,
) -> Result<Transport, ServiceError> {
    history.begin(prepared);
    if passthrough(state, caller, prepared).await? {
        match open_response(state, prepared, tracker, policy, deadline, retries).await {
            Ok((socket, initial, identity, opening_retries)) => {
                history.begin(prepared);
                let (sink, source) = socket.split();
                let source = stream::iter(initial.into_iter().map(Ok))
                    .chain(source)
                    .map(|item| {
                        item.map_err(|error| {
                            super::super::diagnostics::transport_failure(
                                502,
                                "upstream_stream_error",
                                "Upstream WebSocket disconnected",
                                &error,
                            )
                        })
                    })
                    .boxed();
                return Ok(Transport {
                    sink: Some(sink),
                    stream: source,
                    http: false,
                    local: false,
                    identity: Some(identity),
                    opening_retries,
                });
            }
            Err(error) if error.status == 426 && error.code == "websocket_upstream_rejected" => {
                tracker.attempt_error(&error);
            }
            Err(error) => return Err(error),
        }
    }
    if prepared.source_body.get("generate") == Some(&Value::Bool(false)) {
        let events = history.prewarm(&prepared.source_body);
        let source = stream::iter(
            events
                .into_iter()
                .map(|event| Ok(UpstreamMessage::Text(event.to_string().into()))),
        )
        .boxed();
        return Ok(Transport {
            sink: None,
            stream: source,
            http: true,
            local: true,
            identity: None,
            opening_retries: OpeningRetries::default(),
        });
    }
    let body = history.replay(prepared)?;
    prepared.body = mapping::rewrite_body(state, prepared, body.clone()).await?;
    if let Some(object) = prepared.body.as_object_mut() {
        object.remove("type");
        object.remove("generate");
    }
    if prepared.rebuild_applied {
        if let Some(plan) = prepared.rebuild_plan.as_mut() {
            plan.body = body;
        }
    } else if prepared.execution.policy.account_switch_rebuild {
        prepared.bridge_body = Some(body);
    } else {
        prepared.source_body = body;
    }
    let response =
        open_response_stream(state, prepared, false, true, tracker, policy, deadline).await?;
    if !response.status.is_success() {
        let status = response.status.as_u16();
        tracker.quota_headers(&response.headers);
        let retry_after = super::super::events::retry_after_headers(&response.headers);
        let bytes = collect_bytes(state, response.bytes, policy, deadline).await?;
        let body: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        let (message, scope) =
            super::super::diagnostics::upstream_error_metadata_status(status, &body);
        let message = message.or_else(|| {
            let text = String::from_utf8_lossy(&bytes);
            (!text.trim().is_empty()).then(|| super::super::diagnostics::sanitize_error_text(&text))
        });
        return Err(ServiceError::new(
            status,
            if !response
                .headers
                .get("cf-mitigated")
                .is_some_and(|value| value == "challenge")
                && super::super::upstream_errors::request_fault(status, &body)
            {
                super::super::upstream_errors::REQUEST_SCOPED
            } else {
                "upstream_http_error"
            },
            message.unwrap_or_else(|| format!("Upstream rejected the request with HTTP {status}")),
        )
        .with_retry_after(retry_after)
        .with_credential_scope(scope)
        .with_error_details(super::super::diagnostics::error_object(&body).clone()));
    }
    history.begin(prepared);
    let HttpResponseStream { bytes, is_sse, .. } = response;
    let source = if is_sse {
        sse_messages(bytes, state.options.max_event_bytes)
    } else {
        let response = collect_json(state, bytes, policy, deadline).await?;
        json_messages(response)?
    };
    Ok(Transport {
        sink: None,
        stream: source,
        http: true,
        local: false,
        identity: None,
        opening_retries: OpeningRetries::default(),
    })
}

pub(super) fn sse_messages(
    source: super::super::upstream::ByteStream,
    limit: usize,
) -> BoxStream<'static, Result<UpstreamMessage, ServiceError>> {
    struct Reader {
        source: super::super::upstream::ByteStream,
        decoder: SseDecoder,
        pending: VecDeque<Value>,
        finished: bool,
    }
    let reader = Reader {
        source,
        decoder: SseDecoder::without_output_repair(limit),
        pending: VecDeque::new(),
        finished: false,
    };
    stream::unfold(reader, |mut reader| async move {
        loop {
            if let Some(event) = reader.pending.pop_front() {
                return Some((Ok(UpstreamMessage::Text(event.to_string().into())), reader));
            }
            if reader.finished {
                return None;
            }
            let events = match reader.source.next().await {
                Some(Ok(bytes)) => reader.decoder.push(&bytes),
                Some(Err(error)) => Err(super::super::diagnostics::transport_failure(
                    502,
                    "upstream_stream_error",
                    "Upstream HTTP stream disconnected",
                    &error,
                )),
                None => {
                    reader.finished = true;
                    reader.decoder.finish()
                }
            };
            match events {
                Ok(events) => reader.pending.extend(events),
                Err(error) => {
                    reader.finished = true;
                    return Some((Err(error), reader));
                }
            }
        }
    })
    .boxed()
}

async fn collect_json(
    state: &AppState,
    source: super::super::upstream::ByteStream,
    policy: &ExecutionPolicy,
    deadline: Deadline,
) -> Result<Value, ServiceError> {
    let data = collect_bytes(state, source, policy, deadline).await?;
    serde_json::from_slice(&data).map_err(|_| {
        ServiceError::new(
            502,
            "invalid_upstream_response",
            "Upstream did not return valid JSON",
        )
    })
}

async fn collect_bytes(
    state: &AppState,
    mut source: super::super::upstream::ByteStream,
    policy: &ExecutionPolicy,
    deadline: Deadline,
) -> Result<Vec<u8>, ServiceError> {
    let mut data = Vec::new();
    loop {
        let mut stop = state.stop.clone();
        let next = tokio::select! {
            _=stop.wait()=>return Err(stopping_error()),
            _=deadline::sleep_until(deadline)=>return Err(super::super::retry::total_timeout()),
            result=tokio::time::timeout(policy.idle_timeout,source.next())=>result.map_err(|_|ServiceError::new(504,"upstream_idle_timeout","Upstream response stopped responding"))?,
        };
        match next {
            Some(Ok(bytes))
                if data.len().saturating_add(bytes.len()) <= state.options.max_response_bytes =>
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
                return Err(super::super::diagnostics::transport_failure(
                    502,
                    "upstream_stream_error",
                    "Unable to read the upstream response",
                    &error,
                ))
            }
            None => break,
        }
    }
    Ok(data)
}

fn json_messages(
    response: Value,
) -> Result<BoxStream<'static, Result<UpstreamMessage, ServiceError>>, ServiceError> {
    if !response["id"].as_str().is_some_and(|id| !id.is_empty()) || !response["output"].is_array() {
        return Err(ServiceError::new(
            502,
            "invalid_upstream_response",
            "Upstream did not return a valid Responses object",
        ));
    }
    let kind = match response["status"].as_str() {
        Some("completed") => "response.completed",
        Some("failed") => "response.failed",
        Some("incomplete") => "response.incomplete",
        Some("cancelled") => "response.cancelled",
        _ => {
            return Err(ServiceError::new(
                502,
                "incomplete_upstream_stream",
                "Upstream did not return a terminal Responses object",
            ))
        }
    };
    let mut created = response.clone();
    created["status"] = json!("in_progress");
    created["output"] = json!([]);
    // A terminal error or usage on the synthetic created event would settle
    // the bridge before its actual failure/cancellation event can be delivered.
    for field in ["error", "incomplete_details", "usage"] {
        created.as_object_mut().unwrap().remove(field);
    }
    let mut events = vec![json!({"type":"response.created","response":created})];
    for (index, item) in response["output"].as_array().unwrap().iter().enumerate() {
        events.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
    }
    events.push(json!({"type":kind,"response":response}));
    Ok(stream::iter(
        events
            .into_iter()
            .map(|event| Ok(UpstreamMessage::Text(event.to_string().into()))),
    )
    .boxed())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn json_bridge_emits_tools_and_preserves_original_terminal_fields() {
        let response = json!({"id":"r","status":"completed","extension":true,"output":[{"type":"function_call","call_id":"c","name":"tool","arguments":"{}"}]});
        let events = json_messages(response.clone())
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert_eq!(events.len(), 3);
        let UpstreamMessage::Text(text) = events[2].as_ref().unwrap() else {
            panic!()
        };
        let event: Value = serde_json::from_str(text).unwrap();
        assert_eq!(event["response"], response);
        assert!(json_messages(json!({"id":"r","status":"in_progress","output":[]})).is_err());
    }
}
