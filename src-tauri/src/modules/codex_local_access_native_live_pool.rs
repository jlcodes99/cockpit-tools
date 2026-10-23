//! The real-pool listener keeps mutable state in a private snapshot. A separate
//! ignored helper prepares current credentials through the normal refresh transaction.
//! Request payloads, headers, credentials and production metrics are never recorded here.
use super::*;
use futures_util::future::BoxFuture;
use native_contract::{
    Caller, CapabilityFallbackDecision, ExecutionPolicy, Host, MultiAgentPolicy,
    OfficialFallbackFailure, RequestReport, Route, RouteRequest, ServiceError,
};
use serde::Deserialize;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize};

#[derive(Deserialize)]
struct Config {
    source_id: String,
    registration_id: String,
    key: String,
    model: String,
    output_dir: PathBuf,
}

struct PoolHost {
    inner: NativeHostBridge,
    source: String,
    registration: String,
    caller_id: String,
    model: String,
    first_selection: AtomicBool,
    selections: AtomicUsize,
    reports: Mutex<std::fs::File>,
}

impl PoolHost {
    fn candidate(&self, model: &str) -> Result<(), ServiceError> {
        if model != self.model {
            return Err(native_error(
                400,
                "acceptance_model_mismatch",
                "Only the configured acceptance model is allowed",
            ));
        }
        if self.selections.fetch_add(1, Ordering::AcqRel) >= 20 {
            return Err(native_error(
                429,
                "acceptance_candidate_limit",
                "Acceptance selection safety limit reached",
            ));
        }
        Ok(())
    }
}

impl Host for PoolHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            let caller = self.inner.authorize(bearer).await?;
            if caller.id != self.caller_id {
                return Err(native_error(
                    403,
                    "acceptance_caller_mismatch",
                    "Only the configured acceptance caller is allowed",
                ));
            }
            Ok(caller)
        })
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }
    fn multi_agent_policy<'a>(
        &'a self,
        caller: &'a Caller,
        route: Option<&'a Route>,
        model: &'a str,
    ) -> BoxFuture<'a, Result<MultiAgentPolicy, ServiceError>> {
        self.inner.multi_agent_policy(caller, route, model)
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            self.candidate(&request.model)?;
            let first = self.first_selection.swap(false, Ordering::AcqRel);
            let mut selection_request = request;
            if first {
                if selection_request.preferred_account_id.is_some()
                    || selection_request.target_account_id.is_some()
                    || !selection_request.excluded_account_ids.is_empty()
                {
                    return Err(native_error(
                        400,
                        "acceptance_requires_fresh_history",
                        "The first acceptance selection must have fresh unpinned history",
                    ));
                }
                // This local selection hint never changes mapping's original
                // request or Prepared, so later replay uses the real pool scope.
                selection_request.preferred_account_id = Some(self.source.clone());
            }
            let route = self.inner.route(caller, selection_request).await?;
            if first
                && (!route.chatgpt_plan
                    || route.account_id != self.source
                    || route.official_registration_id.as_deref()
                        != Some(self.registration.as_str()))
            {
                return Err(native_error(400, "acceptance_initial_official_unavailable", "The selected source cannot currently use its saved official authorization; acceptance is not applicable"));
            }
            Ok(route)
        })
    }
    fn refresh_route<'a>(
        &'a self,
        caller: &'a Caller,
        previous: &'a Route,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            self.candidate(&request.model)?;
            self.inner.refresh_route(caller, previous, request).await
        })
    }
    fn fallback_to_local<'a>(
        &'a self,
        caller: &'a Caller,
        failed: &'a Route,
        request: RouteRequest,
        failure: OfficialFallbackFailure,
    ) -> BoxFuture<'a, Result<Option<Route>, ServiceError>> {
        Box::pin(async move {
            self.candidate(&request.model)?;
            self.inner
                .fallback_to_local(caller, failed, request, failure)
                .await
        })
    }
    fn observe_official_failure<'a>(
        &'a self,
        route: &'a Route,
        model: &'a str,
        failure: &'a OfficialFallbackFailure,
    ) -> BoxFuture<'a, ()> {
        self.inner.observe_official_failure(route, model, failure)
    }
    fn capability_fallback_decision<'a>(
        &'a self,
        caller: &'a Caller,
        route: &'a Route,
        request: &'a RouteRequest,
    ) -> BoxFuture<'a, Result<CapabilityFallbackDecision, ServiceError>> {
        self.inner
            .capability_fallback_decision(caller, route, request)
    }
    fn prepare_body<'a>(
        &'a self,
        caller: &'a Caller,
        route: &'a Route,
        headers: &'a axum::http::HeaderMap,
        body: Value,
        image: bool,
        compact: bool,
    ) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner
            .prepare_body(caller, route, headers, body, image, compact)
    }
    fn token_count_model<'a>(
        &'a self,
        caller: &'a Caller,
        model: &'a str,
    ) -> BoxFuture<'a, Result<String, ServiceError>> {
        self.inner.token_count_model(caller, model)
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        self.inner.execution_policy()
    }
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, ServiceError>> {
        self.inner.websockets_enabled()
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        caller: &'a Caller,
        route: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        self.inner.responses_websocket_passthrough(caller, route)
    }
    fn invalidate_transport(&self, identity: &str) -> bool {
        self.inner.invalidate_transport(identity)
    }
    fn set_cooling_disabled(&self, disabled: bool) {
        self.inner.set_cooling_disabled(disabled);
    }
    fn set_session_affinity_enabled(&self, enabled: bool) {
        self.inner.set_session_affinity_enabled(enabled);
    }
    fn observe_rate_limits(
        &self,
        account: &str,
        snapshot: native_api::rate_limits::QuotaSnapshot,
        started: Instant,
    ) {
        self.inner.observe_rate_limits(account, snapshot, started);
    }
    fn route_failed_attempt<'a>(
        &'a self,
        route: &'a Route,
        model: &'a str,
        status: u16,
        wait: Option<Duration>,
        credential_scope: bool,
        message: Option<&'a str>,
    ) -> BoxFuture<'a, ()> {
        self.inner
            .route_failed_attempt(route, model, status, wait, credential_scope, message)
    }
    fn route_failed_with_context<'a>(
        &'a self,
        account: &'a str,
        model: &'a str,
        status: u16,
        wait: Option<Duration>,
        credential_scope: bool,
        message: Option<&'a str>,
    ) -> BoxFuture<'a, ()> {
        self.inner.route_failed_with_context(
            account,
            model,
            status,
            wait,
            credential_scope,
            message,
        )
    }
    fn retry_cooldown_wait<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
        max_wait: Duration,
    ) -> BoxFuture<'a, Result<Option<Duration>, ServiceError>> {
        self.inner.retry_cooldown_wait(caller, request, max_wait)
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.inner.record_failure_cooldown(&report);
            self.inner.record_soft_binding_result(&report).await;
            let attempts = report
                .diagnostics
                .as_ref()
                .map(|context| {
                    context.attempts.iter().map(|attempt| {
                let fallback = attempt.authorization_fallback.as_ref().map(|trace| json!({
                    "trigger_stage":trace.trigger_stage,"reason":trace.reason,
                    "from_account_id":trace.from_account_id,"to_account_id":trace.to_account_id,
                    "from_route":trace.from_route,"to_route":trace.to_route,
                    "status":trace.status,"error_code":trace.error_code
                }));
                json!({"sequence":attempt.sequence,"account_id":attempt.account_id,
                    "model":attempt.model_id,"status":attempt.status,"success":attempt.success,
                    "authorization_route":attempt.authorization_route,
                    "error_category":attempt.error_category,"failure_phase":attempt.failure_phase,
                    "fallback":fallback})
            }).collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let evidence = json!({"status":report.status,"success":report.success,
                "account_id":report.account_id,"model":report.upstream_model,
                "retry_after_ms":report.retry_after.map(|wait| wait.as_millis().min(u64::MAX as u128) as u64),
                "error_category":report.error,"attempts":attempts});
            if let Ok(mut file) = self.reports.lock() {
                if writeln!(file, "{evidence}")
                    .and_then(|_| file.flush())
                    .is_err()
                {
                    eprintln!("ACCEPTANCE_REPORT_WRITE_FAILED");
                }
            }
        })
    }
}

fn private_file(path: &Path, append: bool) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn read_private_config() -> Result<Config, Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::var_os("COCKPIT_NATIVE_POOL_LIVE_CONFIG")
            .ok_or("Missing private pool acceptance configuration")?,
    );
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1 << 20 {
        return Err("Invalid pool acceptance configuration".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Pool acceptance configuration must be private".into());
        }
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit COCKPIT_NATIVE_POOL_LIVE_CONFIG; consumes existing real account-pool usage"]
async fn native_pool_process_driver() -> Result<(), Box<dyn std::error::Error>> {
    let config = read_private_config()?;
    let snapshot = PathBuf::from(
        std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR")
            .ok_or("Pool acceptance requires an explicit private data snapshot")?,
    )
    .canonicalize()?;
    let marker = std::fs::symlink_metadata(snapshot.join(".native-pool-live-snapshot"))?;
    if !marker.is_file()
        || marker.file_type().is_symlink()
        || crate::modules::account::get_data_dir()?.canonicalize()? != snapshot
    {
        return Err("Pool acceptance snapshot guard failed".into());
    }
    std::fs::create_dir_all(&config.output_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config.output_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let identity = config.output_dir.join("history.sqlite");
    if identity.exists() {
        return Err("Pool acceptance requires a new private history database".into());
    }
    let mut collection: CodexLocalAccessCollection =
        serde_json::from_slice(&std::fs::read(local_access_file_path()?)?)?;
    if !collection.enabled
        || !collection.account_ids.contains(&config.source_id)
        || collection.official_account_bindings.get(&config.source_id)
            != Some(&config.registration_id)
    {
        return Err("Pool acceptance source is not an enabled saved official member".into());
    }
    // All memberships, keys, scopes, model limits and bindings remain as saved.
    // Logging alone is disabled for this private acceptance listener.
    collection.debug_logs = false;
    collection.request_payload_logging = false;
    crate::modules::chatgpt_plan::validate_source_binding(
        &config.registration_id,
        &config.source_id,
    )
    .await?;
    let inner = NativeHostBridge::for_collection(collection);
    let caller = inner.authorize(&config.key).await?;
    let identity_snapshot = snapshot.join("codex_api_account_identity.sqlite");
    let identity_seeded = if identity_snapshot.exists() {
        let metadata = std::fs::symlink_metadata(&identity_snapshot)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Invalid installation identity snapshot".into());
        }
        // The supplied snapshot must already be a consistent SQLite backup.
        // The normal test Host still owns and mutates only its private fixture.
        std::fs::copy(identity_snapshot, inner.identity_fixture.path())?;
        true
    } else {
        false
    };
    let host = Arc::new(PoolHost {
        inner,
        source: config.source_id,
        registration: config.registration_id,
        caller_id: caller.id,
        model: config.model,
        first_selection: AtomicBool::new(true),
        selections: AtomicUsize::new(0),
        reports: Mutex::new(private_file(
            &config.output_dir.join("reports.jsonl"),
            true,
        )?),
    });
    let mut handle = native_api::start(
        "127.0.0.1:0".parse()?,
        host,
        identity,
        native_api::Options::default(),
    )
    .await?;
    let mut ready = private_file(&config.output_dir.join("ready.json"), false)?;
    write!(
        ready,
        "{}",
        json!({"port":handle.addr.port(),"pid":std::process::id(),
        "production_host":true,"isolated_pool":true,"identity_seeded":identity_seeded})
    )?;
    ready.flush()?;
    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    std::thread::spawn(move || {
        let line = std::io::stdin().lock().lines().next();
        let _ = send.blocking_send(line.and_then(Result::ok));
    });
    let _ = receive.recv().await;
    let stopped = handle.shutdown(Duration::from_secs(10)).await;
    crate::modules::codex_proxy_engine::shutdown_all();
    stopped?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Explicit real-data credential preparation before creating an isolated live-pool snapshot"]
async fn native_pool_live_prepare_credentials() -> Result<(), Box<dyn std::error::Error>> {
    let config = read_private_config()?;
    let data = PathBuf::from(
        std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR")
            .ok_or("Credential preparation requires an explicit real account data directory")?,
    )
    .canonicalize()?;
    if data.join(".native-pool-live-snapshot").exists()
        || crate::modules::account::get_data_dir()?.canonicalize()? != data
    {
        return Err(
            "Credential preparation must run separately before creating the private snapshot"
                .into(),
        );
    }
    let collection: CodexLocalAccessCollection =
        serde_json::from_slice(&std::fs::read(local_access_file_path()?)?)?;
    if !collection.account_ids.contains(&config.source_id)
        || collection.official_account_bindings.get(&config.source_id)
            != Some(&config.registration_id)
    {
        return Err("Credential preparation source is not the saved official member".into());
    }
    // Use the normal cross-process credential lock and durable refresh transaction.
    // This separate process never changes directory or service configuration,
    // and never clears route health; normal refresh owns its credential outcome.
    match tokio::time::timeout(
        Duration::from_secs(60),
        crate::modules::chatgpt_plan::inference_snapshot_for_source(
            &config.registration_id,
            &config.source_id,
            false,
        ),
    )
    .await
    {
        Ok(Ok(snapshot)) => {
            drop(snapshot);
            println!("ACCEPTANCE_CREDENTIALS_READY");
            Ok(())
        }
        Ok(Err(code)) => {
            let safe = if code.len() <= 128
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            {
                code.as_str()
            } else {
                "CHATGPT_PLAN_CREDENTIAL_PREPARATION_FAILED"
            };
            eprintln!("ACCEPTANCE_CREDENTIAL_PREPARATION_FAILED:{safe}");
            Err("Credential preparation failed".into())
        }
        Err(_) => Err("ACCEPTANCE_CREDENTIAL_PREPARATION_TIMEOUT".into()),
    }
}
