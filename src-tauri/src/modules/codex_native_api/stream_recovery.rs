//! One frozen native request may be resampled after published control metadata.
//! This is not response resumption. Never replay output or upstream hosted tools.
use super::{
    contract::{ExecutionPolicy, Route, ServiceError, WireApi, MIXED_MODEL_ROUTING_CALLER_ID},
    deadline::{self, Deadline},
    events::{SseDecoder, SseFrame},
    http::HttpResponseStream,
    lifecycle::{stopping_error, RequestTracker},
    mapping::Prepared,
    server::AppState,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub(super) struct Candidate {
    request: reqwest::Request,
    route: Route,
    captured: tokio::time::Instant,
    context: super::multi_agent_compat::Context,
    pub(super) raw_proof: Option<RawProof>,
}

/// Native namespace/plaintext restoration can hide partial raw frames behind
/// activity comments. Preserve proof from its original decoder, before restore.
#[derive(Clone)]
pub(super) struct RawProof(Arc<Mutex<RawState>>);

struct RawState {
    allowed: bool,
    boundary: bool,
    events: usize,
    bytes: usize,
    transport_failed: bool,
}

impl RawProof {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(RawState {
            allowed: true,
            boundary: false,
            events: 0,
            bytes: 0,
            transport_failed: false,
        })))
    }

    pub(super) fn observe(&self, frames: &[SseFrame], decoder: &SseDecoder, bytes: usize) {
        let Ok(mut proof) = self.0.lock() else {
            return;
        };
        proof.bytes = proof.bytes.saturating_add(bytes);
        proof.boundary = decoder.at_frame_boundary();
        for frame in frames {
            proof.allowed &= control_frame(frame);
            proof.events += usize::from(frame.value.is_some());
        }
        proof.allowed &= proof.bytes < super::bootstrap::MAX_BYTES
            && proof.events < super::bootstrap::MAX_EVENTS;
    }

    pub(super) fn deny(&self) {
        if let Ok(mut proof) = self.0.lock() {
            proof.allowed = false;
        }
    }

    pub(super) fn transport_failure(&self, error: &ServiceError) {
        if let Ok(mut proof) = self.0.lock() {
            proof.transport_failed =
                error.code == "upstream_stream_error" && error.transport_failure_kind().is_some();
        }
    }

    fn permits(&self) -> bool {
        self.0.lock().is_ok_and(|proof| {
            proof.allowed && proof.boundary && proof.events > 0 && proof.transport_failed
        })
    }
}

fn client_tool(tool: &Value, depth: usize) -> bool {
    let Some(fields) = tool.as_object() else {
        return false;
    };
    if depth > 8
        || !tool["name"]
            .as_str()
            .is_some_and(|name| !name.trim().is_empty())
        || tool
            .get("execution")
            .is_some_and(|v| !v.is_null() && v != "client")
        || ["namespace", "description"].iter().any(|key| {
            tool.get(*key)
                .is_some_and(|v| !v.is_null() && !v.is_string())
        })
        || ["strict", "defer_loading"].iter().any(|key| {
            tool.get(*key)
                .is_some_and(|v| !v.is_null() && !v.is_boolean())
        })
    {
        return false;
    }
    match tool["type"].as_str() {
        Some("function") => {
            fields.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "type"
                        | "name"
                        | "namespace"
                        | "description"
                        | "parameters"
                        | "strict"
                        | "defer_loading"
                        | "execution"
                )
            }) && tool
                .get("parameters")
                .is_none_or(|value| value.is_null() || value.is_object())
        }
        Some("custom") => {
            fields.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "type"
                        | "name"
                        | "namespace"
                        | "description"
                        | "format"
                        | "defer_loading"
                        | "execution"
                )
            }) && tool
                .get("format")
                .is_none_or(|value| value.is_null() || custom_format(value))
        }
        Some("namespace") => {
            fields
                .keys()
                .all(|key| matches!(key.as_str(), "type" | "name" | "description" | "tools"))
                && tool["tools"]
                    .as_array()
                    .is_some_and(|tools| tools.iter().all(|tool| client_tool(tool, depth + 1)))
        }
        _ => false,
    }
}

fn custom_format(value: &Value) -> bool {
    let Some(fields) = value.as_object() else {
        return false;
    };
    match value["type"].as_str() {
        Some("text") => fields.keys().all(|key| key == "type"),
        Some("grammar") => {
            fields
                .keys()
                .all(|key| matches!(key.as_str(), "type" | "syntax" | "definition"))
                && matches!(value["syntax"].as_str(), Some("lark" | "regex"))
                && value["definition"].is_string()
        }
        _ => false,
    }
}

fn historical_input(value: &Value) -> bool {
    if value.is_string() {
        return true;
    }
    let Some(items) = value.as_array() else {
        return false;
    };
    items.iter().all(|item| {
        if !item.is_object()
            || ["tools", "tool_choice", "input_extensions", "capabilities"]
                .iter()
                .any(|key| item.get(*key).is_some_and(|value| !value.is_null()))
        {
            return false;
        }
        match item["type"].as_str() {
            None => {
                !item.as_object().unwrap().contains_key("type")
                    && matches!(
                        item["role"].as_str(),
                        Some("user" | "assistant" | "developer" | "system")
                    )
            }
            Some("message") => matches!(
                item["role"].as_str(),
                Some("user" | "assistant" | "developer" | "system")
            ),
            Some(
                "function_call"
                | "custom_tool_call"
                | "function_call_output"
                | "custom_tool_call_output"
                | "reasoning"
                | "compaction",
            ) => true,
            // additional_tools and tool_search_output can carry new declarations.
            // Unknown execution-bearing input is not ordinary historical output.
            _ => false,
        }
    })
}

fn client_only(body: &Value) -> bool {
    if !body.is_object()
        || [
            "prompt",
            "conversation",
            "multi_agent",
            "previous_response_id",
            "input_extensions",
            "capabilities",
            "additional_tools",
            "tool_resources",
            "connectors",
        ]
        .iter()
        .any(|key| body.get(*key).is_some_and(|value| !value.is_null()))
        || body
            .get("background")
            .is_some_and(|value| !value.is_null() && value != false)
        || body
            .get("input")
            .is_some_and(|value| !historical_input(value))
    {
        return false;
    }
    let tools = body.get("tools");
    if !tools.is_none_or(|value| {
        value.is_null()
            || value
                .as_array()
                .is_some_and(|tools| tools.iter().all(|tool| client_tool(tool, 0)))
    }) {
        return false;
    }
    match body.get("tool_choice") {
        None | Some(Value::Null) => true,
        Some(Value::String(choice)) => matches!(choice.as_str(), "none" | "auto" | "required"),
        Some(choice) => choice.as_object().is_some_and(|fields| {
            fields
                .keys()
                .all(|key| matches!(key.as_str(), "type" | "name" | "namespace"))
                && matches!(choice["type"].as_str(), Some("function" | "custom"))
                && choice["name"].as_str().is_some_and(|name| !name.is_empty())
                && choice
                    .get("namespace")
                    .is_none_or(|value| value.is_null() || value.is_string())
        }),
    }
}

fn eligible(prepared: &Prepared, effective: &Value) -> bool {
    prepared.route.chatgpt
        && !prepared.route.chatgpt_plan
        && prepared.route.wire_api == WireApi::Responses
        && prepared.caller.id != MIXED_MODEL_ROUTING_CALLER_ID
        && !prepared.compact
        && prepared.raw_chat.is_none()
        && prepared.image_route_request.is_none()
        && !prepared.request.image_generation_requested
        && prepared.grok_context.is_none()
        && prepared.plan_tools_context.is_none()
        && !prepared.multi_agent_policy.options.orphan_delegation
        && !super::grok_errors::applies(&prepared.route)
        && !super::workbuddy_acp::is_route(&prepared.route)
        && prepared.request.auxiliary.is_none()
        && !prepared.source_headers.contains_key("x-codex-turn-state")
        && !prepared.headers.contains_key("x-codex-turn-state")
        && prepared.source_body["stream"] == true
        && effective["stream"] == true
        && client_only(&prepared.source_body)
        && client_only(effective)
}

impl Candidate {
    pub(super) fn capture(
        prepared: &Prepared,
        effective: &Value,
        request: &reqwest::Request,
    ) -> Option<Box<Self>> {
        if !eligible(prepared, effective) {
            return None;
        }
        let request = request.try_clone()?;
        request.body()?.as_bytes()?;
        let mut route = prepared.route.clone();
        // A frozen request must not retain a semaphore permit during refresh.
        route.permit = None;
        Some(Box::new(Self {
            request,
            route,
            captured: tokio::time::Instant::now(),
            context: prepared.multi_agent_context.clone(),
            raw_proof: prepared.multi_agent_context.is_active().then(RawProof::new),
        }))
    }

    fn same_route(&self, route: &Route) -> bool {
        let old = &self.route;
        old.account_id == route.account_id
            && old.continuity_id() == route.continuity_id()
            && old.owner_id == route.owner_id
            && old.base_url == route.base_url
            && old.model == route.model
            && old.wire_api == route.wire_api
            && old.chatgpt == route.chatgpt
            && old.chatgpt_plan == route.chatgpt_plan
            && old.headers == route.headers
            && old.proxy_url == route.proxy_url
            && old.proxy_route == route.proxy_route
            && old.official_authorization_revision == route.official_authorization_revision
            && old.official_registration_id == route.official_registration_id
            && old.official_fallback_registration_id == route.official_fallback_registration_id
            && old.official_fallback_authorization_revision
                == route.official_fallback_authorization_revision
            && old.authorization_scope_fingerprint == route.authorization_scope_fingerprint
            && old.installation_profile == route.installation_profile
            && old.default_service_tier == route.default_service_tier
    }

    /// Execute the exact immutable request once. Do not enter the generic send
    /// loop, compression fallback, schema fallback, or account retry machinery.
    pub(super) async fn reopen(
        mut self: Box<Self>,
        state: &Arc<AppState>,
        prepared: &mut Prepared,
        tracker: &mut RequestTracker,
        error: &ServiceError,
        policy: &ExecutionPolicy,
        total: Deadline,
    ) -> Result<HttpResponseStream, ServiceError> {
        if state.stop.stopped() {
            return Err(stopping_error());
        }
        let total = tracker.overload.deadline(total);
        if tokio::time::Instant::now() >= total {
            return Err(super::retry::total_timeout());
        }
        tracker.attempt_error(error);
        tracker.network_recovery.observe(error, policy);
        tracker.network_recovery.check()?;
        let recovery = tracker.network_recovery.clone();
        let mut stop = state.stop.clone();
        tokio::select! {
            biased;
            _ = stop.wait() => Err(stopping_error()),
            _ = deadline::sleep_until(total) => Err(super::retry::total_timeout()),
            _ = recovery.wait() => Err(recovery.error()),
            result = async {
                super::retry::delay(state, policy.retry_base_delay, policy.retry_max_delay, total).await?;
                super::retry::renew_transport_after_failure(state, prepared, tracker, error).await?;
                if !self.same_route(&prepared.route) || prepared.multi_agent_policy.options.orphan_delegation {
                    return Err(ServiceError::new(409, "transport_route_changed",
                        "Stream recovery cannot change the route or authorization of a frozen request"));
                }
                tracker.check_quota_before_send()?;
                if let Some(timeout) = self.request.timeout().copied() {
                    let remaining = timeout.saturating_sub(self.captured.elapsed());
                    if remaining.is_zero() { return Err(super::retry::total_timeout()); }
                    *self.request.timeout_mut() = Some(remaining);
                }
                let opening = super::request_upload::Opening::new(
                    total.cap(prepared.execution.prepare_deadline()), total, policy.open_timeout);
                let progress = super::request_upload::Opening::eligible(&self.request, true);
                tracker.begin_attempt("http");
                tracker.upstream_request_bytes(self.request.body().and_then(reqwest::Body::as_bytes).unwrap().len());
                tracker.upstream_headers(self.request.headers());
                tracker.quota_request_started();
                // All body sizes become noncloneable, so reqwest itself cannot
                // replay this dispatch on H2 NACK. Preserve its exact size/bytes.
                let request = single_dispatch(opening.track(self.request, true));
                let response = opening.run_if(progress, prepared.route.client.execute(request)).await
                    .map_err(|_| if tokio::time::Instant::now() >= total { super::retry::total_timeout() }
                        else { super::connection_error::timeout_with_request_bytes(&prepared.route, tracker.max_request_bytes()) })?
                    .map_err(|error| super::connection_error::request_with_request_bytes(
                        &prepared.route, &error, tracker.max_request_bytes()))?;
                let response = super::http_compression::decode_response(response)?;
                tracker.upstream_http_version(response.version());
                tracker.upstream_response_headers(response.headers());
                tracker.quota_headers(response.headers());
                let status = response.status();
                let headers = response.headers().clone();
                super::mapping::observe_headers(state, prepared, &headers).await?;
                let upstream = super::upstream::canonical_stream(state, response, &prepared.route, false,
                    None, None, self.context, &prepared.body, None, policy.idle_timeout, None).await?;
                if !status.is_success() {
                    let bytes = super::http::read_bounded(state, upstream.bytes, tracker, policy, total).await?;
                    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                    tracker.capture_upstream_error(status.as_u16(), &value);
                    return Err(super::output_error::event_failure(&value, status.as_u16())
                        .with_retry_after(super::events::retry_after_headers(&headers)));
                }
                if !upstream.is_sse {
                    return Err(ServiceError::new(502, "invalid_upstream_response",
                        "Stream recovery requires a Responses event stream"));
                }
                tracker.network_recovery.recovered();
                Ok(HttpResponseStream { status, headers, bytes: upstream.bytes, is_sse: true, canonical: None })
            } => result,
        }
    }
}

fn single_dispatch(mut request: reqwest::Request) -> reqwest::Request {
    let body = request.body_mut().take().expect("captured reusable body");
    *request.body_mut() = Some(reqwest::Body::wrap(body));
    request
}

#[derive(Default)]
pub(super) struct Fence {
    candidate: Option<Box<Candidate>>,
    events: usize,
    bytes: usize,
    published: bool,
}

impl Fence {
    pub(super) fn new(candidate: Option<Box<Candidate>>) -> Self {
        Self {
            candidate,
            ..Default::default()
        }
    }

    pub(super) fn observe(&mut self, frame: &SseFrame) {
        if self.candidate.is_none() {
            return;
        }
        self.bytes = self.bytes.saturating_add(frame.bytes.len());
        if self.bytes >= super::bootstrap::MAX_BYTES || !control_frame(frame) {
            self.candidate = None;
            return;
        }
        if frame.value.is_some() {
            self.events += 1;
            self.published = true;
            if self.events >= super::bootstrap::MAX_EVENTS {
                self.candidate = None;
            }
        }
    }

    pub(super) fn take(
        &mut self,
        error: &ServiceError,
        decoder: &SseDecoder,
        attempts: usize,
    ) -> Option<Box<Candidate>> {
        if error.code == "upstream_stream_error"
            && error.transport_failure_kind().is_some()
            && self.published
            && attempts == 1
            && decoder.at_frame_boundary()
            && self
                .candidate
                .as_ref()
                .is_some_and(|candidate| candidate.raw_proof.as_ref().is_none_or(RawProof::permits))
        {
            self.candidate.take()
        } else {
            self.candidate = None;
            None
        }
    }
}

fn control_frame(frame: &SseFrame) -> bool {
    if frame.identity_event.is_some() {
        return false;
    }
    let Some(value) = &frame.value else {
        return frame
            .bytes
            .split(|byte| *byte == b'\n')
            .all(|line| line.trim_ascii().is_empty() || line.trim_ascii_start().starts_with(b":"));
    };
    let Some(kind) = value["type"].as_str() else {
        return false;
    };
    if !matches!(
        kind,
        "response.created"
            | "response.queued"
            | "response.in_progress"
            | "keepalive"
            | "codex.rate_limits"
            | "codex.response.metadata"
            | "response.metadata"
            | "response.subscription_usage"
    ) || !super::bootstrap::pre_execution(value)
    {
        return false;
    }
    let mut declared = false;
    for line in frame.bytes.split(|byte| *byte == b'\n') {
        if let Some(event) = line.trim_ascii().strip_prefix(b"event:") {
            if declared || event.trim_ascii() != kind.as_bytes() {
                return false;
            }
            declared = true;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recovery_candidate_stays_off_the_inline_stream_state() {
        eprintln!("stream recovery layout Candidate={} Option<Candidate>={} Option<Box<Candidate>>={} Fence={}",
            std::mem::size_of::<Candidate>(), std::mem::size_of::<Option<Candidate>>(),
            std::mem::size_of::<Option<Box<Candidate>>>(), std::mem::size_of::<Fence>());
        assert_eq!(
            std::mem::size_of::<Option<Box<Candidate>>>(),
            std::mem::size_of::<usize>()
        );
        assert!(
            std::mem::size_of::<Fence>() <= 64,
            "large recovery state must be heap-owned"
        );
    }

    #[test]
    fn raw_proof_requires_the_source_failure_and_the_entire_batch_boundary() {
        let created = "data: {\"type\":\"response.created\",\"response\":{\"output\":[]}}\n\n";
        let mut error = ServiceError::new(502, "upstream_stream_error", "synthetic disconnect");
        error.transport_failure = Some(super::super::contract::TransportFailureKind::Network);
        for (tail, allowed) in [
            ("", true),
            (
                "data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{",
                false,
            ),
            (
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"business\"}\n\n",
                false,
            ),
        ] {
            let bytes = format!("{created}{tail}");
            let mut decoder = SseDecoder::new(1024 * 1024);
            let frames = decoder.push_frames(bytes.as_bytes()).unwrap();
            let proof = RawProof::new();
            proof.observe(&frames, &decoder, bytes.len());
            assert!(
                !proof.permits(),
                "metadata alone never certifies a source failure"
            );
            proof.transport_failure(&error);
            assert_eq!(proof.permits(), allowed, "{tail}");
            proof.deny();
            assert!(!proof.permits());
        }
    }

    #[tokio::test]
    async fn replay_body_cannot_be_cloned_by_the_http_client() {
        use axum::body::HttpBody;
        let request = reqwest::Client::new()
            .post("http://127.0.0.1/fixture")
            .body("exact synthetic body")
            .build()
            .unwrap();
        assert!(request.try_clone().is_some());
        let mut request = single_dispatch(request);
        assert!(request.try_clone().is_none());
        let body = request.body_mut().take().unwrap();
        assert_eq!(body.size_hint().exact(), Some(20));
        let bytes = axum::body::to_bytes(axum::body::Body::new(body), 1024)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"exact synthetic body");
    }

    #[test]
    fn client_execution_admission_fails_closed() {
        assert!(client_only(
            &json!({"tools":[{"type":"namespace","name":"n","tools":[{"type":"function","name":"f"},{"type":"custom","name":"c"}]}]})
        ));
        for body in [
            json!({"tools":{}}),
            json!({"tools":[{"type":"unknown","name":"x"}]}),
            json!({"tools":[{"type":"function","name":"f","execution":"server"}]}),
            json!({"tool_choice":{"type":"allowed_tools","tools":[{"type":"web_search"}]}}),
            json!({"prompt":{"id":"hidden_tools"}}),
            json!({"background":true}),
            json!({"tools":[{"type":"function","name":"f","parameters":42}]}),
            json!({"tools":[{"type":"function","name":"f","strict":"yes"}]}),
            json!({"tools":[{"type":"custom","name":"f","format":42}]}),
            json!({"tools":[{"type":"custom","name":"f","defer_loading":"server"}]}),
            json!({"tool_choice":{"type":"function","name":"f","namespace":{}}}),
            json!({"input":[{"type":"additional_tools","tools":[{"type":"mcp"}]}]}),
            json!({"input":[{"type":"tool_search_output","output":{"tools":[{"type":"mcp"}]}}]}),
            json!({"input_extensions":{"tools":[{"type":"mcp"}]}}),
            json!({"previous_response_id":"unfinished"}),
        ] {
            assert!(!client_only(&body), "{body}");
        }
        assert!(
            client_only(
                &json!({"input":[{"type":"function_call_output","call_id":"c",
            "output":{"type":"additional_tools","tools":[{"type":"mcp"}]}}]})
            ),
            "opaque historical tool output is data, not a new declaration"
        );
    }

    #[test]
    fn only_complete_matching_control_frames_pass() {
        for bytes in [
            "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.created\",\"response\":{\"output\":[]}}\n\n",
            "event: response.output_item.added\n\n",
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"arguments\":\"\"}}\n\n",
        ] {
            let mut decoder = SseDecoder::new(1024 * 1024);
            let frames = decoder.push_frames(bytes.as_bytes()).unwrap();
            assert!(!frames.is_empty());
            assert!(frames.iter().any(|frame| !control_frame(frame)), "{bytes}");
        }
        let event = b"event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{\"}\n\n";
        for split in 1..event.len() {
            let mut decoder = SseDecoder::new(1024 * 1024);
            let frames = decoder.push_frames(&event[..split]).unwrap();
            assert!(
                !decoder.at_frame_boundary() || frames.iter().any(|frame| !control_frame(frame)),
                "split {split}"
            );
        }
    }
}
