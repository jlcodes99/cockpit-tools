//! Native standalone search, audio and Realtime transport. These endpoints
//! use operation-specific account admission and keep call continuations pinned.
use super::{
    contract::{Route, RouteRequest, ServiceError},
    lifecycle::{stopping_error, RequestTracker},
    server::{downstream_response, outbound_headers, AppState},
};
use axum::{
    body::{to_bytes, Body},
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Request, State,
    },
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::{stream, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

const AUDIO_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
#[path = "auxiliary_audio.rs"]
mod audio;
#[path = "auxiliary_state.rs"]
mod bindings;
#[path = "auxiliary_realtime.rs"]
mod realtime;
#[cfg(test)]
#[path = "auxiliary_tests.rs"]
mod tests;

pub(super) async fn http(State(state): State<Arc<AppState>>, request: Request) -> Response {
    handle(state, request)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}
async fn handle(state: Arc<AppState>, request: Request) -> Result<Response, ServiceError> {
    let mut tracker = Some(state.begin()?);
    let result = handle_tracked(state, request, &mut tracker).await;
    if let (Err(error), Some(tracker)) = (&result, tracker.as_mut()) {
        tracker.fail(error).await;
    }
    result
}

async fn handle_tracked(
    state: Arc<AppState>,
    request: Request,
    tracker_slot: &mut Option<RequestTracker>,
) -> Result<Response, ServiceError> {
    let tracker = tracker_slot
        .as_mut()
        .expect("HTTP request tracker is present");
    let path = request.uri().path().to_owned();
    let audio_deadline = is_audio(&path).then(|| Instant::now() + AUDIO_REQUEST_TIMEOUT);
    let allow_secret = path.contains("realtime") || path.starts_with("/v1/live");
    tracker.context(request.method().as_str(), &path, request.headers(), false);
    let auth = bindings::authorize(&state, request.headers(), allow_secret).await?;
    tracker.authorized(&auth.caller, "");
    if path.contains("/translations") && path.contains("realtime")
        || path.ends_with("/transcription_sessions")
        || ["/accept", "/reject", "/refer"]
            .iter()
            .any(|s| path.ends_with(s))
    {
        return Err(ServiceError::new(
            501,
            "realtime_capability_not_supported",
            "This Realtime capability is unavailable on the Codex upstream",
        ));
    }
    let headers = request.headers().clone();
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_owned();
    let mut stop = state.stop.clone();
    let bytes = tokio::select! {
        _=stop.wait()=>return Err(stopping_error()),
        result=tokio::time::timeout_at(audio_deadline.unwrap_or_else(|| Instant::now() + state.options.request_timeout),to_bytes(request.into_body(),if is_audio(&path) {state.options.max_upload_bytes.min(26 * 1024 * 1024)} else {state.options.max_body_bytes}))=>result.map_err(|_|ServiceError::new(408,"request_body_timeout","Request body timed out"))?.map_err(super::request_body::read_error)?,
    };
    let bytes = super::http::decode_body_with_limit(
        &state,
        bytes,
        &headers,
        if is_audio(&path) {
            state.options.max_upload_bytes.min(26 * 1024 * 1024)
        } else {
            state.options.max_body_bytes
        },
    )
    .await?
    .into();
    let policy = super::retry::execution_policy(&state).await?;
    let execution = super::execution::Execution::new(tracker, &policy);
    let audio_deadline = audio_deadline.map(|deadline| execution.total_deadline.cap(deadline));
    execution
        .run(handle_prepared(
            state,
            tracker_slot,
            path,
            auth,
            headers,
            mime,
            bytes,
            audio_deadline,
        ))
        .await
}

async fn handle_prepared(
    state: Arc<AppState>,
    tracker_slot: &mut Option<RequestTracker>,
    path: String,
    auth: bindings::AuxiliaryAuth,
    headers: HeaderMap,
    mime: String,
    bytes: axum::body::Bytes,
    audio_deadline: Option<Instant>,
) -> Result<Response, ServiceError> {
    let tracker = tracker_slot
        .as_mut()
        .expect("HTTP request tracker is present");
    if path.ends_with("/client_secrets") || path.ends_with("/sessions") {
        if auth.session.is_some() {
            return Err(ServiceError::new(
                403,
                "realtime_client_secret_scope_mismatch",
                "Realtime credentials cannot create more credentials",
            ));
        }
        let body = if bytes.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&bytes).map_err(|_| invalid("Session must be JSON"))?
        };
        let model = body
            .pointer("/session/model")
            .or_else(|| body.get("model"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .unwrap_or("gpt-realtime");
        let route = select(&state, &auth, &headers, model, "realtime", None).await?;
        tracker.routed(&auth.caller, &route, model);
        let result =
            bindings::create_secret(&state, &auth, &body, path.ends_with("/sessions"), &route)?;
        tracker.complete(200, true, None, None).await;
        let mut response = Json(result).into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return Ok(response);
    }
    if is_audio(&path) {
        let audio =
            super::execution::prepare(&state, audio::AudioRequest::parse(&path, &mime, bytes))
                .await?;
        let route = select(
            &state,
            &auth,
            &headers,
            &audio.model,
            &format!("audio.{}", audio.operation),
            None,
        )
        .await?;
        tracker.routed(&auth.caller, &route, &audio.model);
        let url = if route.chatgpt {
            format!(
                "{}/transcribe",
                route
                    .base_url
                    .trim_end_matches('/')
                    .trim_end_matches("/codex")
            )
        } else {
            format!(
                "{}/audio/{}",
                route.base_url.trim_end_matches('/'),
                audio.operation
            )
        };
        let request = audio.apply(
            route
                .client
                .post(url)
                .headers(auth_headers(&headers, &route)?),
            route.chatgpt,
            &route.model,
        )?;
        let response = send_until(&state, request, audio_deadline, tracker).await?;
        let status = response.status();
        let response_headers = response.headers().clone();
        if route.chatgpt && !audio.native {
            let bytes = read_until_limited(&state, response, audio_deadline, if status.is_success() { 4 * 1024 * 1024 } else { usize::MAX }).await?;
            if !status.is_success() {
                capture_auxiliary_error(tracker, status.as_u16(), &response_headers, &bytes);
                tracker
                    .complete(
                        status.as_u16(),
                        false,
                        None,
                        Some("audio_upstream_error".into()),
                    )
                    .await;
                return Ok(downstream_response(
                    status,
                    &response_headers,
                    Body::from(bytes),
                ));
            }
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let Some(text) = value.get("text").and_then(Value::as_str) else {
                let error = ServiceError::new(
                    502,
                    "audio_invalid_upstream_response",
                    "Transcription response must be JSON with a text string",
                );
                return Err(error);
            };
            tracker
                .complete(status.as_u16(), true, value.get("usage").cloned(), None)
                .await;
            let (body, content_type) = if audio.format == "text" {
                (text.to_owned(), "text/plain; charset=utf-8")
            } else {
                (json!({"text":text}).to_string(), "application/json")
            };
            let mut response = downstream_response(status, &response_headers, Body::from(body));
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            return Ok(response);
        }
        return stream_response_until(
            state,
            response,
            tracker_slot
                .take()
                .expect("HTTP request tracker is present"),
            route,
            audio_deadline,
            usize::MAX,
        );
    }
    if path.ends_with("/alpha/search") {
        let mut body: Value =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Search requires JSON"))?;
        if !body.is_object() {
            return Err(invalid("Search body must be an object"));
        }
        let model = body["model"].as_str().unwrap_or("").trim().to_owned();
        let search_id = body["id"]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let mut identity_headers = headers.clone();
        if let Some(search_id) = &search_id {
            identity_headers.insert(
                "session-id",
                HeaderValue::from_str(search_id).map_err(|_| invalid("Invalid search id"))?,
            );
        }
        let route = select(&state, &auth, &identity_headers, &model, "search", None).await?;
        tracker.routed(&auth.caller, &route, &model);
        let object = body.as_object_mut().expect("Search body checked above");
        object.remove("prompt_cache_key");
        object.remove("prompt_cache_retention");
        let mapped = rewrite(&state, &auth, &route, &identity_headers, &mut body).await?;
        if search_id.is_some() {
            body["id"] = json!(mapped
                .get("session-id")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| invalid("Invalid search identity"))?);
        }
        let response = send(
            &state,
            route
                .client
                .post(format!(
                    "{}/alpha/search",
                    route.base_url.trim_end_matches('/')
                ))
                .headers(mapped)
                .json(&body),
            tracker,
        )
        .await?;
        return stream_response(
            state,
            response,
            tracker_slot
                .take()
                .expect("HTTP request tracker is present"),
            route,
        );
    }
    if path.ends_with("/hangup") {
        let id = path
            .trim_end_matches("/hangup")
            .rsplit('/')
            .next()
            .unwrap_or("");
        let binding = bindings::call(&state, &auth, id)?;
        let route = select(
            &state,
            &auth,
            &headers,
            &binding.model,
            "realtime",
            Some(&binding.account_id),
        )
        .await?;
        if route.owner_id != binding.owner_id {
            return Err(ServiceError::new(
                409,
                "realtime_owner_changed",
                "Realtime credential owner has changed",
            ));
        }
        tracker.routed(&auth.caller, &route, &binding.model);
        let response = send(
            &state,
            route
                .client
                .post(realtime::hangup_url(&route, id))
                .headers(auth_headers(&headers, &route)?)
                .body(bytes),
            tracker,
        )
        .await?;
        if response.status().is_success() {
            bindings::remove_call(&state, &auth, id);
        }
        return stream_response(
            state,
            response,
            tracker_slot
                .take()
                .expect("HTTP request tracker is present"),
            route,
        );
    }
    let mut body =
        super::execution::prepare(&state, audio::call_body(&mime, bytes, auth.session.clone()))
            .await?;
    let sdp = body["sdp"]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| invalid("A nonempty SDP offer is required"))?
        .to_owned();
    if body.get("session").is_none() {
        body["session"] = json!({});
    }
    if !body["session"].is_object() {
        return Err(invalid("Call session must be an object"));
    }
    if body["session"]
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "realtime")
    {
        return Err(ServiceError::new(
            501,
            "realtime_capability_not_supported",
            "This realtime session type is not supported",
        ));
    }
    let model = body
        .pointer("/session/model")
        .or_else(|| body.get("model"))
        .and_then(Value::as_str)
        .unwrap_or("gpt-realtime")
        .to_owned();
    let route = select(&state, &auth, &headers, &model, "realtime", None).await?;
    tracker.routed(&auth.caller, &route, &model);
    body["session"]["model"] = json!(upstream_live_model(&model, &route));
    body.as_object_mut().unwrap().remove("model");
    // Official AVAS calls remove the client session id from the session object.
    body["session"].as_object_mut().unwrap().remove("id");
    body["sdp"] = json!(sdp.clone());
    let mapped = rewrite(&state, &auth, &route, &headers, &mut body).await?;
    let call_path = if !route.chatgpt && path.ends_with("/live") {
        "live"
    } else {
        "realtime/calls?intent=quicksilver&architecture=avas"
    };
    let request = route
        .client
        .post(format!(
            "{}/{}",
            route.base_url.trim_end_matches('/'),
            call_path
        ))
        .headers(mapped);
    let request = if route.chatgpt {
        request.json(&body)
    } else {
        request.multipart(
            reqwest::multipart::Form::new()
                .text("sdp", sdp)
                .text("session", body["session"].to_string()),
        )
    };
    let response = send(&state, request, tracker).await?;
    let status = response.status();
    let mut response_headers = response.headers().clone();
    let bytes = read(&state, response).await?;
    if !status.is_success() {
        capture_auxiliary_error(tracker, status.as_u16(), &response_headers, &bytes);
    }
    if status.is_success() {
        let id = response_headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .and_then(call_id_from_location)
            .or_else(|| {
                serde_json::from_slice::<Value>(&bytes).ok().and_then(|v| {
                    v["call_id"]
                        .as_str()
                        .filter(|id| bindings::valid_call_id(id))
                        .map(str::to_string)
                })
            });
        let id = id
            .ok_or_else(|| {
                ServiceError::new(
                    502,
                    "invalid_call_id",
                    "Realtime upstream omitted the call identifier",
                )
            })?
            .to_owned();
        bindings::bind_call(&state, &auth, &route, &id, &model)?;
        response_headers.insert(
            header::LOCATION,
            HeaderValue::from_str(&local_call_location(
                response_headers
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default(),
                &path,
                &id,
            ))
            .map_err(|_| invalid("Invalid call identifier"))?,
        );
    }
    tracker.first_response();
    tracker
        .complete(
            status.as_u16(),
            status.is_success(),
            None,
            (!status.is_success()).then(|| "realtime_upstream_error".into()),
        )
        .await;
    Ok(downstream_response(
        status,
        &response_headers,
        Body::from(bytes),
    ))
}

pub(super) async fn websocket(
    State(state): State<Arc<AppState>>,
    request: Request,
) -> Result<Response, ServiceError> {
    let mut tracker = Some(state.begin()?);
    let result = websocket_tracked(state, request, &mut tracker).await;
    if let (Err(error), Some(tracker)) = (&result, tracker.as_mut()) {
        tracker.fail(error).await;
    }
    result
}

async fn websocket_tracked(
    state: Arc<AppState>,
    request: Request,
    tracker_slot: &mut Option<RequestTracker>,
) -> Result<Response, ServiceError> {
    let tracker = tracker_slot
        .as_mut()
        .expect("WebSocket request tracker is present");
    tracker.context("GET", request.uri().path(), request.headers(), false);
    let auth = bindings::authorize(&state, request.headers(), true).await?;
    tracker.authorized(&auth.caller, "");
    let headers = request.headers().clone();
    let query: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let path = request.uri().path().to_owned();
    let call_id = query.get("call_id").cloned().or_else(|| {
        path.strip_prefix("/v1/live/")
            .or_else(|| path.strip_prefix("/v1/realtime/calls/"))
            .or_else(|| path.strip_prefix("/backend-api/codex/realtime/calls/"))
            .or_else(|| path.strip_prefix("/backend-api/codex/"))
            .map(str::to_owned)
    });
    let binding = call_id
        .as_ref()
        .map(|id| bindings::call(&state, &auth, id))
        .transpose()?;
    let connection = call_id
        .as_deref()
        .map(|id| bindings::connect_call(&state, &auth, id))
        .transpose()?;
    let model = binding
        .as_ref()
        .map(|b| b.model.clone())
        .or_else(|| query.get("model").cloned())
        .or_else(|| {
            auth.session
                .as_ref()
                .and_then(|s| s["model"].as_str().map(str::to_owned))
        })
        .unwrap_or_else(|| "gpt-realtime".into());
    let policy = super::retry::execution_policy(&state).await?;
    let execution = super::execution::Execution::new(tracker, &policy);
    let prepare_deadline = execution.prepare_deadline();
    execution
        .run(async move {
            let route = select(
                &state,
                &auth,
                &headers,
                &model,
                "realtime",
                binding.as_ref().map(|b| b.account_id.as_str()),
            )
            .await?;
            if binding
                .as_ref()
                .is_some_and(|b| b.owner_id != route.owner_id)
            {
                return Err(ServiceError::new(
                    409,
                    "realtime_owner_changed",
                    "Realtime credential owner has changed",
                ));
            }
            tracker_slot
                .as_mut()
                .expect("WebSocket request tracker is present")
                .routed(&auth.caller, &route, &model);
            let mut body = json!({});
            let mut mapped = rewrite(&state, &auth, &route, &headers, &mut body).await?;
            mapped.remove("sec-websocket-key");
            mapped.remove("sec-websocket-extensions");
            mapped.remove("sec-websocket-protocol");
            let protocols = realtime::offered_protocols(&headers);
            if !protocols.is_empty() {
                mapped.insert(
                    header::SEC_WEBSOCKET_PROTOCOL,
                    HeaderValue::from_str(&protocols.join(", "))
                        .map_err(|_| invalid("Invalid Realtime subprotocol"))?,
                );
            }
            let url = realtime_websocket_url(&route, &path, call_id.as_deref(), &model)?;
            let (mut parts, _) = request.into_parts();
            let ws = WebSocketUpgrade::from_request_parts(&mut parts, &state)
                .await
                .map_err(|_| invalid("A WebSocket upgrade is required"))?;
            let remaining_open = prepare_deadline.saturating_duration_since(Instant::now());
            let tracker = tracker_slot
                .as_mut()
                .expect("WebSocket request tracker is present");
            let connected = super::execution::prepare(
                &state,
                super::transport::connect_websocket_tracked(
                    &route,
                    url.as_str(),
                    mapped,
                    state.options.max_body_bytes,
                    remaining_open.min(policy.websocket_connect_timeout),
                    state.stop.clone(),
                    |headers| {
                        tracker.begin_attempt("websocket");
                        tracker.upstream_headers(headers);
                    },
                ),
            )
            .await;
            let mut upstream = match connected {
                Ok(upstream) => upstream,
                Err(error) => {
                    if error.upstream_not_sent { tracker.discard_unsent_attempt(); }
                    return Err(error);
                }
            };
            tracker.upstream_response_headers(&upstream.1);
            let selected_protocol = realtime::selected_protocol(&protocols, &upstream.1)?;
            let ws = if let Some(protocol) = selected_protocol {
                ws.protocols([protocol])
            } else {
                ws
            };
            if let Some(mut session) = auth.session.clone() {
                let object = session
                    .as_object_mut()
                    .ok_or_else(|| invalid("Realtime session must be an object"))?;
                for field in ["model", "id", "object", "expires_at", "client_secret"] {
                    object.remove(field);
                }
                // Apply the secret's preset before accepting client frames, as
                // the Go direct Realtime handler does. Routing stays owner-pinned.
                let event = json!({"type":"session.update","session":session});
                super::execution::prepare(
                    &state,
                    timed(
                        &state,
                        upstream
                            .0
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                event.to_string().into(),
                            )),
                    ),
                )
                .await?;
            }
            let tracker = tracker_slot
                .take()
                .expect("WebSocket request tracker is present");
            Ok(ws
                .max_message_size(state.options.max_body_bytes)
                .max_frame_size(state.options.max_body_bytes)
                .on_upgrade(move |socket| {
                    relay(
                        state, route, auth, model, connection, socket, upstream.0, tracker,
                    )
                }))
        })
        .await
}

async fn relay(
    state: Arc<AppState>,
    route: Route,
    auth: bindings::AuxiliaryAuth,
    model: String,
    connection: Option<bindings::CallConnection>,
    mut client: WebSocket,
    mut upstream: tokio_tungstenite::WebSocketStream<reqwest::Upgraded>,
    mut tracker: RequestTracker,
) {
    use tokio_tungstenite::tungstenite::Message as U;
    let mut call_stop = connection.as_ref().map(|connection| connection.stop());
    let _connection = connection.map(bindings::CallConnection::activate);
    let mut usage = None;
    let mut stop = state.stop.clone();
    let result=async {loop {tokio::select!{
        _=stop.wait()=>return Err(stopping_error()),
        _=async { match call_stop.as_mut() { Some(receiver) => { if !*receiver.borrow() { let _ = receiver.changed().await; } }, None => std::future::pending::<()>().await } }=>return Ok(()),
        next=tokio::time::timeout(state.options.idle_timeout,client.recv())=>match next {
            Ok(Some(Ok(Message::Text(text))))=>{let mut value:Value=serde_json::from_str(&text).map_err(|_|invalid("Realtime messages must be JSON"))?;rewrite_realtime_message(&state, &mut value, &auth, &route, &model).await?;timed(&state,upstream.send(U::Text(value.to_string().into()))).await?;},
            Ok(Some(Ok(Message::Binary(bytes))))=>{
                // JSON carried in Binary frames has the same caller/model scope
                // as Text. Opaque frames retain their bytes and the upgrade's limit.
                let bytes=if let Ok(mut value)=serde_json::from_slice::<Value>(&bytes){
                    rewrite_realtime_message(&state,&mut value,&auth,&route,&model).await?;
                    value.to_string().into_bytes().into()
                }else{bytes};
                timed(&state,upstream.send(U::Binary(bytes))).await?;
            },
            Ok(Some(Ok(Message::Ping(value))))=>timed(&state,client.send(Message::Pong(value))).await?,
            Ok(Some(Ok(Message::Pong(_))))=>{},
            Ok(None)|Ok(Some(Ok(Message::Close(_))))=>return Ok(()),
            _=>return Err(ServiceError::new(502,"realtime_client_disconnected","Realtime client disconnected")),
        },
        next=tokio::time::timeout(state.options.idle_timeout,upstream.next())=>match next {
            Ok(Some(Ok(U::Text(text))))=>{tracker.first_response();if let Ok(value)=serde_json::from_str::<Value>(&text){if let Some(value)=value.pointer("/response/usage"){usage=Some(value.clone());tracker.capture_usage(&usage);}}timed(&state,client.send(Message::Text(text.to_string().into()))).await?;},
            Ok(Some(Ok(U::Binary(value))))=>{tracker.first_response();timed(&state,client.send(Message::Binary(value))).await?;},
            Ok(Some(Ok(U::Ping(value))))=>timed(&state,upstream.send(U::Pong(value))).await?,
            Ok(Some(Ok(U::Pong(_)|U::Frame(_))))=>{},
            Ok(None)|Ok(Some(Ok(U::Close(_))))=>return Ok(()),
            _=>return Err(ServiceError::new(502,"realtime_upstream_disconnected","Realtime upstream disconnected")),
        }
    }}}.await;
    match result {
        Ok(()) => tracker.complete(200, true, usage, None).await,
        Err(error) => tracker.fail(&error).await,
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), upstream.close(None)).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), client.close()).await;
}
use axum::extract::FromRequestParts;
fn invalid(message: &str) -> ServiceError {
    ServiceError::new(400, "invalid_auxiliary_request", message)
}
fn is_audio(path: &str) -> bool {
    path.ends_with("/transcribe") || path.contains("/audio/")
}
fn upstream_live_model(model: &str, route: &Route) -> String {
    let normalized = model.trim().to_ascii_lowercase();
    if route.chatgpt
        && (normalized.is_empty()
            || normalized == "gpt-realtime"
            || normalized.starts_with("gpt-realtime-")
            || normalized.contains("realtime-preview"))
    {
        "gpt-live-1-codex".into()
    } else {
        route.model.clone()
    }
}
fn call_id_from_location(location: &str) -> Option<String> {
    let url = url::Url::parse(location)
        .or_else(|_| url::Url::parse(&format!("http://localhost{}", location)))
        .ok()?;
    if let Some((_, id)) = url.query_pairs().find(|(key, _)| key == "call_id") {
        return bindings::valid_call_id(&id).then(|| id.into_owned());
    }
    let id = url.path().rsplit('/').next()?;
    (bindings::valid_call_id(id) && !matches!(id, "calls" | "live" | "realtime" | "codex"))
        .then(|| id.to_owned())
}
fn local_call_location(upstream: &str, path: &str, id: &str) -> String {
    if upstream.contains("/live/") || path == "/v1/live" {
        format!("/v1/live/{id}")
    } else if path.starts_with("/backend-api/") {
        format!("/backend-api/codex/{id}")
    } else {
        format!("/v1/realtime/calls/{id}")
    }
}
fn realtime_websocket_url(
    route: &Route,
    path: &str,
    call_id: Option<&str>,
    model: &str,
) -> Result<url::Url, ServiceError> {
    realtime::websocket_url(route, path, call_id, model)
}
fn auth_headers(incoming: &HeaderMap, route: &Route) -> Result<HeaderMap, ServiceError> {
    let mut headers = outbound_headers(incoming);
    headers.remove(header::CONTENT_TYPE);
    // Realtime uses x-session-id; normalize it through the shared identity map.
    if let Some(value) = headers.remove("x-session-id") {
        if !headers.contains_key("session-id") {
            headers.insert("session-id", value);
        }
    }
    headers.extend(route.headers.clone());
    if !route.chatgpt && route.installation_profile.is_some() {
        // API-key audio is a provider-native endpoint. Its client is still
        // isolated by account, but no Codex installation is sent to it.
        headers.remove("x-codex-installation-id");
    } else if route.chatgpt {
        if let Some(profile) = &route.installation_profile {
            crate::modules::codex_api_identity::apply_headers(profile, &mut headers)
                .map_err(|message| ServiceError::new(400, "invalid_protocol_identity", message))?;
        }
    }
    // Auxiliary bodies have already been decoded or rebuilt as JSON/multipart.
    headers.remove(header::CONTENT_ENCODING);
    Ok(headers)
}
async fn select(
    state: &AppState,
    auth: &bindings::AuxiliaryAuth,
    headers: &HeaderMap,
    model: &str,
    operation: &str,
    preferred: Option<&str>,
) -> Result<Route, ServiceError> {
    let has_call_permit = preferred.is_some();
    let preferred = preferred.or(auth.account_id.as_deref());
    if auth
        .session
        .as_ref()
        .and_then(|s| s["model"].as_str())
        .is_some_and(|allowed| allowed != model)
    {
        return Err(ServiceError::new(
            403,
            "realtime_model_scope_mismatch",
            "Realtime credential cannot select a different model",
        ));
    }
    let target = headers
        .get("x-cockpit-target-account-id")
        .and_then(|v| v.to_str().ok());
    if preferred
        .zip(target)
        .is_some_and(|(expected, target)| expected != target)
    {
        return Err(ServiceError::new(
            403,
            "realtime_account_scope_mismatch",
            "Realtime credential cannot select another account",
        ));
    }
    let request = RouteRequest {
        model: model.into(),
        session_id: headers
            .get("session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        thread_id: headers
            .get("thread-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        parent_thread_id: None,
        preferred_account_id: preferred.map(str::to_owned),
        soft_preferred_account_id: None,
        target_account_id: headers
            .get("x-cockpit-target-account-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        force_refresh: false,
        image_generation_requested: false,
        image_input: false,
        excluded_account_ids: Vec::new(),
        requires_permit: !has_call_permit,
        required_capabilities: Vec::new(),
        capability_fallback_from: None,
        auxiliary: Some(operation.into()),
    };
    let route = super::execution::route(state, &auth.caller, request).await?;
    if route.chatgpt_plan {
        return Err(ServiceError::new(
            400,
            "subscription_sharing_unsupported_capability",
            "ChatGPT plan accounts support HTTP Responses inference only",
        ));
    }
    if preferred.is_some_and(|id| route.continuity_id() != id)
        || auth
            .owner_id
            .as_ref()
            .is_some_and(|owner| &route.owner_id != owner)
    {
        return Err(ServiceError::new(
            409,
            "realtime_owner_changed",
            "Realtime credential owner has changed",
        ));
    }
    Ok(route)
}
async fn rewrite(
    state: &AppState,
    auth: &bindings::AuxiliaryAuth,
    route: &Route,
    incoming: &HeaderMap,
    body: &mut Value,
) -> Result<HeaderMap, ServiceError> {
    let store = state.identity.clone();
    let caller = auth.caller.id.clone();
    let account = route.continuity_id().to_owned();
    let owner = route.owner_id.clone();
    let mut headers = auth_headers(incoming, route)?;
    let mut data = body.clone();
    let (mut headers, mut data) = super::execution::prepare(
        state,
        super::blocking::run_tracked(state, move || {
            let had_metadata = data.get("client_metadata").is_some();
            let context = store.inspect(&caller, &headers, &data)?;
            store.rewrite(&context, &account, &owner, &mut headers, &mut data)?;
            if let Some(session) = headers.get("session-id").cloned() {
                headers.insert("x-session-id", session);
            }
            if !had_metadata {
                data.as_object_mut()
                    .map(|object| object.remove("client_metadata"));
            }
            Ok((headers, data))
        }),
    )
    .await?;
    super::account_installation::apply(route, &mut headers, &mut data)?;
    *body = data;
    Ok(headers)
}
async fn send(
    state: &AppState,
    request: reqwest::RequestBuilder,
    tracker: &mut RequestTracker,
) -> Result<reqwest::Response, ServiceError> {
    send_until(state, request, None, tracker).await
}
async fn send_until(
    state: &AppState,
    request: reqwest::RequestBuilder,
    deadline: Option<Instant>,
    tracker: &mut RequestTracker,
) -> Result<reqwest::Response, ServiceError> {
    let deadline = scoped_deadline(deadline, true)
        .unwrap_or_else(|| Instant::now() + state.options.request_timeout);
    let mut stop = state.stop.clone();
    let (client, request) = request.build_split();
    let request = request.map_err(|_| {
        ServiceError::new(502, "upstream_unavailable", "Unable to connect to upstream")
            .with_upstream_not_sent(true)
    })?;
    let result = tokio::select! {
        biased;
        _=stop.wait()=>Err(stopping_error()),
        r=tokio::time::timeout_at(deadline,async {
            if Instant::now() >= deadline {
                return Err(ServiceError::new(504,"upstream_timeout","Upstream request timed out"));
            }
            tracker.begin_attempt("http");
            tracker.upstream_headers(request.headers());
            tracker.check_quota_before_send()?;
            client.execute(request).await.map_err(|error| {
                ServiceError::new(502,"upstream_unavailable","Unable to connect to upstream")
                    .with_upstream_not_sent(error.is_connect() || error.is_builder())
            })
        })=>r.map_err(|_|ServiceError::new(504,"upstream_timeout","Upstream request timed out"))?
    };
    let response = match result {
        Ok(response) => response,
        Err(error) => {
            if error.upstream_not_sent { tracker.discard_unsent_attempt(); }
            return Err(error);
        }
    };
    tracker.quota_headers(response.headers());
    tracker.upstream_response_headers(response.headers());
    Ok(response)
}
async fn read(state: &AppState, response: reqwest::Response) -> Result<Vec<u8>, ServiceError> {
    read_until(state, response, None).await
}
async fn read_until(
    state: &AppState,
    response: reqwest::Response,
    deadline: Option<Instant>,
) -> Result<Vec<u8>, ServiceError> {
    read_until_limited(state, response, deadline, 32 * 1024 * 1024).await
}
async fn read_until_limited(
    state: &AppState,
    response: reqwest::Response,
    deadline: Option<Instant>,
    limit: usize,
) -> Result<Vec<u8>, ServiceError> {
    let deadline = scoped_deadline(deadline, false);
    let mut source = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let mut stop = state.stop.clone();
        let next = tokio::select! {_=stop.wait()=>return Err(stopping_error()),v=timed_read(deadline,state.options.idle_timeout,source.next())=>v?};
        match next {
            Some(Ok(chunk)) => {
                if bytes.len().saturating_add(chunk.len()) > state.options.max_response_bytes.min(limit) {
                    return Err(ServiceError::new(
                        502,
                        "upstream_response_too_large",
                        "Upstream response exceeds its limit",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            Some(Err(_)) => {
                return Err(ServiceError::new(
                    502,
                    "upstream_stream_error",
                    "Upstream disconnected",
                ))
            }
            None => return Ok(bytes),
        }
    }
}
// Audio keeps the legacy two-minute request budget across every chunk. Idle
// traffic must not refresh the total deadline indefinitely.
async fn timed_read<T>(
    deadline: Option<Instant>,
    idle_timeout: Duration,
    future: impl std::future::Future<Output = T>,
) -> Result<T, ServiceError> {
    let idle_deadline = Instant::now() + idle_timeout;
    let deadline = deadline.map_or(idle_deadline, |deadline| deadline.min(idle_deadline));
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| ServiceError::new(504, "upstream_timeout", "Upstream response timed out"))
}

fn scoped_deadline(deadline: Option<Instant>, opening: bool) -> Option<Instant> {
    let configured = super::execution::current().and_then(|scope| {
        if opening {
            Some(scope.prepare_deadline())
        } else {
            scope.total_deadline.instant()
        }
    });
    match (deadline, configured) {
        (Some(original), Some(configured)) => Some(original.min(configured)),
        (original, configured) => original.or(configured),
    }
}
async fn timed<T, E>(
    state: &AppState,
    future: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, ServiceError> {
    let mut stop = state.stop.clone();
    tokio::select! {
        _ = stop.wait() => Err(stopping_error()),
        value = tokio::time::timeout(state.options.request_timeout, future) => value
            .map_err(|_| ServiceError::new(504, "realtime_write_timeout", "Realtime write timed out"))?
            .map_err(|_| ServiceError::new(502, "realtime_write_failed", "Realtime write failed")),
    }
}

async fn rewrite_realtime_message(
    state: &AppState,
    value: &mut Value,
    auth: &bindings::AuxiliaryAuth,
    route: &Route,
    model: &str,
) -> Result<(), ServiceError> {
    if !value.is_object() {
        return Err(invalid("Realtime messages must be objects"));
    }
    if let Some(session) = value.get_mut("session") {
        if !session.is_object() {
            return Err(invalid("Realtime session must be an object"));
        }
        if session
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "realtime")
        {
            return Err(ServiceError::new(
                501,
                "realtime_capability_not_supported",
                "This realtime session type is not supported",
            ));
        }
        if let Some(requested) = session.get("model") {
            if requested.as_str().is_none_or(|requested| {
                requested != model
                    && requested != route.model
                    && requested != upstream_live_model(model, route)
            }) {
                return Err(ServiceError::new(
                    403,
                    "realtime_model_scope_mismatch",
                    "An active Realtime connection cannot change model",
                ));
            }
            // Direct/sideband WebSockets preserve the authorized client model.
            // The Codex model mapping belongs only to WebRTC call bootstrap.
        }
        if let Some(id) = session.get("id").cloned() {
            let mut metadata = json!({"client_metadata":{"session_id":id}});
            let headers = rewrite(state, auth, route, &HeaderMap::new(), &mut metadata).await?;
            session["id"] = json!(headers
                .get("session-id")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| invalid("Invalid Realtime session id"))?);
        }
    }
    Ok(())
}

fn stream_response(
    state: Arc<AppState>,
    response: reqwest::Response,
    tracker: RequestTracker,
    route: Route,
) -> Result<Response, ServiceError> {
    stream_response_until(state, response, tracker, route, None, 32 * 1024 * 1024)
}
fn stream_response_until(
    state: Arc<AppState>,
    response: reqwest::Response,
    tracker: RequestTracker,
    route: Route,
    deadline: Option<Instant>,
    limit: usize,
) -> Result<Response, ServiceError> {
    // The downstream body is polled after the preparation task-local scope ends.
    // Capture its absolute total budget now, including time spent selecting an account.
    let deadline = scoped_deadline(deadline, false);
    let status = response.status();
    let headers = response.headers().clone();
    let error_headers = headers.clone();
    // Successful audio/SSE remains a zero-buffer forwarding path. Error bodies
    // use the same existing total response limit so classification needs no logs.
    let error_body = (!status.is_success()).then(Vec::<u8>::new);
    let source = response.bytes_stream().boxed();
    let body = Body::from_stream(stream::unfold(
        (
            state,
            source,
            tracker,
            route,
            0usize,
            false,
            error_body,
            error_headers,
        ),
        move |(
            state,
            mut source,
            mut tracker,
            route,
            mut received,
            finished,
            mut error_body,
            error_headers,
        )| async move {
            if finished {
                return None;
            }
            let mut stop = state.stop.clone();
            let next = tokio::select! {_=stop.wait()=>Err(stopping_error()),value=timed_read(deadline,state.options.idle_timeout,source.next())=>value};
            match next {
                Ok(Some(Ok(bytes))) => {
                    received = received.saturating_add(bytes.len());
                    if received > state.options.max_response_bytes.min(limit) {
                        let error = ServiceError::new(
                            502,
                            "upstream_response_too_large",
                            "Upstream response exceeds its limit",
                        );
                        tracker.fail(&error).await;
                        return Some((
                            Err(std::io::Error::other(error.message)),
                            (
                                state,
                                source,
                                tracker,
                                route,
                                received,
                                true,
                                error_body,
                                error_headers,
                            ),
                        ));
                    }
                    tracker.first_response();
                    if let Some(body) = &mut error_body {
                        body.extend_from_slice(&bytes);
                    }
                    Some((
                        Ok::<_, std::io::Error>(bytes),
                        (
                            state,
                            source,
                            tracker,
                            route,
                            received,
                            false,
                            error_body,
                            error_headers,
                        ),
                    ))
                }
                Ok(None) => {
                    if let Some(body) = error_body {
                        capture_auxiliary_error(
                            &mut tracker,
                            status.as_u16(),
                            &error_headers,
                            &body,
                        );
                    }
                    tracker
                        .complete(
                            status.as_u16(),
                            status.is_success(),
                            None,
                            (!status.is_success()).then(|| "upstream_http_error".into()),
                        )
                        .await;
                    None
                }
                failure => {
                    let error = match failure {
                        Err(error) => error,
                        _ => {
                            ServiceError::new(502, "upstream_stream_error", "Upstream disconnected")
                        }
                    };
                    tracker.fail(&error).await;
                    Some((
                        Err(std::io::Error::other(error.message)),
                        (
                            state,
                            source,
                            tracker,
                            route,
                            received,
                            true,
                            error_body,
                            error_headers,
                        ),
                    ))
                }
            }
        },
    ));
    Ok(downstream_response(status, &headers, body))
}

fn capture_auxiliary_error(
    tracker: &mut RequestTracker,
    status: u16,
    headers: &HeaderMap,
    bytes: &[u8],
) {
    let value = serde_json::from_slice::<Value>(bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()));
    tracker.capture_upstream_error(status, &value);
    let wait = match (
        super::events::retry_after_headers(headers),
        super::events::retry_after_event(&value),
    ) {
        (Some(headers), Some(body)) => Some(headers.max(body)),
        (headers, body) => headers.or(body),
    };
    tracker.set_retry_after(wait);
    if headers
        .get("cf-mitigated")
        .is_some_and(|value| value == "challenge")
    {
        tracker.capture_response_error(Some("cloudflare challenge"), false);
        tracker.capture_request_fault(false);
    }
}
