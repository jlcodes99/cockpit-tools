//! Explicit real-upstream driver. Production Host routing and wire behavior,
//! one existing official grant, private loopback key and separate history DB.
//! Metrics stay in the acceptance directory, never the user's service pool.
use super::*;
use futures_util::future::BoxFuture;
use native_contract::{
    Caller, ExecutionPolicy, Host, MultiAgentPolicy, RequestReport, Route, RouteRequest,
    ServiceError,
};
use serde::Deserialize;
use std::io::{BufRead, Write};

#[derive(Deserialize)]
struct Config {
    source_id: String,
    registration_id: String,
    key: String,
    model: String,
    output_dir: PathBuf,
    max_requests: usize,
}

struct AcceptanceHost {
    inner: NativeHostBridge,
    registration: String,
    source: String,
    model: String,
    remaining: Arc<Semaphore>,
    reports: Mutex<std::fs::File>,
}

impl Host for AcceptanceHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(bearer)
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
            if request.model != self.model {
                return Err(native_error(
                    400,
                    "acceptance_model_mismatch",
                    "Only the selected acceptance model is allowed",
                ));
            }
            self.remaining
                .clone()
                .try_acquire_owned()
                .map_err(|_| {
                    native_error(
                        429,
                        "acceptance_budget_exhausted",
                        "Acceptance request budget reached",
                    )
                })?
                .forget();
            let route = self.inner.route(caller, request).await?;
            if !route.chatgpt_plan
                || route.official_registration_id.as_deref() != Some(self.registration.as_str())
                || route.account_id != self.source
            {
                return Err(native_error(
                    403,
                    "acceptance_route_mismatch",
                    "Acceptance must use the explicitly selected official grant",
                ));
            }
            Ok(route)
        })
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
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        self.inner.execution_policy()
    }
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, ServiceError>> {
        self.inner.websockets_enabled()
    }
    fn invalidate_transport(&self, identity: &str) -> bool {
        self.inner.invalidate_transport(identity)
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
        self.inner.route_failed_attempt(
            route,
            model,
            status,
            retry_after,
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
            let evidence = json!({"status":report.status,"success":report.success,"error":report.error,
                "official":report.official_registration_id.as_deref() == Some(self.registration.as_str()),
                "source_matches":report.account_id == self.source,"model":report.upstream_model,
                "elapsed_ms":report.elapsed.as_millis(),"usage":report.usage,
                "request_id":report.diagnostics.as_ref().map(|d| &d.request_id),
                "thread":report.diagnostics.as_ref().and_then(|d| d.client_thread_id.as_deref())});
            let mut file = self.reports.lock().unwrap();
            writeln!(file, "{evidence}").unwrap();
            file.flush().unwrap();
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit COCKPIT_OFFICIAL_LIVE_CONFIG; consumes existing official grant usage"]
async fn official_native_host_process_driver() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::var_os("COCKPIT_OFFICIAL_LIVE_CONFIG")
            .ok_or("Missing private acceptance configuration")?,
    );
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1 << 20 {
        return Err("Invalid acceptance configuration".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Acceptance configuration must be private".into());
        }
    }
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    if config.key.len() < 32 || !(1..=64).contains(&config.max_requests) {
        return Err("Invalid acceptance budget/key".into());
    }
    crate::modules::chatgpt_plan::validate_source_binding(
        &config.registration_id,
        &config.source_id,
    )
    .await?;
    let mut collection: CodexLocalAccessCollection =
        serde_json::from_slice(&std::fs::read(local_access_file_path()?)?)?;
    if collection.official_account_bindings.get(&config.source_id) != Some(&config.registration_id)
    {
        return Err("Selected grant is not the saved source binding".into());
    }
    collection.enabled = true;
    collection.api_key = config.key;
    collection.api_keys.clear();
    collection.account_ids = vec![config.source_id.clone()];
    collection
        .official_account_bindings
        .retain(|source, _| source == &config.source_id);
    collection.responses_websockets_enabled = true;
    collection.debug_logs = false;
    collection.request_payload_logging = false;
    let reports = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config.output_dir.join("requests.jsonl"))?;
    let host = Arc::new(AcceptanceHost {
        inner: NativeHostBridge::for_collection(collection),
        registration: config.registration_id,
        source: config.source_id,
        model: config.model,
        remaining: Arc::new(Semaphore::new(config.max_requests)),
        reports: Mutex::new(reports),
    });
    let identity = config.output_dir.join("history.sqlite");
    let options = native_api::Options::default();
    let mut handle = native_api::start(
        "127.0.0.1:0".parse()?,
        host.clone(),
        identity.clone(),
        options.clone(),
    )
    .await?;
    let ready = config.output_dir.join("ready.json");
    let publish = |addr: std::net::SocketAddr| {
        std::fs::write(&ready, json!({"port":addr.port(),"pid":std::process::id(),"production_host":true,"official_only":true}).to_string())
    };
    publish(handle.addr)?;
    let (send, mut receive) = tokio::sync::mpsc::channel(8);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else {
                return;
            };
            if send.blocking_send(line).is_err() {
                return;
            }
        }
    });
    while let Some(command) = receive.recv().await {
        if command.trim() == "restart" {
            let addr = handle.addr;
            handle.shutdown(Duration::from_secs(10)).await?;
            handle =
                native_api::start(addr, host.clone(), identity.clone(), options.clone()).await?;
            publish(handle.addr)?;
            println!("ACCEPTANCE_RESTARTED");
        } else {
            break;
        }
    }
    handle.shutdown(Duration::from_secs(10)).await?;
    Ok(())
}
