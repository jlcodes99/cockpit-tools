// Pull diagnostics through a private local channel. Large optional snapshots must
// never hold the sidecar's synchronous stdout emitter or slow ordinary usage events.
const REQUEST_DIAGNOSTICS_POLL_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_DIAGNOSTICS_POLL_INTERVAL: Duration = Duration::from_millis(500);
const REQUEST_DIAGNOSTICS_POLL_IDLE_INTERVAL: Duration = Duration::from_secs(2);
const REQUEST_DIAGNOSTICS_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const REQUEST_DIAGNOSTICS_POLL_BATCH_LIMIT: usize = 8;
const REQUEST_DIAGNOSTICS_POLL_CONCURRENCY: usize = 4;
static REQUEST_DIAGNOSTICS_POLL_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RequestDiagnosticEndpoint {
    profile_runtime_key: Option<String>,
    port: u16,
    pid: u32,
    generation: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RequestDiagnosticsBatch {
    events: Vec<CodexLocalAccessRequestDetail>,
    #[serde(default)]
    dropped: u64,
}

#[derive(Default)]
struct RequestDiagnosticPollFailure {
    consecutive: u32,
    retry_at: Option<Instant>,
}

impl RequestDiagnosticPollFailure {
    fn should_poll(&self, now: Instant) -> bool {
        self.retry_at.is_none_or(|retry_at| now >= retry_at)
    }

    fn fail(&mut self, now: Instant) -> bool {
        self.consecutive = self.consecutive.saturating_add(1);
        let delay = 1_u64 << self.consecutive.saturating_sub(1).min(5);
        self.retry_at = Some(now + Duration::from_secs(delay.min(30)));
        // Metadata warning on the first failure and occasionally thereafter.
        self.consecutive == 1 || self.consecutive % 16 == 0
    }
}

fn owned_request_diagnostic_pid(child: Option<&Child>) -> Option<u32> {
    // Exit/reap belongs to the existing gateway monitor. Consuming try_wait here
    // could erase the PID before it observes the crash and prevent recovery.
    child.and_then(Child::id)
}

async fn running_request_diagnostic_endpoints() -> Vec<RequestDiagnosticEndpoint> {
    let mut endpoints = Vec::new();
    {
        let runtime = gateway_runtime().lock().await;
        if runtime.running {
            if let Some((port, pid)) = runtime
                .actual_port
                .filter(|port| *port > 0)
                .zip(owned_request_diagnostic_pid(runtime.sidecar_child.as_ref()))
            {
                endpoints.push(RequestDiagnosticEndpoint {
                    profile_runtime_key: None,
                    port,
                    pid,
                    generation: runtime.sidecar_generation,
                });
            }
        }
    }
    {
        let mut runtimes = provider_gateway_runtime_store().lock().await;
        for (runtime_key, runtime) in runtimes.iter_mut() {
            if let Some((port, pid)) = runtime
                .actual_port
                .filter(|port| *port > 0)
                .zip(owned_request_diagnostic_pid(runtime.sidecar_child.as_ref()))
            {
                endpoints.push(RequestDiagnosticEndpoint {
                    profile_runtime_key: Some(runtime_key.clone()),
                    port,
                    pid,
                    generation: None,
                });
            }
        }
    }
    endpoints.sort_unstable_by_key(|endpoint| endpoint.port);
    endpoints.dedup_by_key(|endpoint| endpoint.port);
    endpoints
}

async fn request_diagnostic_endpoint_is_running(endpoint: &RequestDiagnosticEndpoint) -> bool {
    match endpoint.profile_runtime_key.as_deref() {
        None => {
            let runtime = gateway_runtime().lock().await;
            runtime.running
                && runtime.actual_port == Some(endpoint.port)
                && runtime.sidecar_generation == endpoint.generation
                && owned_request_diagnostic_pid(runtime.sidecar_child.as_ref())
                    == Some(endpoint.pid)
        }
        Some(key) => {
            let mut runtimes = provider_gateway_runtime_store().lock().await;
            runtimes.get_mut(key).is_some_and(|runtime| {
                runtime.actual_port == Some(endpoint.port)
                    && owned_request_diagnostic_pid(runtime.sidecar_child.as_ref())
                        == Some(endpoint.pid)
            })
        }
    }
}

async fn wait_for_request_diagnostic_endpoint_stop(endpoint: &RequestDiagnosticEndpoint) {
    loop {
        if !request_diagnostic_endpoint_is_running(endpoint).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn pull_request_diagnostics_from_port(
    client: &Client,
    port: u16,
    request_timeout: Duration,
) -> Result<RequestDiagnosticsBatch, String> {
    timeout(request_timeout, async {
        let response = client.get(format!("http://{CODEX_LOCAL_ACCESS_DEFAULT_CLIENT_URL_HOST}:{port}/v1/cockpit/diagnostics/events"))
            .bearer_auth(internal_api_service_key()).send().await
            .map_err(|error| format!("请求 Sidecar 诊断失败: {error}"))?;
        if !response.status().is_success() { return Err(format!("读取 Sidecar 诊断失败: HTTP {}", response.status())); }
        if response.content_length().is_some_and(|length| length > REQUEST_DIAGNOSTICS_RESPONSE_LIMIT as u64) {
            return Err("Sidecar 诊断响应超过 2 MiB".into());
        }
        let mut body = Vec::new();
        let mut chunks = response.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|error| format!("读取 Sidecar 诊断响应失败: {error}"))?;
            if chunk.len() > REQUEST_DIAGNOSTICS_RESPONSE_LIMIT.saturating_sub(body.len()) {
                return Err("Sidecar 诊断响应超过 2 MiB".into());
            }
            body.extend_from_slice(&chunk);
        }
        let batch: RequestDiagnosticsBatch = serde_json::from_slice(&body)
            // Do not echo malformed payload values into ordinary application logs.
            .map_err(|error| format!("Sidecar 诊断响应格式无效: line={}, column={}", error.line(), error.column()))?;
        if batch.events.len() > REQUEST_DIAGNOSTICS_POLL_BATCH_LIMIT { return Err("Sidecar 诊断批次超过 8 条".into()); }
        Ok(batch)
    }).await.map_err(|_| "读取 Sidecar 诊断超时".to_string())?
}

async fn pull_request_diagnostics_with_cancel(
    client: &Client,
    port: u16,
    request_timeout: Duration,
    cancel: impl std::future::Future<Output = ()>,
) -> Result<Option<RequestDiagnosticsBatch>, String> {
    tokio::select! {
        biased;
        _ = cancel => Ok(None),
        result = pull_request_diagnostics_from_port(client, port, request_timeout) => result.map(Some),
    }
}

fn ensure_request_diagnostics_poller_started() {
    if REQUEST_DIAGNOSTICS_POLL_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        // This is one application-lifetime worker; each round rechecks live owned
        // children and cancels requests if the process or endpoint changes.
        let client =
            match build_localhost_http_client(REQUEST_DIAGNOSTICS_POLL_TIMEOUT, "请求诊断拉取")
            {
                Ok(client) => client,
                Err(error) => {
                    REQUEST_DIAGNOSTICS_POLL_RUNNING.store(false, Ordering::SeqCst);
                    logger::log_codex_api_warn(&error);
                    return;
                }
            };
        let mut failures: HashMap<RequestDiagnosticEndpoint, RequestDiagnosticPollFailure> =
            HashMap::new();
        let mut dropped_counts: HashMap<RequestDiagnosticEndpoint, u64> = HashMap::new();
        loop {
            let endpoints = running_request_diagnostic_endpoints().await;
            let active = endpoints.iter().cloned().collect::<HashSet<_>>();
            failures.retain(|endpoint, _| active.contains(endpoint));
            dropped_counts.retain(|endpoint, _| active.contains(endpoint));
            let now = Instant::now();
            let targets = endpoints
                .into_iter()
                .filter(|endpoint| {
                    failures
                        .get(endpoint)
                        .is_none_or(|failure| failure.should_poll(now))
                })
                .collect::<Vec<_>>();
            let idle = active.is_empty();
            let mut results = stream::iter(targets)
                .map(|endpoint| {
                    let client = &client;
                    async move {
                        let payload_epoch =
                            REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH.load(Ordering::SeqCst);
                        let log_epoch = REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst);
                        let result = pull_request_diagnostics_with_cancel(
                            client,
                            endpoint.port,
                            REQUEST_DIAGNOSTICS_POLL_TIMEOUT,
                            wait_for_request_diagnostic_endpoint_stop(&endpoint),
                        )
                        .await;
                        (endpoint, result, payload_epoch, log_epoch)
                    }
                })
                .buffer_unordered(REQUEST_DIAGNOSTICS_POLL_CONCURRENCY);
            while let Some((endpoint, result, payload_epoch, log_epoch)) = results.next().await {
                match result {
                    Ok(Some(batch)) if request_diagnostic_endpoint_is_running(&endpoint).await => {
                        failures.remove(&endpoint);
                        let previous_dropped =
                            dropped_counts.get(&endpoint).copied().unwrap_or_default();
                        if batch.dropped > previous_dropped {
                            logger::log_codex_api_warn(&format!(
                                "Sidecar 诊断队列繁忙，已跳过 {} 条诊断；普通请求日志不受影响",
                                batch.dropped - previous_dropped
                            ));
                        }
                        if dropped_counts.len() < 512 || dropped_counts.contains_key(&endpoint) {
                            dropped_counts.insert(endpoint.clone(), batch.dropped);
                        }
                        for detail in batch.events {
                            queue_request_diagnostics_at_epoch(detail, payload_epoch, log_epoch);
                        }
                    }
                    Ok(_) => {
                        failures.remove(&endpoint);
                    }
                    Err(error) => {
                        // Keep the failure/backoff cache bounded even with many profiles.
                        if failures.len() < 512 || failures.contains_key(&endpoint) {
                            let failure = failures.entry(endpoint.clone()).or_default();
                            if failure.fail(Instant::now()) {
                                logger::log_codex_api_warn(&format!(
                                    "请求诊断后台拉取失败: port={}, error={error}",
                                    endpoint.port
                                ));
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(if idle {
                REQUEST_DIAGNOSTICS_POLL_IDLE_INTERVAL
            } else {
                REQUEST_DIAGNOSTICS_POLL_INTERVAL
            })
            .await;
        }
    });
}
