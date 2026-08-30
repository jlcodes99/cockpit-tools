// Trae 账号测试：平台识别、导入、加密存储和 Token 刷新窗口。
// 测试作为原 tests 模块内容被 include，super 引用保持不变。
use super::*;
    use crate::models::trae::TraeAccount;

    fn sample_account() -> TraeAccount {
        TraeAccount {
            id: "trae_test".to_string(),
            email: "lijie769328281@gmail.com".to_string(),
            user_id: Some("7463021402682639361".to_string()),
            nickname: Some("李杰".to_string()),
            tags: None,
            access_token: "old-access".to_string(),
            refresh_token: Some("old-refresh".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(1_777_220_302),
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: None,
            trae_profile_raw: Some(serde_json::json!({
                "Result": {
                    "ScreenName": "李杰",
                    "NonPlainTextEmail": "lijie769328281@gmail.com",
                    "UserID": "7463021402682639361",
                    "AvatarUrl": "https://example.com/avatar.png",
                    "Description": "",
                    "StoreCountry": "jp",
                    "StoreCountrySrc": "uid",
                    "AIRegion": "SG",
                }
            })),
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: Some("row".to_string()),
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: 0,
            last_used: 0,
        }
    }

    fn runtime_payload_with_auth(auth_raw: Value) -> TraeImportPayload {
        TraeImportPayload {
            email: "lijie769328281@gmail.com".to_string(),
            user_id: Some("7463021402682639361".to_string()),
            nickname: None,
            access_token: "new-access".to_string(),
            refresh_token: Some("new-refresh".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(1_800_000_000),
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: Some(auth_raw),
            trae_profile_raw: None,
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: None,
            status: None,
            status_reason: None,
        }
    }

    #[test]
    fn runtime_session_merge_preserves_oauth_platform_and_device_context() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "platformId": "trae_solo_cn",
            "authClientId": TRAE_SOLO_AUTH_CLIENT_ID,
            "host": "https://api.trae.com.cn",
            "callbackQuery": { "scope": "solo" },
            "deviceInfo": { "PlatformCode": "SOLO_PC", "DeviceID": "device-new" },
            "deviceKeyPair": {
                "privateKeyPEM": "private-new",
                "publicKeyPEM": "public-new"
            },
            "exchangeResponse": { "Result": { "ClientID": TRAE_SOLO_AUTH_CLIENT_ID } },
            "nested": { "keep": "oauth" }
        }));
        let payload = runtime_payload_with_auth(serde_json::json!({
            "platformId": "trae_solo_cn",
            "authClientId": TRAE_SOLO_AUTH_CLIENT_ID,
            "host": "https://api.trae.com.cn",
            "callbackQuery": { "scope": "stale" },
            "deviceInfo": { "PlatformCode": "SOLO_PC", "DeviceID": "device-stale" },
            "deviceKeyPair": {
                "privateKeyPEM": "private-stale",
                "publicKeyPEM": "public-stale"
            },
            "exchangeResponse": { "Result": { "ClientID": "stale-client" } },
            "nested": { "runtime": "merged" }
        }));

        assert!(runtime_payload_matches_account_platform(&account, &payload));
        apply_runtime_session_payload(&mut account, payload);

        let auth = account.trae_auth_raw.as_ref().expect("merged auth");
        assert_eq!(account.access_token, "new-access");
        assert_eq!(account.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(auth["platformId"], "trae_solo_cn");
        assert_eq!(auth["callbackQuery"]["scope"], "solo");
        assert_eq!(auth["deviceInfo"]["DeviceID"], "device-new");
        assert_eq!(auth["deviceKeyPair"]["publicKeyPEM"], "public-new");
        assert_eq!(
            auth["exchangeResponse"]["Result"]["ClientID"],
            TRAE_SOLO_AUTH_CLIENT_ID
        );
        assert_eq!(auth["nested"]["keep"], "oauth");
        assert_eq!(auth["nested"]["runtime"], "merged");
    }

    #[test]
    fn runtime_session_rejects_snapshot_from_another_trae_platform() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "platformId": "trae_solo_cn",
            "authClientId": TRAE_SOLO_AUTH_CLIENT_ID,
            "host": "https://api.trae.com.cn"
        }));
        let payload = runtime_payload_with_auth(serde_json::json!({
            "platformId": "trae_cn",
            "authClientId": TRAE_AUTH_CLIENT_ID,
            "host": "https://api.trae.cn"
        }));

        assert!(!runtime_payload_matches_account_platform(
            &account, &payload
        ));
    }

    #[test]
    fn non_running_storage_must_be_newer_than_saved_credentials() {
        let base = std::time::UNIX_EPOCH + std::time::Duration::from_secs(100);
        assert!(!runtime_storage_is_newer(
            base + std::time::Duration::from_secs(10),
            base
        ));
        assert!(!runtime_storage_is_newer(base, base));
        assert!(runtime_storage_is_newer(
            base,
            base + std::time::Duration::from_secs(1)
        ));
    }

    #[test]
    fn account_identity_match_uses_user_id_as_primary_key() {
        let mut account = sample_account();
        account.email = "same@example.com".to_string();
        account.user_id = Some("uid-a".to_string());

        assert!(account_matches_import_identity(
            &account,
            Some("uid-a"),
            Some("same@example.com")
        ));
        assert!(!account_matches_import_identity(
            &account,
            Some("uid-b"),
            Some("same@example.com")
        ));
    }

    #[test]
    fn account_identity_match_falls_back_to_email_only_when_needed() {
        let mut account = sample_account();
        account.email = "fallback@example.com".to_string();
        account.user_id = Some("uid-a".to_string());

        assert!(account_matches_import_identity(
            &account,
            None,
            Some("fallback@example.com")
        ));
    }

    #[test]
    fn apply_exchange_response_preserves_existing_auth_context() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "host": "https://api-sg-central.trae.ai",
            "loginHost": "https://api-sg-central.trae.ai",
            "refreshExpiredAt": "2026-10-09T16:18:22.466Z",
            "tokenReleaseAt": "2026-04-12T16:18:25.030Z",
            "account": {
                "username": "李杰"
            }
        }));

        let response = serde_json::json!({
            "Result": {
                "Token": "new-access",
                "RefreshToken": "new-refresh",
                "TokenType": "Bearer",
                "TokenExpireAt": 1777220302466_u64,
                "RefreshExpireAt": 1791562702466_u64
            }
        });
        let context = TraeRefreshRoutingContext {
            platform: TraePlatformKind::Trae,
            client_id: TRAE_AUTH_CLIENT_ID.to_string(),
            login_host: "https://growsg-normal.trae.ai".to_string(),
            login_region: Some("sg".to_string()),
            store_region: Some("SG".to_string()),
            ai_region: Some("SG".to_string()),
        };

        apply_exchange_response(&mut account, &response, &context);

        let auth_raw = account
            .trae_auth_raw
            .as_ref()
            .and_then(Value::as_object)
            .expect("auth raw should be object");

        assert_eq!(account.access_token, "new-access");
        assert_eq!(account.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(
            auth_raw.get("host").and_then(Value::as_str),
            Some("https://api-sg-central.trae.ai")
        );
        assert_eq!(
            auth_raw.get("refreshExpiredAt").and_then(Value::as_str),
            Some("2026-10-09T16:18:22.466Z")
        );
        assert_eq!(
            auth_raw
                .get("exchangeResponse")
                .and_then(|value| value.get("Result"))
                .and_then(|value| value.get("RefreshExpireAt"))
                .and_then(Value::as_u64),
            Some(1791562702466_u64)
        );
        assert_eq!(
            auth_raw
                .get("account")
                .and_then(|value| value.get("username"))
                .and_then(Value::as_str),
            Some("李杰")
        );
    }

    #[test]
    fn product_auth_client_id_uses_platform_and_quality() {
        let root = serde_json::json!({
            "quality": "stable",
            "iCubeApp": {
                "authConfig": {
                    "TRAE": {
                        "stable": "trae-stable-client"
                    },
                    "SOLO": {
                        "stable": "solo-stable-client"
                    }
                }
            }
        });

        assert_eq!(
            read_product_auth_client_id(&root, TraePlatformKind::Trae).as_deref(),
            Some("trae-stable-client")
        );
        assert_eq!(
            read_product_auth_client_id(&root, TraePlatformKind::TraeSolo).as_deref(),
            Some("solo-stable-client")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_uninstall_display_name_matching_is_platform_scoped() {
        assert!(windows_uninstall_display_name_matches(
            TraePlatformKind::Trae,
            "Trae (User)"
        ));
        assert!(!windows_uninstall_display_name_matches(
            TraePlatformKind::Trae,
            "Trae CN (User)"
        ));
        assert!(windows_uninstall_display_name_matches(
            TraePlatformKind::TraeCn,
            "Trae CN (User)"
        ));
        assert!(windows_uninstall_display_name_matches(
            TraePlatformKind::TraeSoloCn,
            "TRAE Work CN (User)"
        ));
        assert!(windows_uninstall_display_name_matches(
            TraePlatformKind::TraeSoloCn,
            "TRAE SOLO CN"
        ));
        assert!(!windows_uninstall_display_name_matches(
            TraePlatformKind::TraeSolo,
            "TRAE Work CN (User)"
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_registry_path_normalization_handles_icons_and_install_dirs() {
        assert_eq!(
            normalize_windows_registry_path("\"D:\\Apps\\Trae CN\\Trae CN.exe\",0"),
            Some(PathBuf::from("D:\\Apps\\Trae CN\\Trae CN.exe"))
        );
        assert_eq!(
            normalize_windows_registry_path("D:\\Apps\\TRAE SOLO CN\\"),
            Some(PathBuf::from("D:\\Apps\\TRAE SOLO CN\\"))
        );
    }

    #[test]
    fn refresh_routing_context_prefers_stored_dynamic_auth_client_id() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "platformId": "trae_solo",
            "authClientId": TRAE_SOLO_AUTH_CLIENT_ID
        }));
        account.trae_server_raw = Some(serde_json::json!({
            "platform": {
                "authClientId": "solo-dynamic-client"
            }
        }));

        let context = build_refresh_routing_context(&account);

        assert_eq!(context.platform, TraePlatformKind::TraeSolo);
        assert_eq!(context.client_id, "solo-dynamic-client");
    }

    #[test]
    fn platform_scoped_payload_identity_keeps_cn_and_global_accounts_separate() {
        let global_payload = TraeImportPayload {
            email: "same@example.com".to_string(),
            user_id: Some("user-1".to_string()),
            nickname: None,
            access_token: "access-global".to_string(),
            refresh_token: Some("refresh-global".to_string()),
            token_type: None,
            expires_at: None,
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: Some(serde_json::json!({
                "platformId": "trae",
                "authDomain": "www.trae.ai",
                "authClientId": TRAE_AUTH_CLIENT_ID
            })),
            trae_profile_raw: None,
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: None,
            status: None,
            status_reason: None,
        };
        let cn_payload = TraeImportPayload {
            trae_auth_raw: Some(serde_json::json!({
                "platformId": "trae_cn",
                "authDomain": "www.trae.cn",
                "authClientId": TRAE_AUTH_CLIENT_ID
            })),
            access_token: "access-cn".to_string(),
            refresh_token: Some("refresh-cn".to_string()),
            ..global_payload.clone()
        };

        let global_platform = resolve_payload_platform_kind(&global_payload);
        let cn_platform = resolve_payload_platform_kind(&cn_payload);

        assert_eq!(global_platform, TraePlatformKind::Trae);
        assert_eq!(cn_platform, TraePlatformKind::TraeCn);
        assert_ne!(
            resolve_platform_scoped_payload_identity(global_platform, &global_payload),
            resolve_platform_scoped_payload_identity(cn_platform, &cn_payload)
        );
    }

    #[test]
    fn ensure_auth_raw_for_inject_preserves_unknown_fields_from_existing_payload() {
        let mut account = sample_account();
        account.nickname = Some("tester".to_string());
        account.trae_auth_raw = Some(serde_json::json!({
            "strictToken": "from-source",
            "account": {
                "sourceBadge": "raw",
                "username": "raw-name"
            },
            "userRegion": {
                "sourceRegionMeta": "raw",
                "_aiRegion": "US",
                "region": "US"
            }
        }));
        let existing = serde_json::json!({
            "existingOnly": "keep-me",
            "tokenType": "Bearer",
            "account": {
                "tenantId": "tenant-123",
                "username": "old-name"
            },
            "userRegion": {
                "resolvedFrom": "storage",
                "_aiRegion": "US",
                "region": "US"
            }
        });

        let auth_raw = ensure_auth_raw_for_inject(&account, Some(&existing));
        let auth_obj = auth_raw.as_object().expect("auth raw should be object");

        assert_eq!(
            auth_obj.get("existingOnly").and_then(Value::as_str),
            Some("keep-me")
        );
        assert_eq!(
            auth_obj.get("strictToken").and_then(Value::as_str),
            Some("from-source")
        );
        assert_eq!(
            auth_obj.get("accessToken").and_then(Value::as_str),
            Some("old-access")
        );
        assert_eq!(
            auth_obj.get("token").and_then(Value::as_str),
            Some("old-access")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("tenantId"))
                .and_then(Value::as_str),
            Some("tenant-123")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("sourceBadge"))
                .and_then(Value::as_str),
            Some("raw")
        );
        assert_eq!(
            auth_obj
                .get("userRegion")
                .and_then(|value| value.get("resolvedFrom"))
                .and_then(Value::as_str),
            Some("storage")
        );
        assert_eq!(
            auth_obj
                .get("userRegion")
                .and_then(|value| value.get("sourceRegionMeta"))
                .and_then(Value::as_str),
            Some("raw")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("username"))
                .and_then(Value::as_str),
            Some("tester")
        );
        assert_eq!(
            auth_obj
                .get("userRegion")
                .and_then(|value| value.get("_aiRegion"))
                .and_then(Value::as_str),
            Some("SG")
        );
    }

    #[test]
    fn ensure_auth_raw_for_inject_recovers_refresh_expiry_and_host() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "host": "https://www.trae.ai",
            "storeRegion": "SG",
            "AIRegion": "SG",
            "loginRegion": "sg",
            "Result": {
                "RefreshExpireAt": 1791562702466_u64
            }
        }));

        let auth_raw = ensure_auth_raw_for_inject(&account, None);
        let auth_obj = auth_raw.as_object().expect("auth raw should be object");

        assert_eq!(
            auth_obj.get("host").and_then(Value::as_str),
            Some("https://www.trae.ai")
        );
        assert_eq!(
            auth_obj.get("loginHost").and_then(Value::as_str),
            Some("https://www.trae.ai")
        );
        assert_eq!(
            auth_obj.get("accessToken").and_then(Value::as_str),
            Some("old-access")
        );
        assert_eq!(
            auth_obj.get("refreshExpiredAt").and_then(Value::as_str),
            Some("2026-10-09T16:18:22.466Z")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("username"))
                .and_then(Value::as_str),
            Some("李杰")
        );
    }

    #[test]
    fn ensure_auth_raw_for_inject_uses_official_scope_pair() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "callbackQuery": {
                "scope": "trae"
            }
        }));

        let auth_raw = ensure_auth_raw_for_inject(&account, None);
        let auth_obj = auth_raw.as_object().expect("auth raw should be object");

        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("scope"))
                .and_then(Value::as_str),
            Some("marscode")
        );
        assert_eq!(
            auth_obj
                .get("account")
                .and_then(|value| value.get("loginScope"))
                .and_then(Value::as_str),
            Some("trae")
        );
    }

    #[test]
    fn ensure_auth_raw_for_inject_prefers_callback_host_for_storage() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "host": "https://growsg-normal.trae.ai",
            "loginHost": "https://growsg-normal.trae.ai",
            "callbackQuery": {
                "host": "https://api-sg-central.trae.ai",
                "scope": "trae"
            },
            "storeRegion": "SG",
            "AIRegion": "SG",
            "loginRegion": "sg"
        }));

        let auth_raw = ensure_auth_raw_for_inject(&account, None);
        let auth_obj = auth_raw.as_object().expect("auth raw should be object");

        assert_eq!(
            auth_obj.get("host").and_then(Value::as_str),
            Some("https://api-sg-central.trae.ai")
        );
        assert_eq!(
            auth_obj.get("loginHost").and_then(Value::as_str),
            Some("https://api-sg-central.trae.ai")
        );
    }

    #[test]
    fn device_proof_message_matches_official_newline_format() {
        let message = build_device_proof_message("client-1", "refresh-1", 1_783_355_376, "nonce-1");

        assert_eq!(
            message,
            "POST\n/trae/api/v3/oauth/ExchangeToken\nclient-1\nrefresh-1\n1783355376\nnonce-1"
        );
    }

    #[test]
    fn storage_provider_resolution_ignores_device_key_pair_entries() {
        let mut root = Map::new();
        root.insert(
            "iCubeAuthInfo://icube-dc:7633793279305631249".to_string(),
            Value::String("device-key-pair".to_string()),
        );
        root.insert(
            "iCubeEntitlementInfo://icube-dc:7633793279305631249".to_string(),
            Value::String("{}".to_string()),
        );

        assert_eq!(
            resolve_storage_provider_id(&root),
            TRAE_DEFAULT_AUTH_PROVIDER_ID
        );
        assert!(!has_trae_auth_storage_key(&root));

        root.insert(
            TRAE_STORAGE_AUTH_KEY.to_string(),
            Value::String("auth-payload".to_string()),
        );
        assert_eq!(
            resolve_storage_provider_id(&root),
            TRAE_DEFAULT_AUTH_PROVIDER_ID
        );
        assert!(has_trae_auth_storage_key(&root));
    }

    #[test]
    fn device_key_pair_for_inject_uses_official_device_storage_key() {
        let mut account = sample_account();
        account.trae_auth_raw = Some(serde_json::json!({
            "deviceInfo": {
                "DeviceID": "7633793279305631249"
            },
            "deviceKeyPair": {
                "privateKeyPEM": "private-key",
                "publicKeyPEM": "public-key"
            }
        }));
        let mut root = Map::new();

        write_device_key_pair_for_inject(&mut root, &account).expect("write device key");

        assert!(root.contains_key("iCubeAuthInfo://icube-dc:7633793279305631249"));
        assert!(!has_trae_auth_storage_key(&root));
        let decoded = root
            .get("iCubeAuthInfo://icube-dc:7633793279305631249")
            .and_then(|value| parse_value_or_json_string_or_icube_cipher(Some(value)))
            .expect("decoded device key");
        assert_eq!(
            decoded.get("privateKeyPEM").and_then(Value::as_str),
            Some("private-key")
        );
        assert_eq!(
            decoded.get("publicKeyPEM").and_then(Value::as_str),
            Some("public-key")
        );
    }
