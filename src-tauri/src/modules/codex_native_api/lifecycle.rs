use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use tokio::sync::{oneshot, watch, Notify, OwnedSemaphorePermit};

use super::{
    contract::{Caller, Host, RequestReport, Route, ServiceError},
    diagnostics::{DiagnosticAttempt, DiagnosticContext, DiagnosticPayload},
};

#[derive(Clone)]
pub(super) struct StopSignal(pub watch::Receiver<bool>);

impl StopSignal {
    pub(super) fn stopped(&self) -> bool {
        *self.0.borrow()
    }

    pub(super) async fn wait(&mut self) {
        while !*self.0.borrow() {
            if self.0.changed().await.is_err() {
                return;
            }
        }
    }
}

pub(super) struct Activity {
    pub count: AtomicUsize,
    pub tasks: AtomicUsize,
    pub accepting: AtomicBool,
    pub changed: Notify,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            count: AtomicUsize::new(0),
            tasks: AtomicUsize::new(0),
            accepting: AtomicBool::new(true),
            changed: Notify::new(),
        }
    }
}

/// Tracks work whose blocking worker may outlive its cancelled async waiter.
/// A service must keep the handoff fence until that actual worker has stopped.
pub(super) struct BlockingWorkGuard(Arc<Activity>);

impl BlockingWorkGuard {
    pub(super) fn new(activity: Arc<Activity>) -> Self {
        activity.tasks.fetch_add(1, Ordering::SeqCst);
        Self(activity)
    }
}

impl Drop for BlockingWorkGuard {
    fn drop(&mut self) {
        self.0.tasks.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

pub(super) struct RequestTracker {
    activity: Arc<Activity>,
    host: Arc<dyn Host>,
    stop: StopSignal,
    started: Instant,
    busy: bool,
    report: Option<RequestReport>,
    pending_dispatch: Option<Box<PendingDispatch>>,
    dispatch_count: usize,
    dispatch_open: bool,
    dispatch_checkpoint: Option<Box<DispatchCheckpoint>>,
    installation_profile: Option<crate::modules::codex_api_identity::InstallationProfile>,
    transport_identity: Option<String>,
    diagnostics: Option<DiagnosticContext>,
    capture_payloads: bool,
    operation: &'static str,
    downstream: Option<oneshot::Receiver<DownstreamOutcome>>,
    global_permit: Option<OwnedSemaphorePermit>,
    capacity: Option<Arc<tokio::sync::Semaphore>>,
    route_permit: Option<Arc<dyn Send + Sync>>,
    rebuild_abort: Option<Arc<AtomicBool>>,
    quota_account: Option<String>,
    quota_started: Instant,
}

/// A selected candidate is not an upstream attempt until its transport starts.
/// Keep its identity separate from the last real response while preparing it.
struct PendingDispatch {
    report: RequestReport,
    attempt: DiagnosticAttempt,
    transport_identity: Option<String>,
    quota_account: Option<String>,
    proxy_route: Option<serde_json::Value>,
}

/// Only lightweight diagnostic positions are saved: request payload buffers
/// must not be copied whenever a candidate starts connecting.
struct DispatchCheckpoint {
    report: Option<RequestReport>,
    installation_profile: Option<crate::modules::codex_api_identity::InstallationProfile>,
    transport_identity: Option<String>,
    quota_account: Option<String>,
    quota_started: Instant,
    diagnostics: Option<DiagnosticCheckpoint>,
    retry: Option<Box<PendingDispatch>>,
}

struct DiagnosticCheckpoint {
    attempts: usize,
    payloads: usize,
    proxy_route: Option<serde_json::Value>,
    upstream_request_scoped: bool,
    failure_phase: Option<String>,
    first_response_ms: Option<u64>,
    truncated: bool,
}

const DOWNSTREAM_OUTCOME_TIMEOUT: Duration = Duration::from_secs(3600);

enum DownstreamOutcome {
    Success,
    Failed {
        status: u16,
        code: String,
        message: String,
        retry_after: Option<Duration>,
    },
    Cancelled,
}

/// Records the final adapter outcome after upstream inference has completed.
/// This does not retain a request/account permit or block dispatch returning.
pub(super) struct DownstreamOutcomeGuard {
    sender: Option<oneshot::Sender<DownstreamOutcome>>,
}

impl DownstreamOutcomeGuard {
    pub(super) fn success(&mut self) {
        self.settle(DownstreamOutcome::Success);
    }

    pub(super) fn fail(&mut self, error: &ServiceError) {
        self.settle(DownstreamOutcome::Failed {
            status: error.status,
            code: error.code.to_owned(),
            message: super::diagnostics::sanitize_error_text(&error.message),
            retry_after: error.retry_after,
        });
    }

    fn settle(&mut self, outcome: DownstreamOutcome) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(outcome);
        }
    }
}

impl Drop for DownstreamOutcomeGuard {
    fn drop(&mut self) {
        self.settle(DownstreamOutcome::Cancelled);
    }
}

impl RequestTracker {
    pub(super) fn new(
        activity: Arc<Activity>,
        host: Arc<dyn Host>,
        stop: StopSignal,
        permit: impl Into<Option<OwnedSemaphorePermit>>,
    ) -> Self {
        let permit = permit.into();
        let capacity = permit.as_ref().map(|permit| permit.semaphore().clone());
        activity.count.fetch_add(1, Ordering::SeqCst);
        activity.tasks.fetch_add(1, Ordering::SeqCst);
        Self {
            activity,
            host,
            stop,
            started: Instant::now(),
            busy: true,
            report: None,
            pending_dispatch: None,
            dispatch_count: 0,
            dispatch_open: false,
            dispatch_checkpoint: None,
            installation_profile: None,
            transport_identity: None,
            diagnostics: None,
            capture_payloads: false,
            operation: "text",
            downstream: None,
            global_permit: permit,
            capacity,
            route_permit: None,
            rebuild_abort: None,
            quota_account: None,
            quota_started: Instant::now(),
        }
    }

    /// Only the first guard for a turn owns its outcome; additional requests
    /// for a guard are harmless and cannot replace a still-pending result.
    pub(super) fn defer_downstream(&mut self) -> DownstreamOutcomeGuard {
        if self.downstream.is_some() {
            return DownstreamOutcomeGuard { sender: None };
        }
        let (sender, receiver) = oneshot::channel();
        self.downstream = Some(receiver);
        DownstreamOutcomeGuard {
            sender: Some(sender),
        }
    }

    pub(super) fn context(
        &mut self,
        method: &str,
        endpoint: &str,
        headers: &axum::http::HeaderMap,
        capture_payloads: bool,
    ) {
        self.dispatch_count = 0;
        self.dispatch_open = false;
        self.dispatch_checkpoint = None;
        self.pending_dispatch = None;
        self.quota_account = None;
        self.transport_identity = None;
        self.capture_payloads = capture_payloads;
        let identity = super::diagnostics::capture_header_identity(headers);
        self.diagnostics = Some(DiagnosticContext {
            request_id: uuid::Uuid::now_v7().to_string(),
            endpoint: endpoint.to_string(),
            method: method.to_string(),
            client_identity_captured: identity.captured,
            client_session_id: identity.session,
            client_thread_id: identity.thread,
            client_identity_metadata_captured: identity.metadata_captured,
            client_installation_id: identity.installation,
            client_chatgpt_account_id: identity.account,
            truncated: !identity.captured || !identity.metadata_captured,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            client_instance_id: [
                "x-cockpit-instance-id",
                "x-codex-instance-id",
                "x-client-instance-id",
            ]
            .iter()
            .find_map(|name| {
                headers
                    .get(*name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            }),
            ..Default::default()
        });
    }

    pub(super) fn payload(
        &mut self,
        stage: &str,
        transport: &str,
        headers: &axum::http::HeaderMap,
        body: &str,
    ) {
        if !self.capture_payloads {
            return;
        }
        let Some(context) = &mut self.diagnostics else {
            return;
        };
        if context.payloads.len() >= 16 {
            context.truncated = true;
            return;
        }
        let limit = 256 * 1024;
        let mut end = body.len().min(limit);
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        context.payloads.push(DiagnosticPayload {
            stage: stage.into(),
            transport: transport.into(),
            attempt_sequence: context.attempts.last().map(|v| v.sequence),
            content_type: headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .into(),
            headers: headers
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
                .collect(),
            body: if end < body.len() {
                "\"[omitted: diagnostic limit exceeded]\"".into()
            } else {
                body.into()
            },
            truncated: end < body.len(),
            original_bytes: body.len() as u64,
        });
    }

    pub(super) fn authorized(&mut self, caller: &Caller, model: &str) {
        if let Some(report) = self
            .report
            .as_mut()
            .filter(|report| report.account_id.is_empty())
        {
            report.requested_model = model.into();
            report.upstream_model = model.into();
        }
        if self.report.is_none() {
            self.report = Some(RequestReport {
                official_access_fingerprint: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                official_authorization_error: None,
                operation: self.operation,
                diagnostics: None,
                caller: caller.clone(),
                account_id: String::new(),
                requested_model: model.into(),
                upstream_model: model.into(),
                status: 499,
                success: false,
                elapsed: Duration::ZERO,
                usage: None,
                error: Some("client_cancelled".into()),
                error_message: None,
                credential_scope: false,
                retry_after: None,
                requested_service_tier: None,
                response_service_tier: None,
                reasoning_effort: None,
            });
        }
    }

    pub(super) fn deadline(&self, timeout: Duration) -> super::deadline::Deadline {
        if timeout.is_zero() {
            super::deadline::Deadline::NONE
        } else {
            (tokio::time::Instant::now() + timeout.saturating_sub(self.started.elapsed())).into()
        }
    }

    /// Called only immediately before dispatching a real upstream operation.
    /// Candidate selection, credential reads and local handshakes are not sends.
    pub(super) fn begin_attempt(&mut self, transport: &str) {
        if self.dispatch_open {
            return;
        }
        let mut checkpoint = Box::new(DispatchCheckpoint {
            report: self.report.clone(),
            installation_profile: self.installation_profile.clone(),
            transport_identity: self.transport_identity.clone(),
            quota_account: self.quota_account.clone(),
            quota_started: self.quota_started,
            diagnostics: self.diagnostics.as_ref().map(|context| DiagnosticCheckpoint {
                attempts: context.attempts.len(),
                payloads: context.payloads.len(),
                proxy_route: context.proxy_route.clone(),
                upstream_request_scoped: context.upstream_request_scoped,
                failure_phase: context.failure_phase.clone(),
                first_response_ms: context.first_response_ms,
                truncated: context.truncated,
            }),
            retry: None,
        });
        let pending = self.pending_dispatch.take();
        let next = pending.as_ref().map(|pending| pending.attempt.clone());
        if let Some(pending) = pending {
            self.transport_identity = pending.transport_identity;
            self.quota_account = pending.quota_account;
            self.installation_profile = pending.attempt.account_identity;
            self.report = Some(pending.report);
            if let Some(context) = &mut self.diagnostics {
                context.proxy_route = pending.proxy_route;
            }
        }
        self.quota_started = Instant::now();
        self.dispatch_count += 1;
        self.dispatch_open = true;
        self.capture_request_fault(false);
        if let Some(report) = &mut self.report {
            report.credential_scope = false;
            report.error_message = None;
            report.official_authorization_error = None;
            let previous = self.diagnostics.as_ref().and_then(|context| context.attempts.last());
            let mut attempt = next.unwrap_or_else(|| DiagnosticAttempt {
                authorization_route: previous.and_then(|attempt| attempt.authorization_route.clone()),
                authorization_fallback: previous.and_then(|attempt| attempt.authorization_fallback.clone()),
                service_tier_adaptation: previous.and_then(|attempt| attempt.service_tier_adaptation.clone()),
                account_identity: self.installation_profile.clone(),
                account_id: report.account_id.clone(),
                model_id: report.upstream_model.clone(),
                ..Default::default()
            });
            checkpoint.retry = Some(Box::new(PendingDispatch {
                report: report.clone(),
                attempt: attempt.clone(),
                transport_identity: self.transport_identity.clone(),
                quota_account: self.quota_account.clone(),
                proxy_route: self.diagnostics.as_ref().and_then(|context| context.proxy_route.clone()),
            }));
            if let Some(context) = &mut self.diagnostics {
                attempt.sequence = context.attempts.len() as u32 + 1;
                attempt.transport = transport.to_owned();
                attempt.started_at_ms = chrono::Utc::now().timestamp_millis();
                context.attempts.push(attempt);
            }
        }
        self.dispatch_checkpoint = Some(checkpoint);
    }

    pub(super) fn has_upstream_attempt(&self) -> bool {
        self.dispatch_count > 0
    }

    pub(super) fn upstream_attempt_count(&self) -> usize {
        self.dispatch_count
    }

    pub(super) fn upstream_account_ids(&self) -> std::collections::HashSet<String> {
        self.diagnostics.as_ref().map(|context| context.attempts.iter()
            .map(|attempt| attempt.account_id.clone()).collect()).unwrap_or_default()
    }

    /// Use only when transport evidence proves no HTTP request reached upstream
    /// (e.g. reqwest's is_connect). A timeout/read failure is not such evidence.
    pub(super) fn discard_unsent_attempt(&mut self) -> bool {
        if !self.dispatch_open {
            return false;
        }
        let Some(checkpoint) = self.dispatch_checkpoint.take() else {
            return false;
        };
        self.dispatch_open = false;
        self.dispatch_count = self.dispatch_count.saturating_sub(1);
        self.report = checkpoint.report;
        self.installation_profile = checkpoint.installation_profile;
        self.transport_identity = checkpoint.transport_identity;
        self.quota_account = checkpoint.quota_account;
        self.quota_started = checkpoint.quota_started;
        // The same candidate can be tried again without another routed call.
        self.pending_dispatch = checkpoint.retry;
        if let (Some(context), Some(before)) = (&mut self.diagnostics, checkpoint.diagnostics) {
            context.attempts.truncate(before.attempts);
            context.payloads.truncate(before.payloads);
            context.proxy_route = before.proxy_route;
            context.upstream_request_scoped = before.upstream_request_scoped;
            context.failure_phase = before.failure_phase;
            context.first_response_ms = before.first_response_ms;
            context.truncated = before.truncated;
        }
        true
    }

    pub(super) fn upstream_headers(&mut self, headers: &axum::http::HeaderMap) {
        self.upstream_identity(super::diagnostics::capture_header_identity(headers));
    }

    pub(super) fn upstream_response_headers(&mut self, headers: &axum::http::HeaderMap) {
        let request_id = ["x-request-id", "x-openai-request-id", "openai-request-id"]
            .iter().filter_map(|name| headers.get(*name).and_then(|value| value.to_str().ok()))
            .find(|value| !value.is_empty() && value.len() <= 256
                && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')))
            .map(str::to_owned);
        if let Some(attempt) = self.diagnostics.as_mut().and_then(|context| context.attempts.last_mut()) {
            attempt.upstream_request_id = request_id;
        }
    }

    pub(super) fn authorization_fallback_unavailable(
        &mut self,
        _account_id: &str,
        _model: &str,
        _failure: &super::contract::OfficialFallbackFailure,
        _error: &ServiceError,
    ) {
        // The candidate was never sent. Its preparation error is returned to
        // the caller, but must not impersonate a failed upstream execution.
    }

    pub(super) fn quota_headers(&self, headers: &axum::http::HeaderMap) {
        if let (Some(account_id), Some(snapshot)) = (&self.quota_account,
            super::rate_limits::QuotaSnapshot::from_headers(headers)) {
            self.host.observe_rate_limits(account_id, snapshot, self.quota_started);
        }
    }

    pub(super) fn check_quota_before_send(&self) -> Result<(), ServiceError> {
        // Routing stages a candidate without recording a send. Fence that exact
        // candidate, including an official route whose quota account is None.
        let account_id = match &self.pending_dispatch {
            Some(candidate) => candidate.quota_account.as_deref(),
            None => self.quota_account.as_deref(),
        };
        if let Some(account_id) = account_id {
            self.host.check_quota_before_send(account_id)
                .map_err(|error| error.with_upstream_not_sent(true))?;
            self.host.quota_request_started(account_id);
        }
        Ok(())
    }

    pub(super) fn quota_event(&self, event: &serde_json::Value) {
        if let (Some(account_id), Some(snapshot)) = (&self.quota_account,
            super::rate_limits::QuotaSnapshot::from_event(event)) {
            self.host.observe_rate_limits(account_id, snapshot, self.quota_started);
        }
    }

    pub(super) fn upstream_identity(&mut self, identity: super::diagnostics::HeaderIdentity) {
        let Some(context) = &mut self.diagnostics else {
            return;
        };
        if !identity.captured || !identity.metadata_captured {
            context.truncated = true;
        }
        if let Some(attempt) = context
            .attempts
            .last_mut()
            .filter(|attempt| attempt.status.is_none())
        {
            attempt.upstream_identity_captured = identity.captured;
            attempt.upstream_session_id = identity.session;
            attempt.upstream_thread_id = identity.thread;
            attempt.upstream_identity_metadata_captured = identity.metadata_captured;
            attempt.upstream_installation_id = identity.installation;
            attempt.upstream_chatgpt_account_id = identity.account;
        }
    }

    pub(super) fn capability_fallback(
        &mut self,
        trace: Option<super::diagnostics::CapabilityFallbackTrace>,
    ) {
        if let Some(attempt) = self
            .diagnostics
            .as_mut()
            .and_then(|v| v.attempts.last_mut())
        {
            if attempt.status.is_none() {
                attempt.capability_fallback = trace;
            }
        }
    }

    pub(super) fn first_response(&mut self) {
        if let Some(context) = &mut self.diagnostics {
            context
                .first_response_ms
                .get_or_insert(self.started.elapsed().as_millis() as u64);
        }
    }

    pub(super) fn attempt_result(&mut self, status: u16, success: bool, error: Option<&str>) {
        if !self.dispatch_open {
            return;
        }
        self.dispatch_open = false;
        self.dispatch_checkpoint = None;
        if let Some(report) = &mut self.report {
            report.status = status;
            report.success = success;
            report.error = error.map(str::to_string);
        }
        if let Some(context) = &mut self.diagnostics {
            if let Some(attempt) = context.attempts.last_mut().filter(|v| v.status.is_none()) {
                attempt.status = Some(status);
                attempt.success = success;
                attempt.latency_ms = chrono::Utc::now()
                    .timestamp_millis()
                    .saturating_sub(attempt.started_at_ms)
                    .max(0) as u64;
                attempt.error_category = error.map(str::to_string);
                attempt.failure_phase = (!success).then(|| "upstream".to_owned());
            }
        }
    }

    pub(super) fn attempt_error(&mut self, error: &ServiceError) {
        if !self.dispatch_open {
            return;
        }
        let open_attempt = self.diagnostics.as_ref().and_then(|context| context.attempts.last())
            .is_some_and(|attempt| attempt.status.is_none());
        if let Some(details) = error.response_error.as_ref() {
            self.capture_official_authorization_error(error.status, details);
        }
        let retired = if error.transport_failure == Some(super::contract::TransportFailureKind::TlsDecrypt) {
            self.transport_identity.as_deref().map(|identity| self.host.invalidate_transport(identity))
        } else {
            None
        };
        self.capture_request_fault(error.code == super::upstream_errors::REQUEST_SCOPED);
        self.capture_response_error(Some(&error.message), error.credential_scope);
        self.attempt_result(error.status, false, Some(error.code));
        if let Some(context) = &mut self.diagnostics {
            let mut phase = super::diagnostics::failure_phase(error.code);
            if phase == "upstream" && context.attempts.is_empty() {
                phase = "routing";
            }
            context.failure_phase = Some(phase.to_owned());
            if let Some(attempt) = context.attempts.last_mut().filter(|_| open_attempt) {
                if error.transport_failure.is_some() {
                    attempt.transport_error = Some("tls_decrypt".into());
                    // Repeated reporting of the same failure must preserve a
                    // successful invalidation rather than overwrite it as false.
                    if let Some(retired) = retired {
                        attempt.transport_pool_retired = Some(attempt.transport_pool_retired.unwrap_or(false) || retired);
                    }
                }
                attempt.error_message =
                    Some(super::diagnostics::sanitize_error_text(&error.message));
                attempt.failure_phase = Some(phase.to_owned());
            }
        }
    }

    pub(super) fn capture_response_error(&mut self, message: Option<&str>, credential_scope: bool) {
        if let Some(report) = &mut self.report {
            report.credential_scope |= credential_scope;
            if let Some(message) = message.filter(|message| !message.is_empty()) {
                report.error_message = Some(super::diagnostics::sanitize_error_text(message));
            }
        }
        if let (Some(context), Some(message)) = (&mut self.diagnostics, message) {
            if let Some(attempt) = context.attempts.last_mut().filter(|attempt| attempt.status.is_none()) {
                attempt.error_message = Some(super::diagnostics::sanitize_error_text(message));
            }
        }
    }

    pub(super) fn capture_request_fault(&mut self, request_scoped: bool) {
        if let Some(context) = &mut self.diagnostics {
            context.upstream_request_scoped = request_scoped;
        }
    }

    pub(super) fn capture_upstream_error(&mut self, status: u16, value: &serde_json::Value) {
        self.capture_official_authorization_error(status, value);
        let (message, scope) = super::diagnostics::upstream_error_metadata_status(status, value);
        self.capture_response_error(message.as_deref(), scope);
        self.capture_request_fault(super::upstream_errors::request_fault(status, value));
    }

    fn capture_official_authorization_error(&mut self, status: u16, value: &serde_json::Value) {
        if let Some(report) = self.report.as_mut().filter(|report| report.official_authorization_revision.is_some()) {
            report.official_authorization_error = Some(crate::modules::chatgpt_plan::inference_error(status, value));
        }
    }

    fn finish_diagnostic(
        &mut self,
        status: u16,
        success: bool,
        error: Option<&str>,
    ) -> Option<DiagnosticContext> {
        if self.dispatch_count == 0 {
            self.diagnostics = None;
            return None;
        }
        self.attempt_result(status, success, error);
        let mut context = self.diagnostics.take()?;
        context.output_generation_ms = context
            .first_response_ms
            .map(|first| (self.started.elapsed().as_millis() as u64).saturating_sub(first));
        if !success && context.failure_phase.is_none() {
            context.failure_phase = Some(
                if context.attempts.is_empty() {
                    "routing"
                } else {
                    error
                        .map(super::diagnostics::failure_phase)
                        .unwrap_or("upstream")
                }
                .into(),
            );
            if let Some(attempt) = context.attempts.last_mut().filter(|attempt| attempt.failure_phase.is_none()) {
                attempt.failure_phase = context.failure_phase.clone();
            }
        }
        Some(context)
    }

    pub(super) fn start_turn(&mut self) -> Result<(), ServiceError> {
        if self.stop.stopped() || !self.activity.accepting.load(Ordering::SeqCst) {
            return Err(stopping_error());
        }
        if !self.busy {
            if let Some(capacity) = &self.capacity {
                self.global_permit = Some(capacity.clone().try_acquire_owned().map_err(|_| {
                    ServiceError::new(
                        429,
                        "service_busy",
                        "The API service has reached its concurrent request limit",
                    )
                })?);
            }
            self.activity.count.fetch_add(1, Ordering::SeqCst);
            self.busy = true;
        }
        if self.stop.stopped() || !self.activity.accepting.load(Ordering::SeqCst) {
            self.idle();
            return Err(stopping_error());
        }
        self.started = Instant::now();
        Ok(())
    }

    pub(super) fn idle(&mut self) {
        if self.busy {
            self.activity.count.fetch_sub(1, Ordering::SeqCst);
            self.busy = false;
            self.global_permit = None;
            self.activity.changed.notify_waiters();
        }
    }

    pub(super) fn capture_usage(&mut self, usage: &Option<serde_json::Value>) {
        if let (Some(report), Some(usage)) = (self.report.as_mut(), usage.as_ref()) {
            report.usage = Some(usage.clone());
        }
    }

    pub(super) fn routed(&mut self, caller: &Caller, route: &Route, model: &str) {
        self.route_permit = route.permit.clone();
        let attempt = DiagnosticAttempt {
            authorization_route: Some(if route.chatgpt_plan { "official" } else { "local_gateway" }.into()),
            authorization_fallback: (!route.chatgpt_plan).then(|| route.official_fallback_failure.as_ref()
                .map(|failure| super::official_fallback::trace(&route.account_id, failure))).flatten(),
            account_identity: route.installation_profile.clone(),
            account_id: route.account_id.clone(),
            model_id: route.model.clone(),
            ..Default::default()
        };
        let report = RequestReport {
            official_authorization_revision: route.official_authorization_revision.clone(),
            official_registration_id: route.official_registration_id.clone(),
            official_fallback_registration_id: route.official_fallback_registration_id.clone(),
            official_fallback_authorization_revision: route.official_fallback_authorization_revision.clone(),
            official_authorization_error: None,
            official_access_fingerprint: route.chatgpt_plan.then(|| route.headers.get("authorization")
                .and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer "))
                .map(crate::modules::chatgpt_plan::token_fingerprint)).flatten(),
            operation: self.operation,
            diagnostics: None,
            caller: caller.clone(),
            account_id: route.account_id.clone(),
            requested_model: model.to_string(),
            upstream_model: route.model.clone(),
            status: 499,
            success: false,
            elapsed: Duration::ZERO,
            usage: None,
            error: Some("client_cancelled".to_string()),
            error_message: None,
            credential_scope: false,
            retry_after: None,
            requested_service_tier: None,
            response_service_tier: None,
            reasoning_effort: None,
        };
        self.pending_dispatch = Some(Box::new(PendingDispatch {
            report,
            attempt,
            transport_identity: route.transport_identity.clone(),
            quota_account: (route.chatgpt && !route.chatgpt_plan).then(|| route.account_id.clone()),
            proxy_route: route.proxy_route.clone(),
        }));
    }

    pub(super) fn prepared_metadata(&mut self, prepared: &super::mapping::Prepared) {
        self.request_metadata(&prepared.body);
        let requested = prepared.source_body.get("service_tier")
            .and_then(serde_json::Value::as_str)
            .or(prepared.route.default_service_tier.as_deref());
        let attempt = if let Some(pending) = &mut self.pending_dispatch {
            Some(&mut pending.attempt)
        } else {
            self.diagnostics.as_mut().and_then(|context| context.attempts.last_mut())
        };
        if let Some(attempt) = attempt {
            attempt.service_tier_adaptation = requested
                .filter(|tier| prepared.route.chatgpt_plan && !matches!(*tier, "" | "auto" | "default"))
                .map(|tier| crate::models::codex_local_access::CodexLocalAccessServiceTierAdaptation {
                    requested_tier: tier.to_owned(), sent_tier: "default".into(), reason: "official_default_only".into(),
                });
        }
    }

    fn metadata_report(&mut self) -> Option<&mut RequestReport> {
        if let Some(pending) = &mut self.pending_dispatch {
            Some(&mut pending.report)
        } else {
            self.report.as_mut()
        }
    }

    pub(super) fn request_metadata(&mut self, body: &serde_json::Value) {
        if let Some(report) = self.metadata_report() {
            report.requested_service_tier = body
                .get("service_tier")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            report.reasoning_effort = body
                .pointer("/reasoning/effort")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
    }

    pub(super) fn upstream_http_version(&mut self, version: reqwest::Version) {
        if let Some(attempt) = self.diagnostics.as_mut().and_then(|context| context.attempts.last_mut()) {
            attempt.upstream_http_version = Some(format!("{version:?}"));
        }
    }

    pub(super) fn track_rebuild(&mut self, next: Option<Arc<AtomicBool>>) {
        if let Some(previous) = self.rebuild_abort.take() {
            if next
                .as_ref()
                .is_none_or(|next| !Arc::ptr_eq(&previous, next))
            {
                previous.store(true, Ordering::Release);
            }
        }
        self.rebuild_abort = next;
    }

    pub(super) fn routing_identity(&mut self, request: &super::contract::RouteRequest) {
        if let Some(context) = &mut self.diagnostics {
            context.routing_identity = Some(super::contract::RoutingIdentity {
                session_id: request.session_id.clone(),
                thread_id: request.thread_id.clone(),
                parent_thread_id: request.parent_thread_id.clone(),
            });
        }
    }

    pub(super) fn set_operation(&mut self, operation: &'static str) {
        self.operation = operation;
        if let Some(report) = &mut self.report {
            report.operation = operation;
        }
    }

    pub(super) fn response_tier(&mut self, value: Option<&str>) {
        if let (Some(report), Some(value)) = (self.report.as_mut(), value) {
            report.response_service_tier = Some(value.to_string());
        }
    }

    pub(super) fn set_retry_after(&mut self, value: Option<Duration>) {
        if let (Some(report), Some(value)) = (self.report.as_mut(), value) {
            report.retry_after = Some(
                report
                    .retry_after
                    .map_or(value, |current| current.max(value)),
            );
        }
    }

    pub(super) fn release_route(&mut self) {
        self.route_permit = None;
    }

    pub(super) async fn complete(
        &mut self,
        status: u16,
        success: bool,
        usage: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        // A later candidate may fail locally after a real upstream rejection.
        // Persist that real response under its original account, not the
        // unsent candidate or its preparation/cancellation error.
        let (status, success, error) = if !self.dispatch_open && self.dispatch_count > 0 {
            self.report.as_ref().map(|report| (report.status, report.success, report.error.clone()))
                .unwrap_or((status, success, error))
        } else {
            (status, success, error)
        };
        if let Some(abort) = self.rebuild_abort.take().filter(|_| !success) {
            abort.store(true, Ordering::Release);
        }
        if success {
            self.capture_request_fault(false);
        }
        self.idle();
        self.route_permit = None;
        self.pending_dispatch = None;
        let diagnostics = self.finish_diagnostic(status, success, error.as_deref());
        let downstream = self.downstream.take();
        if let Some(mut report) = self.report.take() {
            report.diagnostics = diagnostics.clone();
            report.status = status;
            report.success = success;
            if usage.is_some() {
                report.usage = usage;
            }
            report.error = error;
            if success {
                report.error_message = None;
                report.credential_scope = false;
            }
            report.elapsed = self.started.elapsed();
            // The worker owns the record before the first await. Disconnecting the
            // caller cannot cancel persistence, and shutdown waits for this worker.
            self.persist(Some(report), diagnostics, downstream).await;
        } else {
            self.persist(None, diagnostics, downstream).await;
        }
    }

    fn persistence_task(
        &self,
        mut report: Option<RequestReport>,
        mut diagnostics: Option<DiagnosticContext>,
        downstream: Option<oneshot::Receiver<DownstreamOutcome>>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if self.dispatch_count == 0 || report.is_none() && diagnostics.is_none() {
            return None;
        }
        let runtime = tokio::runtime::Handle::try_current().ok()?;
        let host = self.host.clone();
        let activity = self.activity.clone();
        let stop = self.stop.clone();
        let started = self.started;
        activity.tasks.fetch_add(1, Ordering::SeqCst);
        Some(runtime.spawn(async move {
            let _guard = PersistenceGuard(activity);
            settle_downstream(
                &mut report,
                &mut diagnostics,
                downstream,
                stop,
                started,
                DOWNSTREAM_OUTCOME_TIMEOUT,
            )
            .await;
            let record = async {
                if let Some(report) = report {
                    if tokio::time::timeout(Duration::from_secs(5), host.record(report))
                        .await
                        .is_err()
                    {
                        eprintln!("Native API request record persistence timed out");
                    }
                }
            };
            let diagnostic = async {
                if let Some(context) = diagnostics {
                    if tokio::time::timeout(Duration::from_secs(5), host.diagnostic(context))
                        .await
                        .is_err()
                    {
                        eprintln!("Native API diagnostic persistence timed out");
                    }
                }
            };
            tokio::join!(record, diagnostic);
        }))
    }

    async fn persist(
        &self,
        report: Option<RequestReport>,
        diagnostics: Option<DiagnosticContext>,
        downstream: Option<oneshot::Receiver<DownstreamOutcome>>,
    ) {
        let _ = self.persistence_task(report, diagnostics, downstream);
        // Let an immediately-ready host record run without waiting on slow storage.
        tokio::task::yield_now().await;
    }

    pub(super) async fn fail(&mut self, error: &ServiceError) {
        let dispatched = self.dispatch_open;
        self.attempt_error(error);
        if dispatched && error.retry_after.is_some() {
            self.set_retry_after(error.retry_after);
        }
        self.complete(error.status, false, None, Some(error.code.to_string()))
            .await;
    }

    pub(super) async fn complete_local(&mut self) {
        self.idle();
        self.route_permit = None;
        self.pending_dispatch = None;
        let diagnostics = self.finish_diagnostic(200, true, None);
        self.report = None;
        self.persist(None, diagnostics, None).await;
    }

    /// A local handshake rejection negotiates HTTP without contacting upstream.
    pub(super) async fn transport_fallback(&mut self, _error: &ServiceError) {
        debug_assert_eq!(self.dispatch_count, 0);
        self.complete_local().await;
    }
}

impl Drop for RequestTracker {
    fn drop(&mut self) {
        if let Some(abort) = self.rebuild_abort.take() {
            abort.store(true, Ordering::Release);
        }
        self.idle();
        let (status, success, error) = if !self.dispatch_open && self.dispatch_count > 0 {
            self.report.as_ref().map(|report| (report.status, report.success, report.error.clone()))
                .unwrap_or((499, false, Some("client_cancelled".into())))
        } else if self.stop.stopped() {
            (503, false, Some("service_stopping".into()))
        } else {
            (499, false, Some("client_cancelled".into()))
        };
        let diagnostics = self.finish_diagnostic(status, success, error.as_deref());
        let mut report = self.report.take();
        if let Some(report) = &mut report {
            report.elapsed = self.started.elapsed();
            report.status = status;
            report.success = success;
            report.error = error;
            report.diagnostics = diagnostics.clone();
        }
        // Register persistence before releasing the request task so shutdown
        // cannot observe zero tasks between cancellation and recording.
        let downstream = self.downstream.take();
        let _ = self.persistence_task(report, diagnostics, downstream);
        self.activity.tasks.fetch_sub(1, Ordering::SeqCst);
        self.activity.changed.notify_waiters();
    }
}

async fn settle_downstream(
    report: &mut Option<RequestReport>,
    diagnostics: &mut Option<DiagnosticContext>,
    downstream: Option<oneshot::Receiver<DownstreamOutcome>>,
    mut stop: StopSignal,
    started: Instant,
    max_wait: Duration,
) {
    let (Some(report), Some(receiver)) = (report.as_mut(), downstream) else {
        return;
    };
    // An upstream failure is already terminal. Adapter cancellation or a
    // fallback response must never replace its original error or token usage.
    if !report.success {
        return;
    }
    let outcome = tokio::select! {
        biased;
        result = receiver => result.unwrap_or(DownstreamOutcome::Cancelled),
        _ = stop.wait() => DownstreamOutcome::Failed {
            status: 503, code: "service_stopping".into(), message: "The API service is stopping".into(), retry_after: None,
        },
        _ = tokio::time::sleep(max_wait) => DownstreamOutcome::Failed {
            status: 504, code: "downstream_outcome_timeout".into(), message: "Downstream processing timed out".into(), retry_after: None,
        },
    };
    let failure = match outcome {
        DownstreamOutcome::Success => None,
        DownstreamOutcome::Failed {
            status,
            code,
            message,
            retry_after,
        } => Some((status, code, message, retry_after)),
        DownstreamOutcome::Cancelled => Some((
            499,
            "client_cancelled".into(),
            "Client cancelled the response".into(),
            None,
        )),
    };
    if let Some((status, code, message, retry_after)) = failure {
        report.status = status;
        report.success = false;
        report.error = Some(code);
        report.error_message = Some(message);
        report.credential_scope = false;
        if retry_after.is_some() {
            report.retry_after = retry_after;
        }
        if let Some(context) = diagnostics.as_mut() {
            context.failure_phase = Some("downstream".into());
        }
    }
    report.elapsed = started.elapsed();
    if let Some(context) = diagnostics.as_mut() {
        context.output_generation_ms = context.first_response_ms.map(|first| {
            (report.elapsed.as_millis().min(u128::from(u64::MAX)) as u64).saturating_sub(first)
        });
    }
    report.diagnostics = diagnostics.clone();
}

struct PersistenceGuard(Arc<Activity>);

impl Drop for PersistenceGuard {
    fn drop(&mut self) {
        self.0.tasks.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

pub(super) fn stopping_error() -> ServiceError {
    ServiceError::new(
        503,
        "service_stopping",
        "The API service is stopping; retry after switching completes",
    )
}

#[cfg(test)]
mod tests {
    include!("lifecycle_tests.rs");
}
