//! Explicit real-upstream acceptance. Secrets stay in memory; only synthetic
//! prompts and protocol evidence leave the isolated listener.
use super::*;
use futures_util::future::BoxFuture;
use native_contract::{
    Caller, ExecutionPolicy, Host, MultiAgentPolicy, RequestReport, Route, RouteRequest,
    ServiceError,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    phase: String,
    account_id: Option<String>,
    allowed_accounts: Vec<String>,
    service_port: u16,
    model: String,
    output: PathBuf,
    #[serde(default)]
    force_websocket: bool,
}

struct ProtocolHost {
    inner: NativeHostBridge,
    model: String,
    bridge: Arc<AtomicBool>,
    reports: Arc<Mutex<Vec<Value>>>,
    remaining: std::sync::atomic::AtomicUsize,
    selected_account: String,
}
impl Host for ProtocolHost {
    fn invalidate_http2_transport(&self, identity: &str) -> bool {
        self.inner.invalidate_http2_transport(identity)
    }
    fn invalidate_transport(&self, identity: &str) -> bool {
        self.inner.invalidate_transport(identity)
    }
    fn quota_request_started(&self, account: &str) {
        self.inner.quota_request_started(account)
    }
    fn check_quota_before_send(&self, account: &str) -> Result<(), ServiceError> {
        self.inner.check_quota_before_send(account)
    }
    fn observe_rate_limits(
        &self,
        account: &str,
        snapshot: native_api::rate_limits::QuotaSnapshot,
        started: std::time::Instant,
    ) {
        self.inner.observe_rate_limits(account, snapshot, started)
    }
    fn route_failed_attempt<'a>(
        &'a self,
        route: &'a Route,
        model: &'a str,
        status: u16,
        wait: Option<Duration>,
        scope: bool,
        message: Option<&'a str>,
    ) -> BoxFuture<'a, ()> {
        self.inner
            .route_failed_attempt(route, model, status, wait, scope, message)
    }
    fn route_transient_rate_limited<'a>(
        &'a self,
        route: &'a Route,
        model: &'a str,
        wait: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        self.inner.route_transient_rate_limited(route, model, wait)
    }
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(key)
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
            if request.model != self.model
                || self
                    .remaining
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                    .is_err()
            {
                return Err(native_error(
                    429,
                    "acceptance_limit",
                    "Acceptance model/request limit reached",
                ));
            }
            let route = self.inner.route(caller, request).await?;
            if route.account_id != self.selected_account {
                return Err(native_error(
                    403,
                    "acceptance_scope_violation",
                    "Account is outside the selected service scope",
                ));
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
            let route = self.inner.refresh_route(caller, previous, request).await?;
            if route.account_id != self.selected_account {
                return Err(native_error(
                    403,
                    "acceptance_scope_violation",
                    "Retry account is outside the selected service scope",
                ));
            }
            Ok(route)
        })
    }
    fn validate_websocket_control<'a>(
        &'a self,
        caller: &'a Caller,
        route: &'a Route,
        request: &'a RouteRequest,
    ) -> BoxFuture<'a, Result<(), ServiceError>> {
        self.inner
            .validate_websocket_control(caller, route, request)
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        caller: &'a Caller,
        route: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async move {
            Ok(!self.bridge.load(Ordering::Acquire)
                && self
                    .inner
                    .responses_websocket_passthrough(caller, route)
                    .await?)
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
        Box::pin(async move {
            let body = self
                .inner
                .prepare_body(caller, route, headers, body, image, compact)
                .await?;
            self.reports.lock().unwrap().push(json!({"kind":"prepared","control":body["type"],"has_image_tool":native_contains_image_tool(&body)}));
            Ok(body)
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                open_timeout: Duration::from_secs(35),
                idle_timeout: Duration::from_secs(45),
                total_timeout: Duration::from_secs(90),
                credential_retries: 1,
                send_retries: 0,
                status_retries: 0,
                bootstrap_retries: 0,
                websocket_initial_timeout: Duration::from_secs(35),
                websocket_idle_timeout: Duration::from_secs(45),
                ..Default::default()
            })
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.inner.record_failure_cooldown(&report);
            self.inner.record_soft_binding_result(&report).await;
            let attempts:Vec<_>=report.diagnostics.as_ref().into_iter().flat_map(|d|&d.attempts).map(|a|json!({"transport":a.transport,"status":a.status,"success":a.success,"wire_bytes":a.upstream_request_bytes,"error":a.error_category,"http_version":a.upstream_http_version})).collect();
            self.reports.lock().unwrap().push(json!({"status":report.status,"success":report.success,"error":report.error,"elapsed_ms":report.elapsed.as_millis(),"attempts":attempts,"usage":report.usage.as_ref().map(|usage|json!({"input_tokens":usage["input_tokens"],"output_tokens":usage["output_tokens"],"total_tokens":usage["total_tokens"]}))}));
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit COCKPIT_PROTOCOL_LIVE_CONFIG; uses selected saved account and real upstream quota"]
async fn native_live_protocol_acceptance() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = PathBuf::from(
        std::env::var_os("COCKPIT_PROTOCOL_LIVE_CONFIG").ok_or("Missing explicit config")?,
    );
    let config: Config = serde_json::from_slice(&std::fs::read(config_path)?)?;
    let data = PathBuf::from(
        std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR")
            .ok_or("Missing explicit real data directory")?,
    );
    if crate::modules::account::get_data_dir()?.canonicalize()? != data.canonicalize()? {
        return Err("Data directory mismatch".into());
    }
    std::fs::create_dir_all(&config.output)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config.output, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut collection: CodexLocalAccessCollection =
        serde_json::from_slice(&std::fs::read(data.join("codex_local_access.json"))?)?;
    if collection.port != config.service_port
        || config.allowed_accounts.len() != 9
        || config
            .allowed_accounts
            .iter()
            .any(|id| !collection.account_ids.contains(id))
    {
        return Err("The selected service membership changed; no live request was sent".into());
    }
    let mut accounts = HashMap::new();
    let mut inventory = Vec::new();
    // Membership may grow while acceptance is running. New members are never
    // read or selected: authorization is the original explicit allowlist.
    for id in &config.allowed_accounts {
        let path = data.join("codex_accounts").join(format!("{id}.json"));
        let contents = std::fs::read_to_string(&path)?;
        let (account, _): (CodexAccount, bool) =
            crate::modules::secure_account_storage::deserialize_account_file(&path, &contents)?;
        let token_expiry = account
            .tokens
            .access_token
            .split('.')
            .nth(1)
            .and_then(|v| {
                use base64::Engine;
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(v)
                    .ok()
            })
            .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
            .and_then(|v| v["exp"].as_i64());
        inventory.push(json!({"id":id,"oauth":!account.is_api_key_auth(),"reauth":account.requires_reauth,"eligible":native_eligible(&account),"quota_remaining":native_quota(&account),"token_valid":token_expiry.is_some_and(|exp|exp>chrono::Utc::now().timestamp()+120),"websockets":!account.is_api_key_auth()||account.api_supports_websockets,
            "wire_api":account.api_wire_api,"provider_mode":account.api_provider_mode,"models":account.api_model_catalog,"model_mappings":account.api_model_mappings,"startup_model":account.api_startup_model,
            "upstream_host":account.api_base_url.as_deref().and_then(|s|url::Url::parse(s).ok()).and_then(|u|u.host_str().map(str::to_owned))}));
        accounts.insert(id.clone(), account);
    }
    std::fs::write(
        config.output.join("inventory.json"),
        serde_json::to_vec_pretty(&inventory)?,
    )?;
    if config.phase == "inspect" {
        println!("LIVE_PROTOCOL_INVENTORY_READY");
        return Ok(());
    }
    let id = config.account_id.ok_or("Select an account explicitly")?;
    let mut account = accounts
        .remove(&id)
        .ok_or("Account is not a configured service member")?;
    if !native_eligible(&account) {
        return Err("Selected account is not eligible".into());
    }
    // Probe only this account's capability without changing saved settings.
    if config.force_websocket {
        account.api_supports_websockets = true;
    }
    let key = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    collection.api_key = key.clone();
    collection.api_keys.clear();
    collection.account_ids = vec![id.clone()];
    collection.official_account_bindings.clear();
    collection.official_account_source_ids.clear();
    collection.enabled = true;
    collection.responses_websockets_enabled = true;
    collection.debug_logs = false;
    collection.request_payload_logging = false;
    collection.max_retry_credentials = 1;
    collection.max_retry_interval_ms = 0;
    let mut inner = NativeHostBridge::for_collection(collection);
    inner.account_overrides.insert(id.clone(), account);
    let saved_identity = data.join("codex_api_account_identity.sqlite");
    if saved_identity.exists() {
        let source = rusqlite::Connection::open_with_flags(
            saved_identity,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        source.execute(
            "VACUUM INTO ?1",
            [inner.identity_fixture.path().to_string_lossy().as_ref()],
        )?;
    }
    let reports = Arc::new(Mutex::new(Vec::new()));
    let bridge = Arc::new(AtomicBool::new(false));
    let host = Arc::new(ProtocolHost {
        inner,
        model: config.model.clone(),
        bridge: bridge.clone(),
        reports: reports.clone(),
        remaining: std::sync::atomic::AtomicUsize::new(24),
        selected_account: id,
    });
    let result = native_api::live_protocol::run(
        host,
        &key,
        &config.model,
        &config.output,
        bridge,
        &config.phase,
    )
    .await;
    std::fs::write(
        config.output.join("reports.json"),
        serde_json::to_vec_pretty(&*reports.lock().unwrap())?,
    )?;
    result?;
    println!("LIVE_PROTOCOL_ACCEPTANCE_COMPLETE");
    Ok(())
}
