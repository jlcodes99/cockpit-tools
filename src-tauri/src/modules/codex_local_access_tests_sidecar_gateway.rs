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
    fn deepseek_provider_model_slots_keep_upstream_ids() {
        let slots = super::allocate_provider_model_slots(&[
            "deepseek-v4-flash".to_string(),
            "deepseek-v4-pro".to_string(),
            "deepseek-v4-flash-vision-exp".to_string(),
        ]);
        assert_eq!(
            slots,
            vec![
                super::ProviderGatewayModelSlot {
                    client_model: "deepseek-v4-flash".to_string(),
                    upstream_model: "deepseek-v4-flash".to_string(),
                },
                super::ProviderGatewayModelSlot {
                    client_model: "deepseek-v4-pro".to_string(),
                    upstream_model: "deepseek-v4-pro".to_string(),
                },
                super::ProviderGatewayModelSlot {
                    client_model: "deepseek-v4-flash-vision-exp".to_string(),
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
            .get(&super::account_pool_route_key(&event))
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
    fn manual_recovery_suppresses_recovered_accounts_until_window_expires() {
        let mut runtime = super::GatewayRuntime::default();
        super::suppress_local_access_recovery_rows(
            &mut runtime,
            &["account-1".to_string(), " account-2 ".to_string()],
            2_000,
        );

        assert_eq!(
            runtime.recovery_suppressed_accounts.get("account-1"),
            Some(&2_000)
        );
        assert_eq!(
            runtime.recovery_suppressed_accounts.get("account-2"),
            Some(&2_000)
        );
        assert_eq!(runtime.recovery_suppressed_accounts.len(), 2);

        super::prune_runtime_routing_state(&mut runtime, 1_000);
        assert_eq!(runtime.recovery_suppressed_accounts.len(), 2);

        super::prune_runtime_routing_state(&mut runtime, 2_000);
        assert!(runtime.recovery_suppressed_accounts.is_empty());
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

    #[test]
    fn recovery_membership_removes_then_restores_selected_accounts() {
        let current = vec![
            "account-keep".to_string(),
            "account-recover".to_string(),
            "account-other".to_string(),
        ];
        let (selected, remaining, restored) = super::local_access_recovery_membership(
            &current,
            &[" account-recover ".to_string(), "missing".to_string()],
        )
        .expect("selected membership");

        assert_eq!(selected, vec!["account-recover".to_string()]);
        assert_eq!(
            remaining,
            vec!["account-keep".to_string(), "account-other".to_string()]
        );
        assert_eq!(restored, current);
        assert!(super::local_access_recovery_membership(
            &current,
            &["missing".to_string()]
        )
        .is_err());
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
    fn workspace_credits_keep_zero_window_accounts_out_of_quota_cooldown() {
        let now = 1_800_000_i64;
        let raw = serde_json::json!({
            "plan_type": "team",
            "spend_control": { "individual_limit": null, "reached": false },
            "credits": {
                "balance": null, "unlimited": false, "has_credits": true,
                "overage_limit_reached": false
            },
            "ambient_usage": { "default": { "source": { "kind": "credits" } } }
        });
        let account = oauth_account_with_quota(0, 0, Some(raw.clone()));
        assert!(!super::account_quota_cooldown(&account, now).unwrap().exhausted);
        assert_eq!(super::resolve_remaining_quota(&account), Some(1));
        assert!(super::account_quota_cooldown_at_floor(&account, now, 5).unwrap().exhausted);

        for (pointer, value) in [
            ("/plan_type", serde_json::json!("plus")),
            ("/spend_control/reached", serde_json::json!(true)),
            ("/credits/overage_limit_reached", serde_json::json!(true)),
            ("/credits/has_credits", serde_json::json!(false)),
            ("/credits/balance", serde_json::json!(0)),
        ] {
            let mut blocked = raw.clone();
            *blocked.pointer_mut(pointer).unwrap() = value;
            if matches!(pointer, "/spend_control/reached" | "/credits/overage_limit_reached") {
                blocked["credits"]["balance"] = serde_json::json!(100);
            }
            let account = oauth_account_with_quota(0, 0, Some(blocked));
            assert!(super::account_quota_cooldown(&account, now).unwrap().exhausted, "{pointer}");
        }
    }

    #[test]
    fn catalog_context_windows_keep_official_and_override_third_party() {
        let official = super::ProviderGatewayModelSlot {
            client_model: "gpt-6.1-sol".to_string(),
            upstream_model: "gpt-6.1-sol".to_string(),
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
                { "slug": "gpt-6.1-sol", "context_window": 272000, "max_context_window": 272000 },
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
        assert_eq!(window("gpt-6.1-sol"), Some(272000));
        assert_eq!(window("gpt-5.6-sol"), Some(900_000));
        assert_eq!(window("gpt-5.5"), Some(1048576));
        // 统一口径：显式写窗口时必须同时写 90% 的压缩阈值。
        let compact = |slug: &str| {
            parsed["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model["slug"] == slug)
                .and_then(|model| model["auto_compact_token_limit"].as_i64())
        };
        assert_eq!(compact("gpt-5.6-sol"), Some(810_000));
        // 官方 DeepSeek 壳位模型保留目录原值，不会被兜底窗口覆盖。
        assert_eq!(compact("gpt-5.5"), None);
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
        // 统一口径：显式覆盖窗口时同步写入 90% 压缩阈值。
        let compact = |slug: &str| {
            decorated["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model["slug"] == slug)
                .and_then(|model| model["auto_compact_token_limit"].as_i64())
        };
        assert_eq!(compact("gpt-5.6-sol"), Some(810_000));
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
    fn model_scope_error_translations_preserve_model_placeholder_for_all_locales() {
        let value = super::sidecar_localized_messages(
            "codex.localAccess.gatewayErrors.modelNotAvailable",
        );
        let messages = value.as_object().expect("localized error messages");
        assert_eq!(messages.len(), 18);
        for (locale, message) in messages {
            let message = message.as_str().expect("message text");
            assert_eq!(message.matches("{{model}}").count(), 1, "{locale}");
            if locale != "en" && locale != "en-us" {
                assert_ne!(Some(message), messages["en"].as_str(), "{locale}");
            }
        }
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
        empty_stats_snapshot, extract_usage_capture,
        filter_bound_oauth_quota_reserve_account,
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
        normalize_custom_routing_rules, new_empty_local_access_collection,
        normalized_sidecar_error_category,
        open_local_access_logs_db_once, parse_codex_retry_after,
        parse_responses_payload_from_upstream, parse_websocket_upstream_error,
        pin_account_to_front_for_strategy, prepare_gateway_request,
        prepare_gateway_request_with_default_service_tier, prepare_sidecar_launch_config_in_dir,
        prepare_websocket_initial_request, profile_api_key_supports_websockets,
        profile_config_path,
        profile_base_url_matches, provider_gateway_api_key_id,
        provider_gateway_bound_oauth_account_id_for_account,
        provider_gateway_default_model_for_account,
        provider_gateway_image_generation_mode_for_account, provider_gateway_model_slots,
        provider_gateway_models_for_account, provider_model_slots_need_upstream_rewrite,
        read_http_request, read_request_log_reprice_batch, recompute_time_windows,
        reapply_deepseek_profile_config_overrides, recover_invalid_stats_file,
        remove_account_refs_from_collection,
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
        sidecar_config_fingerprint_with_account_proxies, account_proxy_fingerprint,
        sidecar_local_account_usable_for_start, sidecar_payload_default_service_tier,
        sidecar_routing_strategy_value, sidecar_stable_id,
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
        CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID, CODEX_PROVIDER_MODEL_BACKUP_FILE,
        CODEX_PROVIDER_MODEL_CATALOG_FILE,
        DEFAULT_ACCOUNT_CONCURRENCY_WAIT_MS, DEFAULT_MAX_RETRY_INTERVAL_MS,
        DEFAULT_MODEL_PRICING_VERSION,
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
    include!("codex_local_access_tests_env_ordering.rs");

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
        const ISOLATED_MOCK: &str = "COCKPIT_TEST_AGENT_WAKEUP_ISOLATED";
        if std::env::var_os(ISOLATED_MOCK).is_none() {
            // This mock must use its loopback server even when the developer
            // has an application, environment or system proxy configured.
            let data_dir = make_temp_dir("agent-wakeup-isolated");
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"] {
                command.env_remove(key);
            }
            for key in ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR", "COCKPIT_TEST_DATA_DIR"] {
                command.env(key, &data_dir);
            }
            let output = command
                .env(ISOLATED_MOCK, "1")
                .env("NO_PROXY", "*")
                .env("no_proxy", "*")
                .args(["--exact", "modules::codex_local_access::tests::official_wakeup_agent_identity_recovers_invalid_task_once", "--nocapture", "--test-threads=1"])
                .output()
                .expect("run wakeup mock in isolated proxy environment");
            fs::remove_dir_all(data_dir).expect("remove wakeup mock data");
            assert!(output.status.success(), "isolated wakeup mock failed: {}{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"), "wakeup assertion test must execute");
            return;
        }
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
            workbuddy_delegate_revoked_key_ids: Vec::new(),
            account_pools: Vec::new(),
            account_pool_revision: 0,
            access_scope: CodexLocalAccessScope::Localhost,
            client_base_url_host: CodexLocalAccessClientBaseUrlHost::default(),
            image_generation_mode: CodexLocalAccessImageGenerationMode::default(),
            // This fixture represents an existing service that selected 2.5.
            image_generation_model: "gpt-image-2.5".to_string(),
        image_generation_models: crate::models::codex_local_access::default_image_generation_models(),
        image_generation_deleted_models: Vec::new(),
        image_generation_model_catalog_revision: 0,
            image_generation_main_model: None,
            image_generation_account_policies: HashMap::new(),
            chatgpt_plan_codex_image_policies: HashMap::new(),
            chatgpt_plan_service_revision: 0,
            paused_account_ids: Vec::new(),
            official_account_source_ids: HashMap::new(),
            official_account_bindings: HashMap::new(),
        service_account_identity_fingerprints: HashMap::new(),
            image_generation_account_ids: Vec::new(),
        image_generation_account_models: HashMap::new(),
            // These fixtures exercise the explicitly selected Go engine.
            gateway_mode: CodexLocalAccessGatewayMode::Sidecar,
            capability_fallback: Default::default(),
        multi_agent: Default::default(),
            native_gateway_default_migrated: true,
            upstream_proxy_url: None,
            routing_strategy: CodexLocalAccessRoutingStrategy::default(),
            custom_routing_rules: Vec::new(),
            account_model_rules: Vec::new(),
            account_quota_floors: HashMap::new(),
            account_concurrency_limits: HashMap::new(),
            model_aliases: Vec::new(),
            suppress_oauth_model_alias: false,
            model_pricing_version: DEFAULT_MODEL_PRICING_VERSION,
            model_pricings: Vec::new(),
            excluded_models: Vec::new(),
            session_affinity: true,
            session_identity_mapping_enabled: true,
            session_affinity_ttl_ms: DEFAULT_SESSION_AFFINITY_TTL_MS,
            session_affinity_default_enabled_migrated: true,
            responses_websockets_enabled: false,
            max_retry_credentials: 0,
            max_retry_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
            continuous_overload_retry: false,
            timeouts: CodexLocalAccessTimeouts::default(),
            active_timeout_preset_id: "long_wait".to_string(),
            timeout_presets: Vec::new(),
            disable_cooling: false,
            restrict_free_accounts: true,
            debug_logs: true,
        request_payload_logging: false,
            immediate_sse_response: false,
            max_concurrent_image_requests: 1,
            max_account_concurrency: 0,
            account_concurrency_default_migrated: true,
            account_concurrency_wait_ms: DEFAULT_ACCOUNT_CONCURRENCY_WAIT_MS,
            bound_oauth_account_id: None,
            bound_oauth_quota_reserve: None,
            account_ids,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn provider_gateway_model_aliases_stay_off_the_oauth_channel() {
        let dir = make_temp_dir("codex-provider-gateway-alias");
        let mut collection = test_local_access_collection(vec!["provider-account".to_string()]);
        collection.model_aliases = vec![super::CodexLocalAccessModelAlias {
            source_model: "deepseek-flash".to_string(),
            alias: "gpt-5.5".to_string(),
            fork: false,
        }];
        // 实例供应商网关会标记该字段：别名只用于对话改写，不能写进 OAuth 通道。
        collection.suppress_oauth_model_alias = true;

        super::prepare_sidecar_launch_config_in_dir_sync(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            HashMap::new(),
            true,
            None,
        )
        .expect("prepare provider gateway sidecar config");

        let config: Value = serde_json::from_str(
            &fs::read_to_string(super::sidecar_config_path(&dir)).expect("read sidecar config"),
        )
        .expect("parse sidecar config");
        assert!(
            config.get("oauth-model-alias").is_none(),
            "provider gateway must not rewrite models on the OAuth channel: {config}"
        );

        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(super::sidecar_manifest_path(&dir)).expect("read sidecar manifest"),
        )
        .expect("parse sidecar manifest");
        assert_eq!(
            manifest
                .get("modelAliases")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1),
            "client-visible model aliases must stay in the manifest"
        );

        let _ = fs::remove_dir_all(dir);
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
    fn sidecar_manifest_exposes_image_generation_accounts() {
        let mut collection = test_local_access_collection(vec!["chat-account".to_string()]);
        let mut api_key = build_local_access_api_key(Some("DeepSeek"));
        api_key.key = "deepseek-key".to_string();
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec!["chat-account".to_string()];
        collection.api_keys = vec![api_key];
        collection.image_generation_account_ids =
            vec!["gpt-image-account".to_string(), "gpt-image-account".to_string()];

        let manifest_values = sidecar_api_key_manifest_values(&collection);
        let scoped = manifest_values
            .iter()
            .find(|value| value.get("key").and_then(Value::as_str) == Some("deepseek-key"))
            .expect("provider gateway key should be emitted");
        let image_account_ids = scoped
            .get("imageGenerationAccountIds")
            .and_then(Value::as_array)
            .expect("imageGenerationAccountIds should be an array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(image_account_ids, vec!["gpt-image-account"]);

        // 生图账号不参与对话路由，但必须进入 sidecar 账号清单以写入凭据。
        assert!(super::effective_sidecar_account_ids(&collection)
            .iter()
            .any(|account_id| account_id == "gpt-image-account"));
    }

    #[test]
    fn sidecar_manifest_exposes_image_generation_accounts_for_default_key() {
        let mut collection = test_local_access_collection(vec!["chat-account".to_string()]);
        collection.image_generation_account_ids = vec![
            "gpt-image-account".to_string(),
            "gpt-image-account".to_string(),
        ];

        let legacy = sidecar_api_key_manifest_values(&collection)
            .into_iter()
            .find(|value| value.get("id").and_then(Value::as_str) == Some("legacy"))
            .expect("default API key should be emitted");
        let image_account_ids = legacy
            .get("imageGenerationAccountIds")
            .and_then(Value::as_array)
            .expect("imageGenerationAccountIds should be an array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(image_account_ids, vec!["gpt-image-account"]);
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
    fn windows_manual_proxy_respects_bypass_for_wakeup_target() {
        use super::windows_proxy_url_for_request;

        let target = url::Url::parse("https://daily-cloudcode-pa.googleapis.com").unwrap();
        let server = "http=proxy.local:7890;https=127.0.0.1:7897";
        let clash_bypass = "localhost;127.*;192.168.*;*.local;<local>";
        assert_eq!(
            windows_proxy_url_for_request(server, clash_bypass, &target).as_deref(),
            Some("http://127.0.0.1:7897")
        );
        for bypass in [
            "*",
            "*.googleapis.com",
            "DAILY-CLOUDCODE-PA.GOOGLEAPIS.COM",
            "https://daily-cloudcode-pa.googleapis.com:443",
        ] {
            assert_eq!(windows_proxy_url_for_request(server, bypass, &target), None);
        }
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
    fn official_deepseek_responses_gateway_uses_original_model_ids() {
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

        account.api_instance_access_mode = Some("gateway".to_string());
        assert!(account_requires_provider_gateway(&account));
        assert_eq!(
            provider_gateway_model_slots(&provider_gateway_models_for_account(&account))
                .iter()
                .map(|slot| (slot.client_model.as_str(), slot.upstream_model.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("deepseek-v4-flash", "deepseek-v4-flash"),
                ("deepseek-v4-pro", "deepseek-v4-pro"),
            ]
        );

        account.api_instance_access_mode = Some("direct".to_string());
        assert!(!account_requires_provider_gateway(&account));

        account.api_instance_access_mode = Some("cdp".to_string());
        assert!(!account_requires_provider_gateway(&account));
    }

    #[tokio::test]
    async fn account_pool_compaction_cleanup_does_not_enable_removed_feature() {
        let deepseek_gateway = CodexLocalAccessProviderGateway {
        auth_scheme: None,
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: "sk-deepseek".to_string(),
            upstream_model: "deepseek-v4-pro".to_string(),
            upstream_models: vec!["deepseek-v4-pro".to_string()],
            wire_api: Some("chat_completions".to_string()),
            supports_vision: false,
            model_capabilities: HashMap::new(),
            vision_routing_model: None,
        };
        let mut key = CodexLocalAccessApiKey {
            workbuddy_delegate: None,
            max_reasoning_effort: None,
            id: "provider_gateway_deepseek".to_string(),
            label: "Provider Gateway: deepseek".to_string(),
            key: "agt_test".to_string(),
            provider_gateway: Some(deepseek_gateway.clone()),
            model_routing: None,
            inherit_account_pool: Some(false),
            account_pool_id: None,
            account_ids: Vec::new(),
            priority_account_ids: Vec::new(),
            preferred_account_id: None,
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
            budget: Default::default(),
            budget_usage: Default::default(),
            enabled: true,
            created_at: 0,
            updated_at: 0,
            last_used_at: None,
        };
        let mut collection = new_empty_local_access_collection().expect("collection");
        collection.enabled = true;
        collection.api_keys = vec![key.clone()];

        // 非 GPT 账号池与 DeepSeek 账号池都不再写已移除的压缩开关。
        key.provider_gateway = Some(CodexLocalAccessProviderGateway {
        auth_scheme: None,
            base_url: "https://token-plan-cn.xiaomimimo.com/v1".to_string(),
            ..deepseek_gateway.clone()
        });
        let mut other_collection = new_empty_local_access_collection().expect("collection");
        other_collection.enabled = true;
        other_collection.api_keys = vec![key.clone()];

        let profile_dir = make_temp_dir("codex-pool-local-compaction");
        let config_path = profile_config_path(&profile_dir);
        let original = "model = \"deepseek-v4-pro\"\nservice_tier = \"priority\"\n\n[features]\njs_repl = false\n";
        crate::modules::codex_config_format::write_codex_config_toml_atomic(
            &config_path,
            original,
        )
        .expect("write profile config");

        super::write_local_access_profile_takeover(&profile_dir, &other_collection, None, true)
            .await.expect("take over non-gpt pool profile");
        let non_gpt_applied = fs::read_to_string(&config_path).expect("read profile config");
        assert!(
            !non_gpt_applied.contains("remote_compaction_v2"),
            "非 GPT 账号池不得新增失效开关: {non_gpt_applied}"
        );
        assert!(
            non_gpt_applied.contains("service_tier = \"priority\""),
            "旧开关清理保留其它配置: {non_gpt_applied}"
        );

        super::write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await.expect("take over deepseek pool profile");
        let applied = fs::read_to_string(&config_path).expect("read profile config");
        assert!(!applied.contains("remote_compaction_v2"));
        assert!(!applied.contains("token_budget"));
        assert!(applied.contains("js_repl = false"));
        // 官方账号的 service_tier 必须保留。
        assert!(applied.contains("service_tier = \"priority\""));

        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deepseek_provider_gateway_profile_keeps_valid_overrides() {
        let profile_dir = make_temp_dir("codex-deepseek-gateway-compaction");
        let config_path = profile_config_path(&profile_dir);
        crate::modules::codex_config_format::write_codex_config_toml_atomic(
            &config_path,
            "model = \"gpt-5.6-sol\"\n\n[features]\njs_repl = false\n\n[model_providers.codex_local_access]\nname = \"OpenAI\"\n",
        )
        .expect("write profile config");

        let mut deepseek = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "deepseek@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-flash".to_string()],
        );
        deepseek.api_wire_api = Some("responses".to_string());
        deepseek.api_sync_model_catalog_to_codex = true;

        reapply_deepseek_profile_config_overrides(&profile_dir, &deepseek)
            .expect("reapply deepseek fallback");
        let applied = fs::read_to_string(&config_path).expect("read profile config");
        assert!(!applied.contains("remote_compaction_v2"));
        assert!(!applied.contains("token_budget"));
        assert!(applied.contains("js_repl = false"));

        // 其它供应商的网关 profile 不做任何改写。
        let other_dir = make_temp_dir("codex-generic-gateway-compaction");
        let other_config_path = profile_config_path(&other_dir);
        let original = "model = \"gpt-5.6-sol\"\n\n[features]\njs_repl = false\n";
        crate::modules::codex_config_format::write_codex_config_toml_atomic(
            &other_config_path,
            original,
        )
        .expect("write other profile config");
        let other = CodexAccount::new_api_key(
            "other-account-id".to_string(),
            "other@example.com".to_string(),
            "sk-other".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.moonshot.cn/v1".to_string()),
            Some("moonshot".to_string()),
            Some("Moonshot".to_string()),
            vec!["kimi-k2".to_string()],
        );
        reapply_deepseek_profile_config_overrides(&other_dir, &other)
            .expect("skip non-deepseek fallback");
        assert_eq!(
            fs::read_to_string(&other_config_path).expect("read other profile config"),
            original
        );

        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
        fs::remove_dir_all(&other_dir).expect("cleanup temp dir");
    }

    #[test]
    fn official_template_catalog_keeps_deepseek_ids_and_upstream_metadata() {
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
                    "use_responses_lite": true,
                    "multi_agent_version": "v1",
                    "base_instructions": "flash-instr"
                },
                {
                    "slug": "deepseek-v4-pro",
                    "display_name": "DeepSeek-V4-Pro",
                    "apply_patch_tool_type": "freeform",
                    "shell_type": "shell_command",
                    "tool_mode": "code_mode_only",
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
        assert_eq!(models[0]["use_responses_lite"], false);
        assert_eq!(models[0]["multi_agent_version"], "v1");
        assert_eq!(
            models[0].get("slug").and_then(Value::as_str),
            Some("deepseek-v4-flash")
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
            Some("deepseek-v4-pro")
        );
        assert_eq!(
            models[1].get("display_name").and_then(Value::as_str),
            Some("DeepSeek-V4-Pro")
        );
        assert_eq!(models[1]["tool_mode"], "direct");
        assert_eq!(models[1]["shell_type"], "shell_command");
    }
