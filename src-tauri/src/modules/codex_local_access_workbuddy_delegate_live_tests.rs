//! Opt-in real Codex acceptance listener. Secrets arrive through anonymous stdin;
//! the production scope selector and native actor are used, not a fake upstream.
//! This test never runs in ordinary CI, does not load an account store, and never
//! submits a prompt by itself. The external driver supplies an isolated Codex.
use super::*;
use futures_util::future::BoxFuture;
use std::io::BufRead;

#[derive(Deserialize)]
struct LiveConfig {
    access_token: String,
    enterprise_id: Option<String>,
    domain: Option<String>,
    model_id: String,
    effort: String,
    executable: PathBuf,
    guarded_cli: PathBuf,
    artifact_dir: PathBuf,
}

struct LiveHost {
    inner: NativeHostBridge,
    reports: Mutex<Vec<Value>>,
    generic_calls: AtomicU64,
}
impl native_contract::Host for LiveHost {
    fn authorize<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, Result<native_contract::Caller, native_contract::ServiceError>> {
        self.inner.authorize(key)
    }
    fn workbuddy_delegate_route<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        model: &'a str,
    ) -> BoxFuture<
        'a,
        Result<Option<native_contract::WorkbuddyDelegateSelection>, native_contract::ServiceError>,
    > {
        self.inner.workbuddy_delegate_route(caller, model)
    }
    fn models<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
    ) -> BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        self.inner.models(caller)
    }
    fn route<'a>(
        &'a self,
        _: &'a native_contract::Caller,
        _: native_contract::RouteRequest,
    ) -> BoxFuture<'a, Result<native_contract::Route, native_contract::ServiceError>> {
        self.generic_calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async {
            Err(native_error(
                409,
                "acceptance_no_generic_fallback",
                "No generic route is available in native WorkBuddy acceptance",
            ))
        })
    }
    fn record(&self, report: native_contract::RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(json!({
                "account_id":report.account_id,"requested_model":report.requested_model,
                "actual_model":report.upstream_model,"status":report.status,
                "success":report.success,"error":report.error,
                "elapsed_ms":report.elapsed.as_millis(),"usage":report.usage,
                "transports":report.diagnostics.as_ref().map(|d|d.attempts.iter().map(|a|a.transport.clone()).collect::<Vec<_>>())
            }));
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Requires explicit human-approved model budget and private stdin bootstrap; never runs in CI"]
async fn workbuddy_delegate_real_codex_acceptance_listener(
) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("COCKPIT_WORKBUDDY_SYNTHETIC_ACCEPTANCE").as_deref()
        != Ok("explicitly-approved")
    {
        return Err("Explicit synthetic acceptance approval is required".into());
    }
    let line = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).map(|_| line)
    })
    .await??;
    let config: LiveConfig = serde_json::from_str(&line)?;
    if config.access_token.trim().is_empty()
        || !config.artifact_dir.is_absolute()
        || !config.artifact_dir.is_dir()
        || !config.executable.is_file()
        || !config.guarded_cli.is_file()
    {
        return Err("Invalid private acceptance bootstrap".into());
    }
    drop(line);
    let public_model = format!("workbuddy/{}", config.model_id);
    let provider_id = "codex_workbuddy_synthetic_acceptance";
    let source_id = "workbuddy_synthetic_acceptance";
    let local_key = format!("wb-e2e-{}", uuid::Uuid::new_v4().simple());
    let native_config = CodexLocalAccessWorkbuddyDelegate {
        provider_account_id: provider_id.into(),
        model_id: config.model_id.clone(),
        public_model: public_model.clone(),
        reasoning_effort: config.effort.clone(),
    };
    let key: CodexLocalAccessApiKey = serde_json::from_value(json!({
        "id":"wb-e2e-dedicated-key","label":"Synthetic WorkBuddy acceptance", "key":local_key,
        "enabled":true,"inheritAccountPool":false,"accountIds":[provider_id],
        "allowedModels":[public_model],"workbuddyDelegate":native_config
    }))?;
    let mut collection = new_empty_local_access_collection()?;
    collection.enabled = true;
    collection.gateway_mode = CodexLocalAccessGatewayMode::Native;
    collection.api_key = "unusable-collection-master-key".into();
    collection.api_keys = vec![key];
    collection.account_ids.clear();
    collection.responses_websockets_enabled = false;
    collection.debug_logs = false;
    collection.request_payload_logging = false;
    let mut linked = CodexAccount::new_api_key(
        provider_id.into(),
        "synthetic@example.invalid".into(),
        String::new(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some(codex_account::WORKBUDDY_ACP_BASE_URL.into()),
        Some("workbuddy_acp".into()),
        None,
        vec!["workbuddy-default".into()],
    );
    linked.api_credential_kind = Some(codex_account::WORKBUDDY_ACP_PROVIDER_KIND.into());
    linked.api_credential_ref = Some(source_id.into());
    linked.api_wire_api = Some("responses".into());
    let source: crate::models::workbuddy::WorkbuddyAccount = serde_json::from_value(json!({
        "id":source_id,"email":"synthetic@example.invalid","access_token":config.access_token,
        "created_at":0,"last_used":0,"status":"normal",
        "enterprise_id":config.enterprise_id,"domain":config.domain
    }))?;
    let mut inner = NativeHostBridge::for_collection(collection);
    inner.account_overrides.insert(provider_id.into(), linked);
    inner
        .workbuddy_delegate_test_sources
        .insert(source_id.into(), source);
    let host = Arc::new(LiveHost {
        inner,
        reports: Mutex::new(Vec::new()),
        generic_calls: AtomicU64::new(0),
    });
    let mut handle = native_api::start(
        "127.0.0.1:0".parse()?,
        host.clone(),
        config.artifact_dir.join("identity.sqlite"),
        native_api::Options::default(),
    )
    .await?;
    handle.set_workbuddy_test_cli(config.executable, config.guarded_cli);
    let (profile, auth) =
        workbuddy_delegate_profile_contents(&native_config, &local_key, handle.addr.port())?;
    let codex_home = config.artifact_dir.join("codex-home");
    fs::create_dir(&codex_home)?;
    write_secret_string_atomic(&codex_home.join("config.toml"), &profile)?;
    write_secret_string_atomic(&codex_home.join("auth.json"), &auth)?;
    println!(
        "WORKBUDDY_ACCEPTANCE_READY {}",
        json!({"addr":handle.addr.to_string(),"codex_home":codex_home,"model":public_model,"account_id":provider_id})
    );
    use std::io::Write;
    std::io::stdout().flush()?;
    let stopped = tokio::time::timeout(
        Duration::from_secs(900),
        tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line).map(|_| ())
        }),
    )
    .await;
    handle.shutdown(Duration::from_secs(10)).await?;
    let evidence = json!({"generic_fallback_calls":host.generic_calls.load(Ordering::Acquire),"reports":*host.reports.lock().unwrap()});
    fs::write(
        config.artifact_dir.join("gateway-evidence.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    if stopped.is_err() {
        return Err("Acceptance listener deadline expired".into());
    }
    assert_eq!(host.generic_calls.load(Ordering::Acquire), 0);
    Ok(())
}
