// Codex Local Access 测试：Sidecar scheduler, account pool and provider gateway behavior。
// 测试与生产实现共享 super 作用域，验证真实网关、持久化和请求协议行为。
    const CODEX_TEST_MODEL_ID: &str = "custom-model";

    #[test]
    fn sidecar_scheduler_state_updates_runtime_cooldown_and_health() {
        let now = 1_000_000_i64;
        let mut runtime = super::GatewayRuntime::default();
        let event = super::SidecarAuthResultEvent {
            account_id: "account-1".to_string(),
            account_email: "user@example.com".to_string(),
            request_kind: "text".to_string(),
            success: false,
            http_status: Some(401),
            error_code: Some("auth_unavailable".to_string()),
            error_message: Some("invalid or expired token".to_string()),
            model: "gpt-5.5".to_string(),
            auth_available: Some(false),
            next_retry_at_ms: Some(now + 30 * 60 * 1000),
            auth_state_reason: Some("unauthorized".to_string()),
            ..Default::default()
        };

        super::apply_sidecar_scheduler_state(&mut runtime, &event, now);

        let health = runtime
            .account_health
            .get("account-1")
            .expect("account health should be created");
        assert_eq!(health.sidecar_scheduler_available, Some(false));
        assert_eq!(
            health.sidecar_scheduler_reason.as_deref(),
            Some("unauthorized")
        );
        assert!(super::sidecar_scheduler_blocks_account(Some(health), now));
        assert!(super::account_health_blocks_dispatch(Some(health), now));
        assert_eq!(runtime.model_cooldowns.len(), 1);
    }

    #[test]
    fn deepseek_provider_model_slots_include_deterministic_vision_shell() {
        let slots = super::allocate_provider_model_slots(&[
            "deepseek-v4-flash".to_string(),
            "deepseek-v4-pro".to_string(),
            "deepseek-v4-flash-vision-exp".to_string(),
        ]);
        assert_eq!(
            slots,
            vec![
                super::ProviderGatewayModelSlot {
                    client_model: "gpt-5.5".to_string(),
                    upstream_model: "deepseek-v4-flash".to_string(),
                },
                super::ProviderGatewayModelSlot {
                    client_model: "gpt-5.4".to_string(),
                    upstream_model: "deepseek-v4-pro".to_string(),
                },
                super::ProviderGatewayModelSlot {
                    client_model: "gpt-5.4-mini".to_string(),
                    upstream_model: "deepseek-v4-flash-vision-exp".to_string(),
                },
            ]
        );
    }

    #[test]
    fn sidecar_scheduler_state_expires_without_stale_page_cooldown() {
        let now = 1_000_000_i64;
        let mut runtime = super::GatewayRuntime::default();
        let event = super::SidecarAuthResultEvent {
            account_id: "account-1".to_string(),
            account_email: String::new(),
            request_kind: String::new(),
            success: false,
            http_status: Some(503),
            error_code: Some("upstream_timeout".to_string()),
            error_message: None,
            model: "gpt-5.5".to_string(),
            auth_available: Some(false),
            next_retry_at_ms: Some(now + 1),
            auth_state_reason: Some("transient_upstream".to_string()),
            ..Default::default()
        };

        super::apply_sidecar_scheduler_state(&mut runtime, &event, now);
        let later = now + 2;
        super::prune_runtime_routing_state(&mut runtime, later);

        let health = runtime
            .account_health
            .get("account-1")
            .expect("account health should be created");
        assert!(!super::sidecar_scheduler_blocks_account(
            Some(health),
            later
        ));
        assert!(!super::account_health_blocks_dispatch(Some(health), later));
        assert!(runtime.model_cooldowns.is_empty());
    }

    #[test]
    fn accountless_auth_failure_is_recorded_as_pool_health() {
        let mut runtime = super::GatewayRuntime::default();
        let event = super::SidecarAuthResultEvent {
            api_key_id: "key-1".to_string(),
            api_key_label: "Windows test".to_string(),
            provider: "codex".to_string(),
            model: "gpt-5.5".to_string(),
            request_kind: "text".to_string(),
            error_code: Some("auth_unavailable".to_string()),
            error_message: Some("no auth available".to_string()),
            candidate_auths: 2,
            scoped_auths: 2,
            unavailable_auths: 2,
            account_statuses: vec![super::SidecarAccountStatus {
                account_id: "account-1".to_string(),
                account_email: "one@example.com".to_string(),
                available: false,
                reason_code: "auth_refresh_failed".to_string(),
                reason_message: "invalid refresh token".to_string(),
            }],
            ..Default::default()
        };

        super::apply_sidecar_account_pool_health(&mut runtime, &event, true, 1_000_000);

        assert!(runtime.account_health.is_empty());
        let health = runtime
            .account_pool_health
            .get("key-1")
            .expect("pool health should be created without an account id");
        assert!(health.diagnostic_available);
        assert_eq!(health.candidate_auths, 2);
        assert_eq!(health.unavailable_auths, 2);
        assert_eq!(health.account_statuses.len(), 1);
        assert_eq!(health.account_statuses[0].reason_code, "auth_refresh_failed");
        assert_eq!(health.last_failure_at, 1_000_000);
    }

    #[test]
    fn successful_auth_result_clears_pool_health_for_same_api_key() {
        let mut runtime = super::GatewayRuntime::default();
        runtime.account_pool_health.insert(
            "key-1".to_string(),
            super::RuntimeAccountPoolHealth::default(),
        );
        let event = super::SidecarAuthResultEvent {
            api_key_id: "key-1".to_string(),
            account_id: "account-1".to_string(),
            success: true,
            ..Default::default()
        };

        super::apply_sidecar_account_pool_health(&mut runtime, &event, false, 1_000_000);

        assert!(runtime.account_pool_health.is_empty());
    }

    #[test]
    fn manual_recovery_removes_only_selected_pool_member_status() {
        let mut runtime = super::GatewayRuntime::default();
        let mut pool = super::RuntimeAccountPoolHealth::default();
        pool.account_statuses = vec![
            super::RuntimeAccountPoolMemberHealth {
                account_id: "account-1".to_string(),
                ..Default::default()
            },
            super::RuntimeAccountPoolMemberHealth {
                account_id: "account-2".to_string(),
                ..Default::default()
            },
        ];
        runtime.account_pool_health.insert("key-1".to_string(), pool);

        super::clear_runtime_account_health(&mut runtime, &["account-1".to_string()], false);

        let remaining = &runtime.account_pool_health["key-1"].account_statuses;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].account_id, "account-2");
    }

    #[test]
    fn manual_recovery_clears_only_selected_runtime_account_health() {
        let mut runtime = super::GatewayRuntime::default();
        runtime.account_health.insert(
            "account-1".to_string(),
            super::RuntimeAccountHealth::default(),
        );
        runtime.account_health.insert(
            "account-2".to_string(),
            super::RuntimeAccountHealth::default(),
        );
        runtime.model_cooldowns.insert(
            super::build_cooldown_key("account-1", "gpt-5.5").unwrap(),
            super::AccountModelCooldown {
                model_key: "gpt-5.5".to_string(),
                next_retry_at_ms: 2_000_000,
                reason: "unauthorized".to_string(),
            },
        );
        runtime.account_pool_health.insert(
            "key-1".to_string(),
            super::RuntimeAccountPoolHealth::default(),
        );

        super::clear_runtime_account_health(&mut runtime, &["account-1".to_string()], false);

        assert!(!runtime.account_health.contains_key("account-1"));
        assert!(runtime.account_health.contains_key("account-2"));
        assert!(runtime.model_cooldowns.is_empty());
        assert!(runtime.account_pool_health.contains_key("key-1"));
    }

    #[test]
    fn full_manual_recovery_clears_aggregate_pool_health() {
        let mut runtime = super::GatewayRuntime::default();
        runtime.account_pool_health.insert(
            "key-1".to_string(),
            super::RuntimeAccountPoolHealth::default(),
        );

        super::clear_runtime_account_health(
            &mut runtime,
            &["account-1".to_string(), "account-2".to_string()],
            true,
        );

        assert!(runtime.account_pool_health.is_empty());
    }

    #[test]
    fn quota_exhaustion_blocks_dispatch_until_reset_or_recovery() {
        let mut runtime = super::GatewayRuntime::default();
        runtime.account_quota_cooldowns.insert(
            "account-1".to_string(),
            super::AccountQuotaCooldown {
                exhausted: true,
                reset_at_ms: Some(2_000_000),
                updated_at_ms: 1_000_000,
            },
        );

        assert!(super::account_recovery_blocked_by_quota(&runtime, "account-1", 1_500_000));
        assert!(!super::account_recovery_blocked_by_quota(&runtime, "account-1", 2_000_000));

        super::mark_quota_cooldowns_recovered(
            &mut runtime,
            &["account-1".to_string()],
            1_750_000,
        );
        assert!(!super::account_recovery_blocked_by_quota(&runtime, "account-1", 1_800_000));
        let recovered = runtime.account_quota_cooldowns.get("account-1").expect("recovered snapshot");
        assert!(!recovered.active(1_800_000));
        assert_eq!(recovered.updated_at_ms, 1_750_000);
    }

    #[test]
    fn removing_accounts_clears_runtime_health_and_quota_cooldown() {
        let mut runtime = super::GatewayRuntime::default();
        runtime.account_health.insert(
            "account-1".to_string(),
            super::RuntimeAccountHealth::default(),
        );
        runtime.account_quota_cooldowns.insert(
            "account-1".to_string(),
            super::AccountQuotaCooldown {
                exhausted: true,
                reset_at_ms: None,
                updated_at_ms: 1_000_000,
            },
        );
        runtime.account_quota_cooldowns.insert(
            "account-2".to_string(),
            super::AccountQuotaCooldown {
                exhausted: true,
                reset_at_ms: None,
                updated_at_ms: 1_000_000,
            },
        );

        super::clear_runtime_account_health(&mut runtime, &["account-1".to_string()], false);
        super::clear_runtime_quota_cooldowns(&mut runtime, &["account-1".to_string()]);

        assert!(!runtime.account_health.contains_key("account-1"));
        assert!(!runtime.account_quota_cooldowns.contains_key("account-1"));
        assert!(runtime.account_quota_cooldowns.contains_key("account-2"));
    }

    fn oauth_account_with_quota(
        hourly: i32,
        weekly: i32,
        raw_data: Option<serde_json::Value>,
    ) -> CodexAccount {
        let mut account = test_account_with_plan("free");
        account.quota = Some(crate::models::codex::CodexQuota {
            hourly_percentage: hourly,
            hourly_reset_time: None,
            hourly_window_minutes: None,
            hourly_window_present: Some(true),
            weekly_percentage: weekly,
            weekly_reset_time: None,
            weekly_window_minutes: None,
            weekly_window_present: Some(true),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data,
        });
        account.usage_updated_at = Some(1_700);
        account
    }

    #[test]
    fn remaining_credits_keep_zero_window_accounts_out_of_quota_cooldown() {
        let now = 1_800_000_i64;
        let credit_account = oauth_account_with_quota(
            0,
            0,
            Some(serde_json::json!({
                "credits": { "balance": "351.02", "remaining": 351.02, "unlimited": false }
            })),
        );
        let cooldown = super::account_quota_cooldown(&credit_account, now)
            .expect("credit account should produce a cooldown snapshot");
        assert!(!cooldown.exhausted);
        assert_eq!(super::resolve_remaining_quota(&credit_account), Some(1));

        let spend_control_account = oauth_account_with_quota(
            0,
            0,
            Some(serde_json::json!({
                "spend_control": {
                    "individual_limit": { "limit": "400", "used": "48.98", "remaining": "351.02" }
                }
            })),
        );
        let spend_cooldown = super::account_quota_cooldown(&spend_control_account, now)
            .expect("spend-control credits should keep the account schedulable");
        assert!(!spend_cooldown.exhausted);

        let exhausted_account = oauth_account_with_quota(0, 0, None);
        let exhausted = super::account_quota_cooldown(&exhausted_account, now)
            .expect("zero windows without credits should be exhausted");
        assert!(exhausted.exhausted);
        assert_eq!(super::resolve_remaining_quota(&exhausted_account), Some(0));
    }

    #[test]
    fn catalog_context_windows_keep_official_and_override_third_party() {
        let official = super::ProviderGatewayModelSlot {
            client_model: "gpt-5.4".to_string(),
            upstream_model: "gpt-5.4".to_string(),
        };
        let remapped = super::ProviderGatewayModelSlot {
            client_model: "gpt-5.6-sol".to_string(),
            upstream_model: "custom-flash".to_string(),
        };
        let deepseek = super::ProviderGatewayModelSlot {
            client_model: "gpt-5.5".to_string(),
            upstream_model: "deepseek-v4-flash".to_string(),
        };
        let catalog = serde_json::json!({
            "models": [
                { "slug": "gpt-5.4", "context_window": 272000, "max_context_window": 272000 },
                { "slug": "gpt-5.6-sol", "context_window": 372000, "max_context_window": 372000 },
                { "slug": "gpt-5.5", "context_window": 1048576, "max_context_window": 1048576 }
            ]
        })
        .to_string();
        let mut explicit = std::collections::HashMap::new();
        explicit.insert("custom-flash".to_string(), 900_000);
        let decorated = super::decorate_catalog_context_windows(
            &catalog,
            &[official, remapped, deepseek],
            &explicit,
            Some(516_000),
        )
        .expect("decorate catalog");
        let parsed: serde_json::Value = serde_json::from_str(&decorated).expect("parse decorated");
        let window = |slug: &str| {
            parsed["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model["slug"] == slug)
                .and_then(|model| model["context_window"].as_i64())
        };
        assert_eq!(window("gpt-5.4"), Some(272000));
        assert_eq!(window("gpt-5.6-sol"), Some(900_000));
        assert_eq!(window("gpt-5.5"), Some(1048576));
    }

    #[test]
    fn api_service_catalog_applies_explicit_windows_and_keeps_official_defaults() {
        let catalog = super::build_codex_client_models_response(&[
            "gpt-5.4".to_string(),
            "gpt-5.6-sol".to_string(),
        ]);
        let official_window = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == "gpt-5.4")
            .and_then(|model| model["context_window"].as_i64());
        let mut windows = std::collections::HashMap::new();
        windows.insert("gpt-5.6-sol".to_string(), 900_000);
        let decorated = super::apply_explicit_context_windows_to_client_models(catalog, &windows);
        let window = |slug: &str| {
            decorated["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model["slug"] == slug)
                .and_then(|model| model["context_window"].as_i64())
        };
        assert_eq!(window("gpt-5.4"), official_window);
        assert_eq!(window("gpt-5.6-sol"), Some(900_000));
    }

    #[test]
    fn sidecar_account_manifest_includes_model_context_windows() {
        let mut account = crate::models::codex::CodexAccount::new_api_key(
            "deepseek-window-1".to_string(),
            "deepseek@example.com".to_string(),
            "sk-test".to_string(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-flash".to_string()],
        );
        account
            .api_model_context_windows
            .insert("gpt-5.6-sol".to_string(), 900_000);
        let collection = test_local_access_collection(vec![account.id.clone()]);
        let manifest =
            super::sidecar_account_manifest_value(&account, Some("auth.json"), &collection);
        assert_eq!(manifest["modelContextWindows"]["gpt-5.6-sol"], 900_000);
    }

    #[test]
    fn sidecar_localized_messages_include_chinese_and_english() {
        let value =
            super::sidecar_localized_messages("codex.localAccess.gatewayErrors.authUnavailable");
        let object = value
            .as_object()
            .expect("localized messages should be an object");
        assert!(object
            .get("zh-cn")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("账号池")));
        assert!(object
            .get("en")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("account")));
    }

    #[test]
    fn calendar_stats_windows_start_at_local_day_week_and_month() {
        use chrono::{Datelike, TimeZone, Timelike};

        let now = chrono::Local
            .with_ymd_and_hms(2026, 7, 15, 12, 30, 0)
            .single()
            .expect("test local time should exist");
        let (day_start, week_start, month_start) =
            super::local_calendar_window_starts(now.timestamp_millis());
        let local_date_time = |timestamp| {
            chrono::Local
                .timestamp_millis_opt(timestamp)
                .single()
                .expect("calendar boundary should be a local instant")
        };
        let day = local_date_time(day_start);
        let week = local_date_time(week_start);
        let month = local_date_time(month_start);

        assert_eq!((day.year(), day.month(), day.day()), (2026, 7, 15));
        assert_eq!((week.year(), week.month(), week.day()), (2026, 7, 13));
        assert_eq!((month.year(), month.month(), month.day()), (2026, 7, 1));
        assert_eq!((day.hour(), day.minute(), day.second()), (0, 0, 0));
        assert_eq!((week.hour(), week.minute(), week.second()), (0, 0, 0));
        assert_eq!((month.hour(), month.minute(), month.second()), (0, 0, 0));
    }

    #[test]
    fn account_window_stats_match_local_account_id_not_shared_team_id() {
        // Multiple members can share one official Team/Workspace account_id.
        // Request logs must remain isolated by the local account record ID.
        assert!(super::account_window_stat_identity_matches(
            "codex-member-a",
            "codex-member-a"
        ));
        assert!(super::account_window_stat_identity_matches(
            "codex-member-b",
            "codex-member-b"
        ));
        assert!(!super::account_window_stat_identity_matches(
            "codex-member-a",
            "codex-member-b"
        ));
        assert!(!super::account_window_stat_identity_matches(
            "codex-member-a",
            ""
        ));
    }

    #[test]
    fn account_window_stats_isolate_local_accounts_with_shared_team_id() {
        let dir = make_temp_dir("codex-local-access-window-identity");
        let db_path = dir.join("request_logs.sqlite");
        let conn = open_local_access_logs_db_once(&db_path, true).expect("open logs db");
        let mut events = Vec::new();
        for (request_id, account_id, input_tokens) in [
            ("req-member-a", "local-member-a", 11),
            ("req-member-b", "local-member-b", 22),
        ] {
            let usage = UsageCapture {
                input_tokens,
                output_tokens: 1,
                total_tokens: input_tokens + 1,
                cached_tokens: 0,
                reasoning_tokens: 0,
                token_breakdown: None,
            };
            let event = append_usage_event(
                &mut events,
                1_700_000_000_000,
                Some(request_id),
                Some(account_id),
                Some("shared-team@example.com"),
                Some("key-1"),
                Some("shared-team"),
                None,
                Some("gpt-5.4"),
                Some(CodexLocalAccessGatewayMode::Sidecar),
                CodexLocalAccessRequestKind::Text,
                None,
                None,
                true,
                Some(200),
                None,
                None,
                1,
                Some(&usage),
                None,
                1,
                0.0,
            );
            insert_local_access_usage_event(&conn, &event).expect("insert request log");
        }

        let rows = super::query_local_access_account_window_stats_from_conn(
            &conn,
            vec![
                CodexLocalAccessAccountWindowQuery {
                    account_id: "local-member-a".to_string(),
                    window_key: "primary".to_string(),
                    start_at: 1_699_999_999_000,
                    end_at: 1_700_000_001_000,
                },
                CodexLocalAccessAccountWindowQuery {
                    account_id: "local-member-b".to_string(),
                    window_key: "primary".to_string(),
                    start_at: 1_699_999_999_000,
                    end_at: 1_700_000_001_000,
                },
            ],
        )
        .expect("query account window stats");

        let stats = rows
            .into_iter()
            .map(|row| (row.account_id, row.input_tokens))
            .collect::<HashMap<_, _>>();
        assert_eq!(stats.get("local-member-a"), Some(&11));
        assert_eq!(stats.get("local-member-b"), Some(&22));

        drop(conn);
        fs::remove_dir_all(dir).expect("cleanup logs db");
    }

    #[test]
    fn port_in_reserved_ranges_detects_membership() {
        assert!(super::port_in_reserved_ranges(1450, &[(1400, 1500)]));
        assert!(!super::port_in_reserved_ranges(1399, &[(1400, 1500)]));
    }

    #[test]
    fn parse_windows_excluded_port_ranges_reads_start_end() {
        let sample = "\nProtocol tcp Port Exclusion Ranges\n\nStart Port    End Port\n  1450         1459  \n  50000        50059\n";
        let ranges = super::parse_windows_excluded_port_ranges(sample);
        assert!(ranges.contains(&(1450, 1459)) || ranges.iter().any(|r| r.0 == 1450));
        assert!(super::port_in_reserved_ranges(1455, &ranges));
    }

    #[test]
    fn format_gateway_bind_error_mentions_reserved_when_matched() {
        let err = std::io::Error::from(std::io::ErrorKind::AddrInUse);
        let msg =
            super::format_gateway_bind_error_message("127.0.0.1", 1455, &err, &[(1400, 1500)]);
        assert!(
            msg.contains("保留") || msg.contains("excludedportrange") || msg.contains("Windows"),
            "msg={msg}"
        );
    }

    #[test]
    fn sidecar_bind_errors_are_retryable() {
        assert!(super::is_retryable_sidecar_bind_error(
            r#"API 服务 sidecar 在 ready 前退出: exit status: 1; ready_seen=false, last_stdout={"message":"listen tcp 127.0.0.1:61331: bind: An attempt was made to access a socket in a way forbidden by its access permissions.","type":"error"}, last_stderr=未捕获 stderr"#
        ));
        assert!(super::is_retryable_sidecar_bind_error(
            "listen tcp 127.0.0.1:61331: bind: address already in use"
        ));
        assert!(!super::is_retryable_sidecar_bind_error(
            "API 服务 sidecar 配置文件解析失败"
        ));
    }

    use base64::{engine::general_purpose, Engine as _};
    use ed25519_dalek::{pkcs8::EncodePrivateKey, SigningKey};

    use super::{
        account_model_rule_blocks_model, account_requires_bound_oauth_local_gateway,
        account_requires_provider_gateway, account_upstream_base_url, account_usage_priority,
        add_api_key_token_usage, align_codex_prompt_cache, api_key_inherits_account_pool,
        api_key_priority_account_ids, api_key_token_limit_exceeded,
        append_eligible_local_access_account_ids, append_usage_event,
        apply_account_usage_priority_ids, apply_codex_image_model_visibility,
        apply_codex_official_headers, apply_routing_strategy,
        backup_current_profile_model_before_provider_gateway, bound_oauth_quota_refresh_failures,
        bound_oauth_quota_reserve_blocks_account, bridge_websocket_streams,
        build_account_scoped_upstream_body, build_base_url_with_host,
        build_chat_completion_payload, build_chat_completion_stream_body,
        build_codex_client_models_response, build_collection_base_url, build_images_api_payload,
        build_local_access_api_key, build_local_models_response,
        build_model_provider_gateway_test_collection, build_ordered_account_ids,
        build_request_routing_hint, build_runtime_account, build_upstream_websocket_url,
        calculate_usage_cost_usd, calendar_stats_window_starts, canonical_model_for_client_model,
        account_has_gpt_reserve_entitlement,
        classify_upstream_error_category, cleanup_profile_takeover_without_backup,
        cleanup_provider_gateway_profile_model_overrides, codex_price,
        collect_local_access_profile_takeover_dirs_from_store, compare_routing_candidates,
        count_request_logs_for_model_ids, default_codex_model_ids, effective_api_key_account_ids,
        empty_stats_snapshot, extract_usage_capture, filter_bound_oauth_quota_reserve_account,
        filter_websocket_client_message, insert_local_access_usage_event,
        load_stats_windows_and_recent_events_from_conn,
        inspect_local_access_profile_attachment, inspect_local_access_profile_config,
        is_codex_local_access_auth_text, is_codex_local_access_config_for_api_key,
        is_codex_oauth_auth_text, is_image_generation_capability_error,
        is_local_access_eligible_account, is_local_access_gateway_base_url,
        is_provider_gateway_eligible_account, is_responses_completion_event,
        is_stream_incomplete_error_message, is_upstream_response_failed_error_message,
        legacy_stream_error_category, local_access_chat_completions_url,
        local_access_ineligible_reason, lookup_codex_model_provider_base_url_in_dir,
        macos_proxy_url_from_scutil_map, max_credential_attempts_for_strategy,
        merge_collection_and_account_excluded_models, model_pricing,
        model_provider_direct_test_client_model, model_provider_test_uses_provider_gateway,
        normalize_account_id_list, normalize_account_model_rules, normalize_collection_api_keys,
        normalize_custom_routing_rules, normalized_sidecar_error_category,
        open_local_access_logs_db_once, parse_codex_retry_after,
        parse_responses_payload_from_upstream, parse_websocket_upstream_error,
        pin_account_to_front_for_strategy, prepare_gateway_request,
        prepare_gateway_request_with_default_service_tier, prepare_sidecar_launch_config_in_dir,
        prepare_websocket_initial_request, profile_api_key_supports_websockets,
        profile_base_url_matches, provider_gateway_api_key_id,
        provider_gateway_bound_oauth_account_id_for_account,
        provider_gateway_default_model_for_account,
        provider_gateway_image_generation_mode_for_account, provider_gateway_model_slots,
        provider_gateway_models_for_account, provider_model_slots_need_upstream_rewrite,
        read_http_request, read_request_log_reprice_batch, recompute_time_windows,
        recover_invalid_stats_file, remove_account_refs_from_collection,
        remove_codex_local_access_config, reprice_request_logs_for_collection,
        request_image_generation_mode, request_logs_has_column, request_ordered_account_ids,
        resolve_collection_api_key, resolve_effective_model_pricing, resolve_plan_rank,
        resolve_sidecar_upstream_base_url, resolve_sidecar_upstream_base_url_with,
        resolve_supported_model_alias, resolve_upstream_target,
        restore_config_toml_from_takeover_backup, sanitize_collection_with_accounts,
        scutil_proxy_map, selected_account_ids_have_image_generation_capacity,
        send_agent_identity_wakeup_request_with_base_urls,
        should_retry_single_account_upstream_status, should_treat_response_as_stream,
        should_try_next_account, sidecar_account_manifest_value,
        sidecar_account_needs_background_refresh, sidecar_api_key_account_scope_values,
        sidecar_api_key_manifest_values, sidecar_api_key_priority_state_values,
        sidecar_auth_account_is_scoped, sidecar_auth_file_name, sidecar_auth_json_for_account,
        sidecar_auths_dir, sidecar_client_api_keys, sidecar_codex_api_key_auth_id,
        sidecar_codex_key_config_value, sidecar_config_fingerprint,
        sidecar_local_account_usable_for_start, sidecar_payload_default_service_tier,
        sidecar_quota_reserve_snapshot_value, sidecar_routing_strategy_value, sidecar_stable_id,
        sidecar_usage_event_is_client_canceled, sidecar_usage_event_should_auto_restart,
        now_ms, stats_snapshot_without_events, supported_codex_model_ids,
        sync_provider_gateway_runtime_auth_file,
        system_proxy_target_scheme, system_proxy_value_url,
        tool_declares_image_generation_capability, usage_event_from_row,
        validate_api_key_account_scope_update, validate_client_model_visible,
        validate_loaded_local_access_bound_oauth_account, visible_codex_model_ids_for_api_key,
        visible_codex_model_ids_for_api_key_with_accounts,
        visible_codex_model_ids_for_api_key_with_supported_models, websocket_accept_value,
        websocket_connect_error_from_http_response, windows_proxy_url_from_server,
        windows_reg_dword_enabled, windows_reg_query_map,
        write_local_access_profile_model_override, write_local_access_profile_takeover,
        write_provider_gateway_model_catalog, write_string_atomic, write_string_atomic_if_changed,
        AccountUsagePriority, CodexLocalAccessCollection, CodexLocalAccessGatewayMode,
        CodexLocalAccessScope, CodexModelProviderGatewayChatTestRequest, GatewayResponseAdapter,
        ParsedRequest, ResolvedLocalApiKey, ResponseUsageCollector, RoutingCandidate,
        SidecarUsageDetails, SidecarUsageEvent, UsageCapture,
        BOUND_OAUTH_QUOTA_RESERVE_MAX_SNAPSHOT_AGE_SECONDS, CODEX_AUTO_REVIEW_MODEL_ID,
        CODEX_GPT_RESERVE_MODEL_ID,
        CODEX_IMAGEGEN_ACTOR_HEADER, CODEX_IMAGE_MODEL_ID,
        CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE, CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE,
        CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
        CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE,
        CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE, CODEX_PROFILE_AUTH_FILE, CODEX_PROFILE_CONFIG_FILE,
        CODEX_PROVIDER_MODEL_BACKUP_FILE, CODEX_PROVIDER_MODEL_CATALOG_FILE,
        DEFAULT_MAX_RETRY_INTERVAL_MS, DEFAULT_MODEL_PRICING_VERSION,
        DEFAULT_SESSION_AFFINITY_TTL_MS, MAX_HTTP_REQUEST_BYTES,
        STATE_RECENT_USAGE_EVENT_LIMIT,
    };
    use super::{
        is_cockpit_managed_local_access_config, restore_profile_takeover_backup,
        CodexLocalAccessProfileTakeoverBackup, CODEX_LOCAL_ACCESS_AUTH_PROJECTION_FILE,
        CODEX_MODEL_CACHE_FILE,
    };
    use crate::models::codex::{
        CodexAccount, CodexAgentIdentity, CodexApiProviderMode, CodexAppSpeed, CodexQuota,
        CodexQuotaErrorInfo, CodexTokens,
    };
    use crate::models::codex_local_access::{
        CodexLocalAccessAccountModelRule, CodexLocalAccessAccountWindowQuery,
        CodexLocalAccessApiKey, CodexLocalAccessClientBaseUrlHost,
        CodexLocalAccessCustomRoutingRule, CodexLocalAccessImageGenerationMode,
        CodexLocalAccessModelRoute, CodexLocalAccessModelRouting,
        CodexLocalAccessProviderGateway, CodexLocalAccessQuotaReserve, CodexLocalAccessRequestKind,
        CodexLocalAccessRoutingStrategy, CodexLocalAccessStats, CodexLocalAccessStatsWindow,
        CodexLocalAccessTimeouts, CodexLocalAccessUsageEvent, CodexTokenBreakdown,
    };
    use crate::models::{
        DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile, InstanceStore,
    };
    use futures_util::{SinkExt, StreamExt};
    use rand::rngs::OsRng;
    use reqwest::StatusCode;
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use std::{
        collections::{HashMap, HashSet},
        fs,
        path::PathBuf,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;
    use tokio::time::Duration;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;
    use toml_edit::{value, Document};

    #[tokio::test]
    async fn read_http_request_rejects_declared_request_above_limit() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let request = format!(
            "POST /v1/responses HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_HTTP_REQUEST_BYTES
        );
        client.write_all(request.as_bytes()).await.unwrap();

        let err = tokio::time::timeout(
            Duration::from_millis(100),
            read_http_request(&mut server, Duration::from_secs(5)),
        )
        .await
        .expect("oversized request should be rejected before reading body")
        .expect_err("request should be rejected");

        assert_eq!(err, "请求体过大");
    }

    fn agent_identity_wakeup_test_account() -> CodexAccount {
        let signing_key = SigningKey::generate(&mut OsRng);
        let private_key = signing_key.to_pkcs8_der().expect("encode PKCS#8");
        let mut account = CodexAccount::new(
            format!("agent-wakeup-{}", uuid::Uuid::new_v4()),
            "agent-wakeup@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.account_id = Some("team-wakeup".to_string());
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-wakeup".to_string(),
            agent_private_key: general_purpose::STANDARD.encode(private_key.as_bytes()),
            task_id: Some("task-old".to_string()),
            account_id: "team-wakeup".to_string(),
            chatgpt_user_id: "user-wakeup".to_string(),
            email: Some(account.email.clone()),
            plan_type: Some("k12".to_string()),
            chatgpt_account_is_fedramp: true,
        });
        account
    }

    fn wakeup_assertion_task_id(request: &str) -> Option<String> {
        let authorization = request.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim())
        })?;
        let encoded = authorization.strip_prefix("AgentAssertion ")?;
        let payload = general_purpose::URL_SAFE_NO_PAD.decode(encoded).ok()?;
        serde_json::from_slice::<Value>(&payload)
            .ok()?
            .get("task_id")?
            .as_str()
            .map(str::to_string)
    }

    async fn read_wakeup_test_http_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 2048];
        let header_end = loop {
            let read = stream.read(&mut chunk).await.expect("read request");
            assert!(read > 0, "connection closed before request headers");
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut chunk).await.expect("read request body");
            assert!(read > 0, "connection closed before request body");
            bytes.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[tokio::test]
    async fn official_wakeup_agent_identity_recovers_invalid_task_once() {
        let account = agent_identity_wakeup_test_account();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let base_url = format!("http://{}", listener.local_addr().expect("local address"));
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut wakeup_calls = 0;
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().await.expect("accept request");
                let request = read_wakeup_test_http_request(&mut stream).await;
                let request_line = request.lines().next().unwrap_or_default().to_string();
                let (status, content_type, body) = if request_line.contains("/task/register") {
                    ("200 OK", "application/json", r#"{"task_id":"task-new"}"#)
                } else if wakeup_calls == 0 {
                    wakeup_calls += 1;
                    (
                        "401 Unauthorized",
                        "application/json",
                        r#"{"error":{"code":"invalid_task_id"}}"#,
                    )
                } else {
                    wakeup_calls += 1;
                    (
                        "200 OK",
                        "text/event-stream",
                        "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"awake\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_wakeup\",\"status\":\"completed\"}}\n\n",
                    )
                };
                requests.push(request);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write response");
            }
            requests
        });

        let mut headers = HashMap::new();
        headers.insert("accept".to_string(), "text/event-stream".to_string());
        headers.insert("content-type".to_string(), "application/json".to_string());
        headers.insert("x-openai-fedramp".to_string(), "true".to_string());
        let response = send_agent_identity_wakeup_request_with_base_urls(
            &account,
            "/responses",
            &headers,
            br#"{"model":"gpt-5.4-mini","stream":true}"#,
            None,
            Duration::from_secs(2),
            &CodexLocalAccessTimeouts::default(),
            &base_url,
            &base_url,
        )
        .await
        .expect("recover Agent Identity task and retry wakeup");

        assert!(response.status.is_success());
        assert_eq!(
            response
                .account
                .agent_identity
                .as_ref()
                .and_then(|identity| identity.task_id.as_deref()),
            Some("task-new")
        );
        let parsed = parse_responses_payload_from_upstream(response.body.as_bytes())
            .expect("parse wakeup SSE response");
        assert_eq!(
            parsed
                .get("response")
                .and_then(|value| value.get("output_text"))
                .and_then(Value::as_str),
            Some("awake")
        );

        let requests = server.await.expect("mock server");
        let wakeup_requests = requests
            .iter()
            .filter(|request| {
                request
                    .lines()
                    .next()
                    .is_some_and(|line| line.contains("/responses"))
            })
            .collect::<Vec<_>>();
        assert_eq!(wakeup_requests.len(), 2);
        assert_eq!(
            wakeup_assertion_task_id(wakeup_requests[0]),
            Some("task-old".to_string())
        );
        assert_eq!(
            wakeup_assertion_task_id(wakeup_requests[1]),
            Some("task-new".to_string())
        );
        assert!(wakeup_requests.iter().all(|request| {
            let lower = request.to_ascii_lowercase();
            lower.contains("originator: codex-tui")
                && lower.contains("chatgpt-account-id: team-wakeup")
                && lower.contains("x-openai-fedramp: true")
                && !lower.contains("authorization: bearer ")
        }));
    }

    fn test_local_access_collection(account_ids: Vec<String>) -> CodexLocalAccessCollection {
        CodexLocalAccessCollection {
            enabled: true,
            port: 14998,
            api_key: "local-api-key".to_string(),
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
            active_timeout_preset_id: "long_wait".to_string(),
            timeout_presets: Vec::new(),
            disable_cooling: false,
            restrict_free_accounts: true,
            debug_logs: true,
            immediate_sse_response: false,
            max_concurrent_image_requests: 1,
            bound_oauth_account_id: None,
            bound_oauth_quota_reserve: None,
            account_ids,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn sidecar_preparation_stops_after_lifecycle_generation_changes() {
        let collection = test_local_access_collection(vec!["account-1".to_string()]);
        let dir = std::env::temp_dir().join(format!(
            "cockpit-sidecar-cancel-test-{}",
            uuid::Uuid::new_v4()
        ));
        let current_generation = super::current_gateway_lifecycle_generation();
        let error = super::prepare_sidecar_launch_config_in_dir_sync(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            HashMap::new(),
            true,
            Some(super::GatewayPreparationContext {
                generation: current_generation.wrapping_add(1),
                total: 1,
            }),
        )
        .expect_err("stale preparation should be cancelled");

        assert_eq!(error, super::GATEWAY_PREPARATION_CANCELLED);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_collection_defaults_responses_websockets_to_disabled() {
        let mut legacy = serde_json::to_value(test_local_access_collection(Vec::new()))
            .expect("serialize collection");
        legacy
            .as_object_mut()
            .expect("collection should serialize as an object")
            .remove("responsesWebsocketsEnabled");

        let collection: CodexLocalAccessCollection =
            serde_json::from_value(legacy).expect("deserialize legacy collection");
        assert!(!collection.responses_websockets_enabled);
    }

    #[test]
    fn api_key_token_limit_defaults_and_accumulates_across_requests() {
        let mut collection = test_local_access_collection(vec!["account-1".to_string()]);
        let mut api_key = build_local_access_api_key(Some("Limited"));
        api_key.id = "limited-key".to_string();
        api_key.token_limit = Some(10_000_000);
        collection.api_keys = vec![api_key];

        assert!(add_api_key_token_usage(
            &mut collection,
            "limited-key",
            4_000_000
        ));
        assert!(add_api_key_token_usage(
            &mut collection,
            "limited-key",
            6_000_000
        ));
        let resolved = resolve_collection_api_key(&collection, collection.api_keys[0].key.as_str())
            .expect("limited key should resolve");
        assert_eq!(resolved.token_used, 10_000_000);
        assert_eq!(
            api_key_token_limit_exceeded(&resolved),
            Some((10_000_000, 10_000_000))
        );

        let legacy_value = json!({
            "id": "legacy-key",
            "label": "Legacy",
            "key": "legacy-secret",
            "allowedModels": [],
            "excludedModels": [],
            "enabled": true,
            "createdAt": 1,
            "updatedAt": 1
        });
        let legacy_key: CodexLocalAccessApiKey =
            serde_json::from_value(legacy_value).expect("legacy key should deserialize");
        assert_eq!(legacy_key.token_limit, None);
        assert_eq!(legacy_key.token_used, 0);
    }

    fn test_oauth_account_with_quota(
        account_id: &str,
        hourly_percentage: i32,
        weekly_percentage: i32,
        hourly_window_present: Option<bool>,
        weekly_window_present: Option<bool>,
    ) -> CodexAccount {
        let mut account = CodexAccount::new(
            account_id.to_string(),
            format!("{}@example.com", account_id),
            CodexTokens {
                id_token: "id-token".to_string(),
                access_token: "access-token".to_string(),
                refresh_token: Some("refresh-token".to_string()),
            },
        );
        account.quota = Some(CodexQuota {
            hourly_percentage,
            hourly_reset_time: None,
            hourly_window_minutes: Some(300),
            hourly_window_present,
            weekly_percentage,
            weekly_reset_time: None,
            weekly_window_minutes: Some(10_080),
            weekly_window_present,
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: None,
        });
        account.usage_updated_at = Some(chrono::Utc::now().timestamp());
        account
    }

    #[test]
    fn custom_api_key_scope_filters_duplicates_and_updates_manifest_scope() {
        let mut collection = test_local_access_collection(vec![
            "account-a".to_string(),
            "account-b".to_string(),
            "account-c".to_string(),
        ]);
        let mut api_key = build_local_access_api_key(Some("Team A"));
        api_key.key = "team-a-key".to_string();
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = normalize_account_id_list(vec![
            "account-b".to_string(),
            "account-b".to_string(),
            " account-c ".to_string(),
            "".to_string(),
        ]);
        collection.api_keys = vec![api_key];

        let manifest_values = sidecar_api_key_manifest_values(&collection);
        let scoped = manifest_values
            .iter()
            .find(|value| value.get("key").and_then(Value::as_str) == Some("team-a-key"))
            .expect("scoped key should be emitted");

        let account_ids = scoped
            .get("accountIds")
            .and_then(Value::as_array)
            .expect("accountIds should be an array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();

        assert_eq!(account_ids, vec!["account-b", "account-c"]);
        assert_eq!(
            scoped.get("responsesWebsockets").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            scoped.get("boundOAuth").and_then(Value::as_bool),
            Some(false)
        );

        collection.api_keys[0].token_limit = Some(10_000_000);
        collection.api_keys[0].token_used = 2_500_000;
        let limited = sidecar_api_key_manifest_values(&collection)
            .into_iter()
            .find(|value| value.get("key").and_then(Value::as_str) == Some("team-a-key"))
            .expect("limited key should be emitted");
        assert_eq!(
            limited.get("tokenLimit").and_then(Value::as_u64),
            Some(10_000_000)
        );
        assert_eq!(
            limited.get("tokenUsed").and_then(Value::as_u64),
            Some(2_500_000)
        );

        collection.bound_oauth_account_id = Some("oauth-account".to_string());
        let oauth_bound = sidecar_api_key_manifest_values(&collection)
            .into_iter()
            .find(|value| value.get("key").and_then(Value::as_str) == Some("team-a-key"))
            .expect("OAuth-bound key should still be emitted");
        assert_eq!(
            oauth_bound.get("boundOAuth").and_then(Value::as_bool),
            Some(true)
        );
        collection.bound_oauth_account_id = None;

        collection.responses_websockets_enabled = true;
        let enabled = sidecar_api_key_manifest_values(&collection)
            .into_iter()
            .find(|value| value.get("key").and_then(Value::as_str) == Some("team-a-key"))
            .expect("scoped key should still be emitted");
        assert_eq!(
            enabled.get("responsesWebsockets").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn api_key_priority_queue_respects_custom_scope_session_affinity_and_fallbacks() {
        let mut collection = test_local_access_collection(vec![
            "account-a".to_string(),
            "account-b".to_string(),
            "account-c".to_string(),
        ]);
        let mut api_key = build_local_access_api_key(Some("Team A"));
        api_key.id = "key-team-a".to_string();
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec![
            "account-a".to_string(),
            "account-b".to_string(),
            "account-c".to_string(),
        ];
        api_key.priority_account_ids = vec!["account-a".to_string(), "account-b".to_string()];
        collection.api_keys = vec![api_key.clone()];

        let resolved = ResolvedLocalApiKey {
            id: api_key.id.clone(),
            label: api_key.label.clone(),
            provider_gateway: None,
            inherit_account_pool: false,
            account_ids: api_key.account_ids.clone(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };

        assert_eq!(
            api_key_priority_account_ids(&collection, &resolved),
            vec!["account-a", "account-b"]
        );
        assert_eq!(
            request_ordered_account_ids(
                &collection,
                &resolved.account_ids,
                CodexLocalAccessRoutingStrategy::Auto,
                0,
                &["account-c".to_string()],
            ),
            vec!["account-c", "account-a", "account-b"],
            "an existing session affinity must win over the Key preference"
        );
        assert_eq!(
            request_ordered_account_ids(
                &collection,
                &resolved.account_ids,
                CodexLocalAccessRoutingStrategy::Auto,
                0,
                &api_key_priority_account_ids(&collection, &resolved),
            ),
            vec!["account-a", "account-b", "account-c"],
            "new requests should try priority accounts in queue order"
        );
        assert_eq!(
            request_ordered_account_ids(
                &collection,
                &["account-b".to_string(), "account-c".to_string()],
                CodexLocalAccessRoutingStrategy::Auto,
                0,
                &api_key_priority_account_ids(&collection, &resolved),
            ),
            vec!["account-b", "account-c"],
            "when the first priority account is unavailable, the next priority account is tried"
        );

        let priorities = sidecar_api_key_priority_state_values(&collection);
        assert_eq!(
            priorities
                .get("priorityAccountIds")
                .and_then(|value| value.get("key-team-a"))
                .and_then(Value::as_array)
                .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(vec!["account-a", "account-b"])
        );

        collection.api_keys[0].inherit_account_pool = Some(true);
        assert!(normalize_collection_api_keys(&mut collection));
        assert!(collection.api_keys[0].priority_account_ids.is_empty());
        assert!(sidecar_api_key_priority_state_values(&collection)
            .get("priorityAccountIds")
            .and_then(Value::as_object)
            .is_some_and(|values| values.is_empty()));
    }

    #[test]
    fn legacy_api_key_scope_migrates_from_account_ids() {
        let mut collection =
            test_local_access_collection(vec!["account-a".to_string(), "account-b".to_string()]);
        let mut inherited_key = build_local_access_api_key(Some("Inherited"));
        inherited_key.inherit_account_pool = None;
        inherited_key.account_ids.clear();
        let mut scoped_key = build_local_access_api_key(Some("Scoped"));
        scoped_key.inherit_account_pool = None;
        scoped_key.account_ids = vec!["account-b".to_string()];
        collection.api_keys = vec![inherited_key, scoped_key];

        assert!(normalize_collection_api_keys(&mut collection));

        assert_eq!(collection.api_keys[0].inherit_account_pool, Some(true));
        assert_eq!(collection.api_keys[1].inherit_account_pool, Some(false));
        assert_eq!(
            effective_api_key_account_ids(&collection, &collection.api_keys[0]),
            vec!["account-a", "account-b"]
        );
        assert_eq!(
            effective_api_key_account_ids(&collection, &collection.api_keys[1]),
            vec!["account-b"]
        );
    }

    #[test]
    fn sidecar_stable_id_matches_config_synthesizer_rule() {
        assert_eq!(
            sidecar_stable_id("codex:apikey", &["sk-test", "https://api.deepseek.com/v1"]),
            "codex:apikey:b1193dcdb71b"
        );
    }

    #[test]
    fn sidecar_payload_default_service_tier_builds_supported_format_priority_default_rule() {
        let payload =
            sidecar_payload_default_service_tier(Some("priority")).expect("payload should exist");
        let rules = payload
            .get("default")
            .and_then(Value::as_array)
            .expect("default rules should exist");

        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0]
                .get("params")
                .and_then(|params| params.get("service_tier"))
                .and_then(Value::as_str),
            Some("priority")
        );

        let models = rules[0]
            .get("models")
            .and_then(Value::as_array)
            .expect("model rules should exist");
        let payload_formats = models
            .iter()
            .filter_map(|model| model.get("protocol").and_then(Value::as_str))
            .collect::<HashSet<_>>();

        assert_eq!(models.len(), 3);
        assert!(models
            .iter()
            .all(|model| { model.get("name").and_then(Value::as_str) == Some("*") }));
        assert!(payload_formats.contains("codex"));
        assert!(payload_formats.contains("openai"));
        assert!(payload_formats.contains("openai-response"));
    }

    #[test]
    fn sidecar_payload_default_service_tier_skips_none_and_unsupported_values() {
        assert!(sidecar_payload_default_service_tier(None).is_none());
        // "fast" normalizes to priority and is injectable as a default.
        let fast = sidecar_payload_default_service_tier(Some("fast")).expect("fast -> priority");
        let rules = fast
            .get("default")
            .and_then(Value::as_array)
            .expect("default rules");
        assert_eq!(
            rules[0]
                .get("params")
                .and_then(|params| params.get("service_tier"))
                .and_then(Value::as_str),
            Some("priority")
        );
        // standard/default should not force an explicit upstream field.
        assert!(sidecar_payload_default_service_tier(Some("standard")).is_none());
        assert!(sidecar_payload_default_service_tier(Some("default")).is_none());
    }

    #[test]
    fn system_proxy_target_scheme_defaults_to_https_for_invalid_url() {
        assert_eq!(
            system_proxy_target_scheme("https://api.openai.com/v1"),
            "https"
        );
        assert_eq!(system_proxy_target_scheme("not a url"), "https");
    }

    #[test]
    fn macos_scutil_proxy_prefers_https_static_proxy() {
        let output = r#"
<dictionary> {
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7891
  HTTPSProxy : proxy.local
  SOCKSEnable : 1
  SOCKSPort : 7892
  SOCKSProxy : socks.local
}
"#;

        let values = scutil_proxy_map(output);

        assert_eq!(
            macos_proxy_url_from_scutil_map(&values, "https").as_deref(),
            Some("http://proxy.local:7891")
        );
    }

    #[test]
    fn macos_scutil_proxy_falls_back_to_socks() {
        let output = r#"
<dictionary> {
  HTTPEnable : 0
  HTTPSEnable : 0
  SOCKSEnable : 1
  SOCKSPort : 7892
  SOCKSProxy : 127.0.0.1
}
"#;

        let values = scutil_proxy_map(output);

        assert_eq!(
            macos_proxy_url_from_scutil_map(&values, "https").as_deref(),
            Some("socks5://127.0.0.1:7892")
        );
    }

    #[test]
    fn windows_proxy_server_prefers_https_entry_for_https_target() {
        assert_eq!(
            windows_proxy_url_from_server(
                "http=127.0.0.1:7890;https=proxy.local:7891;socks=127.0.0.1:7892",
                "https"
            )
            .as_deref(),
            Some("http://proxy.local:7891")
        );
    }

    #[test]
    fn windows_proxy_server_supports_single_host_port() {
        assert_eq!(
            windows_proxy_url_from_server("127.0.0.1:7890", "https").as_deref(),
            Some("http://127.0.0.1:7890")
        );
    }

    #[test]
    fn windows_reg_query_map_reads_proxy_fields() {
        let output = r#"
HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Internet Settings
    ProxyEnable    REG_DWORD    0x1
    ProxyServer    REG_SZ       http=127.0.0.1:7890;https=proxy.local:7891
"#;
        let values = windows_reg_query_map(output);

        assert!(windows_reg_dword_enabled(values.get("ProxyEnable")));
        assert_eq!(
            values.get("ProxyServer").map(String::as_str),
            Some("http=127.0.0.1:7890;https=proxy.local:7891")
        );
    }

    #[test]
    fn system_proxy_value_url_preserves_explicit_https_scheme() {
        assert_eq!(
            system_proxy_value_url("https", "https://proxy.local:8443").as_deref(),
            Some("https://proxy.local:8443")
        );
    }

    #[test]
    fn sidecar_codex_api_key_auth_id_uses_api_key_identity() {
        let account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "deepseek@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );

        assert_eq!(
            sidecar_codex_api_key_auth_id(&account).as_deref(),
            Some("codex:apikey:b1193dcdb71b")
        );
    }

    #[test]
    fn chat_completions_api_key_requires_provider_gateway() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "deepseek@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("chat_completions".to_string());

        assert!(account_requires_provider_gateway(&account));
    }

    #[test]
    fn official_deepseek_responses_uses_shell_remap_gateway() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "deepseek@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        assert!(account_requires_provider_gateway(&account));
        assert_eq!(
            provider_gateway_model_slots(&provider_gateway_models_for_account(&account))
                .iter()
                .map(|slot| (slot.client_model.as_str(), slot.upstream_model.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("gpt-5.5", "deepseek-v4-flash"),
                ("gpt-5.4", "deepseek-v4-pro"),
            ]
        );

        account.api_instance_access_mode = Some("direct".to_string());
        assert!(!account_requires_provider_gateway(&account));

        account.api_instance_access_mode = Some("cdp".to_string());
        assert!(!account_requires_provider_gateway(&account));
    }

    #[test]
    fn official_template_catalog_keeps_shell_slug_and_upstream_metadata() {
        let slots = provider_gateway_model_slots(&[
            "deepseek-v4-flash".to_string(),
            "deepseek-v4-pro".to_string(),
        ]);
        let official = r#"{
            "models": [
                {
                    "slug": "deepseek-v4-flash",
                    "display_name": "DeepSeek-V4-Flash",
                    "apply_patch_tool_type": "freeform",
                    "shell_type": "shell_command",
                    "base_instructions": "flash-instr"
                },
                {
                    "slug": "deepseek-v4-pro",
                    "display_name": "DeepSeek-V4-Pro",
                    "apply_patch_tool_type": "freeform",
                    "shell_type": "shell_command",
                    "base_instructions": "pro-instr"
                }
            ]
        }"#;
        let json = super::build_official_template_mapped_catalog_json(&slots, official)
            .expect("build mapped catalog");
        let catalog: Value = serde_json::from_str(&json).expect("parse mapped catalog");
        let models = catalog
            .get("models")
            .and_then(Value::as_array)
            .expect("models");
        assert_eq!(
            models[0].get("slug").and_then(Value::as_str),
            Some("gpt-5.5")
        );
        assert_eq!(
            models[0].get("display_name").and_then(Value::as_str),
            Some("DeepSeek-V4-Flash")
        );
        assert_eq!(
            models[0]
                .get("apply_patch_tool_type")
                .and_then(Value::as_str),
            Some("freeform")
        );
        assert_eq!(
            models[1].get("slug").and_then(Value::as_str),
            Some("gpt-5.4")
        );
        assert_eq!(
            models[1].get("display_name").and_then(Value::as_str),
            Some("DeepSeek-V4-Pro")
        );
    }

    #[test]
    fn responses_bound_oauth_api_key_uses_local_access_pool() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "relay@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());
        account.bound_oauth_account_id = Some("oauth-1".to_string());

        assert!(!account_requires_provider_gateway(&account));
        assert!(!account_requires_bound_oauth_local_gateway(&account));
        assert!(is_local_access_eligible_account(&account, false));

        account.bound_oauth_use_local_gateway = true;
        assert!(!account_requires_provider_gateway(&account));
        // 绑定 OAuth：不走本地网关路径（与改前一致）
        assert!(!account_requires_bound_oauth_local_gateway(&account));
        assert!(is_local_access_eligible_account(&account, false));

        account.bound_oauth_account_id = None;
        assert!(!account_requires_provider_gateway(&account));
        assert!(!account_requires_bound_oauth_local_gateway(&account));
        assert!(is_local_access_eligible_account(&account, false));
    }

    #[test]
    fn chat_bound_oauth_api_key_keeps_provider_gateway_branch() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "chat@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("chat_completions".to_string());
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.bound_oauth_use_local_gateway = true;

        assert!(account_requires_provider_gateway(&account));
        assert!(!account_requires_bound_oauth_local_gateway(&account));
    }

    #[test]
    fn responses_api_key_does_not_require_provider_gateway() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "openai@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.openai.com/v1".to_string()),
            Some("openai".to_string()),
            Some("OpenAI".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());

        assert!(!account_requires_provider_gateway(&account));
    }

    #[test]
    fn provider_gateway_models_prefers_account_catalog() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "deepseek@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-pro".to_string(),
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_model_catalog.push(" ".to_string());

        assert_eq!(
            provider_gateway_models_for_account(&account),
            vec![
                "deepseek-v4-pro".to_string(),
                "deepseek-v4-flash".to_string()
            ]
        );
    }

    #[test]
    fn provider_gateway_models_empty_for_unknown_provider_without_catalog() {
        let account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "custom@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://example-provider.test/v1".to_string()),
            Some("custom-provider".to_string()),
            Some("Custom Provider".to_string()),
            Vec::new(),
        );

        assert!(provider_gateway_models_for_account(&account).is_empty());
        assert!(provider_gateway_default_model_for_account(&account).is_empty());
    }

    #[test]
    fn provider_gateway_model_slots_are_stable_and_bounded() {
        let slots = provider_gateway_model_slots(&[
            "deepseek-v4-pro".to_string(),
            "deepseek-v4-flash".to_string(),
            "deepseek-v4-lite".to_string(),
            "deepseek-v4-extra".to_string(),
            "gpt-5.5".to_string(),
            "custom-overflow-a".to_string(),
            "custom-overflow-b".to_string(),
            "custom-overflow-c".to_string(),
            "custom-overflow-d".to_string(),
            "custom-overflow-e".to_string(),
            "custom-overflow-f".to_string(),
        ]);

        assert_eq!(
            slots
                .iter()
                .map(|slot| (slot.client_model.as_str(), slot.upstream_model.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("gpt-5.5", "gpt-5.5"),
                ("gpt-5.6-sol", "deepseek-v4-pro"),
                ("gpt-5.6-terra", "deepseek-v4-flash"),
                ("gpt-5.6-luna", "deepseek-v4-lite"),
                ("gpt-5.4", "deepseek-v4-extra"),
                ("gpt-5.4-mini", "custom-overflow-a"),
                ("gpt-5.3-codex", "custom-overflow-b"),
                ("gpt-5.3-codex-spark", "custom-overflow-c"),
                ("gpt-5.2", "custom-overflow-d"),
                // Shell pool exhausted: keep upstream IDs so all models remain listed.
                ("custom-overflow-e", "custom-overflow-e"),
                ("custom-overflow-f", "custom-overflow-f"),
            ]
        );
        assert!(provider_model_slots_need_upstream_rewrite(&slots));
    }

    #[test]
    fn provider_gateway_model_slots_keep_identity_for_official_shells() {
        let slots = provider_gateway_model_slots(&[
            "gpt-5.6-sol".to_string(),
            "gpt-5.5".to_string(),
            "grok-4.5".to_string(),
        ]);
        assert_eq!(
            slots
                .iter()
                .map(|slot| (slot.client_model.as_str(), slot.upstream_model.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("gpt-5.6-sol", "gpt-5.6-sol"),
                ("gpt-5.5", "gpt-5.5"),
                ("gpt-5.6-terra", "grok-4.5"),
            ]
        );
    }

    #[test]
    fn provider_gateway_model_slots_keep_identity_for_gpt_6_astra() {
        let slots = provider_gateway_model_slots(&["gpt-6-astra".to_string()]);
        assert_eq!(
            slots,
            vec![super::ProviderGatewayModelSlot {
                client_model: "gpt-6-astra".to_string(),
                upstream_model: "gpt-6-astra".to_string(),
            }]
        );
    }

    #[test]
    fn responses_sync_catalog_with_custom_models_requires_instance_gateway_but_remains_local_access_eligible(
    ) {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "relay@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.apikey.fun/v1".to_string()),
            Some("apikey_fun".to_string()),
            Some("APIKEY.FUN".to_string()),
            vec!["grok-4.5".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        assert!(account_requires_provider_gateway(&account));
        assert!(is_local_access_eligible_account(&account, true));
        assert_eq!(local_access_ineligible_reason(&account, true), None);

        let account_id = account.id.clone();
        let (next_ids, synced_ids, added_ids, skipped) = append_eligible_local_access_account_ids(
            &[],
            vec![account_id.clone()],
            &[account],
            true,
        );
        assert_eq!(next_ids, vec![account_id.clone()]);
        assert_eq!(synced_ids, vec![account_id.clone()]);
        assert_eq!(added_ids, vec![account_id]);
        assert!(skipped.is_empty());
    }

    #[test]
    fn responses_sync_catalog_with_only_official_shells_skips_provider_gateway() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "relay@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.apikey.fun/v1".to_string()),
            Some("apikey_fun".to_string()),
            Some("APIKEY.FUN".to_string()),
            vec!["gpt-5.5".to_string(), "gpt-5.6-sol".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        assert!(!account_requires_provider_gateway(&account));
    }

    #[test]
    fn provider_gateway_writes_static_catalog_for_chat_completions_models() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-override-state-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        let slots = provider_gateway_model_slots(&[
            "deepseek-v4-pro".to_string(),
            "deepseek-v4-flash".to_string(),
        ]);
        let client_models = slots
            .iter()
            .map(|slot| slot.client_model.clone())
            .collect::<Vec<_>>();
        backup_current_profile_model_before_provider_gateway(&profile_dir, &client_models)
            .expect("track provider models");
        write_local_access_profile_model_override(&profile_dir, "gpt-5.5")
            .expect("write model override");
        write_provider_gateway_model_catalog(&profile_dir, &slots)
            .expect("write provider model catalog");

        let catalog: Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE))
                .expect("read provider model catalog"),
        )
        .expect("parse provider model catalog");
        let models = catalog
            .get("models")
            .and_then(Value::as_array)
            .expect("models should be an array");
        for (model_id, display_name) in [
            ("gpt-5.5", "deepseek-v4-flash"),
            ("gpt-5.4", "deepseek-v4-pro"),
        ] {
            assert!(models.iter().any(|model| {
                model.get("slug").and_then(Value::as_str) == Some(model_id)
                    && model.get("display_name").and_then(Value::as_str) == Some(display_name)
                    && model.get("visibility").and_then(Value::as_str) == Some("list")
            }));
        }
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains(&format!(
            "model_catalog_json = \"{}\"",
            CODEX_PROVIDER_MODEL_CATALOG_FILE
        )));
        assert!(config.contains("model = \"gpt-5.5\""));

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!config.contains("model = \"gpt-5.6-sol\""));
        assert!(!profile_dir
            .join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
            .exists());
        assert!(!profile_dir
            .join(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
            .exists());
        assert!(!profile_dir.join(CODEX_PROVIDER_MODEL_BACKUP_FILE).exists());

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_removes_managed_model_override() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-cleanup-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        fs::write(
            profile_dir.join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"deepseek-v4-pro"},{"slug":"deepseek-v4-flash"}]}"#,
        )
        .expect("write stale model catalog");
        fs::write(
            profile_dir.join(CODEX_PROFILE_CONFIG_FILE),
            format!(
                "model_catalog_json = \"{}\"\nmodel = \"deepseek-v4-pro\"\n",
                CODEX_PROVIDER_MODEL_CATALOG_FILE
            ),
        )
        .expect("write stale config");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        assert!(!profile_dir
            .join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
            .exists());
        assert!(!profile_dir
            .join(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
            .exists());
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!config.contains("model = \"deepseek-v4-pro\""));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_restores_previous_official_model() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-restore-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        write_local_access_profile_model_override(&profile_dir, "gpt-5.5")
            .expect("write official model");
        backup_current_profile_model_before_provider_gateway(
            &profile_dir,
            &[
                "deepseek-v4-pro".to_string(),
                "deepseek-v4-flash".to_string(),
            ],
        )
        .expect("backup official model");
        write_local_access_profile_model_override(&profile_dir, "deepseek-v4-pro")
            .expect("write provider model");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        assert!(!profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE).exists());
        assert!(!profile_dir.join(CODEX_PROVIDER_MODEL_BACKUP_FILE).exists());
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!config.contains("model = \"deepseek-v4-pro\""));
        assert!(config.contains("model = \"gpt-5.5\""));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_restores_previous_model_without_catalog_file() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-restore-no-catalog-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        write_local_access_profile_model_override(&profile_dir, "gpt-5.5")
            .expect("write official model");
        backup_current_profile_model_before_provider_gateway(
            &profile_dir,
            &[
                "deepseek-v4-pro".to_string(),
                "deepseek-v4-flash".to_string(),
            ],
        )
        .expect("backup official model");
        write_local_access_profile_model_override(&profile_dir, "deepseek-v4-pro")
            .expect("write provider model");

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        let existing = fs::read_to_string(&config_path).expect("read config");
        let mut doc = existing.parse::<Document>().expect("parse config");
        doc["model_catalog_json"] = value(CODEX_PROVIDER_MODEL_CATALOG_FILE);
        let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        write_string_atomic(&config_path, &content).expect("write config");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        assert!(!profile_dir.join(CODEX_PROVIDER_MODEL_BACKUP_FILE).exists());
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!config.contains("model = \"deepseek-v4-pro\""));
        assert!(config.contains("model = \"gpt-5.5\""));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_restores_original_model_matching_provider_shell() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-restore-matching-shell-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        write_local_access_profile_model_override(&profile_dir, "gpt-5.5")
            .expect("write original model");
        backup_current_profile_model_before_provider_gateway(
            &profile_dir,
            &["gpt-5.5".to_string(), "gpt-5.4".to_string()],
        )
        .expect("backup original model");
        write_provider_gateway_model_catalog(
            &profile_dir,
            &provider_gateway_model_slots(&[
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ]),
        )
        .expect("write provider catalog");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains("model = \"gpt-5.5\""));
        assert!(!config.contains("model_catalog_json"));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_reapplies_enabled_experimental_catalog() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-restore-experimental-catalog-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");
        fs::write(
            profile_dir.join(CODEX_PROFILE_CONFIG_FILE),
            "model = \"gpt-5.6-sol\"\n",
        )
        .expect("write original config");
        crate::modules::codex_account::save_quick_config_for_base_dir(
            &profile_dir,
            None,
            None,
            Some(true),
            None,
        )
        .expect("enable experimental catalog");

        backup_current_profile_model_before_provider_gateway(
            &profile_dir,
            &["gpt-5.5".to_string(), "gpt-5.4".to_string()],
        )
        .expect("backup original model");
        write_local_access_profile_model_override(&profile_dir, "gpt-5.5")
            .expect("write provider model");
        write_provider_gateway_model_catalog(
            &profile_dir,
            &provider_gateway_model_slots(&[
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ]),
        )
        .expect("write provider catalog");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains(&format!(
            "model_catalog_json = \"{}\"",
            CODEX_PROVIDER_MODEL_CATALOG_FILE
        )));
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(profile_dir
            .join(CODEX_PROVIDER_MODEL_CATALOG_FILE)
            .is_file());
        assert!(
            crate::modules::codex_account::read_quick_config_from_config_toml(&profile_dir)
                .expect("read quick config")
                .experimental_model_catalog_enabled
        );

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_keeps_non_cockpit_model_catalog() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-keep-external-catalog-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        fs::write(
            &config_path,
            r#"model_provider = "ccswitch_deepseek"
model_catalog_json = "cc-switch-model-catalog.json"
model = "deepseek-v4-pro"

[model_providers.ccswitch_deepseek]
name = "CCSwitch DeepSeek"
base_url = "https://deepseek.example.com/v1"
wire_api = "responses"
"#,
        )
        .expect("write config");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        let config = fs::read_to_string(config_path).expect("read config");
        assert!(config.contains("model_catalog_json = \"cc-switch-model-catalog.json\""));
        assert!(config.contains("model_provider = \"ccswitch_deepseek\""));
        assert!(config.contains("model = \"deepseek-v4-pro\""));
        assert!(config.contains("[model_providers.ccswitch_deepseek]"));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_preserves_unowned_managed_model_catalog() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-preserve-unowned-catalog-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        let catalog_path = profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE);
        let config = format!(
            "model_catalog_json = \"{}\"\nmodel = \"custom-model\"\nmodel_context_window = 1000000\n",
            CODEX_PROVIDER_MODEL_CATALOG_FILE
        );
        let catalog = r#"{"models":[{"slug":"custom-model"}]}"#;
        fs::write(&config_path, &config).expect("write config");
        fs::write(&catalog_path, catalog).expect("write catalog");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        assert_eq!(
            fs::read_to_string(&config_path).expect("read config"),
            config
        );
        assert_eq!(
            fs::read_to_string(&catalog_path).expect("read catalog"),
            catalog
        );

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_takeover_cleanup_preserves_local_access_provider_for_history_sessions() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-preserve-history-provider-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        let config_content = r#"model_provider = "codex_local_access"
model = "gpt-5.6-sol"
model_context_window = 1000000

[model_providers.codex_local_access]
name = "Codex Local Access"
base_url = "http://127.0.0.1:51525/v1"
wire_api = "responses"
requires_openai_auth = true
http_headers = { "x-cockpit-instance-id" = "default" }
"#;
        fs::write(&config_path, config_content).expect("write initial config");

        let cleaned = remove_codex_local_access_config(config_content).expect("cleanup config");
        assert!(!cleaned.contains("model_provider = \"codex_local_access\""));
        assert!(cleaned.contains("model_context_window = 1000000"));
        assert!(cleaned.contains("[model_providers.codex_local_access]"));
        assert!(cleaned.contains("base_url = \"http://127.0.0.1:51525/v1\""));
        assert!(cleaned.contains("wire_api = \"responses\""));
        assert!(!cleaned.contains("x-cockpit-instance-id"));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn normalizes_account_model_rules_for_collection_accounts() {
        let rules = normalize_account_model_rules(
            vec![
                CodexLocalAccessAccountModelRule {
                    account_id: " account-a ".to_string(),
                    excluded_models: vec!["gpt-5.4-mini".to_string(), "GPT-5.4-MINI".to_string()],
                },
                CodexLocalAccessAccountModelRule {
                    account_id: "account-b".to_string(),
                    excluded_models: vec!["".to_string(), "gpt-5.3-*".to_string()],
                },
                CodexLocalAccessAccountModelRule {
                    account_id: "missing".to_string(),
                    excluded_models: vec!["gpt-5.2".to_string()],
                },
            ],
            &["account-a".to_string(), "account-b".to_string()],
        );

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].account_id, "account-a");
        assert_eq!(rules[0].excluded_models, vec!["gpt-5.4-mini"]);
        assert_eq!(rules[1].account_id, "account-b");
        assert_eq!(rules[1].excluded_models, vec!["gpt-5.3-*"]);
    }

    #[test]
    fn remove_account_refs_clears_all_local_access_references() {
        let mut collection = test_local_access_collection(vec![
            "account-a".to_string(),
            "account-b".to_string(),
            "account-c".to_string(),
        ]);
        let mut scoped_key = build_local_access_api_key(Some("scoped"));
        scoped_key.inherit_account_pool = Some(false);
        scoped_key.account_ids = vec!["account-b".to_string(), "account-c".to_string()];
        collection.api_keys = vec![scoped_key];
        collection.custom_routing_rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: "account-b".to_string(),
                priority: 10,
                weight: 2,
                is_backup: false,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "account-c".to_string(),
                priority: 5,
                weight: 1,
                is_backup: false,
                is_preferred: false,
            },
        ];
        collection.account_model_rules = vec![CodexLocalAccessAccountModelRule {
            account_id: "account-b".to_string(),
            excluded_models: vec!["gpt-5.4-mini".to_string()],
        }];
        collection.bound_oauth_account_id = Some("account-b".to_string());
        collection.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
            hourly_percent: 20,
            weekly_percent: 30,
        });

        let changed = remove_account_refs_from_collection(
            &mut collection,
            &HashSet::from(["account-b".to_string()]),
        );

        assert!(changed);
        assert_eq!(
            collection.account_ids,
            vec!["account-a".to_string(), "account-c".to_string()]
        );
        assert_eq!(collection.api_keys[0].account_ids, vec!["account-c"]);
        assert_eq!(collection.custom_routing_rules.len(), 1);
        assert_eq!(collection.custom_routing_rules[0].account_id, "account-c");
        assert!(collection.account_model_rules.is_empty());
        assert!(collection.bound_oauth_account_id.is_none());
        assert!(collection.bound_oauth_quota_reserve.is_none());
    }

    #[test]
    fn custom_scope_does_not_broaden_when_last_account_is_removed() {
        let mut collection =
            test_local_access_collection(vec!["account-a".to_string(), "account-b".to_string()]);
        let mut scoped_key = build_local_access_api_key(Some("scoped"));
        scoped_key.key = "scoped-key".to_string();
        scoped_key.inherit_account_pool = Some(false);
        scoped_key.account_ids = vec!["account-b".to_string()];
        collection.api_keys = vec![scoped_key];

        assert!(remove_account_refs_from_collection(
            &mut collection,
            &HashSet::from(["account-b".to_string()]),
        ));

        let api_key = &collection.api_keys[0];
        assert!(!api_key_inherits_account_pool(api_key));
        assert!(api_key.account_ids.is_empty());
        assert!(effective_api_key_account_ids(&collection, api_key).is_empty());
        assert!(sidecar_api_key_manifest_values(&collection)
            .iter()
            .all(|value| value.get("key").and_then(Value::as_str) != Some("scoped-key")));
    }

    #[test]
    fn deleted_scoped_account_does_not_fall_back_to_legacy_sidecar_pool() {
        let mut collection = test_local_access_collection(vec!["account-a".to_string()]);
        let mut scoped_key = build_local_access_api_key(Some("Scoped"));
        scoped_key.key = collection.api_key.clone();
        scoped_key.inherit_account_pool = Some(false);
        scoped_key.account_ids = vec!["account-b".to_string()];
        collection.api_keys = vec![scoped_key];

        assert!(remove_account_refs_from_collection(
            &mut collection,
            &HashSet::from(["account-b".to_string()]),
        ));

        assert!(collection.api_keys[0].account_ids.is_empty());
        assert!(sidecar_api_key_manifest_values(&collection)
            .iter()
            .all(|value| value.get("key").and_then(Value::as_str) != Some("local-api-key")));
        assert!(!sidecar_client_api_keys(&collection, &HashMap::new())
            .iter()
            .any(|key| key == "local-api-key"));
        assert!(
            sidecar_api_key_account_scope_values(&collection, &HashMap::new())
                .get("local-api-key")
                .is_none()
        );
    }

    #[test]
    fn sidecar_excludes_key_with_unresolved_custom_scope() {
        let mut collection = test_local_access_collection(vec!["account-a".to_string()]);
        let mut scoped_key = build_local_access_api_key(Some("Scoped"));
        scoped_key.key = "scoped-key".to_string();
        scoped_key.inherit_account_pool = Some(false);
        scoped_key.account_ids = vec!["missing-account".to_string()];
        collection.api_keys = vec![scoped_key];

        assert!(!sidecar_client_api_keys(&collection, &HashMap::new())
            .iter()
            .any(|key| key == "scoped-key"));
        assert!(
            sidecar_api_key_account_scope_values(&collection, &HashMap::new())
                .get("scoped-key")
                .is_none()
        );
    }

    #[test]
    fn bound_oauth_gateway_key_rejects_inherited_or_empty_scope_updates() {
        let account_id = "api-bound-oauth-1".to_string();
        let mut collection = test_local_access_collection(Vec::new());
        collection.bound_oauth_account_id = Some("oauth-1".to_string());

        let mut api_key = build_local_access_api_key(Some("Bound OAuth Local Gateway"));
        api_key.id = provider_gateway_api_key_id(&account_id);
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec![account_id.clone()];

        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&[]),
            Some(true),
        )
        .is_err());
        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&[]),
            Some(false),
        )
        .is_err());
        assert!(
            validate_api_key_account_scope_update(&collection, &api_key, None, Some(true),)
                .is_err()
        );
        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&[account_id.clone()]),
            Some(false),
        )
        .is_ok());
        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&["api-bound-oauth-2".to_string()]),
            Some(false),
        )
        .is_err());
        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&[account_id.clone(), "api-bound-oauth-2".to_string()]),
            Some(false),
        )
        .is_err());

        let regular_collection = test_local_access_collection(vec!["account-a".to_string()]);
        let regular_key = build_local_access_api_key(Some("Regular Key"));
        assert!(validate_api_key_account_scope_update(
            &regular_collection,
            &regular_key,
            None,
            Some(true),
        )
        .is_ok());
    }

    #[test]
    fn custom_scope_model_visibility_uses_scoped_accounts_for_image_capacity() {
        let mut paid_account = test_account_with_plan("pro");
        paid_account.id = "account-paid".to_string();
        let mut free_account = test_account_with_plan("free");
        free_account.id = "account-free".to_string();
        let collection = test_local_access_collection(vec![paid_account.id.clone()]);
        let api_key = ResolvedLocalApiKey {
            id: "key-scoped".to_string(),
            label: "Scoped".to_string(),
            provider_gateway: None,
            inherit_account_pool: false,
            account_ids: vec![free_account.id.clone()],
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };

        let models = visible_codex_model_ids_for_api_key_with_accounts(
            &collection,
            &api_key,
            &[paid_account, free_account],
            None,
        );

        assert!(!models.iter().any(|model| model == CODEX_IMAGE_MODEL_ID));
    }

    #[test]
    fn sidecar_fingerprint_ignores_remaining_quota() {
        let config = r#"{"host":"127.0.0.1","port":58393}"#;
        let manifest_a = r#"{
          "accounts": [
            {"id": "account-a", "email": "a@example.com", "remainingQuota": 10, "planRank": 2}
          ],
          "routingStrategy": "auto"
        }"#;
        let manifest_b = r#"{
          "accounts": [
            {"id": "account-a", "email": "a@example.com", "remainingQuota": 90, "planRank": 2}
          ],
          "routingStrategy": "auto"
        }"#;
        let manifest_c = r#"{
          "accounts": [
            {"id": "account-a", "email": "a@example.com", "remainingQuota": 90, "planRank": 3}
          ],
          "routingStrategy": "auto"
        }"#;

        assert_eq!(
            sidecar_config_fingerprint(config, manifest_a),
            sidecar_config_fingerprint(config, manifest_b)
        );
        assert_ne!(
            sidecar_config_fingerprint(config, manifest_b),
            sidecar_config_fingerprint(config, manifest_c)
        );
    }

    #[test]
    fn sidecar_fingerprint_ignores_token_usage_but_tracks_token_limit() {
        let config = r#"{"host":"127.0.0.1","port":58393}"#;
        let manifest_a = r#"{"apiKeys":[{"id":"key-1","tokenLimit":1000,"tokenUsed":100}]}"#;
        let manifest_b = r#"{"apiKeys":[{"id":"key-1","tokenLimit":1000,"tokenUsed":900}]}"#;
        let manifest_c = r#"{"apiKeys":[{"id":"key-1","tokenLimit":2000,"tokenUsed":900}]}"#;

        assert_eq!(
            sidecar_config_fingerprint(config, manifest_a),
            sidecar_config_fingerprint(config, manifest_b),
            "usage updates must not restart active sidecar streams"
        );
        assert_ne!(
            sidecar_config_fingerprint(config, manifest_a),
            sidecar_config_fingerprint(config, manifest_c),
            "limit changes must restart the sidecar to apply the new policy"
        );
    }

    #[test]
    fn sidecar_fingerprint_ignores_hot_reloadable_payload_defaults() {
        let manifest = r#"{"accounts":[],"routingStrategy":"auto"}"#;
        let standard = r#"{"host":"127.0.0.1","port":58393}"#;
        let priority = r#"{
          "host": "127.0.0.1",
          "port": 58393,
          "payload": {"default":[{"models":[{"name":"gpt-*","protocol":"openai/responses"}],"params":{"service_tier":"priority"}}]}
        }"#;
        let other_port = r#"{
          "host": "127.0.0.1",
          "port": 58394,
          "payload": {"default":[{"models":[{"name":"gpt-*","protocol":"openai/responses"}],"params":{"service_tier":"priority"}}]}
        }"#;

        assert_eq!(
            sidecar_config_fingerprint(standard, manifest),
            sidecar_config_fingerprint(priority, manifest),
        );
        assert_ne!(
            sidecar_config_fingerprint(priority, manifest),
            sidecar_config_fingerprint(other_port, manifest),
        );
    }

    #[test]
    fn sidecar_fingerprint_ignores_dynamic_quota_snapshot_changes() {
        let config = r#"{"host":"127.0.0.1","port":58393}"#;
        let manifest_available_high = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 10,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 100,
              "hourlyRemainingPercent": 50,
              "weeklyRemainingPercent": 60,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;
        let manifest_available_low = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 10,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 100,
              "hourlyRemainingPercent": 11,
              "weeklyRemainingPercent": 21,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;
        let manifest_new_snapshot = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 10,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 200,
              "hourlyRemainingPercent": 11,
              "weeklyRemainingPercent": 21,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;
        let manifest_blocked = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 10,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 100,
              "hourlyRemainingPercent": 10,
              "weeklyRemainingPercent": 21,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;
        let manifest_threshold_changed = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 15,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 100,
              "hourlyRemainingPercent": 50,
              "weeklyRemainingPercent": 60,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;

        assert_eq!(
            sidecar_config_fingerprint(config, manifest_available_high),
            sidecar_config_fingerprint(config, manifest_available_low)
        );
        assert_eq!(
            sidecar_config_fingerprint(config, manifest_available_low),
            sidecar_config_fingerprint(config, manifest_new_snapshot)
        );
        assert_eq!(
            sidecar_config_fingerprint(config, manifest_available_low),
            sidecar_config_fingerprint(config, manifest_blocked)
        );
        assert_ne!(
            sidecar_config_fingerprint(config, manifest_available_high),
            sidecar_config_fingerprint(config, manifest_threshold_changed)
        );
    }

    #[test]
    fn bound_oauth_quota_reserve_blocks_at_threshold_and_fails_closed() {
        let reserve = CodexLocalAccessQuotaReserve {
            hourly_percent: 20,
            weekly_percent: 10,
        };
        let blocked =
            test_oauth_account_with_quota("account-bound", 20, 90, Some(true), Some(true));
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&blocked)
        ));

        let available =
            test_oauth_account_with_quota("account-bound", 21, 11, Some(true), Some(true));
        assert!(!bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&available)
        ));

        let ignored_hourly =
            test_oauth_account_with_quota("account-bound", 0, 11, Some(false), Some(true));
        assert!(!bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&ignored_hourly)
        ));
        assert!(bound_oauth_quota_reserve_blocks_account(&reserve, None));

        let mut quota_error = available;
        quota_error.quota_error = Some(CodexQuotaErrorInfo {
            code: Some("quota_refresh_failed".to_string()),
            message: "refresh failed".to_string(),
            timestamp: 1,
        });
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&quota_error)
        ));

        let mut stale =
            test_oauth_account_with_quota("account-stale", 80, 80, Some(true), Some(true));
        stale.usage_updated_at = Some(
            chrono::Utc::now().timestamp() - BOUND_OAUTH_QUOTA_RESERVE_MAX_SNAPSHOT_AGE_SECONDS - 1,
        );
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&stale)
        ));

        let mut missing_timestamp =
            test_oauth_account_with_quota("account-missing", 80, 80, Some(true), Some(true));
        missing_timestamp.usage_updated_at = None;
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&missing_timestamp)
        ));

        let mut future_timestamp =
            test_oauth_account_with_quota("account-future", 80, 80, Some(true), Some(true));
        future_timestamp.usage_updated_at = Some(chrono::Utc::now().timestamp() + 60);
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&future_timestamp)
        ));

        let transient =
            test_oauth_account_with_quota("account-transient", 80, 80, Some(true), Some(true));
        bound_oauth_quota_refresh_failures()
            .lock()
            .unwrap()
            .insert(transient.id.clone());
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&transient)
        ));
        bound_oauth_quota_refresh_failures()
            .lock()
            .unwrap()
            .remove(&transient.id);
    }

    #[test]
    fn bound_oauth_quota_reserve_filters_only_the_bound_account() {
        let reserve = CodexLocalAccessQuotaReserve {
            hourly_percent: 20,
            weekly_percent: 10,
        };
        let blocked =
            test_oauth_account_with_quota("account-bound", 20, 90, Some(true), Some(true));
        let scoped = vec!["account-bound".to_string(), "account-other".to_string()];

        assert_eq!(
            filter_bound_oauth_quota_reserve_account(
                scoped,
                "account-bound",
                &reserve,
                Some(&blocked),
            ),
            vec!["account-other"]
        );
    }

    #[test]
    fn sidecar_manifest_keeps_thresholds_and_snapshot_state_is_separate() {
        let mut collection = test_local_access_collection(vec!["account-bound".to_string()]);
        collection.bound_oauth_account_id = Some("account-bound".to_string());
        collection.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
            hourly_percent: 20,
            weekly_percent: 10,
        });
        let account =
            test_oauth_account_with_quota("account-bound", 75, 40, Some(true), Some(false));

        let manifest = sidecar_account_manifest_value(&account, Some("auth.json"), &collection);
        let reserve = manifest
            .get("quotaReserve")
            .expect("quota reserve should exist");
        assert_eq!(reserve["hourlyThresholdPercent"], json!(20));
        assert_eq!(reserve["weeklyThresholdPercent"], json!(10));
        assert!(reserve.get("snapshotUpdatedAtUnixSeconds").is_none());
        assert!(reserve.get("hourlyRemainingPercent").is_none());

        let snapshot = sidecar_quota_reserve_snapshot_value(&collection, &account)
            .expect("quota reserve snapshot should exist");
        assert_eq!(
            snapshot["snapshotUpdatedAtUnixSeconds"],
            json!(account.usage_updated_at)
        );
        assert_eq!(snapshot["hourlyRemainingPercent"], json!(75));
        assert_eq!(snapshot["weeklyRemainingPercent"], json!(40));
        assert_eq!(snapshot["hourlyWindowPresent"], json!(true));
        assert_eq!(snapshot["weeklyWindowPresent"], json!(false));
    }

    #[test]
    fn sidecar_fingerprint_ignores_all_dynamic_reserve_states() {
        let config = r#"{"host":"127.0.0.1","port":58393}"#;
        let manifest = |remaining: Value| {
            json!({
                "accounts": [{
                    "id": "account-a",
                    "quotaReserve": {
                        "hourlyThresholdPercent": 20,
                        "weeklyThresholdPercent": 10,
                        "hourlyRemainingPercent": remaining,
                        "weeklyRemainingPercent": 80,
                        "hourlyWindowPresent": true,
                        "weeklyWindowPresent": true
                    }
                }]
            })
            .to_string()
        };
        let available_80 = manifest(json!(80));
        let available_70 = manifest(json!(70));
        let blocked_20 = manifest(json!(20));
        let unknown = manifest(Value::Null);

        assert_eq!(
            sidecar_config_fingerprint(config, &available_80),
            sidecar_config_fingerprint(config, &available_70)
        );
        assert_eq!(
            sidecar_config_fingerprint(config, &available_70),
            sidecar_config_fingerprint(config, &blocked_20)
        );
        assert_eq!(
            sidecar_config_fingerprint(config, &blocked_20),
            sidecar_config_fingerprint(config, &unknown)
        );
    }

    #[test]
    fn account_model_rules_block_matching_model_only() {
        let mut collection =
            test_local_access_collection(vec!["account-a".to_string(), "account-b".to_string()]);
        collection.account_model_rules = vec![CodexLocalAccessAccountModelRule {
            account_id: "account-a".to_string(),
            excluded_models: vec!["gpt-5.4-*".to_string()],
        }];

        assert!(account_model_rule_blocks_model(
            &collection,
            "account-a",
            "gpt-5.4-mini"
        ));
        assert!(!account_model_rule_blocks_model(
            &collection,
            "account-a",
            "gpt-5.3-codex"
        ));
        assert!(!account_model_rule_blocks_model(
            &collection,
            "account-b",
            "gpt-5.4-mini"
        ));
    }

    #[test]
    fn sidecar_account_excluded_models_merge_global_and_account_rules() {
        let mut collection = test_local_access_collection(vec!["account-a".to_string()]);
        collection.excluded_models = vec!["gpt-5.2".to_string()];
        collection.account_model_rules = vec![CodexLocalAccessAccountModelRule {
            account_id: "account-a".to_string(),
            excluded_models: vec!["gpt-5.4-mini".to_string(), "GPT-5.4-MINI".to_string()],
        }];

        assert_eq!(
            merge_collection_and_account_excluded_models(&collection, "account-a"),
            vec!["gpt-5.2".to_string(), "gpt-5.4-mini".to_string()]
        );
    }

    #[test]
    fn gpt_reserve_entitlement_requires_an_allowed_additional_rate_limit() {
        let mut account = test_account_with_plan("pro");
        account.quota = Some(CodexQuota {
            hourly_percentage: 100,
            hourly_reset_time: None,
            hourly_window_minutes: Some(300),
            hourly_window_present: Some(true),
            weekly_percentage: 100,
            weekly_reset_time: None,
            weekly_window_minutes: Some(10_080),
            weekly_window_present: Some(true),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: Some(json!({
                "additional_rate_limits": [{
                    "limit_name": "GPT-Reserve",
                    "rate_limit": { "allowed": true }
                }],
                "rate_limit": { "allowed": false },
                "rate_limit_upsell": { "banner_type": "luna_reserve" }
            })),
        });
        assert!(account_has_gpt_reserve_entitlement(&account));
        let valid = account.quota.as_ref().unwrap().raw_data.clone().unwrap();
        for (regular_allowed, reserve_allowed, banner) in [
            (true, true, "luna_reserve"),
            (false, false, "luna_reserve"),
            (false, true, "upgrade"),
        ] {
            let mut raw = valid.clone();
            raw["rate_limit"]["allowed"] = json!(regular_allowed);
            raw["additional_rate_limits"][0]["rate_limit"]["allowed"] = json!(reserve_allowed);
            raw["rate_limit_upsell"]["banner_type"] = json!(banner);
            account.quota.as_mut().unwrap().raw_data = Some(raw);
            assert!(!account_has_gpt_reserve_entitlement(&account),
                "must reject regular={regular_allowed}, reserve={reserve_allowed}, banner={banner}");
        }
        account.quota.as_mut().unwrap().raw_data = Some(valid.clone());
        account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
        assert!(!account_has_gpt_reserve_entitlement(&account));
        account.quota.as_mut().unwrap().raw_data = None;
        assert!(!account_has_gpt_reserve_entitlement(&account));
    }

    #[test]
    fn api_service_model_list_keeps_reserve_without_eligible_accounts() {
        let mut entitled = test_account_with_plan("pro");
        entitled.id = "reserve-entitled".to_string();
        entitled.quota = Some(CodexQuota {
            hourly_percentage: 100,
            hourly_reset_time: None,
            hourly_window_minutes: Some(300),
            hourly_window_present: Some(true),
            weekly_percentage: 100,
            weekly_reset_time: None,
            weekly_window_minutes: Some(10_080),
            weekly_window_present: Some(true),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: Some(json!({
                "additional_rate_limits": [{
                    "limit_name": "GPT reserve",
                    "rate_limit": { "allowed": true }
                }],
                "rate_limit": { "allowed": false },
                "rate_limit_upsell": { "banner_type": "luna_reserve" }
            })),
        });
        let mut ordinary = entitled.clone();
        ordinary.id = "reserve-ordinary".to_string();
        ordinary.quota.as_mut().unwrap().raw_data = Some(json!({
            "additional_rate_limits": []
        }));

        let mut collection = test_local_access_collection(vec![entitled.id.clone()]);
        let api_key = ResolvedLocalApiKey {
            id: "reserve-key".to_string(),
            label: "Reserve".to_string(),
            provider_gateway: None,
            inherit_account_pool: true,
            account_ids: Vec::new(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };
        let entitled_models = visible_codex_model_ids_for_api_key_with_accounts(
            &collection,
            &api_key,
            &[entitled.clone(), ordinary.clone()],
            None,
        );
        assert!(entitled_models
            .iter()
            .any(|model| model.eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID)));

        collection.account_ids = vec![ordinary.id.clone()];
        let ordinary_models = visible_codex_model_ids_for_api_key_with_accounts(
            &collection,
            &api_key,
            &[entitled, ordinary],
            None,
        );
        assert!(ordinary_models
            .iter()
            .any(|model| model.eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID)));
    }

    #[test]
    fn scoped_api_key_pool_discovers_spark_entitlement_from_effective_accounts() {
        let mut plus = test_account_with_plan("plus");
        plus.id = "scoped-plus".to_string();
        plus.quota = Some(CodexQuota {
            hourly_percentage: 100,
            hourly_reset_time: None,
            hourly_window_minutes: Some(300),
            hourly_window_present: Some(true),
            weekly_percentage: 100,
            weekly_reset_time: None,
            weekly_window_minutes: Some(10_080),
            weekly_window_present: Some(true),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: Some(json!({ "additional_rate_limits": [] })),
        });

        let mut pro = test_account_with_plan("pro");
        pro.id = "scoped-pro".to_string();
        pro.quota = Some(CodexQuota {
            hourly_percentage: 100,
            hourly_reset_time: None,
            hourly_window_minutes: Some(300),
            hourly_window_present: Some(true),
            weekly_percentage: 100,
            weekly_reset_time: None,
            weekly_window_minutes: Some(10_080),
            weekly_window_present: Some(true),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: Some(json!({
                "additional_rate_limits": [{
                    "limit_name": "GPT-5.3-Codex-Spark",
                    "metered_feature": "codex_spark",
                    "rate_limit": { "allowed": true }
                }]
            })),
        });

        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key.clear();
        let mut api_key = build_local_access_api_key(Some("Scoped Spark"));
        api_key.key = "scoped-spark-key".to_string();
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec![plus.id.clone(), pro.id.clone()];
        collection.api_keys = vec![api_key];

        let dir = make_temp_dir("codex-scoped-spark-entitlement");
        let overrides = HashMap::from([(plus.id.clone(), plus.clone()), (pro.id.clone(), pro)]);
        super::prepare_sidecar_launch_config_in_dir_sync(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            overrides,
            true,
            None,
        )
        .expect("prepare scoped sidecar config");

        let auth_path = sidecar_auths_dir(&dir).join(sidecar_auth_file_name(&plus.id));
        let auth: Value =
            serde_json::from_str(&fs::read_to_string(auth_path).expect("read scoped Plus auth"))
                .expect("parse scoped Plus auth");
        let excluded = auth
            .get("excluded_models")
            .and_then(Value::as_array)
            .expect("excluded_models should be an array");
        assert!(
            excluded
                .iter()
                .filter_map(Value::as_str)
                .any(|model| model.eq_ignore_ascii_case("gpt-5.3-codex-spark")),
            "Plus account should exclude Spark discovered from its scoped Pro peer"
        );

        fs::remove_dir_all(dir).expect("cleanup scoped sidecar config");
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        for _ in 0..10 {
            let dir = std::env::temp_dir().join(format!(
                "{}-{}-{}",
                prefix,
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            if fs::create_dir(&dir).is_ok() {
                return dir;
            }
        }
        panic!("create temp dir failed");
    }

    struct LocalAccessTestDataGuard {
        data_dir: PathBuf,
        previous_test_data_dir: Option<String>,
        previous_data_dir: Option<String>,
        takeover_backup_path: PathBuf,
        previous_takeover_backup: Option<Vec<u8>>,
    }

    impl LocalAccessTestDataGuard {
        fn new(prefix: &str) -> Self {
            let data_dir = make_temp_dir(prefix);
            let previous_test_data_dir = std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR").ok();
            let previous_data_dir = std::env::var("COCKPIT_TOOLS_DATA_DIR").ok();
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &data_dir);

            let takeover_backup_path =
                super::local_access_takeover_backups_path().expect("resolve takeover backup path");
            let previous_takeover_backup = fs::read(&takeover_backup_path).ok();
            if takeover_backup_path.exists() {
                fs::remove_file(&takeover_backup_path).expect("clear takeover backup for test");
            }

            Self {
                data_dir,
                previous_test_data_dir,
                previous_data_dir,
                takeover_backup_path,
                previous_takeover_backup,
            }
        }
    }

    impl Drop for LocalAccessTestDataGuard {
        fn drop(&mut self) {
            match self.previous_test_data_dir.as_deref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
            }
            match self.previous_data_dir.as_deref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
            }
            match self.previous_takeover_backup.as_deref() {
                Some(content) => {
                    if let Some(parent) = self.takeover_backup_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&self.takeover_backup_path, content);
                }
                None => {
                    let _ = fs::remove_file(&self.takeover_backup_path);
                }
            }
            let _ = fs::remove_dir_all(&self.data_dir);
        }
    }

    fn test_account_with_plan(plan_type: &str) -> CodexAccount {
        let mut account = CodexAccount::new(
            format!("acc-{}", plan_type),
            format!("{}@example.com", plan_type),
            CodexTokens {
                id_token: String::new(),
                access_token: "access-token".to_string(),
                refresh_token: None,
            },
        );
        account.plan_type = Some(plan_type.to_string());
        account
    }

    #[test]
    fn append_local_access_accounts_is_incremental_and_reports_skips() {
        let existing = test_account_with_plan("plus");
        let mut added = test_account_with_plan("team");
        added.id = "added".to_string();
        let free = test_account_with_plan("free");
        let mut chat = CodexAccount::new_api_key(
            "chat".to_string(),
            "chat@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://example.com/v1".to_string()),
            None,
            None,
            Vec::new(),
        );
        chat.api_wire_api = Some("chat_completions".to_string());

        let (next_ids, synced_ids, added_ids, skipped) = append_eligible_local_access_account_ids(
            &[existing.id.clone(), "preserved".to_string()],
            vec![
                existing.id.clone(),
                "added".to_string(),
                "added".to_string(),
                free.id.clone(),
                "chat".to_string(),
                "missing".to_string(),
            ],
            &[existing.clone(), added, free.clone(), chat],
            true,
        );

        assert_eq!(
            next_ids,
            vec![
                existing.id.clone(),
                "preserved".to_string(),
                "added".to_string()
            ]
        );
        assert_eq!(synced_ids, vec![existing.id, "added".to_string()]);
        assert_eq!(added_ids, vec!["added".to_string()]);
        assert_eq!(
            skipped
                .iter()
                .map(|item| (item.account_id.as_str(), item.reason.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (free.id.as_str(), "free_restricted"),
                ("chat", "chat_completions_api_key"),
                ("missing", "not_found"),
            ]
        );
    }

    #[test]
    fn pending_oauth_accounts_are_not_eligible_for_local_access_pool() {
        let mut pending = test_account_with_plan("plus");
        pending.id = "pending".to_string();
        pending.authorization_status = Some("pending".to_string());
        pending.tokens.access_token = String::new();
        pending.tokens.id_token = String::new();
        pending.tokens.refresh_token = None;

        assert!(!is_local_access_eligible_account(&pending, true));
        assert_eq!(
            local_access_ineligible_reason(&pending, true),
            Some("pending_oauth")
        );

        let (next_ids, synced_ids, added_ids, skipped) = append_eligible_local_access_account_ids(
            &[],
            vec![pending.id.clone()],
            &[pending.clone()],
            true,
        );
        assert!(next_ids.is_empty());
        assert!(synced_ids.is_empty());
        assert!(added_ids.is_empty());
        assert_eq!(
            skipped
                .iter()
                .map(|item| (item.account_id.as_str(), item.reason.as_str()))
                .collect::<Vec<_>>(),
            vec![("pending", "pending_oauth")]
        );
    }

    #[test]
    fn free_agent_identity_remains_eligible_for_local_access_pool() {
        let mut account = test_account_with_plan("free");
        account.tokens.access_token.clear();
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-free-agent".to_string(),
            agent_private_key: "private-key".to_string(),
            task_id: Some("task-free-agent".to_string()),
            account_id: "account-free-agent".to_string(),
            chatgpt_user_id: "user-free-agent".to_string(),
            email: Some("free-agent@example.com".to_string()),
            plan_type: Some("free".to_string()),
            chatgpt_account_is_fedramp: false,
        });

        assert!(is_local_access_eligible_account(&account, true));
        let (_, synced_ids, added_ids, skipped) = append_eligible_local_access_account_ids(
            &[],
            vec![account.id.clone()],
            &[account.clone()],
            true,
        );
        assert_eq!(synced_ids, vec![account.id.clone()]);
        assert_eq!(added_ids, vec![account.id]);
        assert!(skipped.is_empty());
    }

    #[test]
    fn agent_identity_cannot_be_used_as_local_access_oauth_binding() {
        let mut account = test_account_with_plan("plus");
        account.tokens.refresh_token = Some("refresh-token".to_string());
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-oauth-binding".to_string(),
            agent_private_key: "private-key".to_string(),
            task_id: Some("task-oauth-binding".to_string()),
            account_id: "account-oauth-binding".to_string(),
            chatgpt_user_id: "user-oauth-binding".to_string(),
            email: Some("agent-oauth-binding@example.com".to_string()),
            plan_type: Some("plus".to_string()),
            chatgpt_account_is_fedramp: false,
        });

        let error = validate_loaded_local_access_bound_oauth_account(account)
            .expect_err("Agent Identity must not be accepted as an OAuth binding");

        assert!(error.contains("不能作为 OAuth 绑定账号"));
    }

    fn make_test_jwt(payload: Value) -> String {
        let header = json!({ "alg": "none", "typ": "JWT" });
        format!(
            "{}.{}.sig",
            general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&header).expect("serialize jwt header")),
            general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&payload).expect("serialize jwt payload"))
        )
    }

    #[test]
    fn write_string_atomic_if_changed_skips_identical_content() {
        let dir = make_temp_dir("codex-sidecar-write-if-changed");
        let path = dir.join("auth.json");

        assert!(write_string_atomic_if_changed(&path, "{\"token\":\"a\"}")
            .expect("initial write should succeed"));
        assert!(!write_string_atomic_if_changed(&path, "{\"token\":\"a\"}")
            .expect("unchanged write should succeed"));
        assert!(write_string_atomic_if_changed(&path, "{\"token\":\"b\"}")
            .expect("changed write should succeed"));
        assert_eq!(
            fs::read_to_string(&path).expect("read updated content"),
            "{\"token\":\"b\"}"
        );

        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn sidecar_oauth_auth_json_includes_access_token_expiry() {
        let account = CodexAccount::new(
            "account-exp".to_string(),
            "exp@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: make_test_jwt(json!({
                    "sub": "access-exp",
                    "exp": 4_102_444_800i64,
                })),
                refresh_token: Some("refresh-token".to_string()),
            },
        );
        let mut collection = test_local_access_collection(vec![account.id.clone()]);

        let auth_json = sidecar_auth_json_for_account(&account, &collection, None);

        assert_eq!(
            auth_json.get("last_refresh").and_then(Value::as_str),
            account
                .token_updated_at
                .map(|value| value.to_string())
                .as_deref()
        );
        assert_eq!(
            auth_json.get("expired").and_then(Value::as_i64),
            Some(4_102_444_800i64)
        );
        assert_eq!(
            auth_json.get("refresh_token").and_then(Value::as_str),
            Some("")
        );
        assert_eq!(
            auth_json.get("refresh_owner").and_then(Value::as_str),
            Some("cockpit_token_authority")
        );
        assert_eq!(
            auth_json.get("websockets").and_then(Value::as_bool),
            Some(false)
        );

        collection.responses_websockets_enabled = true;
        let enabled_auth_json = sidecar_auth_json_for_account(&account, &collection, None);
        assert_eq!(
            enabled_auth_json.get("websockets").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn provider_gateway_runtime_auth_sync_rewrites_access_token_without_refresh_token() {
        let sidecar_dir = make_temp_dir("codex-provider-runtime-auth-sync");
        let auths_dir = sidecar_auths_dir(&sidecar_dir);
        fs::create_dir_all(&auths_dir).expect("create auths dir");
        let mut account = test_account_with_plan("plus");
        account.tokens.access_token = "access-before".to_string();
        account.tokens.refresh_token = Some("refresh-must-not-leak".to_string());
        let collection = test_local_access_collection(vec![account.id.clone()]);
        let auth_path = auths_dir.join(sidecar_auth_file_name(&account.id));
        fs::write(&auth_path, "{}").expect("write existing auth file");

        account.tokens.access_token = "access-after".to_string();
        assert!(
            sync_provider_gateway_runtime_auth_file(&account, &collection, &sidecar_dir)
                .expect("sync provider runtime auth")
        );

        let auth: Value = serde_json::from_str(
            &fs::read_to_string(&auth_path).expect("read provider runtime auth"),
        )
        .expect("parse provider runtime auth");
        assert_eq!(
            auth.get("access_token").and_then(Value::as_str),
            Some("access-after")
        );
        assert_eq!(auth.get("refresh_token").and_then(Value::as_str), Some(""));
        assert_eq!(
            auth.get("refresh_owner").and_then(Value::as_str),
            Some("cockpit_token_authority")
        );

        fs::remove_dir_all(sidecar_dir).expect("cleanup provider runtime auth sync");
    }

    #[test]
    fn sidecar_agent_identity_auth_json_preserves_signing_credentials_without_tokens() {
        let mut account = CodexAccount::new(
            "agent-account".to_string(),
            "agent@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-test".to_string(),
            agent_private_key: "private-key-test".to_string(),
            task_id: Some("task-test".to_string()),
            account_id: "team-test".to_string(),
            chatgpt_user_id: "user-test".to_string(),
            email: Some(account.email.clone()),
            plan_type: Some("plus".to_string()),
            chatgpt_account_is_fedramp: false,
        });
        account.account_id = Some("team-test".to_string());
        account.plan_type = Some("plus".to_string());
        let collection = test_local_access_collection(vec![account.id.clone()]);

        let auth_json = sidecar_auth_json_for_account(&account, &collection, None);
        assert_eq!(
            auth_json.get("auth_mode").and_then(Value::as_str),
            Some("agentIdentity")
        );
        assert_eq!(
            auth_json.get("agent_runtime_id").and_then(Value::as_str),
            Some("runtime-test")
        );
        assert_eq!(
            auth_json.get("task_id").and_then(Value::as_str),
            Some("task-test")
        );
        assert!(auth_json.get("access_token").is_none());
        assert!(sidecar_local_account_usable_for_start(&account));

        let manifest =
            sidecar_account_manifest_value(&account, Some("agent-account.json"), &collection);
        assert_eq!(
            manifest.get("authKind").and_then(Value::as_str),
            Some("agent_identity")
        );
    }

    #[test]
    fn sidecar_recovered_agent_identity_task_is_adopted_only_for_matching_credentials() {
        let dir = make_temp_dir("codex-sidecar-agent-task-adoption");
        let auth_path = dir.join("agent.json");
        let mut account = CodexAccount::new(
            "agent-task-adoption".to_string(),
            "agent@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-test".to_string(),
            agent_private_key: "private-key-test".to_string(),
            task_id: Some("task-old".to_string()),
            account_id: "team-test".to_string(),
            chatgpt_user_id: "user-test".to_string(),
            email: Some(account.email.clone()),
            plan_type: Some("k12".to_string()),
            chatgpt_account_is_fedramp: false,
        });
        fs::write(
            &auth_path,
            serde_json::to_vec(&json!({
                "auth_mode": "agentIdentity",
                "agent_runtime_id": "runtime-test",
                "agent_private_key": "private-key-test",
                "task_id": "task-recovered",
            }))
            .expect("serialize auth"),
        )
        .expect("write auth");

        assert!(
            super::adopt_sidecar_agent_identity_task(&mut account, &auth_path)
                .expect("adopt recovered task")
        );
        assert_eq!(
            account
                .agent_identity
                .as_ref()
                .and_then(|identity| identity.task_id.as_deref()),
            Some("task-recovered")
        );

        account
            .agent_identity
            .as_mut()
            .expect("identity")
            .agent_private_key = "different-private-key".to_string();
        account.agent_identity.as_mut().expect("identity").task_id =
            Some("task-current".to_string());
        assert!(
            !super::adopt_sidecar_agent_identity_task(&mut account, &auth_path)
                .expect("reject mismatched credentials")
        );
        assert_eq!(
            account
                .agent_identity
                .as_ref()
                .and_then(|identity| identity.task_id.as_deref()),
            Some("task-current")
        );

        fs::remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn build_runtime_account_enables_websockets_for_local_api_service() {
        let account = build_runtime_account(
            "http://127.0.0.1:1455/v1".to_string(),
            "agt_codex_test".to_string(),
            None,
            true,
        );
        assert!(account.api_supports_websockets);
        assert_eq!(account.api_wire_api.as_deref(), Some("responses"));
    }

    #[test]
    fn provider_gateway_api_key_disables_profile_websockets() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_keys.push(CodexLocalAccessApiKey {
            id: "provider_gateway_deepseek".to_string(),
            label: "Provider Gateway: DeepSeek".to_string(),
            key: "deepseek-local-key".to_string(),
            provider_gateway: Some(CodexLocalAccessProviderGateway {
                base_url: "https://api.deepseek.com/v1".to_string(),
                api_key: "sk-deepseek".to_string(),
                upstream_model: "deepseek-v4-pro".to_string(),
                upstream_models: vec!["deepseek-v4-pro".to_string()],
                wire_api: Some("chat_completions".to_string()),
                supports_vision: false,
            model_capabilities: HashMap::new(),
            vision_routing_model: None,
        }),
            model_routing: None,
            inherit_account_pool: Some(false),
            account_ids: vec!["deepseek-account".to_string()],
            priority_account_ids: Vec::new(),
            preferred_account_id: None,
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
            enabled: true,
            created_at: 0,
            updated_at: 0,
            last_used_at: None,
        });
        assert!(!profile_api_key_supports_websockets(&collection, "deepseek-local-key"));
        assert!(!profile_api_key_supports_websockets(&collection, &collection.api_key));
        collection.responses_websockets_enabled = true;
        assert!(!profile_api_key_supports_websockets(&collection, "deepseek-local-key"));
        assert!(profile_api_key_supports_websockets(&collection, &collection.api_key));
    }

    #[test]
    fn mixed_model_routing_manifest_keeps_oauth_default_and_disables_websockets() {
        let mut collection = test_local_access_collection(vec!["oauth-account".to_string()]);
        collection.bound_oauth_account_id = Some("oauth-account".to_string());
        collection.responses_websockets_enabled = true;
        let mut api_key = build_local_access_api_key(Some("Mixed"));
        api_key.key = "mixed-local-key".to_string();
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec!["oauth-account".to_string()];
        api_key.provider_gateway = None;
        api_key.model_routing = Some(CodexLocalAccessModelRouting {
            default_route: "oauth".to_string(),
            failure_policy: "strict".to_string(),
            routes: vec![CodexLocalAccessModelRoute {
                id: "route-cpa".to_string(),
                namespace: "cpa".to_string(),
                provider_account_id: "cpa-account".to_string(),
                provider_gateway: CodexLocalAccessProviderGateway {
                    base_url: "https://cpa.example.com/v1".to_string(),
                    api_key: "sk-cpa".to_string(),
                    upstream_model: "gpt-5.5".to_string(),
                    upstream_models: vec!["gpt-5.5".to_string(), "grok-4.6".to_string()],
                    wire_api: Some("responses".to_string()),
                    supports_vision: false,
                    model_capabilities: HashMap::new(),
                    vision_routing_model: None,
                },
            }],
        });
        collection.api_keys = vec![api_key];

        let mixed = sidecar_api_key_manifest_values(&collection)
            .into_iter()
            .find(|value| value.get("key").and_then(Value::as_str) == Some("mixed-local-key"))
            .expect("mixed routing key should be emitted");
        assert!(mixed.get("providerGateway").and_then(Value::as_object).is_none());
        assert_eq!(
            mixed
                .pointer("/modelRouting/defaultRoute")
                .and_then(Value::as_str),
            Some("oauth")
        );
        assert_eq!(
            mixed
                .pointer("/modelRouting/failurePolicy")
                .and_then(Value::as_str),
            Some("strict")
        );
        assert_eq!(
            mixed
                .pointer("/modelRouting/routes/0/namespace")
                .and_then(Value::as_str),
            Some("cpa")
        );
        assert_eq!(
            mixed
                .get("responsesWebsockets")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert!(!profile_api_key_supports_websockets(
            &collection,
            "mixed-local-key"
        ));
    }
