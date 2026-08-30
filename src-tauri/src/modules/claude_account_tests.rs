// Claude 账号测试：OAuth、桌面 profile、cookie 和账号合并行为。
// 测试内容作为原 tests 模块的内部实现，super 引用和 cfg 条件保持不变。
use super::*;

    fn cloudflare_challenge_profile() -> Value {
        serde_json::json!({
            "fetchContext": "cookie_direct",
            "errors": {
                "organizationUsage": "HTTP 403 Cloudflare challenge-platform cf-ray=test"
            }
        })
    }

    fn successful_usage_profile(fetch_context: &str) -> Value {
        serde_json::json!({
            "fetchContext": fetch_context,
            "endpoints": {
                "organizationUsage": {
                    "five_hour": { "utilization": 42 },
                    "seven_day": { "utilization": 18 }
                }
            }
        })
    }

    #[tokio::test]
    async fn cloudflare_challenge_uses_hidden_electron_probe_result() {
        let direct = cloudflare_challenge_profile();
        let probed = successful_usage_profile("page");
        let expected = probed.clone();
        let probe_called = std::cell::Cell::new(false);

        let resolved = resolve_desktop_web_profile_with_hidden_probe(
            "claude-desktop-test",
            Ok(direct),
            || {
                probe_called.set(true);
                std::future::ready(Ok(probed))
            },
        )
        .await
        .expect("page-context probe should recover the usage profile");

        assert!(probe_called.get());
        assert_eq!(resolved, expected);
        assert!(desktop_web_usage_to_quota(&resolved).is_some());
    }

    #[tokio::test]
    async fn successful_silent_profile_does_not_launch_hidden_probe() {
        let direct = successful_usage_profile("cookie_direct");
        let expected = direct.clone();
        let probe_called = std::cell::Cell::new(false);

        let resolved = resolve_desktop_web_profile_with_hidden_probe(
            "claude-desktop-test",
            Ok(direct),
            || {
                probe_called.set(true);
                std::future::ready(Err("unexpected hidden probe".to_string()))
            },
        )
        .await
        .expect("successful silent refresh should be preserved");

        assert!(!probe_called.get());
        assert_eq!(resolved, expected);
    }

    #[tokio::test]
    async fn failed_or_cooled_down_hidden_probe_preserves_challenge_profile() {
        let direct = cloudflare_challenge_profile();
        let expected = direct.clone();

        let resolved = resolve_desktop_web_profile_with_hidden_probe(
            "claude-desktop-test",
            Ok(direct),
            || {
                std::future::ready(Err(
                    "hidden Electron refresh is in its 600 second cooldown".to_string()
                ))
            },
        )
        .await
        .expect("a failed fallback should retain the direct refresh diagnostics");

        assert_eq!(resolved, expected);
        assert!(desktop_web_profile_has_cloudflare_challenge(&resolved));
    }

    #[test]
    fn hidden_probe_cooldown_is_deterministic_per_account() {
        let mut attempts = HashMap::new();
        let started_at = Instant::now();
        let cooldown = Duration::from_secs(600);

        assert!(should_attempt_desktop_hidden_probe_at(
            &mut attempts,
            "account-a",
            started_at,
            cooldown,
        ));
        assert!(!should_attempt_desktop_hidden_probe_at(
            &mut attempts,
            "account-a",
            started_at + Duration::from_secs(599),
            cooldown,
        ));
        assert!(should_attempt_desktop_hidden_probe_at(
            &mut attempts,
            "account-b",
            started_at + Duration::from_secs(599),
            cooldown,
        ));
        assert!(should_attempt_desktop_hidden_probe_at(
            &mut attempts,
            "account-a",
            started_at + cooldown,
            cooldown,
        ));
    }

    #[test]
    fn rejects_oauth_authorize_url_as_callback_input() {
        let error = parse_oauth_callback_input(
            "https://claude.com/cai/oauth/authorize?code=true&client_id=test-client",
        )
        .expect_err("authorize entry URL should not be accepted as callback code");

        assert!(error.contains("授权入口链接"));
    }

    #[test]
    fn parses_oauth_callback_url_with_state() {
        let (code, state) = parse_oauth_callback_input(
            "https://platform.claude.com/oauth/code/callback?code=actual-code&state=state-1",
        )
        .expect("callback URL should parse");

        assert_eq!(code, "actual-code");
        assert_eq!(state.as_deref(), Some("state-1"));
    }

    #[test]
    fn slims_claude_code_config_snapshot_to_switch_required_fields() {
        let full_config = serde_json::json!({
            "oauthAccount": {
                "emailAddress": "alice@testmail.dev",
                "accountUuid": "b55de31d-da47-4433-9a73-bbba05affeeb"
            },
            "email": "alice@testmail.dev",
            "hasCompletedOnboarding": true,
            "cachedGrowthBookFeatures": {
                "tengu_amber_lattice": {
                    "plugins": ["security-guidance", "code-review"]
                }
            },
            "cachedDynamicConfigs": {
                "tengu-top-of-feed-tip": {
                    "color": "warning",
                    "tip": "large cached payload"
                }
            }
        });

        let slimmed = slim_claude_code_config_snapshot(&full_config);

        assert!(slimmed.get("oauthAccount").is_some());
        assert_eq!(
            read_string_path(&slimmed, &["oauthAccount", "emailAddress"]).as_deref(),
            Some("alice@testmail.dev")
        );
        assert_eq!(
            read_string_path(&slimmed, &["email"]).as_deref(),
            Some("alice@testmail.dev")
        );
        assert_eq!(
            read_bool_path(&slimmed, &["hasCompletedOnboarding"]),
            Some(true)
        );
        assert!(slimmed.get("cachedGrowthBookFeatures").is_none());
        assert!(slimmed.get("cachedDynamicConfigs").is_none());
    }

    #[test]
    fn slims_only_claude_cli_oauth_account_snapshots() {
        let config = serde_json::json!({
            "oauthAccount": {
                "emailAddress": "alice@testmail.dev"
            },
            "cachedGrowthBookFeatures": {
                "large": true
            }
        });
        let mut account = test_desktop_account(
            "claude_desktop",
            "alice@testmail.dev",
            None,
            Some("/tmp/snapshot"),
            10,
            20,
        );
        account.claude_config_raw = Some(config.clone());
        assert!(!slim_claude_account_snapshots(&mut account));
        assert_eq!(account.claude_config_raw.as_ref(), Some(&config));

        account.auth_mode = ClaudeAuthMode::OAuth;
        assert!(slim_claude_account_snapshots(&mut account));
        let slimmed = account.claude_config_raw.as_ref().expect("slimmed config");
        assert!(slimmed.get("oauthAccount").is_some());
        assert!(slimmed.get("cachedGrowthBookFeatures").is_none());
    }

    #[test]
    fn rejects_desktop_oauth_json_import() {
        let error = parse_import_item(&serde_json::json!({
            "id": "claude_desktop_alice",
            "email": "alice@testmail.dev",
            "auth_mode": "desktop_oauth",
            "desktop_profile_dir": "/tmp/claude-desktop-snapshot",
            "claude_credentials_raw": {
                "authMode": "desktop_oauth",
                "profileSnapshot": true
            },
            "claude_config_raw": {
                "desktopProfile": {
                    "snapshotDir": "/tmp/claude-desktop-snapshot"
                }
            },
            "created_at": 10,
            "last_used": 20
        }))
        .expect_err("desktop oauth account JSON should be rejected");

        assert!(error.contains("不支持 JSON 导入"));
    }

    #[test]
    fn derives_oauth_plan_from_subscription_type_before_billing_source() {
        let credentials = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-test",
                "refreshToken": "sk-ant-ort01-test",
                "subscriptionType": "Pro",
                "profile": {
                    "account": {
                        "has_claude_pro": true,
                        "has_claude_max": false
                    },
                    "organization": {
                        "organization_type": "claude_pro",
                        "billing_type": "apple_subscription"
                    }
                }
            }
        });
        let config = serde_json::json!({
            "oauthAccount": {
                "emailAddress": "alice@testmail.dev",
                "accountUuid": "b55de31d-da47-4433-9a73-bbba05affeeb",
                "organizationUuid": "d6faab9e-25dc-4d42-bce1-08f2dfe21bf6",
                "billingType": "apple_subscription",
                "organizationType": "claude_pro",
                "subscriptionType": "Pro"
            }
        });

        let account = derive_account_from_snapshots(credentials, config, None)
            .expect("account should be derived");

        assert_eq!(account.plan_type.as_deref(), Some("Pro"));
    }

    #[test]
    fn normalizes_existing_oauth_plan_from_billing_source_to_subscription() {
        let credentials = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-test",
                "subscriptionType": "Pro"
            }
        });
        let config = serde_json::json!({
            "oauthAccount": {
                "emailAddress": "alice@testmail.dev",
                "billingType": "apple_subscription",
                "subscriptionType": "Pro"
            }
        });
        let mut account = derive_account_from_snapshots(credentials, config, None)
            .expect("account should be derived");
        account.plan_type = Some("apple_subscription".to_string());

        assert!(normalize_account_plan_from_snapshots(&mut account));
        assert_eq!(account.plan_type.as_deref(), Some("Pro"));
    }

    #[test]
    fn extracts_desktop_local_profile_from_indexeddb_blob_text() {
        let blob = br#"
            datao" accounto" tagged_id" user_abc"
            uuid"$b55de31d-da47-4433-9a73-bbba05affeeb"
            email_address" alice@testmail.dev"
            full_name" Alice Chen"
            display_name" Alice"
            membershipsA o" organizationo" idI"
            uuid"$d6faab9e-25dc-4d42-bce1-08f2dfe21bf6"
            name" Alice Workspace"
            settings
        "#;

        let profile = extract_desktop_local_profile_from_bytes(Path::new("IndexedDB/blob/1"), blob)
            .expect("profile should be extracted");

        assert_eq!(profile.email.as_deref(), Some("alice@testmail.dev"));
        assert_eq!(
            profile.account_uuid.as_deref(),
            Some("b55de31d-da47-4433-9a73-bbba05affeeb")
        );
        assert_eq!(profile.display_name.as_deref(), Some("Alice"));
        assert_eq!(profile.full_name.as_deref(), Some("Alice Chen"));
        assert_eq!(
            profile.organization_uuid.as_deref(),
            Some("d6faab9e-25dc-4d42-bce1-08f2dfe21bf6")
        );
        assert_eq!(
            profile.organization_name.as_deref(),
            Some("Alice Workspace")
        );
    }

    #[test]
    fn extracts_desktop_subscription_and_usage_from_web_profile() {
        let profile = serde_json::json!({
            "fetchedAt": "2026-06-13T12:00:00Z",
            "endpoints": {
                "accountProfile": {
                    "account": {
                        "email_address": "alice@testmail.dev",
                        "uuid": "b55de31d-da47-4433-9a73-bbba05affeeb"
                    }
                },
                "subscriptionDetails": {
                    "plan_type": "claude_max_20x"
                },
                "organizationUsage": {
                    "five_hour": {
                        "utilization": 42,
                        "resets_at": "2026-06-13T17:00:00Z"
                    },
                    "sevenDay": {
                        "utilization": 0.88,
                        "resetsAt": 1781366400
                    },
                    "seven_day_sonnet": {
                        "utilization": 12,
                        "resets_at": "2026-06-14T09:00:00Z"
                    }
                }
            }
        });

        let summary = desktop_web_profile_summary(&profile);
        assert_eq!(
            read_string_path(&summary, &["email"]).as_deref(),
            Some("alice@testmail.dev")
        );
        assert_eq!(
            read_string_path(&summary, &["planType"]).as_deref(),
            Some("Max 20x")
        );

        let quota = desktop_web_usage_to_quota(&profile).expect("usage should produce quota");
        assert_eq!(quota.five_hour_percentage, 42);
        assert_eq!(quota.seven_day_percentage, 88);
        assert_eq!(quota.seven_day_sonnet_percentage, Some(12));
        assert!(quota.five_hour_reset_time.is_some());
        assert!(quota.seven_day_sonnet_reset_time.is_some());
    }

    #[test]
    fn desktop_usage_percentage_one_is_one_percent() {
        let profile = serde_json::json!({
            "endpoints": {
                "organizationUsage": {
                    "five_hour": {
                        "utilization": 1,
                        "resets_at": "2026-06-17T19:20:00Z"
                    },
                    "sevenDay": {
                        "utilization": 0.01,
                        "resetsAt": 1781366400
                    }
                }
            }
        });

        let quota = desktop_web_usage_to_quota(&profile).expect("usage should produce quota");
        assert_eq!(quota.five_hour_percentage, 1);
        assert_eq!(quota.seven_day_percentage, 1);
    }

    #[test]
    fn maps_default_claude_rate_limit_tier_to_free_plan() {
        let profile = serde_json::json!({
            "endpoints": {
                "account": {
                    "email_address": "alice@testmail.dev",
                    "memberships": [
                        {
                            "organization": {
                                "rate_limit_tier": "default_claude_ai",
                                "rate_limit_upsell": "upgrade_to_pro"
                            }
                        }
                    ]
                }
            }
        });

        let summary = desktop_web_profile_summary(&profile);
        assert_eq!(
            read_string_path(&summary, &["planType"]).as_deref(),
            Some("Free")
        );
        assert_eq!(
            read_string_path(&summary, &["rawPlan"]).as_deref(),
            Some("default_claude_ai")
        );
    }

    #[test]
    fn extracts_desktop_profile_snapshot_id_from_legacy_paths() {
        let snapshot_id = "claude_desktop_0b1d3d4df02c2376d62a623bb8c67332";
        assert_eq!(
            desktop_profile_snapshot_id_from_path(Path::new(
                r"C:\Users\Lenovo\.antigravity_cockpit\claude_desktop_profiles\claude_desktop_0b1d3d4df02c2376d62a623bb8c67332"
            ))
            .as_deref(),
            Some(snapshot_id)
        );
        assert_eq!(
            desktop_profile_snapshot_id_from_path(Path::new(
                r"C:\Users\Lenovo.antigravity_cockpit\claude_desktop_profiles\claude_desktop_0b1d3d4df02c2376d62a623bb8c67332"
            ))
            .as_deref(),
            Some(snapshot_id)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn decrypts_chromium_v10_cookie_with_host_digest_prefix() {
        let encrypted = test_hex_to_bytes(
            "763130cba8d8b3b813f784aae46dea9258b58b3d19f5f789dc4778df01527afd73e93eaa0590f58c4d6b38d78e1aa843ee5a3cebf07ae55d7ce19bb941b6b37c668fc5",
        );
        let value = decrypt_chromium_v10_cookie(".claude.ai", &encrypted, "test-password")
            .expect("cookie should decrypt");
        assert_eq!(value, "session-test-value");
    }

    #[cfg(target_os = "macos")]
    fn test_hex_to_bytes(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks(2)
            .map(|chunk| {
                let text = std::str::from_utf8(chunk).expect("valid hex");
                u8::from_str_radix(text, 16).expect("valid hex byte")
            })
            .collect()
    }

    fn test_desktop_account(
        id: &str,
        email: &str,
        account_uuid: Option<&str>,
        snapshot_dir: Option<&str>,
        created_at: i64,
        last_used: i64,
    ) -> ClaudeAccount {
        ClaudeAccount {
            id: id.to_string(),
            email: email.to_string(),
            auth_mode: ClaudeAuthMode::DesktopOAuth,
            account_uuid: account_uuid.map(ToString::to_string),
            organization_uuid: None,
            organization_name: None,
            plan_type: None,
            avatar_url: None,
            profile_updated_at: None,
            quota: None,
            quota_error: None,
            usage_updated_at: None,
            status: None,
            status_reason: None,
            api_key: None,
            api_base_url: None,
            api_provider_id: None,
            api_provider_name: None,
            api_provider_source_tag: None,
            api_provider_website: None,
            api_provider_api_key_url: None,
            api_key_field: None,
            api_model_catalog: None,
            api_extra_env: None,
            desktop_gateway_auth_scheme: None,
            desktop_gateway_credential_kind: None,
            desktop_gateway_config_id: None,
            desktop_gateway_profile_dir: None,
            desktop_gateway_models: None,
            desktop_gateway_connection_mode: None,
            desktop_gateway_upstream_models: None,
            desktop_gateway_model_mappings: None,
            desktop_profile_dir: snapshot_dir.map(ToString::to_string),
            desktop_profile_imported_at: Some(last_used),
            claude_credentials_raw: None,
            claude_config_raw: None,
            claude_usage_raw: None,
            tags: None,
            account_note: None,
            created_at,
            last_used,
        }
    }

    #[test]
    fn merges_same_desktop_identity_without_touching_non_desktop_accounts() {
        let mut base = test_desktop_account(
            "claude_desktop_old",
            "Claude",
            Some("B55DE31D-DA47-4433-9A73-BBBA05AFFEEB"),
            Some("/tmp/old-snapshot"),
            10,
            20,
        );
        base.tags = Some(vec!["work".to_string()]);
        base.plan_type = Some("Claude".to_string());

        let mut incoming = test_desktop_account(
            "claude_desktop_new",
            "alice@testmail.dev",
            Some("b55de31d-da47-4433-9a73-bbba05affeeb"),
            Some("/tmp/new-snapshot"),
            30,
            40,
        );
        incoming.organization_uuid = Some("org-1".to_string());
        incoming.organization_name = Some("Alice Workspace".to_string());
        incoming.plan_type = Some("Max 20x".to_string());
        incoming.avatar_url = Some("https://example.test/avatar.png".to_string());
        incoming.tags = Some(vec!["work".to_string(), "max".to_string()]);

        assert!(desktop_accounts_same_identity(&base, &incoming));

        let mut oauth_account = incoming.clone();
        oauth_account.auth_mode = ClaudeAuthMode::OAuth;
        assert!(!desktop_accounts_same_identity(&base, &oauth_account));

        let merged = merge_desktop_account_fields(&base, &incoming);
        assert_eq!(merged.id, base.id);
        assert_eq!(merged.email, "alice@testmail.dev");
        assert_eq!(
            merged.account_uuid.as_deref(),
            Some("b55de31d-da47-4433-9a73-bbba05affeeb")
        );
        assert_eq!(merged.organization_uuid.as_deref(), Some("org-1"));
        assert_eq!(merged.organization_name.as_deref(), Some("Alice Workspace"));
        assert_eq!(merged.plan_type.as_deref(), Some("Max 20x"));
        assert_eq!(
            merged.avatar_url.as_deref(),
            Some("https://example.test/avatar.png")
        );
        assert_eq!(
            merged.desktop_profile_dir.as_deref(),
            Some("/tmp/new-snapshot")
        );
        assert_eq!(merged.created_at, 10);
        assert_eq!(merged.last_used, 40);
        assert_eq!(
            merged.tags,
            Some(vec!["max".to_string(), "work".to_string()])
        );
    }
