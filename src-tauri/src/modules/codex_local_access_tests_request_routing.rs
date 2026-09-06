// Codex Local Access 测试：Usage extraction, routing, request conversion and WebSocket behavior。
// 测试与生产实现共享 super 作用域，验证真实网关、持久化和请求协议行为。
    #[test]
    fn extracts_usage_from_codex_response_completed_payload() {
        let payload = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 16,
                    "input_tokens_details": {
                        "cached_tokens": 3
                    },
                    "output_tokens": 5,
                    "output_tokens_details": {
                        "reasoning_tokens": 2
                    },
                    "total_tokens": 21
                }
            }
        });

        let usage = extract_usage_capture(&payload).expect("usage should be parsed");
        assert_eq!(usage.input_tokens, 16);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.cached_tokens, 3);
        assert_eq!(usage.reasoning_tokens, 2);
        assert_eq!(usage.total_tokens, 21);
    }

    #[test]
    fn extracts_usage_from_codex_response_done_payload() {
        assert!(is_responses_completion_event("response.done"));

        let payload = json!({
            "type": "response.done",
            "response": {
                "id": "resp_123",
                "usage": {
                    "input_tokens": 32,
                    "input_tokens_details": {
                        "cached_tokens": 9
                    },
                    "output_tokens": 6,
                    "output_tokens_details": {
                        "reasoning_tokens": 3
                    },
                    "total_tokens": 41
                }
            }
        });

        let usage = extract_usage_capture(&payload).expect("usage should be parsed");
        assert_eq!(usage.input_tokens, 32);
        assert_eq!(usage.output_tokens, 6);
        assert_eq!(usage.cached_tokens, 9);
        assert_eq!(usage.reasoning_tokens, 3);
        assert_eq!(usage.total_tokens, 41);
    }

    #[test]
    fn extracts_usage_from_openai_prompt_and_completion_details() {
        let payload = json!({
            "usage": {
                "prompt_tokens": 8,
                "prompt_tokens_details": {
                    "cached_tokens": 1
                },
                "completion_tokens": 4,
                "completion_tokens_details": {
                    "reasoning_tokens": 2
                }
            }
        });

        let usage = extract_usage_capture(&payload).expect("usage should be parsed");
        assert_eq!(usage.input_tokens, 8);
        assert_eq!(usage.output_tokens, 4);
        assert_eq!(usage.cached_tokens, 1);
        assert_eq!(usage.reasoning_tokens, 2);
        assert_eq!(usage.total_tokens, 14);
    }

    #[test]
    fn parses_sse_usage_when_request_is_stream_even_if_content_type_is_json() {
        assert!(should_treat_response_as_stream(
            "application/json; charset=utf-8",
            true
        ));

        let mut collector = ResponseUsageCollector::new(true);
        collector.feed(
            br#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_123","usage":{"input_tokens":16,"input_tokens_details":{"cached_tokens":0},"output_tokens":5,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":21}}}

"#,
        );

        let capture = collector.finish();
        let usage = capture.usage.expect("stream usage should be parsed");
        assert_eq!(usage.input_tokens, 16);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.total_tokens, 21);
        assert_eq!(capture.response_id.as_deref(), Some("resp_123"));
    }

    #[test]
    fn parses_codex_retry_after_from_usage_limit_payload() {
        let wait = parse_codex_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"type":"usage_limit_reached","resets_in_seconds":12}}"#,
        )
        .expect("retry after should be parsed");

        assert_eq!(wait, Duration::from_secs(12));
    }

    #[test]
    fn retries_next_account_for_transient_upstream_status() {
        assert!(should_try_next_account(
            StatusCode::SERVICE_UNAVAILABLE,
            "upstream temporarily unavailable"
        ));
        assert!(should_try_next_account(
            StatusCode::BAD_GATEWAY,
            "gateway error"
        ));
    }

    #[test]
    fn retries_single_account_for_transient_upstream_status() {
        assert!(should_retry_single_account_upstream_status(
            StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(should_retry_single_account_upstream_status(
            StatusCode::UNAUTHORIZED
        ));
        assert!(should_retry_single_account_upstream_status(
            StatusCode::GATEWAY_TIMEOUT
        ));
        assert!(!should_retry_single_account_upstream_status(
            StatusCode::TOO_MANY_REQUESTS
        ));
        assert!(!should_retry_single_account_upstream_status(
            StatusCode::FORBIDDEN
        ));
    }

    #[test]
    fn does_not_retry_forbidden_without_quota_or_capacity_markers() {
        assert!(!should_try_next_account(
            StatusCode::FORBIDDEN,
            r#"{"error":"forbidden"}"#,
        ));
    }

    #[test]
    fn retries_next_account_for_image_generation_capability_error() {
        let body = r#"{"error":{"message":"Image generation is not enabled for this group"}}"#;
        assert!(is_image_generation_capability_error(
            StatusCode::FORBIDDEN,
            body,
        ));
        assert!(should_try_next_account(StatusCode::FORBIDDEN, body));
        assert_eq!(
            classify_upstream_error_category(StatusCode::FORBIDDEN, body),
            Some("image_generation_not_enabled")
        );
    }

    #[test]
    fn classifies_stream_incomplete_errors_separately() {
        let decoding_error = "读取上游响应失败: error decoding response body";
        let disconnected_error = "stream error: stream disconnected before completion: stream closed before response.completed/response.done";
        let response_failed_error = "stream error: stream disconnected before completion: stream closed before response.completed/response.done, last_event=response.failed";

        assert!(is_stream_incomplete_error_message(decoding_error));
        assert!(is_stream_incomplete_error_message(disconnected_error));
        assert!(is_upstream_response_failed_error_message(
            response_failed_error
        ));
        assert_eq!(
            legacy_stream_error_category(decoding_error),
            "stream_incomplete"
        );
        assert_eq!(
            legacy_stream_error_category(disconnected_error),
            "stream_incomplete"
        );
        assert_eq!(
            legacy_stream_error_category(response_failed_error),
            "upstream_response_failed"
        );
    }

    #[test]
    fn sidecar_response_failed_overrides_generic_request_failed() {
        let event = SidecarUsageEvent {
            request_id: "req-1".to_string(),
            model: "gpt-5.4".to_string(),
            alias: String::new(),
            account_id: "account-1".to_string(),
            account_email: "user@example.com".to_string(),
            api_key_id: "key-1".to_string(),
            api_key_label: "Default".to_string(),
            client_instance_id: "instance-1".to_string(),
            request_kind: "text".to_string(),
            service_tier: None,
            reasoning_effort: None,
            success: false,
            status: Some(200),
            error_category: Some("request_failed".to_string()),
            error_message: Some("stream error: stream disconnected before completion: stream closed before response.completed/response.done, last_event=response.failed".to_string()),
            latency_ms: 1754,
            usage: SidecarUsageDetails::default(),
        };

        assert_eq!(
            normalized_sidecar_error_category(&event).as_deref(),
            Some("upstream_response_failed")
        );
    }

    #[test]
    fn sidecar_timeout_event_requests_automatic_restart() {
        let event: SidecarUsageEvent = serde_json::from_value(json!({
            "success": false,
            "status": 504,
            "errorCategory": "gateway_context_canceled",
            "errorMessage": "Post https://chatgpt.com/backend-api/codex/responses: context canceled"
        }))
        .expect("timeout event should deserialize");

        assert!(sidecar_usage_event_should_auto_restart(&event));
        assert!(!sidecar_usage_event_is_client_canceled(&event));
        assert_eq!(
            normalized_sidecar_error_category(&event).as_deref(),
            Some("gateway_context_canceled")
        );
    }

    #[test]
    fn sidecar_client_disconnect_does_not_request_automatic_restart() {
        let event: SidecarUsageEvent = serde_json::from_value(json!({
            "success": false,
            "status": 504,
            "errorCategory": "client_canceled",
            "errorMessage": "client disconnected while streaming response"
        }))
        .expect("client disconnect event should deserialize");

        assert!(!sidecar_usage_event_should_auto_restart(&event));
    }

    #[test]
    fn sidecar_usage_reasoning_effort_deserializes_and_round_trips_to_sqlite() {
        let sidecar_event: SidecarUsageEvent =
            serde_json::from_str(r#"{"reasoningEffort":"xhigh"}"#)
                .expect("sidecar reasoning effort should deserialize");
        assert_eq!(sidecar_event.reasoning_effort.as_deref(), Some("xhigh"));

        let dir = make_temp_dir("codex-sidecar-reasoning-effort");
        let db_path = dir.join("request_logs.sqlite");
        let conn = open_local_access_logs_db_once(&db_path, true).expect("open logs db");
        let mut events = Vec::new();
        let persisted = append_usage_event(
            &mut events,
            1_700_000_000_000,
            Some("req-reasoning"),
            Some("acc-1"),
            Some("user@example.com"),
            None,
            None,
            None,
            Some("gpt-5.4"),
            Some(CodexLocalAccessGatewayMode::Sidecar),
            CodexLocalAccessRequestKind::Text,
            None,
            sidecar_event.reasoning_effort.as_deref(),
            true,
            Some(200),
            None,
            None,
            20,
            None,
            None,
            1,
            0.0,
        );
        insert_local_access_usage_event(&conn, &persisted).expect("insert request log");

        let loaded = conn
            .query_row(
                "SELECT * FROM request_logs WHERE request_id = ?1",
                ["req-reasoning"],
                usage_event_from_row,
            )
            .expect("read request log");
        assert_eq!(loaded.reasoning_effort.as_deref(), Some("xhigh"));

        drop(conn);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn prefers_affinity_account_before_round_robin_order() {
        let ordered = build_ordered_account_ids(
            &[
                "acc-a".to_string(),
                "acc-b".to_string(),
                "acc-c".to_string(),
            ],
            1,
            Some("acc-c"),
        );

        assert_eq!(ordered, vec!["acc-c", "acc-b", "acc-a"]);
    }

    #[test]
    fn codex_plan_rank_matches_current_rate_card() {
        let mut promax = test_account_with_plan("pro");
        promax.auth_file_plan_type = Some("promax".to_string());
        let mut prolite = test_account_with_plan("pro");
        prolite.auth_file_plan_type = Some("prolite".to_string());

        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("free")),
            Some(100)
        );
        assert_eq!(resolve_plan_rank(&test_account_with_plan("go")), Some(200));
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("plus")),
            Some(300)
        );
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("team")),
            Some(300)
        );
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("business")),
            Some(300)
        );
        assert_eq!(resolve_plan_rank(&test_account_with_plan("pro")), Some(500));
        assert_eq!(resolve_plan_rank(&prolite), Some(500));
        assert_eq!(resolve_plan_rank(&promax), Some(600));
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("enterprise")),
            Some(700)
        );
        assert_eq!(resolve_plan_rank(&test_account_with_plan("edu")), Some(700));
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("health")),
            Some(700)
        );
        assert_eq!(resolve_plan_rank(&test_account_with_plan("gov")), Some(700));
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("teachers")),
            Some(700)
        );
    }

    #[test]
    fn plan_low_first_places_business_and_team_before_pro() {
        let mut candidates = vec![
            RoutingCandidate {
                account_id: "acc-pro".to_string(),
                plan_rank: Some(500),
                remaining_quota: Some(80),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-plus".to_string(),
                plan_rank: Some(300),
                remaining_quota: Some(40),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-team".to_string(),
                plan_rank: Some(300),
                remaining_quota: Some(70),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-business".to_string(),
                plan_rank: Some(300),
                remaining_quota: Some(60),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-promax".to_string(),
                plan_rank: Some(600),
                remaining_quota: Some(90),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-edu".to_string(),
                plan_rank: Some(700),
                remaining_quota: Some(100),
                subscription_expiry_ms: None,
            },
        ];
        let original_index = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| (candidate.account_id.clone(), index))
            .collect::<HashMap<_, _>>();

        candidates.sort_by(|left, right| {
            compare_routing_candidates(
                left,
                right,
                CodexLocalAccessRoutingStrategy::PlanLowFirst,
                &original_index,
            )
        });

        let ordered = candidates
            .into_iter()
            .map(|candidate| candidate.account_id)
            .collect::<Vec<_>>();

        assert_eq!(
            ordered,
            vec![
                "acc-team",
                "acc-business",
                "acc-plus",
                "acc-pro",
                "acc-promax",
                "acc-edu",
            ]
        );
    }

    #[test]
    fn custom_routing_prefers_higher_priority_accounts() {
        let account_ids = vec![
            "acc-low".to_string(),
            "acc-high-a".to_string(),
            "acc-high-b".to_string(),
        ];
        let rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-low".to_string(),
                priority: 10,
                weight: 1,
                is_backup: false,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-high-a".to_string(),
                priority: 40,
                weight: 1,
                is_backup: false,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-high-b".to_string(),
                priority: 40,
                weight: 1,
                is_backup: false,
                is_preferred: false,
            },
        ];

        let ordered = apply_routing_strategy(
            &account_ids,
            CodexLocalAccessRoutingStrategy::Custom,
            &rules,
            0,
        );

        assert_eq!(ordered, vec!["acc-high-a", "acc-high-b", "acc-low"]);
    }

    #[test]
    fn custom_routing_keeps_backup_accounts_after_regular_accounts() {
        let account_ids = vec!["backup".to_string(), "regular".to_string()];
        let rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: "backup".to_string(),
                priority: 100,
                weight: 1,
                is_backup: true,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "regular".to_string(),
                priority: 0,
                weight: 1,
                is_backup: false,
                is_preferred: false,
            },
        ];

        let ordered = apply_routing_strategy(
            &account_ids,
            CodexLocalAccessRoutingStrategy::Custom,
            &rules,
            0,
        );
        let affinity_ordered = pin_account_to_front_for_strategy(
            ordered,
            &["backup".to_string()],
            CodexLocalAccessRoutingStrategy::Custom,
            &rules,
        );

        assert_eq!(affinity_ordered, vec!["regular", "backup"]);
    }

    #[test]
    fn usage_priority_wraps_every_routing_strategy_and_affinity() {
        let account_ids = vec![
            "lowest".to_string(),
            "normal".to_string(),
            "highest".to_string(),
        ];
        let rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: "lowest".to_string(),
                priority: 100,
                weight: 1,
                is_backup: true,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "normal".to_string(),
                priority: 50,
                weight: 1,
                is_backup: false,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "highest".to_string(),
                priority: 0,
                weight: 1,
                is_backup: false,
                is_preferred: true,
            },
        ];

        for strategy in [
            CodexLocalAccessRoutingStrategy::Auto,
            CodexLocalAccessRoutingStrategy::Random,
            CodexLocalAccessRoutingStrategy::SingleAccount,
            CodexLocalAccessRoutingStrategy::Custom,
        ] {
            let ordered = apply_routing_strategy(&account_ids, strategy, &rules, 0);
            assert_eq!(ordered.first().map(String::as_str), Some("highest"));
            assert_eq!(ordered.last().map(String::as_str), Some("lowest"));

            let affinity_ordered = pin_account_to_front_for_strategy(
                ordered,
                &["lowest".to_string()],
                strategy,
                &rules,
            );
            assert_eq!(affinity_ordered, vec!["highest", "normal", "lowest"]);
        }
    }

    #[test]
    fn account_usage_priority_ids_are_exclusive_and_preserve_unspecified_tier() {
        let mut collection = test_local_access_collection(vec![
            "lowest".to_string(),
            "normal".to_string(),
            "highest".to_string(),
        ]);

        apply_account_usage_priority_ids(
            &mut collection,
            Some(&["lowest".to_string()]),
            Some(&["highest".to_string()]),
        );

        let rules = collection
            .custom_routing_rules
            .iter()
            .map(|rule| {
                (
                    rule.account_id.as_str(),
                    (rule.is_backup, rule.is_preferred),
                )
            })
            .collect::<HashMap<_, _>>();
        assert_eq!(rules.get("lowest"), Some(&(true, false)));
        assert_eq!(rules.get("highest"), Some(&(false, true)));
        assert!(!rules.contains_key("normal"));

        apply_account_usage_priority_ids(&mut collection, Some(&[]), None);
        let highest = collection
            .custom_routing_rules
            .iter()
            .find(|rule| rule.account_id == "highest")
            .expect("highest rule");
        assert!(!highest.is_backup);
        assert!(highest.is_preferred);
    }

    #[test]
    fn legacy_backup_rule_defaults_to_lowest_without_preferred_field() {
        let rule = serde_json::from_value::<CodexLocalAccessCustomRoutingRule>(serde_json::json!({
            "accountId": "legacy-backup",
            "priority": 10,
            "weight": 1,
            "isBackup": true
        }))
        .expect("legacy custom routing rule");

        assert!(rule.is_backup);
        assert!(!rule.is_preferred);
        assert_eq!(
            account_usage_priority(Some(&rule)),
            AccountUsagePriority::Lowest
        );
    }

    #[test]
    fn single_account_routing_keeps_first_account_without_rotation() {
        let account_ids = vec![
            "acc-first".to_string(),
            "acc-second".to_string(),
            "acc-third".to_string(),
        ];

        let ordered = apply_routing_strategy(
            &account_ids,
            CodexLocalAccessRoutingStrategy::SingleAccount,
            &[],
            99,
        );

        assert_eq!(ordered, account_ids);
    }

    #[test]
    fn single_account_routing_limits_credential_attempts_to_one() {
        let mut collection = test_local_access_collection(vec![
            "acc-first".to_string(),
            "acc-second".to_string(),
            "acc-third".to_string(),
        ]);
        collection.routing_strategy = CodexLocalAccessRoutingStrategy::SingleAccount;
        collection.max_retry_credentials = 3;

        let max_attempts = max_credential_attempts_for_strategy(
            &collection,
            collection.account_ids.len(),
            collection.routing_strategy,
        );

        assert_eq!(max_attempts, 1);
    }

    #[test]
    fn sidecar_routing_strategy_serializes_single_account() {
        assert_eq!(
            sidecar_routing_strategy_value(CodexLocalAccessRoutingStrategy::SingleAccount),
            "single_account"
        );
    }

    #[test]
    fn random_routing_serializes_and_keeps_all_candidates() {
        let account_ids = vec![
            "account-a".to_string(),
            "account-b".to_string(),
            "account-c".to_string(),
        ];
        let routed = apply_routing_strategy(
            &account_ids,
            CodexLocalAccessRoutingStrategy::Random,
            &[],
            0,
        );

        assert_eq!(
            sidecar_routing_strategy_value(CodexLocalAccessRoutingStrategy::Random),
            "random"
        );
        assert_eq!(routed.len(), account_ids.len());
        assert!(account_ids
            .iter()
            .all(|account_id| routed.contains(account_id)));
    }

    #[test]
    fn custom_routing_uses_weight_for_same_priority_first_pick() {
        let account_ids = vec!["acc-heavy".to_string(), "acc-light".to_string()];
        let rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-heavy".to_string(),
                priority: 20,
                weight: 3,
                is_backup: false,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-light".to_string(),
                priority: 20,
                weight: 1,
                is_backup: false,
                is_preferred: false,
            },
        ];

        let first_picks = (0..8)
            .map(|start| {
                apply_routing_strategy(
                    &account_ids,
                    CodexLocalAccessRoutingStrategy::Custom,
                    &rules,
                    start,
                )[0]
                .clone()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            first_picks,
            vec![
                "acc-heavy",
                "acc-heavy",
                "acc-heavy",
                "acc-light",
                "acc-heavy",
                "acc-heavy",
                "acc-heavy",
                "acc-light",
            ]
        );
    }

    #[test]
    fn custom_routing_rules_are_normalized_to_collection_accounts() {
        let account_ids = vec!["acc-a".to_string(), "acc-b".to_string()];
        let rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: " acc-a ".to_string(),
                priority: 120,
                weight: 0,
                is_backup: true,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-a".to_string(),
                priority: 20,
                weight: 10,
                is_backup: false,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-removed".to_string(),
                priority: 30,
                weight: 5,
                is_backup: false,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-b".to_string(),
                priority: -5,
                weight: 500,
                is_backup: false,
                is_preferred: false,
            },
        ];

        let normalized = normalize_custom_routing_rules(rules, &account_ids);

        assert_eq!(
            normalized,
            vec![
                CodexLocalAccessCustomRoutingRule {
                    account_id: "acc-a".to_string(),
                    priority: 100,
                    weight: 1,
                    is_backup: true,
                    is_preferred: false,
                },
                CodexLocalAccessCustomRoutingRule {
                    account_id: "acc-b".to_string(),
                    priority: 0,
                    weight: 100,
                    is_backup: false,
                    is_preferred: false,
                },
            ]
        );
    }

    #[test]
    fn builds_routing_hint_from_previous_response_id_and_model() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"GPT-5.4-mini","previous_response_id":"resp_prev"}"#.to_vec(),
        };

        let hint = build_request_routing_hint(&request);
        assert_eq!(hint.model_key, "gpt-5.4-mini");
        assert_eq!(hint.previous_response_id.as_deref(), Some("resp_prev"));
    }

    #[test]
    fn maps_snapshot_model_ids_to_supported_aliases() {
        assert_eq!(
            resolve_supported_model_alias("gpt-5.4-2026-03-05"),
            "gpt-5.4"
        );
        assert_eq!(
            resolve_supported_model_alias("GPT-5.4-Mini-2026-03-05"),
            "gpt-5.4-mini"
        );
        assert_eq!(
            resolve_supported_model_alias("custom-model-2026-03-05"),
            "custom-model-2026-03-05"
        );
    }

    #[test]
    fn local_models_include_codex_image_model() {
        let response = build_local_models_response(&[
            "gpt-5.4".to_string(),
            "gpt-image-2".to_string(),
            CODEX_AUTO_REVIEW_MODEL_ID.to_string(),
        ]);
        let has_image_model = response
            .get("data")
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .any(|model| model.get("id").and_then(Value::as_str) == Some("gpt-image-2"))
            })
            .unwrap_or(false);
        let has_auto_review_model = response
            .get("data")
            .and_then(Value::as_array)
            .map(|models| {
                models.iter().any(|model| {
                    model.get("id").and_then(Value::as_str) == Some(CODEX_AUTO_REVIEW_MODEL_ID)
                })
            })
            .unwrap_or(false);

        assert!(has_image_model);
        assert!(has_auto_review_model);
    }

    #[test]
    fn codex_client_models_use_models_catalog_shape() {
        let response = build_codex_client_models_response(&[
            "gpt-5.4".to_string(),
            "gpt-image-2".to_string(),
            CODEX_AUTO_REVIEW_MODEL_ID.to_string(),
        ]);
        assert!(response.get("object").is_none());
        assert!(response.get("data").is_none());
        let models = response
            .get("models")
            .and_then(Value::as_array)
            .expect("codex client models should be an array");
        assert!(models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.4")));
        assert!(models
            .iter()
            .all(|model| model.get("prefer_websockets").and_then(Value::as_bool) == Some(true)));
        assert!(models.iter().any(|model| {
            model.get("slug").and_then(Value::as_str) == Some(CODEX_AUTO_REVIEW_MODEL_ID)
                && model.get("visibility").and_then(Value::as_str) == Some("hide")
        }));
    }

    #[test]
    fn auto_review_model_bypasses_legacy_gateway_model_filters() {
        let collection = test_local_access_collection(vec!["account-1".to_string()]);
        let api_key = ResolvedLocalApiKey {
            id: "key-1".to_string(),
            label: "Key".to_string(),
            provider_gateway: None,
            inherit_account_pool: true,
            account_ids: Vec::new(),
            model_prefix: Some("team".to_string()),
            allowed_models: vec!["gpt-*".to_string()],
            excluded_models: vec!["codex-*".to_string()],
            token_limit: None,
            token_used: 0,
        };

        let models = visible_codex_model_ids_for_api_key(&collection, &api_key, None);
        assert!(models
            .iter()
            .any(|model| model == CODEX_AUTO_REVIEW_MODEL_ID));
        assert_eq!(
            canonical_model_for_client_model(CODEX_AUTO_REVIEW_MODEL_ID, &collection, &api_key),
            CODEX_AUTO_REVIEW_MODEL_ID
        );
        assert!(validate_client_model_visible(
            CODEX_AUTO_REVIEW_MODEL_ID,
            CODEX_AUTO_REVIEW_MODEL_ID,
            &collection,
            &api_key,
            None,
        ));
    }

    #[test]
    fn api_key_model_visibility_includes_5_6_unless_explicitly_restricted() {
        let collection = test_local_access_collection(vec!["account-1".to_string()]);
        let mut api_key = ResolvedLocalApiKey {
            id: "key-1".to_string(),
            label: "Key".to_string(),
            provider_gateway: None,
            inherit_account_pool: true,
            account_ids: Vec::new(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };

        let models = visible_codex_model_ids_for_api_key(&collection, &api_key, None);
        for model in [
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-6-astra",
        ] {
            assert!(models.iter().any(|item| item == model));
        }

        api_key.allowed_models = vec!["gpt-5.4".to_string()];
        let restricted = visible_codex_model_ids_for_api_key(&collection, &api_key, None);
        assert!(restricted.iter().any(|model| model == "gpt-5.4"));
        assert!(!restricted.iter().any(|model| model.starts_with("gpt-5.6-")));
    }

    #[test]
    fn api_service_custom_models_are_added_without_aliasing() {
        let models = super::merge_api_service_experimental_model_ids(
            vec!["gpt-5.6-sol".to_string()],
            &["custom-model".to_string(), "custom-model-2".to_string()],
        );
        assert_eq!(
            models,
            vec![
                "gpt-5.6-sol".to_string(),
                "custom-model".to_string(),
                "custom-model-2".to_string(),
            ]
        );

        let collection = test_local_access_collection(vec!["account-1".to_string()]);
        let api_key = ResolvedLocalApiKey {
            id: "key-1".to_string(),
            label: "Key".to_string(),
            provider_gateway: None,
            inherit_account_pool: true,
            account_ids: Vec::new(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };
        assert_eq!(
            canonical_model_for_client_model("custom-model", &collection, &api_key),
            "custom-model"
        );
    }

    #[test]
    fn experimental_model_catalog_keeps_image_model_visible_when_capacity_allows_it() {
        let catalog = vec!["gpt-5.6-sol".to_string(), "custom-model".to_string()];
        let visible = apply_codex_image_model_visibility(catalog.clone(), true);
        assert!(visible.iter().any(|model| model == CODEX_IMAGE_MODEL_ID));
        assert_eq!(visible.len(), catalog.len() + 1);

        let hidden = apply_codex_image_model_visibility(visible, false);
        assert!(!hidden.iter().any(|model| model == CODEX_IMAGE_MODEL_ID));
    }

    #[test]
    fn provider_gateway_models_are_visible_for_gateway_api_key() {
        let collection = test_local_access_collection(vec!["account-1".to_string()]);
        let api_key = ResolvedLocalApiKey {
            id: "provider_gateway_account-1".to_string(),
            label: "Provider Gateway: DeepSeek".to_string(),
            provider_gateway: Some(CodexLocalAccessProviderGateway {
                base_url: "https://api.deepseek.com/v1".to_string(),
                api_key: "sk-test".to_string(),
                upstream_model: "deepseek-v4-pro".to_string(),
                upstream_models: vec![
                    "deepseek-v4-pro".to_string(),
                    "deepseek-v4-flash".to_string(),
                ],
                wire_api: Some("chat_completions".to_string()),
                supports_vision: false,
                model_capabilities: HashMap::new(),
                vision_routing_model: None,
            }),
            inherit_account_pool: false,
            account_ids: vec!["account-1".to_string()],
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };

        let models = visible_codex_model_ids_for_api_key(&collection, &api_key, None);

        assert!(models.iter().any(|model| model == "deepseek-v4-pro"));
        assert!(models.iter().any(|model| model == "deepseek-v4-flash"));
        assert!(validate_client_model_visible(
            "deepseek-v4-pro",
            "deepseek-v4-pro",
            &collection,
            &api_key,
            None,
        ));
    }

    #[test]
    fn prepares_chat_completions_request_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"GPT-5.4","stream":true,"messages":[{"role":"user","content":"hello"}]}"#
                .to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        assert_eq!(prepared.target, "/v1/responses");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("model").and_then(Value::as_str),
            Some("gpt-5.4")
        );
        assert!(mapped_body.get("input").is_some());
        assert_eq!(mapped_body.get("store"), Some(&Value::Bool(false)));
        assert_eq!(mapped_body.get("stream"), Some(&Value::Bool(true)));
        assert_eq!(
            mapped_body.get("instructions").and_then(Value::as_str),
            Some("")
        );
        assert_eq!(
            mapped_body
                .get("parallel_tool_calls")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            mapped_body
                .get("reasoning")
                .and_then(|reasoning| reasoning.get("effort"))
                .and_then(Value::as_str),
            Some("medium")
        );
        assert!(!has_image_generation_tool(&mapped_body));

        match adapter {
            GatewayResponseAdapter::ChatCompletions {
                stream,
                requested_model,
                original_request_body: _,
            } => {
                assert!(stream);
                assert_eq!(requested_model, "gpt-5.4");
            }
            _ => panic!("expected chat completions adapter"),
        }
    }

    #[test]
    fn rejects_gpt_image_models_from_chat_completions() {
        for model in ["gpt-image-1", "GPT-IMAGE-2", "team/gpt-image-2"] {
            let request = ParsedRequest {
                method: "POST".to_string(),
                target: "/v1/chat/completions".to_string(),
                headers: HashMap::new(),
                body: format!(
                    r#"{{"model":"{}","messages":[{{"role":"user","content":"draw"}}]}}"#,
                    model
                )
                .into_bytes(),
            };

            let err = prepare_gateway_request(request)
                .expect_err("image-only model should be rejected before proxying");
            assert!(err.contains("Chat Completions"), "model={model}, err={err}");
        }
    }

    #[test]
    fn chat_completions_conversion_enforces_responses_lite_tools() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{
                "model":"gpt-5.6-sol",
                "messages":[{"role":"user","content":"hello"}],
                "tools":[
                    {"type":"function","function":{"name":"lookup","parameters":{}}},
                    {"type":"function","function":{"name":"image_gen.imagegen","parameters":{}}},
                    {"type":"custom","name":"apply_patch"},
                    {"type":"tool_search","execution":"client"},
                    {"type":"tool_search"},
                    {"type":"web_search"},
                    {"type":"image_generation"},
                    {"type":"namespace","name":"mcp__root"}
                ],
                "tool_choice":{"type":"web_search"}
            }"#
            .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body
                .get("parallel_tool_calls")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|tool| tool.get("type").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                }),
            Some(vec!["function", "function", "custom", "tool_search"])
        );
        assert!(mapped_body.get("tool_choice").is_none());
        assert!(mapped_body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|tool| {
                tool.get("name").and_then(Value::as_str) == Some("image_gen.imagegen")
            })));
    }

    #[test]
    fn legacy_responses_requests_inject_default_priority_service_tier() {
        let cases = [
            (
                br#"{"model":"gpt-5.4","input":"hello"}"#.as_slice(),
                None,
                None,
            ),
            (
                br#"{"model":"gpt-5.4","stream":true,"reasoning":{"effort":"low"},"input":"hello"}"#
                    .as_slice(),
                Some(true),
                Some("low"),
            ),
        ];

        for (body, expected_stream, expected_effort) in cases {
            let request = ParsedRequest {
                method: "POST".to_string(),
                target: "/v1/responses".to_string(),
                headers: HashMap::new(),
                body: body.to_vec(),
            };

            let (prepared, _) =
                prepare_gateway_request_with_default_service_tier(request, Some("priority"))
                    .expect("request should map");
            let mapped_body: Value =
                serde_json::from_slice(&prepared.body).expect("mapped body should be json");
            assert_eq!(
                mapped_body.get("service_tier").and_then(Value::as_str),
                Some("priority")
            );
            if let Some(expected_stream) = expected_stream {
                assert_eq!(
                    mapped_body.get("stream").and_then(Value::as_bool),
                    Some(expected_stream)
                );
            }
            if let Some(expected_effort) = expected_effort {
                assert_eq!(
                    mapped_body
                        .get("reasoning")
                        .and_then(|reasoning| reasoning.get("effort"))
                        .and_then(Value::as_str),
                    Some(expected_effort)
                );
            }
        }
    }

    #[test]
    fn legacy_chat_completions_requests_inject_default_priority_service_tier() {
        let cases = [
            (
                br#"{"model":"gpt-5.4","messages":[{"role":"user","content":"hello"}]}"#
                    .as_slice(),
                None,
                None,
            ),
            (
                br#"{"model":"gpt-5.4","stream":true,"reasoning_effort":"low","messages":[{"role":"user","content":"hello"}]}"#
                    .as_slice(),
                Some(true),
                Some("low"),
            ),
        ];

        for (body, expected_stream, expected_effort) in cases {
            let request = ParsedRequest {
                method: "POST".to_string(),
                target: "/v1/chat/completions".to_string(),
                headers: HashMap::new(),
                body: body.to_vec(),
            };

            let (prepared, adapter) =
                prepare_gateway_request_with_default_service_tier(request, Some("priority"))
                    .expect("request should map");
            let mapped_body: Value =
                serde_json::from_slice(&prepared.body).expect("mapped body should be json");
            assert_eq!(
                mapped_body.get("service_tier").and_then(Value::as_str),
                Some("priority")
            );
            if let Some(expected_stream) = expected_stream {
                assert_eq!(
                    mapped_body.get("stream").and_then(Value::as_bool),
                    Some(expected_stream)
                );
                match adapter {
                    GatewayResponseAdapter::ChatCompletions { stream, .. } => {
                        assert_eq!(stream, expected_stream)
                    }
                    _ => panic!("expected chat completions adapter"),
                }
            }
            if let Some(expected_effort) = expected_effort {
                assert_eq!(
                    mapped_body
                        .get("reasoning")
                        .and_then(|reasoning| reasoning.get("effort"))
                        .and_then(Value::as_str),
                    Some(expected_effort)
                );
            }
        }
    }

    #[test]
    fn legacy_chat_completions_requests_preserve_max_reasoning_effort() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-6-astra","reasoning_effort":"max","messages":[{"role":"user","content":"hello"}]}"#.to_vec(),
        };

        let (prepared, _) =
            prepare_gateway_request_with_default_service_tier(request, Some("priority"))
                .expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body
                .get("reasoning")
                .and_then(|reasoning| reasoning.get("effort"))
                .and_then(Value::as_str),
            Some("max")
        );
    }

    #[test]
    fn legacy_chat_completions_requests_preserve_explicit_service_tier() {
        let cases = [
            (
                br#"{"model":"gpt-5.4","service_tier":"priority","messages":[{"role":"user","content":"hello"}]}"#
                    .as_slice(),
                None,
                None,
            ),
            (
                br#"{"model":"gpt-5.4","stream":true,"reasoning_effort":"low","service_tier":"priority","messages":[{"role":"user","content":"hello"}]}"#
                    .as_slice(),
                Some(true),
                Some("low"),
            ),
        ];

        for (body, expected_stream, expected_effort) in cases {
            let request = ParsedRequest {
                method: "POST".to_string(),
                target: "/v1/chat/completions".to_string(),
                headers: HashMap::new(),
                body: body.to_vec(),
            };

            let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
            let mapped_body: Value =
                serde_json::from_slice(&prepared.body).expect("mapped body should be json");
            assert_eq!(
                mapped_body.get("service_tier").and_then(Value::as_str),
                Some("priority")
            );
            if let Some(expected_stream) = expected_stream {
                assert_eq!(
                    mapped_body.get("stream").and_then(Value::as_bool),
                    Some(expected_stream)
                );
                match adapter {
                    GatewayResponseAdapter::ChatCompletions { stream, .. } => {
                        assert_eq!(stream, expected_stream)
                    }
                    _ => panic!("expected chat completions adapter"),
                }
            }
            if let Some(expected_effort) = expected_effort {
                assert_eq!(
                    mapped_body
                        .get("reasoning")
                        .and_then(|reasoning| reasoning.get("effort"))
                        .and_then(Value::as_str),
                    Some(expected_effort)
                );
            }
        }
    }

    #[test]
    fn prepares_images_generation_request_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/images/generations".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-image-2","prompt":"draw a clean icon","size":"1024x1024","response_format":"b64_json"}"#.to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        assert_eq!(prepared.target, "/v1/responses");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("model").and_then(Value::as_str),
            Some("gpt-5.4-mini")
        );
        assert_eq!(
            mapped_body
                .get("tool_choice")
                .and_then(|choice| choice.get("type"))
                .and_then(Value::as_str),
            Some("image_generation")
        );
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("model"))
                .and_then(Value::as_str),
            Some("gpt-image-2")
        );
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("size"))
                .and_then(Value::as_str),
            Some("1024x1024")
        );

        match adapter {
            GatewayResponseAdapter::Images {
                stream,
                response_format,
                stream_prefix,
            } => {
                assert!(!stream);
                assert_eq!(response_format, "b64_json");
                assert_eq!(stream_prefix, "image_generation");
            }
            _ => panic!("expected images adapter"),
        }
    }

    #[test]
    fn rejects_unsupported_images_model() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/images/generations".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-image-1.5","prompt":"draw"}"#.to_vec(),
        };

        let err = prepare_gateway_request(request).expect_err("model should be rejected");
        assert!(err.contains("Use gpt-image-2"));
    }

    #[test]
    fn prepares_multipart_images_edit_request_for_responses_proxy() {
        let boundary = "test-boundary";
        let mut body = Vec::new();
        body.extend_from_slice(b"--test-boundary\r\n");
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
        body.extend_from_slice(b"gpt-image-2\r\n");
        body.extend_from_slice(b"--test-boundary\r\n");
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"prompt\"\r\n\r\n");
        body.extend_from_slice(b"make it brighter\r\n");
        body.extend_from_slice(b"--test-boundary\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"image\"; filename=\"a.png\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
        body.extend_from_slice(b"\x89PNG\r\n\x1a\nabc\r\n");
        body.extend_from_slice(b"--test-boundary--\r\n");
        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            format!("multipart/form-data; boundary={}", boundary),
        );
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/images/edits".to_string(),
            headers,
            body,
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        assert_eq!(prepared.target, "/v1/responses");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("action"))
                .and_then(Value::as_str),
            Some("edit")
        );
        let has_input_image = mapped_body
            .get("input")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("content"))
            .and_then(Value::as_array)
            .map(|content| {
                content.iter().any(|part| {
                    part.get("type").and_then(Value::as_str) == Some("input_image")
                        && part
                            .get("image_url")
                            .and_then(Value::as_str)
                            .map(|url| url.starts_with("data:image/png;base64,"))
                            .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        assert!(has_input_image);

        match adapter {
            GatewayResponseAdapter::Images { stream_prefix, .. } => {
                assert_eq!(stream_prefix, "image_edit");
            }
            _ => panic!("expected images adapter"),
        }
    }

    #[test]
    fn builds_images_api_payload_from_responses_output() {
        let response = json!({
            "response": {
                "created_at": 123,
                "output": [{
                    "type": "image_generation_call",
                    "result": "aGVsbG8=",
                    "output_format": "png",
                    "revised_prompt": "draw a clean icon"
                }],
                "tool_usage": {
                    "image_gen": {
                        "input_images": 0,
                        "output_images": 1
                    }
                }
            }
        });

        let payload =
            build_images_api_payload(&response, "b64_json").expect("payload should build");
        assert_eq!(payload.get("created").and_then(Value::as_i64), Some(123));
        assert_eq!(
            payload
                .get("data")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("b64_json"))
                .and_then(Value::as_str),
            Some("aGVsbG8=")
        );
        assert_eq!(
            payload
                .get("data")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("revised_prompt"))
                .and_then(Value::as_str),
            Some("draw a clean icon")
        );
    }

    #[test]
    fn rewrites_snapshot_model_ids_for_passthrough_requests() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4-2026-03-05","input":"hello"}"#.to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("model").and_then(Value::as_str),
            Some("gpt-5.4")
        );
        assert_eq!(
            mapped_body.get("stream").and_then(Value::as_bool),
            Some(true)
        );

        match adapter {
            GatewayResponseAdapter::Passthrough { request_is_stream } => {
                assert!(request_is_stream);
            }
            _ => panic!("expected passthrough adapter"),
        }
    }

    #[test]
    fn responses_stream_requests_stay_passthrough() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::from([("accept".to_string(), "text/event-stream".to_string())]),
            body: br#"{"model":"gpt-5.4","stream":false,"store":true,"input":"hello","temperature":0.2}"#
                .to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        assert_eq!(prepared.target, "/v1/responses");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("stream").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            mapped_body.get("store").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            mapped_body.get("instructions").and_then(Value::as_str),
            Some("")
        );
        assert!(mapped_body.get("temperature").is_none());
        assert_eq!(
            mapped_body
                .pointer("/input/0/content/0/text")
                .and_then(Value::as_str),
            Some("hello")
        );

        match adapter {
            GatewayResponseAdapter::Passthrough { request_is_stream } => {
                assert!(request_is_stream);
            }
            _ => panic!("expected responses stream passthrough adapter"),
        }
    }

    #[test]
    fn injects_image_generation_tool_for_non_free_responses_accounts() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","input":"draw an icon"}"#.to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert!(!has_image_generation_tool(&mapped_body));
        assert_eq!(
            mapped_body.get("stream").and_then(Value::as_bool),
            Some(true)
        );

        let paid_oauth_account = test_account_with_plan("plus");
        let paid_oauth_body = build_account_scoped_upstream_body(
            "/responses",
            &prepared.body,
            &paid_oauth_account,
            CodexLocalAccessImageGenerationMode::Enabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("paid oauth body should build");
        let paid_oauth_mapped_body: Value = serde_json::from_slice(paid_oauth_body.as_ref())
            .expect("paid oauth body should be json");
        assert!(has_image_generation_tool(&paid_oauth_mapped_body));

        let api_key_account = CodexAccount::new_api_key(
            "api-key-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::OpenaiBuiltin,
            None,
            None,
            None,
            Vec::new(),
        );
        let api_key_body = build_account_scoped_upstream_body(
            "/responses",
            &prepared.body,
            &api_key_account,
            CodexLocalAccessImageGenerationMode::Enabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("api key body should build");
        let api_key_mapped_body: Value =
            serde_json::from_slice(api_key_body.as_ref()).expect("api key body should be json");
        assert!(api_key_mapped_body
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| tools.iter().any(|tool| {
                tool.get("type").and_then(Value::as_str) == Some("image_generation")
                    && tool.get("output_format").and_then(Value::as_str) == Some("png")
            }))
            .unwrap_or(false));

        let free_account = test_account_with_plan("free");
        let free_body = build_account_scoped_upstream_body(
            "/responses",
            &prepared.body,
            &free_account,
            CodexLocalAccessImageGenerationMode::Enabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("free body should build");
        let free_mapped_body: Value =
            serde_json::from_slice(free_body.as_ref()).expect("free body should be json");
        assert!(!has_image_generation_tool(&free_mapped_body));

        let images_only_body = build_account_scoped_upstream_body(
            "/responses",
            &prepared.body,
            &api_key_account,
            CodexLocalAccessImageGenerationMode::ImagesOnly,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("images-only body should build");
        let images_only_mapped_body: Value = serde_json::from_slice(images_only_body.as_ref())
            .expect("images-only body should be json");
        assert!(!has_image_generation_tool(&images_only_mapped_body));

        match adapter {
            GatewayResponseAdapter::Passthrough { request_is_stream } => {
                assert!(request_is_stream);
            }
            _ => panic!("expected passthrough adapter"),
        }
    }

    #[test]
    fn free_oauth_text_removes_client_declared_hosted_image_tool() {
        let account = test_account_with_plan("free");
        let body = br#"{
            "model":"gpt-5.4",
            "input":[
                {"type":"additional_tools","tools":[{"type":"image_generation"}]},
                {"type":"message","role":"user","content":"hello"}
            ],
            "tool_choice":{"type":"image_generation"},
            "tools":[
                {"type":"image_generation","output_format":"png"},
                {"type":"function","name":"image_gen.imagegen"},
                {"type":"function","name":"lookup"}
            ],
            "response":{
                "tool_choice":{"type":"image_generation"},
                "tools":[{"type":"image_generation"},{"type":"function","name":"keep"}]
            }
        }"#;

        let mapped_body = build_account_scoped_upstream_body(
            "/responses",
            body,
            &account,
            CodexLocalAccessImageGenerationMode::Enabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("free text body should build");
        let parsed: Value =
            serde_json::from_slice(mapped_body.as_ref()).expect("body should remain json");

        assert!(parsed.get("tool_choice").is_none());
        assert!(parsed
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| {
                tools.iter().all(|tool| {
                    tool.get("type").and_then(Value::as_str) != Some("image_generation")
                }) && tools.iter().any(|tool| {
                    tool.get("name").and_then(Value::as_str) == Some("image_gen.imagegen")
                })
            }));
        assert_eq!(
            parsed.pointer("/input/0/type").and_then(Value::as_str),
            Some("message")
        );
        assert!(parsed.pointer("/response/tool_choice").is_none());
        assert!(parsed
            .pointer("/response/tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.len() == 1
                && tools[0].get("name").and_then(Value::as_str) == Some("keep")));
    }

    #[test]
    fn responses_lite_models_never_reinject_hosted_tools() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::from([(
                "x-openai-internal-codex-responses-lite".to_string(),
                String::new(),
            )]),
            body: br#"{
                "model":"gpt-5.6-sol",
                "input":"hello",
                "tools":[
                    {"type":"function","name":"lookup"},
                    {"type":"function","name":"image_gen.imagegen"},
                    {"type":"custom","name":"apply_patch"},
                    {"type":"tool_search","execution":"client"},
                    {"type":"image_generation"},
                    {"type":"web_search"},
                    {"type":"namespace","name":"mcp__root"}
                ]
            }"#
            .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let account = test_account_with_plan("plus");
        let mapped = build_account_scoped_upstream_body(
            "/responses",
            &prepared.body,
            &account,
            CodexLocalAccessImageGenerationMode::ImagesOnly,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("Responses Lite body should build");
        let parsed: Value = serde_json::from_slice(mapped.as_ref()).expect("body should be json");
        assert_eq!(
            parsed.get("tools").and_then(Value::as_array).map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool.get("type").and_then(Value::as_str))
                    .collect::<Vec<_>>()
            }),
            Some(vec!["function", "function", "custom", "tool_search"])
        );
        assert!(parsed
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|tool| {
                tool.get("type").and_then(Value::as_str) == Some("function")
                    && tool.get("name").and_then(Value::as_str) == Some("image_gen.imagegen")
            })));
    }

    #[test]
    fn disabled_image_generation_mode_removes_declared_tool_and_choice() {
        let account = test_account_with_plan("plus");
        let body = br#"{
            "model":"gpt-5.4",
            "input":"hello",
            "tool_choice":{"type":"image_generation"},
            "tools":[
                {"type":"web_search_preview"},
                {"type":"image_generation","output_format":"png"}
            ]
        }"#;

        let mapped_body = build_account_scoped_upstream_body(
            "/responses",
            body,
            &account,
            CodexLocalAccessImageGenerationMode::Disabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("disabled body should build");
        let parsed: Value =
            serde_json::from_slice(mapped_body.as_ref()).expect("body should remain json");

        assert!(!has_image_generation_tool(&parsed));
        assert!(parsed.get("tool_choice").is_none());
        assert!(parsed
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| tools
                .iter()
                .any(|tool| tool.get("type").and_then(Value::as_str) == Some("web_search_preview")))
            .unwrap_or(false));
    }

    #[test]
    fn disabled_image_generation_mode_removes_responses_lite_capabilities() {
        let account = test_account_with_plan("plus");
        let body = br#"{
            "model":"gpt-5.6-sol",
            "tool_choice":{"tool":{"type":"namespace","name":"image_gen"}},
            "tools":[
                {"type":"namespace","name":"image_gen","tools":[{"type":"function","name":"imagegen"}]},
                {"type":"function","name":"image_gen.imagegen"},
                {"type":"namespace","name":"codex_app"},
                {"type":"function","name":"lookup"}
            ],
            "input":[
                {"type":"additional_tools","tools":[
                    {"type":"namespace","namespace":"image_gen"},
                    {"type":"function","name":"keep_me"}
                ]},
                {"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AA=="}]},
                {"type":"image_generation_call","result":"AA=="}
            ],
            "response":{
                "tools":[{"type":"image_generation"},{"type":"function","name":"nested_keep"}],
                "tool_choice":{"type":"namespace","namespace":"image_gen"}
            }
        }"#;

        let mapped_body = build_account_scoped_upstream_body(
            "/responses",
            body,
            &account,
            CodexLocalAccessImageGenerationMode::Disabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("disabled Responses Lite body should build");
        let parsed: Value =
            serde_json::from_slice(mapped_body.as_ref()).expect("body should remain json");

        assert!(parsed.get("tool_choice").is_none());
        assert!(parsed
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.len() == 2
                && tools
                    .iter()
                    .all(|tool| !tool_declares_image_generation_capability(tool))));
        assert!(parsed
            .pointer("/input/0/tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.len() == 1
                && tools[0].get("name").and_then(Value::as_str) == Some("keep_me")));
        assert_eq!(
            parsed
                .pointer("/input/1/content/0/type")
                .and_then(Value::as_str),
            Some("input_image")
        );
        assert_eq!(
            parsed.pointer("/input/2/type").and_then(Value::as_str),
            Some("image_generation_call")
        );
        assert!(parsed.pointer("/response/tool_choice").is_none());
        assert_eq!(
            parsed
                .pointer("/response/tools/0/name")
                .and_then(Value::as_str),
            Some("nested_keep")
        );
    }

    #[test]
    fn websocket_followup_messages_apply_the_same_image_generation_filter() {
        let account = test_account_with_plan("plus");
        let payload = r#"{"type":"response.create","response":{"tools":[{"type":"namespace","name":"image_gen"},{"type":"function","name":"keep"}]}}"#;

        for message in [
            Message::Text(payload.into()),
            Message::Binary(payload.as_bytes().to_vec().into()),
        ] {
            let filtered = filter_websocket_client_message(
                message,
                &account,
                CodexLocalAccessImageGenerationMode::Disabled,
                false,
            )
            .expect("WebSocket follow-up payload should filter");
            let parsed = match filtered {
                Message::Text(text) => serde_json::from_str::<Value>(&text).unwrap(),
                Message::Binary(bytes) => serde_json::from_slice::<Value>(&bytes).unwrap(),
                _ => panic!("expected data frame"),
            };
            assert_eq!(
                parsed
                    .pointer("/response/tools/0/name")
                    .and_then(Value::as_str),
                Some("keep")
            );
        }
    }

    #[test]
    fn websocket_followup_messages_filter_responses_lite_tools() {
        let account = test_account_with_plan("plus");
        let payload = r#"{
            "type":"response.create",
            "response":{
                "model":"gpt-5.6-sol",
                "tools":[
                    {"type":"function","name":"keep_function"},
                    {"type":"custom","name":"keep_custom"},
                    {"type":"tool_search","execution":"client"},
                    {"type":"image_generation"},
                    {"type":"web_search"},
                    {"type":"namespace","name":"mcp__root"}
                ],
                "tool_choice":{"type":"image_generation"}
            }
        }"#;

        for message in [
            Message::Text(payload.into()),
            Message::Binary(payload.as_bytes().to_vec().into()),
        ] {
            let filtered = filter_websocket_client_message(
                message,
                &account,
                CodexLocalAccessImageGenerationMode::Enabled,
                false,
            )
            .expect("Responses Lite WebSocket follow-up payload should filter");
            let parsed = match filtered {
                Message::Text(text) => serde_json::from_str::<Value>(&text).unwrap(),
                Message::Binary(bytes) => serde_json::from_slice::<Value>(&bytes).unwrap(),
                _ => panic!("expected data frame"),
            };
            assert_eq!(
                parsed
                    .pointer("/response/tools")
                    .and_then(Value::as_array)
                    .map(|tools| {
                        tools
                            .iter()
                            .filter_map(|tool| tool.get("type").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                    }),
                Some(vec!["function", "custom", "tool_search"])
            );
            assert!(parsed.pointer("/response/tool_choice").is_none());
        }
    }

    #[test]
    fn oauth_responses_does_not_mix_image_gen_function_with_hosted_tool() {
        let account = test_account_with_plan("plus");
        let body = br#"{
            "model":"gpt-5.6-sol",
            "input":"create an image",
            "tools":[
                {
                    "type":"function",
                    "name":"image_gen.imagegen",
                    "description":"Generate an image",
                    "parameters":{"type":"object","properties":{}}
                }
            ]
        }"#;

        let mapped_body = build_account_scoped_upstream_body(
            "/responses",
            body,
            &account,
            CodexLocalAccessImageGenerationMode::Enabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("oauth body should build");
        let parsed: Value =
            serde_json::from_slice(mapped_body.as_ref()).expect("body should remain json");

        assert!(!has_image_generation_tool(&parsed));
        assert!(parsed
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|tool| {
                tool.get("name").and_then(Value::as_str) == Some("image_gen.imagegen")
            })));
    }

    #[test]
    fn oauth_responses_removes_hosted_tool_when_image_gen_namespace_is_present() {
        let account = test_account_with_plan("plus");
        let body = br#"{
            "model":"gpt-5.6-sol",
            "input":"create an image",
            "tool_choice":{"type":"image_generation"},
            "tools":[
                {
                    "type":"namespace",
                    "name":"image_gen",
                    "tools":[{"type":"function","name":"imagegen","parameters":{}}]
                },
                {"type":"image_generation","output_format":"png"}
            ]
        }"#;

        let mapped_body = build_account_scoped_upstream_body(
            "/responses",
            body,
            &account,
            CodexLocalAccessImageGenerationMode::Enabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("oauth body should build");
        let parsed: Value =
            serde_json::from_slice(mapped_body.as_ref()).expect("body should remain json");

        assert!(!has_image_generation_tool(&parsed));
        assert!(parsed.get("tool_choice").is_none());
        assert!(parsed
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|tool| {
                tool.get("type").and_then(Value::as_str) == Some("namespace")
                    && tool.get("name").and_then(Value::as_str) == Some("image_gen")
            })));
    }

    #[test]
    fn oauth_responses_removes_nested_hosted_tool_for_nested_image_gen_namespace() {
        let account = test_account_with_plan("plus");
        let body = br#"{
            "model":"gpt-5.6-sol",
            "input":[
                {
                    "type":"additional_tools",
                    "tools":[
                        {
                            "type":"namespace",
                            "namespace":"image_gen",
                            "tools":[{"type":"function","name":"imagegen","parameters":{}}]
                        }
                    ]
                }
            ],
            "response":{
                "tool_choice":{"type":"image_generation"},
                "tools":[{"type":"image_generation","output_format":"png"}]
            }
        }"#;

        let mapped_body = build_account_scoped_upstream_body(
            "/responses",
            body,
            &account,
            CodexLocalAccessImageGenerationMode::Enabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("oauth body should build");
        let parsed: Value =
            serde_json::from_slice(mapped_body.as_ref()).expect("body should remain json");

        assert_eq!(
            parsed
                .pointer("/input/0/tools/0/namespace")
                .and_then(Value::as_str),
            Some("image_gen")
        );
        assert!(parsed.pointer("/response/tool_choice").is_none());
        assert!(parsed
            .pointer("/response/tools")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty));
    }

    #[test]
    fn image_generation_header_scopes_capability_to_images_endpoints() {
        let mut headers = HashMap::new();

        assert_eq!(
            request_image_generation_mode(CodexLocalAccessImageGenerationMode::Enabled, &headers),
            CodexLocalAccessImageGenerationMode::Enabled
        );

        headers.insert(
            "X-OpenAI-Internal-Codex-Responses-Lite".to_string(),
            String::new(),
        );
        assert_eq!(
            request_image_generation_mode(CodexLocalAccessImageGenerationMode::Enabled, &headers),
            CodexLocalAccessImageGenerationMode::ImagesOnly
        );
        headers.remove("X-OpenAI-Internal-Codex-Responses-Lite");

        headers.insert(
            CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER.to_string(),
            "chat".to_string(),
        );
        assert_eq!(
            request_image_generation_mode(CodexLocalAccessImageGenerationMode::Enabled, &headers),
            CodexLocalAccessImageGenerationMode::ImagesOnly
        );
        assert_eq!(
            request_image_generation_mode(CodexLocalAccessImageGenerationMode::Disabled, &headers),
            CodexLocalAccessImageGenerationMode::Disabled
        );

        headers.insert(
            "x-agtools-disable-image-generation".to_string(),
            "true".to_string(),
        );
        assert_eq!(
            request_image_generation_mode(CodexLocalAccessImageGenerationMode::Enabled, &headers),
            CodexLocalAccessImageGenerationMode::Disabled
        );
    }

    #[test]
    fn normalizes_direct_responses_system_role_for_codex() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","input":[{"type":"message","role":"system","content":"be concise"},{"type":"message","role":"user","content":[{"type":"text","text":"hello"}]}],"tools":[{"type":"web_search_preview"}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.pointer("/input/0/role").and_then(Value::as_str),
            Some("developer")
        );
        assert_eq!(
            mapped_body
                .pointer("/input/0/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            mapped_body
                .pointer("/input/1/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            mapped_body.pointer("/tools/0/type").and_then(Value::as_str),
            Some("web_search")
        );
    }

    #[test]
    fn rewrites_snapshot_model_ids_for_chat_completions_requests() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body:
                br#"{"model":"gpt-5.4-2026-03-05","messages":[{"role":"user","content":"hello"}]}"#
                    .to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("model").and_then(Value::as_str),
            Some("gpt-5.4")
        );

        match adapter {
            GatewayResponseAdapter::ChatCompletions {
                requested_model, ..
            } => {
                assert_eq!(requested_model, "gpt-5.4");
            }
            _ => panic!("expected chat completions adapter"),
        }
    }

    #[test]
    fn drops_unsupported_sampling_params_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","temperature":0.2,"top_p":0.7,"messages":[{"role":"user","content":"hello"}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert!(mapped_body.get("temperature").is_none());
        assert!(mapped_body.get("top_p").is_none());
    }

    #[test]
    fn normalizes_text_content_parts_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        let first_type = mapped_body
            .get("input")
            .and_then(Value::as_array)
            .and_then(|messages| messages.first())
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .and_then(|parts| parts.first())
            .and_then(|part| part.get("type"))
            .and_then(Value::as_str);
        assert_eq!(first_type, Some("input_text"));
    }

    #[test]
    fn normalizes_function_tools_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","messages":[{"role":"user","content":"hello"}],"tools":[{"type":"function","function":{"name":"get_weather","description":"Get weather","parameters":{"type":"object","properties":{"location":{"type":"string"}}},"strict":true}}],"tool_choice":{"type":"function","function":{"name":"get_weather"}}}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("name"))
                .and_then(Value::as_str),
            Some("get_weather")
        );
        assert_eq!(
            mapped_body
                .get("tool_choice")
                .and_then(|choice| choice.get("name"))
                .and_then(Value::as_str),
            Some("get_weather")
        );
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("strict"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn normalizes_tool_history_messages_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"Paris\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"{\"temperature_c\":18}"}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        let input = mapped_body
            .get("input")
            .and_then(Value::as_array)
            .expect("input should be array");
        assert_eq!(
            input
                .first()
                .and_then(|item| item.get("role"))
                .and_then(Value::as_str),
            Some("user")
        );
        assert!(input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call")
                && item.get("name").and_then(Value::as_str) == Some("get_weather")
        }));
        assert!(input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call_output")
                && item.get("call_id").and_then(Value::as_str) == Some("call_1")
        }));
    }

    #[test]
    fn skips_spurious_empty_assistant_message_for_tool_calls() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"Paris\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"{\"temperature_c\":18}"}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        let input = mapped_body
            .get("input")
            .and_then(Value::as_array)
            .expect("input should be array");
        assert_eq!(input.len(), 3);
        assert_eq!(
            input
                .first()
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str),
            Some("message")
        );
        assert_eq!(
            input
                .get(1)
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str),
            Some("function_call")
        );
        assert_eq!(
            input
                .get(2)
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str),
            Some("function_call_output")
        );
    }

    #[test]
    fn builds_chat_completion_payload_from_responses_output() {
        let responses_payload = json!({
            "id": "resp_123",
            "model": "gpt-5.4",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "hello world"
                }]
            }],
            "usage": {
                "input_tokens": 7,
                "output_tokens": 3,
                "total_tokens": 10
            }
        });

        let chat_payload = build_chat_completion_payload(&responses_payload, "gpt-5.4", br#"{}"#);
        assert_eq!(
            chat_payload.get("object").and_then(Value::as_str),
            Some("chat.completion")
        );
        assert_eq!(
            chat_payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some("hello world")
        );
        assert_eq!(
            chat_payload
                .get("usage")
                .and_then(|usage| usage.get("total_tokens"))
                .and_then(Value::as_u64),
            Some(10)
        );
    }

    #[test]
    fn builds_chat_completion_payload_from_function_call_output() {
        let responses_payload = json!({
            "id": "resp_tool_1",
            "model": "gpt-5.4",
            "status": "completed",
            "output": [{
                "type": "function_call",
                "call_id": "call_abc",
                "name": "get_weather",
                "arguments": "{\"location\":\"Paris\"}"
            }]
        });

        let chat_payload = build_chat_completion_payload(&responses_payload, "gpt-5.4", br#"{}"#);
        assert_eq!(
            chat_payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("finish_reason"))
                .and_then(Value::as_str),
            Some("tool_calls")
        );
        assert_eq!(
            chat_payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("tool_calls"))
                .and_then(Value::as_array)
                .and_then(|tool_calls| tool_calls.first())
                .and_then(|tool_call| tool_call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some("get_weather")
        );
    }

    #[test]
    fn restores_shortened_tool_name_in_chat_payload() {
        let original_request = br#"{
            "model":"gpt-5.4",
            "messages":[{"role":"user","content":"run tool"}],
            "tools":[{
                "type":"function",
                "function":{
                    "name":"mcp__very_long_namespace_segment__very_long_server_name__super_long_tool_name_that_needs_shortening",
                    "description":"Long name",
                    "parameters":{"type":"object","properties":{}}
                }
            }]
        }"#;
        let responses_payload = json!({
            "id": "resp_tool_2",
            "model": "gpt-5.4",
            "status": "completed",
            "output": [{
                "type": "function_call",
                "call_id": "call_long",
                "name": "mcp__super_long_tool_name_that_needs_shortening",
                "arguments": "{}"
            }]
        });

        let chat_payload =
            build_chat_completion_payload(&responses_payload, "gpt-5.4", original_request);
        assert_eq!(
            chat_payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("tool_calls"))
                .and_then(Value::as_array)
                .and_then(|tool_calls| tool_calls.first())
                .and_then(|tool_call| tool_call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some(
                "mcp__very_long_namespace_segment__very_long_server_name__super_long_tool_name_that_needs_shortening"
            )
        );
    }

    #[test]
    fn builds_chat_completion_stream_body_with_done_marker() {
        let upstream_sse = br#"data: {"type":"response.created","response":{"id":"resp_1","created_at":123,"model":"gpt-5.4"}}

data: {"type":"response.output_text.delta","delta":"stream-body"}

event: response.done
data: {"response":{"id":"resp_1","created_at":123,"model":"gpt-5.4","status":"completed","usage":{"input_tokens":1,"input_tokens_details":{"cached_tokens":1},"output_tokens":1,"total_tokens":2}}}

"#;

        let stream_body = build_chat_completion_stream_body(upstream_sse, br#"{}"#, "gpt-5.4");
        assert!(stream_body.contains("chat.completion.chunk"));
        assert!(stream_body.contains("stream-body"));
        assert!(stream_body.contains("\"cached_tokens\":1"));
        assert!(stream_body.contains("data: [DONE]"));
    }

    #[test]
    fn parses_responses_sse_payload_to_json() {
        let sse = br#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hello "}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"world"}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","model":"gpt-5.4","status":"completed","usage":{"input_tokens":2,"output_tokens":2,"total_tokens":4}}}

data: [DONE]

"#;

        let parsed = parse_responses_payload_from_upstream(sse).expect("sse should be parsed");
        assert_eq!(
            parsed
                .get("response")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str),
            Some("resp_1")
        );
        assert_eq!(
            parsed
                .get("response")
                .and_then(|value| value.get("output_text"))
                .and_then(Value::as_str),
            Some("hello world")
        );
    }

    #[test]
    fn parses_response_done_sse_payload_to_json() {
        let sse = br#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"done body"}

event: response.done
data: {"response":{"id":"resp_done","model":"gpt-5.4","status":"completed","usage":{"input_tokens":3,"input_tokens_details":{"cached_tokens":2},"output_tokens":1,"total_tokens":4}}}

"#;

        let parsed = parse_responses_payload_from_upstream(sse).expect("sse should be parsed");
        assert_eq!(
            parsed
                .get("response")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str),
            Some("resp_done")
        );
        assert_eq!(
            parsed
                .get("response")
                .and_then(|value| value.get("usage"))
                .and_then(|value| value.get("input_tokens_details"))
                .and_then(|value| value.get("cached_tokens"))
                .and_then(Value::as_u64),
            Some(2)
        );
    }

    #[test]
    fn parses_responses_sse_response_failed_as_upstream_failure() {
        let sse = br#"event: response.failed
data: {"type":"response.failed","response":{"id":"resp_failed","error":{"code":"model_at_capacity","type":"server_error","message":"model overloaded"}}}

"#;

        let error = parse_responses_payload_from_upstream(sse).expect_err("failed event");
        assert!(error.contains("upstream_response_failed"));
        assert!(error.contains("response.failed"));
        assert!(error.contains("model_at_capacity"));
        assert!(error.contains("model overloaded"));
        assert_eq!(
            legacy_stream_error_category(&error),
            "upstream_response_failed"
        );
    }

    #[test]
    fn response_usage_collector_captures_sse_error_event() {
        let sse = br#"event: error
data: {"error":{"code":"server_error","type":"upstream","message":"stream aborted"}}

"#;

        let mut collector = ResponseUsageCollector::new(true);
        collector.feed(sse);
        let capture = collector.finish();

        let error = capture.terminal_error.expect("terminal error");
        assert!(error.contains("upstream_response_failed"));
        assert!(error.contains("server_error"));
        assert!(error.contains("stream aborted"));
    }

    #[test]
    fn resolves_backend_codex_targets_to_upstream_paths() {
        assert_eq!(
            resolve_upstream_target("/backend-api/codex/responses").unwrap(),
            "/responses"
        );
        assert_eq!(
            resolve_upstream_target("/backend-api/codex/responses/compact").unwrap(),
            "/responses/compact"
        );
        assert_eq!(
            resolve_upstream_target("/v1/responses?debug=1").unwrap(),
            "/responses?debug=1"
        );
    }

    #[test]
    fn aligns_prompt_cache_key_with_session_id() {
        let api_key = ResolvedLocalApiKey {
            id: "client-key-1".to_string(),
            label: "Client".to_string(),
            provider_gateway: None,
            inherit_account_pool: true,
            account_ids: Vec::new(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };
        let mut request = ParsedRequest {
            method: "POST".to_string(),
            target: "/backend-api/codex/responses".to_string(),
            headers: HashMap::new(),
            body: serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello",
                "prompt_cache_key": "cache-123",
            }))
            .unwrap(),
        };

        align_codex_prompt_cache(&mut request, &api_key).unwrap();
        let body = serde_json::from_slice::<Value>(&request.body).unwrap();
        assert_eq!(
            request.headers.get("session-id").map(String::as_str),
            Some("cache-123")
        );
        assert_eq!(
            request.headers.get("conversation_id").map(String::as_str),
            Some("cache-123")
        );
        assert_eq!(
            body.get("prompt_cache_key").and_then(Value::as_str),
            Some("cache-123")
        );
    }

    #[test]
    fn legacy_codex_metadata_aligns_with_prompt_cache_key() {
        let api_key = ResolvedLocalApiKey {
            id: "client-key-1".to_string(),
            label: "Client".to_string(),
            provider_gateway: None,
            inherit_account_pool: true,
            account_ids: Vec::new(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };
        let mut request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello",
                "prompt_cache_key": "cache-123",
            }))
            .unwrap(),
        };

        align_codex_prompt_cache(&mut request, &api_key).unwrap();
        let body = serde_json::from_slice::<Value>(&request.body).unwrap();
        let metadata = body
            .get("client_metadata")
            .and_then(Value::as_object)
            .expect("client_metadata should be present");
        assert_eq!(
            metadata.get("x-codex-window-id").and_then(Value::as_str),
            Some("cache-123:0")
        );
        assert!(metadata
            .get("x-codex-installation-id")
            .and_then(Value::as_str)
            .is_some());
        let turn_metadata = metadata
            .get("x-codex-turn-metadata")
            .and_then(Value::as_str)
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
            .expect("turn metadata should be json");
        assert_eq!(
            turn_metadata
                .get("prompt_cache_key")
                .and_then(Value::as_str),
            Some("cache-123")
        );
        assert_eq!(
            turn_metadata.get("window_id").and_then(Value::as_str),
            Some("cache-123:0")
        );
        assert_eq!(
            request
                .headers
                .get("x-client-request-id")
                .map(String::as_str),
            Some("cache-123")
        );
        assert_eq!(
            request.headers.get("thread-id").map(String::as_str),
            Some("cache-123")
        );
        assert_eq!(
            request.headers.get("x-codex-window-id").map(String::as_str),
            Some("cache-123:0")
        );
        assert_eq!(
            request
                .headers
                .get("x-codex-turn-metadata")
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .and_then(|value| {
                    value
                        .get("prompt_cache_key")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .as_deref(),
            Some("cache-123")
        );
    }

    #[test]
    fn legacy_websocket_initial_requests_inject_default_priority_service_tier() {
        let api_key = ResolvedLocalApiKey {
            id: "client-key-1".to_string(),
            label: "Client".to_string(),
            provider_gateway: None,
            inherit_account_pool: true,
            account_ids: Vec::new(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };
        let cases = [
            (
                br#"{"model":"gpt-5.4","input":"hello"}"#.as_slice(),
                None,
                None,
            ),
            (
                br#"{"model":"gpt-5.4","stream":true,"reasoning":{"effort":"low"},"input":"hello"}"#
                    .as_slice(),
                Some(true),
                Some("low"),
            ),
        ];

        for (request_body, expected_stream, expected_effort) in cases {
            let mut request = ParsedRequest {
                method: "GET".to_string(),
                target: "/v1/responses".to_string(),
                headers: HashMap::new(),
                body: request_body.to_vec(),
            };

            prepare_websocket_initial_request(&mut request, &api_key, Some("priority"))
                .expect("websocket request should map");
            let body = serde_json::from_slice::<Value>(&request.body).unwrap();
            assert_eq!(
                body.get("service_tier").and_then(Value::as_str),
                Some("priority")
            );
            assert_eq!(
                body.get("type").and_then(Value::as_str),
                Some("response.create")
            );
            if let Some(expected_stream) = expected_stream {
                assert_eq!(
                    body.get("stream").and_then(Value::as_bool),
                    Some(expected_stream)
                );
            }
            if let Some(expected_effort) = expected_effort {
                assert_eq!(
                    body.get("reasoning")
                        .and_then(|reasoning| reasoning.get("effort"))
                        .and_then(Value::as_str),
                    Some(expected_effort)
                );
            }
        }
    }

    #[test]
    fn legacy_codex_sanitizes_invalid_reasoning_encrypted_content() {
        let api_key = ResolvedLocalApiKey {
            id: "client-key-1".to_string(),
            label: "Client".to_string(),
            provider_gateway: None,
            inherit_account_pool: true,
            account_ids: Vec::new(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };
        let mut valid_signature_bytes = vec![0x80];
        valid_signature_bytes.extend([0u8; 8]);
        valid_signature_bytes.extend([1u8; 16]);
        valid_signature_bytes.extend([2u8; 16]);
        valid_signature_bytes.extend([3u8; 32]);
        let valid_signature = general_purpose::URL_SAFE_NO_PAD.encode(valid_signature_bytes);
        let mut request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [
                    {
                        "id": "rs_bad",
                        "type": "reasoning",
                        "encrypted_content": " not-a-valid-signature "
                    },
                    {
                        "id": "rs_null",
                        "type": "reasoning",
                        "encrypted_content": null
                    },
                    {
                        "id": "rs_good",
                        "type": "reasoning",
                        "encrypted_content": valid_signature
                    },
                    {
                        "role": "user",
                        "content": "hello"
                    }
                ],
                "prompt_cache_key": "cache-123"
            }))
            .unwrap(),
        };

        align_codex_prompt_cache(&mut request, &api_key).unwrap();
        let body = serde_json::from_slice::<Value>(&request.body).unwrap();
        let input = body.get("input").and_then(Value::as_array).unwrap();
        assert!(input[0].get("encrypted_content").is_none());
        assert!(input[1].get("encrypted_content").is_none());
        assert_eq!(
            input[2].get("encrypted_content").and_then(Value::as_str),
            Some(valid_signature.as_str())
        );
    }

    #[test]
    fn applies_codex_official_empty_headers() {
        let mut request = ParsedRequest {
            method: "POST".to_string(),
            target: "/backend-api/codex/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","input":"hello"}"#.to_vec(),
        };

        apply_codex_official_headers(&mut request);

        for key in [
            "version",
            "x-codex-turn-state",
            "x-codex-turn-metadata",
            "x-client-request-id",
            "x-responsesapi-include-timing-metrics",
            "session-id",
            "thread-id",
            "x-codex-window-id",
        ] {
            assert_eq!(request.headers.get(key).map(String::as_str), Some(""));
        }
    }

    #[test]
    fn default_codex_identity_headers_match_official_tui() {
        assert!(super::DEFAULT_CODEX_USER_AGENT.starts_with("codex-tui/0.153.4"));
        assert_eq!(super::DEFAULT_CODEX_ORIGINATOR, "codex-tui");
        assert!(super::DEFAULT_CODEX_USER_AGENT.contains("(codex-tui; 0.153.4)"));
        assert!(!super::DEFAULT_CODEX_USER_AGENT.contains("codex_cli_rs"));
    }

    #[test]
    fn parses_websocket_usage_limit_error() {
        let message = Message::Text(
            r#"{"type":"error","status":429,"body":{"error":{"type":"usage_limit_reached","message":"usage limit reached","resets_in_seconds":7}}}"#
                .into(),
        );

        let error = parse_websocket_upstream_error(&message).expect("error should parse");

        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS.as_u16());
        assert_eq!(error.category, "usage_limit_reached");
        assert_eq!(error.retry_after, Some(Duration::from_secs(7)));
        assert!(error.body.contains("usage_limit_reached"));
    }

    #[test]
    fn parses_websocket_connection_limit_error() {
        let message = Message::Text(
            r#"{"type":"error","status":429,"body":{"error":{"code":"websocket_connection_limit_reached","type":"server_error","message":"too many websocket connections"}},"headers":{"retry-after":"1"}}"#
                .into(),
        );

        let error = parse_websocket_upstream_error(&message).expect("error should parse");

        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS.as_u16());
        assert_eq!(error.category, "websocket_connection_limit_reached");
        assert_eq!(error.retry_after, Some(Duration::from_secs(1)));
        assert!(error.body.contains("websocket_connection_limit_reached"));
    }

    #[test]
    fn websocket_handshake_unauthorized_is_auth_unavailable() {
        let error = websocket_connect_error_from_http_response(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"type":"invalid_token","message":"bad access token"}}"#.to_string(),
        );

        assert_eq!(error.status, Some(StatusCode::UNAUTHORIZED.as_u16()));
        assert_eq!(error.category, "auth_unavailable");
        assert!(error.message.contains("bad access token"));
    }

    #[test]
    fn api_key_accounts_are_eligible_for_local_access_pool() {
        let account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );

        assert!(is_local_access_eligible_account(&account, true));
        assert_eq!(
            account_upstream_base_url(&account),
            "https://relay.example/v1"
        );
    }

    #[test]
    fn deepseek_responses_api_key_accounts_are_not_eligible_for_local_access_pool() {
        let mut account = CodexAccount::new_api_key(
            "deepseek-1".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-flash".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());

        assert!(!is_local_access_eligible_account(&account, false));
        assert_eq!(
            local_access_ineligible_reason(&account, false),
            Some("deepseek_unsupported")
        );
        let (_, synced_ids, added_ids, skipped) = append_eligible_local_access_account_ids(
            &[],
            vec![account.id.clone()],
            &[account.clone()],
            false,
        );
        assert!(synced_ids.is_empty());
        assert!(added_ids.is_empty());
        assert_eq!(
            skipped
                .iter()
                .map(|item| (item.account_id.as_str(), item.reason.as_str()))
                .collect::<Vec<_>>(),
            vec![("deepseek-1", "deepseek_unsupported")]
        );
    }

    #[test]
    fn chat_completions_api_key_accounts_are_not_eligible_for_local_access_pool() {
        let mut account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.minimax.io/v1".to_string()),
            Some("minimax".to_string()),
            Some("MiniMax".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("chat_completions".to_string());

        assert!(!is_local_access_eligible_account(&account, false));
    }

    #[test]
    fn chat_completions_api_key_accounts_are_eligible_for_provider_gateway() {
        let mut account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("chat_completions".to_string());

        assert!(!is_local_access_eligible_account(&account, false));
        assert!(is_provider_gateway_eligible_account(&account));
    }

    fn model_provider_chat_test_request(
        wire_api: &str,
    ) -> CodexModelProviderGatewayChatTestRequest {
        CodexModelProviderGatewayChatTestRequest {
            run_id: "run-1".to_string(),
            provider_id: "provider-1".to_string(),
            provider_name: "Provider".to_string(),
            base_url: "https://relay.example/v1".to_string(),
            api_key_id: Some("key-1".to_string()),
            api_key_name: Some("Key".to_string()),
            api_key: "sk-test".to_string(),
            wire_api: wire_api.to_string(),
            model_catalog: vec!["upstream-model".to_string()],
            model_id: "upstream-model".to_string(),
            prompt: "hello".to_string(),
        }
    }

    #[test]
    fn model_provider_chat_test_uses_provider_gateway_only_for_chat_protocol() {
        assert!(model_provider_test_uses_provider_gateway(
            &model_provider_chat_test_request("chat_completions")
        ));
        assert!(!model_provider_test_uses_provider_gateway(
            &model_provider_chat_test_request("responses")
        ));
    }

    #[test]
    fn model_provider_direct_test_client_model_is_codex_visible() {
        let client_model = model_provider_direct_test_client_model();

        assert!(
            supported_codex_model_ids()
                .iter()
                .any(|model| model.eq_ignore_ascii_case(&client_model)),
            "direct test client model should pass sidecar Codex model visibility"
        );
    }

    #[test]
    fn supported_codex_models_include_official_and_compatibility_models() {
        let models = supported_codex_model_ids();

        for model_id in [
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-6-astra",
            "gpt-5.3-codex",
            "gpt-5.3-codex-spark",
        ] {
            assert!(
                models.iter().any(|model| model == model_id),
                "missing official model {model_id}"
            );
        }
    }

    #[test]
    fn default_codex_models_include_compatibility_5_3_models() {
        assert_eq!(
            default_codex_model_ids(),
            vec![
                "gpt-6-astra",
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.5",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex",
                "gpt-5.3-codex-spark",
            ]
        );
    }
