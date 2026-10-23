//! HTTP adapters for independent Images calls. A requested batch retains one owner.
use crate::modules::codex_native_api::deadline::{self, Deadline};
use std::{collections::VecDeque, sync::Arc};

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{FromRequest, Multipart, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::{stream, stream::BoxStream, StreamExt};
use serde_json::{json, Value};

use super::super::{
    blocking,
    contract::{Caller, ImageRoute, ImageRouteRequest, ServiceError},
    events::{retry_after_headers, SseDecoder},
    identity::Binding,
    lifecycle::{stopping_error, RequestTracker},
    server::{downstream_response, outbound_headers, AppState},
};
use super::{
    merge_responses, native_from_events, native_to_events, ImageAccumulator, ImageOperation,
    ImagePart, ImageRequest,
};

pub(crate) async fn generations(State(state): State<Arc<AppState>>, request: Request) -> Response {
    handle(state, request, ImageOperation::Generation)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

pub(crate) async fn edits(State(state): State<Arc<AppState>>, request: Request) -> Response {
    handle(state, request, ImageOperation::Edit)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn handle(
    state: Arc<AppState>,
    incoming: Request,
    operation: ImageOperation,
) -> Result<Response, ServiceError> {
    let mut tracker = state.begin()?;
    let capture = tokio::time::timeout(
        state.options.request_timeout,
        state.host.payload_logging_enabled(),
    )
    .await
    .unwrap_or(false);
    tracker.context(
        incoming.method().as_str(),
        incoming.uri().path(),
        incoming.headers(),
        capture,
    );
    let caller = state.authorize(incoming.headers()).await?;
    let headers = incoming.headers().clone();
    let request = parse_request(&state, incoming, operation).await?;
    tracker.authorized(&caller, request.model.as_deref().unwrap_or(""));
    let mut policy = match super::super::retry::execution_policy(&state).await {
        Ok(policy) => policy.for_images(),
        Err(error) => {
            tracker.fail(&error).await;
            return Err(error);
        }
    };
    let immediate = request.stream && policy.immediate_sse;
    let keepalive = policy.keepalive;
    policy.immediate_sse = false;
    let budget = super::super::execution::Execution::new(&tracker, &policy);
    let deadline = budget.total_deadline;
    if immediate {
        return Ok(super::super::http::immediate_stream(
            Box::pin(async move {
                budget
                    .run(handle_prepared(state, caller, headers, request, tracker))
                    .await
            }),
            keepalive,
            deadline,
        ));
    }
    budget
        .run(handle_prepared(state, caller, headers, request, tracker))
        .await
}

async fn handle_prepared(
    state: Arc<AppState>,
    caller: Caller,
    headers: HeaderMap,
    mut request: ImageRequest,
    mut tracker: RequestTracker,
) -> Result<Response, ServiceError> {
    let route_request = ImageRouteRequest {
        model: request.model.clone(),
        responses_model: None,
        edit: request.operation == ImageOperation::Edit,
        target_account_id: headers
            .get("x-cockpit-target-account-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        force_refresh: false,
        preferred_account_id: None,
        excluded_account_ids: Vec::new(),
    };
    let prepared = async {
        let selected = super::super::execution::within(
            &state,
            state.host.image_route(&caller, route_request.clone()),
        )
        .await?;
        if !selected.route.chatgpt
            && request.original.get("async").and_then(Value::as_bool) == Some(true)
        {
            return Err(ServiceError::new(
                400,
                "async_image_not_supported",
                "Asynchronous image jobs are not supported",
            ));
        }
        super::super::execution::prepare(&state, request.prepare_urls(&state)).await?;
        Ok::<_, ServiceError>(selected)
    };
    let selected = match prepared.await {
        Ok(selected) => selected,
        Err(error) => {
            tracker.fail(&error).await;
            return Err(error);
        }
    };
    let mut execution = Execution {
        budget: super::super::execution::current_or_new(&state).await?,
        state,
        caller,
        headers,
        request,
        selected,
        route_request,
        tracker,
        usage: None,
        deadline: None,
        owner_committed: false,
    };
    execution.record_route();
    if execution.selected.route.chatgpt {
        forward_oauth(execution).await
    } else {
        forward_native(execution).await
    }
}

async fn parse_request(
    state: &AppState,
    incoming: Request,
    operation: ImageOperation,
) -> Result<ImageRequest, ServiceError> {
    let encoding = incoming
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("identity")
        .trim()
        .to_ascii_lowercase();
    let multipart = incoming
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("multipart/form-data"))
        });
    if multipart {
        if encoding != "identity" {
            return Err(ServiceError::new(
                415,
                "unsupported_content_encoding",
                "Multipart image uploads require identity content encoding",
            ));
        }
        let mut fields = Multipart::from_request(incoming, state)
            .await
            .map_err(|_| {
                ServiceError::new(
                    400,
                    "invalid_image_multipart",
                    "Invalid image multipart request",
                )
            })?;
        let mut parts = Vec::new();
        loop {
            let mut stop = state.stop.clone();
            let next = tokio::select! {
                _ = stop.wait() => return Err(stopping_error()),
                result = tokio::time::timeout(state.options.request_timeout, fields.next_field()) =>
                    result.map_err(|_| ServiceError::new(408,"request_body_timeout","Timed out reading image upload"))?
                        .map_err(|_| ServiceError::new(400,"invalid_image_multipart","Invalid image multipart body"))?,
            };
            let Some(field) = next else {
                break;
            };
            if parts.len() >= 64 {
                return Err(ServiceError::new(
                    413,
                    "request_too_large",
                    "Too many image upload fields",
                ));
            }
            let name = field
                .name()
                .ok_or_else(|| {
                    ServiceError::new(
                        400,
                        "invalid_image_multipart",
                        "Image upload field is missing its name",
                    )
                })?
                .to_owned();
            let filename = field.file_name().map(str::to_owned);
            let content_type = field.content_type().map(str::to_owned);
            let mut stop = state.stop.clone();
            let bytes = tokio::select! {
                _ = stop.wait() => return Err(stopping_error()),
                result = tokio::time::timeout(state.options.request_timeout, field.bytes()) =>
                    result.map_err(|_| ServiceError::new(408,"request_body_timeout","Timed out reading image upload"))?
                        .map_err(|_| ServiceError::new(413,"request_too_large","Image upload exceeds its size limit"))?,
            };
            // Go bounds each uploaded file, not the sum of multipart fields.
            if filename.is_some() && bytes.len() > state.options.max_upload_bytes {
                return Err(ServiceError::new(
                    413,
                    "request_too_large",
                    "Image upload exceeds its size limit",
                ));
            }
            parts.push(ImagePart {
                name,
                filename,
                content_type,
                bytes: bytes.to_vec(),
            });
        }
        return blocking::run(
            state.stop.clone(),
            state.options.request_timeout,
            move || ImageRequest::from_multipart(operation, parts),
        )
        .await;
    }
    let encoding_headers = incoming.headers().clone();
    let mut stop = state.stop.clone();
    let bytes = tokio::select! {
        _ = stop.wait() => return Err(stopping_error()),
        result = tokio::time::timeout(state.options.request_timeout, to_bytes(incoming.into_body(), state.options.max_body_bytes)) =>
            result.map_err(|_| ServiceError::new(408,"request_body_timeout","Timed out reading image request"))?
                .map_err(|_| ServiceError::new(413,"request_too_large","Image request exceeds its size limit"))?,
    };
    let decoded = super::super::http::decode_body_with_limit(
        state,
        bytes,
        &encoding_headers,
        state.options.max_body_bytes,
    )
    .await?;
    blocking::run(
        state.stop.clone(),
        state.options.request_timeout,
        move || {
            let body = serde_json::from_slice(&decoded).map_err(|_| {
                ServiceError::new(400, "invalid_json", "Image request must contain valid JSON")
            })?;
            ImageRequest::from_json(operation, body)
        },
    )
    .await
}

pub(super) struct Execution {
    pub(super) budget: super::super::execution::Execution,
    pub(super) state: Arc<AppState>,
    pub(super) caller: Caller,
    pub(super) headers: HeaderMap,
    pub(super) request: ImageRequest,
    pub(super) selected: ImageRoute,
    pub(super) route_request: ImageRouteRequest,
    pub(super) tracker: RequestTracker,
    pub(super) usage: Option<Value>,
    pub(super) deadline: Option<(Deadline, std::time::Duration)>,
    pub(super) owner_committed: bool,
}

impl Execution {
    pub(super) fn response_limit(&self) -> usize {
        if self.selected.route.chatgpt {
            self.state.options.max_response_bytes
        } else {
            self.state.options.max_image_response_bytes
        }
    }

    pub(super) fn event_limit(&self) -> usize {
        if self.selected.route.chatgpt {
            self.state.options.max_event_bytes
        } else {
            self.response_limit()
        }
    }

    fn can_switch_source(&self) -> bool {
        fn opaque(value: &Value) -> bool {
            match value {
                Value::Object(fields) => fields.iter().any(|(key, value)| {
                    (key == "encrypted_content" && !value.is_null()) || opaque(value)
                }),
                Value::Array(values) => values.iter().any(opaque),
                _ => false,
            }
        }
        !self.owner_committed
            && self.route_request.preferred_account_id.is_none()
            && !self.headers.contains_key("x-codex-turn-state")
            && self
                .request
                .original
                .get("previous_response_id")
                .is_none_or(Value::is_null)
            && !opaque(&self.request.original)
    }
    pub(super) fn asset_budget(&self) -> super::assets::AssetBudget {
        super::assets::AssetBudget::output(
            self.budget.total_deadline,
            self.tracker.deadline(super::assets::ASSET_OPERATION_LIMIT),
            self.budget.policy.idle_timeout,
        )
    }
    pub(super) fn record_route(&mut self) {
        self.tracker
            .set_operation(if self.request.operation == ImageOperation::Edit {
                "image_edit"
            } else {
                "image_generation"
            });
        let mut route = self.selected.route.clone();
        route.model = self.selected.image_model.clone();
        self.tracker.routed(
            &self.caller,
            &route,
            self.request
                .model
                .as_deref()
                .unwrap_or(&self.selected.image_model),
        );
        self.tracker.capture_usage(&self.usage);
    }

    pub(super) async fn send(
        &mut self,
        oauth: bool,
    ) -> Result<(reqwest::Response, Option<Binding>), ServiceError> {
        if !oauth {
            // Go's independent Images API waits for generation with one
            // five-minute context; the OAuth stream-open/idle knobs do not apply.
            let image_timeout = std::time::Duration::from_secs(300);
            self.budget.total_deadline = self.budget.total_deadline
                .min(Deadline::after(image_timeout));
            self.budget.policy.open_timeout = image_timeout;
            self.budget.policy.idle_timeout = image_timeout;
            self.budget = self.budget.renew_open();
        }
        let mut refreshed = false;
        let mut send_attempts = 0u8;
        let mut status_attempts = 0u8;
        let mut open_attempts = 1u8;
        let mut bootstrap_attempts = 0u8;
        let mut credential_attempts = 0usize;
        let mut last_upstream_failure: Option<ServiceError> = None;
        let mut round_attempted = false;
        let policy = self.budget.policy.clone();
        let deadline = self
            .budget
            .total_deadline
            .min(self.tracker.deadline(policy.total_timeout));
        self.deadline = Some((deadline, policy.idle_timeout));
        let streaming = oauth || self.request.stream;
        let mut previous_open_deadline = None;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(super::super::retry::total_timeout());
            }
            if previous_open_deadline == Some(self.budget.open_deadline) {
                self.budget = self.budget.renew_open();
            }
            previous_open_deadline = Some(self.budget.open_deadline);
            let mut headers = outbound_headers(&self.headers);
            headers.extend(self.selected.route.headers.clone());
            headers.remove(header::CONTENT_LENGTH);
            headers.remove(header::CONTENT_ENCODING);
            headers.remove(header::CONTENT_TYPE);
            headers.insert(
                header::ACCEPT,
                HeaderValue::from_static("application/json, text/event-stream"),
            );
            let (path, mut payload, binding) = if oauth {
                let main_model = self.selected.main_model.as_deref().ok_or_else(|| {
                    ServiceError::new(
                        503,
                        "image_main_model_missing",
                        "Image generation needs a configured Responses model",
                    )
                })?;
                let body = self
                    .request
                    .to_responses(main_model, &self.selected.image_model)?;
                let store = self.state.identity.clone();
                let caller = self.caller.id.clone();
                let account = self.selected.route.continuity_id().to_owned();
                let owner = self.selected.route.owner_id.clone();
                let budget = self.budget.clone();
                let (new_headers, body, binding) = budget
                    .run(super::super::execution::prepare(
                        &self.state,
                        blocking::run_prepared(&self.state, move || {
                            let context = store.inspect(&caller, &headers, &body)?;
                            let mut body = body;
                            let binding = store.rewrite(
                                &context,
                                &account,
                                &owner,
                                &mut headers,
                                &mut body,
                            )?;
                            Ok((headers, body, binding))
                        }),
                    ))
                    .await?;
                headers = new_headers;
                ("responses", body, Some(binding))
            } else {
                (
                    self.request.operation.path(),
                    self.request.native_json(&self.selected.image_model),
                    None,
                )
            };
            if oauth {
                super::super::account_installation::apply(
                    &self.selected.route,
                    &mut headers,
                    &mut payload,
                )?;
            } else if self.selected.route.installation_profile.is_some() {
                // A standalone Images API is not a Codex protocol endpoint.
                headers.remove("x-codex-installation-id");
            }
            let url = format!(
                "{}/{}",
                self.selected.route.base_url.trim_end_matches('/'),
                path
            );
            let mut outgoing = self.selected.route.client.post(url).headers(headers);
            if !oauth {
                if let Some(parts) = self.request.native_parts(&self.selected.image_model) {
                    let mut form = reqwest::multipart::Form::new();
                    for part in parts {
                        let mut value = reqwest::multipart::Part::bytes(part.bytes);
                        if let Some(filename) = part.filename {
                            value = value.file_name(filename);
                        }
                        if let Some(mime) = part.content_type {
                            value = value.mime_str(&mime).map_err(|_| {
                                ServiceError::new(
                                    400,
                                    "invalid_image_multipart",
                                    "Invalid image part content type",
                                )
                            })?;
                        }
                        form = form.part(part.name, value);
                    }
                    outgoing = outgoing.multipart(form);
                } else {
                    outgoing = outgoing.json(&payload);
                }
            } else {
                outgoing = outgoing.json(&payload);
            }
            let mut stop = self.state.stop.clone();
            let (client, built) = outgoing.build_split();
            let built = built.map_err(|error| super::super::diagnostics::transport_failure(
                502, "upstream_request_invalid", "Unable to prepare the image upstream request", &error,
            ).with_upstream_not_sent(true));
            let open_deadline = deadline.cap(self.budget.prepare_deadline());
            let outgoing = async {
                let built = built?;
                if tokio::time::Instant::now() >= open_deadline {
                    return Err(ServiceError::new(504,"upstream_connect_timeout","Image upstream response headers timed out"));
                }
                self.tracker.begin_attempt("http");
                self.tracker.payload("upstream_request", "http", built.headers(), &payload.to_string());
                self.tracker.upstream_headers(built.headers());
                self.tracker.check_quota_before_send()?;
                client.execute(built).await.map_err(|error| {
                    super::super::diagnostics::transport_failure(502,"upstream_unavailable","Unable to connect to the image upstream",&error)
                        .with_upstream_not_sent(error.is_connect() || error.is_builder())
                })
            };
            let result = tokio::select! {
                biased;
                _ = stop.wait() => return Err(stopping_error()),
                result = deadline::timeout_at(open_deadline, outgoing) =>
                    result.map_err(|_| ServiceError::new(504,"upstream_connect_timeout","Image upstream response headers timed out"))
                        .and_then(|value| value),
            };
            let mut response = match result {
                Ok(response) => response,
                Err(error) => {
                    if error.upstream_not_sent {
                        self.tracker.discard_unsent_attempt();
                    } else {
                        self.tracker.attempt_error(&error);
                        last_upstream_failure = Some(error.clone());
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err(super::super::retry::total_timeout());
                    }
                    let stream_open_timeout = streaming && error.code == "upstream_connect_timeout";
                    if stream_open_timeout {
                        if open_attempts < policy.stream_open_max_attempts.max(1) {
                            open_attempts += 1;
                            continue;
                        }
                    }
                    if error.upstream_not_sent || stream_open_timeout || send_attempts >= policy.send_retries {
                        if (error.upstream_not_sent || super::super::retry::credential_limit_allows(
                            &policy,
                            credential_attempts,
                        )) && self.alternate_source(None, deadline).await?
                        {
                            if !error.upstream_not_sent { credential_attempts += 1; }
                            send_attempts = 0;
                            status_attempts = 0;
                            open_attempts = 1;
                            refreshed = false;
                            continue;
                        }
                        if !round_attempted
                            && self.cooldown_round(error.status, &policy, deadline).await?
                        {
                            round_attempted = true;
                            credential_attempts = 0;
                            send_attempts = 0;
                            status_attempts = 0;
                            open_attempts = 1;
                            refreshed = false;
                            continue;
                        }
                        return Err(if error.upstream_not_sent {
                            last_upstream_failure.clone().unwrap_or(error)
                        } else { error });
                    }
                    send_attempts += 1;
                    super::super::retry::delay(
                        &self.state,
                        policy.retry_base_delay.saturating_mul(
                            1u32 << u32::from(send_attempts.saturating_sub(1).min(8)),
                        ),
                        policy.send_retry_max_delay,
                        deadline,
                    )
                    .await?;
                    self.renew_transport_after_tls(&error).await?;
                    continue;
                }
            };
            self.tracker.upstream_http_version(response.version());
            self.tracker.upstream_response_headers(response.headers());
            self.tracker.quota_headers(response.headers());
            if oauth && response.status().is_success() {
                // The shared probe stops at the first business event or its
                // 250ms hold limit. Only errors returned inside that fence may
                // replay; failures from the subsequent image reader never do.
                response =
                    match super::super::bootstrap::probe(&self.state, response, &policy, deadline)
                        .await
                    {
                        Ok(response) => response,
                        Err(mut error) => {
                            error.message = format!(
                                "[image_stream stage=bootstrap business_event_seen=false] {}",
                                error.message,
                            );
                            if matches!(
                                error.code,
                                "upstream_stream_error"
                                    | "incomplete_upstream_stream"
                                    | "upstream_timeout"
                                    | "upstream_idle_timeout"
                            ) && bootstrap_attempts < policy.bootstrap_retries
                            {
                                self.tracker.attempt_error(&error);
                                bootstrap_attempts += 1;
                                super::super::retry::delay(
                                    &self.state,
                                    policy.retry_base_delay,
                                    policy.retry_max_delay,
                                    deadline,
                                )
                                .await?;
                                self.renew_transport_after_tls(&error).await?;
                                continue;
                            }
                            return Err(error);
                        }
                    };
            }
            let mut credential_scope = false;
            let mut body_retry_after = None;
            let mut failure_message = None;
            let mut model_unavailable = false;
            let mut request_scoped = false;
            if !response.status().is_success() {
                let inspected = super::super::retry::inspect_status_error(
                    &self.state,
                    response,
                    &policy,
                    deadline,
                    &self.selected.image_model,
                    super::super::grok_errors::applies(&self.selected.route),
                )
                .await?;
                response = inspected.0;
                credential_scope = inspected.1;
                body_retry_after = inspected.3;
                self.tracker
                    .capture_response_error(inspected.2.as_deref(), credential_scope);
                failure_message = inspected.2;
                model_unavailable = inspected.4;
                request_scoped = inspected.5;
                self.tracker.capture_request_fault(request_scoped);
                self.tracker.attempt_result(
                    response.status().as_u16(),
                    false,
                    Some("image_upstream_error"),
                );
                // inspect_status_error has already bounded and buffered this
                // body. Keep only the structured error, not a response/stream.
                let status = response.status();
                let version = response.version();
                let headers = response.headers().clone();
                let bytes = response.bytes().await.map_err(|error| {
                    super::super::diagnostics::transport_failure(status.as_u16(),
                        "upstream_http_error", "Unable to read the image upstream error", &error)
                })?;
                let detail = serde_json::from_slice::<Value>(&bytes).ok()
                    .map(|value| super::super::diagnostics::error_object(&value).clone());
                let mut saved = ServiceError::new(status.as_u16(), "image_upstream_error",
                    failure_message.clone().unwrap_or_else(|| "Image upstream rejected the request".into()))
                    .with_credential_scope(credential_scope)
                    .with_retry_after(retry_after_headers(&headers).max(body_retry_after));
                if let Some(detail) = detail { saved = saved.with_error_details(detail); }
                last_upstream_failure = Some(saved);
                let mut rebuilt = axum::http::Response::new(bytes);
                *rebuilt.status_mut() = status;
                *rebuilt.version_mut() = version;
                *rebuilt.headers_mut() = headers;
                response = reqwest::Response::from(rebuilt);
            }
            let retry_after = match (retry_after_headers(response.headers()), body_retry_after) {
                (Some(headers), Some(body)) => Some(headers.max(body)),
                (headers, body) => headers.or(body),
            };
            self.tracker.set_retry_after(retry_after);
            if request_scoped {
                self.tracker.set_retry_after(retry_after);
                return Ok((response, binding));
            }
            if oauth
                && response.status() == StatusCode::UNAUTHORIZED
                && !refreshed
                && !model_unavailable
            {
                self.tracker.release_route();
                self.selected.route.permit = None;
                let mut route_request = self.route_request.clone();
                route_request.force_refresh = true;
                route_request.preferred_account_id = Some(self.selected.route.continuity_id().to_owned());
                self.budget = self.budget.renew_open();
                let refreshed_route = self
                    .budget
                    .run(super::super::execution::within(
                        &self.state,
                        self.state.host.image_route(&self.caller, route_request),
                    ))
                    .await;
                refreshed = true;
                match refreshed_route {
                    Ok(selected) => {
                        if selected.route.owner_id != self.selected.route.owner_id
                            || selected.route.account_id != self.selected.route.account_id
                            || selected.image_model != self.selected.image_model
                        {
                            return Err(ServiceError::new(
                                409, "refresh_owner_changed",
                                "Image credential refresh cannot change the selected source or model",
                            ));
                        }
                        drop(response);
                        self.selected = selected;
                        self.record_route();
                        continue;
                    }
                    Err(error)
                        if self.state.stop.stopped()
                            || tokio::time::Instant::now() >= deadline
                            || error.code == "refresh_owner_changed" =>
                    {
                        return Err(error)
                    }
                    // Refresh is auxiliary to the actual failed request. Keep
                    // its original status/body and apply the same source fence
                    // before considering a different OAuth image credential.
                    Err(_) => {}
                }
            }
            if matches!(response.status().as_u16(), 408 | 500 | 502 | 503 | 504)
                && !model_unavailable
                && status_attempts < policy.status_retries
            {
                let delay = retry_after.unwrap_or_else(|| {
                    policy
                        .status_retry_base_delay
                        .saturating_mul(1u32 << u32::from(status_attempts.min(8)))
                });
                drop(response);
                status_attempts += 1;
                super::super::retry::delay(
                    &self.state,
                    delay,
                    policy.status_retry_max_delay,
                    deadline,
                )
                .await?;
                continue;
            }
            let mut failure_recorded = false;
            let retryable_source = model_unavailable
                || oauth && matches!(response.status().as_u16(), 401 | 402)
                || matches!(
                    response.status().as_u16(),
                    403 | 408 | 429 | 500 | 502 | 503 | 504
                );
            if retryable_source
                && super::super::retry::credential_limit_allows(&policy, credential_attempts)
                && self.can_switch_source()
                && {
                    failure_recorded = true;
                    self.alternate_source(
                        Some((
                            if model_unavailable {
                                404
                            } else {
                                response.status().as_u16()
                            },
                            retry_after,
                            credential_scope,
                            failure_message.as_deref(),
                        )),
                        deadline,
                    )
                    .await?
                }
            {
                drop(response);
                credential_attempts += 1;
                send_attempts = 0;
                status_attempts = 0;
                open_attempts = 1;
                refreshed = false;
                continue;
            }
            if retryable_source && !round_attempted {
                if !failure_recorded {
                    let mut stop = self.state.stop.clone();
                    tokio::select! {
                        _=stop.wait()=>return Err(stopping_error()),
                        result=deadline::timeout_at(deadline,self.state.host.route_failed_attempt(&self.selected.route,&self.selected.image_model,if model_unavailable { 404 } else { response.status().as_u16() },retry_after,credential_scope,failure_message.as_deref()))=>result.map_err(|_|super::super::retry::total_timeout())?,
                    }
                }
                if self
                    .cooldown_round(response.status().as_u16(), &policy, deadline)
                    .await?
                {
                    round_attempted = true;
                    credential_attempts = 0;
                    send_attempts = 0;
                    status_attempts = 0;
                    open_attempts = 1;
                    refreshed = false;
                    continue;
                }
            }
            self.tracker.set_retry_after(retry_after);
            if let Some(binding) = &binding {
                let store = self.state.identity.clone();
                let binding = binding.clone();
                let headers = response.headers().clone();
                blocking::run_tracked(&self.state, move || {
                    store.observe_headers(&binding, &headers)
                })
                .await?;
            }
            return Ok((response, binding));
        }
    }

    async fn renew_transport_after_tls(&mut self, error: &ServiceError) -> Result<(), ServiceError> {
        if error.transport_failure != Some(super::super::contract::TransportFailureKind::TlsDecrypt) {
            return Ok(());
        }
        self.tracker.release_route();
        self.selected.route.permit = None;
        let mut request = self.route_request.clone();
        request.force_refresh = false;
        request.preferred_account_id = Some(self.selected.route.continuity_id().to_owned());
        self.budget = self.budget.renew_open();
        let selected = self.budget.run(super::super::execution::within(
            &self.state, self.state.host.image_route(&self.caller, request),
        )).await?;
        if selected.route.account_id != self.selected.route.account_id
            || selected.route.owner_id != self.selected.route.owner_id
            || selected.image_model != self.selected.image_model
            || selected.main_model != self.selected.main_model
            || selected.route.model != self.selected.route.model
            || selected.route.wire_api != self.selected.route.wire_api
            || selected.route.base_url != self.selected.route.base_url
            || selected.route.chatgpt != self.selected.route.chatgpt
            || selected.route.chatgpt_plan != self.selected.route.chatgpt_plan
        {
            return Err(ServiceError::new(409, "refresh_owner_changed", "Transport recovery cannot change the image source or model"));
        }
        self.selected = selected;
        self.record_route();
        Ok(())
    }

    async fn alternate_source(
        &mut self,
        upstream_failure: Option<(u16, Option<std::time::Duration>, bool, Option<&str>)>,
        deadline: Deadline,
    ) -> Result<bool, ServiceError> {
        if !self.can_switch_source() {
            return Ok(false);
        }
        self.budget = self.budget.renew_open();
        let was_oauth = self.selected.route.chatgpt;
        let old = self.selected.route.account_id.clone();
        if let Some((status, retry_after, credential_scope, message)) = upstream_failure {
            let mut stop = self.state.stop.clone();
            tokio::select! {
                _=stop.wait()=>return Err(stopping_error()),
                result=deadline::timeout_at(deadline,self.state.host.route_failed_attempt(&self.selected.route,&self.selected.image_model,status,retry_after,credential_scope,message))=>{result.map_err(|_|super::super::retry::total_timeout())?;}
            }
        }
        self.tracker.release_route();
        self.selected.route.permit = None;
        let mut request = self.route_request.clone();
        request.force_refresh = false;
        request.preferred_account_id = None;
        if !request.excluded_account_ids.contains(&old) {
            request.excluded_account_ids.push(old);
        }
        // A different wire protocol would require a different response adapter.
        // Skip incompatible sources without sending an image operation to them.
        for _ in 0..128 {
            let result = self
                .budget
                .run(super::super::execution::within(
                    &self.state,
                    self.state.host.image_route(&self.caller, request.clone()),
                ))
                .await;
            let selected = match result {
                Ok(selected) => selected,
                Err(_) => return Ok(false),
            };
            if request
                .excluded_account_ids
                .contains(&selected.route.account_id)
            {
                return Ok(false);
            }
            if selected.route.chatgpt == was_oauth {
                self.route_request = request;
                self.selected = selected;
                self.record_route();
                return Ok(true);
            }
            request
                .excluded_account_ids
                .push(selected.route.account_id.clone());
        }
        Ok(false)
    }

    async fn cooldown_round(
        &mut self,
        status: u16,
        policy: &super::super::contract::ExecutionPolicy,
        deadline: Deadline,
    ) -> Result<bool, ServiceError> {
        self.budget = self.budget.renew_open();
        let budget = self.budget.clone();
        let state = self.state.clone();
        budget
            .run(super::super::execution::within(
                &state,
                self.cooldown_round_inner(status, policy, deadline),
            ))
            .await
    }

    async fn cooldown_round_inner(
        &mut self,
        status: u16,
        policy: &super::super::contract::ExecutionPolicy,
        deadline: Deadline,
    ) -> Result<bool, ServiceError> {
        if !self.can_switch_source() {
            return Ok(false);
        }
        let request = super::super::contract::RouteRequest {
            model: self
                .request
                .model
                .clone()
                .unwrap_or_else(|| self.selected.image_model.clone()),
            session_id: None,
            thread_id: None,
            parent_thread_id: None,
            preferred_account_id: self.route_request.preferred_account_id.clone(),
            soft_preferred_account_id: None,
            target_account_id: self.route_request.target_account_id.clone(),
            force_refresh: false,
            image_generation_requested: true,
            image_input: false,
            excluded_account_ids: vec![],
            requires_permit: true,
            required_capabilities: Vec::new(),
            capability_fallback_from: None,
            auxiliary: Some(
                if self.request.operation == ImageOperation::Edit {
                    "images.edits"
                } else {
                    "images.generations"
                }
                .into(),
            ),
        };
        let mut stop = self.state.stop.clone();
        let wait = tokio::select! {
            _=stop.wait()=>return Err(stopping_error()),
            result=deadline::timeout_at(deadline,self.state.host.retry_cooldown_wait(&self.caller,request,policy.retry_max_delay))=>result.map_err(|_|super::super::retry::total_timeout())??,
        };
        let Some(mut wait) = wait else {
            return Ok(false);
        };
        if status == 429 && !wait.is_zero() {
            wait = wait.max(std::time::Duration::from_secs(10));
        }
        if wait > policy.retry_max_delay || tokio::time::Instant::now() + wait >= deadline {
            return Ok(false);
        }
        self.tracker.release_route();
        self.selected.route.permit = None;
        super::super::retry::delay(&self.state, wait, policy.retry_max_delay, deadline).await?;
        let mut request = self.route_request.clone();
        request.excluded_account_ids.clear();
        let mut stop = self.state.stop.clone();
        let selected = tokio::select! {
            _=stop.wait()=>return Err(stopping_error()),
            result=deadline::timeout_at(deadline,self.state.host.image_route(&self.caller,request.clone()))=>result.map_err(|_|super::super::retry::total_timeout())??,
        };
        if selected.route.chatgpt != self.selected.route.chatgpt
            || (request.preferred_account_id.is_some()
                && (selected.route.account_id != self.selected.route.account_id
                    || selected.route.owner_id != self.selected.route.owner_id))
        {
            return Err(ServiceError::new(
                409,
                "continuation_owner_mismatch",
                "Retry cannot change the image owner or wire protocol",
            ));
        }
        self.route_request = request;
        self.selected = selected;
        self.record_route();
        Ok(true)
    }

    pub(super) async fn read_bounded(
        &self,
        response: reqwest::Response,
    ) -> Result<Vec<u8>, ServiceError> {
        let (deadline, idle_timeout) = self.deadline.unwrap_or_else(|| {
            (
                (tokio::time::Instant::now() + self.state.options.request_timeout).into(),
                self.state.options.idle_timeout,
            )
        });
        deadline::timeout_at(deadline, read_bounded(&self.state, response, idle_timeout, self.response_limit()))
            .await
            .map_err(|_| super::super::retry::total_timeout())?
    }

    pub(super) async fn read_next(
        &self,
        stream: &mut BoxStream<'static, Result<Bytes, reqwest::Error>>,
    ) -> Result<Option<Bytes>, ServiceError> {
        let (deadline, idle_timeout) = self.deadline.unwrap_or_else(|| {
            (
                (tokio::time::Instant::now() + self.state.options.idle_timeout).into(),
                self.state.options.idle_timeout,
            )
        });
        deadline::timeout_at(deadline, read_next(&self.state, stream, idle_timeout))
            .await
            .map_err(|_| super::super::retry::total_timeout())?
    }
}

async fn forward_native(mut execution: Execution) -> Result<Response, ServiceError> {
    let (upstream, _) = match execution.send(false).await {
        Ok(value) => value,
        Err(error) => {
            execution.tracker.fail(&error).await;
            return Err(error);
        }
    };
    let status = upstream.status();
    let headers = upstream.headers().clone();
    if is_sse(&headers) && status.is_success() && execution.request.stream {
        let current = NativeStream {
            decoder: SseDecoder::new(execution.event_limit()),
            upstream: upstream.bytes_stream().boxed(),
            execution,
            completed: 0,
            terminal: false,
            usage: None,
            progress: ImageStreamProgress::new(true),
        };
        let body = Body::from_stream(stream::unfold(current, |mut current| async move {
            if current.terminal {
                return None;
            }
            let next = current.execution.read_next(&mut current.upstream).await;
            match next {
                Ok(bytes) => {
                    let eof = bytes.is_none();
                    let events = match bytes {
                        Some(bytes) => {
                            current.progress.chunk(bytes.len());
                            if current.progress.bytes > current.execution.response_limit() {
                                return current.error(ServiceError::new(502, "upstream_response_too_large", "Image response exceeds its size limit")).await;
                            }
                            current.execution.tracker.first_response();
                            current.decoder.push(&bytes)
                        }
                        None => current.decoder.finish(),
                    };
                    let events = match events {
                        Ok(events) => events,
                        Err(error) => return current.error(error).await,
                    };
                    let mut output = String::new();
                    for mut event in events {
                        current.progress.event(&event);
                        if let Some(usage) = event.get("usage") {
                            current.usage = Some(usage.clone());
                            current.execution.tracker.capture_usage(&current.usage);
                        }
                        let kind = event
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if matches!(
                            kind.as_str(),
                            "image_generation.completed" | "image_edit.completed"
                        ) {
                            let mut wrapped = json!({"data":[event.clone()]});
                            if let Err(error) = super::assets::normalize_output(
                                &current.execution.state,
                                &mut wrapped,
                                current.execution.request.response_format,
                                current.execution.response_limit(),
                                current.execution.asset_budget(),
                            )
                            .await
                            {
                                return current.error_after(error, output).await;
                            }
                            event = wrapped["data"][0].take();
                            current.completed += 1;
                        }
                        if kind == "error" {
                            return current
                                .error_after(super::result::image_failure(&event), output)
                                .await;
                        }
                        let kind = event
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("message");
                        output.push_str(&format!("event: {kind}\ndata: {event}\n\n"));
                    }
                    if eof {
                        if current.completed == 0 {
                            return current
                                .error_after(
                                    ServiceError::new(
                                        502,
                                        "incomplete_upstream_stream",
                                        "Image stream ended without completion",
                                    ),
                                    output,
                                )
                                .await;
                        }
                        current
                            .execution
                            .tracker
                            .complete(200, true, current.usage.take(), None)
                            .await;
                        current.terminal = true;
                        if output.is_empty() {
                            return None;
                        }
                    }
                    Some((Ok::<Bytes, std::io::Error>(Bytes::from(output)), current))
                }
                Err(error) => current.error(error).await,
            }
        }));
        return Ok(downstream_response(status, &headers, body));
    }
    let bytes = match execution.read_bounded(upstream).await {
        Ok(bytes) => bytes,
        Err(error) => {
            execution.tracker.fail(&error).await;
            return Err(error);
        }
    };
    if is_sse(&headers) && status.is_success() {
        let mut decoder = SseDecoder::new(execution.event_limit());
        let mut events = decoder.push(&bytes)?;
        events.extend(decoder.finish()?);
        let usage = events
            .iter()
            .rev()
            .find_map(|event| event.get("usage"))
            .cloned();
        execution.tracker.capture_usage(&usage);
        let mut value = match native_from_events(&events) {
            Ok(value) if !decoder.has_partial_event() => value,
            Ok(_) => {
                let error = ServiceError::new(
                    502,
                    "incomplete_upstream_stream",
                    "Native image stream has a truncated event",
                );
                execution.tracker.fail(&error).await;
                return Err(error);
            }
            Err(error) => {
                execution.tracker.fail(&error).await;
                return Err(error);
            }
        };
        if let Err(error) = super::assets::normalize_output(
            &execution.state,
            &mut value,
            execution.request.response_format,
            execution.response_limit(),
            execution.asset_budget(),
        )
        .await
        {
            execution.tracker.fail(&error).await;
            return Err(error);
        }
        execution
            .tracker
            .complete(200, true, value.get("usage").cloned(), None)
            .await;
        return Ok(with_upstream_headers(
            axum::Json(value).into_response(),
            &headers,
        ));
    }
    let mut value = serde_json::from_slice::<Value>(&bytes).ok();
    let usage = value.as_ref().and_then(|value| value.get("usage")).cloned();
    execution.tracker.capture_usage(&usage);
    let succeeded = status.is_success()
        && value
            .as_ref()
            .is_none_or(|value| value.get("error").is_none_or(Value::is_null));
    let bytes = if succeeded {
        let value = value.as_mut().ok_or_else(|| {
            ServiceError::new(
                502,
                "invalid_image_response",
                "Native Images response was not valid JSON",
            )
        })?;
        if let Err(error) = super::assets::normalize_output(
            &execution.state,
            value,
            execution.request.response_format,
            execution.response_limit(),
            execution.asset_budget(),
        )
        .await
        {
            execution.tracker.fail(&error).await;
            return Err(error);
        }
        value.to_string().into_bytes()
    } else {
        bytes
    };
    if succeeded && execution.request.stream {
        let events = match value
            .as_ref()
            .ok_or_else(|| {
                ServiceError::new(
                    502,
                    "invalid_image_response",
                    "Native Images response was not valid JSON",
                )
            })
            .and_then(|value| native_to_events(value, execution.request.operation))
        {
            Ok(events) => events,
            Err(error) => {
                execution.tracker.fail(&error).await;
                return Err(error);
            }
        };
        let mut output = String::new();
        for event in events {
            let kind = event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("image_generation.completed");
            output.push_str(&format!("event: {kind}\ndata: {event}\n\n"));
        }
        execution.tracker.complete(200, true, usage, None).await;
        return Ok(with_upstream_headers(
            (
                [
                    (header::CONTENT_TYPE, "text/event-stream"),
                    (header::CACHE_CONTROL, "no-cache"),
                ],
                output,
            )
                .into_response(),
            &headers,
        ));
    }
    execution
        .tracker
        .complete(
            status.as_u16(),
            succeeded,
            usage,
            (!succeeded).then(|| "image_upstream_error".to_owned()),
        )
        .await;
    Ok(downstream_response(status, &headers, Body::from(bytes)))
}

struct NativeStream {
    execution: Execution,
    upstream: BoxStream<'static, Result<Bytes, reqwest::Error>>,
    decoder: SseDecoder,
    completed: u32,
    terminal: bool,
    usage: Option<Value>,
    progress: ImageStreamProgress,
}

impl NativeStream {
    async fn error(self, error: ServiceError) -> Option<(Result<Bytes, std::io::Error>, Self)> {
        self.error_after(error, String::new()).await
    }

    async fn error_after(
        mut self,
        error: ServiceError,
        mut output: String,
    ) -> Option<(Result<Bytes, std::io::Error>, Self)> {
        self.terminal = true;
        let error = self.progress.failure(error);
        self.execution.tracker.fail(&error).await;
        output.push_str(&String::from_utf8_lossy(&error_event(&error)));
        Some((Ok(Bytes::from(output)), self))
    }
}

pub(super) fn is_sse(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
        })
}

pub(super) async fn read_next(
    state: &AppState,
    upstream: &mut BoxStream<'static, Result<Bytes, reqwest::Error>>,
    idle_timeout: std::time::Duration,
) -> Result<Option<Bytes>, ServiceError> {
    let mut stop = state.stop.clone();
    tokio::select! {
        _=stop.wait()=>Err(stopping_error()),
        result=tokio::time::timeout(idle_timeout,upstream.next())=>
            result.map_err(|_|ServiceError::new(504,"upstream_idle_timeout","Image upstream stopped responding"))?
                .transpose().map_err(image_read_failure),
    }
}

pub(super) async fn read_bounded(
    state: &AppState,
    response: reqwest::Response,
    idle_timeout: std::time::Duration,
    limit: usize,
) -> Result<Vec<u8>, ServiceError> {
    let mut upstream = response.bytes_stream().boxed();
    let mut bytes = Vec::new();
    let mut progress = ImageStreamProgress::new(false);
    while let Some(chunk) = read_next(state, &mut upstream, idle_timeout)
        .await
        .map_err(|error| progress.failure(error))?
    {
        progress.chunk(chunk.len());
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(ServiceError::new(
                502,
                "upstream_response_too_large",
                "Image response exceeds its size limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn image_read_failure(error: reqwest::Error) -> ServiceError {
    super::super::diagnostics::transport_failure(
        502,
        "upstream_stream_error",
        "Image upstream disconnected",
        &error,
    )
}

/// Payload-free evidence for a failed read. Event labels are fixed categories,
/// never an arbitrary upstream `type`, prompt, response ID or image payload.
#[derive(Default)]
struct ImageStreamProgress {
    bytes: usize,
    chunks: usize,
    events: usize,
    handshake_events: usize,
    business_events: usize,
    image_events: usize,
    completion_seen: bool,
    inspect_events: bool,
    last_event: &'static str,
}

impl ImageStreamProgress {
    fn new(inspect_events: bool) -> Self {
        Self {
            inspect_events,
            last_event: "none",
            ..Self::default()
        }
    }

    fn chunk(&mut self, bytes: usize) {
        self.bytes = self.bytes.saturating_add(bytes);
        self.chunks = self.chunks.saturating_add(1);
    }

    fn event(&mut self, event: &Value) {
        self.events = self.events.saturating_add(1);
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let handshake = super::super::bootstrap::handshake(event);
        let image = kind.starts_with("response.image_generation_call.")
            || kind.starts_with("image_generation.")
            || kind.starts_with("image_edit.")
            || event.pointer("/item/type").and_then(Value::as_str) == Some("image_generation_call");
        self.handshake_events = self.handshake_events.saturating_add(usize::from(handshake));
        self.business_events = self.business_events.saturating_add(usize::from(!handshake));
        self.image_events = self.image_events.saturating_add(usize::from(image));
        self.completion_seen |= matches!(
            kind,
            "response.completed" | "image_generation.completed" | "image_edit.completed"
        );
        self.last_event = if kind.ends_with(".partial_image") && image {
            "image_partial"
        } else if image {
            "image_event"
        } else if handshake {
            "handshake"
        } else if self.completion_seen {
            "completed"
        } else {
            "business_event"
        };
    }

    fn failure(&self, mut error: ServiceError) -> ServiceError {
        if !matches!(
            error.code,
            "upstream_stream_error"
                | "upstream_timeout"
                | "upstream_idle_timeout"
                | "upstream_total_timeout"
                | "incomplete_upstream_stream"
        ) {
            return error;
        }
        let stage = if self.image_events > 0 {
            "image_activity"
        } else if self.business_events > 0 {
            "business_output"
        } else if self.handshake_events > 0 {
            "handshake"
        } else if self.bytes > 0 {
            "response_bytes"
        } else {
            "awaiting_data"
        };
        let events = if self.inspect_events {
            self.events.to_string()
        } else {
            "uninspected".into()
        };
        error.message = format!(
            "[image_stream stage={stage} bytes={} chunks={} events={events} handshake_events={} business_events={} image_events={} last_event={} completion_seen={}] {}",
            self.bytes, self.chunks, self.handshake_events, self.business_events,
            self.image_events, self.last_event, self.completion_seen, error.message,
        );
        error
    }
}

async fn into_image_stream(
    execution: &Execution,
    response: reqwest::Response,
) -> Result<BoxStream<'static, Result<Bytes, reqwest::Error>>, ServiceError> {
    if is_sse(response.headers()) {
        return Ok(response.bytes_stream().boxed());
    }
    let bytes = execution.read_bounded(response).await?;
    let response: Value = serde_json::from_slice(&bytes).map_err(|_| {
        ServiceError::new(
            502,
            "invalid_image_response",
            "Image Responses upstream returned invalid JSON",
        )
    })?;
    let kind = if response.get("error").is_some_and(|value| !value.is_null())
        || response
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status != "completed")
    {
        "response.failed"
    } else {
        "response.completed"
    };
    let bytes = Bytes::from(format!(
        "data: {}\n\n",
        json!({"type":kind,"response":response})
    ));
    Ok(stream::once(async move { Ok(bytes) }).boxed())
}

fn error_event(error: &ServiceError) -> Bytes {
    Bytes::from(format!(
        "event: error\ndata: {}\n\n",
        super::super::output_error::json_error(error)
    ))
}

/// Keep filtered upstream metadata while allowing the converted body to set its MIME.
pub(super) fn with_upstream_headers(mut response: Response, upstream: &HeaderMap) -> Response {
    let filtered = downstream_response(response.status(), upstream, Body::empty());
    for name in filtered.headers().keys() {
        if !response.headers().contains_key(name) {
            for value in filtered.headers().get_all(name) {
                response.headers_mut().append(name.clone(), value.clone());
            }
        }
    }
    response
}

async fn forward_oauth(mut execution: Execution) -> Result<Response, ServiceError> {
    let (upstream, binding) = match execution.send(true).await {
        Ok(value) => value,
        Err(error) => {
            execution.tracker.fail(&error).await;
            return Err(error);
        }
    };
    if !upstream.status().is_success() {
        let status = upstream.status();
        let headers = upstream.headers().clone();
        let bytes = match execution.read_bounded(upstream).await {
            Ok(bytes) => bytes,
            Err(error) => {
                execution.tracker.fail(&error).await;
                return Err(error);
            }
        };
        execution
            .tracker
            .complete(
                status.as_u16(),
                false,
                None,
                Some("image_upstream_error".into()),
            )
            .await;
        return Ok(downstream_response(status, &headers, Body::from(bytes)));
    }
    let upstream_headers = upstream.headers().clone();
    let accumulator = ImageAccumulator::new(
        execution.request.operation,
        execution.request.response_format,
    )
    .with_output_format(&execution.request.output_format)
    .with_limit(execution.response_limit());
    let stream = match into_image_stream(&execution, upstream).await {
        Ok(stream) => stream,
        Err(error) => {
            execution.tracker.fail(&error).await;
            return Err(error);
        }
    };
    let mut current = OAuthStream {
        decoder: SseDecoder::new(execution.event_limit()),
        upstream: Some(stream),
        binding,
        accumulator,
        execution,
        pending: VecDeque::new(),
        results: Vec::new(),
        finished: false,
        progress: ImageStreamProgress::new(true),
    };
    if !current.execution.request.stream {
        loop {
            match current.advance().await {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    current.execution.tracker.fail(&error).await;
                    return Err(error);
                }
            }
        }
        return Ok(with_upstream_headers(
            (
                StatusCode::OK,
                axum::Json(merge_responses(&current.results)?),
            )
                .into_response(),
            &upstream_headers,
        ));
    }
    let body = Body::from_stream(stream::unfold(current, |mut current| async move {
        loop {
            if let Some(bytes) = current.pending.pop_front() {
                return Some((Ok::<Bytes, std::io::Error>(bytes), current));
            }
            if current.finished {
                return None;
            }
            if let Err(error) = current.advance().await {
                current.finished = true;
                current.execution.tracker.fail(&error).await;
                current.pending.push_back(error_event(&error));
            }
        }
    }));
    Ok(with_upstream_headers(
        (
            [
                (header::CONTENT_TYPE, "text/event-stream"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        )
            .into_response(),
        &upstream_headers,
    ))
}

struct OAuthStream {
    execution: Execution,
    upstream: Option<BoxStream<'static, Result<Bytes, reqwest::Error>>>,
    decoder: SseDecoder,
    binding: Option<Binding>,
    accumulator: ImageAccumulator,
    pending: VecDeque<Bytes>,
    results: Vec<Value>,
    finished: bool,
    progress: ImageStreamProgress,
}

impl OAuthStream {
    async fn advance(&mut self) -> Result<bool, ServiceError> {
        if self.finished {
            return Ok(false);
        }
        if self.accumulator.terminal {
            self.results.push(self.accumulator.finish()?);
            self.execution.owner_committed = true;
            let size: usize = self
                .results
                .iter()
                .filter_map(|value| value.get("data").and_then(Value::as_array))
                .flatten()
                .filter_map(|value| {
                    value
                        .get("b64_json")
                        .or_else(|| value.get("url"))
                        .and_then(Value::as_str)
                })
                .map(str::len)
                .sum();
            if size > self.execution.response_limit() {
                return Err(ServiceError::new(
                    502,
                    "upstream_response_too_large",
                    "Combined image output exceeds its size limit",
                ));
            }
            let combined = merge_responses(&self.results)?;
            self.execution.usage = combined.get("usage").cloned();
            self.execution.tracker.capture_usage(&self.execution.usage);
            self.upstream = None;
            if self.results.len() >= self.execution.request.n as usize {
                self.execution
                    .tracker
                    .complete(200, true, combined.get("usage").cloned(), None)
                    .await;
                self.finished = true;
                return Ok(false);
            }
            // The next image is another opening operation on the same owner.
            // A long first generation may have consumed its opening deadline;
            // retain the request's total deadline and renew only that budget.
            self.execution.budget = self.execution.budget.renew_open();
            let (response, binding) = self.execution.send(true).await?;
            if !response.status().is_success() {
                return Err(ServiceError::new(
                    response.status().as_u16(),
                    "image_upstream_error",
                    "An image operation was rejected by upstream",
                ));
            }
            self.binding = binding;
            self.upstream = Some(into_image_stream(&self.execution, response).await?);
            self.decoder = SseDecoder::new(self.execution.event_limit());
            self.accumulator = ImageAccumulator::new(
                self.execution.request.operation,
                self.execution.request.response_format,
            )
            .with_output_format(&self.execution.request.output_format)
            .with_limit(self.execution.response_limit());
            self.progress = ImageStreamProgress::new(true);
        }
        let upstream = self.upstream.as_mut().ok_or_else(|| {
            ServiceError::new(
                502,
                "incomplete_upstream_stream",
                "Image stream is unavailable",
            )
        })?;
        let next = self
            .execution
            .read_next(upstream)
            .await
            .map_err(|error| self.progress.failure(error))?;
        let eof = next.is_none();
        let events = match next {
            Some(bytes) => {
                self.progress.chunk(bytes.len());
                self.execution.tracker.first_response();
                self.decoder.push(&bytes)?
            }
            None => self.decoder.finish()?,
        };
        for mut event in events {
            self.execution.tracker.quota_event(&event);
            self.progress.event(&event);
            if let Some(usage) = super::result::image_usage(event.get("response").unwrap_or(&event))
            {
                self.accumulator.usage = Some(usage.clone());
                let mut inputs: Vec<_> = self
                    .results
                    .iter()
                    .map(|value| json!({"data":[],"usage":value.get("usage")}))
                    .collect();
                inputs.push(json!({"data":[],"usage":usage}));
                self.execution.usage = merge_responses(&inputs)?.get("usage").cloned();
                self.execution.tracker.capture_usage(&self.execution.usage);
            }
            super::assets::normalize_responses_event(
                &self.execution.state,
                &mut event,
                self.execution.request.response_format,
                self.execution.asset_budget(),
            )
            .await?;
            if let Some(binding) = &self.binding {
                if matches!(
                    event.get("type").and_then(Value::as_str),
                    Some(
                        "response.metadata"
                            | "response.completed"
                            | "response.output_item.done"
                            | "response.done"
                    )
                ) {
                    let store = self.execution.state.identity.clone();
                    let binding = binding.clone();
                    let observed = event.clone();
                    blocking::run_tracked(&self.execution.state, move || {
                        store.observe_event(&binding, &observed)
                    })
                    .await?;
                }
            }
            let placeholder = ImageAccumulator::new(
                self.execution.request.operation,
                self.execution.request.response_format,
            )
            .with_output_format(&self.execution.request.output_format)
            .with_limit(self.execution.response_limit());
            let mut accumulator = std::mem::replace(&mut self.accumulator, placeholder);
            let (accumulator, converted) = blocking::run(
                self.execution.state.stop.clone(),
                self.execution.state.options.request_timeout,
                move || {
                    let converted = accumulator.observe(&event);
                    Ok((accumulator, converted))
                },
            )
            .await?;
            self.accumulator = accumulator;
            let mut usage_inputs: Vec<Value> = self
                .results
                .iter()
                .map(|value| json!({"data":[],"usage":value.get("usage")}))
                .collect();
            usage_inputs.push(json!({"data":[],"usage":self.accumulator.usage}));
            self.execution.usage = merge_responses(&usage_inputs)?.get("usage").cloned();
            self.execution.tracker.capture_usage(&self.execution.usage);
            for mut output in converted? {
                let offset: usize = self
                    .results
                    .iter()
                    .filter_map(|value| value.get("data").and_then(Value::as_array))
                    .map(Vec::len)
                    .sum();
                if let Some(index) = output.get("output_index").and_then(Value::as_u64) {
                    output["output_index"] = json!(offset as u64 + index);
                }
                let kind = output
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("image_generation.completed");
                if self.execution.request.stream {
                    self.pending
                        .push_back(Bytes::from(format!("event: {kind}\ndata: {output}\n\n")));
                }
            }
        }
        if eof && !self.accumulator.terminal {
            return Err(self.progress.failure(ServiceError::new(
                502,
                "incomplete_upstream_stream",
                "Image stream ended before completion",
            )));
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "images_stream_diagnostics_tests.rs"]
mod stream_diagnostics_tests;
