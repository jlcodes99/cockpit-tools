//! Authenticated Claude, Gemini and Ollama edges of the shared execution engine.
use super::{
    compat::{self, CompatFlavor, CompatFrame},
    contract::ServiceError,
    events::SseDecoder,
    http,
    server::AppState,
};
use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};
use std::{collections::VecDeque, sync::Arc};

pub(super) async fn handle(State(state): State<Arc<AppState>>, request: Request) -> Response {
    let flavor = if request.uri().path().starts_with("/api/") {
        CompatFlavor::Ollama
    } else if request.uri().path().starts_with("/v1beta/") {
        CompatFlavor::Gemini
    } else {
        CompatFlavor::Claude
    };
    match execute(state, request, flavor).await {
        Ok(response) => response,
        Err(error) => {
            let mut response = (
                StatusCode::from_u16(error.status).unwrap_or(StatusCode::BAD_GATEWAY),
                Json(compat::error(&error, flavor)),
            )
                .into_response();
            if let Some(delay) = error.retry_after {
                let seconds = delay.as_millis().saturating_add(999) / 1000;
                if let Ok(value) = HeaderValue::from_str(&seconds.max(1).to_string()) {
                    response.headers_mut().insert(header::RETRY_AFTER, value);
                }
            }
            response
        }
    }
}

async fn execute(
    state: Arc<AppState>,
    request: Request,
    flavor: CompatFlavor,
) -> Result<Response, ServiceError> {
    let path = request.uri().path().to_owned();
    if request.method() == axum::http::Method::GET {
        let mut tracker = state.begin()?;
        tracker.context("GET", &path, request.headers(), false);
        let request_headers = request.headers().clone();
        let result = async {
            let caller = state.authorize(&request_headers).await?;
            tracker.authorized(&caller, "");
            if path == "/api/version" {
                return Ok(Json(json!({"version":"0.18.3"})).into_response());
            }
            let catalog = state.model_catalog(&caller).await?;
            let models = compat::models(&catalog, flavor)?;
            if let Some(model) = path.strip_prefix("/v1beta/models/") {
                let item = models["models"]
                    .as_array()
                    .and_then(|items| {
                        items
                            .iter()
                            .find(|v| v["name"] == format!("models/{model}"))
                    })
                    .ok_or_else(|| ServiceError::new(404, "model_not_found", "Model not found"))?;
                return Ok(Json(item.clone()).into_response());
            }
            Ok(Json(models).into_response())
        }
        .await;
        match &result {
            Ok(response) => {
                tracker
                    .complete(response.status().as_u16(), true, None, None)
                    .await
            }
            Err(error) => tracker.fail(error).await,
        }
        return result;
    }
    let model_action = path
        .strip_prefix("/v1beta/models/")
        .and_then(|v| v.rsplit_once(':'))
        .map(|(m, a)| (m.to_string(), a.to_string()));
    let mut input = Some(http::read_request(&state, request).await?);
    let result = execute_body(state, &path, flavor, model_action, &mut input).await;
    if let (Err(error), Some(input)) = (&result, input.as_mut()) {
        input.tracker.fail(error).await;
    }
    result
}

async fn execute_body(
    state: Arc<AppState>,
    path: &str,
    flavor: CompatFlavor,
    model_action: Option<(String, String)>,
    input_slot: &mut Option<http::Input>,
) -> Result<Response, ServiceError> {
    let input = input_slot.as_mut().expect("Compatibility input is present");
    if path == "/api/show" {
        let model = input.body["model"]
            .as_str()
            .or_else(|| input.body["name"].as_str())
            .ok_or_else(|| ServiceError::new(400, "model_required", "A model is required"))?;
        let catalog = state.model_catalog(&input.caller).await?;
        let item = catalog["data"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == model))
            .ok_or_else(|| ServiceError::new(404, "model_not_found", "Model not found"))?;
        let response = Json(compat::ollama_show(item)?).into_response();
        input.tracker.complete(200, true, None, None).await;
        return Ok(response);
    }
    let count = path.ends_with("/count_tokens")
        || model_action
            .as_ref()
            .is_some_and(|(_, action)| action == "countTokens");
    if count {
        let mut body = std::mem::take(&mut input.body);
        if let Some((model, _)) = &model_action {
            body["model"] = json!(model.trim_start_matches("models/"));
        }
        let model = body["model"].as_str().unwrap_or("").trim().to_owned();
        body["model"] = json!(model);
        let catalog = state.model_catalog(&input.caller).await?;
        if !catalog["data"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["id"]
                    .as_str()
                    .is_some_and(|id| id.eq_ignore_ascii_case(&model))
            })
        }) {
            return Err(ServiceError::new(
                403,
                "model_not_allowed",
                "Model is not available to this API key",
            ));
        }
        body["model"] = json!(
            super::execution::prepare(&state, state.host.token_count_model(&input.caller, &model),)
                .await?
        );
        // Existing API exposes a local estimate, rather than performing a generation.
        // Explicitly label it so clients never mistake it for upstream tokenizer usage.
        let count = super::blocking::run(
            state.stop.clone(),
            state.options.request_timeout,
            move || {
                // The Go host first marshals the client protocol object; it does
                // not convert it into generation input before estimating tokens.
                Ok(super::token_count::estimate_preprocessed(&body))
            },
        )
        .await?;
        let result = match flavor {
            CompatFlavor::Claude => json!({"input_tokens":count}),
            CompatFlavor::Gemini => json!({"totalTokens":count}),
            CompatFlavor::Ollama => json!({"prompt_eval_count":count}),
        };
        let mut response = Json(result).into_response();
        response.headers_mut().insert(
            "x-cockpit-token-count",
            HeaderValue::from_static("estimate"),
        );
        input.tracker.complete(200, true, None, None).await;
        return Ok(response);
    }
    let stream_override = if let Some((_, action)) = &model_action {
        match action.as_str() {
            "generateContent" => Some(false),
            "streamGenerateContent" => Some(true),
            _ => {
                return Err(ServiceError::new(
                    404,
                    "unsupported_action",
                    "Unsupported Gemini action",
                ))
            }
        }
    } else {
        None
    };
    let original = std::mem::take(&mut input.body);
    let model = model_action.as_ref().map(|v| v.0.as_str());
    let strict = compat::prepare_request(
        super::compat_options::compatibility(original.clone(), flavor),
        flavor,
        model,
        stream_override,
    );
    let native = compat::prepare_native_chat(&original, flavor, model, stream_override);
    let (converted, raw_chat) = match (strict, native) {
        (Ok(converted), Ok(mut native)) => {
            // The Go-compatible conversion already accepted its ignored options.
            // Preserve the original Chat payload without reviving the old strict error.
            native.responses_error = None;
            (converted, Some(native))
        }
        (Ok(converted), Err(_)) => (converted, None),
        (Err(error), Ok(mut native)) => {
            native.responses_error = Some(error);
            (
                compat::CompatRequest {
                    responses: native.responses.clone(),
                    context: native.context.clone(),
                    stream: native.stream,
                },
                Some(native),
            )
        }
        (Err(error), Err(_)) => return Err(error),
    };
    let expected_choices = raw_chat
        .as_ref()
        .and_then(|raw| raw.body["n"].as_u64())
        .unwrap_or(1)
        .max(1) as usize;
    input.raw_chat = raw_chat.map(|native| super::native_chat::RawChat {
        body: native.body,
        endpoint: "chat/completions",
        conversion_error: native.responses_error,
    });
    input.body = converted.responses;
    let mut outcome = input.tracker.defer_downstream();
    let response = match http::dispatch(
        state.clone(),
        input_slot.take().expect("Compatibility input is present"),
        false,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            outcome.fail(&error);
            return Err(error);
        }
    };
    if !response.status().is_success() {
        outcome.success();
        let (mut parts, body) = response.into_parts();
        let bytes = to_bytes(body, state.options.max_response_bytes)
            .await
            .map_err(|_| {
                ServiceError::new(502, "upstream_error_body", "Unable to read upstream error")
            })?;
        let value = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
        let message = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("Upstream rejected the request");
        let error = ServiceError::new(parts.status.as_u16(), "upstream_error", message);
        parts.headers.remove(header::CONTENT_LENGTH);
        parts.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        return Ok(Response::from_parts(
            parts,
            Body::from(compat::error(&error, flavor).to_string()),
        ));
    }
    let native_chat = response
        .extensions()
        .get::<super::native_chat::Passthrough>()
        .is_some();
    let deferred_protocol = response
        .extensions()
        .get::<super::native_chat::DeferredProtocol>()
        .cloned();
    let (mut parts, body) = response.into_parts();
    parts.headers.remove(header::CONTENT_LENGTH);
    if !converted.stream {
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
            Ok(Response::from_parts(
                parts,
                Body::from(
                    if native_chat {
                        compat::response_from_chat(&value, &converted.context)?
                    } else {
                        compat::response_from_responses(&value, &converted.context)?
                    }
                    .to_string(),
                ),
            ))
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
        HeaderValue::from_static(if flavor == CompatFlavor::Ollama {
            "application/x-ndjson"
        } else {
            "text/event-stream"
        }),
    );
    let state = Output {
        source: body.into_data_stream().boxed(),
        decoder: SseDecoder::new(state.options.max_event_bytes),
        deferred_native: deferred_protocol.as_ref().map(|_| {
            compat::NativeChatStreamAdapter::new(converted.context.clone(), expected_choices)
        }),
        deferred_protocol,
        adapter: if native_chat {
            OutputAdapter::Native(compat::NativeChatStreamAdapter::new(
                converted.context,
                expected_choices,
            ))
        } else {
            OutputAdapter::Responses(compat::CompatStreamAdapter::new(converted.context))
        },
        flavor,
        pending: VecDeque::new(),
        finished: false,
        outcome,
    };
    let body = Body::from_stream(stream::unfold(state, |mut state| async move {
        loop {
            if let Some(value) = state.pending.pop_front() {
                return Some((value, state));
            }
            if state.finished {
                return None;
            }
            let result = match state.source.next().await {
                Some(Ok(bytes)) => match state.decoder.push_frames(&bytes) {
                    Ok(frames) => {
                        let mut error = None;
                        for frame in frames {
                            let Some(event) = frame.value else {
                                if state.flavor != CompatFlavor::Ollama && !done_frame(&frame.bytes)
                                {
                                    state.pending.push_back(Ok(frame.bytes));
                                }
                                continue;
                            };
                            state.choose_adapter();
                            match state.adapter.push(&event) {
                                Ok(frames) => {
                                    enqueue(&mut state.pending, frames, state.flavor);
                                    if state.adapter.is_finished() {
                                        state.outcome.success();
                                        state.finished = true;
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error = Some(e);
                                    break;
                                }
                            }
                        }
                        if error.is_none() && !state.finished && state.decoder.done {
                            state.choose_adapter();
                            match state.adapter.finish(true) {
                                Ok(frames) => {
                                    enqueue(&mut state.pending, frames, state.flavor);
                                    state.outcome.success();
                                    state.finished = true;
                                }
                                Err(failure) => error = Some(failure),
                            }
                        }
                        error.map_or(Ok(()), Err)
                    }
                    Err(e) => Err(e),
                },
                Some(Err(_)) => Err(ServiceError::new(
                    502,
                    "upstream_stream_error",
                    "Responses stream disconnected",
                )),
                None => {
                    state.finished = true;
                    state.choose_adapter();
                    let result = state
                        .adapter
                        .finish(false)
                        .map(|frames| enqueue(&mut state.pending, frames, state.flavor));
                    if result.is_ok() {
                        state.outcome.success();
                    }
                    result
                }
            };
            if let Err(error) = result {
                state.outcome.fail(&error);
                state.finished = true;
                state.pending.push_back(Ok(Bytes::from(format_frame(
                    CompatFrame {
                        event: Some("error".into()),
                        data: compat::error(&error, state.flavor),
                    },
                    state.flavor,
                ))));
            }
        }
    }));
    Ok(Response::from_parts(parts, body))
}
fn format_frame(frame: CompatFrame, flavor: CompatFlavor) -> String {
    if flavor == CompatFlavor::Ollama {
        format!("{}\n", frame.data)
    } else {
        format!(
            "{}data: {}\n\n",
            frame
                .event
                .map(|v| format!("event: {v}\n"))
                .unwrap_or_default(),
            frame.data
        )
    }
}
fn enqueue(
    queue: &mut VecDeque<Result<Bytes, std::io::Error>>,
    frames: Vec<CompatFrame>,
    flavor: CompatFlavor,
) {
    queue.extend(
        frames
            .into_iter()
            .map(|frame| Ok(Bytes::from(format_frame(frame, flavor)))),
    );
}
struct Output {
    source: futures_util::stream::BoxStream<'static, Result<Bytes, axum::Error>>,
    decoder: SseDecoder,
    adapter: OutputAdapter,
    deferred_protocol: Option<super::native_chat::DeferredProtocol>,
    deferred_native: Option<compat::NativeChatStreamAdapter>,
    flavor: CompatFlavor,
    pending: VecDeque<Result<Bytes, std::io::Error>>,
    finished: bool,
    outcome: super::lifecycle::DownstreamOutcomeGuard,
}

impl Output {
    fn choose_adapter(&mut self) {
        if let Some(protocol) = self.deferred_protocol.take() {
            if protocol.is_native() {
                self.adapter = OutputAdapter::Native(
                    self.deferred_native
                        .take()
                        .expect("Deferred native adapter is present"),
                );
            } else {
                self.deferred_native = None;
            }
        }
    }
}

fn done_frame(bytes: &Bytes) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| {
        text.lines().any(|line| {
            line.strip_prefix("data:")
                .is_some_and(|data| data.trim() == "[DONE]")
        })
    })
}

enum OutputAdapter {
    Responses(compat::CompatStreamAdapter),
    Native(compat::NativeChatStreamAdapter),
}
impl OutputAdapter {
    fn is_finished(&self) -> bool {
        match self {
            Self::Responses(adapter) => adapter.is_finished(),
            Self::Native(_) => false,
        }
    }
    fn push(&mut self, event: &Value) -> Result<Vec<CompatFrame>, ServiceError> {
        match self {
            Self::Responses(adapter) => adapter.push(event),
            Self::Native(adapter) => adapter.push(event),
        }
    }
    fn finish(&mut self, done: bool) -> Result<Vec<CompatFrame>, ServiceError> {
        match self {
            Self::Responses(adapter) => adapter.finish(),
            Self::Native(adapter) if done => adapter.finish_done(),
            Self::Native(adapter) => adapter.finish(),
        }
    }
}
