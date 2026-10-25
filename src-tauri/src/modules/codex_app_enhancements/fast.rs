//! Adapt API-service quota gates in the current document on Windows and macOS.
//! Other native enhancements use document preload or the guarded startup path.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant as StdInstant};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::{timeout, Instant};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};
use tokio_tungstenite::{connect_async_with_config, MaybeTlsStream, WebSocketStream};

const GUARD_SCRIPT: &str = include_str!("startup_guard.js");
const PRELOAD_SCRIPT: &str = include_str!("startup_preload.js");
const SCRIPT: &str = include_str!("fast.js");
const QUOTA_SCRIPT: &str = include_str!("quota.js");
const QUOTA_CURRENT_SCRIPT: &str = include_str!("quota_current.js");
const STATE_SCRIPT: &str = r#";(() => {
  const previous = window.__cockpitEnhancements;
  const next = __COCKPIT_STATE_OPTIONS__;
  // Revoke retired bridges without reloading a page with an unsent draft.
  // Keep old hook objects alive: an already evaluated client module may still
  // call them until its next normal navigation. Its native policies now apply.
  if (previous) previous.apiDictation = false;
  try { window.__cockpitApiDictation?.dispose?.(); } catch {}
  if (typeof window.__cockpitNativeDictation?.capability === 'function')
    window.__cockpitNativeDictation.capability = nativeValue => nativeValue;
  const features = [next.nativeFast, next.quotaCompat, next.apiKeyAuthVerified, next.apiService].map(Number).join('');
  const installedFeatures = previous?.nativePatchMode === 'fast-only'
    ? [next.nativeFast, false, next.apiKeyAuthVerified, false].map(Number).join('') : features;
  const active = previous?.nativePatchState === 'active'
    && ['full', 'live-quota', 'fast-only'].includes(previous.nativePatchMode)
    && (previous.nativePatchMode !== 'fast-only' || next.nativeFast === true
      && previous.nativeQuotaPatchState === 'unsupported')
    && previous.nativePatchRequestedFeatures === features
    && previous.nativePatchFeatures === installedFeatures;
  const sameOwnerAndConfig = previous?.owner === next.owner
    && previous.nativeFast === next.nativeFast
    && previous.quotaCompat === next.quotaCompat
    && previous.apiKeyAuthVerified === next.apiKeyAuthVerified
    && previous.apiService === next.apiService;
  const preserveLoadingState = sameOwnerAndConfig
    && ['preloading', 'unsupported'].includes(previous.nativePatchState);
  window.__cockpitEnhancements = {
    ...previous, ...next, apiDictation: false,
    nativePatchState: active ? previous.nativePatchState : preserveLoadingState ? previous.nativePatchState : 'pending',
    nativePatchReason: active || preserveLoadingState ? previous.nativePatchReason : undefined,
    nativeQuotaPatchState: active || preserveLoadingState ? previous?.nativeQuotaPatchState : undefined,
    nativeQuotaPatchReason: active || preserveLoadingState ? previous?.nativeQuotaPatchReason : undefined,
  };
})();"#;
const RUNTIME_PROBE_SCRIPT: &str = r#"(() => {
  const p = window.__cockpitNativeFast, s = window.__cockpitEnhancements;
  const expected = __COCKPIT_BROWSER_SNAPSHOT__;
  const ready = typeof p?.prepareHandoff === 'function'
    && typeof p?.consumeHandoff === 'function' && typeof p?.cancelHandoff === 'function';
  return {
    installed: ready && s?.owner === __COCKPIT_WORKER_OWNER__,
    preloading: ready && s?.nativePatchState === 'preloading'
      && typeof window.__cockpitNativeFastComplete === 'function'
      && Object.keys(expected).every(key => s[key] === expected[key]),
  };
})()"#;
const SOURCE_LIMIT: usize = 24 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(4);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(20);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(35);
const RETRY_DELAYS: [u64; 5] = [1, 2, 5, 10, 15];
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type ContextKey = (u64, i64);
#[path = "startup_draft.rs"]
mod startup_draft;

const DOCUMENT_INSTALL_GUARD: &str = r#";(() => {
  if (window.__cockpitNativeInstallStarted) { __COCKPIT_SKIPPED__ return; }
  __COCKPIT_SOURCE__
})();"#;

fn guard_document_install(source: &str, skipped: &str) -> String {
    DOCUMENT_INSTALL_GUARD
        .replace("__COCKPIT_SKIPPED__", skipped)
        .replace("__COCKPIT_SOURCE__", source)
}

/// The browser endpoint registers the complete bounded startup program before
/// the app's first document. No transform runs on an already-started document.
pub(super) fn browser_preload_source(
    native_fast: bool,
    quota_compat: bool,
    api_key_auth_verified: bool,
    api_service: bool,
    owner: &str,
) -> String {
    guard_document_install(
        &format!(
            "{}\n{PRELOAD_SCRIPT}",
            bootstrap(
                native_fast,
                quota_compat,
                api_key_auth_verified,
                api_service,
                owner
            )
        ),
        "",
    )
}

/// Shared by every target/worker of one explicit profile launch. Reconnection
/// and configuration changes must not create another startup permission.
pub(super) struct StartupReload {
    deadline: StdInstant,
    claimed: AtomicBool,
}

impl StartupReload {
    pub(super) fn new() -> Self {
        Self {
            deadline: StdInstant::now() + Duration::from_secs(45),
            claimed: AtomicBool::new(false),
        }
    }

    pub(super) fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(StdInstant::now())
    }

    fn available(&self) -> bool {
        StdInstant::now() <= self.deadline && !self.claimed.load(Ordering::Acquire)
    }

    fn claim(&self) -> bool {
        self.available()
            && self
                .claimed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }

    /// Only an explicit transient rejection from the atomic renderer guard
    /// proves no navigation was initiated. An unknown result must stay spent.
    fn release_rejected(&self) {
        let _ = self
            .claimed
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire);
    }
}

struct Session {
    socket: Socket,
    command_id: u64,
    contexts: HashMap<i64, String>,
    main_frame: Option<String>,
    generation: u64,
    registrations: HashSet<String>,
    pending_additions: HashSet<u64>,
    pending_removals: HashMap<u64, String>,
}

impl Session {
    async fn receive(&mut self) -> Result<Option<Value>, String> {
        while let Some(message) = self.socket.next().await {
            match message.map_err(|_| "native enhancement transport failed".to_string())? {
                Message::Ping(bytes) => self
                    .socket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|_| "native enhancement transport failed".to_string())?,
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text)
                        .map_err(|_| "native enhancement invalid response".to_string())?;
                    self.observe_registration_reply(&value);
                    self.observe(&value)?;
                    return Ok(Some(value));
                }
                Message::Close(_) => return Ok(None),
                _ => {}
            }
        }
        Ok(None)
    }

    async fn send(&mut self, method: &str, params: Value) -> Result<u64, String> {
        self.command_id += 1;
        let id = self.command_id;
        if method == "Page.addScriptToEvaluateOnNewDocument" {
            self.pending_additions.insert(id);
        } else if method == "Page.removeScriptToEvaluateOnNewDocument" {
            if let Some(identifier) = params.get("identifier").and_then(Value::as_str) {
                self.pending_removals.insert(id, identifier.to_string());
            }
        }
        self.socket
            .send(Message::Text(
                json!({"id":id,"method":method,"params":params})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|_| "native enhancement transport failed".to_string())?;
        Ok(id)
    }

    async fn command(
        &mut self,
        method: &str,
        params: Value,
        limit: Duration,
    ) -> Result<Value, String> {
        timeout(limit, async {
            let id = self.send(method, params).await?;
            while let Some(reply) = self.receive().await? {
                if reply.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if reply.get("error").is_some() {
                    return Err(format!("native enhancement command rejected: {method}"));
                }
                return Ok(reply);
            }
            Err("native enhancement connection closed".to_string())
        })
        .await
        .map_err(|_| format!("native enhancement command timed out: {method}"))?
    }

    fn observe_registration_reply(&mut self, reply: &Value) {
        let Some(id) = reply.get("id").and_then(Value::as_u64) else {
            return;
        };
        if self.pending_additions.remove(&id) {
            if let Some(identifier) = reply
                .pointer("/result/identifier")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                self.registrations.insert(identifier.to_string());
            }
        }
        if let Some(identifier) = self.pending_removals.remove(&id) {
            if reply.get("error").is_none() {
                self.registrations.remove(&identifier);
            }
        }
    }

    fn observe(&mut self, event: &Value) -> Result<(), String> {
        match event.get("method").and_then(Value::as_str) {
            Some("Inspector.detached") => {
                return Err("native enhancement target detached".to_string())
            }
            Some("Runtime.executionContextsCleared") => {
                self.contexts.clear();
                self.generation += 1;
            }
            Some("Runtime.executionContextDestroyed") => {
                if let Some(id) = event
                    .pointer("/params/executionContextId")
                    .and_then(Value::as_i64)
                {
                    self.contexts.remove(&id);
                }
            }
            Some("Runtime.executionContextCreated") => {
                let context = &event["params"]["context"];
                if context
                    .pointer("/auxData/isDefault")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    if let (Some(id), Some(frame)) = (
                        context.get("id").and_then(Value::as_i64),
                        context.pointer("/auxData/frameId").and_then(Value::as_str),
                    ) {
                        self.contexts.insert(id, frame.to_string());
                    }
                }
            }
            Some("Page.frameNavigated") => {
                let frame = &event["params"]["frame"];
                if frame.get("parentId").is_none() {
                    if let Some(id) = frame.get("id").and_then(Value::as_str) {
                        self.main_frame = Some(id.to_string());
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn context(&self) -> Option<ContextKey> {
        let frame = self.main_frame.as_deref()?;
        let mut contexts = self
            .contexts
            .iter()
            .filter(|(_, candidate)| candidate.as_str() == frame);
        let id = *contexts.next()?.0;
        // Never guess between two default worlds during navigation.
        contexts.next().is_none().then_some((self.generation, id))
    }

    async fn evaluate(
        &mut self,
        expression: String,
        context: ContextKey,
        limit: Duration,
    ) -> Result<Value, String> {
        let reply = self
            .command(
                "Runtime.evaluate",
                json!({"expression":expression,"contextId":context.1,"returnByValue":true,"awaitPromise":true}),
                limit,
            )
            .await?;
        if reply.pointer("/result/exceptionDetails").is_some() {
            return Err("native enhancement evaluation rejected".to_string());
        }
        reply
            .pointer("/result/result/value")
            .cloned()
            .ok_or_else(|| "native enhancement returned no value".to_string())
    }
}

#[derive(Default)]
struct Attempt {
    count: usize,
    next: Option<Instant>,
    done: bool,
}

impl Attempt {
    fn due(&self) -> bool {
        !self.done && self.next.is_none_or(|time| Instant::now() >= time)
    }
    fn failed(&mut self, retryable: bool) {
        self.count += 1;
        self.next = RETRY_DELAYS
            .get(self.count - 1)
            .map(|seconds| Instant::now() + Duration::from_secs(*seconds));
        self.done = !retryable || self.next.is_none();
    }
}

/// Retains the websocket in Drop so cancellation still removes registrations,
/// including a registration whose reply arrives after its command was canceled.
struct Registration {
    session: Option<Session>,
    script_id: Option<String>,
    owner: String,
    runtime: String,
    startup_runtime: String,
    runtime_probe: String,
    preload_first: bool,
    live_quota: bool,
    live_quota_armed: bool,
    breakpoint_id: Option<String>,
    nonce: String,
    startup: Option<Arc<StartupReload>>,
    draft_file: PathBuf,
    context: Option<ContextKey>,
    document: Attempt,
    live_quota_attempt: Attempt,
    bootstrapped: bool,
    active: bool,
    native_module_active: bool,
    quota_unsupported_reason: Option<String>,
    capability_only: bool,
    activation_until: Option<Instant>,
    next_state_read: Option<Instant>,
    next_startup_check: Option<Instant>,
    last_diagnostic: Option<Value>,
}

impl Registration {
    fn session(&mut self) -> &mut Session {
        self.session
            .as_mut()
            .expect("registration owns its session")
    }

    async fn cleanup(&mut self) {
        if let Some(mut session) = self.session.take() {
            cleanup(
                &mut session,
                self.script_id.take(),
                self.breakpoint_id.take(),
                &self.owner,
                &self.nonce,
            )
            .await;
        }
    }

    async fn report(&mut self, phase: &str, reason: &str) {
        let state = if self.active && self.capability_only {
            "active-capability"
        } else if self.active {
            "active"
        } else if self.script_id.is_some() {
            "next-document"
        } else {
            "pending"
        };
        let mut diagnostic = json!({"state":state,"phase":phase,"reason":reason});
        if let Some(quota_reason) = &self.quota_unsupported_reason {
            diagnostic["quotaState"] = json!("unsupported");
            diagnostic["quotaReason"] = json!(quota_reason);
        }
        if self.last_diagnostic.as_ref() == Some(&diagnostic) {
            return;
        }
        self.last_diagnostic = Some(diagnostic.clone());
        let mut message =
            format!("[CodexEnhancements] Native compatibility {phase}: {reason}; state={state}");
        if let Some(quota_reason) = &self.quota_unsupported_reason {
            message.push_str(&format!("; quota=unsupported:{quota_reason}"));
        }
        if self.active {
            crate::modules::logger::log_info(&message);
        } else {
            crate::modules::logger::log_warn(&message);
        }
        // Activation requires module evaluation or an installed live quota gate.
        // Registering a future-document script alone is not activation evidence.
        let Some(context) = self
            .context
            .filter(|value| self.session.as_ref().and_then(Session::context) == Some(*value))
        else {
            return;
        };
        let expression = format!("(()=>{{const s=window.__cockpitEnhancements;if(s?.owner==={})s.nativePatchDiagnostic={};return true;}})()", json!(self.owner), diagnostic);
        let _ = self
            .session()
            .evaluate(expression, context, COMMAND_TIMEOUT)
            .await;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        let script_id = self.script_id.take();
        let breakpoint_id = self.breakpoint_id.take();
        let owner = self.owner.clone();
        let nonce = self.nonce.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                cleanup(&mut session, script_id, breakpoint_id, &owner, &nonce).await;
            });
        }
    }
}

async fn cleanup(
    session: &mut Session,
    script_id: Option<String>,
    breakpoint_id: Option<String>,
    owner: &str,
    nonce: &str,
) {
    let _ = timeout(CLEANUP_TIMEOUT, async {
        if let Some(context) = session.context() {
            let expression = format!("(()=>{{const s=window.__cockpitEnhancements;if(s?.owner==={}){{window.__cockpitQuotaCompat?.disposeLiveQuotaGuard?.();window.__cockpitNativeFast?.cancelHandoff?.({});s.nativeFast=false;s.quotaCompat=false;s.apiService=false;window.__cockpitNativeFastDispose?.();}}return true;}})()", json!(owner), json!(nonce));
            let _ = session.evaluate(expression, context, Duration::from_millis(500)).await;
        }
        if let Some(breakpoint_id) = breakpoint_id {
            let _ = session
                .command(
                    "Debugger.removeBreakpoint",
                    json!({"breakpointId":breakpoint_id}),
                    Duration::from_millis(500),
                )
                .await;
            let _ = session
                .command("Debugger.disable", json!({}), Duration::from_millis(500))
                .await;
        }
        if let Some(identifier) = script_id { session.registrations.insert(identifier); }
        loop {
            let identifiers: Vec<_> = session.registrations.iter().cloned().collect();
            for identifier in identifiers {
                let _ = session.command("Page.removeScriptToEvaluateOnNewDocument", json!({"identifier":identifier}), Duration::from_millis(500)).await;
            }
            if session.pending_additions.is_empty() { break; }
            // Replies can arrive out of order; do not rely on an evaluate reply
            // being a barrier for an earlier addScript command.
            if !matches!(session.receive().await, Ok(Some(_))) { break; }
        }
        let _ = session.socket.close(None).await;
    }).await;
}

fn safe_reason(value: &Value, fallback: &str) -> String {
    value
        .pointer("/diagnostics/reason")
        .or_else(|| value.get("reason"))
        .and_then(Value::as_str)
        .filter(|reason| {
            !reason.is_empty()
                && reason.len() <= 64
                && reason
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .unwrap_or(fallback)
        .to_string()
}

fn command_reason(error: &str) -> String {
    let operation = if error.ends_with("Page.addScriptToEvaluateOnNewDocument") {
        "registration"
    } else if error.ends_with("Debugger.enable") {
        "debugger-enable"
    } else if error.ends_with("Debugger.setBreakpointByUrl") {
        "breakpoint-registration"
    } else if error.ends_with("Runtime.evaluate") || error.contains("evaluation") {
        "evaluation"
    } else {
        "transport"
    };
    format!(
        "{operation}-{}",
        if error.contains("timed out") {
            "timeout"
        } else {
            "failed"
        }
    )
}

fn bootstrap(
    native_fast: bool,
    quota_compat: bool,
    api_key_auth_verified: bool,
    api_service: bool,
    owner: &str,
) -> String {
    let options = json!({"nativeFast":native_fast,"quotaCompat":quota_compat,"apiKeyAuthVerified":api_key_auth_verified,"apiService":api_service,"owner":owner});
    let state = STATE_SCRIPT.replace("__COCKPIT_STATE_OPTIONS__", &options.to_string());
    format!("{state}\n{GUARD_SCRIPT}\n{SCRIPT}\n{QUOTA_SCRIPT}\n{QUOTA_CURRENT_SCRIPT}\n;true;")
}

async fn rollback_preparation(registration: &mut Registration) {
    if let Some(context) = registration.session().context() {
        let expression = format!(
            "window.__cockpitNativeFast?.cancelHandoff?.({})",
            json!(registration.nonce)
        );
        let _ = registration
            .session()
            .evaluate(expression, context, COMMAND_TIMEOUT)
            .await;
    }
    let identifiers = registration.session().registrations.clone();
    for identifier in identifiers {
        let _ = registration
            .session()
            .command(
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({"identifier":identifier}),
                COMMAND_TIMEOUT,
            )
            .await;
    }
    registration.script_id = None;
}

async fn prepare_startup_preload(
    registration: &mut Registration,
    reason: &str,
) -> Result<(), String> {
    let source = registration.startup_runtime.clone();
    let reply = registration
        .session()
        .command(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source":source}),
            COMMAND_TIMEOUT,
        )
        .await
        .map_err(|error| command_reason(&error))?;
    registration.script_id = Some(
        reply
            .pointer("/result/identifier")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "registration-failed".to_string())?,
    );
    registration.document.done = true;
    // The preload script performs the first activation. Never request a second
    // document: Windows treats that renderer reload as a new cold start and can
    // leave the application on its startup loader.
    registration.startup = None;
    registration.report("document", reason).await;
    Ok(())
}

async fn prepare_future_document(
    registration: &mut Registration,
    context: ContextKey,
) -> Result<(), String> {
    // The renderer retains the large prepared module in its private closure.
    // Only bounded metadata crosses CDP; polling never copies the source again.
    let expression = format!(
        "window.__cockpitNativeFast.prepareHandoff({})",
        json!(registration.nonce)
    );
    let result = registration
        .session()
        .evaluate(expression, context, PREPARE_TIMEOUT)
        .await
        .map_err(|error| command_reason(&error))?;
    if registration.session().context() != Some(context) {
        return Err("context-changed".to_string());
    }
    if result.get("ok").and_then(Value::as_bool) != Some(true) {
        let reason = safe_reason(&result, "unsupported-document");
        rollback_preparation(registration).await;
        return Err(reason);
    }
    let invalid = if !result
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(local_initial)
    {
        Some("invalid-module-url")
    } else if !result
        .get("sourceLength")
        .and_then(Value::as_u64)
        .is_some_and(|length| length > 0 && length <= SOURCE_LIMIT as u64)
    {
        Some("invalid-module-source-length")
    } else {
        None
    };
    if let Some(reason) = invalid {
        rollback_preparation(registration).await;
        return Err(reason.to_string());
    }
    // This script stays small enough to execute before the app's document CSP.
    // The nonce-scoped handoff is only staged by the final guarded reload or a
    // subsequent local user navigation, and consume immediately restores name.
    let script = guard_document_install(
        &format!(
            "{}\n;window.__cockpitNativeFast.consumeHandoff({});",
            registration.runtime,
            json!(registration.nonce)
        ),
        &format!(
            "window.__cockpitNativeFast?.discardHandoff({}, {});",
            json!(registration.nonce),
            json!(registration.owner)
        ),
    );
    let reply = registration
        .session()
        .command(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source":script}),
            COMMAND_TIMEOUT,
        )
        .await;
    let setup = match reply {
        Ok(reply) => {
            registration.script_id = reply
                .pointer("/result/identifier")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string);
            registration
                .script_id
                .as_ref()
                .map(|_| ())
                .ok_or_else(|| "registration-failed".to_string())
        }
        Err(error) => Err(command_reason(&error)),
    };
    if setup.is_err() {
        rollback_preparation(registration).await;
    }
    setup
}

/// These are snapshots of a changing local page, not permanent failures.
/// Remain pending only within the original launch deadline and reload only
/// after the final atomic guard observes a clean page. Protocol/unsupported
/// failures are deliberately outside this set.
fn startup_transient(reason: &str) -> bool {
    matches!(
        reason,
        "document-loading"
            | "composer-unavailable"
            | "interaction-settling"
            | "ui-busy"
            | "draft-present"
            | "conversation-busy"
    )
}

async fn try_startup_reload(registration: &mut Registration, context: ContextKey) {
    if registration.preload_first {
        // Windows treats renderer reload as a second cold start. Its fresh
        // launch permission authorizes first-document preload only.
        registration.startup = None;
        registration.report("document", "restart-required").await;
        return;
    }
    let Some(startup) = registration.startup.as_ref() else {
        registration.report("document", "restart-required").await;
        return;
    };
    if !startup.available() {
        registration.startup = None;
        registration
            .report("document", "startup-permission-expired")
            .await;
        return;
    }
    if registration
        .next_startup_check
        .is_some_and(|next| Instant::now() < next)
    {
        return;
    }
    registration.next_startup_check = Some(Instant::now() + Duration::from_millis(500));
    let mut readiness = registration
        .session()
        .evaluate(
            "window.__cockpitNativeFast.inspectStartupReload()".to_string(),
            context,
            COMMAND_TIMEOUT,
        )
        .await;
    if registration.session().context() != Some(context) {
        return;
    }
    if let Ok(value) = &readiness {
        if value.get("reason").and_then(Value::as_str) == Some("draft-present") {
            if let Some(snapshot) = value.get("draft").filter(|v| v.is_object()) {
                let snapshot = snapshot.clone();
                match startup_draft::persisted(&registration.draft_file, &snapshot).await {
                    Ok(true) if registration.session().context() == Some(context) => {
                        // This proof is only good for the exact renderer document
                        // that was read. The atomic final guard rechecks the PM
                        // document, idle state, attachments and activity again.
                        let expression =
                            format!("window.__cockpitStartupGuard.approveSavedDraft({snapshot})");
                        readiness = registration
                            .session()
                            .evaluate(expression, context, COMMAND_TIMEOUT)
                            .await;
                    }
                    Err(()) => {
                        registration.startup = None;
                        registration
                            .report("document", "draft-storage-timeout")
                            .await;
                        return;
                    }
                    _ => {}
                }
            }
        }
    }
    if registration.session().context() != Some(context) {
        return;
    }
    match readiness {
        Ok(value) if value.get("ok").and_then(Value::as_bool) == Some(true) => {}
        Ok(value) => {
            let reason = safe_reason(&value, "startup-inspection-failed");
            if !startup_transient(&reason) {
                registration.startup = None;
            }
            registration.report("document", &reason).await;
            return;
        }
        Err(_) => {
            registration.startup = None;
            registration
                .report("document", "startup-inspection-failed")
                .await;
            return;
        }
    }
    let startup = registration
        .startup
        .take()
        .expect("startup permission was inspected");
    if !startup.claim() {
        registration
            .report("document", "startup-permission-expired")
            .await;
        return;
    }
    // Claim is consumed before sending. A lost response can mean navigation
    // already started, so only a known explicit rejection may restore it.
    // The final JS helper checks and reloads in the same turn.
    let expression = format!(
        "window.__cockpitNativeFast.reloadPreparedDocument({})",
        json!(registration.nonce)
    );
    let result = registration
        .session()
        .evaluate(expression, context, COMMAND_TIMEOUT)
        .await;
    match result {
        Ok(value) if value.get("ok").and_then(Value::as_bool) == Some(true) => {
            registration.activation_until = Some(Instant::now() + ACTIVATION_TIMEOUT);
            registration.report("document", "reload-requested").await;
        }
        Ok(value) => {
            let reason = safe_reason(&value, "startup-page-not-clean");
            if value.get("ok").and_then(Value::as_bool) == Some(false)
                && startup_transient(&reason)
                && registration.session().context() == Some(context)
            {
                startup.release_rejected();
                registration.startup = Some(startup);
            }
            registration.report("document", &reason).await;
        }
        Err(_) => {
            registration.activation_until = Some(Instant::now() + ACTIVATION_TIMEOUT);
            registration
                .report("document", "reload-response-unavailable")
                .await;
        }
    }
}

async fn read_activation(
    registration: &mut Registration,
    context: ContextKey,
) -> Result<(), String> {
    if registration.active
        || registration
            .next_state_read
            .is_some_and(|next| Instant::now() < next)
    {
        return Ok(());
    }
    registration.next_state_read = Some(Instant::now() + Duration::from_millis(500));
    let expression = format!("(()=>{{const s=window.__cockpitEnhancements;return s?.owner==={} ? {{state:s.nativePatchState,reason:s.nativePatchReason,mode:s.nativePatchMode,features:s.nativePatchFeatures,quotaState:s.nativeQuotaPatchState,quotaReason:s.nativeQuotaPatchReason}} : {{state:'pending'}};}})()", json!(registration.owner));
    let result = match registration
        .session()
        .evaluate(expression, context, COMMAND_TIMEOUT)
        .await
    {
        Ok(value) => value,
        Err(_) if registration.session().context() != Some(context) => return Ok(()),
        Err(error) => return Err(error),
    };
    if registration.session().context() != Some(context) {
        return Ok(());
    }
    match result.get("state").and_then(Value::as_str) {
        Some("active") => {
            registration.active = true;
            registration.native_module_active = result
                .get("features")
                .and_then(Value::as_str)
                .is_some_and(|features| {
                    features.len() == 4
                        && features.bytes().all(|byte| byte == b'0' || byte == b'1')
                        && (result.get("mode").and_then(Value::as_str) == Some("full")
                            || result.get("mode").and_then(Value::as_str) == Some("fast-only")
                                && features.starts_with('1'))
                });
            registration.quota_unsupported_reason =
                (result.get("quotaState").and_then(Value::as_str) == Some("unsupported")).then(
                    || {
                        safe_reason(
                            &json!({"reason":result.get("quotaReason")}),
                            "quota-unsupported",
                        )
                    },
                );
            registration.capability_only = false;
            registration.activation_until = None;
            registration
                .report("activation", &safe_reason(&result, "module-evaluated"))
                .await;
        }
        Some("active-capability") => {
            registration.active = true;
            registration.capability_only = true;
            registration.activation_until = None;
            registration
                .report("activation", &safe_reason(&result, "capability-only"))
                .await;
        }
        Some("unsupported") => {
            registration.activation_until = None;
            registration
                .report(
                    "activation",
                    &safe_reason(&result, "module-load-unsupported"),
                )
                .await;
        }
        _ if registration
            .activation_until
            .is_some_and(|deadline| Instant::now() >= deadline) =>
        {
            registration.activation_until = None;
            registration
                .report("activation", "module-load-timeout")
                .await;
        }
        _ => {}
    }
    Ok(())
}

fn valid_quota_identifier(value: Option<&str>) -> Option<&str> {
    value.filter(|name| {
        !name.is_empty()
            && name.len() <= 64
            && name.bytes().enumerate().all(|(index, byte)| {
                byte == b'_'
                    || byte == b'$'
                    || byte.is_ascii_alphanumeric() && (index > 0 || !byte.is_ascii_digit())
            })
    })
}

fn live_quota_condition(hard: &str, guard: &str) -> String {
    format!(
        "(window.__cockpitEnhancements?.apiService===true&&window.__cockpitEnhancements?.quotaCompat===true&&({hard}=false,{guard}=false),false)"
    )
}

async fn register_live_runtime(
    registration: &mut Registration,
    context: ContextKey,
) -> Result<(), String> {
    let source = registration.runtime.clone();
    let reply = registration
        .session()
        .command(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source":source}),
            COMMAND_TIMEOUT,
        )
        .await
        .map_err(|error| command_reason(&error))?;
    if registration.session().context() != Some(context) {
        return Err("context-changed".to_string());
    }
    registration.script_id = Some(
        reply
            .pointer("/result/identifier")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "registration-failed".to_string())?,
    );
    registration
        .report("document", "live-runtime-registered")
        .await;
    Ok(())
}

async fn arm_live_quota_guard(
    registration: &mut Registration,
    context: ContextKey,
) -> Result<(), String> {
    let plan = registration
        .session()
        .evaluate(
            "window.__cockpitQuotaCompat.prepareLiveQuotaGate()".to_string(),
            context,
            PREPARE_TIMEOUT,
        )
        .await
        .map_err(|error| command_reason(&error))?;
    if registration.session().context() != Some(context) {
        return Err("context-changed".to_string());
    }
    if plan.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(safe_reason(&plan, "quota-location-unavailable"));
    }
    let location = plan
        .get("plan")
        .filter(|value| value.is_object())
        .ok_or_else(|| "quota-location-invalid".to_string())?;
    let url = location
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| local_primary(value))
        .ok_or_else(|| "quota-location-invalid".to_string())?;
    let line = location
        .get("lineNumber")
        .and_then(Value::as_u64)
        .filter(|value| *value <= u32::MAX as u64)
        .ok_or_else(|| "quota-location-invalid".to_string())?;
    let column = location
        .get("columnNumber")
        .and_then(Value::as_u64)
        .filter(|value| *value <= u32::MAX as u64)
        .ok_or_else(|| "quota-location-invalid".to_string())?;
    let hard = valid_quota_identifier(location.get("hardVariable").and_then(Value::as_str))
        .ok_or_else(|| "quota-location-invalid".to_string())?;
    let guard = valid_quota_identifier(location.get("guardVariable").and_then(Value::as_str))
        .ok_or_else(|| "quota-location-invalid".to_string())?;
    if registration.breakpoint_id.is_none() {
        registration
            .session()
            .command("Debugger.enable", json!({}), Duration::from_secs(15))
            .await
            .map_err(|error| command_reason(&error))?;
        let reply = registration
            .session()
            .command(
                "Debugger.setBreakpointByUrl",
                json!({
                    "lineNumber":line,
                    "columnNumber":column,
                    "urlRegex":format!("^{}$", regex::escape(url)),
                    "condition":live_quota_condition(hard, guard),
                }),
                Duration::from_secs(15),
            )
            .await
            .map_err(|error| command_reason(&error))?;
        registration.breakpoint_id = Some(
            reply
                .pointer("/result/breakpointId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "breakpoint-registration-failed".to_string())?,
        );
    }
    let expression = format!(
        "window.__cockpitQuotaCompat.installLiveQuotaGuard({})",
        location
    );
    let installed = registration
        .session()
        .evaluate(expression, context, COMMAND_TIMEOUT)
        .await
        .map_err(|error| command_reason(&error))?;
    if registration.session().context() != Some(context) {
        return Err("context-changed".to_string());
    }
    if installed.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(safe_reason(&installed, "quota-live-install-failed"));
    }
    if installed
        .get("preservedNativeFast")
        .and_then(Value::as_bool)
        == Some(true)
        || installed
            .get("preservedNativeModule")
            .and_then(Value::as_bool)
            == Some(true)
    {
        registration.active = false;
        registration.next_state_read = None;
        read_activation(registration, context).await?;
        return Ok(());
    }
    registration.live_quota_armed = true;
    registration.active = true;
    registration.activation_until = None;
    registration
        .report("activation", "quota-breakpoint-armed")
        .await;
    Ok(())
}

async fn maintain(registration: &mut Registration) -> Result<(), String> {
    let Some(context) = registration.session().context() else {
        return Ok(());
    };
    if registration.context != Some(context) {
        // A context replacement before our reload may be user navigation.
        // Do not turn its new page into another automatic startup reload.
        if registration.context.is_some() {
            registration.startup = None;
        }
        registration.context = Some(context);
        registration.document = Attempt::default();
        registration.bootstrapped = false;
        registration.active = false;
        registration.native_module_active = false;
        registration.quota_unsupported_reason = None;
        registration.capability_only = false;
        registration.live_quota_armed = false;
        registration.live_quota_attempt = Attempt::default();
        registration.next_state_read = None;
        registration.last_diagnostic = None;
        registration.activation_until = registration
            .script_id
            .as_ref()
            .map(|_| Instant::now() + ACTIVATION_TIMEOUT);
    }
    if !registration.bootstrapped {
        let location = registration
            .session()
            .evaluate("location.href".to_string(), context, COMMAND_TIMEOUT)
            .await?;
        if !location.as_str().is_some_and(local_document) {
            if registration.preload_first
                && registration.startup.is_some()
                && location
                    .as_str()
                    .is_some_and(|value| value.is_empty() || value == "about:blank")
            {
                prepare_startup_preload(registration, "preload-armed").await?;
                return Ok(());
            }
            return Err("native enhancement requires a local application target".to_string());
        }
        if registration.live_quota && !location.as_str().is_some_and(local_main_document) {
            return Ok(());
        }
        let expression = registration.runtime_probe.clone();
        let probe = registration
            .session()
            .evaluate(expression, context, COMMAND_TIMEOUT)
            .await?;
        if probe.get("preloading").and_then(Value::as_bool) == Some(true) {
            // The browser's completion closure still owns this installation.
            // Do not replace its owner until it has completed (or failed).
            registration.report("document", "browser-preloading").await;
            return Ok(());
        }
        let installed = probe
            .get("installed")
            .and_then(Value::as_bool)
            .or_else(|| probe.as_bool())
            == Some(true);
        if !installed {
            let runtime = registration.runtime.clone();
            registration
                .session()
                .evaluate(runtime, context, COMMAND_TIMEOUT)
                .await?;
        }
        if registration.session().context() != Some(context) {
            return Ok(());
        }
        registration.bootstrapped = true;
        read_activation(registration, context).await?;
    }
    if registration.live_quota {
        // A subsequently attached worker must not replace an evaluated Fast
        // module's state with the live-quota guard's narrower feature mask.
        if registration.native_module_active {
            return Ok(());
        }
        if registration.script_id.is_none()
            && registration.document.due()
            && registration.session().pending_additions.is_empty()
        {
            match register_live_runtime(registration, context).await {
                Ok(()) => registration.document.done = true,
                Err(reason) => {
                    registration.document.failed(matches!(
                        reason.as_str(),
                        "context-changed" | "evaluation-timeout" | "transport-failed"
                    ));
                    registration.report("document", &reason).await;
                }
            }
        }
        if !registration.live_quota_armed && registration.live_quota_attempt.due() {
            match arm_live_quota_guard(registration, context).await {
                Ok(()) => {}
                Err(reason) => {
                    let retryable = matches!(
                        reason.as_str(),
                        "quota-location-failed"
                            | "quota-location-unavailable"
                            | "context-changed"
                            | "evaluation-timeout"
                            | "transport-failed"
                    ) || reason.starts_with("debugger-enable-")
                        || reason.starts_with("breakpoint-registration-");
                    registration.live_quota_attempt.failed(retryable);
                    registration.report("activation", &reason).await;
                }
            }
        }
        return Ok(());
    }
    if registration.script_id.is_none()
        && registration.document.due()
        && registration.session().pending_additions.is_empty()
    {
        let prepared = if registration.preload_first {
            prepare_startup_preload(registration, "preload-registered-next-document").await
        } else {
            prepare_future_document(registration, context).await
        };
        match prepared {
            Ok(()) => {
                registration.document.done = true;
                registration.report("document", "registered").await;
                if !registration.active && registration.session().context() == Some(context) {
                    try_startup_reload(registration, context).await;
                }
            }
            Err(reason) => {
                let retryable = matches!(
                    reason.as_str(),
                    "unsupported-entry"
                        | "asset-unavailable"
                        | "context-changed"
                        | "evaluation-timeout"
                        | "transport-failed"
                );
                registration.document.failed(retryable);
                registration.report("document", &reason).await;
            }
        }
    }
    if registration.activation_until.is_some() && registration.session().context() == Some(context)
    {
        read_activation(registration, context).await?;
    }
    if registration.script_id.is_some()
        && !registration.active
        && registration.startup.is_some()
        && registration.session().context() == Some(context)
    {
        try_startup_reload(registration, context).await;
    }
    Ok(())
}

async fn initialize(registration: &mut Registration) -> Result<(), String> {
    registration
        .session()
        .command("Page.enable", json!({}), COMMAND_TIMEOUT)
        .await?;
    let tree = registration
        .session()
        .command("Page.getFrameTree", json!({}), COMMAND_TIMEOUT)
        .await?;
    registration.session().main_frame = tree
        .pointer("/result/frameTree/frame/id")
        .and_then(Value::as_str)
        .map(str::to_string);
    registration
        .session()
        .command("Runtime.enable", json!({}), COMMAND_TIMEOUT)
        .await?;
    maintain(registration).await
}

/// The parent verifies profile credentials and distinguishes a fresh process
/// launch from restoring an already-running target before supplying startup.
pub(super) async fn run(
    websocket_url: String,
    profile_dir: PathBuf,
    native_fast: bool,
    quota_compat: bool,
    api_key_auth_verified: bool,
    api_service: bool,
    preload_first: bool,
    startup: Option<Arc<StartupReload>>,
    stop: watch::Receiver<bool>,
) -> Result<(), String> {
    // The profile reader has verified either pure API authentication or an
    // independent API route with OAuth retained. Only those profiles can reuse
    // a launch transaction's single fresh-document permission.
    let verified_api_route = native_fast && api_key_auth_verified || quota_compat;
    let startup = if api_service && !verified_api_route {
        None
    } else {
        startup
    };
    // Fresh launches install the complete validated module graph. Reattaching
    // to an existing page never creates permission to reload user work.
    let live_quota = cfg!(any(target_os = "windows", target_os = "macos"))
        && api_service
        && quota_compat
        && startup.is_none();
    let preload_first =
        preload_first || (cfg!(target_os = "windows") && verified_api_route && startup.is_some());
    run_with_live_quota(
        websocket_url,
        profile_dir,
        native_fast,
        quota_compat,
        api_key_auth_verified,
        api_service,
        preload_first,
        startup,
        stop,
        live_quota,
    )
    .await
}

async fn run_with_live_quota(
    websocket_url: String,
    profile_dir: PathBuf,
    native_fast: bool,
    quota_compat: bool,
    api_key_auth_verified: bool,
    api_service: bool,
    preload_first: bool,
    startup: Option<Arc<StartupReload>>,
    mut stop: watch::Receiver<bool>,
    live_quota: bool,
) -> Result<(), String> {
    if !native_fast && !quota_compat && !api_service || *stop.borrow() {
        return Ok(());
    }
    if !local_websocket(&websocket_url) {
        return Err("native enhancement requires a loopback page target".to_string());
    }
    let config = WebSocketConfig::default()
        .max_message_size(Some(64 * 1024 * 1024))
        .max_frame_size(Some(64 * 1024 * 1024));
    let browser_port = url::Url::parse(&websocket_url)
        .ok()
        .and_then(|url| url.port_or_known_default())
        .ok_or("invalid browser port")?;
    let (socket, _) = timeout(
        COMMAND_TIMEOUT,
        connect_async_with_config(websocket_url, Some(config), false),
    )
    .await
    .map_err(|_| "native enhancement connection timed out".to_string())?
    .map_err(|_| "native enhancement connection failed".to_string())?;
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    let owner = format!(
        "{}-{}",
        std::process::id(),
        GENERATION.fetch_add(1, Ordering::Relaxed)
    );
    let runtime = bootstrap(
        native_fast,
        quota_compat,
        api_key_auth_verified,
        api_service,
        &owner,
    );
    let startup_runtime = guard_document_install(&format!("{runtime}\n{PRELOAD_SCRIPT}"), "");
    let browser_snapshot = json!({"owner":format!("preload-{browser_port}"),
        "nativeFast":native_fast,"quotaCompat":quota_compat,
        "apiKeyAuthVerified":api_key_auth_verified,"apiService":api_service});
    let runtime_probe = RUNTIME_PROBE_SCRIPT
        .replace(
            "__COCKPIT_BROWSER_SNAPSHOT__",
            &browser_snapshot.to_string(),
        )
        .replace("__COCKPIT_WORKER_OWNER__", &json!(owner).to_string());

    let mut registration = Registration {
        session: Some(Session {
            socket,
            command_id: 0,
            contexts: HashMap::new(),
            main_frame: None,
            generation: 0,
            registrations: HashSet::new(),
            pending_additions: HashSet::new(),
            pending_removals: HashMap::new(),
        }),
        script_id: None,
        owner,
        runtime,
        startup_runtime,
        runtime_probe,
        preload_first,
        live_quota,
        live_quota_armed: false,
        breakpoint_id: None,
        nonce: uuid::Uuid::new_v4().to_string(),
        startup,
        draft_file: profile_dir.join(".codex-global-state.json"),
        context: None,
        document: Attempt::default(),
        live_quota_attempt: Attempt::default(),
        bootstrapped: false,
        active: false,
        native_module_active: false,
        quota_unsupported_reason: None,
        capability_only: false,
        activation_until: None,
        next_state_read: None,
        next_startup_check: None,
        last_diagnostic: None,
    };
    let initialized = tokio::select! { biased; _ = stop.changed() => Ok(()), result = initialize(&mut registration) => result };
    if initialized.is_err() || *stop.borrow() || stop.has_changed().is_err() {
        registration.cleanup().await;
        return initialized;
    }
    let mut retry = tokio::time::interval(Duration::from_millis(250));
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        if *stop.borrow() {
            break Ok(());
        }
        tokio::select! {
            biased;
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break Ok(()); } }
            _ = retry.tick() => {
                let result = tokio::select! { biased; _ = stop.changed() => break Ok(()), result = maintain(&mut registration) => result };
                if let Err(error) = result { break Err(error); }
            }
            incoming = registration.session().receive() => {
                match incoming { Ok(Some(_)) => {}, Ok(None) => break Ok(()), Err(error) => break Err(error) }
            }
        }
    };
    registration.cleanup().await;
    result
}

fn local_websocket(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
    url.scheme() == "ws"
        && loopback
        && url.port().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().starts_with("/devtools/page/")
        && url.path().len() > "/devtools/page/".len()
}

fn local_document(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    (url.scheme() == "app" && url.host_str() == Some("-")
        || url.scheme() == "file" && url.host_str().is_none())
        && url.username().is_empty()
        && url.password().is_none()
}

fn local_main_document(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    local_document(value)
        && url.path() == "/index.html"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

fn local_initial(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    let name = url.path().rsplit('/').next().unwrap_or_default();
    local_document(value)
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().contains("/assets/")
        && name.starts_with("app-initial-")
        && name.ends_with(".js")
        && name.len() > "app-initial-.js".len()
}

fn local_primary(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    let name = url.path().rsplit('/').next().unwrap_or_default();
    local_document(value)
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().contains("/assets/")
        && name.starts_with("app-primary-")
        && name.ends_with(".js")
        && name.len() > "app-primary-.js".len()
}

#[cfg(test)]
#[path = "fast_tests.rs"]
mod tests;
