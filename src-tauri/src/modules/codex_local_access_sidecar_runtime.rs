// Codex Local Access：Sidecar health, startup diagnostics, LAN discovery and profile preparation。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn parse_sidecar_request_kind(value: &str) -> CodexLocalAccessRequestKind {
    match value.trim() {
        "text" => CodexLocalAccessRequestKind::Text,
        "image_generation" => CodexLocalAccessRequestKind::ImageGeneration,
        "image_edit" => CodexLocalAccessRequestKind::ImageEdit,
        _ => CodexLocalAccessRequestKind::Other,
    }
}

fn usage_i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn sidecar_usage_capture(details: &SidecarUsageDetails) -> Option<UsageCapture> {
    let usage = UsageCapture {
        input_tokens: usage_i64_to_u64(details.input_tokens),
        output_tokens: usage_i64_to_u64(details.output_tokens),
        total_tokens: usage_i64_to_u64(details.total_tokens),
        cached_tokens: usage_i64_to_u64(details.cached_tokens),
        reasoning_tokens: usage_i64_to_u64(details.reasoning_tokens),
        token_breakdown: details.token_breakdown.clone(),
    };
    if usage.input_tokens == 0
        && usage.output_tokens == 0
        && usage.total_tokens == 0
        && usage.cached_tokens == 0
        && usage.reasoning_tokens == 0
    {
        None
    } else {
        Some(usage)
    }
}

fn non_empty_sidecar_string(value: &str) -> Option<String> {
    Some(value.trim().to_string()).filter(|value| !value.is_empty())
}

fn sidecar_usage_event_is_client_canceled(event: &SidecarUsageEvent) -> bool {
    if event.error_category.as_deref() == Some("client_canceled") {
        let message = event.error_message.as_deref().unwrap_or_default();
        let is_upstream_context_cancel = matches!(event.status, Some(408 | 500..=599))
            && message.to_ascii_lowercase().contains("context canceled")
            && !is_client_disconnect_error_message(message);
        return !is_upstream_context_cancel;
    }
    event
        .error_message
        .as_deref()
        .map(str::to_ascii_lowercase)
        .map(|message| {
            (message.contains("context canceled") && !matches!(event.status, Some(408 | 500..=599)))
                || message.contains("client canceled")
                || message.contains("client disconnected")
                || message.contains("client closed")
                || is_client_disconnect_error_message(&message)
        })
        .unwrap_or(false)
}

fn sidecar_usage_event_should_auto_restart(event: &SidecarUsageEvent) -> bool {
    if event.success {
        return false;
    }

    let category = event
        .error_category
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let message = event
        .error_message
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    // A caller can cancel its own request while the Sidecar is healthy. Do not
    // restart for those downstream disconnects, even when the transport reports
    // a generic gateway status.
    if category.contains("client_disconnect")
        || category.contains("downstream_client")
        || message.contains("client canceled")
        || message.contains("client disconnected")
        || message.contains("client closed")
        || message.contains("client gone")
        || is_client_disconnect_error_message(&message)
    {
        return false;
    }

    let timeout_category = matches!(
        category.as_str(),
        "gateway_context_canceled"
            | "upstream_timeout"
            | "upstream_first_byte_timeout"
            | "upstream_stream_timeout"
            | "stream_idle"
            | "stream_idle_timeout"
    );
    let timeout_message = message.contains("context canceled")
        || message.contains("upstream timed out")
        || message.contains("stream_idle")
        || message.contains("stream idle")
        || message.contains("stream timeout")
        || message.contains("流式响应超时");
    let timeout_status = matches!(event.status, Some(408 | 504));
    // 本地回环代理被拒（例如账号隧道换端口后 sidecar 仍拨旧端口）必须重建，
    // 否则会一直 502；远端地址被拒不属于本地通道问题，不在此列。
    let loopback_dial_refused = message.contains("connection refused")
        && (message.contains("127.0.0.1")
            || message.contains("localhost")
            || message.contains("[::1]"));

    timeout_category || timeout_message || timeout_status || loopback_dial_refused
}

fn sidecar_auto_restart_control() -> &'static Mutex<SidecarAutoRestartControl> {
    SIDECAR_AUTO_RESTART_CONTROL.get_or_init(|| Mutex::new(SidecarAutoRestartControl::default()))
}

fn sidecar_crash_recovery_control() -> &'static Mutex<SidecarAutoRestartControl> {
    SIDECAR_CRASH_RECOVERY_CONTROL.get_or_init(|| Mutex::new(SidecarAutoRestartControl::default()))
}

fn reset_sidecar_restart_window_if_expired(control: &mut SidecarAutoRestartControl, now: Instant) {
    if control
        .window_started_at
        .is_none_or(|started_at| now.duration_since(started_at) >= SIDECAR_AUTO_RESTART_WINDOW)
    {
        control.window_started_at = Some(now);
        control.last_started_at = None;
        control.attempts = 0;
    }
}

fn sidecar_crash_recovery_allowed(
    collection_enabled: bool,
    running: bool,
    expected_generation: u64,
    current_generation: u64,
    stop_requests: usize,
) -> bool {
    collection_enabled
        && !running
        && stop_requests == 0
        && expected_generation == current_generation
}

async fn gateway_sidecar_crash_recovery_is_allowed(expected_generation: u64) -> bool {
    if GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) > 0
        || gateway_lifecycle_generation_changed(expected_generation)
    {
        return false;
    }
    let runtime = gateway_runtime().lock().await;
    sidecar_crash_recovery_allowed(
        runtime
            .collection
            .as_ref()
            .map(local_access_gateway_should_run)
            .unwrap_or(false),
        runtime.running,
        expected_generation,
        current_gateway_lifecycle_generation(),
        GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst),
    )
}

fn schedule_sidecar_crash_recovery(process_exit: SidecarProcessExit) {
    let now = Instant::now();
    let initial_delay = {
        let Ok(mut control) = sidecar_crash_recovery_control().lock() else {
            logger::log_codex_api_warn(
                "[CodexLocalAccess] Sidecar 崩溃恢复控制锁不可用，跳过本次恢复",
            );
            return;
        };
        reset_sidecar_restart_window_if_expired(&mut control, now);
        if control.in_flight || control.attempts >= SIDECAR_AUTO_RESTART_MAX_ATTEMPTS {
            return;
        }
        control.in_flight = true;
        control
            .last_started_at
            .and_then(|started_at| {
                SIDECAR_CRASH_RECOVERY_MIN_INTERVAL.checked_sub(now.duration_since(started_at))
            })
            .unwrap_or_default()
    };

    tauri::async_runtime::spawn(async move {
        if !initial_delay.is_zero() {
            tokio::time::sleep(initial_delay).await;
        }

        let mut recovered = false;
        loop {
            if !gateway_sidecar_crash_recovery_is_allowed(process_exit.generation).await {
                break;
            }

            let attempt = {
                let Ok(mut control) = sidecar_crash_recovery_control().lock() else {
                    break;
                };
                let now = Instant::now();
                reset_sidecar_restart_window_if_expired(&mut control, now);
                if control.attempts >= SIDECAR_AUTO_RESTART_MAX_ATTEMPTS {
                    break;
                }
                control.attempts = control.attempts.saturating_add(1);
                control.last_started_at = Some(now);
                control.attempts
            };

            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] API 服务 sidecar 异常退出，开始后台恢复: pid={}, generation={}, attempt={}, reason={}",
                process_exit.pid, process_exit.generation, attempt, process_exit.message
            ));
            match ensure_gateway_matches_runtime().await {
                Ok(()) if gateway_runtime().lock().await.running => {
                    logger::log_codex_api_info(
                        "[CodexLocalAccess] API 服务 sidecar 已自动恢复，账号、API Key 和端口配置保持不变",
                    );
                    recovered = true;
                    break;
                }
                Ok(()) => logger::log_codex_api_warn(
                    "[CodexLocalAccess] API 服务 sidecar 恢复未进入运行态，将按限制重试",
                ),
                Err(error) => logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] API 服务 sidecar 自动恢复失败: attempt={}, error={}",
                    attempt, error
                )),
            }

            tokio::time::sleep(SIDECAR_CRASH_RECOVERY_MIN_INTERVAL).await;
        }

        if let Ok(mut control) = sidecar_crash_recovery_control().lock() {
            control.in_flight = false;
        }
        if !recovered && gateway_sidecar_crash_recovery_is_allowed(process_exit.generation).await {
            logger::log_codex_api_warn(
                "[CodexLocalAccess] API 服务 sidecar 自动恢复已达到限制，保留错误状态等待用户处理",
            );
        }
        emit_local_access_state_updated();
    });
}

fn schedule_sidecar_auto_restart(event: &SidecarUsageEvent) {
    if !sidecar_usage_event_should_auto_restart(event) {
        return;
    }

    let now = Instant::now();
    let should_start = {
        let Ok(mut control) = sidecar_auto_restart_control().lock() else {
            logger::log_codex_api_warn(
                "[CodexLocalAccess] 自动重启控制锁不可用，跳过本次 Sidecar 兜底重启",
            );
            return;
        };
        if control.in_flight {
            false
        } else {
            if control.window_started_at.is_none_or(|started_at| {
                now.duration_since(started_at) >= SIDECAR_AUTO_RESTART_WINDOW
            }) {
                control.window_started_at = Some(now);
                control.attempts = 0;
            }
            let interval_elapsed = control.last_started_at.is_none_or(|started_at| {
                now.duration_since(started_at) >= SIDECAR_AUTO_RESTART_MIN_INTERVAL
            });
            if !interval_elapsed || control.attempts >= SIDECAR_AUTO_RESTART_MAX_ATTEMPTS {
                false
            } else {
                control.in_flight = true;
                control.last_started_at = Some(now);
                control.attempts = control.attempts.saturating_add(1);
                true
            }
        }
    };

    if !should_start {
        return;
    }

    let request_id = event.request_id.clone();
    let status = event.status;
    let category = event.error_category.clone().unwrap_or_default();
    tauri::async_runtime::spawn(async move {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess] 检测到 Sidecar 上游超时，自动重启 Sidecar: request_id={}, status={:?}, category={}",
            request_id, status, category
        ));
        match restart_local_access_sidecar().await {
            Ok(_) => logger::log_codex_api_info(
                "[CodexLocalAccess] Sidecar 超时兜底重启完成，账号池和 API Key 配置保持不变",
            ),
            Err(error) => logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] Sidecar 超时兜底重启失败: {}",
                error
            )),
        }
        if let Ok(mut control) = sidecar_auto_restart_control().lock() {
            control.in_flight = false;
        }
    });
}

fn normalized_sidecar_error_category(event: &SidecarUsageEvent) -> Option<String> {
    if sidecar_usage_event_is_client_canceled(event) {
        return Some("client_canceled".to_string());
    }
    if event
        .error_message
        .as_deref()
        .map(is_upstream_response_failed_error_message)
        .unwrap_or(false)
    {
        return Some("upstream_response_failed".to_string());
    }
    if event
        .error_message
        .as_deref()
        .map(is_stream_incomplete_error_message)
        .unwrap_or(false)
    {
        return Some("stream_incomplete".to_string());
    }
    event
        .error_category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn update_sidecar_account_health(event: &SidecarUsageEvent) {
    update_sidecar_account_health_from_values(
        event.account_id.as_str(),
        event.account_email.as_str(),
        event.request_kind.as_str(),
        event.success,
        event.status,
        event.error_category.as_deref(),
        event.error_message.as_deref(),
        sidecar_usage_event_is_client_canceled(event),
    )
    .await;
}

async fn update_sidecar_auth_result_health(event: &SidecarAuthResultEvent, pool_diagnostic: bool) {
    let is_client_canceled = event
        .error_message
        .as_deref()
        .map(str::to_ascii_lowercase)
        .map(|message| {
            message.contains("context canceled")
                || message.contains("client canceled")
                || message.contains("client disconnected")
                || message.contains("client closed")
        })
        .unwrap_or(false);
    update_sidecar_account_health_from_values(
        event.account_id.as_str(),
        event.account_email.as_str(),
        event.request_kind.as_str(),
        event.success,
        event.http_status,
        event.error_code.as_deref(),
        event.error_message.as_deref(),
        is_client_canceled,
    )
    .await;
    sync_sidecar_scheduler_state(event).await;
    update_sidecar_account_pool_health(event, pool_diagnostic).await;
    // Pool failures are the source of the account-pool dialog state. Failed
    // account events also update per-account health; successful request events
    // are intentionally not broadcast to avoid reloading the UI per request.
    if pool_diagnostic || !event.success {
        emit_local_access_state_updated();
    }
}

fn emit_local_access_state_updated() {
    let Some(app) = crate::get_app_handle() else {
        return;
    };
    let _ = app.emit("codex-local-access-state-updated", ());
}

const UNSCOPED_ACCOUNT_POOL_HEALTH_KEY: &str = "__unscoped__";

fn account_pool_health_key(api_key_id: &str) -> String {
    let api_key_id = api_key_id.trim();
    if api_key_id.is_empty() {
        UNSCOPED_ACCOUNT_POOL_HEALTH_KEY.to_string()
    } else {
        api_key_id.to_string()
    }
}

fn is_account_pool_unavailable_error(event: &SidecarAuthResultEvent) -> bool {
    matches!(
        event.error_code.as_deref().map(str::trim),
        Some("auth_not_found" | "auth_unavailable")
    ) || event
        .error_message
        .as_deref()
        .is_some_and(|message| message.to_ascii_lowercase().contains("no auth available"))
}

// Pool selection can fail before Sidecar chooses an auth, so account_id is correctly empty.
// Keep that state separately instead of discarding it or blaming an arbitrary account.
async fn update_sidecar_account_pool_health(event: &SidecarAuthResultEvent, pool_diagnostic: bool) {
    let mut runtime = gateway_runtime().lock().await;
    apply_sidecar_account_pool_health(&mut runtime, event, pool_diagnostic, now_ms());
}

fn apply_sidecar_account_pool_health(
    runtime: &mut GatewayRuntime,
    event: &SidecarAuthResultEvent,
    pool_diagnostic: bool,
    now: i64,
) {
    let key = account_pool_health_key(&event.api_key_id);
    if event.success {
        runtime.account_pool_health.remove(&key);
        return;
    }
    if !pool_diagnostic
        && (!event.account_id.trim().is_empty() || !is_account_pool_unavailable_error(event))
    {
        return;
    }

    let health = runtime.account_pool_health.entry(key).or_default();
    health.api_key_id = event.api_key_id.trim().to_string();
    if !event.api_key_label.trim().is_empty() {
        health.api_key_label = event.api_key_label.trim().to_string();
    }
    if !event.provider.trim().is_empty() {
        health.provider = event.provider.trim().to_string();
    }
    if !event.model.trim().is_empty() {
        health.model = event.model.trim().to_string();
    }
    if !event.request_kind.trim().is_empty() {
        health.request_kind = event.request_kind.trim().to_string();
    }
    if let Some(error_code) = event
        .error_code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        health.error_code = error_code.to_string();
    }
    if let Some(error_message) = event
        .error_message
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        health.error_message = error_message.to_string();
    }
    if pool_diagnostic {
        health.diagnostic_available = true;
        health.candidate_auths = event.candidate_auths;
        health.scoped_auths = event.scoped_auths;
        health.available_auths = event.available_auths;
        health.unavailable_auths = event.unavailable_auths;
        health.model_excluded_auths = event.model_excluded_auths;
        health.quota_reserved_auths = event.quota_reserved_auths;
        health.image_policy_blocked_auths = event.image_policy_blocked_auths;
        health.account_statuses = event
            .account_statuses
            .iter()
            .filter(|item| !item.account_id.trim().is_empty())
            .map(|item| RuntimeAccountPoolMemberHealth {
                account_id: item.account_id.trim().to_string(),
                account_email: item.account_email.trim().to_string(),
                available: item.available,
                reason_code: item.reason_code.trim().to_string(),
                reason_message: item.reason_message.trim().to_string(),
            })
            .collect();
    }
    health.last_failure_at = now;
}

fn apply_sidecar_scheduler_state(
    runtime: &mut GatewayRuntime,
    event: &SidecarAuthResultEvent,
    now: i64,
) {
    let account_id = event.account_id.trim();
    let Some(available) = event.auth_available else {
        return;
    };
    if account_id.is_empty() {
        return;
    }

    let health = runtime
        .account_health
        .entry(account_id.to_string())
        .or_default();
    health.sidecar_scheduler_available = Some(available);
    health.sidecar_scheduler_reason = event
        .auth_state_reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "available")
        .map(str::to_string);
    health.sidecar_scheduler_next_retry_at = event.next_retry_at_ms.filter(|value| *value > now);

    let model = event.model.trim();
    if model.is_empty() {
        if available {
            let prefix = format!("{}{}", account_id, COOLDOWN_KEY_SEPARATOR);
            runtime
                .model_cooldowns
                .retain(|key, _| !key.starts_with(&prefix));
        }
        return;
    }
    let Some(cooldown_key) = build_cooldown_key(account_id, model) else {
        return;
    };
    if available {
        runtime.model_cooldowns.remove(&cooldown_key);
        return;
    }
    let Some(next_retry_at_ms) = event.next_retry_at_ms.filter(|value| *value > now) else {
        runtime.model_cooldowns.remove(&cooldown_key);
        return;
    };
    runtime.model_cooldowns.insert(
        cooldown_key,
        AccountModelCooldown {
            model_key: model.to_string(),
            next_retry_at_ms,
            reason: event
                .auth_state_reason
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("unavailable")
                .to_string(),
        },
    );
}

async fn sync_sidecar_scheduler_state(event: &SidecarAuthResultEvent) {
    if event.account_id.trim().is_empty() || event.auth_available.is_none() {
        return;
    }
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    apply_sidecar_scheduler_state(&mut runtime, event, now);
}

fn clear_runtime_account_health(
    runtime: &mut GatewayRuntime,
    account_ids: &[String],
    clear_aggregate_pool_health: bool,
) {
    let account_ids: HashSet<&str> = account_ids
        .iter()
        .map(String::as_str)
        .filter(|account_id| !account_id.trim().is_empty())
        .collect();
    if account_ids.is_empty() {
        return;
    }
    runtime
        .account_health
        .retain(|account_id, _| !account_ids.contains(account_id.as_str()));
    runtime.model_cooldowns.retain(|key, _| {
        !account_ids
            .iter()
            .any(|account_id| key.starts_with(&format!("{}{}", account_id, COOLDOWN_KEY_SEPARATOR)))
    });
    // Keep pool diagnostics for accounts that were not part of this manual
    // recovery. Clearing the whole map makes a single-account recovery look
    // like "recover all" in the health modal.
    runtime.account_pool_health.retain(|_, health| {
        if health.account_statuses.is_empty() {
            // Older Sidecars did not report per-account statuses. There is no
            // safe way to subtract one member from that aggregate diagnostic.
            // A full recovery covers every collection member, so the stale
            // aggregate can be removed without hiding unrelated failures.
            return !clear_aggregate_pool_health;
        }
        health
            .account_statuses
            .retain(|member| !account_ids.contains(member.account_id.as_str()));
        !health.account_statuses.is_empty()
    });
}

async fn request_sidecar_reset_scheduler(
    collection: &CodexLocalAccessCollection,
    port: u16,
    account_ids: &[String],
) -> Result<Vec<String>, String> {
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }
    let client = build_localhost_http_client(Duration::from_secs(5), "账号调度恢复")?;
    let url = format!(
        "http://{}:{}/v1/cockpit/accounts/reset-scheduler",
        CODEX_LOCAL_ACCESS_DEFAULT_CLIENT_URL_HOST, port
    );
    let response = client
        .post(url)
        .bearer_auth(collection.api_key.trim())
        .json(&json!({ "accountIds": account_ids }))
        .send()
        .await
        .map_err(|error| format!("请求 Sidecar 恢复账号状态失败: {}", error))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Sidecar 恢复账号状态失败: HTTP {} {}",
            status,
            body.chars().take(300).collect::<String>()
        ));
    }

    let reset_account_ids = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|payload| payload.get("accountIds").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::trim).map(str::to_string))
        .filter(|account_id| !account_id.is_empty())
        .collect::<Vec<_>>();
    if reset_account_ids.is_empty() {
        return Err("Sidecar 未确认任何账号调度状态".to_string());
    }
    Ok(reset_account_ids)
}

async fn restore_removed_local_access_accounts(account_ids: &[String]) {
    let account_ids = account_ids
        .iter()
        .map(|account_id| account_id.trim().to_string())
        .filter(|account_id| !account_id.is_empty())
        .collect::<Vec<_>>();
    if account_ids.is_empty() {
        return;
    }
    let (collection, port, running) = {
        let runtime = gateway_runtime().lock().await;
        (
            runtime.collection.clone(),
            runtime
                .actual_port
                .or_else(|| runtime.collection.as_ref().map(|collection| collection.port)),
            runtime.running,
        )
    };
    if running {
        if let (Some(collection), Some(port)) = (collection.as_ref(), port) {
            if let Err(error) =
                request_sidecar_reset_scheduler(collection, port, &account_ids).await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 从 API 服务移除账号后恢复调度状态失败: {}",
                    error
                ));
            }
        }
    }
    let mut runtime = gateway_runtime().lock().await;
    clear_runtime_account_health(&mut runtime, &account_ids, false);
    clear_runtime_quota_cooldowns(&mut runtime, &account_ids);
}

fn local_access_recovery_membership(
    current_account_ids: &[String],
    requested_account_ids: &[String],
) -> Result<(Vec<String>, Vec<String>, Vec<String>), String> {
    let requested: HashSet<String> = requested_account_ids
        .iter()
        .map(|account_id| account_id.trim().to_string())
        .filter(|account_id| !account_id.is_empty())
        .collect();
    let selected: Vec<String> = current_account_ids
        .iter()
        .filter(|account_id| requested.contains(account_id.as_str()))
        .cloned()
        .collect();
    if selected.is_empty() {
        return Err("没有找到可恢复的账号".to_string());
    }
    let remaining_account_ids = current_account_ids
        .iter()
        .filter(|account_id| !requested.contains(account_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    Ok((selected, remaining_account_ids, current_account_ids.to_vec()))
}

/// 手动恢复开始后的抑制窗口：这段时间内被恢复账号不再显示账号池异常行，
/// 避免恢复过程中（账号被短暂移出集合）产生的失败状态立刻把行重新点亮。
const LOCAL_ACCESS_RECOVERY_SUPPRESS_WINDOW: Duration = Duration::from_secs(120);
/// 后台恢复流程结束后的短暂抑制窗口，用于吸收仍在飞行的迟到失败事件。
const LOCAL_ACCESS_RECOVERY_SETTLE_WINDOW: Duration = Duration::from_secs(20);
/// 后台恢复结果事件：界面据此提示“恢复失败，可重试”，避免只报“已提交”。
const LOCAL_ACCESS_RECOVERY_RESULT_EVENT: &str = "codex-local-access-recovery-result";

fn suppress_local_access_recovery_rows(
    runtime: &mut GatewayRuntime,
    account_ids: &[String],
    suppressed_until_ms: i64,
) {
    for account_id in account_ids
        .iter()
        .map(|account_id| account_id.trim())
        .filter(|account_id| !account_id.is_empty())
    {
        runtime
            .recovery_suppressed_accounts
            .insert(account_id.to_string(), suppressed_until_ms);
    }
}

fn emit_local_access_recovery_result(account_ids: &[String], error: Option<&str>) {
    let Some(app) = crate::get_app_handle() else {
        return;
    };
    let _ = app.emit(
        LOCAL_ACCESS_RECOVERY_RESULT_EVENT,
        serde_json::json!({
            "accountIds": account_ids,
            "status": if error.is_some() { "failed" } else { "completed" },
            "error": error.unwrap_or_default(),
        }),
    );
}

/// 手动恢复的后台部分：刷新凭据 → 移出集合 → 加回集合（两次网关重载）。
///
/// 这一段的耗时不可控（凭据刷新最长 15s/批，网关重载含 sidecar 重启），因此不再
/// 阻塞界面：宿主先清掉异常状态并返回快照，这里在后台继续执行并回报结果。
#[allow(clippy::too_many_arguments)]
async fn run_local_access_recovery_in_background(
    selected_account_ids: Vec<String>,
    remaining_account_ids: Vec<String>,
    restored_account_ids: Vec<String>,
    restrict_free_accounts: bool,
    backup_account_ids: Vec<String>,
    preferred_account_ids: Vec<String>,
    image_generation_account_policies:
        HashMap<String, CodexLocalAccessImageGenerationPolicy>,
) -> Result<(), String> {
    refresh_managed_accounts_before_recovery(&selected_account_ids).await;

    // Manual recovery must match "remove then add back": persist membership
    // through the same save path so sidecar auth, quota pool, and scheduler
    // state are rebuilt from a fresh account instead of only resetting runtime
    // flags on the still-loaded credential.
    save_local_access_accounts_with_reload(
        remaining_account_ids,
        restrict_free_accounts,
        Some(backup_account_ids.clone()),
        Some(preferred_account_ids.clone()),
        None,
        None,
        Some(image_generation_account_policies.clone()),
        LocalAccessGatewayReload::Await,
    )
    .await?;
    save_local_access_accounts_with_reload(
        restored_account_ids,
        restrict_free_accounts,
        Some(backup_account_ids),
        Some(preferred_account_ids),
        None,
        None,
        Some(image_generation_account_policies),
        LocalAccessGatewayReload::Await,
    )
    .await?;
    Ok(())
}

pub async fn recover_local_access_accounts(
    account_ids: Vec<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    let collection = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .clone()
            .ok_or_else(|| "API 服务集合尚未创建".to_string())?
    };
    let (selected, remaining_account_ids, restored_account_ids) =
        local_access_recovery_membership(&collection.account_ids, &account_ids)?;

    // 先清宿主侧状态并立即把快照返回给界面，让“恢复”点下去就能看到异常行消失；
    // 凭据刷新与网关重载交给后台任务，失败会通过事件回报。
    {
        let mut runtime = gateway_runtime().lock().await;
        let now = now_ms();
        clear_runtime_account_health(&mut runtime, &selected, false);
        clear_runtime_quota_cooldowns(&mut runtime, &selected);
        suppress_local_access_recovery_rows(
            &mut runtime,
            &selected,
            now.saturating_add(
                duration_to_millis(LOCAL_ACCESS_RECOVERY_SUPPRESS_WINDOW) as i64,
            ),
        );
    }
    emit_local_access_state_updated();

    let (backup_account_ids, preferred_account_ids) = routing_priority_ids(&collection);
    let image_generation_account_policies = collection.image_generation_account_policies.clone();
    let restrict_free_accounts = collection.restrict_free_accounts;
    let background_account_ids = selected.clone();
    tauri::async_runtime::spawn(async move {
        let _recovery_guard = local_access_recovery_lock().lock().await;
        let error = run_local_access_recovery_in_background(
            selected,
            remaining_account_ids,
            restored_account_ids,
            restrict_free_accounts,
            backup_account_ids,
            preferred_account_ids,
            image_generation_account_policies,
        )
        .await
        .err();
        {
            let mut runtime = gateway_runtime().lock().await;
            if error.is_none() {
                // 成功：再用一小段抑制窗口吸收仍在飞行的旧请求结果。
                let now = now_ms();
                suppress_local_access_recovery_rows(
                    &mut runtime,
                    &background_account_ids,
                    now.saturating_add(
                        duration_to_millis(LOCAL_ACCESS_RECOVERY_SETTLE_WINDOW) as i64,
                    ),
                );
            } else {
                // 失败：立即解除抑制，让真实的异常行可以重新出现并允许重试。
                for account_id in &background_account_ids {
                    runtime
                        .recovery_suppressed_accounts
                        .remove(account_id.trim());
                }
            }
        }
        if let Some(error) = error.as_deref() {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 手动恢复账号状态失败（异常行已先清空，界面会提示重试）: account_ids={:?}, error={}",
                background_account_ids, error
            ));
        }
        emit_local_access_recovery_result(&background_account_ids, error.as_deref());
        emit_local_access_state_updated();
    });

    snapshot_state_without_gateway_reload().await
}

/// 账号凭据更新（重新授权 / 本机导入）后立即让本地网关生效。
///
/// 重新授权只把新 Token 写进账号库；运行中的 sidecar 仍持有旧 Token，会继续 401，
/// 界面也继续显示上一次的失败原因。这里立刻写穿 auth 文件，并在后台重置该账号在
/// sidecar 调度器里的失败状态、清掉宿主缓存的错误信息。
pub fn notify_account_credentials_updated(account: &CodexAccount) {
    if account.is_api_key_auth() {
        return;
    }
    if let Err(error) = sync_sidecar_auth_file_for_account(account) {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess] 账号凭据更新后同步 sidecar 认证失败: account_id={}, error={}",
            account.id, error
        ));
        return;
    }
    emit_local_access_state_updated();
    let account_id = account.id.clone();
    tauri::async_runtime::spawn(async move {
        reset_scheduler_state_after_credentials_update(account_id).await;
    });
}

async fn reset_scheduler_state_after_credentials_update(account_id: String) {
    if let Err(error) = ensure_runtime_loaded_without_start().await {
        logger::log_codex_api_info(&format!(
            "[CodexLocalAccess] 账号凭据更新后加载运行态失败（跳过调度状态重置）: account_id={}, error={}",
            account_id, error
        ));
        return;
    }
    let (collection, port, running) = {
        let runtime = gateway_runtime().lock().await;
        let Some(collection) = runtime.collection.clone() else {
            return;
        };
        (
            collection.clone(),
            runtime.actual_port.unwrap_or(collection.port),
            runtime.running,
        )
    };
    if !collection
        .account_ids
        .iter()
        .any(|item| item == &account_id)
    {
        return;
    }
    if running {
        match request_sidecar_reset_scheduler(&collection, port, std::slice::from_ref(&account_id))
            .await
        {
            Ok(reset_account_ids) => {
                let mut runtime = gateway_runtime().lock().await;
                clear_runtime_account_health(&mut runtime, &reset_account_ids, false);
                mark_quota_cooldowns_recovered(&mut runtime, &reset_account_ids, now_ms());
                drop(runtime);
                emit_local_access_state_updated();
                return;
            }
            Err(error) => logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 账号凭据更新后重置 sidecar 调度状态失败: account_id={}, error={}",
                account_id, error
            )),
        }
    }
    // 网关未运行或重置失败时，至少清掉宿主侧缓存的失败信息。
    let mut runtime = gateway_runtime().lock().await;
    clear_runtime_account_health(&mut runtime, std::slice::from_ref(&account_id), false);
    drop(runtime);
    emit_local_access_state_updated();
}

/// 上游 401（invalidated oauth token / auth_unavailable）时 access_token 往往还没过期，
/// 仅重置调度状态后下一次请求仍会立刻 401，表现为「点了恢复没反应」。这里复用后台
/// 刷新链路做一次强制刷新并同步 sidecar auth 文件；刷新失败时保持原状，由界面引导
/// 用户重新授权。
async fn refresh_managed_accounts_before_recovery(account_ids: &[String]) {
    const RECOVERY_REFRESH_TIMEOUT: Duration = Duration::from_secs(15);
    stream::iter(account_ids.iter().cloned())
        .for_each_concurrent(GATEWAY_ACCOUNT_REFRESH_CONCURRENCY, |account_id| async move {
            let Some(account) = codex_account::load_account(&account_id) else {
                return;
            };
            if account.is_api_key_auth() || !codex_account::account_has_refresh_token(&account) {
                return;
            }
            match timeout(
                RECOVERY_REFRESH_TIMEOUT,
                codex_account::force_refresh_managed_account(&account_id, "recover"),
            )
            .await
            {
                Ok(Ok(account)) => {
                    if let Err(error) = sync_sidecar_auth_file_for_account(&account) {
                        logger::log_codex_api_warn(&format!(
                            "[CodexLocalAccess] 恢复账号后同步 sidecar 认证失败: account_id={}, error={}",
                            account_id, error
                        ));
                    }
                }
                Ok(Err(error)) => logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 恢复账号时刷新凭据失败（需要重新授权）: account_id={}, error={}",
                    account_id, error
                )),
                Err(_) => logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 恢复账号时刷新凭据超时: account_id={}, timeout_secs={}",
                    account_id,
                    RECOVERY_REFRESH_TIMEOUT.as_secs()
                )),
            }
        })
        .await;
}

async fn update_sidecar_account_health_from_values(
    account_id: &str,
    account_email: &str,
    request_kind: &str,
    success: bool,
    status: Option<u16>,
    error_category: Option<&str>,
    error_message: Option<&str>,
    is_client_canceled: bool,
) {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return;
    }
    let request_kind = parse_sidecar_request_kind(request_kind);
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    let health = runtime
        .account_health
        .entry(account_id.to_string())
        .or_default();
    if !account_email.trim().is_empty() {
        health.email = account_email.trim().to_string();
    }
    if success {
        health.consecutive_failures = 0;
        health.last_success_at = Some(now);
        health.last_failure_at = None;
        health.last_failure_status = None;
        health.last_failure_category = None;
        health.last_failure_message = None;
        if request_kind_is_image(request_kind) {
            health.image_generation_status = CodexLocalAccessImageGenerationStatus::Available;
            health.image_generation_checked_at = Some(now);
        }
        return;
    }
    if is_client_canceled {
        return;
    }

    health.consecutive_failures = health.consecutive_failures.saturating_add(1);
    health.last_failure_at = Some(now);
    health.last_failure_status = status;
    health.last_failure_category = error_category
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    health.last_failure_message = error_message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if error_category == Some("image_generation_not_enabled") {
        health.image_generation_status = CodexLocalAccessImageGenerationStatus::Unavailable;
        health.image_generation_checked_at = Some(now);
    } else if request_kind_is_image(request_kind)
        && health.image_generation_status == CodexLocalAccessImageGenerationStatus::Unknown
    {
        health.image_generation_checked_at = Some(now);
    }
}

fn resolve_recorded_usage_model_id(
    account_id: Option<&str>,
    reported_model: &str,
) -> Option<String> {
    let reported = reported_model.trim();
    if reported.is_empty() {
        return None;
    }
    let Some(account_id) = account_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Some(reported.to_string());
    };
    let Some(account) = codex_account::load_account(account_id) else {
        return Some(reported.to_string());
    };
    let resolved =
        crate::modules::codex_account::resolve_account_upstream_model(&account, reported);
    if resolved.is_empty() {
        Some(reported.to_string())
    } else {
        Some(resolved)
    }
}

async fn record_sidecar_usage_event(event: SidecarUsageEvent) {
    update_sidecar_account_health(&event).await;
    let account_id = non_empty_sidecar_string(&event.account_id);
    let account_email = non_empty_sidecar_string(&event.account_email);
    let api_key_id = non_empty_sidecar_string(&event.api_key_id);
    let api_key_label = non_empty_sidecar_string(&event.api_key_label);
    let client_instance_id = non_empty_sidecar_string(&event.client_instance_id);
    let reported_model = non_empty_sidecar_string(&event.upstream_model)
        .or_else(|| non_empty_sidecar_string(&event.model))
        .or_else(|| non_empty_sidecar_string(&event.alias))
        .unwrap_or_default();
    let upstream_model = resolve_recorded_usage_model_id(account_id.as_deref(), &reported_model);
    // 客户端请求模型优先用 sidecar 新字段，旧事件回退到 alias/上游模型，
    // 保证历史数据与不支持该字段的 sidecar 版本仍能正常入库。
    let requested_model = non_empty_sidecar_string(&event.requested_model)
        .or_else(|| non_empty_sidecar_string(&event.alias))
        .or_else(|| upstream_model.clone());
    let request_id = non_empty_sidecar_string(&event.request_id);
    let error_category = normalized_sidecar_error_category(&event);
    if let Err(error) = record_request_stats_with_meta(
        account_id.as_deref(),
        account_email.as_deref(),
        api_key_id.as_deref(),
        api_key_label.as_deref(),
        upstream_model.as_deref(),
        parse_sidecar_request_kind(&event.request_kind),
        event.success,
        error_category.as_deref(),
        event.latency_ms,
        sidecar_usage_capture(&event.usage),
        RequestStatsMeta {
            request_id: request_id.as_deref(),
            client_instance_id: client_instance_id.as_deref(),
            http_status: event.status,
            error_message: event.error_message.as_deref(),
            service_tier: event.service_tier.as_deref(),
            reasoning_effort: event.reasoning_effort.as_deref(),
            requested_model: requested_model.as_deref(),
            upstream_model: upstream_model.as_deref(),
        },
    )
    .await
    {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess] 写入 sidecar 请求统计失败: {}",
            error
        ));
    }
    schedule_sidecar_auto_restart(&event);
}

type SharedSidecarStartupDiagnostics = Arc<Mutex<SidecarStartupDiagnostics>>;

fn update_sidecar_stdout_diagnostics(
    diagnostics: &SharedSidecarStartupDiagnostics,
    line: &str,
    ready_seen: bool,
) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.last_stdout = clean_diagnostic_text(line);
        if ready_seen {
            diagnostics.ready_seen = true;
        }
    }
}

fn update_sidecar_stderr_diagnostics(diagnostics: &SharedSidecarStartupDiagnostics, line: &str) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.last_stderr = clean_diagnostic_text(line);
    }
}

fn sidecar_startup_diagnostics_text(diagnostics: &SharedSidecarStartupDiagnostics) -> String {
    let diagnostics = diagnostics.lock().ok().map(|item| item.clone());
    let Some(diagnostics) = diagnostics else {
        return "startup_diagnostics_unavailable".to_string();
    };
    format!(
        "ready_seen={}, last_stdout={}, last_stderr={}",
        diagnostics.ready_seen,
        diagnostics
            .last_stdout
            .as_deref()
            .unwrap_or("未捕获 stdout"),
        diagnostics
            .last_stderr
            .as_deref()
            .unwrap_or("未捕获 stderr")
    )
}

fn sidecar_ready_signal_from_value(value: &Value) -> SidecarReadySignal {
    let port = value
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok());
    let host = value
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    SidecarReadySignal { host, port }
}

async fn handle_sidecar_stdout_line(
    line: &str,
    ready_sender: &mut Option<oneshot::Sender<SidecarReadySignal>>,
    diagnostics: &SharedSidecarStartupDiagnostics,
) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    update_sidecar_stdout_diagnostics(diagnostics, trimmed, false);
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        logger::log_codex_api_info(&format!("[CodexLocalAccess][sidecar] {}", trimmed));
        return;
    };
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "usage" => match serde_json::from_value::<SidecarUsageEvent>(value) {
            Ok(event) => record_sidecar_usage_event(event).await,
            Err(error) => logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] sidecar usage 事件解析失败: {}",
                error
            )),
        },
        "auth_result" => {
            match serde_json::from_value::<SidecarAuthResultEvent>(value.clone()) {
                Ok(event) => update_sidecar_auth_result_health(&event, false).await,
                Err(error) => logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] sidecar auth_result 事件解析失败: {}",
                    error
                )),
            }
            logger::log_codex_api_info(&format!("[CodexLocalAccess][sidecar] {}", trimmed));
        }
        "auth_pool_result" => {
            match serde_json::from_value::<SidecarAuthResultEvent>(value.clone()) {
                Ok(event) => update_sidecar_auth_result_health(&event, true).await,
                Err(error) => logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] sidecar auth_pool_result 事件解析失败: {}",
                    error
                )),
            }
            logger::log_codex_api_info(&format!("[CodexLocalAccess][sidecar] {}", trimmed));
        }
        "ready" => {
            update_sidecar_stdout_diagnostics(diagnostics, trimmed, true);
            if let Some(sender) = ready_sender.take() {
                let _ = sender.send(sidecar_ready_signal_from_value(&value));
            }
            logger::log_codex_api_info(&format!("[CodexLocalAccess] sidecar 已就绪: {}", trimmed));
        }
        "error" => {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(trimmed)
                .to_string();
            {
                let mut runtime = gateway_runtime().lock().await;
                runtime.last_error = Some(message.clone());
            }
            logger::log_codex_api_warn(&format!("[CodexLocalAccess] sidecar 错误: {}", message));
        }
        _ => {
            logger::log_codex_api_info(&format!("[CodexLocalAccess][sidecar] {}", trimmed));
        }
    }
}

async fn drain_sidecar_stdout(
    stdout: tokio::process::ChildStdout,
    ready_sender: oneshot::Sender<SidecarReadySignal>,
    diagnostics: SharedSidecarStartupDiagnostics,
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut ready_sender = Some(ready_sender);
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                handle_sidecar_stdout_line(&line, &mut ready_sender, &diagnostics).await
            }
            Ok(None) => break,
            Err(error) => {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 读取 sidecar stdout 失败: {}",
                    error
                ));
                break;
            }
        }
    }
}

async fn drain_sidecar_stderr(
    stderr: tokio::process::ChildStderr,
    diagnostics: SharedSidecarStartupDiagnostics,
) {
    let mut lines = BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    update_sidecar_stderr_diagnostics(&diagnostics, trimmed);
                    logger::log_codex_api_warn(&format!("[CodexLocalAccess][sidecar] {}", trimmed));
                }
            }
            Ok(None) => break,
            Err(error) => {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 读取 sidecar stderr 失败: {}",
                    error
                ));
                break;
            }
        }
    }
}

async fn wait_for_sidecar_ready(
    ready_receiver: &mut oneshot::Receiver<SidecarReadySignal>,
    child: &mut Child,
    expected_generation: Option<u64>,
) -> Result<SidecarReadySignal, String> {
    let started_at = Instant::now();

    loop {
        if expected_generation.is_some_and(gateway_lifecycle_generation_changed) {
            return Err(GATEWAY_PREPARATION_CANCELLED.to_string());
        }
        let Some(remaining) = SIDECAR_READY_TIMEOUT.checked_sub(started_at.elapsed()) else {
            return Err("API 服务 sidecar 启动后未收到 ready 事件".to_string());
        };
        if remaining.is_zero() {
            return Err("API 服务 sidecar 启动后未收到 ready 事件".to_string());
        }

        let poll_interval = remaining.min(Duration::from_millis(120));
        tokio::select! {
            result = &mut *ready_receiver => {
                return result.map_err(|_| "API 服务 sidecar stdout 在 ready 前关闭".to_string());
            }
            _ = tokio::time::sleep(poll_interval) => {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        return Err(format!("API 服务 sidecar 在 ready 前退出: {}", status));
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return Err(format!("检查 API 服务 sidecar ready 状态失败: {}", error));
                    }
                }
            }
        }
    }
}

async fn probe_sidecar_ready_once(
    collection: &CodexLocalAccessCollection,
    request_timeout: Duration,
) -> Result<(), String> {
    probe_sidecar_ready_endpoint(collection.port, &collection.api_key, request_timeout).await
}

async fn probe_sidecar_ready_endpoint(
    port: u16,
    api_key: &str,
    request_timeout: Duration,
) -> Result<(), String> {
    let url = format!(
        "http://{}:{}/v1/models",
        CODEX_LOCAL_ACCESS_DEFAULT_CLIENT_URL_HOST, port
    );
    let client = build_localhost_http_client(request_timeout, "sidecar 健康检测")?;
    match client
        .get(&url)
        .bearer_auth(api_key.trim())
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => Ok(()),
        Ok(response) => Err(format!("HTTP {}", response.status())),
        Err(error) => Err(error.to_string()),
    }
}

fn bind_host_for_access_scope(scope: CodexLocalAccessScope) -> &'static str {
    match scope {
        CodexLocalAccessScope::Localhost => CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST,
        CodexLocalAccessScope::Lan => CODEX_LOCAL_ACCESS_LAN_BIND_HOST,
    }
}

fn bind_host_for_collection(collection: &CodexLocalAccessCollection) -> &'static str {
    bind_host_for_access_scope(collection.access_scope)
}

#[derive(Debug)]
struct LanIpv4Candidate {
    interface_name: String,
    addr: Ipv4Addr,
}

fn resolve_primary_lan_ipv4() -> Option<Ipv4Addr> {
    let mut candidates = collect_private_lan_ipv4_candidates();
    candidates.sort_by_key(|candidate| {
        (
            lan_interface_score(&candidate.interface_name),
            lan_addr_score(candidate.addr),
            candidate.addr.octets(),
        )
    });
    candidates
        .into_iter()
        .next()
        .map(|candidate| candidate.addr)
}

fn is_lan_ipv4(addr: Ipv4Addr) -> bool {
    addr.is_private()
}

fn lan_interface_score(interface_name: &str) -> u8 {
    let name = interface_name.to_ascii_lowercase();
    if name.starts_with("en")
        || name.starts_with("eth")
        || name.starts_with("wlan")
        || name.starts_with("wi-fi")
        || name.starts_with("wifi")
        || name.starts_with("ethernet")
        || name.contains("wireless")
    {
        return 0;
    }
    if name.starts_with("lo")
        || name.starts_with("utun")
        || name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("awdl")
        || name.starts_with("llw")
        || name.starts_with("bridge")
        || name.starts_with("br-")
        || name.starts_with("docker")
        || name.starts_with("veth")
        || name.starts_with("virbr")
        || name.starts_with("vmnet")
        || name.starts_with("vbox")
        || name.starts_with("tailscale")
        || name.starts_with("wg")
    {
        return 2;
    }
    1
}

fn lan_addr_score(addr: Ipv4Addr) -> u8 {
    let octets = addr.octets();
    if octets[0] == 192 && octets[1] == 168 {
        return 0;
    }
    if octets[0] == 10 {
        return 1;
    }
    2
}

#[cfg(target_os = "macos")]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    let output = StdCommand::new("ifconfig").arg("-a").output();
    match output {
        Ok(output) => parse_ifconfig_ipv4_candidates(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

#[cfg(target_os = "linux")]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    let output = StdCommand::new("ip")
        .args(["-o", "-4", "addr", "show", "scope", "global"])
        .output();
    match output {
        Ok(output) => parse_linux_ip_addr_candidates(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

#[cfg(target_os = "windows")]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    let mut command = StdCommand::new("ipconfig");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    match command.output() {
        Ok(output) => parse_windows_ipconfig_candidates(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn parse_ifconfig_ipv4_candidates(output: &str) -> Vec<LanIpv4Candidate> {
    let mut candidates = Vec::new();
    let mut current_interface = String::new();
    for line in output.lines() {
        if !line
            .chars()
            .next()
            .map(|item| item.is_whitespace())
            .unwrap_or(false)
        {
            current_interface = line
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            continue;
        }
        let mut parts = line.split_whitespace();
        while let Some(part) = parts.next() {
            if part != "inet" {
                continue;
            }
            let Some(raw_addr) = parts.next() else {
                continue;
            };
            if let Ok(addr) = raw_addr.parse::<Ipv4Addr>() {
                if is_lan_ipv4(addr) {
                    candidates.push(LanIpv4Candidate {
                        interface_name: current_interface.clone(),
                        addr,
                    });
                }
            }
        }
    }
    candidates
}

#[cfg(target_os = "linux")]
fn parse_linux_ip_addr_candidates(output: &str) -> Vec<LanIpv4Candidate> {
    let mut candidates = Vec::new();
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        let _index = parts.next();
        let Some(interface_name) = parts.next() else {
            continue;
        };
        while let Some(part) = parts.next() {
            if part != "inet" {
                continue;
            }
            let Some(raw_addr) = parts.next() else {
                continue;
            };
            let addr_text = raw_addr.split('/').next().unwrap_or_default();
            if let Ok(addr) = addr_text.parse::<Ipv4Addr>() {
                if is_lan_ipv4(addr) {
                    candidates.push(LanIpv4Candidate {
                        interface_name: interface_name.trim_end_matches(':').to_string(),
                        addr,
                    });
                }
            }
        }
    }
    candidates
}

#[cfg(target_os = "windows")]
fn parse_windows_ipconfig_candidates(output: &str) -> Vec<LanIpv4Candidate> {
    let mut candidates = Vec::new();
    let mut current_interface = String::new();
    for line in output.lines() {
        let trimmed = line.trim();
        let is_indented = line
            .chars()
            .next()
            .map(|item| item.is_whitespace())
            .unwrap_or(false);
        if trimmed.ends_with(':') && !is_indented {
            current_interface = trimmed.trim_end_matches(':').to_string();
            continue;
        }
        if !trimmed.contains("IPv4") {
            continue;
        }
        let Some(raw_addr) = trimmed.rsplit(':').next() else {
            continue;
        };
        if let Ok(addr) = raw_addr.trim().parse::<Ipv4Addr>() {
            if is_lan_ipv4(addr) {
                candidates.push(LanIpv4Candidate {
                    interface_name: current_interface.clone(),
                    addr,
                });
            }
        }
    }
    candidates
}

fn build_runtime_account(
    base_url: String,
    api_key: String,
    bound_oauth_account_id: Option<String>,
    supports_websockets: bool,
) -> CodexAccount {
    let mut runtime_account = CodexAccount::new_api_key(
        CODEX_LOCAL_ACCESS_RUNTIME_ACCOUNT_ID.to_string(),
        "api-service-local".to_string(),
        api_key,
        CodexApiProviderMode::Custom,
        Some(base_url),
        Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID.to_string()),
        Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_NAME.to_string()),
        Vec::new(),
    );
    runtime_account.account_name = Some("API Service".to_string());
    runtime_account.bound_oauth_account_id = bound_oauth_account_id;
    runtime_account.api_model_catalog = api_service_supported_codex_model_ids();
    runtime_account.api_wire_api = Some("responses".to_string());
    runtime_account.api_supports_websockets = supports_websockets;
    runtime_account
}

fn profile_api_key_supports_websockets(
    collection: &CodexLocalAccessCollection,
    api_key: &str,
) -> bool {
    collection.responses_websockets_enabled
        && collection
            .api_keys
            .iter()
            .find(|item| item.enabled && item.key.trim() == api_key.trim())
            .map(|item| item.provider_gateway.is_none() && item.model_routing.is_none())
            .unwrap_or(true)
}

/// API 服务 profile 模型目录里的一个条目。
///
/// `template` 为该模型的官方模板（例如 DeepSeek 官方 models.json 条目）：存在时按模板补齐
/// 上下文窗口、推理档位、识图声明等字段，与 DeepSeek 网关模式写出的目录保持一致。
#[derive(Debug, Clone)]
struct ProfileModelDefinition {
    model_id: String,
    display_name: String,
    template: Option<Value>,
    image_capable: bool,
}

impl ProfileModelDefinition {
    fn plain(model_id: &str, display_name: &str) -> Self {
        Self {
            model_id: model_id.trim().to_string(),
            display_name: display_name.trim().to_string(),
            template: None,
            image_capable: false,
        }
    }

    /// 账号模型：命中官方模板时沿用模板字段，并保留「图片自动转识图模型」的可发送能力。
    fn with_account_model(
        model_id: &str,
        display_name: &str,
        accounts: &[CodexAccount],
        image_capable: bool,
    ) -> Self {
        let mut definition = Self::plain(model_id, display_name);
        definition.image_capable = image_capable;
        definition.template = official_deepseek_template_for_model(accounts, &definition.model_id);
        definition
    }
}

/// 官方 DeepSeek 账号的模型模板（按账号的模型列表/别名解析），用于还原官方显示名与能力字段。
fn official_deepseek_template_for_model(
    accounts: &[CodexAccount],
    model_id: &str,
) -> Option<Value> {
    let key = model_id.trim();
    if key.is_empty() {
        return None;
    }
    for account in accounts {
        if !is_official_deepseek_account(account)
            || !automatic_api_service_account_model_slots(account)
                .iter()
                .any(|(client, _)| client.eq_ignore_ascii_case(key))
        {
            continue;
        }
        if let Some(template) = account_model_template(account, key) {
            return Some(template);
        }
    }
    None
}

fn write_local_access_profile_model_catalog(
    profile_dir: &Path,
    supports_websockets: bool,
    definitions: &[ProfileModelDefinition],
) -> Result<(), String> {
    let pairs = definitions
        .iter()
        .map(|definition| (definition.model_id.clone(), definition.display_name.clone()))
        .collect::<Vec<_>>();
    let mut client_models =
        codex_protocol::build_codex_client_models_response_with_model_definitions(&pairs);
    apply_profile_model_definition_overrides(&mut client_models, definitions);
    write_local_access_profile_client_models(profile_dir, supports_websockets, client_models)
}

/// 客户端按 `priority` 升序展示模型，这里让 GPT 官方推荐集固定排在最前面，
/// 其后是额度兜底模型，最后才是账号自带的第三方模型。
fn apply_profile_model_ordering(client_models: &mut Value) {
    let Some(models) = client_models.get_mut("models").and_then(Value::as_array_mut) else {
        return;
    };
    let reserve_priority = LOCAL_GATEWAY_VISIBLE_GPT_MODELS.len() as i64;
    let account_priority_base = reserve_priority + 1;
    let mut account_index = 0_i64;
    for model in models.iter_mut() {
        let slug = model
            .get("slug")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default();
        let hidden = model
            .get("visibility")
            .and_then(Value::as_str)
            .is_some_and(|visibility| visibility.eq_ignore_ascii_case("hide"));
        let priority = if let Some(index) = LOCAL_GATEWAY_VISIBLE_GPT_MODELS
            .iter()
            .position(|(model_id, _)| model_id.eq_ignore_ascii_case(&slug))
        {
            index as i64
        } else if slug.eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID) {
            reserve_priority
        } else if hidden {
            // 隐藏模型不参与选择器展示，放在最后避免影响可见顺序。
            1000 + account_index
        } else {
            let priority = account_priority_base + account_index;
            account_index += 1;
            priority
        };
        if let Some(object) = model.as_object_mut() {
            object.insert("priority".to_string(), json!(priority));
        }
    }
}

/// 按账号模型的定义补齐目录字段：官方模板（上下文窗口/推理档位等）与识图可发送能力。
fn apply_profile_model_definition_overrides(
    client_models: &mut Value,
    definitions: &[ProfileModelDefinition],
) {
    let Some(models) = client_models.get_mut("models").and_then(Value::as_array_mut) else {
        return;
    };
    for model in models.iter_mut() {
        let slug = model
            .get("slug")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default();
        let Some(definition) = definitions
            .iter()
            .find(|definition| definition.model_id.eq_ignore_ascii_case(&slug))
        else {
            continue;
        };
        if let (Some(template), Some(object)) =
            (definition.template.as_ref().and_then(Value::as_object), model.as_object_mut())
        {
            for (key, value) in template {
                // slug / 展示字段由我们自己的定义决定，comp_hash 统一由目录生成器决定，
                // 其余按官方模板补齐。
                if matches!(
                    key.as_str(),
                    "slug" | "display_name" | "description" | "visibility" | "comp_hash"
                ) {
                    continue;
                }
                object.insert(key.clone(), value.clone());
            }
        }
        // DeepSeek 官方模板可能含 null，不能覆盖最终的协作声明。
        codex_protocol::apply_deepseek_multi_agent_capability(model);
        if definition.image_capable {
            if let Some(object) = model.as_object_mut() {
                // 与 DeepSeek 网关模式一致：请求里的图片会由网关自动转到识图模型，
                // 因此这里声明为可发送图片，否则客户端会直接剥离图片。
                object.insert("input_modalities".to_string(), json!(["text", "image"]));
                object.insert("supports_image_detail_original".to_string(), json!(true));
            }
        }
    }
}

/// API 服务 profile 的客户端模型目录清单。
///
/// 基线沿用「模型管理」清单（已开启时）或官方模型清单，再补入当前 API Key 可路由的
/// 账号模型（例如 DeepSeek 等 Chat / 第三方账号），这样客户端模型选择器才能显示并切换它们。
///
/// 客户端内部需要的隐藏模型（自动评审、生图）保留元数据，但不显示在选择器里。
fn local_access_profile_hidden_model_definitions() -> Vec<(String, String)> {
    codex_protocol::build_codex_client_models_response(&supported_codex_model_ids())
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|model| {
            model
                .get("visibility")
                .and_then(Value::as_str)
                .is_some_and(|visibility| visibility.eq_ignore_ascii_case("hide"))
        })
        .filter_map(|model| {
            let slug = model.get("slug").and_then(Value::as_str)?.trim();
            if slug.is_empty() {
                return None;
            }
            let display_name = model
                .get("display_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(slug)
                .to_string();
            Some((slug.to_string(), display_name))
        })
        .collect()
}

/// API 服务启动预览：即使实例 profile 还没被接管，也按账号池算出本次会渲染的模型清单。
pub(crate) fn api_service_preview_model_definitions(
    profile_dir: &Path,
) -> Option<Vec<crate::models::codex::CodexExperimentalModelDefinition>> {
    let collection = load_collection_from_disk().ok().flatten()?;
    let definitions =
        local_access_profile_model_definitions(profile_dir, &collection, &collection.api_key, true)
            .ok()?;
    // 客户端内部隐藏条目（生图、自动审查等）只服务于请求链路，不进入启动预览列表。
    let hidden: HashSet<String> = local_access_profile_hidden_model_definitions()
        .into_iter()
        .map(|(model_id, _)| model_id.to_ascii_lowercase())
        .collect();
    Some(
        definitions
            .into_iter()
            .filter(|definition| !hidden.contains(&definition.model_id.to_ascii_lowercase()))
            .map(|definition| crate::models::codex::CodexExperimentalModelDefinition {
                model_id: definition.model_id,
                display_name: definition.display_name,
                reasoning_efforts: None,
                context_window: None,
                auto_compact_token_limit: None,
            })
            .collect(),
    )
}

fn local_access_profile_model_definitions(
    profile_dir: &Path,
    collection: &CodexLocalAccessCollection,
    api_key: &str,
    include_account_pool_models: bool,
) -> Result<Vec<ProfileModelDefinition>, String> {
    let experimental_model_catalog_enabled =
        codex_account::read_quick_config_from_config_toml(profile_dir)?
            .experimental_model_catalog_enabled;
    // 该 profile 能承接的账号池里没有 GPT / Codex 能力时，客户端选择器只展示账号池自己的模型
    // （例如只加了 Grok 账号就只显示 Grok 模型），不再无条件塞入官方推荐 GPT 集。
    // 这里必须只看对话账号：仅用于生图转发的 OAuth 账号不承接对话模型，否则「绑定 OAuth
    // 生图账号」会让只加了 DeepSeek / Grok 的池也展示整套官方 GPT 模型。
    let pool_accounts: Vec<CodexAccount> = conversation_sidecar_account_ids(collection)
        .into_iter()
        .filter_map(|account_id| codex_account::load_account(&account_id))
        .filter(|account| {
            is_local_access_eligible_account(account, collection.restrict_free_accounts)
        })
        .collect();
    let include_official_gpt_models = pool_provides_gpt_models(&pool_accounts);
    let user_took_over_catalog =
        codex_account::has_saved_experimental_model_definitions(profile_dir);
    let mut definitions: Vec<ProfileModelDefinition> = if experimental_model_catalog_enabled {
        let saved_definitions = codex_account::read_experimental_model_definitions(profile_dir);
        let models = if user_took_over_catalog {
            // 用户已接管模型清单：完全以用户定义为准，不再叠加账号池模型或按池过滤。
            saved_definitions
        } else {
            overlay_rendered_pool_models_on_experimental_catalog(profile_dir, saved_definitions)
                .into_iter()
                .filter(|model| {
                    include_official_gpt_models
                        || !is_official_gpt_or_reserve_catalog_model(&model.model_id)
                })
                .collect()
        };
        models
            .into_iter()
            .map(|model| ProfileModelDefinition::plain(&model.model_id, &model.display_name))
            .collect()
    } else {
        // 只暴露官方推荐集里的 GPT 模型（显示名与官方客户端一致），外加客户端内部需要的隐藏条目。
        let mut definitions = if include_official_gpt_models {
            local_gateway_visible_gpt_model_definitions()
                .into_iter()
                .map(|(model_id, display_name)| {
                    ProfileModelDefinition::plain(&model_id, &display_name)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        definitions.extend(
            local_access_profile_hidden_model_definitions()
                .into_iter()
                .map(|(model_id, display_name)| {
                    ProfileModelDefinition::plain(&model_id, &display_name)
                }),
        );
        definitions
    };
    // GPT 官方推荐集的显示名始终跟随官方客户端（带 `GPT-` 前缀），
    // 即使用户在「模型管理」里用了别的名字，API 服务 profile 也保持官方命名。
    for definition in definitions.iter_mut() {
        if let Some((_, official_name)) = LOCAL_GATEWAY_VISIBLE_GPT_MODELS
            .iter()
            .find(|(model_id, _)| model_id.eq_ignore_ascii_case(&definition.model_id))
        {
            definition.display_name = (*official_name).to_string();
        }
        if definition
            .model_id
            .eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID)
        {
            definition.display_name = codex_protocol::CODEX_RESERVE_DISPLAY_NAME.to_string();
        }
    }
    if !include_account_pool_models {
        return Ok(definitions);
    }
    let Some(resolved_key) = resolve_collection_api_key(collection, api_key) else {
        return Ok(definitions);
    };
    let accounts = codex_account::list_accounts_checked().unwrap_or_default();
    let mut seen = definitions
        .iter()
        .map(|definition| definition.model_id.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for (model_id, image_capable) in
        automatic_api_service_profile_extra_models(collection, &resolved_key, &accounts)
    {
        if !seen.insert(model_id.to_ascii_lowercase()) {
            continue;
        }
        let display_name = codex_account::provider_model_display_name(&model_id);
        definitions.push(ProfileModelDefinition::with_account_model(
            &model_id,
            &display_name,
            &accounts,
            image_capable,
        ));
    }
    Ok(definitions)
}

/// 写入 profile 的 Codex 模型目录。
///
/// `definitions` 为 `Some` 时按给定清单（受管模型目录 / 混合路由临时目录）生成，
/// 为 `None` 时使用官方模型清单。该写入只作用于当前 profile，不代表用户开启了模型管理。
fn write_local_access_profile_model_catalog_with_definitions(
    profile_dir: &Path,
    supports_websockets: bool,
    definitions: Option<Vec<(String, String)>>,
) -> Result<(), String> {
    let client_models = match definitions.as_deref() {
        Some(definitions) => {
            codex_protocol::build_codex_client_models_response_with_model_definitions(definitions)
        }
        None => codex_protocol::build_codex_client_models_response(&supported_codex_model_ids()),
    };
    write_local_access_profile_client_models(profile_dir, supports_websockets, client_models)
}

fn write_local_access_profile_client_models(
    profile_dir: &Path,
    supports_websockets: bool,
    mut client_models: Value,
) -> Result<(), String> {
    // 只有目录里已经有官方 GPT / Codex 模型（或用户显式列出 gpt-reserve）时才追加额度兜底
    // 条目。账号池没有能承接官方模型的账号时（例如只加了 DeepSeek + Grok 账号），客户端
    // 选择器里不应再出现 GPT-5.6 Reserve。
    if profile_catalog_allows_reserve(&client_models) {
        codex_protocol::ensure_codex_reserve_fallback(&mut client_models);
    }
    apply_profile_model_ordering(&mut client_models);
    if let Some(models) = client_models
        .get_mut("models")
        .and_then(Value::as_array_mut)
    {
        for model in models {
            model["prefer_websockets"] = json!(supports_websockets);
        }
    }
    let catalog = json!({
        "models": client_models
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    });
    let content = serde_json::to_string_pretty(&catalog)
        .map_err(|e| format!("生成 Codex API 服务模型目录失败: {}", e))?;
    // Apply profile overrides after templates and pool models have been merged;
    // startup and passive reconciliation must use the same final values.
    let content = codex_account::decorate_managed_model_catalog_for_profile(profile_dir, &content)?;
    let catalog_file = CODEX_MANAGED_MODEL_CATALOG_FILE;
    let catalog_changed = write_string_atomic_if_changed(&profile_dir.join(catalog_file), &content)
        .map_err(|e| format!("写入 Codex API 服务模型目录失败: {}", e))?;
    if let Err(err) = crate::modules::codex_managed_model_catalog_version::write_managed_catalog_meta(
        &profile_dir.join(catalog_file),
    ) {
        logger::log_codex_api_warn(&format!(
            "[Codex模型目录] 写入 API 服务模型目录版本戳失败: path={}, error={}",
            profile_dir.join(catalog_file).display(),
            err
        ));
    }
    codex_account::cleanup_legacy_managed_model_catalogs(profile_dir);
    if catalog_changed {
        invalidate_codex_model_cache(profile_dir)?;
    }

    let config_path = profile_config_path(profile_dir);
    let mut doc = crate::modules::codex_config_format::load_codex_config_doc(&config_path)?;
    if doc.get("model_catalog_json").and_then(|item| item.as_str()) == Some(catalog_file) {
        return Ok(());
    }
    doc["model_catalog_json"] = value(catalog_file);
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
}

/// 客户端模型目录里是否已有官方 GPT / Codex 模型（或用户显式列出的 `gpt-reserve`）。
///
/// `ensure_codex_reserve_fallback` 会强制把兜底条目设为可见，因此只在目录本身就有官方
/// GPT 模型、或用户自己就列了 `gpt-reserve` 时调用；否则（例如账号池只有 DeepSeek /
/// Grok）会把没有任何账号可承接的 `GPT-5.6 Reserve` 塞进选择器。
fn profile_catalog_allows_reserve(client_models: &Value) -> bool {
    let Some(models) = client_models.get("models").and_then(Value::as_array) else {
        return false;
    };
    models.iter().any(|model| {
        model
            .get("slug")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|slug| {
                slug.eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID)
                    || LOCAL_GATEWAY_VISIBLE_GPT_MODELS
                        .iter()
                        .any(|(model_id, _)| model_id.eq_ignore_ascii_case(slug))
            })
    })
}

pub(crate) fn invalidate_codex_model_cache(profile_dir: &Path) -> Result<(), String> {
    let cache_path = profile_dir.join(CODEX_MODEL_CACHE_FILE);
    match std::fs::remove_file(&cache_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("清理 Codex 模型缓存失败: {}", error)),
    }
}

fn write_mixed_model_realtime_sideband_override(
    profile_dir: &Path,
    collection: &CodexLocalAccessCollection,
    api_key: &str,
) -> Result<(), String> {
    let is_mixed = collection.api_keys.iter().any(|item| {
        item.enabled
            && item.key.trim() == api_key.trim()
            && item.provider_gateway.is_none()
            && item.model_routing.as_ref().is_some_and(|routing| {
                routing.default_route.eq_ignore_ascii_case("oauth")
            })
    });
    if !is_mixed {
        return Ok(());
    }
    let path = profile_config_path(profile_dir);
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取 Codex 语音接管配置失败: {}", error))?;
    let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&content)
        .map_err(|error| format!("解析 Codex 语音接管配置失败: {}", error))?;
    // WebRTC sideband ignores model provider.base_url by default but reuses its
    // bearer. Keep call creation and sideband on the same gateway/account.
    // An explicit user override is outside Cockpit's ownership.
    if doc.get("experimental_realtime_ws_base_url").is_some() {
        return Ok(());
    }
    doc["experimental_realtime_ws_base_url"] = value(build_collection_base_url(collection));
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&path, &content)
}

async fn write_local_access_profile_takeover(
    profile_dir: &Path,
    collection: &CodexLocalAccessCollection,
    api_key: Option<&str>,
    include_account_pool_models: bool,
) -> Result<(), String> {
    let bound_oauth_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref());
    if let Some(bound_id) = bound_oauth_account_id.as_deref() {
        let _ = validate_local_access_bound_oauth_account(bound_id)?;
        let _ = codex_account::ensure_managed_account_fresh(bound_id).await?;
    }
    let runtime_api_key = api_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| collection.api_key.clone());
    let supports_websockets = profile_api_key_supports_websockets(collection, &runtime_api_key);
    let runtime_account = build_runtime_account(
        build_collection_base_url(collection),
        runtime_api_key.clone(),
        bound_oauth_account_id,
        supports_websockets,
    );
    codex_account::write_account_bundle_to_dir(profile_dir, &runtime_account)?;
    write_mixed_model_realtime_sideband_override(profile_dir, collection, &runtime_api_key)?;
    let definitions = local_access_profile_model_definitions(
        profile_dir,
        collection,
        &runtime_api_key,
        include_account_pool_models,
    )?;
    write_local_access_profile_model_catalog(
        profile_dir,
        supports_websockets,
        &definitions,
    )?;
    if include_account_pool_models {
        // API 服务接管：客户端「可用推理强度」默认不含 max，而账号模型（例如 DeepSeek）
        // 只声明 low/high/max；这里补齐最高档，保证选择器里的档位与 DeepSeek 网关模式一致。
        ensure_profile_max_reasoning_effort(profile_dir)?;
        // 账号池里含 DeepSeek 时压缩不能走远程：DeepSeek 没有服务端压缩，请求一旦被路由过去
        // 必然失败（详见 ensure_local_compaction_for_account_pool 的说明）。
        ensure_local_compaction_for_account_pool(profile_dir, collection)?;
        remember_takeover_ownership(profile_dir)?;
    }
    Ok(())
}

/// 确保 profile 的客户端「可用推理强度」包含 `max`。
///
/// 只做增量补充：已有档位与其顺序保持不变，仅在配置缺失时以客户端默认集合为底，
/// 避免缩小用户已经开放的推理档位。
fn ensure_profile_max_reasoning_effort(profile_dir: &Path) -> Result<(), String> {
    const DESKTOP_TABLE: &str = "desktop";
    const EFFORT_KEY: &str = "enabled-reasoning-efforts";
    const MAX_EFFORT: &str = "max";
    /// Codex 桌面端「可用推理强度」的内置默认值，仅在配置缺失时作为兜底写入。
    const CLIENT_DEFAULT_EFFORTS: [&str; 6] =
        ["low", "medium", "high", "xhigh", "ultra", "persistent"];

    let config_path = profile_config_path(profile_dir);
    if !config_path.exists() {
        return Ok(());
    }
    let mut doc = crate::modules::codex_config_format::load_codex_config_doc(&config_path)?;
    if doc.get(DESKTOP_TABLE).is_none() {
        doc[DESKTOP_TABLE] = toml_edit::table();
    }
    let Some(desktop) = doc[DESKTOP_TABLE].as_table_mut() else {
        // `desktop` 不是表结构说明是用户自定义内容，保持原样，不做覆盖。
        return Ok(());
    };
    let mut changed = false;
    match desktop.get_mut(EFFORT_KEY) {
        Some(item) => {
            let Some(efforts) = item.as_array_mut() else {
                // 配置项不是数组时说明不是客户端写入的结构，保持原样。
                return Ok(());
            };
            let has_max = efforts.iter().any(|effort| {
                effort
                    .as_str()
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case(MAX_EFFORT))
            });
            if !has_max {
                efforts.push(MAX_EFFORT);
                changed = true;
            }
        }
        None => {
            let mut efforts = toml_edit::Array::new();
            for effort in CLIENT_DEFAULT_EFFORTS {
                efforts.push(effort);
            }
            efforts.push(MAX_EFFORT);
            desktop[EFFORT_KEY] = toml_edit::value(efforts);
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
}

fn push_local_access_takeover_dir(
    dirs: &mut Vec<PathBuf>,
    seen: &mut HashSet<String>,
    profile_dir: PathBuf,
) {
    let profile_key = normalize_profile_dir_key(&profile_dir);
    if profile_key.is_empty() || !seen.insert(profile_key) {
        return;
    }
    dirs.push(profile_dir);
}

fn collect_local_access_profile_takeover_dirs_from_store(
    store: crate::models::InstanceStore,
    default_profile: PathBuf,
    include_default_profile: bool,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    if include_default_profile
        && store
            .default_settings
            .bind_account_id
            .as_deref()
            .map(crate::modules::codex_instance::is_api_service_bind_account_id)
            .unwrap_or(false)
    {
        push_local_access_takeover_dir(&mut dirs, &mut seen, default_profile);
    }

    for instance in store.instances {
        let Some(bind_account_id) = instance.bind_account_id.as_deref() else {
            continue;
        };
        if !crate::modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
            continue;
        }
        let profile_text = instance.user_data_dir.trim();
        if profile_text.is_empty() {
            continue;
        }
        push_local_access_takeover_dir(&mut dirs, &mut seen, PathBuf::from(profile_text));
    }

    dirs
}

fn should_include_default_profile_for_takeover(
    is_dev_profile: bool,
    default_profile_is_oauth_runtime: bool,
) -> bool {
    !is_dev_profile && !default_profile_is_oauth_runtime
}

fn collect_local_access_profile_takeover_dirs() -> Vec<PathBuf> {
    let store = match crate::modules::codex_instance::load_instance_store() {
        Ok(store) => store,
        Err(err) => {
            logger::log_codex_api_warn(&format!(
                "Codex API 服务加载 Codex 应用多开失败，跳过自动接管配置: {}",
                err
            ));
            return Vec::new();
        }
    };

    // Dev and production keep separate app data, but the official default Codex
    // profile is shared. Never let an automatically restored gateway claim a
    // default profile that is currently being used by an OAuth-backed official
    // Codex process from the other environment.
    let default_profile_is_oauth_runtime = {
        let default_profile = codex_account::get_codex_home();
        let default_key = normalize_profile_dir_key(&default_profile);
        process::collect_codex_process_entries()
            .into_iter()
            .any(|(_, runtime_home)| {
                (runtime_home.is_none()
                    || runtime_home
                        .as_deref()
                        .map(Path::new)
                        .map(normalize_profile_dir_key)
                        .is_some_and(|key| key == default_key))
                    && codex_account::oauth_account_id_for_runtime_dir(&default_profile).is_some()
            })
    };
    collect_local_access_profile_takeover_dirs_from_store(
        store,
        codex_account::get_codex_home(),
        should_include_default_profile_for_takeover(
            account::is_dev_profile(),
            default_profile_is_oauth_runtime,
        ),
    )
}

async fn ensure_profile_takeover(
    profile_dir: &Path,
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    if !collection.enabled {
        return Ok(());
    }
    let profile_dir = profile_dir.to_path_buf();
    let collection = collection.clone();
    tokio::task::spawn_blocking(move || maintain_local_access_profile(&profile_dir, &collection))
        .await
        .map_err(|error| format!("API profile maintenance task failed: {error}"))?
}

async fn ensure_local_access_profile_takeovers(
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for profile_dir in collect_local_access_profile_takeover_dirs() {
        if let Err(err) = ensure_profile_takeover(&profile_dir, collection).await {
            failures.push(err);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn profile_model_catalog_websocket_preference_matches(
    profile_dir: &Path,
    catalog_file: Option<&str>,
    expected: bool,
) -> bool {
    let catalog_file = catalog_file
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE);
    if !is_cockpit_managed_model_catalog_name(catalog_file) {
        return false;
    }

    let catalog_path = profile_dir.join(catalog_file);
    let Ok(content) = std::fs::read_to_string(catalog_path) else {
        return false;
    };
    let Ok(catalog) = serde_json::from_str::<Value>(&content) else {
        return false;
    };
    let Some(models) = catalog.get("models").and_then(Value::as_array) else {
        return false;
    };

    !models.is_empty()
        && models
            .iter()
            .all(|model| model.get("prefer_websockets").and_then(Value::as_bool) == Some(expected))
}

fn local_access_profile_takeover_needs_sync(
    profile_dir: &Path,
    collection: &CodexLocalAccessCollection,
) -> bool {
    if !collection.enabled {
        return false;
    }

    let attachment = inspect_local_access_profile_attachment(profile_dir, Some(collection));
    if !attachment.attached {
        return false;
    }

    let expected = profile_api_key_supports_websockets(collection, &collection.api_key);
    let config_doc = read_optional_profile_file(&profile_config_path(profile_dir))
        .ok()
        .flatten()
        .and_then(|content| {
            crate::modules::codex_config_format::read_codex_config_doc_from_str(&content).ok()
        });
    let config_provider = config_doc.as_ref().and_then(|doc| {
        doc.get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID))
            .and_then(|item| item.as_table())
    });
    let config_provider_name = config_provider
        .and_then(|provider| provider.get("name"))
        .and_then(|item| item.as_str())
        .map(str::trim);
    let config_supports_websockets = config_provider
        .and_then(|provider| provider.get("supports_websockets"))
        .and_then(|item| item.as_bool());
    let catalog_file = config_doc
        .as_ref()
        .and_then(|doc| doc.get("model_catalog_json").and_then(|item| item.as_str()));

    config_provider_name != Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_NAME)
        || config_supports_websockets != Some(expected)
        || !profile_model_catalog_websocket_preference_matches(profile_dir, catalog_file, expected)
}

fn local_access_profile_takeovers_need_sync(
    collection: &CodexLocalAccessCollection,
) -> bool {
    collection.enabled
        && collect_local_access_profile_takeover_dirs()
            .iter()
            .any(|profile_dir| {
                local_access_profile_takeover_needs_sync(profile_dir, collection)
            })
}

pub(crate) async fn ensure_local_access_profile_takeovers_from_runtime() -> Result<(), String> {
    let collection = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .clone()
            .filter(|collection| collection.enabled)
    };

    if let Some(collection) = collection.as_ref() {
        ensure_local_access_profile_takeovers(collection).await?;
    }
    Ok(())
}

fn generate_local_api_key() -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    format!("agt_codex_{}", suffix)
}

fn generate_local_api_key_id() -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect();
    format!("key_{}", suffix)
}

fn default_local_api_key_label() -> String {
    "Default".to_string()
}

fn normalize_api_key_label(label: Option<&str>, fallback: &str) -> String {
    let normalized = label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .trim()
        .to_string();
    if normalized.is_empty() {
        default_local_api_key_label()
    } else {
        normalized
    }
}

fn build_local_access_api_key(label: Option<&str>) -> CodexLocalAccessApiKey {
    let now = now_ms();
    CodexLocalAccessApiKey {
        id: generate_local_api_key_id(),
        label: normalize_api_key_label(label, &default_local_api_key_label()),
        key: generate_local_api_key(),
        provider_gateway: None,
        model_routing: None,
        inherit_account_pool: Some(true),
        account_ids: Vec::new(),
        priority_account_ids: Vec::new(),
        preferred_account_id: None,
        model_prefix: None,
        allowed_models: Vec::new(),
        excluded_models: Vec::new(),
        token_limit: None,
        token_used: 0,
        enabled: true,
        created_at: now,
        updated_at: now,
        last_used_at: None,
    }
}

fn normalize_collection_api_keys(collection: &mut CodexLocalAccessCollection) -> bool {
    let mut changed = false;
    let now = now_ms();

    if collection.api_keys.is_empty() {
        let key = if collection.api_key.trim().is_empty() {
            generate_local_api_key()
        } else {
            collection.api_key.trim().to_string()
        };
        collection.api_keys.push(CodexLocalAccessApiKey {
            id: generate_local_api_key_id(),
            label: default_local_api_key_label(),
            key,
            provider_gateway: None,
            model_routing: None,
            inherit_account_pool: Some(true),
            account_ids: Vec::new(),
            priority_account_ids: Vec::new(),
            preferred_account_id: None,
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
            enabled: true,
            created_at: now,
            updated_at: now,
            last_used_at: None,
        });
        changed = true;
    }

    let mut seen_ids = HashSet::new();
    let mut seen_keys = HashSet::new();
    let mut normalized = Vec::new();
    for mut item in std::mem::take(&mut collection.api_keys) {
        let key = item.key.trim().to_string();
        if key.is_empty() || !seen_keys.insert(key.clone()) {
            changed = true;
            continue;
        }
        item.key = key;
        if item.id.trim().is_empty() || !seen_ids.insert(item.id.trim().to_string()) {
            item.id = generate_local_api_key_id();
            changed = true;
        } else {
            item.id = item.id.trim().to_string();
        }
        let normalized_label = normalize_api_key_label(Some(item.label.as_str()), "API Key");
        if normalized_label != item.label {
            item.label = normalized_label;
            changed = true;
        }
        let original_account_ids = std::mem::take(&mut item.account_ids);
        let normalized_account_ids = normalize_account_id_list(original_account_ids.clone());
        if normalized_account_ids != original_account_ids {
            item.account_ids = normalized_account_ids;
            changed = true;
        } else {
            item.account_ids = original_account_ids;
        }
        let inherit_account_pool = api_key_inherits_account_pool(&item);
        if item.inherit_account_pool != Some(inherit_account_pool) {
            item.inherit_account_pool = Some(inherit_account_pool);
            changed = true;
        }
        let mut priority_account_ids = normalize_account_id_list(item.priority_account_ids.clone());
        if priority_account_ids.is_empty() {
            if let Some(preferred_account_id) =
                normalize_optional_account_ref(item.preferred_account_id.as_deref())
            {
                priority_account_ids.push(preferred_account_id);
            }
        }
        if inherit_account_pool || api_key_has_fixed_account_scope(collection, &item) {
            priority_account_ids.clear();
        } else {
            priority_account_ids.retain(|account_id| {
                item.account_ids
                    .iter()
                    .any(|selected_account_id| selected_account_id == account_id)
            });
        }
        if item.priority_account_ids != priority_account_ids {
            item.priority_account_ids = priority_account_ids;
            changed = true;
        }
        if item.preferred_account_id.take().is_some() {
            changed = true;
        }
        if item.created_at <= 0 {
            item.created_at = now;
            changed = true;
        }
        if item.updated_at <= 0 {
            item.updated_at = now;
            changed = true;
        }
        let normalized_model_prefix = normalize_model_prefix_value(item.model_prefix.clone());
        if normalized_model_prefix != item.model_prefix {
            item.model_prefix = normalized_model_prefix;
            changed = true;
        }
        let original_allowed_models = std::mem::take(&mut item.allowed_models);
        let normalized_allowed_models = normalize_model_rule_list(original_allowed_models.clone());
        if normalized_allowed_models != original_allowed_models {
            item.allowed_models = normalized_allowed_models;
            changed = true;
        } else {
            item.allowed_models = original_allowed_models;
        }
        let original_excluded_models = std::mem::take(&mut item.excluded_models);
        let normalized_excluded_models =
            normalize_model_rule_list(original_excluded_models.clone());
        if normalized_excluded_models != original_excluded_models {
            item.excluded_models = normalized_excluded_models;
            changed = true;
        } else {
            item.excluded_models = original_excluded_models;
        }
        if item.token_limit == Some(0) {
            item.token_limit = None;
            changed = true;
        }
        normalized.push(item);
    }

    if normalized.is_empty() {
        normalized.push(build_local_access_api_key(Some(
            &default_local_api_key_label(),
        )));
        changed = true;
    }

    let primary_key = normalized
        .iter()
        .find(|item| item.enabled)
        .or_else(|| normalized.first())
        .map(|item| item.key.clone())
        .unwrap_or_else(generate_local_api_key);
    if collection.api_key != primary_key {
        collection.api_key = primary_key;
        changed = true;
    }

    collection.api_keys = normalized;
    changed
}

fn resolve_collection_api_key(
    collection: &CodexLocalAccessCollection,
    api_key: &str,
) -> Option<ResolvedLocalApiKey> {
    let normalized = api_key.trim();
    if normalized.is_empty() {
        return None;
    }
    collection
        .api_keys
        .iter()
        .find(|item| item.enabled && item.key == normalized)
        .map(|item| ResolvedLocalApiKey {
            id: item.id.clone(),
            label: item.label.clone(),
            provider_gateway: item.provider_gateway.clone(),
            inherit_account_pool: api_key_inherits_account_pool(item),
            account_ids: item.account_ids.clone(),
            model_prefix: item.model_prefix.clone(),
            allowed_models: item.allowed_models.clone(),
            excluded_models: item.excluded_models.clone(),
            token_limit: item.token_limit,
            token_used: item.token_used,
        })
        .or_else(|| {
            if collection.api_key == normalized {
                Some(ResolvedLocalApiKey {
                    id: "legacy".to_string(),
                    label: default_local_api_key_label(),
                    provider_gateway: None,
                    inherit_account_pool: true,
                    account_ids: Vec::new(),
                    model_prefix: None,
                    allowed_models: Vec::new(),
                    excluded_models: Vec::new(),
                    token_limit: None,
                    token_used: 0,
                })
            } else {
                None
            }
        })
}

fn scoped_collection_account_ids(
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
) -> Vec<String> {
    if api_key.inherit_account_pool {
        collection.account_ids.clone()
    } else {
        api_key.account_ids.clone()
    }
}

fn api_key_priority_account_ids(
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
) -> Vec<String> {
    if api_key.inherit_account_pool {
        return Vec::new();
    }
    let Some(stored_api_key) = collection
        .api_keys
        .iter()
        .find(|item| item.id == api_key.id)
    else {
        return Vec::new();
    };
    if api_key_has_fixed_account_scope(collection, stored_api_key) {
        return Vec::new();
    }
    normalize_account_id_list(stored_api_key.priority_account_ids.clone())
        .into_iter()
        .filter(|priority_account_id| {
            api_key
                .account_ids
                .iter()
                .any(|account_id| account_id == priority_account_id)
        })
        .collect()
}
