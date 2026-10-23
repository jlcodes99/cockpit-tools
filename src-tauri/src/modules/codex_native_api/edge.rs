//! Public compatibility protocols share one native Responses execution path.
use super::{
    chat::{self, ChatFlavor, StreamOutput},
    contract::ServiceError,
    events::SseDecoder,
    http,
    server::AppState,
};
use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{Request, State},
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
};
use futures_util::{stream, StreamExt};
use std::{collections::VecDeque, sync::Arc};

pub(super) async fn chat_completions(
    State(state): State<Arc<AppState>>,
    request: Request,
) -> Response {
    chat_request(state, request, ChatFlavor::ChatCompletions)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}
pub(super) async fn completions(State(state): State<Arc<AppState>>, request: Request) -> Response {
    chat_request(state, request, ChatFlavor::Completions)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}
async fn chat_request(
    state: Arc<AppState>,
    request: Request,
    flavor: ChatFlavor,
) -> Result<Response, ServiceError> {
    let mut input = http::read_request(&state, request).await?;
    let original = input.body.clone();
    let (adapted, error) = match chat::prepare_request(
        super::compat_options::chat(input.body, flavor == ChatFlavor::Completions),
        flavor,
    ) {
        Ok(value) => (value, None),
        Err(error) => {
            // Native providers can represent these generation options. Remove
            // only those options from the routing copy, retaining all history
            // and identity metadata; the original body still reaches Chat intact.
            let mut routing = original.clone();
            if let Some(object) = routing.as_object_mut() {
                for field in [
                    "n",
                    "stop",
                    "seed",
                    "logit_bias",
                    "prediction",
                    "audio",
                    "suffix",
                    "presence_penalty",
                    "frequency_penalty",
                    "modalities",
                    "echo",
                    "best_of",
                ] {
                    object.remove(field);
                }
                if flavor == ChatFlavor::Completions {
                    object.remove("logprobs");
                }
            }
            if let Ok(adapted) = chat::prepare_request(routing, flavor) {
                (adapted, Some(error))
            } else {
                let model = match original["model"].as_str().filter(|v| !v.trim().is_empty()) {
                    Some(model) => model.to_owned(),
                    None => {
                        input.tracker.fail(&error).await;
                        return Err(error);
                    }
                };
                let mut history = original.clone();
                let mut items = history
                    .get("input")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if let Some(messages) = original["messages"].as_array() {
                    items.extend(messages.iter().cloned());
                } else if let Some(prompt) = original.get("prompt") {
                    items.push(serde_json::json!({"role":"user","content":prompt}));
                }
                if let Some(object) = history.as_object_mut() {
                    object.remove("messages");
                    object.remove("prompt");
                    object.insert("input".into(), serde_json::Value::Array(items));
                }
                (
                    chat::ChatRequest {
                        responses: history,
                        context: chat::ChatContext {
                            flavor,
                            model,
                            include_usage: original
                                .pointer("/stream_options/include_usage")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                            legacy_functions: original.get("functions").is_some(),
                            tool_names: Default::default(),
                        },
                        stream: original["stream"].as_bool().unwrap_or(false),
                    },
                    Some(error),
                )
            }
        }
    };
    input.raw_chat = Some(super::native_chat::RawChat {
        body: original,
        endpoint: if flavor == ChatFlavor::ChatCompletions {
            "chat/completions"
        } else {
            "completions"
        },
        conversion_error: error,
    });
    input.body = adapted.responses;
    let mut outcome = input.tracker.defer_downstream();
    let mut response = match http::dispatch(state.clone(), input, false).await {
        Ok(response) => response,
        Err(error) => {
            outcome.fail(&error);
            return Err(error);
        }
    };
    let deferred = response
        .extensions()
        .get::<super::native_chat::DeferredProtocol>()
        .cloned();
    if response
        .extensions_mut()
        .remove::<super::native_chat::Passthrough>()
        .is_some()
        || !response.status().is_success()
    {
        outcome.success();
        return Ok(response);
    }
    let (mut parts, body) = response.into_parts();
    parts.headers.remove(header::CONTENT_LENGTH);
    if !adapted.stream {
        let result = async {
            let bytes = to_bytes(body, state.options.max_response_bytes)
                .await
                .map_err(|_| {
                    ServiceError::new(
                        502,
                        "invalid_responses_body",
                        "Unable to read the Responses result",
                    )
                })?;
            let value = serde_json::from_slice(&bytes).map_err(|_| {
                ServiceError::new(
                    502,
                    "invalid_responses_body",
                    "Responses result is not JSON",
                )
            })?;
            let output = chat::response_to_chat(&value, &adapted.context)?;
            parts.headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            Ok(Response::from_parts(parts, Body::from(output.to_string())))
        }
        .await;
        match &result {
            Ok(_) => outcome.success(),
            Err(error) => outcome.fail(error),
        }
        return result;
    }
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    let state = ChatOutputStream {
        source: body.into_data_stream().boxed(),
        decoder: SseDecoder::new(state.options.max_event_bytes),
        adapter: chat::ChatStreamAdapter::new(adapted.context),
        pending: VecDeque::new(),
        finished: false,
        outcome,
        deferred,
    };
    let stream = stream::unfold(state, |mut state| async move {
        loop {
            if let Some(value) = state.pending.pop_front() {
                return Some((value, state));
            }
            if state.finished {
                return None;
            }
            let result = match state.source.next().await {
                Some(Ok(bytes))
                    if state
                        .deferred
                        .as_ref()
                        .is_some_and(|protocol| protocol.is_native()) =>
                {
                    return Some((Ok(bytes), state));
                }
                Some(Ok(bytes)) => match state.decoder.push_frames(&bytes) {
                    Ok(frames) => {
                        let mut failure = None;
                        for frame in frames {
                            let Some(event) = frame.value else {
                                if !std::str::from_utf8(&frame.bytes).is_ok_and(|text| {
                                    text.lines().any(|line| {
                                        line.strip_prefix("data:")
                                            .is_some_and(|data| data.trim() == "[DONE]")
                                    })
                                }) {
                                    state.pending.push_back(Ok(frame.bytes));
                                }
                                continue;
                            };
                            match state.adapter.push(&event) {
                                Ok(events) => {
                                    enqueue(&mut state.pending, events);
                                    if state.adapter.is_finished() {
                                        state.outcome.success();
                                        state.finished = true;
                                        break;
                                    }
                                }
                                Err(error) => {
                                    failure = Some(error);
                                    break;
                                }
                            }
                        }
                        failure.map_or(Ok(()), Err)
                    }
                    Err(error) => Err(error),
                },
                Some(Err(_)) => Err(ServiceError::new(
                    502,
                    "upstream_stream_error",
                    "Responses stream disconnected",
                )),
                None => {
                    state.finished = true;
                    let result = if state
                        .deferred
                        .as_ref()
                        .is_some_and(|protocol| protocol.is_native())
                    {
                        Ok(())
                    } else {
                        state
                            .adapter
                            .finish()
                            .map(|events| enqueue(&mut state.pending, events))
                    };
                    if result.is_ok() {
                        state.outcome.success();
                    }
                    result
                }
            };
            if let Err(error) = result {
                state.outcome.fail(&error);
                state.finished = true;
                // Headers may already be committed. Keep the failure readable
                // by Chat clients instead of aborting the body and losing it.
                let value = super::output_error::json_error(&error);
                state
                    .pending
                    .push_back(Ok(Bytes::from(format!("data: {value}\n\n"))));
            }
        }
    });
    Ok(Response::from_parts(parts, Body::from_stream(stream)))
}

fn enqueue(queue: &mut VecDeque<Result<Bytes, std::io::Error>>, events: Vec<StreamOutput>) {
    queue.extend(events.into_iter().map(|event| {
        Ok(Bytes::from(match event {
            StreamOutput::Event(value) => format!("data: {value}\n\n"),
            StreamOutput::Done => "data: [DONE]\n\n".to_owned(),
        }))
    }));
}
struct ChatOutputStream {
    source: futures_util::stream::BoxStream<'static, Result<Bytes, axum::Error>>,
    decoder: SseDecoder,
    adapter: chat::ChatStreamAdapter,
    pending: VecDeque<Result<Bytes, std::io::Error>>,
    finished: bool,
    outcome: super::lifecycle::DownstreamOutcomeGuard,
    deferred: Option<super::native_chat::DeferredProtocol>,
}
