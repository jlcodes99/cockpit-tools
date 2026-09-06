// Codex 本地 API 服务命令实现。
//
// 本片段由 `commands/codex.rs` 通过 `include!` 纳入 `commands::codex` 模块，负责本地网关、
// API Key、账号池、路由策略及服务启停命令。对外调用仍使用
// `commands::codex::<command>`，确保 Tauri command 和内部 Rust 调用路径保持兼容。

#[tauri::command]
pub async fn codex_local_access_get_state() -> Result<CodexLocalAccessState, String> {
    codex_local_access::get_local_access_state().await
}

#[tauri::command]
pub async fn codex_local_access_save_accounts(
    account_ids: Vec<String>,
    restrict_free_accounts: Option<bool>,
    backup_account_ids: Option<Vec<String>>,
    preferred_account_ids: Option<Vec<String>>,
    session_affinity: Option<bool>,
    session_affinity_ttl_ms: Option<i64>,
    image_generation_account_policies: Option<
        HashMap<String, CodexLocalAccessImageGenerationPolicy>,
    >,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::save_local_access_accounts(
        account_ids,
        restrict_free_accounts.unwrap_or(true),
        backup_account_ids,
        preferred_account_ids,
        session_affinity,
        session_affinity_ttl_ms,
        image_generation_account_policies,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_append_accounts(
    account_ids: Vec<String>,
) -> Result<CodexLocalAccessAppendAccountsResult, String> {
    codex_local_access::append_local_access_accounts(account_ids).await
}

#[tauri::command]
pub async fn codex_local_access_remove_account(
    account_id: String,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::remove_local_access_account(&account_id).await
}

#[tauri::command]
pub async fn codex_local_access_recover_accounts(
    account_ids: Vec<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::recover_local_access_accounts(account_ids).await
}

#[tauri::command]
pub async fn codex_local_access_rotate_api_key() -> Result<CodexLocalAccessState, String> {
    codex_local_access::rotate_local_access_api_key().await
}

#[tauri::command]
pub async fn codex_local_access_update_bound_oauth_account(
    bound_oauth_account_id: Option<String>,
    bound_oauth_quota_reserve: Option<CodexLocalAccessQuotaReserve>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_bound_oauth_account(
        bound_oauth_account_id,
        bound_oauth_quota_reserve,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_clear_stats() -> Result<CodexLocalAccessState, String> {
    codex_local_access::clear_local_access_stats().await
}

#[tauri::command]
pub async fn codex_local_access_query_request_logs(
    page: u32,
    page_size: u32,
    stats_range: Option<String>,
    start_at: Option<i64>,
    end_at: Option<i64>,
    model_query: Option<String>,
    account_query: Option<String>,
    api_key_query: Option<String>,
    instance_query: Option<String>,
    gateway_mode: Option<CodexLocalAccessGatewayMode>,
    request_kind: Option<CodexLocalAccessRequestKind>,
    success: Option<bool>,
    error_category: Option<String>,
) -> Result<CodexLocalAccessUsageEventPage, String> {
    codex_local_access::query_local_access_usage_events(
        page,
        page_size,
        stats_range,
        start_at,
        end_at,
        model_query,
        account_query,
        api_key_query,
        instance_query,
        gateway_mode,
        request_kind,
        success,
        error_category,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_query_stats(
    start_at: i64,
    end_at: i64,
) -> Result<crate::models::codex_local_access::CodexLocalAccessStatsWindow, String> {
    codex_local_access::query_local_access_stats_window(start_at, end_at).await
}

#[tauri::command]
pub async fn codex_local_access_query_account_window_stats(
    queries: Vec<CodexLocalAccessAccountWindowQuery>,
) -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> {
    codex_local_access::query_local_access_account_window_stats(queries).await
}

#[tauri::command]
pub async fn codex_local_access_prepare_restart() -> Result<CodexLocalAccessState, String> {
    codex_local_access::prepare_local_access_gateway_for_restart().await
}

#[tauri::command]
pub async fn codex_local_access_restart_sidecar() -> Result<CodexLocalAccessState, String> {
    codex_local_access::restart_local_access_sidecar().await
}

#[tauri::command]
pub async fn codex_local_access_kill_port() -> Result<CodexLocalAccessPortCleanupResult, String> {
    codex_local_access::kill_local_access_port_processes().await
}

#[tauri::command]
pub async fn codex_local_access_update_port(port: u16) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_port(port).await
}

#[tauri::command]
pub async fn codex_local_access_update_routing_strategy(
    strategy: CodexLocalAccessRoutingStrategy,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_routing_strategy(strategy).await
}

#[tauri::command]
pub async fn codex_local_access_update_custom_routing(
    rules: Vec<CodexLocalAccessCustomRoutingRule>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_custom_routing(rules).await
}

#[tauri::command]
pub async fn codex_local_access_update_account_model_rules(
    rules: Vec<CodexLocalAccessAccountModelRule>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_account_model_rules(rules).await
}

#[tauri::command]
pub async fn codex_local_access_update_model_rules(
    model_aliases: Vec<CodexLocalAccessModelAlias>,
    excluded_models: Vec<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_model_rules(model_aliases, excluded_models).await
}

#[tauri::command]
pub async fn codex_local_access_update_model_pricings(
    app: AppHandle,
    model_pricings: Vec<CodexLocalAccessModelPricing>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_model_pricings(app, model_pricings).await
}

#[tauri::command]
pub async fn codex_local_access_reprice_request_logs() -> Result<CodexLocalAccessState, String> {
    codex_local_access::reprice_local_access_request_logs().await
}

#[tauri::command]
pub async fn codex_local_access_update_routing_options(
    session_affinity: bool,
    session_affinity_ttl_ms: i64,
    responses_websockets_enabled: bool,
    max_retry_credentials: u16,
    max_retry_interval_ms: u64,
    disable_cooling: bool,
    immediate_sse_response: bool,
    max_concurrent_image_requests: u16,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_routing_options(
        session_affinity,
        session_affinity_ttl_ms,
        responses_websockets_enabled,
        max_retry_credentials,
        max_retry_interval_ms,
        disable_cooling,
        immediate_sse_response,
        max_concurrent_image_requests,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_update_timeouts(
    timeouts: CodexLocalAccessTimeouts,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_timeouts(timeouts, active_timeout_preset_id).await
}

#[tauri::command]
pub async fn codex_local_access_update_timeout_presets(
    timeout_presets: Vec<CodexLocalAccessTimeoutPreset>,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_timeout_presets(
        timeout_presets,
        active_timeout_preset_id,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_update_upstream_proxy_config(
    upstream_proxy_url: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_upstream_proxy_config(upstream_proxy_url).await
}

#[tauri::command]
pub async fn codex_local_access_update_gateway_mode(
    gateway_mode: CodexLocalAccessGatewayMode,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_gateway_mode(gateway_mode).await
}

#[tauri::command]
pub async fn codex_local_access_update_debug_logs(
    debug_logs: bool,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_debug_logs(debug_logs).await
}

#[tauri::command]
pub async fn codex_local_access_update_access_scope(
    access_scope: CodexLocalAccessScope,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_scope(access_scope).await
}

#[tauri::command]
pub async fn codex_local_access_update_client_base_url_host(
    client_base_url_host: CodexLocalAccessClientBaseUrlHost,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_client_base_url_host(client_base_url_host).await
}

#[tauri::command]
pub async fn codex_local_access_create_api_key(
    label: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::create_local_access_api_key(label).await
}

#[tauri::command]
pub async fn codex_local_access_update_api_key(
    api_key_id: String,
    label: Option<String>,
    enabled: Option<bool>,
    model_prefix: Option<String>,
    allowed_models: Option<Vec<String>>,
    excluded_models: Option<Vec<String>>,
    token_limit: Option<u64>,
    account_ids: Option<Vec<String>>,
    inherit_account_pool: Option<bool>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_api_key(
        api_key_id,
        label,
        enabled,
        model_prefix,
        allowed_models,
        excluded_models,
        token_limit,
        account_ids,
        inherit_account_pool,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_set_api_key_account_priority(
    api_key_id: String,
    account_id: String,
    pinned: bool,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::set_local_access_api_key_account_priority(api_key_id, account_id, pinned)
        .await
}

#[tauri::command]
pub async fn codex_local_access_rotate_named_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::rotate_local_access_named_api_key(api_key_id).await
}

#[tauri::command]
pub async fn codex_local_access_delete_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::delete_local_access_api_key(api_key_id).await
}

#[tauri::command]
pub async fn codex_local_access_set_enabled(
    enabled: bool,
) -> Result<CodexLocalAccessState, String> {
    let codex_home = codex_account::get_codex_home();
    let _profile_lease = codex_account::try_acquire_profile_mutation_lease(
        &codex_home,
        if enabled {
            "api-service-enable"
        } else {
            "api-service-disable"
        },
    )?;
    if enabled {
        stop_default_codex_runtime_before_auth_commit().await?;
    }
    codex_local_access::set_local_access_enabled(enabled).await
}

#[tauri::command]
pub async fn codex_local_access_activate(
    app: AppHandle,
    auto_repair_mode: Option<codex_session_visibility::CodexSessionVisibilityAutoRepairMode>,
    instance_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    let flow_started = Instant::now();
    let target_instance_id = instance_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(crate::commands::codex_instance::DEFAULT_INSTANCE_ID)
        .to_string();
    let launch_target =
        crate::commands::codex_instance::resolve_codex_instance_start_target(&target_instance_id)?;
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] codex_local_access_activate started: instance_id={}, user_data_dir={}",
        target_instance_id,
        launch_target.user_data_dir.display()
    ));
    let codex_home = launch_target.user_data_dir.clone();
    let _profile_lease =
        codex_account::try_acquire_profile_mutation_lease(&codex_home, "api-service-activate")?;
    // 先停止目标 profile 的官方客户端，再写入 API Service 凭据。
    if launch_target.is_default {
        stop_default_codex_runtime_before_auth_commit().await?;
    } else {
        crate::commands::codex_instance::codex_stop_instance(target_instance_id.clone()).await?;
    }
    let previous_credential = read_codex_launch_credential_snapshot_for_dir(
        &codex_home,
        launch_target.bind_account_id.as_deref(),
        launch_target.is_default,
    );
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] previous credential resolved: elapsed_ms={}",
        flow_started.elapsed().as_millis()
    ));
    let activate_started = Instant::now();
    let state = codex_local_access::activate_local_access_for_dir(&codex_home).await?;
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] activate_local_access_for_dir finished: elapsed_ms={}, total_ms={}",
        activate_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let api_service_speed = codex_speed::get_api_service_app_speed_config()?.speed;
    let speed_started = Instant::now();
    if launch_target.is_default {
        codex_speed::write_official_app_speed(api_service_speed.clone())?;
    } else {
        codex_speed::write_app_speed_for_dir(&codex_home, api_service_speed.clone())?;
    }
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] write target profile app speed finished: elapsed_ms={}, total_ms={}",
        speed_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let index_started = Instant::now();
    if launch_target.is_default {
        let mut index = codex_account::load_account_index();
        index.current_account_id = None;
        codex_account::save_account_index(&index)?;
    } else {
        logger::log_info(&format!(
            "已保留全局当前 Codex 账号索引，非默认实例激活不影响默认实例: instance_id={}",
            target_instance_id
        ));
    }
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] account index stage finished: cleared={}, elapsed_ms={}, total_ms={}",
        launch_target.is_default,
        index_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let default_settings_started = Instant::now();
    if launch_target.is_default {
        if let Err(e) = crate::modules::codex_instance::update_default_settings(
            Some(Some(
                crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string(),
            )),
            None,
            None,
            Some(false),
            None,
            None,
        ) {
            logger::log_warn(&format!("更新 Codex 默认实例为 API 服务模式失败: {}", e));
        } else {
            logger::log_info("已同步更新 Codex 默认实例为 API 服务模式");
        }
        if let Err(e) =
            crate::modules::codex_instance::update_default_app_speed(api_service_speed.clone())
        {
            logger::log_warn(&format!("更新 Codex 默认实例 API 服务速度失败: {}", e));
        }
    } else {
        logger::log_info(&format!(
            "已保留非默认实例绑定，不修改 Codex 默认实例: instance_id={}",
            target_instance_id
        ));
    }
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] default settings update finished: elapsed_ms={}, total_ms={}",
        default_settings_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let repair_started = Instant::now();
    repair_codex_session_visibility_after_credential_kind_change(
        "after-api-service-activate",
        previous_credential,
        Some(CodexLaunchCredentialSnapshot {
            kind: "api".to_string(),
            source: format!(
                "target-bind:{}",
                crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID
            ),
        }),
        auto_repair_mode,
    );
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] session visibility repair stage finished: elapsed_ms={}, total_ms={}",
        repair_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let user_config = config::get_user_config();

    logger::log_info("API 服务启动模式下跳过 OpenCode / OpenClaw OAuth 同步");

    if user_config.codex_launch_on_switch {
        let launch_started = Instant::now();
        #[cfg(target_os = "macos")]
        if launch_target.is_default && process::is_codex_running() {
            logger::log_info("检测到 Codex 正在运行，将按默认实例 PID 逻辑重启");
        }
        // 默认与非默认目标均继续进入 `codex_start_instance_internal`，使用与账号总览、
        // 多开实例一致的 profile 准备和客户端启动事务。
        let launch_error = match if launch_target.is_default {
            crate::commands::codex_instance::codex_start_default_with_prepared_profile(
                app.clone(),
                true,
                Some("instance-launch"),
                None,
            )
            .await
        } else {
            crate::commands::codex_instance::codex_start_instance_with_prepared_profile(
                app.clone(),
                target_instance_id.clone(),
                true,
                Some("instance-launch"),
                None,
            )
            .await
        } {
            Ok(_) => None,
            Err(e) => {
                logger::log_warn(&format!("Codex 启动失败: {}", e));
                if e.starts_with("APP_PATH_NOT_FOUND:") {
                    let retry = if launch_target.is_default {
                        serde_json::json!({ "kind": "default" })
                    } else {
                        serde_json::json!({
                            "kind": "instance",
                            "instanceId": target_instance_id,
                        })
                    };
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({ "app": "codex", "retry": retry }),
                    );
                }
                Some(e)
            }
        };
        logger::log_info(&format!(
            "[Codex API Service Switch][Backend] selected instance launch finished: instance_id={}, elapsed_ms={}, total_ms={}",
            target_instance_id,
            launch_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        if let Some(error) = launch_error {
            if error.starts_with("CODEX_SWITCH_AUTH_REQUIRED:") {
                return Err(error);
            }
            return Err(format!(
                "Codex API Service 已激活，但客户端启动失败: {}",
                error
            ));
        }
    } else {
        logger::log_info("已关闭切换 Codex 时自动启动 Codex App");
    }

    let tray_started = Instant::now();
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] codex_local_access_activate finished: tray_elapsed_ms={}, total_ms={}",
        tray_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    Ok(state)
}

#[tauri::command]
pub async fn codex_local_access_test() -> Result<CodexLocalAccessTestResult, String> {
    codex_local_access::test_local_access_with_dialog().await
}

#[tauri::command]
pub async fn codex_local_access_chat_test(
    model_id: String,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<CodexLocalAccessChatResult, String> {
    codex_local_access::chat_local_access_with_dialog(model_id, messages).await
}

#[tauri::command]
pub async fn codex_local_access_chat_test_stream(
    app: AppHandle,
    session_id: String,
    model_id: String,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<(), String> {
    codex_local_access::stream_chat_local_access_with_dialog(app, session_id, model_id, messages)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_usage_summary_is_persisted_for_quota_pool_snapshots() {
        let mut account = CodexAccount::new_api_key(
            "api-key-usage".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://provider.example".to_string()),
            Some("custom-provider".to_string()),
            Some("Custom Provider".to_string()),
            Vec::new(),
        );
        let summary = serde_json::json!({
            "mode": "sub2api",
            "remaining": 5.6,
            "unit": "USD"
        });

        assert!(apply_codex_api_key_usage_summary(
            &mut account,
            summary.clone(),
            Some(1_787_406_691_000),
        )
        .expect("persist provider usage"));
        assert_eq!(account.usage_updated_at, Some(1_787_406_691));
        assert_eq!(
            account
                .quota
                .as_ref()
                .and_then(|quota| quota.raw_data.as_ref())
                .and_then(|raw| raw.get("provider_usage")),
            Some(&summary)
        );

        assert!(!apply_codex_api_key_usage_summary(
            &mut account,
            serde_json::json!({
                "mode": "sub2api",
                "remaining": 1.2,
                "unit": "USD"
            }),
            Some(1_787_406_000_000),
        )
        .expect("ignore stale provider usage"));
        assert_eq!(
            account
                .quota
                .as_ref()
                .and_then(|quota| quota.raw_data.as_ref())
                .and_then(|raw| raw.get("provider_usage")),
            Some(&summary)
        );
    }

    #[test]
    fn provider_usage_root_url_also_tries_v1() {
        assert_eq!(
            codex_provider_usage_base_url_candidates("https://aihub.top/")
                .expect("valid provider URL"),
            vec![
                "https://aihub.top".to_string(),
                "https://aihub.top/v1".to_string(),
            ]
        );
    }

    #[test]
    fn provider_usage_v1_url_is_not_modified() {
        assert_eq!(
            codex_provider_usage_base_url_candidates("https://aihub.top/v1")
                .expect("valid provider URL"),
            vec!["https://aihub.top/v1".to_string()]
        );
    }

    #[tokio::test]
    async fn cockpit_tools_usage_exposes_api_key_balance_in_cny() {
        let server = tiny_http::Server::http("127.0.0.1:0")
            .expect("mock Cockpit Tools provider should start");
        let port = server
            .server_addr()
            .to_ip()
            .expect("mock provider should use an IP address")
            .port();
        let responder = std::thread::spawn(move || {
            let request = server.recv().expect("mock provider should receive quota query");
            assert_eq!(request.url(), "/v1/cockpit/quota");
            request
                .respond(tiny_http::Response::from_string(
                    r#"{"version":1,"scope":"api_key_account_pool","apiKeyBalance":5.6,"accountCount":1,"availableAccountCount":1,"stale":false}"#,
                ))
                .expect("mock provider should return quota response");
        });

        let summary = codex_query_model_provider_usage(
            format!("http://127.0.0.1:{port}/v1"),
            "test-api-key".to_string(),
            Some("cockpit_tools".to_string()),
        )
        .await
        .expect("Cockpit Tools quota should be summarized");

        responder.join().expect("mock provider should finish");
        assert_eq!(summary.mode.as_deref(), Some("cockpit_tools"));
        assert_eq!(summary.balance, Some(5.6));
        assert_eq!(summary.unit.as_deref(), Some("CNY"));
    }

    #[tokio::test]
    async fn account_pool_cleanup_error_does_not_block_local_delete_flow() {
        run_account_pool_cleanup_best_effort("test_error", 1, Duration::from_secs(1), async {
            Err("gateway reload failed".to_string())
        })
        .await;
    }

    #[tokio::test]
    async fn account_pool_cleanup_timeout_does_not_block_local_delete_flow() {
        run_account_pool_cleanup_best_effort(
            "test_timeout",
            1,
            Duration::from_millis(1),
            std::future::pending(),
        )
        .await;
    }

    #[test]
    fn batch_delete_jobs_dir_reuses_existing_directory() {
        let root = std::env::temp_dir().join(format!(
            "codex-batch-delete-dir-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let jobs_dir = root.join(CODEX_BATCH_DELETE_JOBS_DIR);
        fs::create_dir_all(&jobs_dir).expect("create jobs dir");

        ensure_codex_batch_delete_jobs_dir(&jobs_dir).expect("reuse existing jobs dir");
        assert!(jobs_dir.is_dir());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn batch_delete_jobs_dir_rejects_existing_file() {
        let path = std::env::temp_dir().join(format!(
            "codex-batch-delete-file-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        fs::write(&path, b"not a directory").expect("create conflicting file");

        let error = ensure_codex_batch_delete_jobs_dir(&path).expect_err("file must fail");
        assert!(error.contains("不是目录"));

        let _ = fs::remove_file(path);
    }

    fn models(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn responses_native_chat_test_prefers_gpt_55_over_image_model() {
        let catalog = models(&["gpt-image-2", "gpt-5.5", "gpt-5.4"]);

        assert_eq!(
            select_model_provider_chat_test_model("responses", None, &catalog).as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn responses_native_chat_test_skips_image_model_when_preferred_missing() {
        let catalog = models(&["gpt-image-2", "custom-text-model"]);

        assert_eq!(
            select_model_provider_chat_test_model("responses", None, &catalog).as_deref(),
            Some("custom-text-model")
        );
    }

    #[test]
    fn chat_completions_chat_test_keeps_catalog_order() {
        let catalog = models(&["provider-default", "gpt-5.5"]);

        assert_eq!(
            select_model_provider_chat_test_model("chat_completions", None, &catalog).as_deref(),
            Some("provider-default")
        );
    }

    #[test]
    fn explicit_chat_test_model_wins_over_responses_preference() {
        let catalog = models(&["gpt-image-2", "gpt-5.5"]);

        assert_eq!(
            select_model_provider_chat_test_model("responses", Some("custom-model"), &catalog)
                .as_deref(),
            Some("custom-model")
        );
    }

    #[test]
    fn deepseek_defaults_to_native_responses_when_unspecified() {
        assert_eq!(
            normalize_model_provider_wire_api(None, "https://api.deepseek.com/v1"),
            "responses"
        );
        assert_eq!(
            normalize_model_provider_wire_api(
                Some("chat_completions"),
                "https://api.deepseek.com/v1",
            ),
            "chat_completions"
        );
        assert_eq!(
            normalize_model_provider_wire_api(Some("responses"), "https://api.deepseek.com/v1"),
            "responses"
        );
    }

    #[test]
    fn deepseek_balance_url_ignores_optional_v1_path() {
        assert_eq!(
            codex_model_provider_deepseek_balance_url("https://api.deepseek.com/v1")
                .expect("valid URL")
                .as_deref(),
            Some("https://api.deepseek.com/user/balance")
        );
        assert_eq!(
            codex_model_provider_deepseek_balance_url("https://example.com/v1").expect("valid URL"),
            None
        );
    }

    #[test]
    fn deepseek_balance_prefers_cny_and_parses_string_amounts() {
        let summary = summarize_deepseek_balance(
            &serde_json::json!({
                "is_available": true,
                "balance_infos": [
                    {
                        "currency": "USD",
                        "total_balance": "9.00",
                        "granted_balance": "1.00",
                        "topped_up_balance": "8.00"
                    },
                    {
                        "currency": "CNY",
                        "total_balance": "110.00",
                        "granted_balance": "10.00",
                        "topped_up_balance": "100.00"
                    }
                ]
            }),
            12,
        );

        assert_eq!(summary.mode.as_deref(), Some("deepseek"));
        assert_eq!(summary.unit.as_deref(), Some("CNY"));
        assert_eq!(summary.balance, Some(110.0));
        assert_eq!(summary.is_valid, Some(true));
        assert!(summary
            .details
            .iter()
            .any(|detail| detail.key == "grantedBalance" && detail.value == "10"));
    }

    #[test]
    fn token_plan_provider_detection_uses_known_hosts() {
        assert_eq!(
            codex_model_provider_token_plan_provider("https://api.minimaxi.com/v1")
                .expect("valid URL"),
            Some(CodexTokenPlanProvider::MiniMax)
        );
        assert_eq!(
            codex_model_provider_token_plan_provider("https://open.bigmodel.cn/api/coding/paas/v4")
                .expect("valid URL"),
            Some(CodexTokenPlanProvider::Zhipu)
        );
        assert_eq!(
            codex_model_provider_token_plan_provider("https://example.com/v1").expect("valid URL"),
            None
        );
    }

    #[test]
    fn token_plan_urls_ignore_provider_version_path() {
        assert_eq!(
            codex_model_provider_token_plan_urls(
                "https://api.minimaxi.com/v1",
                CodexTokenPlanProvider::MiniMax,
            )
            .expect("valid URL"),
            vec![
                "https://api.minimaxi.com/v1/token_plan/remains",
                "https://api.minimaxi.com/v1/api/openplatform/coding_plan/remains",
            ]
        );
        assert_eq!(
            codex_model_provider_token_plan_urls(
                "https://api.z.ai/api/coding/paas/v4",
                CodexTokenPlanProvider::Zhipu,
            )
            .expect("valid URL"),
            vec!["https://api.z.ai/api/monitor/usage/quota/limit"]
        );
    }

    #[test]
    fn minimax_token_plan_prefers_remaining_percent_for_time_windows() {
        let summary = summarize_minimax_token_plan_usage(
            &serde_json::json!({
                "model_remains": [{
                    "model_name": "MiniMax-M2.7",
                    "current_interval_total_count": 0,
                    "current_interval_usage_count": 0,
                    "current_interval_remaining_percent": 72,
                    "current_weekly_remaining_percent": 61,
                    "end_time": 1773914400000i64,
                    "weekly_end_time": 1774224000000i64
                }]
            }),
            15,
        )
        .expect("token plan response");

        assert_eq!(summary.mode.as_deref(), Some("token_plan"));
        assert_eq!(summary.remaining, Some(72.0));
        assert_eq!(summary.quota_used, Some(28.0));
        assert_eq!(summary.quota_limit, Some(100.0));
        assert!(summary
            .details
            .iter()
            .any(|detail| detail.key == "intervalRemainingPercent" && detail.value == "72"));
        assert!(summary
            .details
            .iter()
            .any(|detail| detail.key == "weeklyExpiresAt" && detail.value == "1774224000"));
    }

    #[test]
    fn zhipu_token_plan_uses_raw_authorization_shape_and_next_reset() {
        let summary = summarize_zhipu_token_plan_usage(
            &serde_json::json!({
                "code": 200,
                "success": true,
                "data": {
                    "level": "pro",
                    "limits": [
                        {
                            "type": "TOKENS_LIMIT",
                            "usage": 800000000,
                            "currentValue": 127694464,
                            "remaining": 672305536,
                            "percentage": 15,
                            "nextResetTime": 1770648402389i64
                        },
                        {
                            "type": "TIME_LIMIT",
                            "percentage": 30
                        }
                    ]
                }
            }),
            21,
        )
        .expect("token plan response");

        assert_eq!(summary.mode.as_deref(), Some("token_plan"));
        assert_eq!(summary.plan_name.as_deref(), Some("pro"));
        assert_eq!(summary.remaining, Some(85.0));
        assert_eq!(summary.unit.as_deref(), Some("%"));
        assert_eq!(
            summary
                .details
                .iter()
                .find(|detail| detail.key == "expiresAt")
                .map(|detail| detail.value.as_str()),
            Some("1770648402")
        );
        assert_eq!(summary.model_stats_count, 1);
    }
}
