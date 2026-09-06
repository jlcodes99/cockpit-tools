// Codex Local Access：Gateway process lifecycle, runtime snapshots and usage accounting。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
async fn ensure_gateway_matches_runtime_once_locked() -> Result<(), String> {
    let (collection, running, actual_port, actual_bind_host, actual_fingerprint, stale_task) = {
        let mut runtime = gateway_runtime().lock().await;
        refresh_gateway_process_status(&mut runtime);
        let stale_task = if !runtime.running {
            runtime.task.take()
        } else {
            None
        };
        (
            runtime.collection.clone(),
            runtime.running,
            runtime.actual_port,
            runtime.actual_bind_host.clone(),
            runtime.sidecar_config_fingerprint.clone(),
            stale_task,
        )
    };

    if let Some(task) = stale_task {
        let _ = task.await;
    }

    let Some(collection) = collection else {
        stop_gateway_locked().await;
        return Ok(());
    };

    if !collection.enabled {
        stop_gateway_locked().await;
        return Ok(());
    }

    let bind_host = bind_host_for_collection(&collection);

    let preparation_total = effective_sidecar_account_ids(&collection).len();
    let preparation_guard = GatewayPreparationGuard::begin(preparation_total);
    let launch_config = match prepare_sidecar_launch_config(
        &collection,
        preparation_guard.context(preparation_total),
    )
    .await
    {
        Ok(config) => config,
        Err(error) if error == GATEWAY_PREPARATION_CANCELLED => {
            logger::log_codex_api_info(
                "[CodexLocalAccess] API 服务账号准备已被新的启动/停止操作取消",
            );
            return Ok(());
        }
        Err(error) => {
            if running {
                stop_gateway_locked().await;
            }
            let mut runtime = gateway_runtime().lock().await;
            runtime.last_error = Some(error.clone());
            return Err(error);
        }
    };
    if running
        && actual_port == Some(collection.port)
        && actual_bind_host.as_deref() == Some(bind_host)
        && actual_fingerprint.as_deref() == Some(launch_config.fingerprint.as_str())
    {
        return Ok(());
    }
    if running {
        log_gateway_mode_info(
            CodexLocalAccessGatewayMode::Sidecar,
            format!(
                "API 服务网关配置已变化，准备重启: mode=sidecar port={}->{} bind={}->{} config_changed={}",
                actual_port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                collection.port,
                actual_bind_host.as_deref().unwrap_or("-"),
                bind_host,
                actual_fingerprint.as_deref() != Some(launch_config.fingerprint.as_str())
            ),
        );
    }

    let stopped_endpoint = stop_gateway_locked().await;
    if let Some(endpoint) = stopped_endpoint {
        wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
    }

    if probe_sidecar_ready_once(&collection, Duration::from_millis(250))
        .await
        .is_ok()
    {
        match process::kill_port_processes(collection.port) {
            Ok(count) if count > 0 => {
                log_gateway_mode_info(
                    CodexLocalAccessGatewayMode::Sidecar,
                    format!(
                        "已停止旧 API 服务 sidecar 以加载新配置: port={}, killed={}",
                        collection.port, count
                    ),
                );
            }
            Ok(_) => {}
            Err(error) => {
                let message = format!("停止旧 API 服务 sidecar 失败: {}", error);
                let mut runtime = gateway_runtime().lock().await;
                runtime.running = false;
                runtime.actual_port = None;
                runtime.actual_bind_host = None;
                runtime.sidecar_config_fingerprint = None;
                runtime.last_error = Some(message.clone());
                return Err(message);
            }
        }
        wait_for_gateway_port_release(bind_host, collection.port).await?;
    }

    let binary = match sidecar_binary_path() {
        Ok(path) => path,
        Err(message) => {
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.sidecar_config_fingerprint = None;
            runtime.last_error = Some(message.clone());
            return Err(message);
        }
    };

    let mut command = TokioCommand::new(&binary);
    sanitize_sidecar_command_env(&mut command);
    command
        .arg("--config")
        .arg(&launch_config.config_path)
        .arg("--manifest")
        .arg(&launch_config.manifest_path)
        .arg("--quota-reserve-state")
        .arg(&launch_config.quota_reserve_path)
        .arg("--quota-pool-state")
        .arg(&launch_config.quota_pool_path)
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .current_dir(
            launch_config
                .config_path
                .parent()
                .unwrap_or_else(|| Path::new(".")),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(0x08000000);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!("启动 API 服务 sidecar 失败: {}", error);
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.sidecar_config_fingerprint = None;
            runtime.last_error = Some(message.clone());
            return Err(message);
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (ready_sender, mut ready_receiver) = oneshot::channel();
    let startup_diagnostics = Arc::new(Mutex::new(SidecarStartupDiagnostics::default()));
    let task_startup_diagnostics = Arc::clone(&startup_diagnostics);
    let task = tokio::spawn(async move {
        let stdout_diagnostics = Arc::clone(&task_startup_diagnostics);
        let stderr_diagnostics = Arc::clone(&task_startup_diagnostics);
        let stdout_task = stdout.map(|stdout| {
            tokio::spawn(drain_sidecar_stdout(
                stdout,
                ready_sender,
                stdout_diagnostics,
            ))
        });
        let stderr_task =
            stderr.map(|stderr| tokio::spawn(drain_sidecar_stderr(stderr, stderr_diagnostics)));
        if let Some(task) = stdout_task {
            let _ = task.await;
        }
        if let Some(task) = stderr_task {
            let _ = task.await;
        }
    });

    let ready_signal = match wait_for_sidecar_ready(
        &mut ready_receiver,
        &mut child,
        Some(preparation_guard.generation),
    )
    .await
    {
        Ok(signal) => signal,
        Err(error) if error == GATEWAY_PREPARATION_CANCELLED => {
            let _ = child.kill().await;
            task.abort();
            let _ = task.await;
            return Ok(());
        }
        Err(error) => {
            let diagnostics = sidecar_startup_diagnostics_text(&startup_diagnostics);
            let message = format!("{}; {}", error, diagnostics);
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][sidecar] sidecar ready 等待失败，将停止进程: {}",
                message
            ));
            let _ = child.kill().await;
            task.abort();
            let _ = task.await;
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.sidecar_config_fingerprint = None;
            runtime.last_error = Some(message.clone());
            return Err(message);
        }
    };
    if let Some(ready_port) = ready_signal.port {
        if ready_port != collection.port {
            let message = format!(
                "API 服务 sidecar ready 端口不一致: expected={}, actual={}, host={}",
                collection.port, ready_port, ready_signal.host
            );
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][sidecar] sidecar ready 校验失败，将停止进程: {}",
                message
            ));
            let _ = child.kill().await;
            task.abort();
            let _ = task.await;
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.sidecar_config_fingerprint = None;
            runtime.last_error = Some(message.clone());
            return Err(message);
        }
    } else {
        let message = format!(
            "API 服务 sidecar ready 事件缺少端口: host={}",
            ready_signal.host
        );
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][sidecar] sidecar ready 校验失败，将停止进程: {}",
            message
        ));
        let _ = child.kill().await;
        task.abort();
        let _ = task.await;
        let mut runtime = gateway_runtime().lock().await;
        runtime.running = false;
        runtime.actual_port = None;
        runtime.actual_bind_host = None;
        runtime.sidecar_config_fingerprint = None;
        runtime.last_error = Some(message.clone());
        return Err(message);
    }

    let port = collection.port;
    let bind_host = bind_host.to_string();
    log_sidecar_proxy_signature(&launch_config.proxy_signature);
    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][sidecar] API 服务 sidecar 已启动: bin={} bind={}:{} base={}",
        binary.display(),
        bind_host,
        port,
        build_base_url(port)
    ));

    let mut runtime = gateway_runtime().lock().await;
    runtime.running = true;
    runtime.actual_port = Some(collection.port);
    runtime.actual_bind_host = Some(bind_host);
    runtime.sidecar_config_fingerprint = Some(launch_config.fingerprint);
    runtime.last_error = None;
    runtime.shutdown_sender = None;
    runtime.task = Some(task);
    runtime.sidecar_child = Some(child);
    Ok(())
}

async fn stop_gateway() -> Option<GatewayBindEndpoint> {
    let _stop_request_guard = GatewayStopRequestGuard::begin();
    advance_gateway_lifecycle_generation();
    let _lifecycle_guard = gateway_lifecycle_lock().lock().await;
    stop_gateway_locked().await
}

async fn stop_gateway_locked() -> Option<GatewayBindEndpoint> {
    let (shutdown_sender, task, child, endpoint) = {
        let mut runtime = gateway_runtime().lock().await;
        let endpoint = runtime
            .actual_port
            .zip(runtime.actual_bind_host.clone())
            .map(|(port, bind_host)| GatewayBindEndpoint { bind_host, port });
        runtime.running = false;
        runtime.actual_port = None;
        runtime.actual_bind_host = None;
        runtime.sidecar_config_fingerprint = None;
        (
            runtime.shutdown_sender.take(),
            runtime.task.take(),
            runtime.sidecar_child.take(),
            endpoint,
        )
    };

    if let Some(sender) = shutdown_sender {
        let _ = sender.send(true);
    }
    if let Some(mut child) = child {
        match timeout(GATEWAY_SHUTDOWN_TIMEOUT, child.kill()).await {
            Ok(Ok(())) => {
                let _ = child.wait().await;
            }
            Ok(Err(error)) => {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 停止 API 服务 sidecar 失败: {}",
                    error
                ));
            }
            Err(_) => {
                logger::log_codex_api_warn(
                    "[CodexLocalAccess] 停止 API 服务 sidecar 超时，继续清理监听任务",
                );
            }
        }
    }
    if let Some(mut task) = task {
        tokio::select! {
            result = &mut task => {
                let _ = result;
            }
            _ = tokio::time::sleep(GATEWAY_SHUTDOWN_TIMEOUT) => {
                logger::log_codex_api_warn("[CodexLocalAccess] 停止本地接入服务超时，已强制中止监听任务");
                task.abort();
                let _ = task.await;
            }
        }
    }

    endpoint
}

fn apply_usage_stats(
    target: &mut CodexLocalAccessUsageStats,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    estimated_cost_usd: f64,
) {
    target.request_count = target.request_count.saturating_add(1);
    if success {
        target.success_count = target.success_count.saturating_add(1);
    } else {
        target.failure_count = target.failure_count.saturating_add(1);
    }
    let normalized_error_category = error_category
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if matches!(normalized_error_category, Some(category) if is_client_canceled_error_category(category))
    {
        target.client_canceled_count = target.client_canceled_count.saturating_add(1);
    }
    if matches!(normalized_error_category, Some(category) if is_upstream_response_failed_error_category(category))
    {
        target.upstream_response_failed_count =
            target.upstream_response_failed_count.saturating_add(1);
    }
    if matches!(normalized_error_category, Some(category) if is_stream_incomplete_error_category(category))
    {
        target.stream_incomplete_count = target.stream_incomplete_count.saturating_add(1);
    }
    // Average latency should only reflect successful requests. Including
    // transport/auth failures (often 0ms) pulls the average down misleadingly.
    if success {
        target.total_latency_ms = target.total_latency_ms.saturating_add(latency_ms);
    }
    match request_kind {
        CodexLocalAccessRequestKind::Text => {
            target.text_request_count = target.text_request_count.saturating_add(1);
        }
        CodexLocalAccessRequestKind::ImageGeneration => {
            target.image_request_count = target.image_request_count.saturating_add(1);
            target.image_generation_request_count =
                target.image_generation_request_count.saturating_add(1);
        }
        CodexLocalAccessRequestKind::ImageEdit => {
            target.image_request_count = target.image_request_count.saturating_add(1);
            target.image_edit_request_count = target.image_edit_request_count.saturating_add(1);
        }
        CodexLocalAccessRequestKind::Other => {}
    }
    if matches!(
        normalized_error_category,
        Some("image_generation_not_enabled" | "image_generation_disabled")
    ) {
        target.image_generation_capability_failure_count = target
            .image_generation_capability_failure_count
            .saturating_add(1);
    }

    if let Some(usage) = usage {
        target.input_tokens = target.input_tokens.saturating_add(usage.input_tokens);
        target.output_tokens = target.output_tokens.saturating_add(usage.output_tokens);
        target.total_tokens = target.total_tokens.saturating_add(usage.total_tokens);
        target.cached_tokens = target.cached_tokens.saturating_add(usage.cached_tokens);
        target.reasoning_tokens = target
            .reasoning_tokens
            .saturating_add(usage.reasoning_tokens);
    }
    if estimated_cost_usd.is_finite() && estimated_cost_usd > 0.0 {
        target.estimated_cost_usd += estimated_cost_usd;
    }
}

fn upsert_account_usage_stats(
    accounts: &mut Vec<CodexLocalAccessAccountStats>,
    account_id: Option<&str>,
    account_email: Option<&str>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    estimated_cost_usd: f64,
    updated_at: i64,
) {
    let Some(account_id) = account_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let normalized_email = account_email
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();

    if let Some(account_stats) = accounts
        .iter_mut()
        .find(|item| item.account_id == account_id)
    {
        if !normalized_email.is_empty() {
            account_stats.email = normalized_email;
        }
        account_stats.updated_at = updated_at;
        apply_usage_stats(
            &mut account_stats.usage,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage,
            estimated_cost_usd,
        );
        return;
    }

    let mut account_stats = CodexLocalAccessAccountStats {
        account_id: account_id.to_string(),
        email: normalized_email,
        usage: CodexLocalAccessUsageStats::default(),
        updated_at,
    };
    apply_usage_stats(
        &mut account_stats.usage,
        request_kind,
        success,
        error_category,
        latency_ms,
        usage,
        estimated_cost_usd,
    );
    accounts.push(account_stats);
}

fn upsert_model_usage_stats(
    models: &mut Vec<CodexLocalAccessModelStats>,
    model_id: Option<&str>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    estimated_cost_usd: f64,
    updated_at: i64,
) {
    let Some(model_id) = model_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };

    if let Some(model_stats) = models.iter_mut().find(|item| item.model_id == model_id) {
        model_stats.updated_at = updated_at;
        apply_usage_stats(
            &mut model_stats.usage,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage,
            estimated_cost_usd,
        );
        return;
    }

    let mut model_stats = CodexLocalAccessModelStats {
        model_id: model_id.to_string(),
        usage: CodexLocalAccessUsageStats::default(),
        updated_at,
    };
    apply_usage_stats(
        &mut model_stats.usage,
        request_kind,
        success,
        error_category,
        latency_ms,
        usage,
        estimated_cost_usd,
    );
    models.push(model_stats);
}

fn upsert_api_key_usage_stats(
    api_keys: &mut Vec<CodexLocalAccessApiKeyStats>,
    api_key_id: Option<&str>,
    api_key_label: Option<&str>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    estimated_cost_usd: f64,
    updated_at: i64,
) {
    let Some(api_key_id) = api_key_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let normalized_label = api_key_label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();

    if let Some(api_key_stats) = api_keys
        .iter_mut()
        .find(|item| item.api_key_id == api_key_id)
    {
        if !normalized_label.is_empty() {
            api_key_stats.label = normalized_label;
        }
        api_key_stats.updated_at = updated_at;
        apply_usage_stats(
            &mut api_key_stats.usage,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage,
            estimated_cost_usd,
        );
        return;
    }

    let mut api_key_stats = CodexLocalAccessApiKeyStats {
        api_key_id: api_key_id.to_string(),
        label: normalized_label,
        usage: CodexLocalAccessUsageStats::default(),
        updated_at,
    };
    apply_usage_stats(
        &mut api_key_stats.usage,
        request_kind,
        success,
        error_category,
        latency_ms,
        usage,
        estimated_cost_usd,
    );
    api_keys.push(api_key_stats);
}

fn build_account_health_snapshot(runtime: &GatewayRuntime) -> Vec<CodexLocalAccessAccountHealth> {
    let now = now_ms();
    let Some(collection) = runtime.collection.as_ref() else {
        return Vec::new();
    };
    let stats_emails: HashMap<&str, &str> = runtime
        .stats
        .accounts
        .iter()
        .map(|item| (item.account_id.as_str(), item.email.as_str()))
        .collect();

    collection
        .account_ids
        .iter()
        .map(|account_id| {
            let health = runtime.account_health.get(account_id);
            let mut cooldowns = runtime
                .model_cooldowns
                .iter()
                .filter_map(|(key, cooldown)| {
                    if cooldown.next_retry_at_ms <= now {
                        return None;
                    }
                    key.strip_prefix(&format!("{}{}", account_id, COOLDOWN_KEY_SEPARATOR))
                        .map(|_| {
                            let remaining_ms = cooldown.next_retry_at_ms.saturating_sub(now).max(0);
                            CodexLocalAccessAccountCooldown {
                                model_id: cooldown.model_key.clone(),
                                next_retry_at: cooldown.next_retry_at_ms,
                                remaining_ms,
                                reason: cooldown.reason.clone(),
                            }
                        })
                })
                .collect::<Vec<_>>();
            let quota_cooldown = runtime.account_quota_cooldowns.get(account_id)
                .filter(|quota| quota.active(now));
            if let Some(quota) = quota_cooldown {
                let next_retry_at = quota.reset_at_ms.unwrap_or_default();
                cooldowns.push(CodexLocalAccessAccountCooldown {
                    model_id: "*".to_string(),
                    next_retry_at,
                    remaining_ms: next_retry_at.saturating_sub(now).max(0),
                    reason: "quota_exhausted".to_string(),
                });
            }
            let image_generation_status = if collection.image_generation_mode
                == CodexLocalAccessImageGenerationMode::Disabled
            {
                CodexLocalAccessImageGenerationStatus::Disabled
            } else {
                health
                    .map(|item| item.image_generation_status)
                    .unwrap_or_default()
            };
            CodexLocalAccessAccountHealth {
                account_id: account_id.clone(),
                email: health
                    .and_then(|item| {
                        Some(item.email.as_str()).filter(|value| !value.trim().is_empty())
                    })
                    .or_else(|| stats_emails.get(account_id.as_str()).copied())
                    .unwrap_or_default()
                    .to_string(),
                available: cooldowns.is_empty()
                    && quota_cooldown.is_none()
                    && !account_health_blocks_routing(health)
                    && !sidecar_scheduler_blocks_account(health, now),
                consecutive_failures: health
                    .map(|item| item.consecutive_failures)
                    .unwrap_or_default(),
                last_success_at: health.and_then(|item| item.last_success_at),
                last_failure_at: health.and_then(|item| item.last_failure_at),
                last_failure_status: health.and_then(|item| item.last_failure_status),
                last_failure_category: health.and_then(|item| item.last_failure_category.clone()),
                last_failure_message: health.and_then(|item| item.last_failure_message.clone()),
                image_generation_status,
                image_generation_checked_at: health
                    .and_then(|item| item.image_generation_checked_at),
                scheduler_available: health.and_then(|item| {
                    item.sidecar_scheduler_available.map(|available| {
                        available || !sidecar_scheduler_blocks_account(Some(item), now)
                    })
                }),
                scheduler_reason: health.and_then(|item| item.sidecar_scheduler_reason.clone()),
                scheduler_next_retry_at: health
                    .and_then(|item| item.sidecar_scheduler_next_retry_at)
                    .filter(|value| *value > now),
                cooldowns,
            }
        })
        .collect()
}

fn build_account_pool_health_snapshot(
    runtime: &GatewayRuntime,
) -> Vec<CodexLocalAccessAccountPoolHealth> {
    let mut pool_health = runtime
        .account_pool_health
        .values()
        .map(|health| CodexLocalAccessAccountPoolHealth {
            api_key_id: health.api_key_id.clone(),
            api_key_label: health.api_key_label.clone(),
            provider: health.provider.clone(),
            model: health.model.clone(),
            request_kind: health.request_kind.clone(),
            error_code: health.error_code.clone(),
            error_message: health.error_message.clone(),
            diagnostic_available: health.diagnostic_available,
            candidate_auths: health.candidate_auths,
            scoped_auths: health.scoped_auths,
            available_auths: health.available_auths,
            unavailable_auths: health.unavailable_auths,
            model_excluded_auths: health.model_excluded_auths,
            quota_reserved_auths: health.quota_reserved_auths,
            image_policy_blocked_auths: health.image_policy_blocked_auths,
            account_statuses: health
                .account_statuses
                .iter()
                .map(|item| CodexLocalAccessAccountPoolMemberHealth {
                    account_id: item.account_id.clone(),
                    account_email: item.account_email.clone(),
                    available: item.available,
                    reason_code: item.reason_code.clone(),
                    reason_message: item.reason_message.clone(),
                })
                .collect(),
            last_failure_at: health.last_failure_at,
        })
        .collect::<Vec<_>>();
    pool_health.sort_by(|left, right| right.last_failure_at.cmp(&left.last_failure_at));
    pool_health
}

#[derive(Debug, Clone, Copy, Default)]
struct RequestStatsMeta<'a> {
    request_id: Option<&'a str>,
    client_instance_id: Option<&'a str>,
    http_status: Option<u16>,
    error_message: Option<&'a str>,
    service_tier: Option<&'a str>,
    reasoning_effort: Option<&'a str>,
}

async fn record_request_stats_with_meta(
    account_id: Option<&str>,
    account_email: Option<&str>,
    api_key_id: Option<&str>,
    api_key_label: Option<&str>,
    model_id: Option<&str>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<UsageCapture>,
    meta: RequestStatsMeta<'_>,
) -> Result<(), String> {
    let persisted_event = {
        let mut runtime = gateway_runtime().lock().await;
        let now = now_ms();
        let usage_ref = usage.as_ref();
        let pricing = resolve_effective_model_pricing(
            runtime.collection.as_ref(),
            model_id,
            usage_ref,
            meta.service_tier,
        );
        let model_pricing_version = runtime
            .collection
            .as_ref()
            .map(|collection| collection.model_pricing_version)
            .unwrap_or(DEFAULT_MODEL_PRICING_VERSION)
            .max(DEFAULT_MODEL_PRICING_VERSION);
        let estimated_cost_usd = calculate_usage_cost_usd(usage_ref, pricing.as_ref());
        let gateway_mode = runtime.collection.as_ref().map(collection_gateway_mode);
        if runtime.stats.since <= 0 {
            runtime.stats.since = now;
        }
        runtime.stats.updated_at = now;
        apply_usage_stats(
            &mut runtime.stats.totals,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage_ref,
            estimated_cost_usd,
        );
        upsert_account_usage_stats(
            &mut runtime.stats.accounts,
            account_id,
            account_email,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage_ref,
            estimated_cost_usd,
            now,
        );
        upsert_model_usage_stats(
            &mut runtime.stats.models,
            model_id,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage_ref,
            estimated_cost_usd,
            now,
        );
        upsert_api_key_usage_stats(
            &mut runtime.stats.api_keys,
            api_key_id,
            api_key_label,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage_ref,
            estimated_cost_usd,
            now,
        );
        let token_usage_changed = api_key_id
            .zip(usage_ref)
            .is_some_and(|(api_key_id, usage)| {
                runtime.collection.as_mut().is_some_and(|collection| {
                    add_api_key_token_usage(
                        collection,
                        api_key_id,
                        effective_usage_total_tokens(usage),
                    )
                })
            });
        runtime.collection_dirty |= token_usage_changed;
        let event = append_usage_event(
            &mut runtime.stats.events,
            now,
            meta.request_id,
            account_id,
            account_email,
            api_key_id,
            api_key_label,
            meta.client_instance_id,
            model_id,
            gateway_mode,
            request_kind,
            meta.service_tier,
            meta.reasoning_effort,
            success,
            meta.http_status,
            error_category,
            meta.error_message,
            latency_ms,
            usage_ref,
            pricing.as_ref(),
            model_pricing_version,
            estimated_cost_usd,
        );

        apply_usage_event_to_current_windows(&mut runtime.stats, &event, now);
        sort_stats_rows(&mut runtime.stats);
        runtime.stats_dirty = true;
        runtime.stats_revision = runtime.stats_revision.wrapping_add(1);
        event
    };

    if let Err(error) = persist_local_access_usage_event(&persisted_event) {
        logger::log_codex_api_warn(&format!(
            "API 服务请求日志写入失败，已保留内存统计并继续处理请求: {}",
            error
        ));
    }

    schedule_stats_flush_if_needed().await;
    if success {
        if let Some(account_id) = account_id {
            if bound_oauth_quota_refresh_target().await.as_deref() == Some(account_id) {
                trigger_bound_oauth_quota_refresh_in_background(
                    "绑定账号请求完成",
                    BOUND_OAUTH_QUOTA_RESERVE_REQUEST_REFRESH_MIN_INTERVAL,
                );
            }
        }
    }
    Ok(())
}

fn stats_model_id_from_response_capture(
    requested_model: &str,
    response_capture: &ResponseCapture,
) -> String {
    response_capture
        .response_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(requested_model)
        .to_string()
}

fn build_state_snapshot_inner(
    runtime: &GatewayRuntime,
    include_default_profile: bool,
) -> CodexLocalAccessState {
    let collection = runtime.collection.clone();
    let member_count = collection
        .as_ref()
        .map(|item| item.account_ids.len())
        .unwrap_or(0);
    let api_port_url = collection
        .as_ref()
        .map(|item| build_api_port_url(item.port));
    let base_url = collection.as_ref().map(build_collection_base_url);
    let default_profile = if include_default_profile {
        collection.as_ref().map(|collection| {
            inspect_local_access_profile_attachment(
                &codex_account::get_codex_home(),
                Some(collection),
            )
        })
    } else {
        None
    };
    let lan_base_url = collection.as_ref().and_then(|item| {
        if item.access_scope == CodexLocalAccessScope::Lan {
            build_lan_base_url(item.port)
        } else {
            None
        }
    });
    let model_ids = collection
        .as_ref()
        .map(|collection| {
            visible_codex_model_ids_for_collection(collection, Some(&runtime.account_health))
        })
        .unwrap_or_else(supported_codex_model_ids);
    let mut stats = stats_snapshot_without_events(&runtime.stats);
    stats.events = runtime
        .stats
        .events
        .iter()
        .rev()
        .take(STATE_RECENT_USAGE_EVENT_LIMIT)
        .cloned()
        .collect();
    let account_health = build_account_health_snapshot(runtime);
    let account_pool_health = build_account_pool_health_snapshot(runtime);
    let quota_reserve_status = collection.as_ref().and_then(build_quota_reserve_status);
    let service_enabled = collection
        .as_ref()
        .is_some_and(|collection| collection.enabled);

    CodexLocalAccessState {
        collection,
        running: runtime.running,
        preparing: service_enabled && GATEWAY_PREPARING.load(Ordering::SeqCst),
        preparation_total: GATEWAY_PREPARATION_TOTAL.load(Ordering::SeqCst),
        preparation_completed: GATEWAY_PREPARATION_COMPLETED.load(Ordering::SeqCst),
        refreshing_accounts: service_enabled
            && GATEWAY_ACCOUNT_REFRESH_RUNNING.load(Ordering::SeqCst),
        account_refresh_total: GATEWAY_ACCOUNT_REFRESH_TOTAL.load(Ordering::SeqCst),
        account_refresh_completed: GATEWAY_ACCOUNT_REFRESH_COMPLETED.load(Ordering::SeqCst),
        default_profile,
        api_port_url,
        base_url,
        lan_base_url,
        model_ids,
        model_pricing_presets: default_model_pricing_presets(),
        last_error: runtime.last_error.clone(),
        member_count,
        stats,
        account_health,
        account_pool_health,
        quota_reserve_status,
    }
}

fn build_state_snapshot(runtime: &GatewayRuntime) -> CodexLocalAccessState {
    build_state_snapshot_inner(runtime, true)
}

fn build_request_state_snapshot(runtime: &GatewayRuntime) -> CodexLocalAccessState {
    build_state_snapshot_inner(runtime, false)
}

fn build_fresh_state_snapshot(runtime: &mut GatewayRuntime) -> CodexLocalAccessState {
    ensure_stats_windows_current(&mut runtime.stats, now_ms());
    build_state_snapshot(runtime)
}

async fn snapshot_state() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    let mut runtime = gateway_runtime().lock().await;
    refresh_gateway_process_status(&mut runtime);
    if runtime
        .last_error
        .as_deref()
        .map(|message| {
            message.starts_with("默认 Codex 配置接管失败:")
                || message.starts_with("Codex 配置接管失败:")
        })
        .unwrap_or(false)
    {
        runtime.last_error = None;
    }
    Ok(build_fresh_state_snapshot(&mut runtime))
}

async fn snapshot_state_without_gateway_reload() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    let mut runtime = gateway_runtime().lock().await;
    Ok(build_fresh_state_snapshot(&mut runtime))
}

pub async fn get_local_access_state() -> Result<CodexLocalAccessState, String> {
    snapshot_state().await
}

/// Resolve the OAuth account used by an API Service-bound Codex profile.
///
/// API Service is represented in the instance store by a synthetic bind ID, so
/// the normal instance binding resolver cannot discover the actual OAuth owner.
pub(crate) async fn bound_oauth_account_id_for_instance_start() -> Result<Option<String>, String> {
    ensure_runtime_loaded_without_start().await?;
    let bound_id = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.as_ref().and_then(|collection| {
            normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref())
        })
    };
    let Some(bound_id) = bound_id else {
        return Ok(None);
    };
    let account = validate_local_access_bound_oauth_account(&bound_id)?;
    Ok(Some(account.id))
}

pub async fn activate_local_access_for_dir(
    profile_dir: &Path,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    let api_key = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .as_ref()
            .map(|collection| collection.api_key.clone())
            .ok_or_else(|| "API 服务集合尚未创建".to_string())?
    };
    save_profile_takeover_backup(profile_dir, &api_key)?;
    let state = set_local_access_enabled(true).await?;
    let collection = state
        .collection
        .clone()
        .ok_or_else(|| "API 服务集合尚未创建".to_string())?;
    write_local_access_profile_takeover(profile_dir, &collection, None).await?;
    Ok(state)
}

pub async fn prepare_local_access_for_bound_profile_dir(
    profile_dir: &Path,
) -> Result<bool, String> {
    ensure_runtime_loaded_without_start().await?;
    let collection = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .clone()
            .ok_or_else(|| "API 服务集合尚未创建".to_string())?
    };

    if !collection.enabled {
        restore_takeover_profiles_after_disable(&collection)?;
        return Ok(false);
    }

    ensure_gateway_matches_runtime().await?;
    ensure_profile_takeover(profile_dir, &collection).await?;
    Ok(true)
}

fn new_empty_local_access_collection() -> Result<CodexLocalAccessCollection, String> {
    Ok(CodexLocalAccessCollection {
        enabled: false,
        port: allocate_initial_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?,
        api_key: generate_local_api_key(),
        api_keys: Vec::new(),
        access_scope: CodexLocalAccessScope::Localhost,
        client_base_url_host: CodexLocalAccessClientBaseUrlHost::default(),
        image_generation_mode: CodexLocalAccessImageGenerationMode::default(),
        image_generation_account_policies: HashMap::new(),
        gateway_mode: CodexLocalAccessGatewayMode::default(),
        upstream_proxy_url: None,
        routing_strategy: CodexLocalAccessRoutingStrategy::default(),
        custom_routing_rules: Vec::new(),
        account_model_rules: Vec::new(),
        model_aliases: Vec::new(),
        model_pricing_version: DEFAULT_MODEL_PRICING_VERSION,
        model_pricings: Vec::new(),
        excluded_models: Vec::new(),
        session_affinity: true,
        session_affinity_ttl_ms: DEFAULT_SESSION_AFFINITY_TTL_MS,
        session_affinity_default_enabled_migrated: true,
        responses_websockets_enabled: false,
        max_retry_credentials: 0,
        max_retry_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
        timeouts: CodexLocalAccessTimeouts::default(),
        active_timeout_preset_id: BUILTIN_TIMEOUT_PRESET_LONG_WAIT_ID.to_string(),
        timeout_presets: Vec::new(),
        disable_cooling: false,
        restrict_free_accounts: true,
        debug_logs: true,
        immediate_sse_response: false,
        max_concurrent_image_requests: 1,
        bound_oauth_account_id: None,
        bound_oauth_quota_reserve: None,
        account_ids: Vec::new(),
        created_at: now_ms(),
        updated_at: now_ms(),
    })
}
