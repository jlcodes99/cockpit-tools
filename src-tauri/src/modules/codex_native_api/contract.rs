//! Host boundary for the native Responses service. No transport or legacy gateway logic lives here.
use std::{sync::Arc, time::Duration};

use axum::http::HeaderMap;
use futures_util::future::BoxFuture;
use serde_json::Value;

pub use super::multi_agent_compat::{ModelProfile, Options as MultiAgentOptions};

#[derive(Clone, Debug, Default)]
pub struct MultiAgentPolicy {
    pub options: MultiAgentOptions,
    pub models: Vec<ModelProfile>,
}

#[derive(Clone, Debug)]
pub struct Caller {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct RouteRequest {
    pub model: String,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub preferred_account_id: Option<String>,
    pub soft_preferred_account_id: Option<String>,
    /// Host-only target; public callers must not acquire broader scope through this header.
    pub target_account_id: Option<String>,
    pub force_refresh: bool,
    pub image_generation_requested: bool,
    /// Actual image content in this turn/history, independent of image-generation tools.
    pub image_input: bool,
    /// Accounts already attempted for this request, never the whole session.
    pub excluded_account_ids: Vec<String>,
    pub requires_permit: bool,
    /// Named non-Responses operation; preserves model ACLs while selecting compatible credentials.
    pub auxiliary: Option<String>,
    /// Effective request features before provider-specific normalization.
    pub required_capabilities: Vec<String>,
    /// Select only explicitly configured backups; never relax caller/model scope.
    pub capability_fallback_from: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CapabilityFallbackDecision {
    pub enabled: bool,
    pub unsupported: Vec<String>,
}

/// Validated client routing identities before account-specific upstream rewriting.
#[derive(Clone, Debug, Default)]
pub struct RoutingIdentity {
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub parent_thread_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ExecutionPolicy {
    /// Internal host capability. The application always enables it; legacy
    /// capability injection remains available for migration and isolation tests.
    pub account_switch_rebuild: bool,
    pub open_timeout: Duration,
    pub idle_timeout: Duration,
    pub image_open_timeout: Duration,
    pub image_idle_timeout: Duration,
    /// Total opening attempts for a streaming request, including its first try.
    pub stream_open_max_attempts: u8,
    /// Zero matches Go: no overall deadline for ordinary requests.
    pub total_timeout: Duration,
    /// Maximum distinct credentials per selection round, including the first.
    /// Zero means no credential-count limit (pool, time and replay fences still apply).
    pub credential_retries: u16,
    pub retry_max_delay: Duration,
    pub send_retries: u8,
    pub send_retry_max_delay: Duration,
    pub retry_base_delay: Duration,
    pub status_retries: u8,
    pub status_retry_base_delay: Duration,
    pub status_retry_max_delay: Duration,
    pub bootstrap_retries: u8,
    pub keepalive: Duration,
    pub immediate_sse: bool,
    pub websocket_connect_timeout: Duration,
    pub websocket_initial_timeout: Duration,
    pub websocket_idle_timeout: Duration,
    pub websocket_heartbeat: Duration,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            account_switch_rebuild: true,
            open_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(180),
            image_open_timeout: Duration::from_secs(60),
            image_idle_timeout: Duration::from_secs(180),
            stream_open_max_attempts: 1,
            total_timeout: Duration::ZERO,
            credential_retries: 0,
            retry_max_delay: Duration::from_secs(1),
            send_retries: 0,
            send_retry_max_delay: Duration::from_millis(1200),
            retry_base_delay: Duration::from_millis(200),
            status_retries: 0,
            status_retry_base_delay: Duration::from_millis(300),
            status_retry_max_delay: Duration::from_millis(1500),
            bootstrap_retries: 0,
            keepalive: Duration::ZERO,
            immediate_sse: false,
            websocket_connect_timeout: Duration::from_secs(30),
            websocket_initial_timeout: Duration::from_secs(30),
            websocket_idle_timeout: Duration::from_secs(180),
            websocket_heartbeat: Duration::ZERO,
        }
    }
}

impl ExecutionPolicy {
    pub fn for_images(mut self) -> Self {
        self.open_timeout = self.image_open_timeout;
        self.idle_timeout = self.image_idle_timeout;
        self
    }
}

#[derive(Clone)]
pub struct Route {
    pub account_id: String,
    /// Credential namespace for continuation state; business membership remains account_id.
    pub continuity_account_id: Option<String>,
    /// Stable owner identity (including workspace), independent of token refreshes.
    pub owner_id: String,
    /// Stable installation identity; independent of continuation ownership.
    pub installation_profile: Option<crate::modules::codex_api_identity::InstallationProfile>,
    /// Opaque configuration/credential digest and client generation. Never sent upstream.
    pub transport_identity: Option<String>,
    /// SIWC authorization epoch used for this attempt; internal only.
    pub official_authorization_revision: Option<String>,
    pub official_registration_id: Option<String>,
    /// Persistent official binding which authorized this same-source local fallback.
    pub official_fallback_registration_id: Option<String>,
    pub official_fallback_authorization_revision: Option<String>,
    /// Opaque snapshot of the caller ACL and official route authorization.
    pub authorization_scope_fingerprint: Option<String>,
    pub official_fallback_failure: Option<OfficialFallbackFailure>,
    /// Responses base: e.g. https://chatgpt.com/backend-api/codex or https://api.openai.com/v1.
    pub base_url: String,
    pub model: String,
    pub headers: HeaderMap,
    pub client: reqwest::Client,
    pub proxy_url: Option<String>,
    /// Sanitized route diagnostics; never forwarded as an upstream header.
    pub proxy_route: Option<Value>,
    pub chatgpt: bool,
    /// Independent Sign in with ChatGPT plan grant; never a Codex backend route.
    pub chatgpt_plan: bool,
    /// Upstream wire protocol, independently of the protocol used by the caller.
    pub wire_api: WireApi,
    pub compact_endpoint: Option<EndpointOverride>,
    pub websocket_endpoint: Option<EndpointOverride>,
    pub default_service_tier: Option<String>,
    /// Held until the request or stream actually ends.
    pub permit: Option<Arc<dyn Send + Sync>>,
}

impl Route {
    pub fn continuity_id(&self) -> &str {
        self.continuity_account_id
            .as_deref()
            .unwrap_or(&self.account_id)
    }
}

#[derive(Clone)]
pub struct EndpointOverride {
    pub base_url: String,
    /// Destination-specific credentials; callers retain only sanitized request headers.
    pub headers: HeaderMap,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WireApi {
    #[default]
    Responses,
    Chat,
}

#[derive(Clone, Debug)]
pub struct ImageRouteRequest {
    pub model: Option<String>,
    /// Main Responses model, when image generation is requested through that API.
    pub responses_model: Option<String>,
    pub edit: bool,
    pub target_account_id: Option<String>,
    pub force_refresh: bool,
    pub preferred_account_id: Option<String>,
    /// Sources already attempted before any image output; never session-wide.
    pub excluded_account_ids: Vec<String>,
}

#[derive(Clone)]
pub struct ImageRoute {
    pub route: Route,
    pub image_model: String,
    pub main_model: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportFailureKind {
    TlsDecrypt,
}

#[derive(Clone, Debug)]
pub struct ServiceError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    /// Structured public upstream error details, sanitized at the output boundary.
    pub response_error: Option<Value>,
    pub retry_after: Option<Duration>,
    pub credential_scope: bool,
    /// Local, typed transport evidence; never inferred from an upstream message.
    pub transport_failure: Option<TransportFailureKind>,
    /// Proven local build/connect failure before an HTTP request reached upstream.
    /// Timeouts, read failures and WebSocket upgrade failures do not establish this.
    pub upstream_not_sent: bool,
}

impl ServiceError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            response_error: None,
            retry_after: None,
            credential_scope: false,
            transport_failure: None,
            upstream_not_sent: false,
        }
    }

    pub fn with_retry_after(mut self, delay: Option<Duration>) -> Self {
        self.retry_after = delay;
        self
    }

    pub fn with_error_details(mut self, error: Value) -> Self {
        self.response_error = Some(error);
        self
    }

    pub fn with_credential_scope(mut self, credential_scope: bool) -> Self {
        self.credential_scope = credential_scope;
        self
    }

    pub fn with_upstream_not_sent(mut self, upstream_not_sent: bool) -> Self {
        self.upstream_not_sent = upstream_not_sent;
        self
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServiceError {}

#[derive(Clone, Debug)]
pub struct RequestReport {
    /// One-way fingerprint of the actual SIWC access token for stale-error fencing.
    /// Never serialized into public logs or request payloads.
    pub official_access_fingerprint: Option<String>,
    pub official_authorization_revision: Option<String>,
    pub official_registration_id: Option<String>,
    /// Persistent official binding which authorized this same-source local fallback.
    pub official_fallback_registration_id: Option<String>,
    pub official_fallback_authorization_revision: Option<String>,
    /// Classified authorization evidence for this attempt, never public payload data.
    pub official_authorization_error: Option<String>,
    pub operation: &'static str,
    pub diagnostics: Option<super::diagnostics::DiagnosticContext>,
    pub caller: Caller,
    pub account_id: String,
    pub requested_model: String,
    pub upstream_model: String,
    pub status: u16,
    pub success: bool,
    pub elapsed: Duration,
    pub usage: Option<Value>,
    pub error: Option<String>,
    pub error_message: Option<String>,
    pub credential_scope: bool,
    pub retry_after: Option<Duration>,
    pub requested_service_tier: Option<String>,
    pub response_service_tier: Option<String>,
    pub reasoning_effort: Option<String>,
}

/// A proven pre-output official rejection, or a transport connection failure.
#[derive(Clone, Debug)]
pub struct OfficialFallbackFailure {
    pub trigger_stage: &'static str,
    pub status: u16,
    pub code: String,
    pub message: Option<String>,
    pub retry_after: Option<Duration>,
}

pub trait Host: Send + Sync + 'static {
    /// Retire only the failed client generation. Fast in-memory work only;
    /// existing streams keep their client clones and must not be cancelled.
    fn invalidate_transport(&self, _identity: &str) -> bool {
        false
    }
    /// Caller-visible model profiles and protocol-derived multi-agent policy.
    /// Before routing only boundary/tool preparation is requested; a selected
    /// route determines whether its protocol needs portable message input.
    fn multi_agent_policy<'a>(
        &'a self,
        _caller: &'a Caller,
        _route: Option<&'a Route>,
        _requested_model: &'a str,
    ) -> BoxFuture<'a, Result<MultiAgentPolicy, ServiceError>> {
        Box::pin(async { Ok(MultiAgentPolicy::default()) })
    }
    /// Fast in-memory observations only; implementations must not wait for disk or network.
    fn observe_rate_limits(
        &self,
        _account_id: &str,
        _snapshot: super::rate_limits::QuotaSnapshot,
        _request_started: std::time::Instant,
    ) {
    }
    fn quota_request_started(&self, _account_id: &str) {}
    fn check_quota_before_send(&self, _account_id: &str) -> Result<(), ServiceError> { Ok(()) }
    /// A runtime policy notification; must not perform blocking I/O.
    fn set_cooling_disabled(&self, _disabled: bool) {}
    /// Disabling affinity clears only the transient routing preference cache.
    fn set_session_affinity_enabled(&self, _enabled: bool) {}
    fn prepare_body<'a>(
        &'a self,
        _caller: &'a Caller,
        _route: &'a Route,
        _headers: &'a HeaderMap,
        body: Value,
        _dedicated_image: bool,
        _compact: bool,
    ) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async move { Ok(body) })
    }
    fn payload_logging_enabled(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }
    fn diagnostic(&self, _context: super::diagnostics::DiagnosticContext) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn management<'a>(
        &'a self,
        _caller: &'a Caller,
        _request: super::diagnostics::ManagementRequest,
    ) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async {
            Err(ServiceError::new(
                404,
                "not_found",
                "Management endpoint not configured",
            ))
        })
    }
    fn route_failed<'a>(
        &'a self,
        _account_id: &'a str,
        _status: u16,
        _retry_after: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn route_failed_for_model<'a>(
        &'a self,
        account_id: &'a str,
        _model: &'a str,
        status: u16,
        retry_after: Option<Duration>,
        _credential_scope: bool,
    ) -> BoxFuture<'a, ()> {
        self.route_failed(account_id, status, retry_after)
    }

    fn route_failed_with_context<'a>(
        &'a self,
        account_id: &'a str,
        model: &'a str,
        status: u16,
        retry_after: Option<Duration>,
        credential_scope: bool,
        _message: Option<&'a str>,
    ) -> BoxFuture<'a, ()> {
        self.route_failed_for_model(account_id, model, status, retry_after, credential_scope)
    }
    fn route_failed_attempt<'a>(
        &'a self,
        route: &'a Route,
        model: &'a str,
        status: u16,
        retry_after: Option<Duration>,
        credential_scope: bool,
        message: Option<&'a str>,
    ) -> BoxFuture<'a, ()> {
        self.route_failed_with_context(
            &route.account_id,
            model,
            status,
            retry_after,
            credential_scope,
            message,
        )
    }
    fn retry_cooldown_wait<'a>(
        &'a self,
        _caller: &'a Caller,
        _request: RouteRequest,
        _max_wait: Duration,
    ) -> BoxFuture<'a, Result<Option<Duration>, ServiceError>> {
        Box::pin(async { Ok(None) })
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _caller: &'a Caller,
        route: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async move { Ok(route.wire_api == WireApi::Responses) })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(ExecutionPolicy::default()) })
    }
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(true) })
    }
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>>;
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>>;
    /// Refresh credentials or transport inside the previous credential namespace.
    /// Hosts with request-local route overrides must preserve those overrides
    /// and recheck their authorization instead of applying the default selector.
    fn refresh_route<'a>(
        &'a self,
        caller: &'a Caller,
        _previous: &'a Route,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        self.route(caller, request)
    }
    /// Persist an observed upstream rejection before deciding whether history
    /// can be replayed. Candidate selection and local preflight are not attempts.
    fn observe_official_failure<'a>(
        &'a self,
        _route: &'a Route,
        _model: &'a str,
        _failure: &'a OfficialFallbackFailure,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    /// Resolve a same-source local route after an official failure. The host
    /// rechecks caller scope and the current authorization revision. Never
    /// mutate the durable official binding or select another source here.
    fn fallback_to_local<'a>(
        &'a self,
        _caller: &'a Caller,
        _failed: &'a Route,
        _request: RouteRequest,
        _failure: OfficialFallbackFailure,
    ) -> BoxFuture<'a, Result<Option<Route>, ServiceError>> {
        Box::pin(async { Ok(None) })
    }
    fn capability_fallback_decision<'a>(
        &'a self,
        _caller: &'a Caller,
        _route: &'a Route,
        _request: &'a RouteRequest,
    ) -> BoxFuture<'a, Result<CapabilityFallbackDecision, ServiceError>> {
        Box::pin(async { Ok(CapabilityFallbackDecision::default()) })
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>>;
    /// Local estimate preprocessing, after public model visibility was checked.
    /// This must not acquire an account permit or issue an upstream request.
    fn token_count_model<'a>(
        &'a self,
        _caller: &'a Caller,
        model: &'a str,
    ) -> BoxFuture<'a, Result<String, ServiceError>> {
        Box::pin(async move { Ok(model.to_owned()) })
    }
    fn image_route<'a>(
        &'a self,
        _caller: &'a Caller,
        _request: ImageRouteRequest,
    ) -> BoxFuture<'a, Result<ImageRoute, ServiceError>> {
        Box::pin(async {
            Err(ServiceError::new(
                503,
                "image_source_unavailable",
                "No image source is configured",
            ))
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()>;
}
